// @author kongweiguang

//! Compatibility adapter for resident crash recovery.
//!
//! Durable journal framing, schema evolution, source-format patches, and replay
//! live in `gmark-document-runtime`. This module preserves the root crate's
//! historical Editor-facing shapes while delegating all durable behavior to that
//! runtime API.

use gmark_document::SourceFormatSnapshot;
use gmark_document_core::{
    PersistenceError, RecoveryBackend, RecoveryRecord, SourceAffinity, SourceAnchor,
    SourceSelection,
};
use gmark_document_runtime::{
    RecoveredResidentDocument, ResidentFileFingerprint, ResidentRecoveryJournal,
    ResidentRecoveryReadStatus, fingerprint_resident_file, load_resident_recovery_journals,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RecoverySelection {
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) reversed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) anchor_affinity: Option<RecoverySelectionAffinity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) head_affinity: Option<RecoverySelectionAffinity>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RecoverySelectionAffinity {
    Before,
    After,
}

impl RecoverySelection {
    pub(crate) fn from_source_selection(selection: SourceSelection) -> Self {
        let range = selection.range();
        Self {
            start: range.start.min(usize::MAX as u64) as usize,
            end: range.end.min(usize::MAX as u64) as usize,
            reversed: selection.reversed(),
            anchor_affinity: Some(selection.anchor.affinity.into()),
            head_affinity: Some(selection.head.affinity.into()),
        }
    }

    pub(crate) fn source_selection(&self) -> SourceSelection {
        let start = self.start.min(self.end);
        let end = self.start.max(self.end);
        let fallback = SourceSelection::from_range(start as u64..end as u64, self.reversed);
        SourceSelection {
            anchor: SourceAnchor::new(
                fallback.anchor.byte_offset,
                self.anchor_affinity
                    .map(Into::into)
                    .unwrap_or(fallback.anchor.affinity),
            ),
            head: SourceAnchor::new(
                fallback.head.byte_offset,
                self.head_affinity
                    .map(Into::into)
                    .unwrap_or(fallback.head.affinity),
            ),
        }
    }
}

impl From<SourceAffinity> for RecoverySelectionAffinity {
    fn from(value: SourceAffinity) -> Self {
        match value {
            SourceAffinity::Before => Self::Before,
            SourceAffinity::After => Self::After,
        }
    }
}

impl From<RecoverySelectionAffinity> for SourceAffinity {
    fn from(value: RecoverySelectionAffinity) -> Self {
        match value {
            RecoverySelectionAffinity::Before => Self::Before,
            RecoverySelectionAffinity::After => Self::After,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RecoveryReadStatus {
    Complete,
    TruncatedTail,
}

#[derive(Clone, Debug)]
pub(crate) struct RecoveredDocument {
    pub(crate) document_id: String,
    pub(crate) journal_path: PathBuf,
    pub(crate) file_path: Option<PathBuf>,
    pub(crate) source: String,
    pub(crate) source_format: SourceFormatSnapshot,
    pub(crate) selection: RecoverySelection,
    pub(crate) view_mode: String,
    pub(crate) read_status: RecoveryReadStatus,
    pub(crate) base_file_changed: bool,
    base_fingerprint: Option<FileFingerprint>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct FileFingerprint {
    path: String,
    size: u64,
    modified_nanos: Option<u128>,
    crc32: u32,
}

/// Root-crate compatibility wrapper around the Wave 1 resident runtime journal.
pub(crate) struct RecoveryJournal {
    inner: ResidentRecoveryJournal,
}

impl RecoveryJournal {
    pub(crate) fn create(
        recovery_dir: &Path,
        file_path: Option<PathBuf>,
        source: String,
    ) -> anyhow::Result<Self> {
        Ok(Self::from_runtime(ResidentRecoveryJournal::create(
            recovery_dir,
            file_path,
            source,
        )?))
    }

    pub(crate) fn resume(document: &RecoveredDocument) -> Self {
        let recovered = document.as_runtime();
        Self {
            inner: ResidentRecoveryJournal::resume(&recovered),
        }
    }

    pub(crate) fn record_formatted(
        &mut self,
        source: &str,
        source_format: SourceFormatSnapshot,
        selection: RecoverySelection,
        view_mode: &str,
    ) -> anyhow::Result<bool> {
        Ok(self.inner.record_formatted_with_affinities(
            source,
            source_format,
            selection.source_selection(),
            selection.anchor_affinity.map(Into::into),
            selection.head_affinity.map(Into::into),
            view_mode,
        )?)
    }

    pub(crate) fn checkpoint_formatted(
        &mut self,
        file_path: Option<PathBuf>,
        source: String,
        source_format: SourceFormatSnapshot,
    ) -> anyhow::Result<()> {
        self.inner
            .checkpoint_formatted(file_path, source, source_format)?;
        Ok(())
    }

    fn from_runtime(inner: ResidentRecoveryJournal) -> Self {
        Self { inner }
    }
}

impl RecoveryBackend for RecoveryJournal {
    fn record(&mut self, record: &RecoveryRecord) -> Result<(), PersistenceError> {
        RecoveryBackend::record(&mut self.inner, record)
    }
}

impl RecoveredDocument {
    fn from_runtime(
        document: RecoveredResidentDocument,
        anchor_affinity: Option<SourceAffinity>,
        head_affinity: Option<SourceAffinity>,
    ) -> Self {
        let mut selection = RecoverySelection::from_source_selection(document.selection);
        selection.anchor_affinity = anchor_affinity.map(Into::into);
        selection.head_affinity = head_affinity.map(Into::into);
        Self {
            document_id: document.document_id,
            journal_path: document.journal_path,
            file_path: document.file_path,
            source: document.source,
            source_format: document.source_format,
            selection,
            view_mode: document.view_mode,
            read_status: match document.read_status {
                ResidentRecoveryReadStatus::Complete => RecoveryReadStatus::Complete,
                ResidentRecoveryReadStatus::TruncatedTail => RecoveryReadStatus::TruncatedTail,
            },
            base_file_changed: document.base_file_changed,
            base_fingerprint: document.base_fingerprint.map(FileFingerprint::from),
        }
    }

    fn as_runtime(&self) -> RecoveredResidentDocument {
        RecoveredResidentDocument {
            document_id: self.document_id.clone(),
            journal_path: self.journal_path.clone(),
            file_path: self.file_path.clone(),
            source: self.source.clone(),
            source_format: self.source_format.clone(),
            selection: self.selection.source_selection(),
            view_mode: self.view_mode.clone(),
            read_status: match self.read_status {
                RecoveryReadStatus::Complete => ResidentRecoveryReadStatus::Complete,
                RecoveryReadStatus::TruncatedTail => ResidentRecoveryReadStatus::TruncatedTail,
            },
            base_file_changed: self.base_file_changed,
            base_fingerprint: self.base_fingerprint.clone().map(Into::into),
        }
    }
}

impl From<ResidentFileFingerprint> for FileFingerprint {
    fn from(value: ResidentFileFingerprint) -> Self {
        Self {
            path: value.path,
            size: value.size,
            modified_nanos: value.modified_nanos,
            crc32: value.crc32,
        }
    }
}

impl From<FileFingerprint> for ResidentFileFingerprint {
    fn from(value: FileFingerprint) -> Self {
        Self {
            path: value.path,
            size: value.size,
            modified_nanos: value.modified_nanos,
            crc32: value.crc32,
        }
    }
}

pub(crate) fn load_recovery_documents(
    recovery_dir: &Path,
) -> anyhow::Result<Vec<RecoveredDocument>> {
    Ok(load_resident_recovery_journals(recovery_dir)?
        .into_iter()
        .map(|journal| {
            RecoveredDocument::from_runtime(
                journal.document,
                journal.anchor_affinity,
                journal.head_affinity,
            )
        })
        .collect())
}

/// Keep the editor's save baseline stable when a successful-save event replaces
/// the user's path spelling with the canonical path returned by the file writer.
pub(crate) fn fingerprint_file(path: &Path) -> anyhow::Result<FileFingerprint> {
    let mut fingerprint: FileFingerprint = fingerprint_resident_file(path)?.into();
    fingerprint.path = path
        .canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .into_owned();
    Ok(fingerprint)
}

#[cfg(test)]
#[path = "../../tests/unit/recovery.rs"]
mod tests;

// A few legacy editor scenarios exercise malformed journal frames directly.
// Keep their codec fixtures outside the production adapter while preserving
// the historical crate-local test entry points.
#[cfg(test)]
pub(crate) use tests::{fingerprint_contents, replay_journal};
