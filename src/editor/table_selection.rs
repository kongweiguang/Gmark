// @author kongweiguang

use std::ops::RangeInclusive;

use gpui::*;

use super::*;

const MAX_TSV_CELLS: usize = 10_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct TableCellRectangle {
    pub(super) table_block_id: EntityId,
    pub(super) anchor: TableCellPosition,
    pub(super) focus: TableCellPosition,
}

impl TableCellRectangle {
    pub(super) fn rows(self) -> RangeInclusive<usize> {
        self.anchor.row.min(self.focus.row)..=self.anchor.row.max(self.focus.row)
    }

    pub(super) fn columns(self) -> RangeInclusive<usize> {
        self.anchor.column.min(self.focus.column)..=self.anchor.column.max(self.focus.column)
    }

    fn contains(self, position: TableCellPosition) -> bool {
        self.rows().contains(&position.row) && self.columns().contains(&position.column)
    }
}

pub(super) fn parse_tsv_matrix(text: &str) -> Option<Vec<Vec<String>>> {
    if !text.contains(['\t', '\n', '\r']) {
        return None;
    }
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let normalized = normalized.strip_suffix('\n').unwrap_or(&normalized);
    let rows = normalized
        .split('\n')
        .map(|row| row.split('\t').map(str::to_owned).collect::<Vec<_>>())
        .collect::<Vec<_>>();
    let width = rows.iter().map(Vec::len).max().unwrap_or(0);
    if rows.is_empty()
        || width == 0
        || rows.iter().any(|row| row.len() != width)
        || rows.len().saturating_mul(width) > MAX_TSV_CELLS
    {
        return None;
    }
    Some(rows)
}

impl Editor {
    /// Highlights only the table cells owned by the active surface, since Split keeps two runtimes mounted.
    pub(super) fn sync_table_cell_rectangle_highlights(&mut self, cx: &mut Context<Self>) {
        self.sync_table_cell_rectangle_highlights_for(self.active_selection_surface, cx);
    }

    /// Keeps focus-local table selection state independent across the Split projections.
    pub(super) fn table_cell_rectangle_for_surface(
        &self,
        surface: super::selection_surface::SelectionSurface,
    ) -> Option<TableCellRectangle> {
        match surface {
            super::selection_surface::SelectionSurface::Main => self.table_cell_rectangle,
            super::selection_surface::SelectionSurface::SplitPreview => {
                self.split_preview_table_cell_rectangle
            }
        }
    }

    /// Stores rectangle changes without allowing a click in one pane to erase the other pane's selection.
    pub(super) fn set_table_cell_rectangle_for_surface(
        &mut self,
        surface: super::selection_surface::SelectionSurface,
        selection: Option<TableCellRectangle>,
    ) {
        match surface {
            super::selection_surface::SelectionSurface::Main => {
                self.table_cell_rectangle = selection;
            }
            super::selection_surface::SelectionSurface::SplitPreview => {
                self.split_preview_table_cell_rectangle = selection;
            }
        }
    }

    /// Reads the active pointer anchor from the pane receiving the drag.
    pub(super) fn table_cell_drag_anchor_for_surface(
        &self,
        surface: super::selection_surface::SelectionSurface,
    ) -> Option<(EntityId, TableCellPosition)> {
        match surface {
            super::selection_surface::SelectionSurface::Main => self.table_cell_drag_anchor,
            super::selection_surface::SelectionSurface::SplitPreview => {
                self.split_preview_table_cell_drag_anchor
            }
        }
    }

    /// Stores the table drag anchor with its owning pane so mouse-up can end only that gesture.
    pub(super) fn set_table_cell_drag_anchor_for_surface(
        &mut self,
        surface: super::selection_surface::SelectionSurface,
        anchor: Option<(EntityId, TableCellPosition)>,
    ) {
        match surface {
            super::selection_surface::SelectionSurface::Main => {
                self.table_cell_drag_anchor = anchor
            }
            super::selection_surface::SelectionSurface::SplitPreview => {
                self.split_preview_table_cell_drag_anchor = anchor;
            }
        }
    }

    /// Uses the existing per-surface registry so overlapping pane coordinates cannot hit the wrong table.
    pub(super) fn table_cell_at_point_for_surface(
        &self,
        surface: super::selection_surface::SelectionSurface,
        position: Point<Pixels>,
        cx: &App,
    ) -> Option<(EntityId, TableCellPosition)> {
        let bindings = match surface {
            super::selection_surface::SelectionSurface::Main => Some(&self.table_cells),
            super::selection_surface::SelectionSurface::SplitPreview => self
                .split_preview
                .as_ref()
                .map(|preview| &preview.table_cells),
        }?;
        bindings.values().find_map(|binding| {
            let bounds = binding.cell.read(cx).last_bounds?;
            let inside = position.x >= bounds.left()
                && position.x <= bounds.right()
                && position.y >= bounds.top()
                && position.y <= bounds.bottom();
            inside.then_some((binding.table_block.entity_id(), binding.position))
        })
    }

    /// Returns table bindings for the selected surface without rebuilding either runtime.
    fn table_bindings_for_surface(
        &self,
        surface: super::selection_surface::SelectionSurface,
    ) -> Option<&HashMap<EntityId, TableCellBinding>> {
        match surface {
            super::selection_surface::SelectionSurface::Main => Some(&self.table_cells),
            super::selection_surface::SelectionSurface::SplitPreview => self
                .split_preview
                .as_ref()
                .map(|preview| &preview.table_cells),
        }
    }

    /// Resolves a table block from its own projection for read-only copying and guarded editing.
    fn table_block_for_surface(
        &self,
        surface: super::selection_surface::SelectionSurface,
        entity_id: EntityId,
        _cx: &App,
    ) -> Option<Entity<Block>> {
        self.table_bindings_for_surface(surface)?
            .values()
            .find(|binding| binding.table_block.entity_id() == entity_id)
            .map(|binding| binding.table_block.clone())
    }

    /// Updates the visible highlight only for the selected surface's runtime entities.
    pub(super) fn sync_table_cell_rectangle_highlights_for(
        &mut self,
        surface: super::selection_surface::SelectionSurface,
        cx: &mut Context<Self>,
    ) {
        let selection = self.table_cell_rectangle_for_surface(surface);
        let bindings = self
            .table_bindings_for_surface(surface)
            .map(|bindings| bindings.values().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        for binding in bindings {
            let selected = selection.is_some_and(|selection| {
                selection.table_block_id == binding.table_block.entity_id()
                    && selection.contains(binding.position)
            });
            binding.cell.update(cx, |cell, cx| {
                let next = if selected {
                    TableAxisHighlight::Selected
                } else {
                    TableAxisHighlight::None
                };
                if cell.table_axis_highlight != next {
                    cell.table_axis_highlight = next;
                    cx.notify();
                }
            });
        }
    }

    /// Prevents cell text ranges from competing with the explicit rectangular table selection.
    fn clear_table_cell_text_selections_for(
        &self,
        surface: super::selection_surface::SelectionSurface,
        table_block_id: EntityId,
        cx: &mut Context<Self>,
    ) {
        let Some(bindings) = self.table_bindings_for_surface(surface) else {
            return;
        };
        let cells = bindings
            .values()
            .filter(|binding| binding.table_block.entity_id() == table_block_id)
            .map(|binding| binding.cell.clone())
            .collect::<Vec<_>>();
        for cell in cells {
            cell.update(cx, |block, cx| {
                let had_selection = !block.selected_range.is_empty()
                    || block.editor_selection_range.is_some()
                    || block.editor_selection_supports_inline_commands
                    || block.is_selecting
                    || block.pointer_selection.is_some();
                if !had_selection {
                    return;
                }
                let cursor = block.cursor_offset();
                block.assign_collapsed_selection_offset(cursor, Default::default(), None);
                block.editor_selection_range = None;
                block.editor_selection_supports_inline_commands = false;
                block.is_selecting = false;
                block.pointer_selection = None;
                cx.notify();
            });
        }
    }

    /// Routes keyboard rectangle selection through the most recently focused document surface.
    pub(super) fn handle_table_cell_selection_key(
        &mut self,
        event: &KeyDownEvent,
        cx: &mut Context<Self>,
    ) -> bool {
        let surface = self.active_selection_surface;
        let key = event.keystroke.key.as_str();
        let current = self.table_cell_rectangle_for_surface(surface);
        if current.is_none() {
            if key != "escape" {
                return false;
            }
            let Some(binding) = self
                .active_entity_id
                .and_then(|id| self.table_bindings_for_surface(surface)?.get(&id).cloned())
            else {
                return false;
            };
            let table_block_id = binding.table_block.entity_id();
            self.clear_cross_block_selection_for_surface(surface, cx);
            self.clear_table_cell_text_selections_for(surface, table_block_id, cx);
            if surface == super::selection_surface::SelectionSurface::Main {
                self.clear_table_axis_selection(cx);
            }
            self.set_table_cell_rectangle_for_surface(
                surface,
                Some(TableCellRectangle {
                    table_block_id,
                    anchor: binding.position,
                    focus: binding.position,
                }),
            );
            self.sync_table_cell_rectangle_highlights_for(surface, cx);
            cx.notify();
            return true;
        }

        let selection = current.expect("checked above");
        if matches!(key, "enter" | "f2" | "escape") {
            self.set_table_cell_rectangle_for_surface(surface, None);
            self.sync_table_cell_rectangle_highlights_for(surface, cx);
            if key != "escape"
                && let Some(table) =
                    self.table_block_for_surface(surface, selection.table_block_id, cx)
            {
                self.focus_table_cell_position(&table, selection.focus, cx);
            }
            cx.notify();
            return true;
        }
        if matches!(key, "delete" | "backspace") {
            if !self.document_surface_is_editable() {
                return true;
            }
            return self.clear_table_cell_rectangle(selection, cx);
        }
        if !matches!(key, "left" | "right" | "up" | "down") {
            return false;
        }
        let Some(table_block) = self.table_block_for_surface(surface, selection.table_block_id, cx)
        else {
            self.set_table_cell_rectangle_for_surface(surface, None);
            return true;
        };
        let Some(table) = table_block.read(cx).record.table.as_ref() else {
            self.set_table_cell_rectangle_for_surface(surface, None);
            return true;
        };
        let max_row = table.rows.len();
        let max_column = table.column_count().saturating_sub(1);
        let mut focus = selection.focus;
        match key {
            "left" => focus.column = focus.column.saturating_sub(1),
            "right" => focus.column = (focus.column + 1).min(max_column),
            "up" => focus.row = focus.row.saturating_sub(1),
            "down" => focus.row = (focus.row + 1).min(max_row),
            _ => {}
        }
        self.set_table_cell_rectangle_for_surface(
            surface,
            Some(if event.keystroke.modifiers.shift {
                TableCellRectangle { focus, ..selection }
            } else {
                TableCellRectangle {
                    anchor: focus,
                    focus,
                    ..selection
                }
            }),
        );
        self.sync_table_cell_rectangle_highlights_for(surface, cx);
        cx.notify();
        true
    }

    /// Clears cells only after an editable Main surface has passed the actual write boundary.
    pub(super) fn clear_table_cell_rectangle(
        &mut self,
        selection: TableCellRectangle,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.document_surface_is_editable()
            || self.active_selection_surface != super::selection_surface::SelectionSurface::Main
        {
            return false;
        }
        let Some(table_block) = self.table_block_for_surface(
            super::selection_surface::SelectionSurface::Main,
            selection.table_block_id,
            cx,
        ) else {
            return false;
        };
        self.sync_table_record_from_runtime(&table_block, cx);
        let Some(mut table) = table_block.read(cx).record.table.clone() else {
            return false;
        };
        if !table.clear_cell_rectangle(selection.rows(), selection.columns()) {
            return true;
        }
        self.prepare_undo_capture(UndoCaptureKind::NonCoalescible, cx);
        table_block.update(cx, move |block, _cx| block.record.table = Some(table));
        self.rebuild_table_runtimes(cx);
        self.set_table_cell_rectangle_for_surface(
            super::selection_surface::SelectionSurface::Main,
            Some(selection),
        );
        self.sync_table_cell_rectangle_highlights(cx);
        self.mark_dirty(cx);
        self.finalize_pending_undo_capture(cx);
        cx.notify();
        true
    }

    /// Serializes the active pane's native table selection as TSV for ordinary Copy.
    pub(super) fn selected_table_cells_tsv(&self, cx: &App) -> Option<String> {
        let surface = self.active_selection_surface;
        let selection = self.table_cell_rectangle_for_surface(surface)?;
        let table = self.table_block_for_surface(surface, selection.table_block_id, cx)?;
        let table = table.read(cx).record.table.as_ref()?.clone();
        let mut lines = Vec::new();
        for row in selection.rows() {
            let cells = if row == 0 {
                &table.header
            } else {
                table.rows.get(row - 1)?
            };
            lines.push(
                selection
                    .columns()
                    .map(|column| {
                        cells
                            .get(column)
                            .map(|cell| cell.visible_text().replace(['\t', '\n', '\r'], " "))
                            .unwrap_or_default()
                    })
                    .collect::<Vec<_>>()
                    .join("\t"),
            );
        }
        Some(lines.join("\n"))
    }

    /// Rejects paste before reading or mutating the table when the focused pane is read-only.
    pub(super) fn paste_table_cells_tsv(&mut self, text: &str, cx: &mut Context<Self>) -> bool {
        if !self.document_surface_is_editable()
            || self.active_selection_surface != super::selection_surface::SelectionSurface::Main
        {
            return false;
        }
        let Some(matrix) = parse_tsv_matrix(text) else {
            return false;
        };
        let surface = self.active_selection_surface;
        let Some(selection) = self.table_cell_rectangle_for_surface(surface) else {
            return false;
        };
        let Some(table_block) = self.table_block_for_surface(surface, selection.table_block_id, cx)
        else {
            return false;
        };
        self.sync_table_record_from_runtime(&table_block, cx);
        let Some(mut table) = table_block.read(cx).record.table.clone() else {
            return false;
        };
        let start_row = *selection.rows().start();
        let start_column = *selection.columns().start();
        let required_visual_rows = start_row + matrix.len();
        while table.rows.len() + 1 < required_visual_rows {
            table.append_row();
        }
        let required_columns = start_column + matrix[0].len();
        while table.column_count() < required_columns {
            let alignment = table
                .alignments
                .last()
                .copied()
                .unwrap_or(TableColumnAlignment::Default);
            table.append_column(alignment);
        }
        for (row_offset, values) in matrix.iter().enumerate() {
            let visual_row = start_row + row_offset;
            let cells = if visual_row == 0 {
                &mut table.header
            } else {
                &mut table.rows[visual_row - 1]
            };
            for (column_offset, value) in values.iter().enumerate() {
                cells[start_column + column_offset] = InlineTextTree::plain(value.clone());
            }
        }
        self.prepare_undo_capture(UndoCaptureKind::NonCoalescible, cx);
        table_block.update(cx, move |block, _cx| block.record.table = Some(table));
        self.rebuild_table_runtimes(cx);
        self.set_table_cell_rectangle_for_surface(
            super::selection_surface::SelectionSurface::Main,
            Some(TableCellRectangle {
                table_block_id: selection.table_block_id,
                anchor: TableCellPosition {
                    row: start_row,
                    column: start_column,
                },
                focus: TableCellPosition {
                    row: start_row + matrix.len() - 1,
                    column: start_column + matrix[0].len() - 1,
                },
            }),
        );
        self.sync_table_cell_rectangle_highlights(cx);
        self.mark_dirty(cx);
        self.finalize_pending_undo_capture(cx);
        cx.notify();
        true
    }
}

#[cfg(test)]
#[path = "../../tests/unit/editor/table_selection.rs"]
mod tests;
