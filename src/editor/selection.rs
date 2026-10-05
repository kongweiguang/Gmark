// @author kongweiguang

//! Editor-level selection spanning multiple rendered blocks.

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::sync::Arc;

use gpui::*;

use super::{
    CrossBlockDrag, CrossBlockSelection, CrossBlockSelectionEndpoint, CrossBlockSourceAnchor,
    Editor, PreparedSplitProjection, SourceTargetMapping, UndoSelectionSnapshot, ViewMode,
    selection_surface::SelectionSurface, selection_virtualization::NormalizedCrossBlockSelection,
};
use crate::components::{
    Block, BlockKind, Copy, CopyAsMarkdown, Cut, Delete, DeleteBack, InlineTextTree, Paste,
    TableCellPosition, UndoCaptureKind, serialize_table_markdown_lines,
};
use crate::perf;

pub(super) struct CrossBlockInlineTarget {
    pub(super) entity: Option<Entity<Block>>,
    pub(super) next_title: InlineTextTree,
    pub(super) selected_clean_range: Range<usize>,
    pub(super) source_content_range: Range<usize>,
    pub(super) replacement: String,
}

#[path = "selection_autoscroll.rs"]
mod autoscroll;
#[path = "selection_pointer.rs"]
mod selection_pointer;

impl Editor {
    /// 树内块与树外表格 cell 共用镜像清理边界，新鼠标手势与过期拖选不能留下旧全文高亮。
    fn clear_cross_block_selection_visuals_for_surface(
        &mut self,
        surface: SelectionSurface,
        cx: &mut Context<Self>,
    ) -> bool {
        let mut changed = false;
        for entity in self.selection_surface_entities(surface) {
            entity.update(cx, |block, cx| {
                if block.editor_selection_range.take().is_some()
                    || block.editor_selection_supports_inline_commands
                {
                    block.editor_selection_supports_inline_commands = false;
                    changed = true;
                    cx.notify();
                }
            });
        }
        changed |=
            self.sync_cross_block_table_cell_text_selections_for(surface, &HashSet::new(), cx);
        changed
    }

    /// Preserves the established Main API while keeping Split Preview's local selection separate.
    pub(super) fn clear_cross_block_selection(&mut self, cx: &mut Context<Self>) {
        self.clear_cross_block_selection_for_surface(SelectionSurface::Main, cx);
    }

    /// 结束所属表面的选区与拖选；视觉镜像统一由清理边界处理，保留兄弟窗格及 cell 局部选区。
    pub(super) fn clear_cross_block_selection_for_surface(
        &mut self,
        surface: SelectionSurface,
        cx: &mut Context<Self>,
    ) {
        let had_selection = self.cross_block_selection_for_surface(surface).is_some();
        self.set_cross_block_selection_for_surface(surface, None);
        self.set_cross_block_drag_for_surface(surface, None);
        let changed_visuals = self.clear_cross_block_selection_visuals_for_surface(surface, cx);
        let changed = had_selection || changed_visuals;
        if changed {
            cx.notify();
        }
    }
}
#[path = "input/selection/clipboard.rs"]
mod clipboard;

impl Editor {
    /// Detects a full selection against the block identities belonging to one rendered surface.
    fn rendered_surface_is_fully_selected(&self, surface: SelectionSurface, cx: &App) -> bool {
        if surface == SelectionSurface::Main && self.virtual_surface.is_some() {
            let Some(selection) = self.cross_block_selection_for_surface(surface) else {
                return false;
            };
            let revision = self.source_document.snapshot().revision();
            return selection
                .source_anchor
                .is_some_and(|anchor| anchor.byte_offset == 0 && anchor.revision == revision)
                && selection.source_focus.is_some_and(|focus| {
                    focus.byte_offset == self.source_document.len() && focus.revision == revision
                });
        }
        let visible = self.selection_surface_entities(surface);
        let Some(first) = visible.first() else {
            return false;
        };
        let Some(last) = visible.last() else {
            return false;
        };
        let Some(selection) = self.cross_block_selection_for_surface(surface) else {
            return false;
        };
        selection.anchor.entity_id == first.entity_id()
            && selection.anchor.offset == 0
            && selection.focus.entity_id == last.entity_id()
            && selection.focus.offset == last.read(cx).visible_len()
    }

    /// 全文选择显式保存完整源码跨度；常驻 Main 使用规范投影坐标，虚拟与 Split 右侧保持原源码坐标。
    fn select_all_rendered_surface(&mut self, surface: SelectionSurface, cx: &mut Context<Self>) {
        if self.rendered_surface_is_fully_selected(surface, cx) {
            return;
        }
        let visible = self.selection_surface_entities(surface);
        let Some(first) = visible.first() else {
            return;
        };
        let Some(last) = visible.last() else {
            return;
        };
        let mut selection = self.cross_block_selection_from_endpoints(
            surface,
            CrossBlockSelectionEndpoint {
                entity_id: first.entity_id(),
                offset: 0,
            },
            CrossBlockSelectionEndpoint {
                entity_id: last.entity_id(),
                offset: last.read(cx).visible_len(),
            },
            None,
            cx,
        );
        let revision = self.source_document.snapshot().revision();
        let source_projection_is_current = match surface {
            SelectionSurface::Main => self
                .virtual_surface
                .as_ref()
                .is_some_and(|virtual_surface| virtual_surface.projection_revision() == revision),
            SelectionSurface::SplitPreview => self
                .split_preview
                .as_ref()
                .is_some_and(|preview| preview.revision == revision),
        };
        if source_projection_is_current {
            selection.source_anchor = Some(CrossBlockSourceAnchor {
                byte_offset: 0,
                revision,
            });
            selection.source_focus = Some(CrossBlockSourceAnchor {
                byte_offset: self.source_document.len(),
                revision,
            });
        } else if surface == SelectionSurface::Main && self.virtual_surface.is_none() {
            // 只靠文字端点无法越过零文字长度的表格；该分支与 resident mapper 共用规范源码真值。
            selection.source_anchor = Some(CrossBlockSourceAnchor {
                byte_offset: 0,
                revision,
            });
            selection.source_focus = Some(CrossBlockSourceAnchor {
                byte_offset: self.document.cached_markdown_text(cx).len(),
                revision,
            });
        }

        self.end_surface_pointer_selection(surface, cx);
        self.dismiss_contextual_overlays(cx);
        if surface == SelectionSurface::Main {
            self.clear_table_axis_preview(cx);
            self.clear_table_axis_selection(cx);
        }
        self.set_table_cell_rectangle_for_surface(surface, None);
        self.set_table_cell_drag_anchor_for_surface(surface, None);
        self.sync_table_cell_rectangle_highlights_for(surface, cx);
        for entity in visible {
            entity.update(cx, |block, cx| {
                let cursor = block.cursor_offset();
                let collapsed = cursor..cursor;
                if block.selected_range != collapsed {
                    block.selected_range = collapsed;
                    cx.notify();
                }
            });
        }
        self.set_cross_block_drag_for_surface(surface, None);
        self.set_cross_block_selection_for_surface(surface, Some(selection));
        self.active_selection_surface = surface;
        self.sync_cross_block_selection_visuals_for_surface(surface, cx);
        cx.notify();
    }

    /// 首按全选真实输入所属表面；Split 的独立 cell 也由注册表归属，不能只检查树内块。
    pub(super) fn on_rendered_select_all_press(
        &mut self,
        block: Entity<Block>,
        cx: &mut Context<Self>,
    ) {
        let surface = match self.view_mode {
            ViewMode::Rendered | ViewMode::Preview => SelectionSurface::Main,
            ViewMode::Split
                if self.selection_surface_for_block_id(block.entity_id())
                    == Some(SelectionSurface::SplitPreview) =>
            {
                SelectionSurface::SplitPreview
            }
            _ => {
                self.rendered_select_all_cycle = None;
                return;
            }
        };
        self.select_all_rendered_surface(surface, cx);
    }

    pub(super) fn cross_block_source_selection_snapshot(
        &self,
        cx: &App,
    ) -> Option<UndoSelectionSnapshot> {
        let normalized = self.normalized_cross_block_selection(cx)?;
        let range = self.cross_block_source_range_for_normalized(normalized, cx)?;
        Some(UndoSelectionSnapshot::from_range(
            range,
            normalized.reversed,
        ))
    }

    /// 历史按完整源码跨度恢复；全文表格使用父块端点保留零文字长度范围，实际焦点仍交给原映射中的文字目标。
    pub(super) fn apply_cross_block_selection_snapshot_if_possible(
        &mut self,
        snapshot: &UndoSelectionSnapshot,
        cx: &mut Context<Self>,
    ) -> bool {
        let range = snapshot.range();
        let reversed = snapshot.reversed();
        if range.is_empty() {
            return false;
        }

        let mappings = self.build_source_target_mappings(cx);
        let Some(start) = self.endpoint_for_source_offset(range.start, &mappings, cx) else {
            return false;
        };
        let Some(end) = self.endpoint_for_source_offset(range.end, &mappings, cx) else {
            return false;
        };
        let mapped_focus_entity_id = if reversed {
            start.entity_id
        } else {
            end.entity_id
        };
        let start_index = self.document.visible_index_for_entity_id(start.entity_id);
        let end_index = self.document.visible_index_for_entity_id(end.entity_id);
        let resident_full_atomic_selection = self.virtual_surface.is_none()
            && (start_index.is_none() || end_index.is_none())
            && range.start == 0
            && range.end == self.document.cached_markdown_text(cx).len();
        let (start, end) = if start_index
            .zip(end_index)
            .is_some_and(|(start, end)| start != end)
        {
            (start, end)
        } else if self.virtual_surface.is_some() || resident_full_atomic_selection {
            let visible = self.document.visible_blocks();
            let (Some(first), Some(last)) = (visible.first(), visible.last()) else {
                return false;
            };
            let first = first.entity.clone();
            let last = last.entity.clone();
            (
                CrossBlockSelectionEndpoint {
                    entity_id: first.entity_id(),
                    offset: 0,
                },
                CrossBlockSelectionEndpoint {
                    entity_id: last.entity_id(),
                    offset: last.read(cx).visible_len(),
                },
            )
        } else {
            return false;
        };

        let revision = self.source_document.snapshot().revision();
        self.cross_block_selection = Some(if reversed {
            CrossBlockSelection {
                anchor: end,
                focus: start,
                source_anchor: Some(CrossBlockSourceAnchor {
                    byte_offset: range.end,
                    revision,
                }),
                source_focus: Some(CrossBlockSourceAnchor {
                    byte_offset: range.start,
                    revision,
                }),
            }
        } else {
            CrossBlockSelection {
                anchor: start,
                focus: end,
                source_anchor: Some(CrossBlockSourceAnchor {
                    byte_offset: range.start,
                    revision,
                }),
                source_focus: Some(CrossBlockSourceAnchor {
                    byte_offset: range.end,
                    revision,
                }),
            }
        });
        self.cross_block_drag = None;
        self.sync_cross_block_selection_visuals(cx);
        let focus_entity_id = if resident_full_atomic_selection {
            mapped_focus_entity_id
        } else if reversed {
            start.entity_id
        } else {
            end.entity_id
        };
        self.focus_block(focus_entity_id);
        cx.notify();
        true
    }

    /// 兼容主表面的调用方；Split 右侧必须显式使用自己的命中投影。
    fn cross_block_endpoint_for_point(
        &self,
        position: Point<Pixels>,
        cx: &App,
    ) -> Option<CrossBlockSelectionEndpoint> {
        self.cross_block_endpoint_for_surface(position, SelectionSurface::Main, cx)
    }

    /// Resolves pointer positions against only mounted blocks from the receiving projection.
    fn cross_block_endpoint_for_surface(
        &self,
        position: Point<Pixels>,
        surface: SelectionSurface,
        cx: &App,
    ) -> Option<CrossBlockSelectionEndpoint> {
        let mut previous: Option<(Entity<Block>, Bounds<Pixels>)> = None;
        for entity in self.selection_surface_entities(surface) {
            let bounds = entity.read(cx).last_bounds;
            let Some(bounds) = bounds else {
                continue;
            };

            if position.y < bounds.top() {
                if let Some((previous, _)) = previous {
                    let offset = previous.read(cx).visible_len();
                    return Some(CrossBlockSelectionEndpoint {
                        entity_id: previous.entity_id(),
                        offset,
                    });
                }
                return Some(CrossBlockSelectionEndpoint {
                    entity_id: entity.entity_id(),
                    offset: 0,
                });
            }

            if position.y <= bounds.bottom() {
                let offset = entity.read(cx).index_for_mouse_position(position);
                return Some(CrossBlockSelectionEndpoint {
                    entity_id: entity.entity_id(),
                    offset,
                });
            }

            previous = Some((entity, bounds));
        }

        previous.map(|(entity, _)| CrossBlockSelectionEndpoint {
            entity_id: entity.entity_id(),
            offset: entity.read(cx).visible_len(),
        })
    }

    /// Keeps the existing Main caller contract and compares endpoints in its visible order.
    fn cross_block_selection_is_empty(&self, selection: CrossBlockSelection) -> bool {
        self.cross_block_selection_is_empty_for_surface(selection, SelectionSurface::Main)
    }

    /// Rejects stale endpoints from another projection before they can become a visible selection.
    fn cross_block_selection_is_empty_for_surface(
        &self,
        selection: CrossBlockSelection,
        surface: SelectionSurface,
    ) -> bool {
        if let (Some(anchor), Some(focus)) = (selection.source_anchor, selection.source_focus) {
            if anchor.revision == self.source_document.snapshot().revision()
                && focus.revision == anchor.revision
            {
                return anchor.byte_offset == focus.byte_offset;
            }
        }
        let visible = self.selection_surface_entities(surface);
        let Some(anchor_index) = visible
            .iter()
            .position(|entity| entity.entity_id() == selection.anchor.entity_id)
        else {
            return true;
        };
        let Some(focus_index) = visible
            .iter()
            .position(|entity| entity.entity_id() == selection.focus.entity_id)
        else {
            return true;
        };
        anchor_index == focus_index && selection.anchor.offset == selection.focus.offset
    }

    /// Preserves Main-only source editing paths while allowing read-only Split selection and copy.
    fn normalized_cross_block_selection(&self, cx: &App) -> Option<NormalizedCrossBlockSelection> {
        self.normalized_cross_block_selection_for_surface(SelectionSurface::Main, cx)
    }

    /// Keeps the established Main test and command entry while surface-aware pointer handling lives in its own module.
    pub(super) fn begin_cross_block_drag_at_point(
        &mut self,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        self.begin_surface_cross_block_drag_at_point(
            SelectionSurface::Main,
            position,
            false,
            None,
            cx,
        );
    }

    /// Preserves Main callers for undo/edit paths and forwards the presentation surface explicitly.
    fn sync_cross_block_selection_visuals(&mut self, cx: &mut Context<Self>) {
        self.sync_cross_block_selection_visuals_for_surface(SelectionSurface::Main, cx);
    }

    /// 整表选择需完整跨度证明；原源码全文锚点可覆盖规范拼写长度不同的 Split/虚拟表格。
    fn cross_block_selection_fully_covers_table_for_surface(
        &self,
        surface: SelectionSurface,
        selection: NormalizedCrossBlockSelection,
        table_index: usize,
        table_entity_id: gpui::EntityId,
        cx: &App,
    ) -> bool {
        if matches!(
            (selection.start_index, selection.end_index),
            (Some(start), Some(end)) if start < table_index && table_index < end
        ) {
            return true;
        }
        let Some((source_start, source_end)) = selection.source_start.zip(selection.source_end)
        else {
            return false;
        };
        let uses_original_source =
            surface == SelectionSurface::SplitPreview || self.virtual_surface.is_some();
        if uses_original_source
            && source_start.byte_offset == 0
            && source_end.byte_offset == self.source_document.len()
            && self.rendered_surface_is_fully_selected(surface, cx)
        {
            return true;
        }
        let table_source_range = match surface {
            SelectionSurface::Main => self
                .build_source_target_mapping_and_range_for_entity(table_entity_id, cx)
                .map(|(_, range)| range),
            SelectionSurface::SplitPreview => self
                .split_preview
                .as_ref()
                .filter(|preview| preview.revision == self.source_document.snapshot().revision())
                .and_then(|preview| preview.source_ranges.get(&table_entity_id).cloned()),
        };
        let Some(table_source_range) = table_source_range else {
            return false;
        };
        table_source_range.start < table_source_range.end
            && source_start.byte_offset <= table_source_range.start
            && source_end.byte_offset >= table_source_range.end
    }

    /// 将跨块选区同步到 block 与 cell 文字范围，同时保留表格结构选择状态。
    pub(super) fn sync_cross_block_selection_visuals_for_surface(
        &mut self,
        surface: SelectionSurface,
        cx: &mut Context<Self>,
    ) {
        let normalized = self.normalized_cross_block_selection_for_surface(surface, cx);
        let visible_blocks = self.selection_surface_entities(surface);
        let table_cells_are_projected = matches!(
            (self.view_mode, surface),
            (
                ViewMode::Rendered | ViewMode::Preview,
                SelectionSurface::Main
            ) | (ViewMode::Split, SelectionSurface::SplitPreview)
        );
        let mut fully_selected_table_ids = HashSet::new();
        let virtual_ranges = if surface == SelectionSurface::Main && self.virtual_surface.is_some()
        {
            normalized
                .map(|selection| {
                    self.virtual_cross_block_selection_ranges_for_entities(
                        selection,
                        &visible_blocks,
                        cx,
                    )
                })
                .unwrap_or_default()
        } else {
            HashMap::new()
        };
        let inline_commands_safe = surface == SelectionSurface::Main
            && self.document_surface_is_editable()
            && normalized.is_some_and(|selection| {
                self.cross_block_selection_supports_inline_commands(surface, selection, cx)
            });
        for (index, entity) in visible_blocks.into_iter().enumerate() {
            let next_range = if surface == SelectionSurface::Main && self.virtual_surface.is_some()
            {
                virtual_ranges.get(&entity.entity_id()).cloned()
            } else {
                normalized.and_then(|selection| {
                    let (Some(start_index), Some(end_index)) =
                        (selection.start_index, selection.end_index)
                    else {
                        return None;
                    };
                    if index < start_index || index > end_index {
                        return None;
                    }
                    let block = entity.read(cx);
                    let len = block.visible_len();
                    let range = if start_index == end_index {
                        selection.start.offset.min(len)..selection.end.offset.min(len)
                    } else if index == start_index {
                        selection.start.offset.min(len)..len
                    } else if index == end_index {
                        0..selection.end.offset.min(len)
                    } else {
                        0..len
                    };
                    (!range.is_empty()).then_some(range)
                })
            };

            let table_is_fully_selected = table_cells_are_projected
                && entity.read(cx).kind() == BlockKind::Table
                && normalized.is_some_and(|selection| {
                    self.cross_block_selection_fully_covers_table_for_surface(
                        surface,
                        selection,
                        index,
                        entity.entity_id(),
                        cx,
                    )
                });
            if table_is_fully_selected {
                fully_selected_table_ids.insert(entity.entity_id());
            }

            entity.update(cx, |block, cx| {
                let next_support = next_range.is_some() && inline_commands_safe;
                if block.editor_selection_range != next_range
                    || block.editor_selection_supports_inline_commands != next_support
                {
                    block.editor_selection_range = next_range.clone();
                    block.editor_selection_supports_inline_commands = next_support;
                    cx.notify();
                }
            });
        }
        let changed_cell_visuals = self.sync_cross_block_table_cell_text_selections_for(
            surface,
            &fully_selected_table_ids,
            cx,
        );
        if changed_cell_visuals {
            cx.notify();
        }
    }
}

#[path = "selection_parts/controller.rs"]
mod controller;

#[cfg(test)]
#[path = "../../tests/unit/editor/selection.rs"]
mod tests;

#[cfg(all(test, target_os = "windows"))]
#[path = "../../tests/unit/editor/ime_input_commands.rs"]
mod ime_input_commands;

#[cfg(test)]
#[path = "../../tests/unit/editor/selection_virtualized.rs"]
mod selection_virtualized;

#[cfg(test)]
#[path = "../../tests/unit/editor/selection_virtualized_clipboard.rs"]
mod selection_virtualized_clipboard;
