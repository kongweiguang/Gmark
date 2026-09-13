// @author kongweiguang

//! 偏好的兼容 TOML 编解码与文件边界。

use anyhow::{Context as _, Result};
use serde::Serialize;

use crate::{
    AppDirs,
    persistence::atomic_write,
    preferences::{
        AccessibilityOverride, AppPreferences, AutoSavePreference, DEFAULT_LANGUAGE_ID,
        DocumentLoadingPreferences, ImagePasteBehavior, ResourceInsertBehavior, ShortcutConfig,
        StartupOpenPreference, StatusBarButton, StatusBarPreferences, ThemeAppearance,
        ThemePalette, VisualAccessibilityPreferences,
    },
};

const DEFAULT_EDITOR_FONT_SIZE: u8 = 16;
const DEFAULT_EDITOR_LINE_HEIGHT_PERCENT: u16 = 160;
const MIN_EDITOR_FONT_SIZE: u8 = 12;
const MAX_EDITOR_FONT_SIZE: u8 = 24;
const MIN_EDITOR_LINE_HEIGHT_PERCENT: u16 = 120;
const MAX_EDITOR_LINE_HEIGHT_PERCENT: u16 = 200;
const EDITOR_LINE_HEIGHT_STEP: u16 = 5;
const DEFAULT_EDITOR_CONTENT_WIDTH: u16 = 1200;
const MIN_EDITOR_CONTENT_WIDTH: u16 = 680;
const MAX_EDITOR_CONTENT_WIDTH: u16 = 1600;
const EDITOR_CONTENT_WIDTH_STEP: u16 = 40;
const MAX_EDITOR_FONT_FAMILY_CHARS: usize = 80;

#[derive(Serialize)]
struct PreferencesFile {
    startup: StartupPreferencesFile,
    updates: UpdatesPreferencesFile,
    language: LanguagePreferencesFile,
    theme: ThemePreferencesFile,
    accessibility: AccessibilityPreferencesFile,
    editor: EditorPreferencesFile,
    status_bar: StatusBarPreferencesFile,
    workspace: WorkspacePreferencesFile,
    documents: DocumentsPreferencesFile,
    keybindings: ShortcutConfig,
}

#[derive(Serialize)]
struct StartupPreferencesFile {
    open: String,
}

#[derive(Serialize)]
struct UpdatesPreferencesFile {
    auto_check: bool,
}

#[derive(Serialize)]
struct LanguagePreferencesFile {
    default_language_id: String,
}

#[derive(Serialize)]
struct ThemePreferencesFile {
    appearance: String,
    palette: String,
}

#[derive(Serialize)]
struct AccessibilityPreferencesFile {
    reduced_motion: String,
    reduced_transparency: String,
    high_contrast: String,
}

#[derive(Serialize)]
struct EditorPreferencesFile {
    show_table_headers: bool,
    resource_insert_behavior: String,
    image_paste_behavior: String,
    auto_save: String,
    spell_check: bool,
    auto_pair_brackets: bool,
    auto_pair_markdown: bool,
    code_folding: bool,
    format_on_save: bool,
    font_size: u8,
    line_height_percent: u16,
    content_width: u16,
    show_tab_bar_actions: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    recent_editing_commands: Vec<String>,
    #[serde(skip_serializing_if = "String::is_empty")]
    font_family: String,
}

#[derive(Serialize)]
struct StatusBarPreferencesFile {
    enabled: bool,
    show_word_count: bool,
    show_cursor_position: bool,
    show_sidebar_toggle: bool,
    show_mode_switch: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    custom_buttons: Vec<StatusBarButton>,
}

#[derive(Serialize)]
struct WorkspacePreferencesFile {
    show_hidden_files: bool,
}

#[derive(Serialize)]
struct DocumentsPreferencesFile {
    loading: DocumentLoadingPreferencesFile,
}

#[derive(Serialize)]
struct DocumentLoadingPreferencesFile {
    #[serde(skip_serializing_if = "Option::is_none")]
    max_resident_mib: Option<u64>,
}

impl From<&AppPreferences> for PreferencesFile {
    fn from(value: &AppPreferences) -> Self {
        Self {
            startup: StartupPreferencesFile {
                open: value.startup_open.as_str().into(),
            },
            updates: UpdatesPreferencesFile {
                auto_check: value.auto_check_updates,
            },
            language: LanguagePreferencesFile {
                default_language_id: value.default_language_id.clone(),
            },
            theme: ThemePreferencesFile {
                appearance: value.theme_appearance.as_str().into(),
                palette: value.theme_palette.as_str().into(),
            },
            accessibility: AccessibilityPreferencesFile {
                reduced_motion: value.visual_accessibility.reduced_motion.as_str().into(),
                reduced_transparency: value
                    .visual_accessibility
                    .reduced_transparency
                    .as_str()
                    .into(),
                high_contrast: value.visual_accessibility.high_contrast.as_str().into(),
            },
            editor: EditorPreferencesFile {
                show_table_headers: value.show_table_headers,
                resource_insert_behavior: value.resource_insert_behavior().as_str().into(),
                // 一个兼容版本内双写旧键，保障旧版 Gmark 降级时仍读取相同行为。
                image_paste_behavior: value.image_paste_behavior.as_str().into(),
                auto_save: value.auto_save.as_str().into(),
                spell_check: value.spell_check,
                auto_pair_brackets: value.auto_pair_brackets,
                auto_pair_markdown: value.auto_pair_markdown,
                code_folding: value.code_folding,
                format_on_save: value.format_on_save,
                font_size: value.editor_font_size,
                line_height_percent: value.editor_line_height_percent,
                content_width: value.editor_content_width,
                show_tab_bar_actions: value.show_tab_bar_actions,
                recent_editing_commands: value.recent_editing_commands.clone(),
                font_family: value.editor_font_family.clone(),
            },
            status_bar: StatusBarPreferencesFile {
                enabled: value.status_bar.enabled,
                show_word_count: value.status_bar.show_word_count,
                show_cursor_position: value.status_bar.show_cursor_position,
                show_sidebar_toggle: value.status_bar.show_sidebar_toggle,
                show_mode_switch: value.status_bar.show_mode_switch,
                custom_buttons: value.status_bar.custom_buttons.clone(),
            },
            workspace: WorkspacePreferencesFile {
                show_hidden_files: value.show_hidden_files,
            },
            documents: DocumentsPreferencesFile {
                loading: DocumentLoadingPreferencesFile {
                    max_resident_mib: value.document_loading.max_resident_mib,
                },
            },
            keybindings: value.keybindings.clone(),
        }
    }
}

/// 从系统配置目录读取偏好；损坏 TOML 按既有语义回退默认值。
pub fn read_app_preferences() -> Result<AppPreferences> {
    read_app_preferences_with_dirs(&AppDirs::from_system()?)
}

/// 从显式配置目录读取偏好；损坏 TOML 按既有语义回退默认值。
pub fn read_app_preferences_with_dirs(dirs: &AppDirs) -> Result<AppPreferences> {
    dirs.validate_config_root()?;
    let path = dirs.app_config_file();
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(AppPreferences::default());
        }
        Err(error) => {
            return Err(error).with_context(|| format!("failed to read '{}'", path.display()));
        }
    };
    let Ok(value) = toml::from_str::<toml::Value>(&text) else {
        return Ok(AppPreferences::default());
    };
    Ok(preferences_from_toml(&value, DEFAULT_LANGUAGE_ID))
}

/// 读取或创建系统配置目录中的偏好，使用既有默认语言。
pub fn load_or_create_app_preferences() -> Result<AppPreferences> {
    let dirs = AppDirs::from_system()?;
    load_or_create_app_preferences_with_dirs(&dirs, DEFAULT_LANGUAGE_ID)
}

/// 读取或创建显式目录中的偏好，并由宿主提供语言检测后的回退 ID。
pub fn load_or_create_app_preferences_with_dirs(
    dirs: &AppDirs,
    fallback_language_id: &str,
) -> Result<AppPreferences> {
    dirs.validate_config_root()?;
    let path = dirs.app_config_file();
    let fallback_language_id = fallback_language_id_or_default(fallback_language_id);
    let preferences = match std::fs::read_to_string(&path) {
        Ok(text) => toml::from_str::<toml::Value>(&text)
            .map(|value| preferences_from_toml(&value, fallback_language_id))
            .unwrap_or_else(|_| preferences_with_fallback_language(fallback_language_id)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            preferences_with_fallback_language(fallback_language_id)
        }
        Err(error) => {
            return Err(error).with_context(|| format!("failed to read '{}'", path.display()));
        }
    };
    save_app_preferences_with_dirs(&preferences, dirs)?;
    Ok(preferences)
}

/// 将偏好写入系统配置目录。
pub fn save_app_preferences(preferences: &AppPreferences) -> Result<()> {
    save_app_preferences_with_dirs(preferences, &AppDirs::from_system()?)
}

/// 将偏好原子写入显式配置目录。
pub fn save_app_preferences_with_dirs(preferences: &AppPreferences, dirs: &AppDirs) -> Result<()> {
    let path = dirs.app_config_file();
    dirs.ensure_config_parent(&path)?;
    let text = toml::to_string_pretty(&PreferencesFile::from(preferences))?;
    atomic_write(&path, text.as_bytes())
}

fn preferences_with_fallback_language(fallback_language_id: &str) -> AppPreferences {
    AppPreferences {
        default_language_id: fallback_language_id.into(),
        ..AppPreferences::default()
    }
}

fn fallback_language_id_or_default(value: &str) -> &str {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        DEFAULT_LANGUAGE_ID
    } else {
        trimmed
    }
}

fn preferences_from_toml(value: &toml::Value, fallback_language_id: &str) -> AppPreferences {
    let startup_open = value
        .get("startup")
        .and_then(|startup| startup.get("open"))
        .and_then(toml::Value::as_str)
        .map(StartupOpenPreference::parse)
        .unwrap_or_default();
    let default_language_id = value
        .get("language")
        .and_then(|language| language.get("default_language_id"))
        .and_then(toml::Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .unwrap_or(fallback_language_id)
        .to_owned();
    let (theme_appearance, theme_palette) = match (
        string_value(value, "theme", "appearance").and_then(ThemeAppearance::parse),
        string_value(value, "theme", "palette").and_then(ThemePalette::parse),
    ) {
        (Some(appearance), Some(palette)) => (appearance, palette),
        _ => (ThemeAppearance::System, ThemePalette::Xcode),
    };
    let keybindings = value
        .get("keybindings")
        .and_then(toml::Value::as_table)
        .map(|table| {
            table
                .iter()
                .filter_map(|(key, value)| {
                    let keys = value
                        .as_array()?
                        .iter()
                        .filter_map(|value| value.as_str().map(str::to_owned))
                        .collect();
                    Some((key.clone(), keys))
                })
                .collect()
        })
        .unwrap_or_default();
    let visual_accessibility = VisualAccessibilityPreferences {
        reduced_motion: accessibility_override(value, "reduced_motion"),
        reduced_transparency: accessibility_override(value, "reduced_transparency"),
        high_contrast: accessibility_override(value, "high_contrast"),
    };
    let legacy_image_paste_behavior = string_value(value, "editor", "image_paste_behavior")
        .map(ImagePasteBehavior::parse)
        .unwrap_or_default();
    let image_paste_behavior = string_value(value, "editor", "resource_insert_behavior")
        .map(ResourceInsertBehavior::parse)
        .unwrap_or(legacy_image_paste_behavior);

    AppPreferences {
        startup_open,
        auto_check_updates: bool_value(value, "updates", "auto_check").unwrap_or(true),
        default_language_id,
        theme_appearance,
        theme_palette,
        visual_accessibility,
        show_table_headers: bool_value(value, "editor", "show_table_headers").unwrap_or(true),
        show_hidden_files: bool_value(value, "workspace", "show_hidden_files").unwrap_or(false),
        image_paste_behavior,
        auto_save: string_value(value, "editor", "auto_save")
            .map(AutoSavePreference::parse)
            .unwrap_or_default(),
        spell_check: bool_value(value, "editor", "spell_check").unwrap_or(true),
        auto_pair_brackets: bool_value(value, "editor", "auto_pair_brackets").unwrap_or(true),
        auto_pair_markdown: bool_value(value, "editor", "auto_pair_markdown").unwrap_or(true),
        code_folding: bool_value(value, "editor", "code_folding").unwrap_or(true),
        format_on_save: bool_value(value, "editor", "format_on_save").unwrap_or(false),
        editor_font_size: integer_value(value, "editor", "font_size")
            .and_then(|number| u8::try_from(number).ok())
            .unwrap_or(DEFAULT_EDITOR_FONT_SIZE)
            .clamp(MIN_EDITOR_FONT_SIZE, MAX_EDITOR_FONT_SIZE),
        editor_line_height_percent: normalize_editor_line_height_percent(
            integer_value(value, "editor", "line_height_percent")
                .and_then(|number| u16::try_from(number).ok())
                .unwrap_or(DEFAULT_EDITOR_LINE_HEIGHT_PERCENT),
        ),
        editor_content_width: normalize_editor_content_width(
            integer_value(value, "editor", "content_width")
                .and_then(|number| u16::try_from(number).ok())
                .unwrap_or(DEFAULT_EDITOR_CONTENT_WIDTH),
        ),
        editor_font_family: string_value(value, "editor", "font_family")
            .map(normalize_editor_font_family)
            .unwrap_or_default(),
        show_tab_bar_actions: bool_value(value, "editor", "show_tab_bar_actions").unwrap_or(false),
        recent_editing_commands: value
            .get("editor")
            .and_then(|editor| editor.get("recent_editing_commands"))
            .and_then(toml::Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(|value| value.as_str().map(str::to_owned))
                    .take(5)
                    .collect()
            })
            .unwrap_or_default(),
        keybindings,
        status_bar: status_bar_from_toml(value).unwrap_or_default(),
        document_loading: DocumentLoadingPreferences {
            max_resident_mib: value
                .get("documents")
                .and_then(|documents| documents.get("loading"))
                .and_then(|loading| loading.get("max_resident_mib"))
                .and_then(toml::Value::as_integer)
                .and_then(|number| u64::try_from(number).ok()),
        },
    }
}

fn accessibility_override(value: &toml::Value, key: &str) -> AccessibilityOverride {
    string_value(value, "accessibility", key)
        .and_then(AccessibilityOverride::parse)
        .unwrap_or_default()
}

fn bool_value(value: &toml::Value, section: &str, key: &str) -> Option<bool> {
    value
        .get(section)
        .and_then(|section| section.get(key))
        .and_then(toml::Value::as_bool)
}

fn string_value<'a>(value: &'a toml::Value, section: &str, key: &str) -> Option<&'a str> {
    value
        .get(section)
        .and_then(|section| section.get(key))
        .and_then(toml::Value::as_str)
}

fn integer_value(value: &toml::Value, section: &str, key: &str) -> Option<i64> {
    value
        .get(section)
        .and_then(|section| section.get(key))
        .and_then(toml::Value::as_integer)
}

fn status_bar_from_toml(value: &toml::Value) -> Option<StatusBarPreferences> {
    let status_bar = value.get("status_bar")?;
    let custom_buttons = status_bar
        .get("custom_buttons")
        .and_then(toml::Value::as_array)
        .map(|buttons| {
            buttons
                .iter()
                .filter_map(|button| {
                    let id = button.get("id")?.as_str()?.to_owned();
                    let label = button.get("label")?.as_str()?.to_owned();
                    let action_id = button
                        .get("action_id")
                        .and_then(toml::Value::as_str)
                        .unwrap_or_default()
                        .to_owned();
                    Some(StatusBarButton {
                        id,
                        label,
                        action_id,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Some(StatusBarPreferences {
        enabled: status_bar
            .get("enabled")
            .and_then(toml::Value::as_bool)
            .unwrap_or(true),
        show_word_count: status_bar
            .get("show_word_count")
            .and_then(toml::Value::as_bool)
            .unwrap_or(true),
        show_cursor_position: status_bar
            .get("show_cursor_position")
            .and_then(toml::Value::as_bool)
            .unwrap_or(true),
        show_sidebar_toggle: status_bar
            .get("show_sidebar_toggle")
            .and_then(toml::Value::as_bool)
            .unwrap_or(true),
        show_mode_switch: status_bar
            .get("show_mode_switch")
            .and_then(toml::Value::as_bool)
            .unwrap_or(true),
        custom_buttons,
    })
}

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
        .filter(|character| !character.is_control())
        .take(MAX_EDITOR_FONT_FAMILY_CHARS)
        .collect()
}
