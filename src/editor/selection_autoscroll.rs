// @author kongweiguang

use std::time::Duration;

use gpui::*;

use super::Editor;
use crate::editor::selection_surface::SelectionSurface;

const SELECTION_EDGE_MARGIN: f32 = 28.0;
const SELECTION_MAX_SCROLL_PER_TICK: f32 = 22.0;
const SELECTION_SCROLL_TICK: Duration = Duration::from_millis(16);

impl Editor {
    /// Uses each pane's own scroll owner so overlapping Split coordinates never scroll the wrong side.
    fn selection_scroll_handle(&self, surface: SelectionSurface) -> Option<ScrollHandle> {
        match surface {
            SelectionSurface::Main => Some(self.scroll_handle.clone()),
            SelectionSurface::SplitPreview => self
                .split_preview
                .as_ref()
                .map(|preview| preview.scroll_handle.clone()),
        }
    }

    /// Computes a bounded edge velocity only while a text or table selection gesture is active.
    fn selection_scroll_velocity(
        &self,
        surface: SelectionSurface,
        pointer: Point<Pixels>,
    ) -> Option<f32> {
        let active_drag = self.cross_block_drag_for_surface(surface).is_some()
            || self.table_cell_drag_anchor_for_surface(surface).is_some()
            || self.pending_surface_pointer_owner(surface).is_some();
        if !active_drag {
            return None;
        }
        let bounds = self.selection_scroll_handle(surface)?.bounds();
        if f32::from(bounds.size.height) <= 1.0 {
            return None;
        }
        let margin = px(SELECTION_EDGE_MARGIN);
        if pointer.y < bounds.top() + margin {
            let distance = f32::from(bounds.top() + margin - pointer.y);
            Some(distance.clamp(2.0, SELECTION_MAX_SCROLL_PER_TICK))
        } else if pointer.y > bounds.bottom() - margin {
            let distance = f32::from(pointer.y - (bounds.bottom() - margin));
            Some(-distance.clamp(2.0, SELECTION_MAX_SCROLL_PER_TICK))
        } else {
            None
        }
    }

    /// Keeps the repeated scroll task dormant unless the pointer enters a viewport edge zone.
    pub(super) fn pointer_near_selection_scroll_edge(
        &self,
        surface: SelectionSurface,
        pointer: Point<Pixels>,
        _cx: &App,
    ) -> bool {
        self.selection_scroll_velocity(surface, pointer).is_some()
    }

    /// Starts one pane-bound timer; a stationary pointer can then extend a drag as rows move.
    pub(super) fn start_selection_autoscroll(&mut self, cx: &mut Context<Self>) {
        if self.selection_autoscroll_task.is_some()
            && self.selection_autoscroll_surface == self.active_selection_surface
        {
            return;
        }
        self.selection_autoscroll_generation = self.selection_autoscroll_generation.wrapping_add(1);
        let generation = self.selection_autoscroll_generation;
        self.selection_autoscroll_task = None;
        self.selection_autoscroll_surface = self.active_selection_surface;
        let this_surface = self.selection_autoscroll_surface;
        self.selection_autoscroll_task = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(SELECTION_SCROLL_TICK).await;
                let keep_scrolling = this
                    .update(cx, |editor, cx| {
                        let keep_scrolling = editor.advance_selection_autoscroll(this_surface, cx);
                        if !keep_scrolling
                            && editor.selection_autoscroll_generation == generation
                            && editor.selection_autoscroll_surface == this_surface
                        {
                            editor.selection_autoscroll_task = None;
                        }
                        keep_scrolling
                    })
                    .unwrap_or(false);
                if !keep_scrolling {
                    break;
                }
            }
        }));
    }

    /// Applies one clamped scroll step and re-hit-tests the unchanged pointer in its original pane.
    fn advance_selection_autoscroll(
        &mut self,
        surface: SelectionSurface,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(pointer) = self.selection_autoscroll_pointer else {
            return false;
        };
        let Some(delta) = self.selection_scroll_velocity(surface, pointer) else {
            return false;
        };
        let Some(handle) = self.selection_scroll_handle(surface) else {
            return false;
        };
        let old_offset = handle.offset();
        let maximum = f32::from(handle.max_offset().height.max(px(0.0)));
        let next_y = (f32::from(old_offset.y) + delta).clamp(-maximum, 0.0);
        if (next_y - f32::from(old_offset.y)).abs() <= 0.1 {
            return false;
        }
        handle.set_offset(point(old_offset.x, px(next_y)));
        self.update_or_defer_surface_pointer_selection(surface, pointer, cx);
        cx.notify();
        true
    }

    /// Cancels edge scrolling on release or pane changes and releases the retained task handle.
    pub(in crate::editor) fn cancel_selection_autoscroll(&mut self) {
        self.selection_autoscroll_generation = self.selection_autoscroll_generation.wrapping_add(1);
        self.selection_autoscroll_task = None;
        self.selection_autoscroll_pointer = None;
    }
}
