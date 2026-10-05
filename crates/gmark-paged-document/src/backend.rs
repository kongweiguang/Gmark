// @author kongweiguang

use std::ops::Range;
use std::path::Path;
use std::sync::Arc;

pub use gmark_document_core::{SourceAffinity, SourceAnchor};

use crate::{
    EncodedSavePlan, ExternalChange, FileIdentity, FileSource, LineIndex, PagedDocumentError,
    PieceDocument, PreparedUtf8Source, SearchCancellation, SearchMatch, SearchOptions,
    TextEncoding,
};

/// 单次视口读取的硬上限，调用方不能通过异常窗口把整条超长行物化进内存。
pub const DEFAULT_VIEWPORT_COLUMN_BYTES: u64 = 64 * 1024;
pub const MAX_VIEWPORT_ROWS: usize = 4_096;
pub const MAX_VIEWPORT_OVERSCAN_ROWS: usize = 512;
pub const MAX_SYSTEM_CLIPBOARD_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionTransfer {
    Clipboard,
    ExportFile,
}

pub const fn selection_transfer_for_len(byte_len: u64) -> SelectionTransfer {
    if byte_len <= MAX_SYSTEM_CLIPBOARD_BYTES {
        SelectionTransfer::Clipboard
    } else {
        SelectionTransfer::ExportFile
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewportRequest {
    pub start_line: u64,
    pub rows: usize,
    pub overscan_rows: usize,
    pub column_start: u64,
    pub column_bytes: u64,
    pub generation: u64,
}

impl ViewportRequest {
    pub fn bounded(
        start_line: u64,
        rows: usize,
        overscan_rows: usize,
        column_start: u64,
        generation: u64,
    ) -> Self {
        Self {
            start_line,
            rows: rows.min(MAX_VIEWPORT_ROWS),
            overscan_rows: overscan_rows.min(MAX_VIEWPORT_OVERSCAN_ROWS),
            column_start,
            column_bytes: DEFAULT_VIEWPORT_COLUMN_BYTES,
            generation,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewportLine {
    pub line: u64,
    pub source_range: Range<u64>,
    pub content_range: Range<u64>,
    pub text: String,
    pub ending: String,
    pub leading_truncated: bool,
    pub trailing_truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewportSnapshot {
    pub generation: u64,
    pub requested_lines: Range<u64>,
    pub exact_line_count: u64,
    pub lines: Vec<ViewportLine>,
}

/// 磁盘后备文档的纯数据层。GPUI 只能把它克隆到后台 worker，并消费不可变快照。
#[derive(Clone)]
pub struct PagedDocumentBackend {
    document: PieceDocument,
    generation: u64,
}

impl PagedDocumentBackend {
    pub fn new(document: PieceDocument) -> Self {
        Self {
            document,
            generation: 0,
        }
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn set_generation(&mut self, generation: u64) {
        self.generation = generation;
    }

    pub fn read_viewport(
        &self,
        request: &ViewportRequest,
    ) -> Result<ViewportSnapshot, PagedDocumentError> {
        self.read_viewport_cancellable(request, &SearchCancellation::default())
    }

    pub fn read_viewport_cancellable(
        &self,
        request: &ViewportRequest,
        cancellation: &SearchCancellation,
    ) -> Result<ViewportSnapshot, PagedDocumentError> {
        let rows = request.rows.min(MAX_VIEWPORT_ROWS);
        let overscan = request.overscan_rows.min(MAX_VIEWPORT_OVERSCAN_ROWS) as u64;
        let start = request.start_line.saturating_sub(overscan);
        let end = request
            .start_line
            .saturating_add(rows as u64)
            .saturating_add(overscan)
            .min(self.document.line_count());
        let column_bytes = request.column_bytes.clamp(1, DEFAULT_VIEWPORT_COLUMN_BYTES);
        let mut lines = Vec::with_capacity(usize::try_from(end - start).unwrap_or_default());
        for line in start..end {
            if cancellation.is_cancelled() {
                return Err(PagedDocumentError::Cancelled);
            }
            if let Some(viewport_line) =
                read_line_window(&self.document, line, request.column_start, column_bytes)?
            {
                lines.push(viewport_line);
            }
        }
        Ok(ViewportSnapshot {
            generation: request.generation,
            requested_lines: start..end,
            exact_line_count: self.document.line_count(),
            lines,
        })
    }

    fn mark_changed(&mut self) {
        self.generation = self.generation.wrapping_add(1);
    }
}

/// 普通 Editor 面向大文档的契约层：编辑 history 与 viewport 共用源码字节坐标；
/// selection 由共享 Controller 按视图实例维护。
#[derive(Clone)]
pub struct PagedDocument {
    backend: PagedDocumentBackend,
    encoding: PagedEncodingState,
}

#[derive(Clone)]
struct PagedEncodingState {
    encoding: TextEncoding,
    original_identity: FileIdentity,
    shadow: Option<Arc<tempfile::NamedTempFile>>,
    save_plan: Option<EncodedSavePlan>,
}

impl PagedDocument {
    pub fn new(document: PieceDocument) -> Self {
        let original_identity = document.base_identity();
        Self {
            backend: PagedDocumentBackend::new(document),
            encoding: PagedEncodingState {
                encoding: TextEncoding::Utf8 { bom: false },
                original_identity,
                shadow: None,
                save_plan: None,
            },
        }
    }

    /// Construct a paged document from an IO-prepared UTF-8 shadow.  The
    /// shadow tempfile and encoded save plan move into this value so they live
    /// exactly as long as the shared Controller session.
    pub fn from_prepared(
        prepared: PreparedUtf8Source,
        index: LineIndex,
    ) -> Result<Self, PagedDocumentError> {
        let (source, encoding, shadow, save_plan) = prepared.into_backend_parts();
        let original_identity = save_plan
            .as_ref()
            .map(|plan| plan.original_identity().clone())
            .unwrap_or(source.identity()?);
        let document = PieceDocument::open(source, index)?;
        Ok(Self {
            backend: PagedDocumentBackend::new(document),
            encoding: PagedEncodingState {
                encoding,
                original_identity,
                shadow,
                save_plan,
            },
        })
    }

    pub fn backend(&self) -> &PagedDocumentBackend {
        &self.backend
    }

    /// Clone the FileSource handle backing the current paged pieces.  This is
    /// a read-only view of the same source (or UTF-8 shadow), never a second
    /// body representation and never a fresh file open.
    pub fn prepared_source(&self) -> Result<FileSource, PagedDocumentError> {
        self.backend.document.base_source()
    }

    /// 当前 Source 内容代次。后台任务只能在代次仍一致时提交会修改正文的结果。
    pub fn revision(&self) -> u64 {
        self.backend.generation()
    }

    pub fn set_revision(&mut self, revision: u64) {
        self.backend.set_generation(revision);
    }

    pub fn advance_revision(&mut self) -> Result<u64, PagedDocumentError> {
        let next = self.revision().checked_add(1).ok_or_else(|| {
            PagedDocumentError::InvalidTransaction("document revision overflow".to_owned())
        })?;
        self.backend.set_generation(next);
        Ok(next)
    }

    pub fn encoding(&self) -> &TextEncoding {
        &self.encoding.encoding
    }

    /// Change the encoding metadata without touching source bytes.  A shadow
    /// keeps an encoded plan even for UTF-8 so source identity checks remain
    /// tied to the original on-disk file; direct UTF-8 sources stay no-plan.
    pub fn set_encoding(&mut self, encoding: TextEncoding) -> bool {
        if self.encoding.encoding == encoding {
            return false;
        }
        self.encoding.encoding = encoding.clone();
        if self.encoding.shadow.is_some() {
            match self.encoding.save_plan.as_mut() {
                Some(plan) => plan.set_encoding(encoding),
                None => {
                    self.encoding.save_plan = Some(EncodedSavePlan::new(
                        encoding,
                        self.encoding.original_identity.clone(),
                    ));
                }
            }
        } else if matches!(&encoding, TextEncoding::Utf8 { bom: false }) {
            self.encoding.save_plan = None;
        } else {
            self.encoding.save_plan = Some(EncodedSavePlan::new(
                encoding,
                self.encoding.original_identity.clone(),
            ));
        }
        true
    }

    pub fn prepared_save_plan(&self) -> Option<EncodedSavePlan> {
        self.encoding.save_plan.clone()
    }

    /// Update the source identity captured by the internal encoded plan after
    /// a successful save or Save As operation.
    pub fn mark_prepared_saved(&mut self, identity: FileIdentity) {
        self.encoding.original_identity = identity.clone();
        if let Some(plan) = self.encoding.save_plan.as_mut() {
            plan.mark_original_saved(identity);
        }
    }

    pub fn read_viewport(
        &self,
        request: &ViewportRequest,
    ) -> Result<ViewportSnapshot, PagedDocumentError> {
        self.backend.read_viewport(request)
    }

    pub fn read_viewport_cancellable(
        &self,
        request: &ViewportRequest,
        cancellation: &SearchCancellation,
    ) -> Result<ViewportSnapshot, PagedDocumentError> {
        self.backend
            .read_viewport_cancellable(request, cancellation)
    }

    pub fn len(&self) -> u64 {
        self.backend.document.len()
    }

    pub fn is_empty(&self) -> bool {
        self.backend.document.is_empty()
    }

    pub fn is_pristine(&self) -> bool {
        self.backend.document.is_pristine()
    }

    pub fn mark_current_pristine(&mut self) {
        self.backend.document.mark_current_pristine();
    }

    pub fn line_count(&self) -> u64 {
        self.backend.document.line_count()
    }

    pub fn line_range(&self, line: u64) -> Option<Range<u64>> {
        self.backend.document.line_range(line)
    }

    pub fn line_for_offset(&self, offset: u64) -> Option<u64> {
        self.backend.document.line_for_offset(offset)
    }

    pub fn line_index(&self) -> LineIndex {
        self.backend.document.line_index()
    }

    pub fn read_range(&self, range: Range<u64>) -> Result<Vec<u8>, PagedDocumentError> {
        self.backend.document.read_range(range)
    }

    pub fn write_to(&self, output: impl std::io::Write) -> Result<(), PagedDocumentError> {
        self.backend.document.write_to(output)
    }

    pub fn write_to_cancellable(
        &self,
        output: impl std::io::Write,
        cancellation: &SearchCancellation,
    ) -> Result<(), PagedDocumentError> {
        self.backend
            .document
            .write_to_cancellable(output, cancellation)
    }

    pub fn read_range_cancellable(
        &self,
        range: Range<u64>,
        cancellation: &SearchCancellation,
    ) -> Result<Vec<u8>, PagedDocumentError> {
        self.backend
            .document
            .read_range_cancellable(range, cancellation)
    }

    pub fn search(
        &self,
        query: &str,
        options: SearchOptions,
        cancellation: &SearchCancellation,
    ) -> Result<Vec<SearchMatch>, PagedDocumentError> {
        self.backend.document.search(query, options, cancellation)
    }

    pub fn external_change(&self) -> Result<ExternalChange, PagedDocumentError> {
        let base_identity = self.backend.document.base_identity();
        if self.encoding.original_identity == base_identity {
            return self.backend.document.external_change();
        }

        // An immutable snapshot save (including Save As and encoded shadow
        // saves) advances the persisted disk identity without rebuilding the
        // PieceTree.  Once those identities diverge, checking the old base
        // path would report our own replacement as an external change.  Read
        // the current persisted target instead and classify conservatively:
        // append detection is only sound while the PieceTree still owns that
        // same on-disk baseline.
        let current_source = FileSource::open(&self.encoding.original_identity.path)?;
        let current = current_source.identity()?;
        if current == self.encoding.original_identity {
            return Ok(ExternalChange::Unchanged);
        }
        if current.os_file_id != self.encoding.original_identity.os_file_id {
            return Ok(ExternalChange::Replaced);
        }
        if current.len < self.encoding.original_identity.len {
            return Ok(ExternalChange::Truncated { len: current.len });
        }
        Ok(ExternalChange::Modified)
    }

    pub fn accept_external_append(
        &mut self,
        source: FileSource,
        index: LineIndex,
    ) -> Result<(), PagedDocumentError> {
        let identity = source.identity()?;
        self.backend
            .document
            .accept_external_append(source, index)?;
        self.mark_prepared_saved(identity);
        self.backend.mark_changed();
        Ok(())
    }

    pub fn save_atomic_cancellable(
        &mut self,
        path: impl AsRef<Path>,
        cancellation: &SearchCancellation,
    ) -> Result<(), PagedDocumentError> {
        self.backend
            .document
            .save_atomic_cancellable(path, cancellation)?;
        self.backend.mark_changed();
        Ok(())
    }

    pub fn save_range_atomic_cancellable(
        &self,
        range: Range<u64>,
        path: impl AsRef<Path>,
        cancellation: &SearchCancellation,
    ) -> Result<(), PagedDocumentError> {
        self.backend
            .document
            .save_range_atomic_cancellable(range, path, cancellation)
    }

    /// Save using the plan retained by this shared document.  The older
    /// plan-parameter methods above remain only while application adapters
    /// migrate; new callers must use this lifecycle-owned entry point.
    pub fn save_prepared_atomic_cancellable(
        &mut self,
        path: impl AsRef<Path>,
        cancellation: &SearchCancellation,
    ) -> Result<FileIdentity, PagedDocumentError> {
        let path = path.as_ref();
        let identity = if let Some(plan) = self.encoding.save_plan.as_ref() {
            plan.save_atomic_cancellable(&self.backend.document, path, cancellation)?
        } else {
            self.backend
                .document
                .save_atomic_cancellable(path, cancellation)?;
            FileSource::open(path)?.identity()?
        };
        self.mark_prepared_saved(identity.clone());
        Ok(identity)
    }

    pub fn save_prepared_atomic_as_cancellable(
        &mut self,
        path: impl AsRef<Path>,
        cancellation: &SearchCancellation,
    ) -> Result<FileIdentity, PagedDocumentError> {
        let path = path.as_ref();
        let identity = if let Some(plan) = self.encoding.save_plan.as_ref() {
            plan.save_atomic_as_cancellable(&self.backend.document, path, cancellation)?
        } else {
            self.backend
                .document
                .save_atomic_cancellable(path, cancellation)?;
            FileSource::open(path)?.identity()?
        };
        self.mark_prepared_saved(identity.clone());
        Ok(identity)
    }

    pub fn save_prepared_range_atomic_cancellable(
        &mut self,
        range: Range<u64>,
        path: impl AsRef<Path>,
        cancellation: &SearchCancellation,
    ) -> Result<FileIdentity, PagedDocumentError> {
        let path = path.as_ref();
        let identity = if let Some(plan) = self.encoding.save_plan.as_ref() {
            plan.save_range_atomic_as_cancellable(
                &self.backend.document,
                range,
                path,
                cancellation,
            )?
        } else {
            self.backend
                .document
                .save_range_atomic_cancellable(range, path, cancellation)?;
            FileSource::open(path)?.identity()?
        };
        Ok(identity)
    }

    pub fn replace_text(
        &mut self,
        range: Range<u64>,
        replacement: &str,
    ) -> Result<(), PagedDocumentError> {
        self.backend
            .document
            .replace_text(range.clone(), replacement)?;
        self.backend.mark_changed();
        Ok(())
    }

    pub fn replace_text_reader(
        &mut self,
        range: Range<u64>,
        reader: impl std::io::Read,
    ) -> Result<(), PagedDocumentError> {
        self.backend
            .document
            .replace_text_reader(range.clone(), reader)?;
        self.backend.mark_changed();
        Ok(())
    }

    /// 派生视图不得直接修改自己的 projection；只能提交带 base revision 的
    /// Source transaction。陈旧 revision 和重叠 range 在触碰正文前即被拒绝。
    pub fn apply_transaction(
        &mut self,
        transaction: &gmark_document_core::Transaction,
    ) -> Result<(), PagedDocumentError> {
        self.apply_transaction_with_group(transaction, None)
    }

    /// 将 Controller 明确分组的 Source 文字输入映射为一份 Paged 撤销根。
    pub fn apply_typing_transaction(
        &mut self,
        transaction: &gmark_document_core::Transaction,
        group_id: gmark_document_core::TypingGroupId,
    ) -> Result<(), PagedDocumentError> {
        self.apply_transaction_with_group(transaction, Some(group_id))
    }

    /// 共享 stale 检查与 revision 推进，避免分组入口绕开普通事务契约。
    fn apply_transaction_with_group(
        &mut self,
        transaction: &gmark_document_core::Transaction,
        group_id: Option<gmark_document_core::TypingGroupId>,
    ) -> Result<(), PagedDocumentError> {
        if transaction.base_revision.0 != self.revision() {
            self.backend.document.break_typing_group();
            return Err(PagedDocumentError::SourceChanged);
        }
        let edits = transaction
            .edits
            .iter()
            .map(|edit| (edit.range.clone(), edit.replacement.clone()))
            .collect::<Vec<_>>();
        match group_id {
            Some(group_id) => self
                .backend
                .document
                .replace_text_batch_in_typing_group(&edits, group_id)?,
            None => self.backend.document.replace_text_batch(&edits)?,
        }
        if !edits.is_empty() {
            self.backend.mark_changed();
        }
        Ok(())
    }

    /// 结束跨按键合并资格，但保留既有撤销与重做内容。
    pub fn break_typing_group(&mut self) {
        self.backend.document.break_typing_group();
    }

    pub fn undo(&mut self) -> bool {
        let changed = self.backend.document.undo();
        if changed {
            self.backend.mark_changed();
        }
        changed
    }

    pub fn redo(&mut self) -> bool {
        let changed = self.backend.document.redo();
        if changed {
            self.backend.mark_changed();
        }
        changed
    }
}

impl From<PieceDocument> for PagedDocument {
    fn from(document: PieceDocument) -> Self {
        Self::new(document)
    }
}

fn read_line_window(
    document: &PieceDocument,
    line: u64,
    requested_start: u64,
    maximum_bytes: u64,
) -> Result<Option<ViewportLine>, PagedDocumentError> {
    let Some(line_range) = document.line_range(line) else {
        return Ok(None);
    };
    let tail_start = line_range.end.saturating_sub(2).max(line_range.start);
    let tail = document.read_range(tail_start..line_range.end)?;
    let ending_len = if tail.ends_with(b"\r\n") {
        2
    } else if tail.ends_with(b"\n") || tail.ends_with(b"\r") {
        1
    } else {
        0
    };
    let content_end = line_range.end.saturating_sub(ending_len);
    let content_len = content_end.saturating_sub(line_range.start);
    let relative_start = requested_start.min(content_len.saturating_sub(maximum_bytes));
    let mut start = line_range.start.saturating_add(relative_start);
    if start > line_range.start && start < content_end {
        let probe_end = (start + 4).min(content_end);
        let probe = document.read_range(start..probe_end)?;
        start = start.saturating_add(
            probe
                .iter()
                .take_while(|byte| **byte & 0b1100_0000 == 0b1000_0000)
                .count() as u64,
        );
    }
    let requested_end = start.saturating_add(maximum_bytes).min(content_end);
    let mut bytes = document.read_range(start..requested_end)?;
    let mut end = requested_end;
    if let Err(error) = std::str::from_utf8(&bytes)
        && error.error_len().is_none()
    {
        bytes.truncate(error.valid_up_to());
        end = start.saturating_add(bytes.len() as u64);
    }
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let ending = if end == content_end && ending_len > 0 {
        String::from_utf8_lossy(&tail[tail.len() - ending_len as usize..]).into_owned()
    } else {
        String::new()
    };
    Ok(Some(ViewportLine {
        line,
        source_range: line_range.clone(),
        content_range: start..end,
        text,
        ending,
        leading_truncated: start > line_range.start,
        trailing_truncated: end < content_end,
    }))
}

impl gmark_document_core::DocumentSnapshot for PagedDocument {
    fn revision(&self) -> gmark_document_core::DocumentRevision {
        gmark_document_core::DocumentRevision(PagedDocument::revision(self))
    }

    fn len(&self) -> u64 {
        PagedDocument::len(self)
    }

    fn read_range(&self, range: Range<u64>) -> Result<Vec<u8>, gmark_document_core::SnapshotError> {
        PagedDocument::read_range(self, range).map_err(|error| match error {
            PagedDocumentError::InvalidRange { start, end, len } => {
                gmark_document_core::SnapshotError::InvalidRange { start, end, len }
            }
            PagedDocumentError::RangeTooLarge => gmark_document_core::SnapshotError::RangeTooLarge,
            error => gmark_document_core::SnapshotError::Read(error.to_string()),
        })
    }
}

impl gmark_document_core::ProjectionCancellation for SearchCancellation {
    fn is_cancelled(&self) -> bool {
        SearchCancellation::is_cancelled(self)
    }
}
