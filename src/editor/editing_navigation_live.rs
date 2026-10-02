// @author kongweiguang

//! Live 单块与跨块结构命令入口，独立于光标导航。

use super::*;

impl Editor {
    /// Applies paragraph/list actions only when the focused block and sibling position support them.
    pub(super) fn apply_live_block_command(
        &mut self,
        command: EditingCommandId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((surface, target)) = self.focused_document_target(window, cx) else {
            return;
        };
        if self.view_mode != ViewMode::Rendered || !self.document_surface_is_editable_for(surface) {
            return;
        }
        if surface != SelectionSurface::Main {
            return;
        }

        let operation = match command {
            EditingCommandId::DuplicateBlock => Some(LineOperation::Duplicate),
            EditingCommandId::DeleteBlock => Some(LineOperation::Delete),
            EditingCommandId::MoveBlockUp => Some(LineOperation::MoveUp),
            EditingCommandId::MoveBlockDown => Some(LineOperation::MoveDown),
            _ => None,
        };
        if operation.is_some_and(|operation| {
            self.handle_virtual_live_selection_line_operation(operation, window, cx)
        }) {
            return;
        }

        if let Some(selection) = self.normalized_cross_block_selection_for_surface(surface, cx) {
            let visible = self.selection_surface_entities(surface);
            let target_is_selected = match (selection.start_index, selection.end_index) {
                (Some(start), Some(end)) => visible
                    .iter()
                    .position(|block| block.entity_id() == target.entity_id())
                    .is_some_and(|index| (start..=end).contains(&index)),
                _ => false,
            };
            if selection
                .start_index
                .zip(selection.end_index)
                .is_some_and(|(start, end)| start != end)
                && target_is_selected
            {
                self.apply_live_cross_block_command(command, selection, window, cx);
                return;
            }
        }

        let Some(location) = self.document.find_block_location(target.entity_id()) else {
            return;
        };
        let siblings = location
            .parent
            .as_ref()
            .map(|parent| parent.read(cx).children.len())
            .unwrap_or_else(|| self.document.root_count());
        let mut context = target.read(cx).editing_command_context();
        context.view_mode = EditingViewMode::Rendered;
        context.selection = EditingSelectionContext::None;
        context.sibling_index = location.index;
        context.sibling_count = siblings;
        if !matches!(target.read(cx).kind(), BlockKind::Paragraph)
            && !target.read(cx).kind().is_list_item()
        {
            return;
        }
        if !command.is_available(context) {
            return;
        }

        let cursor = target.read(cx).cursor_offset();
        self.clear_cross_block_selection_for_surface(SelectionSurface::Main, cx);
        self.prepare_undo_capture(UndoCaptureKind::NonCoalescible, cx);
        target.update(cx, |block, _cx| {
            block.selected_range = cursor..cursor;
            block.selection_reversed = false;
        });
        self.apply_slash_command(target, command, cursor..cursor, cx);
    }
}
