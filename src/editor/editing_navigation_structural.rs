// @author kongweiguang

//! Source-mapped structural commands for selected Live paragraphs and list items.

use gpui::{App, Context, Entity, EntityId, Window};

use super::super::{CrossBlockSelectionEndpoint, DocumentTree, Editor, UndoSelectionSnapshot};
use crate::components::{
    Block, BlockKind, BlockRecord, EditingCommandHistory, EditingCommandId, LineOperation,
    UndoCaptureKind,
};
use crate::editor::selection_surface::SelectionSurface;
use crate::editor::selection_virtualization::NormalizedCrossBlockSelection;

impl Editor {
    /// Expands a cross-block text selection to complete sibling blocks through their source spans.
    fn selected_live_sibling_blocks(
        &self,
        selection: NormalizedCrossBlockSelection,
        source_range: std::ops::Range<usize>,
        cx: &App,
    ) -> Option<(Option<Entity<Block>>, usize, Vec<Entity<Block>>)> {
        let visible = self.selection_surface_entities(SelectionSurface::Main);
        let (Some(start_index), Some(end_index)) = (selection.start_index, selection.end_index)
        else {
            return None;
        };
        let start = visible.get(start_index)?.clone();
        let end = visible.get(end_index)?.clone();
        let start_path = self.block_ancestor_path(&start)?;
        let end_path = self.block_ancestor_path(&end)?;
        let common_parent = start_path
            .iter()
            .skip(1)
            .find(|candidate| {
                end_path
                    .iter()
                    .skip(1)
                    .any(|other| other.entity_id() == candidate.entity_id())
            })
            .cloned();
        let parent_id = common_parent.as_ref().map(Entity::entity_id);
        let start_branch = self.branch_under_parent(&start, parent_id)?;
        let end_branch = self.branch_under_parent(&end, parent_id)?;
        let start_location = self
            .document
            .find_block_location(start_branch.entity_id())?;
        let end_location = self.document.find_block_location(end_branch.entity_id())?;
        let same_parent = match (&start_location.parent, &end_location.parent) {
            (Some(start), Some(end)) => start.entity_id() == end.entity_id(),
            (None, None) => true,
            _ => false,
        };
        if !same_parent || start_location.index > end_location.index {
            return None;
        }

        let siblings = common_parent
            .as_ref()
            .map(|parent| parent.read(cx).children.clone())
            .unwrap_or_else(|| self.document.root_blocks().to_vec());
        let selected = siblings
            .get(start_location.index..=end_location.index)?
            .to_vec();
        let (_, block_ranges) = self.build_source_target_mappings_with_block_ranges(cx);
        for block in &selected {
            let is_endpoint = block.entity_id() == start_branch.entity_id()
                || block.entity_id() == end_branch.entity_id();
            let fully_covered = block_ranges.get(&block.entity_id()).is_some_and(|range| {
                source_range.start <= range.start && range.end <= source_range.end
            });
            let supported_kind = {
                let block = block.read(cx);
                matches!(block.kind(), BlockKind::Paragraph) || block.kind().is_list_item()
            };
            if (!is_endpoint && !fully_covered) || !supported_kind {
                return None;
            }
        }

        Some((start_location.parent, start_location.index, selected))
    }

    /// 列表嵌套由现有树规则决定；虚拟表面只写回变更列表的源码，随后恢复新 revision 的选区。
    pub(in crate::editor) fn apply_live_list_group_indentation(
        &mut self,
        operation: LineOperation,
        selection: NormalizedCrossBlockSelection,
        source_range: std::ops::Range<usize>,
        cx: &mut Context<Self>,
    ) {
        let Some((source_parent, start_index, blocks)) =
            self.selected_live_sibling_blocks(selection, source_range, cx)
        else {
            return;
        };
        if blocks.len() < 2
            || blocks
                .iter()
                .any(|block| !block.read(cx).kind().is_list_item())
        {
            return;
        }

        let sibling_count = source_parent
            .as_ref()
            .map(|parent| parent.read(cx).children.len())
            .unwrap_or_else(|| self.document.root_count());
        let Some(end_index) = start_index.checked_add(blocks.len() - 1) else {
            return;
        };
        if end_index >= sibling_count {
            return;
        }

        let (target_parent, target_index, convert_to_paragraph) = match operation {
            LineOperation::Indent => {
                let Some(previous_index) = start_index.checked_sub(1) else {
                    return;
                };
                let siblings = source_parent
                    .as_ref()
                    .map(|parent| parent.read(cx).children.clone())
                    .unwrap_or_else(|| self.document.root_blocks().to_vec());
                let Some(target) = siblings.get(previous_index).cloned() else {
                    return;
                };
                let target_kind = target.read(cx).kind();
                if blocks
                    .iter()
                    .any(|block| !block.read(cx).kind().can_nest_under(&target_kind))
                {
                    return;
                }
                let child_index = target.read(cx).children.len();
                (Some(target), child_index, false)
            }
            LineOperation::Outdent => match source_parent.as_ref() {
                None => (None, 0, true),
                Some(parent) => {
                    if !parent.read(cx).kind().is_list_item() {
                        return;
                    }
                    let Some(parent_location) =
                        self.document.find_block_location(parent.entity_id())
                    else {
                        return;
                    };
                    let Some(target_index) = parent_location.index.checked_add(1) else {
                        return;
                    };
                    (parent_location.parent, target_index, false)
                }
            },
            _ => return,
        };

        let virtual_scope = self.virtual_surface.as_ref().and_then(|surface| {
            Some((
                surface.region_for_entity(selection.start.entity_id)?,
                surface.source_range_for_entity(selection.start.entity_id)?,
            ))
        });
        self.prepare_undo_capture(UndoCaptureKind::NonCoalescible, cx);
        if convert_to_paragraph {
            self.document.with_structure_mutation(cx, |_document, cx| {
                for block in &blocks {
                    block.update(cx, |block, cx| block.convert_to_paragraph(cx));
                }
            });
        } else {
            let ids = blocks.iter().map(Entity::entity_id).collect::<Vec<_>>();
            self.document.with_structure_mutation(cx, |document, cx| {
                let mut moved = Vec::with_capacity(ids.len());
                for entity_id in ids.iter().rev().copied() {
                    let Some((block, _)) = document.remove_block_by_id_raw(entity_id, cx) else {
                        return;
                    };
                    moved.push(block);
                }
                moved.reverse();
                document.insert_blocks_at_raw(target_parent, target_index, moved, cx);
            });
        }

        let focused_endpoint = if selection.reversed {
            selection.start
        } else {
            selection.end
        };
        self.focus_block(focused_endpoint.entity_id);
        self.active_selection_surface = SelectionSurface::Main;
        let (roots, absolute_start) = if let Some((region, range)) = &virtual_scope {
            let roots = self
                .document
                .root_blocks()
                .iter()
                .filter(|root| {
                    self.virtual_surface.as_ref().is_some_and(|surface| {
                        surface.region_for_entity(root.entity_id()) == Some(*region)
                    })
                })
                .cloned()
                .collect::<Vec<_>>();
            (roots, range.start)
        } else {
            (self.document.root_blocks().to_vec(), 0)
        };
        let mappings = self.build_source_target_mappings_for_roots(&roots, absolute_start, cx);
        let offset_for = |endpoint: CrossBlockSelectionEndpoint| {
            mappings
                .iter()
                .find(|mapping| mapping.entity.entity_id() == endpoint.entity_id)
                .and_then(|mapping| self.endpoint_source_offset_from_mapping(endpoint, mapping, cx))
        };
        let updated_range = offset_for(selection.start).zip(offset_for(selection.end));
        self.cross_block_selection = None;
        self.cross_block_drag = None;
        if let Some((_, range)) = virtual_scope {
            // mounted cache 仍含原根数组；结构变更后只能序列化当前树里的该区域根。
            let markdown = DocumentTree::markdown_text_for_roots(&roots, cx);
            if !self.apply_virtual_cross_block_source_edit(range, &markdown, cx) {
                self.pending_virtual_undo_selection = None;
                self.rebuild_primary_projection_from_source(cx);
                return;
            }
        } else {
            self.rebuild_image_runtimes(cx);
            self.mark_dirty(cx);
        }
        if let Some((start, end)) = updated_range {
            self.apply_selection_snapshot_in_current_mode(
                &UndoSelectionSnapshot::from_range(start..end, selection.reversed),
                cx,
            );
        }
        self.finalize_pending_undo_capture(cx);
        self.request_active_block_scroll_into_view(cx);
        cx.notify();
    }

    /// Returns an entity's ancestors from itself through its root-level branch.
    fn block_ancestor_path(&self, block: &Entity<Block>) -> Option<Vec<Entity<Block>>> {
        let mut path = vec![block.clone()];
        let mut current = block.clone();
        while let Some(parent) = self
            .document
            .find_block_location(current.entity_id())?
            .parent
        {
            current = parent;
            path.push(current.clone());
        }
        Some(path)
    }

    /// Finds the selected entity's direct branch below a common tree parent.
    fn branch_under_parent(
        &self,
        block: &Entity<Block>,
        parent_id: Option<EntityId>,
    ) -> Option<Entity<Block>> {
        let mut current = block.clone();
        loop {
            let location = self.document.find_block_location(current.entity_id())?;
            match (parent_id, location.parent.as_ref()) {
                (Some(parent_id), Some(parent)) if parent.entity_id() == parent_id => {
                    return Some(current);
                }
                (None, None) => return Some(current),
                _ => current = location.parent?,
            }
        }
    }

    /// Commits a multi-block Live row command as one structural and undo transaction.
    pub(super) fn apply_live_cross_block_command(
        &mut self,
        command: EditingCommandId,
        selection: NormalizedCrossBlockSelection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(source_range) = self.cross_block_source_range_for_normalized(selection, cx) else {
            return;
        };
        let Some((parent, start_index, blocks)) =
            self.selected_live_sibling_blocks(selection, source_range, cx)
        else {
            return;
        };
        if blocks.len() < 2 {
            return;
        }

        let sibling_count = parent
            .as_ref()
            .map(|parent| parent.read(cx).children.len())
            .unwrap_or_else(|| self.document.root_count());
        let end_index = start_index + blocks.len() - 1;
        let direction = match command {
            EditingCommandId::DuplicateBlock | EditingCommandId::DeleteBlock => 0,
            EditingCommandId::MoveBlockUp => -1,
            EditingCommandId::MoveBlockDown => 1,
            _ => return,
        };
        if direction < 0 && start_index == 0 || direction > 0 && end_index + 1 >= sibling_count {
            return;
        }

        let first_visible_index = self
            .document
            .visible_index_for_entity_id(blocks[0].entity_id())
            .unwrap_or(0);
        self.prepare_undo_capture(UndoCaptureKind::NonCoalescible, cx);

        match command {
            EditingCommandId::DuplicateBlock => {
                let duplicates = blocks
                    .iter()
                    .map(|block| Self::clone_block_subtree(block, cx))
                    .collect::<Vec<_>>();
                self.document.with_structure_mutation(cx, |document, cx| {
                    document.insert_blocks_at_raw(parent, end_index + 1, duplicates.clone(), cx);
                });
                self.select_live_block_group(&duplicates, window, cx);
            }
            EditingCommandId::DeleteBlock => {
                let ids = blocks
                    .iter()
                    .rev()
                    .map(Entity::entity_id)
                    .collect::<Vec<_>>();
                self.clear_cross_block_selection_for_surface(SelectionSurface::Main, cx);
                self.document.with_structure_mutation(cx, |document, cx| {
                    for entity_id in ids {
                        let _ = document.remove_block_by_id_raw(entity_id, cx);
                    }
                    if document.root_count() == 0 {
                        let paragraph = Self::new_block(cx, BlockRecord::paragraph(String::new()));
                        document.insert_blocks_at_raw(None, 0, vec![paragraph], cx);
                    }
                });
                let visible = self.document.visible_blocks();
                let focus = visible
                    .get(first_visible_index)
                    .or_else(|| {
                        first_visible_index
                            .checked_sub(1)
                            .and_then(|index| visible.get(index))
                    })
                    .map(|visible| visible.entity.clone());
                if let Some(focus) = focus {
                    Self::set_local_cursor(&focus, 0, cx);
                    self.focus_block(focus.entity_id());
                }
            }
            EditingCommandId::MoveBlockUp | EditingCommandId::MoveBlockDown => {
                let ids = blocks.iter().map(Entity::entity_id).collect::<Vec<_>>();
                self.document.with_structure_mutation(cx, |document, cx| {
                    let mut moved = Vec::with_capacity(ids.len());
                    for entity_id in ids.into_iter().rev() {
                        let Some((block, _)) = document.remove_block_by_id_raw(entity_id, cx)
                        else {
                            return;
                        };
                        moved.push(block);
                    }
                    moved.reverse();
                    let target_index = if direction < 0 {
                        start_index - 1
                    } else {
                        start_index + 1
                    };
                    document.insert_blocks_at_raw(parent, target_index, moved, cx);
                });
                self.sync_cross_block_selection_visuals_for_surface(SelectionSurface::Main, cx);
                let focused_endpoint = if selection.reversed {
                    selection.start
                } else {
                    selection.end
                };
                self.focus_block(focused_endpoint.entity_id);
            }
            _ => return,
        }

        self.active_selection_surface = SelectionSurface::Main;
        self.rebuild_image_runtimes(cx);
        EditingCommandHistory::record(command, cx);
        self.mark_dirty(cx);
        self.finalize_pending_undo_capture(cx);
        self.request_active_block_scroll_into_view(cx);
        cx.notify();
    }

    /// Moves the selection to the duplicated blocks so the next edit targets the copy.
    fn select_live_block_group(
        &mut self,
        blocks: &[Entity<Block>],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(first) = blocks.first() else {
            return;
        };
        let Some(last) = blocks.last() else {
            return;
        };
        let start = CrossBlockSelectionEndpoint {
            entity_id: first.entity_id(),
            offset: 0,
        };
        let end = CrossBlockSelectionEndpoint {
            entity_id: last.entity_id(),
            offset: last.read(cx).visible_len(),
        };
        if blocks.len() == 1 {
            self.set_cross_block_selection_for_surface(SelectionSurface::Main, None);
            Self::set_local_selection(first, start.offset, end.offset, cx);
        } else {
            Self::set_local_cursor(last, end.offset, cx);
            let selection = self.cross_block_selection_from_endpoints(
                SelectionSurface::Main,
                start,
                end,
                None,
                cx,
            );
            self.set_cross_block_selection_for_surface(SelectionSurface::Main, Some(selection));
            self.sync_cross_block_selection_visuals_for_surface(SelectionSurface::Main, cx);
        }
        self.active_selection_surface = SelectionSurface::Main;
        self.focus_navigation_target(SelectionSurface::Main, last, window, cx);
    }
}
