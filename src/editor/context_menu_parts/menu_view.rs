// @author kongweiguang

use super::*;
use crate::theme::workbench::SurfaceKind;
use crate::ui::visual_preferences::VisualPreferencesManager;
use gpui::prelude::FluentBuilder;

type ResourceContextHandler = fn(&mut Editor, &ClickEvent, &mut Window, &mut Context<Editor>);

impl Editor {
    /// Renders each transient menu in one overlay while preserving pane-bound command routing.
    pub(in crate::editor) fn render_context_menu_overlay(
        &self,
        theme: &Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let menu = self.context_menu.as_ref()?;
        let c = &theme.colors;
        let visual_preferences = cx
            .try_global::<VisualPreferencesManager>()
            .map(VisualPreferencesManager::current)
            .unwrap_or_default();
        let palette = &c.workbench;
        let material = palette.material(SurfaceKind::Glass, visual_preferences);
        let d = &theme.dimensions;
        let t = &theme.typography;
        let s = cx.global::<I18nManager>().strings().clone();

        match menu {
            ContextMenuState::Insert {
                position,
                submenu_open,
                ..
            } => {
                let viewport = window.viewport_size();
                let panel_width = d.context_menu_panel_width;
                let panel_height = compact_menu_panel_height(1, 0, d);
                let panel_origin =
                    clamped_floating_panel_origin(*position, panel_width, panel_height, viewport);
                let panel_x = panel_origin.x;
                let panel_y = panel_origin.y;
                let submenu_width = d.context_menu_submenu_width;
                let submenu_height = compact_menu_panel_height(INSERT_COMMANDS.len(), 0, d);
                let submenu_origin = clamped_floating_panel_origin(
                    panel_origin,
                    submenu_width,
                    submenu_height,
                    viewport,
                );
                let submenu_x = floating_submenu_x(
                    panel_x,
                    panel_width,
                    submenu_width,
                    d.context_menu_submenu_gap,
                    viewport.width,
                );

                let submenu = submenu_open.then(|| {
                    let mut panel = div()
                        .id("editor-context-menu-submenu")
                        .debug_selector(|| "editor-context-menu-submenu".to_owned())
                        .absolute()
                        .left(submenu_x)
                        .top(submenu_origin.y)
                        .w(px(submenu_width))
                        .p(px(d.menu_panel_padding))
                        .flex()
                        .flex_col()
                        .gap(px(d.menu_panel_gap))
                        .max_h(relative(0.82))
                        .overflow_y_scroll()
                        .occlude()
                        .bg(material.background)
                        .border(px(d.dialog_border_width))
                        .border_color(material.border)
                        .rounded(px(d.menu_panel_radius))
                        .shadow_lg()
                        .on_mouse_down(MouseButton::Left, |_event, _window, cx| {
                            cx.stop_propagation()
                        })
                        .on_hover(cx.listener(Self::on_context_menu_submenu_hover));
                    for (index, command) in INSERT_COMMANDS.into_iter().enumerate() {
                        let descriptor = command.descriptor();
                        let label = s
                            .slash_commands
                            .get(descriptor.localization_key)
                            .cloned()
                            .unwrap_or_else(|| descriptor.localization_key.to_owned());
                        panel = panel.child(
                            div()
                                .id(("editor-context-menu-insert-command", index))
                                .debug_selector(move || {
                                    format!("editor-context-menu-insert-{}", command.stable_id())
                                })
                                .h(px(d.menu_item_height))
                                .px(px(d.menu_item_padding_x))
                                .flex()
                                .items_center()
                                .gap(px(6.0))
                                .rounded(px(d.menu_item_radius))
                                .bg(if self.context_menu_keyboard_submenu_item == Some(index) {
                                    palette.control_hover
                                } else {
                                    material.background
                                })
                                .hover(|this| this.bg(palette.control_hover))
                                .active(|this| this.opacity(0.92))
                                .cursor_pointer()
                                .text_size(px(d.menu_text_size))
                                .font_weight(t.dialog_body_weight.to_font_weight())
                                .text_color(palette.text_primary)
                                .child(
                                    menu_icon_slot(Some(descriptor.icon_path), palette.icon)
                                        .debug_selector(move || {
                                            format!(
                                                "editor-context-menu-insert-{}-icon",
                                                command.stable_id()
                                            )
                                        }),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w(px(0.0))
                                        .overflow_hidden()
                                        .truncate()
                                        .child(label),
                                )
                                .on_click(cx.listener(move |editor, event, window, cx| {
                                    editor
                                        .on_context_menu_insert_command(command, event, window, cx)
                                })),
                        );
                    }
                    panel
                });

                let overlay = div()
                    .id("editor-context-menu-overlay")
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
                    .child(
                        div()
                            .id("editor-context-menu-panel")
                            .debug_selector(|| "editor-context-menu-panel".to_owned())
                            .absolute()
                            .left(panel_x)
                            .top(panel_y)
                            .w(px(panel_width))
                            .p(px(d.menu_panel_padding))
                            .flex()
                            .flex_col()
                            .gap(px(d.menu_panel_gap))
                            .max_h(relative(0.82))
                            .overflow_y_scroll()
                            .bg(material.background)
                            .border(px(d.dialog_border_width))
                            .border_color(material.border)
                            .rounded(px(d.menu_panel_radius))
                            .shadow_lg()
                            .on_mouse_down(MouseButton::Left, |_event, _window, cx| {
                                cx.stop_propagation()
                            })
                            .child(
                                div()
                                    .id("editor-context-menu-insert")
                                    .debug_selector(|| "editor-context-menu-insert".to_owned())
                                    .h(px(d.menu_item_height))
                                    .px(px(d.menu_item_padding_x))
                                    .flex()
                                    .items_center()
                                    .gap(px(6.0))
                                    .rounded(px(d.menu_item_radius))
                                    .bg(
                                        if *submenu_open
                                            || self.context_menu_keyboard_item == Some(0)
                                        {
                                            palette.control_hover
                                        } else {
                                            material.background
                                        },
                                    )
                                    .hover(|this| this.bg(palette.control_hover))
                                    .text_size(px(d.menu_text_size))
                                    .font_weight(t.dialog_body_weight.to_font_weight())
                                    .text_color(palette.text_primary)
                                    .child(
                                        menu_icon_slot(Some(PLUS_ICON), palette.icon)
                                            .debug_selector(|| {
                                                "editor-context-menu-insert-icon".to_owned()
                                            }),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w(px(0.0))
                                            .overflow_hidden()
                                            .truncate()
                                            .child(s.context_menu_insert.clone()),
                                    )
                                    .child(
                                        svg()
                                            .path("icon/ui/chevron-right.svg")
                                            .size(px(14.0))
                                            .flex_shrink_0(),
                                    )
                                    .on_hover(cx.listener(Self::on_context_menu_insert_hover)),
                            ),
                    );

                Some(if let Some(submenu) = submenu {
                    overlay.child(submenu).into_any_element()
                } else {
                    overlay.into_any_element()
                })
            }
            ContextMenuState::Spelling {
                position,
                diagnostic,
                ..
            } => {
                let panel_width = d.context_menu_submenu_width.max(220.0);
                let viewport = window.viewport_size();
                let panel_max_height = (f32::from(viewport.height) - 16.0).max(80.0);
                let panel_height =
                    compact_menu_panel_height(diagnostic.replacements.len() + 1, 0, d)
                        .min(panel_max_height);
                let panel_origin =
                    clamped_floating_panel_origin(*position, panel_width, panel_height, viewport);
                let mut panel = div()
                    .id("editor-spelling-menu-panel")
                    .debug_selector(|| "editor-spelling-menu-panel".to_owned())
                    .absolute()
                    .left(panel_origin.x)
                    .top(panel_origin.y)
                    .w(px(panel_width))
                    .max_h(px(panel_max_height))
                    .p(px(d.menu_panel_padding))
                    .flex()
                    .flex_col()
                    .overflow_y_scroll()
                    .track_scroll(&self.context_menu_scroll_handle)
                    .scrollbar_width(px(0.0))
                    .gap(px(d.menu_panel_gap))
                    .bg(material.background)
                    .border(px(d.dialog_border_width))
                    .border_color(material.border)
                    .rounded(px(d.menu_panel_radius))
                    .shadow_lg()
                    .on_mouse_down(MouseButton::Left, |_event, _window, cx| {
                        cx.stop_propagation()
                    })
                    .child(
                        div()
                            .px(px(d.menu_item_padding_x))
                            .py(px(4.0))
                            .text_size(px((d.menu_text_size - 1.0).max(10.0)))
                            .text_color(palette.text_secondary)
                            .child(diagnostic.message.clone()),
                    );
                for (index, replacement) in diagnostic.replacements.iter().enumerate() {
                    panel = panel.child(
                        div()
                            .id(("editor-spelling-suggestion", index))
                            .debug_selector(move || format!("editor-spelling-suggestion-{index}"))
                            .h(px(d.menu_item_height))
                            .px(px(d.menu_item_padding_x))
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .rounded(px(d.menu_item_radius))
                            .bg(if self.context_menu_keyboard_item == Some(index) {
                                palette.control_hover
                            } else {
                                material.background
                            })
                            .hover(|this| this.bg(palette.control_hover))
                            .on_hover(cx.listener(Self::on_context_menu_pointer_hover))
                            .cursor_pointer()
                            .text_size(px(d.menu_text_size))
                            .text_color(palette.text_primary)
                            .child(menu_icon_slot(Some(CHECK_ICON), palette.icon))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .overflow_hidden()
                                    .truncate()
                                    .child(replacement.clone()),
                            )
                            .on_click(cx.listener(move |this, event, window, cx| {
                                this.apply_spelling_suggestion(index, event, window, cx)
                            })),
                    );
                }
                Some(
                    div()
                        .id("editor-spelling-menu-overlay")
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
            ContextMenuState::TableAxis {
                position,
                selection,
            } => self.render_table_axis_context_menu(position, selection, theme, window, cx),
            ContextMenuState::Resource {
                position,
                entity_id,
            } => {
                let Some(_block) = self.focusable_entity_by_id(*entity_id) else {
                    return None;
                };
                let Some(resource) = self.resource_context_record(cx) else {
                    return None;
                };
                let missing = matches!(
                    self.resource_context_status(cx),
                    Some(crate::components::ResourceStatus::Missing)
                );
                let labels = [
                    resource_menu_text(&s, "resource_open", "Open"),
                    s.file_reveal_in_manager.clone(),
                    resource_menu_text(&s, "resource_edit_title", "Edit Title"),
                    resource_menu_text(&s, "resource_replace", "Replace Resource"),
                    resource_menu_text(&s, "resource_copy_address", "Copy Address"),
                    resource_menu_text(&s, "resource_convert_link", "Convert to Link"),
                    resource_menu_text(&s, "resource_delete", "Delete"),
                    resource_menu_text(&s, "resource_relocate", "Relocate"),
                ];
                let icons = [
                    "icon/ui/external-link.svg",
                    "icon/ui/file.svg",
                    "icon/ui/type.svg",
                    "icon/ui/replace.svg",
                    COPY_ICON,
                    "icon/ui/link.svg",
                    TRASH_ICON,
                    "icon/ui/locate.svg",
                ];
                let enabled = [
                    !resource.is_unsafe_url(),
                    resource.local_path().is_some(),
                    true,
                    true,
                    true,
                    true,
                    true,
                    missing,
                ];
                let disabled_reasons = [
                    (!enabled[0]).then(|| {
                        resource_menu_text(
                            &s,
                            "resource_disabled_unsafe_scheme",
                            "This address uses a blocked scheme",
                        )
                    }),
                    (!enabled[1]).then(|| {
                        resource_menu_text(
                            &s,
                            "resource_disabled_remote_location",
                            "Remote resources have no local file location",
                        )
                    }),
                    None,
                    None,
                    None,
                    None,
                    None,
                    (!enabled[7]).then(|| {
                        resource_menu_text(
                            &s,
                            "resource_disabled_not_missing",
                            "Relocate is available only when the local file is missing",
                        )
                    }),
                ];
                let handlers: [ResourceContextHandler; 8] = [
                    Self::open_resource_from_context_menu,
                    Self::reveal_resource_from_context_menu,
                    Self::edit_resource_title_from_context_menu,
                    Self::replace_resource_from_context_menu,
                    Self::copy_resource_address_from_context_menu,
                    Self::convert_resource_to_link_from_context_menu,
                    Self::delete_resource_from_context_menu,
                    Self::relocate_resource_from_context_menu,
                ];
                let panel_width = d.context_menu_submenu_width.max(240.0);
                let panel_origin = clamped_floating_panel_origin(
                    *position,
                    panel_width,
                    compact_menu_panel_height(labels.len(), 2, d),
                    window.viewport_size(),
                );
                let mut panel = div()
                    .id("resource-context-menu-panel")
                    .debug_selector(|| "resource-context-menu-panel".to_owned())
                    .absolute()
                    .left(panel_origin.x)
                    .top(panel_origin.y)
                    .w(px(panel_width))
                    .p(px(d.menu_panel_padding))
                    .flex()
                    .flex_col()
                    .gap(px(d.menu_panel_gap))
                    .max_h(relative(0.82))
                    .overflow_y_scroll()
                    .bg(material.background)
                    .border(px(d.dialog_border_width))
                    .border_color(material.border)
                    .rounded(px(d.menu_panel_radius))
                    .shadow_lg()
                    .on_mouse_down(MouseButton::Left, |_event, _window, cx| {
                        cx.stop_propagation()
                    });
                for index in 0..labels.len() {
                    let keyboard_selected = self.context_menu_keyboard_item == Some(index);
                    let id = format!("resource-context-menu-{index}");
                    let label = labels[index].clone();
                    let icon = icons[index];
                    let handler = handlers[index];
                    let disabled_reason = disabled_reasons[index].clone();
                    let item = div()
                        .id(ElementId::Name(id.clone().into()))
                        .debug_selector(move || id.clone())
                        .h(px(d.menu_item_height))
                        .px(px(d.menu_item_padding_x))
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .rounded(px(d.menu_item_radius))
                        .bg(if keyboard_selected {
                            palette.control_hover
                        } else {
                            material.background
                        })
                        .text_size(px(d.menu_text_size))
                        .text_color(if enabled[index] {
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
                        .on_hover(cx.listener(Self::on_context_menu_pointer_hover))
                        .when_some(disabled_reason, |item, reason| {
                            item.tooltip(move |_window, cx| {
                                crate::ui::ui_tooltip(reason.clone(), cx)
                            })
                        });
                    panel = if enabled[index] {
                        panel.child(
                            item.hover(|this| this.bg(palette.control_hover))
                                .cursor_pointer()
                                .on_click(cx.listener(handler)),
                        )
                    } else {
                        panel.child(item)
                    };
                    if index == 1 || index == 4 {
                        panel = panel.child(
                            div()
                                .mx(px(d.menu_separator_margin_x))
                                .my(px(d.menu_separator_margin_y))
                                .h(px(d.menu_separator_height))
                                .bg(material.border),
                        );
                    }
                }
                Some(
                    div()
                        .id("resource-context-menu-overlay")
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
            ContextMenuState::Text { position, .. } => {
                self.render_text_context_menu_overlay(*position, theme, window, cx)
            }
            ContextMenuState::Workspace { position, .. } => {
                let panel_width = d.context_menu_submenu_width.max(220.0);
                let panel_origin = clamped_floating_panel_origin(
                    *position,
                    panel_width,
                    compact_menu_panel_height(11, 6, d),
                    window.viewport_size(),
                );
                let item = |id: &'static str,
                            keyboard_index: usize,
                            label: String,
                            icon: &'static str,
                            enabled: bool,
                            handler: fn(
                    &mut Editor,
                    &ClickEvent,
                    &mut Window,
                    &mut Context<Editor>,
                )| {
                    let row = div()
                        .id(id)
                        .debug_selector(move || id.to_owned())
                        .h(px(d.menu_item_height))
                        .px(px(d.menu_item_padding_x))
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .rounded(px(d.menu_item_radius))
                        .bg(if self.context_menu_keyboard_item == Some(keyboard_index) {
                            palette.control_hover
                        } else {
                            material.background
                        })
                        .text_size(px(d.menu_text_size))
                        .text_color(if enabled {
                            palette.text_primary
                        } else {
                            palette.text_secondary
                        })
                        .child(
                            menu_icon_slot(Some(icon), palette.icon)
                                .debug_selector(move || format!("{id}-icon")),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .overflow_hidden()
                                .truncate()
                                .child(label),
                        )
                        .on_hover(cx.listener(Self::on_context_menu_pointer_hover));
                    if enabled {
                        row.hover(|this| this.bg(palette.control_hover))
                            .cursor_pointer()
                            .on_click(cx.listener(handler))
                            .into_any_element()
                    } else {
                        row.into_any_element()
                    }
                };
                let panel = div()
                    .id("workspace-context-menu-panel")
                    .debug_selector(|| "workspace-context-menu-panel".to_owned())
                    .absolute()
                    .left(panel_origin.x)
                    .top(panel_origin.y)
                    .w(px(panel_width))
                    .p(px(d.menu_panel_padding))
                    .flex()
                    .flex_col()
                    .gap(px(d.menu_panel_gap))
                    .max_h(relative(0.82))
                    .overflow_y_scroll()
                    .bg(material.background)
                    .border(px(d.dialog_border_width))
                    .border_color(material.border)
                    .rounded(px(d.menu_panel_radius))
                    .shadow_lg()
                    .on_mouse_down(MouseButton::Left, |_event, _window, cx| {
                        cx.stop_propagation()
                    })
                    .child(item(
                        "workspace-context-open",
                        0,
                        s.workspace_open.clone(),
                        "icon/ui/file.svg",
                        self.workspace_context_target_is_file(),
                        Self::on_workspace_open_menu,
                    ))
                    .child(item(
                        "workspace-context-reveal",
                        1,
                        s.file_reveal_in_manager.clone(),
                        "icon/workspace/folder.svg",
                        true,
                        Self::on_workspace_reveal_menu,
                    ))
                    .child(
                        div()
                            .mx(px(d.menu_separator_margin_x))
                            .my(px(d.menu_separator_margin_y))
                            .h(px(d.menu_separator_height))
                            .bg(material.border),
                    )
                    .child(item(
                        "workspace-context-copy-path",
                        2,
                        s.workspace_copy_path.clone(),
                        COPY_ICON,
                        true,
                        Self::on_workspace_copy_path_menu,
                    ))
                    .child(item(
                        "workspace-context-copy-relative-path",
                        3,
                        s.workspace_copy_relative_path.clone(),
                        COPY_ICON,
                        !self.workspace_context_target_is_root(),
                        Self::on_workspace_copy_relative_path_menu,
                    ))
                    .child(
                        div()
                            .mx(px(d.menu_separator_margin_x))
                            .my(px(d.menu_separator_margin_y))
                            .h(px(d.menu_separator_height))
                            .bg(material.border),
                    )
                    .child(item(
                        "workspace-context-new-file",
                        4,
                        s.workspace_new_file.clone(),
                        PLUS_ICON,
                        true,
                        Self::on_workspace_new_file_menu,
                    ))
                    .child(item(
                        "workspace-context-new-folder",
                        5,
                        s.workspace_new_folder.clone(),
                        "icon/workspace/folder.svg",
                        true,
                        Self::on_workspace_new_folder_menu,
                    ))
                    .child(
                        div()
                            .mx(px(d.menu_separator_margin_x))
                            .my(px(d.menu_separator_margin_y))
                            .h(px(d.menu_separator_height))
                            .bg(material.border),
                    )
                    .child(item(
                        "workspace-context-rename",
                        6,
                        s.workspace_rename.clone(),
                        "icon/ui/type.svg",
                        !self.workspace_context_target_is_root(),
                        Self::on_workspace_rename_menu,
                    ))
                    .child(item(
                        "workspace-context-move",
                        7,
                        s.workspace_move.clone(),
                        ARROW_RIGHT_ICON,
                        !self.workspace_context_target_is_root(),
                        Self::on_workspace_move_menu,
                    ))
                    .child(
                        div()
                            .mx(px(d.menu_separator_margin_x))
                            .my(px(d.menu_separator_margin_y))
                            .h(px(d.menu_separator_height))
                            .bg(material.border),
                    )
                    .child(item(
                        "workspace-context-refresh",
                        8,
                        s.workspace_refresh.clone(),
                        "icon/ui/refresh.svg",
                        true,
                        Self::on_workspace_refresh_menu,
                    ))
                    .child(
                        div()
                            .mx(px(d.menu_separator_margin_x))
                            .my(px(d.menu_separator_margin_y))
                            .h(px(d.menu_separator_height))
                            .bg(material.border),
                    )
                    .child(item(
                        "workspace-context-undo",
                        9,
                        s.workspace_undo_file_operation.clone(),
                        "icon/ui/undo.svg",
                        self.workspace_can_undo_file_operation(),
                        Self::on_workspace_undo_file_operation,
                    ))
                    .child(
                        div()
                            .mx(px(d.menu_separator_margin_x))
                            .my(px(d.menu_separator_margin_y))
                            .h(px(d.menu_separator_height))
                            .bg(material.border),
                    )
                    .child({
                        let enabled = !self.workspace_context_target_is_root();
                        let color = if enabled {
                            palette.danger
                        } else {
                            palette.text_secondary
                        };
                        let row = div()
                            .id("workspace-context-delete")
                            .debug_selector(|| "workspace-context-delete".to_owned())
                            .h(px(d.menu_item_height))
                            .px(px(d.menu_item_padding_x))
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .rounded(px(d.menu_item_radius))
                            .bg(if self.context_menu_keyboard_item == Some(10) {
                                palette.control_hover
                            } else {
                                material.background
                            })
                            .text_size(px(d.menu_text_size))
                            .text_color(color)
                            .child(
                                menu_icon_slot(Some(TRASH_ICON), color)
                                    .debug_selector(|| "workspace-context-delete-icon".to_owned()),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .overflow_hidden()
                                    .truncate()
                                    .child(s.workspace_delete.clone()),
                            )
                            .on_hover(cx.listener(Self::on_context_menu_pointer_hover));
                        if enabled {
                            row.hover(|this| this.bg(palette.control_hover))
                                .cursor_pointer()
                                .on_click(cx.listener(Self::on_workspace_delete_menu))
                                .into_any_element()
                        } else {
                            row.into_any_element()
                        }
                    });
                Some(
                    div()
                        .id("workspace-context-menu-overlay")
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
    }
}

/// Resolves shared resource wording from the command catalog for sibling menu surfaces.
pub(super) fn resource_menu_text(
    strings: &crate::i18n::I18nStrings,
    key: &str,
    english_fallback: &str,
) -> String {
    strings
        .slash_commands
        .get(key)
        .cloned()
        .unwrap_or_else(|| english_fallback.to_owned())
}
