// @author kongweiguang

//! Document-level navigation and Live block operations.

#[path = "editing_navigation_boundary.rs"]
mod boundary;
#[path = "editing_navigation_live.rs"]
mod live;
#[path = "editing_navigation_routes.rs"]
mod routes;
#[path = "editing_navigation_structural.rs"]
mod structural;

pub(in crate::editor) use routes::{EditorActionRoute, FocusedNavigationAction};

use super::*;
use crate::components::{
    DeleteLine, DuplicateLine, EditingCommandId, EditingSelectionContext, EditingViewMode,
    IndentBlock, LineOperation, MoveLineDown, MoveLineUp, MoveToDocumentEnd, MoveToDocumentStart,
    OutdentBlock, PageDown, PageUp, SelectDown, SelectPageDown, SelectPageUp, SelectToDocumentEnd,
    SelectToDocumentStart, SelectUp,
};
use crate::editor::selection_surface::SelectionSurface;

impl Editor {
    /// Moves the caret to a document edge, while an extended move keeps the original source anchor.
    pub(crate) fn on_move_to_document_start(
        &mut self,
        action: &MoveToDocumentStart,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.route_navigation_action_to_focused_owner(
            FocusedNavigationAction::MoveToDocumentStart,
            window,
            cx,
        ) != EditorActionRoute::Local
        {
            return;
        }
        if self.defer_action_for_ime(action, window, cx) {
            cx.stop_propagation();
            return;
        }
        self.move_to_document_boundary(false, false, window, cx);
        cx.stop_propagation();
    }

    /// Moves the caret to a document edge, while an extended move keeps the original source anchor.
    pub(crate) fn on_move_to_document_end(
        &mut self,
        action: &MoveToDocumentEnd,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.route_navigation_action_to_focused_owner(
            FocusedNavigationAction::MoveToDocumentEnd,
            window,
            cx,
        ) != EditorActionRoute::Local
        {
            return;
        }
        if self.defer_action_for_ime(action, window, cx) {
            cx.stop_propagation();
            return;
        }
        self.move_to_document_boundary(true, false, window, cx);
        cx.stop_propagation();
    }

    /// Extends the active surface selection from its current anchor to the document start.
    pub(crate) fn on_select_to_document_start(
        &mut self,
        action: &SelectToDocumentStart,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.route_navigation_action_to_focused_owner(
            FocusedNavigationAction::SelectToDocumentStart,
            window,
            cx,
        ) != EditorActionRoute::Local
        {
            return;
        }
        if self.defer_action_for_ime(action, window, cx) {
            cx.stop_propagation();
            return;
        }
        self.move_to_document_boundary(false, true, window, cx);
        cx.stop_propagation();
    }

    /// Extends the active surface selection from its current anchor to the document end.
    pub(crate) fn on_select_to_document_end(
        &mut self,
        action: &SelectToDocumentEnd,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.route_navigation_action_to_focused_owner(
            FocusedNavigationAction::SelectToDocumentEnd,
            window,
            cx,
        ) != EditorActionRoute::Local
        {
            return;
        }
        if self.defer_action_for_ime(action, window, cx) {
            cx.stop_propagation();
            return;
        }
        self.move_to_document_boundary(true, true, window, cx);
        cx.stop_propagation();
    }

    /// Extends by one visual row and crosses block boundaries without losing selection direction.
    pub(crate) fn on_select_up(
        &mut self,
        action: &SelectUp,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.route_navigation_action_to_focused_owner(
            FocusedNavigationAction::SelectUp,
            window,
            cx,
        ) != EditorActionRoute::Local
        {
            return;
        }
        if self.defer_action_for_ime(action, window, cx) {
            cx.stop_propagation();
            return;
        }
        self.move_selection_by_visual_lines(-1, 1, true, window, cx);
        cx.stop_propagation();
    }

    /// Extends by one visual row and crosses block boundaries without losing selection direction.
    pub(crate) fn on_select_down(
        &mut self,
        action: &SelectDown,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.route_navigation_action_to_focused_owner(
            FocusedNavigationAction::SelectDown,
            window,
            cx,
        ) != EditorActionRoute::Local
        {
            return;
        }
        if self.defer_action_for_ime(action, window, cx) {
            cx.stop_propagation();
            return;
        }
        self.move_selection_by_visual_lines(1, 1, true, window, cx);
        cx.stop_propagation();
    }

    /// Extends by one viewport of visual rows, preserving the active pane's selection anchor.
    pub(crate) fn on_select_page_up(
        &mut self,
        action: &SelectPageUp,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.route_navigation_action_to_focused_owner(
            FocusedNavigationAction::SelectPageUp,
            window,
            cx,
        ) != EditorActionRoute::Local
        {
            return;
        }
        if self.defer_action_for_ime(action, window, cx) {
            cx.stop_propagation();
            return;
        }
        let lines = self.page_line_count(window, cx);
        self.move_selection_by_visual_lines(-1, lines, true, window, cx);
        cx.stop_propagation();
    }

    /// Extends by one viewport of visual rows, preserving the active pane's selection anchor.
    pub(crate) fn on_select_page_down(
        &mut self,
        action: &SelectPageDown,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.route_navigation_action_to_focused_owner(
            FocusedNavigationAction::SelectPageDown,
            window,
            cx,
        ) != EditorActionRoute::Local
        {
            return;
        }
        if self.defer_action_for_ime(action, window, cx) {
            cx.stop_propagation();
            return;
        }
        let lines = self.page_line_count(window, cx);
        self.move_selection_by_visual_lines(1, lines, true, window, cx);
        cx.stop_propagation();
    }

    /// Routes structural row shortcuts through the same tree transaction used by the block menu.
    pub(crate) fn on_duplicate_line(
        &mut self,
        action: &DuplicateLine,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.route_line_operation_to_focused_owner(LineOperation::Duplicate, window, cx) {
            EditorActionRoute::Local => {}
            EditorActionRoute::FocusedEditor
            | EditorActionRoute::DocumentHost
            | EditorActionRoute::PassThrough => return,
        }
        if !self.document_surface_is_editable() {
            cx.stop_propagation();
            return;
        }
        if self.defer_action_for_ime(action, window, cx) {
            cx.stop_propagation();
            return;
        }
        if self.apply_focused_raw_line_operation(LineOperation::Duplicate, window, cx) {
            cx.stop_propagation();
            return;
        }
        self.apply_live_block_command(EditingCommandId::DuplicateBlock, window, cx);
        cx.stop_propagation();
    }

    /// Deletes the focused Live paragraph/list item as one undoable structural operation.
    pub(crate) fn on_delete_line(
        &mut self,
        action: &DeleteLine,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.route_line_operation_to_focused_owner(LineOperation::Delete, window, cx) {
            EditorActionRoute::Local => {}
            EditorActionRoute::FocusedEditor
            | EditorActionRoute::DocumentHost
            | EditorActionRoute::PassThrough => return,
        }
        if !self.document_surface_is_editable() {
            cx.stop_propagation();
            return;
        }
        if self.defer_action_for_ime(action, window, cx) {
            cx.stop_propagation();
            return;
        }
        if self.apply_focused_raw_line_operation(LineOperation::Delete, window, cx) {
            cx.stop_propagation();
            return;
        }
        self.apply_live_block_command(EditingCommandId::DeleteBlock, window, cx);
        cx.stop_propagation();
    }

    /// Moves the focused Live paragraph/list item among its actual siblings in one undo boundary.
    pub(crate) fn on_move_line_up(
        &mut self,
        action: &MoveLineUp,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.route_line_operation_to_focused_owner(LineOperation::MoveUp, window, cx) {
            EditorActionRoute::Local => {}
            EditorActionRoute::FocusedEditor
            | EditorActionRoute::DocumentHost
            | EditorActionRoute::PassThrough => return,
        }
        if !self.document_surface_is_editable() {
            cx.stop_propagation();
            return;
        }
        if self.defer_action_for_ime(action, window, cx) {
            cx.stop_propagation();
            return;
        }
        if self.apply_focused_raw_line_operation(LineOperation::MoveUp, window, cx) {
            cx.stop_propagation();
            return;
        }
        self.apply_live_block_command(EditingCommandId::MoveBlockUp, window, cx);
        cx.stop_propagation();
    }

    /// Moves the focused Live paragraph/list item among its actual siblings in one undo boundary.
    pub(crate) fn on_move_line_down(
        &mut self,
        action: &MoveLineDown,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.route_line_operation_to_focused_owner(LineOperation::MoveDown, window, cx) {
            EditorActionRoute::Local => {}
            EditorActionRoute::FocusedEditor
            | EditorActionRoute::DocumentHost
            | EditorActionRoute::PassThrough => return,
        }
        if !self.document_surface_is_editable() {
            cx.stop_propagation();
            return;
        }
        if self.defer_action_for_ime(action, window, cx) {
            cx.stop_propagation();
            return;
        }
        if self.apply_focused_raw_line_operation(LineOperation::MoveDown, window, cx) {
            cx.stop_propagation();
            return;
        }
        self.apply_live_block_command(EditingCommandId::MoveBlockDown, window, cx);
        cx.stop_propagation();
    }

    /// Settles native composition before indenting Raw/Code lines or applying a Live block indent.
    pub(crate) fn on_indent_block(
        &mut self,
        action: &IndentBlock,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.route_line_operation_to_focused_owner(LineOperation::Indent, window, cx) {
            EditorActionRoute::Local => {}
            EditorActionRoute::FocusedEditor
            | EditorActionRoute::DocumentHost
            | EditorActionRoute::PassThrough => return,
        }
        if !self.document_surface_is_editable() {
            cx.stop_propagation();
            return;
        }
        if self.defer_action_for_ime(action, window, cx) {
            cx.stop_propagation();
            return;
        }
        self.apply_focused_indent_operation(LineOperation::Indent, window, cx);
        cx.stop_propagation();
    }

    /// Settles native composition before outdenting Raw/Code lines or applying a Live block outdent.
    pub(crate) fn on_outdent_block(
        &mut self,
        action: &OutdentBlock,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.route_line_operation_to_focused_owner(LineOperation::Outdent, window, cx) {
            EditorActionRoute::Local => {}
            EditorActionRoute::FocusedEditor
            | EditorActionRoute::DocumentHost
            | EditorActionRoute::PassThrough => return,
        }
        if !self.document_surface_is_editable() {
            cx.stop_propagation();
            return;
        }
        if self.defer_action_for_ime(action, window, cx) {
            cx.stop_propagation();
            return;
        }
        self.apply_focused_indent_operation(LineOperation::Outdent, window, cx);
        cx.stop_propagation();
    }

    /// Applies one indentation action to the focused editable block after the caller's IME gate.
    fn apply_focused_indent_operation(
        &mut self,
        operation: LineOperation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.handle_virtual_live_selection_line_operation(operation, window, cx) {
            return;
        }
        let Some((surface, block)) = self.focused_document_target(window, cx) else {
            return;
        };
        if surface != SelectionSurface::Main
            || !self.document_surface_is_editable_for(surface)
            || self.cross_block_selection_for_surface(surface).is_some()
            || block.read(cx).is_read_only()
        {
            return;
        }

        block.update(cx, |block, cx| match operation {
            LineOperation::Indent => block.on_indent_block(&IndentBlock, window, cx),
            LineOperation::Outdent => block.on_outdent_block(&OutdentBlock, window, cx),
            _ => {}
        });
    }

    /// Lets Raw/Code blocks edit logical lines after the Editor has settled any native IME session.
    fn apply_focused_raw_line_operation(
        &mut self,
        operation: LineOperation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some((surface, block)) = self.focused_document_target(window, cx) else {
            return false;
        };
        if surface != SelectionSurface::Main
            || !self.document_surface_is_editable_for(surface)
            || self.cross_block_selection_for_surface(surface).is_some()
        {
            return false;
        }

        block.update(cx, |block, cx| {
            block.apply_raw_line_operation(operation, window, cx)
        })
    }

    /// Resolves the active endpoint from the focused pane rather than the last-clicked global view.
    fn move_to_document_boundary(
        &mut self,
        at_end: bool,
        extend: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((surface, focused)) = self.focused_document_target(window, cx) else {
            return;
        };
        if self.move_to_virtual_document_boundary(surface, &focused, at_end, extend, window, cx) {
            return;
        }
        let blocks = self.selection_surface_entities(surface);
        let Some(boundary) = (if at_end {
            blocks.last()
        } else {
            blocks.first()
        })
        .cloned() else {
            return;
        };
        let boundary_offset = if at_end {
            boundary.read(cx).visible_len()
        } else {
            0
        };

        if !extend {
            self.clear_cross_block_selection_for_surface(surface, cx);
            Self::set_local_cursor(&boundary, boundary_offset, cx);
            self.focus_navigation_target(surface, &boundary, window, cx);
            cx.notify();
            return;
        }

        let previous = self.cross_block_selection_for_surface(surface);
        let source_anchor = previous.and_then(|selection| selection.source_anchor);
        let anchor = previous
            .map(|selection| selection.anchor)
            .unwrap_or_else(|| {
                let block = focused.read(cx);
                let offset = if block.selection_reversed {
                    block.selected_range.end
                } else {
                    block.selected_range.start
                };
                CrossBlockSelectionEndpoint {
                    entity_id: focused.entity_id(),
                    offset,
                }
            });
        if anchor.entity_id == boundary.entity_id() {
            self.set_cross_block_selection_for_surface(surface, None);
            Self::set_local_selection(&boundary, anchor.offset, boundary_offset, cx);
            self.sync_cross_block_selection_visuals_for_surface(surface, cx);
        } else {
            Self::set_local_cursor(&boundary, boundary_offset, cx);
            self.set_cross_block_selection_for_surface(
                surface,
                Some(self.cross_block_selection_from_endpoints(
                    surface,
                    anchor,
                    CrossBlockSelectionEndpoint {
                        entity_id: boundary.entity_id(),
                        offset: boundary_offset,
                    },
                    source_anchor,
                    cx,
                )),
            );
            self.sync_cross_block_selection_visuals_for_surface(surface, cx);
        }
        self.active_selection_surface = surface;
        self.focus_navigation_target(surface, &boundary, window, cx);
        cx.notify();
    }

    /// Moves one visual row at a time so a page action can continue through neighboring blocks.
    fn move_selection_by_visual_lines(
        &mut self,
        direction: i32,
        line_count: usize,
        extend: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if direction == 0 || line_count == 0 {
            return false;
        }
        let Some((surface, focused)) = self.focused_document_target(window, cx) else {
            return false;
        };
        let blocks = self.selection_surface_entities(surface);
        if blocks.is_empty() {
            return false;
        }

        let existing = self.cross_block_selection_for_surface(surface);
        let mut current = existing
            .and_then(|selection| {
                blocks
                    .iter()
                    .find(|block| block.entity_id() == selection.focus.entity_id)
                    .cloned()
                    .map(|block| (block, selection.focus.offset))
            })
            .unwrap_or_else(|| {
                let offset = focused.read(cx).cursor_offset();
                (focused, offset)
            });
        let mut anchor = existing.map(|selection| selection.anchor);
        let mut source_anchor = existing.and_then(|selection| selection.source_anchor);
        if existing.is_some() && !extend {
            self.clear_cross_block_selection_for_surface(surface, cx);
            Self::set_local_cursor(&current.0, current.1, cx);
            anchor = None;
            source_anchor = None;
        } else if extend && anchor.is_none() {
            let block = current.0.read(cx);
            anchor = Some(CrossBlockSelectionEndpoint {
                entity_id: current.0.entity_id(),
                offset: if block.selection_reversed {
                    block.selected_range.end
                } else {
                    block.selected_range.start
                },
            });
        }
        if extend && source_anchor.is_none() {
            source_anchor = anchor.and_then(|endpoint| {
                self.cross_block_source_anchor_for_endpoint(surface, endpoint, cx)
            });
        }
        let mut cross_selection_active = existing.is_some() && extend;
        let mut moved = false;

        for _ in 0..line_count {
            let preferred_x = current.0.read(cx).preferred_visual_x();
            let local_move = current.0.update(cx, |block, cx| {
                block.move_cursor_by_visual_lines(
                    direction,
                    1,
                    extend && !cross_selection_active,
                    cx,
                )
            });
            if local_move {
                current.1 = current.0.read(cx).cursor_offset();
                if cross_selection_active {
                    let next = CrossBlockSelectionEndpoint {
                        entity_id: current.0.entity_id(),
                        offset: current.1,
                    };
                    let anchor = anchor.unwrap_or(next);
                    if anchor.entity_id == next.entity_id {
                        self.set_cross_block_selection_for_surface(surface, None);
                        Self::set_local_selection(&current.0, anchor.offset, next.offset, cx);
                        cross_selection_active = false;
                    } else {
                        self.set_cross_block_selection_for_surface(
                            surface,
                            Some(self.cross_block_selection_from_endpoints(
                                surface,
                                anchor,
                                next,
                                source_anchor,
                                cx,
                            )),
                        );
                        self.sync_cross_block_selection_visuals_for_surface(surface, cx);
                    }
                }
                moved = true;
                continue;
            }

            let Some(index) = blocks
                .iter()
                .position(|block| block.entity_id() == current.0.entity_id())
            else {
                break;
            };
            let next_index = if direction < 0 {
                index.checked_sub(1)
            } else {
                (index + 1 < blocks.len()).then_some(index + 1)
            };
            let Some(next) = next_index.and_then(|index| blocks.get(index)).cloned() else {
                break;
            };
            let offset = next
                .read(cx)
                .entry_offset_for_vertical_focus(direction < 0, Some(preferred_x));
            if extend {
                let anchor = anchor.unwrap_or(CrossBlockSelectionEndpoint {
                    entity_id: current.0.entity_id(),
                    offset: current.1,
                });
                Self::set_local_cursor_with_vertical_x(&next, offset, Some(preferred_x), cx);
                self.set_cross_block_selection_for_surface(
                    surface,
                    Some(self.cross_block_selection_from_endpoints(
                        surface,
                        anchor,
                        CrossBlockSelectionEndpoint {
                            entity_id: next.entity_id(),
                            offset,
                        },
                        source_anchor,
                        cx,
                    )),
                );
                self.sync_cross_block_selection_visuals_for_surface(surface, cx);
                self.active_selection_surface = surface;
                cross_selection_active = true;
            } else {
                self.clear_cross_block_selection_for_surface(surface, cx);
                Self::set_local_cursor_with_vertical_x(&next, offset, Some(preferred_x), cx);
            }
            self.focus_navigation_target(surface, &next, window, cx);
            current = (next, offset);
            moved = true;
        }

        if moved {
            self.active_selection_surface = surface;
            cx.notify();
        }
        moved
    }

    /// Computes a page step from the focused pane's viewport and current text line height.
    fn page_line_count(&self, window: &Window, cx: &App) -> usize {
        let Some((surface, block)) = self.focused_document_target(window, cx) else {
            return 1;
        };
        let viewport_height = match surface {
            SelectionSurface::Main => self.scroll_handle.bounds().size.height,
            SelectionSurface::SplitPreview => self
                .split_preview
                .as_ref()
                .map(|preview| preview.scroll_handle.bounds().size.height)
                .unwrap_or(px(0.0)),
        };
        let line_height = block.read(cx).last_line_height.max(px(1.0));
        (f32::from(viewport_height) / f32::from(line_height))
            .floor()
            .max(1.0) as usize
    }

    /// Places the caret or an ordered local selection using byte offsets already owned by Block.
    fn set_local_cursor(block: &Entity<Block>, offset: usize, cx: &mut Context<Self>) {
        Self::set_local_cursor_with_vertical_x(block, offset, None, cx);
    }

    /// Seeds a destination block with the source column so repeated vertical moves remain aligned.
    fn set_local_cursor_with_vertical_x(
        block: &Entity<Block>,
        offset: usize,
        preferred_x: Option<gpui::Pixels>,
        cx: &mut Context<Self>,
    ) {
        block.update(cx, |block, cx| {
            let offset = offset.min(block.visible_len());
            block.selected_range = offset..offset;
            block.selection_reversed = false;
            block.marked_range = None;
            block.vertical_motion_x = preferred_x;
            block.cursor_blink_epoch = std::time::Instant::now();
            cx.emit(crate::components::BlockEvent::SelectionChanged);
            cx.notify();
        });
    }

    /// Preserves anchor direction as selection offsets are stored normalized in Block.
    fn set_local_selection(
        block: &Entity<Block>,
        anchor: usize,
        focus: usize,
        cx: &mut Context<Self>,
    ) {
        block.update(cx, |block, cx| {
            let anchor = anchor.min(block.visible_len());
            let focus = focus.min(block.visible_len());
            block.selected_range = anchor.min(focus)..anchor.max(focus);
            block.selection_reversed = focus < anchor && !block.selected_range.is_empty();
            block.marked_range = None;
            block.vertical_motion_x = None;
            block.cursor_blink_epoch = std::time::Instant::now();
            cx.emit(crate::components::BlockEvent::SelectionChanged);
            cx.notify();
        });
    }

    /// Focus stays pane-local; Split Preview never writes the Main editor's pending-focus target.
    fn focus_navigation_target(
        &mut self,
        surface: SelectionSurface,
        block: &Entity<Block>,
        window: &mut Window,
        cx: &App,
    ) {
        self.active_selection_surface = surface;
        match surface {
            SelectionSurface::Main => self.focus_block(block.entity_id()),
            SelectionSurface::SplitPreview => {
                block.read(cx).focus_handle.clone().focus(window);
            }
        }
    }
}
