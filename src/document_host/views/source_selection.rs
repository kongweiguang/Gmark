// @author kongweiguang

//! Source selection synchronization and visual state.

use super::*;

impl DocumentHost {
    /// 输入桥与鼠标共用当前窗口身份；跨块末端未挂载时只回退到仍有效的可见选区行。
    pub(super) fn source_text_input_owner_line(
        &self,
        selection: SourceSelection,
        cx: &App,
    ) -> Option<usize> {
        let document = self.document.as_ref()?;
        let head_line = document
            .line_for_offset(selection.head.byte_offset.min(document.len()))
            .and_then(|line| usize::try_from(line).ok())?;
        if self.active_edit.as_ref().is_some_and(|active| {
            active.line == head_line
                && self.source_row_blocks.get(&head_line).is_some_and(|block| {
                    *block == active.block
                        && block.read(cx).source_host_input_focus_handle.is_none()
                })
        }) {
            return None;
        }
        if self.source_row_blocks.get(&head_line).is_some_and(|block| {
            self.current_source_pointer_row(head_line, cx)
                .is_some_and(|row| {
                    let block = block.read(cx);
                    block
                        .source_layout_identity
                        .as_ref()
                        .is_some_and(|identity| {
                            identity.document_epoch == self.document_epoch
                                && identity.document_revision == document.revision()
                                && identity.source_range == row.content_range
                                && !block.is_read_only()
                        })
                })
        }) {
            return Some(head_line);
        }
        if self.is_current_pinned_source_row(head_line, selection, cx) {
            return Some(head_line);
        }

        let selected = selection.range();
        if selected.is_empty() {
            return None;
        }
        let overlaps_selection = |line: usize| {
            self.current_source_input_row_range(line, cx)
                .is_some_and(|row| row.start < selected.end && selected.start < row.end)
        };
        let anchor_line = document
            .line_for_offset(selection.anchor.byte_offset.min(document.len()))
            .and_then(|line| usize::try_from(line).ok());
        anchor_line
            .filter(|line| overlaps_selection(*line))
            .or_else(|| {
                self.source_row_blocks
                    .keys()
                    .copied()
                    .filter(|line| overlaps_selection(*line))
                    .min()
            })
    }

    /// Returns the current editable source range for a mounted row, including native-focus rows without a Host bridge.
    fn current_source_input_row_range(&self, line: usize, cx: &App) -> Option<Range<u64>> {
        let document = self.document.as_ref()?;
        let block = self.source_row_blocks.get(&line)?.read(cx);
        let row = self.source_rows.get(&line)?;
        let identity = block.source_layout_identity.as_ref()?;
        if self.source_row_epochs.get(&line) != Some(&self.source_cache_epoch)
            || block.is_read_only()
            || identity.document_epoch != self.document_epoch
            || identity.document_revision != document.revision()
            || identity.source_range != row.content_range
        {
            return None;
        }
        Some(row.replace_range.clone())
    }

    /// 验证 pinned 行的真实源码窗口、revision 与插入点，避免 stale viewport 夺走原生输入目标。
    fn is_current_pinned_source_row(
        &self,
        line: usize,
        selection: SourceSelection,
        cx: &App,
    ) -> bool {
        let Some(document) = self.document.as_ref() else {
            return false;
        };
        let Some(block) = self.source_row_blocks.get(&line) else {
            return false;
        };
        let Some(row) = self.source_rows.get(&line) else {
            return false;
        };
        if self.source_row_epochs.get(&line) != Some(&self.source_cache_epoch)
            || document
                .line_for_offset(selection.head.byte_offset.min(document.len()))
                .and_then(|line| usize::try_from(line).ok())
                != Some(line)
        {
            return false;
        }
        let block = block.read(cx);
        block.source_host_input_focus_handle.is_some()
            && !block.is_read_only()
            && block
                .source_layout_identity
                .as_ref()
                .is_some_and(|identity| {
                    identity.document_epoch == self.document_epoch
                        && identity.document_revision == document.revision()
                        && identity.source_range == row.content_range
                        && selection.head.byte_offset >= row.content_range.start
                        && selection.head.byte_offset <= row.content_range.end
                })
    }

    /// 当前共享选区及写入授权使用同一窗口，过渡画面的旧范围不能取得原生输入桥。
    pub(super) fn is_current_source_text_input_owner(
        &self,
        block: &Entity<Block>,
        cx: &App,
    ) -> bool {
        let Some(document) = self.document.as_ref() else {
            return false;
        };
        let selection = document.source_selection();
        let Some(line) = self.source_text_input_owner_line(selection, cx) else {
            return false;
        };
        if self
            .source_row_blocks
            .get(&line)
            .is_none_or(|owner| owner != block)
        {
            return false;
        }
        let block_state = block.read(cx);
        let Some(identity) = block_state.source_layout_identity.as_ref() else {
            return false;
        };
        let Some(row_range) = self
            .current_source_pointer_row(line, cx)
            .map(|row| row.content_range.clone())
        else {
            return false;
        };
        let projected = project_selection_to_source_row(selection.range(), row_range.clone());
        block_state.source_host_input_focus_handle.is_some()
            && !block_state.is_read_only()
            && identity.document_epoch == self.document_epoch
            && identity.document_revision == document.revision()
            && identity.source_range == row_range
            && block_state.editor_selection_range.as_ref() == Some(&projected)
    }

    /// 折叠插入点没有覆盖换行；行首不能因 end-1 落入上一行而被误判为跨行选择。
    pub(super) fn selection_spans_multiple_lines(&self, selection: SourceSelection) -> bool {
        let Some(document) = self.document.as_ref() else {
            return false;
        };
        let range = selection.range();
        if range.is_empty() {
            return false;
        }
        let start = document.line_for_offset(range.start);
        let end = document.line_for_offset(range.end.saturating_sub(1));
        start.zip(end).is_some_and(|(start, end)| start != end)
    }

    /// 真实选区变化结束输入组和旧边界查找；后台长词结果不能追上用户的新命中位置。
    pub(super) fn set_source_selection(
        &mut self,
        selection: SourceSelection,
        cx: &mut Context<Self>,
    ) {
        let Some(document) = self.document.clone() else {
            return;
        };
        let had_pending_text = self.coordinator.source_boundary_actions.iter().any(|action| {
            matches!(action, super::source_ime::DeferredSourceAction::BoundaryText { text, .. } if !text.is_empty())
        });
        if document.source_selection() != selection {
            self.source_typing_group = None;
            if self.coordinator.source_boundary_cancellation.is_some() {
                self.cancel_source_boundary(cx);
            }
        }
        let _ = document.set_source_selection(selection);
        let normalized = document.source_selection().range();
        let start_line = document
            .line_for_offset(normalized.start)
            .and_then(|line| usize::try_from(line).ok())
            .unwrap_or_default();
        let end_offset = if normalized.is_empty() {
            normalized.end
        } else {
            normalized.end.saturating_sub(1)
        };
        let end_line = document
            .line_for_offset(end_offset)
            .and_then(|line| usize::try_from(line).ok())
            .unwrap_or(start_line);
        self.selection_anchor = document
            .line_for_offset(selection.anchor.byte_offset)
            .and_then(|line| usize::try_from(line).ok());
        self.selected_lines = Some(start_line..end_line.saturating_add(1));
        if !had_pending_text && self.coordinator.source_boundary_recovery_text.is_none() {
            self.error = None;
        }
        self.sync_source_selection_visuals(cx);
        cx.notify();
    }

    /// 常规同步复用同一组可见行映射，不把查找或键盘选择误计为拖选耗时。
    pub(super) fn sync_source_selection_visuals(&mut self, cx: &mut Context<Self>) {
        self.sync_source_selection_visuals_with_drag_trace(false, cx);
    }

    /// 边缘拖选同样采样实际改变的文字行；未变的行既不重绘，也不覆盖已有输入样本。
    pub(super) fn sync_source_selection_visuals_with_drag_trace(
        &mut self,
        trace_drag: bool,
        cx: &mut Context<Self>,
    ) {
        let rows = self
            .source_row_blocks
            .iter()
            .map(|(line, block)| (*line, block.clone()))
            .collect::<Vec<_>>();
        for (line, block) in rows {
            if self.apply_source_selection_visual(line, &block, cx) && trace_drag {
                block.update(cx, |block, cx| block.begin_selection_input_trace(cx));
            }
        }
    }

    /// 高亮与命中复用同一 pinned 行，旧水平视口只能保留绘制，不能覆盖当前局部选区。
    pub(super) fn apply_source_selection_visual(
        &self,
        line: usize,
        block: &Entity<Block>,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(document) = self.document.as_ref() else {
            return false;
        };
        let selection = document.source_selection();
        let host_input_owner = self.source_text_input_owner_line(selection, cx) == Some(line);
        let row = self.current_source_pointer_row(line, cx);
        let Some(row) = row else {
            return false;
        };
        let normalized = selection.range();
        let intersection_start = normalized.start.max(row.content_range.start);
        let intersection_end = normalized.end.min(row.content_range.end);
        let is_active_local = self
            .active_edit
            .as_ref()
            .is_some_and(|active| active.line == line)
            && !self.selection_spans_multiple_lines(selection);
        let search_range = normalized
            .is_empty()
            .then(|| self.selected_search_range(line))
            .flatten()
            .filter(|_| {
                self.active_edit
                    .as_ref()
                    .is_none_or(|active| active.line != line)
            });
        block.update(cx, |block, cx| {
            let next = if is_active_local {
                None
            } else if intersection_start < intersection_end {
                Some(project_selection_to_source_row(
                    normalized.clone(),
                    row.content_range.clone(),
                ))
            } else if host_input_owner {
                let caret = selection
                    .head
                    .byte_offset
                    .clamp(row.content_range.start, row.content_range.end)
                    .saturating_sub(row.content_range.start) as usize;
                Some(caret..caret)
            } else {
                search_range
            };
            let changed = block.editor_selection_range != next;
            if changed {
                block.editor_selection_range = next;
            }
            let previous_owner = block.source_host_input_focus_handle.is_some();
            let next_owner = host_input_owner && !block.is_read_only();
            if previous_owner != next_owner {
                block.set_source_host_input_focus_handle(
                    next_owner.then(|| self.focus_handle.clone()),
                    cx,
                );
            }
            if changed {
                cx.notify();
            }
            changed || previous_owner != next_owner
        })
    }
}

/// 将共享源码范围裁到单行，保证跨行选区投影后的 UTF-8 偏移有序且局部化。
pub(super) fn project_selection_to_source_row(range: Range<u64>, row: Range<u64>) -> Range<usize> {
    let start = range
        .start
        .clamp(row.start, row.end)
        .saturating_sub(row.start);
    let end = range
        .end
        .clamp(row.start, row.end)
        .saturating_sub(row.start);
    usize::try_from(start).unwrap_or_default()..usize::try_from(end).unwrap_or_default()
}
