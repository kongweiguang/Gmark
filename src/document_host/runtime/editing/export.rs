// @author kongweiguang

//! Selection export and line-edit event handling.

use super::*;

const SOURCE_TYPING_GROUP_IDLE: Duration = Duration::from_millis(900);

/// 计算 UTF-8 字符边界上的最小差异，避免每次按键都把整行写入恢复日志。
fn minimal_source_edit(old: &str, new: &str) -> (Range<usize>, Range<usize>) {
    let mut prefix = 0;
    for ((old_offset, old_char), (new_offset, new_char)) in
        old.char_indices().zip(new.char_indices())
    {
        if old_char != new_char || old_offset != new_offset {
            break;
        }
        prefix = old_offset + old_char.len_utf8();
    }

    let mut suffix = 0;
    for (old_char, new_char) in old[prefix..].chars().rev().zip(new[prefix..].chars().rev()) {
        if old_char != new_char {
            break;
        }
        suffix += old_char.len_utf8();
    }
    (
        prefix..old.len().saturating_sub(suffix),
        prefix..new.len().saturating_sub(suffix),
    )
}

/// 选区被文字替换时保留完整旧范围，其余输入使用不会拆开 UTF-8 字符的最小差异。
fn source_input_edit(
    old: &str,
    new: &str,
    source_start: u64,
    selection_before: SourceSelection,
    selection_after: SourceSelection,
) -> (Range<u64>, Range<usize>) {
    let selected = selection_before.range();
    if !selected.is_empty() && selection_after.range().is_empty() {
        let local_start = selected
            .start
            .checked_sub(source_start)
            .and_then(|offset| usize::try_from(offset).ok());
        let local_end = selected
            .end
            .checked_sub(source_start)
            .and_then(|offset| usize::try_from(offset).ok());
        let inserted_end = selection_after
            .head
            .byte_offset
            .checked_sub(source_start)
            .and_then(|offset| usize::try_from(offset).ok());
        if let (Some(start), Some(old_end), Some(new_end)) = (local_start, local_end, inserted_end)
            && new_end >= start
            && old.get(..start) == new.get(..start)
            && old.get(old_end..) == new.get(new_end..)
        {
            return (selected, start..new_end);
        }
    }

    let (old_range, new_range) = minimal_source_edit(old, new);
    (
        source_start.saturating_add(old_range.start as u64)
            ..source_start.saturating_add(old_range.end as u64),
        new_range,
    )
}

impl DocumentHost {
    /// 清除视图侧的续组令牌；下次输入生成新 ID 后由 Controller 关闭后端旧组。
    pub(crate) fn break_source_typing_group(&mut self) {
        self.source_typing_group = None;
    }

    /// 只为相邻插入复用输入组；选区替换开启新组，并校验视图、revision、选区和空闲时间。
    pub(super) fn source_typing_group_for_edit(
        &mut self,
        document: &SharedDocument,
        base_revision: u64,
        selection_before: SourceSelection,
        selection_after: SourceSelection,
        edit_range: &Range<u64>,
        replacement: &str,
        undo_kind: Option<UndoCaptureKind>,
    ) -> Option<TypingGroupId> {
        let selection_range = selection_before.range();
        let starts_from_selection = !selection_range.is_empty() && &selection_range == edit_range;
        let starts_from_caret = selection_range.is_empty()
            && edit_range.is_empty()
            && edit_range.start == selection_before.head.byte_offset;
        let eligible = undo_kind == Some(UndoCaptureKind::CoalescibleText)
            && !replacement.is_empty()
            && !replacement.contains(['\r', '\n'])
            && selection_after.range().is_empty()
            && (starts_from_selection || starts_from_caret);
        if !eligible {
            self.break_source_typing_group();
            return None;
        }

        let now = Instant::now();
        let continuing = starts_from_caret
            && self.source_typing_group.as_ref().is_some_and(|group| {
                group.view_id == document.view_id()
                    && group.revision_after == base_revision
                    && group.selection_after == selection_before
                    && now.saturating_duration_since(group.last_input_at)
                        <= SOURCE_TYPING_GROUP_IDLE
            });
        if continuing {
            return self.source_typing_group.as_ref().map(|group| group.id);
        }

        self.break_source_typing_group();
        Some(TypingGroupId::new())
    }

    /// 仅把已提交的 revision 与选区作为下一按键可续组的基线。
    pub(super) fn finish_source_typing_group(
        &mut self,
        group_id: Option<TypingGroupId>,
        document: &SharedDocument,
        selection_after: SourceSelection,
        replacement: &str,
    ) {
        let Some(group_id) = group_id.filter(|_| {
            !replacement.is_empty()
                && !replacement.contains(['\r', '\n'])
                && selection_after.range().is_empty()
        }) else {
            self.break_source_typing_group();
            return;
        };
        self.source_typing_group = Some(SourceTypingGroup {
            id: group_id,
            view_id: document.view_id(),
            revision_after: document.revision(),
            selection_after,
            last_input_at: Instant::now(),
        });
    }

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

    /// 在 Host 事件边界处理源码行输入，使组合终态与跨行写入共享 revision 校验。
    pub(super) fn on_line_edit_event(
        &mut self,
        block: Entity<Block>,
        event: &BlockEvent,
        cx: &mut Context<Self>,
    ) {
        if let BlockEvent::PrepareUndo { kind } = event {
            self.source_pending_undo_capture = Some((block, *kind));
            if *kind != crate::components::UndoCaptureKind::CoalescibleText {
                self.source_typing_group = None;
            }
            return;
        }
        if matches!(event, BlockEvent::ImeCompositionStarted) {
            self.source_typing_group = None;
            self.source_pending_undo_capture = None;
            self.capture_source_ime_snapshot(&block, cx);
            return;
        }
        if let BlockEvent::RequestReplaceCrossBlockSelection {
            text,
            selected_range_relative,
            mark_inserted_text,
            undo_kind,
        } = event
        {
            self.source_pending_undo_capture = None;
            if self.coordinator.source_boundary_cancellation.is_some()
                && !*mark_inserted_text
                && !matches!(
                    undo_kind,
                    crate::components::UndoCaptureKind::ImeComposition
                )
            {
                if self.coordinator.source_boundary_input_owner.as_ref() == Some(&block)
                    && self.is_current_source_text_input_owner(&block, cx)
                    && !block.read(cx).is_read_only()
                {
                    self.queue_source_boundary_text(
                        text,
                        selected_range_relative.clone(),
                        *undo_kind,
                    );
                }
                return;
            }
            self.replace_source_cross_block_input(
                &block,
                text,
                selected_range_relative.as_ref(),
                *mark_inserted_text,
                *undo_kind,
                cx,
            );
            return;
        }
        if matches!(
            event,
            BlockEvent::ImeCompositionEnded { .. } | BlockEvent::ImeCompositionFinishFailed
        ) {
            self.source_typing_group = None;
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
        if self.reloading {
            return;
        }
        if self
            .suppressed_line_edit_text
            .as_deref()
            .is_some_and(|expected| expected == block.read(cx).display_text())
        {
            self.suppressed_line_edit_text = None;
            self.source_pending_undo_capture = None;
            return;
        }
        let Some(active) = self.active_edit.as_ref() else {
            self.source_pending_undo_capture = None;
            return;
        };
        if active.block != block {
            self.source_pending_undo_capture = None;
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
        let replacement = format!("{text}{ending}");
        let old_bytes = match document.read_range(range.clone()) {
            Ok(bytes) => bytes,
            Err(error) => {
                self.error = Some(localized_document_error(&error, cx));
                self.reject_stale_source_edit(cx);
                return;
            }
        };
        let old_text = match String::from_utf8(old_bytes) {
            Ok(text) => text,
            Err(_) => {
                self.error = Some("源码行不是有效 UTF-8，已取消本次输入。".into());
                self.reject_stale_source_edit(cx);
                return;
            }
        };
        let selection_after = Self::source_selection_from_block(block.read(cx), range.start);
        let recovery_selection = Some(selection_after);
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
        let (edit_range, replacement_range) = source_input_edit(
            &old_text,
            &replacement,
            range.start,
            selection_before,
            selection_after,
        );
        let Some(edit_replacement) = replacement.get(replacement_range.clone()) else {
            self.source_pending_undo_capture = None;
            self.error = Some("源码输入范围已失效，请重新选择后输入。".into());
            self.reject_stale_source_edit(cx);
            return;
        };
        if edit_range.is_empty() && edit_replacement.is_empty() {
            self.source_pending_undo_capture = None;
            return;
        }
        let undo_kind = self
            .source_pending_undo_capture
            .take()
            .filter(|(owner, _)| owner == &block)
            .map(|(_, kind)| kind);
        let typing_group_id = self.source_typing_group_for_edit(
            &document,
            base_revision,
            selection_before,
            selection_after,
            &edit_range,
            edit_replacement,
            undo_kind,
        );
        let transaction_id = match document.next_transaction_id() {
            Ok(transaction_id) => transaction_id,
            Err(error) => {
                self.break_source_typing_group();
                self.error = Some(error.to_string().into());
                self.reject_stale_source_edit(cx);
                return;
            }
        };
        let transaction = Transaction::new(
            DocumentRevision(base_revision),
            vec![SourceEdit::new(edit_range.clone(), edit_replacement)],
        );
        let applied = if let Some(group_id) = typing_group_id {
            document.apply_typing_transaction(
                transaction_id,
                transaction,
                selection_before,
                selection_after,
                group_id,
            )
        } else {
            document.apply_transaction(
                transaction_id,
                transaction,
                selection_before,
                selection_after,
            )
        };
        match applied {
            Ok(()) => {
                // Capture the post-edit snapshot outside the Controller lock;
                // the recovery worker performs the journal append later.
                let revision_before = base_revision;
                if let Some(group_id) = typing_group_id {
                    self.enqueue_recovery_typing_transaction(
                        &document,
                        revision_before,
                        edit_range.clone(),
                        edit_replacement,
                        recovery_selection,
                        DocumentViewId::source(),
                        group_id,
                        cx,
                    );
                } else {
                    self.enqueue_recovery_transaction(
                        &document,
                        revision_before,
                        edit_range.clone(),
                        edit_replacement,
                        recovery_selection,
                        DocumentViewId::source(),
                        cx,
                    );
                }
                // 单行与换行输入统一在失效旧缓存后重锚，避免两次改写 Block 和重复滚动。
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
                let was_tailing = self.tail_enabled;
                self.tail_enabled = false;
                if was_tailing {
                    self.coordinator.external_status = Some(
                        cx.global::<I18nManager>()
                            .strings()
                            .large_document_text("tailing_paused_after_edit")
                            .into(),
                    );
                }
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
                if self.retain_source_input_row_after_edit(&block, selection_after, cx) {
                    self.finish_source_typing_group(
                        typing_group_id,
                        &document,
                        selection_after,
                        edit_replacement,
                    );
                }
                self.schedule_search(cx);
                self.schedule_json_graph_projection(cx);
                cx.emit(DocumentHostEvent::StateChanged);
            }
            Err(error) => {
                self.break_source_typing_group();
                self.error = Some(error.to_string().into());
                self.reject_stale_source_edit(cx);
            }
        }
        cx.notify();
    }

    /// IME 替换原选区后重锚同一输入行，保留确认前已排队续键的授权；历史仍与普通输入分开。
    fn replace_source_cross_block_input(
        &mut self,
        block: &Entity<Block>,
        text: &str,
        selected_range_relative: Option<&Range<usize>>,
        mark_inserted_text: bool,
        undo_kind: crate::components::UndoCaptureKind,
        cx: &mut Context<Self>,
    ) {
        if mark_inserted_text
            || matches!(
                undo_kind,
                crate::components::UndoCaptureKind::ImeComposition
            )
        {
            return;
        }
        if self.reloading || block.read(cx).is_read_only() {
            return;
        }
        let Some(document) = self.document.clone() else {
            return;
        };
        let is_ime_commit = undo_kind == crate::components::UndoCaptureKind::ImeCompositionCommit;
        let (range, selection_before, base_revision) = if is_ime_commit {
            let Some(snapshot) = self
                .source_ime_snapshot
                .as_ref()
                .filter(|snapshot| snapshot.row_entity == *block)
            else {
                self.error = Some("输入法选区已失效，请重新选择后输入。".into());
                cx.notify();
                return;
            };
            (
                snapshot.selection.range(),
                snapshot.selection,
                snapshot.revision,
            )
        } else {
            if !self.is_current_source_text_input_owner(block, cx) {
                return;
            }
            let selection = document.source_selection();
            (selection.range(), selection, document.revision())
        };
        let another_ime_owner_active =
            self.source_row_blocks.values().any(|candidate| {
                candidate.entity_id() != block.entity_id()
                    && candidate.read(cx).has_ime_composition()
            }) || self.structured_cell_input.read(cx).has_ime_composition()
                || self.graph_edit_input.read(cx).has_ime_composition()
                || self.search_input.read(cx).has_ime_composition()
                || self.navigation_input.read(cx).has_ime_composition()
                || self.structured_filter_input.read(cx).has_ime_composition();
        if document.revision() != base_revision
            || (is_ime_commit && range.start >= range.end)
            || range.end > document.len()
            || another_ime_owner_active
            || (!is_ime_commit && self.has_active_ime_composition(cx))
        {
            if is_ime_commit {
                self.source_ime_snapshot = None;
            }
            self.error = Some("源码选区在输入期间发生变化，已取消本次输入。".into());
            cx.notify();
            return;
        }
        let Some(transaction_id) = document.next_transaction_id().ok() else {
            self.error = Some("无法创建源码输入事务，请重试。".into());
            cx.notify();
            return;
        };
        let caret = range.start.saturating_add(text.len() as u64);
        let selection_after = selected_range_relative
            .filter(|relative| text.get(relative.start..relative.end).is_some())
            .and_then(|relative| {
                Some(SourceSelection::from_range(
                    range.start.checked_add(relative.start as u64)?
                        ..range.start.checked_add(relative.end as u64)?,
                    false,
                ))
            })
            .unwrap_or_else(|| SourceSelection::collapsed(caret, SourceAffinity::After));
        let typing_group_id = self.source_typing_group_for_edit(
            &document,
            base_revision,
            selection_before,
            selection_after,
            &range,
            text,
            Some(undo_kind),
        );
        let transaction = Transaction::new(
            DocumentRevision(base_revision),
            vec![SourceEdit::new(range.clone(), text.to_owned())],
        );
        if is_ime_commit {
            self.source_ime_snapshot = None;
        }
        let applied = if let Some(group_id) = typing_group_id {
            document.apply_typing_transaction(
                transaction_id,
                transaction,
                selection_before,
                selection_after,
                group_id,
            )
        } else {
            document.apply_transaction(
                transaction_id,
                transaction,
                selection_before,
                selection_after,
            )
        };
        if let Err(error) = applied {
            self.break_source_typing_group();
            self.source_ime_snapshot = None;
            self.error = Some(error.to_string().into());
            cx.notify();
            return;
        }
        self.source_ime_snapshot = None;
        if let Some(group_id) = typing_group_id {
            self.install_source_replacement_with_typing_group(
                range,
                text,
                Some(selection_after),
                true,
                false,
                false,
                group_id,
                cx,
            );
        } else {
            self.break_source_typing_group();
            self.install_source_replacement_with_selection(
                range,
                text,
                Some(selection_after),
                true,
                false,
                false,
                cx,
            );
        }
        if (typing_group_id.is_some() || is_ime_commit)
            && self.retain_source_input_row_after_edit(block, selection_after, cx)
        {
            self.finish_source_typing_group(typing_group_id, &document, selection_after, text);
            if is_ime_commit {
                // 续键已按旧选区产生 Host 事件；保留已确认插入点的桥接，下一次普通输入再接回局部行。
                self.active_edit = None;
                self.sync_source_selection_visuals(cx);
            }
        }
    }
}
