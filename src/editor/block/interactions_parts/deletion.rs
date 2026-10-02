// @author kongweiguang

use super::*;

const MAX_PASTE_OUTPUT_BYTES: usize = 64 * 1024 * 1024;

/// 用 checked 减加估算替换选区后的粘贴结果，防止异常范围或长度绕过 64 MiB 门禁。
fn checked_paste_output_len(
    source_len: usize,
    selected_range: &std::ops::Range<usize>,
    text_len: usize,
) -> Option<usize> {
    if selected_range.end > source_len {
        return None;
    }
    let selected_len = selected_range.end.checked_sub(selected_range.start)?;
    source_len.checked_sub(selected_len)?.checked_add(text_len)
}

impl Block {
    /// 在触发资源识别、换行拆分或源码事务前拒绝超出上限的粘贴，避免大剪贴板内容
    /// 先复制多份再阻塞 GPUI；弹窗让拒绝原因对用户可见且不会改变当前选择。
    fn show_paste_limit_error(&self, window: &mut Window, cx: &mut Context<Self>) {
        let strings = cx.global::<crate::i18n::I18nManager>().strings().clone();
        let buttons = [strings.info_dialog_ok.as_str()];
        let _ = window.prompt(
            PromptLevel::Critical,
            &strings.image_paste_failed_title,
            Some("粘贴内容超过 64 MiB 安全限制"),
            &buttons,
            cx,
        );
    }

    /// 用 checked 算术估算当前块替换选区后的大小，使粘贴上限同时约束输入和结果。
    fn paste_output_exceeds_limit(&self, text: &str) -> bool {
        checked_paste_output_len(self.display_text().len(), &self.selected_range, text.len())
            .is_none_or(|output| output > MAX_PASTE_OUTPUT_BYTES)
    }

    /// 在复制图片进入资源物化前检查原始字节，避免解码和编码阶段放大超限剪贴板。
    fn clipboard_image_exceeds_limit(item: &ClipboardItem) -> bool {
        item.entries().iter().any(|entry| {
            matches!(
                entry,
                ClipboardEntry::Image(image) if image.bytes().len() > MAX_PASTE_OUTPUT_BYTES
            )
        })
    }

    /// 先完成原输入法会话并再次校验只读状态，避免普通删除被解释为候选结果。
    pub(crate) fn on_delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
        if self.guard_input_command(
            crate::components::block::BlockInputCommand::Delete,
            window,
            cx,
        ) {
            return;
        }
        if self.kind() == BlockKind::MermaidBlock
            && self.mermaid_view_mode() == MermaidViewMode::Preview
        {
            cx.emit(BlockEvent::RequestDelete);
            return;
        }

        if self.is_table_cell() {
            if self.selected_range.is_empty() {
                let next = self.next_boundary(self.cursor_offset());
                if next == self.cursor_offset() {
                    return;
                }
                self.select_to(next, cx);
            }
            self.replace_text_in_range(None, "", window, cx);
            return;
        }

        if self.is_source_raw_mode() {
            if self.selected_range.is_empty() {
                self.select_to(self.next_boundary(self.cursor_offset()), cx);
            }
            self.replace_text_in_range(None, "", window, cx);
            return;
        }

        if self.downgrade_leaf_callout_to_quote_at_start(cx)
            || self.downgrade_empty_leaf_quote_to_paragraph(cx)
        {
            return;
        }

        if self.kind().is_separator() {
            self.convert_to_paragraph(cx);
            return;
        }

        if self.selected_range.is_empty() {
            self.select_to(self.next_boundary(self.cursor_offset()), cx);
        }
        self.replace_text_in_range(None, "", window, cx);
    }

    /// 先完成原输入法会话并再次校验只读状态，避免普通删除被解释为候选结果。
    pub(crate) fn on_word_delete_back(
        &mut self,
        _: &WordDeleteBack,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.guard_input_command(
            crate::components::block::BlockInputCommand::WordDeleteBack,
            window,
            cx,
        ) {
            return;
        }
        if self.selected_range.is_empty() {
            if self.cursor_offset() == 0 {
                // Nothing to the left in this block; defer to grapheme
                // backspace, which handles block merge and downgrades.
                self.on_delete_back(&DeleteBack, window, cx);
                return;
            }
            self.select_to(self.previous_word_start(self.cursor_offset()), cx);
        }
        self.replace_text_in_range(None, "", window, cx);
    }

    /// 先完成原输入法会话并再次校验只读状态，避免普通删除被解释为候选结果。
    pub(crate) fn on_word_delete_forward(
        &mut self,
        _: &WordDeleteForward,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.guard_input_command(
            crate::components::block::BlockInputCommand::WordDeleteForward,
            window,
            cx,
        ) {
            return;
        }
        if self.selected_range.is_empty() {
            if self.cursor_offset() == self.visible_len() {
                // Nothing to the right in this block; defer to grapheme
                // delete, which handles block merge and separator removal.
                self.on_delete(&Delete, window, cx);
                return;
            }
            self.select_to(self.next_word_start(self.cursor_offset()), cx);
        }
        self.replace_text_in_range(None, "", window, cx);
    }

    /// Keeps a collapsed local caret as text input; selected rows and hosted fields
    /// retain owner routing.
    pub(crate) fn on_indent_block(
        &mut self,
        _: &IndentBlock,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.handle_contextual_editing_action("tab", window, cx) {
            return;
        }
        if self.uses_raw_text_editing() || self.kind().is_code_block() {
            if self.selected_range.is_empty() && !self.compact_source_host() {
                self.replace_text_in_range(None, "    ", window, cx);
            } else {
                self.apply_raw_line_operation(
                    crate::components::block::LineOperation::Indent,
                    window,
                    cx,
                );
            }
            return;
        }
        if self.is_table_cell() {
            cx.emit(BlockEvent::RequestTableCellMoveHorizontal { delta: 1 });
            return;
        }
        if self.can_adjust_list_nesting() {
            cx.emit(BlockEvent::RequestIndent);
            return;
        }
        if self.kind() == BlockKind::Paragraph || self.kind().is_code_block() {
            self.replace_text_in_range(None, "    ", window, cx);
        }
    }

    /// Uses the same row planner as multi-line Outdent so CRLF and the active selection remain stable.
    pub(crate) fn on_outdent_block(
        &mut self,
        _: &OutdentBlock,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.uses_raw_text_editing() || self.kind().is_code_block() {
            self.apply_raw_line_operation(
                crate::components::block::LineOperation::Outdent,
                window,
                cx,
            );
            return;
        }
        if self.is_table_cell() {
            cx.emit(BlockEvent::RequestTableCellMoveHorizontal { delta: -1 });
            return;
        }
        if self.can_outdent_list_nesting() {
            cx.emit(BlockEvent::RequestOutdent);
        }
    }

    pub(crate) fn on_block_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.math_edit_session.is_some() || self.kind() == BlockKind::MathBlock {
            let modifiers = event.keystroke.modifiers;
            if event.keystroke.key == "enter"
                && (modifiers.platform || modifiers.control)
                && !modifiers.alt
            {
                self.finish_math_edit(cx);
                cx.stop_propagation();
                return;
            }
            if event.keystroke.key == "escape" {
                self.finish_math_edit(cx);
                cx.stop_propagation();
                return;
            }
        }
        if self.host_submit_enabled()
            && event.keystroke.key == "enter"
            && event.keystroke.modifiers == Modifiers::none()
        {
            self.dispatch_host_action(
                BlockHostAction::Submit(self.shared_display_text()),
                window,
                cx,
            );
            return;
        }
        if event.keystroke.key == "escape" && self.cancel_image_selection(cx) {
            cx.stop_propagation();
            return;
        }
        if event.keystroke.key != "tab" {
            return;
        }

        let modifiers = event.keystroke.modifiers;
        if modifiers.control || modifiers.platform || modifiers.alt || modifiers.function {
            return;
        }

        if self.code_language_focus_handle.is_focused(window) {
            return;
        }

        if modifiers.shift {
            self.on_outdent_block(&OutdentBlock, window, cx);
        } else {
            self.on_indent_block(&IndentBlock, window, cx);
        }
        cx.stop_propagation();
    }

    pub(crate) fn on_focus_prev(
        &mut self,
        _: &FocusPrev,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.handle_contextual_editing_action("up", window, cx) {
            return;
        }
        let preferred_x = self.vertical_anchor_x();
        if !self.move_cursor_vertically(-1, preferred_x, cx) {
            if self.is_table_cell() {
                cx.emit(BlockEvent::RequestTableCellMoveVertical { delta: -1 });
                return;
            }
            cx.emit(BlockEvent::RequestFocusPrev {
                preferred_x: Some(f32::from(preferred_x)),
            });
        }
    }

    pub(crate) fn on_focus_next(
        &mut self,
        _: &FocusNext,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.handle_contextual_editing_action("down", window, cx) {
            return;
        }
        let preferred_x = self.vertical_anchor_x();
        if !self.move_cursor_vertically(1, preferred_x, cx) {
            if self.is_table_cell() {
                cx.emit(BlockEvent::RequestTableCellMoveVertical { delta: 1 });
                return;
            }
            // In a code block, Down from the last content line steps into the
            // language field rather than leaving the block, so the language is
            // reachable by keyboard. A further Down there exits the block.
            if self.kind().is_code_block() && !self.code_language_focus_handle.is_focused(window) {
                self.code_language_focus_handle.focus(window);
                cx.notify();
                return;
            }
            cx.emit(BlockEvent::RequestFocusNext {
                preferred_x: Some(f32::from(preferred_x)),
            });
        }
    }

    pub(crate) fn on_move_left(
        &mut self,
        _: &MoveLeft,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.handle_contextual_editing_action("left", window, cx) {
            return;
        }
        if self.selected_range.is_empty() {
            if let Some((target, affinity)) = self.projected_move_left_target(self.cursor_offset())
            {
                self.assign_collapsed_selection_offset(target, affinity, None);
                self.cursor_blink_epoch = std::time::Instant::now();
                cx.notify();
            } else {
                let previous = self.previous_boundary(self.cursor_offset());
                // At the start of a table cell, step into the previous cell
                // rather than stalling at the edge (same path as Shift+Tab).
                if previous == self.cursor_offset() && self.is_table_cell() {
                    cx.emit(BlockEvent::RequestTableCellMoveHorizontal { delta: -1 });
                    return;
                }
                self.move_to(previous, cx);
            }
        } else {
            self.move_to(self.selected_range.start, cx);
        }
    }

    pub(crate) fn on_move_right(
        &mut self,
        _: &MoveRight,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.handle_contextual_editing_action("right", window, cx) {
            return;
        }
        if self.selected_range.is_empty() {
            if let Some((target, affinity)) =
                self.projected_move_right_target(self.selected_range.end)
            {
                self.assign_collapsed_selection_offset(target, affinity, None);
                self.cursor_blink_epoch = std::time::Instant::now();
                cx.notify();
            } else {
                let next = self.next_boundary(self.selected_range.end);
                // At the end of a table cell, step into the next cell rather
                // than stalling at the edge (same path as Tab).
                if next == self.selected_range.end && self.is_table_cell() {
                    cx.emit(BlockEvent::RequestTableCellMoveHorizontal { delta: 1 });
                    return;
                }
                self.move_to(next, cx);
            }
        } else {
            self.move_to(self.selected_range.end, cx);
        }
    }

    /// Keeps Home inside the active wrapped row and leaves an in-progress IME candidate untouched.
    pub(crate) fn on_home(&mut self, _: &Home, window: &mut Window, cx: &mut Context<Self>) {
        if self.has_ime_composition() {
            cx.stop_propagation();
            return;
        }
        if self.handle_contextual_editing_action("home", window, cx) {
            return;
        }
        self.move_to(self.current_visual_line_boundary(false), cx);
    }

    /// Keeps End inside the active wrapped row and leaves an in-progress IME candidate untouched.
    pub(crate) fn on_end(&mut self, _: &End, window: &mut Window, cx: &mut Context<Self>) {
        if self.has_ime_composition() {
            cx.stop_propagation();
            return;
        }
        if self.handle_contextual_editing_action("end", window, cx) {
            return;
        }
        self.move_to(self.current_visual_line_boundary(true), cx);
    }

    pub(crate) fn on_select_left(
        &mut self,
        _: &SelectLeft,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some((target, _)) = self.projected_move_left_target(self.cursor_offset()) {
            self.select_to(target, cx);
        } else {
            self.select_to(self.previous_boundary(self.cursor_offset()), cx);
        }
    }

    pub(crate) fn on_select_right(
        &mut self,
        _: &SelectRight,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some((target, _)) = self.projected_move_right_target(self.cursor_offset()) {
            self.select_to(target, cx);
        } else {
            self.select_to(self.next_boundary(self.cursor_offset()), cx);
        }
    }

    pub(crate) fn on_word_move_left(
        &mut self,
        _: &WordMoveLeft,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_to(self.previous_word_start(self.cursor_offset()), cx);
    }

    pub(crate) fn on_word_move_right(
        &mut self,
        _: &WordMoveRight,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_to(self.next_word_start(self.cursor_offset()), cx);
    }

    pub(crate) fn on_word_select_left(
        &mut self,
        _: &WordSelectLeft,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_to(self.previous_word_start(self.cursor_offset()), cx);
    }

    pub(crate) fn on_word_select_right(
        &mut self,
        _: &WordSelectRight,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_to(self.next_word_start(self.cursor_offset()), cx);
    }

    pub(crate) fn on_block_up(
        &mut self,
        _: &BlockUp,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.emit(BlockEvent::RequestBlockUp);
    }

    pub(crate) fn on_block_down(
        &mut self,
        _: &BlockDown,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.emit(BlockEvent::RequestBlockDown);
    }

    fn select_all_text(&mut self, cx: &mut Context<Self>) {
        self.move_to(0, cx);
        self.select_to(self.visible_len(), cx);
    }

    /// 文档全选等待原候选结束；独立输入字段仍只选择本地文字。
    pub(crate) fn on_select_all(
        &mut self,
        _: &SelectAll,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.guard_input_command(
            crate::components::block::BlockInputCommand::SelectAll,
            _window,
            cx,
        ) {
            return;
        }
        if self.is_read_only() {
            // 只读表面仍允许选择；事件一次升级为所属文档的全文选区。
            self.select_all_text(cx);
            cx.emit(BlockEvent::RequestRenderedSelectAll);
            return;
        }

        // 独立的 SourceRaw Block 也用于大文件行编辑、查找和跳转输入框；它没有
        // Editor 级跨块选择订阅，Ctrl+A 必须留在本地文本内，否则会选中整份大文件。
        if self.compact_source_host()
            && self.selected_range == (0..self.visible_len())
            && !self.display_text().is_empty()
        {
            cx.emit(BlockEvent::RequestRenderedSelectAll);
        } else if self.show_source_line_numbers() || self.is_source_raw_mode() {
            self.select_all_text(cx);
        } else {
            cx.emit(BlockEvent::RequestRenderedSelectAll);
        }
    }

    /// Extends from the existing anchor to the current visual row start without displacing IME preedit.
    pub(crate) fn on_select_home(
        &mut self,
        _: &SelectHome,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.has_ime_composition() {
            cx.stop_propagation();
            return;
        }
        self.select_to(self.current_visual_line_boundary(false), cx);
    }

    /// Extends from the existing anchor to the current visual row end without displacing IME preedit.
    pub(crate) fn on_select_end(
        &mut self,
        _: &SelectEnd,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.has_ime_composition() {
            cx.stop_propagation();
            return;
        }
        self.select_to(self.current_visual_line_boundary(true), cx);
    }

    pub(crate) fn on_copy(&mut self, _: &Copy, _window: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.display_text()[self.selected_range.clone()].to_string(),
            ));
        }
    }

    pub(crate) fn on_copy_as_markdown(
        &mut self,
        _: &CopyAsMarkdown,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.selected_range.is_empty() {
            return;
        }

        let markdown = if self.uses_raw_text_editing() || self.kind().is_code_block() {
            self.display_text()[self.selected_range.clone()].to_owned()
        } else {
            let range = self.selection_clean_range();
            let (_, tail) = self.record.title.split_at(range.start);
            let (selected, _) = tail.split_at(range.end.saturating_sub(range.start));
            selected.serialize_markdown()
        };
        cx.write_to_clipboard(ClipboardItem::new_string(markdown));
    }

    pub(crate) fn on_code_language_newline(
        &mut self,
        _: &Newline,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.code_language_focus_handle.is_focused(window) {
            return;
        }
        cx.stop_propagation();
        self.focus_handle.focus(window);
        cx.notify();
    }

    pub(crate) fn on_code_language_dismiss(
        &mut self,
        _: &DismissTransientUi,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.code_language_focus_handle.is_focused(window) {
            return;
        }
        cx.stop_propagation();
        self.focus_handle.focus(window);
        cx.notify();
    }

    pub(crate) fn on_code_language_delete_back(
        &mut self,
        _: &DeleteBack,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.code_language_focus_handle.is_focused(window) {
            return;
        }
        cx.stop_propagation();
        if self.code_language_selected_range.is_empty() {
            let previous = self.previous_code_language_boundary(self.code_language_cursor_offset());
            self.select_code_language_to(previous, cx);
        }
        self.replace_code_language_text_in_range(
            self.code_language_selected_range.clone(),
            "",
            None,
            false,
            cx,
        );
    }

    pub(crate) fn on_code_language_delete(
        &mut self,
        _: &Delete,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.code_language_focus_handle.is_focused(window) {
            return;
        }
        cx.stop_propagation();
        if self.code_language_selected_range.is_empty() {
            let next = self.next_code_language_boundary(self.code_language_cursor_offset());
            self.select_code_language_to(next, cx);
        }
        self.replace_code_language_text_in_range(
            self.code_language_selected_range.clone(),
            "",
            None,
            false,
            cx,
        );
    }
}

#[path = "deletion_clipboard.rs"]
mod clipboard;

#[cfg(test)]
#[path = "../../../../tests/unit/editor/paste_limits.rs"]
mod tests;

#[cfg(test)]
#[path = "../../../../tests/unit/components/block/visual_line_navigation.rs"]
mod visual_line_navigation_tests;
