// @author kongweiguang

use super::*;

#[path = "clipboard_source_projection.rs"]
mod source_projection;

use source_projection::{VirtualizedClipboardSelection, virtualized_clipboard_selection};

#[cfg(test)]
std::thread_local! {
    static FAIL_RICH_CLIPBOARD_WRITE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static LAST_RICH_CLIPBOARD_WRITE: std::cell::RefCell<Option<(String, String)>> = const { std::cell::RefCell::new(None) };
}

/// Uses a deterministic test adapter while keeping production writes isolated to the native clipboard.
fn write_rich_system_clipboard(html: &str, plain_text: &str) -> bool {
    #[cfg(test)]
    {
        let accepted = !FAIL_RICH_CLIPBOARD_WRITE.with(std::cell::Cell::get);
        if accepted {
            LAST_RICH_CLIPBOARD_WRITE.with(|capture| {
                *capture.borrow_mut() = Some((html.to_owned(), plain_text.to_owned()));
            });
        }
        accepted
    }
    #[cfg(not(test))]
    {
        let Ok(mut clipboard) = arboard::Clipboard::new() else {
            return false;
        };
        clipboard.set_html(html, Some(plain_text)).is_ok()
    }
}

/// Preserves tabular structure for rich paste targets while escaping every cell value.
fn table_tsv_html(tsv: &str) -> String {
    let rows = tsv
        .split('\n')
        .map(|row| {
            let cells = row
                .split('\t')
                .map(|cell| format!("<td>{}</td>", gmark_markdown::escape_html(cell)))
                .collect::<String>();
            format!("<tr>{cells}</tr>")
        })
        .collect::<String>();
    format!("<table><tbody>{rows}</tbody></table>")
}

/// Slices display coordinates without allowing malformed or split UTF-8 endpoints to panic.
fn visible_slice(text: &str, range: Range<usize>) -> Option<&str> {
    let mut start = range.start.min(text.len());
    let mut end = range.end.min(text.len());
    while start > 0 && !text.is_char_boundary(start) {
        start -= 1;
    }
    while end < text.len() && !text.is_char_boundary(end) {
        end += 1;
    }
    (start <= end).then(|| &text[start..end])
}

/// Converts the active display range before slicing clean inline text, since focused projections may expose Markdown delimiters.
fn selected_clean_text(block: &Block, current_range: Range<usize>) -> Option<String> {
    let clean_range = block.current_to_clean_range(current_range);
    let visible_text = block.record.title.visible_text();
    visible_slice(&visible_text, clean_range).map(ToOwned::to_owned)
}

/// Serializes an atomic native table as visible cells, keeping TSV delimiters out of cell content.
fn visible_table_tsv(table: &crate::components::TableData) -> String {
    let mut rows = Vec::with_capacity(table.rows.len() + 1);
    rows.push(
        table
            .header
            .iter()
            .map(|cell| cell.visible_text().replace(['\t', '\n', '\r'], " "))
            .collect::<Vec<_>>()
            .join("\t"),
    );
    rows.extend(table.rows.iter().map(|row| {
        row.iter()
            .map(|cell| cell.visible_text().replace(['\t', '\n', '\r'], " "))
            .collect::<Vec<_>>()
            .join("\t")
    }));
    rows.join("\n")
}

impl Editor {
    /// Resolves virtual Main selections from their revision-bound source span so mounted endpoints never change copy semantics.
    fn virtualized_cross_block_selection(
        &self,
        surface: super::super::selection_surface::SelectionSurface,
        cx: &App,
    ) -> Option<VirtualizedClipboardSelection> {
        if surface != super::super::selection_surface::SelectionSurface::Main
            || self.virtual_surface.is_none()
        {
            return None;
        }
        let selection = self.normalized_cross_block_selection_for_surface(surface, cx)?;
        let source_range = self.cross_block_source_range_for_normalized(selection, cx)?;
        let source = self.source_document.snapshot().text();
        virtualized_clipboard_selection(&source, source_range)
    }

    /// Reads the last successful rich payload in this test thread without exposing it to production clipboard code.
    #[cfg(test)]
    pub(in crate::editor) fn take_rich_clipboard_write_for_test() -> Option<(String, String)> {
        LAST_RICH_CLIPBOARD_WRITE.with(|capture| capture.borrow_mut().take())
    }

    /// 终态后仍在原表面执行命令；跨块与表格入口优先于 Block 本地文字写入。
    pub(in crate::editor) fn replay_document_input_command(
        &mut self,
        surface: crate::editor::selection_surface::SelectionSurface,
        target: &Entity<Block>,
        command: crate::components::block::BlockInputCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use crate::components::block::BlockInputCommand;
        self.active_selection_surface = surface;
        target.update(cx, |block, _cx| block.focus_handle.focus(window));
        match command {
            BlockInputCommand::Cut => {
                if surface == crate::editor::selection_surface::SelectionSurface::Main
                    && matches!(self.view_mode, ViewMode::Source | ViewMode::Split)
                {
                    target.update(cx, |block, cx| {
                        block.replay_input_command(command, window, cx)
                    });
                } else {
                    self.on_cut_capture(&Cut, window, cx);
                }
            }
            BlockInputCommand::Paste => {
                let handled = cx
                    .read_from_clipboard()
                    .and_then(|item| item.text())
                    .is_some_and(|text| self.paste_table_cells_tsv(&text, cx));
                if !handled {
                    target.update(cx, |block, cx| {
                        block.replay_input_command(command, window, cx)
                    });
                }
            }
            BlockInputCommand::Delete
            | BlockInputCommand::DeleteBack
            | BlockInputCommand::WordDeleteBack
            | BlockInputCommand::WordDeleteForward => {
                if !self.document_surface_is_editable_for(surface) {
                    return;
                }
                if let Some(selection) = self.table_cell_rectangle_for_surface(surface) {
                    self.clear_table_cell_rectangle(selection, cx);
                } else if !self.delete_cross_block_selection(cx) {
                    target.update(cx, |block, cx| {
                        block.replay_input_command(command, window, cx)
                    });
                }
            }
            _ => {
                target.update(cx, |block, cx| {
                    block.replay_input_command(command, window, cx)
                });
            }
        }
    }

    /// Scopes a native clipboard failure to this test thread and restores nested prior state on exit.
    #[cfg(test)]
    pub(in crate::editor) fn with_rich_clipboard_write_failure_for_test<T>(
        run: impl FnOnce() -> T,
    ) -> T {
        struct RestoreFailure(bool);
        impl Drop for RestoreFailure {
            /// Restores the prior thread-local value even when the test body panics.
            fn drop(&mut self) {
                FAIL_RICH_CLIPBOARD_WRITE.with(|fail| fail.set(self.0));
            }
        }

        let previous = FAIL_RICH_CLIPBOARD_WRITE.with(|fail| fail.replace(true));
        let _restore = RestoreFailure(previous);
        run()
    }

    /// Reports a non-destructive clipboard failure through the existing short-lived editor notice.
    fn show_clipboard_write_failure(&mut self, cx: &mut Context<Self>) {
        self.show_pane_notice("无法写入剪贴板，文字已保留，请重试", cx);
    }

    /// Uses the selected display slices for plain text so Markdown syntax is never reinterpreted.
    pub(in crate::editor) fn on_copy_capture(
        &mut self,
        _: &Copy,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((surface, target)) = self.focused_document_target(window, cx) else {
            cx.propagate();
            return;
        };
        self.active_selection_surface = surface;
        if surface == super::super::selection_surface::SelectionSurface::Main
            && matches!(self.view_mode, ViewMode::Source | ViewMode::Split)
        {
            cx.propagate();
            return;
        }
        let virtualized = self.virtualized_cross_block_selection(surface, cx);
        if let Some(plain_text) = virtualized
            .as_ref()
            .map(|selection| selection.visible_text.clone())
            .or_else(|| self.selected_visible_text_for_target(surface, &target, cx))
        {
            let markdown = virtualized
                .as_ref()
                .map(|selection| selection.markdown.clone())
                .or_else(|| {
                    self.selected_markdown_text(cx)
                        .filter(|selection| !selection.is_empty())
                });
            let crosses_blocks = self
                .normalized_cross_block_selection_for_surface(surface, cx)
                .is_some_and(|selection| {
                    selection.start.entity_id != selection.end.entity_id
                        || selection.start_index != selection.end_index
                });
            if !self.write_selected_text_clipboard(
                markdown.as_deref(),
                &plain_text,
                &target,
                crosses_blocks,
                cx,
            ) {
                self.show_clipboard_write_failure(cx);
            }
            cx.stop_propagation();
            return;
        }
        if let Some(tsv) = self.selected_table_cells_tsv(cx) {
            cx.write_to_clipboard(ClipboardItem::new_string(tsv.clone()));
            if !write_rich_system_clipboard(&table_tsv_html(&tsv), &tsv) {
                self.show_clipboard_write_failure(cx);
            }
            cx.stop_propagation();
            return;
        }
        cx.propagate();
    }

    /// Extracts the caret owner's visible range or joins normalized cross-block display ranges.
    /// Prefers projected text because selected endpoint blocks may be unmounted.
    pub(in crate::editor) fn selected_visible_text_for_target(
        &self,
        surface: super::super::selection_surface::SelectionSurface,
        target: &Entity<Block>,
        cx: &App,
    ) -> Option<String> {
        if let Some(selection) = self.virtualized_cross_block_selection(surface, cx) {
            return Some(selection.visible_text);
        }
        if let Some(selection) = self.normalized_cross_block_selection_for_surface(surface, cx) {
            let visible = self.selection_surface_entities(surface);
            let mut selected = Vec::new();
            let (Some(start_index), Some(end_index)) = (selection.start_index, selection.end_index)
            else {
                return None;
            };
            for index in start_index..=end_index {
                let entity = visible.get(index)?;
                let block = entity.read(cx);
                let len = block.visible_len();
                let range = if start_index == end_index {
                    selection.start.offset.min(len)..selection.end.offset.min(len)
                } else if index == start_index {
                    selection.start.offset.min(len)..len
                } else if index == end_index {
                    0..selection.end.offset.min(len)
                } else {
                    0..len
                };
                let text = block
                    .record
                    .table
                    .as_ref()
                    .map(visible_table_tsv)
                    .or_else(|| selected_clean_text(block, range))?;
                selected.push(text);
            }
            let text = selected.join("\n\n");
            return (!text.is_empty()).then_some(text);
        }

        let block = target.read(cx);
        if block.selected_range.is_empty() {
            return None;
        }
        selected_clean_text(block, block.selected_range.clone()).filter(|text| !text.is_empty())
    }

    /// Writes visible text plus a context-aware HTML fragment for rich paste targets.
    fn write_selected_text_clipboard(
        &self,
        markdown: Option<&str>,
        plain_text: &str,
        target: &Entity<Block>,
        crosses_blocks: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        cx.write_to_clipboard(ClipboardItem::new_string(plain_text.to_owned()));
        let theme = cx.global::<crate::theme::ThemeManager>().current();
        let base_dir = self.file_path.as_deref().and_then(std::path::Path::parent);
        let target_is_code = target.read(cx).kind().is_code_block();
        let html = if target_is_code && !crosses_blocks {
            format!(
                "<pre><code>{}</code></pre>",
                gmark_markdown::escape_html(plain_text)
            )
        } else if let Some(markdown) = markdown {
            crate::adapters::export::render_clipboard_fragment_with_base_dir(
                markdown, theme, base_dir,
            )
        } else {
            format!("<p>{}</p>", gmark_markdown::escape_html(plain_text))
        };
        write_rich_system_clipboard(&html, plain_text)
    }

    /// Keeps the explicit Markdown command source-based even when ordinary Copy uses visible text.
    pub(in crate::editor) fn on_copy_as_markdown_capture(
        &mut self,
        _: &CopyAsMarkdown,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((surface, _target)) = self.focused_document_target(window, cx) else {
            cx.propagate();
            return;
        };
        self.active_selection_surface = surface;
        if surface == super::super::selection_surface::SelectionSurface::Main
            && matches!(self.view_mode, ViewMode::Source | ViewMode::Split)
        {
            cx.propagate();
            return;
        }
        let Some(markdown) = self
            .selected_markdown_text(cx)
            .or_else(|| self.cross_block_selected_markdown(cx))
        else {
            cx.propagate();
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(markdown));
        cx.stop_propagation();
    }

    /// Deletes only after native clipboard acceptance and treats Cut on Preview as copy-only.
    pub(in crate::editor) fn on_cut_capture(
        &mut self,
        _: &Cut,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((surface, target)) = self.focused_document_target(window, cx) else {
            cx.propagate();
            return;
        };
        self.active_selection_surface = surface;
        if self.document_surface_is_editable_for(surface)
            && target.update(cx, |block, cx| {
                block.guard_input_command(
                    crate::components::block::BlockInputCommand::Cut,
                    window,
                    cx,
                )
            })
        {
            cx.stop_propagation();
            return;
        }
        if surface == super::super::selection_surface::SelectionSurface::Main
            && matches!(self.view_mode, ViewMode::Source | ViewMode::Split)
        {
            cx.propagate();
            return;
        }
        let editable = self.document_surface_is_editable();
        let virtualized = self.virtualized_cross_block_selection(surface, cx);
        if let Some(plain_text) = virtualized
            .as_ref()
            .map(|selection| selection.visible_text.clone())
            .or_else(|| self.selected_visible_text_for_target(surface, &target, cx))
        {
            let markdown = virtualized
                .as_ref()
                .map(|selection| selection.markdown.clone())
                .or_else(|| {
                    self.selected_markdown_text(cx)
                        .filter(|selection| !selection.is_empty())
                });
            let crosses_blocks = self
                .normalized_cross_block_selection_for_surface(surface, cx)
                .is_some_and(|selection| {
                    selection.start.entity_id != selection.end.entity_id
                        || selection.start_index != selection.end_index
                });
            if !self.write_selected_text_clipboard(
                markdown.as_deref(),
                &plain_text,
                &target,
                crosses_blocks,
                cx,
            ) {
                self.show_clipboard_write_failure(cx);
                cx.stop_propagation();
                return;
            }
            if !editable {
                cx.stop_propagation();
            } else if self.cross_block_selected_markdown(cx).is_some() {
                self.delete_cross_block_selection(cx);
                cx.stop_propagation();
            } else {
                target.update(cx, |block, cx| {
                    block.on_delete_back(&DeleteBack, window, cx);
                });
                cx.stop_propagation();
            }
            return;
        }
        if let Some(tsv) = self.selected_table_cells_tsv(cx) {
            cx.write_to_clipboard(ClipboardItem::new_string(tsv.clone()));
            if !write_rich_system_clipboard(&table_tsv_html(&tsv), &tsv) {
                self.show_clipboard_write_failure(cx);
                cx.stop_propagation();
                return;
            }
            if !editable {
                cx.stop_propagation();
            } else if surface == super::super::selection_surface::SelectionSurface::Main
                && let Some(selection) = self.table_cell_rectangle_for_surface(surface)
            {
                self.clear_table_cell_rectangle(selection, cx);
                cx.stop_propagation();
            } else {
                cx.propagate();
            }
            return;
        }
        if !editable {
            cx.stop_propagation();
            return;
        }
        let Some(markdown) = self.cross_block_selected_markdown(cx) else {
            cx.propagate();
            return;
        };
        let Some(plain_text) = self.selected_visible_text_for_target(surface, &target, cx) else {
            cx.propagate();
            return;
        };
        if !self.write_selected_text_clipboard(Some(&markdown), &plain_text, &target, true, cx) {
            self.show_clipboard_write_failure(cx);
            cx.stop_propagation();
            return;
        }
        self.delete_cross_block_selection(cx);
        cx.stop_propagation();
    }

    /// Stops Paste at the read-only surface boundary before a table or block handler sees it.
    pub(in crate::editor) fn on_paste_capture(
        &mut self,
        _: &Paste,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((surface, target)) = self.focused_document_target(window, cx) else {
            cx.propagate();
            return;
        };
        self.active_selection_surface = surface;
        if self.document_surface_is_editable_for(surface)
            && target.update(cx, |block, cx| {
                block.guard_input_command(
                    crate::components::block::BlockInputCommand::Paste,
                    window,
                    cx,
                )
            })
        {
            cx.stop_propagation();
            return;
        }
        if !self.document_surface_is_editable() {
            cx.stop_propagation();
            return;
        }
        if surface == super::super::selection_surface::SelectionSurface::Main
            && matches!(self.view_mode, ViewMode::Source | ViewMode::Split)
        {
            cx.propagate();
            return;
        }
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            cx.propagate();
            return;
        };
        if self.paste_table_cells_tsv(&text, cx) {
            cx.stop_propagation();
        } else {
            cx.propagate();
        }
    }

    /// Leaves read-only selections intact and prevents Delete from reaching a Block mutation handler.
    pub(in crate::editor) fn on_delete_capture(
        &mut self,
        _: &Delete,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((surface, target)) = self.focused_document_target(window, cx) else {
            cx.propagate();
            return;
        };
        self.active_selection_surface = surface;
        if self.document_surface_is_editable_for(surface)
            && target.update(cx, |block, cx| {
                block.guard_input_command(
                    crate::components::block::BlockInputCommand::Delete,
                    window,
                    cx,
                )
            })
        {
            cx.stop_propagation();
            return;
        }
        if !self.document_surface_is_editable() {
            cx.stop_propagation();
            return;
        }
        if let Some(selection) = self.table_cell_rectangle_for_surface(surface)
            && surface == super::super::selection_surface::SelectionSurface::Main
        {
            self.clear_table_cell_rectangle(selection, cx);
            cx.stop_propagation();
            return;
        }
        if !self.delete_cross_block_selection(cx) {
            cx.propagate();
            return;
        }
        cx.stop_propagation();
    }

    /// Leaves Backspace inert on Preview even when the selection spans rendered blocks.
    pub(in crate::editor) fn on_delete_back_capture(
        &mut self,
        _: &DeleteBack,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((surface, target)) = self.focused_document_target(window, cx) else {
            cx.propagate();
            return;
        };
        self.active_selection_surface = surface;
        if self.document_surface_is_editable_for(surface)
            && target.update(cx, |block, cx| {
                block.guard_input_command(
                    crate::components::block::BlockInputCommand::DeleteBack,
                    window,
                    cx,
                )
            })
        {
            cx.stop_propagation();
            return;
        }
        if !self.document_surface_is_editable() {
            cx.stop_propagation();
            return;
        }
        if !self.delete_cross_block_selection(cx) {
            cx.propagate();
            return;
        }
        cx.stop_propagation();
    }
}
