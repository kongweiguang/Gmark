// @author kongweiguang

use super::*;

impl Editor {
    /// 替换与常驻批量行命令共用 Rope 事务和投影恢复，失败时不产生部分正文或撤销记录。
    pub(in crate::editor) fn apply_find_edits(
        &mut self,
        edits: Vec<TextEdit>,
        selection: Range<usize>,
        cx: &mut Context<Self>,
    ) -> bool {
        if edits.is_empty()
            || !self.document_surface_is_editable()
            || self.has_active_ime_composition(cx)
        {
            return false;
        }
        self.finalize_pending_undo_capture(cx);
        self.prepare_undo_capture(UndoCaptureKind::NonCoalescible, cx);
        let revision = self.source_document.revision();
        let updated = match self
            .source_document
            .apply_transaction(Transaction::new(revision, edits))
        {
            Ok(updated) => updated,
            Err(error) => {
                self.pending_undo_capture = None;
                self.pending_virtual_undo_selection = None;
                eprintln!("文档替换事务提交失败: {error}");
                return false;
            }
        };
        let source = updated.text();
        self.projection_cache_task = None;
        self.projection_cache_scheduled_revision = None;
        if self.virtual_surface.is_some() && self.view_mode == super::ViewMode::Rendered {
            let prepared = Arc::new(if let Some(previous) = self.projection_cache.as_deref() {
                PreparedSplitProjection::from_snapshot_incremental_regions_only(updated, previous)
            } else {
                PreparedSplitProjection::from_snapshot_adaptive(
                    updated,
                    Self::VIRTUAL_SURFACE_REGION_THRESHOLD,
                )
            });
            self.active_entity_id = None;
            self.pending_focus = None;
            self.install_virtual_surface_projection(Arc::clone(&prepared), cx);
            self.rebuild_runtime_context_from_markdown(&prepared.source, cx);
            self.projection_cache = Some(prepared);
        } else {
            match self.view_mode {
                super::ViewMode::Rendered => {
                    self.rebuild_primary_projection_from_source_reusing(cx)
                }
                super::ViewMode::Source | super::ViewMode::Split => {
                    let block = Self::new_block(cx, BlockRecord::paragraph(source.clone()));
                    block.update(cx, |block, _cx| block.set_source_document_mode());
                    self.document.replace_roots(vec![block], cx);
                    self.table_cells.clear();
                    if self.view_mode == super::ViewMode::Split {
                        self.schedule_split_preview_projection(cx);
                    }
                }
                super::ViewMode::Preview => return false,
            }
        }
        self.pending_dirty_source = Some(source);
        self.render_row_cache = None;
        self.status_bar.invalidate_word_count();
        self.document_dirty = true;
        self.pending_window_edited = true;
        self.pending_window_title_refresh = true;
        self.apply_selection_snapshot_in_current_mode(
            &UndoSelectionSnapshot::from_range(selection, false),
            cx,
        );
        self.pending_focus = None;
        self.finalize_pending_undo_capture(cx);
        self.schedule_recovery_journal(cx);
        self.schedule_auto_save(cx);
        self.schedule_active_block_spellcheck(cx);
        self.pending_scroll_active_block_into_view = true;
        self.pending_scroll_recheck_after_layout = true;
        cx.notify();
        true
    }
}
