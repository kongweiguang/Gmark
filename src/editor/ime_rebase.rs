// @author kongweiguang

//! 共享事务只重定位未冲突的候选范围，绝不把旧投影写回 Controller。

use super::*;
use crate::components::{BlockImeComposition, BlockImeCompositionOwner};
use gmark_document_core::DocumentMutationMap;

pub(super) struct RebasedImeSession {
    block: Entity<Block>,
    composition: BlockImeComposition,
    source_range: std::ops::Range<usize>,
    original_selection: SourceSelection,
    target_range: std::ops::Range<usize>,
    conflicted: bool,
}

impl Editor {
    /// 事件游标落后时缺少完整 mutation 链，只能取消候选，不能猜测范围。
    pub(super) fn invalidate_shared_ime_sessions(sessions: &mut [RebasedImeSession]) {
        for session in sessions {
            session.conflicted = true;
        }
    }

    /// 暂存范围先回到源码坐标；take 恢复原选择后才做映射，避免使用预编辑偏移。
    pub(super) fn capture_ime_for_shared_rebase(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Vec<RebasedImeSession> {
        let mappings = self.build_source_target_mappings(cx);
        let mut sessions = Vec::new();
        for mapping in mappings {
            let block = mapping.entity.clone();
            let Some(composition) = block.update(cx, |block, _cx| block.take_ime_composition())
            else {
                continue;
            };
            let to_source = |range: std::ops::Range<usize>| {
                let range = block.read(cx).current_range_to_markdown_range(range);
                let start =
                    *mapping.content_to_source.get(range.start)? + mapping.full_source_range.start;
                let end =
                    *mapping.content_to_source.get(range.end)? + mapping.full_source_range.start;
                Some(start..end)
            };
            let normal = matches!(composition.owner, BlockImeCompositionOwner::BlockText);
            let range = if normal {
                to_source(composition.replacement_range.clone())
            } else {
                Some(mapping.full_source_range.clone())
            };
            let selection = if normal {
                to_source(composition.original_selection.range.clone())
            } else {
                range.clone()
            };
            let Some((range, selection)) = range.zip(selection) else {
                block.update(cx, |block, _cx| block.ime_reject_until_end = true);
                self.ime_detached_targets.push(block);
                continue;
            };
            sessions.push(RebasedImeSession {
                block,
                original_selection: SourceSelection::from_range(
                    selection.start as u64..selection.end as u64,
                    composition.original_selection.reversed,
                ),
                composition,
                source_range: range,
                target_range: mapping.full_source_range,
                conflicted: false,
            });
        }
        sessions
    }

    /// 插入发生在候选内部或替换覆盖候选时取消；无交集事务按既有 mutation map 重定位。
    pub(super) fn map_ime_shared_mutation(
        sessions: &mut [RebasedImeSession],
        mutation: &DocumentMutationMap,
    ) {
        for session in sessions {
            session.conflicted |= mutation.edits().iter().any(|edit| {
                let range = &session.source_range;
                if edit.range.is_empty() {
                    range.start as u64 <= edit.range.start && edit.range.start <= range.end as u64
                } else {
                    edit.range.start < range.end as u64 && (range.start as u64) < edit.range.end
                        || range.is_empty()
                            && edit.range.start <= range.start as u64
                            && (range.start as u64) < edit.range.end
                }
            });
            let map = |range: std::ops::Range<usize>| {
                let range = mutation
                    .map_selection(SourceSelection::from_range(
                        range.start as u64..range.end as u64,
                        false,
                    ))
                    .range();
                range.start as usize..range.end as usize
            };
            session.source_range = map(session.source_range.clone());
            session.target_range = map(session.target_range.clone());
            session.original_selection = mutation.map_selection(session.original_selection);
        }
    }

    /// 原 owner 留在同一实体，记录可刷新；身份/结构/片段不再匹配时丢弃暂存而不覆盖新正文。
    pub(super) fn restore_ime_after_shared_rebase(
        &mut self,
        sessions: Vec<RebasedImeSession>,
        cx: &mut Context<Self>,
    ) {
        let mut cancelled = false;
        for mut session in sessions {
            if session.conflicted {
                session
                    .block
                    .update(cx, |block, _cx| block.ime_reject_until_end = true);
                self.ime_detached_targets.push(session.block);
                cancelled = true;
                continue;
            }
            let mappings = self.build_source_target_mappings(cx);
            let Some(mapping) = mappings.iter().find(|mapping| {
                mapping.full_source_range == session.target_range
                    && mapping.entity.read(cx).kind() == session.block.read(cx).kind()
            }) else {
                session
                    .block
                    .update(cx, |block, _cx| block.ime_reject_until_end = true);
                self.ime_detached_targets.push(session.block);
                cancelled = true;
                continue;
            };
            let normal = matches!(
                session.composition.owner,
                BlockImeCompositionOwner::BlockText
            );
            let from_source = |range: std::ops::Range<usize>| {
                let start = range.start.checked_sub(mapping.full_source_range.start)?;
                let end = range.end.checked_sub(mapping.full_source_range.start)?;
                let start = *mapping.source_to_content.get(start)?;
                let end = *mapping.source_to_content.get(end)?;
                Some(
                    mapping
                        .entity
                        .read(cx)
                        .markdown_range_to_current_range(start..end),
                )
            };
            if normal {
                let original = session.original_selection.range();
                let Some((range, selection)) = from_source(session.source_range)
                    .zip(from_source(original.start as usize..original.end as usize))
                else {
                    session
                        .block
                        .update(cx, |block, _cx| block.ime_reject_until_end = true);
                    self.ime_detached_targets.push(session.block);
                    cancelled = true;
                    continue;
                };
                session.composition.replacement_range = range;
                session.composition.original_selection.range = selection;
            }
            if mapping.entity != session.block {
                let (record, children, cell_mode, read_only) = {
                    let block = mapping.entity.read(cx);
                    (
                        block.record.clone(),
                        block.children.clone(),
                        block
                            .table_cell_position()
                            .zip(block.table_cell_alignment()),
                        block.is_read_only(),
                    )
                };
                session.block.update(cx, |block, _cx| {
                    block.record = record;
                    block.children = children;
                    block.set_read_only(read_only);
                    if let Some((position, alignment)) = cell_mode {
                        block.set_table_cell_mode(position, alignment);
                    }
                    block.sync_render_cache();
                    block.last_layout = None;
                });
                if !self.retain_shared_ime_input_entity(&mapping.entity, &session.block, cx) {
                    session
                        .block
                        .update(cx, |block, _cx| block.ime_reject_until_end = true);
                    self.ime_detached_targets.push(session.block);
                    cancelled = true;
                    continue;
                }
            }
            let revision = self.source_document.revision();
            session.composition.base_revision = revision;
            session.block.update(cx, |block, _cx| {
                block.set_document_revision(revision);
                block.rebase_math_edit_after_local_revision(revision);
            });
            let restored = session.block.update(cx, |block, cx| {
                block.restore_ime_composition(session.composition, cx)
            });
            cancelled |= !restored;
            if !restored {
                session
                    .block
                    .update(cx, |block, _cx| block.ime_reject_until_end = true);
                self.ime_detached_targets.push(session.block);
            }
        }
        if cancelled {
            self.ime_completion_requested = false;
            self.show_pane_notice("其他视图修改了输入区域，候选已取消，请继续输入", cx);
        }
    }

    /// 保留系统回调仍绑定的 Entity，并把对应树槽或表格运行时槽原子地改指向它。
    fn retain_shared_ime_input_entity(
        &mut self,
        replaced: &Entity<Block>,
        retained: &Entity<Block>,
        cx: &mut Context<Self>,
    ) -> bool {
        let replaced_id = replaced.entity_id();
        let retained_id = retained.entity_id();
        if self
            .document
            .retain_input_entity(replaced_id, retained.clone(), cx)
        {
            self.remap_shared_ime_entity_references(replaced_id, retained_id);
            return true;
        }

        let Some(binding) = self.table_cells.get(&replaced_id).cloned() else {
            return false;
        };
        if binding.cell.entity_id() != replaced_id {
            return false;
        }
        let Some(mut runtime) = binding.table_block.read(cx).table_runtime.clone() else {
            return false;
        };
        let slot = if binding.position.is_header() {
            runtime.header.get_mut(binding.position.column)
        } else {
            runtime
                .rows
                .get_mut(binding.position.row - 1)
                .and_then(|row| row.get_mut(binding.position.column))
        };
        let Some(slot) = slot else {
            return false;
        };
        if slot.entity_id() != replaced_id {
            return false;
        }
        *slot = retained.clone();
        binding
            .table_block
            .update(cx, move |block, _cx| block.set_table_runtime(runtime));
        self.table_cells.remove(&replaced_id);
        self.table_cells.insert(
            retained_id,
            TableCellBinding {
                table_block: binding.table_block,
                cell: retained.clone(),
                position: binding.position,
            },
        );
        self.remap_shared_ime_entity_references(replaced_id, retained_id);
        true
    }

    /// 投影重建会生成新 Entity ID；窗口焦点和编辑器选区必须继续指向被保留的输入实例。
    fn remap_shared_ime_entity_references(&mut self, replaced: EntityId, retained: EntityId) {
        if replaced == retained {
            return;
        }
        if self.pending_focus == Some(replaced) {
            self.pending_focus = Some(retained);
        }
        if self.active_entity_id == Some(replaced) {
            self.active_entity_id = Some(retained);
        }
        for selection in [
            &mut self.cross_block_selection,
            &mut self.split_preview_cross_block_selection,
        ]
        .into_iter()
        .flatten()
        {
            if selection.anchor.entity_id == replaced {
                selection.anchor.entity_id = retained;
            }
            if selection.focus.entity_id == replaced {
                selection.focus.entity_id = retained;
            }
        }
        for drag in [
            &mut self.cross_block_drag,
            &mut self.split_preview_cross_block_drag,
        ]
        .into_iter()
        .flatten()
        {
            if drag.anchor.entity_id == replaced {
                drag.anchor.entity_id = retained;
            }
        }
        if let Some((entity_id, _)) = self.table_cell_drag_anchor.as_mut()
            && *entity_id == replaced
        {
            *entity_id = retained;
        }
        if let Some((entity_id, _)) = self.split_preview_table_cell_drag_anchor.as_mut()
            && *entity_id == replaced
        {
            *entity_id = retained;
        }
    }
}

#[cfg(all(test, target_os = "windows"))]
#[path = "../../tests/unit/editor/ime_rebase.rs"]
mod tests;
