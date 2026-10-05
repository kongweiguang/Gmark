// @author kongweiguang

use super::*;

#[path = "view.rs"]
mod view;

impl Editor {
    /// 查找先等候选终态，再转给实际编辑窗格，不能搜索窗口壳的空正文。
    pub(crate) fn on_find_in_document_action(
        &mut self,
        _: &crate::components::FindInDocument,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.defer_action_for_ime(&crate::components::FindInDocument, window, cx) {
            cx.stop_propagation();
            return;
        }
        if !self.pane_canvas
            && let Some(editor) = self.focused_pane_entities(cx).0
        {
            editor.update(cx, |editor, cx| {
                editor.on_find_in_document_action(&crate::components::FindInDocument, window, cx);
            });
            return;
        }
        if let Some(document_host) = self.document_host.clone() {
            document_host.update(cx, |document_host, cx| {
                document_host.on_find_in_document(&crate::components::FindInDocument, window, cx);
            });
            return;
        }
        self.open_find_panel(false, window, cx);
    }

    /// 替换复用所属文档的候选门控与面板，保留原输入目标和只读边界。
    pub(crate) fn on_replace_in_document_action(
        &mut self,
        _: &crate::components::ReplaceInDocument,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.defer_action_for_ime(&crate::components::ReplaceInDocument, window, cx) {
            cx.stop_propagation();
            return;
        }
        if !self.pane_canvas
            && let Some(editor) = self.focused_pane_entities(cx).0
        {
            editor.update(cx, |editor, cx| {
                editor.on_replace_in_document_action(
                    &crate::components::ReplaceInDocument,
                    window,
                    cx,
                );
            });
            return;
        }
        if let Some(document_host) = self.document_host.clone() {
            document_host.update(cx, |document_host, cx| {
                document_host.on_find_in_document(&crate::components::FindInDocument, window, cx);
            });
            return;
        }
        self.open_find_panel(true, window, cx);
    }

    /// 候选未结束时暂缓导航，匹配始终取自当前窗格而非父壳。
    pub(crate) fn on_find_next_action(
        &mut self,
        _: &crate::components::FindNext,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.defer_action_for_ime(&crate::components::FindNext, window, cx) {
            cx.stop_propagation();
            return;
        }
        if !self.pane_canvas
            && let Some(editor) = self.focused_pane_entities(cx).0
        {
            editor.update(cx, |editor, cx| {
                editor.on_find_next_action(&crate::components::FindNext, window, cx);
            });
            return;
        }
        if let Some(document_host) = self.document_host.clone() {
            document_host.update(cx, |document_host, cx| {
                document_host.on_find_next(&crate::components::FindNext, window, cx);
            });
            return;
        }
        if self.find_panel.is_none() {
            self.open_find_panel(false, window, cx);
        } else {
            self.navigate_find_match(1, window, cx);
        }
    }

    /// 反向导航沿用同一窗格归属，避免焦点变化后跳到兄弟文档。
    pub(crate) fn on_find_previous_action(
        &mut self,
        _: &crate::components::FindPrevious,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.defer_action_for_ime(&crate::components::FindPrevious, window, cx) {
            cx.stop_propagation();
            return;
        }
        if !self.pane_canvas
            && let Some(editor) = self.focused_pane_entities(cx).0
        {
            editor.update(cx, |editor, cx| {
                editor.on_find_previous_action(&crate::components::FindPrevious, window, cx);
            });
            return;
        }
        if let Some(document_host) = self.document_host.clone() {
            document_host.update(cx, |document_host, cx| {
                document_host.on_find_previous(&crate::components::FindPrevious, window, cx);
            });
            return;
        }
        if self.find_panel.is_none() {
            self.open_find_panel(false, window, cx);
        } else {
            self.navigate_find_match(-1, window, cx);
        }
    }

    /// 将过期的查询和替换任务一并视为 stale，防止文档变更期间用旧结果提交事务。
    pub(in crate::editor) fn refresh_find_if_stale(&mut self, cx: &mut Context<Self>) {
        let stale = self.find_panel.as_ref().is_some_and(|state| {
            state.revision != self.source_document.revision()
                && state.task.is_none()
                && state.replace_task.is_none()
        });
        if stale {
            self.schedule_find(cx);
        }
    }

    /// Keeps the query/replacement input alive until an IME pre-edit reaches a terminal state.
    pub(in crate::editor) fn close_find_panel(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.defer_tool_ime_intent(super::super::tool_ime::ToolImeIntent::FindClose, window, cx)
        {
            return;
        }
        let Some(state) = self.find_panel.take() else {
            return;
        };
        let restore = state.restore_focus;
        if let Some(snapshot) = state.view_state_before_expand {
            self.ensure_markdown_view_state();
            self.view_state
                .replace_tab_state(self.tabs.active_id(), snapshot);
            self.render_row_cache = None;
            self.prev_render_window = None;
            self.row_stride_cache.clear();
        }
        if let Some(restore) = restore {
            let block = match restore.surface {
                super::super::selection_surface::SelectionSurface::Main => self
                    .selection_surface_entities(restore.surface)
                    .into_iter()
                    .find(|block| block.entity_id() == restore.entity_id)
                    .or_else(|| {
                        self.table_cells
                            .values()
                            .find(|binding| binding.cell.entity_id() == restore.entity_id)
                            .map(|binding| binding.cell.clone())
                    }),
                super::super::selection_surface::SelectionSurface::SplitPreview => self
                    .selection_surface_entities(restore.surface)
                    .into_iter()
                    .find(|block| block.entity_id() == restore.entity_id)
                    .or_else(|| {
                        self.split_preview.as_ref().and_then(|preview| {
                            preview
                                .table_cells
                                .values()
                                .find(|binding| binding.cell.entity_id() == restore.entity_id)
                                .map(|binding| binding.cell.clone())
                        })
                    }),
            };
            if let Some(block) = block {
                self.active_selection_surface = restore.surface;
                if restore.surface == super::super::selection_surface::SelectionSurface::Main
                    && self.selection_surface_contains_entity(restore.surface, restore.entity_id)
                {
                    self.focus_block(restore.entity_id);
                } else {
                    block.read(cx).focus_handle.focus(window);
                }
            }
        }
        cx.notify();
    }

    /// Lets candidate navigation and confirmation keys reach the focused composition owner first.
    pub(in crate::editor) fn handle_find_panel_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.has_active_ime_composition(cx) {
            return false;
        }
        let Some(state) = self.find_panel.as_ref() else {
            return false;
        };
        let keyboard_target = state.keyboard_target;
        match event.keystroke.key.as_str() {
            "escape" => self.close_find_panel(window, cx),
            "enter" => {
                if keyboard_target.is_control() {
                    self.activate_find_keyboard_target(keyboard_target, window, cx);
                } else {
                    let delta = if event.keystroke.modifiers.shift {
                        -1
                    } else {
                        1
                    };
                    self.navigate_find_match(delta, window, cx);
                }
            }
            "space" if keyboard_target.is_control() => {
                self.activate_find_keyboard_target(keyboard_target, window, cx);
            }
            "tab" => self.move_find_keyboard_target(event.keystroke.modifiers.shift, window, cx),
            _ => return false,
        }
        true
    }

    /// Defers target changes so a query field remains the native IME owner through END.
    fn move_find_keyboard_target(
        &mut self,
        reverse: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.find_panel.as_ref() else {
            return;
        };
        const FIND_ONLY_ORDER: [FindKeyboardTarget; 7] = [
            FindKeyboardTarget::Query,
            FindKeyboardTarget::CaseSensitive,
            FindKeyboardTarget::WholeWord,
            FindKeyboardTarget::Regex,
            FindKeyboardTarget::Previous,
            FindKeyboardTarget::Next,
            FindKeyboardTarget::Close,
        ];
        const REPLACE_ORDER: [FindKeyboardTarget; 8] = [
            FindKeyboardTarget::Query,
            FindKeyboardTarget::Replacement,
            FindKeyboardTarget::CaseSensitive,
            FindKeyboardTarget::WholeWord,
            FindKeyboardTarget::Regex,
            FindKeyboardTarget::Previous,
            FindKeyboardTarget::Next,
            FindKeyboardTarget::Close,
        ];
        let order = if state.show_replace {
            REPLACE_ORDER.as_slice()
        } else {
            FIND_ONLY_ORDER.as_slice()
        };
        let current = order
            .iter()
            .position(|target| *target == state.keyboard_target)
            .unwrap_or(0);
        let next = if reverse {
            current.checked_sub(1).unwrap_or(order.len() - 1)
        } else {
            (current + 1) % order.len()
        };
        self.focus_find_keyboard_target(order[next], window, cx);
    }

    fn activate_find_keyboard_target(
        &mut self,
        target: FindKeyboardTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match target {
            FindKeyboardTarget::CaseSensitive => {
                self.toggle_find_option(|options| &mut options.case_sensitive, cx)
            }
            FindKeyboardTarget::WholeWord => {
                self.toggle_find_option(|options| &mut options.whole_word, cx)
            }
            FindKeyboardTarget::Regex => self.toggle_find_option(|options| &mut options.regex, cx),
            FindKeyboardTarget::Previous => {
                self.navigate_find_match(-1, window, cx);
                self.focus_find_keyboard_target(FindKeyboardTarget::Previous, window, cx);
            }
            FindKeyboardTarget::Next => {
                self.navigate_find_match(1, window, cx);
                self.focus_find_keyboard_target(FindKeyboardTarget::Next, window, cx);
            }
            FindKeyboardTarget::Close => self.close_find_panel(window, cx),
            FindKeyboardTarget::Query | FindKeyboardTarget::Replacement => {}
        }
    }

    /// Defers focus changes while a find field owns pre-edit text, preserving the original target.
    pub(in crate::editor) fn focus_find_keyboard_target(
        &mut self,
        target: FindKeyboardTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.defer_tool_ime_intent(
            super::super::tool_ime::ToolImeIntent::FindFocus(target),
            window,
            cx,
        ) {
            return;
        }
        let Some(state) = self.find_panel.as_mut() else {
            return;
        };
        state.keyboard_target = target;
        if target.is_control() {
            state.focus_handle.focus(window);
        } else {
            match target {
                FindKeyboardTarget::Query => state.query.read(cx).focus_handle.focus(window),
                FindKeyboardTarget::Replacement => {
                    state.replacement.read(cx).focus_handle.focus(window)
                }
                _ => {}
            }
        }
        cx.notify();
    }

    pub(in crate::editor) fn navigate_find_match(
        &mut self,
        delta: isize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let current_selection = self.capture_source_selection_snapshot(cx).range();
        let Some(state) = self.find_panel.as_mut() else {
            return;
        };
        if state.matches.is_empty() {
            return;
        }
        let len = state.matches.len() as isize;
        if current_selection == state.matches[state.selected] {
            state.selected = (state.selected as isize + delta).rem_euclid(len) as usize;
        } else if delta < 0 {
            state.selected = (state.selected as isize - 1).rem_euclid(len) as usize;
        }
        let range = state.matches[state.selected].clone();
        let visible_offset = state
            .match_metadata
            .get(state.selected)
            .map(|metadata| metadata.visible.start);
        let query = state.query.clone();
        let fold_sources = visible_offset
            .map(|offset| {
                gmark_markdown::parse_markdown(&self.source_document.text())
                    .visible_text_projection()
                    .folds_containing(offset)
                    .map(|fold| fold.source)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let view_state_snapshot = self.view_state.state_for_tab(self.tabs.active_id());
        let mut heading_keys_to_expand = Vec::new();
        let mut callout_keys_to_expand = Vec::new();
        if !fold_sources.is_empty() {
            let source_ranges = self.build_source_target_mappings_with_block_ranges(cx).1;
            for visible in self.document.flatten_visible_blocks() {
                let Some(source_range) = source_ranges.get(&visible.entity.entity_id()).cloned()
                else {
                    continue;
                };
                let (key, heading) = visible.entity.read_with(cx, |block, _cx| {
                    (
                        block
                            .presentation_fold_key
                            .as_ref()
                            .map(ToString::to_string),
                        block.presentation_fold_heading,
                    )
                });
                let Some(key) = key else { continue };
                let matches_fold = fold_sources.iter().any(|source| {
                    source.start == source_range.start && source.end == source_range.end
                });
                if !matches_fold {
                    continue;
                }
                let collapsed = view_state_snapshot.as_ref().is_some_and(|state| {
                    if heading {
                        state.collapsed_headings.get(&key).copied().unwrap_or(false)
                    } else {
                        state.collapsed_callouts.get(&key).copied().unwrap_or(false)
                    }
                });
                if collapsed {
                    if heading {
                        heading_keys_to_expand.push(key);
                    } else {
                        callout_keys_to_expand.push(key);
                    }
                }
            }
        }
        if !heading_keys_to_expand.is_empty() || !callout_keys_to_expand.is_empty() {
            if let Some(state) = self.find_panel.as_mut()
                && state.view_state_before_expand.is_none()
            {
                state.view_state_before_expand = view_state_snapshot;
            }
            self.ensure_markdown_view_state();
            self.view_state
                .update_tab(self.tabs.active_id(), |view_state| {
                    for key in heading_keys_to_expand {
                        view_state.collapsed_headings.remove(&key);
                    }
                    for key in callout_keys_to_expand {
                        view_state.collapsed_callouts.remove(&key);
                    }
                });
            self.render_row_cache = None;
            self.prev_render_window = None;
            self.row_stride_cache.clear();
        }
        if let Some(y) = self
            .virtual_surface
            .as_ref()
            .and_then(|surface| surface.y_for_source_offset(range.start))
        {
            self.scroll_handle.set_offset(point(px(0.0), px(-y)));
            let viewport_height = f32::from(self.scroll_handle.bounds().size.height.max(px(1.0)));
            self.sync_virtual_surface_mounts(y, viewport_height, 800.0, cx);
        }
        self.apply_selection_snapshot_in_current_mode(
            &UndoSelectionSnapshot::from_range(range, false),
            cx,
        );
        self.pending_focus = None;
        self.pending_scroll_active_block_into_view = true;
        self.pending_scroll_recheck_after_layout = true;
        query.read(cx).focus_handle.focus(window);
        cx.notify();
    }

    fn toggle_find_option(
        &mut self,
        option: fn(&mut FindOptions) -> &mut bool,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.find_panel.as_mut() else {
            return;
        };
        let target = option(&mut state.options);
        *target = !*target;
        self.schedule_find(cx);
    }

    /// 在后台从不可变快照规划当前替换，避免单次大正文规划阻塞 GPUI；提交仍由
    /// revision/generation gate 保护，并复用完整事务规划器保持原匹配语义。
    pub(in crate::editor) fn replace_current_find_match(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.document_surface_is_editable() || !self.source_encoding.is_utf8() {
            return;
        }
        if self
            .find_panel
            .as_ref()
            .is_some_and(|state| state.replace_task.is_some())
        {
            return;
        }
        if self.defer_find_replace_for_ime(false, window, cx) {
            return;
        }
        let Some((range, metadata, query, replacement_template, options, revision, generation)) =
            self.find_panel.as_ref().and_then(|state| {
                let range = state.matches.get(state.selected).cloned()?;
                let metadata = state.match_metadata.get(state.selected).cloned()?;
                Some((
                    range,
                    metadata,
                    state.query.read(cx).shared_display_text(),
                    state.replacement.read(cx).shared_display_text(),
                    state.options,
                    state.revision,
                    state.generation,
                ))
            })
        else {
            return;
        };
        if metadata.replaceability != gmark_markdown::Replaceability::Direct {
            return;
        }
        if revision != self.source_document.revision() {
            self.schedule_find(cx);
            return;
        }
        if query.len() > MAX_FIND_QUERY_BYTES {
            if let Some(state) = self.find_panel.as_mut() {
                state.error = Some(format!(
                    "查找内容超过 {} KiB 安全限制",
                    MAX_FIND_QUERY_BYTES / 1024
                ));
            }
            cx.notify();
            return;
        }
        if replacement_template.len() > MAX_REPLACE_OUTPUT_BYTES {
            if let Some(state) = self.find_panel.as_mut() {
                state.error = Some("替换结果超过 64 MiB 安全限制".to_owned());
            }
            cx.notify();
            return;
        }

        let snapshot = self.source_document.snapshot();
        let matches = vec![range];
        let metadata = vec![metadata];
        let query_for_task = query.clone();
        let replacement_for_task = replacement_template.clone();
        if let Some(state) = self.find_panel.as_mut() {
            // 只把不可变快照交给后台；UI 线程不读取完整 source，也不构造替换正文。
            state.replace_task = Some(cx.spawn(async move |this: WeakEntity<Self>, cx| {
                let result = cx
                    .background_spawn(async move {
                        let source = snapshot.text();
                        build_replace_all_plan(
                            &source,
                            query_for_task.as_ref(),
                            replacement_for_task.as_ref(),
                            options,
                            &matches,
                            &metadata,
                        )
                    })
                    .await;
                let _ = this.update(cx, |editor, cx| {
                    let current_revision = editor.source_document.revision();
                    let Some(state) = editor.find_panel.as_mut() else {
                        return;
                    };
                    state.replace_task = None;
                    if !find_replace_result_is_current(
                        revision,
                        generation,
                        current_revision,
                        state.generation,
                    ) {
                        if state.generation == generation && current_revision != revision {
                            state.error = Some("文档已发生变化，请重新搜索".to_owned());
                            cx.notify();
                        }
                        return;
                    }
                    let plan = match result {
                        Ok(plan) => plan,
                        Err(error) => {
                            state.error = Some(error);
                            cx.notify();
                            return;
                        }
                    };
                    if editor.apply_find_edits(plan.edits, plan.selection, cx) {
                        editor.schedule_find(cx);
                    }
                });
            }));
        }
        // 后台规划期间保留查询框焦点，完成后 selection gate 决定是否提交。
        if let Some(state) = self.find_panel.as_ref() {
            state.query.read(cx).focus_handle.focus(window);
        }
        cx.notify();
    }

    /// 在后台从不可变快照构造完整替换计划，主线程只在 revision/generation 仍匹配时一次提交。
    pub(in crate::editor) fn replace_all_find_matches(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.document_surface_is_editable() || !self.source_encoding.is_utf8() {
            return;
        }
        if self
            .find_panel
            .as_ref()
            .is_some_and(|state| state.replace_task.is_some())
        {
            return;
        }
        if self.defer_find_replace_for_ime(true, window, cx) {
            return;
        }
        let Some(state) = self.find_panel.as_ref() else {
            return;
        };
        if state.matches.is_empty() || state.revision != self.source_document.revision() {
            return;
        }
        let generation = state.generation;
        let revision = state.revision;
        let query = state.query.read(cx).shared_display_text();
        let replacement_template = state.replacement.read(cx).shared_display_text();
        let options = state.options;
        let matches = state.matches.clone();
        let metadata = state.match_metadata.clone();
        if query.len() > MAX_FIND_QUERY_BYTES {
            if let Some(state) = self.find_panel.as_mut() {
                state.error = Some(format!(
                    "查找内容超过 {} KiB 安全限制",
                    MAX_FIND_QUERY_BYTES / 1024
                ));
            }
            cx.notify();
            return;
        }
        if replacement_template.len() > MAX_REPLACE_OUTPUT_BYTES {
            if let Some(state) = self.find_panel.as_mut() {
                state.error = Some("替换结果超过 64 MiB 安全限制".to_owned());
            }
            cx.notify();
            return;
        }
        let snapshot = self.source_document.snapshot();
        let query_for_task = query.clone();
        let replacement_for_task = replacement_template.clone();
        if let Some(state) = self.find_panel.as_mut() {
            // 计划基于不可变快照构造，避免解析和替换展开占满 GPUI，同时保留一次
            // revision/generation gate 提交。
            state.replace_task = Some(cx.spawn(async move |this: WeakEntity<Self>, cx| {
                let result = cx
                    .background_spawn(async move {
                        let source = snapshot.text();
                        build_replace_all_plan(
                            &source,
                            query_for_task.as_ref(),
                            replacement_for_task.as_ref(),
                            options,
                            &matches,
                            &metadata,
                        )
                    })
                    .await;
                let _ = this.update(cx, |editor, cx| {
                    let Some(state) = editor.find_panel.as_mut() else {
                        return;
                    };
                    state.replace_task = None;
                    let current_revision = editor.source_document.revision();
                    if !find_replace_result_is_current(
                        revision,
                        generation,
                        current_revision,
                        state.generation,
                    ) {
                        if state.generation == generation && current_revision != revision {
                            state.error = Some("文档已发生变化，请重新搜索".to_owned());
                            cx.notify();
                        }
                        return;
                    }
                    let plan = match result {
                        Ok(plan) => plan,
                        Err(error) => {
                            state.error = Some(error);
                            cx.notify();
                            return;
                        }
                    };
                    if editor.apply_find_edits(plan.edits, plan.selection, cx) {
                        editor.schedule_find(cx);
                    }
                });
            }));
        }
        // 后台计划未完成时仍把键盘导航留在查询框，避免等待期间焦点跳走。
        if let Some(state) = self.find_panel.as_ref() {
            state.query.read(cx).focus_handle.focus(window);
        }
        cx.notify();
    }
}
