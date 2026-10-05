// @author kongweiguang

use std::io::{Read, Write};
use std::ops::Range;
use std::path::Path;
use std::sync::Arc;

use gmark_document::{
    SourceDocument, TextEdit, Transaction as ResidentTransaction, atomic_write_verified,
};
use gmark_document_core::{
    DocumentRevision, DocumentSnapshot, SourceAffinity, SourceAnchor, SourceEdit, TextEncoding,
    Transaction, TypingGroupId,
};
use gmark_paged_document::{
    ExternalChange, FileIdentity, FileSource, PagedDocumentError, SearchCancellation, SearchMatch,
    SearchOptions, ViewportLine, ViewportRequest, ViewportSnapshot,
};
use regex::RegexBuilder;

const HISTORY_LIMIT: usize = 1_024;

/// 当前保存基线通常共享 Rope；只有后台保存了旧 revision 时才保留独立文本。
#[derive(Clone)]
enum PersistedContent {
    Snapshot(gmark_document::DocumentSnapshot),
    Materialized(Arc<str>),
}

impl PersistedContent {
    fn matches(&self, document: &SourceDocument) -> bool {
        match self {
            Self::Snapshot(snapshot) => document.content_matches_snapshot(snapshot),
            Self::Materialized(text) => document.content_matches_text(text),
        }
    }
}

/// 普通文件的 Rope 后端。selection 与正文 history 由共享 Controller 协调，
/// 磁盘身份只用于冲突检测。
#[derive(Clone)]
pub struct ResidentDocument {
    document: SourceDocument,
    encoding: TextEncoding,
    persisted_content: PersistedContent,
    source_identity: FileIdentity,
    lines: Arc<[Range<u64>]>,
    structural_units: u64,
}

impl ResidentDocument {
    pub fn new(text: &str, encoding: TextEncoding, source_identity: FileIdentity) -> Self {
        Self::from_source_document(
            SourceDocument::with_history_limit(text, HISTORY_LIMIT),
            encoding,
            source_identity,
        )
    }

    pub fn from_source_document(
        document: SourceDocument,
        encoding: TextEncoding,
        source_identity: FileIdentity,
    ) -> Self {
        let persisted_snapshot = document.snapshot();
        let normalized = document.text();
        let (lines, structural_units) = build_source_metrics(&normalized);
        Self {
            document,
            encoding,
            persisted_content: PersistedContent::Snapshot(persisted_snapshot),
            source_identity,
            lines,
            structural_units,
        }
    }

    pub fn source_document(&self) -> &SourceDocument {
        &self.document
    }

    pub fn source_document_mut(&mut self) -> &mut SourceDocument {
        &mut self.document
    }

    /// 兼容 Markdown controller 的 SourceDocument transaction 后重建通用行坐标。
    pub fn refresh_source_state(&mut self) {
        self.rebuild_lines();
    }

    pub fn revision(&self) -> u64 {
        self.document.revision().get()
    }

    pub fn set_revision(&mut self, revision: DocumentRevision) {
        self.document
            .set_revision(gmark_document::Revision::from_u64(revision.0));
    }

    pub fn advance_revision(&mut self) -> Result<DocumentRevision, gmark_document::DocumentError> {
        self.document
            .advance_revision()
            .map(|revision| DocumentRevision(revision.get()))
    }

    pub fn set_encoding(&mut self, encoding: TextEncoding) {
        self.encoding = encoding;
    }

    pub fn snapshot(&self) -> Arc<dyn DocumentSnapshot> {
        Arc::new(self.document.snapshot())
    }

    pub fn len(&self) -> u64 {
        self.document.len() as u64
    }

    pub fn is_empty(&self) -> bool {
        self.document.is_empty()
    }

    pub fn is_pristine(&self) -> bool {
        self.persisted_content.matches(&self.document)
    }

    pub fn mark_persisted(&mut self) {
        self.persisted_content = PersistedContent::Snapshot(self.document.snapshot());
    }

    pub fn mark_persisted_snapshot(&mut self, snapshot: gmark_document::DocumentSnapshot) {
        self.persisted_content = PersistedContent::Snapshot(snapshot);
    }

    pub fn mark_persisted_text(&mut self, text: impl Into<Arc<str>>) {
        self.persisted_content = PersistedContent::Materialized(text.into());
    }

    pub fn line_count(&self) -> u64 {
        self.lines.len() as u64
    }

    pub fn structural_units(&self) -> u64 {
        self.structural_units
    }

    pub fn line_range(&self, line: u64) -> Option<Range<u64>> {
        self.lines.get(usize::try_from(line).ok()?).cloned()
    }

    pub fn line_for_offset(&self, offset: u64) -> Option<u64> {
        if offset > self.len() {
            return None;
        }
        let index = self
            .lines
            .partition_point(|range| range.end <= offset && range.end < self.len())
            .min(self.lines.len().saturating_sub(1));
        Some(index as u64)
    }

    pub fn read_range(&self, range: Range<u64>) -> Result<Vec<u8>, PagedDocumentError> {
        let start = usize::try_from(range.start).map_err(|_| PagedDocumentError::RangeTooLarge)?;
        let end = usize::try_from(range.end).map_err(|_| PagedDocumentError::RangeTooLarge)?;
        self.document
            .snapshot()
            .text_for_range(start..end)
            .map(String::into_bytes)
            .map_err(|error| map_document_error(error.to_string(), self.len(), range))
    }

    pub fn read_range_cancellable(
        &self,
        range: Range<u64>,
        cancellation: &SearchCancellation,
    ) -> Result<Vec<u8>, PagedDocumentError> {
        if cancellation.is_cancelled() {
            return Err(PagedDocumentError::Cancelled);
        }
        self.read_range(range)
    }

    pub fn write_to_cancellable(
        &self,
        mut output: impl Write,
        cancellation: &SearchCancellation,
    ) -> Result<(), PagedDocumentError> {
        if cancellation.is_cancelled() {
            return Err(PagedDocumentError::Cancelled);
        }
        output
            .write_all(self.document.text().as_bytes())
            .map_err(|source| PagedDocumentError::Io {
                path: self.source_identity.path.clone(),
                source,
            })
    }

    pub fn write_to(&self, output: impl Write) -> Result<(), PagedDocumentError> {
        self.write_to_cancellable(output, &SearchCancellation::default())
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
        let overscan = request.overscan_rows.min(512) as u64;
        let start = request.start_line.saturating_sub(overscan);
        let end = request
            .start_line
            .saturating_add(request.rows.min(4_096) as u64)
            .saturating_add(overscan)
            .min(self.line_count());
        let mut lines = Vec::with_capacity(usize::try_from(end - start).unwrap_or_default());
        for line in start..end {
            if cancellation.is_cancelled() {
                return Err(PagedDocumentError::Cancelled);
            }
            if let Some(line) = self.read_line_window(
                line,
                request.column_start,
                request.column_bytes.clamp(1, 64 * 1024),
            )? {
                lines.push(line);
            }
        }
        Ok(ViewportSnapshot {
            generation: request.generation,
            requested_lines: start..end,
            exact_line_count: self.line_count(),
            lines,
        })
    }

    pub fn search(
        &self,
        query: &str,
        options: SearchOptions,
        cancellation: &SearchCancellation,
    ) -> Result<Vec<SearchMatch>, PagedDocumentError> {
        if cancellation.is_cancelled() {
            return Err(PagedDocumentError::Cancelled);
        }
        if query.is_empty() || options.result_limit == 0 {
            return Ok(Vec::new());
        }
        let pattern = if options.regex {
            query.to_owned()
        } else {
            regex::escape(query)
        };
        let pattern = if options.whole_word {
            format!(r"\b(?:{pattern})\b")
        } else {
            pattern
        };
        let regex = RegexBuilder::new(&pattern)
            .case_insensitive(!options.case_sensitive)
            .build()
            .map_err(|error| PagedDocumentError::InvalidRegex(error.to_string()))?;
        let text = self.document.text();
        Ok(regex
            .find_iter(&text)
            .take_while(|_| !cancellation.is_cancelled())
            .take(options.result_limit)
            .map(|found| search_match(found.range().start as u64..found.range().end as u64))
            .collect())
    }

    /// 单步替换作为普通事务提交，自动结束任何先前的连续输入分组。
    pub fn replace_text(
        &mut self,
        range: Range<u64>,
        replacement: &str,
    ) -> Result<(), PagedDocumentError> {
        self.apply_edits(
            self.revision(),
            &[SourceEdit::new(range.clone(), replacement)],
            None,
        )?;
        Ok(())
    }

    pub fn replace_text_reader(
        &mut self,
        range: Range<u64>,
        mut reader: impl Read,
    ) -> Result<(), PagedDocumentError> {
        let mut replacement = String::new();
        reader
            .read_to_string(&mut replacement)
            .map_err(|source| PagedDocumentError::Io {
                path: self.source_identity.path.clone(),
                source,
            })?;
        self.replace_text(range, &replacement)
    }

    pub fn apply_transaction(&mut self, transaction: &Transaction) -> Result<(), String> {
        self.apply_edits(transaction.base_revision.0, &transaction.edits, None)
            .map_err(|error| error.to_string())
    }

    /// 应用 Controller 标记的输入组并同步行指标，保持 Resident 与 Paged 撤销边界一致。
    pub fn apply_typing_transaction(
        &mut self,
        transaction: &Transaction,
        group_id: TypingGroupId,
    ) -> Result<(), String> {
        self.apply_edits(
            transaction.base_revision.0,
            &transaction.edits,
            Some(group_id),
        )
        .map_err(|error| error.to_string())
    }

    pub fn undo(&mut self) -> bool {
        match self.document.undo() {
            Ok(Some(_)) => {
                self.rebuild_lines();
                true
            }
            Ok(None) | Err(_) => false,
        }
    }

    pub fn redo(&mut self) -> bool {
        match self.document.redo() {
            Ok(Some(_)) => {
                self.rebuild_lines();
                true
            }
            Ok(None) | Err(_) => false,
        }
    }

    pub fn external_change(&self) -> Result<ExternalChange, PagedDocumentError> {
        let current = FileSource::open(&self.source_identity.path)?.identity()?;
        if current == self.source_identity {
            return Ok(ExternalChange::Unchanged);
        }
        if current.os_file_id != self.source_identity.os_file_id {
            return Ok(ExternalChange::Replaced);
        }
        if current.len < self.source_identity.len {
            return Ok(ExternalChange::Truncated { len: current.len });
        }
        if current.len > self.source_identity.len {
            return Ok(ExternalChange::Appended {
                from: self.source_identity.len,
                to: current.len,
            });
        }
        Ok(ExternalChange::Modified)
    }

    pub fn accept_external_append(&mut self, source: FileSource) -> Result<(), PagedDocumentError> {
        let identity = source.identity()?;
        let bytes = source.read_range(0, identity.len)?;
        let text = std::str::from_utf8(&bytes).map_err(|_| PagedDocumentError::Binary)?;
        *self = Self::new(text, self.encoding.clone(), identity);
        Ok(())
    }

    pub fn save_atomic_cancellable(
        &mut self,
        path: impl AsRef<Path>,
        cancellation: &SearchCancellation,
    ) -> Result<(), PagedDocumentError> {
        if cancellation.is_cancelled() {
            return Err(PagedDocumentError::Cancelled);
        }
        if path.as_ref() == self.source_identity.path
            && !matches!(self.external_change()?, ExternalChange::Unchanged)
        {
            return Err(PagedDocumentError::SourceChanged);
        }
        let bytes = self.encoded_bytes()?;
        atomic_write_verified(path.as_ref(), &bytes).map_err(|error| {
            PagedDocumentError::Persist {
                path: path.as_ref().to_path_buf(),
                source: std::io::Error::other(error.to_string()),
            }
        })?;
        self.source_identity = FileSource::open(path.as_ref())?.identity()?;
        self.persisted_content = PersistedContent::Snapshot(self.document.snapshot());
        Ok(())
    }

    pub fn save_range_atomic_cancellable(
        &self,
        range: Range<u64>,
        path: impl AsRef<Path>,
        cancellation: &SearchCancellation,
    ) -> Result<(), PagedDocumentError> {
        if cancellation.is_cancelled() {
            return Err(PagedDocumentError::Cancelled);
        }
        let bytes = self.read_range(range)?;
        atomic_write_verified(path.as_ref(), &bytes).map_err(|error| PagedDocumentError::Persist {
            path: path.as_ref().to_path_buf(),
            source: std::io::Error::other(error.to_string()),
        })
    }

    pub fn save_copy_atomic_cancellable(
        &self,
        path: impl AsRef<Path>,
        cancellation: &SearchCancellation,
    ) -> Result<FileIdentity, PagedDocumentError> {
        if cancellation.is_cancelled() {
            return Err(PagedDocumentError::Cancelled);
        }
        let bytes = self.encoded_bytes()?;
        atomic_write_verified(path.as_ref(), &bytes).map_err(|error| {
            PagedDocumentError::Persist {
                path: path.as_ref().to_path_buf(),
                source: std::io::Error::other(error.to_string()),
            }
        })?;
        FileSource::open(path.as_ref())?.identity()
    }

    fn apply_edits(
        &mut self,
        base_revision: u64,
        edits: &[SourceEdit],
        group_id: Option<TypingGroupId>,
    ) -> Result<(), PagedDocumentError> {
        if base_revision != self.revision() {
            self.document.break_typing_group();
            return Err(PagedDocumentError::SourceChanged);
        }
        let resident_edits = edits
            .iter()
            .map(|edit| {
                let start = usize::try_from(edit.range.start)
                    .map_err(|_| PagedDocumentError::RangeTooLarge)?;
                let end = usize::try_from(edit.range.end)
                    .map_err(|_| PagedDocumentError::RangeTooLarge)?;
                Ok(TextEdit::new(start..end, edit.replacement.clone()))
            })
            .collect::<Result<Vec<_>, PagedDocumentError>>()?;
        let transaction = ResidentTransaction::new(self.document.revision(), resident_edits);
        let result = match group_id {
            Some(group_id) => self
                .document
                .apply_typing_transaction(transaction, group_id),
            None => self.document.apply_transaction(transaction),
        };
        result.map_err(|error| map_document_error(error.to_string(), self.len(), 0..0))?;
        if !edits.is_empty() {
            self.rebuild_lines();
        }
        Ok(())
    }

    fn read_line_window(
        &self,
        line: u64,
        requested_start: u64,
        maximum_bytes: u64,
    ) -> Result<Option<ViewportLine>, PagedDocumentError> {
        let Some(line_range) = self.line_range(line) else {
            return Ok(None);
        };
        let tail_start = line_range.end.saturating_sub(1).max(line_range.start);
        let tail = self.read_range(tail_start..line_range.end)?;
        let ending_len = usize::from(tail.ends_with(b"\n"));
        let content_end = line_range.end.saturating_sub(ending_len as u64);
        let content_len = content_end.saturating_sub(line_range.start);
        let relative_start = requested_start.min(content_len.saturating_sub(maximum_bytes));
        let mut start = line_range.start.saturating_add(relative_start);
        while start < content_end {
            let byte = self.read_range(start..(start + 1).min(content_end))?;
            if byte
                .first()
                .is_none_or(|byte| byte & 0b1100_0000 != 0b1000_0000)
            {
                break;
            }
            start += 1;
        }
        let requested_end = start.saturating_add(maximum_bytes).min(content_end);
        let mut bytes = self.read_range(start..requested_end)?;
        if let Err(error) = std::str::from_utf8(&bytes)
            && error.error_len().is_none()
        {
            bytes.truncate(error.valid_up_to());
        }
        let end = start.saturating_add(bytes.len() as u64);
        Ok(Some(ViewportLine {
            line,
            source_range: line_range.clone(),
            content_range: start..end,
            text: String::from_utf8_lossy(&bytes).into_owned(),
            ending: if end == content_end && ending_len > 0 {
                "\n"
            } else {
                ""
            }
            .to_owned(),
            leading_truncated: start > line_range.start,
            trailing_truncated: end < content_end,
        }))
    }

    pub fn encoded_bytes(&self) -> Result<Vec<u8>, PagedDocumentError> {
        let utf8 = self.document.serialized_bytes();
        match &self.encoding {
            TextEncoding::Utf8 { bom } => {
                if *bom && !utf8.starts_with(&[0xef, 0xbb, 0xbf]) {
                    let mut bytes = vec![0xef, 0xbb, 0xbf];
                    bytes.extend_from_slice(&utf8);
                    Ok(bytes)
                } else {
                    Ok(utf8)
                }
            }
            TextEncoding::Utf16Le | TextEncoding::Utf16Be => {
                let text = String::from_utf8(utf8).map_err(|_| PagedDocumentError::Binary)?;
                let little_endian = matches!(self.encoding, TextEncoding::Utf16Le);
                let mut bytes = if little_endian {
                    vec![0xff, 0xfe]
                } else {
                    vec![0xfe, 0xff]
                };
                for unit in text.encode_utf16() {
                    let encoded = if little_endian {
                        unit.to_le_bytes()
                    } else {
                        unit.to_be_bytes()
                    };
                    bytes.extend_from_slice(&encoded);
                }
                Ok(bytes)
            }
            TextEncoding::Legacy(label) => {
                let encoding = encoding_rs::Encoding::for_label(label.as_bytes())
                    .ok_or_else(|| PagedDocumentError::UnsupportedEncoding(label.clone()))?;
                let text = String::from_utf8(utf8).map_err(|_| PagedDocumentError::Binary)?;
                let (bytes, _, had_errors) = encoding.encode(&text);
                if had_errors {
                    return Err(PagedDocumentError::UnrepresentableEncoding {
                        encoding: label.clone(),
                    });
                }
                Ok(bytes.into_owned())
            }
        }
    }

    fn rebuild_lines(&mut self) {
        let (lines, structural_units) = build_source_metrics(&self.document.text());
        self.lines = lines;
        self.structural_units = structural_units;
    }
}

fn build_source_metrics(text: &str) -> (Arc<[Range<u64>]>, u64) {
    let mut ranges = Vec::new();
    let mut start = 0usize;
    let mut structural_units = 0u64;
    for (offset, byte) in text.bytes().enumerate() {
        if byte == b'\n' {
            ranges.push(start as u64..(offset + 1) as u64);
            start = offset + 1;
        }
        if matches!(byte, b'|' | b',' | b'\t' | b'{' | b'}' | b'[' | b']') {
            structural_units = structural_units.saturating_add(1);
        }
    }
    ranges.push(start as u64..text.len() as u64);
    (ranges.into(), structural_units)
}

fn search_match(range: Range<u64>) -> SearchMatch {
    SearchMatch {
        anchor: SourceAnchor::new(range.start, SourceAffinity::Before),
        head: SourceAnchor::new(range.end, SourceAffinity::After),
        range,
    }
}

fn map_document_error(message: String, len: u64, range: Range<u64>) -> PagedDocumentError {
    if message.contains("UTF-8") {
        PagedDocumentError::InvalidUtf8Boundary
    } else if range.start > range.end || range.end > len {
        PagedDocumentError::InvalidRange {
            start: range.start,
            end: range.end,
            len,
        }
    } else {
        PagedDocumentError::InvalidTransaction(message)
    }
}

#[cfg(test)]
#[path = "../tests/unit/resident.rs"]
mod tests;
