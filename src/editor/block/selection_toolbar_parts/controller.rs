// @author kongweiguang

use super::*;

impl Block {
    pub(crate) fn refresh_selection_toolbar(&mut self) {
        let Some(range) = self.selection_toolbar_range() else {
            self.selection_toolbar_dismissed_range = None;
            self.selection_toolbar_keyboard_active = false;
            self.selection_toolbar_overflow_open = false;
            self.selection_toolbar_type_menu_open = false;
            return;
        };
        if self.selection_toolbar_dismissed_range.as_ref() != Some(&range) {
            self.selection_toolbar_dismissed_range = None;
        }
    }

    /// 拖选或 IME 预编辑期间隐藏会拦截指针的浮层，让选择手势和组合输入先完成。
    pub(crate) fn selection_toolbar_visible(&self) -> bool {
        if self.is_selecting || self.has_ime_composition() {
            return false;
        }
        let Some(range) = self.selection_toolbar_range() else {
            return false;
        };
        self.selection_toolbar_dismissed_range.as_ref() != Some(&range)
    }

    /// 已测量所属视口时才用浮层几何拦截导航；未挂载视口的单块上下文不依赖窗口全局。
    pub(crate) fn handle_selection_toolbar_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        self.refresh_selection_toolbar();
        if !self.selection_toolbar_visible() {
            return false;
        }
        if self.selection_toolbar_viewport.is_some() {
            let geometry = self.selection_toolbar_geometry(
                window.viewport_size(),
                cx.global::<I18nManager>().current_language_id(),
            );
            if geometry == SelectionToolbarGeometry::Hidden {
                self.selection_toolbar_keyboard_active = false;
                cx.notify();
                return false;
            }
        }
        let modifiers = event.keystroke.modifiers;
        if event.keystroke.key.eq_ignore_ascii_case("f10")
            && modifiers.alt
            && !modifiers.control
            && !modifiers.platform
            && !modifiers.shift
        {
            self.selection_toolbar_keyboard_active = true;
            self.selection_toolbar_keyboard_index = 0;
            cx.notify();
            return true;
        }
        if self.selection_toolbar_keyboard_active {
            let commands = ToolbarCommand::PRIMARY
                .into_iter()
                .filter(|command| {
                    (*command != ToolbarCommand::BlockType || !self.is_table_cell())
                        && self.selection_toolbar_command_available(*command)
                })
                .collect::<Vec<_>>();
            if commands.is_empty() {
                self.selection_toolbar_keyboard_active = false;
                return false;
            }
            self.selection_toolbar_keyboard_index = self
                .selection_toolbar_keyboard_index
                .min(commands.len() - 1);
            match event.keystroke.key.as_str() {
                "left" | "up" => {
                    self.selection_toolbar_keyboard_index =
                        if self.selection_toolbar_keyboard_index == 0 {
                            commands.len() - 1
                        } else {
                            self.selection_toolbar_keyboard_index - 1
                        };
                }
                "right" | "down" => {
                    self.selection_toolbar_keyboard_index =
                        (self.selection_toolbar_keyboard_index + 1) % commands.len();
                }
                "home" => self.selection_toolbar_keyboard_index = 0,
                "end" => self.selection_toolbar_keyboard_index = commands.len() - 1,
                "enter" | "space" => self.invoke_selection_toolbar_command(
                    commands[self.selection_toolbar_keyboard_index],
                    window,
                    cx,
                ),
                "escape" => self.selection_toolbar_keyboard_active = false,
                _ => return false,
            }
            cx.notify();
            return true;
        }
        if event.keystroke.key != "escape" {
            return false;
        }
        let Some(range) = self.selection_toolbar_range() else {
            return false;
        };
        self.selection_toolbar_dismissed_range = Some(range);
        self.selection_toolbar_keyboard_active = false;
        self.selection_toolbar_overflow_open = false;
        self.selection_toolbar_type_menu_open = false;
        cx.notify();
        true
    }

    fn selection_toolbar_command_active(&self, command: ToolbarCommand) -> bool {
        let Some(range) = self.selection_toolbar_range() else {
            return false;
        };
        match command {
            ToolbarCommand::BlockType => self.selection_toolbar_type_menu_open,
            ToolbarCommand::Bold => self
                .record
                .title
                .selection_has_style(range, StyleFlag::Bold),
            ToolbarCommand::Italic => self
                .record
                .title
                .selection_has_style(range, StyleFlag::Italic),
            ToolbarCommand::Strikethrough => self
                .record
                .title
                .selection_has_style(range, StyleFlag::Strikethrough),
            ToolbarCommand::Code => self
                .record
                .title
                .selection_has_style(range, StyleFlag::Code),
            ToolbarCommand::Underline => self
                .record
                .title
                .selection_has_style(range, StyleFlag::Underline),
            ToolbarCommand::Highlight => self
                .record
                .title
                .selection_has_style(range, StyleFlag::Highlight),
            ToolbarCommand::Superscript => self
                .record
                .title
                .selection_has_style(range, StyleFlag::Superscript),
            ToolbarCommand::Subscript => self
                .record
                .title
                .selection_has_style(range, StyleFlag::Subscript),
            ToolbarCommand::InlineMath => false,
            ToolbarCommand::Link => self.record.title.selection_has_link(range),
            ToolbarCommand::Overflow => self.selection_toolbar_overflow_open,
            ToolbarCommand::ClearFormatting => false,
        }
    }

    pub(super) fn selection_toolbar_command_available(&self, command: ToolbarCommand) -> bool {
        if command == ToolbarCommand::BlockType {
            return self.editor_selection_range.is_none()
                && !self.is_table_cell()
                && EditingCommandId::for_block_kind(&self.kind()).is_some();
        }
        let Some(id) = command.editing_command() else {
            return true;
        };
        if !INLINE_COMMANDS.contains(&id) {
            return false;
        }
        id.is_available(self.editing_command_context())
    }

    pub(super) fn apply_selection_toolbar_command(
        &mut self,
        command: ToolbarCommand,
        cx: &mut Context<Self>,
    ) {
        match command {
            ToolbarCommand::BlockType => {
                self.selection_toolbar_type_menu_open = !self.selection_toolbar_type_menu_open;
                self.selection_toolbar_overflow_open = false;
                cx.notify();
                return;
            }
            ToolbarCommand::Bold => self.toggle_inline_format(InlineFormat::Bold, cx),
            ToolbarCommand::Italic => self.toggle_inline_format(InlineFormat::Italic, cx),
            ToolbarCommand::Strikethrough => {
                self.toggle_inline_format(InlineFormat::Strikethrough, cx)
            }
            ToolbarCommand::Code => self.toggle_inline_format(InlineFormat::Code, cx),
            ToolbarCommand::Link => self.toggle_inline_link(cx),
            ToolbarCommand::Underline => self.toggle_inline_format(InlineFormat::Underline, cx),
            ToolbarCommand::Highlight => self.toggle_inline_format(InlineFormat::Highlight, cx),
            ToolbarCommand::Superscript => self.toggle_inline_format(InlineFormat::Superscript, cx),
            ToolbarCommand::Subscript => self.toggle_inline_format(InlineFormat::Subscript, cx),
            ToolbarCommand::InlineMath => {
                self.insert_inline_math(cx);
            }
            ToolbarCommand::ClearFormatting => self.clear_inline_formatting(cx),
            ToolbarCommand::Overflow => {
                self.selection_toolbar_overflow_open = !self.selection_toolbar_overflow_open;
                self.selection_toolbar_type_menu_open = false;
                cx.notify();
                return;
            }
        }
        self.selection_toolbar_overflow_open = false;
        self.selection_toolbar_type_menu_open = false;
    }

    fn invoke_selection_toolbar_command(
        &mut self,
        command: ToolbarCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.editor_selection_range.is_some()
            && let Some(command_id) = command.editing_command()
            && command_id.is_available(EditingContext {
                selection: EditingSelectionContext::AcrossBlocks,
                ..self.editing_command_context()
            })
        {
            self.selection_toolbar_overflow_open = false;
            self.selection_toolbar_type_menu_open = false;
            cx.emit(BlockEvent::RequestEditingCommand {
                command: command_id,
            });
            cx.notify();
            return;
        }
        if command == ToolbarCommand::Link {
            self.open_selection_link_editor(window, cx);
        } else {
            self.apply_selection_toolbar_command(command, cx);
        }
    }
}
#[path = "../selection_toolbar/link_editor.rs"]
mod link_editor;

impl Block {
    fn render_selection_toolbar_button(
        &self,
        command: ToolbarCommand,
        show_block_type_label: bool,
        block_type_width: f32,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let c = &theme.colors;
        let visual_preferences = cx
            .try_global::<VisualPreferencesManager>()
            .map(VisualPreferencesManager::current)
            .unwrap_or_default();
        let palette = &c.workbench;
        let material = palette.material(SurfaceKind::Glass, visual_preferences);
        let strings = cx.global::<I18nManager>().strings();
        let tooltip_label: SharedString = if command == ToolbarCommand::BlockType {
            EditingCommandId::for_block_kind(&self.kind())
                .and_then(|command| {
                    strings
                        .slash_commands
                        .get(command.descriptor().localization_key)
                        .cloned()
                })
                .unwrap_or_else(|| command.label(strings))
                .into()
        } else {
            command.label(strings).into()
        };
        let active = self.selection_toolbar_command_active(command);
        let available = self.selection_toolbar_command_available(command);
        let keyboard_focused = self.selection_toolbar_keyboard_active
            && ToolbarCommand::PRIMARY
                .into_iter()
                .filter(|candidate| {
                    (*candidate != ToolbarCommand::BlockType || !self.is_table_cell())
                        && self.selection_toolbar_command_available(*candidate)
                })
                .nth(self.selection_toolbar_keyboard_index)
                == Some(command);
        let symbol: AnyElement = match command {
            ToolbarCommand::BlockType => {
                let descriptor = EditingCommandId::for_block_kind(&self.kind())
                    .unwrap_or(EditingCommandId::Paragraph)
                    .descriptor();
                div()
                    .flex()
                    .items_center()
                    .gap(px(if show_block_type_label { 6.0 } else { 2.0 }))
                    .child(
                        svg()
                            .path(descriptor.icon_path)
                            .size(px(15.0))
                            .text_color(palette.text_primary),
                    )
                    .when(show_block_type_label, |content| {
                        content.child(
                            div()
                                .min_w(px(0.0))
                                .max_w(px(62.0))
                                .overflow_hidden()
                                .truncate()
                                .text_size(px(12.0))
                                .text_color(palette.text_primary)
                                .child(tooltip_label.clone()),
                        )
                    })
                    .child(
                        svg()
                            .path("icon/ui/chevron-down.svg")
                            .size(px(11.0))
                            .text_color(palette.text_secondary),
                    )
                    .into_any_element()
            }
            ToolbarCommand::Code | ToolbarCommand::Link | ToolbarCommand::Overflow => svg()
                .path(match command {
                    ToolbarCommand::Code => CODE_ICON,
                    ToolbarCommand::Link => LINK_ICON,
                    ToolbarCommand::Overflow => MORE_ICON,
                    _ => unreachable!(),
                })
                .size(px(15.0))
                .text_color(if active {
                    palette.accent
                } else {
                    palette.text_primary
                })
                .into_any_element(),
            _ => {
                let symbol = div()
                    .text_size(px(13.0))
                    .text_color(if active {
                        palette.accent
                    } else {
                        palette.text_primary
                    })
                    .child(command.symbol());
                match command {
                    ToolbarCommand::Bold => symbol.font_weight(FontWeight::BOLD),
                    ToolbarCommand::Italic => symbol.italic(),
                    ToolbarCommand::Strikethrough => symbol.line_through(),
                    ToolbarCommand::Underline => symbol.underline(),
                    ToolbarCommand::Superscript
                    | ToolbarCommand::Subscript
                    | ToolbarCommand::InlineMath
                    | ToolbarCommand::Highlight
                    | ToolbarCommand::ClearFormatting => symbol,
                    ToolbarCommand::BlockType => symbol,
                    _ => symbol,
                }
                .into_any_element()
            }
        };
        div()
            .id(SharedString::from(format!(
                "selection-toolbar-{}",
                command.id()
            )))
            .debug_selector(move || format!("selection-toolbar-{}", command.id()))
            .w(px(if command == ToolbarCommand::BlockType {
                if show_block_type_label {
                    block_type_width
                } else {
                    42.0
                }
            } else {
                28.0
            }))
            .h(px(28.0))
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(4.0))
            .border(px(1.0))
            .border_color(if keyboard_focused {
                palette.focus_ring
            } else {
                material.border
            })
            .bg(if active {
                palette.control_hover
            } else {
                material.background
            })
            .hover(|this| this.bg(palette.control_hover))
            .active(|this| this.opacity(0.86))
            .opacity(if available { 1.0 } else { 0.45 })
            .when(available, |button| button.cursor_pointer())
            .tooltip(move |_window, cx| crate::ui::ui_tooltip(tooltip_label.clone(), cx))
            .child(symbol)
            .when(available, |button| {
                button.on_click(cx.listener(move |block, _event, window, cx| {
                    block.invoke_selection_toolbar_command(command, window, cx);
                }))
            })
            .into_any_element()
    }

    /// 用所属滚动范围限制浮层，并按块壳的统一内容 inset 回换窗口坐标，避免窄窗越界。
    pub(crate) fn render_selection_toolbar(
        &self,
        theme: &Theme,
        viewport: Size<Pixels>,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.selection_toolbar_visible() {
            return None;
        }
        let text_bounds = self.last_bounds?;
        let owner_viewport = resolved_toolbar_viewport(self.selection_toolbar_viewport, viewport);
        let show_block_type = !self.is_table_cell() && self.editor_selection_range.is_none();
        let block_type_width =
            expanded_block_type_width(cx.global::<I18nManager>().current_language_id());
        let expanded_block_type_width = show_block_type.then_some(block_type_width);
        let SelectionToolbarGeometry::Positioned(position) = self
            .selection_toolbar_geometry(viewport, cx.global::<I18nManager>().current_language_id())
        else {
            return None;
        };
        let d = &theme.dimensions;
        let c = &theme.colors;
        let visual_preferences = cx
            .try_global::<VisualPreferencesManager>()
            .map(VisualPreferencesManager::current)
            .unwrap_or_default();
        let palette = &c.workbench;
        let material = palette.material(SurfaceKind::Glass, visual_preferences);
        let solid_material = palette.material(SurfaceKind::Solid, visual_preferences);
        let toolbar_width =
            selection_toolbar_width(text_bounds, owner_viewport, expanded_block_type_width);
        let show_block_type_label = show_block_type && toolbar_width > TOOLBAR_COMPACT_WIDTH;
        let (type_menu_above, type_menu_available_height) =
            attached_surface_placement(position, 312.0, owner_viewport);
        let (link_editor_above, link_editor_available_height) =
            attached_surface_placement(position, 42.0, owner_viewport);
        let (overflow_menu_above, overflow_menu_available_height) =
            attached_surface_placement(position, OVERFLOW_MENU_HEIGHT, owner_viewport);
        let overflow_menu_max_height =
            overflow_menu_available_height.clamp(1.0, OVERFLOW_MENU_HEIGHT);
        let link_editor_max_height = link_editor_available_height.clamp(1.0, 42.0);
        let origin_left =
            f32::from(text_bounds.left()) - crate::components::rendered_content_inset(d);
        let origin_top = f32::from(text_bounds.top()) - d.block_padding_y;
        let overflow = self.selection_toolbar_overflow_open.then(|| {
            let menu = div()
                .id("selection-toolbar-overflow-menu")
                .debug_selector(|| "selection-toolbar-overflow-menu".to_owned())
                .absolute()
                .right_0()
                .p(px(2.0))
                .max_h(px(overflow_menu_max_height))
                .overflow_y_scroll()
                .bg(material.background)
                .border(px(d.dialog_border_width))
                .border_color(material.border)
                .rounded(px(12.0))
                .shadow_md()
                .occlude()
                .on_mouse_down(MouseButton::Left, |_event, _window, cx| {
                    cx.stop_propagation();
                })
                .flex()
                .flex_col()
                .children(
                    [
                        ToolbarCommand::Underline,
                        ToolbarCommand::Highlight,
                        ToolbarCommand::Superscript,
                        ToolbarCommand::Subscript,
                        ToolbarCommand::InlineMath,
                        ToolbarCommand::ClearFormatting,
                    ]
                    .into_iter()
                    .map(|command| {
                        self.render_selection_toolbar_button(command, false, 42.0, theme, cx)
                    }),
                );
            if overflow_menu_above {
                menu.bottom(px(TOOLBAR_HEIGHT + 4.0))
            } else {
                menu.top(px(TOOLBAR_HEIGHT + 4.0))
            }
        });
        let buttons = ToolbarCommand::PRIMARY
            .into_iter()
            .filter(|command| *command != ToolbarCommand::BlockType || show_block_type)
            .map(|command| {
                let button = self.render_selection_toolbar_button(
                    command,
                    show_block_type_label,
                    block_type_width,
                    theme,
                    cx,
                );
                if command == ToolbarCommand::Overflow {
                    div()
                        .relative()
                        .w(px(28.0))
                        .h(px(28.0))
                        .flex_shrink_0()
                        .child(button)
                        .into_any_element()
                } else {
                    button
                }
            })
            .collect::<Vec<_>>();
        let current_kind = self.kind();
        let type_menu = self.selection_toolbar_type_menu_open.then(|| {
            let type_menu_max_height = type_menu_available_height.clamp(1.0, 312.0);
            let items = TRANSFORM_COMMANDS
                .into_iter()
                .map(|command| {
                    let descriptor = command.descriptor();
                    let selected = EditingCommandId::for_block_kind(&current_kind) == Some(command);
                    let label = cx
                        .global::<I18nManager>()
                        .strings()
                        .slash_commands
                        .get(descriptor.localization_key)
                        .cloned()
                        .unwrap_or_else(|| descriptor.localization_key.to_owned());
                    div()
                        .id(SharedString::from(format!(
                            "selection-toolbar-block-type-{command:?}"
                        )))
                        .h(px(d.menu_item_height.max(30.0)))
                        .px(px(d.menu_item_padding_x))
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .rounded(px(d.menu_item_radius))
                        .bg(if selected {
                            palette.control_hover
                        } else {
                            material.background
                        })
                        .hover(|item| item.bg(palette.control_hover))
                        .cursor_pointer()
                        .child(svg().path(descriptor.icon_path).size(px(15.0)))
                        .child(
                            div()
                                .min_w(px(0.0))
                                .text_size(px(d.menu_text_size))
                                .text_color(palette.text_primary)
                                .child(label),
                        )
                        .on_click(cx.listener(move |block, _event, _window, cx| {
                            block.selection_toolbar_type_menu_open = false;
                            cx.emit(BlockEvent::RequestEditingCommand { command });
                            cx.notify();
                        }))
                        .into_any_element()
                })
                .collect::<Vec<_>>();
            let menu = div()
                .id("selection-toolbar-block-type-menu")
                .debug_selector(|| "selection-toolbar-block-type-menu".to_owned())
                .absolute()
                .left_0()
                .w(px(188.0))
                .max_h(px(type_menu_max_height))
                .overflow_y_scroll()
                .p(px(d.menu_panel_padding))
                .flex()
                .flex_col()
                .gap(px(d.menu_panel_gap))
                .bg(material.background)
                .border(px(d.dialog_border_width))
                .border_color(material.border)
                .rounded(px(d.menu_panel_radius.clamp(14.0, 18.0)))
                .shadow_lg()
                .children(items);
            if type_menu_above {
                menu.bottom(px(TOOLBAR_HEIGHT + 4.0))
            } else {
                menu.top(px(TOOLBAR_HEIGHT + 4.0))
            }
        });
        let link_editor = self.selection_toolbar_link_input.as_ref().map(|input| {
            let strings = cx.global::<I18nManager>().strings();
            let apply_label = strings
                .slash_commands
                .get("apply_link")
                .cloned()
                .unwrap_or_else(|| "Apply".to_owned());
            let remove_label = strings
                .slash_commands
                .get("remove_link")
                .cloned()
                .unwrap_or_else(|| "Remove".to_owned());
            let input = input.clone();
            let popover_min_left = (f32::from(text_bounds.left()) + VIEWPORT_INSET)
                .max(f32::from(owner_viewport.left()) + VIEWPORT_INSET);
            let popover_right = (f32::from(text_bounds.right())
                .min(f32::from(owner_viewport.right()))
                - VIEWPORT_INSET)
                .max(popover_min_left + 1.0);
            let popover_width = 292.0_f32.min(popover_right - popover_min_left);
            let popover_window_left = (position.left + toolbar_width - popover_width).clamp(
                popover_min_left,
                (popover_right - popover_width).max(popover_min_left),
            );
            let popover = div()
                .id("selection-toolbar-link-editor")
                .debug_selector(|| "selection-toolbar-link-editor".to_owned())
                .absolute()
                .left(px(popover_window_left - position.left))
                .w(px(popover_width))
                .max_h(px(link_editor_max_height))
                .overflow_y_scroll()
                .p(px(6.0))
                .flex()
                .items_center()
                .gap(px(6.0))
                .bg(material.background)
                .border(px(d.dialog_border_width))
                .border_color(material.border)
                .rounded(px(14.0))
                .shadow_lg()
                .on_mouse_down(MouseButton::Left, |_event, _window, cx| {
                    cx.stop_propagation();
                })
                .child(
                    div()
                        .min_w(px(0.0))
                        .flex_1()
                        .h(px(28.0))
                        .px(px(6.0))
                        .flex()
                        .items_center()
                        .rounded(px(5.0))
                        .border(px(d.dialog_border_width))
                        .border_color(solid_material.border)
                        .bg(solid_material.background)
                        .child(input),
                )
                .when(self.selection_toolbar_link_had_target, |popover| {
                    popover.child(
                        div()
                            .id("selection-toolbar-link-remove")
                            .h(px(28.0))
                            .px(px(8.0))
                            .flex()
                            .items_center()
                            .rounded(px(5.0))
                            .text_size(px(d.menu_text_size))
                            .text_color(palette.text_secondary)
                            .hover(|button| button.bg(palette.control_hover))
                            .cursor_pointer()
                            .child(remove_label)
                            .on_click(cx.listener(|block, _event, window, cx| {
                                block.commit_selection_link_editor(true, window, cx);
                            })),
                    )
                })
                .child(
                    div()
                        .id("selection-toolbar-link-apply")
                        .h(px(28.0))
                        .px(px(9.0))
                        .flex()
                        .items_center()
                        .rounded(px(5.0))
                        .text_size(px(d.menu_text_size))
                        .text_color(palette.text_inverse)
                        .bg(palette.accent)
                        .hover(|button| button.bg(palette.accent_hover))
                        .cursor_pointer()
                        .child(apply_label)
                        .on_click(cx.listener(|block, _event, window, cx| {
                            block.commit_selection_link_editor(false, window, cx);
                        })),
                );
            if link_editor_above {
                popover.bottom(px(TOOLBAR_HEIGHT + 4.0))
            } else {
                popover.top(px(TOOLBAR_HEIGHT + 4.0))
            }
        });
        let panel = div()
            .id("selection-toolbar")
            .debug_selector(|| "selection-toolbar".to_owned())
            .absolute()
            .left(px(position.left - origin_left))
            .top(px(position.top - origin_top))
            .w(px(toolbar_width))
            .h(px(TOOLBAR_HEIGHT))
            .p(px(2.0))
            .flex()
            .items_center()
            .gap(px(2.0))
            .rounded(px(6.0))
            .occlude()
            .bg(material.background)
            .border(px(d.dialog_border_width))
            .border_color(material.border)
            .shadow_lg()
            .on_mouse_down(MouseButton::Left, |_event, _window, cx| {
                cx.stop_propagation();
            })
            .on_mouse_up(MouseButton::Left, |_event, _window, cx| {
                cx.stop_propagation();
            })
            .children(buttons)
            // 菜单作为面板兄弟参与命中测试；嵌在 28px 按钮命中框外时 GPUI 不会派发点击。
            .children(overflow)
            .children(type_menu)
            .children(link_editor);
        Some(deferred(panel).with_priority(20).into_any_element())
    }
}
