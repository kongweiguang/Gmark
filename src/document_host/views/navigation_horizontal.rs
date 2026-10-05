// @author kongweiguang

use super::source_boundaries::{bounded_horizontal_target, resolve_horizontal_target};
use super::*;

/// Source 的水平动作由共享字节选区处理；普通字段和 Live 内容继续沿用 Block 自身规则。
impl DocumentHost {
    /// Captures navigation only when the actual focused surface is Source or Split's Source side.
    pub(super) fn route_source_horizontal(
        &mut self,
        direction: i32,
        by_word: bool,
        extend: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !matches!(
            self.view_mode,
            DocumentHostViewMode::Source | DocumentHostViewMode::Split
        ) || !self.source_text_surface_has_focus(window, cx)
        {
            return false;
        }
        if self.defer_source_action_for_ime(
            super::source_ime::DeferredSourceAction::Horizontal {
                direction,
                by_word,
                extend,
            },
            window,
            cx,
        ) {
            return true;
        }
        self.apply_source_horizontal(direction, by_word, extend, window, cx);
        true
    }

    /// Home/End use the mounted Block's measured wrapped-row boundary, not the logical file line.
    pub(super) fn route_source_visual_line_boundary(
        &mut self,
        at_end: bool,
        extend: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !matches!(
            self.view_mode,
            DocumentHostViewMode::Source | DocumentHostViewMode::Split
        ) || !self.source_text_surface_has_focus(window, cx)
        {
            return false;
        }
        if self.defer_source_action_for_ime(
            super::source_ime::DeferredSourceAction::VisualLineBoundary { at_end, extend },
            window,
            cx,
        ) {
            return true;
        }
        self.apply_source_visual_line_boundary(at_end, extend, window, cx);
        true
    }

    /// 将水平移动解析到锚点附近的有界源码窗口，并只发布绝对字节选区。
    /// 窗口边缘可能只是视口截断，不能当作文档、词或字素边界。
    pub(super) fn apply_source_horizontal(
        &mut self,
        direction: i32,
        by_word: bool,
        extend: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(document) = self.document.clone() else {
            return;
        };
        let before = document.source_selection();
        let range = before.range();
        let offset = before.head.byte_offset.min(document.len());
        let Some(line) = document
            .line_for_offset(offset)
            .and_then(|line| usize::try_from(line).ok())
        else {
            return;
        };
        if !extend && !range.is_empty() {
            let target = if direction < 0 {
                range.start
            } else {
                range.end
            };
            self.publish_source_horizontal_selection(&document, before, target, extend, window, cx);
            return;
        }

        if let Some(block) = self.source_row_blocks.get(&line)
            && block.read(cx).is_read_only()
        {
            return;
        }
        let Ok(source_line) = u64::try_from(line) else {
            return;
        };
        let Some(line_range) = document.line_range(source_line) else {
            return;
        };
        let row = match bounded_source_navigation_window(&document, source_line, offset) {
            Ok(Some(row)) => row,
            Ok(None) | Err(_) => return,
        };
        let local = match usize::try_from(offset.saturating_sub(row.content_range.start)) {
            Ok(local) if local <= row.text.len() && row.text.is_char_boundary(local) => local,
            _ => return,
        };

        let forward = direction > 0;
        let target = if local == 0 && !forward && !row.leading_truncated {
            Some(
                self.adjacent_source_line_boundary(line, false)
                    .unwrap_or(offset),
            )
        } else if local == row.text.len() && forward && !row.trailing_truncated {
            Some(
                self.adjacent_source_line_boundary(line, true)
                    .unwrap_or(offset),
            )
        } else {
            bounded_horizontal_target(&row, line_range.clone(), offset, forward, by_word)
        };
        let Some(target) = target else {
            let resolve_line = line_range.clone();
            self.request_source_boundary(
                move |snapshot, cancel| async move {
                    resolve_horizontal_target(
                        snapshot.as_ref(),
                        resolve_line,
                        offset,
                        forward,
                        by_word,
                        &cancel,
                    )
                },
                move |view, target, window, cx| {
                    let Some(document) = view.document.clone() else {
                        return;
                    };
                    view.publish_source_horizontal_selection(
                        &document, before, target, extend, window, cx,
                    );
                },
                window,
                cx,
            );
            return;
        };
        self.publish_source_horizontal_selection(&document, before, target, extend, window, cx);
    }

    /// 统一发布共享选区；只有目标离开有效窗口时才重锚，避免旧本地选区吞字或反复失效视口。
    fn publish_source_horizontal_selection(
        &mut self,
        document: &SharedDocument,
        before: SourceSelection,
        target: u64,
        extend: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let head = SourceAnchor::new(target, SourceAffinity::After);
        let selection = SourceSelection {
            anchor: if extend { before.anchor } else { head },
            head,
        };
        if selection == before {
            return;
        }
        let Some(line) = document
            .line_for_offset(target.min(document.len()))
            .and_then(|line| usize::try_from(line).ok())
        else {
            return;
        };
        let Ok(source_line) = u64::try_from(line) else {
            return;
        };
        let needs_window_anchor = self.current_source_pointer_row(line, cx).is_none_or(|row| {
            target < row.content_range.start
                || target > row.content_range.end
                || (target == row.content_range.start && row.leading_truncated)
                || (target == row.content_range.end && row.trailing_truncated)
        });
        if needs_window_anchor {
            self.anchor_source_window_for_byte(source_line, target);
        }
        self.set_source_selection(selection, cx);
        self.restore_source_navigation_input(window, cx);
        cx.emit(DocumentHostEvent::StateChanged);
        cx.notify();
    }

    /// 保留普通行的 Block 排版边界；触及被截断的窗口边缘时异步解析真实逻辑行边界。
    pub(super) fn apply_source_visual_line_boundary(
        &mut self,
        at_end: bool,
        extend: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(document) = self.document.clone() else {
            return;
        };
        let before = document.source_selection();
        let offset = before.head.byte_offset.min(document.len());
        let Some(line) = document
            .line_for_offset(offset)
            .and_then(|line| usize::try_from(line).ok())
        else {
            return;
        };
        let Some(row) = self.current_source_pointer_row(line, cx).cloned() else {
            return;
        };
        let Some(block) = self.source_row_blocks.get(&line).cloned() else {
            return;
        };
        let local = usize::try_from(offset.saturating_sub(row.content_range.start))
            .unwrap_or_default()
            .min(block.read(cx).display_text().len());
        let local_target = block.update(cx, |block, _cx| {
            let selected = block.selected_range.clone();
            let reversed = block.selection_reversed;
            block.selected_range = local..local;
            block.selection_reversed = false;
            let target = block.current_visual_line_boundary(at_end);
            block.selected_range = selected;
            block.selection_reversed = reversed;
            target
        });
        let reached_truncated_edge =
            (at_end && row.trailing_truncated && local_target >= row.text.len())
                || (!at_end && row.leading_truncated && local_target == 0);
        if reached_truncated_edge {
            let Ok(source_line) = u64::try_from(line) else {
                return;
            };
            let Some(line_range) = document.line_range(source_line) else {
                return;
            };
            let boundary = if at_end {
                line_range.end
            } else {
                line_range.start
            };
            self.request_source_boundary(
                move |snapshot, cancel| async move {
                    resolve_horizontal_target(
                        snapshot.as_ref(),
                        line_range,
                        boundary,
                        at_end,
                        false,
                        &cancel,
                    )
                },
                move |view, target, window, cx| {
                    let Some(document) = view.document.clone() else {
                        return;
                    };
                    view.publish_source_horizontal_selection(
                        &document, before, target, extend, window, cx,
                    );
                },
                window,
                cx,
            );
            return;
        }
        let Ok(local_target) = u64::try_from(local_target.min(row.text.len())) else {
            return;
        };
        let target = row.content_range.start.saturating_add(local_target);
        let head = SourceAnchor::new(target, SourceAffinity::After);
        if target == offset && (!extend || before.anchor == head) {
            return;
        }
        self.publish_source_horizontal_selection(&document, before, target, extend, window, cx);
    }

    /// Moves between logical Source rows while treating CR, LF, and CRLF as one navigation step.
    fn adjacent_source_line_boundary(&self, line: usize, forward: bool) -> Option<u64> {
        let document = self.document.as_ref()?;
        let adjacent = if forward {
            line.checked_add(1)?
        } else {
            line.checked_sub(1)?
        };
        let range = document.line_range(adjacent as u64)?;
        if forward {
            Some(range.start)
        } else {
            let tail_start = range.end.saturating_sub(2).max(range.start);
            let tail = document.read_range(tail_start..range.end).ok()?;
            let ending = if tail.ends_with(b"\r\n") {
                2
            } else if tail.ends_with(b"\n") || tail.ends_with(b"\r") {
                1
            } else {
                0
            };
            Some(range.end.saturating_sub(ending))
        }
    }
}

/// 为导航和后续源码命中操作提供同一入口；每次只物化锚点附近的一行窗口。
pub(super) fn bounded_source_navigation_window(
    document: &SharedDocument,
    line: u64,
    byte_offset: u64,
) -> Result<Option<BoundedLineWindow>, gmark_paged_document::PagedDocumentError> {
    let Some(line_range) = document.line_range(line) else {
        return Ok(None);
    };
    let relative = byte_offset
        .clamp(line_range.start, line_range.end)
        .saturating_sub(line_range.start);
    let requested_start =
        source_window_start_for_anchor(line_range.end.saturating_sub(line_range.start), relative);
    read_bounded_line_window(document, line, requested_start)
}
