// @author kongweiguang

use std::collections::HashMap;
use std::ops::Range;

use gmark_document::Revision;
use gmark_document_core::{DocumentMutationMap, SourceAffinity, SourceAnchor};
use gpui::{Entity, EntityId};

use super::DocumentTree;
use crate::components::Block;

#[path = "source_regions/insertion.rs"]
mod insertion;

pub(super) struct SourceRegionBindings {
    revision: Revision,
    regions: Vec<SourceRegion>,
}

#[derive(Clone)]
struct SourceRegion {
    source: Range<usize>,
    roots: Range<usize>,
    root_ids: Vec<EntityId>,
    markdown: String,
}

pub(in crate::editor) struct ResidentSourceRegionPlan {
    pub(in crate::editor) revision: Revision,
    pub(in crate::editor) changes: Vec<ResidentSourceRegionChange>,
    regions: Vec<PlannedSourceRegion>,
    base_region_count: usize,
}

pub(in crate::editor) struct ResidentSourceRegionChange {
    pub(in crate::editor) source: Range<usize>,
    pub(in crate::editor) markdown: String,
}

struct PlannedSourceRegion {
    source: Range<usize>,
    roots: Range<usize>,
    root_ids: Vec<EntityId>,
    markdown: String,
    includes_mutation: bool,
    body_prefix_len: usize,
    body_suffix_len: usize,
}

impl DocumentTree {
    /// 投影安装时保存根身份与规范基线；文末空输入根只拥有插入点，不能吃掉段落分隔。
    pub(in crate::editor) fn bind_source_regions(
        &mut self,
        revision: Revision,
        regions: Vec<(Range<usize>, Range<usize>)>,
        cx: &gpui::App,
    ) {
        let mut root_cursor = 0usize;
        let mut bindings = Vec::with_capacity(regions.len());
        for (source, roots) in regions {
            if source.start > source.end || roots.start != root_cursor || roots.end < roots.start {
                self.source_regions = None;
                return;
            }
            let Some(root_blocks) = self.roots.get(roots.clone()) else {
                self.source_regions = None;
                return;
            };
            let markdown = DocumentTree::markdown_text_for_roots(root_blocks, cx);
            let source = if roots.end == self.roots.len()
                && !root_blocks.is_empty()
                && markdown.is_empty()
            {
                source.end..source.end
            } else {
                source
            };
            bindings.push(SourceRegion {
                source,
                roots: roots.clone(),
                root_ids: root_blocks.iter().map(Entity::entity_id).collect(),
                markdown,
            });
            root_cursor = roots.end;
        }
        if root_cursor != self.roots.len() {
            self.source_regions = None;
            return;
        }
        self.source_regions = Some(SourceRegionBindings {
            revision,
            regions: bindings,
        });
    }

    /// 仅在绑定身份仍与根序列一致时返回局部范围，避免结构编辑期间复用过期索引。
    pub(in crate::editor) fn source_region_for_entity(
        &self,
        entity_id: EntityId,
    ) -> Option<(Revision, Range<usize>, Vec<Entity<Block>>)> {
        let bindings = self.source_regions.as_ref()?;
        if !self.root_ids_match_bindings(bindings) {
            return None;
        }
        let root_index = self.root_index_for_entity(entity_id)?;
        let region = bindings
            .regions
            .iter()
            .find(|region| region.roots.contains(&root_index))?;
        let roots = self.roots.get(region.roots.clone())?.to_vec();
        Some((bindings.revision, region.source.clone(), roots))
    }

    /// 根身份差异只授权连续源码组；整组删除同时移除一侧分隔，不能留下两份段落间隙。
    pub(in crate::editor) fn plan_resident_region_changes(
        &self,
        cx: &gpui::App,
    ) -> Result<ResidentSourceRegionPlan, String> {
        let bindings = self
            .source_regions
            .as_ref()
            .ok_or_else(|| "当前投影没有可用的源码区域映射。".to_string())?;
        let mut old_ids = Vec::new();
        let mut old_owners = Vec::new();
        let mut root_cursor = 0usize;
        let mut previous_source_end = 0;
        for (region_index, region) in bindings.regions.iter().enumerate() {
            if region.source.start > region.source.end
                || region.source.start < previous_source_end
                || region.roots.start != root_cursor
                || region.roots.end < region.roots.start
                || region.roots.end - region.roots.start != region.root_ids.len()
            {
                return Err("源码区域映射不连续，无法安全提交结构变化。".to_string());
            }
            previous_source_end = region.source.end;
            root_cursor = region.roots.end;
            old_ids.extend(region.root_ids.iter().copied());
            old_owners.extend(std::iter::repeat_n(region_index, region.root_ids.len()));
        }
        if root_cursor != old_ids.len() {
            return Err("源码区域根身份与原始范围不一致。".to_string());
        }

        let current_ids = self.current_root_ids();
        let current_positions = unique_positions(&current_ids)
            .ok_or_else(|| "当前根序列包含重复实体身份。".to_string())?;
        let mut old_owner_by_id = HashMap::with_capacity(old_ids.len());
        for (index, entity_id) in old_ids.iter().copied().enumerate() {
            if old_owner_by_id
                .insert(entity_id, old_owners[index])
                .is_some()
            {
                return Err("源码区域重复引用同一根实体。".to_string());
            }
        }

        let sequence_change = common_sequence_change(&old_ids, &current_ids);
        if let Some((old_start, old_end, current_end)) = sequence_change
            && old_start == old_end
            && old_start < current_end
            && let Some(plan) = self.plan_pure_root_insertion(
                bindings,
                old_start,
                current_end,
                &old_ids,
                &old_owners,
                cx,
            )?
        {
            return Ok(plan);
        }
        let mut changed_regions = vec![false; bindings.regions.len()];
        if let Some((old_start, old_end, _)) = sequence_change {
            if old_start < old_end {
                let Some(owners) = old_owners.get(old_start..old_end) else {
                    return Err("结构变化根范围无法映射到源码区域。".to_string());
                };
                for region_index in owners {
                    if let Some(changed) = changed_regions.get_mut(*region_index) {
                        *changed = true;
                    }
                }
            } else {
                let insertion = old_start;
                let previous_owner = insertion
                    .checked_sub(1)
                    .and_then(|index| old_owners.get(index).copied());
                let next_owner = old_owners.get(insertion).copied();
                let Some(last_region) = bindings.regions.len().checked_sub(1) else {
                    return Err("新增根没有可授权的源码范围。".to_string());
                };
                let (first, last) = match (previous_owner, next_owner) {
                    (Some(previous), Some(next)) => (previous.min(next), previous.max(next)),
                    (None, Some(next)) => (0, next),
                    (Some(previous), None) => (previous, last_region),
                    (None, None) if bindings.regions.len() == 1 => (0, 0),
                    (None, None) => {
                        return Err("空文档中的新增根无法对应到唯一源码范围。".to_string());
                    }
                };
                let Some(changed) = changed_regions.get_mut(first..=last) else {
                    return Err("新增根边界无法映射到源码区域。".to_string());
                };
                for changed in changed {
                    *changed = true;
                }
            }
        }

        for (region_index, region) in bindings.regions.iter().enumerate() {
            if region.root_ids.is_empty() {
                continue;
            }
            let Some(root_range) = current_range_for_ids(&region.root_ids, &current_positions)
            else {
                if let Some(changed) = changed_regions.get_mut(region_index) {
                    *changed = true;
                }
                continue;
            };
            let Some(roots) = self.roots.get(root_range) else {
                return Err("当前根实体范围已失效。".to_string());
            };
            if DocumentTree::markdown_text_for_roots(roots, cx) != region.markdown
                && let Some(changed) = changed_regions.get_mut(region_index)
            {
                *changed = true;
            }
        }

        let first_changed = changed_regions.iter().position(|changed| *changed);
        let Some(first) = first_changed else {
            let regions = self.plan_unchanged_regions(bindings, cx)?;
            return Ok(ResidentSourceRegionPlan {
                revision: bindings.revision,
                changes: Vec::new(),
                regions,
                base_region_count: bindings.regions.len(),
            });
        };
        let last = changed_regions
            .iter()
            .rposition(|changed| *changed)
            .unwrap_or(first);
        let mut source_start = bindings.regions[first].source.start;
        let mut source_end = bindings.regions[last].source.end;
        if source_start > source_end {
            return Err("受影响源码区域边界倒置。".to_string());
        }

        let left_old_index = bindings.regions[..first]
            .iter()
            .map(|region| region.root_ids.len())
            .sum::<usize>();
        let right_old_index = bindings.regions[..=last]
            .iter()
            .map(|region| region.root_ids.len())
            .sum::<usize>();
        let current_start = left_old_index
            .checked_sub(1)
            .and_then(|index| old_ids.get(index))
            .and_then(|entity_id| current_positions.get(entity_id).copied())
            .map_or(0, |index| index + 1);
        let current_end = old_ids
            .get(right_old_index)
            .and_then(|entity_id| current_positions.get(entity_id).copied())
            .unwrap_or(current_ids.len());
        if current_start > current_end || current_end > current_ids.len() {
            return Err("受影响根实体无法与相邻源码区域对齐。".to_string());
        }
        for entity_id in &current_ids[current_start..current_end] {
            if old_owner_by_id
                .get(entity_id)
                .is_some_and(|owner| *owner < first || *owner > last)
            {
                return Err("结构变化越过未修改源码区域，无法安全提交。".to_string());
            }
        }

        let roots = self
            .roots
            .get(current_start..current_end)
            .ok_or_else(|| "受影响根实体范围已失效。".to_string())?;
        let markdown = DocumentTree::markdown_text_for_roots(roots, cx);
        if markdown.is_empty() {
            // 空白投影没有根实体；删除分隔必须止于下一实体，不能漏下一换行。
            if let Some(next) = bindings.regions[last + 1..]
                .iter()
                .find(|region| !region.root_ids.is_empty())
            {
                source_end = next.source.start;
            } else if let Some(previous) = bindings.regions[..first]
                .iter()
                .rfind(|region| !region.root_ids.is_empty())
            {
                source_start = previous.source.end;
            }
        }
        let change = ResidentSourceRegionChange {
            source: source_start..source_end,
            markdown: markdown.clone(),
        };
        let regions =
            self.plan_changed_group(bindings, first, last, current_start, current_end, cx)?;
        Ok(ResidentSourceRegionPlan {
            revision: bindings.revision,
            changes: vec![change],
            regions,
            base_region_count: bindings.regions.len(),
        })
    }

    /// 提交后按 mutation 映射更新源范围，并以计划中的实体身份重建 root-index。
    pub(in crate::editor) fn rebind_source_regions_after_commit(
        &mut self,
        plan: ResidentSourceRegionPlan,
        mutation: &DocumentMutationMap,
        new_revision: Revision,
        cx: &gpui::App,
    ) -> bool {
        let Some(current) = self.source_regions.as_ref() else {
            return false;
        };
        if current.revision != plan.revision
            || current.regions.len() != plan.base_region_count
            || new_revision.get() < plan.revision.get()
            || (new_revision == plan.revision && !mutation.edits().is_empty())
            || (plan.changes.is_empty() && !mutation.edits().is_empty())
            || !self.matches_planned_roots(&plan.regions, cx)
        {
            return false;
        }

        let changed_region = plan
            .regions
            .iter()
            .position(|region| region.includes_mutation);
        if changed_region.is_some() == plan.changes.is_empty() {
            return false;
        }
        let Some(regions) = plan
            .regions
            .into_iter()
            .enumerate()
            .map(|(index, region)| {
                let before_changed = changed_region.is_some_and(|changed| index < changed);
                let mapped_source = map_region_range(
                    mutation,
                    region.source,
                    region.includes_mutation,
                    before_changed,
                )?;
                let source = mapped_source.start.checked_add(region.body_prefix_len)?
                    ..mapped_source.end.checked_sub(region.body_suffix_len)?;
                if source.start > source.end
                    || (region.includes_mutation
                        && source.end - source.start != region.markdown.len())
                {
                    return None;
                }
                Some(SourceRegion {
                    source,
                    roots: region.roots,
                    root_ids: region.root_ids,
                    markdown: region.markdown,
                })
            })
            .collect::<Option<Vec<_>>>()
        else {
            return false;
        };
        if !source_ranges_are_ordered(&regions) {
            return false;
        }
        self.source_regions = Some(SourceRegionBindings {
            revision: new_revision,
            regions,
        });
        true
    }

    /// 普通单根提交只推进所属区域基线，拒绝夹带未提交的根序列变化。
    pub(in crate::editor) fn advance_source_regions(
        &mut self,
        mutation: &DocumentMutationMap,
        entity_id: EntityId,
        new_revision: Revision,
        cx: &gpui::App,
    ) -> bool {
        let Some(current) = self.source_regions.as_ref() else {
            return false;
        };
        if new_revision.get() <= current.revision.get() || !self.root_ids_match_bindings(current) {
            return false;
        }
        let Some(root_index) = self.root_index_for_entity(entity_id) else {
            return false;
        };
        let Some(root) = self.roots.get(root_index) else {
            return false;
        };
        let root_id = root.entity_id();
        let Some(target_region) = current
            .regions
            .iter()
            .position(|region| region.root_ids.contains(&root_id))
        else {
            return false;
        };
        let mut root_cursor = 0usize;
        let mut regions = Vec::with_capacity(current.regions.len());
        for (index, region) in current.regions.iter().enumerate() {
            let Some(root_end) = root_cursor.checked_add(region.root_ids.len()) else {
                return false;
            };
            let Some(roots) = self.roots.get(root_cursor..root_end) else {
                return false;
            };
            let root_ids = roots.iter().map(Entity::entity_id).collect::<Vec<_>>();
            if root_ids != region.root_ids {
                return false;
            }
            let before_changed = index < target_region;
            let Some(source) = map_region_range(
                mutation,
                region.source.clone(),
                index == target_region,
                before_changed,
            ) else {
                return false;
            };
            regions.push(SourceRegion {
                source,
                roots: root_cursor..root_end,
                root_ids,
                markdown: if index == target_region {
                    DocumentTree::markdown_text_for_roots(roots, cx)
                } else {
                    region.markdown.clone()
                },
            });
            root_cursor = root_end;
        }
        if root_cursor != self.roots.len() || !source_ranges_are_ordered(&regions) {
            return false;
        }
        self.source_regions = Some(SourceRegionBindings {
            revision: new_revision,
            regions,
        });
        true
    }

    /// 供 Source 提交失败 UI 读取当前树所持有的恢复说明。
    pub(in crate::editor) fn source_commit_error(&self) -> Option<&str> {
        self.source_commit_error.as_deref()
    }

    /// 清除提交错误时同时清除已确认处理的跨区替换载荷。
    pub(in crate::editor) fn set_source_commit_error(&mut self, error: Option<String>) {
        if error.is_none() {
            self.source_commit_replacement = None;
        }
        self.source_commit_error = error;
    }

    /// 为未改变的投影区域重建当前 root-index 和规范基线快照。
    fn plan_unchanged_regions(
        &self,
        bindings: &SourceRegionBindings,
        cx: &gpui::App,
    ) -> Result<Vec<PlannedSourceRegion>, String> {
        let mut cursor = 0usize;
        let mut planned = Vec::with_capacity(bindings.regions.len());
        for region in &bindings.regions {
            let end = cursor
                .checked_add(region.root_ids.len())
                .ok_or_else(|| "源码区域根范围溢出。".to_string())?;
            let roots = self
                .roots
                .get(cursor..end)
                .ok_or_else(|| "源码区域根范围已失效。".to_string())?;
            let root_ids = roots.iter().map(Entity::entity_id).collect::<Vec<_>>();
            if root_ids != region.root_ids {
                return Err("未变化源码区域的根身份已偏移。".to_string());
            }
            planned.push(PlannedSourceRegion {
                source: region.source.clone(),
                roots: cursor..end,
                root_ids,
                markdown: DocumentTree::markdown_text_for_roots(roots, cx),
                includes_mutation: false,
                body_prefix_len: 0,
                body_suffix_len: 0,
            });
            cursor = end;
        }
        if cursor != self.roots.len() {
            return Err("根序列包含未绑定的实体。".to_string());
        }
        Ok(planned)
    }

    /// 把变化首尾合并为一个区域，使内部列表上下文和源码分隔符由同次提交处理。
    fn plan_changed_group(
        &self,
        bindings: &SourceRegionBindings,
        first: usize,
        last: usize,
        current_start: usize,
        current_end: usize,
        cx: &gpui::App,
    ) -> Result<Vec<PlannedSourceRegion>, String> {
        let mut planned = Vec::with_capacity(first + 1 + bindings.regions.len() - last - 1);
        let mut cursor = 0usize;
        for region in &bindings.regions[..first] {
            let end = cursor
                .checked_add(region.root_ids.len())
                .ok_or_else(|| "前缀根范围溢出。".to_string())?;
            let roots = self
                .roots
                .get(cursor..end)
                .ok_or_else(|| "未修改前缀根范围已失效。".to_string())?;
            let root_ids = roots.iter().map(Entity::entity_id).collect::<Vec<_>>();
            if root_ids != region.root_ids {
                return Err("结构提交影响了未修改源码前缀。".to_string());
            }
            planned.push(PlannedSourceRegion {
                source: region.source.clone(),
                roots: cursor..end,
                root_ids,
                markdown: DocumentTree::markdown_text_for_roots(roots, cx),
                includes_mutation: false,
                body_prefix_len: 0,
                body_suffix_len: 0,
            });
            cursor = end;
        }
        if cursor != current_start {
            return Err("变化区域无法与未修改前缀的根边界对齐。".to_string());
        }
        let group_roots = self
            .roots
            .get(current_start..current_end)
            .ok_or_else(|| "变化区域根范围已失效。".to_string())?;
        let group_ids = group_roots
            .iter()
            .map(Entity::entity_id)
            .collect::<Vec<_>>();
        let source = bindings.regions[first].source.start..bindings.regions[last].source.end;
        planned.push(PlannedSourceRegion {
            source,
            roots: current_start..current_end,
            root_ids: group_ids,
            markdown: DocumentTree::markdown_text_for_roots(group_roots, cx),
            includes_mutation: true,
            body_prefix_len: 0,
            body_suffix_len: 0,
        });
        cursor = current_end;
        for region in &bindings.regions[last + 1..] {
            let end = cursor
                .checked_add(region.root_ids.len())
                .ok_or_else(|| "后缀根范围溢出。".to_string())?;
            let roots = self
                .roots
                .get(cursor..end)
                .ok_or_else(|| "未修改后缀根范围已失效。".to_string())?;
            let root_ids = roots.iter().map(Entity::entity_id).collect::<Vec<_>>();
            if root_ids != region.root_ids {
                return Err("结构提交影响了未修改源码后缀。".to_string());
            }
            planned.push(PlannedSourceRegion {
                source: region.source.clone(),
                roots: cursor..end,
                root_ids,
                markdown: DocumentTree::markdown_text_for_roots(roots, cx),
                includes_mutation: false,
                body_prefix_len: 0,
                body_suffix_len: 0,
            });
            cursor = end;
        }
        if cursor != self.roots.len() {
            return Err("结构提交后仍有未绑定的根实体。".to_string());
        }
        Ok(planned)
    }

    /// 确认计划仍描述当前根身份和内容，避免异步提交期间覆盖较新的树状态。
    fn matches_planned_roots(&self, planned: &[PlannedSourceRegion], cx: &gpui::App) -> bool {
        let mut cursor = 0usize;
        for region in planned {
            if region.roots.start != cursor || region.roots.end < region.roots.start {
                return false;
            }
            let Some(roots) = self.roots.get(region.roots.clone()) else {
                return false;
            };
            if roots.len() != region.root_ids.len()
                || roots
                    .iter()
                    .map(Entity::entity_id)
                    .ne(region.root_ids.iter().copied())
                || DocumentTree::markdown_text_for_roots(roots, cx) != region.markdown
            {
                return false;
            }
            cursor = region.roots.end;
        }
        cursor == self.roots.len()
    }

    /// 比较当前顶层根序列与绑定快照，任何结构差异都交给区域计划处理。
    fn root_ids_match_bindings(&self, bindings: &SourceRegionBindings) -> bool {
        let current_ids = self.current_root_ids();
        let mut cursor = 0usize;
        for region in &bindings.regions {
            let Some(end) = cursor.checked_add(region.root_ids.len()) else {
                return false;
            };
            if current_ids.get(cursor..end) != Some(region.root_ids.as_slice()) {
                return false;
            }
            cursor = end;
        }
        cursor == current_ids.len()
    }

    /// 返回当前顶层根的稳定 Entity 身份序列。
    fn current_root_ids(&self) -> Vec<EntityId> {
        self.roots.iter().map(Entity::entity_id).collect()
    }
}

/// 唯一化实体位置索引，发现重复身份时让调用方拒绝建立来源映射。
fn unique_positions(ids: &[EntityId]) -> Option<HashMap<EntityId, usize>> {
    let mut positions = HashMap::with_capacity(ids.len());
    for (index, entity_id) in ids.iter().copied().enumerate() {
        if positions.insert(entity_id, index).is_some() {
            return None;
        }
    }
    Some(positions)
}

/// 找到两条根身份序列共享的首尾锚点，中央区间就是唯一需要重新归属的部分。
fn common_sequence_change(old: &[EntityId], current: &[EntityId]) -> Option<(usize, usize, usize)> {
    if old == current {
        return None;
    }
    let mut prefix = 0;
    while prefix < old.len() && prefix < current.len() && old[prefix] == current[prefix] {
        prefix += 1;
    }
    let mut suffix = 0;
    while suffix < old.len().saturating_sub(prefix)
        && suffix < current.len().saturating_sub(prefix)
        && old[old.len() - suffix - 1] == current[current.len() - suffix - 1]
    {
        suffix += 1;
    }
    Some((prefix, old.len() - suffix, current.len() - suffix))
}

/// 只在区域内 Entity 身份连续且顺序不变时返回其当前根范围。
fn current_range_for_ids(
    ids: &[EntityId],
    positions: &HashMap<EntityId, usize>,
) -> Option<Range<usize>> {
    let first = *positions.get(ids.first()?)?;
    let end = first.checked_add(ids.len())?;
    ids.iter()
        .enumerate()
        .all(|(offset, entity_id)| positions.get(entity_id) == first.checked_add(offset).as_ref())
        .then_some(first..end)
}

/// 按事务边界语义映射源码区域，避免相邻未修改范围吞入替换内容。
fn map_region_range(
    mutation: &DocumentMutationMap,
    source: Range<usize>,
    includes_mutation: bool,
    before_changed: bool,
) -> Option<Range<usize>> {
    let (start_affinity, end_affinity) = if includes_mutation {
        (SourceAffinity::Before, SourceAffinity::After)
    } else if source.is_empty() {
        let affinity = if before_changed {
            SourceAffinity::Before
        } else {
            SourceAffinity::After
        };
        (affinity, affinity)
    } else {
        (SourceAffinity::After, SourceAffinity::Before)
    };
    let start = u64::try_from(source.start).ok()?;
    let end = u64::try_from(source.end).ok()?;
    let mapped_start = mutation
        .map_anchor(SourceAnchor::new(start, start_affinity))
        .byte_offset;
    let mapped_end = mutation
        .map_anchor(SourceAnchor::new(end, end_affinity))
        .byte_offset;
    let source = usize::try_from(mapped_start).ok()?..usize::try_from(mapped_end).ok()?;
    (source.start <= source.end).then_some(source)
}

/// 确认重新绑定后的源码范围仍按原投影顺序排列且互不重叠。
fn source_ranges_are_ordered(regions: &[SourceRegion]) -> bool {
    regions.windows(2).all(|pair| {
        pair[0].source.start <= pair[1].source.start && pair[0].source.end <= pair[1].source.start
    })
}
