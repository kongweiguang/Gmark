// @author kongweiguang

//! Surface permissions and atomic structural commits for Markdown tables.

use super::*;

impl Editor {
    /// Ties write permission to this table's pane, independent of whichever pane last gained focus.
    pub(in crate::editor) fn table_block_is_editable(
        &self,
        table_block: &Entity<Block>,
        cx: &App,
    ) -> bool {
        self.selection_surface_for_block_id(table_block.entity_id())
            .is_some_and(|surface| self.document_surface_is_editable_for(surface))
            && !table_block.read(cx).is_read_only()
    }

    /// Rejects append requests before creating undo state for a read-only projection.
    pub(in crate::editor) fn append_table_axis_from_event(
        &mut self,
        table_block: &Entity<Block>,
        append_column: bool,
        cx: &mut Context<Self>,
    ) {
        let is_table = table_block.read(cx).kind() == BlockKind::Table
            && table_block.read(cx).record.table.is_some();
        if !is_table || !self.table_block_is_editable(table_block, cx) {
            return;
        }
        self.prepare_undo_capture(UndoCaptureKind::NonCoalescible, cx);
        if append_column {
            self.append_table_column(table_block, cx);
        } else {
            self.append_table_row(table_block, cx);
        }
        self.finalize_pending_undo_capture(cx);
    }

    /// Provides the final write check for staged row and column structure changes.
    pub(super) fn commit_table_structure_change(
        &mut self,
        table_block: &Entity<Block>,
        table: TableData,
        selection: TableAxisSelection,
        focus: TableCellPosition,
        cx: &mut Context<Self>,
    ) {
        if !self.table_block_is_editable(table_block, cx) {
            return;
        }
        let started_local_capture = if self.pending_undo_capture.is_none() {
            self.prepare_undo_capture(UndoCaptureKind::NonCoalescible, cx);
            true
        } else {
            false
        };
        table_block.update(cx, move |block, _cx| block.record.table = Some(table));
        self.rebuild_table_runtimes(cx);
        self.set_table_axis_selection(Some(selection), cx);
        self.focus_table_cell_position(table_block, focus, cx);
        self.mark_dirty(cx);
        self.request_active_block_scroll_into_view(cx);
        if started_local_capture {
            self.finalize_pending_undo_capture(cx);
        }
        cx.notify();
    }
}
