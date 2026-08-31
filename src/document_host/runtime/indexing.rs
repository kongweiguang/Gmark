// @author kongweiguang

//! Initial indexing and session installation.

use super::*;

impl DocumentHost {
    /// 空文件的正文和行索引都可以在固定成本内完成；先安装这一份权威 session，
    /// 让首帧 Source 行直接拥有可提交的 transaction，而不是把用户首字符送进
    /// 尚未安装 backend 的 provisional Block。任何身份或解码失败都留在调用方显示，
    /// 不会为了这条快路径把大文件扫描搬回 UI 线程。
    fn install_empty_file_session(
        &mut self,
        source: &FileSource,
        _cx: &mut Context<Self>,
    ) -> Result<(), gmark_paged_document::PagedDocumentError> {
        // Probe 与 session 安装之间若文件已经增长，必须回到后台重探测路径；否则
        // 不能把一个实际的大文件误当成空文件并在 UI 线程扫描它的行索引。
        if source.identity()? != self.probe.identity {
            return Err(PagedDocumentError::SourceChanged);
        }
        let prepared = prepare_utf8_source(source.clone(), self.probe.encoding.clone())?;
        if prepared.source().identity()?.len != 0 {
            return Err(PagedDocumentError::SourceChanged);
        }
        let index = LineIndex::build(prepared.source())?;
        let document = build_document_session_from_prepared(
            &self.probe,
            source,
            prepared,
            index.clone(),
            false,
        )?;
        #[cfg(not(test))]
        let recovery_document = document.clone();
        self.index = Some(index);
        self.install_document_session(document);
        if self.document.is_none() {
            return Err(PagedDocumentError::InvalidTransaction(
                "empty file document controller initialization failed".into(),
            ));
        }
        self.provisional_source = None;
        self.provisional_anchor = None;
        self.invalidate_source_rows();
        self.install_empty_source_row();
        #[cfg(not(test))]
        self.start_empty_recovery_journal(source.clone(), recovery_document, _cx);
        Ok(())
    }

    #[cfg(not(test))]
    /// Resolve the recovery path without touching disk on the UI thread, then
    /// create the empty-file journal in the background.  Empty documents still
    /// become editable immediately; a failed journal setup only degrades
    /// recovery and never blocks Controller/session installation.
    fn start_empty_recovery_journal(
        &mut self,
        source: FileSource,
        document: DocumentSession,
        cx: &mut Context<Self>,
    ) {
        let recovery_already_started = self.coordinator.recovery_journal.is_some()
            || self.coordinator.recovery_worker.is_some();
        if recovery_already_started {
            return;
        }
        // Mark recovery as enabled before resolving directories so an AppDirs
        // failure still keeps the first Resident edit in the bounded in-memory
        // handoff instead of silently dropping it with recovery disabled.
        self.coordinator.recovery_enabled = true;
        let recovery_dirs = match gmark_config::AppDirs::from_system() {
            Ok(dirs) => dirs,
            Err(error) => {
                self.coordinator.recovery_error = Some(error.to_string().into());
                return;
            }
        };
        let recovery_dir = recovery_dirs.recovery_dir();
        let encoding = self.probe.encoding.clone();
        let document_epoch = self.document_epoch;
        let recovery_generation = self.coordinator.recovery_generation;
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    recovery_dirs
                        .ensure_state_parent(&recovery_dir.join(".gmark-recovery-root"))
                        .map_err(|error| PagedDocumentError::Recovery(error.to_string()))?;
                    DocumentRecoveryJournal::create(&recovery_dir, &source, encoding, &document)
                })
                .await;
            let _ = this.update(cx, |view, cx| {
                if view.document_epoch != document_epoch
                    || view.coordinator.recovery_generation != recovery_generation
                    || view.document.is_none()
                    || view.coordinator.recovery_journal.is_some()
                    || view.coordinator.recovery_worker.is_some()
                {
                    return;
                }
                match result {
                    Ok(journal) => view.install_recovery_journal(journal, cx),
                    Err(error) => {
                        view.coordinator.recovery_error = Some(error.to_string().into());
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }

    /// 初次打开和关闭标签后的恢复共用同一条索引管线。任务结果由 document epoch、
    /// revision 与 generation 三重门禁，关闭期间取消的旧 worker 永远不能重新安装。
    pub(super) fn start_initial_index(&mut self, cx: &mut Context<Self>) {
        if self.document.is_some() || self.coordinator.index_cancellation.is_some() {
            return;
        }
        let Some(worker_source) = self.provisional_source.clone() else {
            self.error = Some(
                cx.global::<I18nManager>()
                    .strings()
                    .large_document_text("source_backend_unavailable")
                    .into(),
            );
            return;
        };
        if self.probe.len == 0 {
            match self.install_empty_file_session(&worker_source, cx) {
                Ok(()) => return,
                // Probe 与打开句柄之间的增长/替换属于现有后台重探测契约；不能把
                // 这个可恢复竞态误报成 UI 线程失败页。
                Err(PagedDocumentError::SourceChanged) => {}
                Err(error) => {
                    self.error = Some(localized_document_error(&error, cx));
                    return;
                }
            }
        }
        let probe = self.probe.clone();
        let recovery_dirs = runtime_recovery_dirs();
        if recovery_dirs.is_some() {
            self.coordinator.recovery_enabled = true;
        }
        let index_cache_dir = match gmark_config::AppDirs::from_system() {
            Ok(dirs) => {
                let cache_dir = dirs.large_document_indexes_dir();
                match dirs.ensure_cache_parent(&cache_dir.join(".gmark-index-root")) {
                    Ok(()) => Some(cache_dir),
                    Err(error) => {
                        eprintln!("large-document index cache disabled: {error:#}");
                        None
                    }
                }
            }
            Err(error) => {
                eprintln!("large-document index cache disabled: {error:#}");
                None
            }
        };
        let index_cancellation = SearchCancellation::default();
        let index_worker_cancellation = index_cancellation.clone();
        self.coordinator.index_cancellation = Some(index_cancellation);
        self.coordinator.index_generation = self.coordinator.index_generation.wrapping_add(1);
        let task_stamp = DocumentTaskStamp::capture(self, self.coordinator.index_generation);
        self.coordinator.index_task = cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let path = worker_source.path().to_path_buf();
                    let options = probe.options;
                    let force_safe_source = probe.force_safe_source;
                    let mut candidate_probe = probe;
                    for attempt in 0..3 {
                        // Probe 与完整读取之间可能发生替换或增长。任何 identity 变化都用
                        // 原 ProbeOptions 重新规划，并重开稳定句柄。
                        let mut worker_source = FileSource::open(&path)?;
                        if worker_source.identity()? != candidate_probe.identity {
                            candidate_probe = gmark_paged_document::probe_file(&path, options)?;
                            candidate_probe.force_safe_source = force_safe_source;
                            worker_source = FileSource::open(&path)?;
                            if worker_source.identity()? != candidate_probe.identity {
                                if attempt < 2 {
                                    continue;
                                }
                                return Err(PagedDocumentError::SourceChanged);
                            }
                        }
                        let probe = candidate_probe.clone();
                        let build = (|| {
                            let direct_utf8 = matches!(probe.encoding, TextEncoding::Utf8 { .. });
                            let encoding = probe.encoding.clone();
                            let recovery_source = worker_source.clone();
                            let prepared = prepare_utf8_source(worker_source, encoding.clone())?;
                            let source = prepared.source().clone();
                            let index = if direct_utf8 {
                                if let Some(cache_dir) = index_cache_dir.as_ref() {
                                    match LineIndex::build_cached_cancellable(
                                        &source,
                                        cache_dir,
                                        &index_worker_cancellation,
                                    ) {
                                        Ok(index) => index,
                                        Err(PagedDocumentError::Io { .. }) => {
                                            eprintln!(
                                                "large-document line-index cache write failed; using uncached build"
                                            );
                                            LineIndex::build_cancellable(
                                                &source,
                                                &index_worker_cancellation,
                                            )?
                                        }
                                        Err(error) => return Err(error),
                                    }
                                } else {
                                    LineIndex::build_cancellable(
                                        &source,
                                        &index_worker_cancellation,
                                    )?
                                }
                            } else {
                                LineIndex::build_cancellable(&source, &index_worker_cancellation)?
                            };
                            let document = build_document_session_from_prepared(
                                &probe,
                                &recovery_source,
                                prepared,
                                index.clone(),
                                false,
                            )?;
                            let (structure_source, structure_index, structure_bytes) =
                                structure_input_for_session(
                                    &document,
                                    &source,
                                    &index,
                                    &index_worker_cancellation,
                                )?;
                            let recovery = recovery_dirs.as_ref().map(|dirs| {
                                let recovery_dir = dirs.recovery_dir();
                                dirs.ensure_state_parent(
                                    &recovery_dir.join(".gmark-recovery-root"),
                                )
                                .map_err(|error| {
                                    PagedDocumentError::Recovery(error.to_string())
                                })
                                .and_then(|()| {
                                    DocumentRecoveryJournal::create(
                                        &recovery_dir,
                                        &recovery_source,
                                        encoding.clone(),
                                        &document,
                                    )
                                })
                            });
                            Ok::<_, PagedDocumentError>((
                                probe,
                                index,
                                document,
                                recovery,
                                structure_source,
                                structure_index,
                                structure_bytes,
                            ))
                        })();
                        match build {
                            Err(PagedDocumentError::SourceChanged) if attempt < 2 => {
                                candidate_probe = gmark_paged_document::probe_file(&path, options)?;
                                candidate_probe.force_safe_source = force_safe_source;
                            }
                            result => return result,
                        }
                    }
                    Err(PagedDocumentError::SourceChanged)
                })
                .await;
            let _ = this.update(cx, |view, cx| {
                if !task_stamp.accepts_strict(view, view.coordinator.index_generation) {
                    return;
                }
                view.coordinator.index_cancellation = None;
                match result {
                    Ok((
                        probe,
                        index,
                        document,
                        recovery,
                        structure_source,
                        structure_index,
                        structure_bytes,
                    )) => {
                        let strategy_changed = view.probe.strategy != probe.strategy;
                        view.probe = probe;
                        let format = view.probe.format.clone();
                        if view.probe.strategy == OpenStrategy::Paged {
                            view.view_mode = DocumentHostViewMode::Source;
                            view.sync_tab_active_view();
                        }
                        let anchor_line = view
                            .provisional_anchor
                            .and_then(|anchor| document.line_for_offset(anchor.byte_offset))
                            .and_then(|line| usize::try_from(line).ok());
                        view.index = Some(index);
                        view.install_document_session(document);
                        view.provisional_source = None;
                        view.provisional_anchor = None;
                        view.invalidate_source_rows();
                        if let Some(line) = anchor_line {
                            view.scroll_source_line(line, ScrollStrategy::Top);
                        }
                        if let Some(recovery) = recovery {
                            match recovery {
                                Ok(journal) => view.install_recovery_journal(journal, cx),
                                Err(error) => {
                                    view.coordinator.recovery_error = Some(
                                        cx.global::<I18nManager>()
                                            .strings()
                                            .large_document_error(&error)
                                            .into(),
                                    )
                                }
                            }
                        }
                        view.schedule_search(cx);
                        if !derived_views_enabled(view.probe.strategy) {
                            // Paged 安全模式只安装 Source，不启动任何需要全文结构化扫描的任务。
                            view.clear_structure_error();
                            if strategy_changed {
                                cx.emit(DocumentHostEvent::ViewModeChanged(
                                    DocumentHostMode::Source,
                                ));
                            }
                            cx.emit(DocumentHostEvent::StateChanged);
                            cx.notify();
                            return;
                        }
                        if format == DocumentFormat::Json
                            && view.view_mode != DocumentHostViewMode::Source
                        {
                            view.request_registered_projection(cx);
                        }
                        if format == DocumentFormat::Json {
                            view.clear_structure_error();
                            cx.emit(DocumentHostEvent::StateChanged);
                            cx.notify();
                            return;
                        }
                        view.structured_generation = view.structured_generation.wrapping_add(1);
                        let generation = view.structured_generation;
                        let structured_task_stamp = DocumentTaskStamp::capture(view, generation);
                        if !matches!(format, DocumentFormat::PlainText) {
                            view.structure_error = Some(
                                cx.global::<I18nManager>()
                                    .strings()
                                    .large_document_text("indexing_structured")
                                    .into(),
                            );
                            view.structure_error_byte = None;
                        }
                        let structure_cancellation = SearchCancellation::default();
                        view.structured_cancellation = Some(structure_cancellation.clone());
                        view.structured_task = cx.spawn(async move |this, cx| {
                            let structured = cx
                                .background_spawn(async move {
                                    build_structured_index(
                                        &structure_source,
                                        &structure_index,
                                        format,
                                        &structure_cancellation,
                                        structure_bytes,
                                    )
                                })
                                .await;
                            let _ = this.update(cx, |view, cx| {
                                if !structured_task_stamp
                                    .accepts_strict(view, view.structured_generation)
                                    || document_dirty_state(&view.document)
                                {
                                    return;
                                }
                                view.structured_cancellation = None;
                                match structured {
                                    Ok(Some(structured)) => {
                                        view.structured_index = Some(structured);
                                        view.clear_structure_error();
                                    }
                                    Ok(None) => {
                                        view.clear_structure_error();
                                        if matches!(
                                            view.view_mode,
                                            DocumentHostViewMode::Live
                                                | DocumentHostViewMode::Structure
                                        ) {
                                            view.view_mode = DocumentHostViewMode::Source;
                                            view.sync_tab_active_view();
                                        }
                                    }
                                    Err(error) => {
                                        view.set_structure_error(error, cx);
                                        if matches!(
                                            view.view_mode,
                                            DocumentHostViewMode::Live
                                                | DocumentHostViewMode::Structure
                                        ) {
                                            view.view_mode = DocumentHostViewMode::Source;
                                            view.sync_tab_active_view();
                                        }
                                    }
                                }
                                cx.emit(DocumentHostEvent::StateChanged);
                                cx.notify();
                            });
                        });
                    }
                    Err(error) => {
                        view.error = Some(
                            cx.global::<I18nManager>()
                                .strings()
                                .large_document_error(&error)
                                .into(),
                        )
                    }
                }
                cx.emit(DocumentHostEvent::StateChanged);
                cx.notify();
            });
        });
    }
}
