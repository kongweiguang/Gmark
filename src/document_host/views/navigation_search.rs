// @author kongweiguang

//! Search and source paging navigation.

use super::*;

impl DocumentHost {
    pub(super) fn jump_to_search_result(&mut self, cx: &mut Context<Self>) {
        let Some(found_start) = self
            .search_results
            .get(self.search_selected)
            .map(|found| found.range.start)
        else {
            return;
        };
        let line = if let Some(document) = self.document.as_ref() {
            let Some(line) = document
                .line_for_offset(found_start)
                .and_then(|line| usize::try_from(line).ok())
            else {
                return;
            };
            self.anchor_source_window_for_byte(line as u64, found_start);
            line
        } else {
            let estimated = self.probe.estimated_lines.max(1);
            let line = ((found_start as u128 * estimated as u128) / self.probe.len.max(1) as u128)
                .min(usize::MAX as u128) as usize;
            self.source_window_start = 0;
            self.invalidate_source_rows();
            line.min(self.line_count().saturating_sub(1))
        };
        // CSV/TSV 的全文搜索仍以 Source 字节坐标为真值，但命中不能夺走用户当前的
        // 表格工作区；Source 选择留作随后切换或 Split 左栏同步使用。
        let keep_delimited_table = self.is_delimited_document()
            && matches!(
                self.view_mode,
                DocumentHostViewMode::Live
                    | DocumentHostViewMode::Structure
                    | DocumentHostViewMode::Split
            );
        if !keep_delimited_table {
            self.view_mode = DocumentHostViewMode::Source;
            self.sync_tab_active_view();
        }
        self.select_source_lines(line..line.saturating_add(1), false);
        self.scroll_source_line(line, ScrollStrategy::Top);
        cx.notify();
    }

    pub(super) fn navigate_search(&mut self, delta: i32, cx: &mut Context<Self>) {
        if self.search_results.is_empty() {
            return;
        }
        let count = self.search_results.len() as i64;
        self.search_selected =
            (self.search_selected as i64 + i64::from(delta)).rem_euclid(count) as usize;
        self.jump_to_search_result(cx);
    }

    pub(super) fn toggle_search_option(
        &mut self,
        update: impl FnOnce(&mut SearchOptions),
        cx: &mut Context<Self>,
    ) {
        update(&mut self.search_options);
        self.schedule_search(cx);
    }

    /// Defers search focus changes until composition ends and the input update lease is released.
    pub(crate) fn on_find_in_document(
        &mut self,
        _: &FindInDocument,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.defer_source_action_for_ime(
            super::source_ime::DeferredSourceAction::Block(BlockHostAction::Find),
            window,
            cx,
        ) {
            return;
        }
        self.navigation_visible = false;
        self.search_visible = true;
        let host = cx.entity().downgrade();
        self.search_input.update(cx, move |input, _cx| {
            input.set_host_action_handler(move |action, window, cx| {
                let host = host.clone();
                window.defer(cx, move |window, cx| {
                    let _ = host.update(cx, |view, cx| {
                        view.on_search_host_action(action, window, cx)
                    });
                });
            });
            input.focus_handle.focus(window);
        });
        cx.notify();
    }

    /// Defers line-navigation focus changes until composition ends and the input update lease is released.
    pub(crate) fn on_go_to_line(
        &mut self,
        _: &GoToLine,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.defer_source_action_for_ime(
            super::source_ime::DeferredSourceAction::Block(BlockHostAction::GoToLine),
            window,
            cx,
        ) {
            return;
        }
        self.search_visible = false;
        self.navigation_visible = true;
        let host = cx.entity().downgrade();
        self.navigation_input.update(cx, move |input, _cx| {
            input.set_host_action_handler(move |action, window, cx| {
                let host = host.clone();
                window.defer(cx, move |window, cx| {
                    let _ = host.update(cx, |view, cx| {
                        view.on_navigation_host_action(action, window, cx)
                    });
                });
            });
            let len = input.display_text().len();
            input.selected_range = 0..len;
            input.focus_handle.focus(window);
        });
        cx.notify();
    }

    /// Applies Find Next after native composition rather than moving the Source selection mid-preedit.
    pub(crate) fn on_find_next(
        &mut self,
        _: &FindNext,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.defer_source_action_for_ime(
            super::source_ime::DeferredSourceAction::Block(BlockHostAction::FindNext),
            window,
            cx,
        ) {
            return;
        }
        self.navigate_search(1, cx);
    }

    /// Applies Find Previous after native composition rather than moving the Source selection mid-preedit.
    pub(crate) fn on_find_previous(
        &mut self,
        _: &FindPrevious,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.defer_source_action_for_ime(
            super::source_ime::DeferredSourceAction::Block(BlockHostAction::FindPrevious),
            window,
            cx,
        ) {
            return;
        }
        self.navigate_search(-1, cx);
    }

    pub(crate) fn on_dismiss_transient_ui(
        &mut self,
        _: &DismissTransientUi,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.search_visible || self.navigation_visible || self.source_context_menu.is_some() {
            self.search_visible = false;
            self.navigation_visible = false;
            self.source_context_menu = None;
            self.focus_handle.focus(window);
            cx.notify();
        }
    }

    pub(super) fn scroll_page(&mut self, toward_end: bool, cx: &mut Context<Self>) {
        let handle = self.scroll_handle.0.borrow().base_handle.clone();
        let row_height = self.source_row_height.max(1.0);
        let local_top = (-f32::from(handle.offset().y) / row_height)
            .max(0.0)
            .floor() as usize;
        let top = self.source_list_origin.saturating_add(local_top);
        let page_rows = (f32::from(handle.bounds().size.height) / row_height)
            .floor()
            .max(1.0) as usize;
        let target = if toward_end {
            top.saturating_add(page_rows)
                .min(self.line_count().saturating_sub(1))
        } else {
            top.saturating_sub(page_rows)
        };
        // UniformList 的 logical_scroll_top/bottom 只描述当前挂载子树，虚拟列表中会同时
        // 返回 0；必须把稳定行高的像素 offset 映射回全局行，PageUp/Down 才能闭环。
        self.scroll_source_line_strict(target, ScrollStrategy::Top);
        cx.notify();
    }

    /// 按当前 Block 的像素行高计算翻页步幅，软换行时由 Block 布局保留目标 X 坐标。
    pub(super) fn move_source_page(
        &mut self,
        direction: i32,
        extend: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.view_mode == DocumentHostViewMode::Source && self.document.is_some() {
            let line = self
                .document
                .as_ref()
                .and_then(|document| {
                    document.line_for_offset(document.source_selection().head.byte_offset)
                })
                .and_then(|line| usize::try_from(line).ok());
            let line_height = line
                .and_then(|line| self.source_row_blocks.get(&line))
                .map(|block| f32::from(block.read(cx).last_line_height.max(px(1.0))))
                .unwrap_or(self.source_row_height.max(1.0));
            let viewport_height = f32::from(
                self.scroll_handle
                    .0
                    .borrow()
                    .base_handle
                    .bounds()
                    .size
                    .height,
            );
            let visual_rows = (viewport_height / line_height).floor().max(1.0) as usize;
            self.move_source_caret_by_visual_lines(direction, visual_rows, extend, window, cx);
        }
        self.scroll_page(direction > 0, cx);
    }

    /// Shift+Up 按显示行扩展 Source 选区，方向锚点保存在共享文档状态中。
    pub(super) fn on_select_up(
        &mut self,
        _: &SelectUp,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.defer_source_action_for_ime(
            super::source_ime::DeferredSourceAction::Vertical {
                direction: -1,
                extend: true,
            },
            window,
            cx,
        ) {
            return;
        }
        self.move_source_caret_by_visual_lines(-1, 1, true, window, cx);
    }

    /// Shift+Down 按显示行扩展 Source 选区，方向锚点保存在共享文档状态中。
    pub(super) fn on_select_down(
        &mut self,
        _: &SelectDown,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.defer_source_action_for_ime(
            super::source_ime::DeferredSourceAction::Vertical {
                direction: 1,
                extend: true,
            },
            window,
            cx,
        ) {
            return;
        }
        self.move_source_caret_by_visual_lines(1, 1, true, window, cx);
    }

    /// Ctrl+Home 和 Ctrl+Shift+Home 移动或扩展 Source 选区到文档开头。
    pub(super) fn on_move_to_document_start(
        &mut self,
        _: &MoveToDocumentStart,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.on_source_document_boundary(false, false, window, cx);
    }

    /// Ctrl+End 和 Ctrl+Shift+End 移动或扩展 Source 选区到文档结尾。
    pub(super) fn on_move_to_document_end(
        &mut self,
        _: &MoveToDocumentEnd,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.on_source_document_boundary(true, false, window, cx);
    }

    /// Shift+Ctrl+Home 保留原锚点并把 Source 选区头移到文档开头。
    pub(super) fn on_select_to_document_start(
        &mut self,
        _: &SelectToDocumentStart,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.on_source_document_boundary(false, true, window, cx);
    }

    /// Shift+Ctrl+End 保留原锚点并把 Source 选区头移到文档结尾。
    pub(super) fn on_select_to_document_end(
        &mut self,
        _: &SelectToDocumentEnd,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.on_source_document_boundary(true, true, window, cx);
    }

    /// 等待当前 Source 组合输入结束后再移动文档边界，并滚动到目标行。
    pub(super) fn on_source_document_boundary(
        &mut self,
        at_end: bool,
        extend: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.defer_source_action_for_ime(
            super::source_ime::DeferredSourceAction::DocumentBoundary { at_end, extend },
            window,
            cx,
        ) {
            return;
        }
        self.move_source_caret_to_boundary(at_end, extend, cx);
        self.scroll_source_line_strict(
            if at_end {
                self.line_count().saturating_sub(1)
            } else {
                0
            },
            ScrollStrategy::Top,
        );
        self.focus_handle.focus(window);
        self.restore_source_navigation_input(window, cx);
        cx.notify();
    }

    /// 结束组合输入后移动 caret 并翻动一页；结构视图仍仅滚动阅读视口。
    pub(super) fn on_page_up(&mut self, _: &PageUp, window: &mut Window, cx: &mut Context<Self>) {
        if self.defer_source_action_for_ime(
            super::source_ime::DeferredSourceAction::Page {
                direction: -1,
                extend: false,
            },
            window,
            cx,
        ) {
            return;
        }
        self.move_source_page(-1, false, window, cx);
    }

    /// 结束组合输入后移动 caret 并翻动一页；结构视图仍仅滚动阅读视口。
    pub(super) fn on_page_down(
        &mut self,
        _: &PageDown,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.defer_source_action_for_ime(
            super::source_ime::DeferredSourceAction::Page {
                direction: 1,
                extend: false,
            },
            window,
            cx,
        ) {
            return;
        }
        self.move_source_page(1, false, window, cx);
    }

    /// Shift+PageUp 扩展 Source 选区并保留原锚点，再按相同显示行步幅滚动。
    pub(super) fn on_select_page_up(
        &mut self,
        _: &SelectPageUp,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.defer_source_action_for_ime(
            super::source_ime::DeferredSourceAction::Page {
                direction: -1,
                extend: true,
            },
            window,
            cx,
        ) {
            return;
        }
        self.move_source_page(-1, true, window, cx);
    }

    /// Shift+PageDown 扩展 Source 选区并保留原锚点，再按相同显示行步幅滚动。
    pub(super) fn on_select_page_down(
        &mut self,
        _: &SelectPageDown,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.defer_source_action_for_ime(
            super::source_ime::DeferredSourceAction::Page {
                direction: 1,
                extend: true,
            },
            window,
            cx,
        ) {
            return;
        }
        self.move_source_page(1, true, window, cx);
    }

    /// Defers document-boundary scrolling until native composition ends.
    pub(super) fn on_jump_to_top(
        &mut self,
        _: &JumpToTop,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.defer_source_action_for_ime(
            super::source_ime::DeferredSourceAction::Block(BlockHostAction::JumpToTop),
            window,
            cx,
        ) {
            return;
        }
        self.scroll_source_line_strict(0, ScrollStrategy::Top);
        cx.notify();
    }

    /// Defers document-boundary scrolling until native composition ends.
    pub(super) fn on_jump_to_bottom(
        &mut self,
        _: &JumpToBottom,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.defer_source_action_for_ime(
            super::source_ime::DeferredSourceAction::Block(BlockHostAction::JumpToBottom),
            window,
            cx,
        ) {
            return;
        }
        if let Some(last) = self.line_count().checked_sub(1) {
            self.scroll_source_line_strict(last, ScrollStrategy::Bottom);
            cx.notify();
        }
    }
}
