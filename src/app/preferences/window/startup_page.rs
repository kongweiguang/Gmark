// @author kongweiguang

//! Startup and document-loading preference controls.

use super::*;

impl PreferencesWindow {
    pub(super) fn render_startup_page(
        &self,
        theme: &Theme,
        strings: &crate::i18n::I18nStrings,
        cx: &mut Context<Self>,
    ) -> Div {
        let auto_check_label = if cx
            .global::<I18nManager>()
            .current_language_id()
            .starts_with("zh")
        {
            "自动检查软件更新"
        } else {
            "Automatically check for updates"
        };
        let selected = match self.startup_open {
            StartupOpenPreference::NewFile => strings.preferences_startup_new_file.clone(),
            StartupOpenPreference::LastOpenedFile => {
                strings.preferences_startup_last_opened_file.clone()
            }
        };
        let dropdown = div()
            .relative()
            .w(px(280.0))
            .h(px(32.0))
            .flex_shrink_0()
            .child(self.dropdown_button(
                "preferences-startup-dropdown",
                selected,
                PreferencesDropdown::Startup,
                theme,
                cx,
            ));
        let startup_list = if self.startup_dropdown_open {
            let new_file_label = strings.preferences_startup_new_file.clone();
            let last_file_label = strings.preferences_startup_last_opened_file.clone();
            let list = Self::dropdown_list(theme)
                .right_0()
                .id("preferences-startup-dropdown-list")
                .debug_selector(|| "preferences-startup-dropdown-list".to_owned())
                .child(Self::dropdown_item(
                    "preferences-startup-new-file",
                    new_file_label,
                    self.startup_open == StartupOpenPreference::NewFile,
                    self.dropdown_selected_indices[PreferencesDropdown::Startup.index()] == 0,
                    theme,
                    |this, _, _, cx| {
                        this.commit_dropdown_selection(PreferencesDropdown::Startup, 0, cx);
                    },
                    cx,
                ))
                .child(Self::dropdown_item(
                    "preferences-startup-last-opened-file",
                    last_file_label,
                    self.startup_open == StartupOpenPreference::LastOpenedFile,
                    self.dropdown_selected_indices[PreferencesDropdown::Startup.index()] == 1,
                    theme,
                    |this, _, _, cx| {
                        this.commit_dropdown_selection(PreferencesDropdown::Startup, 1, cx);
                    },
                    cx,
                ));
            Some(list)
        } else {
            None
        };
        let auto_save_label = match self.auto_save {
            AutoSavePreference::Off => strings.preferences_auto_save_off.clone(),
            AutoSavePreference::AfterDelay => strings.preferences_auto_save_after_delay.clone(),
        };
        let auto_save_dropdown = div()
            .relative()
            .w(px(280.0))
            .h(px(32.0))
            .flex_shrink_0()
            .child(self.dropdown_button(
                "preferences-auto-save-dropdown",
                auto_save_label,
                PreferencesDropdown::AutoSave,
                theme,
                cx,
            ));
        let auto_save_list = if self.auto_save_dropdown_open {
            let mut list = Self::dropdown_list(theme)
                .top(px(88.0))
                .right_0()
                .id("preferences-auto-save-dropdown-list")
                .debug_selector(|| "preferences-auto-save-dropdown-list".to_owned());
            for (index, option) in [AutoSavePreference::Off, AutoSavePreference::AfterDelay]
                .into_iter()
                .enumerate()
            {
                let label = match option {
                    AutoSavePreference::Off => strings.preferences_auto_save_off.clone(),
                    AutoSavePreference::AfterDelay => {
                        strings.preferences_auto_save_after_delay.clone()
                    }
                };
                list = list.child(Self::dropdown_item(
                    ("preferences-auto-save-option", index),
                    label,
                    self.auto_save == option,
                    self.dropdown_selected_indices[PreferencesDropdown::AutoSave.index()] == index,
                    theme,
                    move |this, _, _, cx| {
                        this.commit_dropdown_selection(PreferencesDropdown::AutoSave, index, cx);
                    },
                    cx,
                ));
            }
            Some(list)
        } else {
            None
        };
        div()
            .relative()
            .w_full()
            .max_w(px(PREFERENCES_FORM_WIDTH))
            .flex()
            .flex_col()
            .gap(px(20.0))
            .child(
                self.labeled_row(&strings.preferences_startup_option, dropdown, theme)
                    .debug_selector(|| "preferences-startup-row".to_owned()),
            )
            .child(
                self.labeled_row(
                    &strings.preferences_auto_save_option,
                    auto_save_dropdown,
                    theme,
                )
                .debug_selector(|| "preferences-auto-save-row".to_owned()),
            )
            .child(
                div()
                    .w_full()
                    .max_w(px(PREFERENCES_FORM_WIDTH))
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(px(theme.typography.dialog_body_size))
                            .text_color(theme.colors.dialog_body)
                            .child(auto_check_label),
                    )
                    .child(self.preference_switch(
                        PreferencesSwitch::AutoCheckUpdates,
                        self.auto_check_updates,
                        cx,
                    )),
            )
            .child(
                div()
                    .w_full()
                    .max_w(px(PREFERENCES_FORM_WIDTH))
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(px(theme.typography.dialog_body_size))
                            .text_color(theme.colors.dialog_body)
                            .child(strings.preferences_spell_check.clone()),
                    )
                    .child(self.preference_switch(
                        PreferencesSwitch::SpellCheck,
                        self.spell_check,
                        cx,
                    )),
            )
            .child(
                div()
                    .w_full()
                    .max_w(px(PREFERENCES_FORM_WIDTH))
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(px(theme.typography.dialog_body_size))
                            .text_color(theme.colors.dialog_body)
                            .child(strings.preferences_show_hidden_files.clone()),
                    )
                    .child(self.preference_switch(
                        PreferencesSwitch::ShowHiddenFiles,
                        self.show_hidden_files,
                        cx,
                    )),
            )
            .child(
                div()
                    .w_full()
                    .pt(px(8.0))
                    .border_t(px(theme.dimensions.dialog_border_width))
                    .border_color(theme.colors.dialog_border)
                    .text_size(px(theme.typography.dialog_title_size))
                    .font_weight(theme.typography.dialog_title_weight.to_font_weight())
                    .text_color(theme.colors.dialog_title)
                    .child(strings.preferences_document_loading.clone()),
            )
            .child(self.labeled_row(
                &strings.preferences_document_max_resident_mib,
                self.numeric_stepper(
                    "preferences-document-resident-mib",
                    PreferencesNumericInput::ResidentMib,
                    PreferencesStepperControl::ResidentMibDecrease,
                    PreferencesStepperControl::ResidentMibIncrease,
                    "MiB",
                    theme,
                    cx,
                ),
                theme,
            ))
            .children(self.document_loading.has_invalid_override().then(|| {
                div()
                    .text_size(px(theme.typography.dialog_body_size))
                    .text_color(theme.colors.dialog_danger_button_bg)
                    .child(strings.preferences_document_loading_invalid.clone())
            }))
            .child(
                div()
                    .text_size(px(theme.typography.dialog_body_size))
                    .text_color(theme.colors.dialog_muted)
                    .child(strings.preferences_document_loading_next_open.clone()),
            )
            // 浮层最后绘制，确保不会被后续设置行覆盖。
            .children(startup_list)
            .children(auto_save_list)
    }
}
