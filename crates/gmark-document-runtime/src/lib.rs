// @author kongweiguang

//! 文档会话运行时：统一 Resident Rope 与 Paged PieceDocument 的权威状态。

use std::io::{Read, Write};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use gmark_document::{DocumentError, LineEnding, SourceFormatSnapshot};
use gmark_document_core::{
    DocumentBackendKind, DocumentProfile, DocumentRevision, DocumentSnapshot, DocumentViewId,
    EditError, LoadingLimits, OpenError, OpenPlan, OpenReason, TextEncoding, Transaction,
};
use gmark_paged_document::{
    EncodedSavePlan, ExternalChange, FileSource, LineIndex, PagedDocument, PagedDocumentError,
    SearchCancellation, SearchMatch, SearchOptions, ViewportRequest, ViewportSnapshot,
};
use thiserror::Error;

mod controller;
mod recovery;
mod resident;
#[path = "lib_parts/session_io.rs"]
mod session_io;

pub use controller::{
    ControllerError, DocumentCommand, DocumentController, DocumentEvent, DocumentEventSubscription,
    DocumentHandle, DocumentId, DocumentLease, DocumentRegistry, DocumentRegistryKey,
    DocumentStateSnapshot, DocumentViewInstanceId, RegistryOpen, SaveAsReservation,
    SaveAsReserveOutcome, SaveFailureCode, SaveStateCallbackRegistration, SaveStateNotification,
    TransactionId, WeakDocumentHandle,
};
pub use recovery::{
    RecoveredResidentDocument, RecoveredResidentJournal, ResidentFileFingerprint,
    ResidentRecoveryError, ResidentRecoveryJournal, ResidentRecoveryReadStatus,
    ResidentRecoverySelection, cleanup_resident_recovery_artifacts, fingerprint_resident_file,
    load_resident_recovery_documents, load_resident_recovery_journals,
    replay_resident_recovery_journal, replay_resident_recovery_journal_with_metadata,
};
pub use resident::ResidentDocument;

/// 后台保存任务使用的不可变输入。快照与 revision 在创建时绑定；保存完成后
/// Controller 只有在当前 revision 仍相同的情况下才会提交 dirty 基线。
#[derive(Clone)]
pub struct DocumentSaveSnapshot {
    pub revision: DocumentRevision,
    pub identity: FileIdentity,
    pub encoding: TextEncoding,
    /// Resident source format captured with the same revision; `None` for a
    /// paged backend whose encoded bytes are already represented by its
    /// immutable snapshot.
    pub source_format: Option<SourceFormatSnapshot>,
    /// Paged backend plan retained by the shared document.  It keeps any
    /// UTF-8 shadow tempfile alive until the save worker has finished.
    pub paged_save_plan: Option<EncodedSavePlan>,
    pub snapshot: Arc<dyn DocumentSnapshot>,
    pub resident_baseline: Option<gmark_document::DocumentSnapshot>,
    written_paged_identity: Arc<Mutex<Option<gmark_paged_document::FileIdentity>>>,
}

impl std::fmt::Debug for DocumentSaveSnapshot {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DocumentSaveSnapshot")
            .field("revision", &self.revision)
            .field("identity", &self.identity)
            .field("encoding", &self.encoding)
            .field("source_format", &self.source_format)
            .field("paged_save_plan", &self.paged_save_plan)
            .field("len", &self.snapshot.len())
            .finish()
    }
}

impl DocumentSaveSnapshot {
    fn remember_paged_identity(&self, identity: gmark_paged_document::FileIdentity) {
        if let Ok(mut remembered) = self.written_paged_identity.lock() {
            *remembered = Some(identity);
        }
    }

    fn written_paged_identity(&self) -> Option<gmark_paged_document::FileIdentity> {
        self.written_paged_identity
            .lock()
            .ok()
            .and_then(|identity| identity.clone())
    }

    pub fn len(&self) -> u64 {
        self.snapshot.len()
    }

    /// Exposes emptiness alongside `len` so callers can avoid materializing a
    /// byte count when deciding whether a snapshot needs persistence.
    pub fn is_empty(&self) -> bool {
        self.snapshot.is_empty()
    }

    pub fn read_range(
        &self,
        range: Range<u64>,
    ) -> Result<Vec<u8>, gmark_document_core::SnapshotError> {
        self.snapshot.read_range(range)
    }

    pub fn read_all(&self) -> Result<Vec<u8>, gmark_document_core::SnapshotError> {
        self.read_range(0..self.len())
    }

    /// Stream this immutable snapshot to its original identity using an
    /// atomic replacement.  The write is lock-free with respect to the
    /// Controller and never consults a newer session revision.
    pub fn save_atomic_cancellable(
        &self,
        path: impl AsRef<Path>,
        cancellation: &SearchCancellation,
    ) -> Result<FileIdentity, PagedDocumentError> {
        let path = path.as_ref();
        if let Some(plan) = &self.paged_save_plan {
            let identity =
                plan.save_snapshot_atomic_cancellable(self.snapshot.as_ref(), path, cancellation)?;
            self.remember_paged_identity(identity.clone());
            return Ok(FileIdentity::from(&identity));
        }

        if cancellation.is_cancelled() {
            return Err(PagedDocumentError::Cancelled);
        }
        let expected = FileSource::open(path)?.identity()?;
        let expected_runtime = FileIdentity::from(&expected);
        if !runtime_identity_matches(&self.identity, &expected_runtime) {
            return Err(PagedDocumentError::SourceChanged);
        }
        let snapshot = Arc::clone(&self.snapshot);
        let encoding = self.encoding.clone();
        let format = self.source_format.clone();
        let identity = gmark_paged_document::atomic_write_stream(
            path,
            Some(&expected),
            cancellation,
            move |output, cancellation| {
                if let Some(format) = format.as_ref() {
                    write_resident_snapshot(
                        snapshot.as_ref(),
                        format,
                        &encoding,
                        output,
                        cancellation,
                    )
                } else {
                    write_raw_snapshot(snapshot.as_ref(), &encoding, output, cancellation)
                }
            },
        )?;
        if self.source_format.is_none() {
            self.remember_paged_identity(identity.clone());
        }
        Ok(FileIdentity::from(&identity))
    }

    /// Stream this immutable snapshot to a Save As target.  Save As does not
    /// validate the target's prior identity, but still stages and replaces it
    /// atomically and preserves cancellation semantics.
    pub fn save_as_atomic_cancellable(
        &self,
        path: impl AsRef<Path>,
        cancellation: &SearchCancellation,
    ) -> Result<FileIdentity, PagedDocumentError> {
        let path = path.as_ref();
        if let Some(plan) = &self.paged_save_plan {
            let identity = plan.save_snapshot_atomic_as_cancellable(
                self.snapshot.as_ref(),
                path,
                cancellation,
            )?;
            self.remember_paged_identity(identity.clone());
            return Ok(FileIdentity::from(&identity));
        }

        let snapshot = Arc::clone(&self.snapshot);
        let encoding = self.encoding.clone();
        let format = self.source_format.clone();
        let identity = gmark_paged_document::atomic_write_stream(
            path,
            None,
            cancellation,
            move |output, cancellation| {
                if let Some(format) = format.as_ref() {
                    write_resident_snapshot(
                        snapshot.as_ref(),
                        format,
                        &encoding,
                        output,
                        cancellation,
                    )
                } else {
                    write_raw_snapshot(snapshot.as_ref(), &encoding, output, cancellation)
                }
            },
        )?;
        if self.source_format.is_none() {
            self.remember_paged_identity(identity.clone());
        }
        Ok(FileIdentity::from(&identity))
    }
}

const SNAPSHOT_STREAM_BYTES: u64 = 8 * 1024 * 1024;

fn paged_identity_for_save(identity: &FileIdentity) -> gmark_paged_document::FileIdentity {
    gmark_paged_document::FileIdentity {
        path: identity.canonical_path.clone(),
        len: identity.len,
        modified_nanos: identity.modified_nanos,
        // Runtime identities deliberately do not depend on the paged crate's
        // platform-specific FileId representation.  The atomic helper treats
        // None as a path/metadata identity check.
        os_file_id: None,
    }
}

fn runtime_identity_matches(expected: &FileIdentity, current: &FileIdentity) -> bool {
    expected.canonical_path == current.canonical_path
        && expected.len == current.len
        && expected.modified_nanos == current.modified_nanos
        && expected
            .platform_id
            .as_ref()
            .is_none_or(|platform_id| current.platform_id.as_ref() == Some(platform_id))
}

fn write_raw_snapshot(
    snapshot: &dyn DocumentSnapshot,
    encoding: &TextEncoding,
    output: &mut dyn Write,
    cancellation: &SearchCancellation,
) -> Result<(), PagedDocumentError> {
    match encoding {
        TextEncoding::Utf8 { bom: true } => {
            output
                .write_all(&[0xef, 0xbb, 0xbf])
                .map_err(|source| PagedDocumentError::Io {
                    path: std::env::temp_dir(),
                    source,
                })?
        }
        TextEncoding::Utf8 { bom: false } => {}
        _ => {
            return Err(PagedDocumentError::InvalidTransaction(
                "a paged snapshot without an encoding plan must be UTF-8".to_owned(),
            ));
        }
    }
    let mut offset = 0_u64;
    while offset < snapshot.len() {
        if cancellation.is_cancelled() {
            return Err(PagedDocumentError::Cancelled);
        }
        let end = offset
            .saturating_add(SNAPSHOT_STREAM_BYTES)
            .min(snapshot.len());
        let bytes = snapshot
            .read_range(offset..end)
            .map_err(|error| PagedDocumentError::InvalidTransaction(error.to_string()))?;
        output
            .write_all(&bytes)
            .map_err(|source| PagedDocumentError::Io {
                path: std::env::temp_dir(),
                source,
            })?;
        offset = end;
    }
    Ok(())
}

fn write_resident_snapshot(
    snapshot: &dyn DocumentSnapshot,
    format: &SourceFormatSnapshot,
    encoding: &TextEncoding,
    output: &mut dyn Write,
    cancellation: &SearchCancellation,
) -> Result<(), PagedDocumentError> {
    match encoding {
        TextEncoding::Utf8 { bom } => {
            if *bom {
                output
                    .write_all(&[0xef, 0xbb, 0xbf])
                    .map_err(|source| PagedDocumentError::Io {
                        path: std::env::temp_dir(),
                        source,
                    })?;
            }
            let mut ending_index = 0usize;
            stream_snapshot_utf8(snapshot, cancellation, |text, _| {
                let transformed = restore_endings(text, format, &mut ending_index);
                output
                    .write_all(transformed.as_bytes())
                    .map_err(|source| PagedDocumentError::Io {
                        path: std::env::temp_dir(),
                        source,
                    })
            })?;
        }
        TextEncoding::Utf16Le | TextEncoding::Utf16Be => {
            let bom = if matches!(encoding, TextEncoding::Utf16Le) {
                [0xff, 0xfe]
            } else {
                [0xfe, 0xff]
            };
            output
                .write_all(&bom)
                .map_err(|source| PagedDocumentError::Io {
                    path: std::env::temp_dir(),
                    source,
                })?;
            let little_endian = matches!(encoding, TextEncoding::Utf16Le);
            let mut ending_index = 0usize;
            stream_snapshot_utf8(snapshot, cancellation, |text, _| {
                let transformed = restore_endings(text, format, &mut ending_index);
                write_utf16_snapshot(output, &transformed, little_endian)
            })?;
        }
        TextEncoding::Legacy(label) => {
            let encoding = encoding_rs::Encoding::for_label(label.as_bytes())
                .ok_or_else(|| PagedDocumentError::UnsupportedEncoding(label.clone()))?;
            let mut writer = SnapshotEncodingWriter {
                output,
                encoder: encoding.new_encoder(),
                encoding_name: label.clone(),
            };
            let mut ending_index = 0usize;
            stream_snapshot_utf8(snapshot, cancellation, |text, last| {
                let transformed = restore_endings(text, format, &mut ending_index);
                writer.encode(&transformed, last)
            })?;
        }
    }
    Ok(())
}

fn write_utf16_snapshot(
    output: &mut dyn Write,
    text: &str,
    little_endian: bool,
) -> Result<(), PagedDocumentError> {
    let mut bytes = Vec::with_capacity(text.len().saturating_mul(2));
    for unit in text.encode_utf16() {
        let encoded = if little_endian {
            unit.to_le_bytes()
        } else {
            unit.to_be_bytes()
        };
        bytes.extend_from_slice(&encoded);
    }
    output
        .write_all(&bytes)
        .map_err(|source| PagedDocumentError::Io {
            path: std::env::temp_dir(),
            source,
        })
}

fn restore_endings(text: &str, format: &SourceFormatSnapshot, ending_index: &mut usize) -> String {
    let bytes = text.as_bytes();
    let mut output = String::with_capacity(text.len());
    let mut segment_start = 0usize;
    for (offset, byte) in bytes.iter().copied().enumerate() {
        if byte != b'\n' {
            continue;
        }
        output.push_str(&text[segment_start..offset]);
        let ending = format
            .endings
            .get(*ending_index)
            .copied()
            .unwrap_or(format.dominant);
        output.push_str(match ending {
            LineEnding::Lf => "\n",
            LineEnding::CrLf => "\r\n",
            LineEnding::Cr => "\r",
        });
        *ending_index = ending_index.saturating_add(1);
        segment_start = offset + 1;
    }
    output.push_str(&text[segment_start..]);
    output
}

struct SnapshotEncodingWriter<'a> {
    output: &'a mut dyn Write,
    encoder: encoding_rs::Encoder,
    encoding_name: String,
}

impl SnapshotEncodingWriter<'_> {
    fn encode(&mut self, mut input: &str, last: bool) -> Result<(), PagedDocumentError> {
        let mut buffer = vec![0u8; 256 * 1024];
        loop {
            let (result, read, written, had_errors) =
                self.encoder.encode_from_utf8(input, &mut buffer, last);
            if had_errors {
                return Err(PagedDocumentError::UnrepresentableEncoding {
                    encoding: self.encoding_name.clone(),
                });
            }
            self.output
                .write_all(&buffer[..written])
                .map_err(|source| PagedDocumentError::Io {
                    path: std::env::temp_dir(),
                    source,
                })?;
            input = &input[read..];
            if matches!(result, encoding_rs::CoderResult::InputEmpty) {
                return Ok(());
            }
        }
    }
}

fn stream_snapshot_utf8(
    snapshot: &dyn DocumentSnapshot,
    cancellation: &SearchCancellation,
    mut visit: impl FnMut(&str, bool) -> Result<(), PagedDocumentError>,
) -> Result<(), PagedDocumentError> {
    let mut offset = 0_u64;
    let mut carry = Vec::new();
    while offset < snapshot.len() {
        if cancellation.is_cancelled() {
            return Err(PagedDocumentError::Cancelled);
        }
        let end = offset
            .saturating_add(SNAPSHOT_STREAM_BYTES)
            .min(snapshot.len());
        let bytes = snapshot
            .read_range(offset..end)
            .map_err(|error| PagedDocumentError::InvalidTransaction(error.to_string()))?;
        offset = end;
        carry.extend_from_slice(&bytes);
        match std::str::from_utf8(&carry) {
            Ok(text) => {
                visit(text, false)?;
                carry.clear();
            }
            Err(error) if error.error_len().is_none() => {
                let valid_up_to = error.valid_up_to();
                if valid_up_to > 0 {
                    let text = std::str::from_utf8(&carry[..valid_up_to])
                        .map_err(|_| PagedDocumentError::InvalidUtf8Boundary)?;
                    visit(text, false)?;
                    carry.drain(..valid_up_to);
                }
            }
            Err(_) => return Err(PagedDocumentError::InvalidUtf8Boundary),
        }
    }
    if !carry.is_empty() {
        let text =
            std::str::from_utf8(&carry).map_err(|_| PagedDocumentError::InvalidUtf8Boundary)?;
        visit(text, true)?;
    } else {
        visit("", true)?;
    }
    Ok(())
}

/// 打开时校验过的文件身份。后端类型不属于持久身份，每次打开都必须重新规划。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileIdentity {
    pub canonical_path: PathBuf,
    pub len: u64,
    pub modified_nanos: Option<u128>,
    pub platform_id: Option<Arc<str>>,
}

impl From<&gmark_paged_document::FileIdentity> for FileIdentity {
    fn from(value: &gmark_paged_document::FileIdentity) -> Self {
        Self {
            canonical_path: value.path.clone(),
            len: value.len,
            modified_nanos: value.modified_nanos,
            platform_id: value
                .os_file_id
                .as_ref()
                .map(|value| Arc::<str>::from(format!("{value:?}"))),
        }
    }
}

/// 两个已知后端的显式和类型；格式 Provider 不得据此分支视图能力。
#[derive(Clone)]
pub enum DocumentStore {
    Resident(Box<ResidentDocument>),
    Paged(Box<PagedDocument>),
}

impl DocumentStore {
    pub const fn kind(&self) -> DocumentBackendKind {
        match self {
            Self::Resident(_) => DocumentBackendKind::Resident,
            Self::Paged(_) => DocumentBackendKind::Paged,
        }
    }

    pub fn revision(&self) -> DocumentRevision {
        match self {
            Self::Resident(document) => DocumentRevision(document.revision()),
            Self::Paged(document) => DocumentRevision(document.revision()),
        }
    }

    pub fn snapshot(&self) -> Arc<dyn DocumentSnapshot> {
        match self {
            Self::Resident(document) => document.snapshot(),
            Self::Paged(document) => Arc::new((**document).clone()),
        }
    }

    /// 普通事务保持单独撤销边界；即使 revision 已过期也先关闭后端输入组。
    fn apply_transaction(&mut self, transaction: &Transaction) -> Result<(), SessionEditError> {
        if transaction.base_revision != self.revision() {
            self.break_typing_group();
            return Err(SessionEditError::Edit(EditError::StaleRevision {
                expected: self.revision(),
                actual: transaction.base_revision,
            }));
        }
        match self {
            Self::Resident(document) => document
                .apply_transaction(transaction)
                .map_err(SessionEditError::Resident),
            Self::Paged(document) => apply_paged_transaction(document, transaction),
        }
    }

    /// 只让 Controller 明确批准的 transaction 参与连续输入撤销组。
    fn apply_typing_transaction(
        &mut self,
        transaction: &Transaction,
        group_id: gmark_document_core::TypingGroupId,
    ) -> Result<(), SessionEditError> {
        if transaction.base_revision != self.revision() {
            self.break_typing_group();
            return Err(SessionEditError::Edit(EditError::StaleRevision {
                expected: self.revision(),
                actual: transaction.base_revision,
            }));
        }
        match self {
            Self::Resident(document) => document
                .apply_typing_transaction(transaction, group_id)
                .map_err(SessionEditError::Resident),
            Self::Paged(document) => {
                apply_paged_typing_transaction(document, transaction, group_id)
            }
        }
    }

    /// 关闭两类后端的输入合并资格，同时保留已有 undo/redo 项。
    fn break_typing_group(&mut self) {
        match self {
            Self::Resident(document) => document.source_document_mut().break_typing_group(),
            Self::Paged(document) => document.break_typing_group(),
        }
    }
}

/// 共享文档的唯一权威正文状态。视图 active mode、selection 与滚动不属于会话，
/// 由 DocumentController 按 DocumentViewInstanceId 管理。
#[derive(Clone)]
pub struct DocumentSession {
    pub profile: DocumentProfile,
    pub store: DocumentStore,
    pub dirty: bool,
    pub file_identity: FileIdentity,
    pub loading_limits: LoadingLimits,
    persisted_encoding: TextEncoding,
    resident_growth_reason: Option<OpenReason>,
    allowed_views: Arc<[DocumentViewId]>,
}

impl DocumentSession {
    pub fn new(
        profile: DocumentProfile,
        store: DocumentStore,
        plan: OpenPlan,
        file_identity: FileIdentity,
    ) -> Result<Self, OpenError> {
        if store.kind() != plan.backend {
            return Err(OpenError::BackendMismatch {
                planned: plan.backend,
                actual: store.kind(),
            });
        }
        let loading_limits = plan.limits;
        let allowed_views: Arc<[DocumentViewId]> = plan
            .allowed_views
            .into_iter()
            .map(|descriptor| descriptor.id)
            .collect::<Vec<_>>()
            .into();
        if !allowed_views.contains(&plan.initial_view) {
            return Err(OpenError::InitialViewUnavailable(plan.initial_view));
        }
        let persisted_encoding = profile.encoding.clone();
        Ok(Self {
            profile,
            store,
            dirty: false,
            file_identity,
            loading_limits,
            persisted_encoding,
            resident_growth_reason: None,
            allowed_views,
        })
    }

    pub fn allowed_views(&self) -> &[DocumentViewId] {
        &self.allowed_views
    }

    pub fn resident_source_document(&self) -> Option<&gmark_document::SourceDocument> {
        match &self.store {
            DocumentStore::Resident(document) => Some(document.source_document()),
            DocumentStore::Paged(_) => None,
        }
    }

    fn resident_source_document_mut(&mut self) -> Option<&mut gmark_document::SourceDocument> {
        match &mut self.store {
            DocumentStore::Resident(document) => Some(document.source_document_mut()),
            DocumentStore::Paged(_) => None,
        }
    }

    pub fn resident_snapshot(&self) -> Option<gmark_document::DocumentSnapshot> {
        self.resident_source_document()
            .map(|document| document.snapshot())
    }

    pub fn source_format_snapshot(&self) -> Option<gmark_document::SourceFormatSnapshot> {
        self.resident_source_document()
            .map(|document| document.source_format())
    }

    pub fn paged_source(&self) -> Result<Option<FileSource>, PagedDocumentError> {
        match &self.store {
            DocumentStore::Resident(_) => Ok(None),
            DocumentStore::Paged(document) => document.prepared_source().map(Some),
        }
    }

    pub fn structured_source(&self) -> Result<Option<FileSource>, PagedDocumentError> {
        self.paged_source()
    }

    pub fn refresh_resident_source_state(&mut self) {
        if let DocumentStore::Resident(document) = &mut self.store {
            document.refresh_source_state();
            self.dirty =
                !document.is_pristine() || self.profile.encoding != self.persisted_encoding;
        }
        self.refresh_resident_profile();
    }
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum SessionEditError {
    #[error(transparent)]
    Edit(#[from] EditError),
    #[error("Resident transaction 失败: {0}")]
    Resident(String),
    #[error("Resident 文档操作失败: {0}")]
    ResidentDocument(#[source] DocumentError),
    #[error("Paged transaction 失败: {0}")]
    Paged(String),
}

fn apply_paged_transaction(
    document: &mut PagedDocument,
    transaction: &Transaction,
) -> Result<(), SessionEditError> {
    document
        .apply_transaction(transaction)
        .map_err(|error| SessionEditError::Paged(error.to_string()))
}

/// 把 Controller 核准的文字输入组送入 PieceTree 历史，沿用普通 Paged stale 校验。
fn apply_paged_typing_transaction(
    document: &mut PagedDocument,
    transaction: &Transaction,
    group_id: gmark_document_core::TypingGroupId,
) -> Result<(), SessionEditError> {
    document
        .apply_typing_transaction(transaction, group_id)
        .map_err(|error| SessionEditError::Paged(error.to_string()))
}

#[cfg(test)]
#[path = "../tests/unit/session.rs"]
mod tests;
