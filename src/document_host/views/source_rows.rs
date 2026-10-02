// @author kongweiguang

//! Mounted Source row Blocks and their bounded presentation cache.

use super::*;

impl DocumentHost {
    /// Rebases an active row across disjoint peer transactions and closes it on overlap or a revision gap.
    pub(super) fn rebase_active_source_edit(
        &mut self,
        view_id: DocumentViewInstanceId,
        revision: DocumentRevision,
        mutation: &DocumentMutationMap,
        cx: &mut Context<Self>,
    ) {
        let Some(document) = self.document.clone() else {
            return;
        };
        let Some(active) = self.active_edit.as_ref() else {
            return;
        };
        if revision.0 <= active.base_revision {
            return;
        }
        if view_id == document.view_id() || active.base_revision.checked_add(1) != Some(revision.0)
        {
            self.reject_stale_source_edit(cx);
            return;
        }
        let Some(range) = map_disjoint_source_range(active.range.clone(), mutation) else {
            self.reject_stale_source_edit(cx);
            return;
        };
        let Some(line) = document
            .line_for_offset(range.start.min(document.len()))
            .and_then(|line| usize::try_from(line).ok())
        else {
            self.reject_stale_source_edit(cx);
            return;
        };
        let previous_line = active.line;
        let block = active.block.clone();
        if let Some(active) = self.active_edit.as_mut() {
            active.range = range;
            active.base_revision = revision.0;
            active.line = line;
        }
        if previous_line != line {
            if self
                .source_row_blocks
                .get(&previous_line)
                .is_some_and(|candidate| *candidate == block)
            {
                self.source_row_blocks.remove(&previous_line);
            }
            self.source_row_blocks.insert(line, block);
        }
    }

    /// Removes the stale editor surface so later Block events cannot overwrite the shared revision.
    pub(super) fn reject_stale_source_edit(&mut self, cx: &mut Context<Self>) {
        let Some(active) = self.active_edit.take() else {
            return;
        };
        active.block.update(cx, |block, cx| {
            block.set_read_only(true);
            cx.notify();
        });
        self.source_row_blocks
            .retain(|_, block| *block != active.block);
        self.error = Some("源码行已在其他视图修改，正在重新同步。".into());
        cx.notify();
    }

    /// 空文件只有一个稳定的 `0..0` 行；提前发布这份快照可让首帧直接挂载
    /// 可编辑 Block，避免 uniform_list 等待后台 viewport 任务时吞掉第一次点击。
    pub(super) fn install_empty_source_row(&mut self) {
        if self
            .document
            .as_ref()
            .is_none_or(|document| !document.is_empty())
        {
            return;
        }
        let row = Arc::new(BoundedLineWindow::new(
            0..0,
            0..0,
            String::new(),
            String::new(),
            false,
            false,
        ));
        self.source_rows.insert(0, row.clone());
        self.source_row_epochs.insert(0, self.source_cache_epoch);
        let document_revision = self.document.as_ref().map_or(0, SharedDocument::revision);
        self.displayed_screen_lines = Arc::new(ScreenLines {
            document_revision,
            generation: self.coordinator.source_generation,
            cache_epoch: self.source_cache_epoch,
            column_window_start: self.source_window_start,
            visible: 0..1,
            rows: Arc::new(BTreeMap::from([(0, row)])),
        });
    }

    /// 按 Block 的真实布局逐显示行移动，并在跨 Source 行时沿用当前像素 X 坐标。
    pub(super) fn move_source_caret_by_visual_lines(
        &mut self,
        direction: i32,
        line_count: usize,
        extend: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if direction == 0 || line_count == 0 {
            return false;
        }
        let Some(document) = self.document.as_ref() else {
            return false;
        };
        let before = document.source_selection();
        let Some(mut current_line) = document
            .line_for_offset(before.head.byte_offset)
            .and_then(|line| usize::try_from(line).ok())
        else {
            return false;
        };
        let Some(mut current) = self.source_row_blocks.get(&current_line).cloned() else {
            return false;
        };
        let Some(row) = self.displayed_screen_lines.row(current_line) else {
            return false;
        };
        let mut focus = usize::try_from(
            before
                .head
                .byte_offset
                .saturating_sub(row.content_range.start),
        )
        .unwrap_or_default()
        .min(current.read(cx).display_text().len());
        current.update(cx, |block, cx| {
            block.selected_range = focus..focus;
            block.selection_reversed = false;
            cx.notify();
        });
        let mut moved = false;

        for _ in 0..line_count {
            let preferred_x = current.read(cx).preferred_visual_x();
            let local_move = current.update(cx, |block, cx| {
                block.move_cursor_by_visual_lines(direction, 1, false, cx)
            });
            if local_move {
                focus = current.read(cx).cursor_offset();
                moved = true;
                continue;
            }

            let next_line = if direction < 0 {
                current_line.checked_sub(1)
            } else {
                current_line
                    .checked_add(1)
                    .filter(|line| *line < self.line_count())
            };
            let Some(next_line) = next_line else {
                break;
            };
            let Some(next) = self.source_row_blocks.get(&next_line).cloned() else {
                break;
            };
            if self.displayed_screen_lines.row(next_line).is_none() {
                break;
            }
            focus = next
                .read(cx)
                .entry_offset_for_vertical_focus(direction < 0, Some(preferred_x));
            next.update(cx, |block, cx| {
                block.move_to_with_preferred_x(focus, Some(preferred_x), cx);
            });
            current = next;
            current_line = next_line;
            moved = true;
        }

        if !moved {
            return false;
        }
        let Some(row) = self.displayed_screen_lines.row(current_line) else {
            return false;
        };
        let head = SourceAnchor::new(
            row.content_range
                .start
                .saturating_add(focus.min(current.read(cx).display_text().len()) as u64),
            SourceAffinity::After,
        );
        self.active_edit = None;
        self.focus_handle.focus(window);
        self.set_source_selection(
            SourceSelection {
                anchor: if extend { before.anchor } else { head },
                head,
            },
            cx,
        );
        self.sync_source_selection_visuals(cx);
        cx.emit(DocumentHostEvent::StateChanged);
        cx.notify();
        true
    }

    /// 移动或扩展共享 Source 选区到文档边界，保持锚点方向而不触碰正文。
    pub(super) fn move_source_caret_to_boundary(
        &mut self,
        at_end: bool,
        extend: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(document) = self.document.as_ref() else {
            return;
        };
        let next = SourceSelection {
            anchor: if extend {
                document.source_selection().anchor
            } else {
                SourceAnchor::new(
                    if at_end { document.len() } else { 0 },
                    SourceAffinity::After,
                )
            },
            head: SourceAnchor::new(
                if at_end { document.len() } else { 0 },
                SourceAffinity::After,
            ),
        };
        self.set_source_selection(next, cx);
    }

    /// 为可见源码行创建有界 Block 输入面；Host 动作延迟到 Block 更新结束，避免回调重入读取同一实体。
    pub(super) fn ensure_source_row_block(
        &mut self,
        line: usize,
        cx: &mut Context<Self>,
    ) -> Option<Entity<Block>> {
        let layout_identity = self.source_layout_identity_for_row(line)?;
        // provisional 行只来自稳定文件句柄的可见窗口，尚无可提交 transaction 的
        // PieceTree 真值。此时保留选择与复制，但必须拒绝键盘、粘贴和 IME 写入；
        // 精确文档安装后复用同一 Block 并恢复编辑，避免用户看到最终会丢失的假修改。
        let read_only = self.document.is_none();
        let syntax_language = crate::components::code_language_for_path(&self.path);
        let syntax_context = self.source_syntax_contexts.get(&line).cloned();
        if let Some(block) = self.source_row_blocks.get(&line) {
            block.update(cx, |block, _cx| {
                block.set_source_layout_identity(layout_identity);
                block.set_read_only(read_only);
                block.set_source_syntax_context(syntax_language, syntax_context);
            });
            return Some(block.clone());
        }
        let row = self.displayed_screen_lines.row(line)?;
        let row_text = row.text.to_string();
        let host = cx.entity().downgrade();
        let block = cx.new(move |cx| {
            let mut block = Block::with_record(
                cx,
                BlockRecord::with_plain_text(BlockKind::Paragraph, row_text),
            );
            block.set_compact_source_host();
            block.set_read_only(read_only);
            block.set_source_syntax_context(syntax_language, syntax_context);
            block.set_source_layout_identity(layout_identity);
            block.set_host_action_handler(move |action, window, cx| {
                let host = host.clone();
                window.defer(cx, move |window, cx| {
                    let _ = host.update(cx, |view, cx| {
                        view.on_line_edit_host_action(action, window, cx)
                    });
                });
            });
            block
        });
        cx.subscribe(&block, Self::on_line_edit_event).detach();
        self.source_row_blocks.insert(line, block.clone());
        self.apply_source_selection_visual(line, &block, cx);
        Some(block)
    }

    /// Binds a row Block's layout caches to the exact source range and document revision it paints.
    fn source_layout_identity_for_row(&self, line: usize) -> Option<SourceLayoutIdentity> {
        let row = self.displayed_screen_lines.row(line)?;
        Some(SourceLayoutIdentity {
            document_epoch: self.document_epoch,
            document_revision: self
                .document
                .as_ref()
                .map(SharedDocument::revision)
                .unwrap_or_default(),
            source_range: row.content_range.clone(),
            column_window_start: self.displayed_screen_lines.column_window_start,
            show_line_endings: self.show_line_endings,
        })
    }
}

/// Maps a cached line only when every peer edit is outside its byte range.
fn map_disjoint_source_range(
    range: Range<u64>,
    mutation: &DocumentMutationMap,
) -> Option<Range<u64>> {
    for edit in mutation.edits() {
        let overlaps = if edit.range.is_empty() {
            edit.range.start >= range.start && edit.range.start <= range.end
        } else {
            edit.range.start < range.end && edit.range.end > range.start
        };
        if overlaps {
            return None;
        }
    }
    let start = mutation
        .map_anchor(SourceAnchor::new(range.start, SourceAffinity::After))
        .byte_offset;
    let end = mutation
        .map_anchor(SourceAnchor::new(range.end, SourceAffinity::Before))
        .byte_offset;
    (start <= end).then_some(start..end)
}

#[cfg(test)]
#[path = "../../../tests/unit/document_views/source_surface.rs"]
mod tests;
