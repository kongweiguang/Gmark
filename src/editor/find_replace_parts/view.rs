// @author kongweiguang

use super::*;
use crate::theme::workbench::SurfaceKind;
use crate::ui::visual_preferences::VisualPreferencesManager;

impl Editor {
    /// Keeps panel controls from detaching an IME owner and handles Escape's action before child Blocks swallow it.
    pub(in crate::editor) fn render_find_panel(
        &self,
        theme: &Theme,
        strings: &I18nStrings,
        top: f32,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let state = self.find_panel.as_ref()?;
        let c = &theme.colors;
        let visual_preferences = cx
            .try_global::<VisualPreferencesManager>()
            .map(VisualPreferencesManager::current)
            .unwrap_or_default();
        let palette = &c.workbench;
        let material = palette.material(SurfaceKind::Glass, visual_preferences);
        let solid_material = palette.material(SurfaceKind::Solid, visual_preferences);
        let d = &theme.dimensions;
        let t = &theme.typography;
        let panel_dismiss_action = cx.listener(
            |editor, _: &crate::components::DismissTransientUi, window, cx| {
                if editor.find_panel.is_none() {
                    return;
                }
                if editor.has_active_ime_composition(cx) {
                    cx.stop_propagation();
                    return;
                }
                editor.close_find_panel(window, cx);
                cx.stop_propagation();
            },
        );
        let count = if let Some(error) = state.error.as_ref() {
            error.clone()
        } else if state.matches.is_empty() {
            strings.find_no_results.clone()
        } else {
            let total = if state.truncated {
                format!("{}+", state.matches.len())
            } else {
                state.matches.len().to_string()
            };
            strings
                .find_match_count_template
                .replace("{current}", &(state.selected + 1).to_string())
                .replace("{total}", &total)
        };
        let editor = cx.entity().downgrade();
        let option_button =
            |id: &'static str,
             label: String,
             icon: &'static str,
             target: FindKeyboardTarget,
             active: bool,
             option: fn(&mut FindOptions) -> &mut bool| {
                let click_editor = editor.clone();
                let hover_editor = editor.clone();
                div()
                    .id(id)
                    .debug_selector(move || id.to_owned())
                    .relative()
                    .h(px(26.0))
                    .min_w(px(26.0))
                    .px(px(5.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(4.0))
                    .border(px(1.0))
                    .border_color(if state.keyboard_target == target {
                        palette.focus_ring
                    } else {
                        hsla(0.0, 0.0, 0.0, 0.0)
                    })
                    .bg(if active {
                        palette.control_hover
                    } else {
                        palette.control_surface
                    })
                    .hover(|this| this.bg(palette.control_hover))
                    .cursor_pointer()
                    .text_color(palette.text_primary)
                    .child(
                        svg()
                            .path(icon)
                            .size(px(15.0))
                            .text_color(if active {
                                palette.accent
                            } else {
                                palette.text_primary
                            })
                            .debug_selector(move || format!("{id}-icon")),
                    )
                    .children(
                        (state.tooltip_visible == Some(id))
                            .then(|| render_find_tooltip(label, theme, visual_preferences)),
                    )
                    .on_hover(move |hovered, _window, cx| {
                        let _ = hover_editor.update(cx, |editor, cx| {
                            editor.set_find_tooltip_hover(id, *hovered, cx);
                        });
                    })
                    .on_click(move |_event, window, cx| {
                        let _ = click_editor.update(cx, |editor, cx| {
                            editor.focus_find_keyboard_target(target, window, cx);
                            editor.toggle_find_option(option, cx);
                        });
                    })
                    .into_any_element()
            };
        let compact_button =
            |id: &'static str,
             label: String,
             handler: fn(&mut Editor, &mut Window, &mut Context<Editor>)| {
                div()
                    .id(id)
                    .h(px(26.0))
                    .px(px(8.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(4.0))
                    .bg(palette.control_surface)
                    .hover(|this| this.bg(palette.control_hover))
                    .cursor_pointer()
                    .text_size(px(t.dialog_button_size))
                    .text_color(palette.text_primary)
                    .on_click(cx.listener(move |editor, _event, window, cx| {
                        handler(editor, window, cx);
                    }))
                    .child(label)
                    .into_any_element()
            };
        let find_row = div()
            .h(px(34.0))
            .flex()
            .items_center()
            .gap(px(4.0))
            .child(
                div()
                    .id("document-find-input")
                    .debug_selector(|| "document-find-input".to_owned())
                    .w(px(210.0))
                    .h(px(30.0))
                    .px(px(7.0))
                    .flex()
                    .items_center()
                    .overflow_hidden()
                    .rounded(px(5.0))
                    .border(px(d.dialog_border_width))
                    .border_color(if state.keyboard_target == FindKeyboardTarget::Query {
                        palette.focus_ring
                    } else {
                        material.border
                    })
                    .bg(solid_material.background)
                    .child(state.query.clone()),
            )
            .child(
                div()
                    .id("document-find-count")
                    .debug_selector(|| "document-find-count".to_owned())
                    .w(px(74.0))
                    .overflow_hidden()
                    .text_ellipsis()
                    .text_size(px(12.0))
                    .text_color(if state.error.is_some() {
                        palette.danger
                    } else {
                        palette.text_secondary
                    })
                    .child(count),
            )
            .child(option_button(
                "document-find-case",
                strings.find_case_sensitive.clone(),
                FIND_CASE_ICON,
                FindKeyboardTarget::CaseSensitive,
                state.options.case_sensitive,
                |options| &mut options.case_sensitive,
            ))
            .child(option_button(
                "document-find-word",
                strings.find_whole_word.clone(),
                FIND_WORD_ICON,
                FindKeyboardTarget::WholeWord,
                state.options.whole_word,
                |options| &mut options.whole_word,
            ))
            .child(option_button(
                "document-find-regex",
                strings.find_regex.clone(),
                FIND_REGEX_ICON,
                FindKeyboardTarget::Regex,
                state.options.regex,
                |options| &mut options.regex,
            ))
            .child(
                div()
                    .id("document-find-previous")
                    .size(px(26.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(4.0))
                    .bg(palette.control_surface)
                    .text_color(palette.text_primary)
                    .border(px(1.0))
                    .border_color(if state.keyboard_target == FindKeyboardTarget::Previous {
                        palette.focus_ring
                    } else {
                        hsla(0.0, 0.0, 0.0, 0.0)
                    })
                    .hover(|this| this.bg(palette.control_hover))
                    .cursor_pointer()
                    .on_click(cx.listener(|editor, _event, window, cx| {
                        editor.navigate_find_match(-1, window, cx);
                        editor.focus_find_keyboard_target(FindKeyboardTarget::Previous, window, cx);
                    }))
                    .child(
                        svg()
                            .path(CHEVRON_UP_ICON)
                            .size(px(15.0))
                            .text_color(palette.text_primary)
                            .debug_selector(|| "document-find-previous-icon".to_owned()),
                    ),
            )
            .child(
                div()
                    .id("document-find-next")
                    .size(px(26.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(4.0))
                    .bg(palette.control_surface)
                    .text_color(palette.text_primary)
                    .border(px(1.0))
                    .border_color(if state.keyboard_target == FindKeyboardTarget::Next {
                        palette.focus_ring
                    } else {
                        hsla(0.0, 0.0, 0.0, 0.0)
                    })
                    .hover(|this| this.bg(palette.control_hover))
                    .cursor_pointer()
                    .on_click(cx.listener(|editor, _event, window, cx| {
                        editor.navigate_find_match(1, window, cx);
                        editor.focus_find_keyboard_target(FindKeyboardTarget::Next, window, cx);
                    }))
                    .child(
                        svg()
                            .path(CHEVRON_DOWN_ICON)
                            .size(px(15.0))
                            .text_color(palette.text_primary)
                            .debug_selector(|| "document-find-next-icon".to_owned()),
                    ),
            )
            .child(
                div()
                    .id("document-find-close")
                    .size(px(26.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(4.0))
                    .bg(palette.control_surface)
                    .text_color(palette.text_primary)
                    .border(px(1.0))
                    .border_color(if state.keyboard_target == FindKeyboardTarget::Close {
                        palette.focus_ring
                    } else {
                        hsla(0.0, 0.0, 0.0, 0.0)
                    })
                    .hover(|this| this.bg(palette.control_hover))
                    .cursor_pointer()
                    .on_click(cx.listener(|editor, _event, window, cx| {
                        editor.close_find_panel(window, cx);
                    }))
                    .child(
                        svg()
                            .path(CLOSE_ICON)
                            .size(px(15.0))
                            .text_color(palette.text_primary)
                            .debug_selector(|| "document-find-close-icon".to_owned()),
                    ),
            )
            .into_any_element();
        let replace_row = state.show_replace.then(|| {
            div()
                .h(px(34.0))
                .flex()
                .items_center()
                .gap(px(5.0))
                .child(
                    div()
                        .id("document-replace-input")
                        .debug_selector(|| "document-replace-input".to_owned())
                        .w(px(288.0))
                        .h(px(30.0))
                        .px(px(7.0))
                        .flex()
                        .items_center()
                        .overflow_hidden()
                        .rounded(px(5.0))
                        .border(px(d.dialog_border_width))
                        .border_color(
                            if state.keyboard_target == FindKeyboardTarget::Replacement {
                                palette.focus_ring
                            } else {
                                material.border
                            },
                        )
                        .bg(solid_material.background)
                        .child(state.replacement.clone()),
                )
                .child(compact_button(
                    "document-replace-current",
                    strings.find_replace.clone(),
                    Editor::replace_current_find_match,
                ))
                .child(compact_button(
                    "document-replace-all",
                    strings.find_replace_all.clone(),
                    Editor::replace_all_find_matches,
                ))
                .into_any_element()
        });
        Some(
            div()
                .id("document-find-panel")
                .debug_selector(|| "document-find-panel".to_owned())
                .absolute()
                .top(px(top + 8.0))
                .right(px(12.0))
                .w(px(540.0))
                .max_w(relative(0.94))
                .track_focus(&state.focus_handle)
                .p(px(6.0))
                .flex()
                .flex_col()
                .gap(px(2.0))
                .occlude()
                .bg(material.background)
                .border(px(d.dialog_border_width))
                .border_color(material.border)
                .rounded(px(14.0))
                .shadow_lg()
                .on_mouse_down(MouseButton::Left, |_event, _window, cx| {
                    cx.stop_propagation();
                })
                .on_action(panel_dismiss_action)
                .child(find_row)
                .children(replace_row)
                .into_any_element(),
        )
    }
}
