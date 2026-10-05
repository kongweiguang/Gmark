// @author kongweiguang

//! Runtime ownership for the editor block tree.
//!
//! [`DocumentTree`] is the only mutable owner of block ordering and parent-child
//! relationships inside the editor. It also maintains a cached
//! [`VisibleTreeSnapshot`] so hot-path lookups do not re-run a full DFS on every
//! focus, scroll, or mutation event.

use std::collections::HashMap;

use gpui::*;

use super::{Editor, persistence};
use crate::components::serialize_table_markdown_lines;
use crate::components::{Block, BlockKind, CalloutVariant, parse_standalone_image};

/// A block together with its position in the current visible DFS order.
#[derive(Clone)]
pub(super) struct VisibleBlock {
    pub entity: Entity<Block>,
}

/// A block's position inside the runtime tree.
#[derive(Clone)]
pub(super) struct BlockLocation {
    pub parent: Option<Entity<Block>>,
    pub index: usize,
}

/// Cached visible-order metadata for the current runtime tree.
#[derive(Default, Clone)]
pub(super) struct VisibleTreeSnapshot {
    visible: Vec<VisibleBlock>,
    visible_index_by_entity: HashMap<EntityId, usize>,
    location_by_entity: HashMap<EntityId, BlockLocation>,
    last_visible_descendant_by_entity: HashMap<EntityId, EntityId>,
}

impl VisibleTreeSnapshot {
    fn clear(&mut self) {
        self.visible.clear();
        self.visible_index_by_entity.clear();
        self.location_by_entity.clear();
        self.last_visible_descendant_by_entity.clear();
    }
}

#[path = "tree_parts/model.rs"]
mod model;
#[path = "tree_parts/source_recovery.rs"]
mod source_recovery;
#[path = "tree_parts/source_regions.rs"]
mod source_regions;

/// Canonical owner of the runtime block tree.
///
/// The Markdown importer builds root blocks and nested list children, then
/// hands the structure to `DocumentTree`. From that point on, every structural
/// edit must go through this type so the runtime tree stays aligned with the
/// subset of Markdown that the importer and serializer can reconstruct.
pub(super) struct DocumentTree {
    roots: Vec<Entity<Block>>,
    snapshot: VisibleTreeSnapshot,
    /// 根块级 Markdown 缓存；普通输入只重建所属根块，结构事务才全量重建。
    root_markdown_cache: Vec<RootMarkdownCache>,
    /// 当前投影区域在规范 LF 源文档及根块序列中的对应关系。
    source_regions: Option<source_regions::SourceRegionBindings>,
    /// 最近一次局部 Source 提交失败，随当前树和标签保留以供恢复 UI 读取。
    source_commit_error: Option<String>,
    /// 等待用户恢复的跨根输入，避免提交失败时载荷只存在于事件栈中。
    source_commit_replacement: Option<String>,
}

struct RootMarkdownCache {
    lines: Vec<String>,
    is_empty_paragraph: bool,
    is_list_item: bool,
}

impl DocumentTree {
    /// 新树从未绑定投影或提交错误开始，避免身份跨文档复用。
    pub(super) fn new(roots: Vec<Entity<Block>>) -> Self {
        Self {
            roots,
            snapshot: VisibleTreeSnapshot::default(),
            root_markdown_cache: Vec::new(),
            source_regions: None,
            source_commit_error: None,
            source_commit_replacement: None,
        }
    }

    pub(super) fn first_root(&self) -> Option<&Entity<Block>> {
        self.roots.first()
    }

    pub(super) fn root_blocks(&self) -> &[Entity<Block>] {
        &self.roots
    }

    pub(super) fn root_count(&self) -> usize {
        self.roots.len()
    }

    pub(super) fn visible_blocks(&self) -> &[VisibleBlock] {
        &self.snapshot.visible
    }

    pub(super) fn flatten_visible_blocks(&self) -> Vec<VisibleBlock> {
        self.snapshot.visible.clone()
    }

    pub(super) fn focused_block_entity_id(&self, window: &Window, cx: &App) -> Option<EntityId> {
        self.snapshot
            .visible
            .iter()
            .find(|visible| visible.entity.read(cx).focus_handle.is_focused(window))
            .map(|visible| visible.entity.entity_id())
    }

    pub(super) fn visible_index_for_entity_id(&self, entity_id: EntityId) -> Option<usize> {
        self.snapshot
            .visible_index_by_entity
            .get(&entity_id)
            .copied()
    }

    pub(super) fn block_entity_by_id(&self, entity_id: EntityId) -> Option<Entity<Block>> {
        self.visible_index_for_entity_id(entity_id)
            .and_then(|index| self.snapshot.visible.get(index))
            .map(|visible| visible.entity.clone())
    }

    pub(super) fn find_block_location(&self, entity_id: EntityId) -> Option<BlockLocation> {
        self.snapshot.location_by_entity.get(&entity_id).cloned()
    }

    /// Returns the sibling immediately before `entity_id` within the same
    /// parent, if any.
    pub(super) fn previous_sibling(&self, entity_id: EntityId, cx: &App) -> Option<Entity<Block>> {
        let location = self.find_block_location(entity_id)?;
        let prev_index = location.index.checked_sub(1)?;
        match &location.parent {
            Some(parent) => parent.read(cx).children.get(prev_index).cloned(),
            None => self.roots.get(prev_index).cloned(),
        }
    }

    pub(super) fn last_visible_descendant(&self, entity_id: EntityId) -> Option<Entity<Block>> {
        let descendant_id = self
            .snapshot
            .last_visible_descendant_by_entity
            .get(&entity_id)
            .copied()?;
        self.block_entity_by_id(descendant_id)
    }

    /// 根序列替换会使旧 root-index 范围和局部提交错误失效，必须等待新投影重绑。
    pub(super) fn replace_roots(&mut self, roots: Vec<Entity<Block>>, cx: &mut Context<Editor>) {
        self.roots = roots;
        self.source_regions = None;
        self.source_commit_error = None;
        self.source_commit_replacement = None;
        self.rebuild_metadata_and_snapshot(cx);
    }

    /// 共享投影重建后保留原 IME 输入实体，系统迟到回调仍能找到原 owner。
    pub(super) fn retain_input_entity(
        &mut self,
        replaced: EntityId,
        retained: Entity<Block>,
        cx: &mut Context<Editor>,
    ) -> bool {
        let Some(location) = self.find_block_location(replaced) else {
            return false;
        };
        let changed = match location.parent {
            Some(parent) => parent.update(cx, |parent, _cx| {
                let Some(slot) = parent.children.get_mut(location.index) else {
                    return false;
                };
                *slot = retained;
                true
            }),
            None => match self.roots.get_mut(location.index) {
                Some(slot) => {
                    *slot = retained;
                    true
                }
                None => false,
            },
        };
        if !changed {
            return false;
        }
        self.rebuild_metadata_and_snapshot(cx);
        true
    }

    pub(super) fn markdown_text(&self, cx: &App) -> String {
        Self::markdown_text_for_roots(&self.roots, cx)
    }

    pub(super) fn markdown_text_for_roots(roots: &[Entity<Block>], cx: &App) -> String {
        let mut lines = Vec::new();
        Self::collect_root_markdown_lines(roots, cx, &mut lines);
        lines.join("\n")
    }

    /// 仅供已同步根缓存的 Editor 事务使用；其他调用必须走权威序列化。
    pub(super) fn cached_markdown_text(&self, cx: &App) -> String {
        if self.root_markdown_cache.len() == self.roots.len() {
            return self.markdown_text_from_root_cache();
        }
        self.markdown_text(cx)
    }

    /// 重建发生内容变化的根块缓存，并保留其余长文档根块的已序列化结果。
    pub(super) fn refresh_markdown_cache_for_entity(&mut self, entity_id: EntityId, cx: &App) {
        let Some(root_index) = self.root_index_for_entity(entity_id) else {
            self.rebuild_root_markdown_cache(cx);
            return;
        };
        if self.root_markdown_cache.len() != self.roots.len() {
            self.rebuild_root_markdown_cache(cx);
            return;
        }
        self.root_markdown_cache[root_index] =
            Self::build_root_markdown_cache(self.roots[root_index].read(cx), cx);
    }
}

#[cfg(test)]
#[path = "../../tests/unit/editor/tree.rs"]
mod tests;
