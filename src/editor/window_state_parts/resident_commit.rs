// @author kongweiguang

//! 常驻 Live 的原源码区域事务；规范投影只提供该区域的新内容，不能授权全文覆盖。

use super::*;
use std::collections::HashSet;

impl Editor {
    /// 替换事件已消费但提交失败时保留确认结果，后续输入不能落到另一块的局部回退路径。
    pub(in crate::editor) fn retain_unsubmitted_resident_text(
        &mut self,
        text: &str,
        cx: &mut Context<Self>,
    ) {
        if let Err(error) = self.document.retain_source_commit_replacement(text) {
            cx.write_to_clipboard(ClipboardItem::new_string(text.to_owned()));
            let copied = cx.read_from_clipboard().and_then(|item| item.text());
            self.retain_resident_source_commit_error(error, cx);
            self.show_pane_notice(
                if copied.as_deref() == Some(text) {
                    "本次文字已复制，请备份编辑区后恢复输入"
                } else {
                    "暂存及复制失败，请保留当前编辑区并重试"
                },
                cx,
            );
            return;
        }
        self.retain_resident_source_commit_error("选区已变化，确认文字等待恢复".to_owned(), cx);
    }

    /// 结构变更复用树的原区域计划；一次事务提交所有局部修改，不让新根等待后台解析取得身份。
    pub(super) fn commit_resident_structure_regions(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        self.document.rebuild_root_markdown_cache(cx);
        let plan = self.document.plan_resident_region_changes(cx)?;
        let snapshot = self.source_document.snapshot();
        let revision = snapshot.revision();
        if plan.revision != revision {
            return Err("当前结构编辑的源码版本已变化".to_owned());
        }
        let mut edits = Vec::new();
        let mut previous_fragments = String::new();
        let mut updated_fragments = String::new();
        for change in &plan.changes {
            let previous = snapshot
                .text_for_range(change.source.clone())
                .map_err(|error| error.to_string())?;
            if let Some(local) = Self::minimal_projection_edit(&previous, &change.markdown) {
                edits.push(gmark_document::TextEdit::new(
                    change.source.start + local.range().start
                        ..change.source.start + local.range().end,
                    local.replacement(),
                ));
            }
            previous_fragments.push_str(&previous);
            updated_fragments.push_str(&change.markdown);
        }
        let mutation = gmark_document_core::DocumentMutationMap::from_transaction(
            &gmark_document_core::Transaction::new(
                gmark_document_core::DocumentRevision(revision.get()),
                edits
                    .iter()
                    .map(|edit| {
                        gmark_document_core::SourceEdit::new(
                            edit.range().start as u64..edit.range().end as u64,
                            edit.replacement(),
                        )
                    })
                    .collect(),
            ),
        );
        let updated = if edits.is_empty() {
            snapshot
        } else {
            self.source_document
                .apply_transaction(gmark_document::Transaction::new(revision, edits))
                .map_err(|error| error.to_string())?
        };
        let rebound = self.document.rebind_source_regions_after_commit(
            plan,
            &mutation,
            updated.revision(),
            cx,
        );
        self.document.set_source_commit_error(None);
        if updated.revision() != revision {
            self.status_bar.apply_virtual_text_edit(
                revision,
                updated.revision(),
                &previous_fragments,
                &updated_fragments,
            );
            self.finish_marking_document_dirty(updated.text(), cx);
            self.pending_dirty_source = None;
        }
        if rebound {
            Ok(())
        } else {
            Err("结构编辑已保留，输入区域需要重新同步".to_owned())
        }
    }

    /// 两种区域提交共用错误状态，保留可见文字并暂停会覆盖它的后台操作。
    pub(super) fn retain_resident_source_commit_error(
        &mut self,
        detail: String,
        cx: &mut Context<Self>,
    ) {
        self.document.set_source_commit_error(Some(detail));
        self.document_dirty = true;
        self.pending_window_edited = true;
        self.pending_window_title_refresh = true;
        self.auto_save_task = None;
        self.projection_cache_task = None;
        self.show_pane_notice("输入区域已变化，请按 Ctrl+C 备份文字后继续", cx);
    }

    /// 虚拟树仅是局部投影，备份必须先保留权威全文，再附上挂载区与失去归属的新根。
    fn source_commit_backup_text(&self, cx: &App) -> String {
        let Some(surface) = self.virtual_surface.as_ref() else {
            return self.document.markdown_text(cx);
        };
        let mut text = self.source_document.text();
        text.push_str("\n\n---\n\n<!-- 未提交的编辑区域，请核对后恢复 -->\n\n");
        let mut roots = surface.flattened_roots();
        let mut seen = roots.iter().map(Entity::entity_id).collect::<HashSet<_>>();
        roots.extend(
            self.document
                .root_blocks()
                .iter()
                .filter(|root| seen.insert(root.entity_id()))
                .cloned(),
        );
        text.push_str(&DocumentTree::markdown_text_for_roots(&roots, cx));
        text
    }

    /// 完整备份并回读成功才解除门禁，虚拟视口外正文、固定区及新根都不能遗漏。
    pub(in crate::editor) fn recover_resident_source_commit(
        &mut self,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.document.source_commit_error().is_none() {
            return false;
        }
        let virtual_backup = self.virtual_surface.is_some();
        let mut text = self.source_commit_backup_text(cx);
        if let Some(pending) = self.document.source_commit_replacement()
            && !pending.is_empty()
        {
            text.push_str("\n\n");
            text.push_str(pending);
        }
        cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
        if cx
            .read_from_clipboard()
            .and_then(|item| item.text())
            .as_deref()
            != Some(text.as_str())
        {
            self.show_pane_notice("文字仍保留在编辑区，复制失败，请重试 Ctrl+C", cx);
            return true;
        }
        self.document.set_source_commit_error(None);
        self.pending_dirty_source = None;
        self.rebuild_primary_projection_from_source(cx);
        self.document_dirty = self.source_document.is_dirty();
        self.pending_window_title_refresh = true;
        self.show_pane_notice(
            if virtual_backup {
                "全文和未提交区域已复制，请检查后恢复文字"
            } else {
                "编辑区全文已复制，请检查后粘贴恢复文字"
            },
            cx,
        );
        true
    }

    /// 区域归属与 revision 必须同时匹配；连续输入同步推进树内范围，不等待后台解析。
    pub(super) fn commit_resident_block_region(
        &mut self,
        entity_id: EntityId,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let (revision, range, roots) = self
            .document
            .source_region_for_entity(entity_id)
            .ok_or_else(|| "当前输入块没有可验证的原源码区域".to_owned())?;
        let snapshot = self.source_document.snapshot();
        if revision != snapshot.revision() {
            return Err("当前输入区域的源码版本已变化".to_owned());
        }
        let previous = snapshot
            .text_for_range(range.clone())
            .map_err(|error| error.to_string())?;
        let markdown = DocumentTree::markdown_text_for_roots(&roots, cx);
        self.document
            .refresh_markdown_cache_for_entity(entity_id, cx);
        let Some(local_edit) = Self::minimal_projection_edit(&previous, &markdown) else {
            self.document.set_source_commit_error(None);
            return Ok(());
        };
        let edit = gmark_document::TextEdit::new(
            range.start + local_edit.range().start..range.start + local_edit.range().end,
            local_edit.replacement(),
        );
        let mutation = gmark_document_core::DocumentMutationMap::from_transaction(
            &gmark_document_core::Transaction::new(
                gmark_document_core::DocumentRevision(revision.get()),
                vec![gmark_document_core::SourceEdit::new(
                    edit.range().start as u64..edit.range().end as u64,
                    edit.replacement(),
                )],
            ),
        );
        let updated = self
            .source_document
            .apply_transaction(gmark_document::Transaction::new(revision, vec![edit]))
            .map_err(|error| error.to_string())?;
        let regions_advanced =
            self.document
                .advance_source_regions(&mutation, entity_id, updated.revision(), cx);
        self.document.set_source_commit_error(None);
        self.status_bar
            .apply_virtual_text_edit(revision, updated.revision(), &previous, &markdown);
        self.finish_marking_document_dirty(updated.text(), cx);
        // 保存与恢复直接读取刚提交的权威快照，不能再次提交规范投影。
        self.pending_dirty_source = None;
        if regions_advanced {
            Ok(())
        } else {
            // 即使映射失效也保留已成功提交的恢复快照；后续输入禁止再次写回规范全文。
            Err("文字已提交，但后续输入区域需要重新同步".to_owned())
        }
    }
}
