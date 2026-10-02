// @author kongweiguang

//! 源码区域驱动的虚拟 Markdown surface 基础索引。
//!
//! 该层不持有 GPUI Entity。它以投影区域为全局顺序，用 Fenwick 树维护可更新的
//! 高度前缀和，让 viewport、源码锚点和实测高度更新保持 O(log n)。

use std::collections::{BTreeMap, HashMap, HashSet};
use std::ops::Range;
use std::sync::Arc;

use gpui::{Entity, EntityId};
use pulldown_cmark::{Event, Options, Parser, Tag};

use super::projection::{PreparedSplitProjection, ProjectionRegionKind};
use super::{Block, Editor};

#[path = "virtual_surface_selection.rs"]
mod selection;

const MIN_REGION_HEIGHT: f32 = 1.0;

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct VirtualSourceAnchor {
    pub(super) region_index: usize,
    pub(super) source_offset: usize,
    pub(super) region_fraction: f32,
}

pub(super) struct VirtualSurfaceLayout {
    pub(super) top_spacer: f32,
    pub(super) bottom_spacer: f32,
    pub(super) pinned_top: Option<f32>,
    pub(super) pinned_roots: Vec<Entity<Block>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct VirtualMountWindow {
    pub(super) regions: Range<usize>,
    /// 焦点区域远离 viewport 时独立挂载，不能把连续窗口扩张到两者之间。
    pub(super) pinned_region: Option<usize>,
}

pub(super) struct VirtualRegionIndex {
    source_ranges: Vec<Range<usize>>,
    heights: Vec<f32>,
    /// Fenwick tree 使用 1-based 索引，存储 f64 以降低十万级区域累计误差。
    height_tree: Vec<f64>,
}

pub(super) struct VirtualSurfaceState {
    projection: Arc<PreparedSplitProjection>,
    region_index: VirtualRegionIndex,
    mounted: BTreeMap<usize, Vec<Entity<Block>>>,
    entity_regions: HashMap<EntityId, usize>,
    mounted_entities: HashMap<EntityId, Entity<Block>>,
    footnote_ordinals: HashMap<String, usize>,
    footnote_definition_regions: HashMap<String, usize>,
    footnote_first_reference_regions: HashMap<String, usize>,
    mount_window: VirtualMountWindow,
}

impl VirtualSurfaceState {
    pub(super) fn new(projection: Arc<PreparedSplitProjection>) -> Self {
        let region_index = VirtualRegionIndex::from_projection(&projection);
        let mount_window = region_index.mount_window(0.0, 720.0, 800.0, Some(0));
        let (footnote_ordinals, footnote_definition_regions, footnote_first_reference_regions) =
            build_virtual_footnote_index(&projection, &region_index);
        Self {
            projection,
            region_index,
            mounted: BTreeMap::new(),
            entity_regions: HashMap::new(),
            mounted_entities: HashMap::new(),
            footnote_ordinals,
            footnote_definition_regions,
            footnote_first_reference_regions,
            mount_window,
        }
    }

    pub(super) fn desired_window(
        &self,
        scroll_y: f32,
        viewport_height: f32,
        overdraw: f32,
        focused_region: Option<usize>,
    ) -> VirtualMountWindow {
        self.region_index
            .mount_window(scroll_y, viewport_height, overdraw, focused_region)
    }

    pub(super) fn projection_revision(&self) -> gmark_document::Revision {
        self.projection.revision
    }

    pub(super) fn y_for_source_offset(&self, source_offset: usize) -> Option<f32> {
        let region = self.region_index.region_for_source_offset(source_offset)?;
        self.region_index.top(region)
    }

    pub(super) fn mount_window(&self) -> &VirtualMountWindow {
        &self.mount_window
    }

    pub(super) fn reconcile_mounts(
        &mut self,
        target: VirtualMountWindow,
        cx: &mut gpui::Context<Editor>,
    ) {
        self.reconcile_mounts_reusing(target, &mut HashMap::new(), cx);
    }

    fn reconcile_mounts_reusing(
        &mut self,
        target: VirtualMountWindow,
        reusable: &mut HashMap<uuid::Uuid, Entity<Block>>,
        cx: &mut gpui::Context<Editor>,
    ) {
        let mut wanted = target.regions.clone().collect::<HashSet<_>>();
        if let Some(pinned) = target.pinned_region {
            wanted.insert(pinned);
        }
        self.mounted.retain(|region, _| wanted.contains(region));

        for region_index in wanted {
            if self.mounted.contains_key(&region_index) {
                continue;
            }
            let roots =
                Editor::materialize_projection_region(cx, &self.projection, region_index, reusable);
            self.mounted.insert(region_index, roots);
        }
        self.mount_window = target;
        self.rebuild_entity_regions(cx);
    }

    /// 安装新 revision，并按稳定 BlockRecord UUID 复用仍匹配的活动/viewport Entity。
    pub(super) fn replace_projection(
        &mut self,
        projection: Arc<PreparedSplitProjection>,
        scroll_y: f32,
        viewport_height: f32,
        overdraw: f32,
        focused_entity: Option<EntityId>,
        cx: &mut gpui::Context<Editor>,
    ) {
        let focused_source_offset = focused_entity
            .and_then(|entity_id| self.source_range_for_entity(entity_id))
            .map(|range| range.start);
        let focused_roots = focused_entity.and_then(|entity_id| {
            let region = self.region_for_entity(entity_id)?;
            self.mounted.get(&region).cloned()
        });
        let mut reusable = self
            .mounted_entities
            .values()
            .map(|entity| (entity.read(cx).record.id, entity.clone()))
            .collect::<HashMap<_, _>>();

        self.projection = projection;
        self.region_index = VirtualRegionIndex::from_projection(&self.projection);
        (
            self.footnote_ordinals,
            self.footnote_definition_regions,
            self.footnote_first_reference_regions,
        ) = build_virtual_footnote_index(&self.projection, &self.region_index);
        self.mounted.clear();
        self.entity_regions.clear();
        self.mounted_entities.clear();
        let focused_region = focused_source_offset
            .and_then(|offset| self.region_index.region_for_source_offset(offset));
        if let (Some(region), Some(roots)) = (focused_region, focused_roots) {
            // 活动区域已经包含用户刚提交的语义；保留 Entity 才能保持 IME/selection/focus。
            self.mounted.insert(region, roots);
        }
        let target = self.region_index.mount_window(
            scroll_y,
            viewport_height.max(1.0),
            overdraw,
            focused_region,
        );
        self.reconcile_mounts_reusing(target, &mut reusable, cx);
    }

    pub(super) fn flattened_roots(&self) -> Vec<Entity<Block>> {
        self.mounted
            .values()
            .flat_map(|roots| roots.iter().cloned())
            .collect()
    }

    pub(super) fn viewport_roots(&self) -> Vec<Entity<Block>> {
        self.mounted
            .range(self.mount_window.regions.clone())
            .flat_map(|(_, roots)| roots.iter().cloned())
            .collect()
    }

    pub(super) fn pinned_roots(&self) -> Vec<Entity<Block>> {
        self.mount_window
            .pinned_region
            .and_then(|region| self.mounted.get(&region))
            .into_iter()
            .flat_map(|roots| roots.iter().cloned())
            .collect()
    }

    pub(super) fn region_for_entity(&self, entity_id: EntityId) -> Option<usize> {
        self.entity_regions.get(&entity_id).copied()
    }

    pub(super) fn entity_by_id(&self, entity_id: EntityId) -> Option<Entity<Block>> {
        self.mounted_entities.get(&entity_id).cloned()
    }

    pub(super) fn source_range_for_entity(&self, entity_id: EntityId) -> Option<Range<usize>> {
        self.region_for_entity(entity_id)
            .and_then(|region| self.region_index.source_range(region))
    }

    pub(super) fn region_roots_for_entity(
        &self,
        entity_id: EntityId,
    ) -> Option<Vec<Entity<Block>>> {
        let region = self.region_for_entity(entity_id)?;
        self.mounted.get(&region).cloned()
    }

    pub(super) fn mapping_input_for_entity(
        &self,
        entity_id: EntityId,
    ) -> Option<(Range<usize>, Vec<Entity<Block>>)> {
        Some((
            self.source_range_for_entity(entity_id)?,
            self.region_roots_for_entity(entity_id)?,
        ))
    }

    pub(super) fn apply_entity_region_source_len(
        &mut self,
        entity_id: EntityId,
        new_len: usize,
    ) -> bool {
        let Some(region) = self.region_for_entity(entity_id) else {
            return false;
        };
        self.region_index.apply_region_source_len(region, new_len)
    }

    #[cfg(test)]
    pub(super) fn mounted_region_count(&self) -> usize {
        self.mounted.len()
    }

    pub(super) fn footnote_ordinal(&self, id: &str) -> Option<usize> {
        self.footnote_ordinals.get(id).copied()
    }

    pub(super) fn footnote_definition_region(&self, id: &str) -> Option<usize> {
        self.footnote_definition_regions.get(id).copied()
    }

    pub(super) fn footnote_definition_y(&self, id: &str) -> Option<f32> {
        self.footnote_definition_region(id)
            .and_then(|region| self.region_index.top(region))
    }

    pub(super) fn footnote_first_reference_y(&self, id: &str) -> Option<f32> {
        self.footnote_first_reference_regions
            .get(id)
            .and_then(|region| self.region_index.top(*region))
    }

    #[cfg(test)]
    pub(super) fn mounted_entity_count(&self) -> usize {
        self.entity_regions.len()
    }

    pub(super) fn top_spacer_height(&self) -> f32 {
        self.region_index
            .top(self.mount_window.regions.start)
            .unwrap_or(0.0)
    }

    pub(super) fn bottom_spacer_height(&self) -> f32 {
        let mounted_bottom = self
            .region_index
            .top(self.mount_window.regions.end)
            .unwrap_or_else(|| self.region_index.total_height());
        (self.region_index.total_height() - mounted_bottom).max(0.0)
    }

    pub(super) fn pinned_top(&self) -> Option<f32> {
        self.mount_window
            .pinned_region
            .and_then(|region| self.region_index.top(region))
    }

    fn rebuild_entity_regions(&mut self, cx: &gpui::App) {
        self.entity_regions.clear();
        self.mounted_entities.clear();
        for (&region_index, roots) in &self.mounted {
            for root in roots {
                collect_entity_regions(
                    root,
                    region_index,
                    &mut self.entity_regions,
                    &mut self.mounted_entities,
                    cx,
                );
            }
        }
    }
}

impl Editor {
    pub(super) fn virtual_surface_layout(&self) -> Option<VirtualSurfaceLayout> {
        let surface = self.virtual_surface.as_ref()?;
        Some(VirtualSurfaceLayout {
            top_spacer: surface.top_spacer_height(),
            bottom_spacer: surface.bottom_spacer_height(),
            pinned_top: surface.pinned_top(),
            pinned_roots: surface.pinned_roots(),
        })
    }

    pub(super) fn install_virtual_surface_projection(
        &mut self,
        projection: Arc<PreparedSplitProjection>,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        let Some(mut surface) = self.virtual_surface.take() else {
            return false;
        };
        let viewport = self.scroll_handle.bounds().size;
        let viewport_height = f32::from(viewport.height.max(gpui::px(1.0)));
        let scroll_y = -f32::from(self.scroll_handle.offset().y);
        surface.replace_projection(
            projection,
            scroll_y.max(0.0),
            viewport_height,
            800.0,
            self.active_entity_id,
            cx,
        );
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
        if self.view_mode == super::ViewMode::Preview {
            self.set_projection_read_only(true, cx);
        }
        self.apply_pending_virtual_footnote_focus(cx);
        self.apply_pending_virtual_footnote_backref_focus(cx);
        true
    }

    fn apply_pending_virtual_footnote_focus(&mut self, cx: &mut gpui::Context<Self>) {
        let Some(id) = self.pending_virtual_footnote_focus.clone() else {
            return;
        };
        let target = self.document.visible_blocks().iter().find_map(|visible| {
            let block = visible.entity.read(cx);
            (block.kind() == crate::components::BlockKind::FootnoteDefinition
                && block.record.title.visible_text() == id)
                .then(|| visible.entity.clone())
        });
        if let Some(target) = target {
            self.pending_virtual_footnote_focus = None;
            self.focus_block_range(&target, 0..0, cx);
        }
    }

    fn apply_pending_virtual_footnote_backref_focus(&mut self, cx: &mut gpui::Context<Self>) {
        let Some(id) = self.pending_virtual_footnote_backref_focus.clone() else {
            return;
        };
        let target = self.footnote_registry.binding(&id).and_then(|binding| {
            let reference = binding.first_reference.as_ref()?;
            let block = self.focusable_entity_by_id(reference.entity_id)?;
            let range = block
                .read(cx)
                .current_range_for_footnote_occurrence(reference.occurrence_index)
                .unwrap_or(0..0);
            Some((block, range))
        });
        if let Some((block, range)) = target {
            self.pending_virtual_footnote_backref_focus = None;
            self.focus_block_range(&block, range, cx);
        }
    }
}

fn collect_entity_regions(
    block: &Entity<Block>,
    region_index: usize,
    entity_regions: &mut HashMap<EntityId, usize>,
    mounted_entities: &mut HashMap<EntityId, Entity<Block>>,
    cx: &gpui::App,
) {
    entity_regions.insert(block.entity_id(), region_index);
    mounted_entities.insert(block.entity_id(), block.clone());
    for child in &block.read(cx).children {
        collect_entity_regions(child, region_index, entity_regions, mounted_entities, cx);
    }
}

fn build_virtual_footnote_index(
    projection: &PreparedSplitProjection,
    region_index: &VirtualRegionIndex,
) -> (
    HashMap<String, usize>,
    HashMap<String, usize>,
    HashMap<String, usize>,
) {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_FOOTNOTES);
    let events = Parser::new_ext(&projection.source, options).into_offset_iter();
    let mut definitions = HashMap::new();
    let mut reference_order = Vec::new();
    let mut first_reference_regions = HashMap::new();
    let mut seen_references = HashSet::new();
    for (event, range) in events {
        match event {
            Event::Start(Tag::FootnoteDefinition(id)) => {
                if let Some(region) = region_index.region_for_source_offset(range.start) {
                    definitions.entry(id.to_string()).or_insert(region);
                }
            }
            Event::FootnoteReference(id) => {
                let id = id.to_string();
                if seen_references.insert(id.clone()) {
                    if let Some(region) = region_index.region_for_source_offset(range.start) {
                        first_reference_regions.insert(id.clone(), region);
                    }
                    reference_order.push(id);
                }
            }
            _ => {}
        }
    }
    let ordinals = reference_order
        .into_iter()
        .filter(|id| definitions.contains_key(id))
        .enumerate()
        .map(|(index, id)| (id, index + 1))
        .collect();
    (ordinals, definitions, first_reference_regions)
}

impl VirtualRegionIndex {
    pub(super) fn from_projection(projection: &PreparedSplitProjection) -> Self {
        let source_ranges = projection
            .regions
            .iter()
            .map(|region| region.bytes.clone())
            .collect::<Vec<_>>();
        let heights = projection
            .regions
            .iter()
            .map(|region| {
                let line_count = region.lines.len().max(1);
                estimate_region_height(region.kind, line_count)
            })
            .collect::<Vec<_>>();
        Self::from_ranges_and_heights(source_ranges, heights)
    }

    fn from_ranges_and_heights(source_ranges: Vec<Range<usize>>, mut heights: Vec<f32>) -> Self {
        debug_assert_eq!(source_ranges.len(), heights.len());
        for height in &mut heights {
            *height = normalize_height(*height);
        }
        let mut index = Self {
            height_tree: vec![0.0; heights.len() + 1],
            source_ranges,
            heights,
        };
        for region_index in 0..index.heights.len() {
            index.add_height(region_index, f64::from(index.heights[region_index]));
        }
        index
    }

    pub(super) fn len(&self) -> usize {
        self.heights.len()
    }

    pub(super) fn is_empty(&self) -> bool {
        self.heights.is_empty()
    }

    pub(super) fn total_height(&self) -> f32 {
        self.prefix_height(self.len()) as f32
    }

    #[cfg(test)]
    pub(super) fn height(&self, region_index: usize) -> Option<f32> {
        self.heights.get(region_index).copied()
    }

    pub(super) fn top(&self, region_index: usize) -> Option<f32> {
        (region_index <= self.len()).then(|| self.prefix_height(region_index) as f32)
    }

    #[cfg(test)]
    pub(super) fn update_height(&mut self, region_index: usize, measured_height: f32) -> bool {
        let Some(previous) = self.heights.get_mut(region_index) else {
            return false;
        };
        let next = normalize_height(measured_height);
        if (*previous - next).abs() < 0.25 {
            return false;
        }
        let delta = f64::from(next - *previous);
        *previous = next;
        self.add_height(region_index, delta);
        true
    }

    pub(super) fn source_range(&self, region_index: usize) -> Option<Range<usize>> {
        self.source_ranges.get(region_index).cloned()
    }

    /// 区域内文本事务发布后平移后续源码范围，直到新 revision 投影替换整个索引。
    pub(super) fn apply_region_source_len(&mut self, region_index: usize, new_len: usize) -> bool {
        let Some(current) = self.source_ranges.get(region_index).cloned() else {
            return false;
        };
        let old_len = current.len();
        self.source_ranges[region_index].end = current.start.saturating_add(new_len);
        if new_len >= old_len {
            let delta = new_len - old_len;
            for range in &mut self.source_ranges[region_index + 1..] {
                range.start = range.start.saturating_add(delta);
                range.end = range.end.saturating_add(delta);
            }
        } else {
            let delta = old_len - new_len;
            for range in &mut self.source_ranges[region_index + 1..] {
                range.start = range.start.saturating_sub(delta);
                range.end = range.end.saturating_sub(delta);
            }
        }
        true
    }

    pub(super) fn region_at_y(&self, y: f32) -> Option<usize> {
        if self.is_empty() {
            return None;
        }
        let target = f64::from(y.max(0.0));
        if target >= self.prefix_height(self.len()) {
            return Some(self.len() - 1);
        }

        // Fenwick lower_bound：找到 prefix <= target 的最大元素数。
        let mut index = 0usize;
        let mut accumulated = 0.0f64;
        let mut step = highest_power_of_two_not_greater_than(self.len());
        while step > 0 {
            let next = index + step;
            if next <= self.len() && accumulated + self.height_tree[next] <= target {
                index = next;
                accumulated += self.height_tree[next];
            }
            step >>= 1;
        }
        Some(index.min(self.len() - 1))
    }

    pub(super) fn mount_window(
        &self,
        scroll_y: f32,
        viewport_height: f32,
        overdraw: f32,
        focused_region: Option<usize>,
    ) -> VirtualMountWindow {
        if self.is_empty() {
            return VirtualMountWindow {
                regions: 0..0,
                pinned_region: None,
            };
        }
        let band_start = (scroll_y - overdraw).max(0.0);
        let band_end = (scroll_y + viewport_height.max(1.0) + overdraw).max(band_start);
        let start = self.region_at_y(band_start).unwrap_or(0);
        let end = self
            .region_at_y(band_end)
            .map_or(self.len(), |index| (index + 1).min(self.len()));
        let regions = start..end.max(start + 1).min(self.len());
        let pinned_region = focused_region
            .filter(|index| *index < self.len())
            .filter(|index| !regions.contains(index));
        VirtualMountWindow {
            regions,
            pinned_region,
        }
    }

    pub(super) fn region_for_source_offset(&self, source_offset: usize) -> Option<usize> {
        if self.source_ranges.is_empty() {
            return None;
        }
        let insertion = self
            .source_ranges
            .partition_point(|range| range.end < source_offset);
        Some(insertion.min(self.source_ranges.len() - 1))
    }

    #[cfg(test)]
    pub(super) fn source_anchor_at_y(&self, y: f32) -> Option<VirtualSourceAnchor> {
        let region_index = self.region_at_y(y)?;
        let range = self.source_ranges.get(region_index)?;
        let top = self.top(region_index)?;
        let height = self.height(region_index)?.max(MIN_REGION_HEIGHT);
        let fraction = ((y - top) / height).clamp(0.0, 1.0);
        Some(VirtualSourceAnchor {
            region_index,
            // 区域内像素比例单独保存；源码锚点使用保证为 UTF-8 边界的区域起点。
            source_offset: range.start,
            region_fraction: fraction,
        })
    }

    #[cfg(test)]
    pub(super) fn y_for_source_anchor(&self, anchor: VirtualSourceAnchor) -> Option<f32> {
        let top = self.top(anchor.region_index)?;
        let height = self.height(anchor.region_index)?;
        Some(top + height * anchor.region_fraction.clamp(0.0, 1.0))
    }

    fn prefix_height(&self, count: usize) -> f64 {
        let mut cursor = count.min(self.len());
        let mut sum = 0.0;
        while cursor > 0 {
            sum += self.height_tree[cursor];
            cursor &= cursor - 1;
        }
        sum
    }

    fn add_height(&mut self, region_index: usize, delta: f64) {
        let mut cursor = region_index + 1;
        while cursor < self.height_tree.len() {
            self.height_tree[cursor] += delta;
            cursor += cursor & cursor.wrapping_neg();
        }
    }
}

fn normalize_height(height: f32) -> f32 {
    if height.is_finite() {
        height.max(MIN_REGION_HEIGHT)
    } else {
        MIN_REGION_HEIGHT
    }
}

fn highest_power_of_two_not_greater_than(value: usize) -> usize {
    if value == 0 {
        return 0;
    }
    1usize << (usize::BITS - 1 - value.leading_zeros())
}

fn estimate_region_height(kind: ProjectionRegionKind, line_count: usize) -> f32 {
    if kind == ProjectionRegionKind::Blank {
        // 空白区是源码映射边界，不是可见内容；只保留索引要求的最小正高度。
        return MIN_REGION_HEIGHT;
    }
    let line_height = match kind {
        ProjectionRegionKind::Blank => unreachable!("blank region handled above"),
        ProjectionRegionKind::AtxHeading | ProjectionRegionKind::SetextHeading => 34.0,
        ProjectionRegionKind::Separator => 20.0,
        ProjectionRegionKind::StandaloneImage => 220.0,
        ProjectionRegionKind::StandaloneResource => 96.0,
        ProjectionRegionKind::RootTableCandidate | ProjectionRegionKind::PipelessTable => 32.0,
        ProjectionRegionKind::FencedCode
        | ProjectionRegionKind::Frontmatter
        | ProjectionRegionKind::IndentedCode
        | ProjectionRegionKind::DisplayMath
        | ProjectionRegionKind::Html
        | ProjectionRegionKind::Comment => 25.0,
        ProjectionRegionKind::List
        | ProjectionRegionKind::Quote
        | ProjectionRegionKind::FootnoteDefinition => 28.0,
        ProjectionRegionKind::ReferenceDefinition | ProjectionRegionKind::Paragraph => 24.0,
    };
    line_height * line_count as f32
}
