// @author kongweiguang

use gpui::*;

use super::Editor;
use crate::editor::render::menu_icon_slot;
use crate::i18n::I18nStrings;
use crate::theme::{Theme, workbench::SurfaceKind};
use crate::ui::visual_preferences::VisualPreferencesManager;

const CLOSE_ICON: &str = "icon/ui/close.svg";

impl Editor {
    /// 组合输入期间保持搜索框挂载；文字直接沿用字段留白，长查询在输入区域内横向滚动。
    pub(in crate::editor) fn render_command_palette_overlay(
        &self,
        theme: &Theme,
        strings: &I18nStrings,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let state = self.command_palette.as_ref()?;
        let c = &theme.colors;
        let visual_preferences = cx
            .try_global::<VisualPreferencesManager>()
            .map(VisualPreferencesManager::current)
            .unwrap_or_default();
        let palette = &c.workbench;
        let overlay_material = palette.material(SurfaceKind::GlassStrong, visual_preferences);
        let input_material = palette.material(SurfaceKind::Solid, visual_preferences);
        let d = &theme.dimensions;
        let t = &theme.typography;
        let editor = cx.entity().downgrade();
        let dismiss_editor = editor.clone();
        let close_editor = editor.clone();
        let close_tooltip: SharedString = strings.ui_close.clone().into();
        let empty_message = if state.input.read(cx).display_text().trim().is_empty() {
            strings.command_palette_prompt.clone()
        } else {
            strings.command_palette_no_results.clone()
        };
        Some(
            div()
                .id("command-palette-overlay")
                .absolute()
                .top_0()
                .left_0()
                .right_0()
                .bottom_0()
                .occlude()
                .flex()
                .justify_center()
                .items_start()
                .pt(px(82.0))
                .bg(palette.overlay_scrim)
                .on_mouse_down(MouseButton::Left, move |_event, window, cx| {
                    let _ = dismiss_editor.update(cx, |editor, cx| {
                        editor.request_command_palette_close(window, cx);
                    });
                })
                .child(
                    div()
                        .id("command-palette-dialog")
                        .debug_selector(|| "command-palette-dialog".to_owned())
                        .w(px(560.0))
                        .max_w(relative(0.92))
                        .max_h(relative(0.74))
                        .flex()
                        .flex_col()
                        .overflow_hidden()
                        .bg(overlay_material.background)
                        .border(px(d.dialog_border_width))
                        .border_color(overlay_material.border)
                        .rounded(px(d.dialog_radius.clamp(22.0, 28.0)))
                        .shadow_lg()
                        .capture_action(cx.listener(|editor, _: &crate::components::DismissTransientUi, window, cx| {
                            if editor.request_command_palette_close(window, cx) { cx.stop_propagation(); }
                        }))
                        .capture_action(cx.listener(|editor, _: &crate::components::Newline, window, cx| {
                            if editor.handle_command_palette_navigation("enter", window, cx) { cx.stop_propagation(); }
                        }))
                        .capture_action(cx.listener(|editor, _: &crate::components::FocusPrev, window, cx| {
                            if editor.handle_command_palette_navigation("up", window, cx) { cx.stop_propagation(); }
                        }))
                        .capture_action(cx.listener(|editor, _: &crate::components::FocusNext, window, cx| {
                            if editor.handle_command_palette_navigation("down", window, cx) { cx.stop_propagation(); }
                        }))
                        .on_mouse_down(MouseButton::Left, |_event, _window, cx| {
                            cx.stop_propagation();
                        })
                        .child(
                            div()
                                .h(px(38.0))
                                .px(px(14.0))
                                .flex()
                                .items_center()
                                .justify_between()
                                .gap(px(12.0))
                                .child(
                                    div()
                                        .min_w(px(0.0))
                                        .overflow_hidden()
                                        .truncate()
                                        .text_size(px(t.dialog_title_size))
                                        .font_weight(t.dialog_title_weight.to_font_weight())
                                        .text_color(palette.text_primary)
                                        .child(strings.command_palette_title.clone()),
                                )
                                .child(
                                    div()
                                        .id("command-palette-close")
                                        .debug_selector(|| "command-palette-close".to_owned())
                                        .size(px(28.0))
                                        .flex_shrink_0()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .rounded(px(5.0))
                                        .cursor_pointer()
                                        .hover(|this| this.bg(palette.control_hover))
                                        .tooltip(move |_window, cx| {
                                            crate::ui::ui_tooltip(close_tooltip.clone(), cx)
                                        })
                                        .child(svg().path(CLOSE_ICON).size(px(15.0)))
                                        .on_click(move |_event, window, cx| {
                                            let _ = close_editor.update(cx, |editor, cx| {
                                                editor.request_command_palette_close(window, cx);
                                            });
                                        }),
                                ),
                        )
                        .child(
                            div()
                                .id("command-palette-input")
                                .debug_selector(|| "command-palette-input".to_owned())
                                .mx(px(12.0))
                                .mb(px(10.0))
                                .min_h(px(40.0))
                                .px(px(10.0))
                                .flex()
                                .items_center()
                                .rounded(px(6.0))
                                .border(px(d.dialog_border_width))
                                .border_color(input_material.border)
                                .bg(input_material.background)
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w(px(0.0))
                                        .overflow_hidden()
                                        .child(state.input.clone()),
                                ),
                        )
                        .child(
                            div()
                                .id("command-palette-results")
                                .debug_selector(|| "command-palette-results".to_owned())
                                .flex_1()
                                .min_h(px(52.0))
                                .overflow_y_scroll()
                                .px(px(8.0))
                                .pb(px(8.0))
                                .children(state.filtered.is_empty().then(|| {
                                    div()
                                        .px(px(10.0))
                                        .py(px(14.0))
                                        .text_size(px(t.dialog_body_size))
                                        .text_color(palette.text_secondary)
                                        .child(empty_message)
                                }))
                                .children(state.filtered.iter().enumerate().filter_map(
                                    |(row, index)| {
                                        let command = state.commands.get(*index)?;
                                        let action = command.action.boxed_clone();
                                        let editor = editor.clone();
                                        Some(
                                            div()
                                                .id(("command-palette-result", row))
                                                .debug_selector(move || {
                                                    format!("command-palette-result-{row}")
                                                })
                                                .min_h(px(50.0))
                                                .w_full()
                                                .px(px(10.0))
                                                .flex()
                                                .items_center()
                                                .gap(px(8.0))
                                                .rounded(px(5.0))
                                                .bg(if row == state.selected {
                                                    palette.selection
                                                } else {
                                                    hsla(0.0, 0.0, 0.0, 0.0)
                                                })
                                                .hover(|this| {
                                                    this.bg(palette.control_hover)
                                                })
                                                .cursor_pointer()
                                                .child(
                                                    menu_icon_slot(
                                                        Some(command.icon),
                                                        palette.icon,
                                                    )
                                                        .debug_selector(move || {
                                                            format!(
                                                                "command-palette-result-icon-{row}"
                                                            )
                                                        }),
                                                )
                                                .child(
                                                    div()
                                                        .min_w(px(0.0))
                                                        .flex_grow()
                                                        .overflow_hidden()
                                                        .flex()
                                                        .flex_col()
                                                        .gap(px(2.0))
                                                        .child(
                                                            div()
                                                                .truncate()
                                                                .debug_selector(move || {
                                                                    format!(
                                                                        "command-palette-result-label-{row}"
                                                                    )
                                                                })
                                                                .text_size(px(t.dialog_body_size))
                                                                .text_color(palette.text_primary)
                                                                .child(command.label.clone()),
                                                        )
                                                        .child(
                                                            div()
                                                                .truncate()
                                                                .debug_selector(move || {
                                                                    format!(
                                                                        "command-palette-result-description-{row}"
                                                                    )
                                                                })
                                                                .text_size(px(
                                                                    t.dialog_body_size * 0.82,
                                                                ))
                                                                .text_color(palette.text_secondary)
                                                                .child(command.description.clone()),
                                                        ),
                                                )
                                                .child(
                                                    div()
                                                        .flex_shrink_0()
                                                        .max_w(px(160.0))
                                                        .overflow_hidden()
                                                        .truncate()
                                                        .text_right()
                                                        .debug_selector(move || {
                                                            format!(
                                                                "command-palette-result-shortcut-{row}"
                                                            )
                                                        })
                                                        .text_size(px(t.dialog_body_size * 0.86))
                                                        .text_color(palette.text_secondary)
                                                        .child(command.shortcut.clone()),
                                                )
                                                .on_click(move |_event, window, cx| {
                                                    let action = action.boxed_clone();
                                                    let _ = editor.update(cx, |editor, _cx| {
                                                        editor.dismiss_command_palette(Some(window));
                                                    });
                                                    window.dispatch_action(action, cx);
                                                }),
                                        )
                                    },
                                )),
                        ),
                )
                .into_any_element(),
        )
    }
}
