// @author kongweiguang

//! Mounted Source row Blocks and their bounded presentation cache.

use super::*;

impl DocumentHost {
    /// Returns a row window only when its pinned Block identity is current for the shared document.
    pub(super) fn current_pinned_source_row(
        &self,
        line: usize,
        cx: &App,
    ) -> Option<&BoundedLineWindow> {
        let document = self.document.as_ref()?;
        let entity = self.source_row_blocks.get(&line)?;
        let block = entity.read(cx);
        let identity = block.source_layout_identity.as_ref()?;
        let row = self.source_rows.get(&line)?;
        let active_edit_matches = self.active_edit.as_ref().is_some_and(|active| {
            active.line == line
                && active.block == *entity
                && active.base_revision == document.revision()
        });
        ((block.source_host_input_focus_handle.is_some() || active_edit_matches)
            && !block.is_read_only()
            && self.source_row_epochs.get(&line) == Some(&self.source_cache_epoch)
            && identity.document_epoch == self.document_epoch
            && identity.document_revision == document.revision()
            && identity.source_range == row.content_range)
            .then_some(row.as_ref())
    }

    /// 命中必须同时属于当前正文和水平窗口；同 revision 的旧帧不能恢复重锚前的输入范围。
    pub(super) fn current_source_pointer_row(
        &self,
        line: usize,
        cx: &App,
    ) -> Option<&BoundedLineWindow> {
        if let Some(row) = self.current_pinned_source_row(line, cx) {
            return Some(row);
        }
        let document = self.document.as_ref()?;
        (self.displayed_screen_lines.document_revision == document.revision()
            && self.displayed_screen_lines.cache_epoch == self.source_cache_epoch
            && self.displayed_screen_lines.column_window_start == self.source_window_start)
            .then(|| self.displayed_screen_lines.row(line))
            .flatten()
    }

    /// 重锚输入行；仅跨到新行时跟随光标，迟到的正文事件不能覆盖用户已移动的视口。
    pub(super) fn pin_source_input_row_after_edit(
        &mut self,
        block: &Entity<Block>,
        selection: SourceSelection,
        cx: &mut Context<Self>,
    ) -> Result<bool, PagedDocumentError> {
        let Some(document) = self.document.clone() else {
            return Ok(false);
        };
        let caret_offset = selection.head.byte_offset;
        let Some(line) = document
            .line_for_offset(caret_offset.min(document.len()))
            .and_then(|line| usize::try_from(line).ok())
        else {
            return Ok(false);
        };
        let Some(line_range) = document.line_range(line as u64) else {
            return Ok(false);
        };
        let requested = caret_offset
            .saturating_sub(line_range.start)
            .saturating_sub(MAX_RENDERED_LINE_BYTES / 2);
        let Some(windowed) = read_bounded_line_window(&document, line as u64, requested)? else {
            return Ok(false);
        };
        let selected = selection.range();
        if selected.start < windowed.content_range.start
            || selected.end > windowed.content_range.end
        {
            return Ok(false);
        }
        let selected_start =
            usize::try_from(selected.start.saturating_sub(windowed.content_range.start))
                .ok()
                .map(|offset| offset.min(windowed.text.len()))
                .unwrap_or_default();
        let selected_end =
            usize::try_from(selected.end.saturating_sub(windowed.content_range.start))
                .ok()
                .map(|offset| offset.min(windowed.text.len()))
                .unwrap_or_default();
        let line_text = windowed.text.to_string();
        let host_input_focus_handle = block.read(cx).source_host_input_focus_handle.clone();
        let window_start = windowed
            .content_range
            .start
            .saturating_sub(line_range.start);
        self.source_row_blocks
            .retain(|_, candidate| candidate != block);
        self.source_row_blocks.insert(line, block.clone());
        self.source_rows.insert(line, Arc::new(windowed.clone()));
        self.source_row_epochs.insert(line, self.source_cache_epoch);
        self.source_window_start = window_start;
        self.selection_anchor = Some(line);
        self.selected_lines = Some(line..line.saturating_add(1));
        self.suppressed_line_edit_text = Some(line_text.clone());
        let reveal_new_line = self.active_edit.as_ref().is_none_or(|active| {
            active.line != line
                && self
                    .source_last_visible
                    .as_ref()
                    .is_some_and(|visible| visible.contains(&active.line))
        });
        self.active_edit = Some(SourceLineEdit {
            line,
            range: windowed.replace_range.clone(),
            base_revision: document.revision(),
            ending: windowed.ending.clone(),
            leading_truncated: windowed.leading_truncated,
            trailing_truncated: windowed.trailing_truncated,
            block: block.clone(),
        });
        let identity = SourceLayoutIdentity {
            document_epoch: self.document_epoch,
            document_revision: document.revision(),
            source_range: windowed.content_range,
            column_window_start: window_start,
            show_line_endings: self.show_line_endings,
        };
        let syntax_language = crate::components::code_language_for_path(&self.path);
        block.update(cx, |block, cx| {
            let old_len = block.display_text().len();
            block.set_read_only(false);
            block.set_source_layout_identity(identity);
            block.set_source_syntax_context(syntax_language, None);
            block.set_source_host_input_focus_handle(host_input_focus_handle, cx);
            block.replace_text_in_visible_range(
                0..old_len,
                &line_text,
                Some(selected_start..selected_end),
                false,
                cx,
            );
            block.selection_reversed = selection.reversed();
            block.selected_range = selected_start..selected_end;
        });
        if reveal_new_line
            && !self
                .source_last_visible
                .as_ref()
                .is_some_and(|visible| visible.contains(&line))
        {
            self.scroll_source_line(line, ScrollStrategy::Center);
        }
        self.sync_source_selection_visuals(cx);
        Ok(true)
    }

    /// 正文提交后无法重锚时关闭旧输入面；普通输入与 IME 共用失败边界，防止旧范围继续写入。
    pub(super) fn retain_source_input_row_after_edit(
        &mut self,
        block: &Entity<Block>,
        selection: SourceSelection,
        cx: &mut Context<Self>,
    ) -> bool {
        match self.pin_source_input_row_after_edit(block, selection, cx) {
            Ok(true) => return true,
            Ok(false) => {}
            Err(error) => self.error = Some(localized_document_error(&error, cx)),
        }
        self.break_source_typing_group();
        self.active_edit = None;
        block.update(cx, |block, cx| {
            block.set_read_only(true);
            cx.notify();
        });
        false
    }

    /// 键盘导航结束残留手势；仅复用容纳新选区的当前行窗口，否则重锚，避免旧输入面拒绝紧随文字。
    pub(super) fn restore_source_navigation_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.end_source_pointer_selection();
        for block in self.source_row_blocks.values() {
            block.update(cx, |block, cx| {
                if block.is_selecting || block.pointer_selection.is_some() {
                    block.is_selecting = false;
                    block.pointer_selection = None;
                    cx.notify();
                }
            });
        }
        let Some(document) = self.document.clone() else {
            return;
        };
        let selection = document.source_selection();
        let Some(line) = document
            .line_for_offset(selection.head.byte_offset.min(document.len()))
            .and_then(|line| usize::try_from(line).ok())
        else {
            return;
        };

        if self.selection_spans_multiple_lines(selection) {
            self.active_edit = None;
            self.focus_handle.focus(window);
            self.sync_source_selection_visuals(cx);
            return;
        }

        let range = selection.range();
        let row_is_current = self.source_row_blocks.contains_key(&line)
            && self
                .current_source_pointer_row(line, cx)
                .is_some_and(|row| {
                    range.start >= row.content_range.start && range.end <= row.content_range.end
                });
        if row_is_current {
            self.focus_source_pointer_line(line, window, cx);
            self.sync_source_selection_visuals(cx);
            return;
        }
        if self.displayed_screen_lines.document_revision == document.revision()
            && self.ensure_source_row_block(line, cx).is_some()
            && self
                .current_source_pointer_row(line, cx)
                .is_some_and(|row| {
                    range.start >= row.content_range.start && range.end <= row.content_range.end
                })
        {
            self.focus_source_pointer_line(line, window, cx);
            self.sync_source_selection_visuals(cx);
            return;
        }

        let focused_block = self
            .source_row_blocks
            .values()
            .find(|block| {
                let block = block.read(cx);
                block.focus_handle.is_focused(window)
                    || block.source_host_input_focus_handle.is_some()
            })
            .cloned();
        let reusable = self
            .active_edit
            .as_ref()
            .map(|active| active.block.clone())
            .or(focused_block)
            .or_else(|| self.source_row_blocks.get(&line).cloned())
            .or_else(|| self.source_row_blocks.values().next().cloned());
        let Some(block) = reusable else {
            self.active_edit = None;
            self.focus_handle.focus(window);
            self.sync_source_selection_visuals(cx);
            return;
        };
        let host_focus = self.focus_handle.clone();
        block.update(cx, |block, cx| {
            block.set_source_host_input_focus_handle(Some(host_focus), cx);
        });
        match self.pin_source_input_row_after_edit(&block, selection, cx) {
            Ok(true) => self.focus_source_pointer_line(line, window, cx),
            Ok(false) => {
                self.active_edit = None;
                self.focus_handle.focus(window);
            }
            Err(error) => {
                self.active_edit = None;
                self.focus_handle.focus(window);
                self.error = Some(format!("无法恢复 Source 输入位置，请重试：{error}").into());
                cx.notify();
            }
        }
        self.sync_source_selection_visuals(cx);
    }

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
        self.focus_source_pointer_line(current_line, window, cx);
        self.sync_source_selection_visuals(cx);
        cx.emit(DocumentHostEvent::StateChanged);
        cx.notify();
        true
    }

    /// 文档首尾导航同步重锚水平窗口；共享选区与挂载输入面必须描述同一范围，保持扩选方向。
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
        let target_line = document.line_for_offset(next.head.byte_offset);
        self.active_edit = None;
        if let Some(line) = target_line {
            self.anchor_source_window_for_byte(line, next.head.byte_offset);
        }
        self.set_source_selection(next, cx);
    }

    /// 为可见源码行创建有界 Block 输入面；Host 动作延迟到 Block 更新结束，避免回调重入读取同一实体。
    pub(super) fn ensure_source_row_block(
        &mut self,
        line: usize,
        cx: &mut Context<Self>,
    ) -> Option<Entity<Block>> {
        let layout_identity = self.source_layout_identity_for_row(line, cx)?;
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
            self.apply_source_selection_visual(line, block, cx);
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
        cx.observe(&block, |view, observed, cx| {
            let input_is_visible = view
                .active_edit
                .as_ref()
                .is_some_and(|active| active.block.entity_id() == observed.entity_id())
                || view.source_ime_snapshot.as_ref().is_some_and(|snapshot| {
                    snapshot.row_entity.entity_id() == observed.entity_id()
                })
                || view.is_pending_source_ime_owner(&observed)
                || view.source_row_blocks.values().any(|candidate| {
                    candidate.entity_id() == observed.entity_id()
                        && candidate.read(cx).source_host_input_focus_handle.is_some()
                });
            if input_is_visible {
                cx.notify();
            }
        })
        .detach();
        self.source_row_blocks.insert(line, block.clone());
        self.apply_source_selection_visual(line, &block, cx);
        Some(block)
    }

    /// 布局缓存和输入授权使用同一当前窗口；尚无正文的 provisional 行仍仅提供只读画面。
    fn source_layout_identity_for_row(
        &self,
        line: usize,
        cx: &App,
    ) -> Option<SourceLayoutIdentity> {
        let pinned_row = self.current_pinned_source_row(line, cx);
        let row = if self.document.is_some() {
            self.current_source_pointer_row(line, cx)
        } else {
            self.displayed_screen_lines.row(line)
        }?;
        let pinned_column_start = pinned_row.and_then(|row| {
            self.document
                .as_ref()?
                .line_range(line as u64)
                .map(|line_range| row.content_range.start.saturating_sub(line_range.start))
        });
        Some(SourceLayoutIdentity {
            document_epoch: self.document_epoch,
            document_revision: self
                .document
                .as_ref()
                .map(SharedDocument::revision)
                .unwrap_or_default(),
            source_range: row.content_range.clone(),
            column_window_start: pinned_column_start
                .unwrap_or(self.displayed_screen_lines.column_window_start),
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
