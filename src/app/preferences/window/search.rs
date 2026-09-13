// @author kongweiguang

//! Search query and result selection behavior.

use super::*;

impl PreferencesWindow {
    pub(super) fn preference_search_results(
        &self,
        strings: &crate::i18n::I18nStrings,
        cx: &App,
    ) -> Vec<PreferenceSearchItem> {
        let query = self.search_query(cx);
        if query.is_empty() {
            return Vec::new();
        }
        let tokens = query.split_whitespace().collect::<Vec<_>>();
        self.preference_search_items(strings)
            .into_iter()
            .filter(|item| {
                // 资源页改名后保留旧的 Image 搜索词，让既有使用习惯和旧配置仍能
                // 在迁移后定位到同一项设置。
                let legacy_alias = matches!(item.nav, PreferencesNav::Image)
                    .then_some(" image 图片")
                    .unwrap_or_default();
                let searchable =
                    format!("{} {}{}", item.category, item.label, legacy_alias).to_lowercase();
                tokens.iter().all(|token| searchable.contains(token))
            })
            .collect()
    }

    pub(super) fn clear_search(&mut self, cx: &mut Context<Self>) {
        let input = self.search_input.clone();
        input.update(cx, |input, cx| {
            let len = input.visible_len();
            input.replace_text_in_visible_range(0..len, "", None, false, cx);
        });
        self.search_selected = 0;
    }

    pub(super) fn clear_search_from_button(
        &mut self,
        _: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.clear_search(cx);
        self.search_input.read(cx).focus_handle.focus(window);
    }

    pub(super) fn clear_search_from_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(event.keystroke.key.as_str(), "enter" | "space") {
            self.clear_search(cx);
            self.search_input.read(cx).focus_handle.focus(window);
            cx.stop_propagation();
        }
    }

    pub(super) fn open_search_result(
        &mut self,
        nav: PreferencesNav,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_nav(nav, cx);
        self.clear_search(cx);
        self.focus_handle.focus(window);
    }

    pub(super) fn select_nav(&mut self, nav: PreferencesNav, cx: &mut Context<Self>) {
        self.nav = nav;
        self.close_all_dropdowns();
        self.recording_shortcut = None;
        self.shortcut_error = None;
        cx.notify();
    }

    pub(super) fn has_unsaved_changes(&self) -> bool {
        self.startup_open != self.saved_startup_open
            || self.auto_check_updates != self.saved_auto_check_updates
            || self.show_hidden_files != self.saved_show_hidden_files
            || self.auto_save != self.saved_auto_save
            || self.spell_check != self.saved_spell_check
            || self.auto_pair_brackets != self.saved_auto_pair_brackets
            || self.auto_pair_markdown != self.saved_auto_pair_markdown
            || self.code_folding != self.saved_code_folding
            || self.format_on_save != self.saved_format_on_save
            || self.editor_font_size != self.saved_editor_font_size
            || self.editor_line_height_percent != self.saved_editor_line_height_percent
            || self.editor_content_width != self.saved_editor_content_width
            || self.editor_font_family != self.saved_editor_font_family
            || self.show_tab_bar_actions != self.saved_show_tab_bar_actions
            || self.theme_appearance != self.saved_theme_appearance
            || self.theme_palette != self.saved_theme_palette
            || self.selected_language_id != self.saved_language_id
            || self.image_paste_behavior != self.saved_image_paste_behavior
            || normalize_shortcut_config(&self.keybindings)
                != normalize_shortcut_config(&self.saved_keybindings)
            || self.document_loading != self.saved_document_loading
            || self.visual_accessibility != self.saved_visual_accessibility
            || self.status_bar_enabled != self.saved_status_bar_enabled
            || self.status_bar_show_word_count != self.saved_status_bar_show_word_count
            || self.status_bar_show_cursor_position != self.saved_status_bar_show_cursor_position
            || self.status_bar_show_sidebar_toggle != self.saved_status_bar_show_sidebar_toggle
            || self.status_bar_show_mode_switch != self.saved_status_bar_show_mode_switch
    }
}
