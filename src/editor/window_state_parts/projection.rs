// @author kongweiguang

//! Projection construction and Split-view transitions for the editor window.

use super::*;

impl Editor {
    // Reason: Keep source projection construction beside its transition callers so the window
    // state module remains focused on commands and user-facing mode coordination.
    pub(in crate::editor) fn source_view_document(&self, cx: &mut Context<Self>) -> DocumentTree {
        let block = Self::new_block(cx, BlockRecord::paragraph(self.source_document.text()));
        let language = if self.is_svg_document() {
            Some("html")
        } else {
            self.document_kind.source_syntax_language()
        };
        block.update(cx, move |block, _cx| {
            block.set_source_document_mode_with_language(language)
        });
        let mut document = DocumentTree::new(vec![block]);
        document.rebuild_metadata_and_snapshot(cx);
        document
    }

    /// 切换模式和显式导航需要当前 revision 的完整语义；共用增量投影避免读取尚未发布的后台缓存。
    pub(in crate::editor) fn prepare_current_projection(&mut self) -> Arc<PreparedSplitProjection> {
        self.projection_cache_task = None;
        self.projection_cache_scheduled_revision = None;
        let snapshot = self.source_document.snapshot();
        if let Some(cached) = self.projection_cache.as_ref()
            && cached.revision == snapshot.revision()
        {
            return Arc::clone(cached);
        }
        let prepared = Arc::new(if let Some(previous) = self.projection_cache.as_deref() {
            if self.virtual_surface.is_some() {
                PreparedSplitProjection::from_snapshot_incremental_regions_only(snapshot, previous)
            } else {
                PreparedSplitProjection::from_snapshot_incremental(snapshot, previous)
            }
        } else {
            PreparedSplitProjection::from_snapshot_adaptive(
                snapshot,
                Self::VIRTUAL_SURFACE_REGION_THRESHOLD,
            )
        });
        self.projection_cache = Some(Arc::clone(&prepared));
        prepared
    }

    // Reason: Expose the existing rebuild entry points at the editor boundary while keeping
    // the implementation details of incremental projection assembly in this part module.
    pub(in crate::editor) fn rebuild_primary_projection_from_source(
        &mut self,
        cx: &mut Context<Self>,
    ) {
        self.rebuild_primary_projection_from_source_internal(false, cx);
    }

    pub(in crate::editor) fn rebuild_primary_projection_from_source_reusing(
        &mut self,
        cx: &mut Context<Self>,
    ) {
        self.rebuild_primary_projection_from_source_internal(true, cx);
    }

    fn rebuild_primary_projection_from_source_internal(
        &mut self,
        reuse_entities: bool,
        cx: &mut Context<Self>,
    ) {
        let prepared = self.prepare_current_projection();
        if Self::should_virtualize_projection(&prepared) {
            let mut surface = VirtualSurfaceState::new(Arc::clone(&prepared));
            let viewport_height = f32::from(self.scroll_handle.bounds().size.height.max(px(720.0)));
            let scroll_y = (-f32::from(self.scroll_handle.offset().y)).max(0.0);
            let target = surface.desired_window(scroll_y, viewport_height, 800.0, None);
            surface.reconcile_mounts(target, cx);
            let mut roots = surface.viewport_roots();
            if roots.is_empty() {
                roots.push(Self::new_block(cx, BlockRecord::paragraph(String::new())));
            }
            self.virtual_surface = Some(surface);
            self.document.replace_roots(roots, cx);
            self.row_stride_cache.clear();
            self.render_row_cache = None;
            self.rebuild_virtual_table_runtimes(cx);
            return;
        }
        self.virtual_surface = None;
        let mut reusable = if reuse_entities {
            self.document
                .visible_blocks()
                .iter()
                .map(|visible| (visible.entity.read(cx).record.id, visible.entity.clone()))
                .collect::<HashMap<_, _>>()
        } else {
            HashMap::new()
        };
        let mut roots = Self::build_blocks_from_projection_reusing(cx, &prepared, &mut reusable);
        if roots.is_empty() {
            roots.push(Self::new_block(cx, BlockRecord::paragraph(String::new())));
        }
        self.document.replace_roots(roots, cx);
        let current_entity_ids = self
            .document
            .visible_blocks()
            .iter()
            .map(|visible| visible.entity.entity_id())
            .collect::<std::collections::HashSet<_>>();
        self.row_stride_cache
            .retain(|entity_id, _| current_entity_ids.contains(entity_id));
        self.rebuild_table_runtimes(cx);
    }

    // Reason: Keep Split-mode setup and teardown together so each transition preserves the
    // existing projection cache and virtual-surface lifecycle exactly as before.
    pub(super) fn enter_split_view(&mut self, cx: &mut Context<Self>) {
        self.projection_cache_task = None;
        self.projection_cache_scheduled_revision = None;
        self.split_projection_task = None;
        self.split_projection_scheduled_revision = None;
        self.virtual_surface = None;
        match self.view_mode {
            ViewMode::Rendered | ViewMode::Preview => {
                let source_document = self.source_view_document(cx);
                self.document = source_document;
                self.table_cells.clear();
                self.split_preview = None;
                // 首个 Split 树必须与 Prepared IR 共用记录 ID，后续 revision 才能复用 Entity。
                self.rebuild_split_preview_projection(cx);
            }
            ViewMode::Source => {
                self.rebuild_split_preview_projection(cx);
            }
            ViewMode::Split => unreachable!(),
        }
    }

    pub(super) fn exit_split_view(&mut self, target: ViewMode, cx: &mut Context<Self>) {
        self.split_resize_session = None;
        self.split_preview_scrollbar_drag = None;
        self.split_preview_scrollbar_hovered = false;
        self.split_preview_scrollbar_fade_task = None;
        self.split_preview_scrollbar_visible_until = Instant::now();
        self.split_projection_task = None;
        self.split_projection_scheduled_revision = None;
        match target {
            ViewMode::Source => {
                self.split_preview = None;
                self.table_cells.clear();
            }
            ViewMode::Rendered | ViewMode::Preview => {
                let projection_is_current = self
                    .split_preview
                    .as_ref()
                    .is_some_and(|state| state.revision == self.source_document.revision());
                if !projection_is_current {
                    self.rebuild_split_preview_projection(cx);
                }
                let should_virtualize = self
                    .projection_cache
                    .as_deref()
                    .is_some_and(Self::should_virtualize_projection);
                if should_virtualize {
                    // Split 右侧当前仍是全量只读树；返回 Live/Preview 时必须恢复
                    // Rope 驱动的虚拟 surface，不能让一次模式切换永久放大全量 Entity。
                    self.split_preview = None;
                    self.table_cells.clear();
                    self.rebuild_primary_projection_from_source(cx);
                    return;
                }
                let state = self
                    .split_preview
                    .take()
                    .expect("Split 模式必须持有右侧预览投影");
                self.document = state.document;
                self.table_cells = state.table_cells;
            }
            ViewMode::Split => unreachable!(),
        }
    }

    /// 根据当前源码重建 Split 右侧投影；解析结果不反向覆盖 SourceDocument。
    pub(in crate::editor) fn rebuild_split_preview_projection(&mut self, cx: &mut Context<Self>) {
        let prepared = self.prepare_current_projection();
        self.install_split_preview_projection(prepared, cx);
    }

    pub(super) fn install_split_preview_projection(
        &mut self,
        prepared: Arc<PreparedSplitProjection>,
        cx: &mut Context<Self>,
    ) {
        debug_assert!(prepared.reused_prefix_regions <= prepared.regions.len());
        self.projection_cache = Some(Arc::clone(&prepared));
        let mut previous_state = self.split_preview.take();
        let scroll_handle = previous_state
            .as_ref()
            .map(|state| state.scroll_handle.clone())
            .unwrap_or_default();
        let scroll_driver = previous_state
            .as_ref()
            .and_then(|state| state.scroll_driver);

        if Self::should_virtualize_projection(&prepared) {
            let viewport_height = f32::from(scroll_handle.bounds().size.height.max(px(720.0)));
            let scroll_y = (-f32::from(scroll_handle.offset().y)).max(0.0);
            let mut surface = previous_state
                .as_mut()
                .and_then(|state| state.virtual_surface.take())
                .unwrap_or_else(|| VirtualSurfaceState::new(Arc::clone(&prepared)));
            if surface.projection_revision() == prepared.revision {
                let target = surface.desired_window(scroll_y, viewport_height, 800.0, None);
                surface.reconcile_mounts(target, cx);
            } else {
                surface.replace_projection(
                    Arc::clone(&prepared),
                    scroll_y,
                    viewport_height,
                    800.0,
                    None,
                    cx,
                );
            }
            let mut roots = surface.viewport_roots();
            if roots.is_empty() {
                roots.push(Self::new_block(cx, BlockRecord::paragraph(String::new())));
            }
            let mut document = DocumentTree::new(roots);
            document.rebuild_metadata_and_snapshot(cx);
            self.split_preview = Some(SplitPreviewState {
                document,
                virtual_surface: Some(surface),
                table_cells: HashMap::new(),
                source_ranges: HashMap::new(),
                scroll_handle,
                scroll_driver,
                row_stride_cache: HashMap::new(),
                previous_visible_ids: Vec::new(),
                previous_render_window: None,
                revision: prepared.revision,
            });
            self.refresh_split_virtual_preview_runtime(cx);
            return;
        }

        let mut reusable_entities = previous_state
            .as_ref()
            .map(|state| {
                state
                    .document
                    .visible_blocks()
                    .iter()
                    .map(|visible| (visible.entity.read(cx).record.id, visible.entity.clone()))
                    .collect::<HashMap<_, _>>()
            })
            .unwrap_or_default();
        let mut roots =
            Self::build_blocks_from_projection_reusing(cx, &prepared, &mut reusable_entities);
        if roots.is_empty() {
            roots.push(Self::new_block(cx, BlockRecord::paragraph(String::new())));
        }
        let mut preview_document = DocumentTree::new(roots);
        preview_document.rebuild_metadata_and_snapshot(cx);

        // 复用现有 runtime 构建器，但事务完成后恢复左侧 Source 文档所有权。
        let source_document = std::mem::replace(&mut self.document, preview_document);
        let source_cells = std::mem::take(&mut self.table_cells);
        self.rebuild_table_runtimes(cx);
        let (_, source_ranges) = self.build_source_target_mappings_with_block_ranges(cx);
        let preview_document = std::mem::replace(&mut self.document, source_document);
        let preview_cells = std::mem::replace(&mut self.table_cells, source_cells);
        Self::set_document_read_only(&preview_document, &preview_cells, true, cx);

        let mut row_stride_cache = previous_state
            .as_ref()
            .map(|state| state.row_stride_cache.clone())
            .unwrap_or_default();
        let current_entity_ids = preview_document
            .visible_blocks()
            .iter()
            .map(|visible| visible.entity.entity_id())
            .collect::<std::collections::HashSet<_>>();
        row_stride_cache.retain(|entity_id, _| current_entity_ids.contains(entity_id));
        let previous_visible_ids = previous_state
            .as_ref()
            .map(|state| state.previous_visible_ids.clone())
            .unwrap_or_default();
        let previous_render_window = previous_state
            .as_ref()
            .and_then(|state| state.previous_render_window);
        self.split_preview = Some(SplitPreviewState {
            document: preview_document,
            virtual_surface: None,
            table_cells: preview_cells,
            source_ranges,
            scroll_handle,
            scroll_driver,
            row_stride_cache,
            previous_visible_ids,
            previous_render_window,
            revision: prepared.revision,
        });
    }
}
