// @author kongweiguang

//! Recovery mailbox execution and background journal draining.
//!
//! The state/queue contract stays in `worker.rs`; this module owns the GPUI
//! task and the background loop so the coordination surface remains small and
//! reviewable without changing the one-worker-per-Controller behavior.

use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures::channel::mpsc;
use futures::stream::StreamExt as _;
use gmark_document_core::{DocumentRevision, RecoveryRecord};
use gmark_document_runtime::DocumentSaveSnapshot;
use gpui::{Context, Task};

use super::super::coordinator::PendingRecoveryRecord;
use super::super::*;

const RECOVERY_QUEUE_CAPACITY: usize = 256;
const RECOVERY_QUEUE_BYTES: usize = 64 * 1024 * 1024;

/// One immutable command captured after a Controller transition.
///
/// `DocumentSaveSnapshot` is also used for Paged documents even though the
/// Paged journal only consumes the transaction. Capturing one common shape
/// prevents a backend branch from reading the live Controller while the
/// worker is writing recovery data.
pub(crate) enum RecoveryJob {
    Record {
        revision: DocumentRevision,
        snapshot: DocumentSaveSnapshot,
        record: RecoveryRecord,
    },
    Checkpoint {
        revision: DocumentRevision,
        snapshot: DocumentSaveSnapshot,
        replacement: Option<DocumentRecoveryJournal>,
    },
    Flush {
        ack: Arc<AtomicBool>,
    },
}

struct RecoveryWorkerFailure {
    journal: DocumentRecoveryJournal,
    job: RecoveryJob,
    error: String,
}

/// Shared document recovery state; GPUI Tasks stay outside so detached panes
/// cannot destroy the only queue or journal writer.
pub(crate) struct SharedRecoveryState {
    inner: Mutex<SharedRecoveryInner>,
    setup_generation: AtomicU64,
}

struct SharedRecoveryInner {
    journal: Option<DocumentRecoveryJournal>,
    worker: Option<RecoveryWorkerMailbox>,
    failure: Option<RecoveryWorkerFailure>,
    pending: Option<PendingRecoveryRecord>,
    setup_in_flight: bool,
    error: Option<String>,
    flush: Option<RecoveryFlushRequest>,
}

#[derive(Clone)]
struct RecoveryWorkerMailbox {
    sender: mpsc::Sender<RecoveryJob>,
    overflow: Arc<RecoveryOverflow>,
    queued_bytes: Arc<AtomicUsize>,
    paged: Arc<AtomicBool>,
}

struct RecoveryFlushRequest {
    ack: Arc<AtomicBool>,
    requested_at: Instant,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RecoveryFlushStatus {
    Idle,
    Pending,
    Completed,
    TimedOut,
    Failed,
}

enum WorkerStart {
    Active,
    Start {
        journal: Box<DocumentRecoveryJournal>,
        failed_job: Option<Box<RecoveryJob>>,
    },
    Unavailable,
}

const DEFAULT_RECOVERY_FLUSH_TIMEOUT: Duration = Duration::from_secs(10);

impl SharedRecoveryState {
    pub(crate) fn new() -> Self {
        Self {
            inner: Mutex::new(SharedRecoveryInner {
                journal: None,
                worker: None,
                failure: None,
                pending: None,
                setup_in_flight: false,
                error: None,
                flush: None,
            }),
            setup_generation: AtomicU64::new(0),
        }
    }

    /// Return one setup generation so concurrent shared views create at most
    /// one journal and late callbacks can be rejected by generation.
    pub(crate) fn begin_setup(&self) -> Option<u64> {
        let mut inner = self.inner.lock().ok()?;
        if inner.setup_in_flight
            || inner.journal.is_some()
            || inner.worker.is_some()
            || inner.failure.is_some()
        {
            return None;
        }
        inner.setup_in_flight = true;
        Some(self.setup_generation.fetch_add(1, Ordering::AcqRel) + 1)
    }

    pub(crate) fn install_journal(
        &self,
        generation: u64,
        journal: DocumentRecoveryJournal,
    ) -> bool {
        let mut inner = match self.inner.lock() {
            Ok(inner) => inner,
            Err(_) => return false,
        };
        if self.setup_generation.load(Ordering::Acquire) != generation
            || inner.journal.is_some()
            || inner.worker.is_some()
            || inner.failure.is_some()
        {
            return false;
        }
        inner.setup_in_flight = false;
        inner.journal = Some(journal);
        true
    }

    /// Install a journal already prepared by a caller that owns the session
    /// snapshot. The shared slot prevents one view replacing another writer.
    pub(crate) fn install_direct_journal(&self, journal: DocumentRecoveryJournal) -> bool {
        let Ok(mut inner) = self.inner.lock() else {
            return false;
        };
        if inner.journal.is_some() || inner.worker.is_some() || inner.failure.is_some() {
            return false;
        }
        inner.setup_in_flight = false;
        inner.journal = Some(journal);
        true
    }

    pub(crate) fn fail_setup(&self, generation: u64, error: impl Into<String>) {
        if self.setup_generation.load(Ordering::Acquire) != generation {
            return;
        }
        if let Ok(mut inner) = self.inner.lock() {
            inner.setup_in_flight = false;
            inner.error = Some(error.into());
        }
    }

    fn take_start(&self) -> WorkerStart {
        let Ok(mut inner) = self.inner.lock() else {
            return WorkerStart::Unavailable;
        };
        if inner.worker.is_some() {
            return WorkerStart::Active;
        }
        if let Some(failure) = inner.failure.take() {
            return WorkerStart::Start {
                journal: Box::new(failure.journal),
                failed_job: Some(Box::new(failure.job)),
            };
        }
        if let Some(journal) = inner.journal.take() {
            return WorkerStart::Start {
                journal: Box::new(journal),
                failed_job: None,
            };
        }
        WorkerStart::Unavailable
    }

    fn install_worker(&self, mailbox: RecoveryWorkerMailbox) -> bool {
        let Ok(mut inner) = self.inner.lock() else {
            return false;
        };
        if inner.worker.is_some() {
            return false;
        }
        inner.worker = Some(mailbox);
        true
    }

    fn mailbox(&self) -> Option<RecoveryWorkerMailbox> {
        self.inner
            .lock()
            .ok()
            .and_then(|inner| inner.worker.clone())
    }

    pub(crate) fn has_worker(&self) -> bool {
        self.inner
            .lock()
            .ok()
            .is_some_and(|inner| inner.worker.is_some())
    }

    fn mark_failure(&self, failure: RecoveryWorkerFailure) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.error = Some(failure.error.clone());
            inner.worker = None;
            inner.failure = Some(failure);
        }
    }

    pub(crate) fn stage_pending(&self, pending: PendingRecoveryRecord) -> bool {
        let Ok(mut inner) = self.inner.lock() else {
            return false;
        };
        if inner
            .pending
            .as_ref()
            .is_none_or(|previous| previous.snapshot.revision <= pending.snapshot.revision)
        {
            inner.pending = Some(pending);
        }
        true
    }

    pub(crate) fn take_pending(&self) -> Option<PendingRecoveryRecord> {
        self.inner.lock().ok()?.pending.take()
    }

    pub(crate) fn restore_pending(&self, pending: PendingRecoveryRecord) {
        let _ = self.stage_pending(pending);
    }

    pub(crate) fn error(&self) -> Option<String> {
        self.inner.lock().ok().and_then(|inner| inner.error.clone())
    }

    /// Preserve one failure so every pane can show the same degraded status
    /// without touching the Controller or waiting for a worker turn.
    pub(crate) fn set_error(&self, error: impl Into<String>) {
        if let Ok(mut inner) = self.inner.lock() {
            if inner.error.is_none() {
                inner.error = Some(error.into());
            }
        }
    }

    fn register_flush(&self) -> Option<Arc<AtomicBool>> {
        let ack = Arc::new(AtomicBool::new(false));
        let mut inner = self.inner.lock().ok()?;
        inner.flush = Some(RecoveryFlushRequest {
            ack: Arc::clone(&ack),
            requested_at: Instant::now(),
        });
        Some(ack)
    }

    pub(crate) fn flush_status(&self, timeout: Duration) -> RecoveryFlushStatus {
        let Ok(inner) = self.inner.lock() else {
            return RecoveryFlushStatus::Failed;
        };
        let Some(flush) = inner.flush.as_ref() else {
            return RecoveryFlushStatus::Idle;
        };
        if flush.ack.load(Ordering::Acquire) {
            return RecoveryFlushStatus::Completed;
        }
        if inner.error.is_some() {
            return RecoveryFlushStatus::Failed;
        }
        if flush.requested_at.elapsed() >= timeout {
            return RecoveryFlushStatus::TimedOut;
        }
        RecoveryFlushStatus::Pending
    }
}

impl RecoveryJob {
    /// Expose revision so Resident coalescing cannot replace a newer checkpoint.
    fn revision(&self) -> DocumentRevision {
        match self {
            Self::Record { revision, .. } | Self::Checkpoint { revision, .. } => *revision,
            Self::Flush { .. } => DocumentRevision(0),
        }
    }

    /// Bound memory accounting before enqueueing so a busy editor cannot grow
    /// recovery state independently of the document's bounded resident state.
    fn estimated_bytes(&self) -> usize {
        let (transaction_bytes, snapshot_bytes) = match self {
            Self::Record {
                record, snapshot, ..
            } => (
                match &record.action {
                    RecoveryAction::Transaction(transaction)
                    | RecoveryAction::TypingTransaction { transaction, .. } => transaction
                        .edits
                        .iter()
                        .map(|edit| edit.replacement.len())
                        .sum::<usize>(),
                    RecoveryAction::Undo | RecoveryAction::Redo => 0,
                },
                resident_snapshot_bytes(snapshot),
            ),
            Self::Checkpoint { snapshot, .. } => (0, resident_snapshot_bytes(snapshot)),
            Self::Flush { .. } => (0, 0),
        };
        transaction_bytes
            .saturating_add(snapshot_bytes)
            .saturating_add(256)
    }

    fn is_checkpoint(&self) -> bool {
        matches!(self, Self::Checkpoint { .. })
    }

    fn is_flush(&self) -> bool {
        matches!(self, Self::Flush { .. })
    }
}

/// Count Resident source material because its immutable snapshot retains the
/// full text; Paged snapshots are accounted by command payload instead.
fn resident_snapshot_bytes(snapshot: &DocumentSaveSnapshot) -> usize {
    snapshot
        .source_format
        .as_ref()
        .map(|_| usize::try_from(snapshot.len()).unwrap_or(usize::MAX))
        .unwrap_or(0)
}

struct RecoveryOverflow {
    pending: Mutex<RecoveryOverflowState>,
}

struct RecoveryOverflowState {
    latest_record: Option<(RecoveryJob, usize)>,
    checkpoint: Option<(RecoveryJob, usize)>,
}

impl RecoveryOverflow {
    /// Keep one replacement slot so Resident recovery progresses without an
    /// unbounded backlog of immutable snapshots.
    fn new() -> Self {
        Self {
            pending: Mutex::new(RecoveryOverflowState {
                latest_record: None,
                checkpoint: None,
            }),
        }
    }

    /// Take the lowest revision barrier first so a checkpoint cannot be hidden
    /// behind a newer Resident record.
    fn take(&self) -> Option<(RecoveryJob, usize)> {
        let mut pending = self.pending.lock().ok()?;
        match (&pending.latest_record, &pending.checkpoint) {
            (Some((record, _)), Some((checkpoint, _)))
                if record.revision() < checkpoint.revision() =>
            {
                pending.latest_record.take()
            }
            (_, Some(_)) => pending.checkpoint.take(),
            (Some(_), None) => pending.latest_record.take(),
            (None, None) => None,
        }
    }
}

#[derive(Debug)]
pub(crate) enum RecoveryQueueError {
    /// Paged journals cannot skip commands because undo/redo order is durable.
    PagedOverflow,
    Disconnected,
}

impl fmt::Display for RecoveryQueueError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PagedOverflow => formatter.write_str(
                "paged recovery queue exceeded its bounded capacity; recovery is degraded",
            ),
            Self::Disconnected => formatter.write_str("recovery worker is no longer available"),
        }
    }
}

/// A single ordered recovery worker for one host/document journal.
pub(crate) struct RecoveryWorker {
    shared: Arc<SharedRecoveryState>,
    task: Task<()>,
}

impl RecoveryWorker {
    /// Start or attach to one worker per shared Controller so command ordering
    /// is local to a document while filesystem calls stay off the UI executor.
    pub(crate) fn start(
        shared: Arc<SharedRecoveryState>,
        task_stamp: DocumentTaskStamp,
        cx: &mut Context<DocumentHost>,
    ) -> Option<Self> {
        let (journal, failed_job) = match shared.take_start() {
            WorkerStart::Active => {
                return Some(Self {
                    shared,
                    task: Task::ready(()),
                });
            }
            WorkerStart::Start {
                journal,
                failed_job,
            } => (*journal, failed_job.map(|job| *job)),
            WorkerStart::Unavailable => return None,
        };
        let paged = Arc::new(AtomicBool::new(journal.is_paged()));
        let (sender, receiver) = mpsc::channel(RECOVERY_QUEUE_CAPACITY);
        let overflow = Arc::new(RecoveryOverflow::new());
        let queued_bytes = Arc::new(AtomicUsize::new(0));
        let mailbox = RecoveryWorkerMailbox {
            sender: sender.clone(),
            overflow: Arc::clone(&overflow),
            queued_bytes: Arc::clone(&queued_bytes),
            paged: Arc::clone(&paged),
        };
        if !shared.install_worker(mailbox) {
            return Some(Self {
                shared,
                task: Task::ready(()),
            });
        }
        let worker_overflow = Arc::clone(&overflow);
        let worker_bytes = Arc::clone(&queued_bytes);
        let worker_paged = Arc::clone(&paged);
        let shared_weak = Arc::downgrade(&shared);
        let task = cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    run_recovery_worker(
                        journal,
                        receiver,
                        worker_overflow,
                        worker_bytes,
                        worker_paged,
                    )
                    .await
                })
                .await;
            if let Err(failure) = result {
                let error = failure.error.clone();
                if let Some(shared) = shared_weak.upgrade() {
                    shared.mark_failure(failure);
                }
                let _ = this.update(cx, |view, cx| {
                    if !task_stamp.accepts_strict(view, task_stamp.generation) {
                        return;
                    }
                    view.coordinator.recovery_error = Some(error.into());
                    cx.notify();
                });
            }
        });
        // Starting the task has no filesystem side effect; all I/O happens in
        // `run_recovery_worker` after the job has crossed the queue boundary.
        let mut worker = Self { shared, task };
        if let Some(failed_job) = failed_job {
            if let Err(error) = worker.enqueue(failed_job) {
                worker.shared.set_error(error.to_string());
            }
        }
        Some(worker)
    }

    /// 视图仅持共享队列能力；实际命令仍按同一有界策略入队。
    pub(crate) fn enqueue(&mut self, job: RecoveryJob) -> Result<(), RecoveryQueueError> {
        Self::enqueue_shared(&self.shared, job)
    }

    /// 保存完成可以晚于原视图关闭，仍须向原文档的共享队列投递准确快照而不能重捕获新正文。
    pub(crate) fn enqueue_shared(
        shared: &SharedRecoveryState,
        job: RecoveryJob,
    ) -> Result<(), RecoveryQueueError> {
        let bytes = job.estimated_bytes();
        let mut worker = shared.mailbox().ok_or(RecoveryQueueError::Disconnected)?;
        let queued = worker.queued_bytes.load(Ordering::Acquire);
        let paged = worker.paged.load(Ordering::Acquire);
        // A flush is an ordered acknowledgement, not a coalescible Resident
        // snapshot.  Dropping it would make close appear durable without an
        // actual worker barrier, so report a bounded queue failure instead.
        if job.is_flush() {
            worker.queued_bytes.fetch_add(bytes, Ordering::AcqRel);
            return match worker.sender.try_send(job) {
                Ok(()) => Ok(()),
                Err(error) => {
                    worker.queued_bytes.fetch_sub(bytes, Ordering::AcqRel);
                    if error.is_full() {
                        Err(RecoveryQueueError::PagedOverflow)
                    } else {
                        Err(RecoveryQueueError::Disconnected)
                    }
                }
            };
        }
        if paged && bytes > RECOVERY_QUEUE_BYTES.saturating_sub(queued) {
            return Err(RecoveryQueueError::PagedOverflow);
        }
        if !paged && bytes > RECOVERY_QUEUE_BYTES.saturating_sub(queued) {
            return Self::merge_latest(&worker, job, bytes);
        }

        worker.queued_bytes.fetch_add(bytes, Ordering::AcqRel);
        match worker.sender.try_send(job) {
            Ok(()) => Ok(()),
            Err(error) => {
                worker.queued_bytes.fetch_sub(bytes, Ordering::AcqRel);
                if paged {
                    Err(RecoveryQueueError::PagedOverflow)
                } else if error.is_full() {
                    Self::merge_latest(&worker, error.into_inner(), bytes)
                } else {
                    // A full Resident queue can safely collapse to its latest
                    // immutable state; the journal computes the next patch.
                    Err(RecoveryQueueError::Disconnected)
                }
            }
        }
    }

    /// Request an ordered flush without waiting on the UI thread.  Callers can
    /// poll the shared status and receive a visible timeout after the bound.
    pub(crate) fn request_flush(&mut self) -> Result<Arc<AtomicBool>, RecoveryQueueError> {
        let ack = self
            .shared
            .register_flush()
            .ok_or(RecoveryQueueError::Disconnected)?;
        if let Err(error) = self.enqueue(RecoveryJob::Flush {
            ack: Arc::clone(&ack),
        }) {
            self.shared.set_error(error.to_string());
            return Err(error);
        }
        Ok(ack)
    }

    /// Expose a bounded status for tests and close diagnostics without making
    /// Drop synchronously wait on slow or failed storage.
    pub(crate) fn flush_status(&self) -> RecoveryFlushStatus {
        self.shared.flush_status(DEFAULT_RECOVERY_FLUSH_TIMEOUT)
    }

    /// Replace the pending Resident snapshot while keeping byte accounting exact.
    fn merge_latest(
        worker: &RecoveryWorkerMailbox,
        job: RecoveryJob,
        bytes: usize,
    ) -> Result<(), RecoveryQueueError> {
        let Ok(mut pending) = worker.overflow.pending.lock() else {
            return Err(RecoveryQueueError::Disconnected);
        };
        let target = if job.is_checkpoint() {
            &mut pending.checkpoint
        } else {
            &mut pending.latest_record
        };
        if target
            .as_ref()
            .is_some_and(|(previous, _)| previous.revision() > job.revision())
        {
            return Ok(());
        }
        if let Some((_, previous_bytes)) = target.replace((job, bytes)) {
            worker
                .queued_bytes
                .fetch_sub(previous_bytes, Ordering::AcqRel);
        }
        worker.queued_bytes.fetch_add(bytes, Ordering::AcqRel);
        Ok(())
    }
}

impl Drop for RecoveryWorker {
    fn drop(&mut self) {
        // A close must publish a bounded barrier before the GPUI task is
        // detached.  We do not wait here: the shared state exposes Pending,
        // Completed, Failed, or TimedOut for the owner to query later.
        let _ = self.request_flush();
        // Detach the ordered worker so queued recovery survives host/entity
        // teardown; all journal writes remain on the background executor.
        let task = std::mem::replace(&mut self.task, Task::ready(()));
        task.detach();
    }
}

/// Drain ordered commands on a background executor; any error stops writes but
/// leaves the existing journal on disk for recovery instead of claiming success.
async fn run_recovery_worker(
    mut journal: DocumentRecoveryJournal,
    mut receiver: mpsc::Receiver<RecoveryJob>,
    overflow: Arc<RecoveryOverflow>,
    queued_bytes: Arc<AtomicUsize>,
    paged_mode: Arc<AtomicBool>,
) -> Result<(), RecoveryWorkerFailure> {
    let mut paged = journal.is_paged();
    let mut last_record_revision = None;
    loop {
        let next = if let Some(job) = overflow.take() {
            Some(job)
        } else {
            receiver.next().await.map(|job| {
                let bytes = job.estimated_bytes();
                (job, bytes)
            })
        };
        let Some((job, bytes)) = next else {
            return Ok(());
        };
        queued_bytes.fetch_sub(bytes, Ordering::AcqRel);
        match job {
            RecoveryJob::Record {
                revision,
                snapshot,
                record,
            } => {
                // Resident overflow keeps the newest snapshot and may be
                // observed before older channel entries.  Ignore those stale
                // entries after the newest revision has been persisted; Paged
                // journals must retain every ordered undo/redo command.
                if paged
                    || last_record_revision.is_none_or(|record_revision| revision > record_revision)
                {
                    if let Err(error) = journal.record_snapshot(&snapshot, &record) {
                        return Err(RecoveryWorkerFailure {
                            journal,
                            job: RecoveryJob::Record {
                                revision,
                                snapshot,
                                record,
                            },
                            error: format!("recovery revision {} failed: {error}", revision.0),
                        });
                    }
                    last_record_revision = Some(revision);
                }
            }
            RecoveryJob::Checkpoint {
                revision,
                snapshot,
                replacement,
            } => {
                // A save completion can arrive after a newer edit was already
                // queued.  Keeping that higher revision is required to avoid
                // falsely declaring the document durable at the older save.
                let checkpoint_is_current =
                    last_record_revision.is_none_or(|record_revision| revision >= record_revision);
                if let Some(next_journal) = replacement {
                    // A replacement is a backend barrier, not an optional
                    // payload.  Checkpoint the old backend only when the save
                    // revision has not been surpassed, then atomically switch
                    // the worker mode before later records are admitted.
                    if checkpoint_is_current {
                        if let Err(error) = journal.checkpoint_snapshot(&snapshot) {
                            return Err(RecoveryWorkerFailure {
                                journal,
                                job: RecoveryJob::Checkpoint {
                                    revision,
                                    snapshot,
                                    replacement: Some(next_journal),
                                },
                                error: format!("recovery checkpoint failed: {error}"),
                            });
                        }
                    }
                    paged = next_journal.is_paged();
                    paged_mode.store(paged, Ordering::Release);
                    journal = next_journal;
                    last_record_revision = Some(revision);
                } else if checkpoint_is_current {
                    if let Err(error) = journal.checkpoint_snapshot(&snapshot) {
                        return Err(RecoveryWorkerFailure {
                            journal,
                            job: RecoveryJob::Checkpoint {
                                revision,
                                snapshot,
                                replacement: None,
                            },
                            error: format!("recovery checkpoint failed: {error}"),
                        });
                    }
                    last_record_revision = Some(revision);
                }
            }
            RecoveryJob::Flush { ack } => {
                // The queue order already places this acknowledgement after
                // every earlier record/checkpoint, so setting it is the only
                // durable close barrier that can run without blocking GPUI.
                ack.store(true, Ordering::Release);
            }
        }
    }
}
