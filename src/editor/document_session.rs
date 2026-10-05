// @author kongweiguang

//! Markdown Editor 对共享 DocumentController 的窄适配器。
//!
//! `DocumentHandle` 才是正文、revision、撤销历史和保存基线的唯一所有者。
//! 本层只保存一个视图实例与其租约；所有读取都在短暂的 Controller 锁内完成，
//! 不在 Editor 中缓存第二份正文。

use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

use gmark_document::{
    DocumentError, DocumentSnapshot, LineEnding, Revision, SourceDocument, SourceFormatSnapshot,
    SourceFormatSummary, Transaction,
};
use gmark_document_core::{
    DocumentFormat, DocumentMutationMap, DocumentProfile, DocumentRevision, DocumentViewInstanceId,
    LoadingLimits, LoadingPolicy, OpenReason, SourceEdit, SourceSelection, TextEncoding,
    Transaction as RuntimeTransaction,
};
use gmark_document_runtime::{
    ControllerError, DocumentCommand, DocumentController, DocumentEvent, DocumentEventSubscription,
    DocumentHandle, DocumentId, DocumentLease, DocumentSaveSnapshot, DocumentSession,
    DocumentStateSnapshot, DocumentStore, FileIdentity, ResidentDocument, SaveFailureCode,
    TransactionId,
};

#[path = "document_session_parts/reload.rs"]
mod reload;
#[path = "document_session_parts/types.rs"]
mod session_types;
pub(crate) use session_types::DocumentEventPoll;
use session_types::ViewLease;
pub(crate) use session_types::{EditorDocumentSession, EditorDocumentSessionError};

impl EditorDocumentSession {
    /// Construct a lease-free shell for UI-only tabs.  Shells deliberately do
    /// not allocate a Controller, registry lease, or view registration.
    pub(super) fn shell() -> Self {
        Self {
            handle: None,
            view: None,
        }
    }

    pub(super) fn new(source: SourceDocument) -> Self {
        Self::new_with_open_context(
            source,
            LoadingPolicy::default().effective_limits(),
            TextEncoding::Utf8 { bom: false },
            None,
        )
    }

    pub(super) fn try_new_with_initial_dirty(
        source: SourceDocument,
        dirty: bool,
    ) -> Result<Self, EditorDocumentSessionError> {
        Self::try_new_with_open_context_and_dirty(
            source,
            LoadingPolicy::default().effective_limits(),
            TextEncoding::Utf8 { bom: false },
            None,
            dirty,
        )
    }

    pub(super) fn new_with_open_context(
        source: SourceDocument,
        limits: LoadingLimits,
        text_encoding: TextEncoding,
        source_identity: Option<gmark_paged_document::FileIdentity>,
    ) -> Self {
        match Self::try_new_with_open_context(source, limits, text_encoding, source_identity) {
            Ok(session) => session,
            Err(error) => panic!("Markdown Resident session construction failed: {error}"),
        }
    }

    pub(super) fn try_new_with_open_context(
        source: SourceDocument,
        limits: LoadingLimits,
        text_encoding: TextEncoding,
        source_identity: Option<gmark_paged_document::FileIdentity>,
    ) -> Result<Self, EditorDocumentSessionError> {
        Self::try_new_with_open_context_and_dirty(
            source,
            limits,
            text_encoding,
            source_identity,
            false,
        )
    }

    /// 新文档仍由此入口创建 Controller，避免调用方直接组装共享运行时状态。
    pub(super) fn try_new_with_open_context_and_dirty(
        source: SourceDocument,
        limits: LoadingLimits,
        text_encoding: TextEncoding,
        source_identity: Option<gmark_paged_document::FileIdentity>,
        initial_dirty: bool,
    ) -> Result<Self, EditorDocumentSessionError> {
        let session = Self::try_prepare_session_with_open_context_and_dirty(
            source,
            limits,
            text_encoding,
            source_identity,
            initial_dirty,
        )?;
        let handle = DocumentHandle::new(DocumentController::new(DocumentId::new(), session));
        Self::from_handle(handle)
    }

    /// 让新建与原位重载共用同一构造路径，保留读取时冻结的编码、身份和内存上限。
    pub(super) fn try_prepare_session_with_open_context_and_dirty(
        source: SourceDocument,
        limits: LoadingLimits,
        text_encoding: TextEncoding,
        source_identity: Option<gmark_paged_document::FileIdentity>,
        initial_dirty: bool,
    ) -> Result<DocumentSession, EditorDocumentSessionError> {
        let len = source.len() as u64;
        let estimated_lines = source.text().lines().count().max(1) as u64;
        let profile = DocumentProfile {
            format: DocumentFormat::Markdown,
            encoding: text_encoding,
            len,
            estimated_lines,
            estimated_structural_units: 0,
        };
        let plan = LoadingPolicy {
            max_resident_bytes: Some(limits.max_resident_bytes),
            ..LoadingPolicy::default()
        }
        .resolve(&profile);
        let paged_identity =
            source_identity.unwrap_or_else(|| gmark_paged_document::FileIdentity {
                path: PathBuf::new(),
                len,
                modified_nanos: None,
                os_file_id: None,
            });
        let identity = FileIdentity::from(&paged_identity);
        let store = DocumentStore::Resident(Box::new(ResidentDocument::from_source_document(
            source,
            profile.encoding.clone(),
            paged_identity,
        )));
        let mut session = DocumentSession::new(profile, store, plan, identity)
            .map_err(|error| ControllerError::open_failed(error.to_string()))?;
        session.dirty = initial_dirty;
        Ok(session)
    }

    /// Construct an adapter for an already registered/shared handle.
    pub(super) fn from_handle(handle: DocumentHandle) -> Result<Self, EditorDocumentSessionError> {
        let lease = handle.lease();
        Self::from_handle_and_lease(handle, lease)
    }

    /// Construct a view using a persisted `DocumentViewInstanceId`.  Restore
    /// paths use this to reconnect view-local selection/history without
    /// silently replacing an already-live view.
    pub(super) fn from_handle_with_view_id(
        handle: DocumentHandle,
        view_id: DocumentViewInstanceId,
    ) -> Result<Self, EditorDocumentSessionError> {
        let lease = handle.lease();
        Self::from_handle_and_lease_with_view_id(handle, lease, view_id)
    }

    /// Construct a new view from an existing lease. Consuming the lease keeps its
    /// registry count unchanged; explicit `fork_view` clones it for a new view.
    pub(super) fn from_lease(lease: DocumentLease) -> Result<Self, EditorDocumentSessionError> {
        let handle = lease.handle();
        Self::from_handle_and_lease(handle, lease)
    }

    /// Construct a view from a moved lease using a persisted view identity.
    pub(super) fn from_lease_with_view_id(
        lease: DocumentLease,
        view_id: DocumentViewInstanceId,
    ) -> Result<Self, EditorDocumentSessionError> {
        let handle = lease.handle();
        Self::from_handle_and_lease_with_view_id(handle, lease, view_id)
    }

    /// Construct a view from an already shared lease token without acquiring
    /// another registry lease. Pane canvases use this when the owning tab has
    /// already forked the view and hands the same `Arc` to the active shell.
    /// Registration remains strict: a duplicate or nil view id is rejected.
    pub(crate) fn from_lease_arc_with_view_id(
        lease: Arc<DocumentLease>,
        view_id: DocumentViewInstanceId,
    ) -> Result<Self, EditorDocumentSessionError> {
        let handle = lease.handle();
        Self::from_handle_and_lease_arc_with_view_id(handle, lease, view_id)
    }

    fn from_handle_and_lease(
        handle: DocumentHandle,
        lease: DocumentLease,
    ) -> Result<Self, EditorDocumentSessionError> {
        Self::from_handle_and_lease_with_view_id(handle, lease, DocumentViewInstanceId::new())
    }

    fn from_handle_and_lease_with_view_id(
        handle: DocumentHandle,
        lease: DocumentLease,
        view_id: DocumentViewInstanceId,
    ) -> Result<Self, EditorDocumentSessionError> {
        Self::from_handle_and_lease_arc_with_view_id(handle, Arc::new(lease), view_id)
    }

    fn from_handle_and_lease_arc_with_view_id(
        handle: DocumentHandle,
        lease: Arc<DocumentLease>,
        view_id: DocumentViewInstanceId,
    ) -> Result<Self, EditorDocumentSessionError> {
        if view_id.uuid().is_nil() {
            return Err(EditorDocumentSessionError::InvalidViewId);
        }
        let (_, events) = handle
            .subscribe_with_snapshot()
            .map_err(EditorDocumentSessionError::from)?;
        {
            let mut controller = handle.lock().map_err(EditorDocumentSessionError::from)?;
            if controller.view_selection(view_id).is_some() {
                return Err(EditorDocumentSessionError::ViewAlreadyRegistered(view_id));
            }
            controller.register_view(view_id);
        }
        let view = Arc::new(ViewLease {
            handle: handle.clone(),
            lease,
            view_id,
            events: std::sync::Mutex::new(events),
            pending: std::sync::Mutex::new(None),
        });
        Ok(Self {
            handle: Some(handle),
            view: Some(view),
        })
    }

    /// Explicitly open another view over the same Controller/document.
    pub(super) fn fork_view(&self) -> Result<Self, EditorDocumentSessionError> {
        let handle = self
            .handle
            .clone()
            .ok_or(EditorDocumentSessionError::Shell)?;
        let lease = self
            .view
            .as_ref()
            .ok_or(EditorDocumentSessionError::Shell)?
            .lease
            .as_ref()
            .clone();
        Self::from_handle_and_lease(handle, lease)
    }

    pub(super) fn handle(&self) -> Result<DocumentHandle, EditorDocumentSessionError> {
        self.handle.clone().ok_or(EditorDocumentSessionError::Shell)
    }

    fn shared_handle(&self) -> Result<&DocumentHandle, EditorDocumentSessionError> {
        self.handle
            .as_ref()
            .ok_or(EditorDocumentSessionError::Shell)
    }

    fn shared_view(&self) -> Result<&Arc<ViewLease>, EditorDocumentSessionError> {
        self.view.as_ref().ok_or(EditorDocumentSessionError::Shell)
    }

    pub(super) fn view_id(&self) -> DocumentViewInstanceId {
        self.view
            .as_ref()
            .map(|view| view.view_id)
            .unwrap_or_else(|| DocumentViewInstanceId::from_uuid(uuid::Uuid::nil()))
    }

    /// Share the view's existing lease with a pane reference without creating
    /// another registry lease. A pane canvas that needs an independent cursor
    /// or event subscription must call [`Self::fork_view`] first and then share
    /// the fork's token through this Arc.
    pub(super) fn lease_arc(&self) -> Option<Arc<DocumentLease>> {
        self.view.as_ref().map(|view| Arc::clone(&view.lease))
    }

    pub(super) fn document_id(&self) -> Result<DocumentId, EditorDocumentSessionError> {
        let handle = self.handle()?;
        let controller = handle.lock().map_err(EditorDocumentSessionError::from)?;
        Ok(controller.document_id())
    }

    pub(super) fn lease_count(&self) -> usize {
        self.handle.as_ref().map_or(0, DocumentHandle::lease_count)
    }

    /// Poll the shared Controller event cursor owned by this view.  Clones of
    /// one adapter share the cursor; `fork_view` creates a fresh cursor.
    pub(super) fn poll_events(&self) -> Result<DocumentEventPoll, EditorDocumentSessionError> {
        let view = self.shared_view()?;
        let mut subscription = view
            .events
            .lock()
            .map_err(|_| EditorDocumentSessionError::Controller(ControllerError::Poisoned))?;
        match subscription.poll() {
            Ok(events) => Ok(DocumentEventPoll {
                events,
                snapshot: None,
            }),
            Err(ControllerError::SubscriptionLagged { .. }) => {
                let (snapshot, replacement) = self
                    .shared_handle()?
                    .subscribe_with_snapshot()
                    .map_err(EditorDocumentSessionError::from)?;
                *subscription = replacement;
                Ok(DocumentEventPoll {
                    events: Vec::new(),
                    snapshot: Some(snapshot),
                })
            }
            Err(error) => Err(EditorDocumentSessionError::from(error)),
        }
    }

    /// Check the shared event cursor without advancing it.  The event pump
    /// uses this readiness probe so an idle shared view does not schedule a
    /// permanent repaint merely to discover that nothing changed.
    pub(super) fn has_pending_events(&self) -> Result<bool, EditorDocumentSessionError> {
        let view = self.shared_view()?;
        if view
            .pending
            .lock()
            .map_err(|_| EditorDocumentSessionError::Controller(ControllerError::Poisoned))?
            .is_some()
        {
            return Ok(true);
        }
        let subscription = self
            .shared_view()?
            .events
            .lock()
            .map_err(|_| EditorDocumentSessionError::Controller(ControllerError::Poisoned))?;
        subscription
            .has_pending()
            .map_err(EditorDocumentSessionError::from)
    }

    pub(super) fn queue_events(&self, poll: DocumentEventPoll) {
        if poll.is_empty() {
            return;
        }
        let Some(view) = self.view.as_ref() else {
            return;
        };
        let Ok(mut pending) = view.pending.lock() else {
            return;
        };
        if let Some(existing) = pending.as_mut() {
            existing.merge(poll);
        } else {
            *pending = Some(poll);
        }
    }

    pub(super) fn take_queued_events(&self) -> Option<DocumentEventPoll> {
        self.view.as_ref()?.pending.lock().ok()?.take()
    }

    pub(super) fn try_apply_transaction(
        &self,
        transaction: Transaction,
    ) -> Result<DocumentSnapshot, EditorDocumentSessionError> {
        let selection_before = self.try_source_selection()?;
        let runtime_transaction = runtime_transaction(&transaction);
        let selection_after = DocumentMutationMap::from_transaction(&runtime_transaction)
            .map_selection(selection_before);
        self.apply_transaction_with_selection(transaction, selection_before, selection_after)
    }

    pub(super) fn apply_transaction(
        &self,
        transaction: Transaction,
    ) -> Result<DocumentSnapshot, EditorDocumentSessionError> {
        self.try_apply_transaction(transaction)
    }

    pub(super) fn apply_transaction_with_selection(
        &self,
        transaction: Transaction,
        selection_before: SourceSelection,
        selection_after: SourceSelection,
    ) -> Result<DocumentSnapshot, EditorDocumentSessionError> {
        let transaction_id = self.next_transaction_id()?;
        self.dispatch(DocumentCommand::ApplyTransaction {
            view_id: self.view_id(),
            transaction_id,
            transaction: runtime_transaction(&transaction),
            selection_before,
            selection_after,
        })?;
        self.try_snapshot()
    }

    pub(super) fn undo(&self) -> Result<Option<DocumentSnapshot>, EditorDocumentSessionError> {
        let before = self.try_revision()?;
        let transaction_id = self.next_transaction_id()?;
        self.dispatch(DocumentCommand::Undo {
            view_id: self.view_id(),
            transaction_id,
        })?;
        let after = self.try_revision()?;
        if after == before {
            Ok(None)
        } else {
            self.try_snapshot().map(Some)
        }
    }

    pub(super) fn redo(&self) -> Result<Option<DocumentSnapshot>, EditorDocumentSessionError> {
        let before = self.try_revision()?;
        let transaction_id = self.next_transaction_id()?;
        self.dispatch(DocumentCommand::Redo {
            view_id: self.view_id(),
            transaction_id,
        })?;
        let after = self.try_revision()?;
        if after == before {
            Ok(None)
        } else {
            self.try_snapshot().map(Some)
        }
    }

    pub(super) fn normalize_line_endings(
        &self,
        ending: LineEnding,
    ) -> Result<Option<DocumentSnapshot>, EditorDocumentSessionError> {
        let before = self.try_revision()?;
        let selection = self.try_source_selection()?;
        let transaction_id = self.next_transaction_id()?;
        self.dispatch(DocumentCommand::NormalizeLineEndings {
            view_id: self.view_id(),
            transaction_id,
            ending,
            selection_before: selection,
            selection_after: selection,
        })?;
        let after = self.try_revision()?;
        if after == before {
            Ok(None)
        } else {
            self.try_snapshot().map(Some)
        }
    }

    pub(super) fn try_restore_source_format(
        &self,
        format: SourceFormatSnapshot,
    ) -> Result<bool, EditorDocumentSessionError> {
        let before = self.try_revision()?;
        let selection = self.try_source_selection()?;
        let transaction_id = self.next_transaction_id()?;
        self.dispatch(DocumentCommand::RestoreSourceFormat {
            view_id: self.view_id(),
            transaction_id,
            format,
            selection_before: selection,
            selection_after: selection,
        })?;
        Ok(self.try_revision()? != before)
    }

    pub(super) fn try_set_encoding(
        &self,
        encoding: TextEncoding,
    ) -> Result<bool, EditorDocumentSessionError> {
        let before = self.try_revision()?;
        let transaction_id = self.next_transaction_id()?;
        self.dispatch(DocumentCommand::SetEncoding {
            view_id: self.view_id(),
            transaction_id,
            encoding,
        })?;
        Ok(self.try_revision()? != before)
    }

    pub(super) fn try_revision(&self) -> Result<Revision, EditorDocumentSessionError> {
        let handle = self.handle()?;
        let controller = handle.lock().map_err(EditorDocumentSessionError::from)?;
        Ok(Revision::from_u64(controller.session().revision()))
    }

    pub(super) fn revision(&self) -> Revision {
        self.try_revision().unwrap_or(Revision::INITIAL)
    }

    pub(super) fn try_snapshot(&self) -> Result<DocumentSnapshot, EditorDocumentSessionError> {
        let handle = self.handle()?;
        let controller = handle.lock().map_err(EditorDocumentSessionError::from)?;
        controller
            .resident_snapshot()
            .ok_or(EditorDocumentSessionError::NotResident)
    }

    pub(super) fn snapshot(&self) -> DocumentSnapshot {
        self.try_snapshot()
            .unwrap_or_else(|_| SourceDocument::new("").snapshot())
    }

    pub(super) fn try_text(&self) -> Result<String, EditorDocumentSessionError> {
        self.try_snapshot().map(|snapshot| snapshot.text())
    }

    pub(super) fn text(&self) -> String {
        self.try_text().unwrap_or_default()
    }

    pub(super) fn try_len(&self) -> Result<usize, EditorDocumentSessionError> {
        let handle = self.handle()?;
        let controller = handle.lock().map_err(EditorDocumentSessionError::from)?;
        let len = controller.session().len();
        Ok(usize::try_from(len).unwrap_or(usize::MAX))
    }

    pub(super) fn len(&self) -> usize {
        self.try_len().unwrap_or_default()
    }

    pub(super) fn try_source_format(
        &self,
    ) -> Result<SourceFormatSnapshot, EditorDocumentSessionError> {
        let handle = self.handle()?;
        let controller = handle.lock().map_err(EditorDocumentSessionError::from)?;
        controller
            .source_format_snapshot()
            .ok_or(EditorDocumentSessionError::NotResident)
    }

    pub(super) fn source_format(&self) -> SourceFormatSnapshot {
        self.try_source_format()
            .unwrap_or_else(|_| SourceDocument::new("").source_format())
    }

    pub(super) fn try_source_format_summary(
        &self,
    ) -> Result<SourceFormatSummary, EditorDocumentSessionError> {
        let handle = self.handle()?;
        let controller = handle.lock().map_err(EditorDocumentSessionError::from)?;
        controller
            .session()
            .resident_source_document()
            .map(SourceDocument::source_format_summary)
            .ok_or(EditorDocumentSessionError::NotResident)
    }

    pub(super) fn source_format_summary(&self) -> SourceFormatSummary {
        self.try_source_format_summary()
            .unwrap_or_else(|_| SourceDocument::new("").source_format_summary())
    }

    pub(super) fn try_serialized_bytes(&self) -> Result<Vec<u8>, EditorDocumentSessionError> {
        let handle = self.handle()?;
        let controller = handle.lock().map_err(EditorDocumentSessionError::from)?;
        controller
            .session()
            .resident_source_document()
            .map(SourceDocument::serialized_bytes)
            .ok_or(EditorDocumentSessionError::NotResident)
    }

    pub(super) fn serialized_bytes(&self) -> Vec<u8> {
        self.try_serialized_bytes().unwrap_or_default()
    }

    pub(super) fn try_serialized_bytes_for_text(
        &self,
        text: &str,
    ) -> Result<Option<Vec<u8>>, EditorDocumentSessionError> {
        let handle = self.handle()?;
        let controller = handle.lock().map_err(EditorDocumentSessionError::from)?;
        controller
            .session()
            .resident_source_document()
            .map(|source| source.serialized_bytes_for_text(text))
            .ok_or(EditorDocumentSessionError::NotResident)
    }

    pub(super) fn serialized_bytes_for_text(&self, text: &str) -> Option<Vec<u8>> {
        self.try_serialized_bytes_for_text(text).ok().flatten()
    }

    pub(super) fn try_is_dirty(&self) -> Result<bool, EditorDocumentSessionError> {
        let handle = self.handle()?;
        let controller = handle.lock().map_err(EditorDocumentSessionError::from)?;
        Ok(controller.session().dirty)
    }

    pub(super) fn is_dirty(&self) -> bool {
        self.try_is_dirty().unwrap_or(false)
    }

    pub(super) fn try_resident_growth_reason(
        &self,
    ) -> Result<Option<OpenReason>, EditorDocumentSessionError> {
        let handle = self.handle()?;
        let controller = handle.lock().map_err(EditorDocumentSessionError::from)?;
        Ok(controller.session().resident_growth_reason())
    }

    pub(super) fn resident_growth_reason(&self) -> Option<OpenReason> {
        self.try_resident_growth_reason().unwrap_or(None)
    }

    pub(super) fn try_save_snapshot(
        &self,
    ) -> Result<DocumentSaveSnapshot, EditorDocumentSessionError> {
        let handle = self.handle()?;
        let controller = handle.lock().map_err(EditorDocumentSessionError::from)?;
        Ok(controller.save_snapshot())
    }

    /// Capture and enqueue one immutable save snapshot while holding the same
    /// Controller lock, so RequestSave cannot observe a different revision.
    pub(super) fn try_request_save_snapshot(
        &self,
    ) -> Result<Option<DocumentSaveSnapshot>, EditorDocumentSessionError> {
        self.try_request_save_context()
            .map(|context| context.map(|(snapshot, _)| snapshot))
    }

    pub(super) fn try_request_save_context(
        &self,
    ) -> Result<Option<(DocumentSaveSnapshot, SourceFormatSnapshot)>, EditorDocumentSessionError>
    {
        let mut controller = self
            .shared_handle()?
            .lock()
            .map_err(EditorDocumentSessionError::from)?;
        let source_format = controller
            .source_format_snapshot()
            .ok_or(EditorDocumentSessionError::NotResident)?;
        let Some(snapshot) = controller
            .request_save_snapshot()
            .map_err(EditorDocumentSessionError::from)?
        else {
            return Ok(None);
        };
        Ok(Some((snapshot, source_format)))
    }

    pub(super) fn try_save_succeeded(
        &self,
        revision: Revision,
        identity: FileIdentity,
    ) -> Result<Option<DocumentSaveSnapshot>, EditorDocumentSessionError> {
        self.shared_handle()?
            .complete_save(DocumentRevision(revision.get()), identity)
            .map_err(EditorDocumentSessionError::from)
    }

    pub(super) fn try_save_failed(
        &self,
        revision: Revision,
        code: SaveFailureCode,
    ) -> Result<Option<DocumentSaveSnapshot>, EditorDocumentSessionError> {
        self.shared_handle()?
            .fail_save(DocumentRevision(revision.get()), code)
            .map_err(EditorDocumentSessionError::from)
    }

    /// Acknowledge the current body as discarded through the shared
    /// Controller.  The runtime enforces the final-lease gate and publishes
    /// the resulting DirtyChanged event; callers must not clear local UI state
    /// when this returns an error.
    pub(super) fn try_discard_changes(&self) -> Result<bool, EditorDocumentSessionError> {
        self.shared_handle()?
            .discard_current_changes()
            .map_err(EditorDocumentSessionError::from)
    }

    #[cfg(test)]
    pub(super) fn set_dirty_for_test(&self, dirty: bool) {
        if dirty {
            // Test callers need to exercise the same authoritative dirty path as a
            // real edit without changing the document body.  An empty insertion is
            // still a controller transaction (and therefore advances revision,
            // history, and dirty state) while leaving the resident text unchanged.
            if self.try_is_dirty().unwrap_or(false) {
                return;
            }
            let Ok(revision) = self.try_revision() else {
                return;
            };
            let Ok(selection) = self.try_source_selection() else {
                return;
            };
            let transaction =
                Transaction::new(revision, vec![gmark_document::TextEdit::new(0..0, "")]);
            let _ = self.apply_transaction_with_selection(transaction, selection, selection);
        } else {
            // Discard is deliberately routed through the shared Controller.  The
            // runtime enforces the final-lease gate; a test must not fabricate a
            // clean UI mirror when another view still owns the document.
            let _ = self.try_discard_changes();
        }
    }

    pub(super) fn try_sync_source_selection(
        &self,
        selection: SourceSelection,
    ) -> Result<(), EditorDocumentSessionError> {
        let mut controller = self
            .shared_handle()?
            .lock()
            .map_err(EditorDocumentSessionError::from)?;
        controller.set_view_selection(self.view_id(), selection);
        Ok(())
    }

    /// Resident 编辑器把选择写回共享 Controller；视图 clone 不会复制正文或选择真值。
    pub(super) fn sync_source_selection(&self, selection: SourceSelection) {
        let _ = self.try_sync_source_selection(selection);
    }

    pub(super) fn try_source_selection(
        &self,
    ) -> Result<SourceSelection, EditorDocumentSessionError> {
        let controller = self
            .shared_handle()?
            .lock()
            .map_err(EditorDocumentSessionError::from)?;
        Ok(controller
            .view_selection(self.view_id())
            .unwrap_or_default())
    }

    #[cfg(test)]
    pub(super) fn source_selection(&self) -> SourceSelection {
        self.try_source_selection().unwrap_or_default()
    }

    fn next_transaction_id(&self) -> Result<TransactionId, EditorDocumentSessionError> {
        self.shared_handle()?
            .next_transaction_id()
            .map_err(EditorDocumentSessionError::from)
    }

    fn dispatch(&self, command: DocumentCommand) -> Result<(), EditorDocumentSessionError> {
        self.shared_handle()?
            .lock()
            .map_err(EditorDocumentSessionError::from)?
            .dispatch(command)
            .map_err(EditorDocumentSessionError::from)
    }
}

fn runtime_transaction(transaction: &Transaction) -> RuntimeTransaction {
    RuntimeTransaction::new(
        DocumentRevision(transaction.base_revision().get()),
        transaction
            .edits()
            .iter()
            .map(|edit| {
                SourceEdit::new(
                    edit.range().start as u64..edit.range().end as u64,
                    edit.replacement().to_owned(),
                )
            })
            .collect(),
    )
}

impl From<SourceDocument> for EditorDocumentSession {
    fn from(source: SourceDocument) -> Self {
        Self::new(source)
    }
}

#[cfg(test)]
#[path = "../../tests/unit/editor/document_session_private.rs"]
mod tests;
