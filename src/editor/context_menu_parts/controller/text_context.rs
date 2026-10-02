// @author kongweiguang

use super::*;
use crate::components::Block;
use crate::editor::selection_surface::SelectionSurface;

impl Editor {
    /// Opens a pane-bound text menu only when the pointer resolved to editable text or a read-only preview.
    pub(in crate::editor) fn open_text_context_menu_for_surface(
        &mut self,
        surface: SelectionSurface,
        entity_id: EntityId,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.text_context_target_is_owned_by_surface(surface, entity_id) {
            return false;
        }
        let Some(block) = self.text_context_block_for_surface(surface, entity_id) else {
            return false;
        };

        let (offset, is_text, selection_contains_hit) = {
            let block_state = block.read(cx);
            let kind = block_state.kind();
            let is_text = block_state.record.resource.is_none()
                && !block_state.showing_rendered_image()
                && !matches!(
                    kind,
                    crate::components::BlockKind::Separator
                        | crate::components::BlockKind::Table
                        | crate::components::BlockKind::MathBlock
                        | crate::components::BlockKind::MermaidBlock
                        | crate::components::BlockKind::HtmlBlock
                        | crate::components::BlockKind::Comment
                );
            let offset = block_state.index_for_mouse_position(position);
            let contains = |range: &std::ops::Range<usize>| {
                !range.is_empty() && range.start <= offset && offset <= range.end
            };
            let selection_contains_hit = contains(&block_state.selected_range)
                || block_state
                    .editor_selection_range
                    .as_ref()
                    .is_some_and(contains);
            (offset, is_text, selection_contains_hit)
        };
        if !is_text {
            return false;
        }

        let selection_contains_hit =
            selection_contains_hit || self.text_context_table_cell_is_selected(surface, entity_id);
        self.close_menu_bar(cx);
        self.dismiss_active_contextual_editing_popovers(cx);
        self.active_selection_surface = surface;
        self.active_entity_id = Some(entity_id);
        block.read(cx).focus_handle.focus(window);

        if !selection_contains_hit {
            self.clear_cross_block_selection_for_surface(surface, cx);
            self.set_table_cell_rectangle_for_surface(surface, None);
            self.sync_table_cell_rectangle_highlights_for(surface, cx);
            if surface == SelectionSurface::Main {
                self.clear_table_axis_preview(cx);
                self.clear_table_axis_selection(cx);
            }
            block.update(cx, |block, cx| block.move_to(offset, cx));
        }

        self.context_menu_submenu_close_task = None;
        let insert_after = match surface {
            SelectionSurface::Main => self
                .table_cells
                .get(&entity_id)
                .map(|binding| binding.table_block.entity_id()),
            SelectionSurface::SplitPreview => self
                .split_preview
                .as_ref()
                .and_then(|preview| preview.table_cells.get(&entity_id))
                .map(|binding| binding.table_block.entity_id()),
        }
        .unwrap_or(entity_id);
        self.context_menu = Some(ContextMenuState::Text {
            position,
            surface,
            entity_id,
            insert_target: TableInsertTarget::After(self.root_ancestor_entity_id(insert_after)),
        });
        self.context_menu_keyboard_item = None;
        self.context_menu_keyboard_submenu_item = None;
        self.context_menu_scroll_handle
            .set_offset(point(px(0.0), px(0.0)));
        cx.notify();
        true
    }

    /// Resolves a text target from its owning pane to avoid a same-document lookup switching Split focus.
    pub(in crate::editor::context_menu) fn text_context_block_for_surface(
        &self,
        surface: SelectionSurface,
        entity_id: EntityId,
    ) -> Option<Entity<Block>> {
        if let Some(block) = self
            .selection_surface_entities(surface)
            .into_iter()
            .find(|block| block.entity_id() == entity_id)
        {
            return Some(block);
        }
        match surface {
            SelectionSurface::Main => self
                .table_cells
                .get(&entity_id)
                .map(|binding| binding.cell.clone()),
            SelectionSurface::SplitPreview => self
                .split_preview
                .as_ref()?
                .table_cells
                .get(&entity_id)
                .map(|binding| binding.cell.clone()),
        }
    }

    /// Confirms pane ownership through its own projection, including table-cell editor entities.
    pub(in crate::editor::context_menu) fn text_context_target_is_owned_by_surface(
        &self,
        surface: SelectionSurface,
        entity_id: EntityId,
    ) -> bool {
        if surface == SelectionSurface::SplitPreview && self.view_mode != ViewMode::Split {
            return false;
        }
        let surface_blocks = self.selection_surface_entities(surface);
        if surface_blocks
            .iter()
            .any(|block| block.entity_id() == entity_id)
        {
            return true;
        }
        let table_block_id = match surface {
            SelectionSurface::Main => self
                .table_cells
                .get(&entity_id)
                .map(|binding| binding.table_block.entity_id()),
            SelectionSurface::SplitPreview => self
                .split_preview
                .as_ref()
                .and_then(|preview| preview.table_cells.get(&entity_id))
                .map(|binding| binding.table_block.entity_id()),
        };
        table_block_id.is_some_and(|table_block_id| {
            surface_blocks
                .iter()
                .any(|block| block.entity_id() == table_block_id)
        })
    }

    /// Retains a rectangular table selection only when the menu belongs to a selected cell.
    fn text_context_table_cell_is_selected(
        &self,
        surface: SelectionSurface,
        entity_id: EntityId,
    ) -> bool {
        let selection = self.table_cell_rectangle_for_surface(surface);
        let binding = match surface {
            SelectionSurface::Main => self.table_cells.get(&entity_id),
            SelectionSurface::SplitPreview => self
                .split_preview
                .as_ref()
                .and_then(|preview| preview.table_cells.get(&entity_id)),
        };
        let (Some(selection), Some(binding)) = (selection, binding) else {
            return false;
        };
        selection.table_block_id == binding.table_block.entity_id()
            && selection.rows().contains(&binding.position.row)
            && selection.columns().contains(&binding.position.column)
    }

    /// Executes a text-menu Action only after rechecking the pane, target, and current enabled state.
    pub(in crate::editor::context_menu) fn execute_text_context_menu_command(
        &mut self,
        command: ContextMenuCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let enabled = self
            .context_menu_command_model(cx)
            .main
            .iter()
            .any(|entry| entry.command == command && entry.enabled);
        if !enabled {
            return;
        }
        let Some(ContextMenuState::Text {
            position,
            surface,
            entity_id,
            insert_target,
        }) = self.context_menu.as_ref()
        else {
            return;
        };
        let (position, surface, entity_id, insert_target) =
            (*position, *surface, *entity_id, *insert_target);
        if !self.text_context_target_is_owned_by_surface(surface, entity_id) {
            self.close_context_menu(cx);
            return;
        }
        let Some(block) = self.text_context_block_for_surface(surface, entity_id) else {
            self.close_context_menu(cx);
            return;
        };

        self.active_selection_surface = surface;
        self.active_entity_id = Some(entity_id);
        block.read(cx).focus_handle.focus(window);
        self.close_context_menu(cx);

        match command {
            ContextMenuCommand::TextOpenLink => {
                if let Some(link) = block.read(cx).pointer_link_hit(position) {
                    block.update(cx, |block, cx| block.open_rendered_link(&link, cx));
                }
            }
            ContextMenuCommand::TextCopy => {
                window.dispatch_action(Box::new(crate::components::Copy), cx)
            }
            ContextMenuCommand::TextCopyAsMarkdown => {
                window.dispatch_action(Box::new(crate::components::CopyAsMarkdown), cx)
            }
            ContextMenuCommand::TextCut => {
                window.dispatch_action(Box::new(crate::components::Cut), cx)
            }
            ContextMenuCommand::TextPaste => {
                window.dispatch_action(Box::new(crate::components::Paste), cx)
            }
            ContextMenuCommand::TextSelectAll => {
                window.dispatch_action(Box::new(crate::components::SelectAll), cx)
            }
            ContextMenuCommand::TextDuplicateLine => {
                window.dispatch_action(Box::new(crate::components::DuplicateLine), cx)
            }
            ContextMenuCommand::TextDeleteLine => {
                window.dispatch_action(Box::new(crate::components::DeleteLine), cx)
            }
            ContextMenuCommand::TextMoveLineUp => {
                window.dispatch_action(Box::new(crate::components::MoveLineUp), cx)
            }
            ContextMenuCommand::TextMoveLineDown => {
                window.dispatch_action(Box::new(crate::components::MoveLineDown), cx)
            }
            ContextMenuCommand::TextInsert => {
                self.open_insert_context_menu(position, insert_target, cx);
                if let Some(ContextMenuState::Insert {
                    insert_hovered,
                    submenu_open,
                    ..
                }) = self.context_menu.as_mut()
                {
                    *insert_hovered = true;
                    *submenu_open = true;
                    self.context_menu_keyboard_submenu_item = Some(0);
                    cx.notify();
                }
            }
            _ => {}
        }
    }
}
