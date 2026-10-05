// @author kongweiguang

use super::*;

const MENU_POINTER_LEAVE_CLOSE_DELAY: Duration = Duration::from_millis(180);

#[path = "resident_commit.rs"]
mod resident_commit;

impl Editor {
    /// 复用 Editor 现有 runtime 构建器初始化 Split 右侧 mounted 文档，随后恢复左侧 Source。
    pub(super) fn refresh_split_virtual_preview_runtime(&mut self, cx: &mut Context<Self>) {
        let Some(mut state) = self.split_preview.take() else {
            return;
        };
        let Some(surface) = state.virtual_surface.take() else {
            self.split_preview = Some(state);
            return;
        };

        let source_document = std::mem::replace(&mut self.document, state.document);
        let source_cells = std::mem::replace(&mut self.table_cells, state.table_cells);
        debug_assert!(self.virtual_surface.is_none());
        self.virtual_surface = Some(surface);
        self.rebuild_virtual_table_runtimes(cx);
        let source_ranges = self
            .virtual_surface
            .as_ref()
            .map(|surface| {
                self.document
                    .visible_blocks()
                    .iter()
                    .filter_map(|visible| {
                        surface
                            .source_range_for_entity(visible.entity.entity_id())
                            .map(|range| (visible.entity.entity_id(), range))
                    })
                    .collect::<HashMap<_, _>>()
            })
            .unwrap_or_default();
        Self::set_document_read_only(&self.document, &self.table_cells, true, cx);

        state.document = std::mem::replace(&mut self.document, source_document);
        state.table_cells = std::mem::replace(&mut self.table_cells, source_cells);
        state.source_ranges = source_ranges;
        state.virtual_surface = self.virtual_surface.take();
        self.split_preview = Some(state);
    }

    /// 根据右侧独立滚动位置惰性换窗，不触碰左侧完整 Source Entity。
    pub(in crate::editor) fn sync_split_virtual_surface_mounts(&mut self, cx: &mut Context<Self>) {
        let Some(mut state) = self.split_preview.take() else {
            return;
        };
        let Some(mut surface) = state.virtual_surface.take() else {
            self.split_preview = Some(state);
            return;
        };
        let viewport_height = f32::from(state.scroll_handle.bounds().size.height.max(px(1.0)));
        let max_scroll_y = f32::from(state.scroll_handle.max_offset().height.max(px(0.0)));
        let scroll_y = (-f32::from(state.scroll_handle.offset().y)).clamp(0.0, max_scroll_y);
        let target = surface.desired_window(scroll_y, viewport_height, 800.0, None);
        if surface.mount_window() == &target {
            state.virtual_surface = Some(surface);
            self.split_preview = Some(state);
            return;
        }
        surface.reconcile_mounts(target, cx);
        let roots = surface.viewport_roots();
        state.virtual_surface = Some(surface);
        if !roots.is_empty() {
            state.document.replace_roots(roots, cx);
            state.previous_visible_ids.clear();
            state.previous_render_window = None;
            state.row_stride_cache.clear();
        }
        self.split_preview = Some(state);
        self.refresh_split_virtual_preview_runtime(cx);
    }

    pub(in crate::editor) fn schedule_split_preview_projection(&mut self, cx: &mut Context<Self>) {
        if self.view_mode != ViewMode::Split {
            return;
        }

        let snapshot = self.source_document.snapshot();
        let revision = snapshot.revision();
        let previous = self.projection_cache.as_ref().map(Arc::clone);
        let regions_only = self
            .split_preview
            .as_ref()
            .is_some_and(|state| state.virtual_surface.is_some());
        self.split_projection_scheduled_revision = Some(revision);
        self.split_projection_task = Some(cx.spawn(async move |this: WeakEntity<Self>, cx| {
            cx.background_executor()
                .timer(Self::SPLIT_PROJECTION_DEBOUNCE)
                .await;
            let prepared = cx
                .background_spawn(async move {
                    Arc::new(if let Some(previous) = previous {
                        if regions_only {
                            PreparedSplitProjection::from_snapshot_incremental_regions_only(
                                snapshot, &previous,
                            )
                        } else {
                            PreparedSplitProjection::from_snapshot_incremental(snapshot, &previous)
                        }
                    } else {
                        PreparedSplitProjection::from_snapshot_adaptive(
                            snapshot,
                            Self::VIRTUAL_SURFACE_REGION_THRESHOLD,
                        )
                    })
                })
                .await;
            let _ = this.update(cx, |editor, cx| {
                editor.split_projection_task = None;
                if editor.split_projection_scheduled_revision != Some(prepared.revision) {
                    return;
                }
                if editor.apply_prepared_split_projection(prepared, cx) {
                    editor.split_projection_scheduled_revision = None;
                    cx.notify();
                }
            });
        }));
    }

    /// 为 Live/Source 合并后台语义计算；结果只发布到共享缓存，不重建当前 Entity。
    pub(in crate::editor) fn schedule_projection_cache_refresh(&mut self, cx: &mut Context<Self>) {
        if matches!(self.view_mode, ViewMode::Split | ViewMode::Preview) {
            return;
        }

        let snapshot = self.source_document.snapshot();
        let revision = snapshot.revision();
        if self
            .projection_cache
            .as_ref()
            .is_some_and(|cached| cached.revision == revision)
        {
            self.projection_cache_task = None;
            self.projection_cache_scheduled_revision = None;
            return;
        }
        let previous = self.projection_cache.as_ref().map(Arc::clone);
        let regions_only = self.virtual_surface.is_some();
        let debounce = if regions_only {
            Self::VIRTUAL_PROJECTION_DEBOUNCE
        } else {
            Self::SPLIT_PROJECTION_DEBOUNCE
        };
        self.projection_cache_scheduled_revision = Some(revision);
        self.projection_cache_task = Some(cx.spawn(async move |this: WeakEntity<Self>, cx| {
            cx.background_executor().timer(debounce).await;
            let prepared = cx
                .background_spawn(async move {
                    Arc::new(if let Some(previous) = previous {
                        if regions_only {
                            PreparedSplitProjection::from_snapshot_incremental_regions_only(
                                snapshot, &previous,
                            )
                        } else {
                            PreparedSplitProjection::from_snapshot_incremental(snapshot, &previous)
                        }
                    } else {
                        PreparedSplitProjection::from_snapshot_adaptive(
                            snapshot,
                            Self::VIRTUAL_SURFACE_REGION_THRESHOLD,
                        )
                    })
                })
                .await;
            let _ = this.update(cx, |editor, cx| {
                editor.projection_cache_task = None;
                if editor.projection_cache_scheduled_revision != Some(prepared.revision)
                    || editor.source_document.revision() != prepared.revision
                    || matches!(editor.view_mode, ViewMode::Split | ViewMode::Preview)
                {
                    return;
                }
                if editor.virtual_surface.is_some() {
                    editor.install_virtual_surface_projection(Arc::clone(&prepared), cx);
                }
                editor.projection_cache = Some(prepared);
                editor.projection_cache_scheduled_revision = None;
            });
        }));
    }

    /// 仅应用仍与 Rope 当前版本一致的投影请求。
    #[cfg(test)]
    pub(in crate::editor) fn apply_split_preview_projection_revision(
        &mut self,
        revision: Revision,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.view_mode != ViewMode::Split || self.source_document.revision() != revision {
            return false;
        }
        let prepared = Arc::new(PreparedSplitProjection::from_snapshot(
            self.source_document.snapshot(),
        ));
        self.install_split_preview_projection(prepared, cx);
        true
    }

    fn apply_prepared_split_projection(
        &mut self,
        prepared: Arc<PreparedSplitProjection>,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.view_mode != ViewMode::Split || self.source_document.revision() != prepared.revision
        {
            return false;
        }
        self.install_split_preview_projection(prepared, cx);
        true
    }

    /// 使用源码字节锚点同步双栏滚动；首帧尚无文本布局时按滚动比例回退。
    pub(in crate::editor) fn sync_split_scroll_handles(&mut self, cx: &App) {
        let Some(driver) = self
            .split_preview
            .as_mut()
            .and_then(|state| state.scroll_driver.take())
        else {
            return;
        };

        let anchor_target = match driver {
            SplitScrollDriver::Source => {
                let source_offset = self.document.first_root().and_then(|block| {
                    block
                        .read(cx)
                        .text_offset_for_window_y(self.scroll_handle.bounds().top())
                });
                source_offset.and_then(|offset| {
                    let state = self.split_preview.as_ref()?;
                    let preview_viewport_top = state.scroll_handle.bounds().top();
                    let target_top = state
                        .document
                        .visible_blocks()
                        .iter()
                        .filter_map(|visible| {
                            let range = state.source_ranges.get(&visible.entity.entity_id())?;
                            let bounds = visible.entity.read(cx).last_bounds?;
                            let distance = if offset < range.start {
                                range.start - offset
                            } else {
                                offset.saturating_sub(range.end)
                            };
                            Some((distance, range.len(), bounds.top()))
                        })
                        .min_by_key(|(distance, len, _)| (*distance, *len))?
                        .2;
                    Some((SplitScrollDriver::Source, preview_viewport_top, target_top))
                })
            }
            SplitScrollDriver::Preview => (|| {
                let state = self.split_preview.as_ref()?;
                let preview_top = state.scroll_handle.bounds().top();
                let source_offset = state
                    .document
                    .visible_blocks()
                    .iter()
                    .filter_map(|visible| {
                        let bounds = visible.entity.read(cx).last_bounds?;
                        let distance = if preview_top < bounds.top() {
                            f32::from(bounds.top() - preview_top)
                        } else if preview_top > bounds.bottom() {
                            f32::from(preview_top - bounds.bottom())
                        } else {
                            0.0
                        };
                        let range = state.source_ranges.get(&visible.entity.entity_id())?;
                        Some((distance, range.len(), range.start))
                    })
                    .min_by(|left, right| {
                        left.0
                            .total_cmp(&right.0)
                            .then_with(|| left.1.cmp(&right.1))
                    })?
                    .2;
                let source_y = self
                    .document
                    .first_root()
                    .and_then(|block| block.read(cx).window_y_for_text_offset(source_offset))?;
                Some((
                    SplitScrollDriver::Preview,
                    self.scroll_handle.bounds().top(),
                    source_y,
                ))
            })(),
        };

        if let Some((target, viewport_top, anchor_top)) = anchor_target {
            match target {
                SplitScrollDriver::Source => {
                    let state = self
                        .split_preview
                        .as_ref()
                        .expect("Split 模式必须持有右侧预览投影");
                    let next_y = state.scroll_handle.offset().y + viewport_top - anchor_top;
                    state
                        .scroll_handle
                        .set_offset(point(px(0.0), next_y.min(px(0.0))));
                }
                SplitScrollDriver::Preview => {
                    let next_y = self.scroll_handle.offset().y + viewport_top - anchor_top;
                    self.scroll_handle
                        .set_offset(point(px(0.0), next_y.min(px(0.0))));
                }
            }
            return;
        }

        let state = self
            .split_preview
            .as_ref()
            .expect("Split 模式必须持有右侧预览投影");
        let source_max = f32::from(self.scroll_handle.max_offset().height.max(px(0.0)));
        let preview_max = f32::from(state.scroll_handle.max_offset().height.max(px(0.0)));
        match driver {
            SplitScrollDriver::Source if source_max > 0.0 => {
                let source_y = -f32::from(self.scroll_handle.offset().y);
                let ratio = (source_y / source_max).clamp(0.0, 1.0);
                state
                    .scroll_handle
                    .set_offset(point(px(0.0), px(-preview_max * ratio)));
            }
            SplitScrollDriver::Preview if preview_max > 0.0 => {
                let preview_y = -f32::from(state.scroll_handle.offset().y);
                let ratio = (preview_y / preview_max).clamp(0.0, 1.0);
                self.scroll_handle
                    .set_offset(point(px(0.0), px(-source_max * ratio)));
            }
            _ => {}
        }
    }

    /// 按真实表面提交；常驻 Live 的结构操作保留未变区域原文，迟到命令仍不能越过只读保护。
    pub(in crate::editor) fn mark_dirty(&mut self, cx: &mut Context<Self>) {
        if !self.document_surface_is_editable() || self.document.source_commit_error().is_some() {
            return;
        }
        if self.virtual_surface.is_some() && self.view_mode == ViewMode::Rendered {
            let result = self
                .active_entity_id
                .ok_or_else(|| "当前虚拟输入没有可验证的目标".to_owned())
                .and_then(|entity_id| self.mark_virtual_block_dirty(entity_id, cx));
            if let Err(error) = result {
                self.retain_resident_source_commit_error(error, cx);
            }
            return;
        }
        if self.view_mode == ViewMode::Rendered && self.virtual_surface.is_none() {
            if let Err(error) = self.commit_resident_structure_regions(cx) {
                self.retain_resident_source_commit_error(error, cx);
            }
            return;
        }
        self.document.rebuild_root_markdown_cache(cx);
        self.mark_dirty_from_cached_markdown(cx);
    }

    /// 历史恢复已将权威源码写回 SourceDocument；保留其原始 Markdown 拼写并复用完整 dirty 调度。
    pub(in crate::editor) fn mark_restored_document_dirty(&mut self, cx: &mut Context<Self>) {
        if !self.document_surface_is_editable() {
            return;
        }
        self.document.rebuild_root_markdown_cache(cx);
        let source = self.source_document.text();
        self.finish_marking_document_dirty(source, cx);
    }

    /// Live 写入只覆盖有原源码归属的区域，后台缓存滞后不得把未编辑根规范化；其它表面沿用自身源码路径。
    pub(in crate::editor) fn mark_block_dirty(
        &mut self,
        entity_id: EntityId,
        cx: &mut Context<Self>,
    ) {
        if !self.document_surface_is_editable() || self.document.source_commit_error().is_some() {
            return;
        }
        if self.virtual_surface.is_some() && self.view_mode == ViewMode::Rendered {
            if let Err(error) = self.mark_virtual_block_dirty(entity_id, cx) {
                self.retain_resident_source_commit_error(error, cx);
            }
            return;
        }
        if self.view_mode == ViewMode::Rendered && self.virtual_surface.is_none() {
            if let Err(error) = self.commit_resident_block_region(entity_id, cx) {
                self.retain_resident_source_commit_error(error, cx);
            }
            return;
        }
        self.document
            .refresh_markdown_cache_for_entity(entity_id, cx);
        self.mark_dirty_from_cached_markdown(cx);
    }

    /// 挂载区域只授权局部事务；归属或读取失败必须保留树态，不能转到全文投影写入。
    fn mark_virtual_block_dirty(
        &mut self,
        entity_id: EntityId,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let _trace = crate::perf::span("virtual_source_commit_sync");
        let (source_range, roots) = self
            .virtual_surface
            .as_ref()
            .and_then(|surface| {
                Some((
                    surface.source_range_for_entity(entity_id)?,
                    surface.region_roots_for_entity(entity_id)?,
                ))
            })
            .ok_or_else(|| "当前虚拟输入块没有可验证的原源码区域".to_owned())?;
        let markdown = DocumentTree::markdown_text_for_roots(&roots, cx);
        let snapshot = self.source_document.snapshot();
        let old_fragment = snapshot
            .text_for_range(source_range.clone())
            .map_err(|error| error.to_string())?;
        let old_revision = snapshot.revision();
        let transaction = gmark_document::Transaction::new(
            snapshot.revision(),
            vec![gmark_document::TextEdit::new(
                source_range,
                markdown.clone(),
            )],
        );
        self.source_document
            .apply_transaction(transaction)
            .map_err(|error| error.to_string())?;
        if let Some(surface) = self.virtual_surface.as_mut() {
            surface.apply_entity_region_source_len(entity_id, markdown.len());
        }
        self.status_bar.apply_virtual_text_edit(
            old_revision,
            self.source_document.revision(),
            &old_fragment,
            &markdown,
        );
        if std::mem::take(&mut self.pending_virtual_global_runtime_refresh) {
            let source = self.source_document.text();
            self.rebuild_runtime_context_from_markdown(&source, cx);
        }

        self.pending_dirty_source = None;
        self.render_row_cache = None;
        self.schedule_projection_cache_refresh(cx);
        if !self.document_dirty {
            self.document_dirty = true;
            self.pending_window_edited = true;
            self.pending_window_title_refresh = true;
            cx.notify();
        }
        self.schedule_recovery_journal(cx);
        self.schedule_auto_save(cx);
        self.schedule_active_block_spellcheck(cx);
        Ok(())
    }

    /// 从已更新的投影生成规范 Markdown，并同步到权威源码后调度保存。
    fn mark_dirty_from_cached_markdown(&mut self, cx: &mut Context<Self>) {
        let source = self.current_document_source_from_cache(cx);
        self.sync_source_document_from_projection(&source);
        self.finish_marking_document_dirty(source, cx);
    }

    /// 两种正文来源共用 dirty 状态、投影刷新与持久化调度，避免恢复路径重写权威源码。
    fn finish_marking_document_dirty(&mut self, source: String, cx: &mut Context<Self>) {
        self.pending_dirty_source = Some(source);
        self.render_row_cache = None;
        match self.view_mode {
            ViewMode::Split => self.schedule_split_preview_projection(cx),
            ViewMode::Rendered | ViewMode::Source => self.schedule_projection_cache_refresh(cx),
            ViewMode::Preview => {}
        }
        if !self.document_dirty {
            self.document_dirty = true;
            self.pending_window_edited = true;
            self.pending_window_title_refresh = true;
            cx.notify();
        }
        self.schedule_recovery_journal(cx);
        self.schedule_auto_save(cx);
        self.schedule_active_block_spellcheck(cx);
    }

    pub(in crate::editor) fn request_active_block_scroll_into_view(
        &mut self,
        cx: &mut Context<Self>,
    ) {
        self.pending_scroll_recheck_after_layout = true;
        if !self.pending_scroll_active_block_into_view {
            self.pending_scroll_active_block_into_view = true;
            cx.notify();
        }
    }

    pub(in crate::editor) fn viewport_size_changed(
        previous: Size<Pixels>,
        current: Size<Pixels>,
    ) -> bool {
        const EPSILON: f32 = 0.5;

        (f32::from(previous.width) - f32::from(current.width)).abs() > EPSILON
            || (f32::from(previous.height) - f32::from(current.height)).abs() > EPSILON
    }

    pub(crate) fn show_info_dialog(&mut self, kind: InfoDialogKind, cx: &mut Context<Self>) {
        if self.show_unsaved_changes_dialog {
            return;
        }

        self.menu_bar_open = None;
        self.menu_submenu_open = None;
        self.menu_keyboard_item = None;
        self.menu_keyboard_submenu_item = None;
        self.menu_submenu_panel_hovered = false;
        self.menu_submenu_bridge_hovered = false;
        self.info_dialog = Some(kind);
        cx.notify();
    }

    pub(crate) fn hide_info_dialog(&mut self, cx: &mut Context<Self>) {
        if self.info_dialog.take().is_some() {
            cx.notify();
        }
    }

    /// 应用图标只打开/关闭它自身的下拉面板；一级导航始终可见。
    pub(crate) fn toggle_menu_bar_expanded(&mut self, cx: &mut Context<Self>) {
        if self.menu_bar_open == Some(0) {
            self.close_menu_bar(cx);
            cx.notify();
        } else {
            self.menu_bar_expanded = true;
            self.open_menu_bar(0, cx);
            cx.notify();
        }
    }

    /// Requests native IME completion on deactivation so staged text is not mistaken for committed input.
    pub(crate) fn install_menu_window_activation_observer(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.menu_window_activation_subscription.is_some() {
            return;
        }
        self.menu_window_activation_subscription =
            Some(cx.observe_window_activation(window, |editor, window, cx| {
                let window_active = window.is_window_active();
                if let Some(block) = editor.focused_edit_target(window, cx) {
                    block.update(cx, |block, cx| {
                        block.set_cursor_blink_window_active(window_active, cx)
                    });
                }
                if !window_active {
                    editor.wait_for_ime_completion(window, cx);
                    // 失焦只需关闭瞬时下拉面板，不隐藏一级导航。否则窗口重回前台时
                    // 会看起来像菜单丢失，也与高频浏览路径相冲突。
                    editor.close_menu_panels(cx);
                    cx.notify();
                } else {
                    editor.refresh_resource_probes(cx);
                    #[cfg(target_os = "macos")]
                    editor.schedule_platform_document_menu_refresh(cx);
                }
            }));
    }

    pub(crate) fn open_menu_bar(&mut self, index: usize, cx: &mut Context<Self>) {
        self.menu_close_task = None;
        if self.menu_bar_open != Some(index) {
            self.menu_bar_open = Some(index);
            self.menu_submenu_open = None;
            self.menu_keyboard_item = None;
            self.menu_keyboard_submenu_item = None;
            self.menu_submenu_panel_hovered = false;
            self.menu_submenu_bridge_hovered = false;
            cx.notify();
        }
    }

    pub(crate) fn open_menu_submenu(&mut self, index: usize, cx: &mut Context<Self>) {
        self.menu_close_task = None;
        if self.menu_submenu_open != Some(index) {
            self.menu_submenu_open = Some(index);
            self.menu_keyboard_submenu_item = None;
            cx.notify();
        }
    }

    pub(crate) fn clear_menu_keyboard_cursor(&mut self, cx: &mut Context<Self>) {
        let changed = self.menu_keyboard_item.take().is_some()
            || self.menu_keyboard_submenu_item.take().is_some();
        if changed {
            cx.notify();
        }
    }

    pub(crate) fn close_menu_submenu(&mut self, cx: &mut Context<Self>) {
        let had_open_submenu = self.menu_submenu_open.take().is_some();
        let had_keyboard_submenu = self.menu_keyboard_submenu_item.take().is_some();
        let had_submenu_hover = self.menu_submenu_panel_hovered || self.menu_submenu_bridge_hovered;
        self.menu_submenu_panel_hovered = false;
        self.menu_submenu_bridge_hovered = false;
        if had_open_submenu || had_keyboard_submenu || had_submenu_hover {
            cx.notify();
        }
    }

    pub(crate) fn set_menu_bar_hovered(&mut self, hovered: bool, cx: &mut Context<Self>) {
        self.menu_bar_hovered = hovered;
        self.refresh_menu_pointer_close(cx);
    }

    pub(crate) fn set_menu_panel_hovered(&mut self, hovered: bool, cx: &mut Context<Self>) {
        self.menu_panel_hovered = hovered;
        self.refresh_menu_pointer_close(cx);
    }

    pub(crate) fn set_menu_submenu_panel_hovered(&mut self, hovered: bool, cx: &mut Context<Self>) {
        self.menu_submenu_panel_hovered = hovered;
        self.refresh_menu_pointer_close(cx);
    }

    /// Hover handler for the invisible gap bridge. The bridge and the submenu
    /// panel overlap, so the cursor crossing between them fires a `false` for
    /// one region and a `true` for the other in the same gesture. Keeping their
    /// hover state in separate flags lets either one hold the menu open
    /// regardless of the order those events arrive.
    pub(crate) fn set_menu_submenu_bridge_hovered(
        &mut self,
        hovered: bool,
        cx: &mut Context<Self>,
    ) {
        self.menu_submenu_bridge_hovered = hovered;
        self.refresh_menu_pointer_close(cx);
    }

    fn refresh_menu_pointer_close(&mut self, cx: &mut Context<Self>) {
        let pointer_inside = self.menu_bar_hovered
            || self.menu_panel_hovered
            || self.menu_submenu_panel_hovered
            || self.menu_submenu_bridge_hovered;
        if pointer_inside || self.menu_bar_open.is_none() {
            self.menu_close_task = None;
            return;
        }

        // 菜单栏、主面板与子菜单之间存在窄缝；短暂延迟既允许指针跨越缝隙，
        // 又保证真正离开整组菜单后面板会自行收起。
        self.menu_close_task = Some(cx.spawn(async move |editor, cx| {
            cx.background_executor()
                .timer(MENU_POINTER_LEAVE_CLOSE_DELAY)
                .await;
            let _ = editor.update(cx, |editor, cx| {
                editor.menu_close_task = None;
                let pointer_inside = editor.menu_bar_hovered
                    || editor.menu_panel_hovered
                    || editor.menu_submenu_panel_hovered
                    || editor.menu_submenu_bridge_hovered;
                if !pointer_inside {
                    editor.close_menu_panels(cx);
                }
            });
        }));
    }

    pub(crate) fn dismiss_menu_bar_from_body(&mut self, cx: &mut Context<Self>) {
        if self.menu_bar_open.is_some() {
            self.close_menu_bar(cx);
            cx.notify();
        }
    }

    /// 明确用户请求可重试失败的系统完成操作；渲染本身不循环重发。
    pub(crate) fn request_save_document(&mut self, cx: &mut Context<Self>) {
        if self.queue_document_action_for_ime(&crate::components::SaveDocument, cx) {
            return;
        }
        if !self.pending_save {
            self.pending_save = true;
            cx.notify();
        }
    }

    /// 另存为复用同一候选终态边界，并允许用户显式重试。
    pub(crate) fn request_save_document_as(&mut self, cx: &mut Context<Self>) {
        if self.queue_document_action_for_ime(&crate::components::SaveDocumentAs, cx) {
            return;
        }
        if !self.pending_save_as {
            self.pending_save_as = true;
            cx.notify();
        }
    }

    pub(crate) fn request_open_link_prompt(
        &mut self,
        prompt_target: String,
        open_target: String,
        cx: &mut Context<Self>,
    ) {
        self.pending_open_link = Some(PendingOpenLink {
            prompt_target,
            open_target,
        });
        cx.notify();
    }

    pub(crate) fn close_menu_bar(&mut self, cx: &mut Context<Self>) {
        self.menu_bar_expanded = true;
        self.close_menu_panels(cx);
    }

    /// 视窗失焦和菜单动作完成只能关闭瞬时面板，不得隐式改变用户选择的导航展开状态。
    pub(crate) fn close_menu_panels(&mut self, cx: &mut Context<Self>) {
        let had_open_menu = self.menu_bar_open.take().is_some();
        let had_open_submenu = self.menu_submenu_open.take().is_some();
        let had_keyboard_item = self.menu_keyboard_item.take().is_some();
        let had_keyboard_submenu = self.menu_keyboard_submenu_item.take().is_some();
        let had_hover_state = self.menu_bar_hovered
            || self.menu_panel_hovered
            || self.menu_submenu_panel_hovered
            || self.menu_submenu_bridge_hovered;
        let had_pending_close = self.menu_close_task.take().is_some();
        self.menu_bar_hovered = false;
        self.menu_panel_hovered = false;
        self.menu_submenu_panel_hovered = false;
        self.menu_submenu_bridge_hovered = false;
        if had_open_menu
            || had_open_submenu
            || had_keyboard_item
            || had_keyboard_submenu
            || had_hover_state
            || had_pending_close
        {
            cx.notify();
        }
    }
}
