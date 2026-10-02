// @author kongweiguang

use super::{applescript_string_literal, build_menus};
use crate::components::{
    AddLanguageConfig, CheckForUpdates, CloseTab, CloseWindow, CommandPalette, CopyAsMarkdown,
    ExportPdf, FindInDocument, FindNext, FindPrevious, NewTab, NewWindow, NoRecentFiles,
    OpenCrashReports, OpenFile, OpenPreferences, OpenPrivacyPolicy, OpenRecentFile, QuickOpen,
    QuitApplication, ReopenClosedTab, ReplaceInDocument, SaveDocument, SelectLanguage, ShowAbout,
    ToggleWorkspace,
};
use crate::i18n::I18nManager;
use crate::theme::ThemeManager;
use gpui::{Action, MenuItem};
use std::path::PathBuf;

fn action_name(item: &MenuItem) -> &str {
    match item {
        MenuItem::Action { name, .. } => name.as_ref(),
        _ => panic!("expected action menu item"),
    }
}

#[test]
fn fallback_menu_actions_use_semantic_local_icons() {
    assert_eq!(
        super::menu_action_icon(&SaveDocument),
        Some("icon/ui/save.svg")
    );
    assert_eq!(
        super::menu_action_icon(&crate::components::Undo),
        Some("icon/ui/undo.svg")
    );
    assert_eq!(
        super::menu_action_icon(&crate::components::Cut),
        Some("icon/ui/scissors.svg")
    );
    assert_eq!(
        super::menu_action_icon(&OpenPreferences),
        Some("icon/ui/sliders.svg")
    );
    assert_eq!(
        super::menu_action_icon(&ExportPdf),
        Some("icon/ui/file-output.svg")
    );
    assert_eq!(
        super::menu_action_icon(&OpenPrivacyPolicy),
        Some("icon/ui/shield.svg")
    );
    assert_eq!(
        super::menu_action_icon(&crate::components::QuickOpen),
        Some("icon/ui/files.svg")
    );
    assert_eq!(
        super::menu_action_icon(&crate::components::CommandPalette),
        Some("icon/ui/search.svg")
    );
    assert_eq!(
        super::menu_action_icon(&crate::components::PreviousTab),
        Some("icon/ui/arrow-left.svg")
    );
    assert_eq!(
        super::menu_action_icon(&crate::components::NextTab),
        Some("icon/ui/arrow-right.svg")
    );
    assert_eq!(
        super::menu_action_icon(&crate::components::ToggleViewMode),
        Some("icon/ui/live.svg")
    );
}

fn submenu_named<'a>(menu: &'a gpui::Menu, name: &str) -> &'a gpui::Menu {
    menu.items
        .iter()
        .find_map(|item| match item {
            MenuItem::Submenu(submenu) if submenu.name.as_ref() == name => Some(submenu),
            _ => None,
        })
        .unwrap_or_else(|| panic!("missing submenu '{name}'"))
}

fn action_named<'a>(menu: &'a gpui::Menu, name: &str) -> &'a MenuItem {
    menu.items
            .iter()
            .find(|item| matches!(item, MenuItem::Action { name: item_name, .. } if item_name.as_ref() == name))
            .unwrap_or_else(|| panic!("missing action '{name}'"))
}

fn action_named_has_type<A: Action>(menu: &gpui::Menu, name: &str) -> bool {
    matches!(
        action_named(menu, name),
        MenuItem::Action { action, .. } if action.as_any().is::<A>()
    )
}

#[test]
fn applescript_string_literal_escapes_special_characters() {
    assert_eq!(
        applescript_string_literal(r#"/Applications/gmark "Test".app/Contents/MacOS/gmark"#),
        r#""/Applications/gmark \"Test\".app/Contents/MacOS/gmark""#
    );
    assert_eq!(
        applescript_string_literal(r#"/Applications/O'Brien\gmark.app"#),
        r#""/Applications/O'Brien\\gmark.app""#
    );
}

// 所有平台的第一项都是应用菜单；Windows/Linux 客户端标题栏将其渲染为菜单图标。
const FILE_IDX: usize = 1;

const EDIT_IDX: usize = 2;

const VIEW_IDX: usize = 3;

const HELP_IDX: usize = 4;

#[test]
fn build_menus_uses_english_fallback_by_default() {
    let theme_manager = ThemeManager::default();
    let i18n_manager = I18nManager::default();
    let menus = build_menus(&theme_manager, &i18n_manager, &[]);

    let menu_names = menus
        .iter()
        .map(|menu| menu.name.to_string())
        .collect::<Vec<_>>();

    #[cfg(target_os = "macos")]
    assert_eq!(menu_names, vec!["Gmark", "File", "Edit", "View", "Help"]);
    #[cfg(not(target_os = "macos"))]
    assert_eq!(menu_names, vec!["Gmark", "File", "Edit", "View", "Help"]);

    // New Tab is the primary document action; New Window remains adjacent.
    #[cfg(target_os = "macos")]
    assert_eq!(action_name(&menus[1].items[0]), "New Tab");
    #[cfg(not(target_os = "macos"))]
    assert_eq!(action_name(&menus[1].items[0]), "New Tab");

    // Open Recent File submenu location differs by platform.
    assert_eq!(
        submenu_named(&menus[FILE_IDX], "Open Recent File")
            .name
            .to_string(),
        "Open Recent File"
    );

    let file_menu = &menus[FILE_IDX];
    assert!(action_named_has_type::<NewTab>(file_menu, "New Tab"));
    assert_eq!(
        action_name(action_named(file_menu, "Close Tab")),
        "Close Tab"
    );
    assert_eq!(
        action_name(action_named(file_menu, "Reopen Closed Tab")),
        "Reopen Closed Tab"
    );
    assert_eq!(
        action_name(action_named(file_menu, "Close Window")),
        "Close Window"
    );
    assert!(action_named_has_type::<CloseTab>(file_menu, "Close Tab"));
    assert!(action_named_has_type::<ReopenClosedTab>(
        file_menu,
        "Reopen Closed Tab"
    ));
    let edit_menu = &menus[EDIT_IDX];
    assert!(action_named_has_type::<FindInDocument>(
        edit_menu,
        "Find in Document"
    ));
    assert!(action_named_has_type::<ReplaceInDocument>(
        edit_menu,
        "Replace in Document"
    ));
    assert!(action_named_has_type::<FindNext>(edit_menu, "Find Next"));
    assert!(action_named_has_type::<FindPrevious>(
        edit_menu,
        "Find Previous"
    ));

    assert_eq!(action_name(&menus[0].items[0]), "Preferences");
    #[cfg(not(target_os = "macos"))]
    {
        assert!(action_named_has_type::<CheckForUpdates>(
            &menus[0],
            "Check for Updates"
        ));
        assert!(action_named_has_type::<ShowAbout>(&menus[0], "About"));
        assert!(action_named_has_type::<QuitApplication>(&menus[0], "Quit"));
    }

    assert!(file_menu.items.iter().all(
        |item| !matches!(item, MenuItem::Submenu(submenu) if submenu.name.as_ref() == "Export" || submenu.name.as_ref() == "Line Endings")
    ));
    let view_menu = &menus[VIEW_IDX];
    assert!(action_named_has_type::<ToggleWorkspace>(
        view_menu,
        "Toggle Workspace"
    ));
    assert!(menus.iter().all(|menu| menu.name.as_ref() != "Paragraph"));
    assert!(menus.iter().all(|menu| menu.name.as_ref() != "Format"));
}

/// Global menus are built without an editor target, so document-only commands stay contextual.
#[test]
fn navigation_keeps_only_global_non_editor_capabilities() {
    let theme_manager = ThemeManager::default();
    let i18n_manager = I18nManager::default();
    let menus = build_menus(&theme_manager, &i18n_manager, &[]);
    let menu_names = menus
        .iter()
        .map(|menu| menu.name.as_ref())
        .collect::<Vec<_>>();

    assert_eq!(menu_names, vec!["Gmark", "File", "Edit", "View", "Help"]);
    let edit_menu = &menus[EDIT_IDX];
    assert!(edit_menu.items.iter().all(
        |item| !matches!(item, MenuItem::Action { action, .. } if action.as_any().is::<CopyAsMarkdown>())
    ));
    let view_menu = &menus[VIEW_IDX];
    assert!(view_menu.items.iter().all(
        |item| !matches!(item, MenuItem::Action { action, .. } if action.as_any().is::<QuickOpen>() || action.as_any().is::<CommandPalette>())
    ));
    assert!(view_menu.items.iter().all(
        |item| !matches!(item, MenuItem::Submenu(submenu) if submenu.name.as_ref() == "Language")
    ));
}

#[test]
fn build_menus_uses_chinese_language_when_selected() {
    let theme_manager = ThemeManager::default();
    let i18n_manager = I18nManager::new_with_language_id("zh-CN");
    let menus = build_menus(&theme_manager, &i18n_manager, &[]);

    assert_eq!(
        submenu_named(
            &menus[FILE_IDX],
            &i18n_manager.strings().menu_open_recent_file,
        )
        .name
        .to_string(),
        i18n_manager.strings().menu_open_recent_file.as_str()
    );

    let menu_names = menus
        .iter()
        .map(|menu| menu.name.to_string())
        .collect::<Vec<_>>();

    #[cfg(target_os = "macos")]
    assert_eq!(menu_names, vec!["Gmark", "文件", "编辑", "视图", "帮助"]);
    #[cfg(not(target_os = "macos"))]
    assert_eq!(menu_names, vec!["Gmark", "文件", "编辑", "视图", "帮助"]);

    #[cfg(target_os = "macos")]
    assert_eq!(action_name(&menus[1].items[0]), "新建标签页");
    #[cfg(not(target_os = "macos"))]
    assert_eq!(action_name(&menus[1].items[0]), "新建标签页");
    let file_menu = &menus[FILE_IDX];
    assert!(file_menu.items.iter().all(
        |item| !matches!(item, MenuItem::Submenu(submenu) if submenu.name.as_ref() == "导出" || submenu.name.as_ref() == "换行符")
    ));
    let view_menu = &menus[VIEW_IDX];
    assert!(action_named_has_type::<ToggleWorkspace>(
        view_menu,
        "切换工作区"
    ));
}

#[test]
fn recent_files_submenu_uses_empty_state_when_history_is_empty() {
    let theme_manager = ThemeManager::default();
    let i18n_manager = I18nManager::default();
    let menus = build_menus(&theme_manager, &i18n_manager, &[]);

    // On macOS: File menu is index 1. Elsewhere it is index 0.
    let recent_menu = submenu_named(&menus[FILE_IDX], "Open Recent File");

    assert_eq!(recent_menu.name.to_string(), "Open Recent File");
    assert_eq!(recent_menu.items.len(), 1);
    assert_eq!(action_name(&recent_menu.items[0]), "No Recent Files");
    match &recent_menu.items[0] {
        MenuItem::Action { action, .. } => {
            assert!(action.as_any().is::<NoRecentFiles>());
        }
        _ => panic!("expected empty recent-file action item"),
    }
}

#[test]
fn recent_files_submenu_dispatches_path_actions() {
    let theme_manager = ThemeManager::default();
    let i18n_manager = I18nManager::default();
    let recent_files = vec![
        PathBuf::from(r"C:\docs\one.md"),
        PathBuf::from(r"D:\notes\two.markdown"),
    ];
    let menus = build_menus(&theme_manager, &i18n_manager, &recent_files);

    let recent_menu = submenu_named(&menus[FILE_IDX], "Open Recent File");

    assert_eq!(recent_menu.items.len(), 2);
    assert_eq!(action_name(&recent_menu.items[0]), r"C:\docs\one.md");
    match &recent_menu.items[0] {
        MenuItem::Action { action, .. } => {
            let action = action
                .as_any()
                .downcast_ref::<OpenRecentFile>()
                .expect("recent file should dispatch OpenRecentFile");
            assert_eq!(action.path, r"C:\docs\one.md");
        }
        _ => panic!("expected recent-file action item"),
    }
}

#[test]
fn fallback_menu_routes_window_context_actions_without_app_defer() {
    assert!(super::is_window_context_menu_action(&NewTab));
    assert!(super::is_window_context_menu_action(&NewWindow));
    assert!(super::is_window_context_menu_action(&OpenFile));
    assert!(super::is_window_context_menu_action(&OpenPreferences));
    assert!(super::is_window_context_menu_action(&OpenRecentFile {
        path: "notes.md".into(),
    }));
    assert!(super::is_window_context_menu_action(&NoRecentFiles));
    assert!(super::is_window_context_menu_action(&AddLanguageConfig));
    assert!(super::is_window_context_menu_action(&OpenCrashReports));
    assert!(super::is_window_context_menu_action(&OpenPrivacyPolicy));
    assert!(super::is_window_context_menu_action(&SaveDocument));
    assert!(super::is_window_context_menu_action(&QuitApplication));
    assert!(super::is_window_context_menu_action(&CloseWindow));
    assert!(!super::is_window_context_menu_action(&SelectLanguage {
        language_id: "en-US".into(),
    }));
}

#[test]
#[cfg(target_os = "macos")]
fn help_menu_first_item_is_check_for_updates() {
    let theme_manager = ThemeManager::default();
    let i18n_manager = I18nManager::default();
    let menus = build_menus(&theme_manager, &i18n_manager, &[]);
    let help_items = &menus[HELP_IDX].items;

    match &help_items[0] {
        MenuItem::Action { action, .. } => {
            assert!(action.as_any().is::<CheckForUpdates>());
        }
        _ => panic!("expected check updates action item"),
    }
    assert!(matches!(help_items[1], MenuItem::Separator));
}

#[test]
#[cfg(not(target_os = "macos"))]
fn help_menu_keeps_diagnostics_and_privacy_after_app_actions_move_to_launcher() {
    let theme_manager = ThemeManager::default();
    let i18n_manager = I18nManager::default();
    let menus = build_menus(&theme_manager, &i18n_manager, &[]);
    let help_items = &menus[HELP_IDX].items;

    assert_eq!(help_items.len(), 2);
    match &help_items[0] {
        MenuItem::Action { action, .. } => {
            assert!(action.as_any().is::<OpenCrashReports>());
        }
        _ => panic!("expected crash reports action item"),
    }
    match &help_items[1] {
        MenuItem::Action { action, .. } => {
            assert!(action.as_any().is::<OpenPrivacyPolicy>());
        }
        _ => panic!("expected privacy action item"),
    }
}

#[test]
#[cfg(target_os = "macos")]
fn help_menu_contains_cli_and_about_on_macos() {
    let theme_manager = ThemeManager::default();
    let i18n_manager = I18nManager::default();
    let menus = build_menus(&theme_manager, &i18n_manager, &[]);
    let help_items = &menus[HELP_IDX].items;

    // Update, CLI, diagnostics/privacy and About are separated into stable groups.
    assert_eq!(help_items.len(), 8);
    assert!(matches!(help_items[3], MenuItem::Separator));
    assert!(matches!(help_items[6], MenuItem::Separator));
    match &help_items[4] {
        MenuItem::Action { action, .. } => {
            assert!(action.as_any().is::<OpenCrashReports>());
        }
        _ => panic!("expected crash reports action item"),
    }
    match &help_items[5] {
        MenuItem::Action { action, .. } => {
            assert!(action.as_any().is::<OpenPrivacyPolicy>());
        }
        _ => panic!("expected privacy action item"),
    }
    match &help_items[7] {
        MenuItem::Action { action, .. } => {
            assert!(action.as_any().is::<ShowAbout>());
        }
        _ => panic!("expected about action item"),
    }
}
