// @author kongweiguang

use std::ops::Range;
use std::path::PathBuf;

use gmark_document_core::{SourceAffinity, SourceSelection, TextEncoding, TypingGroupId};
use serde::{Deserialize, Serialize};

use super::PagedRecoverySelection;
use crate::{FileSource, PagedDocumentError};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct BaseRecord {
    pub(super) path: PathBuf,
    pub(super) len: u64,
    pub(super) modified_nanos: Option<u128>,
    pub(super) sampled_hash: u32,
    pub(super) encoding: RecoveryEncoding,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum RecoveryEncoding {
    Utf8,
    Utf8Bom,
    Utf16Le,
    Utf16Be,
    Legacy,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub(super) enum EditRecord {
    ReplaceChunk {
        transaction: u64,
        start: u64,
        end: u64,
        chunk_index: u32,
        chunk_count: u32,
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        typing_group: Option<TypingGroupId>,
        selection: Option<SelectionRecord>,
        view_mode: String,
    },
    Undo {
        selection: Option<SelectionRecord>,
        view_mode: String,
    },
    Redo {
        selection: Option<SelectionRecord>,
        view_mode: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct SelectionRecord {
    pub(super) start: u64,
    pub(super) end: u64,
    pub(super) reversed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) anchor_affinity: Option<RecoveryAffinity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) head_affinity: Option<RecoveryAffinity>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum RecoveryAffinity {
    Before,
    After,
}

pub(super) struct PendingReplace {
    pub(super) transaction: u64,
    pub(super) range: Range<u64>,
    pub(super) chunk_count: u32,
    pub(super) chunks: Vec<String>,
    pub(super) typing_group: Option<TypingGroupId>,
    pub(super) selection: Option<PagedRecoverySelection>,
    pub(super) view_mode: String,
}

impl RecoveryEncoding {
    pub(super) fn from_encoding(encoding: &TextEncoding) -> Self {
        match encoding {
            TextEncoding::Utf8 { bom: false } => Self::Utf8,
            TextEncoding::Utf8 { bom: true } => Self::Utf8Bom,
            TextEncoding::Utf16Le => Self::Utf16Le,
            TextEncoding::Utf16Be => Self::Utf16Be,
            TextEncoding::Legacy(_) => Self::Legacy,
        }
    }

    pub(super) fn resolve(self, source: &FileSource) -> Result<TextEncoding, PagedDocumentError> {
        match self {
            Self::Utf8 => Ok(TextEncoding::Utf8 { bom: false }),
            Self::Utf8Bom => Ok(TextEncoding::Utf8 { bom: true }),
            Self::Utf16Le => Ok(TextEncoding::Utf16Le),
            Self::Utf16Be => Ok(TextEncoding::Utf16Be),
            Self::Legacy => crate::probe_file(source.path(), crate::ProbeOptions::default())
                .map(|probe| probe.encoding),
        }
    }
}

impl From<PagedRecoverySelection> for SelectionRecord {
    fn from(selection: PagedRecoverySelection) -> Self {
        let range = selection.range();
        Self {
            start: range.start,
            end: range.end,
            reversed: selection.reversed(),
            anchor_affinity: Some(selection.anchor.affinity.into()),
            head_affinity: Some(selection.head.affinity.into()),
        }
    }
}

impl From<SelectionRecord> for PagedRecoverySelection {
    fn from(selection: SelectionRecord) -> Self {
        let mut restored =
            SourceSelection::from_range(selection.start..selection.end, selection.reversed);
        if let Some(affinity) = selection.anchor_affinity {
            restored.anchor.affinity = affinity.into();
        }
        if let Some(affinity) = selection.head_affinity {
            restored.head.affinity = affinity.into();
        }
        restored
    }
}

impl From<SourceAffinity> for RecoveryAffinity {
    fn from(affinity: SourceAffinity) -> Self {
        match affinity {
            SourceAffinity::Before => Self::Before,
            SourceAffinity::After => Self::After,
        }
    }
}

impl From<RecoveryAffinity> for SourceAffinity {
    fn from(affinity: RecoveryAffinity) -> Self {
        match affinity {
            RecoveryAffinity::Before => Self::Before,
            RecoveryAffinity::After => Self::After,
        }
    }
}
