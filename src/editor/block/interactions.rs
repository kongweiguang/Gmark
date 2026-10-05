// @author kongweiguang

//! Action handlers dispatched by GPUI's action system when bound keys are
//! pressed on a focused block.  Each handler maps to a named action declared
//! in [`crate::components::actions`] and delegates structural changes to the
//! parent editor via `BlockEvent` emissions.

use std::time::Duration;

use gpui::*;

use super::CollapsedCaretAffinity;
use super::{
    Block, BlockEvent, BlockHostAction, BlockKind, InlineFormat, InlineLinkHit, InlineTextTree,
    MermaidViewMode, PastedImageSource, UndoCaptureKind, element,
};
use crate::components::markdown::paste::should_split_plain_multiline_paste;
use crate::components::{
    BlockDown, BlockUp, BoldSelection, CodeSelection, Copy, CopyAsMarkdown, Cut, Delete,
    DeleteBack, DismissTransientUi, End, ExitCodeBlock, FindInDocument, FindNext, FindPrevious,
    FocusNext, FocusPrev, GoToLine, HighlightSelection, Home, IndentBlock, InlineMathSelection,
    ItalicSelection, JumpToBottom, JumpToTop, LinkSelection, MoveLeft, MoveRight, Newline,
    OutdentBlock, PageDown, PageUp, Paste, PasteAsPlainText, Redo, SaveDocument, SelectAll,
    SelectEnd, SelectHome, SelectLeft, SelectRight, StrikethroughSelection, SubscriptSelection,
    SuperscriptSelection, UnderlineSelection, Undo, WordDeleteBack, WordDeleteForward,
    WordMoveLeft, WordMoveRight, WordSelectLeft, WordSelectRight,
};

impl Block {
    fn dispatch_host_action(
        &mut self,
        action: BlockHostAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(handler) = self.host_action_handler() else {
            return;
        };
        cx.stop_propagation();
        handler(action, window, cx);
    }

    /// GPUI resolves bound navigation keys into actions before emitting the raw
    /// key-down event. Contextual menus must therefore consume those actions at
    /// the focused block, otherwise the normal caret action destroys the menu
    /// anchor before the editor-level key handler can observe it.
    fn handle_contextual_editing_action(
        &mut self,
        key: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Ok(keystroke) = Keystroke::parse(key) else {
            return false;
        };
        let event = KeyDownEvent {
            keystroke,
            is_held: false,
        };
        let handled = self.handle_code_language_menu_key(&event, window, cx)
            || self.handle_selection_toolbar_key(&event, window, cx)
            || self.handle_slash_menu_key(&event, cx);
        if handled {
            cx.stop_propagation();
        }
        handled
    }

    pub(crate) fn on_host_save(
        &mut self,
        _: &SaveDocument,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_host_action(BlockHostAction::Save, window, cx);
    }

    pub(crate) fn on_host_submit(
        &mut self,
        _: &Newline,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_host_action(
            BlockHostAction::Submit(self.shared_display_text()),
            window,
            cx,
        );
    }

    pub(crate) fn on_host_undo(&mut self, _: &Undo, window: &mut Window, cx: &mut Context<Self>) {
        self.dispatch_host_action(BlockHostAction::Undo, window, cx);
    }

    pub(crate) fn on_host_redo(&mut self, _: &Redo, window: &mut Window, cx: &mut Context<Self>) {
        self.dispatch_host_action(BlockHostAction::Redo, window, cx);
    }

    pub(crate) fn on_host_find(
        &mut self,
        _: &FindInDocument,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_host_action(BlockHostAction::Find, window, cx);
    }

    pub(crate) fn on_host_find_next(
        &mut self,
        _: &FindNext,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_host_action(BlockHostAction::FindNext, window, cx);
    }

    pub(crate) fn on_host_find_previous(
        &mut self,
        _: &FindPrevious,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_host_action(BlockHostAction::FindPrevious, window, cx);
    }

    pub(crate) fn on_host_go_to_line(
        &mut self,
        _: &GoToLine,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_host_action(BlockHostAction::GoToLine, window, cx);
    }

    pub(crate) fn on_host_page_up(
        &mut self,
        _: &PageUp,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_host_action(BlockHostAction::PageUp, window, cx);
    }

    pub(crate) fn on_host_page_down(
        &mut self,
        _: &PageDown,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_host_action(BlockHostAction::PageDown, window, cx);
    }

    pub(crate) fn on_host_jump_to_top(
        &mut self,
        _: &JumpToTop,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_host_action(BlockHostAction::JumpToTop, window, cx);
    }

    pub(crate) fn on_host_jump_to_bottom(
        &mut self,
        _: &JumpToBottom,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_host_action(BlockHostAction::JumpToBottom, window, cx);
    }

    pub(crate) fn on_host_dismiss(
        &mut self,
        _: &DismissTransientUi,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_host_action(BlockHostAction::DismissTransientUi, window, cx);
    }

    fn pasted_image_source_from_clipboard(item: &ClipboardItem) -> Option<PastedImageSource> {
        item.entries().iter().find_map(|entry| match entry {
            ClipboardEntry::Image(image) => Some(PastedImageSource::ClipboardImage(image.clone())),
            ClipboardEntry::String(_) => None,
        })
    }

    fn pasted_image_source_from_text(text: &str) -> Option<PastedImageSource> {
        let trimmed = text.trim();
        if trimmed.is_empty() || trimmed.contains('\n') || trimmed.contains('\r') {
            return None;
        }

        // 这里只做无副作用的路径语法分类；文件存在性、metadata 与大小限制在
        // Editor 的后台 materialize 阶段验证，避免剪贴板回调被 UNC 或慢盘阻塞。
        let path = Self::pasted_local_path_from_text_item(trimmed)?;
        if Self::is_supported_local_image_path(&path) {
            Some(PastedImageSource::LocalPath(path))
        } else {
            Some(PastedImageSource::LocalResource(path))
        }
    }

    /// Parses a single clipboard text item as a local file path.
    ///
    /// Windows file-copy paste reaches GPUI as a plain drive-letter or UNC path;
    /// classify its lexical shape before URL parsing because `url::Url` treats
    /// a drive letter as a URL scheme. The backend owns existence validation.
    fn pasted_local_path_from_text_item(text: &str) -> Option<std::path::PathBuf> {
        let unquoted = text
            .strip_prefix('"')
            .and_then(|rest| rest.strip_suffix('"'))
            .unwrap_or(text);
        let direct_path = std::path::PathBuf::from(unquoted);
        let path = if direct_path.is_absolute() {
            direct_path
        } else if let Ok(url) = url::Url::parse(unquoted) {
            if url.scheme().eq_ignore_ascii_case("file") {
                url.to_file_path().ok()?
            } else {
                return None;
            }
        } else {
            return None;
        };
        Some(path)
    }

    fn is_supported_local_image_path(path: &std::path::Path) -> bool {
        let Some(ext) = path.extension().and_then(|ext| ext.to_str()) else {
            return false;
        };
        matches!(
            ext.to_ascii_lowercase().as_str(),
            "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "bmp" | "tif" | "tiff"
        )
    }

    pub(crate) fn paste_resource_split(&self) -> (InlineTextTree, InlineTextTree) {
        let clean_selected = self.selection_clean_range();
        let (leading, tail) = self.record.title.split_at(clean_selected.start);
        let (_, trailing) = tail.split_at(clean_selected.end.saturating_sub(clean_selected.start));
        (leading, trailing)
    }

    fn is_leaf_quote(&self) -> bool {
        self.kind() == BlockKind::Quote
            && self.children.is_empty()
            && !self.display_text().contains('\n')
    }

    fn is_leaf_callout(&self) -> bool {
        matches!(self.kind(), BlockKind::Callout(_)) && self.children.is_empty()
    }

    fn is_empty_leaf_quote(&self) -> bool {
        self.is_leaf_quote() && self.selected_range.is_empty() && self.is_empty()
    }

    fn downgrade_leaf_callout_to_quote_at_start(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.is_leaf_callout() || !self.selected_range.is_empty() || self.cursor_offset() != 0 {
            return false;
        }

        let BlockKind::Callout(variant) = self.kind() else {
            return false;
        };
        let header_markdown = variant.header_markdown(&self.record.title.serialize_markdown());
        self.record.kind = BlockKind::Quote;
        self.record
            .set_title(InlineTextTree::from_markdown(&header_markdown));
        self.sync_edit_mode_from_kind();
        self.sync_render_cache();
        self.assign_collapsed_selection_offset(0, CollapsedCaretAffinity::Default, None);
        self.marked_range = None;
        self.cursor_blink_epoch = std::time::Instant::now();
        cx.emit(BlockEvent::Changed);
        cx.notify();
        true
    }

    fn downgrade_empty_leaf_quote_to_paragraph(&mut self, cx: &mut Context<Self>) -> bool {
        if self.is_empty_leaf_quote() {
            self.convert_to_paragraph(cx);
            return true;
        }
        false
    }

    fn table_append_column_should_stay_visible(&self) -> bool {
        self.table_append_column_edge_hovered
            || self.table_append_column_zone_hovered
            || self.table_append_column_button_hovered
    }

    fn table_append_row_should_stay_visible(&self) -> bool {
        self.table_append_row_edge_hovered
            || self.table_append_row_zone_hovered
            || self.table_append_row_button_hovered
    }

    fn schedule_table_append_column_close(&mut self, cx: &mut Context<Self>) {
        if !self.table_append_column_hovered {
            return;
        }

        self.table_append_column_close_task = Some(cx.spawn(
            async |this: WeakEntity<Block>, cx: &mut AsyncApp| {
                cx.background_executor()
                    .timer(Duration::from_millis(120))
                    .await;
                let _ = this.update(cx, |block, cx| {
                    block.table_append_column_close_task = None;
                    if !block.table_append_column_should_stay_visible() {
                        block.table_append_column_hovered = false;
                        cx.notify();
                    }
                });
            },
        ));
    }

    fn schedule_table_append_row_close(&mut self, cx: &mut Context<Self>) {
        if !self.table_append_row_hovered {
            return;
        }

        self.table_append_row_close_task = Some(cx.spawn(
            async |this: WeakEntity<Block>, cx: &mut AsyncApp| {
                cx.background_executor()
                    .timer(Duration::from_millis(120))
                    .await;
                let _ = this.update(cx, |block, cx| {
                    block.table_append_row_close_task = None;
                    if !block.table_append_row_should_stay_visible() {
                        block.table_append_row_hovered = false;
                        cx.notify();
                    }
                });
            },
        ));
    }

    fn set_table_append_column_hover_part(
        &mut self,
        edge_hovered: Option<bool>,
        zone_hovered: Option<bool>,
        button_hovered: Option<bool>,
        cx: &mut Context<Self>,
    ) {
        let mut changed = false;
        if let Some(edge_hovered) = edge_hovered
            && self.table_append_column_edge_hovered != edge_hovered
        {
            self.table_append_column_edge_hovered = edge_hovered;
            changed = true;
        }
        if let Some(zone_hovered) = zone_hovered
            && self.table_append_column_zone_hovered != zone_hovered
        {
            self.table_append_column_zone_hovered = zone_hovered;
            changed = true;
        }
        if let Some(button_hovered) = button_hovered
            && self.table_append_column_button_hovered != button_hovered
        {
            self.table_append_column_button_hovered = button_hovered;
            changed = true;
        }

        if self.table_append_column_should_stay_visible() {
            self.table_append_column_close_task = None;
            if !self.table_append_column_hovered {
                self.table_append_column_hovered = true;
                changed = true;
            }
        } else if self.table_append_column_hovered && self.table_append_column_close_task.is_none()
        {
            self.schedule_table_append_column_close(cx);
        }

        if changed {
            cx.notify();
        }
    }

    fn set_table_append_row_hover_part(
        &mut self,
        edge_hovered: Option<bool>,
        zone_hovered: Option<bool>,
        button_hovered: Option<bool>,
        cx: &mut Context<Self>,
    ) {
        let mut changed = false;
        if let Some(edge_hovered) = edge_hovered
            && self.table_append_row_edge_hovered != edge_hovered
        {
            self.table_append_row_edge_hovered = edge_hovered;
            changed = true;
        }
        if let Some(zone_hovered) = zone_hovered
            && self.table_append_row_zone_hovered != zone_hovered
        {
            self.table_append_row_zone_hovered = zone_hovered;
            changed = true;
        }
        if let Some(button_hovered) = button_hovered
            && self.table_append_row_button_hovered != button_hovered
        {
            self.table_append_row_button_hovered = button_hovered;
            changed = true;
        }

        if self.table_append_row_should_stay_visible() {
            self.table_append_row_close_task = None;
            if !self.table_append_row_hovered {
                self.table_append_row_hovered = true;
                changed = true;
            }
        } else if self.table_append_row_hovered && self.table_append_row_close_task.is_none() {
            self.schedule_table_append_row_close(cx);
        }

        if changed {
            cx.notify();
        }
    }

    /// If the code block's last line is a bare fence (three or more backticks
    /// or tildes, no info string), returns the byte offset to cut from so the
    /// whole line is removed; otherwise `None`.
    fn trailing_code_fence_line_start(&self) -> Option<usize> {
        let text = self.display_text();
        let line_start = text.rfind('\n').map(|idx| idx + 1).unwrap_or(0);
        let is_bare_fence = BlockKind::parse_code_fence_opening(&text[line_start..])
            .is_some_and(|fence| fence.language.is_none());
        // Cut from the preceding newline too, unless the fence is the only line.
        is_bare_fence.then(|| line_start.saturating_sub(1))
    }

    /// 普通段落的拆分由父编辑器一次提交；原块先刷新布局，不能先发布截断正文生成半次历史。
    pub(crate) fn on_newline(&mut self, _: &Newline, window: &mut Window, cx: &mut Context<Self>) {
        if self.host_submit_enabled() {
            self.dispatch_host_action(
                BlockHostAction::Submit(self.shared_display_text()),
                window,
                cx,
            );
            return;
        }
        if self.handle_contextual_editing_action("enter", window, cx) {
            return;
        }
        if self.commit_slash_menu(cx) {
            return;
        }
        if self.record.resource.is_some() && !self.uses_raw_text_editing() {
            if let Some(record) = self
                .resource_runtime()
                .map(|runtime| runtime.record.clone())
                .or_else(|| self.record.resource.clone())
            {
                self.request_resource_open(&record, cx);
            }
            return;
        }
        if self.image_selected && self.showing_rendered_image() {
            self.request_image_edit_expansion();
            self.sync_image_focus_state(self.focus_handle.is_focused(window));
            cx.notify();
            return;
        }
        // Enter is ordered from special editors to rich-text splitting:
        // table/source/code/quote-like blocks keep local newline semantics,
        // while normal rendered blocks emit an editor-level split request.
        if self.is_table_cell() {
            cx.emit(BlockEvent::RequestTableCellMoveVertical { delta: 1 });
            return;
        }

        if self.editor_selection_range.is_some() {
            cx.emit(BlockEvent::RequestReplaceCrossBlockSelection {
                text: "\n".to_string(),
                selected_range_relative: None,
                mark_inserted_text: false,
                undo_kind: UndoCaptureKind::NonCoalescible,
            });
            return;
        }

        if self.is_source_raw_mode() {
            if !self.selected_range.is_empty() {
                self.replace_text_in_range(None, "", window, cx);
            }
            self.replace_text_in_range(None, "\n", window, cx);
            return;
        }

        if self.kind() == BlockKind::Paragraph
            && self.selected_range.is_empty()
            && self.cursor_offset() == self.visible_len()
            && BlockKind::parse_separator_line(self.display_text())
            // A dash run is also a setext underline; defer it to the editor so a
            // preceding paragraph can become a heading (the editor falls back to
            // a separator when there is no heading target).
            && BlockKind::parse_setext_underline(self.display_text()).is_none()
        {
            self.convert_to_separator(cx);
            cx.emit(BlockEvent::RequestNewline {
                trailing: InlineTextTree::plain(String::new()),
                source_already_mutated: true,
                insert_before: false,
            });
            return;
        }

        // `$$` then Enter opens a display-math block. Keying off the caret sitting
        // right after a leading `$$` (rather than the line being exactly `$$`)
        // means it also fires after pressing Home on an existing line and typing
        // the fence in front of a formula: the rest of the line becomes the math
        // body instead of being split off into a new paragraph.
        if self.kind() == BlockKind::Paragraph
            && self.selected_range.is_empty()
            && self.cursor_offset() == "$$".len()
            && self.display_text().starts_with("$$")
        {
            let body = self.display_text()["$$".len()..].to_string();
            self.enter_math_block(&body, cx);
            return;
        }

        if self.kind() == BlockKind::Paragraph
            && self.selected_range.is_empty()
            && self.cursor_offset() == self.visible_len()
            && let Some(fence) = BlockKind::parse_code_fence_opening(self.display_text())
        {
            if crate::components::is_mermaid_info_string(
                fence.language.as_deref().map(|language| &**language),
            ) {
                self.enter_mermaid_block(fence, cx);
            } else {
                self.enter_code_block(fence.language, cx);
            }
            return;
        }

        if self.kind().is_separator() {
            cx.emit(BlockEvent::RequestNewline {
                trailing: InlineTextTree::plain(String::new()),
                source_already_mutated: false,
                insert_before: false,
            });
            return;
        }

        if self.kind().is_list_item() && self.selected_range.is_empty() && self.is_empty() {
            cx.emit(BlockEvent::RequestOutdent);
            return;
        }

        if self.kind() == BlockKind::Quote {
            if !self.selected_range.is_empty() {
                self.replace_text_in_range(None, "", window, cx);
            }
            self.prepare_undo_capture(UndoCaptureKind::NonCoalescible, cx);
            self.replace_text_in_range(None, "\n", window, cx);
            return;
        }

        if matches!(self.kind(), BlockKind::Callout(_)) {
            cx.emit(BlockEvent::RequestEnterCalloutBody);
            return;
        }

        // In a code block, Enter inserts a newline into the block content
        // rather than splitting the block.  Pressing Enter on an empty
        // code block exits back to a paragraph.
        if self.kind().is_code_block() {
            if self.selected_range.is_empty() && self.is_empty() {
                self.convert_to_paragraph(cx);
                return;
            }
            // Typing a bare closing fence on the last line and pressing Enter
            // leaves the block, matching source mode.
            if self.selected_range.is_empty()
                && self.cursor_offset() == self.visible_len()
                && let Some(fence_start) = self.trailing_code_fence_line_start()
            {
                let fence_end = self.visible_len();
                self.prepare_undo_capture(UndoCaptureKind::NonCoalescible, cx);
                self.replace_text_in_visible_range(fence_start..fence_end, "", None, false, cx);
                cx.emit(BlockEvent::RequestNewline {
                    trailing: InlineTextTree::plain(String::new()),
                    source_already_mutated: true,
                    insert_before: false,
                });
                return;
            }
            if !self.selected_range.is_empty() {
                self.replace_text_in_range(None, "", window, cx);
            }
            self.prepare_undo_capture(UndoCaptureKind::NonCoalescible, cx);
            self.replace_text_in_range(None, "\n", window, cx);
            return;
        }

        if self.selected_range.is_empty() && self.cursor_offset() == 0 && !self.is_empty() {
            // 块首回车插入同级空块，当前块的类型、内容和子树必须保持为一个整体。
            cx.emit(BlockEvent::RequestNewline {
                trailing: InlineTextTree::plain(String::new()),
                source_already_mutated: false,
                insert_before: true,
            });
            return;
        }

        if self.collapsed_caret_inherits_inline_code_style() {
            self.prepare_undo_capture(UndoCaptureKind::NonCoalescible, cx);
            self.replace_text_in_range(None, "\n", window, cx);
            return;
        }

        if !self.selected_range.is_empty() {
            self.replace_text_in_range(None, "", window, cx);
        }

        let cursor = self.cursor_offset();
        let (leading, trailing) = self.split_title(cursor);
        self.prepare_undo_capture(UndoCaptureKind::NonCoalescible, cx);
        self.record.set_title(leading);
        self.sync_edit_mode_from_kind();
        self.sync_render_cache();
        self.cursor_blink_epoch = std::time::Instant::now();
        self.vertical_motion_x = None;
        cx.notify();
        let cursor = self.visible_len();
        self.assign_collapsed_selection_offset(cursor, CollapsedCaretAffinity::Default, None);
        self.marked_range = None;
        cx.emit(BlockEvent::RequestNewline {
            trailing,
            source_already_mutated: true,
            insert_before: false,
        });
    }

    /// 删除候选所属文字前等待原会话终态，并让只读保护覆盖结构降级与合并。
    pub(crate) fn on_delete_back(
        &mut self,
        _: &DeleteBack,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.guard_input_command(
            crate::components::block::BlockInputCommand::DeleteBack,
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

        if self.try_delete_empty_auto_pair(cx) {
            return;
        }
        if self.is_table_cell() {
            if self.selected_range.is_empty() {
                let previous = self.previous_boundary(self.cursor_offset());
                if previous == self.cursor_offset() {
                    return;
                }
                self.select_to(previous, cx);
            }
            self.replace_text_in_range(None, "", window, cx);
            return;
        }

        if self.is_source_raw_mode() {
            if self.selected_range.is_empty() {
                self.select_to(self.previous_boundary(self.cursor_offset()), cx);
            }
            self.replace_text_in_range(None, "", window, cx);
            return;
        }

        if self.selected_range.is_empty() && self.cursor_offset() == 0 {
            if self.kind() == BlockKind::Paragraph && self.is_direct_list_child() && self.is_empty()
            {
                cx.emit(BlockEvent::RequestOutdent);
                return;
            }
            if self.is_nested_list_item() {
                cx.emit(BlockEvent::RequestDowngradeNestedListItemToChildParagraph);
                return;
            }
            match self.kind() {
                BlockKind::BulletedListItem
                | BlockKind::TaskListItem { .. }
                | BlockKind::NumberedListItem => {
                    cx.emit(BlockEvent::RequestOutdent);
                    return;
                }
                BlockKind::Heading { .. } => {
                    self.convert_to_paragraph(cx);
                    return;
                }
                BlockKind::Quote => {
                    if self.is_leaf_quote() {
                        self.convert_to_paragraph(cx);
                    }
                    return;
                }
                BlockKind::Callout(_) => {
                    if self.downgrade_leaf_callout_to_quote_at_start(cx) {
                        return;
                    }
                    return;
                }
                BlockKind::Separator => {
                    self.convert_to_paragraph(cx);
                    return;
                }
                BlockKind::CodeBlock { .. } => {
                    self.convert_to_paragraph(cx);
                    return;
                }
                _ => {}
            }
        }

        if self.downgrade_leaf_callout_to_quote_at_start(cx)
            || self.downgrade_empty_leaf_quote_to_paragraph(cx)
        {
            return;
        }

        if self.selected_range.is_empty() && self.display_text().is_empty() {
            cx.emit(BlockEvent::RequestDelete);
            return;
        }

        if self.selected_range.is_empty() && self.cursor_offset() == 0 {
            cx.emit(BlockEvent::RequestMergeIntoPrev {
                content: self.record.title.clone(),
            });
            return;
        }

        if self.selected_range.is_empty() {
            self.select_to(self.previous_boundary(self.cursor_offset()), cx);
        }
        self.replace_text_in_range(None, "", window, cx);
    }
}

#[path = "interactions_parts/deletion.rs"]
mod deletion;
#[path = "interactions_parts/line_operations.rs"]
mod line_operations;
#[path = "interactions_parts/navigation.rs"]
mod navigation;
#[path = "interactions_parts/pointer_selection.rs"]
mod pointer_selection;

#[cfg(test)]
#[path = "../../../tests/unit/components/block/interactions.rs"]
mod tests;
