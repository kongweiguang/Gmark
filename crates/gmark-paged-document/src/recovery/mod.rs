// @author kongweiguang

//! Content-free base identity plus CRC-framed edit recovery for disk-backed documents.

use std::ops::Range;
use std::path::PathBuf;

use gmark_document_core::{SourceSelection, TextEncoding, TypingGroupId};

use crate::{PieceDocument, PreparedUtf8Source};

mod frame;
mod journal;
mod records;
mod replay;

pub use replay::{
    inspect_paged_recovery_base, list_paged_recovery_journals, paged_recovery_has_edits,
    replay_paged_recovery,
};

pub(super) const RECOVERY_CHUNK_BYTES: usize = 16 * 1024 * 1024;
pub(super) const MAX_RECOVERY_CHUNKS_PER_REPLACE: u32 = 4_096;
pub(super) const SAMPLE_BYTES: u64 = 64 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PagedRecoveryReadStatus {
    Complete,
    TruncatedTail,
}

/// Recovery 与实时 Source 共用同一个 anchor/affinity 真值，不能在落盘时退化为 Range。
pub type PagedRecoverySelection = SourceSelection;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PagedRecoveryBase {
    pub path: PathBuf,
    pub len: u64,
    pub modified_nanos: Option<u128>,
    pub sampled_hash: u32,
    pub encoding: TextEncoding,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PagedRecoveryCommand {
    Replace {
        range: Range<u64>,
        chunks: Vec<String>,
        typing_group: Option<TypingGroupId>,
    },
    Undo,
    Redo,
}

pub struct RecoveredPagedDocument {
    pub base: PagedRecoveryBase,
    pub journal: PagedRecoveryJournal,
    pub prepared_source: PreparedUtf8Source,
    pub document: PieceDocument,
    pub selection: Option<PagedRecoverySelection>,
    pub view_mode: String,
    pub read_status: PagedRecoveryReadStatus,
}

pub struct PagedRecoveryJournal {
    path: PathBuf,
    next_transaction: u64,
}
