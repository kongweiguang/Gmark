// @author kongweiguang

//! Native table cell and axis runtime state.

use super::*;

impl Block {
    pub(crate) fn is_table_cell(&self) -> bool {
        self.table_cell_position.is_some()
    }

    pub(crate) fn table_cell_position(&self) -> Option<TableCellPosition> {
        self.table_cell_position
    }

    pub(crate) fn table_cell_alignment(&self) -> Option<TableColumnAlignment> {
        self.table_cell_alignment
    }

    pub(crate) fn text_align(&self) -> TextAlign {
        match self
            .table_cell_alignment()
            .unwrap_or(TableColumnAlignment::Default)
        {
            TableColumnAlignment::Default | TableColumnAlignment::Left => TextAlign::Left,
            TableColumnAlignment::Center => TextAlign::Center,
            TableColumnAlignment::Right => TextAlign::Right,
        }
    }

    pub(crate) fn set_table_cell_mode(
        &mut self,
        position: TableCellPosition,
        alignment: TableColumnAlignment,
    ) {
        self.table_cell_position = Some(position);
        self.table_cell_alignment = Some(alignment);
        self.edit_mode = EditMode::RenderedRich;
        self.clear_inline_projection();
        self.sync_render_cache();
    }

    /// 替换 cell 实体后标记视口待同步，确保新实体拿到所属正文的裁剪范围。
    pub(crate) fn set_table_runtime(&mut self, runtime: TableRuntime) {
        self.table_cells_toolbar_viewport_dirty = true;
        self.table_runtime = Some(runtime);
    }

    #[expect(
        dead_code,
        reason = "table layout persistence is consumed by the render integration"
    )]
    pub(crate) fn set_table_column_layout(&mut self, layout: Option<TableColumnLayout>) {
        self.table_column_layout = layout;
    }

    pub(crate) fn start_table_column_resize(
        &mut self,
        boundary: usize,
        start_x: Pixels,
        table_width: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.is_read_only() || self.table_column_layout.is_none() {
            return;
        }
        self.table_column_resize_boundary = Some(boundary);
        self.table_column_resize_session =
            self.table_column_layout
                .clone()
                .map(|start_layout| TableColumnResizeSession {
                    boundary,
                    start_x,
                    start_layout,
                    table_width: table_width.max(1.0),
                });
        self.table_column_resize_focus_handle.focus(window);
        cx.stop_propagation();
        cx.notify();
    }

    pub(crate) fn update_table_column_resize(
        &mut self,
        pointer_x: Pixels,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(session) = self.table_column_resize_session.as_ref() else {
            return false;
        };
        let mut next = session.start_layout.clone();
        let delta = f32::from(pointer_x - session.start_x);
        let minimum = 48.0;
        if !next.resize_boundary(session.boundary, delta, session.table_width, minimum) {
            return false;
        }
        if self.table_column_layout.as_ref() == Some(&next) {
            return false;
        }
        self.table_column_layout = Some(next.clone());
        if let Some(key) = self.table_view_key.as_ref() {
            cx.emit(BlockEvent::TableColumnLayoutChanged {
                key: key.to_string(),
                fractions: next.fractions(),
            });
        }
        cx.notify();
        true
    }

    pub(crate) fn finish_table_column_resize(&mut self, cx: &mut Context<Self>) -> bool {
        let changed = self.table_column_resize_session.take().is_some();
        self.table_column_resize_boundary = None;
        if changed {
            cx.notify();
        }
        changed
    }

    pub(crate) fn on_table_column_resize_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.dragging() {
            self.update_table_column_resize(event.position.x, cx);
        }
    }

    pub(crate) fn on_table_column_resize_mouse_up(
        &mut self,
        _event: &MouseUpEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.finish_table_column_resize(cx);
    }

    pub(crate) fn reset_table_column_layout(&mut self, cx: &mut Context<Self>) {
        self.table_column_layout = None;
        self.table_column_resize_session = None;
        self.table_column_resize_boundary = None;
        if let Some(key) = self.table_view_key.as_ref() {
            cx.emit(BlockEvent::ResetTableColumnLayout {
                key: key.to_string(),
            });
        }
        cx.notify();
    }

    pub(crate) fn on_table_column_resize_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(boundary) = self.table_column_resize_boundary else {
            return;
        };
        let delta = match event.keystroke.key.as_str() {
            "left" => -8.0,
            "right" => 8.0,
            "home" => -32.0,
            "end" => 32.0,
            _ => return,
        };
        let Some(layout) = self.table_column_layout.clone() else {
            return;
        };
        let width = self
            .last_bounds
            .map(|bounds| f32::from(bounds.size.width))
            .unwrap_or(800.0);
        let mut next = layout;
        if !next.resize_boundary(boundary, delta, width.max(1.0), 48.0) {
            return;
        }
        self.table_column_layout = Some(next.clone());
        if let Some(key) = self.table_view_key.as_ref() {
            cx.emit(BlockEvent::TableColumnLayoutChanged {
                key: key.to_string(),
                fractions: next.fractions(),
            });
        }
        self.table_column_resize_focus_handle.focus(window);
        cx.stop_propagation();
        cx.notify();
    }

    pub(crate) fn clear_table_runtime(&mut self) {
        self.table_runtime = None;
        self.table_axis_preview = None;
        self.table_axis_selection = None;
        self.table_axis_highlight = TableAxisHighlight::None;
        self.table_append_column_edge_hovered = false;
        self.table_append_column_hovered = false;
        self.table_append_column_zone_hovered = false;
        self.table_append_column_button_hovered = false;
        self.table_append_column_close_task = None;
        self.table_append_row_edge_hovered = false;
        self.table_append_row_hovered = false;
        self.table_append_row_zone_hovered = false;
        self.table_append_row_button_hovered = false;
        self.table_append_row_close_task = None;
        self.table_column_layout = None;
        self.table_view_key = None;
        self.table_column_resize_session = None;
        self.table_column_resize_boundary = None;
    }

    pub(crate) fn set_table_axis_visual_state(
        &mut self,
        preview: Option<TableAxisMarker>,
        selection: Option<TableAxisMarker>,
    ) {
        self.table_axis_preview = preview;
        self.table_axis_selection = selection;
    }

    pub(crate) fn set_table_axis_highlight(&mut self, highlight: TableAxisHighlight) {
        self.table_axis_highlight = highlight;
    }
}
