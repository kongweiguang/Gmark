// @author kongweiguang

use super::*;

/// 无窗口测试使用 GPUI 的隔离剪贴板；生产路径仍以原生写入成功作为删除前提。
fn accept_native_cut_clipboard(text: &str) -> bool {
    #[cfg(test)]
    {
        let _ = text;
        true
    }
    #[cfg(not(test))]
    {
        arboard::Clipboard::new()
            .and_then(|mut clipboard| clipboard.set_text(text.to_owned()))
            .is_ok()
    }
}

impl Block {
    /// Treats native clipboard acceptance as a precondition so a failed Cut never loses selected text.
    pub(crate) fn on_cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        if self.guard_input_command(crate::components::block::BlockInputCommand::Cut, window, cx) {
            return;
        }
        if self.selected_range.is_empty() {
            return;
        }
        let Some(selected_text) = self
            .display_text()
            .get(self.selected_range.clone())
            .map(str::to_owned)
        else {
            cx.stop_propagation();
            return;
        };
        if !accept_native_cut_clipboard(&selected_text) {
            cx.emit(BlockEvent::RequestClipboardFailure);
            cx.stop_propagation();
            return;
        }

        self.prepare_undo_capture(UndoCaptureKind::NonCoalescible, cx);
        cx.write_to_clipboard(ClipboardItem::new_string(selected_text));
        self.replace_text_in_range(None, "", window, cx);
        cx.stop_propagation();
    }

    /// 在识别图片路径或派发结构化粘贴前先验证最终块大小，避免超限输入触发多份中间拷贝。
    pub(crate) fn on_paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        if self.guard_input_command(
            crate::components::block::BlockInputCommand::Paste,
            window,
            cx,
        ) {
            return;
        }
        if self.kind().is_separator() && !self.uses_raw_text_editing() {
            return;
        }

        if let Some(item) = cx.read_from_clipboard() {
            if Self::clipboard_image_exceeds_limit(&item) {
                self.show_paste_limit_error(window, cx);
                return;
            }
            if let Some(source) = Self::pasted_image_source_from_clipboard(&item) {
                let (leading, trailing) = self.paste_resource_split();
                cx.emit(BlockEvent::RequestPasteImage {
                    leading,
                    source,
                    trailing,
                });
                return;
            }

            let Some(text) = item.text() else {
                return;
            };
            if self.paste_output_exceeds_limit(&text) {
                self.show_paste_limit_error(window, cx);
                return;
            }
            if let Some(source) = Self::pasted_image_source_from_text(&text) {
                let (leading, trailing) = self.paste_resource_split();
                cx.emit(BlockEvent::RequestPasteImage {
                    leading,
                    source,
                    trailing,
                });
                return;
            }

            self.paste_text(text, window, cx);
        }
    }

    /// 纯文本入口复用同一结果上限，保证绕过富文本识别也不会部分写入超大内容。
    pub(crate) fn on_paste_as_plain_text(
        &mut self,
        _: &PasteAsPlainText,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.guard_input_command(
            crate::components::block::BlockInputCommand::PasteAsPlainText,
            window,
            cx,
        ) {
            return;
        }
        if self.kind().is_separator() && !self.uses_raw_text_editing() {
            return;
        }
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return;
        };
        if self.paste_output_exceeds_limit(&text) {
            self.show_paste_limit_error(window, cx);
            return;
        }
        self.paste_text(text, window, cx);
    }

    /// 所有内部粘贴调用再次校验结果长度，防止未来新增调用方绕过剪贴板入口门禁。
    fn paste_text(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_read_only() || self.has_ime_composition() {
            return;
        }
        if self.paste_output_exceeds_limit(&text) {
            self.show_paste_limit_error(window, cx);
            return;
        }
        // Only rendered rich-text blocks apply paste correction. Raw/code
        // contexts preserve bytes, and table cells flatten newlines so the
        // surrounding table structure is not accidentally split.
        if self.editor_selection_range.is_some() {
            cx.emit(BlockEvent::RequestReplaceCrossBlockSelection {
                text,
                selected_range_relative: None,
                mark_inserted_text: false,
                undo_kind: UndoCaptureKind::NonCoalescible,
            });
            return;
        }

        if self.is_table_cell() {
            let flattened = text.replace("\r\n", " ").replace(['\r', '\n'], " ");
            self.prepare_undo_capture(UndoCaptureKind::NonCoalescible, cx);
            self.replace_text_in_range(None, &flattened, window, cx);
            return;
        }

        if self.uses_raw_text_editing() {
            self.prepare_undo_capture(UndoCaptureKind::NonCoalescible, cx);
            self.replace_text_in_range(None, &text, window, cx);
            return;
        }

        if text.contains('\n') || text.contains('\r') {
            let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
            if self.quote_depth > 0 {
                self.prepare_undo_capture(UndoCaptureKind::NonCoalescible, cx);
                self.replace_text_in_range(None, &normalized, window, cx);
                return;
            }
            let clean_selected = self.selection_clean_range();
            let (leading, tail) = self.record.title.split_at(clean_selected.start);
            let (_, trailing) =
                tail.split_at(clean_selected.end.saturating_sub(clean_selected.start));
            let lines = normalized
                .split('\n')
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>();
            let split_physical_lines = should_split_plain_multiline_paste(&lines);
            cx.emit(BlockEvent::RequestPasteMultiline {
                leading,
                lines,
                trailing,
                split_physical_lines,
            });
            return;
        }

        self.prepare_undo_capture(UndoCaptureKind::NonCoalescible, cx);
        self.replace_text_in_range(None, &text, window, cx);
    }
}
