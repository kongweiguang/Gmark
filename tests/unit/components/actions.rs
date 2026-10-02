// @author kongweiguang

use super::{
    ShortcutCategory, ShortcutCommand, format_shortcut_for_display, normalize_shortcut_config,
    resolved_shortcut_keys, shortcut_conflict_for, shortcut_definitions,
};
use gpui::Keystroke;
use std::collections::{BTreeMap, BTreeSet};

/// 将静态键位转换为运行时比较形式，保持断言只关注产品契约而非分配细节。
fn owned_keys(keys: &[&str]) -> Vec<String> {
    keys.iter().map(|key| (*key).to_owned()).collect()
}

/// 根据当前测试目标选择平台期望值，同时让同一断言明确记录 Windows/Linux 与 macOS 契约。
fn current_platform_keys(windows_linux: &[&str], macos: &[&str]) -> Vec<String> {
    if cfg!(target_os = "macos") {
        owned_keys(macos)
    } else {
        owned_keys(windows_linux)
    }
}

/// 同时核对两套静态默认值，Windows CI 也能防止 macOS 键位在重构中回退。
fn assert_platform_defaults(command: ShortcutCommand, windows_linux: &[&str], macos: &[&str]) {
    let definition = shortcut_definitions()
        .iter()
        .find(|definition| definition.command == command)
        .expect("shortcut command should have a definition");
    assert_eq!(definition.default_keys.windows_linux, windows_linux);
    assert_eq!(definition.default_keys.macos, macos);
    assert_eq!(
        resolved_shortcut_keys(&BTreeMap::new(), command),
        current_platform_keys(windows_linux, macos)
    );
}

#[test]
fn shortcut_display_formatting_uses_ui_labels() {
    assert_eq!(format_shortcut_for_display("ctrl-alt-i"), "Ctrl+Alt+I");
    #[cfg(target_os = "macos")]
    assert_eq!(format_shortcut_for_display("cmd-shift-s"), "Cmd+Shift+S");
    #[cfg(target_os = "windows")]
    assert_eq!(format_shortcut_for_display("cmd-shift-s"), "Shift+Win+S");
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    assert_eq!(format_shortcut_for_display("cmd-shift-s"), "Shift+Super+S");
    assert_eq!(format_shortcut_for_display("escape"), "Esc");
}

#[test]
fn shortcut_display_formatting_supports_space_separated_candidates() {
    #[cfg(target_os = "macos")]
    assert_eq!(format_shortcut_for_display("ctrl-a cmd-a"), "Ctrl+A Cmd+A");
    #[cfg(target_os = "windows")]
    assert_eq!(format_shortcut_for_display("ctrl-a cmd-a"), "Ctrl+A Win+A");
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    assert_eq!(
        format_shortcut_for_display("ctrl-a cmd-a"),
        "Ctrl+A Super+A"
    );
}

#[test]
fn shortcut_display_formatting_preserves_unreadable_fallbacks() {
    assert_eq!(format_shortcut_for_display(""), "");
    assert_eq!(
        format_shortcut_for_display("ctrl--shortcut"),
        "ctrl--shortcut"
    );
}

/// 自定义键可以替换默认值，但测试刻意避开 IDEA 已占用的偏好设置组合。
#[test]
fn custom_shortcut_replaces_command_defaults() {
    let mut config = BTreeMap::new();
    config.insert(
        "save_document".to_string(),
        vec!["ctrl-alt-shift-s".to_string()],
    );

    assert_eq!(
        resolved_shortcut_keys(&config, ShortcutCommand::SaveDocument),
        vec!["ctrl-alt-shift-s".to_string()]
    );
}

/// 锁定 IDEA 优先、Typora 补充的两套产品键位，避免只在当前 Windows 构建上验证 macOS。
#[test]
fn idea_first_defaults_cover_windows_linux_and_macos() {
    assert_platform_defaults(ShortcutCommand::SaveDocument, &["ctrl-s"], &["cmd-s"]);
    assert_platform_defaults(
        ShortcutCommand::PasteAsPlainText,
        &["ctrl-alt-shift-v"],
        &["cmd-alt-shift-v"],
    );
    assert_platform_defaults(ShortcutCommand::Redo, &["ctrl-shift-z"], &["cmd-shift-z"]);
    assert_platform_defaults(
        ShortcutCommand::FormatDocument,
        &["ctrl-alt-l"],
        &["cmd-alt-l"],
    );
    assert_platform_defaults(
        ShortcutCommand::OpenPreferences,
        &["ctrl-alt-s"],
        &["cmd-,"],
    );
    assert_platform_defaults(
        ShortcutCommand::CommandPalette,
        &["ctrl-shift-a"],
        &["cmd-shift-a"],
    );
    assert_platform_defaults(ShortcutCommand::GoToLine, &["ctrl-g"], &["cmd-l"]);
    assert_platform_defaults(ShortcutCommand::FindInDocument, &["ctrl-f"], &["cmd-f"]);
    assert_platform_defaults(ShortcutCommand::ReplaceInDocument, &["ctrl-r"], &["cmd-r"]);
    assert_platform_defaults(ShortcutCommand::FindNext, &["f3"], &["cmd-g"]);
    assert_platform_defaults(
        ShortcutCommand::FindPrevious,
        &["shift-f3"],
        &["cmd-shift-g"],
    );
    assert_platform_defaults(
        ShortcutCommand::PreviousTab,
        &["alt-left"],
        &["cmd-shift-["],
    );
    assert_platform_defaults(ShortcutCommand::NextTab, &["alt-right"], &["cmd-shift-]"]);
    assert_platform_defaults(ShortcutCommand::ToggleViewMode, &["ctrl-/"], &["cmd-/"]);
    assert_platform_defaults(ShortcutCommand::ToggleWorkspace, &["alt-1"], &["cmd-1"]);
    assert_platform_defaults(ShortcutCommand::ToggleFocusMode, &["f8"], &["f8"]);
    assert_platform_defaults(ShortcutCommand::ToggleTypewriterMode, &["f9"], &["f9"]);
    assert_platform_defaults(ShortcutCommand::SetParagraph, &["ctrl-0"], &["cmd-0"]);
    assert_platform_defaults(ShortcutCommand::SetHeading1, &["ctrl-1"], &["cmd-1"]);
    assert_platform_defaults(ShortcutCommand::SetHeading6, &["ctrl-6"], &["cmd-6"]);
    assert_platform_defaults(
        ShortcutCommand::CodeSelection,
        &["ctrl-shift-`"],
        &["cmd-shift-`"],
    );
    assert_platform_defaults(
        ShortcutCommand::StrikethroughSelection,
        &["alt-shift-5"],
        &["cmd-shift-x"],
    );
}

/// 校验两套默认键都可解析、无平台修饰键串用，并且同一上下文不存在重复占用。
#[test]
fn platform_defaults_are_valid_and_conflict_free() {
    for macos in [false, true] {
        let platform_name = if macos { "macOS" } else { "Windows/Linux" };
        for definition in shortcut_definitions() {
            let keys = if macos {
                definition.default_keys.macos
            } else {
                definition.default_keys.windows_linux
            };
            if !keys.is_empty() {
                for key in keys {
                    assert!(
                        Keystroke::parse(key).is_ok(),
                        "{} has invalid {platform_name} key {key}",
                        definition.id
                    );
                }
                assert_eq!(
                    keys.iter().collect::<BTreeSet<_>>().len(),
                    keys.len(),
                    "{} has duplicate keys",
                    definition.id
                );
            }
            if macos {
                assert!(
                    keys.iter().all(|key| !key.starts_with("ctrl-")),
                    "{} mixes Windows/Linux Ctrl keys into macOS: {keys:?}",
                    definition.id
                );
            } else {
                assert!(
                    keys.iter().all(|key| !key.starts_with("cmd-")),
                    "{} mixes macOS Cmd keys into Windows/Linux: {keys:?}",
                    definition.id
                );
            }
        }

        for (index, left) in shortcut_definitions().iter().enumerate() {
            let left_keys = if macos {
                left.default_keys.macos
            } else {
                left.default_keys.windows_linux
            };
            for right in shortcut_definitions().iter().skip(index + 1) {
                if left.context != right.context {
                    continue;
                }
                let right_keys = if macos {
                    right.default_keys.macos
                } else {
                    right.default_keys.windows_linux
                };
                assert!(
                    !left_keys.iter().any(|key| right_keys.contains(key)),
                    "{} and {} conflict on {platform_name}: {left_keys:?} / {right_keys:?}",
                    left.id,
                    right.id
                );
            }
        }
    }
}

/// 没有行业惯例的窗格操作保持未绑定，但仍允许用户在偏好设置中显式配置。
#[test]
fn pane_commands_are_configurable_without_global_defaults() {
    let commands = [
        ShortcutCommand::SplitRight,
        ShortcutCommand::SplitDown,
        ShortcutCommand::ClosePane,
        ShortcutCommand::FocusPaneLeft,
        ShortcutCommand::FocusPaneRight,
        ShortcutCommand::FocusPaneUp,
        ShortcutCommand::FocusPaneDown,
        ShortcutCommand::MoveTabToPaneLeft,
        ShortcutCommand::MoveTabToPaneRight,
        ShortcutCommand::MoveTabToPaneUp,
        ShortcutCommand::MoveTabToPaneDown,
        ShortcutCommand::BalancePanes,
    ];

    for command in commands {
        assert!(resolved_shortcut_keys(&BTreeMap::new(), command).is_empty());
        let definition = shortcut_definitions()
            .iter()
            .find(|definition| definition.command == command)
            .expect("pane command should have a shortcut definition");
        assert_eq!(definition.category, ShortcutCategory::Navigation);
        assert!(definition.default_keys.windows_linux.is_empty());
        assert!(definition.default_keys.macos.is_empty());
        let mut config = BTreeMap::new();
        config.insert(
            definition.id.to_owned(),
            vec!["ctrl-alt-shift-p".to_owned()],
        );
        assert_eq!(
            resolved_shortcut_keys(&config, command),
            vec!["ctrl-alt-shift-p".to_owned()]
        );
    }
}

/// 链接、复制 Markdown 与打开目录沿用 Markdown/桌面编辑器的成熟组合。
#[test]
fn common_markdown_and_file_shortcuts_are_default() {
    assert_platform_defaults(ShortcutCommand::LinkSelection, &["ctrl-k"], &["cmd-k"]);
    assert_platform_defaults(
        ShortcutCommand::CopyAsMarkdown,
        &["ctrl-shift-c"],
        &["cmd-shift-c"],
    );
    assert_platform_defaults(
        ShortcutCommand::OpenFolder,
        &["ctrl-shift-o"],
        &["cmd-shift-o"],
    );
    assert_platform_defaults(
        ShortcutCommand::NewTab,
        &["ctrl-n", "ctrl-t"],
        &["cmd-n", "cmd-t"],
    );
    assert_platform_defaults(
        ShortcutCommand::ReopenClosedTab,
        &["ctrl-shift-t"],
        &["cmd-shift-t"],
    );
}

/// 全选使用平台主修饰键，并且不会与同上下文的默认动作冲突。
#[test]
fn select_all_has_default_shortcuts() {
    let keys = current_platform_keys(&["ctrl-a"], &["cmd-a"]);
    assert_eq!(
        resolved_shortcut_keys(&BTreeMap::new(), ShortcutCommand::SelectAll),
        keys
    );
    assert!(
        shortcut_conflict_for(
            ShortcutCommand::SelectAll,
            &current_platform_keys(&["ctrl-a"], &["cmd-a"]),
            &BTreeMap::new()
        )
        .is_none()
    );
}

#[test]
fn select_all_shortcut_can_be_customized() {
    let mut config = BTreeMap::new();
    config.insert("select_all".to_string(), vec!["ctrl-shift-a".to_string()]);

    assert_eq!(
        resolved_shortcut_keys(&config, ShortcutCommand::SelectAll),
        vec!["ctrl-shift-a".to_string()]
    );
}

#[test]
fn legacy_split_select_all_shortcut_config_maps_to_unified_command() {
    let mut config = BTreeMap::new();
    config.insert(
        "select_all_source_text".to_string(),
        vec!["ctrl-shift-a".to_string()],
    );

    assert_eq!(
        resolved_shortcut_keys(&config, ShortcutCommand::SelectAll),
        vec!["ctrl-shift-a".to_string()]
    );

    let normalized = normalize_shortcut_config(&config);
    assert_eq!(
        normalized.get("select_all"),
        Some(&vec!["ctrl-shift-a".to_string()])
    );
    assert!(!normalized.contains_key("select_all_source_text"));
    assert!(!normalized.contains_key("select_focused_block_text_rendered"));

    config.clear();
    config.insert(
        "select_focused_block_text_rendered".to_string(),
        vec!["ctrl-alt-shift-a".to_string()],
    );

    assert_eq!(
        resolved_shortcut_keys(&config, ShortcutCommand::SelectAll),
        vec!["ctrl-alt-shift-a".to_string()]
    );

    let normalized = normalize_shortcut_config(&config);
    assert_eq!(
        normalized.get("select_all"),
        Some(&vec!["ctrl-alt-shift-a".to_string()])
    );
    assert!(!normalized.contains_key("select_all_source_text"));
    assert!(!normalized.contains_key("select_focused_block_text_rendered"));
}

/// 退出遵循 macOS 的 Cmd+Q；Windows/Linux 继续交给系统 Alt+F4，不抢占应用级组合。
#[test]
fn close_and_quit_defaults_are_platform_specific() {
    assert_platform_defaults(
        ShortcutCommand::CloseWindow,
        &["ctrl-shift-w"],
        &["cmd-shift-w"],
    );
    assert_platform_defaults(ShortcutCommand::QuitApplication, &[], &["cmd-q"]);
}

/// 单词、块和选择移动使用各平台原生导航修饰键，而不是同时注册 Ctrl 与 Alt。
#[test]
fn word_and_block_shortcuts_follow_platform_navigation() {
    assert_platform_defaults(ShortcutCommand::WordMoveLeft, &["ctrl-left"], &["alt-left"]);
    assert_platform_defaults(
        ShortcutCommand::WordDeleteBack,
        &["ctrl-backspace"],
        &["alt-backspace"],
    );
    assert_platform_defaults(ShortcutCommand::BlockUp, &["ctrl-up"], &["alt-up"]);
    assert_platform_defaults(
        ShortcutCommand::WordSelectRight,
        &["ctrl-shift-right"],
        &["alt-shift-right"],
    );
}

/// Page actions remain viewport-oriented; caret actions own the document-start/end bindings.
#[test]
fn page_navigation_shortcuts_have_defaults() {
    assert_eq!(
        resolved_shortcut_keys(&BTreeMap::new(), ShortcutCommand::PageUp),
        vec!["pageup".to_string()]
    );
    assert_eq!(
        resolved_shortcut_keys(&BTreeMap::new(), ShortcutCommand::PageDown),
        vec!["pagedown".to_string()]
    );
    assert_eq!(
        resolved_shortcut_keys(&BTreeMap::new(), ShortcutCommand::JumpToTop),
        Vec::<String>::new()
    );
    assert_eq!(
        resolved_shortcut_keys(&BTreeMap::new(), ShortcutCommand::JumpToBottom),
        Vec::<String>::new()
    );
    assert_platform_defaults(
        ShortcutCommand::MoveToDocumentStart,
        &["ctrl-home"],
        &["cmd-up"],
    );
    assert_platform_defaults(
        ShortcutCommand::MoveToDocumentEnd,
        &["ctrl-end"],
        &["cmd-down"],
    );
    assert_platform_defaults(
        ShortcutCommand::SelectToDocumentStart,
        &["ctrl-shift-home"],
        &["cmd-shift-up"],
    );
    assert_platform_defaults(
        ShortcutCommand::SelectToDocumentEnd,
        &["ctrl-shift-end"],
        &["cmd-shift-down"],
    );
    assert_eq!(
        resolved_shortcut_keys(&BTreeMap::new(), ShortcutCommand::SelectPageUp),
        vec!["shift-pageup".to_string()]
    );
    assert_eq!(
        resolved_shortcut_keys(&BTreeMap::new(), ShortcutCommand::SelectPageDown),
        vec!["shift-pagedown".to_string()]
    );
}

/// Row-edit commands use familiar platform modifiers and remain independent from clipboard and redo bindings.
#[test]
fn line_edit_shortcuts_have_platform_defaults() {
    assert_platform_defaults(ShortcutCommand::DuplicateLine, &["ctrl-d"], &["cmd-d"]);
    assert_platform_defaults(ShortcutCommand::DeleteLine, &["ctrl-y"], &["cmd-y"]);
    assert_eq!(
        resolved_shortcut_keys(&BTreeMap::new(), ShortcutCommand::MoveLineUp),
        vec!["alt-shift-up".to_string()]
    );
    assert_eq!(
        resolved_shortcut_keys(&BTreeMap::new(), ShortcutCommand::MoveLineDown),
        vec!["alt-shift-down".to_string()]
    );
}

#[test]
fn invalid_or_empty_shortcuts_fall_back_to_defaults() {
    let mut config = BTreeMap::new();
    config.insert("save_document".to_string(), vec!["".to_string()]);
    config.insert("open_file".to_string(), vec!["a".to_string()]);

    let normalized = normalize_shortcut_config(&config);
    assert!(!normalized.contains_key("save_document"));
    assert!(!normalized.contains_key("open_file"));
}

/// 自定义键与同上下文命令冲突时回退到当前平台默认值，避免静默覆盖剪切。
#[test]
fn conflicting_custom_shortcut_falls_back_to_default() {
    let mut config = BTreeMap::new();
    config.insert(
        "copy".to_string(),
        current_platform_keys(&["ctrl-x"], &["cmd-x"]),
    );

    let normalized = normalize_shortcut_config(&config);
    assert!(!normalized.contains_key("copy"));
    assert_eq!(
        resolved_shortcut_keys(&config, ShortcutCommand::Copy),
        current_platform_keys(&["ctrl-c"], &["cmd-c"])
    );
}

/// 偏好设置录制阶段应在写盘前指出当前平台的剪切冲突。
#[test]
fn detects_shortcut_conflicts_for_preferences_drafts() {
    let conflict = shortcut_conflict_for(
        ShortcutCommand::Copy,
        &current_platform_keys(&["ctrl-x"], &["cmd-x"]),
        &BTreeMap::new(),
    )
    .expect("copy should conflict with cut");

    assert_eq!(conflict.id, "cut");
}
