// @author kongweiguang

use std::path::Path;

use gmark_document_core::{
    DocumentBackendKind, DocumentProfile, LoadingPolicy, OpenPolicyResolver,
};
pub use gmark_document_core::{DocumentFormat, TextEncoding};

use crate::{FileSource, PagedDocumentError};

pub const DEFAULT_MAX_RESIDENT_BYTES: u64 =
    gmark_document_core::DEFAULT_LOADING_LIMITS.max_resident_bytes;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenStrategy {
    Resident,
    Paged,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProbeOptions {
    pub max_resident_bytes: u64,
    pub sample_bytes: usize,
}

impl Default for ProbeOptions {
    fn default() -> Self {
        Self {
            max_resident_bytes: DEFAULT_MAX_RESIDENT_BYTES,
            sample_bytes: 64 * 1024,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenProbe {
    pub len: u64,
    pub identity: crate::FileIdentity,
    pub options: ProbeOptions,
    pub format: DocumentFormat,
    pub encoding: TextEncoding,
    pub strategy: OpenStrategy,
    /// 单次“安全 Source”打开只属于本次 Probe，不写回全局偏好。
    pub force_safe_source: bool,
    pub estimated_lines: u64,
    pub estimated_structural_units: u64,
}

impl OpenProbe {
    pub fn profile(&self) -> DocumentProfile {
        DocumentProfile {
            len: self.len,
            format: self.format.clone(),
            encoding: self.encoding.clone(),
            estimated_lines: self.estimated_lines,
            estimated_structural_units: self.estimated_structural_units,
        }
    }
}

/// 只读取首尾样本以控制大文件打开成本；首样本边界可能切断字符，因此将其是否到达 EOF 传给编码判断。
pub fn probe_file(
    path: impl AsRef<Path>,
    options: ProbeOptions,
) -> Result<OpenProbe, PagedDocumentError> {
    let path = path.as_ref();
    let source = FileSource::open(path)?;
    let identity = source.identity()?;
    let sample_len = identity.len.min(options.sample_bytes as u64) as usize;
    let head = source.read_range(0, sample_len as u64)?;
    let head_is_prefix = identity.len > head.len() as u64;
    let tail_start = identity.len.saturating_sub(options.sample_bytes as u64);
    let tail = if tail_start > sample_len as u64 {
        source.read_range(tail_start, identity.len)?
    } else {
        Vec::new()
    };
    let encoding = detect_encoding(&head, &tail, head_is_prefix)?;
    let format = detect_format(path);
    let sampled = (head.len() + tail.len()).max(1) as u64;
    let newline_count = head
        .iter()
        .chain(&tail)
        .filter(|byte| **byte == b'\n')
        .count() as u64;
    let structural_count = head
        .iter()
        .chain(&tail)
        .filter(|byte| matches!(**byte, b'|' | b',' | b'\t' | b'{' | b'}' | b'[' | b']'))
        .count() as u64;
    let scale = identity.len.div_ceil(sampled);
    let estimated_lines = newline_count.saturating_add(1).saturating_mul(scale);
    let estimated_structural_units = structural_count.saturating_mul(scale);
    let profile = DocumentProfile {
        len: identity.len,
        format: format.clone(),
        encoding: encoding.clone(),
        estimated_lines,
        estimated_structural_units,
    };
    let plan = OpenPolicyResolver.resolve(
        LoadingPolicy {
            max_resident_bytes: Some(options.max_resident_bytes),
            force_safe_source: false,
        },
        &profile,
    );
    let strategy = match plan.backend {
        DocumentBackendKind::Resident => OpenStrategy::Resident,
        DocumentBackendKind::Paged => OpenStrategy::Paged,
    };
    Ok(OpenProbe {
        len: identity.len,
        identity,
        options,
        format,
        encoding,
        strategy,
        force_safe_source: false,
        estimated_lines,
        estimated_structural_units,
    })
}

fn detect_format(path: &Path) -> DocumentFormat {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match extension.as_str() {
        "md" | "markdown" => DocumentFormat::Markdown,
        "json" => DocumentFormat::Json,
        "jsonl" | "ndjson" => DocumentFormat::JsonLines,
        "csv" => DocumentFormat::Delimited { delimiter: b',' },
        "tsv" | "tab" => DocumentFormat::Delimited { delimiter: b'\t' },
        _ => DocumentFormat::PlainText,
    }
}

/// 仅忽略非 EOF head 样本末尾的不完整 UTF-8 序列，避免把采样器截断误认成文件编码；样本内错误仍回退。
fn detect_encoding(
    head: &[u8],
    tail: &[u8],
    head_is_prefix: bool,
) -> Result<TextEncoding, PagedDocumentError> {
    if head.starts_with(&[0xef, 0xbb, 0xbf]) {
        return Ok(TextEncoding::Utf8 { bom: true });
    }
    if head.starts_with(&[0xff, 0xfe]) {
        return Ok(TextEncoding::Utf16Le);
    }
    if head.starts_with(&[0xfe, 0xff]) {
        return Ok(TextEncoding::Utf16Be);
    }
    if !looks_like_text_sample(head, tail) {
        return Err(PagedDocumentError::Binary);
    }
    let head_is_utf8 = match std::str::from_utf8(head) {
        Ok(_) => true,
        Err(error) => head_is_prefix && error.error_len().is_none(),
    };
    if head_is_utf8 && tail_is_utf8(tail) {
        return Ok(TextEncoding::Utf8 { bom: false });
    }
    let mut detector = chardetng::EncodingDetector::new();
    detector.feed(head, tail.is_empty());
    if !tail.is_empty() {
        detector.feed(tail, true);
    }
    let encoding = detector.guess(None, true);
    Ok(TextEncoding::Legacy(encoding.name().to_owned()))
}

/// BOM 已在调用方处理；剩余文本不应包含 NUL。其它 C0/DEL 控制字允许极少量，
/// 兼容日志中的偶发控制符，同时拒绝压缩包、图片和可执行文件常见的控制字密度。
fn looks_like_text_sample(head: &[u8], tail: &[u8]) -> bool {
    let mut sampled = 0usize;
    let mut controls = 0usize;
    for byte in head.iter().chain(tail) {
        sampled = sampled.saturating_add(1);
        if *byte == 0 {
            return false;
        }
        if matches!(*byte, 0x01..=0x08 | 0x0b..=0x0c | 0x0e..=0x1f | 0x7f) {
            controls = controls.saturating_add(1);
        }
    }
    controls <= (sampled / 100).max(1)
}

fn tail_is_utf8(tail: &[u8]) -> bool {
    if tail.is_empty() {
        return true;
    }
    let first_boundary = tail
        .iter()
        .position(|byte| byte & 0b1100_0000 != 0b1000_0000)
        .unwrap_or(tail.len());
    std::str::from_utf8(&tail[first_boundary..]).is_ok()
}
