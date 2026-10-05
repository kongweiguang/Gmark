// @author kongweiguang

//! Undo and redo coordination.

use super::*;

impl DocumentHost {
    /// 先等待候选终态再移动历史；Source 同步重锚输入行，避免续键或关闭提示记录外层容器焦点。
    pub(crate) fn on_undo(&mut self, _: &Undo, window: &mut Window, cx: &mut Context<Self>) {
        if self.defer_source_action_for_ime(
            super::source_ime::DeferredSourceAction::Block(BlockHostAction::Undo),
            window,
            cx,
        ) {
            return;
        }
        if self.reloading {
            return;
        }
        let changed = self
            .document
            .as_ref()
            .is_some_and(|document| document.undo_changed().unwrap_or(false));
        if changed {
            let restored_selection = self.document.as_ref().map(SharedDocument::source_selection);
            if let Some(document) = self.document.clone() {
                self.enqueue_recovery_action(
                    &document,
                    RecoveryAction::Undo,
                    restored_selection,
                    DocumentViewId::source(),
                    cx,
                );
            }
            self.active_edit = None;
            if let Some(selection) = restored_selection {
                self.set_source_selection(selection, cx);
            }
            self.focus_handle.focus(window);
            self.invalidate_source_rows();
            if self.view_mode == DocumentHostViewMode::Source {
                self.restore_source_navigation_input(window, cx);
            }
            let dirty = self
                .document
                .as_ref()
                .is_some_and(|document| !document.is_pristine());
            self.schedule_search(cx);
            let preserve_live_table = self.is_delimited_document()
                && matches!(
                    self.view_mode,
                    DocumentHostViewMode::Live | DocumentHostViewMode::Split
                )
                && self.structured_index.is_some();
            if preserve_live_table {
                self.structured_pending = None;
                self.structured_cell_overrides.clear();
                self.structured_cell_source_edits.clear();
                self.schedule_delimited_snapshot_rebuild(cx);
                self.clear_structure_error();
            } else if dirty {
                self.structured_index = None;
                self.invalidate_structured_runtime();
            } else {
                self.rebuild_clean_structured_index(cx);
            }
            self.schedule_json_graph_projection(cx);
            if dirty && !preserve_live_table {
                self.schedule_delimited_snapshot_rebuild(cx);
            }
            cx.emit(DocumentHostEvent::StateChanged);
            cx.notify();
        }
    }

    /// 重做保留原保存快照并恢复 Source 的真实输入焦点；新 revision 仍由 Controller 独立标记 dirty。
    pub(crate) fn on_redo(&mut self, _: &Redo, window: &mut Window, cx: &mut Context<Self>) {
        if self.defer_source_action_for_ime(
            super::source_ime::DeferredSourceAction::Block(BlockHostAction::Redo),
            window,
            cx,
        ) {
            return;
        }
        if self.reloading {
            return;
        }
        let changed = self
            .document
            .as_ref()
            .is_some_and(|document| document.redo_changed().unwrap_or(false));
        if changed {
            let restored_selection = self.document.as_ref().map(SharedDocument::source_selection);
            if let Some(document) = self.document.clone() {
                self.enqueue_recovery_action(
                    &document,
                    RecoveryAction::Redo,
                    restored_selection,
                    DocumentViewId::source(),
                    cx,
                );
            }
            self.active_edit = None;
            if let Some(selection) = restored_selection {
                self.set_source_selection(selection, cx);
            }
            self.focus_handle.focus(window);
            self.invalidate_source_rows();
            if self.view_mode == DocumentHostViewMode::Source {
                self.restore_source_navigation_input(window, cx);
            }
            let preserve_live_table = self.is_delimited_document()
                && matches!(
                    self.view_mode,
                    DocumentHostViewMode::Live | DocumentHostViewMode::Split
                )
                && self.structured_index.is_some();
            if preserve_live_table {
                self.structured_pending = None;
                self.structured_cell_overrides.clear();
                self.structured_cell_source_edits.clear();
            } else {
                self.structured_index = None;
                self.invalidate_structured_runtime();
            }
            self.clear_structure_error();
            self.schedule_search(cx);
            self.schedule_json_graph_projection(cx);
            self.schedule_delimited_snapshot_rebuild(cx);
            if preserve_live_table {
                self.clear_structure_error();
            }
            cx.emit(DocumentHostEvent::StateChanged);
            cx.notify();
        }
    }
}
