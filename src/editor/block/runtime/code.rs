// @author kongweiguang

//! Code-block runtime cache management.

use super::*;

fn normalize_code_language_input(text: &str) -> String {
    text.replace("\r\n", " ")
        .replace(['\r', '\n'], " ")
        .trim()
        .to_string()
}

impl Block {
    /// 只为可编辑 Live 块打开语言菜单；Split/Preview 的实体是投影副本，允许它们
    /// 改语言会制造不写回 SourceDocument 的局部高亮状态。
    pub(crate) fn open_code_language_menu(&mut self, cx: &mut Context<Self>) {
        if self.is_read_only() {
            self.code_language_menu_open = false;
            return;
        }
        let current = self.code_language_text();
        self.code_language_menu_selected = crate::components::CODE_LANGUAGE_MENU_ITEMS
            .iter()
            .position(|language| *language == current)
            .unwrap_or(0);
        self.code_language_menu_open = true;
        cx.notify();
    }

    /// 切换可编辑代码块的语言菜单；只读投影保留相同代码表面但不创建编辑入口。
    pub(crate) fn toggle_code_language_menu(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.is_read_only() || !self.kind().is_code_block() {
            self.code_language_menu_open = false;
            return;
        }
        self.code_language_focus_handle.focus(window);
        if self.code_language_menu_open {
            self.code_language_menu_open = false;
            cx.notify();
        } else {
            self.open_code_language_menu(cx);
        }
    }

    /// 从菜单提交语言变更；在核心命令层再次拒绝只读实体，避免鼠标、键盘或测试
    /// 绕过渲染层后让 Split 预览与 Live/Source 真值分叉。
    pub(crate) fn select_code_language_menu_item(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.is_read_only() {
            self.code_language_menu_open = false;
            return;
        }
        let Some(language) = crate::components::CODE_LANGUAGE_MENU_ITEMS.get(index) else {
            return;
        };
        let next = SharedString::from(*language);
        let current = match &self.record.kind {
            BlockKind::CodeBlock { language } => language.clone(),
            _ => return,
        };
        self.code_language_menu_open = false;
        if current.as_ref() == Some(&next) {
            cx.notify();
            return;
        }
        self.prepare_undo_capture(UndoCaptureKind::NonCoalescible, cx);
        self.record.kind = BlockKind::CodeBlock {
            language: Some(next.clone()),
        };
        self.code_language_selected_range = next.len()..next.len();
        self.code_language_selection_reversed = false;
        self.code_language_marked_range = None;
        self.sync_code_highlight();
        cx.emit(BlockEvent::Changed);
        cx.notify();
    }

    /// 只在可编辑语言输入持有焦点且菜单打开时消费导航键；只读预览必须把按键
    /// 留给文档级滚动和导航。
    pub(crate) fn handle_code_language_menu_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.is_read_only() || !self.code_language_focus_handle.is_focused(window) {
            self.code_language_menu_open = false;
            return false;
        }
        let modifiers = event.keystroke.modifiers;
        if modifiers.control || modifiers.platform || modifiers.alt || modifiers.function {
            return false;
        }
        if !self.code_language_menu_open {
            return false;
        }
        let count = crate::components::CODE_LANGUAGE_MENU_ITEMS.len();
        match event.keystroke.key.as_str() {
            "up" => {
                self.code_language_menu_selected = if self.code_language_menu_selected == 0 {
                    count - 1
                } else {
                    self.code_language_menu_selected - 1
                };
            }
            "down" => {
                self.code_language_menu_selected = (self.code_language_menu_selected + 1) % count;
            }
            "enter" => {
                self.select_code_language_menu_item(self.code_language_menu_selected, cx);
            }
            "escape" => self.code_language_menu_open = false,
            _ => return false,
        }
        cx.notify();
        true
    }

    pub(crate) fn copy_code_block(&mut self, cx: &mut Context<Self>) {
        if !self.kind().is_code_block() {
            return;
        }
        cx.write_to_clipboard(ClipboardItem::new_string(
            self.record.title.visible_text().to_owned(),
        ));
        self.code_copy_feedback = true;
        self.code_copy_feedback_task = Some(cx.spawn(
            async |this: WeakEntity<Block>, cx: &mut AsyncApp| {
                cx.background_executor()
                    .timer(Duration::from_millis(1_200))
                    .await;
                let _ = this.update(cx, |block, cx| {
                    block.code_copy_feedback = false;
                    block.code_copy_feedback_task = None;
                    cx.notify();
                });
            },
        ));
        cx.notify();
    }

    pub(crate) fn code_highlight_result(&self) -> Option<&CodeHighlightResult> {
        self.code_highlight.as_ref()
    }

    pub(crate) fn set_source_syntax_language(&mut self, language: Option<&'static str>) {
        let next = language.map(SharedString::from);
        if self.source_syntax_language == next && self.source_syntax_context.is_none() {
            return;
        }
        self.source_syntax_language = next;
        self.source_syntax_context = None;
        self.sync_code_highlight();
    }

    pub(crate) fn set_source_syntax_context(
        &mut self,
        language: Option<&'static str>,
        context: Option<SourceSyntaxContext>,
    ) {
        let next = language.map(SharedString::from);
        let same_context = match (&self.source_syntax_context, &context) {
            (Some(current), Some(next)) => current.same_identity(next),
            (None, None) => true,
            _ => false,
        };
        if self.source_syntax_language == next && same_context {
            return;
        }
        self.source_syntax_language = next;
        self.source_syntax_context = context;
        self.sync_code_highlight();
    }

    pub(super) fn sync_code_highlight(&mut self) {
        let visible_text = self.render_cache.visible_text();
        self.code_highlight = match &self.record.kind {
            BlockKind::CodeBlock { language } => {
                highlight_code_block(language.as_deref().map(|value| &**value), visible_text)
            }
            _ => {
                let language = self.source_syntax_language.as_deref().map(|value| &**value);
                self.source_syntax_context
                    .as_ref()
                    .map(|context| project_highlight_result(context.highlight(visible_text)))
                    .or_else(|| highlight_code_block(language, visible_text))
            }
        };
    }

    pub(crate) fn code_language_text(&self) -> &str {
        match &self.record.kind {
            BlockKind::CodeBlock {
                language: Some(language),
            } => language.as_ref(),
            _ => "",
        }
    }

    pub(crate) fn code_language_cursor_offset(&self) -> usize {
        if self.code_language_selection_reversed {
            self.code_language_selected_range.start
        } else {
            self.code_language_selected_range.end
        }
    }

    pub(crate) fn code_language_range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        Self::utf8_range_to_utf16_in(self.code_language_text(), range)
    }

    pub(crate) fn code_language_range_from_utf16(
        &self,
        range_utf16: &Range<usize>,
    ) -> Range<usize> {
        Self::utf16_range_to_utf8_in(self.code_language_text(), range_utf16)
    }

    pub(crate) fn previous_code_language_boundary(&self, offset: usize) -> usize {
        self.code_language_text()
            .grapheme_indices(true)
            .rev()
            .find_map(|(idx, _)| (idx < offset).then_some(idx))
            .unwrap_or(0)
    }

    pub(crate) fn next_code_language_boundary(&self, offset: usize) -> usize {
        self.code_language_text()
            .grapheme_indices(true)
            .find_map(|(idx, _)| (idx > offset).then_some(idx))
            .unwrap_or(self.code_language_text().len())
    }

    pub(crate) fn move_code_language_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        let clamped = offset.min(self.code_language_text().len());
        self.code_language_selected_range = clamped..clamped;
        self.code_language_selection_reversed = false;
        self.code_language_marked_range = None;
        self.cursor_blink_epoch = Instant::now();
        cx.notify();
    }

    pub(crate) fn select_code_language_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        let clamped = offset.min(self.code_language_text().len());
        if self.code_language_selection_reversed {
            self.code_language_selected_range.start = clamped;
        } else {
            self.code_language_selected_range.end = clamped;
        }
        if self.code_language_selected_range.end < self.code_language_selected_range.start {
            self.code_language_selection_reversed = !self.code_language_selection_reversed;
            self.code_language_selected_range =
                self.code_language_selected_range.end..self.code_language_selected_range.start;
        }
        self.cursor_blink_epoch = Instant::now();
        cx.notify();
    }

    /// 替换 Live 代码块语言文本；只读投影没有事务所有权，因此任何输入路径都必须
    /// 在这里停止，而不能依赖上层控件是否恰好禁用。
    /// Preserves existing keyboard behavior by classifying direct language edits as coalescible.
    pub(crate) fn replace_code_language_text_in_range(
        &mut self,
        range: Range<usize>,
        new_text: &str,
        selected_range_relative: Option<Range<usize>>,
        mark_inserted_text: bool,
        cx: &mut Context<Self>,
    ) {
        self.replace_code_language_text_in_range_with_undo(
            range,
            new_text,
            selected_range_relative,
            mark_inserted_text,
            UndoCaptureKind::CoalescibleText,
            cx,
        );
    }

    /// Lets IME commits seal one transaction without a second coalescible undo capture.
    pub(crate) fn replace_code_language_text_in_range_with_undo(
        &mut self,
        range: Range<usize>,
        new_text: &str,
        selected_range_relative: Option<Range<usize>>,
        mark_inserted_text: bool,
        undo_kind: UndoCaptureKind,
        cx: &mut Context<Self>,
    ) {
        if self.is_read_only() || !self.kind().is_code_block() {
            return;
        }

        self.prepare_undo_capture(undo_kind, cx);

        let current = self.code_language_text().to_string();
        let range = range.start.min(current.len())..range.end.min(current.len());
        let inserted = new_text.replace("\r\n", " ").replace(['\r', '\n'], " ");
        let mut raw_next = String::new();
        raw_next.push_str(&current[..range.start]);
        raw_next.push_str(&inserted);
        raw_next.push_str(&current[range.end..]);

        let trimmed_start = raw_next.len() - raw_next.trim_start().len();
        let normalized = normalize_code_language_input(&raw_next);
        let normalized_len = normalized.len();
        let raw_inserted_end = range.start + inserted.len();
        let next_cursor = selected_range_relative
            .as_ref()
            .map(|relative| range.start + relative.end)
            .unwrap_or(raw_inserted_end)
            .saturating_sub(trimmed_start)
            .min(normalized_len);
        let next_selection = selected_range_relative
            .as_ref()
            .map(|relative| {
                let start = (range.start + relative.start)
                    .saturating_sub(trimmed_start)
                    .min(normalized_len);
                let end = (range.start + relative.end)
                    .saturating_sub(trimmed_start)
                    .min(normalized_len);
                start.min(end)..start.max(end)
            })
            .unwrap_or_else(|| next_cursor..next_cursor);
        let next_marked = if mark_inserted_text && !inserted.is_empty() {
            let start = range
                .start
                .saturating_sub(trimmed_start)
                .min(normalized_len);
            let end = raw_inserted_end
                .saturating_sub(trimmed_start)
                .min(normalized_len);
            (start < end).then_some(start..end)
        } else {
            None
        };

        let old_language = match &self.record.kind {
            BlockKind::CodeBlock { language } => language.clone(),
            _ => None,
        };
        self.record.kind = BlockKind::CodeBlock {
            language: (!normalized.is_empty()).then(|| SharedString::from(normalized)),
        };
        self.code_language_selected_range = next_selection;
        self.code_language_selection_reversed = selected_range_relative
            .as_ref()
            .is_some_and(|relative| relative.end < relative.start);
        self.code_language_marked_range = next_marked;
        self.cursor_blink_epoch = Instant::now();
        self.sync_code_highlight();

        let next_language = match &self.record.kind {
            BlockKind::CodeBlock { language } => language.clone(),
            _ => None,
        };
        if old_language != next_language {
            cx.emit(BlockEvent::Changed);
        }
        cx.notify();
    }

    pub(crate) fn code_language_index_for_mouse_position(&self, position: Point<Pixels>) -> usize {
        let text = self.code_language_text();
        if text.is_empty() {
            return 0;
        }

        let (Some(bounds), Some(line)) = (
            self.code_language_last_bounds.as_ref(),
            self.code_language_last_layout.as_ref(),
        ) else {
            return 0;
        };
        if position.x <= bounds.left() {
            return 0;
        }
        if position.x >= bounds.right() {
            return text.len();
        }
        line.closest_index_for_x(position.x - bounds.left())
    }

    pub(crate) fn reset_code_language_input_layout(&mut self) {
        self.code_language_last_layout = None;
        self.code_language_last_bounds = None;
        self.code_language_is_selecting = false;
    }
}
