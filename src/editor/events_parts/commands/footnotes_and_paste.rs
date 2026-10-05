// @author kongweiguang

use super::*;
use crate::editor::PendingResourceInsertion;

impl Editor {
    pub(in crate::editor) fn next_footnote_id(markdown: &str) -> String {
        (1usize..)
            .map(|index| format!("note-{index}"))
            .find(|id| !markdown.contains(&format!("[^{id}]")))
            .unwrap_or_else(|| "note-1".to_owned())
    }

    pub(in crate::editor) fn new_footnote_definition_block(
        cx: &mut Context<Self>,
        id: String,
    ) -> Entity<super::Block> {
        let definition = Self::new_block(
            cx,
            BlockRecord::with_plain_text(BlockKind::FootnoteDefinition, id),
        );
        let body = Self::new_block(cx, BlockRecord::paragraph(String::new()));
        definition.update(cx, |definition, _cx| definition.children = vec![body]);
        definition
    }

    pub(in crate::editor) fn handle_paste_image_request(
        &mut self,
        block: Entity<super::Block>,
        leading: &InlineTextTree,
        source: &PastedImageSource,
        trailing: &InlineTextTree,
        cx: &mut Context<Self>,
    ) {
        if let PastedImageSource::LocalResource(source) = source {
            let behavior = crate::preferences::read_app_preferences()
                .map(|preferences| preferences.resource_insert_behavior())
                .unwrap_or(crate::preferences::ImagePasteBehavior::None);
            if self.file_path.is_none() && behavior != crate::preferences::ImagePasteBehavior::None
            {
                // Save As owns the boundary before any copy side effect. A
                // cancelled or failed save simply drops this pending intent.
                self.pending_resource_insertion = Some(PendingResourceInsertion::Pasted {
                    block,
                    leading: leading.clone(),
                    trailing: trailing.clone(),
                    source: source.clone(),
                });
                self.request_save_document_as(cx);
                return;
            }
        }

        self.complete_paste_image_request(block, leading, source, trailing, cx);
    }

    pub(super) fn complete_paste_image_request(
        &mut self,
        block: Entity<super::Block>,
        leading: &InlineTextTree,
        source: &PastedImageSource,
        trailing: &InlineTextTree,
        cx: &mut Context<Self>,
    ) {
        if let PastedImageSource::LocalResource(path) | PastedImageSource::LocalPath(path) = source
        {
            let label = match source {
                PastedImageSource::LocalPath(path) => path
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .filter(|stem| !stem.is_empty())
                    .unwrap_or("image")
                    .to_owned(),
                PastedImageSource::LocalResource(_) | PastedImageSource::ClipboardImage(_) => {
                    String::new()
                }
            };
            self.schedule_pasted_resource_insert(
                block,
                leading.clone(),
                path.clone(),
                label,
                trailing.clone(),
                cx,
            );
            return;
        }

        let markdown = match self.pasted_image_markdown(source) {
            Ok(markdown) => markdown,
            Err(err) => {
                self.show_image_paste_error(err, cx);
                return;
            }
        };
        self.commit_paste_image_markdown(block, leading, markdown, None, trailing, cx);
    }

    /// 把路径粘贴的文件系统读取、复制和 Markdown 生成放到后台，避免 UNC 或大文件占用 UI。
    fn schedule_pasted_resource_insert(
        &mut self,
        block: Entity<super::Block>,
        leading: InlineTextTree,
        source: PathBuf,
        label: String,
        trailing: InlineTextTree,
        cx: &mut Context<Self>,
    ) {
        let Some(fingerprint) = self.current_dropped_resource_target(&block, cx) else {
            return;
        };
        let document_path = self.file_path.clone();
        let behavior = crate::preferences::read_app_preferences()
            .map(|preferences| preferences.resource_insert_behavior())
            .unwrap_or(crate::preferences::ImagePasteBehavior::None);
        let weak_editor = cx.entity().downgrade();
        let error_block = block.clone();
        let error_fingerprint = fingerprint.clone();
        let fallback_text = source.to_string_lossy().into_owned();
        cx.spawn(async move |_, cx| {
            let result = cx
                .background_spawn(async move {
                    Self::materialize_resource_with_limits(
                        &label,
                        &source,
                        document_path.as_deref(),
                        behavior,
                        None,
                    )
                })
                .await;
            match result {
                Err(error) => {
                    let missing_source =
                        crate::editor::file_drop::resource_materialization_is_missing(&error);
                    let _ = weak_editor.update(cx, |editor, cx| {
                        let Some(current) =
                            editor.current_dropped_resource_target(&error_block, cx)
                        else {
                            return;
                        };
                        if !crate::editor::file_drop::resource_drop_target_is_current(
                            &error_fingerprint,
                            &current,
                        ) {
                            return;
                        }
                        if missing_source {
                            editor.commit_paste_plain_text(
                                error_block,
                                &leading,
                                fallback_text,
                                &trailing,
                                cx,
                            );
                        } else {
                            editor.show_image_paste_error(error, cx);
                        }
                    });
                }
                Ok((markdown, materialized)) => {
                    // 在 update 前持有 guard，实体消失或任务取消时仍会删除本次创建的副本。
                    let mut cleanup =
                        crate::editor::file_drop::ResourceCleanupGuard::new(materialized);
                    let _ = weak_editor.update(cx, move |editor, cx| {
                        let Some(current) = editor.current_dropped_resource_target(&block, cx)
                        else {
                            return;
                        };
                        if !crate::editor::file_drop::resource_drop_target_is_current(
                            &fingerprint,
                            &current,
                        ) {
                            return;
                        }
                        editor.commit_paste_image_markdown(
                            block,
                            &leading,
                            markdown,
                            Some(&mut cleanup),
                            &trailing,
                            cx,
                        );
                    });
                }
            }
        })
        .detach();
    }

    /// 跨块拒绝不得退成局部粘贴，成功才接管图片；失败仍由原 guard 回收副本。
    fn commit_paste_image_markdown(
        &mut self,
        block: Entity<super::Block>,
        leading: &InlineTextTree,
        markdown: String,
        mut cleanup: Option<&mut crate::editor::file_drop::ResourceCleanupGuard>,
        trailing: &InlineTextTree,
        cx: &mut Context<Self>,
    ) {
        let had_cross_selection = self.cross_block_selection.is_some();
        if self.replace_cross_block_selection_with_text(
            &markdown,
            None,
            false,
            crate::components::UndoCaptureKind::NonCoalescible,
            cx,
        ) {
            if let Some(cleanup) = cleanup.as_mut() {
                cleanup.disarm();
            }
            return;
        }
        if had_cross_selection || self.document.source_commit_error().is_some() {
            return;
        }

        self.prepare_undo_capture(crate::components::UndoCaptureKind::NonCoalescible, cx);
        let can_insert_image_block = self.view_mode == super::ViewMode::Rendered
            && block.read(cx).kind() == BlockKind::Paragraph
            && self.table_cell_binding(block.entity_id()).is_none()
            && !block.read(cx).uses_raw_text_editing();

        if can_insert_image_block {
            if !self.insert_image_block_after_paragraph(&block, leading, &markdown, trailing, cx) {
                self.finalize_pending_undo_capture(cx);
                return;
            }
        } else {
            self.replace_current_block_selection_with_image_text(
                &block, leading, &markdown, trailing, cx,
            );
        }

        if let Some(cleanup) = cleanup.as_mut() {
            cleanup.disarm();
        }
        self.mark_dirty(cx);
        self.finalize_pending_undo_capture(cx);
        cx.notify();
    }

    /// 缺失资源的文本仍绑定原选区；跨块失败保留文字并等待恢复，不能写入活动块的局部范围。
    fn commit_paste_plain_text(
        &mut self,
        block: Entity<super::Block>,
        leading: &InlineTextTree,
        text: String,
        trailing: &InlineTextTree,
        cx: &mut Context<Self>,
    ) {
        let had_cross_selection = self.cross_block_selection.is_some();
        if self.replace_cross_block_selection_with_text(
            &text,
            None,
            false,
            crate::components::UndoCaptureKind::NonCoalescible,
            cx,
        ) {
            return;
        }
        if had_cross_selection || self.document.source_commit_error().is_some() {
            self.retain_unsubmitted_resident_text(&text, cx);
            return;
        }

        self.prepare_undo_capture(crate::components::UndoCaptureKind::NonCoalescible, cx);
        let (kind, title, cursor) = block.read_with(cx, |block, _cx| {
            let mut title = leading.clone();
            title.append_tree(InlineTextTree::plain(text.clone()));
            let cursor = title.visible_len();
            title.append_tree(trailing.clone());
            (block.kind(), title, cursor)
        });
        Self::set_block_title_and_kind(&block, kind, title, cursor, cx);
        if let Some(binding) = self.table_cell_binding(block.entity_id()) {
            self.sync_table_record_from_runtime(&binding.table_block, cx);
        }
        self.focus_block(block.entity_id());
        self.mark_dirty(cx);
        self.finalize_pending_undo_capture(cx);
        cx.notify();
    }

    pub(in crate::editor) fn jump_to_footnote_definition(
        &mut self,
        id: &str,
        cx: &mut Context<Self>,
    ) -> bool {
        if let Some(block) = self
            .footnote_registry
            .binding(id)
            .and_then(|binding| self.focusable_entity_by_id(binding.definition_entity_id))
        {
            self.focus_block_range(&block, 0..0, cx);
            return true;
        }
        let Some(y) = self
            .virtual_surface
            .as_ref()
            .and_then(|surface| surface.footnote_definition_y(id))
        else {
            return false;
        };
        self.pending_virtual_footnote_focus = Some(id.to_string());
        self.scroll_handle
            .set_offset(point(px(0.0), px(-y.max(0.0))));
        cx.notify();
        true
    }

    pub(in crate::editor) fn jump_to_footnote_backref(
        &mut self,
        id: &str,
        cx: &mut Context<Self>,
    ) -> bool {
        if let Some((block, range)) = self.footnote_registry.binding(id).and_then(|binding| {
            let first_reference = binding.first_reference.as_ref()?;
            let block = self.focusable_entity_by_id(first_reference.entity_id)?;
            let range = block
                .read(cx)
                .current_range_for_footnote_occurrence(first_reference.occurrence_index)
                .unwrap_or(0..0);
            Some((block, range))
        }) {
            self.focus_block_range(&block, range, cx);
            return true;
        }
        let Some(y) = self
            .virtual_surface
            .as_ref()
            .and_then(|surface| surface.footnote_first_reference_y(id))
        else {
            return false;
        };
        self.pending_virtual_footnote_backref_focus = Some(id.to_string());
        self.scroll_handle
            .set_offset(point(px(0.0), px(-y.max(0.0))));
        cx.notify();
        true
    }
}
