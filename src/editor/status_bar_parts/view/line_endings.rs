// @author kongweiguang

use gmark_document::LineEnding;

use super::*;

/// 打开菜单前保存输入焦点，格式选择不应让后续输入或保存落在已经移除的菜单项。
pub(in crate::editor) fn render_line_ending_picker(
    state: &mut StatusBarState,
    current_label: String,
    theme: &Theme,
    cx: &mut Context<Editor>,
) -> AnyElement {
    let open = state.line_ending_menu_open;
    let button_focus_handle = state
        .line_ending_button_focus_handle
        .get_or_insert_with(|| cx.focus_handle())
        .clone();
    if state.line_ending_focus_handles.is_none() {
        state.line_ending_focus_handles = Some(std::array::from_fn(|_| cx.focus_handle()));
    }
    let focus_handles = state
        .line_ending_focus_handles
        .as_ref()
        .expect("line-ending focus handles must be initialized")
        .clone();
    let first_item_focus = focus_handles[0].clone();
    let pointer_focus_handle = button_focus_handle.clone();
    let menu_items = [
        ("lf", "LF", LineEnding::Lf),
        ("crlf", "CRLF", LineEnding::CrLf),
        ("cr", "CR", LineEnding::Cr),
    ]
    .into_iter()
    .enumerate()
    .map(|(index, (id, label, ending))| {
        render_line_ending_menu_item(
            id,
            label,
            ending,
            current_label == label,
            index,
            focus_handles.clone(),
            button_focus_handle.clone(),
            theme,
            cx,
        )
    })
    .collect::<Vec<_>>();

    div()
        .id("status-bar-line-ending-picker")
        .debug_selector(|| "status-bar-line-ending-picker".to_owned())
        .relative()
        .h(px(theme.dimensions.status_bar_height))
        .flex()
        .items_center()
        .child(
            div()
                .id("status-bar-line-ending-button")
                .debug_selector(|| "status-bar-line-ending-button".to_owned())
                .h(px(theme.dimensions.status_bar_height))
                .min_w(px(30.0))
                .px(px(5.0))
                .tab_index(0)
                .track_focus(&button_focus_handle)
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(4.0))
                .border(px(1.0))
                .border_color(hsla(0.0, 0.0, 0.0, 0.0))
                .bg(if open {
                    theme.colors.workbench.control_hover
                } else {
                    hsla(0.0, 0.0, 0.0, 0.0)
                })
                .hover(|this| this.bg(theme.colors.workbench.control_hover))
                .focus(|this| this.border_color(theme.colors.workbench.focus_ring))
                .cursor_pointer()
                .text_size(px(theme.dimensions.status_bar_text_size))
                .text_color(theme.colors.workbench.text_secondary)
                .child(current_label)
                .on_click(cx.listener(move |editor, _: &ClickEvent, window, cx| {
                    if !editor.status_bar.line_ending_menu_open {
                        let owner = editor
                            .focused_pane_entities(cx)
                            .0
                            .map_or(cx.entity_id(), |entity| entity.entity_id());
                        editor.status_bar.line_ending_restore_focus = window
                            .focused(cx)
                            .filter(|_| !pointer_focus_handle.is_focused(window))
                            .map(|focus| (owner, focus));
                    }
                    pointer_focus_handle.focus(window);
                    editor.status_bar.format_overflow_open = false;
                    editor.status_bar.mode_menu_open = false;
                    editor.status_bar.line_ending_menu_open =
                        !editor.status_bar.line_ending_menu_open;
                    if !editor.status_bar.line_ending_menu_open {
                        editor.status_bar.line_ending_restore_focus = None;
                    }
                    cx.notify();
                }))
                .on_key_down(
                    cx.listener(move |editor, event: &KeyDownEvent, window, cx| {
                        match event.keystroke.key.as_str() {
                            "enter" | "space" => {
                                editor.status_bar.line_ending_restore_focus = None;
                                editor.status_bar.format_overflow_open = false;
                                editor.status_bar.mode_menu_open = false;
                                editor.status_bar.line_ending_menu_open =
                                    !editor.status_bar.line_ending_menu_open;
                                if editor.status_bar.line_ending_menu_open {
                                    first_item_focus.focus(window);
                                }
                                cx.notify();
                                cx.stop_propagation();
                            }
                            "escape" if editor.status_bar.line_ending_menu_open => {
                                editor.status_bar.line_ending_menu_open = false;
                                cx.notify();
                                cx.stop_propagation();
                            }
                            _ => {}
                        }
                    }),
                ),
        )
        .children(open.then(|| {
            div()
                .id("status-bar-line-ending-menu")
                .debug_selector(|| "status-bar-line-ending-menu".to_owned())
                .absolute()
                .right(px(0.0))
                .bottom(px(theme.dimensions.status_bar_height + 4.0))
                .w(px(92.0))
                .occlude()
                .p(px(4.0))
                .flex()
                .flex_col()
                .gap(px(2.0))
                .bg(theme.colors.workbench.elevated_surface)
                .border(px(theme.dimensions.dialog_border_width))
                .border_color(theme.colors.workbench.border_subtle)
                .rounded(px(10.0))
                .shadow_lg()
                .children(menu_items)
        }))
        .into_any_element()
}

/// 鼠标与键盘确认走同一收尾，先转换格式再恢复原选区所属输入的焦点。
fn render_line_ending_menu_item(
    id: &'static str,
    label: &'static str,
    ending: LineEnding,
    active: bool,
    index: usize,
    focus_handles: [FocusHandle; 3],
    button_focus_handle: FocusHandle,
    theme: &Theme,
    cx: &mut Context<Editor>,
) -> AnyElement {
    let focus_handle = focus_handles[index].clone();
    let keyboard_focus_handles = focus_handles.clone();
    div()
        .id(SharedString::from(format!("status-bar-line-ending-{id}")))
        .debug_selector(move || format!("status-bar-line-ending-{id}"))
        .h(px(30.0))
        .w_full()
        .px(px(8.0))
        .tab_index(0)
        .track_focus(&focus_handle)
        .flex()
        .items_center()
        .gap(px(8.0))
        .rounded(px(6.0))
        .border(px(1.0))
        .border_color(hsla(0.0, 0.0, 0.0, 0.0))
        .bg(if active {
            theme.colors.workbench.control_hover
        } else {
            hsla(0.0, 0.0, 0.0, 0.0)
        })
        .hover(|this| this.bg(theme.colors.workbench.control_hover))
        .focus(|this| this.border_color(theme.colors.workbench.focus_ring))
        .cursor_pointer()
        .child(
            div()
                .flex_1()
                .text_size(px(theme.dimensions.status_bar_text_size))
                .text_color(theme.colors.workbench.text_primary)
                .child(label),
        )
        .children(active.then(|| {
            svg()
                .path("icon/ui/check.svg")
                .size(px(14.0))
                .text_color(theme.colors.workbench.accent)
        }))
        .on_click(cx.listener(move |editor, _: &ClickEvent, window, cx| {
            editor.finish_line_ending_menu_choice(ending, window, cx);
            cx.stop_propagation();
        }))
        .on_key_down(
            cx.listener(move |editor, event: &KeyDownEvent, window, cx| {
                match event.keystroke.key.as_str() {
                    "enter" | "space" => {
                        editor.finish_line_ending_menu_choice(ending, window, cx);
                        cx.stop_propagation();
                    }
                    "escape" => {
                        editor.status_bar.line_ending_menu_open = false;
                        editor.status_bar.line_ending_restore_focus = None;
                        button_focus_handle.focus(window);
                        cx.notify();
                        cx.stop_propagation();
                    }
                    "up" => {
                        keyboard_focus_handles[(index + 2) % 3].focus(window);
                        cx.stop_propagation();
                    }
                    "down" => {
                        keyboard_focus_handles[(index + 1) % 3].focus(window);
                        cx.stop_propagation();
                    }
                    _ => {}
                }
            }),
        )
        .into_any_element()
}
