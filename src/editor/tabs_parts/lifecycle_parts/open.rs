// @author kongweiguang

use super::*;
use crate::editor::document_session::EditorDocumentSession;

impl Editor {
    pub(in crate::editor) fn show_pane_tab_close_prompt(&mut self, cx: &mut Context<Self>) {
        self.tabs.show_close_dialog = true;
        cx.notify();
    }

    /// 后台打开结果也经统一导航入口，避免候选尚未结束就拆卸原输入目标。
    pub(crate) fn install_new_tab(
        &mut self,
        opened: crate::document_io::OpenedMarkdown,
        path: PathBuf,
        cx: &mut Context<Self>,
    ) {
        if !self.can_switch_tabs() {
            return;
        }
        let snapshot = Self::snapshot_for_opened_document(opened, path);
        self.new_tab_from_snapshot(snapshot, cx);
    }

    /// Source Host 可以提前准备，但激活前必须等待旧视图的输入终态。
    pub(in crate::editor) fn install_new_source_backed_tab(
        &mut self,
        path: PathBuf,
        probe: gmark_paged_document::OpenProbe,
        source: gmark_paged_document::FileSource,
        cx: &mut Context<Self>,
    ) {
        if !self.can_switch_tabs() {
            return;
        }
        let structured_preview = probe.strategy == gmark_paged_document::OpenStrategy::Resident
            && matches!(
                probe.format,
                gmark_document_core::DocumentFormat::Json
                    | gmark_document_core::DocumentFormat::Delimited { .. }
            );
        let mut snapshot = Self::snapshot_for_untitled_document(DocumentKind::from_path(&path));
        snapshot.file_path = Some(path.clone());
        snapshot.saved_file_fingerprint = crate::recovery::fingerprint_file(&path).ok();
        snapshot.recovery_journal = None;
        snapshot.view_mode = if structured_preview {
            ViewMode::Preview
        } else {
            ViewMode::Source
        };
        let source_backed_view =
            cx.new(move |cx| crate::document_host::DocumentHost::new(path, probe, source, cx));
        Self::subscribe_document_host(&source_backed_view, cx);
        snapshot.document_host = Some(source_backed_view);
        self.new_tab_from_snapshot(snapshot, cx);
    }

    pub(crate) fn install_file_open_failure_tab(
        &mut self,
        path: PathBuf,
        reason: String,
        cx: &mut Context<Self>,
    ) {
        let snapshot = Self::snapshot_for_file_open_failure(path, reason);
        self.new_tab_from_snapshot(snapshot, cx);
    }

    pub(crate) fn install_image_preview_tab(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let snapshot = Self::snapshot_for_image_preview(path);
        self.new_tab_from_snapshot(snapshot, cx);
    }

    pub(crate) fn install_initial_image_preview(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let snapshot = Self::snapshot_for_image_preview(path);
        self.install_tab_snapshot(snapshot, cx);
        self.schedule_workspace_session_save(cx);
    }

    pub(crate) fn snapshot_for_image_preview(path: PathBuf) -> DocumentTabSnapshot {
        let mut snapshot = Self::snapshot_for_untitled_document(DocumentKind::from_path(&path));
        snapshot.file_path = Some(path.clone());
        snapshot.image_preview_path = Some(path);
        snapshot.image_preview_zoom = 1.0;
        snapshot.saved_file_fingerprint = None;
        snapshot.recovery_journal = None;
        snapshot.view_mode = ViewMode::Preview;
        snapshot
    }

    pub(crate) fn install_initial_file_open_failure(
        &mut self,
        path: PathBuf,
        reason: String,
        cx: &mut Context<Self>,
    ) {
        let snapshot = Self::snapshot_for_file_open_failure(path, reason);
        self.install_tab_snapshot(snapshot, cx);
        self.schedule_workspace_session_save(cx);
    }

    pub(crate) fn snapshot_for_file_open_failure(
        path: PathBuf,
        reason: String,
    ) -> DocumentTabSnapshot {
        let mut snapshot = Self::snapshot_for_untitled_document(DocumentKind::from_path(&path));
        snapshot.file_path = Some(path.clone());
        snapshot.file_open_failure = Some(FileOpenFailure {
            path: path.clone(),
            reason,
            action_error: None,
        });
        // 失败页没有可编辑正文，不需要变更检测；禁止在 UI 线程为大文件做全量指纹读取。
        snapshot.saved_file_fingerprint = None;
        snapshot.recovery_journal = None;
        snapshot.view_mode = ViewMode::Source;
        snapshot
    }

    pub(crate) fn snapshot_for_restored_document(
        tab: &RestoredTab,
        cx: &mut Context<Self>,
    ) -> Option<DocumentTabSnapshot> {
        match &tab.opened {
            crate::document_io::OpenedDocument::Resident(opened) => Some(
                Self::snapshot_for_opened_document(opened.clone(), tab.path.clone()),
            ),
            crate::document_io::OpenedDocument::ResidentFormat(probe)
            | crate::document_io::OpenedDocument::Paged(probe) => {
                let source = gmark_paged_document::FileSource::open(&tab.path).ok()?;
                let mut snapshot =
                    Self::snapshot_for_untitled_document(DocumentKind::from_path(&tab.path));
                snapshot.file_path = Some(tab.path.clone());
                snapshot.saved_file_fingerprint = crate::recovery::fingerprint_file(&tab.path).ok();
                snapshot.recovery_journal = None;
                snapshot.view_mode = if probe.strategy
                    == gmark_paged_document::OpenStrategy::Resident
                    && matches!(
                        probe.format,
                        gmark_document_core::DocumentFormat::Json
                            | gmark_document_core::DocumentFormat::Delimited { .. }
                    ) {
                    ViewMode::Preview
                } else {
                    ViewMode::Source
                };
                let path = tab.path.clone();
                let probe = probe.clone();
                let document_host = cx.new(move |cx| {
                    crate::document_host::DocumentHost::new(path, probe, source, cx)
                });
                Self::subscribe_document_host(&document_host, cx);
                snapshot.document_host = Some(document_host);
                Some(snapshot)
            }
            crate::document_io::OpenedDocument::Image => {
                Some(Self::snapshot_for_image_preview(tab.path.clone()))
            }
        }
    }

    pub(crate) fn snapshot_for_opened_document(
        opened: crate::document_io::OpenedMarkdown,
        path: PathBuf,
    ) -> DocumentTabSnapshot {
        let source_document = match EditorDocumentSession::try_new_with_open_context(
            SourceDocument::new(&opened.text),
            opened.loading_limits,
            opened.text_encoding.clone(),
            opened.file_identity.clone(),
        ) {
            Ok(source_document) => source_document,
            Err(error) => return Self::snapshot_for_file_open_failure(path, error.to_string()),
        };
        #[cfg(not(test))]
        let source = source_document.text();
        #[cfg(not(test))]
        let recovery_journal = crate::config::AppDirs::from_system()
            .and_then(|dirs| {
                let recovery_dir = dirs.recovery_dir();
                dirs.ensure_state_parent(&recovery_dir.join(".gmark-recovery-root"))?;
                crate::recovery::RecoveryJournal::create(
                    &recovery_dir,
                    Some(path.clone()),
                    source.clone(),
                )
            })
            .map(|journal| Arc::new(Mutex::new(journal)))
            .ok();
        #[cfg(test)]
        let recovery_journal = None;
        let requires_conversion = !opened.encoding.is_utf8();
        DocumentTabSnapshot {
            document_host: None,
            source_document,
            shared_document: false,
            source_encoding: opened.encoding,
            document_kind: DocumentKind::from_path(&path),
            file_path: Some(path.clone()),
            image_preview_path: None,
            image_preview_zoom: 1.0,
            file_open_failure: None,
            saved_file_fingerprint: crate::recovery::fingerprint_file(&path).ok(),
            document_dirty: false,
            view_mode: if requires_conversion || crate::document_io::is_svg_path(&path) {
                ViewMode::Preview
            } else {
                ViewMode::Rendered
            },
            selection: UndoSelectionSnapshot::collapsed(
                0,
                gmark_document_core::SourceAffinity::Before,
            ),
            scroll_offset: point(px(0.0), px(0.0)),
            undo_history: Vec::new(),
            redo_history: Vec::new(),
            pending_undo_capture: None,
            virtual_undo_selections: Vec::new(),
            virtual_redo_selections: Vec::new(),
            pending_virtual_undo_selection: None,
            recovery_journal,
            external_file_conflict: false,
            recovered_session: false,
            show_encoding_conversion_dialog: requires_conversion,
            external_conflict_preview: None,
            allow_external_overwrite_once: false,
        }
    }

    pub(super) fn snapshot_for_untitled_document(
        document_kind: DocumentKind,
    ) -> DocumentTabSnapshot {
        let source = document_kind.initial_source();
        let source_document = EditorDocumentSession::new(SourceDocument::new(source));
        #[cfg(not(test))]
        let recovery_journal = crate::config::AppDirs::from_system()
            .and_then(|dirs| {
                let recovery_dir = dirs.recovery_dir();
                dirs.ensure_state_parent(&recovery_dir.join(".gmark-recovery-root"))?;
                crate::recovery::RecoveryJournal::create(&recovery_dir, None, source.to_owned())
            })
            .map(|journal| Arc::new(Mutex::new(journal)))
            .ok();
        #[cfg(test)]
        let recovery_journal = None;
        DocumentTabSnapshot {
            document_host: None,
            source_document,
            shared_document: false,
            source_encoding: crate::document_io::DocumentEncoding::Utf8,
            document_kind,
            file_path: None,
            image_preview_path: None,
            image_preview_zoom: 1.0,
            file_open_failure: None,
            saved_file_fingerprint: None,
            document_dirty: false,
            view_mode: document_kind.initial_view_mode(),
            selection: UndoSelectionSnapshot::collapsed(
                0,
                gmark_document_core::SourceAffinity::Before,
            ),
            scroll_offset: point(px(0.0), px(0.0)),
            undo_history: Vec::new(),
            redo_history: Vec::new(),
            pending_undo_capture: None,
            virtual_undo_selections: Vec::new(),
            virtual_redo_selections: Vec::new(),
            pending_virtual_undo_selection: None,
            recovery_journal,
            external_file_conflict: false,
            recovered_session: false,
            show_encoding_conversion_dialog: false,
            external_conflict_preview: None,
            allow_external_overwrite_once: false,
        }
    }

    pub(crate) fn new_untitled_tab(&mut self, cx: &mut Context<Self>) -> bool {
        self.new_document_tab(DocumentKind::Markdown, cx)
    }

    pub(crate) fn new_untyped_tab(&mut self, cx: &mut Context<Self>) -> bool {
        self.new_document_tab(DocumentKind::Unspecified, cx)
    }

    pub(crate) fn new_document_tab(
        &mut self,
        document_kind: DocumentKind,
        cx: &mut Context<Self>,
    ) -> bool {
        let mut snapshot = Self::snapshot_for_untitled_document(document_kind);
        if let Some(format) = document_kind.document_host_format() {
            let logical_path = PathBuf::from(document_kind.untitled_name());
            let initial_source = document_kind.initial_source();
            let document_host = cx.new(move |cx| {
                crate::document_host::DocumentHost::new_untitled(
                    logical_path,
                    format,
                    initial_source,
                    cx,
                )
            });
            Self::subscribe_document_host(&document_host, cx);
            snapshot.document_host = Some(document_host);
        }
        self.new_tab_from_snapshot(snapshot, cx)
    }

    /// 新标签数据准备完成也先等待输入终态，不能拆卸正在接收系统回调的旧视图。
    pub(crate) fn new_tab_from_snapshot(
        &mut self,
        snapshot: DocumentTabSnapshot,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.has_active_ime_composition(cx) {
            self.queue_ime_operation(
                crate::editor::ime_lifecycle::DeferredImeOperation::NewTab(Box::new(snapshot)),
                cx,
            );
            return false;
        }
        if !self.can_switch_tabs() {
            return false;
        }
        let current = self.capture_active_tab(cx);
        self.tabs.records[self.tabs.active].snapshot = Some(current);
        self.tabs.records.push(TabRecord {
            id: uuid::Uuid::new_v4(),
            pinned: false,
            snapshot: None,
        });
        self.tabs.active = self.tabs.records.len() - 1;
        self.install_tab_snapshot(snapshot, cx);
        self.schedule_workspace_session_save(cx);
        true
    }

    /// Activate an already-open process-wide document without reading or
    /// writing its target path.  The moved session/lease becomes the new tab's
    /// view; dropping this tab later releases only that view lease.
    pub(in crate::editor) fn open_shared_document_tab(
        &mut self,
        source_document: EditorDocumentSession,
        path: PathBuf,
        source_encoding: crate::document_io::DocumentEncoding,
        cx: &mut Context<Self>,
    ) -> bool {
        let snapshot =
            DocumentTabSnapshot::from_shared_document(source_document, path, source_encoding);
        self.new_tab_from_snapshot(snapshot, cx)
    }

    pub(in crate::editor) fn open_shared_document_host_tab(
        &mut self,
        open: crate::app::document_service::SharedDocumentHostOpen,
        cx: &mut Context<Self>,
    ) -> bool {
        let dirty = open
            .lease
            .handle()
            .lock()
            .map(|controller| controller.session().dirty)
            .unwrap_or(true);
        let view_id = gmark_document_core::DocumentViewInstanceId::new();
        let crate::app::document_service::SharedDocumentHostOpen {
            lease,
            probe,
            file_path,
            encoding,
            ..
        } = open;
        let handle = lease.handle();
        let host_path = file_path.clone();
        let host = cx.new(move |cx| {
            crate::document_host::DocumentHost::from_shared_with_view_id_or_error(
                host_path,
                probe,
                handle,
                lease,
                view_id,
                crate::document_host::DocumentHostViewPresentation::default(),
                cx,
            )
        });
        let snapshot = DocumentTabSnapshot::from_shared_host(host, file_path, encoding, dirty);
        self.new_tab_from_snapshot(snapshot, cx)
    }

    pub(super) fn snapshot_for_recovered_document(
        recovered: crate::recovery::RecoveredDocument,
    ) -> DocumentTabSnapshot {
        if recovered.read_status == crate::recovery::RecoveryReadStatus::TruncatedTail {
            eprintln!(
                "recovery journal '{}' had a corrupt tail; restored the last CRC-valid record",
                recovered.journal_path.display()
            );
        }
        if recovered.base_file_changed {
            eprintln!(
                "recovered document base changed externally: {}",
                recovered.file_path.as_deref().map_or_else(
                    || "<untitled>".to_owned(),
                    |path| path.display().to_string()
                )
            );
        }
        let selection =
            UndoSelectionSnapshot::from_source_selection(recovered.selection.source_selection());
        let view_mode = match recovered.view_mode.as_str() {
            "source" => ViewMode::Source,
            "split" => ViewMode::Split,
            "preview" => ViewMode::Preview,
            _ => ViewMode::Rendered,
        };
        let file_path = recovered.file_path.clone();
        let source = recovered.source.clone();
        let source_document = match EditorDocumentSession::try_new_with_initial_dirty(
            SourceDocument::new(&source),
            true,
        ) {
            Ok(source_document) => source_document,
            Err(error) => {
                eprintln!("recovery tab initialization failed: {error}");
                return Self::snapshot_for_file_open_failure(
                    recovered.file_path.unwrap_or(recovered.journal_path),
                    error.to_string(),
                );
            }
        };
        match source_document.try_restore_source_format(recovered.source_format.clone()) {
            Ok(true) => {}
            Ok(false) => eprintln!("恢复日志中的源码格式与恢复文本不匹配，已使用默认格式"),
            Err(error) => eprintln!("恢复日志中的源码格式提交失败: {error}"),
        }
        DocumentTabSnapshot {
            document_host: None,
            source_document,
            shared_document: false,
            source_encoding: crate::document_io::DocumentEncoding::Utf8,
            document_kind: file_path
                .as_deref()
                .map(DocumentKind::from_path)
                .unwrap_or(DocumentKind::Markdown),
            saved_file_fingerprint: file_path
                .as_deref()
                .and_then(|path| crate::recovery::fingerprint_file(path).ok()),
            file_path,
            image_preview_path: None,
            image_preview_zoom: 1.0,
            file_open_failure: None,
            document_dirty: true,
            view_mode,
            selection,
            scroll_offset: point(px(0.0), px(0.0)),
            undo_history: Vec::new(),
            redo_history: Vec::new(),
            pending_undo_capture: None,
            virtual_undo_selections: Vec::new(),
            virtual_redo_selections: Vec::new(),
            pending_virtual_undo_selection: None,
            recovery_journal: Some(Arc::new(Mutex::new(
                crate::recovery::RecoveryJournal::resume(&recovered),
            ))),
            external_file_conflict: recovered.base_file_changed,
            recovered_session: true,
            show_encoding_conversion_dialog: false,
            external_conflict_preview: None,
            allow_external_overwrite_once: false,
        }
    }

    pub(crate) fn append_recovered_tabs(
        &mut self,
        recovered: Vec<crate::recovery::RecoveredDocument>,
        cx: &mut Context<Self>,
    ) {
        for recovered in recovered {
            self.tabs.records.push(TabRecord {
                id: uuid::Uuid::new_v4(),
                pinned: false,
                snapshot: Some(Self::snapshot_for_recovered_document(recovered)),
            });
        }
        self.schedule_workspace_session_save(cx);
        cx.notify();
    }
}
