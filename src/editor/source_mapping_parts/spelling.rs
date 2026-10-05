// @author kongweiguang

use super::super::*;
use gmark_markdown::{VisibleTextProjection, VisibleTextSegment};

/// 只缓存源码拼写之间的导航映射；编辑替换仍由既有块/源码事务决定范围。
pub(in crate::editor) struct SourceSpellingMap {
    source: String,
    canonical: String,
    source_projection: VisibleTextProjection,
    canonical_projection: VisibleTextProjection,
    common_prefix: usize,
    common_suffix: usize,
}

impl SourceSpellingMap {
    /// 语义文字必须完全一致，不能把尚未发布的编辑或不同解析结果当成拼写差异。
    fn new(source: String, canonical: String) -> Self {
        let source_projection = gmark_markdown::parse_markdown(&source).visible_text_projection();
        let canonical_projection =
            gmark_markdown::parse_markdown(&canonical).visible_text_projection();
        let common_prefix = source
            .chars()
            .zip(canonical.chars())
            .take_while(|(left, right)| left == right)
            .map(|(ch, _)| ch.len_utf8())
            .sum();
        let common_suffix = source[common_prefix..]
            .chars()
            .rev()
            .zip(canonical[common_prefix..].chars().rev())
            .take_while(|(left, right)| left == right)
            .map(|(ch, _)| ch.len_utf8())
            .sum();
        Self {
            source,
            canonical,
            source_projection,
            canonical_projection,
            common_prefix,
            common_suffix,
        }
    }

    /// 完全相同的语法间隙也保持偏移，例如跳到标题行的 `#` 之前不能落在上一段末尾。
    fn unchanged_gap_offset(
        from: &str,
        from_projection: &VisibleTextProjection,
        to: &str,
        to_projection: &VisibleTextProjection,
        offset: usize,
    ) -> Option<usize> {
        let before = from_projection
            .segments
            .iter()
            .filter_map(|segment| {
                let source = Self::segment_source_range(from, from_projection, segment)?;
                (source.end <= offset).then_some((source.end, segment.visible.end))
            })
            .max_by_key(|(source, _)| *source)?;
        let after = from_projection
            .segments
            .iter()
            .filter_map(|segment| {
                let source = Self::segment_source_range(from, from_projection, segment)?;
                (offset <= source.start).then_some((source.start, segment.visible.start))
            })
            .min_by_key(|(source, _)| *source)?;
        let target_start = Self::source_offset(to, to_projection, before.1);
        let target_end = Self::source_offset(to, to_projection, after.1);
        (from.get(before.0..after.0)? == to.get(target_start..target_end)?)
            .then_some(target_start + offset.saturating_sub(before.0))
    }

    /// 精确片段按字节映射；代码包裹可剥离，实体等派生片段仅保留完整边界。
    fn segment_source_range(
        spelling: &str,
        projection: &VisibleTextProjection,
        segment: &VisibleTextSegment,
    ) -> Option<std::ops::Range<usize>> {
        let source = segment.source?;
        let raw = spelling.get(source.start..source.end)?;
        let visible = projection.text.get(segment.visible.clone())?;
        if raw == visible {
            return Some(source.start..source.end);
        }
        let relative = raw.find(visible)?;
        if raw[relative + visible.len()..].contains(visible) {
            return None;
        }
        Some(source.start + relative..source.start + relative + visible.len())
    }

    /// 选择端点按所属文字片段对齐，语法间隙使用最近边界，避免吞入相邻可见字符。
    fn visible_offset(spelling: &str, projection: &VisibleTextProjection, offset: usize) -> usize {
        projection
            .segments
            .iter()
            .filter_map(|segment| {
                let source = Self::segment_source_range(spelling, projection, segment)
                    .or_else(|| segment.source.map(|source| source.start..source.end))?;
                let distance = source
                    .start
                    .saturating_sub(offset)
                    .max(offset.saturating_sub(source.end));
                let visible = if source.len() == segment.visible.len() {
                    segment.visible.start + offset.saturating_sub(source.start).min(source.len())
                } else if offset <= source.start {
                    segment.visible.start
                } else {
                    segment.visible.end
                };
                Some((distance, visible))
            })
            .min_by_key(|(distance, _)| *distance)
            .map_or(0, |(_, visible)| visible)
    }

    /// 同一文字边界可属于两种格式；优先可见片段的精确边界，不用比例拆分 Unicode。
    fn source_offset(spelling: &str, projection: &VisibleTextProjection, offset: usize) -> usize {
        projection
            .segments
            .iter()
            .filter_map(|segment| {
                let source = Self::segment_source_range(spelling, projection, segment)
                    .or_else(|| segment.source.map(|source| source.start..source.end))?;
                let distance = segment
                    .visible
                    .start
                    .saturating_sub(offset)
                    .max(offset.saturating_sub(segment.visible.end));
                let mapped = if source.len() == segment.visible.len() {
                    source.start
                        + offset
                            .saturating_sub(segment.visible.start)
                            .min(source.len())
                } else if offset <= segment.visible.start {
                    source.start
                } else {
                    source.end
                };
                Some((distance, mapped))
            })
            .min_by_key(|(distance, _)| *distance)
            .map_or(0, |(_, mapped)| mapped)
    }

    /// 两端独立映射并保留方向；该快照仅恢复导航/历史，不授权直接替换派生文字。
    fn map(&self, snapshot: UndoSelectionSnapshot, to_source: bool) -> UndoSelectionSnapshot {
        if self.source_projection.text != self.canonical_projection.text {
            return snapshot;
        }
        let (from, from_projection, to, to_projection) = if to_source {
            (
                &self.canonical,
                &self.canonical_projection,
                &self.source,
                &self.source_projection,
            )
        } else {
            (
                &self.source,
                &self.source_projection,
                &self.canonical,
                &self.canonical_projection,
            )
        };
        let map_offset = |offset| {
            if offset <= self.common_prefix {
                return offset;
            }
            if offset >= from.len() {
                return to.len();
            }
            if offset >= from.len().saturating_sub(self.common_suffix) {
                return to.len() - (from.len() - offset);
            }
            if let Some(mapped) =
                Self::unchanged_gap_offset(from, from_projection, to, to_projection, offset)
            {
                return mapped;
            }
            let visible = Self::visible_offset(from, from_projection, offset);
            let mut mapped = Self::source_offset(to, to_projection, visible).min(to.len());
            while mapped > 0 && !to.is_char_boundary(mapped) {
                mapped -= 1;
            }
            mapped
        };
        let range = snapshot.range();
        let start = map_offset(range.start);
        let end = map_offset(range.end);
        UndoSelectionSnapshot::from_range(start.min(end)..start.max(end), snapshot.reversed())
    }
}

impl Editor {
    /// 同一区域的导航只解析其原文和规范拼写；跨区域快照仍使用完整语义映射。
    fn local_selection_spellings(
        &self,
        snapshot: UndoSelectionSnapshot,
        to_source: bool,
        cx: &App,
    ) -> Option<(String, String, usize, usize)> {
        let target = self.current_edit_target_from_state(cx)?;
        let (revision, source_range, roots) =
            self.document.source_region_for_entity(target.entity_id())?;
        if revision != self.source_document.revision() || self.pending_dirty_source.is_some() {
            return None;
        }
        let first_index = self
            .document
            .root_index_for_entity(roots.first()?.entity_id())?;
        let canonical_start = self.document.cached_root_source_start(first_index)?;
        let canonical = DocumentTree::markdown_text_for_roots(&roots, cx);
        let canonical_end = canonical_start.checked_add(canonical.len())?;
        let (from_range, to_start) = if to_source {
            (canonical_start..canonical_end, source_range.start)
        } else {
            (source_range.clone(), canonical_start)
        };
        let range = snapshot.range();
        if range.start < from_range.start || range.end > from_range.end {
            return None;
        }
        let source = self
            .source_document
            .snapshot()
            .text_for_range(source_range)
            .ok()?;
        Some((source, canonical, from_range.start, to_start))
    }

    /// 按原区域复用解析；整篇映射仅服务跨区域快照，普通长文输入不承担全文重解析。
    pub(in crate::editor) fn map_selection_source_spelling(
        &self,
        snapshot: UndoSelectionSnapshot,
        to_source: bool,
        cx: &App,
    ) -> UndoSelectionSnapshot {
        if self.virtual_surface.is_some() {
            return snapshot;
        }
        let local_spellings = self.local_selection_spellings(snapshot, to_source, cx);
        let is_local = local_spellings.is_some();
        let (source, canonical, from_start, to_start) = local_spellings.unwrap_or_else(|| {
            (
                self.pending_dirty_source
                    .clone()
                    .unwrap_or_else(|| self.source_document.text()),
                self.document.cached_markdown_text(cx),
                0,
                0,
            )
        });
        let range = snapshot.range();
        let local = UndoSelectionSnapshot::from_range(
            range.start - from_start..range.end - from_start,
            snapshot.reversed(),
        );
        // 仅已发布的完整投影可直接映射文档首尾；结构编辑的临时末尾仍需语义检查。
        let (from_len, to_len) = if to_source {
            (canonical.len(), source.len())
        } else {
            (source.len(), canonical.len())
        };
        if !is_local
            && [local.range().start, local.range().end]
                .into_iter()
                .all(|offset| offset == 0 || offset == from_len)
            && self.pending_dirty_source.is_none()
            && self
                .current_edit_target_from_state(cx)
                .and_then(|target| self.document.source_region_for_entity(target.entity_id()))
                .is_some_and(|(revision, _, _)| revision == self.source_document.revision())
        {
            let mapped = |offset| {
                if offset == 0 {
                    to_start
                } else {
                    to_start + to_len
                }
            };
            return UndoSelectionSnapshot::from_range(
                mapped(local.range().start)..mapped(local.range().end),
                local.reversed(),
            );
        }
        if canonical == source {
            return UndoSelectionSnapshot::from_range(
                local.range().start + to_start..local.range().end + to_start,
                local.reversed(),
            );
        }
        let Ok(mut cache) = self.source_spelling_cache.try_borrow_mut() else {
            return snapshot;
        };
        if cache
            .as_ref()
            .is_none_or(|cached| cached.source != source || cached.canonical != canonical)
        {
            *cache = Some(SourceSpellingMap::new(source, canonical));
        }
        if cache
            .as_ref()
            .is_some_and(|cached| cached.source_projection.text != cached.canonical_projection.text)
        {
            return snapshot;
        }
        let mapped = cache
            .as_ref()
            .map_or(local, |cached| cached.map(local, to_source));
        UndoSelectionSnapshot::from_range(
            mapped.range().start + to_start..mapped.range().end + to_start,
            mapped.reversed(),
        )
    }
}
