// @author kongweiguang

use super::*;

const PAGED_FOLD_OVERSCAN_ROWS: usize = 2_048;
const PAGED_FOLD_WINDOW_BYTES: u64 = 4 * 1024 * 1024;

impl DocumentHost {
    pub(super) fn render_source_gutter(
        line: usize,
        display_line: usize,
        end_line: Option<usize>,
        collapsed: bool,
        gutter_width: f32,
        fold_lane_width: f32,
        number_width: f32,
        text_color: gpui::Hsla,
        separator_color: gpui::Hsla,
        hover_color: gpui::Hsla,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let toggle = end_line.map(|end_line| {
            Self::render_source_fold_toggle(line, end_line, collapsed, text_color, hover_color, cx)
        });
        div()
            .w(px(gutter_width))
            .h_full()
            .flex()
            .items_center()
            .border_r(px(1.0))
            .border_color(separator_color)
            .text_color(text_color)
            .child(
                div()
                    .w(px(fold_lane_width))
                    .h_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .children(toggle),
            )
            .child(
                div()
                    .w(px(number_width))
                    .pr(px(12.0))
                    .text_align(gpui::TextAlign::Right)
                    .child((display_line + 1).to_string()),
            )
            .into_any_element()
    }

    pub(super) fn render_source_fold_placeholder(
        label: String,
        background: gpui::Hsla,
        accent_color: gpui::Hsla,
        _punctuation_color: gpui::Hsla,
        _line_number_color: gpui::Hsla,
    ) -> gpui::AnyElement {
        div()
            .ml(px(8.0))
            .px(px(5.0))
            .rounded(px(4.0))
            .bg(background)
            .text_color(accent_color)
            .child(label)
            .into_any_element()
    }

    pub(super) fn render_source_fold_toggle(
        line: usize,
        end_line: usize,
        collapsed: bool,
        text_color: gpui::Hsla,
        hover_color: gpui::Hsla,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        div()
            .id(("source-fold-toggle", line))
            .debug_selector(move || format!("source-fold-toggle-{line}-{end_line}"))
            .size(px(18.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(4.0))
            .cursor_pointer()
            .hover(move |button| button.bg(hover_color))
            .tooltip(move |_window, cx| {
                crate::ui::ui_tooltip(
                    format!(
                        "{}第 {}–{} 行",
                        if collapsed { "展开" } else { "折叠" },
                        line + 1,
                        end_line + 1
                    ),
                    cx,
                )
            })
            .child(
                svg()
                    .path(if collapsed {
                        "icon/ui/chevron-right.svg"
                    } else {
                        "icon/ui/chevron-down.svg"
                    })
                    .size(px(12.0))
                    .text_color(text_color),
            )
            .on_click(cx.listener(move |this, _event, _window, cx| {
                cx.stop_propagation();
                this.toggle_fold_at_source_line(line, cx);
            }))
            .into_any_element()
    }

    /// Render 可以高频调用此入口；revision 与窗口门禁保证不会重复启动同一解析任务。
    pub(super) fn maybe_schedule_fold_refresh(&mut self, cx: &mut Context<Self>) {
        if !self.folding_enabled
            || !self.source_language.supports_folding()
            || self.closed_suspended
        {
            return;
        }
        let Some(document) = self.document.clone() else {
            return;
        };
        let revision = document.revision();
        let line_count = self.line_count().max(1);
        let paged = self.probe.strategy == OpenStrategy::Paged;
        let (line_window, byte_range) = if paged {
            let visible = self.source_last_visible.clone().unwrap_or_else(|| {
                self.source_list_origin..self.source_list_origin.saturating_add(1)
            });
            let start = visible.start.saturating_sub(PAGED_FOLD_OVERSCAN_ROWS);
            let end = visible
                .end
                .saturating_add(PAGED_FOLD_OVERSCAN_ROWS)
                .min(line_count)
                .max(start.saturating_add(1));
            if self.fold_snapshot_revision == Some(revision)
                && self.fold_window.as_ref().is_some_and(|window| {
                    window.start <= visible.start && window.end >= visible.end
                })
            {
                return;
            }
            let Some(start_byte) = document.line_range(start as u64).map(|range| range.start)
            else {
                return;
            };
            let end_byte = document
                .line_range(end.saturating_sub(1) as u64)
                .map(|range| range.end)
                .unwrap_or(document.len())
                .min(start_byte.saturating_add(PAGED_FOLD_WINDOW_BYTES));
            (start..end, start_byte..end_byte)
        } else {
            if self.fold_snapshot_revision == Some(revision) && self.fold_window.is_none() {
                return;
            }
            (0..line_count, 0..document.len())
        };

        if let Some(cancellation) = self.fold_cancellation.take() {
            cancellation.cancel();
        }
        self.fold_generation = self.fold_generation.wrapping_add(1);
        let generation = self.fold_generation;
        let task_stamp = DocumentTaskStamp::capture(self, generation);
        let cancellation = SearchCancellation::default();
        let worker_cancellation = cancellation.clone();
        let language = self.source_language;
        let byte_base = byte_range.start;
        let line_base = line_window.start;
        let requested_window = line_window.clone();
        let reaches_eof = byte_range.end >= document.len();
        let document_epoch = task_stamp.document_epoch;
        let resident_parser = Arc::clone(&self.fold_parser);
        self.fold_cancellation = Some(cancellation);
        self.fold_task = cx.spawn(async move |this, cx| {
            let parsed = cx
                .background_spawn(async move {
                    let mut bytes =
                        document.read_range_cancellable(byte_range, &worker_cancellation)?;
                    if worker_cancellation.is_cancelled() {
                        return Err(PagedDocumentError::Cancelled);
                    }
                    // 4 MiB 上限可能截在 UTF-8 字符或超长行中间。Paged 只发布边界完整的
                    // 行，避免把半行误判为语法错误或生成无法闭合的折叠区域。
                    if paged && !reaches_eof {
                        let complete_end = bytes
                            .iter()
                            .rposition(|byte| *byte == b'\n')
                            .map_or(0, |index| index + 1);
                        bytes.truncate(complete_end);
                    }
                    let source =
                        String::from_utf8(bytes).map_err(|_| PagedDocumentError::Binary)?;
                    let complete_lines = source.bytes().filter(|byte| *byte == b'\n').count()
                        + usize::from(reaches_eof && !source.is_empty() && !source.ends_with('\n'));
                    let parsed_window = line_base..line_base.saturating_add(complete_lines);
                    let regions = if paged {
                        crate::source_tools::discover_fold_regions(
                            language, &source, byte_base, line_base,
                        )
                    } else {
                        resident_parser
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .parse(document_epoch, language, &source)
                    };
                    Ok::<_, PagedDocumentError>((regions, parsed_window))
                })
                .await;
            let _ = this.update(cx, |view, cx| {
                if !task_stamp.accepts_strict(view, view.fold_generation) {
                    return;
                }
                view.fold_cancellation = None;
                match parsed {
                    Ok((regions, parsed_window)) => {
                        let total = view.line_count();
                        if paged {
                            view.fold_projection.replace_window_regions(
                                total,
                                parsed_window,
                                regions,
                            );
                            // 即使窗口中只有一个超过 4 MiB 的长行，也记录本次有界尝试，
                            // 防止每一帧重复读取同一不可折叠窗口。
                            view.fold_window = Some(requested_window);
                        } else {
                            view.fold_projection.set_regions(total, regions);
                            view.fold_window = None;
                        }
                        view.apply_pending_source_folds(paged);
                        view.fold_snapshot_revision =
                            view.document.as_ref().map(SharedDocument::revision);
                    }
                    Err(PagedDocumentError::Cancelled) => {}
                    Err(error) => view.error = Some(localized_document_error(&error, cx)),
                }
                cx.notify();
            });
        });
    }

    /// Retains the native IME owner while invalidating row entities after the fold projection changes.
    pub(crate) fn toggle_fold_at_source_line(&mut self, line: usize, cx: &mut Context<Self>) {
        let Some(id) = self
            .fold_projection
            .region_starting(line)
            .map(|region| region.id)
        else {
            return;
        };
        if self.fold_projection.toggle(id) {
            if self.fold_projection.is_collapsed(id) {
                self.pending_source_collapsed_folds.insert(id);
            } else {
                self.pending_source_collapsed_folds.remove(&id);
            }
            self.active_edit = None;
            self.clear_source_row_blocks_except_ime_owner();
            cx.emit(DocumentHostEvent::StateChanged);
            cx.notify();
        }
    }

    /// Preserves the composing Block across fold invalidation until its native terminal event arrives.
    fn set_fold_at_source_line(&mut self, line: usize, collapsed: bool, cx: &mut Context<Self>) {
        let region = self.fold_projection.region_starting(line).or_else(|| {
            self.fold_projection
                .regions()
                .iter()
                .filter(|region| line >= region.start_line && line <= region.end_line)
                .max_by_key(|region| region.depth)
        });
        let Some(id) = region.map(|region| region.id) else {
            return;
        };
        if self.fold_projection.set_collapsed(id, collapsed) {
            if collapsed {
                self.pending_source_collapsed_folds.insert(id);
            } else {
                self.pending_source_collapsed_folds.remove(&id);
            }
            self.active_edit = None;
            self.clear_source_row_blocks_except_ime_owner();
            cx.emit(DocumentHostEvent::StateChanged);
            cx.notify();
        }
    }

    pub(crate) fn on_collapse_fold(
        &mut self,
        _: &CollapseFold,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let line = self.selected_lines.as_ref().map_or(0, |lines| lines.start);
        self.set_fold_at_source_line(line, true, cx);
    }

    pub(crate) fn on_expand_fold(
        &mut self,
        _: &ExpandFold,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let line = self.selected_lines.as_ref().map_or(0, |lines| lines.start);
        self.set_fold_at_source_line(line, false, cx);
    }

    pub(crate) fn on_collapse_all_folds(
        &mut self,
        _: &CollapseAllFolds,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.collapse_all_source_folds(cx);
    }

    pub(crate) fn on_expand_all_folds(
        &mut self,
        _: &ExpandAllFolds,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.expand_all_source_folds(cx);
    }

    /// Collapses the projection without unmounting the row that still owns native composition.
    pub(super) fn collapse_all_source_folds(&mut self, cx: &mut Context<Self>) {
        self.active_edit = None;
        self.fold_projection.collapse_all();
        self.pending_source_collapsed_folds = self
            .fold_projection
            .regions()
            .iter()
            .map(|region| region.id)
            .collect();
        self.clear_source_row_blocks_except_ime_owner();
        cx.emit(DocumentHostEvent::StateChanged);
        cx.notify();
    }

    /// Rebuilds expanded row surfaces while retaining any Block awaiting its composition terminal event.
    pub(super) fn expand_all_source_folds(&mut self, cx: &mut Context<Self>) {
        self.fold_projection.expand_all();
        self.pending_source_collapsed_folds.clear();
        self.clear_source_row_blocks_except_ime_owner();
        cx.emit(DocumentHostEvent::StateChanged);
        cx.notify();
    }

    /// Unfolds a target line while keeping the native composition owner addressable through the rerender.
    pub(super) fn ensure_source_line_visible(&mut self, line: usize) {
        if self.fold_projection.ensure_line_visible(line) {
            self.clear_source_row_blocks_except_ime_owner();
        }
    }

    fn apply_pending_source_folds(&mut self, paged: bool) {
        for id in &self.pending_source_collapsed_folds {
            self.fold_projection.set_collapsed(*id, true);
        }
        if !paged {
            self.pending_source_collapsed_folds.clear();
        }
    }

    pub(super) fn source_fold_placeholder(&self, line: usize) -> Option<String> {
        let region = self.fold_projection.region_starting(line)?;
        self.fold_projection.is_collapsed(region.id).then(|| {
            let closing = region
                .closing
                .map(|closing| format!(" {closing}"))
                .unwrap_or_default();
            format!("…{closing} · {} 行", region.hidden_line_count())
        })
    }
}
