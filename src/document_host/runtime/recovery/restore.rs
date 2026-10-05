// @author kongweiguang

//! Journal replay and recovered-session installation.

use super::coordinator::RecoveredAutoSaveGate;
use super::*;

impl DocumentHost {
    pub(crate) fn from_recovery(
        path: PathBuf,
        probe: OpenProbe,
        source: FileSource,
        journal_path: PathBuf,
        cx: &mut Context<Self>,
    ) -> Self {
        let recovery_started = crate::perf::start();
        let recovery_profile = probe.profile();
        let recovery_plan = session_plan(&recovery_profile, &probe, probe.strategy, false);
        let recovery_format = probe.format.clone();
        let recovered_structure_enabled = derived_views_enabled(probe.strategy);
        let fallback_source = source.clone();
        let fallback_encoding = probe.encoding.clone();
        // Recovery must win before a compatibility host can publish a regular
        // controller body; keep the host unbound until the replayed session is
        // ready, then install exactly one shared controller.
        let mut view = Self::new_with_source(path, probe, Some(source), cx);
        // 替换普通索引任务；Task drop 会取消尚未发布的普通打开结果，恢复日志始终胜出。
        if let Some(cancellation) = view.coordinator.index_cancellation.take() {
            cancellation.cancel();
        }
        let cancellation = SearchCancellation::default();
        view.coordinator.index_cancellation = Some(cancellation.clone());
        view.coordinator.index_generation = view.coordinator.index_generation.wrapping_add(1);
        let task_stamp = DocumentTaskStamp::capture(&view, view.coordinator.index_generation);
        view.coordinator.index_task = cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    match replay_paged_recovery(&journal_path) {
                        Ok(recovered) => {
                            // Resident 恢复文档的磁盘基线已经过期；结构视图必须从恢复后的
                            // PieceDocument 快照构建，不能继续读取原文件，也不必等到保存。
                            let structured = (|| {
                                if !recovered_structure_enabled {
                                    return Ok(None);
                                }
                                let bytes: Arc<[u8]> = recovered
                                    .document
                                    .read_range_cancellable(
                                        0..recovered.document.len(),
                                        &cancellation,
                                    )?
                                    .into();
                                build_structured_index(
                                    recovered.prepared_source.source(),
                                    &recovered.document.line_index(),
                                    recovery_format.clone(),
                                    &cancellation,
                                    Some(bytes),
                                )
                            })();
                            Ok((Some((recovered, structured)), None))
                        }
                        Err(recovery_error) => {
                            if cancellation.is_cancelled() {
                                return Err(PagedDocumentError::Cancelled);
                            }
                            let prepared = prepare_utf8_source(fallback_source, fallback_encoding)?;
                            let index =
                                LineIndex::build_cancellable(prepared.source(), &cancellation)?;
                            let document =
                                PieceDocument::open(prepared.source().clone(), index.clone())?;
                            Ok::<_, gmark_paged_document::PagedDocumentError>((
                                None,
                                Some((prepared, index, document, recovery_error)),
                            ))
                        }
                    }
                })
                .await;
            if let Some(started) = recovery_started {
                let (success, detail) = match &result {
                    Ok((Some(_), _)) => (true, "replayed"),
                    Ok((None, _)) => (false, "fallback"),
                    Err(_) => (false, "failed"),
                };
                crate::perf::emit_document(
                    "document_recovery",
                    started,
                    usize::try_from(recovery_profile.len).ok(),
                    Some(success),
                    &recovery_profile.format,
                    &recovery_plan,
                    Some(detail),
                );
            }
            let _ = this.update(cx, |view, cx| {
                if !task_stamp.accepts_strict(view, view.coordinator.index_generation) {
                    return;
                }
                view.coordinator.index_cancellation = None;
                match result {
                    Ok((Some((recovered, structured)), _)) => {
                        let strings = cx.global::<I18nManager>().strings_arc();
                        let selection = recovered.selection;
                        let selected_line = selection.as_ref().and_then(|selection| {
                            recovered
                                .document
                                .line_for_offset(selection.range().end)
                                .and_then(|line| usize::try_from(line).ok())
                        });
                        view.index = Some(recovered.document.line_index());
                        let identity = recovered.prepared_source.source().identity();
                        let mut document = match identity.and_then(|identity| {
                            build_paged_session(&view.probe, recovered.document, identity)
                        }) {
                            Ok(document) => document,
                            Err(error) => {
                                view.error = Some(localized_document_error(&error, cx));
                                return;
                            }
                        };
                        // Replayed recovery content is intentionally ahead of
                        // the on-disk baseline; carry that fact in the shared
                        // Controller session before publishing it.
                        document.dirty = true;
                        if let Some(selection) = selection {
                            document_view_state_mut(&mut view.document, &mut view.tab_view_state)
                                .source
                                .selection = selection;
                            document_view_state_mut(&mut view.document, &mut view.tab_view_state)
                                .source
                                .top_byte_anchor = selection.head;
                        }
                        view.install_document_session(document);
                        if let Some(document) = view.document.as_ref() {
                            RecoveredAutoSaveGate::require_explicit_save(document);
                        }
                        view.provisional_source = None;
                        view.invalidate_source_rows();
                        view.install_recovery_journal(
                            DocumentRecoveryJournal::Paged(recovered.journal),
                            cx,
                        );
                        view.coordinator.recovery_error = (recovered.read_status
                            == gmark_paged_document::PagedRecoveryReadStatus::TruncatedTail)
                            .then(|| strings.large_document_text("recovered_tail").into());
                        view.invalidate_structured_runtime();
                        match structured {
                            Ok(index) => {
                                view.structured_index = index;
                                view.clear_structure_error();
                            }
                            Err(error) => {
                                view.structured_index = None;
                                view.set_structure_error(error, cx);
                            }
                        }
                        view.view_mode = DocumentHostViewMode::Source;
                        view.sync_tab_active_view();
                        view.tail_enabled = false;
                        if let Some(line) = selected_line {
                            view.selection_anchor = Some(line);
                            view.selected_lines = Some(line..line.saturating_add(1));
                            view.scroll_handle
                                .scroll_to_item(line, ScrollStrategy::Center);
                        }
                    }
                    Ok((None, Some((prepared, index, document, recovery_error)))) => {
                        let strings = cx.global::<I18nManager>().strings_arc();
                        view.index = Some(index);
                        let identity = match prepared.source().identity() {
                            Ok(identity) => identity,
                            Err(error) => {
                                view.error = Some(localized_document_error(&error, cx));
                                return;
                            }
                        };
                        let document = match build_paged_session(&view.probe, document, identity) {
                            Ok(document) => document,
                            Err(error) => {
                                view.error = Some(localized_document_error(&error, cx));
                                return;
                            }
                        };
                        view.install_document_session(document);
                        view.provisional_source = None;
                        view.invalidate_source_rows();
                        view.coordinator.recovery_error = Some(
                            strings
                                .large_document_text("recovery_conflict_template")
                                .replace("{error}", &recovery_error.to_string())
                                .into(),
                        );
                        view.view_mode = DocumentHostViewMode::Source;
                        view.sync_tab_active_view();
                        view.tail_enabled = false;
                    }
                    Ok((None, None)) => {}
                    Err(error) => {
                        view.error = Some(
                            cx.global::<I18nManager>()
                                .strings()
                                .large_document_error(&error)
                                .into(),
                        )
                    }
                }
                cx.notify();
            });
        });
        view
    }
}
