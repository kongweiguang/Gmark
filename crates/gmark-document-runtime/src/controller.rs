// @author kongweiguang

//! 共享文档 Controller 与 Registry 的公共契约和共享状态。
//!
//! 文件、Watcher、Recovery 等 adapter 由应用层提供；本模块只串行化文档命令、保存
//! 请求和代次校验。因此跨窗口可以共享正文、撤销和保存队列，而每个 DocumentTab 仍
//! 独占 selection、滚动和活动 ViewMode。具体实现按生命周期、命令、租约和 Registry
//! 拆到 [`controller_parts`]，避免单个入口文件承载无关的实现细节。

// Reason: typed identity/path errors preserve recovery details across lock and transition boundaries; remove when the public error contract is boxed without losing those details.
#![allow(clippy::result_large_err)]
// Reason: the callback registry keeps its Send/Sync trait object inline for allocation-free registration; remove when the registry is factored into named callback aliases.
#![allow(clippy::type_complexity)]

use std::any::{Any, TypeId};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, Weak};
use std::time::Duration;

use gmark_document::{LineEnding, SourceFormatSnapshot};
use gmark_document_core::{
    DocumentMutationMap, DocumentRevision, SourceEdit, SourceSelection, TextEncoding, Transaction,
    TypingGroupId,
};
use gmark_paged_document::{FileSource, LineIndex};
use thiserror::Error;
use uuid::Uuid;

use crate::{DocumentSaveSnapshot, DocumentSession, FileIdentity, SessionEditError};

/// 可写入恢复记录的文档身份。与文件路径解耦，Save As 不会重建正文身份。
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
pub struct DocumentId(Uuid);

impl DocumentId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub const fn from_uuid(value: Uuid) -> Self {
        Self(value)
    }

    pub const fn uuid(self) -> Uuid {
        self.0
    }
}

impl Default for DocumentId {
    fn default() -> Self {
        Self::new()
    }
}

pub use gmark_document_core::DocumentViewInstanceId;

/// 调用方生成的事务身份，用于把状态变化事件与原始命令对应起来。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TransactionId(pub u64);

/// Controller 的唯一窄写入口。UI/Provider 不直接拿后端实体或写磁盘。
#[derive(Clone)]
pub enum DocumentCommand {
    ApplyTransaction {
        view_id: DocumentViewInstanceId,
        transaction_id: TransactionId,
        transaction: Transaction,
        selection_before: SourceSelection,
        selection_after: SourceSelection,
    },
    ApplyTypingTransaction {
        view_id: DocumentViewInstanceId,
        transaction_id: TransactionId,
        transaction: Transaction,
        selection_before: SourceSelection,
        selection_after: SourceSelection,
        group_id: TypingGroupId,
    },
    NormalizeLineEndings {
        view_id: DocumentViewInstanceId,
        transaction_id: TransactionId,
        ending: LineEnding,
        selection_before: SourceSelection,
        selection_after: SourceSelection,
    },
    RestoreSourceFormat {
        view_id: DocumentViewInstanceId,
        transaction_id: TransactionId,
        format: SourceFormatSnapshot,
        selection_before: SourceSelection,
        selection_after: SourceSelection,
    },
    SetEncoding {
        view_id: DocumentViewInstanceId,
        transaction_id: TransactionId,
        encoding: TextEncoding,
    },
    /// Commit an IO-prepared append while validating the revision and file
    /// identity observed before the lock-free probe.
    AcceptExternalAppend {
        expected_revision: DocumentRevision,
        expected_identity: FileIdentity,
        source: FileSource,
        index: LineIndex,
        identity: FileIdentity,
    },
    /// Atomically install an IO-prepared session after validating the
    /// revision, identity, and dirty state observed before the load.
    ReloadPreparedDocument {
        expected_revision: DocumentRevision,
        expected_identity: FileIdentity,
        prepared: DocumentSession,
    },
    /// Reload after an explicit user decision to discard edits, while still
    /// rejecting any revision or file-identity change that happened during IO.
    ReloadPreparedDocumentAfterConfirmation {
        expected_revision: DocumentRevision,
        expected_identity: FileIdentity,
        prepared: DocumentSession,
    },
    Undo {
        view_id: DocumentViewInstanceId,
        transaction_id: TransactionId,
    },
    Redo {
        view_id: DocumentViewInstanceId,
        transaction_id: TransactionId,
    },
    /// Acknowledge the current shared body as discarded/persisted after the
    /// caller has made its close decision. The handle-level helper enforces
    /// that the caller owns the final lease before issuing this command.
    DiscardChanges {
        expected_revision: DocumentRevision,
    },
    /// 合并为“保存最新 revision”；实际 IO 由 Save adapter 在收到事件后执行。
    RequestSave,
    SaveSucceeded {
        revision: DocumentRevision,
        identity: FileIdentity,
    },
    SaveFailed {
        revision: DocumentRevision,
        code: SaveFailureCode,
    },
    ExternalConflict {
        identity: FileIdentity,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveFailureCode {
    Cancelled,
    Conflict,
    PermissionDenied,
    NoSpace,
    Uncertain,
    Other,
}

/// Controller 事件不含正文、路径或外部错误字符串，适合状态栏、恢复和安全诊断订阅。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DocumentEvent {
    RevisionChanged {
        sequence: u64,
        document_id: DocumentId,
        view_id: DocumentViewInstanceId,
        transaction_id: TransactionId,
        revision: DocumentRevision,
        dirty: bool,
        mutation: DocumentMutationMap,
        selection: SourceSelection,
    },
    DirtyChanged {
        sequence: u64,
        document_id: DocumentId,
        revision: DocumentRevision,
        dirty: bool,
    },
    Saved {
        sequence: u64,
        document_id: DocumentId,
        revision: DocumentRevision,
        dirty: bool,
        identity: FileIdentity,
    },
    IdentityChanged {
        sequence: u64,
        document_id: DocumentId,
        revision: DocumentRevision,
        identity: FileIdentity,
    },
    ExternalConflict {
        sequence: u64,
        document_id: DocumentId,
        revision: DocumentRevision,
        identity: FileIdentity,
    },
}

impl DocumentEvent {
    pub const fn sequence(&self) -> u64 {
        match self {
            Self::RevisionChanged { sequence, .. }
            | Self::DirtyChanged { sequence, .. }
            | Self::Saved { sequence, .. }
            | Self::IdentityChanged { sequence, .. }
            | Self::ExternalConflict { sequence, .. } => *sequence,
        }
    }
}

/// 写入器的串行队列。一次在途保存只允许有一个“最新”待保存 revision。
#[derive(Clone, Debug, Default)]
struct SaveQueue {
    in_flight: Option<DocumentSaveSnapshot>,
    pending: Option<DocumentSaveSnapshot>,
}

/// DocumentSession 的副作用协调器；自身不保存 selection、滚动或投影状态。
pub struct DocumentController {
    document_id: DocumentId,
    session: DocumentSession,
    save_queue: SaveQueue,
    events: VecDeque<DocumentEvent>,
    event_sequence: u64,
    views: BTreeMap<DocumentViewInstanceId, ViewRuntimeState>,
    undo_transactions: Vec<TransactionRuntimeRecord>,
    redo_transactions: Vec<TransactionRuntimeRecord>,
    active_typing_group: Option<ActiveTypingGroup>,
    next_transaction_id: u64,
}

const MAX_EVENT_LOG: usize = 4_096;

#[derive(Clone, Copy, Debug)]
struct ViewRuntimeState {
    selection: SourceSelection,
}

#[derive(Clone, Debug)]
struct TransactionRuntimeRecord {
    view_id: DocumentViewInstanceId,
    mutation: DocumentMutationMap,
    selection_before: SourceSelection,
    selection_after: SourceSelection,
    typing_group: Option<TypingGroupId>,
}

#[derive(Clone, Debug)]
struct ActiveTypingGroup {
    group_id: TypingGroupId,
    view_id: DocumentViewInstanceId,
    revision: DocumentRevision,
    selection_after: SourceSelection,
    mutation: DocumentMutationMap,
}

impl DocumentController {
    /// 清除 Controller 与后端的分组资格，确保旧 ID 不能在边界后重接历史。
    fn break_typing_group(&mut self) {
        self.active_typing_group = None;
        self.session.break_typing_group();
    }
}

/// 跨窗口共享的文档句柄。锁只覆盖 Controller 状态转换，慢速 IO 由 adapter 在锁外执行。
#[derive(Clone)]
pub struct DocumentHandle(Arc<DocumentHandleInner>);

#[derive(Clone, Default)]
pub struct WeakDocumentHandle(Weak<DocumentHandleInner>);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SaveStateNotification {
    pub in_flight_revision: Option<DocumentRevision>,
    pub pending_revision: Option<DocumentRevision>,
}

pub struct SaveStateCallbackRegistration {
    handle: Weak<DocumentHandleInner>,
    id: usize,
    active: AtomicBool,
}

struct DocumentHandleInner {
    controller: Mutex<DocumentController>,
    leases: AtomicUsize,
    /// Serializes lease acquisition with lifecycle decisions that validate
    /// the global count. Releases stay lock-free so dropping a lease while a
    /// controller guard is held cannot form a lock cycle.
    lease_gate: Mutex<()>,
    registry: Mutex<Option<(Weak<RegistryInner>, DocumentRegistryKey)>>,
    last_lease_callback: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
    save_state_callbacks: Mutex<BTreeMap<usize, Arc<dyn Fn(SaveStateNotification) + Send + Sync>>>,
    next_save_callback_id: AtomicUsize,
    /// Adapter state is anchored to the shared Controller rather than a view
    /// Entity, so document-scoped background work survives pane migration.
    shared_extensions: Mutex<HashMap<TypeId, Arc<dyn Any + Send + Sync>>>,
}

/// 明确表示一个打开视图对共享文档的生命周期租约。句柄 clone 不会延长租约；
/// 最后一个租约释放时 Registry 才允许移除条目。
pub struct DocumentLease {
    handle: DocumentHandle,
    released: AtomicBool,
}

/// 同一规范路径共享；硬链接的不同规范路径不共享，因为原子替换会断开其链接关系。
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum DocumentRegistryKey {
    Untitled(DocumentId),
    File(PathBuf),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegistryOpen {
    Existing,
    Inserted,
}

pub enum SaveAsReserveOutcome {
    Reserved(SaveAsReservation),
    Occupied {
        handle: DocumentHandle,
        lease: DocumentLease,
    },
}

/// 应用级 Registry；打开同一文件时返回同一 Handle，而 Tab 仍从自身 ViewState 开始。
pub struct DocumentRegistry {
    inner: Arc<RegistryInner>,
}

struct RegistryInner {
    documents: Mutex<BTreeMap<DocumentRegistryKey, Arc<RegistrySlot>>>,
}

struct RegistrySlot {
    state: Mutex<RegistrySlotState>,
    ready: Condvar,
}

enum RegistrySlotState {
    Opening,
    Ready(DocumentHandle),
    Reserved(DocumentHandle),
    Failed(ControllerError),
}

/// Save As 的目标 key 暂时占用。原子保存失败或 reservation 被丢弃时，目标
/// key 会自动释放；成功提交后它指向原共享 handle。
pub struct SaveAsReservation {
    registry: Weak<RegistryInner>,
    target: DocumentRegistryKey,
    source: DocumentHandle,
    committed: bool,
}

#[derive(Clone, Debug)]
pub struct DocumentStateSnapshot {
    pub document_id: DocumentId,
    pub revision: DocumentRevision,
    pub dirty: bool,
    pub identity: FileIdentity,
    pub sequence: u64,
    pub save_in_flight_revision: Option<DocumentRevision>,
    pub save_pending_revision: Option<DocumentRevision>,
    pub save: DocumentSaveSnapshot,
}

/// 订阅游标只会读取创建时序号之后的事件；Controller 的 event log 不在
/// 订阅者之间 drain，因此 snapshot 与首个事件在同一把 mutex 下无丢失窗口。
pub struct DocumentEventSubscription {
    handle: DocumentHandle,
    next_sequence: u64,
}

#[derive(Clone, Debug, Error)]
pub enum ControllerError {
    #[error(transparent)]
    Session(#[from] SessionEditError),
    #[error("document controller lock was poisoned")]
    Poisoned,
    #[error("document open failed: {0}")]
    OpenFailed(String),
    #[error("document opening timed out after {timeout_ms}ms: {key:?}")]
    OpenTimedOut {
        key: DocumentRegistryKey,
        timeout_ms: u64,
    },
    #[error(
        "document event subscription lagged: expected sequence {expected}, oldest retained {oldest}"
    )]
    SubscriptionLagged { expected: u64, oldest: u64 },
    #[error("document mutation map failed: {0}")]
    Mutation(String),
    #[error("document key is already occupied: {0:?}")]
    KeyOccupied(DocumentRegistryKey),
    #[error("document key is reserved by another save")]
    KeyReserved(DocumentRegistryKey),
    #[error("document is not registered")]
    DocumentNotRegistered,
    #[error("a last-lease callback is already registered")]
    LastLeaseCallbackRegistered,
    #[error("external transition expected revision {expected:?}, current revision is {actual:?}")]
    ExternalRevisionMismatch {
        expected: DocumentRevision,
        actual: DocumentRevision,
    },
    #[error("external transition expected identity {expected:?}, current identity is {actual:?}")]
    ExternalIdentityMismatch {
        expected: FileIdentity,
        actual: FileIdentity,
    },
    #[error("external transition requires a clean document")]
    DocumentDirty,
    #[error("discard requires the final document lease")]
    SharedDocumentStillLeased,
    #[error("save-as reservation is missing or already committed")]
    SaveAsReservationMissing,
    #[error("save completion for {actual:?} does not match in-flight save {expected:?}")]
    UnexpectedSaveCompletion {
        expected: Option<DocumentRevision>,
        actual: DocumentRevision,
    },
}

impl ControllerError {
    pub fn open_failed(message: impl Into<String>) -> Self {
        Self::OpenFailed(message.into())
    }

    pub fn source_document_error(&self) -> Option<&gmark_document::DocumentError> {
        match self {
            Self::Session(SessionEditError::ResidentDocument(error)) => Some(error),
            _ => None,
        }
    }
}

/// Replace the placeholder sequence supplied by mutation helpers only when an event enters the
/// bounded log, so every subscriber observes one monotonic source of truth.
fn with_sequence(event: DocumentEvent, sequence: u64) -> DocumentEvent {
    match event {
        DocumentEvent::RevisionChanged {
            document_id,
            view_id,
            transaction_id,
            revision,
            dirty,
            mutation,
            selection,
            ..
        } => DocumentEvent::RevisionChanged {
            sequence,
            document_id,
            view_id,
            transaction_id,
            revision,
            dirty,
            mutation,
            selection,
        },
        DocumentEvent::DirtyChanged {
            document_id,
            revision,
            dirty,
            ..
        } => DocumentEvent::DirtyChanged {
            sequence,
            document_id,
            revision,
            dirty,
        },
        DocumentEvent::Saved {
            document_id,
            revision,
            dirty,
            identity,
            ..
        } => DocumentEvent::Saved {
            sequence,
            document_id,
            revision,
            dirty,
            identity,
        },
        DocumentEvent::IdentityChanged {
            document_id,
            revision,
            identity,
            ..
        } => DocumentEvent::IdentityChanged {
            sequence,
            document_id,
            revision,
            identity,
        },
        DocumentEvent::ExternalConflict {
            document_id,
            revision,
            identity,
            ..
        } => DocumentEvent::ExternalConflict {
            sequence,
            document_id,
            revision,
            identity,
        },
    }
}

/// Build an inverse map from the immutable pre-edit snapshot so selections can be relocated
/// without retaining mutable backend state in the view layer.
fn build_mutation_map(
    transaction: &Transaction,
    snapshot: &dyn gmark_document_core::DocumentSnapshot,
) -> Result<DocumentMutationMap, ControllerError> {
    let mut inverse = Vec::with_capacity(transaction.edits.len());
    let mut delta = 0_i128;
    for edit in &transaction.edits {
        let removed = snapshot
            .read_range(edit.range.clone())
            .map_err(|error| ControllerError::Mutation(error.to_string()))?;
        let removed = String::from_utf8(removed)
            .map_err(|error| ControllerError::Mutation(error.to_string()))?;
        let start = if delta >= 0 {
            edit.range
                .start
                .checked_add(delta as u64)
                .ok_or_else(|| ControllerError::Mutation("selection coordinate overflow".into()))?
        } else {
            edit.range
                .start
                .checked_sub((-delta) as u64)
                .ok_or_else(|| ControllerError::Mutation("selection coordinate underflow".into()))?
        };
        let end = start
            .checked_add(edit.replacement.len() as u64)
            .ok_or_else(|| ControllerError::Mutation("selection coordinate overflow".into()))?;
        inverse.push(SourceEdit::new(start..end, removed));
        delta += edit.replacement.len() as i128 - (edit.range.end - edit.range.start) as i128;
    }
    Ok(DocumentMutationMap::with_inverse(
        &transaction.edits,
        &inverse,
    ))
}

/// Advance revisions through one checked helper so external transitions and edits share the same
/// overflow error instead of silently wrapping a persisted document generation.
fn next_revision(current: DocumentRevision) -> Result<DocumentRevision, ControllerError> {
    current
        .0
        .checked_add(1)
        .map(DocumentRevision)
        .ok_or_else(|| ControllerError::Mutation("document revision overflow".to_owned()))
}

#[path = "controller_parts/mod.rs"]
mod controller_parts;

#[cfg(test)]
#[path = "../tests/unit/controller.rs"]
mod tests;
