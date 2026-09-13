// @author kongweiguang

use super::*;

impl PreferencesWindow {
    pub(super) fn new(preferences: AppPreferences, cx: &mut Context<Self>) -> Self {
        let language_options = cx.global::<I18nManager>().available_languages().to_vec();
        let selected_language_id = if language_options
            .iter()
            .any(|entry| entry.id == preferences.default_language_id)
        {
            preferences.default_language_id.clone()
        } else {
            DEFAULT_LANGUAGE_ID.into()
        };
        let startup_open = preferences.startup_open;
        let auto_check_updates = preferences.auto_check_updates;
        let show_hidden_files = preferences.show_hidden_files;
        let auto_save = preferences.auto_save;
        let spell_check = preferences.spell_check;
        let auto_pair_brackets = preferences.auto_pair_brackets;
        let auto_pair_markdown = preferences.auto_pair_markdown;
        let code_folding = preferences.code_folding;
        let format_on_save = preferences.format_on_save;
        let editor_font_size = preferences.editor_font_size;
        let editor_line_height_percent = preferences.editor_line_height_percent;
        let editor_content_width = preferences.editor_content_width;
        let editor_font_family = preferences.editor_font_family;
        let show_tab_bar_actions = preferences.show_tab_bar_actions;
        let image_paste_behavior = preferences.image_paste_behavior;
        let keybindings = preferences.keybindings;
        let document_loading = preferences.document_loading.clone();
        let visual_accessibility = preferences.visual_accessibility;
        // 字体枚举成本较高，偏好设置窗口创建时只读取一次，渲染阶段复用稳定列表。
        let mut font_options = cx.text_system().all_font_names();
        font_options.retain(|font| !font.trim().is_empty());
        font_options.sort_by_key(|font| font.to_lowercase());
        font_options.dedup_by(|left, right| left.eq_ignore_ascii_case(right));
        if !editor_font_family.is_empty()
            && !font_options
                .iter()
                .any(|font| font.eq_ignore_ascii_case(&editor_font_family))
        {
            // 保留旧配置中的自定义字体，即使当前系统暂时没有枚举到它。
            font_options.insert(0, editor_font_family.clone());
        }
        // 空字符串是持久化契约中的“跟随系统字体”。
        font_options.insert(0, String::new());
        let search_placeholder = cx
            .global::<I18nManager>()
            .strings()
            .preferences_search_placeholder
            .clone();
        let search_input = cx.new(|cx| {
            let mut block = Block::with_record(cx, BlockRecord::paragraph(String::new()));
            block.set_compact_source_host();
            block.set_input_placeholder(search_placeholder);
            block
        });
        cx.subscribe(&search_input, Self::on_search_input_event)
            .detach();
        let numeric_values = [
            u64::from(editor_font_size),
            u64::from(editor_line_height_percent),
            u64::from(editor_content_width),
            document_loading.effective_max_resident_mib(),
        ];
        let numeric_inputs = std::array::from_fn(|index| {
            let value = numeric_values[index].to_string();
            cx.new(move |cx| {
                let mut block = Block::with_record(cx, BlockRecord::paragraph(value));
                block.set_compact_source_host();
                block
            })
        });
        for input in &numeric_inputs {
            cx.subscribe(input, Self::on_numeric_input_event).detach();
        }
        Self {
            nav: PreferencesNav::File,
            startup_open,
            auto_check_updates,
            show_hidden_files,
            auto_save,
            spell_check,
            auto_pair_brackets,
            auto_pair_markdown,
            code_folding,
            format_on_save,
            editor_font_size,
            editor_line_height_percent,
            editor_content_width,
            editor_font_family: editor_font_family.clone(),
            show_tab_bar_actions,
            theme_appearance: preferences.theme_appearance,
            theme_palette: preferences.theme_palette,
            selected_language_id: selected_language_id.clone(),
            image_paste_behavior,
            keybindings: keybindings.clone(),
            document_loading: document_loading.clone(),
            saved_startup_open: startup_open,
            saved_auto_check_updates: auto_check_updates,
            saved_show_hidden_files: show_hidden_files,
            saved_auto_save: auto_save,
            saved_spell_check: spell_check,
            saved_auto_pair_brackets: auto_pair_brackets,
            saved_auto_pair_markdown: auto_pair_markdown,
            saved_code_folding: code_folding,
            saved_format_on_save: format_on_save,
            saved_editor_font_size: editor_font_size,
            saved_editor_line_height_percent: editor_line_height_percent,
            saved_editor_content_width: editor_content_width,
            saved_editor_font_family: editor_font_family,
            saved_show_tab_bar_actions: show_tab_bar_actions,
            saved_theme_appearance: preferences.theme_appearance,
            saved_theme_palette: preferences.theme_palette,
            saved_language_id: selected_language_id,
            saved_image_paste_behavior: image_paste_behavior,
            saved_keybindings: keybindings,
            saved_document_loading: document_loading,
            visual_accessibility,
            saved_visual_accessibility: visual_accessibility,
            language_options,
            font_options,
            focus_handle: cx.focus_handle(),
            action_focus_handles: std::array::from_fn(|_| cx.focus_handle()),
            nav_focus_handles: std::array::from_fn(|_| cx.focus_handle()),
            dropdown_focus_handles: std::array::from_fn(|_| cx.focus_handle()),
            theme_appearance_focus_handles: std::array::from_fn(|_| cx.focus_handle()),
            theme_palette_focus_handles: std::array::from_fn(|_| cx.focus_handle()),
            accessibility_focus_handles: std::array::from_fn(|_| {
                std::array::from_fn(|_| cx.focus_handle())
            }),
            dropdown_selected_indices: [0; PreferencesDropdown::COUNT],
            switch_focus_handles: std::array::from_fn(|_| cx.focus_handle()),
            stepper_focus_handles: std::array::from_fn(|_| cx.focus_handle()),
            numeric_inputs,
            search_input,
            search_selected: 0,
            startup_dropdown_open: false,
            auto_save_dropdown_open: false,
            language_dropdown_open: false,
            image_dropdown_open: false,
            font_dropdown_open: false,
            recording_shortcut: None,
            shortcut_error: None,
            status_bar_enabled: preferences.status_bar.enabled,
            status_bar_show_word_count: preferences.status_bar.show_word_count,
            status_bar_show_cursor_position: preferences.status_bar.show_cursor_position,
            status_bar_show_sidebar_toggle: preferences.status_bar.show_sidebar_toggle,
            status_bar_show_mode_switch: preferences.status_bar.show_mode_switch,
            status_bar_custom_buttons: preferences.status_bar.custom_buttons.clone(),
            saved_status_bar_enabled: preferences.status_bar.enabled,
            saved_status_bar_show_word_count: preferences.status_bar.show_word_count,
            saved_status_bar_show_cursor_position: preferences.status_bar.show_cursor_position,
            saved_status_bar_show_sidebar_toggle: preferences.status_bar.show_sidebar_toggle,
            saved_status_bar_show_mode_switch: preferences.status_bar.show_mode_switch,
        }
    }

    pub(super) fn on_search_input_event(
        &mut self,
        _: Entity<Block>,
        event: &BlockEvent,
        cx: &mut Context<Self>,
    ) {
        if matches!(event, BlockEvent::Changed) {
            self.search_selected = 0;
            cx.notify();
        }
    }

    pub(super) fn search_query(&self, cx: &App) -> String {
        self.search_input
            .read(cx)
            .display_text()
            .trim()
            .to_lowercase()
    }

    pub(super) fn preference_search_items(
        &self,
        strings: &crate::i18n::I18nStrings,
    ) -> Vec<PreferenceSearchItem> {
        let mut items = vec![
            PreferenceSearchItem {
                nav: PreferencesNav::File,
                category: strings.preferences_nav_file.clone(),
                label: strings.preferences_startup_option.clone(),
            },
            PreferenceSearchItem {
                nav: PreferencesNav::File,
                category: strings.preferences_nav_file.clone(),
                label: strings.preferences_auto_save_option.clone(),
            },
            PreferenceSearchItem {
                nav: PreferencesNav::File,
                category: strings.preferences_nav_file.clone(),
                label: strings.preferences_spell_check.clone(),
            },
            PreferenceSearchItem {
                nav: PreferencesNav::File,
                category: strings.preferences_nav_file.clone(),
                label: strings.preferences_show_hidden_files.clone(),
            },
            PreferenceSearchItem {
                nav: PreferencesNav::File,
                category: strings.preferences_nav_file.clone(),
                label: strings.preferences_document_loading.clone(),
            },
            PreferenceSearchItem {
                nav: PreferencesNav::File,
                category: strings.preferences_nav_file.clone(),
                label: strings.preferences_document_max_resident_mib.clone(),
            },
            PreferenceSearchItem {
                nav: PreferencesNav::Editor,
                category: strings.preferences_nav_editor.clone(),
                label: strings.preferences_editor_font_size.clone(),
            },
            PreferenceSearchItem {
                nav: PreferencesNav::Editor,
                category: strings.preferences_nav_editor.clone(),
                label: strings.preferences_auto_pair_brackets.clone(),
            },
            PreferenceSearchItem {
                nav: PreferencesNav::Editor,
                category: strings.preferences_nav_editor.clone(),
                label: strings.preferences_auto_pair_markdown.clone(),
            },
            PreferenceSearchItem {
                nav: PreferencesNav::Editor,
                category: strings.preferences_nav_editor.clone(),
                label: strings.preferences_editor_line_height.clone(),
            },
            PreferenceSearchItem {
                nav: PreferencesNav::Editor,
                category: strings.preferences_nav_editor.clone(),
                label: strings.preferences_editor_content_width.clone(),
            },
            PreferenceSearchItem {
                nav: PreferencesNav::Editor,
                category: strings.preferences_nav_editor.clone(),
                label: strings.preferences_editor_font_family.clone(),
            },
            PreferenceSearchItem {
                nav: PreferencesNav::Editor,
                category: strings.preferences_nav_editor.clone(),
                label: strings.preferences_show_tab_bar_actions.clone(),
            },
            PreferenceSearchItem {
                nav: PreferencesNav::Theme,
                category: strings.preferences_nav_theme.clone(),
                label: strings.preferences_theme_appearance.clone(),
            },
            PreferenceSearchItem {
                nav: PreferencesNav::Theme,
                category: strings.preferences_nav_theme.clone(),
                label: strings.preferences_theme_dark.clone(),
            },
            PreferenceSearchItem {
                nav: PreferencesNav::Theme,
                category: strings.preferences_nav_theme.clone(),
                label: strings.preferences_theme_light.clone(),
            },
            PreferenceSearchItem {
                nav: PreferencesNav::Theme,
                category: strings.preferences_nav_theme.clone(),
                label: strings.preferences_follow_system_theme.clone(),
            },
            PreferenceSearchItem {
                nav: PreferencesNav::Theme,
                category: strings.preferences_nav_theme.clone(),
                label: strings.preferences_palette.clone(),
            },
            PreferenceSearchItem {
                nav: PreferencesNav::Theme,
                category: strings.preferences_nav_theme.clone(),
                label: strings.menu_language.clone(),
            },
            PreferenceSearchItem {
                nav: PreferencesNav::Theme,
                category: strings.preferences_accessibility_title.clone(),
                label: strings.preferences_reduced_motion.clone(),
            },
            PreferenceSearchItem {
                nav: PreferencesNav::Theme,
                category: strings.preferences_accessibility_title.clone(),
                label: strings.preferences_reduced_transparency.clone(),
            },
            PreferenceSearchItem {
                nav: PreferencesNav::Theme,
                category: strings.preferences_accessibility_title.clone(),
                label: strings.preferences_high_contrast.clone(),
            },
            PreferenceSearchItem {
                nav: PreferencesNav::Image,
                category: strings.preferences_nav_image.clone(),
                label: strings.preferences_image_insert_behavior.clone(),
            },
            PreferenceSearchItem {
                nav: PreferencesNav::StatusBar,
                category: strings.preferences_nav_status_bar.clone(),
                label: strings.preferences_status_bar_enabled.clone(),
            },
            PreferenceSearchItem {
                nav: PreferencesNav::StatusBar,
                category: strings.preferences_nav_status_bar.clone(),
                label: strings.preferences_status_bar_show_word_count.clone(),
            },
            PreferenceSearchItem {
                nav: PreferencesNav::StatusBar,
                category: strings.preferences_nav_status_bar.clone(),
                label: strings.preferences_status_bar_show_cursor_position.clone(),
            },
            PreferenceSearchItem {
                nav: PreferencesNav::StatusBar,
                category: strings.preferences_nav_status_bar.clone(),
                label: strings.preferences_status_bar_show_sidebar_toggle.clone(),
            },
            PreferenceSearchItem {
                nav: PreferencesNav::StatusBar,
                category: strings.preferences_nav_status_bar.clone(),
                label: strings.preferences_status_bar_show_mode_switch.clone(),
            },
        ];
        items.extend(
            shortcut_definitions()
                .iter()
                .map(|definition| PreferenceSearchItem {
                    nav: PreferencesNav::Shortcuts,
                    category: strings.preferences_nav_shortcuts.clone(),
                    label: Self::shortcut_command_label(definition.command, strings),
                }),
        );
        items
    }
}
