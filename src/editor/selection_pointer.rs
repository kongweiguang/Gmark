// @author kongweiguang

use super::*;
use crate::editor::ime_lifecycle;

impl Editor {
    /// Preserves the focused local anchor for Shift-click and uses the hit point for a fresh drag.
    pub(super) fn begin_surface_cross_block_drag_at_point(
        &mut self,
        surface: SelectionSurface,
        position: Point<Pixels>,
        extend: bool,
        window: Option<&Window>,
        cx: &mut Context<Self>,
    ) {
        self.active_selection_surface = surface;
        let existing = self.cross_block_selection_for_surface(surface);
        let anchor = if extend {
            existing
                .map(|selection| selection.anchor)
                .or_else(|| {
                    window.and_then(|window| {
                        self.focused_local_selection_anchor_for_surface(surface, window, cx)
                    })
                })
                .or_else(|| self.cross_block_endpoint_for_surface(position, surface, cx))
        } else {
            self.cross_block_endpoint_for_surface(position, surface, cx)
        };
        let mut changed = false;
        if !extend {
            changed = existing.is_some();
            self.set_cross_block_selection_for_surface(surface, None);
            changed |= self.clear_cross_block_selection_visuals_for_surface(surface, cx);
        }
        let drag = anchor.map(|anchor| self.cross_block_drag_from_endpoint(surface, anchor, cx));
        self.set_cross_block_drag_for_surface(surface, drag);
        if changed {
            cx.notify();
        }
    }

    /// Captures the document coordinate before virtual viewport changes can release the hit entity.
    pub(in crate::editor) fn cross_block_drag_from_endpoint(
        &self,
        surface: SelectionSurface,
        anchor: CrossBlockSelectionEndpoint,
        cx: &App,
    ) -> CrossBlockDrag {
        CrossBlockDrag {
            anchor,
            source_anchor: self.cross_block_source_anchor_for_endpoint(surface, anchor, cx),
        }
    }

    /// Reads only the previously focused document Block so Shift-click cannot inherit a stale selection from the hit target.
    fn focused_local_selection_anchor_for_surface(
        &self,
        surface: SelectionSurface,
        window: &Window,
        cx: &App,
    ) -> Option<CrossBlockSelectionEndpoint> {
        let entity = self
            .selection_surface_entities(surface)
            .into_iter()
            .find(|entity| entity.read(cx).focus_handle.is_focused(window))?;
        let block = entity.read(cx);
        let clean_offset = block.pointer_selection_clean_anchor();
        let current_offset = block
            .clean_to_current_range(clean_offset..clean_offset)
            .start;
        Some(CrossBlockSelectionEndpoint {
            entity_id: entity.entity_id(),
            offset: current_offset,
        })
    }

    /// Starts Main-surface selection before the focused Block handles caret placement.
    pub(in crate::editor) fn on_editor_capture_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.button != MouseButton::Left {
            cx.propagate();
            return;
        }
        self.begin_surface_pointer_selection(SelectionSurface::Main, event, window, cx);
        cx.propagate();
    }

    /// Gives Split Preview its own capture target because its blocks belong to a separate projection.
    pub(in crate::editor) fn on_split_preview_capture_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.button != MouseButton::Left || self.view_mode != ViewMode::Split {
            cx.propagate();
            return;
        }
        self.begin_surface_pointer_selection(SelectionSurface::SplitPreview, event, window, cx);
        cx.propagate();
    }

    /// Starts a pane-local text or table gesture while allowing the target Block to place its caret.
    fn begin_surface_pointer_selection(
        &mut self,
        surface: SelectionSurface,
        event: &MouseDownEvent,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        self.active_selection_surface = surface;
        self.rendered_select_all_cycle = None;
        let can_select_cross_block = match surface {
            SelectionSurface::Main => {
                matches!(self.view_mode, ViewMode::Rendered | ViewMode::Preview)
            }
            SelectionSurface::SplitPreview => self.view_mode == ViewMode::Split,
        };
        if !can_select_cross_block {
            self.cancel_selection_autoscroll();
            return;
        }

        if let Some((table_block_id, position)) =
            self.table_cell_at_point_for_surface(surface, event.position, cx)
        {
            self.set_cross_block_drag_for_surface(surface, None);
            self.set_table_cell_drag_anchor_for_surface(surface, None);
            if event.modifiers.shift
                && let Some(selection) = self
                    .table_cell_rectangle_for_surface(surface)
                    .filter(|selection| selection.table_block_id == table_block_id)
            {
                self.set_table_cell_rectangle_for_surface(
                    surface,
                    Some(crate::editor::table_selection::TableCellRectangle {
                        table_block_id,
                        anchor: selection.anchor,
                        focus: position,
                    }),
                );
                self.clear_cross_block_selection_for_surface(surface, cx);
                self.sync_table_cell_rectangle_highlights_for(surface, cx);
                cx.stop_propagation();
                return;
            }
            if self.table_cell_rectangle_for_surface(surface).is_some() {
                self.set_table_cell_rectangle_for_surface(surface, None);
                self.sync_table_cell_rectangle_highlights_for(surface, cx);
            }
            self.clear_cross_block_selection_for_surface(surface, cx);
            cx.propagate();
            return;
        }

        if self.table_cell_rectangle_for_surface(surface).is_some() {
            self.set_table_cell_rectangle_for_surface(surface, None);
            self.sync_table_cell_rectangle_highlights_for(surface, cx);
        }
        self.set_table_cell_drag_anchor_for_surface(surface, None);
        if self.defer_surface_pointer_selection_begin(surface, event, window, cx) {
            return;
        }
        self.begin_surface_cross_block_drag_at_point(
            surface,
            event.position,
            event.modifiers.shift,
            Some(window),
            cx,
        );
    }

    /// Opens text commands after child controls have had a chance to claim the right-click.
    pub(in crate::editor) fn on_text_context_menu_mouse_down_for_surface(
        &mut self,
        surface: SelectionSurface,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.button != MouseButton::Right
            || (surface == SelectionSurface::SplitPreview && self.view_mode != ViewMode::Split)
        {
            cx.propagate();
            return;
        }

        self.active_selection_surface = surface;
        let entity_id = self.text_context_target_for_surface(surface, event.position, cx);
        if entity_id.is_some_and(|entity_id| {
            self.open_text_context_menu_for_surface(surface, entity_id, event.position, window, cx)
        }) {
            cx.stop_propagation();
            return;
        }
        cx.propagate();
    }

    /// Resolves a rendered text block or the concrete table-cell Block under a pointer.
    fn text_context_target_for_surface(
        &self,
        surface: SelectionSurface,
        position: Point<Pixels>,
        cx: &App,
    ) -> Option<EntityId> {
        if let Some((table_block_id, cell_position)) =
            self.table_cell_at_point_for_surface(surface, position, cx)
        {
            let bindings = match surface {
                SelectionSurface::Main => Some(&self.table_cells),
                SelectionSurface::SplitPreview => self
                    .split_preview
                    .as_ref()
                    .map(|preview| &preview.table_cells),
            }?;
            return bindings
                .values()
                .find(|binding| {
                    binding.table_block.entity_id() == table_block_id
                        && binding.position == cell_position
                })
                .map(|binding| binding.cell.entity_id());
        }

        self.cross_block_endpoint_for_surface(position, surface, cx)
            .map(|endpoint| endpoint.entity_id)
    }

    /// Routes Main-surface drag movement into its projection and edge-scroll task.
    pub(in crate::editor) fn on_editor_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.on_surface_pointer_move(SelectionSurface::Main, event, cx);
    }

    /// Updates right-pane drag selection without consulting the left document's block index.
    pub(in crate::editor) fn on_split_preview_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.on_surface_pointer_move(SelectionSurface::SplitPreview, event, cx);
    }

    /// Keeps edge scrolling alive while the pointer is stationary and extends the matching surface.
    fn on_surface_pointer_move(
        &mut self,
        surface: SelectionSurface,
        event: &MouseMoveEvent,
        cx: &mut Context<Self>,
    ) {
        if !event.dragging() {
            return;
        }
        self.active_selection_surface = surface;
        self.selection_autoscroll_pointer = Some(event.position);
        self.update_or_defer_surface_pointer_selection(surface, event.position, cx);
        if self.pointer_near_selection_scroll_edge(surface, event.position, cx) {
            self.start_selection_autoscroll(cx);
        } else {
            self.cancel_selection_autoscroll();
        }
    }

    /// Applies one drag update using the surface's table or text anchor and preserves Block multi-click units.
    pub(super) fn update_surface_pointer_selection(
        &mut self,
        surface: SelectionSurface,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        if let Some((table_block_id, anchor)) = self.table_cell_drag_anchor_for_surface(surface) {
            let Some((focus_table_id, focus)) =
                self.table_cell_at_point_for_surface(surface, position, cx)
            else {
                return;
            };
            if focus_table_id != table_block_id {
                return;
            }
            self.set_table_cell_rectangle_for_surface(
                surface,
                Some(crate::editor::table_selection::TableCellRectangle {
                    table_block_id,
                    anchor,
                    focus,
                }),
            );
            self.clear_cross_block_selection_for_surface(surface, cx);
            self.sync_table_cell_rectangle_highlights_for(surface, cx);
            cx.notify();
            return;
        }
        let Some(focus) = self.cross_block_endpoint_for_surface(position, surface, cx) else {
            return;
        };
        self.apply_surface_pointer_selection_to_endpoint(surface, focus, cx);
    }

    /// Defers a movement while its gesture is queued so a resolved composition cannot be overwritten by stale hit testing.
    pub(super) fn update_or_defer_surface_pointer_selection(
        &mut self,
        surface: SelectionSurface,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        if let Some((tab, owner)) = self.pending_surface_pointer_owner(surface) {
            if let Some(focus) = self.capture_surface_pointer_endpoint(
                surface,
                position,
                self.source_document.revision(),
                cx,
            ) {
                self.queue_ime_operation(
                    ime_lifecycle::DeferredImeOperation::SurfacePointerSelection {
                        tab,
                        surface,
                        owner,
                        interaction: ime_lifecycle::DeferredSurfacePointerInteraction::Move {
                            focus,
                        },
                    },
                    cx,
                );
            }
            return;
        }
        self.update_surface_pointer_selection(surface, position, cx);
    }

    /// Applies a focus endpoint only while its source anchor and virtual projection match the document revision.
    pub(in crate::editor) fn apply_surface_pointer_selection_to_endpoint(
        &mut self,
        surface: SelectionSurface,
        focus: CrossBlockSelectionEndpoint,
        cx: &mut Context<Self>,
    ) {
        let Some(drag) = self.cross_block_drag_for_surface(surface) else {
            return;
        };
        let current_revision = self.source_document.snapshot().revision();
        let stale_virtual_projection = surface == SelectionSurface::Main
            && self
                .virtual_surface
                .as_ref()
                .is_some_and(|virtual_surface| {
                    virtual_surface.projection_revision() != current_revision
                });
        let stale_source_anchor = drag
            .source_anchor
            .is_some_and(|source_anchor| source_anchor.revision != current_revision);
        if stale_virtual_projection || stale_source_anchor {
            self.set_cross_block_drag_for_surface(surface, None);
            self.set_cross_block_selection_for_surface(surface, None);
            self.clear_cross_block_selection_visuals_for_surface(surface, cx);
            self.show_pane_notice("文档已变化，选区已重置，请重新选择", cx);
            return;
        }
        let visible = self.selection_surface_entities(surface);
        let anchor_index = visible
            .iter()
            .position(|entity| entity.entity_id() == drag.anchor.entity_id);
        let focus_index = visible
            .iter()
            .position(|entity| entity.entity_id() == focus.entity_id);
        let Some(anchor_entity) =
            self.cross_block_selection_entity_for_surface(drag.anchor.entity_id, surface)
        else {
            return;
        };
        let Some(focus_entity) =
            self.cross_block_selection_entity_for_surface(focus.entity_id, surface)
        else {
            return;
        };
        let previous_selection = self.cross_block_selection_for_surface(surface);
        let existing_selection = previous_selection.is_some();
        let pointer_session = anchor_entity.read(cx).pointer_selection_session();
        let source_focus = self.cross_block_source_anchor_for_endpoint(surface, focus, cx);
        let reversed = match (drag.source_anchor, source_focus) {
            (Some(anchor), Some(focus)) => focus.byte_offset < anchor.byte_offset,
            _ => match (anchor_index, focus_index) {
                (Some(anchor), Some(focus)) => focus < anchor,
                _ => return,
            },
        };

        // A Block already owns an in-block gesture; its stable session keeps word and line units intact.
        if drag.anchor.entity_id == focus.entity_id && !existing_selection {
            return;
        }
        if drag.anchor.entity_id == focus.entity_id && pointer_session.is_some() {
            self.set_cross_block_selection_for_surface(surface, None);
            self.sync_cross_block_selection_visuals_for_surface(surface, cx);
            if previous_selection.is_some() {
                focus_entity.update(cx, |block, cx| block.begin_selection_input_trace(cx));
            }
            cx.notify();
            return;
        }

        let selection = if let Some(session) = pointer_session {
            let anchor_range = anchor_entity
                .read(cx)
                .clean_to_current_range(session.anchor_range);
            let (focus_offset, anchor_offset) = if reversed {
                let clean_focus = focus_entity.read(cx).current_to_clean_offset(focus.offset);
                let focus_range = focus_entity
                    .read(cx)
                    .pointer_selection_unit_range(clean_focus, session.granularity);
                let focus_range = focus_entity.read(cx).clean_to_current_range(focus_range);
                (focus_range.start, anchor_range.end)
            } else {
                let clean_focus = focus_entity.read(cx).current_to_clean_offset(focus.offset);
                let focus_range = focus_entity
                    .read(cx)
                    .pointer_selection_unit_range(clean_focus, session.granularity);
                let focus_range = focus_entity.read(cx).clean_to_current_range(focus_range);
                (focus_range.end, anchor_range.start)
            };
            self.cross_block_selection_from_endpoints(
                surface,
                CrossBlockSelectionEndpoint {
                    entity_id: drag.anchor.entity_id,
                    offset: anchor_offset,
                },
                CrossBlockSelectionEndpoint {
                    entity_id: focus.entity_id,
                    offset: focus_offset,
                },
                drag.source_anchor,
                cx,
            )
        } else {
            self.cross_block_selection_from_endpoints(
                surface,
                drag.anchor,
                focus,
                drag.source_anchor,
                cx,
            )
        };
        if self.cross_block_selection_is_empty_for_surface(selection, surface) {
            self.set_cross_block_selection_for_surface(surface, None);
        } else {
            self.set_cross_block_selection_for_surface(surface, Some(selection));
        }
        self.sync_cross_block_selection_visuals_for_surface(surface, cx);
        if self.cross_block_selection_for_surface(surface) != previous_selection {
            focus_entity.update(cx, |block, cx| block.begin_selection_input_trace(cx));
        }
        cx.notify();
    }

    /// Captures the focused composition owner so parent-level drags join the same terminal queue as Block input.
    fn active_surface_ime_owner(&self, window: &Window, cx: &App) -> Option<Entity<Block>> {
        if let Some((_, block)) = self.focused_document_target(window, cx)
            && block.read(cx).has_ime_composition()
        {
            return Some(block);
        }

        self.ime_detached_targets
            .iter()
            .find(|block| block.read(cx).has_ime_composition())
            .cloned()
            .or_else(|| {
                [SelectionSurface::Main, SelectionSurface::SplitPreview]
                    .into_iter()
                    .flat_map(|surface| self.selection_surface_entities(surface))
                    .find(|block| block.read(cx).has_ime_composition())
            })
            .or_else(|| {
                self.table_cells
                    .values()
                    .find(|binding| binding.cell.read(cx).has_ime_composition())
                    .map(|binding| binding.cell.clone())
            })
            .or_else(|| {
                self.split_preview
                    .as_ref()?
                    .table_cells
                    .values()
                    .find(|binding| binding.cell.read(cx).has_ime_composition())
                    .map(|binding| binding.cell.clone())
            })
    }

    /// Resolves a pointer hit once and records a clean offset plus source revision for terminal replay.
    fn capture_surface_pointer_endpoint(
        &self,
        surface: SelectionSurface,
        position: Point<Pixels>,
        revision: gmark_document::Revision,
        cx: &App,
    ) -> Option<ime_lifecycle::DeferredSurfacePointerEndpoint> {
        let endpoint = self.cross_block_endpoint_for_surface(position, surface, cx)?;
        self.capture_surface_pointer_endpoint_from_endpoint(surface, endpoint, revision, cx)
    }

    /// Converts a visible Block endpoint to its stable clean-text coordinate without retaining layout geometry.
    fn capture_surface_pointer_endpoint_from_endpoint(
        &self,
        surface: SelectionSurface,
        endpoint: CrossBlockSelectionEndpoint,
        revision: gmark_document::Revision,
        cx: &App,
    ) -> Option<ime_lifecycle::DeferredSurfacePointerEndpoint> {
        let block = self
            .selection_surface_entities(surface)
            .into_iter()
            .find(|block| block.entity_id() == endpoint.entity_id)?;
        let clean_offset = block.read(cx).pointer_clean_offset(endpoint.offset);
        Some(ime_lifecycle::DeferredSurfacePointerEndpoint::new(
            endpoint.entity_id,
            clean_offset,
            revision,
        ))
    }

    /// Queues the down before the child Block can change focus or its projection during active preedit.
    fn defer_surface_pointer_selection_begin(
        &mut self,
        surface: SelectionSurface,
        event: &MouseDownEvent,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(owner) = self.active_surface_ime_owner(window, cx) else {
            return false;
        };
        let revision = self.source_document.revision();
        let Some(hit) =
            self.capture_surface_pointer_endpoint(surface, event.position, revision, cx)
        else {
            return false;
        };
        let target_entity_id = hit.entity_id;
        let extend = event.modifiers.shift;
        let existing = self.cross_block_selection_for_surface(surface);
        let anchor = if extend {
            existing
                .and_then(|selection| {
                    self.capture_surface_pointer_endpoint_from_endpoint(
                        surface,
                        selection.anchor,
                        revision,
                        cx,
                    )
                })
                .or_else(|| {
                    self.focused_local_selection_anchor_for_surface(surface, window, cx)
                        .and_then(|anchor| {
                            self.capture_surface_pointer_endpoint_from_endpoint(
                                surface, anchor, revision, cx,
                            )
                        })
                })
                .unwrap_or(hit)
        } else {
            hit
        };

        owner.update(cx, |block, _cx| {
            block.set_ime_surface_selection_pending(true)
        });
        self.set_cross_block_drag_for_surface(surface, None);
        self.queue_ime_operation(
            ime_lifecycle::DeferredImeOperation::SurfacePointerSelection {
                tab: self.tabs.records.get(self.tabs.active).map(|tab| tab.id),
                surface,
                owner,
                interaction: ime_lifecycle::DeferredSurfacePointerInteraction::Begin {
                    anchor,
                    target_entity_id,
                    extend,
                },
            },
            cx,
        );
        true
    }

    /// Ends Main-surface pointer ownership even when release occurs outside the document.
    pub(in crate::editor) fn on_editor_mouse_up(
        &mut self,
        _event: &MouseUpEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.end_surface_pointer_selection(SelectionSurface::Main, cx);
    }

    /// Ends right-pane drag ownership and auto-scroll on both inside and outside releases.
    pub(in crate::editor) fn on_split_preview_mouse_up(
        &mut self,
        _event: &MouseUpEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.end_surface_pointer_selection(SelectionSurface::SplitPreview, cx);
    }

    /// Cancels pointer state on release or focus loss while leaving completed selections intact.
    pub(super) fn end_surface_pointer_selection(
        &mut self,
        surface: SelectionSurface,
        cx: &mut Context<Self>,
    ) {
        let deferred_owner = self.pending_surface_pointer_owner(surface);
        let deferred_end_is_queued = self.surface_pointer_selection_end_is_queued(surface);
        if let Some((tab, owner)) = deferred_owner.as_ref() {
            self.queue_ime_operation(
                ime_lifecycle::DeferredImeOperation::SurfacePointerSelection {
                    tab: *tab,
                    surface,
                    owner: owner.clone(),
                    interaction: ime_lifecycle::DeferredSurfacePointerInteraction::End,
                },
                cx,
            );
        }
        self.set_cross_block_drag_for_surface(surface, None);
        self.set_table_cell_drag_anchor_for_surface(surface, None);
        self.cancel_selection_autoscroll();
        if deferred_owner.is_none() && !deferred_end_is_queued {
            self.end_block_pointer_selection_sessions(cx);
        }
    }
}
