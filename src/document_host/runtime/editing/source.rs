// @author kongweiguang

//! Source selection, clipboard, and replacement operations.

use super::*;

impl DocumentHost {
    /// Reject oversized UTF-8 paste payloads before reading selection state or issuing a
    /// Controller transaction, so a clipboard allocation cannot partially mutate a large source.
    pub(crate) fn source_paste_exceeds_limit(text: &str) -> bool {
        u64::try_from(text.len()).map_or(true, |bytes| {
            bytes > gmark_paged_document::MAX_SYSTEM_CLIPBOARD_BYTES
        })
    }

    pub(super) fn selected_source_byte_range(&self) -> Option<Range<u64>> {
        if let Some(document) = self.document.as_ref() {
            let range = document.source_selection().range();
            return (!range.is_empty()).then_some(range);
        }
        None
    }

    pub(super) fn select_source_lines(&mut self, lines: Range<usize>, reversed: bool) {
        self.selection_anchor = if reversed {
            lines.end.checked_sub(1)
        } else {
            Some(lines.start)
        };
        self.selected_lines = Some(lines.clone());
        let Some(document) = self.document.as_ref() else {
            return;
        };
        let Some(start) = document
            .line_range(lines.start as u64)
            .map(|range| range.start)
        else {
            return;
        };
        let Some(end) = lines
            .end
            .checked_sub(1)
            .and_then(|line| document.line_range(line as u64))
            .map(|range| range.end)
        else {
            return;
        };
        let selection = SourceSelection::from_range(start..end, reversed);
        let _ = document.set_source_selection(selection);
    }

    /// Captures Ctrl+A on Source text before a compact row can consume the first press locally.
    pub(super) fn on_select_all_capture(
        &mut self,
        action: &SelectAll,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !matches!(
            self.view_mode,
            DocumentHostViewMode::Source | DocumentHostViewMode::Split
        ) || !self.source_text_surface_has_focus(window, cx)
        {
            cx.propagate();
            return;
        }
        self.on_select_all(action, window, cx);
        cx.stop_propagation();
    }

    /// Waits for native Source composition before replacing the active row selection with the whole document.
    pub(super) fn on_select_all(
        &mut self,
        _: &SelectAll,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.defer_source_action_for_ime(
            super::source_ime::DeferredSourceAction::SelectAll,
            window,
            cx,
        ) {
            return;
        }
        self.active_edit = None;
        let line_count = self.line_count();
        if line_count == 0 {
            return;
        }
        self.select_source_lines(0..line_count, false);
        self.sync_source_selection_visuals(cx);
        self.focus_handle.focus(window);
        cx.notify();
    }

    /// Copies the final Source selection after native composition settles, while retaining structured-view copy behavior.
    pub(super) fn on_copy(&mut self, _: &Copy, window: &mut Window, cx: &mut Context<Self>) {
        if self.defer_source_action_for_ime(
            super::source_ime::DeferredSourceAction::Copy,
            window,
            cx,
        ) {
            return;
        }
        if self.view_mode != DocumentHostViewMode::Source {
            if self.is_json_document()
                && let Some(range) = self
                    .graph_selected_item
                    .as_ref()
                    .and_then(|item| self.resolve_json_graph_edit_target(item))
                    .map(|target| target.range)
                && let Some(document) = self.document.clone()
            {
                if selection_transfer_for_len(range.end.saturating_sub(range.start))
                    == SelectionTransfer::ExportFile
                {
                    self.error = Some(
                        cx.global::<I18nManager>()
                            .strings()
                            .large_document_text("clipboard_limit")
                            .into(),
                    );
                    cx.notify();
                    return;
                }
                self.start_clipboard_read(document, range, false, cx);
                return;
            }
            if let Some(text) = self.selected_structured_cell_text() {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
                self.error = None;
                cx.notify();
                return;
            }
        }
        let Some(lines) = self.selected_lines.clone() else {
            return;
        };
        if lines.len() <= 1 && self.active_edit.is_some() {
            return;
        }
        let Some(document) = self.document.clone() else {
            // 首屏索引尚未完成时，单个可见行已经是稳定的解码快照，复制不应失效。
            // 多行仍等待精确行坐标，避免把估算行窗口拼成错误正文。
            if lines.len() == 1
                && let Some(row) = self.displayed_screen_lines.row(lines.start)
            {
                cx.write_to_clipboard(ClipboardItem::new_string(row.text.to_string()));
                self.error = None;
                cx.notify();
            }
            return;
        };
        let Some(range) = self.selected_source_byte_range() else {
            return;
        };
        if selection_transfer_for_len(range.end.saturating_sub(range.start))
            == SelectionTransfer::ExportFile
        {
            self.error = Some(
                cx.global::<I18nManager>()
                    .strings()
                    .large_document_text("clipboard_limit")
                    .into(),
            );
            cx.notify();
            return;
        }
        self.start_clipboard_read(document, range, false, cx);
    }

    /// 固定命令时的快照与 revision；剪切只有原生剪贴板确认成功后才能删除，
    /// 后台读取失败、目标变化或剪贴板被占用都必须保留原文及选区。
    fn start_clipboard_read(
        &mut self,
        document: SharedDocument,
        range: Range<u64>,
        delete_after_copy: bool,
        cx: &mut Context<Self>,
    ) {
        // Capture the immutable Controller snapshot before yielding to the
        // worker.  Reading the live session in the worker would race a paste
        // transaction and make the clipboard contain the replacement rather
        // than the command's selected bytes.
        let snapshot = match document.snapshot() {
            Ok(snapshot) => snapshot,
            Err(error) => {
                self.error = Some(error.to_string().into());
                cx.notify();
                return;
            }
        };
        if let Some(cancellation) = self.clipboard_cancellation.take() {
            cancellation.cancel();
        }
        self.clipboard_generation = self.clipboard_generation.wrapping_add(1);
        let generation = self.clipboard_generation;
        let task_stamp = DocumentTaskStamp::capture(self, generation);
        let revision = document.revision();
        let read_range = range.clone();
        self.metrics.copy_requests = self.metrics.copy_requests.saturating_add(1);
        let cancellation = SearchCancellation::default();
        self.clipboard_cancellation = Some(cancellation.clone());
        self.coordinator.external_status = Some(
            cx.global::<I18nManager>()
                .strings()
                .large_document_text("copying_selection")
                .into(),
        );
        self.clipboard_task = cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    Self::read_snapshot_range_cancellable(snapshot, read_range, &cancellation)
                })
                .await;
            let _ = this.update(cx, |view, cx| {
                if !task_stamp.accepts_identity(view, view.clipboard_generation) {
                    return;
                }
                view.clipboard_cancellation = None;
                view.coordinator.external_status = None;
                match result {
                    Ok(bytes) => {
                        let text = String::from_utf8_lossy(&bytes).into_owned();
                        if delete_after_copy && !view.accept_native_cut_clipboard(&text) {
                            view.error = Some("无法写入剪贴板，文字已保留，请重试".into());
                            cx.notify();
                            return;
                        }
                        view.metrics.copied_bytes =
                            view.metrics.copied_bytes.saturating_add(bytes.len() as u64);
                        // 生产 Cut 已完成原生写入；只为无窗口测试同步 GPUI 剪贴板，
                        // 避免再次写入一个不能报告失败的平台接口。
                        if !delete_after_copy || cfg!(test) {
                            cx.write_to_clipboard(ClipboardItem::new_string(text));
                        }
                        if delete_after_copy {
                            let current_revision =
                                view.document.as_ref().map(SharedDocument::revision);
                            if current_revision == Some(revision) {
                                view.replace_source_range(range, "", cx);
                            } else {
                                view.error = Some(
                                    cx.global::<I18nManager>()
                                        .strings()
                                        .large_document_text("cut_revision_changed")
                                        .into(),
                                );
                            }
                        } else {
                            view.error = None;
                        }
                    }
                    Err(error) => view.error = Some(localized_document_error(&error, cx)),
                }
                cx.notify();
            });
        });
        cx.notify();
    }

    /// GPUI 的写入接口不返回错误，剪切及未写入文字的恢复先用原生 Result 确认接收；
    /// 无窗口测试只替换该适配器的结果，不接触用户的系统剪贴板。
    pub(super) fn accept_native_cut_clipboard(&mut self, text: &str) -> bool {
        #[cfg(test)]
        {
            let _ = text;
            !std::mem::take(&mut self.fail_next_native_clipboard_write_for_test)
        }
        #[cfg(not(test))]
        {
            arboard::Clipboard::new()
                .and_then(|mut clipboard| clipboard.set_text(text.to_owned()))
                .is_ok()
        }
    }

    /// Read an immutable snapshot in bounded chunks so a cancelled copy does
    /// not retain a live Controller lock or materialize the whole source on
    /// the UI thread.
    fn read_snapshot_range_cancellable(
        snapshot: Arc<dyn DocumentSnapshot>,
        range: Range<u64>,
        cancellation: &SearchCancellation,
    ) -> Result<Vec<u8>, PagedDocumentError> {
        if range.start > range.end || range.end > snapshot.len() {
            return Err(PagedDocumentError::InvalidTransaction(
                "selection range is outside the immutable document snapshot".to_owned(),
            ));
        }
        const COPY_CHUNK_BYTES: u64 = 8 * 1024 * 1024;
        let mut offset = range.start;
        let mut bytes = Vec::new();
        while offset < range.end {
            if cancellation.is_cancelled() {
                return Err(PagedDocumentError::Cancelled);
            }
            let end = offset.saturating_add(COPY_CHUNK_BYTES).min(range.end);
            let chunk = snapshot
                .read_range(offset..end)
                .map_err(|error| PagedDocumentError::InvalidTransaction(error.to_string()))?;
            bytes.extend_from_slice(&chunk);
            offset = end;
        }
        if cancellation.is_cancelled() {
            return Err(PagedDocumentError::Cancelled);
        }
        Ok(bytes)
    }

    /// 保存持有不可变快照，当前选区仍可修改；重新载入会替换正文，必须继续拒绝写入。
    pub(super) fn delete_selected_source(&mut self, cx: &mut Context<Self>) {
        if self.reloading {
            return;
        }
        if self
            .selected_lines
            .as_ref()
            .is_none_or(|lines| lines.len() <= 1 && self.active_edit.is_some())
        {
            return;
        }
        let Some(range) = self.selected_source_byte_range() else {
            return;
        };
        self.replace_source_range(range, "", cx);
    }

    /// Rejects shared Source writes during native composition before issuing one replacement transaction.
    pub(super) fn replace_source_range(
        &mut self,
        range: Range<u64>,
        replacement: &str,
        cx: &mut Context<Self>,
    ) {
        if self.has_active_ime_composition(cx) {
            self.error = Some("请先确认或取消输入法候选，再执行文档修改。".into());
            cx.notify();
            return;
        }
        let Some(document) = self.document.as_ref() else {
            return;
        };
        if let Err(error) = document.replace_range(range.clone(), replacement) {
            self.error = Some(localized_document_error(&error, cx));
            cx.notify();
            return;
        }
        let preserve_view = (self.probe.format == DocumentFormat::Json
            && self.view_mode == DocumentHostViewMode::Split)
            || (self.is_delimited_document()
                && matches!(
                    self.view_mode,
                    DocumentHostViewMode::Live | DocumentHostViewMode::Split
                ));
        self.install_source_replacement(range, replacement, preserve_view, false, false, cx);
    }

    fn replace_structured_cell_source_range(
        &mut self,
        range: Range<u64>,
        replacement: &str,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(document) = self.document.as_ref() else {
            return false;
        };
        if let Err(error) = document.replace_range(range.clone(), replacement) {
            self.error = Some(localized_document_error(&error, cx));
            cx.notify();
            return false;
        }
        self.install_source_replacement(range, replacement, true, true, false, cx);
        true
    }

    /// CSV 表格操作在旧索引追平前继续使用其基线坐标；源码 transaction 成功后
    /// 同步记录长度变化，让紧接着发生的单元格、行操作仍能命中当前正文。
    pub(super) fn replace_delimited_table_source_range(
        &mut self,
        baseline_range: Range<u64>,
        replacement: &str,
        cx: &mut Context<Self>,
    ) -> bool {
        let current_range = self.current_structured_record_range(&baseline_range);
        let delta = i64::try_from(replacement.len())
            .unwrap_or(i64::MAX)
            .saturating_sub(
                i64::try_from(current_range.end - current_range.start).unwrap_or(i64::MAX),
            );
        if !self.replace_structured_cell_source_range(current_range, replacement, cx) {
            return false;
        }
        self.structured_cell_source_edits
            .push((baseline_range, delta));
        true
    }

    pub(super) fn replace_source_range_from_graph(
        &mut self,
        base_revision: u64,
        range: Range<u64>,
        replacement: &str,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(document) = self.document.as_ref() else {
            return false;
        };
        if document.revision() != base_revision {
            self.graph_edit_error = Some(localized_document_error(
                &PagedDocumentError::InvalidTransaction("stale document revision".into()),
                cx,
            ));
            cx.notify();
            return false;
        }
        if let Err(error) = document.replace_range(range.clone(), replacement) {
            self.graph_edit_error = Some(localized_document_error(&error, cx));
            cx.notify();
            return false;
        }
        self.install_source_replacement(range, replacement, true, false, false, cx);
        true
    }

    /// Finishes a committed source edit with a collapsed caret for ordinary replace commands.
    pub(super) fn install_source_replacement(
        &mut self,
        range: Range<u64>,
        replacement: &str,
        preserve_view: bool,
        preserve_structure: bool,
        preserve_folds: bool,
        cx: &mut Context<Self>,
    ) {
        let caret = range.start.saturating_add(replacement.len() as u64);
        self.install_source_replacement_with_selection(
            range,
            replacement,
            Some(SourceSelection::collapsed(caret, SourceAffinity::After)),
            preserve_view,
            preserve_structure,
            preserve_folds,
            cx,
        );
    }

    /// 完成已提交的源码编辑并记录显式选区，使撤销可以恢复原范围与方向。
    pub(super) fn install_source_replacement_with_selection(
        &mut self,
        range: Range<u64>,
        replacement: &str,
        selection: Option<SourceSelection>,
        preserve_view: bool,
        preserve_structure: bool,
        preserve_folds: bool,
        cx: &mut Context<Self>,
    ) {
        self.install_source_replacement_with_selection_and_group(
            range,
            replacement,
            selection,
            preserve_view,
            preserve_structure,
            preserve_folds,
            None,
            cx,
        );
    }

    /// 安装已提交的输入组事务，只追加带组 ID 的恢复记录，避免重复写入普通事务。
    pub(super) fn install_source_replacement_with_typing_group(
        &mut self,
        range: Range<u64>,
        replacement: &str,
        selection: Option<SourceSelection>,
        preserve_view: bool,
        preserve_structure: bool,
        preserve_folds: bool,
        group_id: TypingGroupId,
        cx: &mut Context<Self>,
    ) {
        self.install_source_replacement_with_selection_and_group(
            range,
            replacement,
            selection,
            preserve_view,
            preserve_structure,
            preserve_folds,
            Some(group_id),
            cx,
        );
    }

    /// 统一更新 Source 投影，并按事务类型写入唯一一条普通或分组恢复记录。
    fn install_source_replacement_with_selection_and_group(
        &mut self,
        range: Range<u64>,
        replacement: &str,
        selection: Option<SourceSelection>,
        preserve_view: bool,
        preserve_structure: bool,
        preserve_folds: bool,
        typing_group_id: Option<TypingGroupId>,
        cx: &mut Context<Self>,
    ) {
        if !preserve_folds && let Some(document) = self.document.as_ref() {
            let start_line = document
                .line_for_offset(range.start.min(document.len()))
                .and_then(|line| usize::try_from(line).ok())
                .unwrap_or_default();
            let end_line = document
                .line_for_offset(range.end.min(document.len()))
                .and_then(|line| usize::try_from(line).ok())
                .unwrap_or(start_line);
            self.fold_projection.apply_source_edit(
                range.clone(),
                start_line,
                end_line,
                replacement,
            );
        }
        let selection_offset = selection.map_or_else(
            || range.start.saturating_add(replacement.len() as u64),
            |selection| selection.head.byte_offset,
        );
        if let Some(document) = self.document.clone() {
            // Read the base revision before entering the recovery queue.  The
            // queue owns all journal I/O, so a recovery failure cannot re-lock
            // the Controller while this UI transaction is still unwinding.
            let base_revision = document.revision().saturating_sub(1);
            if let Some(group_id) = typing_group_id {
                self.enqueue_recovery_typing_transaction(
                    &document,
                    base_revision,
                    range.clone(),
                    replacement,
                    selection,
                    recovery_view_id(self.view_mode),
                    group_id,
                    cx,
                );
            } else {
                self.enqueue_recovery_transaction(
                    &document,
                    base_revision,
                    range.clone(),
                    replacement,
                    selection,
                    recovery_view_id(self.view_mode),
                    cx,
                );
            }
        }
        let Some(document) = self.document.as_ref() else {
            return;
        };
        let line = document
            .line_for_offset(selection_offset.min(document.len()))
            .and_then(|line| usize::try_from(line).ok())
            .unwrap_or_default();
        self.active_edit = None;
        self.source_drag_anchor = None;
        if typing_group_id.is_none() {
            self.break_source_typing_group();
        }
        self.selection_anchor = Some(line);
        self.selected_lines = Some(line..line.saturating_add(1));
        self.tail_enabled = false;
        if !preserve_view {
            self.view_mode = DocumentHostViewMode::Source;
            self.sync_tab_active_view();
        }
        if !preserve_structure {
            self.structured_index = None;
            self.invalidate_structured_runtime();
        }
        // Source 是大文件编辑时的稳定模式；结构索引失效属于内部状态，
        // 不应伪装成顶部错误横幅。用户真正请求结构视图时再说明不可用原因。
        self.clear_structure_error();
        self.error = None;
        self.invalidate_source_rows();
        self.schedule_search(cx);
        self.derived_projection_stale = self.derived_projection_snapshot.is_some();
        self.schedule_json_graph_projection(cx);
        self.schedule_delimited_snapshot_rebuild(cx);
        if let Some(selection) = selection {
            self.set_source_selection(selection, cx);
        }
        if preserve_structure {
            // 单格编辑期间旧索引与覆盖值仍可用，后台追平不应显示成整表刷新状态。
            self.clear_structure_error();
        }
        cx.emit(DocumentHostEvent::StateChanged);
        cx.notify();
    }

    /// 候选结束后替换源码选区；后台保存不锁住当前正文，独立行仍由原输入 Block 处理。
    pub(super) fn on_paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        if self.defer_source_action_for_ime(
            super::source_ime::DeferredSourceAction::Paste,
            window,
            cx,
        ) {
            return;
        }
        // 聚焦行由 Block 的 EntityInputHandler 处理；宿主只处理跨行或卸载选区。
        if self.active_edit.is_some() || self.reloading {
            return;
        }
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return;
        };
        if Self::source_paste_exceeds_limit(&text) {
            self.error = Some(
                cx.global::<I18nManager>()
                    .strings()
                    .large_document_text("clipboard_limit")
                    .into(),
            );
            cx.notify();
            return;
        }
        let Some(range) = self
            .document
            .as_ref()
            .map(|document| document.source_selection().range())
        else {
            return;
        };
        self.replace_source_range(range, &text, cx);
    }

    /// 剪切绑定命令时的不可变快照与 revision；保存可并发，删除仍等待剪贴板成功并重检 revision。
    pub(super) fn on_cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        if self.defer_source_action_for_ime(
            super::source_ime::DeferredSourceAction::Cut,
            window,
            cx,
        ) {
            return;
        }
        if self.reloading || self.active_edit.is_some() {
            return;
        }
        let Some(range) = self.selected_source_byte_range() else {
            return;
        };
        if selection_transfer_for_len(range.end.saturating_sub(range.start))
            == SelectionTransfer::ExportFile
        {
            self.error = Some(
                cx.global::<I18nManager>()
                    .strings()
                    .large_document_text("selection_export_limit")
                    .into(),
            );
            cx.notify();
            return;
        }
        let Some(document) = self.document.clone() else {
            return;
        };
        self.start_clipboard_read(document, range, true, cx);
    }

    /// Defers range deletion until the Source composition has delivered its terminal event.
    pub(super) fn on_delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
        if self.defer_source_action_for_ime(
            super::source_ime::DeferredSourceAction::Delete,
            window,
            cx,
        ) {
            return;
        }
        self.delete_selected_source(cx);
    }

    /// Defers backward range deletion until the Source composition has delivered its terminal event.
    pub(super) fn on_delete_back(
        &mut self,
        _: &DeleteBack,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.defer_source_action_for_ime(
            super::source_ime::DeferredSourceAction::DeleteBack,
            window,
            cx,
        ) {
            return;
        }
        self.delete_selected_source(cx);
    }
}
