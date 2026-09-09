// @author kongweiguang

use super::*;

pub(super) const CURSOR_BLINK_INTERVAL: Duration = Duration::from_millis(500);

pub(super) fn cursor_opacity_for_elapsed(elapsed: Duration) -> f32 {
    let phase = elapsed.as_millis() / CURSOR_BLINK_INTERVAL.as_millis();
    if phase.is_multiple_of(2) { 1.0 } else { 0.0 }
}

impl Block {
    pub(in super::super) fn mark_changed(&mut self, cx: &mut Context<Self>) {
        self.sync_edit_mode_from_kind();
        self.sync_render_cache();
        self.cursor_blink_epoch = Instant::now();
        self.clear_vertical_motion();
        cx.emit(BlockEvent::Changed);
        cx.notify();
    }

    pub(crate) fn convert_to_paragraph(&mut self, cx: &mut Context<Self>) {
        self.prepare_undo_capture(UndoCaptureKind::NonCoalescible, cx);
        self.record.kind = BlockKind::Paragraph;
        self.record.raw_fallback = None;
        self.quote_reparse_requested = false;
        self.mark_changed(cx);
    }

    pub(crate) fn convert_to_separator(&mut self, cx: &mut Context<Self>) {
        self.prepare_undo_capture(UndoCaptureKind::NonCoalescible, cx);
        self.make_separator();
        cx.emit(BlockEvent::Changed);
        cx.notify();
    }

    /// Turns this block into a separator in place without emitting events or
    /// capturing undo, so editor-level flows that already manage those can
    /// reuse the conversion.
    pub(crate) fn make_separator(&mut self) {
        self.clear_inline_projection();
        self.record.kind = BlockKind::Separator;
        self.record.raw_fallback = None;
        self.record.set_title(InlineTextTree::plain(String::new()));
        self.quote_reparse_requested = false;
        self.sync_edit_mode_from_kind();
        self.sync_render_cache();
        self.assign_collapsed_selection_offset(0, CollapsedCaretAffinity::Default, None);
        self.marked_range = None;
        self.cursor_blink_epoch = Instant::now();
        self.clear_vertical_motion();
    }

    pub(crate) fn enter_code_block(
        &mut self,
        language: Option<SharedString>,
        cx: &mut Context<Self>,
    ) {
        self.prepare_undo_capture(UndoCaptureKind::NonCoalescible, cx);
        self.clear_inline_projection();
        self.record.kind = BlockKind::CodeBlock { language };
        self.record.raw_fallback = None;
        self.record.set_title(InlineTextTree::plain(String::new()));
        self.quote_reparse_requested = false;
        self.sync_edit_mode_from_kind();
        self.sync_render_cache();
        self.assign_collapsed_selection_offset(0, CollapsedCaretAffinity::Default, None);
        self.marked_range = None;
        self.cursor_blink_epoch = Instant::now();
        self.clear_vertical_motion();
        cx.emit(BlockEvent::Changed);
        cx.notify();
    }

    /// 将直接输入的 Mermaid 围栏升级为原生工作台，并把光标放在围栏正文中。
    pub(crate) fn enter_mermaid_block(&mut self, fence: CodeFenceOpening, cx: &mut Context<Self>) {
        let opening = self.display_text().trim_end().to_owned();
        let closing = fence.ch.to_string().repeat(fence.len);
        let source = format!("{opening}\n\n{closing}");
        let cursor = opening.len() + 1;

        self.prepare_undo_capture(UndoCaptureKind::NonCoalescible, cx);
        self.clear_inline_projection();
        self.record.kind = BlockKind::MermaidBlock;
        self.mermaid_view_mode = MermaidViewMode::Source;
        self.record.set_title(InlineTextTree::plain(source));
        self.quote_reparse_requested = false;
        self.sync_edit_mode_from_kind();
        self.sync_render_cache();
        self.assign_collapsed_selection_offset(cursor, CollapsedCaretAffinity::Default, None);
        self.marked_range = None;
        self.cursor_blink_epoch = Instant::now();
        self.clear_vertical_motion();
        cx.emit(BlockEvent::Changed);
        cx.notify();
    }

    /// Convert the current paragraph into a display-math block. `body` becomes
    /// the formula source between the fences (empty for a fresh `$$` block), and
    /// the caret lands at the start of that body line.
    pub(crate) fn enter_math_block(&mut self, body: &str, cx: &mut Context<Self>) {
        let source = format!("$$\n{body}\n$$");
        let cursor = "$$\n".len();

        self.prepare_undo_capture(UndoCaptureKind::NonCoalescible, cx);
        self.clear_inline_projection();
        self.record.kind = BlockKind::MathBlock;
        self.record.set_title(InlineTextTree::plain(source));
        self.quote_reparse_requested = false;
        self.sync_edit_mode_from_kind();
        self.sync_render_cache();
        self.assign_collapsed_selection_offset(cursor, CollapsedCaretAffinity::Default, None);
        self.marked_range = None;
        self.cursor_blink_epoch = Instant::now();
        self.clear_vertical_motion();
        cx.emit(BlockEvent::Changed);
        cx.notify();
    }

    /// Toggle a style flag directly on the fragment tree without ever
    /// manipulating raw marker characters.  The selection range determines
    /// which fragments have their [`InlineStyle`] flag flipped.
    ///
    /// Serializers later translate these flags back to markers on export.
    pub(crate) fn toggle_inline_format(&mut self, format: InlineFormat, cx: &mut Context<Self>) {
        if self.editor_selection_range.is_some() {
            if self.editor_selection_supports_inline_commands {
                let command = match format {
                    InlineFormat::Bold => EditingCommandId::Bold,
                    InlineFormat::Italic => EditingCommandId::Italic,
                    InlineFormat::Strikethrough => EditingCommandId::Strikethrough,
                    InlineFormat::Underline => EditingCommandId::Underline,
                    InlineFormat::Highlight => EditingCommandId::Highlight,
                    InlineFormat::Superscript => EditingCommandId::Superscript,
                    InlineFormat::Subscript => EditingCommandId::Subscript,
                    InlineFormat::Code => EditingCommandId::InlineCode,
                };
                cx.emit(BlockEvent::RequestEditingCommand { command });
            }
            return;
        }
        if self.selected_range.is_empty() || self.uses_raw_text_editing() {
            return;
        }

        let mut next_title = self.record.title.clone();
        let selection = self.selection_clean_range();
        let changed = match format {
            InlineFormat::Bold => next_title.toggle_bold(selection.clone()),
            InlineFormat::Italic => next_title.toggle_italic(selection.clone()),
            InlineFormat::Strikethrough => next_title.toggle_strikethrough(selection.clone()),
            InlineFormat::Underline => next_title.toggle_underline(selection.clone()),
            InlineFormat::Highlight => next_title.toggle_highlight(selection.clone()),
            InlineFormat::Superscript => next_title.toggle_superscript(selection.clone()),
            InlineFormat::Subscript => next_title.toggle_subscript(selection.clone()),
            InlineFormat::Code => next_title.toggle_code(selection.clone()),
        };
        if !changed {
            return;
        }

        self.prepare_undo_capture(UndoCaptureKind::NonCoalescible, cx);
        self.apply_title_edit(
            next_title,
            selection.end,
            None,
            Some(selection),
            Some(self.selection_reversed),
            false,
            false,
            cx,
        );
    }

    /// 显式格式命令保留选区方向，不套用键入闭合符时的光标逃逸规则。
    pub(crate) fn clear_inline_formatting(&mut self, cx: &mut Context<Self>) {
        if self.editor_selection_range.is_some() {
            if self.editor_selection_supports_inline_commands {
                cx.emit(BlockEvent::RequestEditingCommand {
                    command: EditingCommandId::ClearFormatting,
                });
            }
            return;
        }
        if self.selected_range.is_empty() || self.uses_raw_text_editing() {
            return;
        }
        let selection = self.selection_clean_range();
        let mut next_title = self.record.title.clone();
        if !next_title.clear_text_formatting(selection.clone()) {
            return;
        }
        self.prepare_undo_capture(UndoCaptureKind::NonCoalescible, cx);
        self.apply_title_edit(
            next_title,
            selection.end,
            None,
            Some(selection),
            Some(self.selection_reversed),
            false,
            false,
            cx,
        );
    }

    pub(crate) fn insert_inline_math(&mut self, cx: &mut Context<Self>) {
        if self.editor_selection_range.is_some() {
            return;
        }
        if self.uses_raw_text_editing() || self.is_read_only() {
            return;
        }
        let range = self.selection_clean_range();
        let (text, selected) = if range.is_empty() {
            ("$  $".to_owned(), 2..2)
        } else {
            let selected_text = self.display_text()[range.clone()].to_owned();
            let len = selected_text.len();
            (format!("${selected_text}$"), 1..len + 1)
        };
        self.prepare_undo_capture(UndoCaptureKind::NonCoalescible, cx);
        self.replace_text_in_visible_range(range, &text, Some(selected), false, cx);
    }

    /// 链接切换沿用选区事务，避免使用字符输入的闭合符亲和性。
    pub(crate) fn toggle_inline_link(&mut self, cx: &mut Context<Self>) {
        if self.editor_selection_range.is_some() {
            return;
        }
        if self.selected_range.is_empty() || self.uses_raw_text_editing() {
            return;
        }
        let selection = self.selection_clean_range();
        let mut next_title = self.record.title.clone();
        if !next_title.toggle_inline_link(selection.clone()) {
            return;
        }
        self.prepare_undo_capture(UndoCaptureKind::NonCoalescible, cx);
        self.apply_title_edit(
            next_title,
            selection.end,
            None,
            Some(selection),
            Some(self.selection_reversed),
            false,
            false,
            cx,
        );
    }

    fn current_line_layout_and_offset(&self) -> Option<(&WrappedLine, usize)> {
        let lines = self.last_layout.as_ref()?;
        let text = self.display_text();
        let ranges = super::element::hard_line_ranges(text);
        let (line_idx, offset_in_line) =
            super::element::line_index_for_offset(&ranges, self.cursor_offset());
        Some((lines.get(line_idx)?, offset_in_line))
    }

    pub(in super::super) fn vertical_anchor_x(&self) -> Pixels {
        self.vertical_motion_x
            .or_else(|| {
                self.current_line_layout_and_offset()
                    .and_then(|(layout, offset_in_line)| {
                        super::element::position_for_offset(
                            layout,
                            offset_in_line,
                            self.last_line_height,
                            true,
                        )
                        .map(|position| position.x)
                    })
            })
            .unwrap_or(px(0.0))
    }

    /// Attempt to move the cursor up (direction < 0) or down one visual line
    /// within the current block.  Returns false if the cursor is already at
    /// the first or last line, so the editor can transfer focus instead.
    pub(in super::super) fn move_cursor_vertically(
        &mut self,
        direction: i32,
        preferred_x: Pixels,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(lines) = self.last_layout.as_ref() else {
            return false;
        };

        let text = self.display_text();
        let ranges = super::element::hard_line_ranges(text);
        let (current_line_idx, offset_in_line) =
            super::element::line_index_for_offset(&ranges, self.cursor_offset());
        let Some(current_layout) = lines.get(current_line_idx) else {
            return false;
        };
        let Some(current_position) = super::element::position_for_offset(
            current_layout,
            offset_in_line,
            self.last_line_height,
            true,
        ) else {
            return false;
        };

        let current_y =
            super::element::wrapped_line_top(lines, self.last_line_height, current_line_idx)
                + current_position.y;
        let target_y = if direction < 0 {
            current_y - self.last_line_height + self.last_line_height / 2.0
        } else {
            current_y + self.last_line_height + self.last_line_height / 2.0
        };
        if target_y < px(0.0) {
            return false;
        }

        let total_height = lines.iter().fold(px(0.0), |height, line| {
            height + super::element::wrapped_line_height(line, self.last_line_height)
        });
        if target_y >= total_height {
            return false;
        }

        let Some((target_line_idx, target_y_in_line)) =
            super::element::wrapped_line_for_y(lines, self.last_line_height, target_y)
        else {
            return false;
        };
        let target_layout = &lines[target_line_idx];
        let target_point = point(preferred_x, target_y_in_line);
        let target_offset_in_line =
            match target_layout.closest_index_for_position(target_point, self.last_line_height) {
                Ok(idx) | Err(idx) => idx,
            };

        let flat_offset = ranges[target_line_idx].start + target_offset_in_line;
        self.move_to_with_preferred_x(flat_offset, Some(preferred_x), cx);
        true
    }

    /// Compute the character offset where the cursor should land when focus
    /// enters this block from above or below.  Uses the stored vertical
    /// motion anchor so cursor horizontal position is preserved across
    /// different-height blocks.
    pub fn entry_offset_for_vertical_focus(
        &self,
        prefer_last_line: bool,
        preferred_x: Option<Pixels>,
    ) -> usize {
        let Some(lines) = self.last_layout.as_ref() else {
            return if prefer_last_line {
                self.visible_len()
            } else {
                0
            };
        };

        let text = self.display_text();
        let ranges = super::element::hard_line_ranges(text);
        let target_line_idx = if prefer_last_line { lines.len() - 1 } else { 0 };
        let target_layout = &lines[target_line_idx];
        let target_x = preferred_x.unwrap_or(px(0.0));
        let target_y = if prefer_last_line {
            super::element::wrapped_line_height(target_layout, self.last_line_height)
                - self.last_line_height / 2.0
        } else {
            self.last_line_height / 2.0
        };

        let offset_in_line = match target_layout
            .closest_index_for_position(point(target_x, target_y), self.last_line_height)
        {
            Ok(idx) | Err(idx) => idx,
        };
        ranges[target_line_idx].start + offset_in_line
    }

    pub fn move_to_with_preferred_x(
        &mut self,
        offset: usize,
        preferred_x: Option<Pixels>,
        cx: &mut Context<Self>,
    ) {
        self.assign_collapsed_selection_offset(
            offset,
            CollapsedCaretAffinity::Default,
            preferred_x,
        );
        self.cursor_blink_epoch = Instant::now();
        cx.emit(BlockEvent::SelectionChanged);
        cx.notify();
    }

    /// Starts the cursor blink loop. A caret only has two visually meaningful
    /// states, so repaint at the 500ms phase boundary instead of sampling a
    /// cosine at 30Hz and rebuilding the editor while the user is idle.
    ///
    /// The blink task is automatically cancelled when the block loses focus
    /// (the task handle is dropped in [`Block::render`]).
    pub(in super::super) fn start_cursor_blink(&mut self, cx: &mut Context<Self>) {
        self.cursor_blink_epoch = Instant::now();
        self.cursor_blink_task = Some(cx.spawn(
            async |this: WeakEntity<Block>, cx: &mut AsyncApp| loop {
                cx.background_executor().timer(CURSOR_BLINK_INTERVAL).await;
                if this
                    .update(cx, |_this: &mut Block, cx: &mut Context<Block>| cx.notify())
                    .is_err()
                {
                    break;
                }
            },
        ));
    }

    /// 窗口失活时取消定时器并保持光标常亮；重新激活后的 Block render 会按需重启。
    pub(crate) fn set_cursor_blink_window_active(&mut self, active: bool, cx: &mut Context<Self>) {
        self.cursor_blink_epoch = Instant::now();
        if !active {
            self.cursor_blink_task = None;
        }
        cx.notify();
    }

    /// Standard two-phase caret blink: visible for 500ms, hidden for 500ms.
    pub fn cursor_opacity(&self) -> f32 {
        cursor_opacity_for_elapsed(self.cursor_blink_epoch.elapsed())
    }

    pub fn cursor_offset(&self) -> usize {
        if self.selection_reversed {
            self.selected_range.start
        } else {
            self.selected_range.end
        }
    }

    pub(crate) fn end_pointer_selection_session(&mut self) -> bool {
        let changed = self.is_selecting || self.code_language_is_selecting;
        self.is_selecting = false;
        self.code_language_is_selecting = false;
        changed
    }

    pub(super) fn selection_anchor_focus(&self) -> (usize, usize) {
        if self.selection_reversed {
            (self.selected_range.end, self.selected_range.start)
        } else {
            (self.selected_range.start, self.selected_range.end)
        }
    }

    pub(super) fn clean_selection_anchor_focus(&self) -> (usize, usize) {
        let (anchor, focus) = self.selection_anchor_focus();
        (
            self.current_to_clean_offset(anchor),
            self.current_to_clean_offset(focus),
        )
    }

    pub(super) fn set_selection_from_anchor_focus(&mut self, anchor: usize, focus: usize) {
        let clamped_anchor = anchor.min(self.visible_len());
        let clamped_focus = focus.min(self.visible_len());
        self.selected_range = clamped_anchor.min(clamped_focus)..clamped_anchor.max(clamped_focus);
        self.selection_reversed = !self.selected_range.is_empty() && clamped_focus < clamped_anchor;
    }

    pub(super) fn set_selection_from_clean_anchor_focus(
        &mut self,
        anchor: usize,
        focus: usize,
        anchor_affinity: CollapsedCaretAffinity,
        focus_affinity: CollapsedCaretAffinity,
    ) {
        // Map each endpoint back through its own affinity. Several display
        // positions can share one clean offset (a trailing link's `](url)`
        // delimiters all collapse onto the anchor-text end), so the plain
        // clean->display cursor map would snap an endpoint that sat after the
        // closing delimiter back to just inside it. Honoring the captured
        // affinity keeps such endpoints in place across a projection rebuild.
        self.set_selection_from_anchor_focus(
            self.clean_to_current_cursor_offset_with_affinity(anchor, anchor_affinity),
            self.clean_to_current_cursor_offset_with_affinity(focus, focus_affinity),
        );
    }

    pub fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.move_to_with_preferred_x(offset, None, cx);
    }

    pub fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        let clamped_offset = offset.min(self.visible_len());
        if self.selection_reversed {
            self.selected_range.start = clamped_offset;
        } else {
            self.selected_range.end = clamped_offset;
        }
        if self.selected_range.end < self.selected_range.start {
            self.selection_reversed = !self.selection_reversed;
            self.selected_range = self.selected_range.end..self.selected_range.start;
        }
        self.cursor_blink_epoch = Instant::now();
        self.clear_vertical_motion();
        self.sync_collapsed_caret_affinity();
        cx.emit(BlockEvent::SelectionChanged);
        cx.notify();
    }

    pub(in super::super) fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        Self::utf8_range_to_utf16_in(self.display_text(), range)
    }

    pub(in super::super) fn range_from_utf16(&self, range_utf16: &Range<usize>) -> Range<usize> {
        Self::utf16_range_to_utf8_in(self.display_text(), range_utf16)
    }

    pub fn previous_boundary(&self, offset: usize) -> usize {
        let text = self.display_text();
        let mut cursor = GraphemeCursor::new(offset.min(text.len()), text.len(), true);
        cursor.prev_boundary(text, 0).ok().flatten().unwrap_or(0)
    }

    pub fn next_boundary(&self, offset: usize) -> usize {
        let text = self.display_text();
        let mut cursor = GraphemeCursor::new(offset.min(text.len()), text.len(), true);
        cursor
            .next_boundary(text, 0)
            .ok()
            .flatten()
            .unwrap_or(text.len())
    }

    /// Offset of the start of the word before `offset`, or 0 if there is none.
    pub fn previous_word_start(&self, offset: usize) -> usize {
        let text = self.display_text();
        let offset = offset.min(text.len());
        text.unicode_word_indices()
            .map(|(start, _)| start)
            .take_while(|start| *start < offset)
            .last()
            .unwrap_or(0)
    }

    /// Offset of the start of the word after `offset`, or the text length if
    /// there is none.
    pub fn next_word_start(&self, offset: usize) -> usize {
        let text = self.display_text();
        let offset = offset.min(text.len());
        text.unicode_word_indices()
            .map(|(start, _)| start)
            .find(|start| *start > offset)
            .unwrap_or(text.len())
    }

    /// Reverse of `display_offset`: maps an expanded display offset
    /// back to the clean tree offset.
    pub(super) fn unexpand_offset(&self, expanded: usize) -> usize {
        let Some(projection) = &self.projection else {
            return expanded;
        };
        projection
            .display_to_clean
            .get(expanded.min(projection.display_to_clean.len().saturating_sub(1)))
            .copied()
            .unwrap_or(expanded)
    }

    pub fn index_for_mouse_position(&self, position: Point<Pixels>) -> usize {
        if self.display_text().is_empty() {
            return 0;
        }

        let (Some(bounds), Some(lines)) = (self.last_bounds.as_ref(), self.last_layout.as_ref())
        else {
            return 0;
        };

        if position.y < bounds.top() {
            return 0;
        }
        if position.y > bounds.bottom() {
            return self.visible_len();
        }

        let text = self.display_text();
        let ranges = super::element::hard_line_ranges(text);
        let relative_y = position.y - bounds.top();
        let Some((line_idx, y_in_line)) =
            super::element::wrapped_line_for_y(lines, self.last_line_height, relative_y)
        else {
            return 0;
        };
        let layout = &lines[line_idx];
        let origin_x = super::element::aligned_line_left(layout, *bounds, self.text_align());

        let offset_in_line = match layout.closest_index_for_position(
            point(position.x - origin_x, y_in_line),
            self.last_line_height,
        ) {
            Ok(idx) | Err(idx) => idx,
        };
        ranges[line_idx].start + offset_in_line
    }

    pub(crate) fn active_range_or_cursor_bounds(&self) -> Option<Bounds<Pixels>> {
        let bounds = self.last_bounds?;
        let lines = self.last_layout.as_ref()?;
        let line_height = self.last_line_height;
        let text = self.display_text();
        let active_range = self
            .editor_selection_range
            .clone()
            .or_else(|| self.marked_range.clone())
            .unwrap_or_else(|| self.selected_range.clone());

        if active_range.is_empty() {
            return super::element::cursor_bounds_for_offset(
                lines,
                bounds,
                line_height,
                text,
                self.cursor_offset(),
                self.text_align(),
                px(1.0),
            );
        }

        super::element::range_bounds(
            lines,
            bounds,
            line_height,
            text,
            active_range,
            self.text_align(),
        )
    }
}
