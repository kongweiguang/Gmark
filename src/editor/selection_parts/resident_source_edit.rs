// @author kongweiguang

//! 常驻跨块编辑只由当前投影的原源码区域授权，规范文字偏移不直接取得全文写入权限。

use super::*;
use crate::editor::DocumentTree;
use gmark_document::Revision;

struct ResidentSelectionSourceEdit {
    revision: Revision,
    edit: Option<gmark_document::TextEdit>,
    inserted_start: usize,
    previous_region: String,
    replacement_region: String,
}

impl Editor {
    /// 树外表格 cell 必须先回到所属父表格；只有投影绑定的真实区域才能提供写入范围。
    fn resident_selection_source_region(
        &self,
        entity_id: EntityId,
    ) -> Option<(Revision, Range<usize>, Vec<Entity<Block>>)> {
        let owner = self
            .table_cells
            .get(&entity_id)
            .map_or(entity_id, |binding| binding.table_block.entity_id());
        self.document.source_region_for_entity(owner)
    }

    /// 以当前 revision 的区域和真实根组证明规范偏移，只在授权区域内生成原源码 edit；历史拼写映射不参与写入。
    fn prepare_resident_selection_source_edit(
        &self,
        selection: NormalizedCrossBlockSelection,
        canonical_range: Range<usize>,
        text: &str,
        cx: &App,
    ) -> Result<ResidentSelectionSourceEdit, String> {
        let snapshot = self.source_document.snapshot();
        let (start_revision, start_source, start_roots) = self
            .resident_selection_source_region(selection.start.entity_id)
            .ok_or_else(|| "选区起点没有可验证的源码区域".to_owned())?;
        let (end_revision, end_source, end_roots) = self
            .resident_selection_source_region(selection.end.entity_id)
            .ok_or_else(|| "选区终点没有可验证的源码区域".to_owned())?;
        if start_revision != snapshot.revision() || end_revision != snapshot.revision() {
            return Err("选区所在区域的版本已变化".to_owned());
        }
        let first_index = start_roots
            .first()
            .and_then(|root| self.document.root_index_for_entity(root.entity_id()))
            .ok_or_else(|| "选区起始根已移除".to_owned())?;
        let last_index = end_roots
            .last()
            .and_then(|root| self.document.root_index_for_entity(root.entity_id()))
            .ok_or_else(|| "选区末尾根已移除".to_owned())?;
        if first_index > last_index || start_source.start > end_source.end {
            return Err("选区区域的次序已变化".to_owned());
        }
        let roots = self
            .document
            .root_blocks()
            .get(first_index..=last_index)
            .ok_or_else(|| "选区根组已变化".to_owned())?;
        let source_range = start_source.start..end_source.end;
        for root in roots {
            let (revision, range, _) = self
                .resident_selection_source_region(root.entity_id())
                .ok_or_else(|| "选区中的根没有原源码归属".to_owned())?;
            if revision != snapshot.revision()
                || range.start < source_range.start
                || range.end > source_range.end
            {
                return Err("选区中的区域已变化".to_owned());
            }
        }
        let canonical_start = self
            .document
            .cached_root_source_start(first_index)
            .ok_or_else(|| "选区投影位置已变化".to_owned())?;
        let mut replacement_region = DocumentTree::markdown_text_for_roots(roots, cx);
        let canonical_end = canonical_start
            .checked_add(replacement_region.len())
            .ok_or_else(|| "选区投影范围过大".to_owned())?;
        if self
            .document
            .cached_markdown_text(cx)
            .get(canonical_start..canonical_end)
            != Some(replacement_region.as_str())
            || canonical_range.start < canonical_start
            || canonical_range.end > canonical_end
        {
            return Err("选区与所属区域的投影不一致".to_owned());
        }
        let local_range =
            canonical_range.start - canonical_start..canonical_range.end - canonical_start;
        if replacement_region.get(local_range.clone()).is_none() {
            return Err("选区不在完整文字边界上".to_owned());
        }
        let inserted_start = source_range
            .start
            .checked_add(local_range.start)
            .ok_or_else(|| "选区插入位置过大".to_owned())?;
        replacement_region.replace_range(local_range, text);
        let previous_region = snapshot
            .text_for_range(source_range.clone())
            .map_err(|error| error.to_string())?;
        let edit = if let Some(local_edit) =
            Self::minimal_projection_edit(&previous_region, &replacement_region)
        {
            let start = local_edit
                .range()
                .start
                .checked_add(source_range.start)
                .ok_or_else(|| "选区写入起点过大".to_owned())?;
            let end = local_edit
                .range()
                .end
                .checked_add(source_range.start)
                .ok_or_else(|| "选区写入终点过大".to_owned())?;
            Some(gmark_document::TextEdit::new(
                start..end,
                local_edit.replacement(),
            ))
        } else {
            None
        };
        Ok(ResidentSelectionSourceEdit {
            revision: snapshot.revision(),
            edit,
            inserted_start,
            previous_region,
            replacement_region,
        })
    }

    /// 原源码事务失败时只撤回本次捕获的历史；成功后从权威正文重建，不能再次同步规范全文。
    // 原因：沿用跨块替换的输入契约，避免新增平行参数对象；上游统一为编辑请求类型时移除。
    #[allow(clippy::too_many_arguments)]
    pub(super) fn replace_resident_cross_block_selection(
        &mut self,
        selection: NormalizedCrossBlockSelection,
        canonical_range: Range<usize>,
        text: &str,
        selected_range_relative: Option<Range<usize>>,
        mark_inserted_text: bool,
        undo_kind: UndoCaptureKind,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.view_mode != ViewMode::Rendered
            || self.virtual_surface.is_some()
            || self.document.source_commit_error().is_some()
            || !self.document_surface_is_editable()
            || self.active_selection_surface
                != crate::editor::selection_surface::SelectionSurface::Main
        {
            return false;
        }
        let prepared =
            match self.prepare_resident_selection_source_edit(selection, canonical_range, text, cx)
            {
                Ok(prepared) => prepared,
                Err(error) => {
                    eprintln!("常驻跨块选区写入拒绝: {error}");
                    self.show_pane_notice("选区已变化，请重新选择后重试", cx);
                    return false;
                }
            };
        let inserted_start = prepared.inserted_start;
        let Some(inserted_end) = inserted_start.checked_add(text.len()) else {
            self.show_pane_notice("插入文字过长，请分段重试", cx);
            return false;
        };
        if let Some(edit) = prepared.edit {
            let owns_history_capture = self.pending_undo_capture.is_none();
            self.prepare_undo_capture(undo_kind, cx);
            let updated =
                match self
                    .source_document
                    .apply_transaction(gmark_document::Transaction::new(
                        prepared.revision,
                        vec![edit],
                    )) {
                    Ok(updated) => updated,
                    Err(error) => {
                        if owns_history_capture {
                            self.pending_undo_capture = None;
                        }
                        eprintln!("常驻跨块源码事务失败: {error}");
                        self.show_pane_notice("正文已变化，请重新选择后重试", cx);
                        return false;
                    }
                };
            self.rebuild_primary_projection_from_source(cx);
            self.status_bar.apply_virtual_text_edit(
                prepared.revision,
                updated.revision(),
                &prepared.previous_region,
                &prepared.replacement_region,
            );
            self.mark_restored_document_dirty(cx);
        } else {
            // 无正文变化时只恢复选择；历史捕获必须继续看到原源码，而不是规范投影。
            self.pending_dirty_source = Some(self.source_document.text());
        }
        self.cross_block_selection = None;
        self.cross_block_drag = None;
        let selected_source_range = selected_range_relative
            .map(|relative| {
                inserted_start + relative.start.min(text.len())
                    ..inserted_start + relative.end.min(text.len())
            })
            .unwrap_or(inserted_end..inserted_end);
        self.apply_selection_snapshot_in_current_mode(
            &UndoSelectionSnapshot::from_range(selected_source_range, false),
            cx,
        );
        if mark_inserted_text && !text.is_empty() {
            let marked = self.map_selection_source_spelling(
                UndoSelectionSnapshot::from_range(inserted_start..inserted_end, false),
                false,
                cx,
            );
            self.apply_marked_source_range(marked.range(), cx);
        }
        self.finalize_pending_undo_capture(cx);
        self.sync_table_axis_visuals(cx);
        self.dismiss_contextual_overlays(cx);
        self.sync_cross_block_selection_visuals(cx);
        self.request_active_block_scroll_into_view(cx);
        cx.notify();
        true
    }

    /// 非区域路由仍共用既有重建边界；Live 的区域事务入口不会调用此全文同步方法。
    pub(super) fn rebuild_after_cross_block_source_edit(
        &mut self,
        source: String,
        cx: &mut Context<Self>,
    ) {
        self.sync_source_document_from_projection(&source);
        match self.view_mode {
            ViewMode::Rendered => self.rebuild_primary_projection_from_source(cx),
            ViewMode::Source | ViewMode::Split => {
                let block = Self::new_block(cx, crate::components::BlockRecord::paragraph(source));
                block.update(cx, |block, _cx| block.set_source_document_mode());
                self.document.replace_roots(vec![block], cx);
                self.table_cells.clear();
            }
            ViewMode::Preview => {}
        }
    }
}
