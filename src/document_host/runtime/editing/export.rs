// @author kongweiguang

//! Selection export and line-edit event handling.

use super::*;

impl DocumentHost {
    pub(super) fn on_export_selection(
        &mut self,
        _: &ExportSelection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.start_selection_export(false, window, cx);
    }

    pub(crate) fn export_selection_from_menu(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.start_selection_export(false, window, cx);
    }

    pub(super) fn export_selection_as_utf8(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.start_selection_export(true, window, cx);
    }

    fn start_selection_export(
        &mut self,
        force_utf8: bool,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(range) = self.selected_source_byte_range() else {
            return;
        };
        let Some(document) = self.document.clone() else {
            return;
        };
        let default_dir = self
            .path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default();
        let file_name = self
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("selection");
        let suggested_name = if force_utf8 {
            format!("{file_name}.selection.utf8.txt")
        } else {
            format!("{file_name}.selection.txt")
        };
        let prompt = cx.prompt_for_new_path(&default_dir, Some(&suggested_name));
        if let Some(cancellation) = self.selection_export_cancellation.take() {
            cancellation.cancel();
        }
        let cancellation = SearchCancellation::default();
        self.selection_export_cancellation = Some(cancellation.clone());
        self.selection_export_generation = self.selection_export_generation.wrapping_add(1);
        let generation = self.selection_export_generation;
        let task_stamp = DocumentTaskStamp::capture(self, generation);
        let export_bytes = range.end.saturating_sub(range.start);
        self.metrics.export_requests = self.metrics.export_requests.saturating_add(1);
        self.selection_export_task = cx.spawn(async move |this, cx| {
            let path = match prompt.await {
                Ok(Ok(Some(path))) => path,
                Ok(Ok(None)) | Err(_) => {
                    let _ = this.update(cx, |view, _cx| {
                        if task_stamp.accepts_identity(view, view.selection_export_generation) {
                            view.selection_export_cancellation = None;
                        }
                    });
                    return;
                }
                Ok(Err(_error)) => {
                    let _ = this.update(cx, |view, cx| {
                        if task_stamp.accepts_identity(view, view.selection_export_generation) {
                            view.selection_export_cancellation = None;
                            view.error = Some(
                                cx.global::<I18nManager>()
                                    .strings()
                                    .large_document_text("error_export_selection")
                                    .into(),
                            );
                            cx.notify();
                        }
                    });
                    return;
                }
            };
            let result = cx
                .background_spawn(async move {
                    if cancellation.is_cancelled() {
                        return Err(PagedDocumentError::Cancelled);
                    }
                    let bytes = document.read_range(range.clone())?;
                    if cancellation.is_cancelled() {
                        return Err(PagedDocumentError::Cancelled);
                    }
                    gmark_document::atomic_write(&path, &bytes).map_err(|error| {
                        PagedDocumentError::Io {
                            path: path.clone(),
                            source: std::io::Error::other(error.to_string()),
                        }
                    })?;
                    Ok::<_, PagedDocumentError>(if force_utf8 {
                        "UTF-8".to_owned()
                    } else {
                        "UTF-8".to_owned()
                    })
                })
                .await;
            let _ = this.update(cx, |view, cx| {
                if !task_stamp.accepts_identity(view, view.selection_export_generation) {
                    return;
                }
                view.selection_export_cancellation = None;
                match result {
                    Ok(encoding) => {
                        view.metrics.exported_bytes =
                            view.metrics.exported_bytes.saturating_add(export_bytes);
                        view.coordinator.external_status = Some(
                            cx.global::<I18nManager>()
                                .strings()
                                .large_document_text("selection_exported_template")
                                .replace("{encoding}", &encoding)
                                .into(),
                        );
                        view.error = None;
                    }
                    Err(PagedDocumentError::UnrepresentableEncoding { encoding }) => {
                        view.error = Some(
                            cx.global::<I18nManager>()
                                .strings()
                                .large_document_text("selection_encoding_error_template")
                                .replace("{encoding}", &encoding)
                                .into(),
                        );
                    }
                    Err(error) => view.error = Some(localized_document_error(&error, cx)),
                }
                cx.notify();
            });
        });
    }

    /// Commit a Source row edit once and enqueue its immutable post-edit state;
    /// this keeps IME/finalized input from performing journal I/O in the UI lock.
    pub(super) fn on_line_edit_event(
        &mut self,
        block: Entity<Block>,
        event: &BlockEvent,
        cx: &mut Context<Self>,
    ) {
        if matches!(
            event,
            BlockEvent::ImeCompositionEnded { .. } | BlockEvent::ImeCompositionFinishFailed
        ) {
            self.on_source_ime_terminal(&block, event, cx);
            return;
        }
        if matches!(event, BlockEvent::SelectionChanged) {
            self.sync_selection_from_active_source_block(&block, cx);
            return;
        }
        if matches!(event, BlockEvent::RequestRenderedSelectAll)
            && self
                .active_edit
                .as_ref()
                .is_some_and(|active| active.block == block)
        {
            self.active_edit = None;
            self.select_source_lines(0..self.line_count(), false);
            self.sync_source_selection_visuals(cx);
            cx.notify();
            return;
        }
        if !matches!(event, BlockEvent::Changed) {
            return;
        }
        if self.saving || self.reloading {
            return;
        }
        if self
            .suppressed_line_edit_text
            .as_deref()
            .is_some_and(|expected| expected == block.read(cx).display_text())
        {
            self.suppressed_line_edit_text = None;
            return;
        }
        let Some(active) = self.active_edit.as_ref() else {
            return;
        };
        if active.block != block {
            return;
        }
        let range = active.range.clone();
        let ending = active.ending.clone();
        let active_line = active.line;
        let base_revision = active.base_revision;
        let Some(document) = self.document.clone() else {
            self.active_edit = None;
            self.error = Some(
                cx.global::<I18nManager>()
                    .strings()
                    .large_document_text("source_backend_unavailable")
                    .into(),
            );
            cx.notify();
            return;
        };
        if document.revision() != base_revision {
            self.reject_stale_source_edit(cx);
            return;
        }
        if block.read(cx).marked_range.is_some() {
            // IME composition belongs to the platform input transaction. Keep the
            // transient marked text in the mounted Source Block and commit it to
            // PieceTree/recovery only when the composition is finalized; otherwise
            // every pinyin candidate update would become a separate undo step.
            cx.notify();
            return;
        }
        let text = block.read(cx).display_text().to_owned();
        let caret_in_text = block.read(cx).selected_range.end.min(text.len());
        let replacement = format!("{text}{ending}");
        let recovery_selection = block.read(cx).selected_range.clone();
        let recovery_selection = u64::try_from(recovery_selection.start)
            .ok()
            .zip(u64::try_from(recovery_selection.end).ok())
            .and_then(|(start, end)| {
                let start =
                    SourceAnchor::new(range.start.checked_add(start)?, SourceAffinity::Before);
                let end = SourceAnchor::new(range.start.checked_add(end)?, SourceAffinity::After);
                Some(if block.read(cx).selection_reversed {
                    SourceSelection {
                        anchor: end,
                        head: start,
                    }
                } else {
                    SourceSelection {
                        anchor: start,
                        head: end,
                    }
                })
            });
        let edit_lines = self.document.as_ref().map(|document| {
            let start = document
                .line_for_offset(range.start.min(document.len()))
                .and_then(|line| usize::try_from(line).ok())
                .unwrap_or(active_line);
            let end = document
                .line_for_offset(range.end.min(document.len()))
                .and_then(|line| usize::try_from(line).ok())
                .unwrap_or(start);
            (start, end)
        });
        // 0 字节文件的首个 Changed 事件可能与旧 provisional 视图同帧到达；在提交
        // 前补做有界 session 安装，确保该字符进入权威文档，而不是被 document=None
        // 的早退静默吞掉。非空文件仍沿用后台索引，不会在 UI 线程扫描正文。
        if self.document.is_none() && self.probe.len == 0 {
            self.start_initial_index(cx);
        }
        let selection_before = document.source_selection();
        let selection_after = recovery_selection.unwrap_or_else(|| {
            SourceSelection::collapsed(
                range.start.saturating_add(caret_in_text as u64),
                SourceAffinity::After,
            )
        });
        let transaction_id = match document.next_transaction_id() {
            Ok(transaction_id) => transaction_id,
            Err(error) => {
                self.error = Some(error.to_string().into());
                self.reject_stale_source_edit(cx);
                return;
            }
        };
        let transaction = Transaction::new(
            DocumentRevision(base_revision),
            vec![SourceEdit::new(range.clone(), replacement.clone())],
        );
        match document.apply_transaction(
            transaction_id,
            transaction,
            selection_before,
            selection_after,
        ) {
            Ok(()) => {
                // Capture the post-edit snapshot outside the Controller lock;
                // the recovery worker performs the journal append later.
                let revision_before = base_revision;
                self.enqueue_recovery_transaction(
                    &document,
                    revision_before,
                    range.clone(),
                    replacement.as_str(),
                    recovery_selection,
                    DocumentViewId::source(),
                    cx,
                );
                let reanchored = text
                    .contains(['\r', '\n'])
                    .then(|| {
                        let caret_offset = range.start.saturating_add(caret_in_text as u64);
                        let line = document.line_for_offset(caret_offset.min(document.len()))?;
                        let line_range = document.line_range(line)?;
                        let requested = caret_offset
                            .saturating_sub(line_range.start)
                            .saturating_sub(MAX_RENDERED_LINE_BYTES / 2);
                        let windowed = read_bounded_line_window(&document, line, requested)
                            .ok()
                            .flatten()?;
                        let caret = usize::try_from(
                            caret_offset.saturating_sub(windowed.content_range.start),
                        )
                        .ok()?
                        .min(windowed.text.len());
                        let window_start = windowed
                            .content_range
                            .start
                            .saturating_sub(line_range.start);
                        Some((usize::try_from(line).ok()?, windowed, caret, window_start))
                    })
                    .flatten();
                if let Some((line, windowed, caret, window_start)) = reanchored {
                    let line_text = windowed.text.to_string();
                    self.source_row_blocks
                        .retain(|_, candidate| *candidate != block);
                    self.source_row_blocks.insert(line, block.clone());
                    if let Some(active) = self.active_edit.as_mut() {
                        active.line = line;
                        active.range = windowed.replace_range;
                        active.base_revision = document.revision();
                        active.ending = windowed.ending;
                        active.leading_truncated = windowed.leading_truncated;
                        active.trailing_truncated = windowed.trailing_truncated;
                    }
                    self.source_window_start = window_start;
                    self.suppressed_line_edit_text = Some(line_text.clone());
                    block.update(cx, |block, cx| {
                        let old_len = block.display_text().len();
                        block.replace_text_in_visible_range(
                            0..old_len,
                            &line_text,
                            Some(caret..caret),
                            false,
                            cx,
                        );
                    });
                    self.selection_anchor = Some(line);
                    self.selected_lines = Some(line..line.saturating_add(1));
                    self.scroll_handle
                        .scroll_to_item(line, ScrollStrategy::Center);
                } else if let Some(active) = self.active_edit.as_mut() {
                    active.range = range.start..range.start + replacement.len() as u64;
                    active.base_revision = document.revision();
                }
                if let Some(selection) = recovery_selection {
                    let _ = document.set_source_selection(selection);
                }
                if let Some((start_line, end_line)) = edit_lines {
                    self.fold_projection.apply_source_edit(
                        range.clone(),
                        start_line,
                        end_line,
                        &replacement,
                    );
                }
                self.tail_enabled = false;
                self.coordinator.external_status = Some(
                    cx.global::<I18nManager>()
                        .strings()
                        .large_document_text("tailing_paused_after_edit")
                        .into(),
                );
                let preserve_json_split = self.probe.format == DocumentFormat::Json
                    && self.view_mode == DocumentHostViewMode::Split;
                if !preserve_json_split {
                    self.view_mode = DocumentHostViewMode::Source;
                    self.sync_tab_active_view();
                }
                self.structured_index = None;
                self.invalidate_structured_runtime();
                self.clear_structure_error();
                self.error = None;
                self.invalidate_source_rows();
                self.schedule_search(cx);
                self.schedule_json_graph_projection(cx);
                cx.emit(DocumentHostEvent::StateChanged);
            }
            Err(error) => {
                self.error = Some(error.to_string().into());
                self.reject_stale_source_edit(cx);
            }
        }
        cx.notify();
    }
}
