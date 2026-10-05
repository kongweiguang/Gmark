// @author kongweiguang

use super::*;

impl Editor {
    /// 同时接收 Esc Action 和裸按键，避免 Action 优先派发时跳过提示关闭与焦点恢复。
    pub(in crate::editor) fn render_tab_close_dialog_overlay(
        &self,
        theme: &crate::theme::Theme,
        strings: &crate::i18n::I18nStrings,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.tabs.show_close_dialog {
            return None;
        }
        let d = &theme.dimensions;
        let focus_ring = vec![BoxShadow {
            color: theme.colors.workbench.focus_ring,
            offset: point(px(0.0), px(0.0)),
            blur_radius: px(0.0),
            spread_radius: px(2.0),
        }];
        Some(
            modal_overlay("tab-close-dialog-overlay", theme)
                .on_action(cx.listener(
                    |editor, _: &crate::components::DismissTransientUi, window, cx| {
                        if editor.tabs.is_close_dialog_open() {
                            editor.cancel_tab_close_dialog(window, cx);
                        }
                    },
                ))
                .on_key_down(cx.listener(Self::on_close_dialog_key_down))
                .child(
                    dialog_panel("tab-close-dialog", d.dialog_width.min(520.0), theme)
                        .child(
                            dialog_content("tab-close-dialog-content", theme)
                                .child(dialog_title_with_icon(
                                    "tab-close-title",
                                    strings.unsaved_changes_title.clone(),
                                    DialogTitleIcon::Warning,
                                    theme,
                                ))
                                .child(dialog_body(strings.unsaved_changes_message.clone(), theme)),
                        )
                        .child(
                            dialog_actions(theme)
                                .child(
                                    dialog_button(
                                        "cancel-tab-close",
                                        strings.unsaved_changes_cancel.clone(),
                                        DialogButtonKind::Secondary,
                                        theme,
                                    )
                                    .tab_index(0)
                                    .track_focus(&self.close_dialog_focus_handles[0])
                                    .focus(|style| style.shadow(focus_ring.clone()))
                                    .on_click(cx.listener(Self::on_cancel_tab_close)),
                                )
                                .child(
                                    dialog_button(
                                        "discard-tab-close",
                                        strings.unsaved_changes_discard_and_close.clone(),
                                        DialogButtonKind::Danger,
                                        theme,
                                    )
                                    .tab_index(1)
                                    .track_focus(&self.close_dialog_focus_handles[1])
                                    .focus(|style| style.shadow(focus_ring.clone()))
                                    .on_click(cx.listener(Self::on_discard_tab_close)),
                                )
                                .child(
                                    dialog_button(
                                        "save-tab-close",
                                        strings.unsaved_changes_save_and_close.clone(),
                                        DialogButtonKind::Primary,
                                        theme,
                                    )
                                    .tab_index(2)
                                    .track_focus(&self.close_dialog_focus_handles[2])
                                    .focus(|style| style.shadow(focus_ring.clone()))
                                    .on_click(cx.listener(Self::on_save_tab_close)),
                                ),
                        ),
                )
                .into_any_element(),
        )
    }

    pub(in crate::editor) fn render_tab_context_menu_overlay(
        &self,
        theme: &crate::theme::Theme,
        strings: &crate::i18n::I18nStrings,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if self.focus_mode {
            return None;
        }
        let menu = self.tabs.context_menu.as_ref()?;
        let index = menu.index;
        let position = menu.position;
        let pinned = self
            .tabs
            .records
            .get(index)
            .is_some_and(|record| record.pinned);
        let can_close_others = self.tabs.records.len() > 1;
        let c = &theme.colors;
        let d = &theme.dimensions;
        let t = &theme.typography;
        let visual_preferences = cx
            .try_global::<crate::ui::visual_preferences::VisualPreferencesManager>()
            .map(crate::ui::visual_preferences::VisualPreferencesManager::current)
            .unwrap_or_default();
        let workbench = &c.workbench;
        let menu_material = workbench.material(
            crate::ui::theme::workbench::SurfaceKind::GlassStrong,
            visual_preferences,
        );
        let panel_width = d.context_menu_submenu_width.max(200.0);
        let panel_origin = clamped_floating_panel_origin(
            position,
            panel_width,
            compact_menu_panel_height(4, 0, d),
            window.viewport_size(),
        );
        let editor = cx.entity().downgrade();
        let dismiss_editor = editor.clone();
        let key_dismiss_editor = editor.clone();
        let pin_editor = editor.clone();
        let close_editor = editor.clone();
        let close_others_editor = editor;
        let item = |id: &'static str,
                    keyboard_index: usize,
                    label: String,
                    icon: &'static str,
                    enabled: bool| {
            div()
                .id(id)
                .debug_selector(move || id.to_owned())
                .h(px(d.menu_item_height))
                .px(px(d.menu_item_padding_x))
                .flex()
                .items_center()
                .gap(px(6.0))
                .rounded(px(d.menu_item_radius))
                .bg(if self.context_menu_keyboard_item == Some(keyboard_index) {
                    workbench.control_hover
                } else {
                    menu_material.background
                })
                .text_size(px(d.menu_text_size))
                .font_weight(t.dialog_body_weight.to_font_weight())
                .text_color(if enabled {
                    workbench.text_primary
                } else {
                    workbench.text_tertiary
                })
                .opacity(if enabled { 1.0 } else { 0.5 })
                .child(
                    menu_icon_slot(Some(icon), workbench.icon)
                        .debug_selector(move || format!("{id}-icon")),
                )
                .on_hover(cx.listener(Self::on_context_menu_pointer_hover))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .overflow_hidden()
                        .truncate()
                        .child(label),
                )
        };

        Some(
            div()
                .id("tab-context-menu-overlay")
                .absolute()
                .top_0()
                .left_0()
                .right_0()
                .bottom_0()
                .occlude()
                .on_mouse_down(MouseButton::Left, move |_event, _window, cx| {
                    let _ = dismiss_editor.update(cx, |editor, cx| {
                        editor.tabs.context_menu = None;
                        cx.notify();
                    });
                })
                .on_key_down(move |event, _window, cx| {
                    if event.keystroke.key == "escape" {
                        let _ = key_dismiss_editor.update(cx, |editor, cx| {
                            editor.dismiss_tab_context_menu();
                            cx.notify();
                        });
                        cx.stop_propagation();
                    }
                })
                .child(
                    div()
                        .id("tab-context-menu")
                        .debug_selector(|| "tab-context-menu".to_owned())
                        .absolute()
                        .left(panel_origin.x)
                        .top(panel_origin.y)
                        .w(px(panel_width))
                        .p(px(d.menu_panel_padding))
                        .flex()
                        .flex_col()
                        .gap(px(d.menu_panel_gap))
                        .bg(menu_material.background)
                        .border(px(d.dialog_border_width))
                        .border_color(menu_material.border)
                        .rounded(px(d.menu_panel_radius))
                        .shadow_lg()
                        .max_h(px((f32::from(window.viewport_size().height)
                            - f32::from(panel_origin.y)
                            - 12.0)
                            .max(d.menu_item_height * 2.0)))
                        .overflow_y_scroll()
                        .on_mouse_down(MouseButton::Left, |_event, _window, cx| {
                            cx.stop_propagation()
                        })
                        .child(
                            item(
                                "tab-context-pin",
                                0,
                                if pinned {
                                    strings.menu_unpin_tab.clone()
                                } else {
                                    strings.menu_pin_tab.clone()
                                },
                                "icon/editor/tab-pin.svg",
                                true,
                            )
                            .hover(|this| this.bg(workbench.control_hover))
                            .cursor_pointer()
                            .on_click(move |_event, _window, cx| {
                                let _ = pin_editor.update(cx, |editor, cx| {
                                    editor.tabs.context_menu = None;
                                    editor.toggle_pin_tab(index, cx);
                                });
                            }),
                        )
                        .child(
                            item(
                                "tab-context-close",
                                1,
                                strings.menu_close_tab.clone(),
                                TAB_CLOSE_ICON,
                                true,
                            )
                            .hover(|this| this.bg(workbench.control_hover))
                            .cursor_pointer()
                            .on_click(move |_event, _window, cx| {
                                let _ = close_editor.update(cx, |editor, cx| {
                                    editor.tabs.context_menu = None;
                                    editor.request_close_tab_index(index, cx);
                                });
                            }),
                        )
                        .child(
                            item(
                                "tab-context-close-others",
                                2,
                                strings.menu_close_other_tabs.clone(),
                                TAB_CLOSE_ICON,
                                can_close_others,
                            )
                            .when(can_close_others, |this| {
                                this.hover(|this| this.bg(workbench.control_hover))
                                    .cursor_pointer()
                                    .on_click(move |_event, _window, cx| {
                                        let _ = close_others_editor.update(cx, |editor, cx| {
                                            editor.tabs.context_menu = None;
                                            editor.request_close_other_tabs(index, cx);
                                        });
                                    })
                            }),
                        ),
                )
                .into_any_element(),
        )
    }
}
