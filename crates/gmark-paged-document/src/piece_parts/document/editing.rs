// @author kongweiguang

use std::io::Read;
use std::ops::Range;
use std::sync::Arc;

use gmark_document_core::TypingGroupId;

use super::super::*;

impl PieceDocument {
    pub fn accept_external_append(
        &mut self,
        source: FileSource,
        index: LineIndex,
    ) -> Result<(), PagedDocumentError> {
        if !self.undo.is_empty() || !self.redo.is_empty() || self.pieces.piece_count() > 1 {
            return Err(PagedDocumentError::SourceChanged);
        }
        let identity = source.identity()?;
        if identity.len < self.base_identity.len
            || identity.os_file_id != self.base_identity.os_file_id
            || source.sampled_prefix_hash(self.base_identity.len)? != self.base_sample
        {
            return Err(PagedDocumentError::SourceChanged);
        }
        self.len = identity.len;
        self.base_identity = identity;
        self.base_sample = source.sampled_prefix_hash(self.len)?;
        self.base_index = index.clone();
        self.source = Some(source);
        self.pieces = PieceTree::from_iter((self.len > 0).then_some(Piece {
            source: PieceSource::Base,
            range: 0..self.len,
            newlines: index.newline_count(),
        }));
        Ok(())
    }

    pub fn replace_text(
        &mut self,
        range: Range<u64>,
        replacement: &str,
    ) -> Result<(), PagedDocumentError> {
        self.replace_text_chunks(range, std::iter::once(replacement))
    }

    /// 以一个撤销事务写入多个 UTF-8 块，恢复超大粘贴时无需先拼成同等大小的临时字符串。
    pub fn replace_text_chunks<'a>(
        &mut self,
        range: Range<u64>,
        chunks: impl IntoIterator<Item = &'a str>,
    ) -> Result<(), PagedDocumentError> {
        self.replace_text_chunks_with_group(range, chunks, None)
    }

    /// 以流式文本块恢复已持久化的 typing 组，同时保留 PieceTree 的组首撤销根。
    pub fn replace_text_chunks_in_typing_group<'a>(
        &mut self,
        range: Range<u64>,
        chunks: impl IntoIterator<Item = &'a str>,
        group_id: TypingGroupId,
    ) -> Result<(), PagedDocumentError> {
        self.replace_text_chunks_with_group(range, chunks, Some(group_id))
    }

    /// 普通替换与恢复输入组共用范围校验和 PieceTree 安装，只在历史根处区分分组。
    fn replace_text_chunks_with_group<'a>(
        &mut self,
        range: Range<u64>,
        chunks: impl IntoIterator<Item = &'a str>,
        group_id: Option<TypingGroupId>,
    ) -> Result<(), PagedDocumentError> {
        if range.start > range.end || range.end > self.len {
            return Err(PagedDocumentError::InvalidRange {
                start: range.start,
                end: range.end,
                len: self.len,
            });
        }
        if !self.is_char_boundary(range.start)? || !self.is_char_boundary(range.end)? {
            return Err(PagedDocumentError::InvalidUtf8Boundary);
        }
        if group_id.is_none() {
            self.break_typing_group();
        }
        let mut replacement_len = 0u64;
        let mut can_continue = true;
        let mut replacement_pieces = Vec::new();
        for chunk in chunks {
            if chunk.is_empty() {
                continue;
            }
            if chunk.contains('\r') || chunk.contains('\n') {
                can_continue = false;
            }
            replacement_len = replacement_len
                .checked_add(chunk.len() as u64)
                .ok_or(PagedDocumentError::RangeTooLarge)?;
            replacement_pieces.push(Piece {
                source: PieceSource::Add,
                range: self.additions.append(chunk.as_bytes())?,
                newlines: chunk.bytes().filter(|byte| *byte == b'\n').count() as u64,
            });
        }

        let mut cursor = PieceCursor::new(self, 0);
        let mut next = cursor.slice(range.start)?;
        cursor.seek_forward(range.end);
        next.append(PieceTree::from_iter(replacement_pieces));
        next.append(cursor.slice(self.len)?);
        drop(cursor);
        let typing = match group_id {
            Some(group_id) => Some(TypingHistorySpan {
                group_id,
                inserted_end: range
                    .start
                    .checked_add(replacement_len)
                    .ok_or(PagedDocumentError::RangeTooLarge)?,
                can_continue: can_continue && replacement_len > 0,
            }),
            None => None,
        };
        let continuing = group_id.and_then(|group_id| {
            self.continuing_typing_span_for_range(
                group_id,
                &range,
                replacement_len,
                can_continue && replacement_len > 0,
            )
        });
        if let Some(continuing) = continuing {
            if let Some(entry) = self.undo.last_mut() {
                entry.typing = Some(continuing);
            }
        } else {
            self.record_undo_root(self.pieces.clone(), self.len, typing);
        }
        self.redo.clear();
        self.pieces = next;
        self.len = self.len - (range.end - range.start) + replacement_len;
        self.active_typing_group = typing
            .filter(|span| span.can_continue)
            .map(|span| span.group_id);
        Ok(())
    }

    /// 从读取器有界流式安装 UTF-8 替换，并把完整流视为单个撤销事务。
    pub fn replace_text_reader(
        &mut self,
        range: Range<u64>,
        mut reader: impl Read,
    ) -> Result<(), PagedDocumentError> {
        if range.start > range.end || range.end > self.len {
            return Err(PagedDocumentError::InvalidRange {
                start: range.start,
                end: range.end,
                len: self.len,
            });
        }
        if !self.is_char_boundary(range.start)? || !self.is_char_boundary(range.end)? {
            return Err(PagedDocumentError::InvalidUtf8Boundary);
        }
        self.break_typing_group();

        const CHUNK_BYTES: usize = 1024 * 1024;
        let mut pending = Vec::with_capacity(CHUNK_BYTES + 4);
        let mut scratch = vec![0u8; CHUNK_BYTES];
        let mut replacement_len = 0u64;
        let mut replacement_pieces = Vec::new();
        loop {
            let read = reader
                .read(&mut scratch)
                .map_err(|source| PagedDocumentError::Io {
                    path: std::env::temp_dir(),
                    source,
                })?;
            pending.extend_from_slice(&scratch[..read]);
            let complete = match std::str::from_utf8(&pending) {
                Ok(_) => pending.len(),
                Err(error) if error.error_len().is_none() => error.valid_up_to(),
                Err(_) => return Err(PagedDocumentError::Binary),
            };
            if complete > 0 {
                let bytes = &pending[..complete];
                replacement_len = replacement_len
                    .checked_add(bytes.len() as u64)
                    .ok_or(PagedDocumentError::RangeTooLarge)?;
                replacement_pieces.push(Piece {
                    source: PieceSource::Add,
                    range: self.additions.append(bytes)?,
                    newlines: bytes.iter().filter(|byte| **byte == b'\n').count() as u64,
                });
                pending.drain(..complete);
            }
            if read == 0 {
                break;
            }
        }
        if !pending.is_empty() {
            return Err(PagedDocumentError::Binary);
        }

        let mut cursor = PieceCursor::new(self, 0);
        let mut next = cursor.slice(range.start)?;
        cursor.seek_forward(range.end);
        next.append(PieceTree::from_iter(replacement_pieces));
        next.append(cursor.slice(self.len)?);
        drop(cursor);
        self.record_undo_root(self.pieces.clone(), self.len, None);
        self.redo.clear();
        self.pieces = next;
        self.len = self.len - (range.end - range.start) + replacement_len;
        Ok(())
    }

    /// 将基于同一 Source revision 的多个不相交编辑作为独立撤销事务提交。
    /// 倒序应用可保持所有 range 都在原始字节坐标系中，并关闭旧输入组。
    pub fn replace_text_batch(
        &mut self,
        edits: &[(Range<u64>, Arc<str>)],
    ) -> Result<(), PagedDocumentError> {
        self.replace_text_batch_with_group(edits, None)
    }

    /// 为 Controller 已确认连续的 Source 输入复用组首 PieceTree 根。
    pub fn replace_text_batch_in_typing_group(
        &mut self,
        edits: &[(Range<u64>, Arc<str>)],
        group_id: TypingGroupId,
    ) -> Result<(), PagedDocumentError> {
        self.replace_text_batch_with_group(edits, Some(group_id))
    }

    /// 内部逐段替换产生的临时历史与断组均须复原，再按整个公开事务记录唯一撤销根。
    fn replace_text_batch_with_group(
        &mut self,
        edits: &[(Range<u64>, Arc<str>)],
        group_id: Option<TypingGroupId>,
    ) -> Result<(), PagedDocumentError> {
        if edits.is_empty() {
            self.break_typing_group();
            return Ok(());
        }
        let mut ordered = edits.to_vec();
        ordered.sort_by_key(|(range, _)| (range.start, range.end));
        for pair in ordered.windows(2) {
            let previous = &pair[0].0;
            let next = &pair[1].0;
            if previous.end > next.start
                || (previous.is_empty() && next.is_empty() && previous.start == next.start)
            {
                return Err(PagedDocumentError::InvalidTransaction(
                    "derived edit ranges overlap or contain ambiguous inserts".into(),
                ));
            }
        }
        for (range, _) in &ordered {
            if range.start > range.end || range.end > self.len {
                return Err(PagedDocumentError::InvalidRange {
                    start: range.start,
                    end: range.end,
                    len: self.len,
                });
            }
            if !self.is_char_boundary(range.start)? || !self.is_char_boundary(range.end)? {
                return Err(PagedDocumentError::InvalidUtf8Boundary);
            }
        }

        let original_pieces = self.pieces.clone();
        let original_len = self.len;
        let original_undo = self.undo.clone();
        let original_redo = self.redo.clone();
        let original_typing_group = self.active_typing_group;
        for (range, replacement) in ordered.iter().rev() {
            if let Err(error) = self.replace_text(range.clone(), replacement) {
                self.pieces = original_pieces;
                self.len = original_len;
                self.undo = original_undo;
                self.redo = original_redo;
                self.active_typing_group = original_typing_group;
                return Err(error);
            }
        }
        self.undo = original_undo;
        self.redo.clear();
        self.active_typing_group = original_typing_group;
        if let Some(span) = group_id.and_then(|id| self.continuing_typing_span(id, &ordered)) {
            if let Some(entry) = self.undo.last_mut() {
                entry.typing = Some(span);
            }
        } else {
            let typing = group_id.and_then(|id| typing_span_for_batch(id, &ordered));
            self.record_undo_root(original_pieces, original_len, typing);
        }
        self.active_typing_group = group_id;
        Ok(())
    }

    /// 撤销结束合并窗口，防止同一旧 ID 在撤销后重新接回该历史条目。
    pub fn undo(&mut self) -> bool {
        self.break_typing_group();
        let Some(entry) = self.undo.pop() else {
            return false;
        };
        self.redo.push(PieceHistoryEntry {
            pieces: self.pieces.clone(),
            len: self.len,
            typing: entry.typing,
        });
        self.pieces = entry.pieces;
        self.len = entry.len;
        true
    }

    /// 重做保留撤销根元数据，但不让后续输入继续加入刚重做的历史组。
    pub fn redo(&mut self) -> bool {
        self.break_typing_group();
        let Some(entry) = self.redo.pop() else {
            return false;
        };
        self.record_undo_root(self.pieces.clone(), self.len, entry.typing);
        self.pieces = entry.pieces;
        self.len = entry.len;
        true
    }

    /// 单独打断分组而不改动 PieceTree 历史，供共享 Controller 处理视图边界。
    pub(crate) fn break_typing_group(&mut self) {
        self.active_typing_group = None;
    }

    fn continuing_typing_span(
        &self,
        group_id: TypingGroupId,
        edits: &[(Range<u64>, Arc<str>)],
    ) -> Option<TypingHistorySpan> {
        let [(range, replacement)] = edits else {
            return None;
        };
        self.continuing_typing_span_for_range(
            group_id,
            range,
            replacement.len() as u64,
            !replacement.is_empty() && !replacement.contains('\r') && !replacement.contains('\n'),
        )
    }

    /// 只扩展同 ID 的单行相邻尾插，保持 replay 与实时事务使用相同的坐标门槛。
    fn continuing_typing_span_for_range(
        &self,
        group_id: TypingGroupId,
        range: &Range<u64>,
        replacement_len: u64,
        can_continue: bool,
    ) -> Option<TypingHistorySpan> {
        if self.active_typing_group != Some(group_id) || !can_continue || !range.is_empty() {
            return None;
        }
        let previous = self.undo.last()?.typing?;
        if previous.group_id != group_id
            || !previous.can_continue
            || range.start != previous.inserted_end
        {
            return None;
        }
        Some(TypingHistorySpan {
            group_id,
            inserted_end: range.start.checked_add(replacement_len)?,
            can_continue: true,
        })
    }

    fn record_undo_root(&mut self, pieces: PieceTree, len: u64, typing: Option<TypingHistorySpan>) {
        if self.undo.len() == DEFAULT_HISTORY_LIMIT {
            self.undo.remove(0);
        }
        self.undo.push(PieceHistoryEntry {
            pieces,
            len,
            typing,
        });
    }
}

fn typing_span_for_batch(
    group_id: TypingGroupId,
    edits: &[(Range<u64>, Arc<str>)],
) -> Option<TypingHistorySpan> {
    let [(range, replacement)] = edits else {
        return None;
    };
    Some(TypingHistorySpan {
        group_id,
        inserted_end: range.start.checked_add(replacement.len() as u64)?,
        can_continue: !replacement.is_empty()
            && !replacement.contains('\r')
            && !replacement.contains('\n'),
    })
}
