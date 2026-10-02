// @author kongweiguang

use gpui::{App, Entity, EntityId, Window};

use super::{Block, CrossBlockDrag, CrossBlockSelection, Editor, ViewMode};

/// Identifies which rendered document owns pointer selection and clipboard actions.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum SelectionSurface {
    #[default]
    Main,
    SplitPreview,
}

impl Editor {
    /// Keeps mutating commands attached to the pane that owns the current editing focus.
    pub(super) fn document_surface_is_editable(&self) -> bool {
        self.document_surface_is_editable_for(self.active_selection_surface)
    }

    /// Uses an explicit pane for delayed block events whose active pointer pane may have changed.
    pub(super) fn document_surface_is_editable_for(&self, surface: SelectionSurface) -> bool {
        self.view_mode != ViewMode::Preview
            && !(self.view_mode == ViewMode::Split && surface == SelectionSurface::SplitPreview)
    }

    /// Resolves primary text focus from mounted blocks or the still-owned virtual selection endpoint.
    pub(super) fn focused_document_target(
        &self,
        window: &Window,
        cx: &App,
    ) -> Option<(SelectionSurface, Entity<Block>)> {
        for surface in [SelectionSurface::Main, SelectionSurface::SplitPreview] {
            if let Some(entity) =
                self.selection_surface_entities(surface)
                    .into_iter()
                    .find(|entity| {
                        entity.read_with(cx, |block, _cx| block.focus_handle.is_focused(window))
                    })
            {
                return Some((surface, entity));
            }
            let table_cells = match surface {
                SelectionSurface::Main => Some(&self.table_cells),
                SelectionSurface::SplitPreview => self
                    .split_preview
                    .as_ref()
                    .map(|preview| &preview.table_cells),
            };
            if let Some(entity) = table_cells.and_then(|cells| {
                cells.values().find_map(|binding| {
                    binding
                        .cell
                        .read_with(cx, |cell, _cx| cell.focus_handle.is_focused(window))
                        .then(|| binding.cell.clone())
                })
            }) {
                return Some((surface, entity));
            }
            if surface == SelectionSurface::Main
                && let Some(entity) = self.focused_virtual_main_selection_target(window, cx)
            {
                return Some((surface, entity));
            }
        }
        None
    }

    /// Keeps real Main focus attached across virtualization; Preview shares this source surface for read-only selection.
    fn focused_virtual_main_selection_target(
        &self,
        window: &Window,
        cx: &App,
    ) -> Option<Entity<Block>> {
        if !matches!(self.view_mode, ViewMode::Rendered | ViewMode::Preview)
            || self.active_selection_surface != SelectionSurface::Main
        {
            return None;
        }
        let virtual_surface = self.virtual_surface.as_ref()?;
        let selected = self.cross_block_selection_for_surface(SelectionSurface::Main);
        let drag = self.cross_block_drag_for_surface(SelectionSurface::Main);

        selected
            .into_iter()
            .flat_map(|selection| [selection.focus.entity_id, selection.anchor.entity_id])
            .chain(drag.into_iter().map(|drag| drag.anchor.entity_id))
            .chain(self.active_entity_id)
            .find_map(|entity_id| {
                let entity = virtual_surface.entity_by_id(entity_id)?;
                entity
                    .read_with(cx, |block, _cx| block.focus_handle.is_focused(window))
                    .then_some(entity)
            })
    }

    /// Resolves document-owned nested editors for history only, leaving selection and clipboard
    /// commands bound to the primary text surface and preventing utility fields from borrowing it.
    pub(super) fn focused_document_history_target(
        &self,
        window: &Window,
        cx: &App,
    ) -> Option<(SelectionSurface, Entity<Block>)> {
        if let Some(target) = self.focused_document_target(window, cx) {
            return Some(target);
        }

        for surface in [SelectionSurface::Main, SelectionSurface::SplitPreview] {
            if let Some(entity) =
                self.selection_surface_entities(surface)
                    .into_iter()
                    .find(|entity| {
                        entity.read_with(cx, |block, _cx| {
                            block.math_source_focus_handle.is_focused(window)
                                || block.math_structure_focus_handle.is_focused(window)
                                || block.code_language_focus_handle.is_focused(window)
                        })
                    })
            {
                return Some((surface, entity));
            }
        }
        None
    }

    /// Rejects delayed events from detached blocks and distinguishes Main from Split's projection.
    pub(super) fn selection_surface_for_block_id(
        &self,
        entity_id: EntityId,
    ) -> Option<SelectionSurface> {
        [SelectionSurface::Main, SelectionSurface::SplitPreview]
            .into_iter()
            .find(|surface| {
                if self.selection_surface_contains_entity(*surface, entity_id) {
                    return true;
                }
                let table_cells = match surface {
                    SelectionSurface::Main => Some(&self.table_cells),
                    SelectionSurface::SplitPreview => self
                        .split_preview
                        .as_ref()
                        .map(|preview| &preview.table_cells),
                };
                table_cells.is_some_and(|cells| {
                    cells
                        .values()
                        .any(|binding| binding.cell.entity_id() == entity_id)
                })
            })
    }

    /// Returns the block order belonging to one surface without conflating Split's projections.
    pub(super) fn selection_surface_entities(
        &self,
        surface: SelectionSurface,
    ) -> Vec<Entity<Block>> {
        let visible = match surface {
            SelectionSurface::Main => self.document.visible_blocks(),
            SelectionSurface::SplitPreview => self
                .split_preview
                .as_ref()
                .map(|preview| preview.document.visible_blocks())
                .unwrap_or_default(),
        };
        visible.iter().map(|block| block.entity.clone()).collect()
    }

    /// Reads the selection owned by a surface while keeping Main's established field contract.
    pub(super) fn cross_block_selection_for_surface(
        &self,
        surface: SelectionSurface,
    ) -> Option<CrossBlockSelection> {
        match surface {
            SelectionSurface::Main => self.cross_block_selection,
            SelectionSurface::SplitPreview => self.split_preview_cross_block_selection,
        }
    }

    /// Stores a surface-local selection so clicking the other pane cannot replace its anchor.
    pub(super) fn set_cross_block_selection_for_surface(
        &mut self,
        surface: SelectionSurface,
        selection: Option<CrossBlockSelection>,
    ) {
        match surface {
            SelectionSurface::Main => self.cross_block_selection = selection,
            SelectionSurface::SplitPreview => self.split_preview_cross_block_selection = selection,
        }
    }

    /// Reads the active drag anchor from the same surface as the current pointer event.
    pub(super) fn cross_block_drag_for_surface(
        &self,
        surface: SelectionSurface,
    ) -> Option<CrossBlockDrag> {
        match surface {
            SelectionSurface::Main => self.cross_block_drag,
            SelectionSurface::SplitPreview => self.split_preview_cross_block_drag,
        }
    }

    /// Stores drag ownership separately because both Split panes can remain mounted.
    pub(super) fn set_cross_block_drag_for_surface(
        &mut self,
        surface: SelectionSurface,
        drag: Option<CrossBlockDrag>,
    ) {
        match surface {
            SelectionSurface::Main => self.cross_block_drag = drag,
            SelectionSurface::SplitPreview => self.split_preview_cross_block_drag = drag,
        }
    }

    /// Checks membership against a surface before treating a shared cell binding as its hit target.
    pub(super) fn selection_surface_contains_entity(
        &self,
        surface: SelectionSurface,
        entity_id: EntityId,
    ) -> bool {
        self.selection_surface_entities(surface)
            .iter()
            .any(|entity| entity.entity_id() == entity_id)
    }
}
