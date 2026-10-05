// @author kongweiguang

//! 异步缓存水平窗口前的精确 Unicode 列号，避免长行扫描进入绘制线程。

use super::source_boundaries::resolve_grapheme_prefix_for_window;
use super::*;
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SourceCursorColumnKey {
    document_epoch: u64,
    revision: u64,
    line: usize,
    window_start: u64,
}

#[derive(Clone, Copy)]
struct CachedSourceCursorPrefix {
    key: SourceCursorColumnKey,
    grapheme_count: usize,
    resume_offset: u64,
}

pub(super) struct SourceCursorColumnState {
    key: Option<SourceCursorColumnKey>,
    cached: Option<CachedSourceCursorPrefix>,
    generation: u64,
    cancellation: Option<SearchCancellation>,
    task: Task<()>,
}

impl Default for SourceCursorColumnState {
    /// 仅保留一个可替换任务，限制活动行或文档版本变化时的后台工作量。
    fn default() -> Self {
        Self {
            key: None,
            cached: None,
            generation: 0,
            cancellation: None,
            task: Task::ready(()),
        }
    }
}

impl SourceCursorColumnState {
    /// 目标变化时取消旧扫描并清除缓存键，避免迟到结果被后续绘制复用。
    pub(super) fn reset(&mut self) {
        if self.key.is_none() && self.cancellation.is_none() && self.cached.is_none() {
            return;
        }
        self.generation = self.generation.wrapping_add(1);
        if let Some(cancellation) = self.cancellation.take() {
            cancellation.cancel();
        }
        self.key = None;
        self.cached = None;
        self.task = Task::ready(());
    }

    /// 记住当前不可扫描的目标，避免每帧重复尝试同一失败路径。
    fn mark_unavailable(&mut self, key: SourceCursorColumnKey) {
        if self.key == Some(key) {
            return;
        }
        self.reset();
        self.key = Some(key);
    }

    /// 每个行版本和水平窗口最多启动一次有界内存扫描。
    fn request(
        &mut self,
        key: SourceCursorColumnKey,
        snapshot: Arc<dyn DocumentSnapshot>,
        line_range: Range<u64>,
        cx: &mut Context<DocumentHost>,
    ) {
        if self.key == Some(key) {
            return;
        }
        self.reset();
        self.key = Some(key);
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        let cancellation = SearchCancellation::default();
        let worker_cancellation = cancellation.clone();
        self.cancellation = Some(cancellation);
        let work = cx.background_executor().spawn(async move {
            resolve_grapheme_prefix_for_window(
                snapshot.as_ref(),
                &line_range,
                key.window_start,
                &worker_cancellation,
            )
        });
        self.task = cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, move |host, cx| {
                host.install_source_cursor_prefix(key, generation, result, cx);
            });
        });
    }

    /// 只返回与当前文档、行版本和窗口完全匹配的前缀结果。
    fn cached(&self, key: SourceCursorColumnKey) -> Option<CachedSourceCursorPrefix> {
        self.cached.filter(|cached| cached.key == key)
    }
}

impl DocumentHost {
    /// 仅由 Source 状态栏请求昂贵的行首扫描；键未变化时复用已有结果。
    pub(crate) fn prepare_cursor_position(&mut self, cx: &mut Context<Self>) {
        let Some(document) = self.document.clone() else {
            self.source_cursor_column.reset();
            return;
        };
        let Some(active) = self.active_edit.as_ref() else {
            self.source_cursor_column.reset();
            return;
        };
        let key = SourceCursorColumnKey {
            document_epoch: self.document_epoch,
            revision: active.base_revision,
            line: active.line,
            window_start: active.range.start,
        };
        if active.base_revision != document.revision() {
            self.source_cursor_column.reset();
            return;
        }
        let Some(line_range) = document.line_range(active.line as u64) else {
            self.source_cursor_column.mark_unavailable(key);
            return;
        };
        if active.range.start < line_range.start || active.range.start > line_range.end {
            self.source_cursor_column.mark_unavailable(key);
            return;
        }
        if active.range.start == line_range.start {
            self.source_cursor_column.reset();
            return;
        }
        if self.source_cursor_column.key == Some(key) {
            return;
        }
        let snapshot = match document.snapshot() {
            Ok(snapshot) if snapshot.revision().0 == key.revision => snapshot,
            _ => {
                self.source_cursor_column.mark_unavailable(key);
                return;
            }
        };
        self.source_cursor_column
            .request(key, snapshot, line_range, cx);
    }

    /// 只有活动输入面仍拥有该版本、行和窗口时才安装后台结果。
    fn install_source_cursor_prefix(
        &mut self,
        key: SourceCursorColumnKey,
        generation: u64,
        result: Result<(usize, u64), PagedDocumentError>,
        cx: &mut Context<Self>,
    ) {
        if self.source_cursor_column.generation != generation
            || self.source_cursor_column.key != Some(key)
        {
            return;
        }
        self.source_cursor_column.cancellation = None;
        self.source_cursor_column.task = Task::ready(());
        let current_key = self
            .active_edit
            .as_ref()
            .map(|active| SourceCursorColumnKey {
                document_epoch: self.document_epoch,
                revision: active.base_revision,
                line: active.line,
                window_start: active.range.start,
            });
        let current_revision = self.document.as_ref().map(SharedDocument::revision);
        if current_key != Some(key) || current_revision != Some(key.revision) {
            self.source_cursor_column.reset();
            return;
        }
        if let Ok((grapheme_count, resume_offset)) = result
            && resume_offset >= key.window_start
        {
            self.source_cursor_column.cached = Some(CachedSourceCursorPrefix {
                key,
                grapheme_count,
                resume_offset,
            });
        }
        cx.notify();
    }

    /// 缓存未就绪时暂不报告列号，避免显示看似合理但错误的位置。
    pub(crate) fn cursor_position(&self, cx: &App) -> Option<(usize, usize)> {
        if let Some(active) = &self.active_edit {
            let document = self.document.as_ref()?;
            if document.revision() != active.base_revision {
                return None;
            }
            let line_range = document.line_range(active.line as u64)?;
            if active.range.start < line_range.start || active.range.start > line_range.end {
                return None;
            }
            let block = active.block.read(cx);
            let text = block.display_text();
            let offset = block.cursor_offset().min(text.len());
            if !text.is_char_boundary(offset) {
                return None;
            }
            let prefix_count = if active.range.start == line_range.start {
                0usize
            } else {
                let key = SourceCursorColumnKey {
                    document_epoch: self.document_epoch,
                    revision: active.base_revision,
                    line: active.line,
                    window_start: active.range.start,
                };
                let cached = self.source_cursor_column.cached(key)?;
                let local_start =
                    usize::try_from(cached.resume_offset.saturating_sub(active.range.start))
                        .ok()?;
                if local_start > offset || !text.is_char_boundary(local_start) {
                    return None;
                }
                let local_count = text[local_start..offset].graphemes(true).count();
                return Some((
                    active.line.saturating_add(1),
                    cached
                        .grapheme_count
                        .saturating_add(local_count)
                        .saturating_add(1),
                ));
            };
            let local_count = text[..offset].graphemes(true).count();
            return Some((
                active.line.saturating_add(1),
                prefix_count.saturating_add(local_count).saturating_add(1),
            ));
        }
        let line = self
            .selected_lines
            .as_ref()
            .map_or(0, |selection| selection.start)
            .saturating_add(1);
        Some((line, 1))
    }
}
