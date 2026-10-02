// @author kongweiguang

//! Source selection synchronization and visual state.

use super::*;

impl DocumentHost {
    pub(super) fn selection_spans_multiple_lines(&self, selection: SourceSelection) -> bool {
        let Some(document) = self.document.as_ref() else {
            return false;
        };
        let range = selection.range();
        let start = document.line_for_offset(range.start);
        let end = document.line_for_offset(range.end.saturating_sub(1));
        start.zip(end).is_some_and(|(start, end)| start != end)
    }

    pub(super) fn set_source_selection(
        &mut self,
        selection: SourceSelection,
        cx: &mut Context<Self>,
    ) {
        let Some(document) = self.document.as_ref() else {
            return;
        };
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
        self.error = None;
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

    /// 只在布局中的选区实际改变时通知 Block，避免文件边缘和重复坐标触发空刷新。
    pub(super) fn apply_source_selection_visual(
        &self,
        line: usize,
        block: &Entity<Block>,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(document) = self.document.as_ref() else {
            return false;
        };
        let Some(row) = self.displayed_screen_lines.row(line) else {
            return false;
        };
        let selection = document.source_selection();
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
                Some(
                    usize::try_from(intersection_start - row.content_range.start)
                        .unwrap_or_default()
                        ..usize::try_from(intersection_end - row.content_range.start)
                            .unwrap_or(block.display_text().len()),
                )
            } else {
                search_range
            };
            if block.editor_selection_range == next {
                return false;
            }
            block.editor_selection_range = next;
            cx.notify();
            true
        })
    }
}
