// @author kongweiguang

//! Lock-backed document access used by every `DocumentHost` view.
//!
//! A host owns a view instance and a lease, but never owns a second
//! `DocumentSession`.  The Controller inside `DocumentHandle` is the sole
//! authority for body bytes, revision, dirty state, and history.  Cloned
//! values intentionally retain only the handle so background work cannot
//! accidentally extend the registry lifetime.

use std::ops::Range;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, MutexGuard};

use gmark_document_core::{
    DocumentBackendKind, DocumentRevision, DocumentSnapshot, SourceSelection, TextEncoding,
    Transaction, TypingGroupId,
};
use gmark_document_runtime::{
    ControllerError, DocumentCommand, DocumentHandle, DocumentLease, DocumentSaveSnapshot,
    DocumentSession, DocumentViewInstanceId, FileIdentity, SaveFailureCode, TransactionId,
};
use gmark_paged_document::{
    ExternalChange, FileSource, LineIndex, PagedDocumentError, SearchCancellation, SearchMatch,
    SearchOptions, ViewportRequest, ViewportSnapshot,
};

use super::recovery_worker::SharedRecoveryState;

/// Shared body plus one explicit view lifetime token.
pub(crate) struct SharedDocument {
    handle: DocumentHandle,
    lease: Option<DocumentLease>,
    view_id: DocumentViewInstanceId,
    /// Registration follows the view lifetime across host/background clones.
    /// Keeping it shared prevents a stale worker clone from recreating a
    /// closed controller view after suspension.
    registered: Arc<AtomicBool>,
    /// Recovery is anchored to the shared Controller handle so detached panes
    /// reuse one mailbox instead of starting view-local journal writers.
    recovery: Arc<SharedRecoveryState>,
}

impl Clone for SharedDocument {
    fn clone(&self) -> Self {
        Self {
            handle: self.handle.clone(),
            // A cloned access is a read/write capability for an existing view,
            // never another registry lease.  The owning host keeps the token.
            lease: None,
            view_id: self.view_id,
            registered: Arc::clone(&self.registered),
            recovery: Arc::clone(&self.recovery),
        }
    }
}

impl SharedDocument {
    pub(crate) fn from_handle(
        handle: DocumentHandle,
        lease: DocumentLease,
        view_id: DocumentViewInstanceId,
    ) -> Result<Self, ControllerError> {
        {
            let mut controller = handle.lock()?;
            controller.register_view(view_id);
        }
        let recovery = shared_recovery_state(&handle)?;
        Ok(Self {
            handle,
            lease: Some(lease),
            view_id,
            registered: Arc::new(AtomicBool::new(true)),
            recovery,
        })
    }

    /// Restore a persisted pane identity without silently aliasing an already
    /// registered Controller view.  Persisted nil UUIDs are invalid and must
    /// fail closed rather than falling back to a random identity.
    pub(crate) fn from_handle_with_view_id(
        handle: DocumentHandle,
        lease: DocumentLease,
        view_id: DocumentViewInstanceId,
    ) -> Result<Self, ControllerError> {
        if view_id.uuid().is_nil() {
            return Err(ControllerError::Mutation(
                "persisted document view id must not be nil".into(),
            ));
        }
        {
            let mut controller = handle.lock()?;
            if controller.view_selection(view_id).is_some() {
                return Err(ControllerError::Mutation(format!(
                    "persisted document view id is already registered: {}",
                    view_id.uuid()
                )));
            }
            controller.register_view(view_id);
        }
        let recovery = shared_recovery_state(&handle)?;
        Ok(Self {
            handle,
            lease: Some(lease),
            view_id,
            registered: Arc::new(AtomicBool::new(true)),
            recovery,
        })
    }

    pub(crate) fn from_controller(
        controller: gmark_document_runtime::DocumentController,
    ) -> Result<Self, ControllerError> {
        let handle = DocumentHandle::new(controller);
        let lease = handle.lease();
        Self::from_handle(handle, lease, DocumentViewInstanceId::new())
    }

    pub(crate) fn handle(&self) -> DocumentHandle {
        self.handle.clone()
    }

    /// Return the Controller-lifetime recovery state used by every active and
    /// detached view; the caller only receives an Arc and never owns journal IO.
    pub(crate) fn recovery_state(&self) -> Arc<SharedRecoveryState> {
        Arc::clone(&self.recovery)
    }

    pub(crate) fn view_id(&self) -> DocumentViewInstanceId {
        self.view_id
    }

    /// Move the only lease into an inactive-view snapshot. The Controller view
    /// is closed before the token leaves the live host, so stale view-scoped
    /// selection updates cannot outlive the active Entity.
    pub(crate) fn detach_parts(
        mut self,
    ) -> Option<(DocumentHandle, DocumentLease, DocumentViewInstanceId)> {
        let lease = self.lease.take()?;
        let handle = self.handle.clone();
        let view_id = self.view_id;
        if let Ok(mut controller) = handle.lock() {
            controller.close_view(view_id);
        }
        self.registered.store(false, Ordering::Release);
        Some((handle, lease, view_id))
    }

    pub(crate) fn register_view(&self) -> Result<(), ControllerError> {
        self.lock()?.register_view(self.view_id);
        self.registered.store(true, Ordering::Release);
        Ok(())
    }

    pub(crate) fn close_view(&self) -> Result<(), ControllerError> {
        self.lock()?.close_view(self.view_id);
        self.registered.store(false, Ordering::Release);
        Ok(())
    }

    pub(crate) fn is_view_registered(&self) -> bool {
        self.registered.load(Ordering::Acquire)
    }

    pub(crate) fn lease_count(&self) -> usize {
        self.handle.lease_count()
    }

    pub(crate) fn lock(
        &self,
    ) -> Result<MutexGuard<'_, gmark_document_runtime::DocumentController>, ControllerError> {
        self.handle.lock()
    }

    pub(crate) fn with_session<T>(
        &self,
        f: impl FnOnce(&DocumentSession) -> T,
    ) -> Result<T, ControllerError> {
        Ok(f(self.lock()?.session()))
    }

    pub(crate) fn document_id(
        &self,
    ) -> Result<gmark_document_runtime::DocumentId, ControllerError> {
        Ok(self.lock()?.document_id())
    }

    pub(crate) fn profile(&self) -> Option<gmark_document_core::DocumentProfile> {
        self.with_session(|session| session.profile.clone()).ok()
    }

    pub(crate) fn identity(&self) -> Result<FileIdentity, ControllerError> {
        self.with_session(|session| session.file_identity.clone())
    }

    pub(crate) fn backend_kind(&self) -> Option<DocumentBackendKind> {
        self.with_session(|session| session.store.kind()).ok()
    }

    pub(crate) fn resident_growth_reason(&self) -> Option<gmark_document_core::OpenReason> {
        self.with_session(DocumentSession::resident_growth_reason)
            .ok()
            .flatten()
    }

    pub(crate) fn revision(&self) -> u64 {
        self.with_session(DocumentSession::revision)
            .unwrap_or_default()
    }

    pub(crate) fn revision_doc(&self) -> DocumentRevision {
        DocumentRevision(self.revision())
    }

    pub(crate) fn dirty(&self) -> bool {
        self.with_session(|session| session.dirty).unwrap_or(false)
    }

    pub(crate) fn len(&self) -> u64 {
        self.with_session(DocumentSession::len).unwrap_or_default()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.with_session(DocumentSession::is_empty).unwrap_or(true)
    }

    pub(crate) fn is_pristine(&self) -> bool {
        self.with_session(DocumentSession::is_pristine)
            .unwrap_or(true)
    }

    pub(crate) fn line_count(&self) -> u64 {
        self.with_session(DocumentSession::line_count)
            .unwrap_or_default()
    }

    pub(crate) fn line_range(&self, line: u64) -> Option<Range<u64>> {
        self.with_session(|session| session.line_range(line))
            .ok()
            .flatten()
    }

    pub(crate) fn line_for_offset(&self, offset: u64) -> Option<u64> {
        self.with_session(|session| session.line_for_offset(offset))
            .ok()
            .flatten()
    }

    pub(crate) fn line_index(&self) -> Option<LineIndex> {
        self.with_session(DocumentSession::line_index)
            .ok()
            .flatten()
    }

    /// Return the Controller-owned UTF-8 source backing a paged session.  The
    /// shadow/encoding plan stays inside the runtime backend; a view only
    /// borrows a clone of the source handle for lock-free indexing.
    pub(crate) fn structured_source(&self) -> Result<Option<FileSource>, PagedDocumentError> {
        self.with_session(|session| session.structured_source())
            .map_err(controller_io_error)
            .and_then(|result| result)
    }

    pub(crate) fn read_range(&self, range: Range<u64>) -> Result<Vec<u8>, PagedDocumentError> {
        self.with_session(|session| session.read_range(range))
            .map_err(controller_io_error)
            .and_then(|result| result)
    }

    pub(crate) fn read_range_cancellable(
        &self,
        range: Range<u64>,
        cancellation: &SearchCancellation,
    ) -> Result<Vec<u8>, PagedDocumentError> {
        self.with_session(|session| session.read_range_cancellable(range, cancellation))
            .map_err(controller_io_error)
            .and_then(|result| result)
    }

    pub(crate) fn read_viewport(
        &self,
        request: &ViewportRequest,
    ) -> Result<ViewportSnapshot, PagedDocumentError> {
        self.with_session(|session| session.read_viewport(request))
            .map_err(controller_io_error)
            .and_then(|result| result)
    }

    pub(crate) fn read_viewport_cancellable(
        &self,
        request: &ViewportRequest,
        cancellation: &SearchCancellation,
    ) -> Result<ViewportSnapshot, PagedDocumentError> {
        self.with_session(|session| session.read_viewport_cancellable(request, cancellation))
            .map_err(controller_io_error)
            .and_then(|result| result)
    }

    pub(crate) fn write_to_cancellable(
        &self,
        output: impl std::io::Write,
        cancellation: &SearchCancellation,
    ) -> Result<(), PagedDocumentError> {
        self.with_session(|session| session.write_to_cancellable(output, cancellation))
            .map_err(controller_io_error)
            .and_then(|result| result)
    }

    pub(crate) fn serialized_bytes(&self) -> Result<Vec<u8>, PagedDocumentError> {
        self.with_session(DocumentSession::serialized_bytes)
            .map_err(controller_io_error)
            .and_then(|result| result)
    }

    pub(crate) fn save_range_atomic_cancellable(
        &self,
        range: Range<u64>,
        path: impl AsRef<std::path::Path>,
        cancellation: &SearchCancellation,
    ) -> Result<(), PagedDocumentError> {
        let bytes = self.read_range_cancellable(range, cancellation)?;
        let path = path.as_ref().to_path_buf();
        gmark_document::atomic_write(&path, &bytes).map_err(|error| PagedDocumentError::Persist {
            path,
            source: std::io::Error::other(error.to_string()),
        })
    }

    pub(crate) fn search(
        &self,
        query: &str,
        options: SearchOptions,
        cancellation: &SearchCancellation,
    ) -> Result<Vec<SearchMatch>, PagedDocumentError> {
        self.with_session(|session| session.search(query, options, cancellation))
            .map_err(controller_io_error)
            .and_then(|result| result)
    }

    pub(crate) fn external_change(&self) -> Result<ExternalChange, PagedDocumentError> {
        self.with_session(DocumentSession::external_change)
            .map_err(controller_io_error)
            .and_then(|result| result)
    }

    pub(crate) fn accept_external_append(
        &self,
        expected_revision: DocumentRevision,
        expected_identity: FileIdentity,
        source: FileSource,
        index: LineIndex,
        identity: FileIdentity,
    ) -> Result<(), ControllerError> {
        self.lock()?
            .dispatch(DocumentCommand::AcceptExternalAppend {
                expected_revision,
                expected_identity,
                source,
                index,
                identity,
            })
    }

    pub(crate) fn reload_prepared_document(
        &self,
        expected_revision: DocumentRevision,
        expected_identity: FileIdentity,
        prepared: DocumentSession,
    ) -> Result<(), ControllerError> {
        self.lock()?
            .dispatch(DocumentCommand::ReloadPreparedDocument {
                expected_revision,
                expected_identity,
                prepared,
            })
    }

    pub(crate) fn source_selection(&self) -> SourceSelection {
        self.lock()
            .ok()
            .and_then(|controller| controller.view_selection(self.view_id))
            .unwrap_or_default()
    }

    pub(crate) fn set_source_selection(
        &self,
        selection: SourceSelection,
    ) -> Result<(), ControllerError> {
        if !self.is_view_registered() {
            return Ok(());
        }
        self.lock()?.set_view_selection(self.view_id, selection);
        Ok(())
    }

    pub(crate) fn set_selection(
        &self,
        range: Range<u64>,
        reversed: bool,
    ) -> Result<(), ControllerError> {
        self.set_source_selection(SourceSelection::from_range(range, reversed))
    }

    pub(crate) fn next_transaction_id(&self) -> Result<TransactionId, ControllerError> {
        self.handle.next_transaction_id()
    }

    pub(crate) fn apply_transaction(
        &self,
        transaction_id: TransactionId,
        transaction: Transaction,
        selection_before: SourceSelection,
        selection_after: SourceSelection,
    ) -> Result<(), ControllerError> {
        self.lock()?.dispatch(DocumentCommand::ApplyTransaction {
            view_id: self.view_id,
            transaction_id,
            transaction,
            selection_before,
            selection_after,
        })
    }

    /// 仅转发明确标记的连续 Source 输入组；历史合并边界仍由 Controller 校验 revision 与组 ID。
    pub(crate) fn apply_typing_transaction(
        &self,
        transaction_id: TransactionId,
        transaction: Transaction,
        selection_before: SourceSelection,
        selection_after: SourceSelection,
        group_id: TypingGroupId,
    ) -> Result<(), ControllerError> {
        self.lock()?
            .dispatch(DocumentCommand::ApplyTypingTransaction {
                view_id: self.view_id,
                transaction_id,
                transaction,
                selection_before,
                selection_after,
                group_id,
            })
    }

    pub(crate) fn replace_range(
        &self,
        range: Range<u64>,
        replacement: impl Into<std::sync::Arc<str>>,
    ) -> Result<DocumentRevision, PagedDocumentError> {
        let before = self.source_selection();
        let replacement = replacement.into();
        let caret = range.start.saturating_add(replacement.len() as u64);
        let transaction_id = self.next_transaction_id().map_err(controller_io_error)?;
        let transaction = Transaction::new(
            DocumentRevision(self.revision()),
            vec![gmark_document_core::SourceEdit::new(range, replacement)],
        );
        let after = SourceSelection::collapsed(caret, gmark_document_core::SourceAffinity::After);
        self.apply_transaction(transaction_id, transaction, before, after)
            .map_err(controller_io_error)?;
        Ok(DocumentRevision(self.revision()))
    }

    pub(crate) fn set_encoding(
        &self,
        encoding: TextEncoding,
    ) -> Result<DocumentRevision, ControllerError> {
        let transaction_id = self.next_transaction_id()?;
        self.handle
            .set_encoding(self.view_id, transaction_id, encoding)
    }

    pub(crate) fn undo(&self) -> Result<(), ControllerError> {
        let transaction_id = self.next_transaction_id()?;
        self.lock()?.dispatch(DocumentCommand::Undo {
            view_id: self.view_id,
            transaction_id,
        })
    }

    pub(crate) fn undo_changed(&self) -> Result<bool, ControllerError> {
        let before = self.revision();
        self.undo()?;
        Ok(self.revision() != before)
    }

    pub(crate) fn redo(&self) -> Result<(), ControllerError> {
        let transaction_id = self.next_transaction_id()?;
        self.lock()?.dispatch(DocumentCommand::Redo {
            view_id: self.view_id,
            transaction_id,
        })
    }

    pub(crate) fn redo_changed(&self) -> Result<bool, ControllerError> {
        let before = self.revision();
        self.redo()?;
        Ok(self.revision() != before)
    }

    pub(crate) fn request_save_snapshot(
        &self,
    ) -> Result<Option<DocumentSaveSnapshot>, ControllerError> {
        self.handle.request_save_snapshot()
    }

    pub(crate) fn save_succeeded(
        &self,
        revision: DocumentRevision,
        identity: FileIdentity,
    ) -> Result<(), ControllerError> {
        self.handle.complete_save(revision, identity).map(|_| ())
    }

    pub(crate) fn save_failed(
        &self,
        revision: DocumentRevision,
        code: SaveFailureCode,
    ) -> Result<(), ControllerError> {
        self.handle.fail_save(revision, code).map(|_| ())
    }

    pub(crate) fn discard_current_changes(&self) -> Result<bool, ControllerError> {
        self.handle.discard_current_changes()
    }

    pub(crate) fn snapshot(&self) -> Result<std::sync::Arc<dyn DocumentSnapshot>, ControllerError> {
        Ok(self.lock()?.session().snapshot())
    }

    /// Capture the immutable post-command input used by background recovery.
    /// The lock is held only while cloning in-memory runtime state; journal
    /// serialization and filesystem writes happen after this method returns.
    pub(crate) fn save_snapshot(&self) -> Result<DocumentSaveSnapshot, ControllerError> {
        Ok(self.lock()?.save_snapshot())
    }
}

fn shared_recovery_state(
    handle: &DocumentHandle,
) -> Result<Arc<SharedRecoveryState>, ControllerError> {
    if let Some(recovery) = handle.shared_extension::<SharedRecoveryState>()? {
        return Ok(recovery);
    }
    // The typed extension is installed under DocumentHandle's own mutex, so
    // concurrent pane construction receives the same state and cannot create
    // duplicate workers for one Controller.
    handle.install_shared_extension(Arc::new(SharedRecoveryState::new()))
}

impl Drop for SharedDocument {
    fn drop(&mut self) {
        let Some(lease) = self.lease.take() else {
            return;
        };
        if let Ok(mut controller) = self.handle.lock() {
            controller.close_view(self.view_id);
        }
        self.registered.store(false, Ordering::Release);
        drop(lease);
    }
}

impl DocumentSnapshot for SharedDocument {
    fn revision(&self) -> DocumentRevision {
        self.revision_doc()
    }

    fn len(&self) -> u64 {
        SharedDocument::len(self)
    }

    fn read_range(&self, range: Range<u64>) -> Result<Vec<u8>, gmark_document_core::SnapshotError> {
        SharedDocument::read_range(self, range)
            .map_err(|error| gmark_document_core::SnapshotError::Read(error.to_string()))
    }
}

fn controller_io_error(error: ControllerError) -> PagedDocumentError {
    PagedDocumentError::InvalidTransaction(error.to_string())
}

#[cfg(test)]
#[path = "../../tests/unit/document_host_shared_private.rs"]
mod tests;
