// @author kongweiguang

//! DocumentHost integration for the shared recovery mailbox.

use super::super::*;
use super::{RecoveryJob, RecoveryQueueError, RecoveryWorker};
use gmark_document_core::DocumentRevision;

impl DocumentHost {
    /// Start one shared journal setup for a Controller-backed document. The
    /// immutable snapshot is captured before spawning, while directory,
    /// source, and journal I/O stay on the background executor; a second pane
    /// only observes the in-flight generation and reuses its result.
    pub(crate) fn start_shared_recovery(&mut self, cx: &mut Context<Self>) {
        let Some(recovery_dirs) = runtime_recovery_dirs() else {
            return;
        };
        self.coordinator.recovery_enabled = true;
        let Some(document) = self.document.as_ref() else {
            return;
        };
        let recovery = document.recovery_state();
        let Some(generation) = recovery.begin_setup() else {
            // A concurrent constructor may already be preparing or owning the
            // journal. Starting here is harmless for either state.
            self.start_recovery_worker(cx);
            return;
        };
        let snapshot = match document.save_snapshot() {
            Ok(snapshot) => snapshot,
            Err(error) => {
                recovery.fail_setup(generation, error.to_string());
                self.coordinator.recovery_error = Some(error.to_string().into());
                return;
            }
        };
        let recovery_dir = recovery_dirs.recovery_dir();
        let path = self.path.clone();
        let document_epoch = self.document_epoch;
        let host_generation = self.coordinator.recovery_generation;
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    recovery_dirs
                        .ensure_state_parent(&recovery_dir.join(".gmark-recovery-root"))
                        .map_err(|error| PagedDocumentError::Recovery(error.to_string()))?;
                    let source = FileSource::open(&path)
                        .map_err(|error| PagedDocumentError::Recovery(error.to_string()))?;
                    DocumentRecoveryJournal::create_from_snapshot(&recovery_dir, &source, &snapshot)
                })
                .await;
            match result {
                Ok(journal) => {
                    if !recovery.install_journal(generation, journal) {
                        return;
                    }
                    // Edits may advance revision while setup runs; the bounded
                    // pending handoff deliberately carries those newer edits.
                    let _ = this.update(cx, |view, cx| {
                        if view.document_epoch != document_epoch
                            || view.coordinator.recovery_generation != host_generation
                        {
                            return;
                        }
                        view.start_recovery_worker(cx);
                        cx.notify();
                    });
                }
                Err(error) => {
                    recovery.fail_setup(generation, error.to_string());
                    let _ = this.update(cx, |view, cx| {
                        if view.document_epoch != document_epoch
                            || view.coordinator.recovery_generation != host_generation
                        {
                            return;
                        }
                        view.coordinator.recovery_error = Some(error.to_string().into());
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    /// Transfer journal ownership into the ordered worker before any edit can
    /// enqueue a record, preventing a UI callback from retaining mutable I/O.
    pub(crate) fn install_recovery_journal(
        &mut self,
        journal: DocumentRecoveryJournal,
        cx: &mut Context<Self>,
    ) {
        self.coordinator.recovery_enabled = true;
        if let Some(document) = self.document.as_ref() {
            let recovery = document.recovery_state();
            if !recovery.install_direct_journal(journal) {
                recovery.set_error("recovery journal was already installed for this document");
            }
        } else {
            self.coordinator.recovery_journal = Some(journal);
        }
        self.start_recovery_worker(cx);
    }

    /// Lazily create the worker when a journal becomes available, so normal
    /// hosts without recovery configured keep the existing lightweight path.
    pub(crate) fn start_recovery_worker(&mut self, cx: &mut Context<Self>) {
        let Some(document) = self.document.as_ref() else {
            return;
        };
        let recovery = document.recovery_state();
        if let Some(journal) = self.coordinator.recovery_journal.take()
            && !recovery.install_direct_journal(journal)
        {
            recovery.set_error("recovery journal handoff was rejected");
        }
        if self.coordinator.recovery_worker.is_some() && recovery.has_worker() {
            self.handoff_pending_recovery();
            return;
        }
        let old_worker = self.coordinator.recovery_worker.take();
        drop(old_worker);
        let task_stamp = DocumentTaskStamp::capture(self, self.coordinator.recovery_generation);
        self.coordinator.recovery_worker = RecoveryWorker::start(recovery.clone(), task_stamp, cx);
        if self.coordinator.recovery_worker.is_none()
            && let Some(error) = recovery.error()
        {
            self.coordinator.recovery_error = Some(error.into());
        }
        self.handoff_pending_recovery();
    }

    /// Hand off the newest mirrored Resident command and consume the older
    /// copy, preventing one first edit from being replayed twice.
    fn handoff_pending_recovery(&mut self) {
        let local = self.coordinator.take_pending_recovery();
        let shared = self
            .document
            .as_ref()
            .and_then(|document| document.recovery_state().take_pending());
        let pending = match (local, shared) {
            (Some(local), Some(shared)) if local.snapshot.revision >= shared.snapshot.revision => {
                Some(local)
            }
            (Some(_local), Some(shared)) => Some(shared),
            (Some(local), None) => Some(local),
            (None, Some(shared)) => Some(shared),
            (None, None) => None,
        };
        let Some(pending) = pending else {
            return;
        };
        let revision = pending.snapshot.revision;
        let snapshot = pending.snapshot;
        let record = pending.record;
        let job = RecoveryJob::Record {
            revision,
            snapshot: snapshot.clone(),
            record: record.clone(),
        };
        let result = match self.coordinator.recovery_worker.as_mut() {
            Some(worker) => worker.enqueue(job),
            None => Err(RecoveryQueueError::Disconnected),
        };
        if let Err(error) = result {
            self.coordinator.recovery_error = Some(
                format!(
                    "recovery journal handoff for revision {} failed: {error}",
                    revision.0
                )
                .into(),
            );
            let pending = super::super::coordinator::PendingRecoveryRecord { snapshot, record };
            if let Some(document) = self.document.as_ref() {
                let _ = document.recovery_state().stage_pending(pending);
            } else {
                self.coordinator.restore_pending_recovery(pending);
            }
        }
    }

    /// Capture the post-command immutable snapshot before submitting recovery;
    /// journal I/O is reachable only after the worker receives this job.
    pub(crate) fn enqueue_recovery_record(
        &mut self,
        document: &SharedDocument,
        record: RecoveryRecord,
        cx: &mut Context<Self>,
    ) {
        self.start_recovery_worker(cx);
        if self.coordinator.recovery_worker.is_none() && !self.coordinator.recovery_enabled {
            return;
        }
        let snapshot = match document.save_snapshot() {
            Ok(snapshot) => snapshot,
            Err(error) => {
                self.coordinator.recovery_error = Some(error.to_string().into());
                return;
            }
        };
        let Some(worker) = self.coordinator.recovery_worker.as_mut() else {
            if self.coordinator.recovery_enabled {
                let shared_snapshot = snapshot.clone();
                let shared_record = record.clone();
                if self.coordinator.stage_pending_recovery(snapshot, record) {
                    if let Some(shared) = self.document.as_ref().map(SharedDocument::recovery_state)
                    {
                        let _ = shared.stage_pending(
                            super::super::coordinator::PendingRecoveryRecord {
                                snapshot: shared_snapshot,
                                record: shared_record,
                            },
                        );
                    }
                    if self.coordinator.recovery_error.is_none() {
                        self.coordinator.recovery_error = Some(
                            "recovery journal is not ready; latest resident change is retained in memory"
                                .into(),
                        );
                    }
                } else if self.coordinator.recovery_error.is_none() {
                    self.coordinator.recovery_error = Some(
                        "recovery journal is not ready; paged recovery command was not persisted"
                            .into(),
                    );
                }
            }
            return;
        };
        if let Err(error) = worker.enqueue(RecoveryJob::Record {
            revision: snapshot.revision,
            snapshot,
            record,
        }) {
            self.coordinator.recovery_error = Some(error.to_string().into());
        }
    }

    /// Serialize checkpoint with all earlier edits; replacement is admitted
    /// only through the worker barrier so later records cannot cross backends.
    pub(crate) fn enqueue_recovery_checkpoint(
        &mut self,
        document: &SharedDocument,
        replacement: Option<DocumentRecoveryJournal>,
        cx: &mut Context<Self>,
    ) {
        self.start_recovery_worker(cx);
        let snapshot = match document.save_snapshot() {
            Ok(snapshot) => snapshot,
            Err(error) => {
                self.coordinator.recovery_error = Some(error.to_string().into());
                return;
            }
        };
        self.enqueue_recovery_checkpoint_snapshot(snapshot, replacement, cx);
    }

    /// Enqueue the exact persisted snapshot; recapturing live state here could
    /// incorrectly clear recovery for edits made during the save.
    pub(crate) fn enqueue_recovery_checkpoint_snapshot(
        &mut self,
        snapshot: DocumentSaveSnapshot,
        replacement: Option<DocumentRecoveryJournal>,
        cx: &mut Context<Self>,
    ) {
        self.start_recovery_worker(cx);
        let Some(worker) = self.coordinator.recovery_worker.as_mut() else {
            if self.coordinator.recovery_enabled && self.coordinator.recovery_error.is_none() {
                self.coordinator.recovery_error =
                    Some("recovery journal is not ready; checkpoint was not persisted".into());
            }
            self.coordinator
                .clear_pending_recovery_through(snapshot.revision);
            if let Some(replacement) = replacement {
                if let Some(document) = self.document.as_ref() {
                    let recovery = document.recovery_state();
                    if !recovery.install_direct_journal(replacement) {
                        recovery.set_error("recovery checkpoint replacement was rejected");
                    }
                } else {
                    self.coordinator.recovery_journal = Some(replacement);
                }
            }
            return;
        };
        if let Err(error) = worker.enqueue(RecoveryJob::Checkpoint {
            revision: snapshot.revision,
            snapshot,
            replacement,
        }) {
            self.coordinator.recovery_error = Some(error.to_string().into());
        }
    }

    /// Convert one source mutation into the common recovery command so Graph,
    /// delimited, formatting, paste, and delete share ordering rules.
    pub(crate) fn enqueue_recovery_transaction(
        &mut self,
        document: &SharedDocument,
        base_revision: u64,
        range: std::ops::Range<u64>,
        replacement: &str,
        selection: Option<SourceSelection>,
        view_id: DocumentViewId,
        cx: &mut Context<Self>,
    ) {
        self.enqueue_recovery_record(
            document,
            RecoveryRecord {
                action: RecoveryAction::Transaction(Transaction::new(
                    DocumentRevision(base_revision),
                    vec![SourceEdit::new(range, replacement)],
                )),
                selection,
                view_id,
            },
            cx,
        );
    }

    /// Queue history actions because Paged replay must retain every ordered
    /// undo/redo transition rather than reconstructing only the latest text.
    pub(crate) fn enqueue_recovery_action(
        &mut self,
        document: &SharedDocument,
        action: RecoveryAction,
        selection: Option<SourceSelection>,
        view_id: DocumentViewId,
        cx: &mut Context<Self>,
    ) {
        self.enqueue_recovery_record(
            document,
            RecoveryRecord {
                action,
                selection,
                view_id,
            },
            cx,
        );
    }
}
