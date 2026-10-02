// @author kongweiguang

//! Atomic save and post-save session reconciliation.

use super::*;
use std::io::Write as _;

impl DocumentHost {
    /// Waits for the Source input target's native terminal event before snapshotting the document for save.
    pub(crate) fn on_save_document(
        &mut self,
        _: &SaveDocument,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.defer_source_action_for_ime(
            super::source_ime::DeferredSourceAction::Block(BlockHostAction::Save),
            window,
            cx,
        ) {
            return;
        }
        if self.coordinator.external_monitor_paused {
            self.error = Some(
                cx.global::<I18nManager>()
                    .strings()
                    .large_document_text("disk_changed_save_as_reload")
                    .into(),
            );
            cx.emit(DocumentHostEvent::StateChanged);
            cx.notify();
            return;
        }
        // 保存会卸载活动行 Block；先把焦点交还宿主，保存结束后快捷键仍能继续工作。
        self.focus_handle.focus(window);
        if crate::source_tools::format_on_save_for_file(
            &self.path,
            crate::preferences::EditorSettings::format_on_save(cx),
        ) && self.probe.strategy != OpenStrategy::Paged
        {
            self.start_format_before_save(window.window_handle(), cx);
            return;
        }
        self.start_save(self.path.clone(), false, window.window_handle(), cx);
    }

    /// Waits for Source IME completion before opening Save As and keeps the dialog callback bound to this Host.
    pub(crate) fn on_save_document_as(
        &mut self,
        _: &SaveDocumentAs,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.defer_source_action_for_ime(
            super::source_ime::DeferredSourceAction::SaveAs,
            window,
            cx,
        ) {
            return;
        }
        let default_dir = self.path.parent().map(PathBuf::from).unwrap_or_default();
        let suggested_name = self
            .path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned());
        let prompt = cx.prompt_for_new_path(&default_dir, suggested_name.as_deref());
        let window_handle = window.window_handle();
        cx.spawn(async move |this, cx| match prompt.await {
            Ok(Ok(Some(path))) => {
                let _ = this.update(cx, |host, cx| {
                    host.save_as_path(path, window_handle, cx);
                });
            }
            Ok(Ok(None)) | Err(_) => {}
            Ok(Err(_)) => {
                let _ = this.update(cx, |host, cx| {
                    host.error = Some("另存为路径不可用，请重试。".into());
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// Starts Save As only after the UI action has crossed its native IME completion boundary.
    pub(crate) fn save_as_path(
        &mut self,
        path: PathBuf,
        window_handle: gpui::AnyWindowHandle,
        cx: &mut Context<Self>,
    ) {
        if self.has_active_ime_composition(cx) {
            self.error = Some("请先确认或取消输入法候选，再保存文档。".into());
            cx.notify();
            return;
        }
        self.start_save(path, true, window_handle, cx);
    }

    /// Captures one immutable revision only when no transient native composition can enter the snapshot.
    pub(super) fn start_save(
        &mut self,
        path: PathBuf,
        save_as: bool,
        window_handle: gpui::AnyWindowHandle,
        cx: &mut Context<Self>,
    ) {
        if self.has_active_ime_composition(cx) {
            self.error = Some("请先确认或取消输入法候选，再保存文档。".into());
            cx.notify();
            return;
        }
        if self.saving || self.reloading || (!document_dirty_state(&self.document) && !save_as) {
            return;
        }
        let Some(document) = self.document.clone() else {
            return;
        };
        if let Some(cancellation) = self.coordinator.save.cancellation.take() {
            cancellation.cancel();
        }
        self.coordinator.save.generation = self.coordinator.save.generation.wrapping_add(1);
        let task_stamp = DocumentTaskStamp::capture(self, self.coordinator.save.generation);
        let save_started = crate::perf::start();
        let save_profile = self.probe.profile();
        let save_plan = session_plan(&save_profile, &self.probe, self.probe.strategy, false);
        let snapshot = match document.request_save_snapshot() {
            Ok(Some(snapshot)) => snapshot,
            Ok(None) => {
                self.coordinator.save.generation = self.coordinator.save.generation.wrapping_add(1);
                return;
            }
            Err(error) => {
                self.error = Some(error.to_string().into());
                return;
            }
        };
        let cancellation = SearchCancellation::default();
        self.coordinator.save.cancellation = Some(cancellation.clone());
        let save_scroll_offset = self.scroll_handle.0.borrow().base_handle.offset();
        if let Some(cancellation) = self.coordinator.search_cancellation.take() {
            cancellation.cancel();
        }
        self.coordinator.search_task = Task::ready(());
        self.coordinator.source_task = Task::ready(());
        self.structured_task = Task::ready(());
        self.structured_filter_task = Task::ready(());
        self.json_expand_task = Task::ready(());
        self.coordinator.external_generation = self.coordinator.external_generation.wrapping_add(1);
        self.active_edit = None;
        self.saving = true;
        self.error = None;
        let snapshot_for_event = snapshot.clone();
        let save_path = path.clone();
        cx.emit(DocumentHostEvent::StateChanged);
        self.coordinator.save.task = cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    if cancellation.is_cancelled() {
                        return Err(PagedDocumentError::Cancelled);
                    }
                    write_save_snapshot(&snapshot, &save_path, &cancellation, save_as)?;
                    let identity = FileSource::open(&save_path)?.identity()?;
                    Ok::<_, PagedDocumentError>(identity)
                })
                .await;
            let saved = result.is_ok();
            if let Some(started) = save_started {
                crate::perf::emit_document(
                    "document_save",
                    started,
                    usize::try_from(save_profile.len).ok(),
                    Some(saved),
                    &save_profile.format,
                    &save_plan,
                    Some(if save_as { "save_as" } else { "save" }),
                );
            }
            let _ = this.update(cx, |view, cx| {
                if !task_stamp.accepts_identity(view, view.coordinator.save.generation) {
                    return;
                }
                view.coordinator.save.cancellation = None;
                view.saving = false;
                match result {
                    Ok(identity) => {
                        let revision = snapshot_for_event.revision;
                        let save_accepted = document
                            .save_succeeded(
                                revision,
                                gmark_document_runtime::FileIdentity::from(&identity),
                            )
                            .is_ok();
                        if save_accepted {
                            // The saved revision is the new durable baseline;
                            // enqueue that exact immutable snapshot after the
                            // Controller transition so newer edits cannot be
                            // mistaken for persisted content.
                            view.enqueue_recovery_checkpoint_snapshot(
                                snapshot_for_event.clone(),
                                None,
                                cx,
                            );
                        }
                        view.document_epoch = view.document_epoch.wrapping_add(1);
                        view.invalidate_source_rows();
                        view.scroll_handle
                            .0
                            .borrow()
                            .base_handle
                            .set_offset(save_scroll_offset);
                        view.active_edit = None;
                        // The immutable save verified the pre-write identity and
                        // installed the written identity as the new Controller
                        // baseline. Any monitor result captured before this save
                        // is therefore stale, including an own-save replacement
                        // observed while the worker was completing.
                        view.coordinator.pending_external_change = None;
                        view.coordinator.external_monitor_paused = false;
                        view.coordinator.external_status = None;
                        if save_as {
                            view.path = path.clone();
                            cx.emit(DocumentHostEvent::SavedAs(path.clone()));
                        }
                    }
                    Err(error) => {
                        let _ = document
                            .save_failed(snapshot_for_event.revision, SaveFailureCode::Other);
                        view.error = Some(error.to_string().into());
                    }
                }
                cx.emit(DocumentHostEvent::StateChanged);
                cx.notify();
            });
            if saved {
                let _ = cx.update_window(
                    window_handle,
                    |_view: AnyView, window: &mut Window, _cx: &mut App| {
                        window.set_window_edited(false);
                    },
                );
            }
        });
        cx.notify();
    }
}

/// Stream the immutable Controller snapshot without holding the Controller
/// mutex.  The runtime owns encoding, source-format restoration, and the
/// atomic writer; the host only selects save-vs-save-as semantics.
fn write_save_snapshot(
    snapshot: &gmark_document_runtime::DocumentSaveSnapshot,
    path: &Path,
    cancellation: &SearchCancellation,
    save_as: bool,
) -> Result<(), PagedDocumentError> {
    if cancellation.is_cancelled() {
        return Err(PagedDocumentError::Cancelled);
    }
    if save_as {
        snapshot.save_as_atomic_cancellable(path, cancellation)?;
    } else {
        snapshot.save_atomic_cancellable(path, cancellation)?;
    }
    Ok(())
}

pub(super) fn delimited_record_terminator(bytes: &[u8]) -> &'static str {
    if bytes.ends_with(b"\r\n") {
        "\r\n"
    } else if bytes.ends_with(b"\n") {
        "\n"
    } else if bytes.ends_with(b"\r") {
        "\r"
    } else {
        ""
    }
}

pub(super) fn transform_delimited_adapter(
    document: SharedDocument,
    delimiter: u8,
    edit: DelimitedEdit,
    cancellation: &SearchCancellation,
    progress: &AtomicU64,
) -> Result<String, PagedDocumentError> {
    let resident_source =
        document.backend_kind() == Some(gmark_document_core::DocumentBackendKind::Resident);
    let (column, header) = match edit {
        DelimitedEdit::InsertColumn { before, header } => (before, Some(header)),
        DelimitedEdit::DeleteColumn { column } => (column, None),
        _ => {
            return Err(PagedDocumentError::InvalidTransaction(
                "column worker received a non-column edit".into(),
            ));
        }
    };
    let mut input = tempfile::NamedTempFile::new().map_err(|source| PagedDocumentError::Io {
        path: std::env::temp_dir(),
        source,
    })?;
    document.write_to_cancellable(input.as_file_mut(), cancellation)?;
    input
        .as_file_mut()
        .sync_all()
        .map_err(|source| PagedDocumentError::Io {
            path: input.path().to_path_buf(),
            source,
        })?;
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .has_headers(false)
        .flexible(true)
        .from_path(input.path())
        .map_err(|source| PagedDocumentError::Io {
            path: input.path().to_path_buf(),
            source: std::io::Error::new(std::io::ErrorKind::InvalidData, source),
        })?;
    let bytes = FileSource::open(input.path())?;
    let source_len = bytes.identity()?.len;
    let mut output = tempfile::NamedTempFile::new().map_err(|source| PagedDocumentError::Io {
        path: std::env::temp_dir(),
        source,
    })?;
    let output_path = output.path().to_path_buf();
    let mut record = csv::ByteRecord::new();
    let mut physical = 0u64;
    loop {
        if physical.is_multiple_of(1_024) && cancellation.is_cancelled() {
            return Err(PagedDocumentError::Cancelled);
        }
        let start = reader.position().byte();
        if !reader
            .read_byte_record(&mut record)
            .map_err(|source| PagedDocumentError::Io {
                path: input.path().to_path_buf(),
                source: std::io::Error::new(std::io::ErrorKind::InvalidData, source),
            })?
        {
            break;
        }
        let end = reader.position().byte();
        let raw_end = if end < source_len {
            (end + 1).min(source_len)
        } else {
            end
        };
        let raw = bytes.read_range(start, raw_end)?;
        let terminator = if resident_source {
            "\n"
        } else {
            delimited_record_terminator(&raw)
        };
        let mut fields = record
            .iter()
            .map(|field| String::from_utf8_lossy(field).into_owned())
            .collect::<Vec<_>>();
        if let Some(header) = &header {
            fields.insert(
                column.min(fields.len()),
                if physical == 0 {
                    header.clone()
                } else {
                    String::new()
                },
            );
        } else if column < fields.len() {
            fields.remove(column);
        }
        output
            .write_all(serialize_delimited_record(&fields, delimiter, terminator).as_bytes())
            .map_err(|source| PagedDocumentError::Io {
                path: output_path.clone(),
                source,
            })?;
        physical += 1;
        progress.store(physical, Ordering::Relaxed);
    }
    if physical == 0
        && let Some(header) = &header
    {
        output
            .write_all(
                serialize_delimited_record(std::slice::from_ref(header), delimiter, "").as_bytes(),
            )
            .map_err(|source| PagedDocumentError::Io {
                path: output_path.clone(),
                source,
            })?;
    }
    output
        .as_file_mut()
        .sync_all()
        .map_err(|source| PagedDocumentError::Io {
            path: output_path.clone(),
            source,
        })?;
    let bytes = std::fs::read(output.path()).map_err(|source| PagedDocumentError::Io {
        path: output_path,
        source,
    })?;
    String::from_utf8(bytes)
        .map_err(|error| PagedDocumentError::InvalidTransaction(error.to_string()))
}
