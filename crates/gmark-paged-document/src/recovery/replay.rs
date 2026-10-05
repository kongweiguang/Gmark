// @author kongweiguang

use std::fs::{self, File};
use std::io::BufReader;
use std::path::{Path, PathBuf};

use gmark_recovery_codec::RecordKind;

use super::frame::{FrameRead, read_frame, verify_base};
use super::records::{BaseRecord, EditRecord, PendingReplace};
use super::{
    MAX_RECOVERY_CHUNKS_PER_REPLACE, PagedRecoveryBase, PagedRecoveryCommand, PagedRecoveryJournal,
    PagedRecoveryReadStatus, PagedRecoverySelection, RecoveredPagedDocument,
};
use crate::{FileSource, LineIndex, PagedDocumentError, PieceDocument, prepare_utf8_source};

pub fn replay_paged_recovery(
    journal_path: impl AsRef<Path>,
) -> Result<RecoveredPagedDocument, PagedDocumentError> {
    let journal_path = journal_path.as_ref();
    let (base, commands, selection, view_mode, read_status, next_transaction) =
        read_journal(journal_path)?;
    let source = FileSource::open(&base.path)?;
    verify_base(&source, &base)?;
    let prepared_source = prepare_utf8_source(source, base.encoding.clone())?;
    let index = LineIndex::build(prepared_source.source())?;
    let mut document = PieceDocument::open(prepared_source.source().clone(), index)?;
    for command in commands {
        match command {
            PagedRecoveryCommand::Replace {
                range,
                chunks,
                typing_group,
            } => match typing_group {
                Some(group_id) => document.replace_text_chunks_in_typing_group(
                    range,
                    chunks.iter().map(String::as_str),
                    group_id,
                )?,
                None => document.replace_text_chunks(range, chunks.iter().map(String::as_str))?,
            },
            PagedRecoveryCommand::Undo => {
                if !document.undo() {
                    return Err(PagedDocumentError::Recovery(
                        "recovery undo has no matching edit".to_owned(),
                    ));
                }
            }
            PagedRecoveryCommand::Redo => {
                if !document.redo() {
                    return Err(PagedDocumentError::Recovery(
                        "recovery redo has no matching undo".to_owned(),
                    ));
                }
            }
        }
    }
    Ok(RecoveredPagedDocument {
        base,
        journal: PagedRecoveryJournal {
            path: journal_path.to_path_buf(),
            next_transaction,
        },
        prepared_source,
        document,
        selection,
        view_mode,
        read_status,
    })
}

pub fn list_paged_recovery_journals(
    recovery_dir: impl AsRef<Path>,
) -> Result<Vec<PathBuf>, PagedDocumentError> {
    let recovery_dir = recovery_dir.as_ref();
    let entries = match fs::read_dir(recovery_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => {
            return Err(PagedDocumentError::Io {
                path: recovery_dir.to_path_buf(),
                source,
            });
        }
    };
    let mut paths = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "large-journal")
                && !recovery_journal_is_suppressed(path)
        })
        .collect::<Vec<_>>();
    paths.sort();
    Ok(paths)
}

/// A sidecar is written only after a saved journal could neither be deleted nor renamed.
/// Treat metadata failures as suppression too: replaying a possibly stale journal is unsafe.
fn recovery_journal_is_suppressed(journal_path: &Path) -> bool {
    let mut marker_path = journal_path.to_path_buf();
    marker_path.set_extension("large-journal.suppressed");
    match fs::metadata(&marker_path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(_) => true,
        Ok(_) => {
            // Startup cleanup must never reveal stale recovery: only delete the marker after the
            // original journal has been removed (or is already absent).
            match fs::remove_file(journal_path) {
                Ok(()) => {
                    let _ = fs::remove_file(marker_path);
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    let _ = fs::remove_file(marker_path);
                }
                Err(_) => {}
            }
            true
        }
    }
}

pub fn inspect_paged_recovery_base(
    journal_path: impl AsRef<Path>,
) -> Result<PagedRecoveryBase, PagedDocumentError> {
    let journal_path = journal_path.as_ref();
    let file = File::open(journal_path).map_err(|source| PagedDocumentError::Io {
        path: journal_path.to_path_buf(),
        source,
    })?;
    let mut reader = BufReader::new(file);
    let FrameRead::Frame(RecordKind::Base, payload) = read_frame(&mut reader)? else {
        return Err(PagedDocumentError::Recovery(
            "large recovery journal has no valid base frame".to_owned(),
        ));
    };
    let record: BaseRecord = serde_json::from_slice(&payload)
        .map_err(|error| PagedDocumentError::Recovery(error.to_string()))?;
    let source = FileSource::open(&record.path)?;
    Ok(PagedRecoveryBase {
        path: record.path,
        len: record.len,
        modified_nanos: record.modified_nanos,
        sampled_hash: record.sampled_hash,
        encoding: record.encoding.resolve(&source)?,
    })
}

pub fn paged_recovery_has_edits(
    journal_path: impl AsRef<Path>,
) -> Result<bool, PagedDocumentError> {
    let journal_path = journal_path.as_ref();
    let file = File::open(journal_path).map_err(|source| PagedDocumentError::Io {
        path: journal_path.to_path_buf(),
        source,
    })?;
    let mut reader = BufReader::new(file);
    if !matches!(
        read_frame(&mut reader)?,
        FrameRead::Frame(RecordKind::Base, _)
    ) {
        return Err(PagedDocumentError::Recovery(
            "large recovery journal has no valid base frame".to_owned(),
        ));
    }
    Ok(matches!(
        read_frame(&mut reader)?,
        FrameRead::Frame(RecordKind::Edit, _)
    ))
}

type JournalRead = (
    PagedRecoveryBase,
    Vec<PagedRecoveryCommand>,
    Option<PagedRecoverySelection>,
    String,
    PagedRecoveryReadStatus,
    u64,
);

fn read_journal(path: &Path) -> Result<JournalRead, PagedDocumentError> {
    let file = File::open(path).map_err(|source| PagedDocumentError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mut reader = BufReader::new(file);
    let mut base = None;
    let mut commands = Vec::new();
    let mut pending: Option<PendingReplace> = None;
    let mut selection = None;
    let mut view_mode = "source".to_owned();
    let mut status = PagedRecoveryReadStatus::Complete;
    let mut max_transaction = 0u64;
    loop {
        let (kind, payload) = match read_frame(&mut reader)? {
            FrameRead::End => break,
            FrameRead::Truncated => {
                status = PagedRecoveryReadStatus::TruncatedTail;
                break;
            }
            FrameRead::Frame(kind, payload) => (kind, payload),
        };
        match kind {
            RecordKind::Base => {
                if base.is_some() || !commands.is_empty() || pending.is_some() {
                    return Err(PagedDocumentError::Recovery(
                        "recovery journal contains an out-of-order base".to_owned(),
                    ));
                }
                let record: BaseRecord = serde_json::from_slice(&payload)
                    .map_err(|error| PagedDocumentError::Recovery(error.to_string()))?;
                let source = FileSource::open(&record.path)?;
                base = Some(PagedRecoveryBase {
                    path: record.path,
                    len: record.len,
                    modified_nanos: record.modified_nanos,
                    sampled_hash: record.sampled_hash,
                    encoding: record.encoding.resolve(&source)?,
                });
            }
            RecordKind::Edit => {
                if base.is_none() {
                    return Err(PagedDocumentError::Recovery(
                        "recovery edit appears before its base".to_owned(),
                    ));
                }
                let record: EditRecord = match serde_json::from_slice(&payload) {
                    Ok(record) => record,
                    Err(_) => {
                        status = PagedRecoveryReadStatus::TruncatedTail;
                        break;
                    }
                };
                if !consume_edit(
                    record,
                    &mut pending,
                    &mut commands,
                    &mut selection,
                    &mut view_mode,
                    &mut max_transaction,
                )? {
                    status = PagedRecoveryReadStatus::TruncatedTail;
                    break;
                }
            }
        }
    }
    if pending.is_some() {
        status = PagedRecoveryReadStatus::TruncatedTail;
    }
    let base = base.ok_or_else(|| {
        PagedDocumentError::Recovery("recovery journal has no valid base".to_owned())
    })?;
    Ok((
        base,
        commands,
        selection,
        view_mode,
        status,
        max_transaction.wrapping_add(1).max(1),
    ))
}

fn consume_edit(
    record: EditRecord,
    pending: &mut Option<PendingReplace>,
    commands: &mut Vec<PagedRecoveryCommand>,
    selection: &mut Option<PagedRecoverySelection>,
    view_mode: &mut String,
    max_transaction: &mut u64,
) -> Result<bool, PagedDocumentError> {
    match record {
        EditRecord::ReplaceChunk {
            transaction,
            start,
            end,
            chunk_index,
            chunk_count,
            text,
            typing_group,
            selection: next_selection,
            view_mode: next_view_mode,
        } => {
            *max_transaction = (*max_transaction).max(transaction);
            if chunk_count > MAX_RECOVERY_CHUNKS_PER_REPLACE {
                return Err(PagedDocumentError::Recovery(
                    "recovery replacement exceeds the chunk limit".to_owned(),
                ));
            }
            if start > end || chunk_count == 0 || chunk_index >= chunk_count {
                return Ok(false);
            }
            if chunk_index == 0 {
                if pending.is_some() {
                    return Ok(false);
                }
                let mut chunks = Vec::new();
                chunks
                    .try_reserve_exact(chunk_count as usize)
                    .map_err(|_| {
                        PagedDocumentError::Recovery(
                            "recovery replacement chunk allocation failed".to_owned(),
                        )
                    })?;
                *pending = Some(PendingReplace {
                    transaction,
                    range: start..end,
                    chunk_count,
                    chunks,
                    typing_group,
                    selection: next_selection.map(Into::into),
                    view_mode: next_view_mode,
                });
            }
            let Some(current) = pending.as_mut() else {
                return Ok(false);
            };
            if current.transaction != transaction
                || current.range != (start..end)
                || current.chunk_count != chunk_count
                || current.typing_group != typing_group
                || chunk_index as usize != current.chunks.len()
            {
                return Ok(false);
            }
            current.chunks.push(text);
            if current.chunks.len() == current.chunk_count as usize {
                let completed = pending
                    .take()
                    .expect("checked pending recovery transaction");
                *selection = completed.selection;
                *view_mode = completed.view_mode;
                commands.push(PagedRecoveryCommand::Replace {
                    range: completed.range,
                    chunks: completed.chunks,
                    typing_group: completed.typing_group,
                });
            }
            Ok(true)
        }
        EditRecord::Undo {
            selection: next_selection,
            view_mode: next_view_mode,
        } => {
            if pending.is_some() {
                return Ok(false);
            }
            *selection = next_selection.map(Into::into);
            *view_mode = next_view_mode;
            commands.push(PagedRecoveryCommand::Undo);
            Ok(true)
        }
        EditRecord::Redo {
            selection: next_selection,
            view_mode: next_view_mode,
        } => {
            if pending.is_some() {
                return Ok(false);
            }
            *selection = next_selection.map(Into::into);
            *view_mode = next_view_mode;
            commands.push(PagedRecoveryCommand::Redo);
            Ok(true)
        }
    }
}
