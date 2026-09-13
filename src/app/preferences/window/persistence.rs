// @author kongweiguang

//! Save, cancel, and restore behavior for the preferences window.

use super::*;

impl PreferencesWindow {
    pub(super) fn restore_saved_theme(&mut self, cx: &mut Context<Self>) {
        if self.theme_appearance == self.saved_theme_appearance
            && self.theme_palette == self.saved_theme_palette
            && self.editor_font_size == self.saved_editor_font_size
            && self.editor_line_height_percent == self.saved_editor_line_height_percent
            && self.editor_content_width == self.saved_editor_content_width
            && self.editor_font_family == self.saved_editor_font_family
        {
            return;
        }
        let saved_appearance = self.saved_theme_appearance;
        let saved_palette = self.saved_theme_palette;
        let platform_appearance = cx.window_appearance();
        cx.update_global::<ThemeManager, _>(|theme_manager, _cx| {
            theme_manager.set_theme_preference(
                saved_appearance,
                saved_palette,
                platform_appearance,
            );
            theme_manager.set_editor_typography(
                self.saved_editor_font_size,
                self.saved_editor_line_height_percent,
            );
            theme_manager.set_editor_content_width(self.saved_editor_content_width);
        });
        self.theme_appearance = self.saved_theme_appearance;
        self.theme_palette = self.saved_theme_palette;
        self.editor_font_size = self.saved_editor_font_size;
        self.editor_line_height_percent = self.saved_editor_line_height_percent;
        self.editor_content_width = self.saved_editor_content_width;
        self.editor_font_family = self.saved_editor_font_family.clone();
        let restored_font = self.saved_editor_font_family.clone();
        cx.update_global::<EditorSettings, _>(|settings, _cx| {
            settings.editor_font_family = restored_font;
        });
        cx.refresh_windows();
        cx.notify();
    }

    pub(super) fn cancel(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.restore_saved_visual_accessibility(cx);
        self.restore_saved_theme(cx);
        window.remove_window();
    }

    pub(super) fn cancel_from_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(event.keystroke.key.as_str(), "enter" | "space") {
            self.restore_saved_visual_accessibility(cx);
            self.restore_saved_theme(cx);
            window.remove_window();
            cx.stop_propagation();
        }
    }

    pub(super) fn on_titlebar_close(
        &mut self,
        event: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.standard_click() {
            self.restore_saved_visual_accessibility(cx);
            self.restore_saved_theme(cx);
            window.remove_window();
        }
    }

    pub(super) fn save(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.save_changes(window, cx);
    }

    pub(super) fn save_from_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(event.keystroke.key.as_str(), "enter" | "space") {
            self.save_changes(window, cx);
            cx.stop_propagation();
        }
    }

    fn save_changes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.has_unsaved_changes() || self.has_invalid_numeric_input(cx) {
            return;
        }

        let mut preferences = match save_preferences_from_window(
            self.startup_open,
            self.auto_check_updates,
            self.show_hidden_files,
            self.auto_save,
            self.spell_check,
            self.auto_pair_brackets,
            self.auto_pair_markdown,
            self.code_folding,
            self.format_on_save,
            self.editor_font_size,
            self.editor_line_height_percent,
            self.editor_content_width,
            &self.editor_font_family,
            self.show_tab_bar_actions,
            self.theme_appearance,
            self.theme_palette,
            &self.selected_language_id,
            self.image_paste_behavior,
            self.keybindings.clone(),
            &self.document_loading,
            &StatusBarPreferences {
                enabled: self.status_bar_enabled,
                show_word_count: self.status_bar_show_word_count,
                show_cursor_position: self.status_bar_show_cursor_position,
                show_sidebar_toggle: self.status_bar_show_sidebar_toggle,
                show_mode_switch: self.status_bar_show_mode_switch,
                custom_buttons: self.status_bar_custom_buttons.clone(),
            },
        ) {
            Ok(preferences) => preferences,
            Err(err) => {
                let strings = cx.global::<I18nManager>().strings().clone();
                let ok = strings.info_dialog_ok;
                let buttons = [ok.as_str()];
                let _ = window.prompt(
                    PromptLevel::Critical,
                    &strings.preferences_save_failed_title,
                    Some(&err.to_string()),
                    &buttons,
                    cx,
                );
                return;
            }
        };

        if preferences.visual_accessibility != self.visual_accessibility {
            preferences = match super::super::storage::save_visual_accessibility_preferences(
                self.visual_accessibility,
            ) {
                Ok(preferences) => preferences,
                Err(err) => {
                    let strings = cx.global::<I18nManager>().strings().clone();
                    let ok = strings.info_dialog_ok;
                    let buttons = [ok.as_str()];
                    let _ = window.prompt(
                        PromptLevel::Critical,
                        &strings.preferences_save_failed_title,
                        Some(&err.to_string()),
                        &buttons,
                        cx,
                    );
                    return;
                }
            };
        }

        self.apply_saved_preferences(preferences, window, cx);
    }

    pub(super) fn apply_saved_preferences(
        &mut self,
        preferences: AppPreferences,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let platform_appearance = window.appearance();
        cx.update_global::<ThemeManager, _>(|theme_manager, _cx| {
            theme_manager.set_theme_preference(
                preferences.theme_appearance,
                preferences.theme_palette,
                platform_appearance,
            );
            theme_manager.set_editor_typography(
                preferences.editor_font_size,
                preferences.editor_line_height_percent,
            );
            theme_manager.set_editor_content_width(preferences.editor_content_width);
        });
        cx.update_global::<I18nManager, _>(|i18n_manager, _cx| {
            let _ = i18n_manager.set_language_by_id(&preferences.default_language_id);
        });
        cx.clear_key_bindings();
        install_keybindings(cx, &preferences.keybindings);
        crate::app_menu::install_menus(cx);
        cx.update_global::<EditorSettings, _>(|settings, _cx| {
            settings.show_hidden_files = preferences.show_hidden_files;
            settings.auto_save = preferences.auto_save;
            settings.spell_check = preferences.spell_check;
            settings.auto_pair_brackets = preferences.auto_pair_brackets;
            settings.auto_pair_markdown = preferences.auto_pair_markdown;
            settings.code_folding = preferences.code_folding;
            settings.format_on_save = preferences.format_on_save;
            settings.editor_font_family = preferences.editor_font_family.clone();
            settings.show_tab_bar_actions = preferences.show_tab_bar_actions;
            settings.status_bar_settings.status_bar_enabled = preferences.status_bar.enabled;
            settings.status_bar_settings.status_bar_show_word_count =
                preferences.status_bar.show_word_count;
            settings.status_bar_settings.status_bar_show_cursor_position =
                preferences.status_bar.show_cursor_position;
            settings.status_bar_settings.status_bar_show_sidebar_toggle =
                preferences.status_bar.show_sidebar_toggle;
            settings.status_bar_settings.status_bar_show_mode_switch =
                preferences.status_bar.show_mode_switch;
            settings.status_bar_settings.custom_buttons = preferences.status_bar.custom_buttons;
        });
        crate::updater::UpdateCoordinator::set_auto_check(preferences.auto_check_updates, cx);
        if cx
            .try_global::<crate::ui::visual_preferences::VisualPreferencesManager>()
            .is_some()
        {
            cx.update_global::<crate::ui::visual_preferences::VisualPreferencesManager, _>(
                |manager, _cx| {
                    manager.set_preferences(preferences.visual_accessibility);
                },
            );
        }
        cx.refresh_windows();
        window.activate_window();
        self.focus_handle.focus(window);
        self.saved_startup_open = self.startup_open;
        self.saved_auto_check_updates = self.auto_check_updates;
        self.saved_show_hidden_files = self.show_hidden_files;
        self.saved_auto_save = self.auto_save;
        self.saved_spell_check = self.spell_check;
        self.saved_auto_pair_brackets = self.auto_pair_brackets;
        self.saved_auto_pair_markdown = self.auto_pair_markdown;
        self.saved_code_folding = self.code_folding;
        self.saved_format_on_save = self.format_on_save;
        self.saved_editor_font_size = self.editor_font_size;
        self.saved_editor_line_height_percent = self.editor_line_height_percent;
        self.saved_editor_content_width = self.editor_content_width;
        self.saved_editor_font_family = self.editor_font_family.clone();
        self.saved_show_tab_bar_actions = self.show_tab_bar_actions;
        self.saved_theme_appearance = self.theme_appearance;
        self.saved_theme_palette = self.theme_palette;
        self.saved_language_id = self.selected_language_id.clone();
        self.saved_image_paste_behavior = self.image_paste_behavior;
        self.saved_keybindings = normalize_shortcut_config(&self.keybindings);
        self.saved_document_loading = self.document_loading.clone();
        self.visual_accessibility = preferences.visual_accessibility;
        self.saved_visual_accessibility = preferences.visual_accessibility;
        self.saved_status_bar_enabled = self.status_bar_enabled;
        self.saved_status_bar_show_word_count = self.status_bar_show_word_count;
        self.saved_status_bar_show_cursor_position = self.status_bar_show_cursor_position;
        self.saved_status_bar_show_sidebar_toggle = self.status_bar_show_sidebar_toggle;
        self.saved_status_bar_show_mode_switch = self.status_bar_show_mode_switch;
        cx.notify();
    }
}
