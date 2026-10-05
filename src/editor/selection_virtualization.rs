// @author kongweiguang

use std::{collections::HashMap, ops::Range};

use gpui::{App, Context, Entity, EntityId};

use super::selection::CrossBlockInlineTarget;
use super::selection_surface::SelectionSurface;
use super::virtual_surface::VirtualSurfaceState;
use super::{
    Block, CrossBlockSelectionEndpoint, CrossBlockSourceAnchor, Editor, SourceTargetMapping,
};
use crate::components::markdown::inline::StyleFlag;
use crate::components::{BlockKind, EditingCommandId, InlineTextTree, UndoCaptureKind};

/// Cross-block selection normalized in one projection and its current source revision.
#[derive(Clone, Copy)]
pub(super) struct NormalizedCrossBlockSelection {
    pub(super) start: CrossBlockSelectionEndpoint,
    pub(super) end: CrossBlockSelectionEndpoint,
    pub(super) start_index: Option<usize>,
    pub(super) end_index: Option<usize>,
    pub(super) source_start: Option<CrossBlockSourceAnchor>,
    pub(super) source_end: Option<CrossBlockSourceAnchor>,
    pub(super) reversed: bool,
}

impl Editor {
    /// 源码锚点与字素边界一致；纯表格的零文字端点只有完整、当前源码跨度证明时才代表全文选择。
    pub(super) fn normalized_cross_block_selection_for_surface(
        &self,
        surface: SelectionSurface,
        cx: &App,
    ) -> Option<NormalizedCrossBlockSelection> {
        let selection = self.cross_block_selection_for_surface(surface)?;
        let current_revision = self.source_document.snapshot().revision();
        let stored_anchors = match (selection.source_anchor, selection.source_focus) {
            (Some(anchor), Some(focus))
                if anchor.revision == current_revision && focus.revision == current_revision =>
            {
                Some((anchor, focus))
            }
            (Some(_), _) | (_, Some(_)) => return None,
            (None, None) => None,
        };
        let source_anchor = stored_anchors
            .map(|(anchor, _)| anchor)
            .or_else(|| self.cross_block_source_anchor_for_endpoint(surface, selection.anchor, cx));
        let source_focus = stored_anchors
            .map(|(_, focus)| focus)
            .or_else(|| self.cross_block_source_anchor_for_endpoint(surface, selection.focus, cx));
        let anchor = self
            .clamp_cross_block_endpoint_for_surface(selection.anchor, surface, cx)
            .or_else(|| stored_anchors.map(|_| selection.anchor))?;
        let focus = self
            .clamp_cross_block_endpoint_for_surface(selection.focus, surface, cx)
            .or_else(|| stored_anchors.map(|_| selection.focus))?;
        let visible = self.selection_surface_entities(surface);
        let anchor_index = visible
            .iter()
            .position(|entity| entity.entity_id() == anchor.entity_id);
        let focus_index = visible
            .iter()
            .position(|entity| entity.entity_id() == focus.entity_id);
        let reversed = match (source_anchor, source_focus) {
            (Some(anchor_source), Some(focus_source)) => {
                focus_source.byte_offset < anchor_source.byte_offset
            }
            _ => {
                let (Some(anchor_index), Some(focus_index)) = (anchor_index, focus_index) else {
                    return None;
                };
                focus_index < anchor_index
                    || (focus_index == anchor_index && focus.offset < anchor.offset)
            }
        };
        let (start, end, start_index, end_index) = if reversed {
            (focus, anchor, focus_index, anchor_index)
        } else {
            (anchor, focus, anchor_index, focus_index)
        };
        let raw_start = start;
        let raw_end = end;
        let start = self
            .cross_block_endpoint_on_grapheme_boundary_for_surface(start, false, surface, cx)
            .or_else(|| (source_anchor.is_some() && source_focus.is_some()).then_some(start))?;
        let end = self
            .cross_block_endpoint_on_grapheme_boundary_for_surface(end, true, surface, cx)
            .or_else(|| (source_anchor.is_some() && source_focus.is_some()).then_some(end))?;
        let (source_start, source_end) = if reversed {
            (source_focus, source_anchor)
        } else {
            (source_anchor, source_focus)
        };
        // 新推导的锚点必须在边界扩展后计算；已存锚点只在挂载端点确实被修正时重映射。
        let source_start = if stored_anchors.is_none() || start.offset != raw_start.offset {
            self.cross_block_source_anchor_for_endpoint(surface, start, cx)
                .or(source_start)
        } else {
            source_start
        };
        let source_end = if stored_anchors.is_none() || end.offset != raw_end.offset {
            self.cross_block_source_anchor_for_endpoint(surface, end, cx)
                .or(source_end)
        } else {
            source_end
        };
        let same_display_endpoint =
            start_index.is_some() && start_index == end_index && start.offset == end.offset;
        let atomic_document_is_fully_selected = same_display_endpoint
            && stored_anchors.is_some()
            && visible.len() == 1
            && start.offset == 0
            && self
                .cross_block_selection_entity_for_surface(start.entity_id, surface)
                .is_some_and(|entity| entity.read(cx).kind() == BlockKind::Table)
            && source_start.zip(source_end).is_some_and(|(start, end)| {
                let source_len = if surface == SelectionSurface::SplitPreview
                    || self.virtual_surface.is_some()
                {
                    self.source_document.len()
                } else {
                    self.document.cached_markdown_text(cx).len()
                };
                start.byte_offset == 0 && end.byte_offset == source_len && source_len > 0
            });
        if source_start
            .zip(source_end)
            .is_some_and(|(start, end)| start.byte_offset == end.byte_offset)
            || (same_display_endpoint && !atomic_document_is_fully_selected)
        {
            return None;
        }
        Some(NormalizedCrossBlockSelection {
            start,
            end,
            start_index,
            end_index,
            source_start,
            source_end,
            reversed,
        })
    }

    /// Finds visible or pinned Main entities so selection endpoints survive virtual viewport swaps.
    pub(super) fn cross_block_selection_entity_for_surface(
        &self,
        entity_id: gpui::EntityId,
        surface: SelectionSurface,
    ) -> Option<Entity<Block>> {
        self.selection_surface_entities(surface)
            .into_iter()
            .find(|entity| entity.entity_id() == entity_id)
            .or_else(|| match surface {
                SelectionSurface::Main => self
                    .virtual_surface
                    .as_ref()
                    .and_then(|virtual_surface| virtual_surface.entity_by_id(entity_id)),
                SelectionSurface::SplitPreview => None,
            })
    }

    /// Clamps pointer and keyboard endpoints before reading the block's UTF-8 text length.
    fn clamp_cross_block_endpoint_for_surface(
        &self,
        endpoint: CrossBlockSelectionEndpoint,
        surface: SelectionSurface,
        cx: &App,
    ) -> Option<CrossBlockSelectionEndpoint> {
        let entity = self.cross_block_selection_entity_for_surface(endpoint.entity_id, surface)?;
        let len = entity.read(cx).visible_len();
        Some(CrossBlockSelectionEndpoint {
            entity_id: endpoint.entity_id,
            offset: endpoint.offset.min(len),
        })
    }

    /// 显示与源码写入共用字素边界，防止有效 UTF-8 偏移拆开 emoji 或组合音标。
    fn cross_block_endpoint_on_grapheme_boundary_for_surface(
        &self,
        endpoint: CrossBlockSelectionEndpoint,
        toward_end: bool,
        surface: SelectionSurface,
        cx: &App,
    ) -> Option<CrossBlockSelectionEndpoint> {
        let entity = self.cross_block_selection_entity_for_surface(endpoint.entity_id, surface)?;
        let block = entity.read(cx);
        let text = block.display_text();
        let offset =
            crate::ui::text_editing::clamp_grapheme_boundary(text, endpoint.offset, toward_end);
        Some(CrossBlockSelectionEndpoint {
            entity_id: endpoint.entity_id,
            offset,
        })
    }

    /// Reprojects canonical source anchors onto mounted blocks after a virtual viewport swap.
    pub(super) fn virtual_cross_block_selection_ranges_for_entities(
        &self,
        selection: NormalizedCrossBlockSelection,
        visible_blocks: &[Entity<Block>],
        cx: &App,
    ) -> HashMap<EntityId, Range<usize>> {
        let mut ranges = HashMap::new();
        let Some((source_start, source_end)) = selection.source_start.zip(selection.source_end)
        else {
            return ranges;
        };
        let revision = self.source_document.snapshot().revision();
        if source_start.revision != revision
            || source_end.revision != revision
            || self
                .virtual_surface
                .as_ref()
                .is_none_or(|surface| surface.projection_revision() != revision)
        {
            return ranges;
        }

        let Some(surface) = self.virtual_surface.as_ref() else {
            return ranges;
        };
        let mut region_roots = HashMap::new();
        for entity in visible_blocks {
            let entity_id = entity.entity_id();
            let Some(region) = surface.region_for_entity(entity_id) else {
                continue;
            };
            let Some(source_range) = surface.source_range_for_entity(entity_id) else {
                continue;
            };
            let Some(roots) = surface.region_roots_for_entity(entity_id) else {
                continue;
            };
            region_roots
                .entry(region)
                .or_insert_with(|| (source_range.start, roots));
        }

        let mut mappings_by_id = HashMap::new();
        let mut block_ranges_by_id = HashMap::new();
        for (absolute_start, roots) in region_roots.into_values() {
            let (mappings, block_ranges) = self
                .build_source_target_mappings_with_block_ranges_for_roots(
                    &roots,
                    absolute_start,
                    cx,
                );
            mappings_by_id.extend(
                mappings
                    .into_iter()
                    .map(|mapping| (mapping.entity.entity_id(), mapping)),
            );
            block_ranges_by_id.extend(block_ranges);
        }

        for entity in visible_blocks {
            let entity_id = entity.entity_id();
            let Some(block_range) = block_ranges_by_id.get(&entity_id).cloned().or_else(|| {
                mappings_by_id
                    .get(&entity_id)
                    .map(|mapping| mapping.full_source_range.clone())
            }) else {
                continue;
            };
            let Some(range) = Self::virtual_cross_block_selection_range_for_entity(
                source_start.byte_offset..source_end.byte_offset,
                entity,
                mappings_by_id.get(&entity_id),
                block_range,
                cx,
            ) else {
                continue;
            };
            ranges.insert(entity_id, range);
        }
        ranges
    }

    /// Converts one source intersection through its block map without remapping its neighboring regions.
    fn virtual_cross_block_selection_range_for_entity(
        source_range: Range<usize>,
        entity: &Entity<Block>,
        mapping: Option<&SourceTargetMapping>,
        block_range: Range<usize>,
        cx: &App,
    ) -> Option<Range<usize>> {
        let selected_start = source_range.start.max(block_range.start);
        let selected_end = source_range.end.min(block_range.end);
        if selected_start >= selected_end {
            return None;
        }

        let block = entity.read(cx);
        let visible_len = block.visible_len();
        let Some(mapping) = mapping else {
            return (selected_start == block_range.start && selected_end == block_range.end)
                .then_some(0..visible_len)
                .filter(|range| !range.is_empty());
        };

        let visible_offset = |source_offset: usize| -> Option<usize> {
            if source_offset <= mapping.full_source_range.start {
                return Some(0);
            }
            if source_offset >= mapping.full_source_range.end {
                return Some(visible_len);
            }
            let local_source = source_offset - mapping.full_source_range.start;
            let content_offset = *mapping.source_to_content.get(local_source)?;
            Some(
                block
                    .markdown_offset_to_current_offset(content_offset)
                    .min(visible_len),
            )
        };
        let start = visible_offset(selected_start)?;
        let end = visible_offset(selected_end)?;
        (start < end).then_some(start..end)
    }

    /// Enables inline formatting only for writable selected text in the active rendered projection.
    pub(super) fn cross_block_selection_supports_inline_commands(
        &self,
        surface: SelectionSurface,
        selection: NormalizedCrossBlockSelection,
        cx: &App,
    ) -> bool {
        if surface == SelectionSurface::Main
            && let Some(virtual_surface) = self.virtual_surface.as_ref()
        {
            let Some((start, end)) = selection.source_start.zip(selection.source_end) else {
                return false;
            };
            let revision = self.source_document.snapshot().revision();
            return start.revision == revision
                && end.revision == revision
                && virtual_surface.projection_revision() == revision
                && virtual_surface
                    .source_range_supports_inline_commands(start.byte_offset..end.byte_offset);
        }

        let visible = self.selection_surface_entities(surface);
        let (Some(start_index), Some(end_index)) = (selection.start_index, selection.end_index)
        else {
            return false;
        };
        (start_index..=end_index).all(|index| {
            let Some(visible_block) = visible.get(index) else {
                return false;
            };
            let block = visible_block.read(cx);
            if block.is_read_only()
                || block.uses_raw_text_editing()
                || block.showing_rendered_image()
                || matches!(block.kind(), BlockKind::Table | BlockKind::Separator)
            {
                return false;
            }
            let len = block.visible_len();
            let current_range = if start_index == end_index {
                selection.start.offset.min(len)..selection.end.offset.min(len)
            } else if index == start_index {
                selection.start.offset.min(len)..len
            } else if index == end_index {
                0..selection.end.offset.min(len)
            } else {
                0..len
            };
            let clean_range = block.current_to_clean_range(current_range);
            clean_range.is_empty() || block.record.title.selection_supports_toolbar(clean_range)
        })
    }
}

/// Resolves the text style represented by a formatting command; `None` inside `Some` means clear formatting.
fn inline_style_for_command(command: EditingCommandId) -> Option<Option<StyleFlag>> {
    match command {
        EditingCommandId::Bold => Some(Some(StyleFlag::Bold)),
        EditingCommandId::Italic => Some(Some(StyleFlag::Italic)),
        EditingCommandId::Underline => Some(Some(StyleFlag::Underline)),
        EditingCommandId::Highlight => Some(Some(StyleFlag::Highlight)),
        EditingCommandId::Superscript => Some(Some(StyleFlag::Superscript)),
        EditingCommandId::Subscript => Some(Some(StyleFlag::Subscript)),
        EditingCommandId::Strikethrough => Some(Some(StyleFlag::Strikethrough)),
        EditingCommandId::InlineCode => Some(Some(StyleFlag::Code)),
        EditingCommandId::ClearFormatting => Some(None),
        _ => None,
    }
}

impl Editor {
    /// Applies one Main inline command while preserving revision checks and a single undo transaction.
    pub(in crate::editor) fn apply_cross_block_inline_command(
        &mut self,
        command: EditingCommandId,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.document_surface_is_editable()
            || self.active_selection_surface != SelectionSurface::Main
        {
            return false;
        }
        let Some(selection) =
            self.normalized_cross_block_selection_for_surface(SelectionSurface::Main, cx)
        else {
            return false;
        };
        let Some(style) = inline_style_for_command(command) else {
            return false;
        };
        let virtual_edit =
            self.virtual_surface.is_some() && self.view_mode == super::ViewMode::Rendered;
        let (candidates, all_styled) = if virtual_edit {
            let Some(source_range) = self.cross_block_source_range_for_normalized(selection, cx)
            else {
                return false;
            };
            let Some(candidates) =
                self.virtual_cross_block_inline_candidates(selection, source_range, style, cx)
            else {
                return false;
            };
            candidates
        } else {
            let Some(candidates) = self.visible_cross_block_inline_candidates(selection, style, cx)
            else {
                return false;
            };
            candidates
        };

        let enabled = style.map(|_| !all_styled);
        let mut targets = Vec::with_capacity(candidates.len());
        let mut changed = false;
        for (entity, clean_range, mut next_title, source_content_range) in candidates {
            let target_changed = if let Some(flag) = style {
                next_title.set_text_style(clean_range.clone(), flag, enabled.unwrap_or(true))
            } else {
                next_title.clear_text_formatting(clean_range.clone())
            };
            changed |= target_changed;
            let replacement = next_title.serialize_markdown();
            targets.push(CrossBlockInlineTarget {
                entity,
                next_title,
                selected_clean_range: clean_range,
                source_content_range,
                replacement,
            });
        }
        if !changed {
            return false;
        }

        if virtual_edit {
            self.prepare_undo_capture(UndoCaptureKind::NonCoalescible, cx);
            if !self.apply_virtual_cross_block_inline_targets(selection, &targets, cx) {
                self.pending_virtual_undo_selection = None;
                return false;
            }
        } else {
            let Some(commits) = targets
                .into_iter()
                .map(|target| Some((target.entity?, target.next_title)))
                .collect::<Option<Vec<_>>>()
            else {
                return false;
            };
            self.prepare_undo_capture(UndoCaptureKind::NonCoalescible, cx);
            for (entity, title) in commits {
                entity.update(cx, move |block, cx| {
                    block.record.set_title(title);
                    block.sync_render_cache();
                    cx.notify();
                });
            }
            self.mark_dirty(cx);
            self.sync_cross_block_selection_visuals_for_surface(SelectionSurface::Main, cx);
        }
        self.finalize_pending_undo_capture(cx);
        self.request_active_block_scroll_into_view(cx);
        cx.notify();
        true
    }

    /// Builds source-backed inline candidates from only the prepared regions touched by a virtual selection.
    fn virtual_cross_block_inline_candidates(
        &self,
        selection: NormalizedCrossBlockSelection,
        source_range: Range<usize>,
        style: Option<StyleFlag>,
        cx: &mut Context<Self>,
    ) -> Option<(
        Vec<(
            Option<Entity<Block>>,
            Range<usize>,
            InlineTextTree,
            Range<usize>,
        )>,
        bool,
    )> {
        let virtual_surface = self.virtual_surface.as_ref()?;
        let revision = self.source_document.snapshot().revision();
        let spans = virtual_surface.region_spans_for_source_range(source_range.clone());
        if virtual_surface.projection_revision() != revision
            || !virtual_surface.source_range_supports_inline_commands(source_range.clone())
            || spans.is_empty()
        {
            return None;
        }

        let endpoint_ids = [selection.start.entity_id, selection.end.entity_id];
        let mut candidates = Vec::new();
        let mut all_styled = style.is_some();
        for (region, region_range) in spans {
            let roots = self
                .virtual_surface
                .as_ref()?
                .roots_for_region(region, cx)?;
            let (mappings, block_ranges) = self
                .build_source_target_mappings_with_block_ranges_for_roots(
                    &roots,
                    region_range.start,
                    cx,
                );
            for mapping in mappings {
                let entity = mapping.entity.clone();
                let entity_id = entity.entity_id();
                let block_range = block_ranges
                    .get(&entity_id)
                    .cloned()
                    .unwrap_or_else(|| mapping.full_source_range.clone());
                let Some(current_range) = Self::virtual_cross_block_selection_range_for_entity(
                    source_range.clone(),
                    &entity,
                    Some(&mapping),
                    block_range,
                    cx,
                ) else {
                    continue;
                };
                let block = entity.read(cx);
                if block.is_read_only()
                    || block.uses_raw_text_editing()
                    || block.showing_rendered_image()
                    || matches!(block.kind(), BlockKind::Table | BlockKind::Separator)
                {
                    return None;
                }
                let clean_range = block.current_to_clean_range(current_range);
                if clean_range.is_empty() {
                    continue;
                }
                let title = &block.record.title;
                if !title.selection_supports_toolbar(clean_range.clone()) {
                    return None;
                }
                let title_map = title.markdown_offset_map();
                let markdown_len = title_map.markdown().len();
                let relative_start = *mapping.content_to_source.first()?;
                let relative_end = *mapping.content_to_source.get(markdown_len)?;
                if let Some(flag) = style {
                    all_styled &= title.selection_has_style(clean_range.clone(), flag);
                }
                let endpoint_entity = endpoint_ids.contains(&entity_id).then(|| entity.clone());
                candidates.push((
                    endpoint_entity,
                    clean_range,
                    title.clone(),
                    mapping.full_source_range.start + relative_start
                        ..mapping.full_source_range.start + relative_end,
                ));
            }
        }
        (!candidates.is_empty()).then_some((candidates, all_styled))
    }

    /// Keeps nonvirtual formatting on the established visible-block path and validates every selected block.
    fn visible_cross_block_inline_candidates(
        &self,
        selection: NormalizedCrossBlockSelection,
        style: Option<StyleFlag>,
        cx: &App,
    ) -> Option<(
        Vec<(
            Option<Entity<Block>>,
            Range<usize>,
            InlineTextTree,
            Range<usize>,
        )>,
        bool,
    )> {
        let (Some(start_index), Some(end_index)) = (selection.start_index, selection.end_index)
        else {
            return None;
        };
        let visible = self.selection_surface_entities(SelectionSurface::Main);
        let (mappings, _) = self.build_source_target_mappings_with_block_ranges(cx);
        let mappings = mappings
            .into_iter()
            .map(|mapping| (mapping.entity.entity_id(), mapping))
            .collect::<HashMap<_, _>>();
        let mut candidates = Vec::new();
        let mut all_styled = style.is_some();
        for index in start_index..=end_index {
            let entity = visible.get(index)?.clone();
            let block = entity.read(cx);
            if block.is_read_only()
                || block.uses_raw_text_editing()
                || block.showing_rendered_image()
                || matches!(block.kind(), BlockKind::Table | BlockKind::Separator)
            {
                return None;
            }
            let len = block.visible_len();
            let current_range = if start_index == end_index {
                selection.start.offset.min(len)..selection.end.offset.min(len)
            } else if index == start_index {
                selection.start.offset.min(len)..len
            } else if index == end_index {
                0..selection.end.offset.min(len)
            } else {
                0..len
            };
            let clean_range = block.current_to_clean_range(current_range);
            if !clean_range.is_empty()
                && !block
                    .record
                    .title
                    .selection_supports_toolbar(clean_range.clone())
            {
                return None;
            }
            let mapping = mappings.get(&entity.entity_id())?;
            let markdown_len = block.record.title.markdown_offset_map().markdown().len();
            let relative_start = *mapping.content_to_source.first()?;
            let relative_end = *mapping.content_to_source.get(markdown_len)?;
            if let Some(flag) = style
                && !clean_range.is_empty()
            {
                all_styled &= block
                    .record
                    .title
                    .selection_has_style(clean_range.clone(), flag);
            }
            candidates.push((
                Some(entity),
                clean_range,
                block.record.title.clone(),
                mapping.full_source_range.start + relative_start
                    ..mapping.full_source_range.start + relative_end,
            ));
        }
        Some((candidates, all_styled))
    }
}
