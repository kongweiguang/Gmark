// @author kongweiguang

//! Native table runtime installation and table-editing operations.

use super::*;

#[path = "table_edit_parts/write_boundary.rs"]
mod write_boundary;

impl Editor {
    pub(crate) fn new_table_block(cx: &mut Context<Self>, table: TableData) -> Entity<Block> {
        Self::new_block(cx, BlockRecord::table(table))
    }

    pub(super) fn install_table_runtime_for_block(
        &mut self,
        table_block: &Entity<Block>,
        table: &TableData,
        cx: &mut Context<Self>,
    ) {
        let header = table
            .header
            .iter()
            .cloned()
            .enumerate()
            .map(|(column, title)| {
                let alignment = table
                    .alignments
                    .get(column)
                    .copied()
                    .unwrap_or(TableColumnAlignment::Default);
                let position = TableCellPosition { row: 0, column };
                let cell = Self::new_table_cell_block(cx, title, position, alignment);
                self.table_cells.insert(
                    cell.entity_id(),
                    TableCellBinding {
                        table_block: table_block.clone(),
                        cell: cell.clone(),
                        position,
                    },
                );
                cell
            })
            .collect::<Vec<_>>();

        let rows = table
            .rows
            .iter()
            .cloned()
            .enumerate()
            .map(|(body_row_index, row)| {
                row.into_iter()
                    .enumerate()
                    .map(|(column, title)| {
                        let alignment = table
                            .alignments
                            .get(column)
                            .copied()
                            .unwrap_or(TableColumnAlignment::Default);
                        let position = TableCellPosition {
                            row: body_row_index + 1,
                            column,
                        };
                        let cell = Self::new_table_cell_block(cx, title, position, alignment);
                        self.table_cells.insert(
                            cell.entity_id(),
                            TableCellBinding {
                                table_block: table_block.clone(),
                                cell: cell.clone(),
                                position,
                            },
                        );
                        cell
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();

        table_block.update(cx, {
            let runtime = TableRuntime { header, rows };
            move |block, _cx| block.set_table_runtime(runtime.clone())
        });
    }

    pub(super) fn rebuild_table_runtimes(&mut self, cx: &mut Context<Self>) {
        self.rebuild_table_runtimes_inner(cx);
        self.rebuild_image_runtimes(cx);
        self.sync_table_axis_visuals(cx);
        self.sync_table_cell_rectangle_highlights(cx);
    }

    pub(super) fn rebuild_virtual_table_runtimes(&mut self, cx: &mut Context<Self>) {
        self.rebuild_table_runtimes_inner(cx);
        self.rebuild_footnote_registry(cx);
        self.sync_mounted_runtime_contexts(cx);
        self.sync_table_axis_visuals(cx);
        self.sync_table_cell_rectangle_highlights(cx);
    }

    fn rebuild_table_runtimes_inner(&mut self, cx: &mut Context<Self>) {
        self.table_cells.clear();
        self.table_axis_preview = None;
        let visible = self.document.visible_blocks().to_vec();
        for block in &visible {
            block
                .entity
                .update(cx, |block, _cx| block.clear_table_runtime());
        }
        for visible in visible {
            let Some(table) = visible.entity.read(cx).record.table.clone() else {
                continue;
            };
            if visible.entity.read(cx).kind() == BlockKind::Table {
                self.install_table_runtime_for_block(&visible.entity, &table, cx);
            }
        }
    }

    /// Prevents mounted preview cells from synchronizing their runtime state into the source table.
    pub(super) fn sync_table_record_from_runtime(
        &mut self,
        table_block: &Entity<Block>,
        cx: &mut Context<Self>,
    ) {
        if !self.table_block_is_editable(table_block, cx) {
            return;
        }
        let Some(runtime) = table_block.read(cx).table_runtime.clone() else {
            return;
        };
        let alignments = table_block
            .read(cx)
            .record
            .table
            .as_ref()
            .map(|table| table.alignments.clone())
            .unwrap_or_default();
        let header = runtime
            .header
            .iter()
            .map(|cell| cell.read(cx).record.title.clone())
            .collect::<Vec<_>>();
        let rows = runtime
            .rows
            .iter()
            .map(|row| {
                row.iter()
                    .map(|cell| cell.read(cx).record.title.clone())
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        table_block.update(cx, move |block, _cx| {
            block.record.table = Some(TableData {
                header,
                rows,
                alignments,
            });
        });
    }

    /// Rejects direct appends before syncing cells or opening an undo transaction.
    pub(super) fn append_table_column(
        &mut self,
        table_block: &Entity<Block>,
        cx: &mut Context<Self>,
    ) {
        if !self.table_block_is_editable(table_block, cx) {
            return;
        }
        self.sync_table_record_from_runtime(table_block, cx);

        let Some(mut table) = table_block.read(cx).record.table.clone() else {
            return;
        };
        let started_local_capture = if self.pending_undo_capture.is_none() {
            self.prepare_undo_capture(UndoCaptureKind::NonCoalescible, cx);
            true
        } else {
            false
        };
        let alignment = table
            .alignments
            .last()
            .copied()
            .unwrap_or(TableColumnAlignment::Default);
        table.append_column(alignment);

        table_block.update(cx, move |block, _cx| {
            block.record.table = Some(table.clone());
        });
        self.rebuild_table_runtimes(cx);
        if let Some(cell) = table_block
            .read(cx)
            .table_runtime
            .as_ref()
            .and_then(|runtime| runtime.header.last())
        {
            self.focus_block(cell.entity_id());
        }
        self.mark_dirty(cx);
        self.request_active_block_scroll_into_view(cx);
        if started_local_capture {
            self.finalize_pending_undo_capture(cx);
        }
        cx.notify();
    }

    /// Rejects direct appends before syncing cells or opening an undo transaction.
    pub(super) fn append_table_row(&mut self, table_block: &Entity<Block>, cx: &mut Context<Self>) {
        if !self.table_block_is_editable(table_block, cx) {
            return;
        }
        self.sync_table_record_from_runtime(table_block, cx);

        let Some(mut table) = table_block.read(cx).record.table.clone() else {
            return;
        };
        let started_local_capture = if self.pending_undo_capture.is_none() {
            self.prepare_undo_capture(UndoCaptureKind::NonCoalescible, cx);
            true
        } else {
            false
        };
        table.append_row();

        table_block.update(cx, move |block, _cx| {
            block.record.table = Some(table.clone());
        });
        self.rebuild_table_runtimes(cx);
        if let Some(cell) = table_block
            .read(cx)
            .table_runtime
            .as_ref()
            .and_then(|runtime| runtime.rows.last())
            .and_then(|row| row.first())
        {
            self.focus_block(cell.entity_id());
        }
        self.mark_dirty(cx);
        self.request_active_block_scroll_into_view(cx);
        if started_local_capture {
            self.finalize_pending_undo_capture(cx);
        }
        cx.notify();
    }

    pub(super) fn preview_table_axis(
        &mut self,
        table_block_id: EntityId,
        kind: TableAxisKind,
        index: usize,
        hovered: bool,
        cx: &mut Context<Self>,
    ) {
        let marker = TableAxisSelection {
            table_block_id,
            kind,
            index,
        };
        if hovered {
            self.set_table_axis_preview(Some(marker), cx);
        } else if self.table_axis_preview == Some(marker) {
            // Only clear on a leave that still owns the preview. Adjacent
            // handles share one preview slot, and a leave can arrive after
            // the next handle's enter; clearing unconditionally would erase
            // the highlight the pointer just moved onto.
            self.set_table_axis_preview(None, cx);
        }
    }

    pub(super) fn select_table_axis(
        &mut self,
        table_block_id: EntityId,
        kind: TableAxisKind,
        index: usize,
        cx: &mut Context<Self>,
    ) {
        let selection = TableAxisSelection {
            table_block_id,
            kind,
            index,
        };
        self.set_table_axis_preview(Some(selection), cx);
        self.set_table_axis_selection(Some(selection), cx);
    }

    pub(super) fn open_table_axis_menu(
        &mut self,
        table_block_id: EntityId,
        kind: TableAxisKind,
        index: usize,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        self.select_table_axis(table_block_id, kind, index, cx);
        if let Some(selection) = self.table_axis_selection {
            self.open_table_axis_context_menu(position, selection, cx);
        }
    }

    /// Keeps column alignment changes behind the table's owning surface write boundary.
    pub(super) fn set_table_column_alignment(
        &mut self,
        table_block: &Entity<Block>,
        column: usize,
        alignment: TableColumnAlignment,
        cx: &mut Context<Self>,
    ) {
        if !self.table_block_is_editable(table_block, cx) {
            return;
        }
        self.sync_table_record_from_runtime(table_block, cx);
        let Some(mut table) = table_block.read(cx).record.table.clone() else {
            return;
        };
        let started_local_capture = if self.pending_undo_capture.is_none() {
            self.prepare_undo_capture(UndoCaptureKind::NonCoalescible, cx);
            true
        } else {
            false
        };
        table.set_column_alignment(column, alignment);
        table_block.update(cx, move |block, _cx| {
            block.record.table = Some(table.clone());
        });
        self.rebuild_table_runtimes(cx);
        let selection = TableAxisSelection {
            table_block_id: table_block.entity_id(),
            kind: TableAxisKind::Column,
            index: column,
        };
        self.set_table_axis_selection(Some(selection), cx);
        self.focus_table_cell_position(table_block, TableCellPosition { row: 0, column }, cx);
        self.mark_dirty(cx);
        self.request_active_block_scroll_into_view(cx);
        if started_local_capture {
            self.finalize_pending_undo_capture(cx);
        }
        cx.notify();
    }

    pub(super) fn insert_table_row(
        &mut self,
        table_block: &Entity<Block>,
        visual_index: usize,
        cx: &mut Context<Self>,
    ) {
        self.sync_table_record_from_runtime(table_block, cx);
        let Some(mut table) = table_block.read(cx).record.table.clone() else {
            return;
        };
        if !table.insert_empty_visual_row(visual_index) {
            return;
        }
        self.commit_table_structure_change(
            table_block,
            table,
            TableAxisSelection {
                table_block_id: table_block.entity_id(),
                kind: TableAxisKind::Row,
                index: visual_index,
            },
            TableCellPosition {
                row: visual_index,
                column: 0,
            },
            cx,
        );
    }

    pub(super) fn duplicate_table_row(
        &mut self,
        table_block: &Entity<Block>,
        visual_index: usize,
        cx: &mut Context<Self>,
    ) {
        self.sync_table_record_from_runtime(table_block, cx);
        let Some(mut table) = table_block.read(cx).record.table.clone() else {
            return;
        };
        if !table.duplicate_visual_row(visual_index) {
            return;
        }
        let inserted = visual_index + 1;
        self.commit_table_structure_change(
            table_block,
            table,
            TableAxisSelection {
                table_block_id: table_block.entity_id(),
                kind: TableAxisKind::Row,
                index: inserted,
            },
            TableCellPosition {
                row: inserted,
                column: 0,
            },
            cx,
        );
    }

    pub(super) fn insert_table_column(
        &mut self,
        table_block: &Entity<Block>,
        column: usize,
        cx: &mut Context<Self>,
    ) {
        self.sync_table_record_from_runtime(table_block, cx);
        let Some(mut table) = table_block.read(cx).record.table.clone() else {
            return;
        };
        let alignment = table
            .alignments
            .get(column.saturating_sub(1))
            .copied()
            .or_else(|| table.alignments.get(column).copied())
            .unwrap_or(TableColumnAlignment::Default);
        if !table.insert_empty_column(column, alignment) {
            return;
        }
        self.commit_table_structure_change(
            table_block,
            table,
            TableAxisSelection {
                table_block_id: table_block.entity_id(),
                kind: TableAxisKind::Column,
                index: column,
            },
            TableCellPosition { row: 0, column },
            cx,
        );
    }

    pub(super) fn duplicate_table_column(
        &mut self,
        table_block: &Entity<Block>,
        column: usize,
        cx: &mut Context<Self>,
    ) {
        self.sync_table_record_from_runtime(table_block, cx);
        let Some(mut table) = table_block.read(cx).record.table.clone() else {
            return;
        };
        if !table.duplicate_column(column) {
            return;
        }
        let inserted = column + 1;
        self.commit_table_structure_change(
            table_block,
            table,
            TableAxisSelection {
                table_block_id: table_block.entity_id(),
                kind: TableAxisKind::Column,
                index: inserted,
            },
            TableCellPosition {
                row: 0,
                column: inserted,
            },
            cx,
        );
    }

    /// Prevents row reordering from changing source state through a read-only projection.
    pub(super) fn move_table_row(
        &mut self,
        table_block: &Entity<Block>,
        visual_row: usize,
        delta: i32,
        cx: &mut Context<Self>,
    ) {
        if !self.table_block_is_editable(table_block, cx) {
            return;
        }
        self.sync_table_record_from_runtime(table_block, cx);
        let Some(mut table) = table_block.read(cx).record.table.clone() else {
            return;
        };
        let next_row = if delta < 0 {
            visual_row.checked_sub(delta.unsigned_abs() as usize)
        } else {
            visual_row.checked_add(delta as usize)
        };
        let Some(next_row) = next_row else {
            return;
        };
        // Visual rows are the header (0) plus every body row, so the last valid
        // index is `rows.len()`.
        if next_row > table.rows.len() {
            return;
        }
        let started_local_capture = if self.pending_undo_capture.is_none() {
            self.prepare_undo_capture(UndoCaptureKind::NonCoalescible, cx);
            true
        } else {
            false
        };
        table.swap_visual_rows(visual_row, next_row);
        table_block.update(cx, move |block, _cx| {
            block.record.table = Some(table.clone());
        });
        self.rebuild_table_runtimes(cx);
        let selection = TableAxisSelection {
            table_block_id: table_block.entity_id(),
            kind: TableAxisKind::Row,
            index: next_row,
        };
        self.set_table_axis_selection(Some(selection), cx);
        self.focus_table_cell_position(
            table_block,
            TableCellPosition {
                row: next_row,
                column: 0,
            },
            cx,
        );
        self.mark_dirty(cx);
        self.request_active_block_scroll_into_view(cx);
        if started_local_capture {
            self.finalize_pending_undo_capture(cx);
        }
        cx.notify();
    }

    /// Prevents column reordering from changing source state through a read-only projection.
    pub(super) fn move_table_column(
        &mut self,
        table_block: &Entity<Block>,
        column: usize,
        delta: i32,
        cx: &mut Context<Self>,
    ) {
        if !self.table_block_is_editable(table_block, cx) {
            return;
        }
        self.sync_table_record_from_runtime(table_block, cx);
        let Some(mut table) = table_block.read(cx).record.table.clone() else {
            return;
        };
        let next_column = if delta < 0 {
            column.checked_sub(delta.unsigned_abs() as usize)
        } else {
            column.checked_add(delta as usize)
        };
        let Some(next_column) = next_column else {
            return;
        };
        if next_column >= table.column_count() {
            return;
        }
        let started_local_capture = if self.pending_undo_capture.is_none() {
            self.prepare_undo_capture(UndoCaptureKind::NonCoalescible, cx);
            true
        } else {
            false
        };
        table.swap_columns(column, next_column);
        table_block.update(cx, move |block, _cx| {
            block.record.table = Some(table.clone());
        });
        self.rebuild_table_runtimes(cx);
        let selection = TableAxisSelection {
            table_block_id: table_block.entity_id(),
            kind: TableAxisKind::Column,
            index: next_column,
        };
        self.set_table_axis_selection(Some(selection), cx);
        self.focus_table_cell_position(
            table_block,
            TableCellPosition {
                row: 0,
                column: next_column,
            },
            cx,
        );
        self.mark_dirty(cx);
        self.request_active_block_scroll_into_view(cx);
        if started_local_capture {
            self.finalize_pending_undo_capture(cx);
        }
        cx.notify();
    }

    /// Rejects row deletion before runtime synchronization or undo capture.
    pub(super) fn delete_table_row(
        &mut self,
        table_block: &Entity<Block>,
        row_index: usize,
        cx: &mut Context<Self>,
    ) {
        if !self.table_block_is_editable(table_block, cx) {
            return;
        }
        self.sync_table_record_from_runtime(table_block, cx);
        let Some(mut table) = table_block.read(cx).record.table.clone() else {
            return;
        };
        if row_index >= table.rows.len() {
            return;
        }
        let started_local_capture = if self.pending_undo_capture.is_none() {
            self.prepare_undo_capture(UndoCaptureKind::NonCoalescible, cx);
            true
        } else {
            false
        };
        table.remove_body_row(row_index);
        let remaining_body_rows = table.rows.len();
        table_block.update(cx, move |block, _cx| {
            block.record.table = Some(table.clone());
        });
        self.rebuild_table_runtimes(cx);
        // Row selections are addressed by visual index, where the first body row
        // is `1` (the header is `0`). With no body rows left, fall back to the
        // header so focus lands on a cell that still exists.
        let focus_visual_row = if remaining_body_rows == 0 {
            0
        } else {
            row_index.min(remaining_body_rows - 1) + 1
        };
        if remaining_body_rows == 0 {
            self.clear_table_axis_selection(cx);
        } else {
            self.set_table_axis_selection(
                Some(TableAxisSelection {
                    table_block_id: table_block.entity_id(),
                    kind: TableAxisKind::Row,
                    index: focus_visual_row,
                }),
                cx,
            );
        }
        self.focus_table_cell_position(
            table_block,
            TableCellPosition {
                row: focus_visual_row,
                column: 0,
            },
            cx,
        );
        self.mark_dirty(cx);
        self.request_active_block_scroll_into_view(cx);
        if started_local_capture {
            self.finalize_pending_undo_capture(cx);
        }
        cx.notify();
    }

    /// Rejects header deletion before runtime synchronization or undo capture.
    pub(super) fn delete_table_header_row(
        &mut self,
        table_block: &Entity<Block>,
        cx: &mut Context<Self>,
    ) {
        if !self.table_block_is_editable(table_block, cx) {
            return;
        }
        self.sync_table_record_from_runtime(table_block, cx);
        let Some(mut table) = table_block.read(cx).record.table.clone() else {
            return;
        };
        // The first body row is promoted into the header, so there must be at
        // least one body row to delete the header.
        if table.rows.is_empty() {
            return;
        }
        let started_local_capture = if self.pending_undo_capture.is_none() {
            self.prepare_undo_capture(UndoCaptureKind::NonCoalescible, cx);
            true
        } else {
            false
        };
        table.remove_header_row();
        table_block.update(cx, move |block, _cx| {
            block.record.table = Some(table.clone());
        });
        self.rebuild_table_runtimes(cx);
        self.clear_table_axis_selection(cx);
        self.focus_table_cell_position(table_block, TableCellPosition { row: 0, column: 0 }, cx);
        self.mark_dirty(cx);
        self.request_active_block_scroll_into_view(cx);
        if started_local_capture {
            self.finalize_pending_undo_capture(cx);
        }
        cx.notify();
    }

    /// Rejects column deletion before runtime synchronization or undo capture.
    pub(super) fn delete_table_column(
        &mut self,
        table_block: &Entity<Block>,
        column: usize,
        cx: &mut Context<Self>,
    ) {
        if !self.table_block_is_editable(table_block, cx) {
            return;
        }
        self.sync_table_record_from_runtime(table_block, cx);
        let Some(mut table) = table_block.read(cx).record.table.clone() else {
            return;
        };
        if table.column_count() <= 1 || column >= table.column_count() {
            return;
        }
        let started_local_capture = if self.pending_undo_capture.is_none() {
            self.prepare_undo_capture(UndoCaptureKind::NonCoalescible, cx);
            true
        } else {
            false
        };
        table.remove_column(column);
        let focus_column = column.min(table.column_count().saturating_sub(1));
        table_block.update(cx, move |block, _cx| {
            block.record.table = Some(table.clone());
        });
        self.rebuild_table_runtimes(cx);
        let selection = TableAxisSelection {
            table_block_id: table_block.entity_id(),
            kind: TableAxisKind::Column,
            index: focus_column,
        };
        self.set_table_axis_selection(Some(selection), cx);
        self.focus_table_cell_position(
            table_block,
            TableCellPosition {
                row: 0,
                column: focus_column,
            },
            cx,
        );
        self.mark_dirty(cx);
        self.request_active_block_scroll_into_view(cx);
        if started_local_capture {
            self.finalize_pending_undo_capture(cx);
        }
        cx.notify();
    }

    /// Removes the table block entirely, leaving an empty paragraph in its place
    /// so the caret has somewhere to land. Used when deleting the last remaining
    /// row or column, which empties the table.
    /// The owning surface is checked first so Preview cannot replace source structure.
    pub(super) fn remove_table_block(
        &mut self,
        table_block: &Entity<Block>,
        cx: &mut Context<Self>,
    ) {
        if !self.table_block_is_editable(table_block, cx) {
            return;
        }
        let Some(location) = self.document.find_block_location(table_block.entity_id()) else {
            return;
        };
        let started_local_capture = if self.pending_undo_capture.is_none() {
            self.prepare_undo_capture(UndoCaptureKind::NonCoalescible, cx);
            true
        } else {
            false
        };
        // Insert the replacement paragraph after the table first, then remove the
        // table, so the document is never momentarily empty.
        let paragraph = Self::new_block(cx, BlockRecord::paragraph(String::new()));
        self.document.insert_blocks_at(
            location.parent.clone(),
            location.index + 1,
            vec![paragraph.clone()],
            cx,
        );
        let table_id = table_block.entity_id();
        self.document.with_structure_mutation(cx, |document, cx| {
            let _ = document.remove_block_by_id_raw(table_id, cx);
        });
        self.rebuild_table_runtimes(cx);
        self.clear_table_axis_selection(cx);
        self.focus_block(paragraph.entity_id());
        self.mark_dirty(cx);
        self.request_active_block_scroll_into_view(cx);
        if started_local_capture {
            self.finalize_pending_undo_capture(cx);
        }
        cx.notify();
    }
}
#[path = "block/table/axis_state.rs"]
mod axis_state;
