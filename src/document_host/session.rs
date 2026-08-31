// @author kongweiguang

//! Session planning and recovery-journal adaptation for the document host.

use std::fs::{self, OpenOptions};
use std::io::{self, Read as _, Write as _};

use super::*;

#[cfg(test)]
thread_local! {
    static TEST_CHECKPOINT_FAILURES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static TEST_RENAME_FAILURES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static TEST_REMOVE_FAILURES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static TEST_SUPPRESSION_MARKER_FAILURES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

const RECOVERY_SUPPRESSION_MARKER: &[u8] = b"gmark-recovery-suppressed-v1\n";

/// 普通 crate 单测不应自动绑定用户配置目录；恢复协议测试必须显式提供临时目录，
/// 才能同时验证持久化语义并避免测试夹具在下次真实启动时被恢复成用户 Tab。
pub(super) fn runtime_recovery_dirs() -> Option<gmark_config::AppDirs> {
    #[cfg(test)]
    {
        None
    }
    #[cfg(not(test))]
    {
        match gmark_config::AppDirs::from_system() {
            Ok(dirs) => Some(dirs),
            Err(error) => {
                eprintln!("recovery persistence disabled: {error:#}");
                None
            }
        }
    }
}

pub(super) fn session_plan(
    profile: &gmark_document_core::DocumentProfile,
    probe: &OpenProbe,
    strategy: OpenStrategy,
    retain_resident_session: bool,
) -> gmark_document_core::OpenPlan {
    let mut policy = gmark_document_core::LoadingPolicy {
        max_resident_bytes: Some(probe.options.max_resident_bytes),
        force_safe_source: probe.force_safe_source,
    };
    // 恢复日志可能显式要求 Paged，即使当前文件已缩小；仍沿用打开时阈值，
    // 只把本次会话强制为安全 Source，不污染偏好设置。
    if strategy == OpenStrategy::Paged
        && gmark_document_core::OpenPolicyResolver
            .resolve(policy, profile)
            .backend
            == gmark_document_core::DocumentBackendKind::Resident
    {
        policy.force_safe_source = true;
    }
    let plan = gmark_document_core::OpenPolicyResolver.resolve(policy, profile);
    if retain_resident_session
        && strategy == OpenStrategy::Resident
        && plan.backend == gmark_document_core::DocumentBackendKind::Paged
    {
        let mut resident = gmark_document_core::OpenPolicyResolver.resolve(
            gmark_document_core::LoadingPolicy {
                max_resident_bytes: Some(u64::MAX),
                ..gmark_document_core::LoadingPolicy::default()
            },
            profile,
        );
        resident.limits = plan.limits;
        return resident;
    }
    plan
}

/// Probe 后只在这里安装正文后端；格式 Controller 之后只能持有统一 session。
/// Build the single runtime session from the IO-prepared source.  The
/// `PreparedUtf8Source` is moved into the Paged backend so its shadow and
/// encoded save plan have exactly the same lifetime as the shared Controller
/// body.  No host-side copy of either value is retained.
pub(super) fn build_document_session_from_prepared(
    probe: &OpenProbe,
    original_source: &FileSource,
    prepared: PreparedUtf8Source,
    index: LineIndex,
    retain_resident_session: bool,
) -> Result<DocumentSession, PagedDocumentError> {
    let profile = probe.profile();
    let plan = session_plan(&profile, probe, probe.strategy, retain_resident_session);
    let source_identity = original_source.identity()?;
    if source_identity != probe.identity {
        return Err(PagedDocumentError::SourceChanged);
    }
    let file_identity = gmark_document_runtime::FileIdentity::from(&source_identity);
    let prepared_source = prepared.source().clone();
    let store = if probe.strategy == OpenStrategy::Resident {
        let bytes = prepared_source.read_range(0, prepared_source.identity()?.len)?;
        if original_source.identity()? != probe.identity {
            return Err(PagedDocumentError::SourceChanged);
        }
        let text = std::str::from_utf8(&bytes).map_err(|_| PagedDocumentError::Binary)?;
        gmark_document_runtime::DocumentStore::Resident(Box::new(
            gmark_document_runtime::ResidentDocument::new(
                text,
                probe.encoding.clone(),
                source_identity,
            ),
        ))
    } else {
        let document = PagedDocumentAdapter::from_prepared(prepared, index)?;
        gmark_document_runtime::DocumentStore::Paged(Box::new(document))
    };
    if original_source.identity()? != probe.identity {
        return Err(PagedDocumentError::SourceChanged);
    }
    DocumentSession::new(profile, store, plan, file_identity)
        .map_err(|error| PagedDocumentError::InvalidTransaction(error.to_string()))
}

/// Test-only compatibility fixture for callers that already own a source and
/// index.  Production indexing/reload always pass the prepared source above;
/// this adapter prepares a fresh source solely to keep older unit fixtures
/// concise and never installs a second runtime body.
#[cfg(test)]
pub(super) fn build_document_session(
    probe: &OpenProbe,
    original_source: &FileSource,
    _utf8_source: FileSource,
    index: LineIndex,
    retain_resident_session: bool,
) -> Result<DocumentSession, PagedDocumentError> {
    if original_source.identity()? != probe.identity {
        return Err(PagedDocumentError::SourceChanged);
    }
    let prepared = prepare_utf8_source(original_source.clone(), probe.encoding.clone())?;
    let index = if matches!(probe.encoding, TextEncoding::Utf8 { .. }) {
        index
    } else {
        LineIndex::build(prepared.source())?
    };
    build_document_session_from_prepared(
        probe,
        original_source,
        prepared,
        index,
        retain_resident_session,
    )
}

pub(super) fn build_paged_session(
    probe: &OpenProbe,
    document: PieceDocument,
    identity: gmark_paged_document::FileIdentity,
) -> Result<DocumentSession, PagedDocumentError> {
    let profile = probe.profile();
    let mut plan = session_plan(&profile, probe, OpenStrategy::Paged, false);
    if derived_views_enabled(probe.strategy) {
        // 恢复正文用 PieceDocument 保留 undo/redo，所以后端仍是 Paged；但原文件在
        // Resident 阈值内时，结构索引由恢复快照构建，必须继承其表格/图视图白名单。
        let resident_plan = session_plan(&profile, probe, OpenStrategy::Resident, false);
        plan.allowed_views = resident_plan.allowed_views;
    }
    DocumentSession::new(
        profile,
        gmark_document_runtime::DocumentStore::Paged(Box::new(PagedDocumentAdapter::new(document))),
        plan,
        gmark_document_runtime::FileIdentity::from(&identity),
    )
    .map_err(|error| PagedDocumentError::InvalidTransaction(error.to_string()))
}

pub(super) fn verify_saved_session_readback(
    expected: &DocumentSession,
    actual: &DocumentSession,
    cancellation: &SearchCancellation,
) -> Result<(), PagedDocumentError> {
    if expected.len() != actual.len() {
        return Err(PagedDocumentError::InvalidTransaction(
            "saved readback length differs from the save snapshot".into(),
        ));
    }
    const VERIFY_CHUNK_BYTES: u64 = 8 * 1024 * 1024;
    let mut start = 0u64;
    while start < expected.len() {
        if cancellation.is_cancelled() {
            return Err(PagedDocumentError::Cancelled);
        }
        let end = start.saturating_add(VERIFY_CHUNK_BYTES).min(expected.len());
        if expected.read_range(start..end)? != actual.read_range(start..end)? {
            return Err(PagedDocumentError::InvalidTransaction(
                "saved readback bytes differ from the save snapshot".into(),
            ));
        }
        start = end;
    }
    Ok(())
}

type StructureInput = (FileSource, LineIndex, Option<Arc<[u8]>>);

pub(super) fn structure_input_for_session(
    document: &DocumentSession,
    prepared_source: &FileSource,
    prepared_index: &LineIndex,
    cancellation: &SearchCancellation,
) -> Result<StructureInput, PagedDocumentError> {
    if document.store.kind() == gmark_document_core::DocumentBackendKind::Paged {
        return Ok((prepared_source.clone(), prepared_index.clone(), None));
    }
    if cancellation.is_cancelled() {
        return Err(PagedDocumentError::Cancelled);
    }
    let bytes: Arc<[u8]> = document
        .snapshot()
        .read_range(0..document.len())
        .map_err(|error| PagedDocumentError::InvalidTransaction(error.to_string()))?
        .into();
    Ok((prepared_source.clone(), prepared_index.clone(), Some(bytes)))
}

pub(super) fn modifier_horizontal_wheel_delta(
    shift: bool,
    control: bool,
    delta_x: f32,
    delta_y: f32,
) -> Option<f32> {
    ((shift || control) && delta_y.abs() >= delta_x.abs()).then_some(delta_y)
}

pub(super) fn recovery_view_id(mode: DocumentHostViewMode) -> DocumentViewId {
    match mode {
        DocumentHostViewMode::Source => DocumentViewId::source(),
        DocumentHostViewMode::Live => DocumentViewId::new("live"),
        DocumentHostViewMode::Structure => DocumentViewId::new("preview"),
        DocumentHostViewMode::Split => DocumentViewId::new("split"),
    }
}

/// Recovery backend selection follows the installed document store, not the file's
/// most recent probe. This keeps a resident session on the runtime journal contract
/// even when its on-disk file later grows past the open threshold.
pub(crate) enum DocumentRecoveryJournal {
    Resident(Box<ResidentRecoveryJournal>),
    Paged(PagedRecoveryJournal),
}

impl DocumentRecoveryJournal {
    /// Select the overflow policy without borrowing a live session or touching
    /// the filesystem, keeping queue admission a UI-safe decision.
    pub(super) fn is_paged(&self) -> bool {
        matches!(self, Self::Paged(_))
    }

    pub(super) fn create(
        recovery_dir: &Path,
        source: &FileSource,
        encoding: TextEncoding,
        document: &DocumentSession,
    ) -> Result<Self, PagedDocumentError> {
        // 进程重启后内存队列不复存在；retired 和 suppression sidecar 仍是可安全重试的
        // 清理标记，尽力回收它们不能阻止本次新日志建立。
        let _ = retry_retired_recovery_journal_artifacts(recovery_dir);
        match document.store.kind() {
            gmark_document_core::DocumentBackendKind::Resident => {
                let source_document = document.resident_source_document().ok_or_else(|| {
                    PagedDocumentError::Recovery(
                        "resident recovery requires a resident source document".to_owned(),
                    )
                })?;
                ResidentRecoveryJournal::create_formatted(
                    recovery_dir,
                    Some(source.path().to_path_buf()),
                    source_document.text(),
                    source_document.source_format(),
                )
                .map(|journal| Self::Resident(Box::new(journal)))
                .map_err(map_resident_recovery_error)
            }
            gmark_document_core::DocumentBackendKind::Paged => {
                PagedRecoveryJournal::create(recovery_dir, source, encoding).map(Self::Paged)
            }
        }
    }

    /// Create a journal from an immutable Controller snapshot.  Shared hosts
    /// use this background-only constructor because reconstructing a live
    /// `DocumentSession` would re-enter the Controller while recovery setup is
    /// still racing the first edit.
    pub(super) fn create_from_snapshot(
        recovery_dir: &Path,
        source: &FileSource,
        snapshot: &DocumentSaveSnapshot,
    ) -> Result<Self, PagedDocumentError> {
        let _ = retry_retired_recovery_journal_artifacts(recovery_dir);
        if let Some(format) = snapshot.source_format.clone() {
            let bytes = snapshot
                .read_all()
                .map_err(|error| PagedDocumentError::Recovery(error.to_string()))?;
            let text = String::from_utf8(bytes)
                .map_err(|error| PagedDocumentError::Recovery(error.to_string()))?;
            return ResidentRecoveryJournal::create_formatted(
                recovery_dir,
                Some(snapshot.identity.canonical_path.clone()),
                text,
                format,
            )
            .map(|journal| Self::Resident(Box::new(journal)))
            .map_err(map_resident_recovery_error);
        }
        PagedRecoveryJournal::create(recovery_dir, source, snapshot.encoding.clone())
            .map(Self::Paged)
    }

    /// Resident journals receive the resulting source snapshot so undo/redo and
    /// formatting-only changes retain the same journal semantics as direct edits.
    #[cfg(test)]
    pub(super) fn record_after_change(
        &mut self,
        document: &DocumentSession,
        record: &RecoveryRecord,
    ) -> Result<(), gmark_document_core::PersistenceError> {
        match self {
            Self::Resident(journal) => {
                let source_document = document.resident_source_document().ok_or_else(|| {
                    gmark_document_core::PersistenceError::Recovery(
                        "resident recovery requires a resident source document".to_owned(),
                    )
                })?;
                journal
                    .record_formatted(
                        &source_document.text(),
                        source_document.source_format(),
                        record.selection.unwrap_or_default(),
                        record.view_id.as_str(),
                    )
                    .map(|_| ())
                    .map_err(|error| {
                        gmark_document_core::PersistenceError::Recovery(error.to_string())
                    })
            }
            Self::Paged(journal) => journal.record(record),
        }
    }

    /// Records a Controller-captured immutable snapshot without borrowing the
    /// Controller.  This is the worker-only path: resident source material is
    /// read from the detached snapshot and Paged journals consume the ordered
    /// command, so no recovery I/O can re-enter the Controller mutex.
    pub(super) fn record_snapshot(
        &mut self,
        snapshot: &gmark_document_runtime::DocumentSaveSnapshot,
        record: &RecoveryRecord,
    ) -> Result<(), PagedDocumentError> {
        match self {
            Self::Resident(journal) => {
                let format = snapshot.source_format.clone().ok_or_else(|| {
                    PagedDocumentError::Recovery(
                        "resident recovery snapshot has no source format".to_owned(),
                    )
                })?;
                let bytes = snapshot
                    .read_all()
                    .map_err(|error| PagedDocumentError::Recovery(error.to_string()))?;
                let source = String::from_utf8(bytes)
                    .map_err(|error| PagedDocumentError::Recovery(error.to_string()))?;
                journal
                    .record_formatted(
                        &source,
                        format,
                        record.selection.unwrap_or_default(),
                        record.view_id.as_str(),
                    )
                    .map(|_| ())
                    .map_err(map_resident_recovery_error)
            }
            Self::Paged(journal) => gmark_document_core::RecoveryBackend::record(journal, record)
                .map_err(|error| PagedDocumentError::Recovery(error.to_string())),
        }
    }

    /// Checkpoints a detached save snapshot.  The caller may enqueue this
    /// after a successful save; the active worker retains the journal state so
    /// later edits cannot race a UI-thread file removal.
    pub(super) fn checkpoint_snapshot(
        &mut self,
        snapshot: &gmark_document_runtime::DocumentSaveSnapshot,
    ) -> Result<(), PagedDocumentError> {
        #[cfg(test)]
        if take_test_checkpoint_failure() {
            return Err(PagedDocumentError::Recovery(
                "test recovery checkpoint failure".to_owned(),
            ));
        }
        match self {
            Self::Resident(journal) => {
                let format = snapshot.source_format.clone().ok_or_else(|| {
                    PagedDocumentError::Recovery(
                        "resident checkpoint snapshot has no source format".to_owned(),
                    )
                })?;
                let bytes = snapshot
                    .read_all()
                    .map_err(|error| PagedDocumentError::Recovery(error.to_string()))?;
                let source = String::from_utf8(bytes)
                    .map_err(|error| PagedDocumentError::Recovery(error.to_string()))?;
                journal
                    .checkpoint_formatted(
                        Some(snapshot.identity.canonical_path.clone()),
                        source,
                        format,
                    )
                    .map_err(map_resident_recovery_error)
            }
            Self::Paged(journal) => journal.checkpoint(),
        }
    }

    /// A successful save/discard removes the old durable session. The coordinator
    /// retains a failed removal as separate retired cleanup work.
    #[cfg(test)]
    pub(super) fn checkpoint(
        &mut self,
        document: &DocumentSession,
    ) -> Result<(), PagedDocumentError> {
        #[cfg(test)]
        if take_test_checkpoint_failure() {
            return Err(PagedDocumentError::Recovery(
                "test recovery checkpoint failure".to_owned(),
            ));
        }
        match self {
            Self::Resident(journal) => {
                let source_document = document.resident_source_document().ok_or_else(|| {
                    PagedDocumentError::Recovery(
                        "resident recovery requires a resident source document".to_owned(),
                    )
                })?;
                journal
                    .checkpoint_formatted(
                        Some(document.file_identity.canonical_path.clone()),
                        source_document.text(),
                        source_document.source_format(),
                    )
                    .map_err(map_resident_recovery_error)
            }
            Self::Paged(journal) => journal.checkpoint(),
        }
    }

    #[cfg(test)]
    pub(super) fn discard(self) -> Result<(), PagedDocumentError> {
        match self {
            Self::Resident(journal) => journal.discard().map_err(map_resident_recovery_error),
            Self::Paged(journal) => journal.checkpoint(),
        }
    }

    /// Converts an old active journal into cleanup-only work after its checkpoint failed.
    pub(super) fn retire_for_cleanup(self) -> (RetiredRecoveryJournal, Option<PagedDocumentError>) {
        let path = match &self {
            Self::Resident(journal) => journal.path().to_path_buf(),
            Self::Paged(journal) => journal.path().to_path_buf(),
        };
        let retired_path = retired_recovery_journal_path(&path);
        match rename_recovery_journal(&path, &retired_path) {
            Ok(()) => (
                RetiredRecoveryJournal {
                    journal_path: retired_path,
                    suppression_marker: None,
                },
                None,
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (
                RetiredRecoveryJournal {
                    journal_path: path,
                    suppression_marker: None,
                },
                None,
            ),
            Err(_) => {
                let marker = suppression_marker_path(&path);
                let cleanup = RetiredRecoveryJournal {
                    journal_path: path,
                    suppression_marker: Some(marker),
                };
                let marker_error = cleanup.ensure_suppression_marker().err();
                (cleanup, marker_error)
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn fail_next_checkpoint_for_test() {
        TEST_CHECKPOINT_FAILURES.with(|failures| {
            failures.set(failures.get().saturating_add(1));
        });
    }

    #[cfg(test)]
    pub(crate) fn fail_next_rename_for_test() {
        TEST_RENAME_FAILURES.with(|failures| {
            failures.set(failures.get().saturating_add(1));
        });
    }

    #[cfg(test)]
    pub(crate) fn fail_next_remove_for_test() {
        TEST_REMOVE_FAILURES.with(|failures| {
            failures.set(failures.get().saturating_add(1));
        });
    }

    #[cfg(test)]
    pub(crate) fn fail_next_suppression_marker_for_test() {
        TEST_SUPPRESSION_MARKER_FAILURES.with(|failures| {
            failures.set(failures.get().saturating_add(1));
        });
    }
}

/// Cleanup-only journal state. It is either renamed out of scanner scope or paired with a
/// suppression sidecar while its original path remains available for future deletion attempts.
pub(super) struct RetiredRecoveryJournal {
    journal_path: PathBuf,
    suppression_marker: Option<PathBuf>,
}

impl RetiredRecoveryJournal {
    pub(super) fn retry_cleanup(&self) -> Result<(), PagedDocumentError> {
        if self.suppression_marker.is_some() {
            self.ensure_suppression_marker()?;
        }
        // suppression marker 存在时必须先删除旧 journal，再删除 marker；否则重启窗口会
        // 重新扫描已经保存的内容。
        remove_recovery_journal_file(&self.journal_path)?;
        if let Some(marker) = &self.suppression_marker {
            remove_recovery_journal_file(marker)?;
        }
        Ok(())
    }

    fn ensure_suppression_marker(&self) -> Result<(), PagedDocumentError> {
        let Some(marker) = &self.suppression_marker else {
            return Ok(());
        };
        create_suppression_marker(marker)
    }
}

fn retry_retired_recovery_journal_artifacts(recovery_dir: &Path) -> Result<(), PagedDocumentError> {
    let entries = match fs::read_dir(recovery_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(source) => {
            return Err(PagedDocumentError::Io {
                path: recovery_dir.to_path_buf(),
                source,
            });
        }
    };
    for entry in entries {
        let entry = entry.map_err(|source| PagedDocumentError::Io {
            path: recovery_dir.to_path_buf(),
            source,
        })?;
        let path = entry.path();
        let Some(name) = path.file_name() else {
            continue;
        };
        let name = name.to_string_lossy();
        if name.ends_with(".journal.retired") || name.ends_with(".large-journal.retired") {
            remove_recovery_journal_file(&path)?;
            continue;
        }
        if let Some(journal_path) = journal_path_for_suppression_marker(&path) {
            // 启动清理同样遵守 journal -> marker 的顺序。journal 删除失败时保留 marker，
            // 让扫描器继续忽略该旧内容，而不是把 marker 单独删掉。
            remove_recovery_journal_file(&journal_path)?;
            remove_recovery_journal_file(&path)?;
        }
    }
    Ok(())
}

fn retired_recovery_journal_path(path: &Path) -> PathBuf {
    let mut retired = path.to_path_buf();
    match path.extension() {
        Some(extension) => {
            retired.set_extension(format!("{}.retired", extension.to_string_lossy()))
        }
        None => retired.set_extension("retired"),
    };
    retired
}

fn suppression_marker_path(journal_path: &Path) -> PathBuf {
    let mut marker = journal_path.to_path_buf();
    match journal_path.extension() {
        Some(extension) => {
            marker.set_extension(format!("{}.suppressed", extension.to_string_lossy()))
        }
        None => marker.set_extension("suppressed"),
    };
    marker
}

fn journal_path_for_suppression_marker(marker_path: &Path) -> Option<PathBuf> {
    let extension = marker_path.extension()?.to_string_lossy();
    let journal_extension = extension.strip_suffix(".suppressed")?;
    if !matches!(journal_extension, "journal" | "large-journal") {
        return None;
    }
    let mut journal = marker_path.to_path_buf();
    journal.set_extension(journal_extension);
    Some(journal)
}

fn rename_recovery_journal(source: &Path, destination: &Path) -> io::Result<()> {
    #[cfg(test)]
    if take_test_rename_failure() {
        return Err(test_recovery_io_error("rename"));
    }
    fs::rename(source, destination)
}

fn create_suppression_marker(marker_path: &Path) -> Result<(), PagedDocumentError> {
    #[cfg(test)]
    if take_test_suppression_marker_failure() {
        return Err(PagedDocumentError::Io {
            path: marker_path.to_path_buf(),
            source: test_recovery_io_error("create suppression marker"),
        });
    }
    let mut marker = match OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(marker_path)
    {
        Ok(marker) => marker,
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            return validate_suppression_marker(marker_path);
        }
        Err(source) => {
            return Err(PagedDocumentError::Io {
                path: marker_path.to_path_buf(),
                source,
            });
        }
    };
    let write_result = marker
        .write_all(RECOVERY_SUPPRESSION_MARKER)
        .and_then(|()| marker.sync_all());
    if let Err(source) = write_result {
        drop(marker);
        let _ = fs::remove_file(marker_path);
        return Err(PagedDocumentError::Io {
            path: marker_path.to_path_buf(),
            source,
        });
    }
    Ok(())
}

fn validate_suppression_marker(marker_path: &Path) -> Result<(), PagedDocumentError> {
    let mut marker = OpenOptions::new()
        .read(true)
        .write(true)
        .open(marker_path)
        .map_err(|source| PagedDocumentError::Io {
            path: marker_path.to_path_buf(),
            source,
        })?;
    let mut contents = [0; RECOVERY_SUPPRESSION_MARKER.len()];
    marker
        .read_exact(&mut contents)
        .map_err(|source| PagedDocumentError::Io {
            path: marker_path.to_path_buf(),
            source,
        })?;
    let mut trailing = [0u8; 1];
    if marker
        .read(&mut trailing)
        .map_err(|source| PagedDocumentError::Io {
            path: marker_path.to_path_buf(),
            source,
        })?
        != 0
        || contents != RECOVERY_SUPPRESSION_MARKER
    {
        return Err(PagedDocumentError::Recovery(
            "recovery suppression marker has unexpected contents".to_owned(),
        ));
    }
    marker.sync_all().map_err(|source| PagedDocumentError::Io {
        path: marker_path.to_path_buf(),
        source,
    })
}

fn remove_recovery_journal_file(path: &Path) -> Result<(), PagedDocumentError> {
    #[cfg(test)]
    if take_test_remove_failure() {
        return Err(PagedDocumentError::Io {
            path: path.to_path_buf(),
            source: test_recovery_io_error("remove"),
        });
    }
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(PagedDocumentError::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
}

#[cfg(test)]
fn take_test_checkpoint_failure() -> bool {
    TEST_CHECKPOINT_FAILURES.with(take_test_failure)
}

#[cfg(test)]
fn take_test_rename_failure() -> bool {
    TEST_RENAME_FAILURES.with(take_test_failure)
}

#[cfg(test)]
fn take_test_remove_failure() -> bool {
    TEST_REMOVE_FAILURES.with(take_test_failure)
}

#[cfg(test)]
fn take_test_suppression_marker_failure() -> bool {
    TEST_SUPPRESSION_MARKER_FAILURES.with(take_test_failure)
}

#[cfg(test)]
fn take_test_failure(failures: &std::cell::Cell<usize>) -> bool {
    let remaining = failures.get();
    if remaining == 0 {
        false
    } else {
        failures.set(remaining - 1);
        true
    }
}

#[cfg(test)]
fn test_recovery_io_error(operation: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        format!("test recovery {operation} failure"),
    )
}

fn map_resident_recovery_error(error: ResidentRecoveryError) -> PagedDocumentError {
    PagedDocumentError::Recovery(error.to_string())
}

#[cfg(test)]
pub(super) fn record_recovery_transaction(
    journal: &mut DocumentRecoveryJournal,
    document: &DocumentSession,
    base_revision: u64,
    range: Range<u64>,
    replacement: impl Into<Arc<str>>,
    selection: Option<SourceSelection>,
    view_id: DocumentViewId,
) -> Result<(), gmark_document_core::PersistenceError> {
    journal.record_after_change(
        document,
        &RecoveryRecord {
            action: RecoveryAction::Transaction(Transaction::new(
                gmark_document_core::DocumentRevision(base_revision),
                vec![SourceEdit::new(range, replacement)],
            )),
            selection,
            view_id,
        },
    )
}

pub(super) fn derived_views_enabled(strategy: OpenStrategy) -> bool {
    strategy == OpenStrategy::Resident
}
