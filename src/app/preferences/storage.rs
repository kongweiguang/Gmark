// @author kongweiguang

//! GPUI adapters around the configuration-domain preference storage.

use std::{collections::BTreeMap, path::PathBuf};

use gpui::App;

use super::*;
use crate::config::AppDirs;

pub(crate) fn read_app_preferences() -> anyhow::Result<AppPreferences> {
    read_app_preferences_with_dirs(&AppDirs::from_system()?)
}

pub(crate) fn read_app_preferences_with_dirs(dirs: &AppDirs) -> anyhow::Result<AppPreferences> {
    Ok(normalize_loaded_preferences(
        gmark_config::read_app_preferences_with_dirs(dirs)?,
    ))
}

pub(crate) fn load_or_create_app_preferences() -> anyhow::Result<AppPreferences> {
    let dirs = AppDirs::from_system()?;
    load_or_create_app_preferences_with_dirs_and_locales(&dirs, sys_locale::get_locales())
}

pub(super) fn load_or_create_app_preferences_with_dirs_and_locales<I, S>(
    dirs: &AppDirs,
    locales: I,
) -> anyhow::Result<AppPreferences>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let fallback_language_id = language_id_for_locale_preferences(locales);
    let preferences = normalize_loaded_preferences(
        gmark_config::load_or_create_app_preferences_with_dirs(dirs, fallback_language_id)?,
    );
    // Shortcut validation belongs to the GPUI layer. Persist its legacy-normalized
    // result after the domain crate has handled TOML compatibility and atomic I/O.
    gmark_config::save_app_preferences_with_dirs(&preferences, dirs)?;
    Ok(preferences)
}

pub(crate) fn save_app_preferences(preferences: &AppPreferences) -> anyhow::Result<()> {
    gmark_config::save_app_preferences(preferences)
}

pub(crate) fn save_app_preferences_with_dirs(
    preferences: &AppPreferences,
    dirs: &AppDirs,
) -> anyhow::Result<()> {
    gmark_config::save_app_preferences_with_dirs(preferences, dirs)
}

/// Persists only the visual accessibility section while preserving every
/// existing preference field. The preferences window calls this after its
/// legacy all-fields save adapter until that adapter is replaced by a value object.
// Reason: TASK-008 consumes this focused persistence adapter; remove after that wiring lands.
#[allow(dead_code)]
pub(crate) fn save_visual_accessibility_preferences(
    visual_accessibility: VisualAccessibilityPreferences,
) -> anyhow::Result<AppPreferences> {
    update_app_preferences(|preferences| {
        preferences.visual_accessibility = visual_accessibility;
    })
}

pub(crate) fn first_existing_recent_markdown_file() -> Option<PathBuf> {
    gmark_config::read_recent_files()
        .ok()?
        .into_iter()
        .find(|path| path.is_file())
}

pub(crate) fn apply_configured_language(cx: &mut App, language_id: &str) -> anyhow::Result<bool> {
    let mut applied = false;
    let changed = cx.update_global::<I18nManager, _>(|i18n_manager, _cx| {
        let changed = i18n_manager.set_language_by_id(language_id);
        applied = changed || i18n_manager.current_language_id() == language_id;
        changed
    });
    if !applied {
        return Ok(false);
    }
    update_app_preferences(|preferences| {
        preferences.default_language_id = language_id.into();
    })?;
    Ok(changed)
}

pub(crate) fn import_language_config_and_select(
    cx: &mut App,
    path: impl AsRef<std::path::Path>,
) -> anyhow::Result<String> {
    let imported_id = cx.update_global::<I18nManager, _>(|i18n_manager, _cx| {
        i18n_manager.import_language_config(path)
    })?;
    update_app_preferences(|preferences| {
        preferences.default_language_id = imported_id.clone();
    })?;
    Ok(imported_id)
}

// 原因: PreferencesWindow 需要逐字段承接既有 UI 草稿和持久化兼容字段，暂时不能改变调用形状；移除条件: 窗口保存改为传递专用的 preferences-save 参数对象。
#[allow(clippy::too_many_arguments)]
pub(crate) fn save_preferences_from_window(
    startup_open: StartupOpenPreference,
    auto_check_updates: bool,
    show_hidden_files: bool,
    auto_save: AutoSavePreference,
    spell_check: bool,
    auto_pair_brackets: bool,
    auto_pair_markdown: bool,
    code_folding: bool,
    format_on_save: bool,
    editor_font_size: u8,
    editor_line_height_percent: u16,
    editor_content_width: u16,
    editor_font_family: &str,
    show_tab_bar_actions: bool,
    theme_appearance: ThemeAppearance,
    theme_palette: ThemePalette,
    default_language_id: &str,
    image_paste_behavior: ImagePasteBehavior,
    keybindings: BTreeMap<String, Vec<String>>,
    document_loading: &DocumentLoadingPreferences,
    status_bar: &StatusBarPreferences,
) -> anyhow::Result<AppPreferences> {
    let dirs = AppDirs::from_system()?;
    save_preferences_from_window_with_dirs(
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
        editor_font_family,
        show_tab_bar_actions,
        theme_appearance,
        theme_palette,
        default_language_id,
        image_paste_behavior,
        keybindings,
        document_loading,
        status_bar,
        &dirs,
    )
}

// 原因: 显式目录 adapter 必须镜像旧窗口参数列表，保证测试和 TOML 兼容写入共享同一字段映射；移除条件: 两个保存路径都改为接受共享的 preferences-save 参数对象。
#[allow(clippy::too_many_arguments)]
pub(super) fn save_preferences_from_window_with_dirs(
    startup_open: StartupOpenPreference,
    auto_check_updates: bool,
    show_hidden_files: bool,
    auto_save: AutoSavePreference,
    spell_check: bool,
    auto_pair_brackets: bool,
    auto_pair_markdown: bool,
    code_folding: bool,
    format_on_save: bool,
    editor_font_size: u8,
    editor_line_height_percent: u16,
    editor_content_width: u16,
    editor_font_family: &str,
    show_tab_bar_actions: bool,
    theme_appearance: ThemeAppearance,
    theme_palette: ThemePalette,
    default_language_id: &str,
    image_paste_behavior: ImagePasteBehavior,
    keybindings: BTreeMap<String, Vec<String>>,
    document_loading: &DocumentLoadingPreferences,
    status_bar: &StatusBarPreferences,
    dirs: &AppDirs,
) -> anyhow::Result<AppPreferences> {
    let mut preferences =
        load_or_create_app_preferences_with_dirs_and_locales(dirs, sys_locale::get_locales())?;
    preferences.startup_open = startup_open;
    preferences.auto_check_updates = auto_check_updates;
    preferences.show_hidden_files = show_hidden_files;
    preferences.auto_save = auto_save;
    preferences.spell_check = spell_check;
    preferences.auto_pair_brackets = auto_pair_brackets;
    preferences.auto_pair_markdown = auto_pair_markdown;
    preferences.code_folding = code_folding;
    preferences.format_on_save = format_on_save;
    preferences.editor_font_size =
        editor_font_size.clamp(MIN_EDITOR_FONT_SIZE, MAX_EDITOR_FONT_SIZE);
    preferences.editor_line_height_percent =
        normalize_editor_line_height_percent(editor_line_height_percent);
    preferences.editor_content_width = normalize_editor_content_width(editor_content_width);
    preferences.editor_font_family = normalize_editor_font_family(editor_font_family);
    preferences.show_tab_bar_actions = show_tab_bar_actions;
    preferences.theme_appearance = theme_appearance;
    preferences.theme_palette = theme_palette;
    preferences.default_language_id = default_language_id.into();
    preferences.image_paste_behavior = image_paste_behavior;
    preferences.keybindings = normalize_shortcut_config(&keybindings);
    preferences.document_loading = document_loading.clone();
    preferences.status_bar = status_bar.clone();
    save_app_preferences_with_dirs(&preferences, dirs)?;
    Ok(preferences)
}

fn normalize_loaded_preferences(mut preferences: AppPreferences) -> AppPreferences {
    preferences.keybindings = normalize_shortcut_config(&preferences.keybindings);
    preferences
}

fn update_app_preferences(
    update: impl FnOnce(&mut AppPreferences),
) -> anyhow::Result<AppPreferences> {
    let mut preferences = load_or_create_app_preferences()?;
    update(&mut preferences);
    save_app_preferences(&preferences)?;
    Ok(preferences)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PreferencesNav {
    File,
    Editor,
    Theme,
    Image,
    Shortcuts,
    StatusBar,
}
