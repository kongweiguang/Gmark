// @author kongweiguang

use std::ops::Range;

use gpui::{Context, Entity};

use super::super::{Block, Editor};
use super::{ProjectionRegionKind, VirtualSurfaceState};
use crate::editor::selection_surface::SelectionSurface;

impl VirtualSurfaceState {
    /// Visits only regions intersecting a canonical source range, keeping selection work logarithmic before mapping.
    pub(in crate::editor) fn region_spans_for_source_range(
        &self,
        range: Range<usize>,
    ) -> Vec<(usize, Range<usize>)> {
        if range.start >= range.end {
            return Vec::new();
        }
        let Some(first) = self.region_index.region_for_source_offset(range.start) else {
            return Vec::new();
        };
        let Some(last) = self
            .region_index
            .region_for_source_offset(range.end.saturating_sub(1))
        else {
            return Vec::new();
        };
        (first..=last)
            .filter_map(|index| {
                let span = self.region_index.source_range(index)?;
                (span.start < range.end && range.start < span.end).then_some((index, span))
            })
            .collect()
    }

    /// Reuses mounted endpoint entities and creates only the selected region when it is outside the viewport.
    pub(in crate::editor) fn roots_for_region(
        &self,
        region: usize,
        cx: &mut Context<Editor>,
    ) -> Option<Vec<Entity<Block>>> {
        self.mounted.get(&region).cloned().or_else(|| {
            (region < self.projection.regions.len()).then(|| {
                Editor::materialize_projection_region(
                    cx,
                    &self.projection,
                    region,
                    &mut std::collections::HashMap::new(),
                )
            })
        })
    }

    /// Lets pointer feedback enable formatting cheaply while the command revalidates materialized blocks.
    pub(in crate::editor) fn source_range_supports_inline_commands(
        &self,
        range: Range<usize>,
    ) -> bool {
        let regions = self.region_spans_for_source_range(range);
        !regions.is_empty()
            && regions.iter().all(|(index, _)| {
                self.projection.regions.get(*index).is_some_and(|region| {
                    matches!(
                        region.kind,
                        ProjectionRegionKind::Blank
                            | ProjectionRegionKind::Paragraph
                            | ProjectionRegionKind::List
                            | ProjectionRegionKind::Quote
                            | ProjectionRegionKind::AtxHeading
                            | ProjectionRegionKind::SetextHeading
                            | ProjectionRegionKind::FootnoteDefinition
                    )
                })
            })
    }
}

impl Editor {
    /// Replaces viewport roots, then reapplies source-anchored selection to the newly mounted blocks.
    pub(in crate::editor) fn sync_virtual_surface_mounts(
        &mut self,
        scroll_y: f32,
        viewport_height: f32,
        overdraw: f32,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(mut surface) = self.virtual_surface.take() else {
            return false;
        };
        let focused_region = self
            .active_entity_id
            .and_then(|entity_id| surface.region_for_entity(entity_id));
        let target =
            surface.desired_window(scroll_y, viewport_height.max(1.0), overdraw, focused_region);
        if surface.mount_window() == &target {
            self.virtual_surface = Some(surface);
            return false;
        }

        surface.reconcile_mounts(target, cx);
        let roots = surface.viewport_roots();
        self.virtual_surface = Some(surface);
        if roots.is_empty() {
            return false;
        }
        self.document.replace_roots(roots, cx);
        self.prev_visible_block_ids.clear();
        self.prev_render_window = None;
        self.row_stride_cache.clear();
        self.render_row_cache = None;
        self.rebuild_virtual_table_runtimes(cx);
        if self.view_mode == super::super::ViewMode::Preview {
            self.set_projection_read_only(true, cx);
        }
        self.sync_cross_block_selection_visuals_for_surface(SelectionSurface::Main, cx);
        self.apply_pending_virtual_footnote_focus(cx);
        self.apply_pending_virtual_footnote_backref_focus(cx);
        true
    }
}
