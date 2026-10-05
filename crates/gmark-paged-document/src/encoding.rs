// @author kongweiguang

//! 非 UTF-8 大文本的流式 UTF-8 影子与原编码原子保存。

use std::io::Write;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use encoding_rs::{CoderResult, Encoding, UTF_16BE, UTF_16LE};
use gmark_document_core::DocumentSnapshot;

use crate::{FileIdentity, FileSource, PagedDocumentError, PieceDocument, TextEncoding};

const TRANSCODE_BLOCK_BYTES: u64 = 8 * 1024 * 1024;

pub struct PreparedUtf8Source {
    source: FileSource,
    encoding: TextEncoding,
    _shadow: Option<tempfile::NamedTempFile>,
    save_plan: Option<EncodedSavePlan>,
}

#[derive(Clone, Debug)]
pub struct EncodedSavePlan {
    encoding: TextEncoding,
    original_identity: FileIdentity,
}

impl PreparedUtf8Source {
    pub fn source(&self) -> &FileSource {
        &self.source
    }

    pub fn save_plan(&self) -> Option<EncodedSavePlan> {
        self.save_plan.clone()
    }

    pub fn encoding(&self) -> &TextEncoding {
        &self.encoding
    }

    pub fn mark_original_saved(&mut self, identity: FileIdentity) {
        if let Some(plan) = self.save_plan.as_mut() {
            plan.original_identity = identity;
        }
    }

    pub(crate) fn into_backend_parts(
        self,
    ) -> (
        FileSource,
        TextEncoding,
        Option<Arc<tempfile::NamedTempFile>>,
        Option<EncodedSavePlan>,
    ) {
        (
            self.source,
            self.encoding,
            self._shadow.map(Arc::new),
            self.save_plan,
        )
    }
}

impl EncodedSavePlan {
    pub fn encoding(&self) -> &TextEncoding {
        &self.encoding
    }

    pub fn original_identity(&self) -> &FileIdentity {
        &self.original_identity
    }

    pub(crate) fn new(encoding: TextEncoding, original_identity: FileIdentity) -> Self {
        Self {
            encoding,
            original_identity,
        }
    }

    pub(crate) fn set_encoding(&mut self, encoding: TextEncoding) {
        self.encoding = encoding;
    }

    pub(crate) fn mark_original_saved(&mut self, identity: FileIdentity) {
        self.original_identity = identity;
    }

    pub fn save_atomic(
        &self,
        document: &PieceDocument,
        path: impl AsRef<Path>,
    ) -> Result<FileIdentity, PagedDocumentError> {
        self.save_atomic_cancellable(document, path, &crate::SearchCancellation::default())
    }

    pub fn save_atomic_cancellable(
        &self,
        document: &PieceDocument,
        path: impl AsRef<Path>,
        cancellation: &crate::SearchCancellation,
    ) -> Result<FileIdentity, PagedDocumentError> {
        let path = path.as_ref();
        let current = FileSource::open(path)?.identity()?;
        if current != self.original_identity {
            return Err(PagedDocumentError::SourceChanged);
        }
        self.save_atomic_inner(document, path, Some(&self.original_identity), cancellation)
    }

    /// Stream an immutable UTF-8 snapshot through this plan's encoding and
    /// atomically replace the original target.  The snapshot is the only
    /// source consulted after the call starts; no mutable PieceDocument or
    /// Controller state is read by the writer.
    pub fn save_snapshot_atomic_cancellable(
        &self,
        snapshot: &dyn DocumentSnapshot,
        path: impl AsRef<Path>,
        cancellation: &crate::SearchCancellation,
    ) -> Result<FileIdentity, PagedDocumentError> {
        let expected = self.original_identity.clone();
        crate::source::atomic_write_stream(
            path,
            Some(&expected),
            cancellation,
            |output, cancellation| self.write_snapshot(snapshot, output, cancellation),
        )
    }

    /// Stream an immutable snapshot to a Save As target without checking the
    /// source identity.  Existing target contents are replaced atomically.
    pub fn save_snapshot_atomic_as_cancellable(
        &self,
        snapshot: &dyn DocumentSnapshot,
        path: impl AsRef<Path>,
        cancellation: &crate::SearchCancellation,
    ) -> Result<FileIdentity, PagedDocumentError> {
        crate::source::atomic_write_stream(path, None, cancellation, |output, cancellation| {
            self.write_snapshot(snapshot, output, cancellation)
        })
    }

    pub fn save_atomic_as(
        &self,
        document: &PieceDocument,
        path: impl AsRef<Path>,
    ) -> Result<FileIdentity, PagedDocumentError> {
        self.save_atomic_as_cancellable(document, path, &crate::SearchCancellation::default())
    }

    pub fn save_atomic_as_cancellable(
        &self,
        document: &PieceDocument,
        path: impl AsRef<Path>,
        cancellation: &crate::SearchCancellation,
    ) -> Result<FileIdentity, PagedDocumentError> {
        self.save_atomic_inner(document, path.as_ref(), None, cancellation)
    }

    /// 将取消令牌传入最终替换阶段，避免 Windows 锁等待后提交过期的范围导出。
    pub fn save_range_atomic_as_cancellable(
        &self,
        document: &PieceDocument,
        range: Range<u64>,
        path: impl AsRef<Path>,
        cancellation: &crate::SearchCancellation,
    ) -> Result<FileIdentity, PagedDocumentError> {
        let path = path.as_ref();
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        let mut temporary =
            tempfile::NamedTempFile::new_in(parent).map_err(|source| PagedDocumentError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        self.write_range(document, range, &mut temporary, cancellation)?;
        temporary
            .as_file()
            .sync_all()
            .map_err(|source| PagedDocumentError::Io {
                path: temporary.path().to_path_buf(),
                source,
            })?;
        if let Ok(metadata) = std::fs::metadata(path) {
            temporary
                .as_file()
                .set_permissions(metadata.permissions())
                .map_err(|source| PagedDocumentError::Io {
                    path: temporary.path().to_path_buf(),
                    source,
                })?;
        }
        if cancellation.is_cancelled() {
            return Err(PagedDocumentError::Cancelled);
        }
        crate::source::persist_temporary(temporary, path, cancellation)?;
        crate::source::sync_parent_directory(parent)?;
        FileSource::open(path)?.identity()
    }

    /// 同一取消令牌覆盖原子替换退避，因为锁等待仍属于本次保存事务。
    fn save_atomic_inner(
        &self,
        document: &PieceDocument,
        path: &Path,
        expected_identity: Option<&FileIdentity>,
        cancellation: &crate::SearchCancellation,
    ) -> Result<FileIdentity, PagedDocumentError> {
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        let mut temporary =
            tempfile::NamedTempFile::new_in(parent).map_err(|source| PagedDocumentError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        self.write_range(document, 0..document.len(), &mut temporary, cancellation)?;
        temporary
            .as_file()
            .sync_all()
            .map_err(|source| PagedDocumentError::Io {
                path: temporary.path().to_path_buf(),
                source,
            })?;
        if let Ok(metadata) = std::fs::metadata(path) {
            temporary
                .as_file()
                .set_permissions(metadata.permissions())
                .map_err(|source| PagedDocumentError::Io {
                    path: temporary.path().to_path_buf(),
                    source,
                })?;
        }
        if let Some(expected) = expected_identity
            && FileSource::open(path)?.identity()? != *expected
        {
            return Err(PagedDocumentError::SourceChanged);
        }
        if cancellation.is_cancelled() {
            return Err(PagedDocumentError::Cancelled);
        }
        crate::source::persist_temporary(temporary, path, cancellation)?;
        crate::source::sync_parent_directory(parent)?;
        FileSource::open(path)?.identity()
    }

    fn write_range(
        &self,
        document: &PieceDocument,
        range: Range<u64>,
        temporary: &mut tempfile::NamedTempFile,
        cancellation: &crate::SearchCancellation,
    ) -> Result<(), PagedDocumentError> {
        write_bom(temporary, &self.encoding)?;
        match &self.encoding {
            TextEncoding::Utf16Le => {
                document.for_each_utf8_range_chunk(range, TRANSCODE_BLOCK_BYTES, |bytes| {
                    if cancellation.is_cancelled() {
                        return Err(PagedDocumentError::Cancelled);
                    }
                    write_utf16(temporary, bytes, true)
                })?;
            }
            TextEncoding::Utf16Be => {
                document.for_each_utf8_range_chunk(range, TRANSCODE_BLOCK_BYTES, |bytes| {
                    if cancellation.is_cancelled() {
                        return Err(PagedDocumentError::Cancelled);
                    }
                    write_utf16(temporary, bytes, false)
                })?;
            }
            _ => {
                let encoding = resolve_encoding(&self.encoding)?;
                let mut writer = EncodingWriter::new(temporary, encoding, self.encoding_name());
                document.for_each_utf8_range_chunk(range, TRANSCODE_BLOCK_BYTES, |bytes| {
                    if cancellation.is_cancelled() {
                        return Err(PagedDocumentError::Cancelled);
                    }
                    let text = std::str::from_utf8(bytes)
                        .map_err(|_| PagedDocumentError::InvalidUtf8Boundary)?;
                    writer.encode(text, false)
                })?;
                writer.finish()?;
            }
        }
        Ok(())
    }

    fn write_snapshot(
        &self,
        snapshot: &dyn DocumentSnapshot,
        output: &mut dyn Write,
        cancellation: &crate::SearchCancellation,
    ) -> Result<(), PagedDocumentError> {
        write_bom(output, &self.encoding)?;
        match &self.encoding {
            TextEncoding::Utf16Le => stream_snapshot_utf8(snapshot, cancellation, |text, _| {
                write_utf16(output, text.as_bytes(), true)
            }),
            TextEncoding::Utf16Be => stream_snapshot_utf8(snapshot, cancellation, |text, _| {
                write_utf16(output, text.as_bytes(), false)
            }),
            TextEncoding::Utf8 { .. } => stream_snapshot_utf8(snapshot, cancellation, |text, _| {
                output
                    .write_all(text.as_bytes())
                    .map_err(|source| PagedDocumentError::Io {
                        path: std::env::temp_dir(),
                        source,
                    })
            }),
            TextEncoding::Legacy(label) => {
                let mut encoder =
                    EncodingWriter::new(output, resolve_encoding(&self.encoding)?, label.clone());
                stream_snapshot_utf8(snapshot, cancellation, |text, last| {
                    if cancellation.is_cancelled() {
                        return Err(PagedDocumentError::Cancelled);
                    }
                    encoder.encode(text, last)
                })?;
                Ok(())
            }
        }
    }

    pub fn encoding_name(&self) -> String {
        match &self.encoding {
            TextEncoding::Utf16Le => "UTF-16LE".to_owned(),
            TextEncoding::Utf16Be => "UTF-16BE".to_owned(),
            TextEncoding::Legacy(label) => label.clone(),
            TextEncoding::Utf8 { .. } => "UTF-8".to_owned(),
        }
    }
}

/// Visit a snapshot as valid UTF-8 chunks while carrying at most one codepoint
/// across read boundaries.  This keeps encoders streaming even when a backend
/// returns ranges that split a multibyte character.
fn stream_snapshot_utf8(
    snapshot: &dyn DocumentSnapshot,
    cancellation: &crate::SearchCancellation,
    mut visit: impl FnMut(&str, bool) -> Result<(), PagedDocumentError>,
) -> Result<(), PagedDocumentError> {
    let mut offset = 0_u64;
    let mut carry = Vec::new();
    let len = snapshot.len();
    while offset < len {
        if cancellation.is_cancelled() {
            return Err(PagedDocumentError::Cancelled);
        }
        let end = offset.saturating_add(TRANSCODE_BLOCK_BYTES).min(len);
        let bytes = snapshot
            .read_range(offset..end)
            .map_err(|error| PagedDocumentError::InvalidTransaction(error.to_string()))?;
        offset = end;
        carry.extend_from_slice(&bytes);
        match std::str::from_utf8(&carry) {
            Ok(text) => {
                visit(text, false)?;
                carry.clear();
            }
            Err(error) if error.error_len().is_none() => {
                let valid_up_to = error.valid_up_to();
                if valid_up_to > 0 {
                    let text = std::str::from_utf8(&carry[..valid_up_to])
                        .map_err(|_| PagedDocumentError::InvalidUtf8Boundary)?;
                    visit(text, false)?;
                    carry.drain(..valid_up_to);
                }
            }
            Err(_) => return Err(PagedDocumentError::InvalidUtf8Boundary),
        }
    }
    if !carry.is_empty() {
        let text =
            std::str::from_utf8(&carry).map_err(|_| PagedDocumentError::InvalidUtf8Boundary)?;
        visit(text, true)?;
    } else {
        visit("", true)?;
    }
    Ok(())
}

pub fn prepare_utf8_source(
    original: FileSource,
    encoding: TextEncoding,
) -> Result<PreparedUtf8Source, PagedDocumentError> {
    if matches!(&encoding, TextEncoding::Utf8 { .. }) {
        return Ok(PreparedUtf8Source {
            source: original,
            encoding,
            _shadow: None,
            save_plan: None,
        });
    }
    let identity = original.identity()?;
    let decoder_encoding = resolve_encoding(&encoding)?;
    let mut decoder = decoder_encoding.new_decoder_with_bom_removal();
    let mut shadow = tempfile::NamedTempFile::new().map_err(|source| PagedDocumentError::Io {
        path: std::env::temp_dir(),
        source,
    })?;
    let mut offset = 0u64;
    while offset < identity.len {
        let end = (offset + TRANSCODE_BLOCK_BYTES).min(identity.len);
        let input = original.read_range(offset, end)?;
        let shadow_path = shadow.path().to_path_buf();
        decode_block(
            &mut decoder,
            &input,
            end == identity.len,
            &mut shadow,
            shadow_path,
        )?;
        offset = end;
    }
    if identity.len == 0 {
        let shadow_path = shadow.path().to_path_buf();
        decode_block(&mut decoder, &[], true, &mut shadow, shadow_path)?;
    }
    shadow.flush().map_err(|source| PagedDocumentError::Io {
        path: shadow.path().to_path_buf(),
        source,
    })?;
    let source = FileSource::open(shadow.path())?;
    Ok(PreparedUtf8Source {
        source,
        encoding: encoding.clone(),
        _shadow: Some(shadow),
        save_plan: Some(EncodedSavePlan {
            encoding,
            original_identity: identity,
        }),
    })
}

fn resolve_encoding(encoding: &TextEncoding) -> Result<&'static Encoding, PagedDocumentError> {
    match encoding {
        TextEncoding::Utf8 { .. } => Ok(encoding_rs::UTF_8),
        TextEncoding::Utf16Le => Ok(UTF_16LE),
        TextEncoding::Utf16Be => Ok(UTF_16BE),
        TextEncoding::Legacy(label) => Encoding::for_label(label.as_bytes())
            .ok_or_else(|| PagedDocumentError::UnsupportedEncoding(label.clone())),
    }
}

fn decode_block(
    decoder: &mut encoding_rs::Decoder,
    mut input: &[u8],
    last: bool,
    output: &mut impl Write,
    output_path: PathBuf,
) -> Result<(), PagedDocumentError> {
    let mut buffer = vec![0u8; 256 * 1024];
    loop {
        let (result, read, written, had_errors) = decoder.decode_to_utf8(input, &mut buffer, last);
        if had_errors {
            return Err(PagedDocumentError::Binary);
        }
        output
            .write_all(&buffer[..written])
            .map_err(|source| PagedDocumentError::Io {
                path: output_path.clone(),
                source,
            })?;
        input = &input[read..];
        if matches!(result, CoderResult::InputEmpty) {
            return Ok(());
        }
    }
}

fn write_bom<W: Write + ?Sized>(
    output: &mut W,
    encoding: &TextEncoding,
) -> Result<(), PagedDocumentError> {
    let bom: &[u8] = match encoding {
        TextEncoding::Utf16Le => &[0xff, 0xfe],
        TextEncoding::Utf16Be => &[0xfe, 0xff],
        TextEncoding::Utf8 { bom: true } => &[0xef, 0xbb, 0xbf],
        _ => &[],
    };
    output
        .write_all(bom)
        .map_err(|source| PagedDocumentError::Io {
            path: std::env::temp_dir(),
            source,
        })
}

fn write_utf16<W: Write + ?Sized>(
    output: &mut W,
    bytes: &[u8],
    little_endian: bool,
) -> Result<(), PagedDocumentError> {
    let text = std::str::from_utf8(bytes).map_err(|_| PagedDocumentError::InvalidUtf8Boundary)?;
    let mut encoded = Vec::with_capacity(text.len().saturating_mul(2));
    for unit in text.encode_utf16() {
        let bytes = if little_endian {
            unit.to_le_bytes()
        } else {
            unit.to_be_bytes()
        };
        encoded.extend_from_slice(&bytes);
    }
    output
        .write_all(&encoded)
        .map_err(|source| PagedDocumentError::Io {
            path: std::env::temp_dir(),
            source,
        })
}

struct EncodingWriter<'a, W: ?Sized> {
    output: &'a mut W,
    encoder: encoding_rs::Encoder,
    encoding_name: String,
}

impl<'a, W: Write + ?Sized> EncodingWriter<'a, W> {
    fn new(output: &'a mut W, encoding: &'static Encoding, encoding_name: String) -> Self {
        Self {
            output,
            encoder: encoding.new_encoder(),
            encoding_name,
        }
    }

    fn finish(&mut self) -> Result<(), PagedDocumentError> {
        self.encode("", true)
    }

    fn encode(&mut self, mut input: &str, last: bool) -> Result<(), PagedDocumentError> {
        let mut buffer = vec![0u8; 256 * 1024];
        loop {
            let (result, read, written, had_errors) =
                self.encoder.encode_from_utf8(input, &mut buffer, last);
            if had_errors {
                return Err(PagedDocumentError::UnrepresentableEncoding {
                    encoding: self.encoding_name.clone(),
                });
            }
            self.output
                .write_all(&buffer[..written])
                .map_err(|source| PagedDocumentError::Io {
                    path: std::env::temp_dir(),
                    source,
                })?;
            input = &input[read..];
            if matches!(result, CoderResult::InputEmpty) {
                return Ok(());
            }
        }
    }
}
