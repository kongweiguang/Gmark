// @author kongweiguang

//! Disk reload and encoding conversion.

use super::*;

impl DocumentHost {
    pub(super) fn reload_from_disk(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.reload_from_disk_with_encoding(None, window, cx);
    }

    pub(crate) fn reopen_with_encoding(
        &mut self,
        encoding: TextEncoding,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if document_dirty_state(&self.document) {
            self.error = Some(
                cx.global::<I18nManager>()
                    .strings()
                    .large_document_text("reopen_dirty_error")
                    .into(),
            );
            cx.emit(DocumentHostEvent::StateChanged);
            cx.notify();
            return;
        }
        self.reload_from_disk_with_encoding(Some(encoding), window, cx);
    }

    pub(super) fn reload_from_disk_with_encoding(
        &mut self,
        forced_encoding: Option<TextEncoding>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.saving || self.reloading {
            return;
        }
        self.cancel_selection_transfers();
        let Some(current_document) = self.document.as_ref() else {
            return;
        };
        let expected_revision = current_document.revision_doc();
        let expected_identity = match current_document.identity() {
            Ok(identity) => identity,
            Err(error) => {
                self.error = Some(localized_document_error(
                    &gmark_paged_document::PagedDocumentError::InvalidTransaction(
                        error.to_string(),
                    ),
                    cx,
                ));
                cx.emit(DocumentHostEvent::StateChanged);
                cx.notify();
                return;
            }
        };
        let path = self.path.clone();
        let configured_loading = gmark_config::read_app_preferences()
            .map(|preferences| preferences.document_loading.policy())
            .unwrap_or_default();
        let loading = if forced_encoding.is_some() {
            gmark_document_core::LoadingPolicy {
                max_resident_bytes: Some(self.probe.options.max_resident_bytes),
                force_safe_source: self.probe.force_safe_source,
            }
        } else {
            configured_loading
        };
        let loading_limits = loading.effective_limits();
        let recovery_dirs = runtime_recovery_dirs();
        if recovery_dirs.is_some() {
            self.coordinator.recovery_enabled = true;
        }
        let window_handle = window.window_handle();
        if let Some(cancellation) = self.coordinator.index_cancellation.take() {
            cancellation.cancel();
        }
        let cancellation = SearchCancellation::default();
        self.coordinator.index_cancellation = Some(cancellation.clone());
        self.coordinator.index_generation = self.coordinator.index_generation.wrapping_add(1);
        let task_stamp = DocumentTaskStamp::capture(self, self.coordinator.index_generation);
        self.coordinator.external_generation = self.coordinator.external_generation.wrapping_add(1);
        self.active_edit = None;
        self.reloading = true;
        self.error = None;
        cx.emit(DocumentHostEvent::StateChanged);
        self.coordinator.index_task = cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let original = FileSource::open(&path)?;
                    let mut probe = gmark_paged_document::probe_file(
                        &path,
                        gmark_paged_document::ProbeOptions {
                            max_resident_bytes: loading_limits.max_resident_bytes,
                            ..gmark_paged_document::ProbeOptions::default()
                        },
                    )?;
                    probe.force_safe_source = loading.force_safe_source;
                    let plan =
                        gmark_document_core::OpenPolicyResolver.resolve(loading, &probe.profile());
                    probe.strategy = match plan.backend {
                        gmark_document_core::DocumentBackendKind::Resident => {
                            OpenStrategy::Resident
                        }
                        gmark_document_core::DocumentBackendKind::Paged => OpenStrategy::Paged,
                    };
                    if let Some(encoding) = forced_encoding {
                        probe.encoding = encoding;
                    }
                    let reopened_encoding = text_encoding_label(&probe.encoding);
                    let original_for_session = original.clone();
                    let prepared = prepare_utf8_source(original, probe.encoding.clone())?;
                    let prepared_source = prepared.source().clone();
                    let index = LineIndex::build_cancellable(&prepared_source, &cancellation)?;
                    let document = build_document_session_from_prepared(
                        &probe,
                        &original_for_session,
                        prepared,
                        index.clone(),
                        false,
                    )?;
                    let recovery = recovery_dirs.as_ref().map(|dirs| {
                        let recovery_dir = dirs.recovery_dir();
                        dirs.ensure_state_parent(&recovery_dir.join(".gmark-recovery-root"))
                            .map_err(|error| PagedDocumentError::Recovery(error.to_string()))
                            .and_then(|()| {
                                DocumentRecoveryJournal::create(
                                    &recovery_dir,
                                    &original_for_session,
                                    probe.encoding.clone(),
                                    &document,
                                )
                            })
                    });
                    let (structure_source, structure_index, structure_bytes) =
                        structure_input_for_session(
                            &document,
                            &prepared_source,
                            &index,
                            &cancellation,
                        )?;
                    let structured = if derived_views_enabled(probe.strategy) {
                        build_structured_index(
                            &structure_source,
                            &structure_index,
                            probe.format.clone(),
                            &cancellation,
                            structure_bytes,
                        )?
                    } else {
                        None
                    };
                    Ok::<_, gmark_paged_document::PagedDocumentError>((
                        probe,
                        index,
                        document,
                        structured,
                        recovery,
                        reopened_encoding,
                    ))
                })
                .await;
            let reloaded = result.is_ok();
            let _ = this.update(cx, |view, cx| {
                if !task_stamp.accepts_strict(view, view.coordinator.index_generation) {
                    view.reloading = false;
                    return;
                }
                view.coordinator.index_cancellation = None;
                view.reloading = false;
                match result {
                    Ok((probe, index, document, structured, recovery, reopened_encoding)) => {
                        let Some(current_document) = view.document.as_ref() else {
                            view.error = Some(localized_document_error(
                                &gmark_paged_document::PagedDocumentError::InvalidTransaction(
                                    "shared document disappeared during reload".to_owned(),
                                ),
                                cx,
                            ));
                            return;
                        };
                        if let Err(error) = current_document.reload_prepared_document(
                            expected_revision,
                            expected_identity.clone(),
                            document,
                        ) {
                            view.error = Some(localized_document_error(
                                &gmark_paged_document::PagedDocumentError::InvalidTransaction(
                                    error.to_string(),
                                ),
                                cx,
                            ));
                            view.coordinator.pending_external_change =
                                Some(ExternalChange::Modified);
                            return;
                        }
                        let (replacement, recovery_creation_error) = match recovery {
                            Some(Ok(journal)) => (Some(journal), None),
                            Some(Err(error)) => (None, Some(error)),
                            None => (None, None),
                        };
                        if let Some(current) = view.document.clone() {
                            view.enqueue_recovery_checkpoint(&current, replacement, cx);
                        }
                        if let Some(error) = recovery_creation_error {
                            view.coordinator.recovery_error =
                                Some(localized_document_error(&error, cx));
                        }
                        view.probe = probe;
                        view.document_epoch = view.document_epoch.wrapping_add(1);
                        view.provisional_source = None;
                        view.index = Some(index);
                        view.invalidate_source_rows();
                        view.structured_index = structured;
                        view.invalidate_structured_runtime();
                        view.active_edit = None;
                        view.coordinator.pending_external_change = None;
                        view.coordinator.external_monitor_paused = false;
                        view.coordinator.external_status = Some(
                            cx.global::<I18nManager>()
                                .strings()
                                .large_document_text("reopened_as_template")
                                .replace("{encoding}", &reopened_encoding)
                                .into(),
                        );
                    }
                    Err(error) => view.error = Some(localized_document_error(&error, cx)),
                }
                cx.emit(DocumentHostEvent::StateChanged);
                cx.notify();
            });
            if reloaded {
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

    pub(super) fn keep_local_after_external_change(&mut self, cx: &mut Context<Self>) {
        self.coordinator.pending_external_change = None;
        self.coordinator.external_monitor_paused = true;
        self.tail_enabled = false;
        self.coordinator.external_status = Some(
            cx.global::<I18nManager>()
                .strings()
                .large_document_text("keeping_local")
                .into(),
        );
        cx.emit(DocumentHostEvent::StateChanged);
        cx.notify();
    }
}
