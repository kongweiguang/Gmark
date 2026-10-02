// @author kongweiguang

//! Pointer selection and edge scrolling for virtualized Source rows.

use super::source_ime::{DeferredSourceAction, DeferredSourcePointerAction, SourcePointerSnapshot};
use super::*;
use crate::ui::text_editing::word_range_at;

impl DocumentHost {
    /// Ends the pointer session so mode changes and window closure cannot leave a timer running.
    pub(super) fn end_source_pointer_selection(&mut self) {
        self.source_drag_anchor = None;
        self.stop_source_drag_autoscroll();
    }

    /// Replays a click from its captured source anchor so committed IME text cannot shift the hit.
    pub(super) fn replay_source_pointer(
        &mut self,
        snapshot: SourcePointerSnapshot,
        click_count: usize,
        shift: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.document.as_ref().map_or(0, SharedDocument::revision) != snapshot.revision {
            self.end_source_pointer_selection();
            return;
        }
        let line = snapshot.line;
        let Some(block) = self.ensure_source_row_block(line, cx) else {
            self.end_source_pointer_selection();
            return;
        };
        let Some(row) = self.displayed_screen_lines.row(line) else {
            self.end_source_pointer_selection();
            return;
        };
        if row.content_range != snapshot.row_range
            || row.text.as_ref() != snapshot.row_entity.read(cx).display_text()
        {
            self.end_source_pointer_selection();
            return;
        }
        let local = usize::try_from(
            snapshot
                .hit
                .byte_offset
                .saturating_sub(row.content_range.start),
        )
        .unwrap_or_default()
        .min(block.read(cx).display_text().len());
        block.update(cx, |block, cx| {
            block.selected_range = local..local;
            block.selection_reversed = false;
            cx.notify();
        });
        self.activate_source_row_after_pointer(
            line,
            click_count,
            shift,
            snapshot.selection,
            window,
            cx,
        );
    }

    /// Activates a row only after the current Source IME owner has finished its native session.
    pub(super) fn activate_source_row_from_pointer(
        &mut self,
        line: usize,
        event: &gpui::MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.pending_source_ime_action.is_some() || self.has_active_ime_composition(cx) {
            let Some(snapshot) = self.capture_source_pointer_snapshot(line, event.position, cx)
            else {
                self.end_source_pointer_selection();
                self.error = Some("源码视图正在同步，暂时无法开始拖选。".into());
                cx.notify();
                return;
            };
            if self.defer_source_action_for_ime(
                DeferredSourceAction::Pointer(DeferredSourcePointerAction::Down {
                    snapshot: snapshot.clone(),
                    click_count: event.click_count,
                    shift: event.modifiers.shift,
                }),
                window,
                cx,
            ) {
                return;
            }
            self.replay_source_pointer(
                snapshot,
                event.click_count,
                event.modifiers.shift,
                window,
                cx,
            );
            return;
        }
        let previous = self
            .document
            .as_ref()
            .map(SharedDocument::source_selection)
            .unwrap_or_default();
        self.activate_source_row_after_pointer(
            line,
            event.click_count,
            event.modifiers.shift,
            previous,
            window,
            cx,
        );
    }

    /// Applies click count and Shift anchoring without changing the active native target mid-composition.
    pub(super) fn activate_source_row_after_pointer(
        &mut self,
        line: usize,
        click_count: usize,
        shift: bool,
        previous: SourceSelection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.saving || self.reloading {
            return;
        }
        let Some(block) = self.source_row_blocks.get(&line).cloned() else {
            return;
        };
        let Some(row) = self.displayed_screen_lines.row(line).cloned() else {
            return;
        };
        if click_count == 2 {
            block.update(cx, |block, cx| {
                let caret = block.selected_range.end.min(block.display_text().len());
                block.selected_range = word_range_at(block.display_text(), caret);
                block.selection_reversed = false;
                cx.notify();
            });
        }

        let local_selection = source_selection_from_block(block.read(cx), row.content_range.start);
        let selection = if click_count >= 3 {
            self.document
                .as_ref()
                .and_then(|document| document.line_range(line as u64))
                .map(|range| {
                    if !shift {
                        SourceSelection::from_range(range, false)
                    } else if previous.anchor.byte_offset <= range.start {
                        SourceSelection {
                            anchor: previous.anchor,
                            head: SourceAnchor::new(range.end, SourceAffinity::After),
                        }
                    } else {
                        SourceSelection {
                            anchor: previous.anchor,
                            head: SourceAnchor::new(range.start, SourceAffinity::Before),
                        }
                    }
                })
                .unwrap_or(local_selection)
        } else if shift {
            SourceSelection {
                anchor: previous.anchor,
                head: local_selection.head,
            }
        } else {
            local_selection
        };
        self.set_source_selection(selection, cx);
        self.source_drag_anchor = Some(selection.anchor);

        if block.read(cx).is_read_only() || click_count >= 3 {
            self.active_edit = None;
            self.focus_handle.focus(window);
        } else if shift && self.selection_spans_multiple_lines(selection) {
            self.active_edit = None;
            self.focus_handle.focus(window);
        } else {
            let base_revision = self.document.as_ref().map_or(0, SharedDocument::revision);
            self.active_edit = Some(SourceLineEdit {
                line,
                range: row.replace_range,
                base_revision,
                ending: row.ending,
                leading_truncated: row.leading_truncated,
                trailing_truncated: row.trailing_truncated,
                block: block.clone(),
            });
            block.read(cx).focus_handle.focus(window);
        }
        self.sync_source_selection_visuals(cx);
        cx.emit(DocumentHostEvent::StateChanged);
        cx.notify();
    }

    /// Captures source-byte hit data from the currently painted row before IME completion can reflow it.
    fn capture_source_pointer_snapshot(
        &self,
        line: usize,
        position: Point<Pixels>,
        cx: &App,
    ) -> Option<SourcePointerSnapshot> {
        let document = self.document.as_ref()?;
        let row = self.displayed_screen_lines.row(line)?;
        let revision = document.revision();
        if self.displayed_screen_lines.document_revision != revision {
            return None;
        }
        let row_entity = self.source_row_blocks.get(&line)?.clone();
        let local = row_entity
            .read(cx)
            .index_for_mouse_position(position)
            .min(row.text.len());
        Some(SourcePointerSnapshot {
            row_entity,
            line,
            row_range: row.content_range.clone(),
            selection: document.source_selection(),
            hit: SourceAnchor::new(
                row.content_range
                    .start
                    .saturating_add(local as u64)
                    .min(row.content_range.end),
                SourceAffinity::After,
            ),
            revision,
        })
    }

    /// Synchronizes the shared document selection from its active row's local UTF-8 range.
    pub(super) fn sync_selection_from_active_source_block(
        &mut self,
        block: &Entity<Block>,
        cx: &mut Context<Self>,
    ) {
        let Some(active) = self
            .active_edit
            .as_ref()
            .filter(|active| active.block == *block)
        else {
            return;
        };
        let Some(row) = self.displayed_screen_lines.row(active.line) else {
            return;
        };
        let selection = source_selection_from_block(block.read(cx), row.content_range.start);
        self.set_source_selection(selection, cx);
        self.sync_source_selection_visuals(cx);
        cx.emit(DocumentHostEvent::StateChanged);
    }

    /// Maps cross-row dragging to source anchors and scrolls only after the pointer crosses an edge.
    pub(super) fn on_source_surface_mouse_move(
        &mut self,
        event: &gpui::MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !event.dragging() {
            let pointer_is_deferred =
                self.pending_source_ime_action
                    .as_ref()
                    .is_some_and(|pending| {
                        pending
                            .actions
                            .iter()
                            .any(|action| matches!(action, DeferredSourceAction::Pointer(_)))
                    });
            if !pointer_is_deferred {
                self.end_source_pointer_selection();
            }
            return;
        }
        let Some((line, _)) = self.source_block_at_point(event.position, cx) else {
            return;
        };
        let Some(snapshot) = self.capture_source_pointer_snapshot(line, event.position, cx) else {
            return;
        };
        if self.pending_source_ime_action.is_some() || self.has_active_ime_composition(cx) {
            let _ = self.defer_source_action_for_ime(
                DeferredSourceAction::Pointer(DeferredSourcePointerAction::Move {
                    snapshot,
                    position: event.position,
                }),
                window,
                cx,
            );
            return;
        }
        let Some(anchor) = self.source_drag_anchor else {
            return;
        };
        self.apply_source_pointer_move(snapshot, event.position, anchor, window, cx);
    }

    /// Applies a captured endpoint and samples paint only after the visible Source Block selection changes.
    fn apply_source_pointer_move(
        &mut self,
        snapshot: SourcePointerSnapshot,
        position: Point<Pixels>,
        anchor: SourceAnchor,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.document.as_ref().map_or(0, SharedDocument::revision) != snapshot.revision {
            return;
        }
        let line = snapshot.line;
        let Some(block) = self.ensure_source_row_block(line, cx) else {
            return;
        };
        let Some(row) = self.displayed_screen_lines.row(line).cloned() else {
            return;
        };
        if row.content_range != snapshot.row_range
            || row.text.as_ref() != snapshot.row_entity.read(cx).display_text()
        {
            return;
        }
        let before = self
            .document
            .as_ref()
            .map(SharedDocument::source_selection)
            .unwrap_or_default();
        let head = snapshot.hit;
        self.active_edit = None;
        self.focus_handle.focus(window);
        self.set_source_selection(SourceSelection { anchor, head }, cx);
        let after = self
            .document
            .as_ref()
            .map(SharedDocument::source_selection)
            .unwrap_or_default();
        if after != before {
            let range = after.range();
            let visual = (range.start.max(row.content_range.start)
                < range.end.min(row.content_range.end))
            .then(|| {
                usize::try_from(range.start.max(row.content_range.start) - row.content_range.start)
                    .unwrap_or_default()
                    ..usize::try_from(
                        range.end.min(row.content_range.end) - row.content_range.start,
                    )
                    .unwrap_or(row.text.len())
            });
            let draws_selection = visual.is_some();
            block.update(cx, |block, cx| {
                if block.editor_selection_range != visual {
                    block.editor_selection_range = visual;
                    if draws_selection {
                        block.begin_selection_input_trace(cx);
                    }
                    cx.notify();
                }
            });
        }
        self.sync_source_selection_visuals(cx);

        let viewport = self.scroll_handle.0.borrow().base_handle.bounds();
        if position.y <= viewport.top() + px(self.source_row_height * 1.5) {
            self.start_source_drag_autoscroll(-1, cx);
        } else if position.y >= viewport.bottom() - px(self.source_row_height) {
            self.start_source_drag_autoscroll(1, cx);
        } else {
            self.stop_source_drag_autoscroll();
        }
        cx.emit(DocumentHostEvent::StateChanged);
        cx.notify();
    }

    /// Stops edge scrolling immediately when the pointer button is released.
    pub(super) fn on_source_surface_mouse_up(
        &mut self,
        _: &gpui::MouseUpEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.pending_source_ime_action.is_some() || self.has_active_ime_composition(cx) {
            let revision = self.document.as_ref().map_or(0, SharedDocument::revision);
            if self.defer_source_action_for_ime(
                DeferredSourceAction::Pointer(DeferredSourcePointerAction::End { revision }),
                window,
                cx,
            ) {
                return;
            }
        }
        self.end_source_pointer_selection();
    }

    /// Replays deferred gesture edges in FIFO order and clears edge scrolling only at its captured release.
    pub(super) fn replay_source_pointer_action(
        &mut self,
        action: DeferredSourcePointerAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            DeferredSourcePointerAction::Down {
                snapshot,
                click_count,
                shift,
            } => self.replay_source_pointer(snapshot, click_count, shift, window, cx),
            DeferredSourcePointerAction::Move { snapshot, position } => {
                if let Some(anchor) = self.source_drag_anchor {
                    self.apply_source_pointer_move(snapshot, position, anchor, window, cx);
                }
            }
            DeferredSourcePointerAction::End { revision } => {
                if self.document.as_ref().map_or(0, SharedDocument::revision) == revision {
                    self.end_source_pointer_selection();
                }
            }
        }
    }

    /// Starts the single Source edge-scroll task and reuses it while direction stays unchanged.
    pub(super) fn start_source_drag_autoscroll(&mut self, direction: i8, cx: &mut Context<Self>) {
        let direction = direction.signum();
        if direction == 0 || self.source_drag_autoscroll_direction == direction {
            return;
        }
        self.source_drag_autoscroll_direction = direction;
        self.source_drag_autoscroll_task = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(16))
                    .await;
                let keep_running = this
                    .update(cx, |view, cx| view.source_drag_autoscroll_tick(cx))
                    .unwrap_or(false);
                if !keep_running {
                    break;
                }
            }
        });
    }

    /// Stops the timer while retaining the drag anchor so the pointer can return to the viewport.
    fn stop_source_drag_autoscroll(&mut self) {
        self.source_drag_autoscroll_direction = 0;
        self.source_drag_autoscroll_task = Task::ready(());
    }

    /// 到文件边缘时仍完成最后一次扩选，然后停止定时器；保留锚点以支持反向拖回。
    /// 只同步真实变化的选区，边缘帧使用与普通鼠标拖选相同的绘制耗时采样。
    pub(super) fn source_drag_autoscroll_tick(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(anchor) = self.source_drag_anchor else {
            self.source_drag_autoscroll_direction = 0;
            return false;
        };
        let direction = self.source_drag_autoscroll_direction;
        if direction == 0 {
            return false;
        }
        let visible = self.displayed_screen_lines.visible.clone();
        let target_line = if direction < 0 {
            visible.start
        } else {
            visible.end.saturating_sub(1)
        };
        let Some(row) = self.displayed_screen_lines.row(target_line) else {
            return true;
        };
        let head = if direction < 0 {
            SourceAnchor::new(row.content_range.start, SourceAffinity::Before)
        } else {
            SourceAnchor::new(row.content_range.end, SourceAffinity::After)
        };
        self.active_edit = None;
        let selection = SourceSelection { anchor, head };
        let changed = self
            .document
            .as_ref()
            .is_some_and(|document| document.source_selection() != selection);
        if changed {
            self.set_source_selection(selection, cx);
            self.sync_source_selection_visuals_with_drag_trace(true, cx);
        }

        let reached_edge = if direction < 0 {
            visible.start == 0
        } else {
            visible.end >= self.line_count()
        };
        if reached_edge {
            self.source_drag_autoscroll_direction = 0;
            if changed {
                cx.emit(DocumentHostEvent::StateChanged);
                cx.notify();
            }
            return false;
        }

        let next = if direction < 0 {
            visible.start.saturating_sub(1)
        } else {
            visible.end.min(self.line_count().saturating_sub(1))
        };
        self.scroll_source_line_strict(next, ScrollStrategy::Top);
        cx.emit(DocumentHostEvent::StateChanged);
        cx.notify();
        true
    }
}

/// Converts Block-local selection orientation to stable UTF-8 source anchors.
fn source_selection_from_block(block: &Block, source_start: u64) -> SourceSelection {
    let start = SourceAnchor::new(
        source_start.saturating_add(block.selected_range.start as u64),
        SourceAffinity::Before,
    );
    let end = SourceAnchor::new(
        source_start.saturating_add(block.selected_range.end as u64),
        SourceAffinity::After,
    );
    if block.selection_reversed {
        SourceSelection {
            anchor: end,
            head: start,
        }
    } else {
        SourceSelection {
            anchor: start,
            head: end,
        }
    }
}
