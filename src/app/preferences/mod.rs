// @author kongweiguang

//! Persistent app preferences and the preferences window.

use gpui::prelude::FluentBuilder;
use gpui::*;

use crate::components::{
    Block, BlockEvent, BlockRecord, ShortcutCategory, ShortcutCommand, ShortcutDefinition,
    install_keybindings, normalize_shortcut_config, normalize_shortcut_keys,
    resolved_shortcut_keys, shortcut_conflict_for, shortcut_definitions, switch::Switch,
};
use crate::i18n::{I18nManager, LanguageCatalogEntry, language_id_for_locale_preferences};
use crate::theme::{Theme, ThemeAppearance, ThemeManager, ThemePalette};
use crate::window_chrome::{custom_titlebar_height, gmark_window_options, render_custom_titlebar};

pub(crate) use gmark_config::{
    AppPreferences, AutoSavePreference, DocumentLoadingPreferences, ImagePasteBehavior,
    ResourceInsertBehavior, StartupOpenPreference, StatusBarButton, StatusBarPreferences,
    VisualAccessibilityPreferences,
};

const DEFAULT_LANGUAGE_ID: &str = gmark_config::DEFAULT_LANGUAGE_ID;
#[cfg(test)]
const DEFAULT_EDITOR_FONT_SIZE: u8 = 16;
#[cfg(test)]
const DEFAULT_EDITOR_LINE_HEIGHT_PERCENT: u16 = 160;
const MIN_EDITOR_FONT_SIZE: u8 = 12;
const MAX_EDITOR_FONT_SIZE: u8 = 24;
const MIN_EDITOR_LINE_HEIGHT_PERCENT: u16 = 120;
const MAX_EDITOR_LINE_HEIGHT_PERCENT: u16 = 200;
const EDITOR_LINE_HEIGHT_STEP: u16 = 5;
#[cfg(test)]
const DEFAULT_EDITOR_CONTENT_WIDTH: u16 = 1200;
const MIN_EDITOR_CONTENT_WIDTH: u16 = 680;
const MAX_EDITOR_CONTENT_WIDTH: u16 = 1600;
const EDITOR_CONTENT_WIDTH_STEP: u16 = 40;
const MAX_EDITOR_FONT_FAMILY_CHARS: usize = 80;

fn normalize_editor_line_height_percent(value: u16) -> u16 {
    let clamped = value.clamp(
        MIN_EDITOR_LINE_HEIGHT_PERCENT,
        MAX_EDITOR_LINE_HEIGHT_PERCENT,
    );
    ((clamped + EDITOR_LINE_HEIGHT_STEP / 2) / EDITOR_LINE_HEIGHT_STEP * EDITOR_LINE_HEIGHT_STEP)
        .clamp(
            MIN_EDITOR_LINE_HEIGHT_PERCENT,
            MAX_EDITOR_LINE_HEIGHT_PERCENT,
        )
}

fn normalize_editor_content_width(value: u16) -> u16 {
    let clamped = value.clamp(MIN_EDITOR_CONTENT_WIDTH, MAX_EDITOR_CONTENT_WIDTH);
    ((clamped + EDITOR_CONTENT_WIDTH_STEP / 2) / EDITOR_CONTENT_WIDTH_STEP
        * EDITOR_CONTENT_WIDTH_STEP)
        .clamp(MIN_EDITOR_CONTENT_WIDTH, MAX_EDITOR_CONTENT_WIDTH)
}

fn normalize_editor_font_family(value: &str) -> String {
    value
        .trim()
        .chars()
        .filter(|ch| !ch.is_control())
        .take(MAX_EDITOR_FONT_FAMILY_CHARS)
        .collect()
}
const PREFERENCES_NAV_WIDTH: f32 = 200.0;
const PREFERENCES_FORM_WIDTH: f32 = 560.0;
const CHEVRON_DOWN_ICON: &str = "icon/ui/chevron-down.svg";
const CHECK_ICON: &str = "icon/ui/check.svg";
const SEARCH_ICON: &str = "icon/ui/search.svg";
const CLOSE_ICON: &str = "icon/ui/close.svg";
const MINUS_ICON: &str = "icon/ui/minus.svg";
const PLUS_ICON: &str = "icon/ui/plus.svg";

/// Status Bar Settings
struct StatusBarSettings {
    status_bar_enabled: bool,
    status_bar_show_word_count: bool,
    status_bar_show_cursor_position: bool,
    status_bar_show_sidebar_toggle: bool,
    status_bar_show_mode_switch: bool,
    custom_buttons: Vec<StatusBarButton>,
}

/// Runtime-accessible editor settings mirrored from [`AppPreferences`] so the
/// render path can read them without touching disk. Toggling persists the new
/// value back to the preferences file.
pub struct EditorSettings {
    show_table_headers: bool,
    show_hidden_files: bool,
    auto_save: AutoSavePreference,
    spell_check: bool,
    auto_pair_brackets: bool,
    auto_pair_markdown: bool,
    code_folding: bool,
    format_on_save: bool,
    editor_font_family: String,
    show_tab_bar_actions: bool,
    status_bar_settings: StatusBarSettings,
}

impl Global for EditorSettings {}

impl EditorSettings {
    #[cfg(test)]
    pub fn init(
        cx: &mut App,
        show_table_headers: bool,
        auto_save: AutoSavePreference,
        spell_check: bool,
    ) {
        Self::init_with_typography(
            cx,
            show_table_headers,
            auto_save,
            spell_check,
            DEFAULT_EDITOR_FONT_SIZE,
            DEFAULT_EDITOR_LINE_HEIGHT_PERCENT,
            DEFAULT_EDITOR_CONTENT_WIDTH,
            "",
            false,
        );
    }

    pub fn init_with_typography(
        cx: &mut App,
        show_table_headers: bool,
        auto_save: AutoSavePreference,
        spell_check: bool,
        editor_font_size: u8,
        editor_line_height_percent: u16,
        editor_content_width: u16,
        editor_font_family: &str,
        show_tab_bar_actions: bool,
    ) {
        let loaded_preferences = read_app_preferences().ok();
        let status_bar = loaded_preferences
            .as_ref()
            .map(|preferences| preferences.status_bar.clone())
            .unwrap_or_default();
        let auto_pair_brackets = loaded_preferences
            .as_ref()
            .map(|preferences| preferences.auto_pair_brackets)
            .unwrap_or(true);
        let show_hidden_files = loaded_preferences
            .as_ref()
            .map(|preferences| preferences.show_hidden_files)
            .unwrap_or(false);
        let auto_pair_markdown = loaded_preferences
            .as_ref()
            .map(|preferences| preferences.auto_pair_markdown)
            .unwrap_or(true);
        let code_folding = loaded_preferences
            .as_ref()
            .map(|preferences| preferences.code_folding)
            .unwrap_or(true);
        let format_on_save = loaded_preferences
            .as_ref()
            .map(|preferences| preferences.format_on_save)
            .unwrap_or(false);
        Self::set_global(
            cx,
            show_table_headers,
            show_hidden_files,
            auto_save,
            spell_check,
            auto_pair_brackets,
            auto_pair_markdown,
            code_folding,
            format_on_save,
            editor_font_family,
            show_tab_bar_actions,
            &status_bar,
        );
        cx.update_global::<ThemeManager, _>(|theme_manager, _cx| {
            theme_manager.set_editor_typography(editor_font_size, editor_line_height_percent);
            theme_manager.set_editor_content_width(editor_content_width);
        });
    }

    fn set_global(
        cx: &mut App,
        show_table_headers: bool,
        show_hidden_files: bool,
        auto_save: AutoSavePreference,
        spell_check: bool,
        auto_pair_brackets: bool,
        auto_pair_markdown: bool,
        code_folding: bool,
        format_on_save: bool,
        editor_font_family: &str,
        show_tab_bar_actions: bool,
        status_bar: &StatusBarPreferences,
    ) {
        cx.set_global(Self {
            show_table_headers,
            show_hidden_files,
            auto_save,
            spell_check,
            auto_pair_brackets,
            auto_pair_markdown,
            code_folding,
            format_on_save,
            editor_font_family: normalize_editor_font_family(editor_font_family),
            show_tab_bar_actions,
            status_bar_settings: StatusBarSettings {
                status_bar_enabled: status_bar.enabled,
                status_bar_show_word_count: status_bar.show_word_count,
                status_bar_show_cursor_position: status_bar.show_cursor_position,
                status_bar_show_sidebar_toggle: status_bar.show_sidebar_toggle,
                status_bar_show_mode_switch: status_bar.show_mode_switch,
                custom_buttons: status_bar.custom_buttons.clone(),
            },
        });
    }

    /// Whether table top rows are styled as headers. Defaults to `true` when
    /// the global has not been installed (e.g. in unit tests).
    pub fn show_table_headers(cx: &App) -> bool {
        cx.try_global::<Self>()
            .map(|settings| settings.show_table_headers)
            .unwrap_or(true)
    }

    pub(crate) fn show_hidden_files(cx: &App) -> bool {
        cx.try_global::<Self>()
            .is_some_and(|settings| settings.show_hidden_files)
    }

    #[cfg(test)]
    pub(crate) fn set_show_hidden_files_for_test(cx: &mut App, show: bool) {
        cx.update_global::<Self, _>(|settings, _cx| {
            settings.show_hidden_files = show;
        });
    }

    pub fn set_show_table_headers(cx: &mut App, show_table_headers: bool) {
        let status_bar = cx
            .try_global::<Self>()
            .map(|s| StatusBarPreferences {
                enabled: s.status_bar_settings.status_bar_enabled,
                show_word_count: s.status_bar_settings.status_bar_show_word_count,
                show_cursor_position: s.status_bar_settings.status_bar_show_cursor_position,
                show_sidebar_toggle: s.status_bar_settings.status_bar_show_sidebar_toggle,
                show_mode_switch: s.status_bar_settings.status_bar_show_mode_switch,
                custom_buttons: s.status_bar_settings.custom_buttons.clone(),
            })
            .unwrap_or_default();
        let auto_save = cx
            .try_global::<Self>()
            .map(|settings| settings.auto_save)
            .unwrap_or_default();
        let spell_check = cx
            .try_global::<Self>()
            .map(|settings| settings.spell_check)
            .unwrap_or(true);
        let auto_pair_brackets = cx
            .try_global::<Self>()
            .map(|settings| settings.auto_pair_brackets)
            .unwrap_or(true);
        let auto_pair_markdown = cx
            .try_global::<Self>()
            .map(|settings| settings.auto_pair_markdown)
            .unwrap_or(true);
        let code_folding = cx
            .try_global::<Self>()
            .map(|settings| settings.code_folding)
            .unwrap_or(true);
        let format_on_save = cx
            .try_global::<Self>()
            .map(|settings| settings.format_on_save)
            .unwrap_or(false);
        let editor_font_family = cx
            .try_global::<Self>()
            .map(|settings| settings.editor_font_family.clone())
            .unwrap_or_default();
        let show_tab_bar_actions = cx
            .try_global::<Self>()
            .map(|settings| settings.show_tab_bar_actions)
            .unwrap_or(false);
        Self::set_global(
            cx,
            show_table_headers,
            cx.try_global::<Self>()
                .is_some_and(|settings| settings.show_hidden_files),
            auto_save,
            spell_check,
            auto_pair_brackets,
            auto_pair_markdown,
            code_folding,
            format_on_save,
            &editor_font_family,
            show_tab_bar_actions,
            &status_bar,
        );
        match read_app_preferences() {
            Ok(mut preferences) => {
                preferences.show_table_headers = show_table_headers;
                if let Err(err) = save_app_preferences(&preferences) {
                    eprintln!("failed to save table header preference: {err}");
                }
            }
            Err(err) => eprintln!("failed to read table header preference: {err}"),
        }
    }

    pub fn status_bar_preferences(cx: &App) -> StatusBarPreferences {
        cx.try_global::<Self>()
            .map(|s| StatusBarPreferences {
                enabled: s.status_bar_settings.status_bar_enabled,
                show_word_count: s.status_bar_settings.status_bar_show_word_count,
                show_cursor_position: s.status_bar_settings.status_bar_show_cursor_position,
                show_sidebar_toggle: s.status_bar_settings.status_bar_show_sidebar_toggle,
                show_mode_switch: s.status_bar_settings.status_bar_show_mode_switch,
                custom_buttons: s.status_bar_settings.custom_buttons.clone(),
            })
            .unwrap_or_default()
    }

    #[cfg(test)]
    pub(crate) fn set_status_bar_preferences_for_test(
        cx: &mut App,
        preferences: StatusBarPreferences,
    ) {
        cx.update_global::<Self, _>(|settings, _cx| {
            settings.status_bar_settings = StatusBarSettings {
                status_bar_enabled: preferences.enabled,
                status_bar_show_word_count: preferences.show_word_count,
                status_bar_show_cursor_position: preferences.show_cursor_position,
                status_bar_show_sidebar_toggle: preferences.show_sidebar_toggle,
                status_bar_show_mode_switch: preferences.show_mode_switch,
                custom_buttons: preferences.custom_buttons,
            };
        });
    }

    pub(crate) fn auto_save(cx: &App) -> AutoSavePreference {
        cx.try_global::<Self>()
            .map(|settings| settings.auto_save)
            .unwrap_or_default()
    }

    pub(crate) fn spell_check(cx: &App) -> bool {
        cx.try_global::<Self>()
            .map(|settings| settings.spell_check)
            .unwrap_or(false)
    }

    pub(crate) fn auto_pair_brackets(cx: &App) -> bool {
        cx.try_global::<Self>()
            .map(|settings| settings.auto_pair_brackets)
            .unwrap_or(true)
    }

    pub(crate) fn auto_pair_markdown(cx: &App) -> bool {
        cx.try_global::<Self>()
            .map(|settings| settings.auto_pair_markdown)
            .unwrap_or(true)
    }

    pub(crate) fn code_folding(cx: &App) -> bool {
        cx.try_global::<Self>()
            .map(|settings| settings.code_folding)
            .unwrap_or(true)
    }

    pub(crate) fn format_on_save(cx: &App) -> bool {
        cx.try_global::<Self>()
            .is_some_and(|settings| settings.format_on_save)
    }

    pub(crate) fn editor_font_family(cx: &App) -> String {
        cx.try_global::<Self>()
            .map(|settings| settings.editor_font_family.clone())
            .unwrap_or_default()
    }

    pub(crate) fn show_tab_bar_actions(cx: &App) -> bool {
        cx.try_global::<Self>()
            .is_some_and(|settings| settings.show_tab_bar_actions)
    }

    #[cfg(test)]
    pub(crate) fn set_show_tab_bar_actions_for_test(cx: &mut App, show: bool) {
        cx.update_global::<Self, _>(|settings, _cx| {
            settings.show_tab_bar_actions = show;
        });
    }
}
mod storage;
use storage::PreferencesNav;
#[cfg(test)]
use storage::*;
pub(crate) use storage::{
    apply_configured_language, first_existing_recent_markdown_file,
    import_language_config_and_select, load_or_create_app_preferences, read_app_preferences,
    save_app_preferences, save_preferences_from_window,
};

mod window;
pub(crate) use window::{localized_shortcut_command_label, open_preferences_window};
