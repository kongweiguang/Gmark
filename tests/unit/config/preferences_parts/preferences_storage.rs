// @author kongweiguang

use super::{
    AppPreferences, AutoSavePreference, DocumentLoadingPreferences, EditorSettings,
    PreferencesAccessibilityControl,
    ImagePasteBehavior, PreferencesDropdown, PreferencesNav, PreferencesNumericInput,
    PreferencesStepperControl, PreferencesSwitch, ResourceInsertBehavior,
    StartupOpenPreference, StatusBarButton, StatusBarPreferences,
    load_or_create_app_preferences_with_dirs_and_locales, open_preferences_window_with_state,
    parse_numeric_input, read_app_preferences_with_dirs, save_app_preferences_with_dirs,
    save_preferences_from_window_with_dirs,
};
use crate::config::AppDirs;
use crate::i18n::I18nManager;
use crate::theme::{ThemeAppearance, ThemeManager, ThemePalette};
use gpui::{KeyDownEvent, Keystroke, Modifiers, TestAppContext, VisualTestContext, px, size};
use std::collections::BTreeMap;

fn init_preferences_test_app(cx: &mut TestAppContext) {
    cx.update(|cx| {
        I18nManager::init_with_language_id(cx, "en-US");
        ThemeManager::init_with_preference(cx, ThemeAppearance::System, ThemePalette::Xcode);
        crate::components::init(cx);
        EditorSettings::init(cx, true, AutoSavePreference::Off, true);
    });
}

#[test]
fn editor_preferences_match_product_defaults() {
    let preferences = AppPreferences::default();

    assert!(preferences.editor_font_family.is_empty());
    assert_eq!(preferences.editor_font_size, 16);
    assert_eq!(preferences.editor_line_height_percent, 160);
    assert_eq!(preferences.editor_content_width, 1200);
    assert!(preferences.auto_pair_brackets);
    assert!(preferences.auto_pair_markdown);
    assert!(!preferences.show_hidden_files);
    assert!(!preferences.show_tab_bar_actions);
    assert_eq!(
        preferences.resource_insert_behavior(),
        ResourceInsertBehavior::None
    );
}

#[test]
fn missing_preferences_file_returns_defaults() {
    let root = std::env::temp_dir().join(format!(
        "gmark-preferences-missing-{}",
        uuid::Uuid::new_v4()
    ));
    let dirs = AppDirs::from_root(&root);
    let preferences =
        read_app_preferences_with_dirs(&dirs).expect("missing preferences should load");
    assert_eq!(preferences, AppPreferences::default());
    let _ = std::fs::remove_dir_all(root);
}
#[test]
fn partial_or_invalid_preferences_fall_back_by_field() {
    let root = std::env::temp_dir().join(format!(
        "gmark-preferences-partial-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&root).expect("temp root should exist");
    let dirs = AppDirs::from_root(&root);
    std::fs::write(
        dirs.app_config_file(),
        r#"
                [startup]
                open = "not-valid"

                [theme]
                default_theme_id = "gmark-light"
            "#,
    )
    .expect("preferences should be written");

    let preferences =
        read_app_preferences_with_dirs(&dirs).expect("partial preferences should load");
    assert_eq!(preferences.startup_open, StartupOpenPreference::NewFile);
    assert_eq!(preferences.default_language_id, "en-US");
    assert_eq!(preferences.theme_appearance, ThemeAppearance::System);
    assert_eq!(preferences.theme_palette, ThemePalette::Xcode);
    assert_eq!(preferences.image_paste_behavior, ImagePasteBehavior::None);
    assert!(preferences.auto_pair_brackets);
    assert!(preferences.auto_pair_markdown);
    assert!(!preferences.show_hidden_files);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn legacy_workspace_sidebar_position_is_ignored_and_not_written_back() {
    let root = std::env::temp_dir().join(format!(
        "gmark-preferences-sidebar-legacy-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&root).expect("temp root should exist");
    let dirs = AppDirs::from_root(&root);
    std::fs::write(
        dirs.app_config_file(),
        "[editor]\nworkspace_sidebar_position = \"right\"\n",
    )
    .expect("legacy preference should be written");

    let loaded = read_app_preferences_with_dirs(&dirs).expect("legacy preferences should load");
    assert_eq!(loaded, AppPreferences::default());
    save_app_preferences_with_dirs(&loaded, &dirs).expect("normalized preferences should save");
    let saved = std::fs::read_to_string(dirs.app_config_file()).expect("config should exist");
    assert!(!saved.contains("workspace_sidebar_position"));
    std::fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn invalid_theme_pair_falls_back_to_system_xcode() {
    let root = std::env::temp_dir().join(format!(
        "gmark-preferences-invalid-theme-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&root).expect("temp root should exist");
    let dirs = AppDirs::from_root(&root);
    std::fs::write(
        dirs.app_config_file(),
        r#"
                [theme]
                appearance = "dark"
                palette = "not-a-palette"
            "#,
    )
    .expect("invalid theme preferences should be written");

    let preferences =
        read_app_preferences_with_dirs(&dirs).expect("invalid theme preferences should load");
    assert_eq!(preferences.theme_appearance, ThemeAppearance::System);
    assert_eq!(preferences.theme_palette, ThemePalette::Xcode);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn invalid_image_paste_behavior_falls_back_to_none() {
    let root = std::env::temp_dir().join(format!(
        "gmark-preferences-image-invalid-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&root).expect("temp root should exist");
    let dirs = AppDirs::from_root(&root);
    std::fs::write(
        dirs.app_config_file(),
        r#"
                [editor]
                image_paste_behavior = "somewhere-dangerous"
            "#,
    )
    .expect("preferences should be written");

    let preferences = read_app_preferences_with_dirs(&dirs).expect("preferences should load");
    assert_eq!(preferences.image_paste_behavior, ImagePasteBehavior::None);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn resource_insert_behavior_migrates_legacy_key_and_new_key_wins() {
    let root = std::env::temp_dir().join(format!(
        "gmark-preferences-resource-migration-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&root).expect("temp root should exist");
    let dirs = AppDirs::from_root(&root);

    std::fs::write(
        dirs.app_config_file(),
        r#"
                [editor]
                image_paste_behavior = "copy_to_named_assets_folder"
            "#,
    )
    .expect("legacy preferences should be written");
    let migrated = load_or_create_app_preferences_with_dirs_and_locales(&dirs, ["en-US"])
        .expect("legacy preferences should migrate");
    assert_eq!(
        migrated.resource_insert_behavior(),
        ResourceInsertBehavior::CopyToNamedAssetsFolder
    );
    let normalized =
        std::fs::read_to_string(dirs.app_config_file()).expect("migrated config should exist");
    assert!(normalized.contains(
        "resource_insert_behavior = \"copy_to_named_assets_folder\""
    ));
    assert!(normalized.contains(
        "image_paste_behavior = \"copy_to_named_assets_folder\""
    ));

    std::fs::write(
        dirs.app_config_file(),
        r#"
                [editor]
                resource_insert_behavior = "copy_to_document_folder"
                image_paste_behavior = "copy_to_assets_folder"
            "#,
    )
    .expect("mixed-version preferences should be written");
    let preferred =
        read_app_preferences_with_dirs(&dirs).expect("mixed-version preferences should load");
    assert_eq!(
        preferred.resource_insert_behavior(),
        ResourceInsertBehavior::CopyToDocumentFolder
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn resource_insert_behavior_round_trips_all_modes_and_dual_writes() {
    let root = std::env::temp_dir().join(format!(
        "gmark-preferences-resource-round-trip-{}",
        uuid::Uuid::new_v4()
    ));
    let dirs = AppDirs::from_root(&root);

    for behavior in [
        ResourceInsertBehavior::None,
        ResourceInsertBehavior::CopyToDocumentFolder,
        ResourceInsertBehavior::CopyToAssetsFolder,
        ResourceInsertBehavior::CopyToNamedAssetsFolder,
    ] {
        let preferences = AppPreferences {
            image_paste_behavior: behavior,
            ..AppPreferences::default()
        };
        save_app_preferences_with_dirs(&preferences, &dirs)
            .expect("resource preferences should save");

        let loaded =
            read_app_preferences_with_dirs(&dirs).expect("resource preferences should reload");
        assert_eq!(loaded.resource_insert_behavior(), behavior);

        let text =
            std::fs::read_to_string(dirs.app_config_file()).expect("config should exist");
        assert!(text.contains(&format!(
            "resource_insert_behavior = \"{}\"",
            behavior.as_str()
        )));
        assert!(text.contains(&format!(
            "image_paste_behavior = \"{}\"",
            behavior.as_str()
        )));
    }

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn unknown_auto_save_value_falls_back_to_off() {
    let root = std::env::temp_dir().join(format!(
        "gmark-preferences-auto-save-invalid-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&root).expect("temp root should exist");
    let dirs = AppDirs::from_root(&root);
    std::fs::write(
        dirs.app_config_file(),
        r#"
                [editor]
                auto_save = "always_without_conflict_checks"
            "#,
    )
    .expect("preferences should be written");

    let preferences = read_app_preferences_with_dirs(&dirs).expect("preferences should load");
    assert_eq!(preferences.auto_save, AutoSavePreference::Off);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn editor_typography_preferences_are_bounded_and_quantized() {
    let root = std::env::temp_dir().join(format!(
        "gmark-preferences-typography-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&root).expect("temp root should exist");
    let dirs = AppDirs::from_root(&root);
    std::fs::write(
        dirs.app_config_file(),
        r#"
                [editor]
                font_size = 99
                line_height_percent = 163
                content_width = 901
                font_family = " Georgia "
            "#,
    )
    .expect("preferences should be written");

    let preferences = read_app_preferences_with_dirs(&dirs).expect("preferences should load");
    assert_eq!(preferences.editor_font_size, 24);
    assert_eq!(preferences.editor_line_height_percent, 165);
    assert_eq!(preferences.editor_content_width, 920);
    assert_eq!(preferences.editor_font_family, "Georgia");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn damaged_preferences_file_returns_defaults() {
    let root = std::env::temp_dir().join(format!(
        "gmark-preferences-damaged-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&root).expect("temp root should exist");
    let dirs = AppDirs::from_root(&root);
    std::fs::write(dirs.app_config_file(), "not = [valid").expect("preferences should be written");

    let preferences =
        read_app_preferences_with_dirs(&dirs).expect("damaged preferences should load");
    assert_eq!(preferences, AppPreferences::default());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn saves_and_reads_preferences() {
    let root =
        std::env::temp_dir().join(format!("gmark-preferences-save-{}", uuid::Uuid::new_v4()));
    let dirs = AppDirs::from_root(&root);
    let preferences = AppPreferences {
        startup_open: StartupOpenPreference::LastOpenedFile,
        auto_check_updates: false,
        default_language_id: "zh-CN".into(),
        theme_appearance: ThemeAppearance::Light,
        theme_palette: ThemePalette::Xcode,
        visual_accessibility: Default::default(),
        show_table_headers: false,
        show_hidden_files: true,
        image_paste_behavior: ImagePasteBehavior::CopyToAssetsFolder,
        auto_save: AutoSavePreference::AfterDelay,
        spell_check: false,
        auto_pair_brackets: false,
        auto_pair_markdown: true,
        code_folding: true,
        format_on_save: false,
        editor_font_size: 19,
        editor_line_height_percent: 170,
        editor_content_width: 920,
        editor_font_family: "Georgia".into(),
        show_tab_bar_actions: true,
        recent_editing_commands: Vec::new(),
        keybindings: BTreeMap::new(),
        status_bar: StatusBarPreferences::default(),
        document_loading: DocumentLoadingPreferences {
            max_resident_mib: Some(32),
        },
    };

    save_app_preferences_with_dirs(&preferences, &dirs)
        .expect("preferences should save to config.toml");
    let loaded = read_app_preferences_with_dirs(&dirs).expect("preferences should read back");
    assert_eq!(loaded, preferences);

    let text = std::fs::read_to_string(dirs.app_config_file()).expect("config.toml should exist");
    assert!(text.contains("open = \"last_opened_file\""));
    assert!(text.contains("auto_check = false"));
    assert!(text.contains("default_language_id = \"zh-CN\""));
    assert!(text.contains("appearance = \"light\""));
    assert!(text.contains("palette = \"xcode\""));
    assert!(!text.contains("default_theme_id"));
    assert!(text.contains("show_table_headers = false"));
    assert!(text.contains("show_hidden_files = true"));
    assert!(text.contains("resource_insert_behavior = \"copy_to_assets_folder\""));
    assert!(text.contains("image_paste_behavior = \"copy_to_assets_folder\""));
    assert!(text.contains("auto_save = \"after_delay\""));
    assert!(text.contains("auto_pair_brackets = false"));
    assert!(text.contains("auto_pair_markdown = true"));
    assert!(text.contains("font_size = 19"));
    assert!(text.contains("line_height_percent = 170"));
    assert!(text.contains("content_width = 920"));
    assert!(text.contains("font_family = \"Georgia\""));
    assert!(!text.contains("workspace_sidebar_position"));
    assert!(text.contains("max_resident_mib = 32"));
    assert!(!text.contains("preset ="));
    assert!(!text.contains("max_resident_lines"));
    assert!(!text.contains("max_structural_units"));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn loading_preferences_apply_valid_overrides_and_ignore_invalid_values() {
    let valid = DocumentLoadingPreferences {
        max_resident_mib: Some(32),
    }
    .policy()
    .effective_limits();
    assert_eq!(valid.max_resident_bytes, 32 * 1024 * 1024);

    let fallback = DocumentLoadingPreferences {
        max_resident_mib: Some(0),
    }
    .policy()
    .effective_limits();
    assert_eq!(fallback, gmark_document_core::DEFAULT_LOADING_LIMITS);

    let valid_preferences = DocumentLoadingPreferences {
        max_resident_mib: Some(24),
    };
    assert!(!valid_preferences.has_invalid_override());
    assert!(
        DocumentLoadingPreferences {
            max_resident_mib: Some(1_025),
        }
        .has_invalid_override()
    );
}

#[test]
fn missing_preferences_file_is_created_with_detected_language() {
    let root =
        std::env::temp_dir().join(format!("gmark-preferences-create-{}", uuid::Uuid::new_v4()));
    let dirs = AppDirs::from_root(&root);
    let preferences = load_or_create_app_preferences_with_dirs_and_locales(&dirs, ["zh-HK"])
        .expect("preferences should be created");
    assert_eq!(preferences.default_language_id, "zh-CN");
    assert!(dirs.app_config_file().exists());
    let text = std::fs::read_to_string(dirs.app_config_file()).expect("config.toml should exist");
    assert!(text.contains("[language]"));
    assert!(text.contains("default_language_id = \"zh-CN\""));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn legacy_preferences_are_normalized_with_language() {
    let root =
        std::env::temp_dir().join(format!("gmark-preferences-legacy-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).expect("temp root should exist");
    let dirs = AppDirs::from_root(&root);
    std::fs::write(
        dirs.app_config_file(),
        r#"
                [startup]
                open = "last_opened_file"

                [theme]
                default_theme_id = "gmark-light"
            "#,
    )
    .expect("legacy preferences should be written");

    let preferences = load_or_create_app_preferences_with_dirs_and_locales(&dirs, ["en-GB"])
        .expect("legacy preferences should normalize");
    assert_eq!(
        preferences.startup_open,
        StartupOpenPreference::LastOpenedFile
    );
    assert_eq!(preferences.default_language_id, "en-US");
    assert_eq!(preferences.theme_appearance, ThemeAppearance::System);
    assert_eq!(preferences.theme_palette, ThemePalette::Xcode);
    let text = std::fs::read_to_string(dirs.app_config_file()).expect("config.toml should exist");
    assert!(text.contains("[language]"));
    assert!(text.contains("appearance = \"system\""));
    assert!(text.contains("palette = \"xcode\""));
    assert!(!text.contains("default_theme_id"));
    let _ = std::fs::remove_dir_all(root);
}
