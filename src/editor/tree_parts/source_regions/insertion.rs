// @author kongweiguang

use std::ops::Range;

use gpui::{Entity, EntityId};

use super::{
    DocumentTree, PlannedSourceRegion, ResidentSourceRegionChange, ResidentSourceRegionPlan,
    SourceRegionBindings, current_range_for_ids, unique_positions,
};

impl DocumentTree {
    /// 为边界新增根保留相邻源码；只有前区域自身变化时才将新增根并入该区域替换。
    pub(super) fn plan_pure_root_insertion(
        &self,
        bindings: &SourceRegionBindings,
        insertion: usize,
        current_end: usize,
        old_ids: &[EntityId],
        old_owners: &[usize],
        cx: &gpui::App,
    ) -> Result<Option<ResidentSourceRegionPlan>, String> {
        if old_ids.len() != old_owners.len()
            || insertion > old_ids.len()
            || current_end <= insertion
            || current_end > self.roots.len()
        {
            return Err("新增根范围无法映射到源码区域。".to_string());
        }

        let current_ids = self.current_root_ids();
        let positions = unique_positions(&current_ids)
            .ok_or_else(|| "当前根序列包含重复实体身份。".to_string())?;
        let inserted_roots = self
            .roots
            .get(insertion..current_end)
            .ok_or_else(|| "新增根实体范围已失效。".to_string())?;
        let markdown = DocumentTree::markdown_text_for_roots(inserted_roots, cx);
        // Enter 新增的空段仍拥有实体身份和源码间隙；空正文不能被误判为没有提交权限。

        let previous_owner = insertion
            .checked_sub(1)
            .and_then(|index| old_owners.get(index).copied());
        let next_owner = old_owners.get(insertion).copied();
        if previous_owner.is_some() && previous_owner == next_owner {
            return Ok(None);
        }
        for owner in [previous_owner, next_owner].into_iter().flatten() {
            if owner >= bindings.regions.len() {
                return Err("新增根相邻区域索引已失效。".to_string());
            }
        }

        let unchanged = bindings
            .regions
            .iter()
            .map(|region| self.region_matches_current(region, &positions, cx))
            .collect::<Result<Vec<_>, _>>()?;
        let dirty = unchanged
            .iter()
            .enumerate()
            .filter_map(|(index, unchanged)| (!unchanged).then_some(index))
            .collect::<Vec<_>>();

        if dirty.is_empty() {
            let Some((anchor, insertion_region, prefix_len, suffix_len)) =
                insertion_anchor(bindings, previous_owner, next_owner)?
            else {
                return Ok(None);
            };
            return self
                .plan_gap_insertion(
                    bindings,
                    insertion_region,
                    anchor,
                    insertion..current_end,
                    &current_ids,
                    &markdown,
                    prefix_len,
                    suffix_len,
                    cx,
                )
                .map(Some);
        }

        if let Some(previous) = previous_owner
            && dirty.as_slice() == [previous]
            && next_owner.is_none_or(|next| unchanged.get(next) == Some(&true))
        {
            return self.plan_previous_owner_insertion(
                bindings,
                previous,
                insertion,
                current_end,
                &positions,
                cx,
            );
        }

        Ok(None)
    }

    /// 只判断单个旧区域的实体连续性与 Markdown 基线是否仍匹配。
    fn region_matches_current(
        &self,
        region: &super::SourceRegion,
        positions: &std::collections::HashMap<EntityId, usize>,
        cx: &gpui::App,
    ) -> Result<bool, String> {
        if region.root_ids.is_empty() {
            return Ok(true);
        }
        let Some(root_range) = current_range_for_ids(&region.root_ids, positions) else {
            return Ok(false);
        };
        let roots = self
            .roots
            .get(root_range)
            .ok_or_else(|| "相邻源码区域根范围已失效。".to_string())?;
        Ok(DocumentTree::markdown_text_for_roots(roots, cx) == region.markdown)
    }

    /// 在源码边界插入独立根，仅改写分隔符和新增正文并保留旧区域字节。
    fn plan_gap_insertion(
        &self,
        bindings: &SourceRegionBindings,
        insertion_region: usize,
        anchor: usize,
        inserted_range: Range<usize>,
        current_ids: &[EntityId],
        markdown: &str,
        body_prefix_len: usize,
        body_suffix_len: usize,
        cx: &gpui::App,
    ) -> Result<ResidentSourceRegionPlan, String> {
        let replacement_len = body_prefix_len
            .checked_add(markdown.len())
            .and_then(|length| length.checked_add(body_suffix_len))
            .ok_or_else(|| "新增源码范围长度溢出。".to_string())?;
        let inserted_end = inserted_range
            .start
            .checked_add(inserted_range.end - inserted_range.start)
            .ok_or_else(|| "新增根范围溢出。".to_string())?;
        let inserted_ids = current_ids
            .get(inserted_range.clone())
            .ok_or_else(|| "新增根身份范围已失效。".to_string())?;
        let mut cursor = 0usize;
        let mut planned = Vec::with_capacity(bindings.regions.len() + 1);
        for index in 0..=bindings.regions.len() {
            if index == insertion_region {
                if cursor != inserted_range.start {
                    return Err("源码插入点与当前根边界不一致。".to_string());
                }
                planned.push(PlannedSourceRegion {
                    source: anchor..anchor,
                    roots: inserted_range.start..inserted_end,
                    root_ids: inserted_ids.to_vec(),
                    markdown: markdown.to_string(),
                    includes_mutation: true,
                    body_prefix_len,
                    body_suffix_len,
                });
                cursor = inserted_end;
            }
            let Some(region) = bindings.regions.get(index) else {
                continue;
            };
            let end = cursor
                .checked_add(region.root_ids.len())
                .ok_or_else(|| "相邻根范围溢出。".to_string())?;
            let roots = self
                .roots
                .get(cursor..end)
                .ok_or_else(|| "未变化源码区域根范围已失效。".to_string())?;
            let root_ids = roots.iter().map(Entity::entity_id).collect::<Vec<_>>();
            if root_ids != region.root_ids {
                return Err("插入计划影响了未变化源码区域。".to_string());
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
        if cursor != self.roots.len()
            || planned
                .iter()
                .filter(|region| region.includes_mutation)
                .count()
                != 1
        {
            return Err("插入计划没有覆盖全部根实体。".to_string());
        }

        let replacement = format!(
            "{}{}{}",
            "\n".repeat(body_prefix_len),
            markdown,
            "\n".repeat(body_suffix_len)
        );
        if replacement.len() != replacement_len {
            return Err("新增源码分隔符长度不一致。".to_string());
        }
        Ok(ResidentSourceRegionPlan {
            revision: bindings.revision,
            changes: vec![ResidentSourceRegionChange {
                source: anchor..anchor,
                markdown: replacement,
            }],
            regions: planned,
            base_region_count: bindings.regions.len(),
        })
    }

    /// 将内容变化限制在前区域，同时把其后新根纳入同一提交以保住稳定后缀。
    fn plan_previous_owner_insertion(
        &self,
        bindings: &SourceRegionBindings,
        previous: usize,
        insertion: usize,
        current_end: usize,
        positions: &std::collections::HashMap<EntityId, usize>,
        cx: &gpui::App,
    ) -> Result<Option<ResidentSourceRegionPlan>, String> {
        let region = bindings
            .regions
            .get(previous)
            .ok_or_else(|| "变化源码区域索引已失效。".to_string())?;
        let Some(previous_roots) = current_range_for_ids(&region.root_ids, positions) else {
            return Ok(None);
        };
        if previous_roots.end != insertion || current_end < previous_roots.end {
            return Ok(None);
        }
        let roots = self
            .roots
            .get(previous_roots.start..current_end)
            .ok_or_else(|| "变化源码区域与新增根范围已失效。".to_string())?;
        let markdown = DocumentTree::markdown_text_for_roots(roots, cx);
        let replacement_ids = roots.iter().map(Entity::entity_id).collect::<Vec<_>>();
        let mut cursor = 0usize;
        let mut planned = Vec::with_capacity(bindings.regions.len());
        for (index, old_region) in bindings.regions.iter().enumerate() {
            if index == previous {
                if cursor != previous_roots.start {
                    return Ok(None);
                }
                planned.push(PlannedSourceRegion {
                    source: old_region.source.clone(),
                    roots: previous_roots.start..current_end,
                    root_ids: replacement_ids.clone(),
                    markdown: markdown.clone(),
                    includes_mutation: true,
                    body_prefix_len: 0,
                    body_suffix_len: 0,
                });
                cursor = current_end;
                continue;
            }
            let end = cursor
                .checked_add(old_region.root_ids.len())
                .ok_or_else(|| "相邻根范围溢出。".to_string())?;
            let roots = self
                .roots
                .get(cursor..end)
                .ok_or_else(|| "未变化源码区域根范围已失效。".to_string())?;
            let root_ids = roots.iter().map(Entity::entity_id).collect::<Vec<_>>();
            if root_ids != old_region.root_ids {
                return Ok(None);
            }
            planned.push(PlannedSourceRegion {
                source: old_region.source.clone(),
                roots: cursor..end,
                root_ids,
                markdown: DocumentTree::markdown_text_for_roots(roots, cx),
                includes_mutation: false,
                body_prefix_len: 0,
                body_suffix_len: 0,
            });
            cursor = end;
        }
        if cursor != self.roots.len()
            || planned
                .iter()
                .filter(|candidate| candidate.includes_mutation)
                .count()
                != 1
        {
            return Ok(None);
        }
        Ok(Some(ResidentSourceRegionPlan {
            revision: bindings.revision,
            changes: vec![ResidentSourceRegionChange {
                source: region.source.clone(),
                markdown,
            }],
            regions: planned,
            base_region_count: bindings.regions.len(),
        }))
    }
}

/// 选择唯一源码锚点，并确定新增根应处于旧区域列表的哪个边界。
fn insertion_anchor(
    bindings: &SourceRegionBindings,
    previous_owner: Option<usize>,
    next_owner: Option<usize>,
) -> Result<Option<(usize, usize, usize, usize)>, String> {
    match (previous_owner, next_owner) {
        (Some(previous), Some(next)) => {
            let previous_region = bindings
                .regions
                .get(previous)
                .ok_or_else(|| "前置源码区域索引已失效。".to_string())?;
            let next_region = bindings
                .regions
                .get(next)
                .ok_or_else(|| "后续源码区域索引已失效。".to_string())?;
            if previous_region.source.end > next_region.source.start {
                return Err("相邻源码区域重叠，无法确定插入锚点。".to_string());
            }
            Ok(Some((previous_region.source.end, previous + 1, 2, 0)))
        }
        (None, Some(next)) => {
            let next_region = bindings
                .regions
                .get(next)
                .ok_or_else(|| "首个源码区域索引已失效。".to_string())?;
            Ok(Some((next_region.source.start, next, 0, 2)))
        }
        (Some(previous), None) => {
            let previous_region = bindings
                .regions
                .get(previous)
                .ok_or_else(|| "末尾源码区域索引已失效。".to_string())?;
            Ok(Some((previous_region.source.end, previous + 1, 2, 0)))
        }
        (None, None) => Ok(None),
    }
}
