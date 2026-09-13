// @author kongweiguang

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use anyhow::Result;
use gmark_config::{
    AccessibilityOverride, AppDirs, AppPreferences, AutoSavePreference, DocumentLoadingPreferences,
    ResourceInsertBehavior, StartupOpenPreference, StatusBarButton, StatusBarPreferences,
    SystemVisualPreferences, ThemeAppearance, ThemePalette, VisualAccessibilityPreferences,
    load_or_create_app_preferences_with_dirs, load_or_create_installation_id_with_dirs,
    read_app_preferences_with_dirs, read_recent_files_with_dirs, record_recent_file_with_dirs,
    remove_recent_file_with_dirs, save_app_preferences_with_dirs,
};
use tempfile::TempDir;

fn temporary_dirs() -> Result<(TempDir, AppDirs)> {
    let temporary = TempDir::new()?;
    let dirs = AppDirs::from_root(temporary.path());
    Ok((temporary, dirs))
}

#[test]
fn config_paths_preserve_the_existing_contract() -> Result<()> {
    let (_temporary, dirs) = temporary_dirs()?;
    assert_eq!(dirs.languages_dir(), dirs.config_root().join("languages"));
    assert_eq!(dirs.history_file(), dirs.state_root().join(".history"));
    assert_eq!(
        dirs.app_config_file(),
        dirs.config_root().join("config.toml")
    );
    assert_eq!(dirs.recovery_dir(), dirs.state_root().join("recovery"));
    assert_eq!(
        dirs.crash_reports_dir(),
        dirs.state_root().join("crash-reports")
    );
    assert_eq!(dirs.updates_dir(), dirs.cache_root().join("updates"));
    assert_eq!(
        dirs.installation_id_file(),
        dirs.state_root().join("installation-id")
    );
    assert_eq!(
        dirs.workspace_session_file(),
        dirs.state_root().join("workspace-session.json")
    );
    Ok(())
}

#[test]
fn preferences_round_trip_with_stable_toml_keys() -> Result<()> {
    let (_temporary, dirs) = temporary_dirs()?;
    let preferences = AppPreferences {
        startup_open: StartupOpenPreference::LastOpenedFile,
        auto_check_updates: false,
        default_language_id: "zh-CN".into(),
        theme_appearance: ThemeAppearance::Dark,
        theme_palette: ThemePalette::Obsidian,
        visual_accessibility: VisualAccessibilityPreferences {
            reduced_motion: AccessibilityOverride::Enabled,
            reduced_transparency: AccessibilityOverride::Disabled,
            high_contrast: AccessibilityOverride::System,
        },
        show_table_headers: false,
        show_hidden_files: true,
        image_paste_behavior: ResourceInsertBehavior::CopyToNamedAssetsFolder,
        auto_save: AutoSavePreference::AfterDelay,
        spell_check: false,
        auto_pair_brackets: false,
        auto_pair_markdown: false,
        code_folding: false,
        format_on_save: true,
        editor_font_size: 18,
        editor_line_height_percent: 165,
        editor_content_width: 1240,
        editor_font_family: "Iosevka".into(),
        show_tab_bar_actions: true,
        recent_editing_commands: vec!["table.insert".into()],
        keybindings: BTreeMap::from([("editor.save".into(), vec!["ctrl-s".into()])]),
        status_bar: StatusBarPreferences {
            enabled: false,
            show_word_count: false,
            show_cursor_position: false,
            show_sidebar_toggle: false,
            show_mode_switch: false,
            custom_buttons: vec![StatusBarButton {
                id: "publish".into(),
                label: "Publish".into(),
                action_id: "app.publish".into(),
            }],
        },
        document_loading: DocumentLoadingPreferences {
            max_resident_mib: Some(64),
        },
    };

    save_app_preferences_with_dirs(&preferences, &dirs)?;
    assert_eq!(read_app_preferences_with_dirs(&dirs)?, preferences);
    let toml = fs::read_to_string(dirs.app_config_file())?;
    assert!(toml.contains("resource_insert_behavior = \"copy_to_named_assets_folder\""));
    assert!(toml.contains("image_paste_behavior = \"copy_to_named_assets_folder\""));
    assert!(toml.contains("[keybindings]"));
    assert!(toml.contains("[workspace]"));
    assert!(toml.contains("show_hidden_files = true"));
    assert!(toml.contains("[documents.loading]"));
    assert!(toml.contains("[accessibility]"));
    assert!(toml.contains("reduced_motion = \"enabled\""));
    Ok(())
}

#[test]
fn visual_accessibility_invalid_values_fall_back_independently() -> Result<()> {
    let (_temporary, dirs) = temporary_dirs()?;
    fs::create_dir_all(dirs.config_root())?;
    fs::write(
        dirs.app_config_file(),
        r#"
[accessibility]
reduced_motion = "enabled"
reduced_transparency = "invalid"
high_contrast = "disabled"
"#,
    )?;

    let preferences = read_app_preferences_with_dirs(&dirs)?;
    assert_eq!(
        preferences.visual_accessibility,
        VisualAccessibilityPreferences {
            reduced_motion: AccessibilityOverride::Enabled,
            reduced_transparency: AccessibilityOverride::System,
            high_contrast: AccessibilityOverride::Disabled,
        }
    );
    Ok(())
}

#[test]
fn explicit_visual_accessibility_overrides_win_over_system_values() {
    let preferences = VisualAccessibilityPreferences {
        reduced_motion: AccessibilityOverride::Disabled,
        reduced_transparency: AccessibilityOverride::Enabled,
        high_contrast: AccessibilityOverride::System,
    };
    let resolved = preferences.resolve(SystemVisualPreferences {
        reduced_motion: true,
        reduced_transparency: false,
        high_contrast: true,
    });

    assert!(!resolved.reduced_motion);
    assert!(resolved.reduced_transparency);
    assert!(resolved.high_contrast);
}

#[test]
fn preferences_default_and_invalid_values_follow_compatibility_rules() -> Result<()> {
    let (_temporary, dirs) = temporary_dirs()?;
    assert_eq!(
        read_app_preferences_with_dirs(&dirs)?,
        AppPreferences::default()
    );

    fs::create_dir_all(dirs.config_root())?;
    fs::write(dirs.app_config_file(), "this is not valid = [")?;
    assert_eq!(
        read_app_preferences_with_dirs(&dirs)?,
        AppPreferences::default()
    );
    let created = load_or_create_app_preferences_with_dirs(&dirs, "zh-CN")?;
    assert_eq!(created.default_language_id, "zh-CN");

    fs::write(
        dirs.app_config_file(),
        r#"
[language]
default_language_id = "   "

[theme]
appearance = "dark"
palette = "unknown"

[editor]
image_paste_behavior = "copy_to_assets_folder"
font_size = 99
line_height_percent = 121
content_width = 1599
font_family = "  Iosevka\u0001Term  "
recent_editing_commands = ["one", "two", "three", "four", "five", "six"]

[documents.loading]
max_resident_mib = 0
"#,
    )?;
    let preferences = read_app_preferences_with_dirs(&dirs)?;
    assert_eq!(preferences.default_language_id, "en-US");
    assert_eq!(preferences.theme_appearance, ThemeAppearance::System);
    assert_eq!(preferences.theme_palette, ThemePalette::Xcode);
    assert_eq!(
        preferences.image_paste_behavior,
        ResourceInsertBehavior::CopyToAssetsFolder
    );
    assert_eq!(preferences.editor_font_size, 24);
    assert_eq!(preferences.editor_line_height_percent, 120);
    assert_eq!(preferences.editor_content_width, 1600);
    assert_eq!(preferences.editor_font_family, "IosevkaTerm");
    assert_eq!(preferences.recent_editing_commands.len(), 5);
    assert!(preferences.document_loading.has_invalid_override());
    assert_eq!(
        preferences.document_loading.policy().max_resident_bytes,
        None
    );
    Ok(())
}

#[test]
fn recent_files_dedupe_trim_and_limit_history() -> Result<()> {
    let (_temporary, dirs) = temporary_dirs()?;
    for index in 0..22 {
        record_recent_file_with_dirs(Path::new(&format!("note-{index}.md")), &dirs)?;
    }
    let recent = read_recent_files_with_dirs(&dirs)?;
    assert_eq!(recent.len(), 20);
    assert_eq!(recent.first(), Some(&PathBuf::from("note-21.md")));
    assert_eq!(recent.last(), Some(&PathBuf::from("note-2.md")));

    let updated = record_recent_file_with_dirs(Path::new("note-10.md"), &dirs)?;
    assert_eq!(updated.first(), Some(&PathBuf::from("note-10.md")));
    assert_eq!(updated.len(), 20);
    let without_ten = remove_recent_file_with_dirs(Path::new("note-10.md"), &dirs)?;
    assert!(!without_ten.contains(&PathBuf::from("note-10.md")));

    fs::write(dirs.history_file(), "  a.md\n\n a.md\n b.md\n")?;
    assert_eq!(
        read_recent_files_with_dirs(&dirs)?,
        vec![PathBuf::from("a.md"), PathBuf::from("b.md")]
    );
    Ok(())
}

#[test]
fn installation_id_is_created_once_and_then_remains_stable() -> Result<()> {
    let (_temporary, dirs) = temporary_dirs()?;
    let first = load_or_create_installation_id_with_dirs(&dirs)?;
    let second = load_or_create_installation_id_with_dirs(&dirs)?;
    assert_eq!(first, second);
    assert_eq!(
        fs::read_to_string(dirs.installation_id_file())?.trim(),
        first.to_string()
    );
    Ok(())
}
