// @author kongweiguang

use std::io::Write;
use std::ops::Range;
use std::path::Path;

use super::super::*;

impl PieceDocument {
    pub fn save_atomic(&mut self, path: impl AsRef<Path>) -> Result<(), PagedDocumentError> {
        self.save_atomic_cancellable(path, &SearchCancellation::default())
    }

    /// 取消令牌覆盖替换等待；只有替换成功后才发布新的 base source。
    pub fn save_atomic_cancellable(
        &mut self,
        path: impl AsRef<Path>,
        cancellation: &SearchCancellation,
    ) -> Result<(), PagedDocumentError> {
        let path = path.as_ref();
        if path == self.source()?.path() && self.source()?.identity()? != self.base_identity {
            return Err(PagedDocumentError::SourceChanged);
        }
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        let mut temporary =
            tempfile::NamedTempFile::new_in(parent).map_err(|source| PagedDocumentError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        const COPY_CHUNK: u64 = 8 * 1024 * 1024;
        let mut offset = 0u64;
        while offset < self.len {
            if cancellation.is_cancelled() {
                return Err(PagedDocumentError::Cancelled);
            }
            let end = (offset + COPY_CHUNK).min(self.len);
            let bytes = self.read_range(offset..end)?;
            temporary
                .write_all(&bytes)
                .map_err(|source| PagedDocumentError::Io {
                    path: temporary.path().to_path_buf(),
                    source,
                })?;
            offset = end;
        }
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
        // 写临时文件可能持续数分钟；替换前必须再次核验 live identity，不能用
        // 保存开始时的检查覆盖期间发生的外部修改。
        if path == self.source()?.path() && self.source()?.identity()? != self.base_identity {
            return Err(PagedDocumentError::SourceChanged);
        }
        if cancellation.is_cancelled() {
            return Err(PagedDocumentError::Cancelled);
        }
        // Windows 目标被当前进程持有时无法原子替换；所有 base piece 已写完，可安全关闭句柄。
        self.source.take();
        if let Err(error) = crate::source::persist_temporary(temporary, path, cancellation) {
            self.source = FileSource::open(path).ok();
            return Err(error);
        }
        crate::source::sync_parent_directory(parent)?;

        let source = FileSource::open(path)?;
        let index = LineIndex::build(&source)?;
        self.base_identity = source.identity()?;
        self.base_sample = source.sampled_prefix_hash(self.len)?;
        self.source = Some(source);
        self.base_index = index.clone();
        self.pieces = PieceTree::from_iter((self.len > 0).then_some(Piece {
            source: PieceSource::Base,
            range: 0..self.len,
            newlines: index.newline_count(),
        }));
        self.additions = AppendStore::default();
        self.persisted_pieces = None;
        self.undo.clear();
        self.redo.clear();
        Ok(())
    }

    /// 将源码选区流式导出到独立文件；不物化完整选区，也不改变文档 pristine/history。
    /// 将取消传递到最后的替换边界，避免取消后的范围导出仍发布暂存文件。
    pub fn save_range_atomic_cancellable(
        &self,
        range: Range<u64>,
        path: impl AsRef<Path>,
        cancellation: &SearchCancellation,
    ) -> Result<(), PagedDocumentError> {
        if range.start > range.end || range.end > self.len {
            return Err(PagedDocumentError::InvalidRange {
                start: range.start,
                end: range.end,
                len: self.len,
            });
        }
        let path = path.as_ref();
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        let mut temporary =
            tempfile::NamedTempFile::new_in(parent).map_err(|source| PagedDocumentError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        const COPY_CHUNK: u64 = 8 * 1024 * 1024;
        let mut offset = range.start;
        while offset < range.end {
            if cancellation.is_cancelled() {
                return Err(PagedDocumentError::Cancelled);
            }
            let end = offset.saturating_add(COPY_CHUNK).min(range.end);
            let bytes = self.read_range(offset..end)?;
            temporary
                .write_all(&bytes)
                .map_err(|source| PagedDocumentError::Io {
                    path: temporary.path().to_path_buf(),
                    source,
                })?;
            offset = end;
        }
        temporary
            .as_file()
            .sync_all()
            .map_err(|source| PagedDocumentError::Io {
                path: temporary.path().to_path_buf(),
                source,
            })?;
        crate::source::persist_temporary(temporary, path, cancellation)?;
        crate::source::sync_parent_directory(parent)
    }
}
