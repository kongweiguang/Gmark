// @author kongweiguang

use super::*;

impl Editor {
    pub(in crate::editor) fn on_editor_surface_mouse_down(
        &mut self,
        _event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let had_menu = self.menu_bar_open.is_some();
        if had_menu {
            self.dismiss_menu_bar_from_body(cx);
        }
        let had_contextual_editing = self.dismiss_active_contextual_editing_popovers(cx);
        if had_menu || had_contextual_editing {
            cx.notify();
        }
    }

    pub(super) fn schedule_context_menu_submenu_close(&mut self, cx: &mut Context<Self>) {
        if !matches!(self.context_menu, Some(ContextMenuState::Insert { .. })) {
            return;
        }

        let weak_editor = cx.entity().downgrade();
        self.context_menu_submenu_close_task = Some(cx.spawn(
            async move |_this: WeakEntity<Self>, cx: &mut AsyncApp| {
                cx.background_executor()
                    .timer(Duration::from_millis(120))
                    .await;
                let _ = weak_editor.update(cx, |editor, cx| {
                    editor.context_menu_submenu_close_task = None;
                    let Some(ContextMenuState::Insert {
                        insert_hovered,
                        submenu_hovered,
                        submenu_open,
                        ..
                    }) = editor.context_menu.as_mut()
                    else {
                        return;
                    };
                    if !*insert_hovered && !*submenu_hovered && *submenu_open {
                        *submenu_open = false;
                        cx.notify();
                    }
                });
            },
        ));
    }

    pub(in crate::editor) fn set_context_menu_hover_state(
        &mut self,
        hovered: bool,
        submenu: bool,
        cx: &mut Context<Self>,
    ) {
        let mut changed = false;
        let mut should_clear_close = false;
        let mut should_schedule_close = false;

        if let Some(ContextMenuState::Insert {
            insert_hovered,
            submenu_hovered,
            submenu_open,
            ..
        }) = self.context_menu.as_mut()
        {
            if submenu {
                if *submenu_hovered != hovered {
                    *submenu_hovered = hovered;
                    changed = true;
                }
            } else if *insert_hovered != hovered {
                *insert_hovered = hovered;
                changed = true;
            }

            if hovered {
                should_clear_close = true;
                if !*submenu_open {
                    *submenu_open = true;
                    changed = true;
                }
            } else {
                let insert_still_hovered = *insert_hovered;
                let submenu_still_hovered = *submenu_hovered;
                if !insert_still_hovered && !submenu_still_hovered {
                    should_schedule_close = true;
                }
            }
        }

        if should_clear_close {
            self.context_menu_submenu_close_task = None;
        }
        if should_schedule_close {
            self.schedule_context_menu_submenu_close(cx);
        }
        if changed {
            cx.notify();
        }
    }

    pub(in crate::editor) fn on_editor_context_menu_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.view_mode != ViewMode::Rendered {
            return;
        }
        cx.stop_propagation();
        self.open_insert_context_menu(event.position, TableInsertTarget::Append, cx);
    }

    pub(in crate::editor) fn on_block_context_menu_mouse_down(
        &mut self,
        entity_id: EntityId,
        event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.view_mode != ViewMode::Rendered {
            return;
        }
        cx.stop_propagation();
        if let Some(block) = self.focusable_entity_by_id(entity_id) {
            if block.read(cx).record.resource.is_some() {
                self.close_menu_bar(cx);
                self.context_menu = Some(ContextMenuState::Resource {
                    position: event.position,
                    entity_id,
                });
                self.context_menu_keyboard_item = None;
                self.context_menu_keyboard_submenu_item = None;
                self.context_menu_scroll_handle
                    .set_offset(point(px(0.0), px(0.0)));
                cx.notify();
                return;
            }
            let offset = block.read(cx).index_for_mouse_position(event.position);
            let diagnostic = block
                .read(cx)
                .spelling_diagnostics
                .iter()
                .find(|diagnostic| diagnostic.range.contains(&offset))
                .cloned();
            if let Some(diagnostic) = diagnostic {
                self.close_menu_bar(cx);
                self.context_menu = Some(ContextMenuState::Spelling {
                    position: event.position,
                    entity_id,
                    diagnostic,
                });
                self.context_menu_keyboard_item = None;
                self.context_menu_keyboard_submenu_item = None;
                self.context_menu_scroll_handle
                    .set_offset(point(px(0.0), px(0.0)));
                cx.notify();
                return;
            }
        }
        // 单元格内部保留表格自身的上下文；其余根块都允许在后方插入，
        // 与块操作菜单的插入能力保持一致。
        if self.table_cell_binding(entity_id).is_some() {
            return;
        }
        let target = TableInsertTarget::After(self.root_ancestor_entity_id(entity_id));
        self.open_insert_context_menu(event.position, target, cx);
    }

    pub(in crate::editor) fn on_dismiss_context_menu_overlay(
        &mut self,
        _event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let restore_focus = self
            .active_entity_id
            .and_then(|entity_id| self.focusable_entity_by_id(entity_id));
        self.dismiss_contextual_overlays(cx);
        if let Some(block) = restore_focus {
            block.read(cx).focus_handle.focus(window);
        }
    }

    /// 子窗格先关闭自己的浮层，再让窗口壳关闭全局菜单；所属冲突提示独占取消操作。
    pub(in crate::editor) fn on_dismiss_transient_ui(
        &mut self,
        _: &DismissTransientUi,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.show_external_conflict_dialog {
            self.cancel_external_conflict(cx);
            return;
        }
        if let Some(document_host) = self.document_host.clone() {
            document_host.update(cx, |view, cx| {
                view.on_dismiss_transient_ui(&DismissTransientUi, window, cx);
            });
        }
        self.close_menu_bar(cx);
        self.dismiss_contextual_overlays(cx);
        if self.pane_canvas {
            cx.propagate();
        }
    }

    pub(in crate::editor) fn on_context_menu_insert_hover(
        &mut self,
        hovered: &bool,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if *hovered {
            self.clear_context_menu_keyboard_cursor(cx);
        }
        self.set_context_menu_hover_state(*hovered, false, cx);
    }

    pub(in crate::editor) fn on_context_menu_submenu_hover(
        &mut self,
        hovered: &bool,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if *hovered {
            self.clear_context_menu_keyboard_cursor(cx);
        }
        self.set_context_menu_hover_state(*hovered, true, cx);
    }

    pub(in crate::editor) fn on_context_menu_pointer_hover(
        &mut self,
        hovered: &bool,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if *hovered {
            self.clear_context_menu_keyboard_cursor(cx);
        }
    }

    pub(in crate::editor) fn apply_spelling_suggestion(
        &mut self,
        suggestion_index: usize,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(ContextMenuState::Spelling {
            entity_id,
            diagnostic,
            ..
        }) = self.context_menu.take()
        else {
            return;
        };
        self.context_menu_keyboard_item = None;
        self.context_menu_keyboard_submenu_item = None;
        let Some(replacement) = diagnostic.replacements.get(suggestion_index).cloned() else {
            cx.notify();
            return;
        };
        let Some(block) = self.focusable_entity_by_id(entity_id) else {
            cx.notify();
            return;
        };
        block.update(cx, move |block, cx| {
            if diagnostic.range.end > block.display_text().len()
                || !block
                    .display_text()
                    .is_char_boundary(diagnostic.range.start)
                || !block.display_text().is_char_boundary(diagnostic.range.end)
                || block.display_text()[diagnostic.range.clone()] != diagnostic.original
            {
                return;
            }
            block.prepare_undo_capture(UndoCaptureKind::NonCoalescible, cx);
            block.replace_text_in_visible_range(diagnostic.range, &replacement, None, false, cx);
        });
        cx.notify();
    }
}
