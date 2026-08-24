// @author kongweiguang

//! Shortcut labels, capture, and editing controls.

use super::*;
use crate::ui::actions::format_shortcut_for_display;

impl PreferencesWindow {
    pub(super) fn shortcut_category_label(
        category: ShortcutCategory,
        strings: &crate::i18n::I18nStrings,
    ) -> String {
        match category {
            ShortcutCategory::File => strings.preferences_shortcuts_group_file.clone(),
            ShortcutCategory::Edit => strings.preferences_shortcuts_group_edit.clone(),
            ShortcutCategory::Navigation => strings.preferences_shortcuts_group_navigation.clone(),
            ShortcutCategory::Formatting => strings.preferences_shortcuts_group_formatting.clone(),
            ShortcutCategory::Block => strings.preferences_shortcuts_group_block.clone(),
            ShortcutCategory::Other => strings.preferences_shortcuts_group_other.clone(),
        }
    }

    pub(super) fn shortcut_command_label(
        command: ShortcutCommand,
        strings: &crate::i18n::I18nStrings,
    ) -> String {
        match command {
            ShortcutCommand::Newline => strings.preferences_shortcut_newline.clone(),
            ShortcutCommand::DeleteBack => strings.preferences_shortcut_delete_back.clone(),
            ShortcutCommand::Delete => strings.preferences_shortcut_delete.clone(),
            ShortcutCommand::WordDeleteBack => {
                strings.preferences_shortcut_word_delete_back.clone()
            }
            ShortcutCommand::WordDeleteForward => {
                strings.preferences_shortcut_word_delete_forward.clone()
            }
            ShortcutCommand::FocusPrev => strings.preferences_shortcut_focus_prev.clone(),
            ShortcutCommand::FocusNext => strings.preferences_shortcut_focus_next.clone(),
            ShortcutCommand::MoveLeft => strings.preferences_shortcut_move_left.clone(),
            ShortcutCommand::MoveRight => strings.preferences_shortcut_move_right.clone(),
            ShortcutCommand::WordMoveLeft => strings.preferences_shortcut_word_move_left.clone(),
            ShortcutCommand::WordMoveRight => strings.preferences_shortcut_word_move_right.clone(),
            ShortcutCommand::Home => strings.preferences_shortcut_home.clone(),
            ShortcutCommand::End => strings.preferences_shortcut_end.clone(),
            ShortcutCommand::BlockUp => strings.preferences_shortcut_block_up.clone(),
            ShortcutCommand::BlockDown => strings.preferences_shortcut_block_down.clone(),
            ShortcutCommand::PageUp => strings.preferences_shortcut_page_up.clone(),
            ShortcutCommand::PageDown => strings.preferences_shortcut_page_down.clone(),
            ShortcutCommand::JumpToTop => strings.preferences_shortcut_jump_to_top.clone(),
            ShortcutCommand::JumpToBottom => strings.preferences_shortcut_jump_to_bottom.clone(),
            ShortcutCommand::SelectLeft => strings.preferences_shortcut_select_left.clone(),
            ShortcutCommand::SelectRight => strings.preferences_shortcut_select_right.clone(),
            ShortcutCommand::WordSelectLeft => {
                strings.preferences_shortcut_word_select_left.clone()
            }
            ShortcutCommand::WordSelectRight => {
                strings.preferences_shortcut_word_select_right.clone()
            }
            ShortcutCommand::SelectHome => strings.preferences_shortcut_select_home.clone(),
            ShortcutCommand::SelectEnd => strings.preferences_shortcut_select_end.clone(),
            ShortcutCommand::SelectAll => strings.preferences_shortcut_select_all.clone(),
            ShortcutCommand::Copy => strings.preferences_shortcut_copy.clone(),
            ShortcutCommand::CopyAsMarkdown => {
                strings.preferences_shortcut_copy_as_markdown.clone()
            }
            ShortcutCommand::Cut => strings.preferences_shortcut_cut.clone(),
            ShortcutCommand::Paste => strings.preferences_shortcut_paste.clone(),
            ShortcutCommand::PasteAsPlainText => {
                strings.preferences_shortcut_paste_as_plain_text.clone()
            }
            ShortcutCommand::Undo => strings.preferences_shortcut_undo.clone(),
            ShortcutCommand::Redo => strings.preferences_shortcut_redo.clone(),
            ShortcutCommand::BoldSelection => strings.preferences_shortcut_bold_selection.clone(),
            ShortcutCommand::ItalicSelection => {
                strings.preferences_shortcut_italic_selection.clone()
            }
            ShortcutCommand::UnderlineSelection => {
                strings.preferences_shortcut_underline_selection.clone()
            }
            ShortcutCommand::StrikethroughSelection => {
                strings.preferences_shortcut_strikethrough_selection.clone()
            }
            ShortcutCommand::CodeSelection => strings.preferences_shortcut_code_selection.clone(),
            ShortcutCommand::LinkSelection => strings.preferences_shortcut_link_selection.clone(),
            ShortcutCommand::HighlightSelection => strings.slash_commands["highlight"].clone(),
            ShortcutCommand::SuperscriptSelection => strings.slash_commands["superscript"].clone(),
            ShortcutCommand::SubscriptSelection => strings.slash_commands["subscript"].clone(),
            ShortcutCommand::InlineMathSelection => strings.slash_commands["inline_math"].clone(),
            ShortcutCommand::SetParagraph => strings
                .slash_commands
                .get("paragraph")
                .cloned()
                .unwrap_or_else(|| "Paragraph".to_owned()),
            ShortcutCommand::SetHeading1 => strings.slash_commands["heading_1"].clone(),
            ShortcutCommand::SetHeading2 => strings.slash_commands["heading_2"].clone(),
            ShortcutCommand::SetHeading3 => strings.slash_commands["heading_3"].clone(),
            ShortcutCommand::SetHeading4 => strings.slash_commands["heading_4"].clone(),
            ShortcutCommand::SetHeading5 => strings.slash_commands["heading_5"].clone(),
            ShortcutCommand::SetHeading6 => strings.slash_commands["heading_6"].clone(),
            ShortcutCommand::IndentBlock => strings.preferences_shortcut_indent_block.clone(),
            ShortcutCommand::OutdentBlock => strings.preferences_shortcut_outdent_block.clone(),
            ShortcutCommand::ExitCodeBlock => strings.preferences_shortcut_exit_code_block.clone(),
            ShortcutCommand::SaveDocument => strings.preferences_shortcut_save_document.clone(),
            ShortcutCommand::SaveDocumentAs => {
                strings.preferences_shortcut_save_document_as.clone()
            }
            ShortcutCommand::CollapseFold => "Collapse Fold".to_owned(),
            ShortcutCommand::ExpandFold => "Expand Fold".to_owned(),
            ShortcutCommand::CollapseAllFolds => "Collapse All Folds".to_owned(),
            ShortcutCommand::ExpandAllFolds => "Expand All Folds".to_owned(),
            ShortcutCommand::FormatDocument => "Format Document".to_owned(),
            ShortcutCommand::FormatSelection => "Format Selection".to_owned(),
            ShortcutCommand::CancelFormatting => "Cancel Formatting".to_owned(),
            ShortcutCommand::NewWindow => strings.preferences_shortcut_new_window.clone(),
            ShortcutCommand::NewTab => strings.preferences_shortcut_new_tab.clone(),
            ShortcutCommand::OpenFile => strings.preferences_shortcut_open_file.clone(),
            ShortcutCommand::OpenFolder => strings.preferences_shortcut_open_folder.clone(),
            ShortcutCommand::OpenPreferences => strings.menu_preferences.clone(),
            ShortcutCommand::QuitApplication => {
                strings.preferences_shortcut_quit_application.clone()
            }
            ShortcutCommand::CloseWindow => strings.preferences_shortcut_close_window.clone(),
            ShortcutCommand::CloseTab => strings.preferences_shortcut_close_tab.clone(),
            ShortcutCommand::ReopenClosedTab => {
                strings.preferences_shortcut_reopen_closed_tab.clone()
            }
            ShortcutCommand::PreviousTab => strings.preferences_shortcut_previous_tab.clone(),
            ShortcutCommand::NextTab => strings.preferences_shortcut_next_tab.clone(),
            ShortcutCommand::DismissTransientUi => {
                strings.preferences_shortcut_dismiss_transient_ui.clone()
            }
            ShortcutCommand::ToggleViewMode => {
                strings.preferences_shortcut_toggle_view_mode.clone()
            }
            ShortcutCommand::ToggleWorkspace => {
                strings.preferences_shortcut_toggle_workspace.clone()
            }
            ShortcutCommand::ToggleDocumentSidebar => strings.menu_toggle_document_sidebar.clone(),
            ShortcutCommand::SplitRight => strings.preferences_shortcut_split_right.clone(),
            ShortcutCommand::SplitDown => strings.preferences_shortcut_split_down.clone(),
            ShortcutCommand::ClosePane => strings.preferences_shortcut_close_pane.clone(),
            ShortcutCommand::FocusPaneLeft => strings.preferences_shortcut_focus_pane_left.clone(),
            ShortcutCommand::FocusPaneRight => {
                strings.preferences_shortcut_focus_pane_right.clone()
            }
            ShortcutCommand::FocusPaneUp => strings.preferences_shortcut_focus_pane_up.clone(),
            ShortcutCommand::FocusPaneDown => strings.preferences_shortcut_focus_pane_down.clone(),
            ShortcutCommand::MoveTabToPaneLeft => {
                strings.preferences_shortcut_move_tab_to_pane_left.clone()
            }
            ShortcutCommand::MoveTabToPaneRight => {
                strings.preferences_shortcut_move_tab_to_pane_right.clone()
            }
            ShortcutCommand::MoveTabToPaneUp => {
                strings.preferences_shortcut_move_tab_to_pane_up.clone()
            }
            ShortcutCommand::MoveTabToPaneDown => {
                strings.preferences_shortcut_move_tab_to_pane_down.clone()
            }
            ShortcutCommand::BalancePanes => strings.preferences_shortcut_balance_panes.clone(),
            ShortcutCommand::QuickOpen => strings.preferences_shortcut_quick_open.clone(),
            ShortcutCommand::CommandPalette => strings.preferences_shortcut_command_palette.clone(),
            ShortcutCommand::GoToLine => strings.preferences_shortcut_go_to_line.clone(),
            ShortcutCommand::FindInDocument => {
                strings.preferences_shortcut_find_in_document.clone()
            }
            ShortcutCommand::ReplaceInDocument => {
                strings.preferences_shortcut_replace_in_document.clone()
            }
            ShortcutCommand::FindNext => strings.preferences_shortcut_find_next.clone(),
            ShortcutCommand::FindPrevious => strings.preferences_shortcut_find_previous.clone(),
            ShortcutCommand::ToggleFocusMode => {
                strings.preferences_shortcut_toggle_focus_mode.clone()
            }
            ShortcutCommand::ToggleTypewriterMode => {
                strings.preferences_shortcut_toggle_typewriter_mode.clone()
            }
        }
    }

    pub(super) fn format_template(template: &str, key: &str, value: &str) -> String {
        template.replace(key, value)
    }

    pub(super) fn begin_recording_shortcut(
        &mut self,
        command: ShortcutCommand,
        _: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.recording_shortcut = Some(command);
        self.shortcut_error = None;
        window.focus(&self.focus_handle);
        cx.notify();
    }

    pub(super) fn reset_shortcut(
        &mut self,
        command: ShortcutCommand,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(definition) = shortcut_definitions()
            .iter()
            .find(|definition| definition.command == command)
        {
            self.keybindings.remove(definition.id);
        }
        if self.recording_shortcut == Some(command) {
            self.recording_shortcut = None;
        }
        self.shortcut_error = None;
        cx.notify();
    }

    /// 录制并校验当前平台键位；与平台默认值相同的输入不写入冗余用户覆盖。
    pub(super) fn capture_shortcut_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.recording_shortcut.is_none() {
            let search_focused = self.search_input.read(cx).focus_handle.is_focused(window);
            let query = self.search_query(cx);
            if !search_focused || query.is_empty() {
                if event.keystroke.key == "escape" {
                    self.restore_saved_visual_accessibility(cx);
                    self.restore_saved_theme(cx);
                    window.remove_window();
                    cx.stop_propagation();
                }
                return;
            }

            let results = {
                let strings = cx.global::<I18nManager>().strings();
                self.preference_search_results(strings, cx)
            };
            match event.keystroke.key.as_str() {
                "up" => {
                    self.search_selected = self.search_selected.saturating_sub(1);
                }
                "down" => {
                    self.search_selected =
                        (self.search_selected + 1).min(results.len().saturating_sub(1));
                }
                "enter" => {
                    if let Some(result) = results.get(self.search_selected) {
                        self.open_search_result(result.nav, window, cx);
                    }
                }
                "escape" => self.clear_search(cx),
                _ => return,
            }
            cx.stop_propagation();
            cx.notify();
            return;
        }

        let Some(command) = self.recording_shortcut else {
            return;
        };
        cx.stop_propagation();
        if event.is_held {
            return;
        }

        let key = event.keystroke.unparse();
        if key == "escape" {
            self.recording_shortcut = None;
            self.shortcut_error = None;
            cx.notify();
            return;
        }

        let Some(keys) = normalize_shortcut_keys(std::slice::from_ref(&key)) else {
            let strings = cx.global::<I18nManager>().strings();
            self.shortcut_error = Some(Self::format_template(
                &strings.preferences_shortcut_invalid_template,
                "{shortcut}",
                &key,
            ));
            cx.notify();
            return;
        };

        if let Some(conflict) = shortcut_conflict_for(command, &keys, &self.keybindings) {
            let strings = cx.global::<I18nManager>().strings();
            let label = Self::shortcut_command_label(conflict.command, strings);
            self.shortcut_error = Some(Self::format_template(
                &strings.preferences_shortcut_conflict_template,
                "{command}",
                &label,
            ));
            cx.notify();
            return;
        }

        if let Some(definition) = shortcut_definitions()
            .iter()
            .find(|definition| definition.command == command)
        {
            let defaults = definition
                .default_keys
                .current()
                .iter()
                .map(|key| key.to_string())
                .collect::<Vec<_>>();
            if keys == defaults {
                self.keybindings.remove(definition.id);
            } else {
                self.keybindings.insert(definition.id.to_string(), keys);
            }
        }
        self.recording_shortcut = None;
        self.shortcut_error = None;
        cx.notify();
    }

    pub(super) fn shortcut_chip(label: &str, theme: &Theme) -> impl IntoElement {
        let c = &theme.colors;
        let d = &theme.dimensions;
        let t = &theme.typography;
        div()
            .min_w(px(58.0))
            .h(px(24.0))
            .px(px(8.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px((d.menu_item_radius - 1.0).max(0.0)))
            .border(px(d.dialog_border_width))
            .border_color(c.dialog_border)
            .bg(c.code_bg)
            .text_size(px((t.dialog_body_size - 1.0).max(10.0)))
            .text_color(c.code_text)
            .child(SharedString::from(label.to_string()))
    }

    pub(super) fn shortcut_action_button(
        id: impl Into<ElementId>,
        label: String,
        theme: &Theme,
        on_click: impl Fn(&mut Self, &ClickEvent, &mut Window, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let c = &theme.colors;
        let d = &theme.dimensions;
        let t = &theme.typography;
        div()
            .id(id)
            .h(px(28.0))
            .px(px(10.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px((d.dialog_radius - 5.0).max(0.0)))
            .border(px(d.dialog_border_width))
            .border_color(c.dialog_border)
            .bg(c.dialog_secondary_button_bg)
            .hover(|this| this.bg(c.dialog_secondary_button_hover))
            .cursor_pointer()
            .text_size(px((t.dialog_button_size - 1.0).max(10.0)))
            .font_weight(t.dialog_button_weight.to_font_weight())
            .text_color(c.dialog_secondary_button_text)
            .child(label)
            .on_click(cx.listener(on_click))
    }

    pub(super) fn render_shortcut_row(
        &self,
        definition: ShortcutDefinition,
        theme: &Theme,
        strings: &crate::i18n::I18nStrings,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let c = &theme.colors;
        let d = &theme.dimensions;
        let t = &theme.typography;
        let is_recording = self.recording_shortcut == Some(definition.command);
        let keys = resolved_shortcut_keys(&self.keybindings, definition.command);
        let label = Self::shortcut_command_label(definition.command, strings);
        let command = definition.command;

        let mut chips = div().flex().flex_wrap().gap(px(6.0));
        if is_recording {
            chips = chips.child(Self::shortcut_chip(
                &strings.preferences_shortcut_recording,
                theme,
            ));
        } else {
            for key in keys {
                let display_key = format_shortcut_for_display(&key);
                chips = chips.child(Self::shortcut_chip(&display_key, theme));
            }
        }

        div()
            .w_full()
            .min_h(px(42.0))
            .px(px(10.0))
            .py(px(6.0))
            .flex()
            .items_center()
            .justify_between()
            .gap(px(12.0))
            .rounded(px(d.menu_item_radius))
            .bg(c.dialog_surface)
            .child(
                div()
                    .min_w(px(144.0))
                    .text_size(px(t.dialog_body_size))
                    .text_color(c.dialog_body)
                    .child(label),
            )
            .child(div().flex_1().child(chips))
            .child(
                div()
                    .flex()
                    .gap(px(6.0))
                    .child(Self::shortcut_action_button(
                        ("preferences-shortcut-record", definition.command as u32),
                        strings.preferences_shortcut_record.clone(),
                        theme,
                        move |this, event, window, cx| {
                            this.begin_recording_shortcut(command, event, window, cx)
                        },
                        cx,
                    ))
                    .child(Self::shortcut_action_button(
                        ("preferences-shortcut-reset", definition.command as u32),
                        strings.preferences_shortcut_reset.clone(),
                        theme,
                        move |this, event, window, cx| {
                            this.reset_shortcut(command, event, window, cx)
                        },
                        cx,
                    )),
            )
    }

    pub(super) fn render_shortcuts_page(
        &self,
        theme: &Theme,
        strings: &crate::i18n::I18nStrings,
        cx: &mut Context<Self>,
    ) -> Div {
        let c = &theme.colors;
        let d = &theme.dimensions;
        let t = &theme.typography;
        let mut content = div()
            .id("preferences-shortcuts-scroll")
            .debug_selector(|| "preferences-shortcuts-scroll".to_owned())
            .w_full()
            .flex_1()
            .min_h(px(0.0))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(18.0))
            .pr(px(4.0));

        let categories = [
            ShortcutCategory::File,
            ShortcutCategory::Edit,
            ShortcutCategory::Navigation,
            ShortcutCategory::Formatting,
            ShortcutCategory::Block,
            ShortcutCategory::Other,
        ];

        for category in categories {
            let mut group = div().w_full().flex().flex_col().gap(px(8.0)).child(
                div()
                    .w_full()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .child(
                        div()
                            .text_size(px(t.dialog_body_size))
                            .font_weight(t.dialog_button_weight.to_font_weight())
                            .text_color(c.dialog_title)
                            .child(Self::shortcut_category_label(category, strings)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .h(px(d.dialog_border_width.max(1.0)))
                            .bg(c.dialog_border),
                    ),
            );
            for definition in shortcut_definitions()
                .iter()
                .copied()
                .filter(|definition| definition.category == category)
            {
                group = group.child(self.render_shortcut_row(definition, theme, strings, cx));
            }
            content = content.child(group);
        }

        let mut page = div()
            .w_full()
            .h_full()
            .flex_1()
            .min_h(px(0.0))
            .flex()
            .flex_col()
            .items_center()
            .gap(px(8.0));
        if let Some(error) = &self.shortcut_error {
            page = page.child(
                div()
                    .w_full()
                    .flex_shrink_0()
                    .text_size(px(t.dialog_body_size))
                    .text_color(c.dialog_danger_button_bg)
                    .child(error.clone()),
            );
        }
        page.child(content)
    }
}
