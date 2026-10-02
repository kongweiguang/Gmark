// @author kongweiguang

//! Incremental tabular navigation and line editing.

use super::*;

impl DocumentHost {
    pub(super) fn request_structured_rows(
        &mut self,
        visible: Range<usize>,
        cx: &mut Context<Self>,
    ) {
        if matches!(self.structured_index, Some(StructuredIndex::Json { .. })) {
            self.request_json_rows(visible, cx);
            return;
        }
        let Some(index) = self.structured_index.clone() else {
            return;
        };
        let filter_active = !self
            .structured_filter_input
            .read(cx)
            .display_text()
            .trim()
            .is_empty();
        let row_count = if filter_active {
            self.structured_filtered_rows.len() as u64
        } else {
            index.row_count()
        };
        let start = visible.start.saturating_sub(STRUCTURED_OVERSCAN_ROWS) as u64;
        let end = (visible.end.saturating_add(STRUCTURED_OVERSCAN_ROWS) as u64).min(row_count);
        if start >= end {
            return;
        }
        let logical_rows = if filter_active {
            let Some(start) = usize::try_from(start).ok() else {
                return;
            };
            let Some(end) = usize::try_from(end).ok() else {
                return;
            };
            let Some(rows) = self.structured_filtered_rows.get(start..end) else {
                return;
            };
            rows.to_vec()
        } else {
            (start..end).collect::<Vec<_>>()
        };
        if logical_rows
            .iter()
            .all(|row| self.structured_rows.contains_key(row))
        {
            return;
        }
        // 同一时刻只允许一次视口读取。拖动滚动条会在相邻帧给出略有差异的范围；
        // 若每帧替换 Task，磁盘读取会持续被取消，画面只能在加载占位之间闪烁。
        if self.structured_pending.is_some() {
            return;
        }

        self.structured_generation = self.structured_generation.wrapping_add(1);
        let generation = self.structured_generation;
        let task_stamp = DocumentTaskStamp::capture(self, generation);
        let requested = start..end;
        let requested_center = logical_rows
            .get(logical_rows.len() / 2)
            .copied()
            .unwrap_or(start);
        let requested_for_read = requested.clone();
        let requested_for_completion = requested.clone();
        let column_start = self.structured_column_window_start;
        let column_end = column_start.saturating_add(STRUCTURED_COLUMN_WINDOW);
        let columns = column_start..column_end;
        self.structured_pending = Some(requested.clone());
        self.structured_task = cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    if filter_active {
                        let mut rows = Vec::with_capacity(logical_rows.len());
                        for row in logical_rows {
                            rows.extend(index.read_rows(row, 1, columns.clone())?);
                        }
                        Ok(rows)
                    } else {
                        index.read_rows(
                            requested_for_read.start,
                            usize::try_from(requested_for_read.end - requested_for_read.start)
                                .unwrap_or(STRUCTURED_OVERSCAN_ROWS * 3),
                            columns,
                        )
                    }
                })
                .await;
            let _ = this.update(cx, |view, cx| {
                if !task_stamp.accepts_strict(view, view.structured_generation) {
                    if view.structured_pending.as_ref() == Some(&requested_for_completion) {
                        view.structured_pending = None;
                        cx.notify();
                    }
                    return;
                }
                view.structured_pending = None;
                match result {
                    Ok(rows) => {
                        view.structured_rows
                            .extend(rows.into_iter().map(|row| (row.index, row)));
                        // 保留相邻 viewport 的重叠行，避免小步滚动把上一帧重新打回占位；
                        // 超预算后只淘汰离本次请求中心最远的端点，内存仍与文件大小解耦。
                        prune_structured_row_cache(
                            &mut view.structured_rows,
                            requested_center,
                            MAX_STRUCTURED_CACHED_ROWS,
                        );
                        view.clear_structure_error();
                    }
                    Err(error) => view.set_structure_error(error, cx),
                }
                cx.notify();
            });
        });
    }

    /// Split 中只同步左侧源码位置，不改变当前模式，保证右侧预览仍留在原位。
    pub(super) fn reveal_structured_row_in_split(&mut self, row: u64, cx: &mut Context<Self>) {
        let Some(byte_offset) = self
            .structured_rows
            .get(&row)
            .map(|row| row.byte_range.start)
        else {
            return;
        };
        let Some(line) = self
            .document
            .as_ref()
            .and_then(|document| document.line_for_offset(byte_offset.min(document.len())))
            .and_then(|line| usize::try_from(line).ok())
        else {
            return;
        };
        self.anchor_source_window_for_byte(line as u64, byte_offset);
        self.select_source_lines(line..line.saturating_add(1), false);
        self.scroll_source_line(line, ScrollStrategy::Top);
        cx.notify();
    }

    pub(super) fn jump_byte_offset_to_source(&mut self, byte_offset: u64, cx: &mut Context<Self>) {
        let Some(line) = self
            .document
            .as_ref()
            .and_then(|document| document.line_for_offset(byte_offset.min(document.len())))
            .and_then(|line| usize::try_from(line).ok())
        else {
            return;
        };
        self.anchor_source_window_for_byte(line as u64, byte_offset);
        self.view_mode = DocumentHostViewMode::Source;
        self.sync_tab_active_view();
        self.select_source_lines(line..line.saturating_add(1), false);
        self.scroll_source_line(line, ScrollStrategy::Top);
        cx.notify();
    }

    pub(super) fn source_list_len(&self) -> usize {
        self.fold_projection
            .visible_line_count()
            .saturating_sub(self.source_list_origin)
            .min(SOURCE_LIST_WINDOW_ROWS)
    }

    pub(super) fn scroll_source_line(&mut self, line: usize, strategy: ScrollStrategy) {
        let local = self.prepare_source_list_target(line);
        self.scroll_handle.scroll_to_item(local, strategy);
    }

    pub(super) fn scroll_source_line_strict(&mut self, line: usize, strategy: ScrollStrategy) {
        let local = self.prepare_source_list_target(line);
        self.scroll_handle.scroll_to_item_strict(local, strategy);
    }

    fn prepare_source_list_target(&mut self, requested: usize) -> usize {
        let real_total = self.line_count().max(1);
        let real_target = requested.min(real_total.saturating_sub(1));
        self.ensure_source_line_visible(real_target);
        let total = self.fold_projection.visible_line_count().max(1);
        let target = self.fold_projection.visible_line_for_real(real_target);
        let window_end = self
            .source_list_origin
            .saturating_add(SOURCE_LIST_WINDOW_ROWS)
            .min(total);
        if target < self.source_list_origin || target >= window_end {
            self.source_list_origin = source_list_origin_for_target(total, target);
        }
        target.saturating_sub(self.source_list_origin)
    }

    pub(super) fn line_count(&self) -> usize {
        self.document.as_ref().map_or_else(
            || {
                usize::try_from(self.probe.estimated_lines)
                    .unwrap_or(usize::MAX)
                    .max(self.preview_lines.len())
            },
            |document| usize::try_from(document.line_count()).unwrap_or(usize::MAX),
        )
    }

    pub(super) fn line_window(&self, line: usize) -> Option<&BoundedLineWindow> {
        self.displayed_screen_lines.row(line)
    }

    pub(super) fn line_text(&self, line: usize) -> SharedString {
        if let Some(window) = self.line_window(line) {
            return window.rendered(self.show_line_endings);
        }
        self.preview_lines.get(line).cloned().unwrap_or_default()
    }

    pub(super) fn selected_search_range(&self, line: usize) -> Option<Range<usize>> {
        let found = self.search_results.get(self.search_selected)?;
        let document = self.document.as_ref()?;
        if document.line_for_offset(found.range.start)? != line as u64 {
            return None;
        }
        let window = self.line_window(line)?;
        if found.range.start >= window.content_range.end
            || found.range.end <= window.content_range.start
        {
            return None;
        }
        let rendered = &window.text;
        let start = usize::try_from(
            found
                .range
                .start
                .max(window.content_range.start)
                .saturating_sub(window.content_range.start),
        )
        .ok()?;
        let end = usize::try_from(
            found
                .range
                .end
                .min(window.content_range.end)
                .saturating_sub(window.content_range.start),
        )
        .ok()?;
        if start >= end
            || end > rendered.len()
            || !rendered.is_char_boundary(start)
            || !rendered.is_char_boundary(end)
        {
            return None;
        }
        Some(start..end)
    }

    /// Routes row commands through the IME queue before handing navigation and editing to the Host.
    pub(super) fn on_line_edit_host_action(
        &mut self,
        action: BlockHostAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !matches!(
            action,
            BlockHostAction::Submit(_) | BlockHostAction::DismissTransientUi
        ) && self.defer_source_action_for_ime(
            super::source_ime::DeferredSourceAction::Block(action.clone()),
            window,
            cx,
        ) {
            return;
        }
        match action {
            BlockHostAction::Submit(_) => {}
            BlockHostAction::LineOperation(operation) => {
                self.on_source_row_line_operation(operation, window, cx);
            }
            BlockHostAction::Save => self.on_save_document(&SaveDocument, window, cx),
            BlockHostAction::Undo => self.on_undo(&Undo, window, cx),
            BlockHostAction::Redo => self.on_redo(&Redo, window, cx),
            BlockHostAction::Find => self.on_find_in_document(&FindInDocument, window, cx),
            BlockHostAction::FindNext => self.on_find_next(&FindNext, window, cx),
            BlockHostAction::FindPrevious => self.on_find_previous(&FindPrevious, window, cx),
            BlockHostAction::GoToLine => self.on_go_to_line(&GoToLine, window, cx),
            BlockHostAction::PageUp => {
                self.leave_line_edit_for_viewport_navigation(window);
                self.on_page_up(&PageUp, window, cx);
            }
            BlockHostAction::PageDown => {
                self.leave_line_edit_for_viewport_navigation(window);
                self.on_page_down(&PageDown, window, cx);
            }
            BlockHostAction::JumpToTop => {
                self.leave_line_edit_for_viewport_navigation(window);
                self.on_jump_to_top(&JumpToTop, window, cx);
            }
            BlockHostAction::JumpToBottom => {
                self.leave_line_edit_for_viewport_navigation(window);
                self.on_jump_to_bottom(&JumpToBottom, window, cx);
            }
            BlockHostAction::DismissTransientUi => {
                self.on_dismiss_transient_ui(&DismissTransientUi, window, cx)
            }
        }
    }

    pub(super) fn leave_line_edit_for_viewport_navigation(&mut self, window: &mut Window) {
        // 翻页会卸载当前虚拟行；焦点若继续留在该 Block，下一次快捷键没有可达的
        // element path。编辑已按 Changed 事件增量提交，可以安全回到宿主焦点。
        self.active_edit = None;
        self.focus_handle.focus(window);
    }

    pub(super) fn select_or_edit_line(
        &mut self,
        line: usize,
        event: &gpui::MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.activate_source_row_from_pointer(line, event, window, cx);
    }
}
