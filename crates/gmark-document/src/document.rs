// @author kongweiguang

use std::{collections::VecDeque, fmt, ops::Range, sync::Arc};

use gmark_document_core::TypingGroupId;
use thiserror::Error;
use zed_rope::Rope;

use crate::source_format::{FormatOperation, FormatPatch, SourceFormat};
use crate::{LineEnding, SourceFormatSnapshot, SourceFormatSummary};

/// 单调递增的文档版本号。
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Revision(u64);

impl Revision {
    /// 初始文档版本。
    pub const INITIAL: Self = Self(0);

    /// 从持久值恢复版本号。
    pub const fn from_u64(value: u64) -> Self {
        Self(value)
    }

    /// 返回可序列化的版本值。
    pub const fn get(self) -> u64 {
        self.0
    }

    fn next(self) -> Result<Self, DocumentError> {
        self.0
            .checked_add(1)
            .map(Self)
            .ok_or(DocumentError::RevisionExhausted)
    }
}

/// 基于 transaction 起始版本的 UTF-8 字节范围替换。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TextEdit {
    range: Range<usize>,
    replacement: Arc<str>,
}

impl TextEdit {
    /// 创建一个字节范围替换。
    pub fn new(range: Range<usize>, replacement: impl Into<Arc<str>>) -> Self {
        Self {
            range,
            replacement: replacement.into(),
        }
    }

    /// 返回 transaction 起始版本中的替换范围。
    pub fn range(&self) -> &Range<usize> {
        &self.range
    }

    /// 返回替换文本。
    pub fn replacement(&self) -> &str {
        &self.replacement
    }
}

/// 一组针对同一基础版本、按起始位置升序排列的文本编辑。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Transaction {
    base_revision: Revision,
    edits: Vec<TextEdit>,
}

impl Transaction {
    /// 创建 transaction。编辑范围在应用时统一校验，避免构造后文档已变化。
    pub fn new(base_revision: Revision, edits: Vec<TextEdit>) -> Self {
        Self {
            base_revision,
            edits,
        }
    }

    /// 返回 transaction 基础版本。
    pub fn base_revision(&self) -> Revision {
        self.base_revision
    }

    /// 返回有序编辑列表。
    pub fn edits(&self) -> &[TextEdit] {
        &self.edits
    }
}

/// 不可变文档快照，可安全保留给后台解析和导出任务。
#[derive(Clone)]
pub struct DocumentSnapshot {
    revision: Revision,
    source: Rope,
}

impl fmt::Debug for DocumentSnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DocumentSnapshot")
            .field("revision", &self.revision)
            .field("len", &self.source.len())
            .finish()
    }
}

impl DocumentSnapshot {
    /// 返回快照版本。
    pub fn revision(&self) -> Revision {
        self.revision
    }

    /// 返回 UTF-8 字节长度。
    pub fn len(&self) -> usize {
        self.source.len()
    }

    /// 判断快照是否为空。
    pub fn is_empty(&self) -> bool {
        self.source.is_empty()
    }

    /// 复制指定范围文本。
    pub fn text_for_range(&self, range: Range<usize>) -> Result<String, DocumentError> {
        validate_range(&self.source, &range)?;
        Ok(self.source.slice(range).to_string())
    }

    /// 返回完整源文本。
    pub fn text(&self) -> String {
        self.source.to_string()
    }

    /// 在不物化第二份完整正文的前提下比较规范化源码内容。
    pub fn matches_text(&self, text: &str) -> bool {
        self.source.len() == text.len()
            && self.source.chunks().flat_map(str::bytes).eq(text.bytes())
    }

    /// Materializes this immutable snapshot and restores its persisted byte format.
    ///
    /// Intended for background save workers so Rope traversal and line-ending
    /// expansion never need to run on the UI thread.
    pub fn formatted_text_and_bytes(
        &self,
        format: SourceFormatSnapshot,
    ) -> Option<(String, Vec<u8>)> {
        let text = self.source.to_string();
        let format = SourceFormat::from_normalized(&text, format)?;
        let bytes = format.serialized_bytes(&text);
        Some((text, bytes))
    }
}

impl gmark_document_core::DocumentSnapshot for DocumentSnapshot {
    fn revision(&self) -> gmark_document_core::DocumentRevision {
        gmark_document_core::DocumentRevision(self.revision.get())
    }

    fn len(&self) -> u64 {
        u64::try_from(self.source.len()).unwrap_or(u64::MAX)
    }

    fn read_range(&self, range: Range<u64>) -> Result<Vec<u8>, gmark_document_core::SnapshotError> {
        let len = u64::try_from(DocumentSnapshot::len(self)).unwrap_or(u64::MAX);
        if range.start > range.end || range.end > len {
            return Err(gmark_document_core::SnapshotError::InvalidRange {
                start: range.start,
                end: range.end,
                len,
            });
        }
        let start = usize::try_from(range.start)
            .map_err(|_| gmark_document_core::SnapshotError::RangeTooLarge)?;
        let end = usize::try_from(range.end)
            .map_err(|_| gmark_document_core::SnapshotError::RangeTooLarge)?;
        self.text_for_range(start..end)
            .map(String::into_bytes)
            .map_err(|error| gmark_document_core::SnapshotError::Read(error.to_string()))
    }
}

#[derive(Clone)]
struct HistoryEntry {
    forward: Vec<TextEdit>,
    inverse: Vec<TextEdit>,
    format_patch: FormatPatch,
    typing: Option<TypingHistorySpan>,
}

#[derive(Clone, Copy)]
struct TypingHistorySpan {
    group_id: TypingGroupId,
    inserted_end: usize,
    can_continue: bool,
}

/// 源码优先文档，负责 revision、transaction 和 undo/redo 历史。
#[derive(Clone)]
pub struct SourceDocument {
    revision: Revision,
    source: Rope,
    source_format: SourceFormat,
    undo: VecDeque<HistoryEntry>,
    redo: Vec<HistoryEntry>,
    active_typing_group: Option<TypingGroupId>,
    history_limit: usize,
}

impl SourceDocument {
    /// 默认保留的 transaction 组数。
    pub const DEFAULT_HISTORY_LIMIT: usize = 200;

    /// 从 UTF-8 文本创建文档。
    pub fn new(text: &str) -> Self {
        Self::with_history_limit(text, Self::DEFAULT_HISTORY_LIMIT)
    }

    /// 从 UTF-8 文本创建文档，并指定最大撤销组数。
    pub fn with_history_limit(text: &str, history_limit: usize) -> Self {
        let (text, source_format) = SourceFormat::parse(text);
        Self {
            revision: Revision::INITIAL,
            source: Rope::from(text),
            source_format,
            undo: VecDeque::new(),
            redo: Vec::new(),
            active_typing_group: None,
            history_limit,
        }
    }

    /// 从已规范化为 LF 的源码和格式快照恢复文档。
    pub fn from_normalized(
        text: &str,
        format: SourceFormatSnapshot,
        history_limit: usize,
    ) -> Option<Self> {
        let source_format = SourceFormat::from_normalized(text, format)?;
        Some(Self {
            revision: Revision::INITIAL,
            source: Rope::from(text),
            source_format,
            undo: VecDeque::new(),
            redo: Vec::new(),
            active_typing_group: None,
            history_limit,
        })
    }

    /// 返回当前版本。
    pub fn revision(&self) -> Revision {
        self.revision
    }

    /// 安装外部协调版本时同时清空历史与输入组，禁止旧正文的分组跨会话边界。
    pub fn set_revision(&mut self, revision: Revision) {
        self.revision = revision;
        self.undo.clear();
        self.redo.clear();
        self.break_typing_group();
    }

    /// 元数据 revision 保留正文撤销项，但关闭输入组以免编码等命令夹入连续按键。
    pub fn advance_revision(&mut self) -> Result<Revision, DocumentError> {
        self.break_typing_group();
        let next_revision = self.revision.next()?;
        self.revision = next_revision;
        Ok(next_revision)
    }

    /// 返回当前不可变快照。
    pub fn snapshot(&self) -> DocumentSnapshot {
        DocumentSnapshot {
            revision: self.revision,
            source: self.source.clone(),
        }
    }

    /// 比较当前 Rope 与不可变快照的正文；revision 不参与 pristine 语义。
    pub fn content_matches_snapshot(&self, snapshot: &DocumentSnapshot) -> bool {
        self.source.len() == snapshot.source.len()
            && self
                .source
                .chunks()
                .flat_map(str::bytes)
                .eq(snapshot.source.chunks().flat_map(str::bytes))
    }

    /// 比较当前 Rope 与调用方已有文本，避免为了 dirty 检查复制整篇正文。
    pub fn content_matches_text(&self, text: &str) -> bool {
        self.source.len() == text.len()
            && self.source.chunks().flat_map(str::bytes).eq(text.bytes())
    }

    /// 返回完整源文本。
    pub fn text(&self) -> String {
        self.source.to_string()
    }

    /// 返回带原 BOM 和逐行换行样式的实际保存字节。
    pub fn serialized_bytes(&self) -> Vec<u8> {
        self.source_format
            .serialized_bytes(&self.source.to_string())
    }

    /// 使用调用方已物化的当前规范化源码生成保存字节，避免长文档重复复制 Rope。
    pub fn serialized_bytes_for_text(&self, normalized: &str) -> Option<Vec<u8>> {
        if normalized.len() != self.source.len()
            || normalized.bytes().filter(|byte| *byte == b'\n').count()
                != self.source_format.ending_count()
        {
            return None;
        }
        Some(self.source_format.serialized_bytes(normalized))
    }

    /// 返回恢复日志可持久化的源码格式快照。
    pub fn source_format(&self) -> SourceFormatSnapshot {
        self.source_format.snapshot()
    }

    /// 返回不复制逐行换行映射的格式摘要，适用于状态栏等每帧读取路径。
    pub fn source_format_summary(&self) -> SourceFormatSummary {
        self.source_format.summary()
    }

    /// 用恢复日志或编辑器历史中的格式快照替换当前格式。
    pub fn restore_source_format(&mut self, format: SourceFormatSnapshot) -> bool {
        let source = self.source.to_string();
        let Some(format) = SourceFormat::from_normalized(&source, format) else {
            return false;
        };
        self.source_format = format;
        true
    }

    /// 恢复格式并作为可撤销 revision 提交。恢复日志回放仍使用
    /// [`restore_source_format`] 的无 revision 入口，避免打开阶段改变正文代次。
    pub fn restore_source_format_transaction(
        &mut self,
        format: SourceFormatSnapshot,
    ) -> Result<Option<DocumentSnapshot>, DocumentError> {
        self.break_typing_group();
        let source = self.source.to_string();
        let Some(next_format) = SourceFormat::from_normalized(&source, format) else {
            return Ok(None);
        };
        let current = self.source_format.snapshot();
        let next_snapshot = next_format.snapshot();
        if current == next_snapshot {
            return Ok(None);
        }
        let next_revision = self.revision.next()?;
        let format_patch = FormatPatch {
            operations: vec![FormatOperation {
                newline_index: 0,
                removed: current.endings,
                inserted: next_snapshot.endings.clone(),
            }],
            dominant_override: Some((current.dominant, next_snapshot.dominant)),
        };
        self.source_format = next_format;
        self.revision = next_revision;
        self.record_undo(HistoryEntry {
            forward: Vec::new(),
            inverse: Vec::new(),
            format_patch,
            typing: None,
        });
        self.redo.clear();
        Ok(Some(self.snapshot()))
    }

    /// 显式把所有换行规范化为目标样式，并作为可撤销的格式事务分配新 revision。
    pub fn normalize_line_endings(
        &mut self,
        ending: LineEnding,
    ) -> Result<Option<DocumentSnapshot>, DocumentError> {
        self.break_typing_group();
        let Some(format_patch) = self.source_format.normalization_patch(ending) else {
            return Ok(None);
        };
        let next_revision = self.revision.next()?;
        self.source_format.apply(&format_patch);
        self.revision = next_revision;
        self.record_undo(HistoryEntry {
            forward: Vec::new(),
            inverse: Vec::new(),
            format_patch,
            typing: None,
        });
        self.redo.clear();
        Ok(Some(self.snapshot()))
    }

    /// 返回当前 UTF-8 字节长度，不物化 Rope 文本。
    pub fn len(&self) -> usize {
        self.source.len()
    }

    /// 判断当前文档是否为空。
    pub fn is_empty(&self) -> bool {
        self.source.is_empty()
    }

    /// 判断当前是否可以撤销。
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    /// 判断当前是否可以重做。
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// 普通 transaction 总是结束 typing group；坐标校验和原子应用契约保持不变。
    pub fn apply_transaction(
        &mut self,
        transaction: Transaction,
    ) -> Result<DocumentSnapshot, DocumentError> {
        self.apply_transaction_with_group(transaction, None)
    }

    /// 仅把 Controller 明确标记的连续输入组入同一撤销项，普通调用仍保持独立历史。
    pub fn apply_typing_transaction(
        &mut self,
        transaction: Transaction,
        group_id: TypingGroupId,
    ) -> Result<DocumentSnapshot, DocumentError> {
        self.apply_transaction_with_group(transaction, Some(group_id))
    }

    /// 组内仅合并单一替换后的相邻插入，避免通用事务组合改变格式与坐标语义。
    fn apply_transaction_with_group(
        &mut self,
        transaction: Transaction,
        group_id: Option<TypingGroupId>,
    ) -> Result<DocumentSnapshot, DocumentError> {
        if transaction.base_revision != self.revision {
            self.break_typing_group();
            return Err(DocumentError::StaleRevision {
                expected: self.revision,
                actual: transaction.base_revision,
            });
        }

        if let Err(error) = validate_edits(&self.source, &transaction.edits) {
            self.break_typing_group();
            return Err(error);
        }
        if transaction.edits.is_empty() {
            self.break_typing_group();
            return Ok(self.snapshot());
        }

        let next_revision = match self.revision.next() {
            Ok(revision) => revision,
            Err(error) => {
                self.break_typing_group();
                return Err(error);
            }
        };
        let inverse = match build_inverse_edits(&self.source, &transaction.edits) {
            Ok(inverse) => inverse,
            Err(error) => {
                self.break_typing_group();
                return Err(error);
            }
        };
        let format_patch = self
            .source_format
            .build_patch(&self.source, &transaction.edits);
        apply_edits(&mut self.source, &transaction.edits);
        self.source_format.apply(&format_patch);
        self.revision = next_revision;
        let typing =
            group_id.and_then(|group_id| typing_span_for_edits(group_id, &transaction.edits));
        let entry = HistoryEntry {
            forward: transaction.edits,
            inverse,
            format_patch,
            typing,
        };
        if let Some(group_id) = group_id {
            let merged = if self.active_typing_group == Some(group_id) {
                self.undo
                    .back()
                    .and_then(|previous| merge_adjacent_typing_entry(previous, &entry, group_id))
            } else {
                None
            };
            if let Some(merged) = merged {
                if let Some(previous) = self.undo.back_mut() {
                    *previous = merged;
                }
            } else {
                self.record_undo(entry);
            }
            self.active_typing_group = typing
                .filter(|span| span.can_continue)
                .map(|span| span.group_id);
        } else {
            self.break_typing_group();
            self.record_undo(entry);
        }
        self.redo.clear();
        Ok(self.snapshot())
    }

    /// 撤销前关闭输入组，确保后续复用旧 ID 不能改写已撤销的历史边界。
    pub fn undo(&mut self) -> Result<Option<DocumentSnapshot>, DocumentError> {
        self.break_typing_group();
        let Some(entry) = self.undo.back().cloned() else {
            return Ok(None);
        };
        let next_revision = self.revision.next()?;
        validate_edits(&self.source, &entry.inverse)?;
        apply_edits(&mut self.source, &entry.inverse);
        self.source_format.apply_inverse(&entry.format_patch);
        self.undo.pop_back();
        self.redo.push(entry);
        self.revision = next_revision;
        Ok(Some(self.snapshot()))
    }

    /// 重做保留合并后的单条事务，但关闭输入组以免新按键接回旧历史。
    pub fn redo(&mut self) -> Result<Option<DocumentSnapshot>, DocumentError> {
        self.break_typing_group();
        let Some(entry) = self.redo.last().cloned() else {
            return Ok(None);
        };
        let next_revision = self.revision.next()?;
        validate_edits(&self.source, &entry.forward)?;
        apply_edits(&mut self.source, &entry.forward);
        self.source_format.apply(&entry.format_patch);
        self.redo.pop();
        self.record_undo(entry);
        self.revision = next_revision;
        Ok(Some(self.snapshot()))
    }

    /// 只关闭 typing 合并窗口，不清除可撤销内容。
    pub fn break_typing_group(&mut self) {
        self.active_typing_group = None;
    }

    fn record_undo(&mut self, entry: HistoryEntry) {
        if self.history_limit == 0 {
            return;
        }
        if self.undo.len() == self.history_limit {
            self.undo.pop_front();
        }
        self.undo.push_back(entry);
    }
}

/// 仅允许一处单行替换后继续合并，避免多编辑事务或换行格式变化跨输入组传播。
fn typing_span_for_edits(group_id: TypingGroupId, edits: &[TextEdit]) -> Option<TypingHistorySpan> {
    let [edit] = edits else {
        return None;
    };
    Some(TypingHistorySpan {
        group_id,
        inserted_end: edit.range.start.checked_add(edit.replacement.len())?,
        can_continue: !edit.replacement.is_empty()
            && !edit.replacement.contains('\r')
            && !edit.replacement.contains('\n'),
    })
}

/// 合并组首替换和后续尾部插入，保留组首逆文和格式补丁供一次撤销恢复。
fn merge_adjacent_typing_entry(
    previous: &HistoryEntry,
    next: &HistoryEntry,
    group_id: TypingGroupId,
) -> Option<HistoryEntry> {
    let previous_typing = previous.typing?;
    let next_typing = next.typing?;
    let [previous_forward] = previous.forward.as_slice() else {
        return None;
    };
    let [previous_inverse] = previous.inverse.as_slice() else {
        return None;
    };
    let [next_forward] = next.forward.as_slice() else {
        return None;
    };
    let [next_inverse] = next.inverse.as_slice() else {
        return None;
    };
    if previous_typing.group_id != group_id
        || next_typing.group_id != group_id
        || !previous_typing.can_continue
        || !next_typing.can_continue
        || !next_forward.range.is_empty()
        || next_forward.replacement.is_empty()
        || !next_inverse.replacement.is_empty()
        || next_forward.range.start != previous_typing.inserted_end
        || next_inverse.range != (next_forward.range.start..next_typing.inserted_end)
    {
        return None;
    }
    let combined_len = previous_forward
        .replacement
        .len()
        .checked_add(next_forward.replacement.len())?;
    let combined_end = previous_forward.range.start.checked_add(combined_len)?;
    let mut replacement = String::with_capacity(combined_len);
    replacement.push_str(&previous_forward.replacement);
    replacement.push_str(&next_forward.replacement);

    Some(HistoryEntry {
        forward: vec![TextEdit::new(
            previous_forward.range.clone(),
            Arc::<str>::from(replacement),
        )],
        inverse: vec![TextEdit::new(
            previous_forward.range.start..combined_end,
            previous_inverse.replacement.clone(),
        )],
        format_patch: previous.format_patch.clone(),
        typing: Some(next_typing),
    })
}

/// 文档 transaction 校验或执行错误。
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum DocumentError {
    /// transaction 基础版本已过期。
    #[error("transaction 版本过期，当前为 {expected:?}，收到 {actual:?}")]
    StaleRevision {
        /// 当前文档版本。
        expected: Revision,
        /// transaction 声明的版本。
        actual: Revision,
    },
    /// 编辑范围越界或首尾倒置。
    #[error("编辑范围 {range:?} 超出文档长度 {len}")]
    InvalidRange {
        /// 非法范围。
        range: Range<usize>,
        /// 当前文档字节长度。
        len: usize,
    },
    /// 编辑范围没有落在 UTF-8 字符边界。
    #[error("字节偏移 {offset} 不是 UTF-8 字符边界")]
    InvalidCharacterBoundary {
        /// 非法字节偏移。
        offset: usize,
    },
    /// 编辑没有按起始位置升序排列。
    #[error("编辑范围顺序错误，前一范围起点 {previous_start}，当前范围起点 {current_start}")]
    UnorderedEdits {
        /// 前一范围起点。
        previous_start: usize,
        /// 当前范围起点。
        current_start: usize,
    },
    /// 两个编辑范围发生重叠。
    #[error("编辑范围重叠，前一范围结束于 {previous_end}，当前范围起始于 {current_start}")]
    OverlappingEdits {
        /// 前一范围结束位置。
        previous_end: usize,
        /// 当前范围起始位置。
        current_start: usize,
    },
    /// 文档版本号耗尽。
    #[error("文档 revision 已达到上限")]
    RevisionExhausted,
    /// 编辑后的字节坐标无法表示。
    #[error("编辑后的字节坐标溢出")]
    OffsetOverflow,
}

fn validate_range(source: &Rope, range: &Range<usize>) -> Result<(), DocumentError> {
    if range.start > range.end || range.end > source.len() {
        return Err(DocumentError::InvalidRange {
            range: range.clone(),
            len: source.len(),
        });
    }
    for offset in [range.start, range.end] {
        if !source.is_char_boundary(offset) {
            return Err(DocumentError::InvalidCharacterBoundary { offset });
        }
    }
    Ok(())
}

fn validate_edits(source: &Rope, edits: &[TextEdit]) -> Result<(), DocumentError> {
    let mut previous: Option<&Range<usize>> = None;
    for edit in edits {
        validate_range(source, &edit.range)?;
        if let Some(previous) = previous {
            if edit.range.start < previous.start {
                return Err(DocumentError::UnorderedEdits {
                    previous_start: previous.start,
                    current_start: edit.range.start,
                });
            }
            if edit.range.start < previous.end {
                return Err(DocumentError::OverlappingEdits {
                    previous_end: previous.end,
                    current_start: edit.range.start,
                });
            }
        }
        previous = Some(&edit.range);
    }
    Ok(())
}

fn build_inverse_edits(source: &Rope, edits: &[TextEdit]) -> Result<Vec<TextEdit>, DocumentError> {
    let mut inverse = Vec::with_capacity(edits.len());
    let mut delta = 0_i128;

    for edit in edits {
        let start = shift_offset(edit.range.start, delta)?;
        let end = start
            .checked_add(edit.replacement.len())
            .ok_or(DocumentError::OffsetOverflow)?;
        let removed = source.slice(edit.range.clone()).to_string();
        inverse.push(TextEdit::new(start..end, removed));

        let removed_len = edit.range.end - edit.range.start;
        delta += edit.replacement.len() as i128 - removed_len as i128;
    }

    Ok(inverse)
}

fn shift_offset(offset: usize, delta: i128) -> Result<usize, DocumentError> {
    let shifted = offset as i128 + delta;
    usize::try_from(shifted).map_err(|_| DocumentError::OffsetOverflow)
}

fn apply_edits(source: &mut Rope, edits: &[TextEdit]) {
    for edit in edits.iter().rev() {
        source.replace(edit.range.clone(), &edit.replacement);
    }
}
