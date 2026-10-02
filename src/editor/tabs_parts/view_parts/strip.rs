// @author kongweiguang

use super::*;

impl Editor {
    /// Renders stable tab chrome; toolbar focus is deferred before pointer focus can displace an IME owner.
    pub(in crate::editor) fn render_tab_strip(
        &mut self,
        theme: &crate::theme::Theme,
        top: f32,
        left: f32,
        right: f32,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let height = self.tab_strip_height();
        if height == 0.0 {
            // GPUI may retain the previous absolute tab-strip paint node for
            // one frame when an optional child disappears during pane
            // promotion. Replace it with a zero-height element carrying the
            // same identity so the old chrome is invalidated immediately,
            // without leaving any clickable global-tab descendants.
            return Some(
                div()
                    .id("document-tab-strip")
                    .debug_selector(|| "document-tab-strip-cleared".to_owned())
                    .absolute()
                    .left(px(left))
                    .right(px(right))
                    .top(px(top))
                    .h(px(0.0))
                    .overflow_hidden()
                    .into_any_element(),
            );
        }
        let c = &theme.colors;
        let visual_preferences = cx
            .try_global::<crate::ui::visual_preferences::VisualPreferencesManager>()
            .map(crate::ui::visual_preferences::VisualPreferencesManager::current)
            .unwrap_or_default();
        let workbench = &c.workbench;
        let strip_material = workbench.material(
            crate::ui::theme::workbench::SurfaceKind::Glass,
            visual_preferences,
        );
        let tab_strip_background = c.tab_strip_background;
        let show_tab_bar_actions = EditorSettings::show_tab_bar_actions(cx);
        let i18n = cx.global::<crate::i18n::I18nManager>();
        let chinese = i18n.current_language_id().starts_with("zh");
        let strings = i18n.strings().clone();
        let split_pane_tooltip: SharedString = if chinese {
            "拆分窗格".into()
        } else {
            "Split Pane".into()
        };
        let close_tab_tooltip = strings.menu_close_tab.clone();
        let new_tab_tooltip = strings.menu_new_tab.clone();
        let tab_drop_background = workbench.accent_soft;
        let editor = cx.entity().downgrade();
        let strip_release_editor = editor.clone();
        let strip_detach_editor = editor.clone();
        let new_tab_editor = editor.clone();
        let new_tab_key_editor = editor.clone();
        let (tab_focus_handles, new_tab_focus_handle) = self.ensure_tab_strip_focus_handles(cx);
        let toolbar_button = |action: DocumentToolbarAction,
                              icon: &'static str,
                              tooltip: SharedString,
                              active: bool| {
            let focus_handle = self.document_toolbar_focus_handles[action.index()].clone();
            let pointer_focus_handle = focus_handle.clone();
            let click_editor = editor.clone();
            let key_editor = editor.clone();
            let icon_color = if active {
                workbench.text_primary
            } else {
                workbench.text_secondary
            };
            div()
                .id(("document-toolbar-action", action.index()))
                .debug_selector(move || format!("document-toolbar-action-{}", action.index()))
                .size(px(TAB_TOOL_BUTTON_SIZE))
                .tab_index(0)
                .track_focus(&focus_handle)
                .flex_shrink_0()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(6.0))
                .border(px(1.0))
                .border_color(if active {
                    workbench.border_strong
                } else {
                    transparent_color(workbench.border_subtle)
                })
                .bg(if active {
                    workbench.control_hover
                } else {
                    transparent_color(workbench.control_surface)
                })
                .hover(|this| {
                    this.bg(workbench.control_hover)
                        .text_color(workbench.text_primary)
                })
                .focus(|this| this.border_color(workbench.focus_ring))
                .cursor_pointer()
                .tooltip(move |_window, cx| crate::ui::ui_tooltip(tooltip.clone(), cx))
                .child(svg().path(icon).size(px(15.0)).text_color(icon_color))
                .on_click(move |event, window, cx| {
                    let deferred = click_editor
                        .update(cx, |editor, cx| {
                            editor.defer_document_toolbar_action_for_ime(action, window, cx)
                        })
                        .unwrap_or(false);
                    if !deferred {
                        pointer_focus_handle.focus(window);
                    }
                    if deferred {
                        cx.stop_propagation();
                        return;
                    }
                    let _ = click_editor.update(cx, |editor, cx| {
                        editor.activate_document_toolbar_action(
                            action,
                            Some(event.position()),
                            window,
                            cx,
                        );
                    });
                    cx.stop_propagation();
                })
                .on_key_down(move |event, window, cx| {
                    if event.keystroke.key == "escape" {
                        let dismissed = key_editor
                            .update(cx, |editor, cx| {
                                let dismissed = editor.tabs.dismiss_new_or_split_menu();
                                if dismissed {
                                    cx.notify();
                                }
                                dismissed
                            })
                            .unwrap_or(false);
                        if dismissed {
                            cx.stop_propagation();
                        }
                    } else if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                        let _ = key_editor.update(cx, |editor, cx| {
                            editor.activate_document_toolbar_action(action, None, window, cx);
                        });
                        cx.stop_propagation();
                    }
                })
                .into_any_element()
        };
        let new_tab_button = div()
            .id("document-new-tab")
            .debug_selector(|| "document-new-tab".to_owned())
            .size(px(TAB_TOOL_BUTTON_SIZE))
            .tab_index(0)
            .track_focus(&new_tab_focus_handle)
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(6.0))
            .text_color(workbench.icon)
            .hover(|this| {
                this.bg(workbench.control_hover)
                    .text_color(workbench.text_primary)
            })
            .focus(|this| {
                this.bg(workbench.control_hover)
                    .text_color(workbench.text_primary)
            })
            .cursor_pointer()
            .tooltip(move |_window, cx| crate::ui::ui_tooltip(new_tab_tooltip.clone(), cx))
            .child(
                svg()
                    .path(NEW_TAB_ICON)
                    .size(px(14.0))
                    .text_color(workbench.icon),
            )
            .on_click(move |event, _window, cx| {
                let _ = new_tab_editor.update(cx, |editor, cx| {
                    editor.tabs.context_menu = None;
                    editor.tabs.new_tab_menu = Some(NewTabMenu {
                        position: event.position(),
                        pane: None,
                    });
                    cx.notify();
                });
                cx.stop_propagation();
            })
            .on_key_down(move |event, _window, cx| {
                let _ = new_tab_key_editor.update(cx, |editor, cx| {
                    editor.on_new_tab_key_down(event, cx);
                });
            });
        Some(
            div()
                .id("document-tab-strip")
                .debug_selector(|| "document-tab-strip".to_owned())
                .absolute()
                .left(px(left))
                .right(px(right))
                .top(px(top))
                .h(px(height))
                .flex()
                .items_center()
                .overflow_hidden()
                .bg(tab_strip_background)
                .on_mouse_up(MouseButton::Left, move |_event, _window, cx| {
                    let _ = strip_release_editor.update(cx, |editor, _cx| {
                        editor.tabs.dragging_tab = None;
                    });
                })
                .on_mouse_up_out(MouseButton::Left, move |event, window, cx| {
                    let viewport = window.viewport_size();
                    let outside_window = event.position.x < px(0.0)
                        || event.position.y < px(0.0)
                        || event.position.x >= viewport.width
                        || event.position.y >= viewport.height;
                    let detached = strip_detach_editor
                        .update(cx, |editor, cx| {
                            let id = editor.tabs.dragging_tab.take();
                            if outside_window {
                                id.and_then(|id| editor.detach_tab_by_id(id, cx))
                            } else {
                                None
                            }
                        })
                        .ok()
                        .flatten();
                    if let Some(detached) = detached {
                        let rollback = detached.clone();
                        if let Err(error) = crate::app_menu::open_detached_tab_window(cx, detached)
                        {
                            eprintln!("failed to detach tab: {error}");
                            let _ = strip_detach_editor.update(cx, |editor, cx| {
                                editor.reattach_detached_tab(rollback, cx);
                            });
                        }
                    }
                })
                .child(
                    div()
                        .id("document-tab-scroll")
                        .debug_selector(|| "document-tab-scroll".to_owned())
                        .h_full()
                        .flex_1()
                        .min_w(px(0.0))
                        .flex()
                        .items_center()
                        .overflow_x_scroll()
                        .children(self.tabs.records.iter().enumerate().map(|(index, record)| {
                            let active = index == self.tabs.active;
                            let separates_inactive_tabs =
                                index > 0 && !active && self.tabs.active != index.saturating_sub(1);
                            let (path, dirty, document_kind, image_preview) = if active {
                                (
                                    self.file_path.as_deref(),
                                    self.is_document_dirty(),
                                    self.document_kind,
                                    self.image_preview_path.is_some(),
                                )
                            } else {
                                record
                                    .snapshot
                                    .as_ref()
                                    .map(|snapshot| {
                                        (
                                            snapshot.file_path.as_deref(),
                                            snapshot.document_dirty,
                                            snapshot.document_kind,
                                            snapshot.image_preview_path.is_some(),
                                        )
                                    })
                                    .unwrap_or((None, false, DocumentKind::Markdown, false))
                            };
                            let title = path
                                .and_then(Path::file_name)
                                .map(|name| name.to_string_lossy().into_owned())
                                .unwrap_or_else(|| document_kind.untitled_name().to_owned());
                            let display_title = middle_ellipsis(&title, 28);
                            let leading_icon = if record.pinned {
                                TAB_PIN_ICON
                            } else if image_preview {
                                TAB_IMAGE_ICON
                            } else {
                                document_kind.icon()
                            };
                            let title_tooltip: SharedString = title.clone().into();
                            let close_tooltip: SharedString = close_tab_tooltip.clone().into();
                            let switch_editor = editor.clone();
                            let close_editor = editor.clone();
                            let context_editor = editor.clone();
                            let drop_editor = editor.clone();
                            let drag_editor = editor.clone();
                            let key_editor = editor.clone();
                            let focus_handle = tab_focus_handles[index].clone();
                            let tab_id = record.id.as_u64_pair().0;
                            let group = SharedString::from(format!("document-tab-group-{tab_id}"));
                            let drag_payload = TabDragPayload {
                                id: record.id,
                                title: title.clone(),
                                background: tab_strip_background,
                                text: workbench.text_primary,
                            };
                            div()
                                .id(("document-tab", tab_id))
                                .group(group.clone())
                                .debug_selector(move || format!("document-tab-{index}"))
                                .min_w(px(TAB_MIN_WIDTH))
                                .max_w(px(TAB_MAX_WIDTH))
                                .h(px(30.0))
                                .when(active, |this| this.h_full())
                                .mx(px(2.0))
                                .tab_index(0)
                                .track_focus(&focus_handle)
                                .px(px(10.0))
                                .flex()
                                .items_center()
                                .gap(px(7.0))
                                .relative()
                                .bg(if active {
                                    workbench.editor_surface
                                } else {
                                    tab_strip_background
                                })
                                .when(separates_inactive_tabs, |this| {
                                    this.child(
                                        crate::editor::tabs::terminal_inactive_tab_separator(
                                            workbench.border_subtle,
                                            -2.0,
                                            30.0,
                                            format!("document-tab-inactive-separator-{index}"),
                                        ),
                                    )
                                })
                                .when(active, |this| {
                                    this.rounded_t(px(
                                        crate::editor::tabs::TERMINAL_TAB_SHOULDER_RADIUS,
                                    ))
                                    // 凹肩先于内容绘制，确保它只参与轮廓合成，不覆盖文字与交互控件。
                                    .child(crate::editor::tabs::terminal_tab_shoulder_cutout(
                                        workbench.editor_surface,
                                        tab_strip_background,
                                        true,
                                        format!("document-tab-active-bottom-curve-left-{index}"),
                                    ))
                                    .child(
                                        crate::editor::tabs::terminal_tab_shoulder_cutout(
                                            workbench.editor_surface,
                                            tab_strip_background,
                                            false,
                                            format!(
                                                "document-tab-active-bottom-curve-right-{index}"
                                            ),
                                        ),
                                    )
                                })
                                .hover(|this| {
                                    this.bg(if active {
                                        workbench.editor_surface
                                    } else {
                                        workbench.control_hover
                                    })
                                })
                                .focus(|this| {
                                    this.bg(if active {
                                        workbench.editor_surface
                                    } else {
                                        workbench.control_hover
                                    })
                                })
                                // 选中 Tab 用编辑器表面铺满自身并向两侧外扩
                                // 8px 肩部，圆形栏色切口负责把活动面连续接回正文。
                                .cursor_pointer()
                                .tooltip(move |_window, cx| {
                                    crate::ui::ui_tooltip(title_tooltip.clone(), cx)
                                })
                                .child(
                                    div()
                                        .size(px(16.0))
                                        .flex_shrink_0()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .debug_selector(move || {
                                            format!("document-tab-leading-{index}")
                                        })
                                        .child(svg().path(leading_icon).size(px(13.0)).text_color(
                                            if record.pinned {
                                                workbench.accent
                                            } else {
                                                workbench.icon
                                            },
                                        )),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w(px(0.0))
                                        .overflow_hidden()
                                        .truncate()
                                        .debug_selector(move || {
                                            format!("document-tab-title-{index}")
                                        })
                                        .text_size(px(theme.typography.text_size * 0.88))
                                        .text_color(if active {
                                            workbench.text_primary
                                        } else {
                                            workbench.text_secondary
                                        })
                                        .child(display_title),
                                )
                                .child(
                                    div()
                                        .id(("document-tab-close", tab_id))
                                        .debug_selector(move || {
                                            format!("document-tab-close-{index}")
                                        })
                                        .relative()
                                        .size(px(18.0))
                                        .flex_shrink_0()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .rounded(px(4.0))
                                        .hover(|this| this.bg(workbench.control_hover))
                                        .cursor_pointer()
                                        .tooltip(move |_window, cx| {
                                            crate::ui::ui_tooltip(close_tooltip.clone(), cx)
                                        })
                                        .child(
                                            div()
                                                .absolute()
                                                .size(px(6.0))
                                                .rounded_full()
                                                .bg(workbench.accent)
                                                .debug_selector(move || {
                                                    format!("document-tab-dirty-{index}")
                                                })
                                                .opacity(if dirty { 1.0 } else { 0.0 })
                                                .group_hover(group.clone(), |this| {
                                                    this.opacity(0.0)
                                                }),
                                        )
                                        .child(
                                            svg()
                                                .absolute()
                                                .path(TAB_CLOSE_ICON)
                                                .size(px(13.0))
                                                .debug_selector(move || {
                                                    format!("document-tab-close-icon-{index}")
                                                })
                                                .text_color(workbench.icon)
                                                .opacity(if active && !dirty { 1.0 } else { 0.0 })
                                                .group_hover(group, |this| this.opacity(1.0)),
                                        )
                                        .on_click(move |_event, _window, cx| {
                                            let _ = close_editor.update(cx, |editor, cx| {
                                                editor.request_close_tab_index(index, cx);
                                            });
                                            cx.stop_propagation();
                                        }),
                                )
                                .on_click(move |_event, _window, cx| {
                                    let _ = switch_editor.update(cx, |editor, cx| {
                                        editor.switch_to_tab_index(index, cx);
                                    });
                                })
                                .on_key_down(move |event, window, cx| {
                                    let _ = key_editor.update(cx, |editor, cx| {
                                        editor.on_tab_strip_key_down(index, event, window, cx);
                                    });
                                })
                                .on_mouse_down(MouseButton::Right, move |event, _window, cx| {
                                    let _ = context_editor.update(cx, |editor, cx| {
                                        editor.tabs.context_menu = Some(TabContextMenu {
                                            index,
                                            position: event.position,
                                        });
                                        editor.context_menu_keyboard_item = None;
                                        editor.context_menu_keyboard_submenu_item = None;
                                        editor
                                            .context_menu_scroll_handle
                                            .set_offset(point(px(0.0), px(0.0)));
                                        cx.notify();
                                    });
                                    cx.stop_propagation();
                                })
                                .on_drag(drag_payload, move |payload, position, _, cx| {
                                    let id = payload.id;
                                    let _ = drag_editor.update(cx, |editor, _cx| {
                                        editor.tabs.dragging_tab = Some(id);
                                    });
                                    cx.new(|_| TabDragPreview {
                                        payload: payload.clone(),
                                        position,
                                    })
                                })
                                .drag_over::<TabDragPayload>(move |style, _, _, _| {
                                    style.bg(tab_drop_background)
                                })
                                .on_drop(move |payload: &TabDragPayload, _window, cx| {
                                    let _ = drop_editor.update(cx, |editor, cx| {
                                        editor.tabs.dragging_tab = None;
                                        if let Some(source) = editor
                                            .tabs
                                            .records
                                            .iter()
                                            .position(|record| record.id == payload.id)
                                        {
                                            editor.reorder_tab(source, index, cx);
                                        }
                                    });
                                })
                        })),
                )
                .child(
                    div()
                        .id("document-tab-trailing-tools")
                        .debug_selector(|| "document-tab-trailing-tools".to_owned())
                        .h_full()
                        .px(px(TAB_TOOL_GROUP_PADDING))
                        .gap(px(2.0))
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .bg(tab_strip_background)
                        .border_l(px(theme.dimensions.dialog_border_width))
                        .border_color(strip_material.border)
                        .child(new_tab_button)
                        .child(toolbar_button(
                            DocumentToolbarAction::SplitPane,
                            "icon/ui/split.svg",
                            split_pane_tooltip,
                            self.tabs.split_pane_menu.is_some(),
                        ))
                        .children(show_tab_bar_actions.then(|| {
                            toolbar_button(
                                DocumentToolbarAction::QuickOpen,
                                QUICK_OPEN_ICON,
                                strings.quick_open_title.clone().into(),
                                false,
                            )
                        }))
                        .children(show_tab_bar_actions.then(|| {
                            toolbar_button(
                                DocumentToolbarAction::Find,
                                FIND_ICON,
                                strings.preferences_shortcut_find_in_document.clone().into(),
                                self.find_panel.is_some(),
                            )
                        }))
                        .children(show_tab_bar_actions.then(|| {
                            toolbar_button(
                                DocumentToolbarAction::CommandPalette,
                                COMMAND_PALETTE_ICON,
                                strings.command_palette_title.clone().into(),
                                self.command_palette.is_some(),
                            )
                        })),
                )
                .into_any_element(),
        )
    }
}
