// @author kongweiguang

use super::*;

impl Editor {
    pub(super) fn on_editor_main_content_capture_mouse_down(
        &mut self,
        _: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 侧栏中的按钮会主动截断冒泡；在主内容捕获阶段统一关闭导航菜单，
        // 同时不阻止目标控件继续处理本次点击。
        self.dismiss_menu_bar_from_body(cx);
    }

    pub(super) fn on_titlebar_close(
        &mut self,
        event: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.standard_click() {
            self.request_close_current_window(window, cx);
        }
    }

    pub(super) fn on_editor_content_mouse_down(
        &mut self,
        _: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut changed = false;
        if self.workspace.is_open && workspace_uses_overlay(f32::from(window.viewport_size().width))
        {
            self.workspace.is_open = false;
            changed = true;
        }
        if self.status_bar.format_overflow_open {
            self.status_bar.format_overflow_open = false;
            changed = true;
        }
        if self.status_bar.mode_menu_open {
            self.status_bar.mode_menu_open = false;
            changed = true;
        }
        if self.dismiss_tab_context_menu() {
            changed = true;
        }
        if self.dismiss_new_tab_menu() {
            changed = true;
        }
        if self.dismiss_split_pane_menu() {
            changed = true;
        }
        if changed {
            cx.notify();
        }
    }

    pub(crate) fn install_close_guard(&mut self, cx: &mut Context<Self>, window: &mut Window) {
        if self.close_guard_installed {
            return;
        }

        self.force_install_close_guard(cx, window);
    }

    pub(crate) fn force_install_close_guard(
        &mut self,
        cx: &mut Context<Self>,
        window: &mut Window,
    ) {
        self.install_workspace_session_window_observer(window, cx);
        let editor = cx.entity().downgrade();
        window.on_window_should_close(cx, move |window, cx| {
            editor
                .update(cx, |this, cx| this.on_window_should_close(window, cx))
                .unwrap_or(true)
        });
        self.close_guard_installed = true;
    }

    /// 焦点转移等待原输入目标收尾，避免平台结果被提交到新块。
    pub(in crate::editor) fn apply_pending_focus(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .pending_focus
            .and_then(|id| self.focusable_entity_by_id(id))
            .is_some_and(|block| block.read(cx).focus_handle.is_focused(window))
        {
            self.pending_focus = None;
        }
        if self.pending_focus.is_some() && self.wait_for_ime_completion(window, cx) {
            return;
        }
        if let Some(entity_id) = self.pending_focus.take()
            && let Some(block) = self.focusable_entity_by_id(entity_id)
        {
            block.update(cx, |block, cx| {
                block.focus_handle.focus(window);
                // A pane sibling may already have painted its text input
                // handler for this frame. Mark the newly focused block dirty so
                // its next paint registers the handler for this pane instead.
                cx.notify();
            });
        }
    }

    pub(super) fn ensure_focused_caret_visible(&mut self, window: &Window, cx: &App) -> bool {
        let Some(focused_block) = self.focused_edit_target(window, cx) else {
            return false;
        };
        let Some(active_bounds) =
            focused_block.read_with(cx, |block, _cx| block.active_range_or_cursor_bounds())
        else {
            return false;
        };

        let viewport = self.scroll_handle.bounds();
        let padding = px(20.0);
        let top_limit = viewport.top() + padding;
        let bottom_limit = viewport.bottom() - padding;
        let mut offset = self.scroll_handle.offset();
        let mut changed = false;

        if self.typewriter_mode {
            let target = px(super::focus_modes::typewriter_target_y(
                f32::from(viewport.top()),
                f32::from(viewport.size.height),
            ));
            let delta = target - active_bounds.bottom();
            if f32::from(delta).abs() > 0.5 {
                offset.y += delta;
                changed = true;
            }
        } else if active_bounds.top() < top_limit {
            offset.y += top_limit - active_bounds.top();
            changed = true;
        } else if active_bounds.bottom() > bottom_limit {
            offset.y -= active_bounds.bottom() - bottom_limit;
            changed = true;
        }

        if changed {
            let max_offset_y = self.scroll_handle.max_offset().height.max(px(0.0));
            offset.y = offset.y.min(px(0.0)).max(-max_offset_y);
            self.scroll_handle.set_offset(offset);
        }

        true
    }

    pub(super) fn apply_pending_scroll_into_view(
        &mut self,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        if self.scrollbar_drag.is_some() {
            return;
        }

        if !self.pending_scroll_active_block_into_view {
            return;
        }

        // scroll_to_item indexed children by position, which the spacers break;
        // the focused block is always mounted, so pixel math on its bounds works.
        let has_bounds = self.ensure_focused_caret_visible(window, cx);
        if self.pending_scroll_recheck_after_layout {
            self.pending_scroll_recheck_after_layout = false;
            self.schedule_scroll_recheck(cx);
            return;
        }

        if !has_bounds {
            self.schedule_scroll_recheck(cx);
            return;
        }

        self.pending_scroll_active_block_into_view = false;
        self.scroll_recheck_task = None;
    }

    /// Requests a repaint one frame out so a still-pending scroll-into-view can
    /// retry once the target block has been laid out. `cx.notify()` is swallowed
    /// when called from within `render`, so without this the retry would wait
    /// for the next external notify (e.g. the cursor blink, ~0.5s later).
    pub(super) fn schedule_scroll_recheck(&mut self, cx: &mut Context<Self>) {
        self.scroll_recheck_task = Some(cx.spawn(async move |this: WeakEntity<Self>, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(16))
                .await;
            let _ = this.update(cx, |_this, cx| cx.notify());
        }));
    }

    /// 保存只读取已提交正文；保持 pending 请求直到候选确认为事务或明确取消。
    pub(in crate::editor) fn sync_pending_save(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.pending_save {
            if self.wait_for_ime_completion(window, cx) {
                return;
            }
            self.pending_save = false;
            self.save_document(window, cx);
        }
    }

    /// 另存为不能在组合输入中途抓取快照，取消输入后仍执行用户保存意图。
    pub(in crate::editor) fn sync_pending_save_as(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.pending_save_as {
            if self.wait_for_ime_completion(window, cx) {
                return;
            }
            self.pending_save_as = false;
            self.save_document_as(window, cx);
        }
    }

    pub(super) fn sync_pending_open_link(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(link) = self.pending_open_link.take() else {
            return;
        };

        let strings = cx.global::<I18nManager>().strings_arc();
        let buttons = [
            strings.open_link_open.as_str(),
            strings.open_link_cancel.as_str(),
        ];
        let prompt = window.prompt(
            PromptLevel::Info,
            &strings.open_link_title,
            Some(&link.prompt_target),
            &buttons,
            cx,
        );
        let window_handle = window.window_handle();
        cx.spawn(async move |_this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let Ok(choice) = prompt.await else {
                return;
            };
            if choice == 0 {
                let _ = cx.update_window(window_handle, |_view: AnyView, _window, cx| {
                    cx.open_url(&link.open_target);
                });
            }
        })
        .detach();
    }
}
