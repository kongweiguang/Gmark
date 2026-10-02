// @author kongweiguang

use super::menu_view::resource_menu_text;
use super::*;
use crate::theme::workbench::SurfaceKind;
use crate::ui::visual_preferences::VisualPreferencesManager;
use gpui::prelude::FluentBuilder;

impl Editor {
    /// Keeps text-menu layout isolated while routing each row through the live pane-bound command model.
    pub(super) fn render_text_context_menu_overlay(
        &self,
        position: Point<Pixels>,
        theme: &Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let model = self.context_menu_command_model(cx);
        if model.main.is_empty() {
            return None;
        }
        let colors = &theme.colors;
        let visual_preferences = cx
            .try_global::<VisualPreferencesManager>()
            .map(VisualPreferencesManager::current)
            .unwrap_or_default();
        let palette = &colors.workbench;
        let material = palette.material(SurfaceKind::Glass, visual_preferences);
        let dimensions = &theme.dimensions;
        let typography = &theme.typography;
        let strings = cx.global::<I18nManager>().strings().clone();
        let group_for = |command| match command {
            ContextMenuCommand::TextOpenLink => 0,
            ContextMenuCommand::TextDuplicateLine
            | ContextMenuCommand::TextDeleteLine
            | ContextMenuCommand::TextMoveLineUp
            | ContextMenuCommand::TextMoveLineDown => 2,
            ContextMenuCommand::TextInsert => 3,
            _ => 1,
        };
        let separator_count = model
            .main
            .windows(2)
            .filter(|items| group_for(items[0].command) != group_for(items[1].command))
            .count();
        let panel_width = dimensions.context_menu_submenu_width.max(220.0);
        let viewport = window.viewport_size();
        let panel_max_height = (f32::from(viewport.height) - 16.0).max(80.0);
        let panel_height = compact_menu_panel_height(model.main.len(), separator_count, dimensions)
            .min(panel_max_height);
        let panel_origin =
            clamped_floating_panel_origin(position, panel_width, panel_height, viewport);
        let mut panel = div()
            .id("editor-text-context-menu-panel")
            .debug_selector(|| "editor-text-context-menu-panel".to_owned())
            .absolute()
            .left(panel_origin.x)
            .top(panel_origin.y)
            .w(px(panel_width))
            .p(px(dimensions.menu_panel_padding))
            .flex()
            .flex_col()
            .gap(px(dimensions.menu_panel_gap))
            .max_h(px(panel_max_height))
            .overflow_y_scroll()
            .track_scroll(&self.context_menu_scroll_handle)
            .scrollbar_width(px(0.0))
            .bg(material.background)
            .border(px(dimensions.dialog_border_width))
            .border_color(material.border)
            .rounded(px(dimensions.menu_panel_radius))
            .shadow_lg()
            .on_mouse_down(MouseButton::Left, |_event, _window, cx| {
                cx.stop_propagation()
            });
        let mut previous_group = None;
        for (index, entry) in model.main.iter().copied().enumerate() {
            let group = group_for(entry.command);
            if previous_group.is_some_and(|previous| previous != group) {
                panel = panel.child(
                    div()
                        .mx(px(dimensions.menu_separator_margin_x))
                        .my(px(dimensions.menu_separator_margin_y))
                        .h(px(dimensions.menu_separator_height))
                        .bg(material.border),
                );
            }
            previous_group = Some(group);
            let (label, icon) = match entry.command {
                ContextMenuCommand::TextOpenLink => (
                    resource_menu_text(&strings, "resource_open_link", "Open Link"),
                    "icon/ui/link.svg",
                ),
                ContextMenuCommand::TextCopy => {
                    (strings.preferences_shortcut_copy.clone(), COPY_ICON)
                }
                ContextMenuCommand::TextCopyAsMarkdown => (
                    strings.preferences_shortcut_copy_as_markdown.clone(),
                    COPY_ICON,
                ),
                ContextMenuCommand::TextCut => {
                    (strings.preferences_shortcut_cut.clone(), COPY_ICON)
                }
                ContextMenuCommand::TextPaste => (
                    strings.preferences_shortcut_paste.clone(),
                    "icon/ui/clipboard.svg",
                ),
                ContextMenuCommand::TextSelectAll => {
                    (strings.preferences_shortcut_select_all.clone(), COPY_ICON)
                }
                ContextMenuCommand::TextDuplicateLine => (
                    strings.preferences_shortcut_duplicate_line.clone(),
                    COPY_ICON,
                ),
                ContextMenuCommand::TextDeleteLine => {
                    (strings.preferences_shortcut_delete_line.clone(), TRASH_ICON)
                }
                ContextMenuCommand::TextMoveLineUp => (
                    strings.preferences_shortcut_move_line_up.clone(),
                    "icon/ui/arrow-up.svg",
                ),
                ContextMenuCommand::TextMoveLineDown => (
                    strings.preferences_shortcut_move_line_down.clone(),
                    "icon/ui/arrow-down.svg",
                ),
                ContextMenuCommand::TextInsert => (strings.context_menu_insert.clone(), PLUS_ICON),
                _ => continue,
            };
            let row = div()
                .id(("editor-text-context-command", index))
                .debug_selector(move || format!("editor-text-context-command-{index}"))
                .h(px(dimensions.menu_item_height))
                .px(px(dimensions.menu_item_padding_x))
                .flex()
                .items_center()
                .gap(px(6.0))
                .rounded(px(dimensions.menu_item_radius))
                .bg(if self.context_menu_keyboard_item == Some(index) {
                    palette.control_hover
                } else {
                    material.background
                })
                .text_size(px(dimensions.menu_text_size))
                .font_weight(typography.dialog_body_weight.to_font_weight())
                .text_color(if entry.enabled {
                    palette.text_primary
                } else {
                    palette.text_secondary
                })
                .child(menu_icon_slot(Some(icon), palette.icon))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .overflow_hidden()
                        .truncate()
                        .child(label),
                )
                .on_hover(cx.listener(Self::on_context_menu_pointer_hover));
            let row = if entry.enabled {
                row.hover(|this| this.bg(palette.control_hover))
                    .cursor_pointer()
                    .on_click(cx.listener(move |editor, _event, window, cx| {
                        editor.execute_context_menu_command(entry.command, window, cx)
                    }))
                    .into_any_element()
            } else {
                row.into_any_element()
            };
            panel = panel.child(row);
        }
        Some(
            div()
                .id("editor-text-context-menu-overlay")
                .absolute()
                .top_0()
                .left_0()
                .right_0()
                .bottom_0()
                .occlude()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(Self::on_dismiss_context_menu_overlay),
                )
                .child(panel)
                .into_any_element(),
        )
    }
}
