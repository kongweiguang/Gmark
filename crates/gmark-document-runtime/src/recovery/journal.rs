// @author kongweiguang

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use gmark_document::{SourceDocument, SourceFormatSnapshot, TextEdit, Transaction};
use gmark_document_core::{
    PersistenceError, RecoveryAction, RecoveryBackend, RecoveryRecord, SourceAffinity,
};
use gmark_recovery_codec::{RecordKind, encode_record_payload};
use serde::Serialize;

use super::format::{build_format_patch, minimal_edit, stored_format, validate_source_format};
use super::types::{StoredBaseRecord, StoredEditRecord, StoredSelection};
use super::{RecoveredResidentDocument, ResidentRecoveryError, ResidentRecoverySelection};
use crate::fingerprint_resident_file;

const MAX_EDITS_BEFORE_COMPACTION: usize = 256;
const COMPACTION_OVERHEAD_BYTES: u64 = 8 * 1024 * 1024;

/// Resident 文本的独立 Base + 最小 UTF-8 patch journal。
#[derive(Debug)]
pub struct ResidentRecoveryJournal {
    document_id: String,
    journal_path: PathBuf,
    file_path: Option<PathBuf>,
    base_fingerprint: Option<super::ResidentFileFingerprint>,
    base_source: Arc<str>,
    last_source: Arc<str>,
    base_format: SourceFormatSnapshot,
    last_format: SourceFormatSnapshot,
    initialized: bool,
    edit_count: usize,
}

impl ResidentRecoveryJournal {
    /// Creates a journal state. The base frame is deferred until the first change.
    pub fn create(
        recovery_dir: impl AsRef<Path>,
        file_path: Option<PathBuf>,
        source: impl Into<String>,
    ) -> Result<Self, ResidentRecoveryError> {
        let source = source.into();
        let document = SourceDocument::new(&source);
        Self::create_formatted(
            recovery_dir,
            file_path,
            document.text(),
            document.source_format(),
        )
    }

    /// Creates a journal state from LF-normalized source and its original byte format.
    pub fn create_formatted(
        recovery_dir: impl AsRef<Path>,
        file_path: Option<PathBuf>,
        source: impl Into<String>,
        source_format: SourceFormatSnapshot,
    ) -> Result<Self, ResidentRecoveryError> {
        let recovery_dir = recovery_dir.as_ref();
        let source = source.into();
        validate_source_format(&source, &source_format)?;
        fs::create_dir_all(recovery_dir)
            .map_err(|source| ResidentRecoveryError::io("create", recovery_dir, source))?;
        let document_id = uuid::Uuid::new_v4().to_string();
        let base_fingerprint = file_path
            .as_deref()
            .and_then(|path| fingerprint_resident_file(path).ok());
        let source: Arc<str> = source.into();
        Ok(Self {
            journal_path: recovery_dir.join(format!("{document_id}.journal")),
            document_id,
            file_path,
            base_fingerprint,
            base_source: Arc::clone(&source),
            last_source: source,
            base_format: source_format.clone(),
            last_format: source_format,
            initialized: false,
            edit_count: 0,
        })
    }

    /// Continues appending to a successfully replayed journal.
    pub fn resume(document: &RecoveredResidentDocument) -> Self {
        let source: Arc<str> = Arc::from(document.source.as_str());
        Self {
            document_id: document.document_id.clone(),
            journal_path: document.journal_path.clone(),
            file_path: document.file_path.clone(),
            base_fingerprint: document.base_fingerprint.clone(),
            base_source: Arc::clone(&source),
            last_source: source,
            base_format: document.source_format.clone(),
            last_format: document.source_format.clone(),
            initialized: true,
            edit_count: 0,
        }
    }

    /// Returns the durable journal path assigned to this recovery session.
    pub fn path(&self) -> &Path {
        &self.journal_path
    }

    /// Records source using the format inferred from that source.
    pub fn record(
        &mut self,
        source: &str,
        selection: ResidentRecoverySelection,
        view_mode: impl AsRef<str>,
    ) -> Result<bool, ResidentRecoveryError> {
        let document = SourceDocument::new(source);
        self.record_formatted(source, document.source_format(), selection, view_mode)
    }

    /// Records normalized source, exact serialization format, selection, and active view.
    pub fn record_formatted(
        &mut self,
        source: &str,
        source_format: SourceFormatSnapshot,
        selection: ResidentRecoverySelection,
        view_mode: impl AsRef<str>,
    ) -> Result<bool, ResidentRecoveryError> {
        self.record_formatted_with_affinities(
            source,
            source_format,
            selection,
            Some(selection.anchor.affinity),
            Some(selection.head.affinity),
            view_mode,
        )
    }

    /// Records a source snapshot while retaining whether each selection affinity
    /// was absent in a legacy journal frame.
    pub fn record_formatted_with_affinities(
        &mut self,
        source: &str,
        source_format: SourceFormatSnapshot,
        selection: ResidentRecoverySelection,
        anchor_affinity: Option<SourceAffinity>,
        head_affinity: Option<SourceAffinity>,
        view_mode: impl AsRef<str>,
    ) -> Result<bool, ResidentRecoveryError> {
        validate_source_format(source, &source_format)?;
        let text_edit = minimal_edit(&self.last_source, source);
        if text_edit.is_none() && self.last_format == source_format {
            return Ok(false);
        }
        let (range, replacement) = text_edit.unwrap_or((0..0, ""));
        if !self.initialized {
            self.write_base()?;
            self.initialized = true;
        }
        if self.should_compact(source.len()) {
            self.write_compacted_base(
                source,
                &source_format,
                selection,
                anchor_affinity,
                head_affinity,
                view_mode.as_ref(),
            )?;
            let source: Arc<str> = Arc::from(source);
            self.base_source = Arc::clone(&source);
            self.last_source = source;
            self.base_format = source_format.clone();
            self.last_format = source_format;
            self.edit_count = 0;
            return Ok(true);
        }
        let edit = StoredEditRecord {
            start: range.start,
            end: range.end,
            replacement: replacement.to_owned(),
            selection: StoredSelection::from_source_selection_with_affinities(
                selection,
                anchor_affinity,
                head_affinity,
            ),
            view_mode: view_mode.as_ref().to_owned(),
            format_patch: Some(build_format_patch(&self.last_format, &source_format)),
        };
        append_record(&self.journal_path, RecordKind::Edit, &edit)?;
        self.last_source = Arc::from(source);
        self.last_format = source_format;
        self.edit_count = self.edit_count.saturating_add(1);
        Ok(true)
    }

    /// Clears the durable session after a successful save and establishes a new base.
    pub fn checkpoint(
        &mut self,
        file_path: Option<PathBuf>,
        source: impl Into<String>,
    ) -> Result<(), ResidentRecoveryError> {
        let source = source.into();
        let document = SourceDocument::new(&source);
        self.checkpoint_formatted(file_path, document.text(), document.source_format())
    }

    /// Checkpoints LF-normalized source with its original byte-format snapshot.
    pub fn checkpoint_formatted(
        &mut self,
        file_path: Option<PathBuf>,
        source: impl Into<String>,
        source_format: SourceFormatSnapshot,
    ) -> Result<(), ResidentRecoveryError> {
        let source = source.into();
        validate_source_format(&source, &source_format)?;
        remove_journal_file(&self.journal_path)?;
        self.file_path = file_path;
        self.base_fingerprint = self
            .file_path
            .as_deref()
            .and_then(|path| fingerprint_resident_file(path).ok());
        let source: Arc<str> = source.into();
        self.base_source = Arc::clone(&source);
        self.last_source = source;
        self.base_format = source_format.clone();
        self.last_format = source_format;
        self.initialized = false;
        self.edit_count = 0;
        Ok(())
    }

    /// Permanently deletes this unsaved recovery session.
    ///
    /// The journal is consumed so discarded state cannot be resumed accidentally.
    pub fn discard(self) -> Result<(), ResidentRecoveryError> {
        remove_journal_file(&self.journal_path)
    }

    fn write_base(&self) -> Result<(), ResidentRecoveryError> {
        let base = StoredBaseRecord {
            document_id: self.document_id.clone(),
            file_path: self
                .file_path
                .as_ref()
                .map(|path| path.to_string_lossy().into_owned()),
            fingerprint: self.base_fingerprint.clone(),
            source: self.base_source.to_string(),
            source_format: Some(stored_format(&self.base_format)),
            selection: None,
            view_mode: None,
        };
        let bytes = encode_record(RecordKind::Base, &base)?;
        gmark_document::atomic_write(&self.journal_path, &bytes).map_err(|source| {
            ResidentRecoveryError::AtomicWrite {
                path: self.journal_path.clone(),
                source,
            }
        })
    }

    fn should_compact(&self, source_len: usize) -> bool {
        if self.edit_count >= MAX_EDITS_BEFORE_COMPACTION {
            return true;
        }
        let limit = u64::try_from(source_len)
            .unwrap_or(u64::MAX)
            .saturating_mul(4)
            .saturating_add(COMPACTION_OVERHEAD_BYTES);
        fs::metadata(&self.journal_path)
            .map(|metadata| metadata.len() > limit)
            .unwrap_or(false)
    }

    fn write_compacted_base(
        &self,
        source: &str,
        source_format: &SourceFormatSnapshot,
        selection: ResidentRecoverySelection,
        anchor_affinity: Option<SourceAffinity>,
        head_affinity: Option<SourceAffinity>,
        view_mode: &str,
    ) -> Result<(), ResidentRecoveryError> {
        let base = StoredBaseRecord {
            document_id: self.document_id.clone(),
            file_path: self
                .file_path
                .as_ref()
                .map(|path| path.to_string_lossy().into_owned()),
            fingerprint: self.base_fingerprint.clone(),
            source: source.to_owned(),
            source_format: Some(stored_format(source_format)),
            selection: Some(StoredSelection::from_source_selection_with_affinities(
                selection,
                anchor_affinity,
                head_affinity,
            )),
            view_mode: Some(view_mode.to_owned()),
        };
        let bytes = encode_record(RecordKind::Base, &base)?;
        gmark_document::atomic_write(&self.journal_path, &bytes).map_err(|source| {
            ResidentRecoveryError::AtomicWrite {
                path: self.journal_path.clone(),
                source,
            }
        })
    }
}

impl RecoveryBackend for ResidentRecoveryJournal {
    fn record(&mut self, record: &RecoveryRecord) -> Result<(), PersistenceError> {
        let (transaction, group_id) = match &record.action {
            RecoveryAction::Transaction(transaction) => (transaction, None),
            RecoveryAction::TypingTransaction {
                transaction,
                group_id,
            } => (transaction, Some(*group_id)),
            RecoveryAction::Undo | RecoveryAction::Redo => {
                return Err(PersistenceError::Recovery(
                    "Resident recovery requires the resulting source transaction".to_owned(),
                ));
            }
        };
        let mut document =
            SourceDocument::from_normalized(&self.last_source, self.last_format.clone(), 0)
                .ok_or_else(|| {
                    PersistenceError::Recovery(
                        "Resident recovery baseline format is inconsistent".to_owned(),
                    )
                })?;
        let edits = transaction
            .edits
            .iter()
            .map(|edit| {
                let start = usize::try_from(edit.range.start).map_err(|_| {
                    PersistenceError::Recovery(
                        "recovery edit offset does not fit this platform".to_owned(),
                    )
                })?;
                let end = usize::try_from(edit.range.end).map_err(|_| {
                    PersistenceError::Recovery(
                        "recovery edit offset does not fit this platform".to_owned(),
                    )
                })?;
                Ok(TextEdit::new(start..end, edit.replacement.clone()))
            })
            .collect::<Result<Vec<_>, PersistenceError>>()?;
        let transaction = Transaction::new(document.revision(), edits);
        let result = match group_id {
            Some(group_id) => document.apply_typing_transaction(transaction, group_id),
            None => document.apply_transaction(transaction),
        };
        result.map_err(|error| PersistenceError::Recovery(error.to_string()))?;
        self.record_formatted(
            &document.text(),
            document.source_format(),
            record.selection.unwrap_or_default(),
            record.view_id.as_str(),
        )
        .map(|_| ())
        .map_err(|error| PersistenceError::Recovery(error.to_string()))
    }
}

fn encode_record<T: Serialize>(
    kind: RecordKind,
    payload: &T,
) -> Result<Vec<u8>, ResidentRecoveryError> {
    let payload = serde_json::to_vec(payload).map_err(|source| {
        ResidentRecoveryError::json("failed to serialize recovery record", source)
    })?;
    encode_record_payload(kind, &payload).map_err(Into::into)
}

fn append_record<T: Serialize>(
    path: &Path,
    kind: RecordKind,
    payload: &T,
) -> Result<(), ResidentRecoveryError> {
    let bytes = encode_record(kind, payload)?;
    let mut file = OpenOptions::new()
        .create(false)
        .append(true)
        .open(path)
        .map_err(|source| ResidentRecoveryError::io("open", path, source))?;
    file.write_all(&bytes)
        .map_err(|source| ResidentRecoveryError::io("append", path, source))?;
    file.flush()
        .map_err(|source| ResidentRecoveryError::io("flush", path, source))?;
    file.sync_data()
        .map_err(|source| ResidentRecoveryError::io("sync", path, source))?;
    Ok(())
}

fn remove_journal_file(path: &Path) -> Result<(), ResidentRecoveryError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(ResidentRecoveryError::io("remove", path, source)),
    }
}

#[cfg(test)]
#[path = "../../tests/unit/recovery_journal.rs"]
mod tests;
