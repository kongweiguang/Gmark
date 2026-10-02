// @author kongweiguang

use std::ops::Range;

use super::super::pointer_selection::PointerSelectionSession;
use super::*;
use crate::components::block::BlockImeInteraction;

impl Block {
    /// Preserve the pointer hit before focus or IME completion can rebuild its rich-text projection.
    fn begin_text_pointer_selection(
        &mut self,
        event: &MouseDownEvent,
        offset: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let clean_offset = self.pointer_clean_offset(offset);
        self.is_selecting = true;
        self.request_ime_interaction(
            BlockImeInteraction::PointerSelection {
                clean_offset,
                click_count: event.click_count,
                shift: event.modifiers.shift,
            },
            window,
            cx,
        );
    }

    /// Applies a pointer intent after IME resolution without re-entering the mouse event route.
    pub(crate) fn apply_pointer_selection_interaction(
        &mut self,
        clean_offset: usize,
        click_count: usize,
        shift: bool,
        cx: &mut Context<Self>,
    ) {
        self.is_selecting = true;
        let current_offset = self
            .clean_to_current_range(clean_offset..clean_offset)
            .start;
        if shift {
            self.pointer_selection = None;
            self.select_to(current_offset, cx);
            return;
        }

        let Some(granularity) = self.pointer_selection_granularity_for_click_count(click_count)
        else {
            self.pointer_selection = None;
            self.move_to(current_offset, cx);
            return;
        };
        let range = self.pointer_selection_unit_range(clean_offset, granularity);

        self.pointer_selection = Some(PointerSelectionSession::new(range.clone(), granularity));
        self.apply_pointer_clean_selection(range, false, cx);
    }

    /// Replays a baseline-coordinate drag update after the initial pointer intent established its anchor.
    pub(crate) fn apply_pointer_selection_move_interaction(
        &mut self,
        clean_offset: usize,
        cx: &mut Context<Self>,
    ) {
        if !self.is_selecting {
            return;
        }
        if self.extend_pointer_selection_at_clean_offset(clean_offset, cx) {
            return;
        }
        let current_offset = self
            .clean_to_current_range(clean_offset..clean_offset)
            .start;
        self.select_to(current_offset, cx);
    }

    /// Apply clean-text endpoints through the current inline projection while
    /// retaining which endpoint is the fixed anchor for a reverse drag.
    fn apply_pointer_clean_selection(
        &mut self,
        range: Range<usize>,
        reversed: bool,
        cx: &mut Context<Self>,
    ) {
        let display_range = self.clean_to_current_range(range);
        let (anchor, focus) = if reversed {
            (display_range.end, display_range.start)
        } else {
            (display_range.start, display_range.end)
        };
        self.move_to(anchor, cx);
        if focus != anchor {
            self.select_to(focus, cx);
        }
    }

    /// Recompute a multi-click drag from its original stable clean-text range;
    /// this avoids accumulating off-by-one errors as the pointer crosses units.
    fn extend_pointer_selection(&mut self, offset: usize, cx: &mut Context<Self>) -> bool {
        let clean_offset = self.pointer_clean_offset(offset);
        self.extend_pointer_selection_at_clean_offset(clean_offset, cx)
    }

    /// Extends from a clean baseline offset so deferred IME moves do not reuse candidate bytes.
    fn extend_pointer_selection_at_clean_offset(
        &mut self,
        clean_offset: usize,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(session) = self.pointer_selection.clone() else {
            return false;
        };
        let (range, reversed) = {
            let clean_text = self.render_cache.visible_text();
            session.selection_for_offset(clean_text, clean_offset)
        };
        self.apply_pointer_clean_selection(range, reversed, cx);
        true
    }

    /// Route mouse-down through stable text offsets while leaving explicit
    /// resource and image controls in charge of their own gestures.
    pub(crate) fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 原生表格容器包裹着独立的单元格编辑器；容器不得重复处理从单元格冒泡的点击，
        // 否则会在单元格请求聚焦后立即把焦点抢回表格块。
        if self.kind() == BlockKind::Table && self.table_runtime.is_some() {
            self.is_selecting = false;
            self.pointer_selection = None;
            return;
        }

        if self.kind() == BlockKind::MermaidBlock {
            let source_hit = self.last_bounds.is_some_and(|bounds| {
                event.position.x >= bounds.left()
                    && event.position.x <= bounds.right()
                    && event.position.y >= bounds.top()
                    && event.position.y <= bounds.bottom()
            });
            if self.mermaid_view_mode() == MermaidViewMode::Preview || !source_hit {
                self.is_selecting = false;
                self.pointer_selection = None;
                self.focus_handle.focus(window);
                cx.stop_propagation();
                cx.notify();
                return;
            }
        }

        // Resource cards are rendered inside the same block shell as editable
        // Markdown. Claim their pointer gesture before the text handler can
        // move the caret and switch the projection back to source text.
        let resource_card_visible = self.record.resource.is_some()
            && (!self.focus_handle.is_focused(window) || self.resource_selected);
        if resource_card_visible {
            self.is_selecting = false;
            self.pointer_selection = None;
            self.focus_handle.focus(window);
            if event.click_count >= 2 {
                if let Some(record) = self
                    .record
                    .resource
                    .as_ref()
                    .map(|resource| resource.with_base_dir(self.image_base_dir()))
                {
                    self.request_resource_open(&record, cx);
                }
            } else {
                self.resource_selected = true;
                cx.notify();
            }
            cx.stop_propagation();
            return;
        }

        if self.showing_rendered_image() {
            self.is_selecting = false;
            self.pointer_selection = None;
            if event.click_count >= 2 {
                self.request_image_edit_expansion();
            } else {
                self.select_rendered_image(cx);
            }
            if self.focus_handle.is_focused(window) {
                if self.sync_image_focus_state(true) {
                    cx.notify();
                }
            } else {
                cx.emit(BlockEvent::RequestFocus);
            }
            cx.stop_propagation();
            return;
        }

        let offset = self.index_for_mouse_position(event.position);
        let was_focused = self.focus_handle.is_focused(window);

        // Ctrl+click follows a rendered link; an unmodified double click stays
        // in the text path so the label can be selected as an ordinary word.
        if event.modifiers.secondary() && self.pointer_link_hit(event.position).is_some() {
            self.is_selecting = false;
            self.pointer_selection = None;
            cx.stop_propagation();
            return;
        }

        self.begin_text_pointer_selection(event, offset, window, cx);
        if !was_focused && !self.ime_pointer_selection_pending() {
            cx.emit(BlockEvent::RequestFocus);
        }
    }

    /// Use the same text selection granularity in Preview while keeping the
    /// block read-only and allowing only an explicit Ctrl+click to follow links.
    pub(crate) fn on_read_only_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.modifiers.secondary() && self.pointer_link_hit(event.position).is_some() {
            self.is_selecting = false;
            self.pointer_selection = None;
            cx.stop_propagation();
            return;
        }

        let offset = self.index_for_mouse_position(event.position);
        self.focus_handle.focus(window);
        self.begin_text_pointer_selection(event, offset, window, cx);
    }

    /// End Preview's pointer session and follow a link only when the initiating
    /// pointer gesture was an explicit secondary click.
    pub(crate) fn on_read_only_mouse_up(
        &mut self,
        event: &MouseUpEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.ime_pointer_selection_pending() {
            self.request_ime_interaction(BlockImeInteraction::PointerSelectionEnd, window, cx);
            return;
        }
        let was_selecting = self.is_selecting;
        self.is_selecting = false;
        self.pointer_selection = None;
        if !was_selecting
            && event.modifiers.secondary()
            && let Some(link) = self.pointer_link_hit(event.position)
        {
            self.open_rendered_link(&link, cx);
        }
    }

    /// Resolve an inline link from the last stable layout so menu actions cannot use stale hit data.
    pub(crate) fn pointer_link_hit(&self, position: Point<Pixels>) -> Option<super::InlineLinkHit> {
        self.last_layout
            .as_ref()
            .zip(self.last_bounds)
            .and_then(|(lines, bounds)| {
                super::element::link_at_position(
                    self,
                    lines,
                    bounds,
                    self.last_line_height,
                    position,
                )
            })
            .cloned()
    }

    /// Handle mouse-down on a rendered inline link (in a mixed inline-visual
    /// block). A Cmd/Ctrl+click is claimed here so it follows the link instead
    /// of focusing the block; the destination opens on the matching mouse-up.
    pub(crate) fn on_rendered_link_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Only Cmd/Ctrl+click follows the link; a plain click falls through so
        // the block focuses for editing like any other inline text.
        if event.modifiers.secondary() {
            cx.stop_propagation();
        }
    }

    /// Open a rendered inline link's destination through the editor prompt.
    pub(crate) fn open_rendered_link(
        &mut self,
        link: &super::InlineLinkHit,
        cx: &mut Context<Self>,
    ) {
        cx.stop_propagation();
        cx.emit(BlockEvent::RequestOpenLink {
            prompt_target: link.prompt_target.clone(),
            open_target: link.open_target.clone(),
        });
    }

    /// Finish block text selection and preserve explicit image, footnote, and Ctrl+click gestures.
    pub(crate) fn on_mouse_up(
        &mut self,
        event: &MouseUpEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.finish_image_resize(cx) {
            cx.stop_propagation();
            return;
        }
        if self.ime_pointer_selection_pending() {
            self.request_ime_interaction(BlockImeInteraction::PointerSelectionEnd, window, cx);
            return;
        }
        self.is_selecting = false;
        self.pointer_selection = None;

        // Only an explicit Ctrl+click follows a rendered link. Plain double
        // clicks are reserved for selecting the word under the pointer.
        if event.modifiers.secondary()
            && let Some(link) = self.pointer_link_hit(event.position)
        {
            self.open_rendered_link(&link, cx);
            return;
        }

        if event.click_count >= 2 {
            let footnote = self
                .last_layout
                .as_ref()
                .zip(self.last_bounds)
                .and_then(|(lines, bounds)| {
                    super::element::footnote_at_position(
                        self,
                        lines,
                        bounds,
                        self.last_line_height,
                        event.position,
                    )
                })
                .cloned();
            if let Some(footnote) = footnote {
                cx.stop_propagation();
                cx.emit(BlockEvent::RequestJumpToFootnoteDefinition { id: footnote.id });
            }
        }
    }

    /// Keep footnote back references out of text selection and route them to their definition.
    pub(crate) fn on_footnote_backref_mouse_down(
        &mut self,
        _: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.stop_propagation();
        if !self.focus_handle.is_focused(window) {
            cx.emit(BlockEvent::RequestFocus);
        }
    }

    /// Return a footnote back-reference click to the source that owns the definition.
    pub(crate) fn on_footnote_backref_mouse_up(
        &mut self,
        _: &MouseUpEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = self.footnote_definition_id() else {
            return;
        };
        cx.stop_propagation();
        cx.emit(BlockEvent::RequestJumpToFootnoteBackref { id });
    }

    /// Extend pointer selections only during an active drag and report the input-to-paint sample.
    pub(crate) fn on_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.image_resize_session.is_some() {
            if event.dragging() {
                self.update_image_resize(event.position.x, cx);
            } else {
                self.finish_image_resize(cx);
            }
            cx.stop_propagation();
            return;
        }
        if self.ime_pointer_selection_pending() {
            if event.dragging() {
                let offset = self.index_for_mouse_position(event.position);
                let clean_offset = self.pointer_clean_offset(offset);
                self.request_ime_interaction(
                    BlockImeInteraction::PointerSelectionMove { clean_offset },
                    window,
                    cx,
                );
            } else {
                self.request_ime_interaction(BlockImeInteraction::PointerSelectionEnd, window, cx);
            }
            return;
        }
        if self.is_selecting {
            // A stale selecting flag can survive a missed mouse-up. Only extend
            // the selection while the platform still reports an active drag.
            if !event.dragging() {
                self.is_selecting = false;
                self.pointer_selection = None;
                cx.notify();
                return;
            }
            let previous_range = self.selected_range.clone();
            let previous_direction = self.selection_reversed;
            let offset = self.index_for_mouse_position(event.position);
            if !self.extend_pointer_selection(offset, cx) {
                self.select_to(offset, cx);
            }
            if self.selected_range != previous_range
                || self.selection_reversed != previous_direction
            {
                self.begin_selection_input_trace(cx);
            }
        }
    }
}
