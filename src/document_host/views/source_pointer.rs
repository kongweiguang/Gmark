// @author kongweiguang

//! Pointer selection and edge scrolling for virtualized Source rows.

use super::source_boundaries::bounded_word_range;
use super::source_ime::{DeferredSourceAction, DeferredSourcePointerAction, SourcePointerSnapshot};
use super::*;

/// 按下单位始终完整保留，反向拖动只改变方向而不丢失最初选中的词。
pub(super) fn source_word_drag_selection(
    anchor: Range<u64>,
    target: Range<u64>,
) -> SourceSelection {
    if target.end <= anchor.start {
        SourceSelection::from_range(target.start..anchor.end, true)
    } else if target.start >= anchor.end {
        SourceSelection::from_range(anchor.start..target.end, false)
    } else {
        SourceSelection::from_range(anchor, false)
    }
}

/// Keeps a multi-click selection at its original unit while the pointer crosses text or rows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum SourceDragGranularity {
    #[default]
    Character,
    Word,
    Line,
}

impl DocumentHost {
    /// Synthesizes a multi-click activation while the following movement still travels through the real GPUI mouse path.
    #[cfg(test)]
    pub(crate) fn activate_source_pointer_for_test(
        &mut self,
        line: usize,
        position: Point<Pixels>,
        click_count: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(block) = self.ensure_source_row_block(line, cx) else {
            return;
        };
        let Some(row) = self.current_source_pointer_row(line, cx) else {
            return;
        };
        let previous = self
            .document
            .as_ref()
            .map(SharedDocument::source_selection)
            .unwrap_or_default();
        let local = block
            .read(cx)
            .index_for_mouse_position(position)
            .min(row.text.len());
        block.update(cx, |block, cx| {
            block.selected_range = local..local;
            block.selection_reversed = false;
            cx.notify();
        });
        self.activate_source_row_after_pointer(line, click_count, false, previous, window, cx);
    }

    /// Ends the pointer session and drops its captured unit so later gestures cannot inherit it.
    pub(super) fn end_source_pointer_selection(&mut self) {
        self.source_drag_anchor = None;
        self.source_drag_anchor_range = None;
        self.source_drag_granularity = SourceDragGranularity::Character;
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
        let Some(row) = self.current_source_pointer_row(line, cx) else {
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
        if self.pending_source_ime_action.is_some()
            || self.coordinator.source_boundary_cancellation.is_some()
            || self.has_active_ime_composition(cx)
        {
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

    /// 多击按真实源码选择完整单位；截断窗口只提供命中锚点，不决定完整词或字素范围。
    pub(super) fn activate_source_row_after_pointer(
        &mut self,
        line: usize,
        click_count: usize,
        shift: bool,
        previous: SourceSelection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.reloading {
            return;
        }
        let Some(block) = self.source_row_blocks.get(&line).cloned() else {
            return;
        };
        let Some(row) = self.current_source_pointer_row(line, cx).cloned() else {
            return;
        };
        let local_selection =
            Self::source_selection_from_block(block.read(cx), row.content_range.start);
        let word = if click_count == 2 && !shift {
            let hit = local_selection.range().start;
            if let Some(range) = bounded_word_range(&row, hit) {
                Some(SourceSelection::from_range(range, false))
            } else if let Some(document) = self.document.clone()
                && let Some(line_range) = document.line_range(line as u64)
            {
                // 未知范围保持精确命中，后续词拖选可以同时解析按下点和目标点。
                let caret = SourceSelection::collapsed(hit, SourceAffinity::After);
                self.install_source_pointer_selection(line, click_count, shift, caret, window, cx);
                self.request_source_word_click(
                    line,
                    click_count,
                    shift,
                    hit,
                    line_range,
                    window,
                    cx,
                );
                return;
            } else {
                None
            }
        } else {
            None
        };
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
            word.unwrap_or(local_selection)
        };
        self.install_source_pointer_selection(line, click_count, shift, selection, window, cx);
    }

    /// 同步与后台选词共用焦点、拖选单位及通知出口，鼠标释放后不能重新启动拖选任务。
    pub(super) fn install_source_pointer_selection(
        &mut self,
        line: usize,
        click_count: usize,
        shift: bool,
        selection: SourceSelection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(block) = self.source_row_blocks.get(&line).cloned() else {
            return;
        };
        self.set_source_selection(selection, cx);
        self.source_drag_anchor = Some(selection.anchor);
        let selected_range = selection.range();
        self.source_drag_anchor_range = Some(if !shift && click_count >= 2 {
            selected_range
        } else {
            selection.anchor.byte_offset..selection.anchor.byte_offset
        });
        self.source_drag_granularity = if shift {
            SourceDragGranularity::Character
        } else if click_count >= 3 {
            SourceDragGranularity::Line
        } else if click_count == 2 {
            SourceDragGranularity::Word
        } else {
            SourceDragGranularity::Character
        };

        if block.read(cx).is_read_only() || click_count >= 3 {
            self.active_edit = None;
            self.focus_handle.focus(window);
        } else if shift && self.selection_spans_multiple_lines(selection) {
            self.active_edit = None;
            self.focus_handle.focus(window);
        } else {
            self.focus_source_pointer_line(line, window, cx);
        }
        self.sync_source_selection_visuals(cx);
        cx.emit(DocumentHostEvent::StateChanged);
        cx.notify();
    }

    /// 点击与单行拖选共用输入目标；仅跨行选区交给宿主，避免 Windows 同点移动事件丢失原生输入。
    pub(super) fn focus_source_pointer_line(
        &mut self,
        line: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(block) = self.source_row_blocks.get(&line).cloned() else {
            return;
        };
        let Some(row) = self.current_source_pointer_row(line, cx).cloned() else {
            return;
        };
        let Some(document) = self.document.as_ref() else {
            return;
        };
        let range = document.source_selection().range();
        if block.read(cx).is_read_only()
            || range.start < row.content_range.start
            || range.end > row.content_range.end
        {
            self.active_edit = None;
            self.focus_handle.focus(window);
            return;
        }
        let reversed = document.source_selection().reversed();
        let base_revision = document.revision();
        block.update(cx, |block, cx| {
            block.selected_range = (range.start - row.content_range.start) as usize
                ..(range.end - row.content_range.start) as usize;
            block.selection_reversed = reversed;
            block.set_source_host_input_focus_handle(None, cx);
            cx.notify();
        });
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

    /// 在输入法终态可能重排源码行前捕获字节命中位置，供终态后按原文档锚点回放。
    fn capture_source_pointer_snapshot(
        &self,
        line: usize,
        position: Point<Pixels>,
        cx: &App,
    ) -> Option<SourcePointerSnapshot> {
        let document = self.document.as_ref()?;
        let row = self.current_source_pointer_row(line, cx)?;
        let revision = document.revision();
        let row_entity = self.source_row_blocks.get(&line)?.clone();
        let local = row_entity
            .read(cx)
            .index_for_mouse_position(position)
            .min(row_entity.read(cx).display_text().len());
        if row_entity.read(cx).display_text() != row.text.as_ref() {
            return None;
        }
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
        let Some(row) = self.current_source_pointer_row(active.line, cx) else {
            return;
        };
        let selection = Self::source_selection_from_block(block.read(cx), row.content_range.start);
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
        if self.pending_source_ime_action.is_some()
            || self.coordinator.source_boundary_cancellation.is_some()
            || self.has_active_ime_composition(cx)
        {
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

    /// 拖选按按下时捕获的词或逻辑行粒度扩展；字符拖选沿用原生命中锚点。
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
        let Some(_block) = self.ensure_source_row_block(line, cx) else {
            return;
        };
        let Some(row) = self.current_source_pointer_row(line, cx).cloned() else {
            return;
        };
        if row.content_range != snapshot.row_range
            || row.text.as_ref() != snapshot.row_entity.read(cx).display_text()
        {
            return;
        }
        let Some(selection) = self.source_drag_selection(anchor, snapshot.hit, line, &row) else {
            let Some(document) = self.document.as_ref() else {
                return;
            };
            let Some(line_range) = document.line_range(line as u64) else {
                return;
            };
            let Some(anchor_line_range) = document
                .line_for_offset(anchor.byte_offset)
                .and_then(|line| document.line_range(line))
            else {
                return;
            };
            self.request_source_word_drag(
                line,
                position,
                anchor,
                snapshot.hit.byte_offset,
                anchor_line_range,
                line_range,
                window,
                cx,
            );
            return;
        };
        self.install_source_pointer_move(selection, line, position, window, cx);
    }

    /// 后台与即时命中安装同一绝对选区；重新校验窗口后再投影，保留单行原生输入目标。
    pub(super) fn install_source_pointer_move(
        &mut self,
        selection: SourceSelection,
        line: usize,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(block) = self.source_row_blocks.get(&line).cloned() else {
            return;
        };
        let Some(row) = self.current_source_pointer_row(line, cx).cloned() else {
            return;
        };
        let before = self
            .document
            .as_ref()
            .map(SharedDocument::source_selection)
            .unwrap_or_default();
        self.active_edit = None;
        self.focus_handle.focus(window);
        self.set_source_selection(selection, cx);
        self.focus_source_pointer_line(line, window, cx);
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
        if self.source_drag_anchor.is_none() {
            self.stop_source_drag_autoscroll();
        } else if position.y <= viewport.top() + px(self.source_row_height * 1.5) {
            self.start_source_drag_autoscroll(-1, cx);
        } else if position.y >= viewport.bottom() - px(self.source_row_height) {
            self.start_source_drag_autoscroll(1, cx);
        } else {
            self.stop_source_drag_autoscroll();
        }
        cx.emit(DocumentHostEvent::StateChanged);
        cx.notify();
    }

    /// 词单位不确定时交给后台解析，不能用窗口截断后的部分字素发布临时词选区。
    fn source_drag_selection(
        &self,
        anchor: SourceAnchor,
        hit: SourceAnchor,
        line: usize,
        row: &BoundedLineWindow,
    ) -> Option<SourceSelection> {
        let Some(anchor_range) = self.source_drag_anchor_range.clone() else {
            return Some(SourceSelection { anchor, head: hit });
        };
        let target_range = match self.source_drag_granularity {
            SourceDragGranularity::Character => return Some(SourceSelection { anchor, head: hit }),
            SourceDragGranularity::Line => self
                .document
                .as_ref()
                .and_then(|document| document.line_range(line as u64)),
            SourceDragGranularity::Word if anchor_range.is_empty() => return None,
            SourceDragGranularity::Word => bounded_word_range(row, hit.byte_offset),
        };
        let Some(target_range) = target_range else {
            return None;
        };
        Some(source_word_drag_selection(anchor_range, target_range))
    }

    /// 鼠标释放立即停止滚动；原生候选或词边界未结束时只把手势终点按顺序排队。
    pub(super) fn on_source_surface_mouse_up(
        &mut self,
        _: &gpui::MouseUpEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.stop_source_drag_autoscroll();
        if self.pending_source_ime_action.is_some()
            || self.coordinator.source_boundary_cancellation.is_some()
            || self.has_active_ime_composition(cx)
        {
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
    /// 折叠插入点两端统一向后亲和，避免仅同步光标就打断连续输入的撤销组。
    pub(super) fn source_selection_from_block(block: &Block, source_start: u64) -> SourceSelection {
        if block.selected_range.is_empty() {
            return SourceSelection::collapsed(
                source_start.saturating_add(block.selected_range.start as u64),
                SourceAffinity::After,
            );
        }
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
}
