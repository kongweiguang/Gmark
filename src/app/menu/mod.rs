// @author kongweiguang

//! Native application menu, app-level actions, and window close routing.
//!
//! This module owns menu construction and the actions that operate on the
//! active editor window. The Quit action is routed to the current window so the
//! existing unsaved-changes dialog remains authoritative for that window.

use std::path::{Path, PathBuf};

use gpui::*;

use crate::components::{
    AddLanguageConfig, BlockKind, BoldSelection, CheckForUpdates, CloseTab, CloseWindow,
    CodeSelection, CommandPalette, Copy, CopyAsMarkdown, Cut, DeleteLine, DuplicateLine,
    EditingCommandHistory, ExportHtml, ExportImage, ExportPdf, ExportSelection, FindInDocument,
    FindNext, FindPrevious, FocusStructuredColumns, FocusStructuredFilter, HighlightSelection,
    InlineMathSelection, InsertResource, InstallCliTool, ItalicSelection, LinkSelection,
    MoveLineDown, MoveLineUp, NewTab, NewWindow, NextTab, NoRecentFiles, NormalizeLineEndingsCr,
    NormalizeLineEndingsCrLf, NormalizeLineEndingsLf, OpenCrashReports, OpenFile, OpenFolder,
    OpenPreferences, OpenPrivacyPolicy, OpenRecentFile, OpenSafeSource, Paste, PasteAsPlainText,
    PreviousTab, QuickOpen, QuitApplication, Redo, ReopenClosedTab, ReplaceInDocument,
    SaveDocument, SaveDocumentAs, SelectAll, SelectLanguage, SetBulletedList, SetCodeBlock,
    SetHeading1, SetHeading2, SetHeading3, SetHeading4, SetHeading5, SetHeading6, SetNumberedList,
    SetParagraph, SetQuote, SetTaskList, ShowAbout, ShowDocumentInfo, ShowDocumentOutline,
    ShowStructureView, ShowStructuredInspector, StrikethroughSelection, SubscriptSelection,
    SuperscriptSelection, ToggleDocumentSidebar, ToggleFocusMode, ToggleTypewriterMode,
    ToggleViewMode, ToggleWorkspace, UnderlineSelection, Undo, UninstallCliTool,
};
use crate::config::{
    apply_configured_language, import_language_config_and_select, open_preferences_window,
};
use crate::editor::{Editor, InfoDialogKind};
use crate::export::ExportFormat;
use crate::i18n::I18nManager;
use crate::theme::ThemeManager;
use crate::window_chrome::{
    gmark_window_options, gmark_window_options_with_bounds, restored_window_bounds,
};

const PRIVACY_POLICY_URL: &str = "https://github.com/kongweiguang/gmark/blob/main/PRIVACY.md";

/// 自绘菜单和命令面板只消费语义图标 ID；原生菜单仍完全交给平台绘制。
pub(crate) fn menu_action_icon(action: &dyn Action) -> Option<&'static str> {
    let action = action.as_any();
    if action.is::<NewTab>() || action.is::<NewWindow>() {
        Some("icon/ui/plus.svg")
    } else if action.is::<ReopenClosedTab>() || action.is::<CheckForUpdates>() {
        Some("icon/ui/refresh.svg")
    } else if action.is::<CloseTab>() || action.is::<CloseWindow>() {
        Some("icon/ui/close.svg")
    } else if action.is::<OpenFile>()
        || action.is::<OpenSafeSource>()
        || action.is::<OpenRecentFile>()
        || action.is::<NoRecentFiles>()
    {
        Some("icon/ui/files.svg")
    } else if action.is::<OpenFolder>() || action.is::<OpenCrashReports>() {
        Some("icon/workspace/folder.svg")
    } else if action.is::<OpenPreferences>() {
        Some("icon/ui/sliders.svg")
    } else if action.is::<SaveDocument>() || action.is::<SaveDocumentAs>() {
        Some("icon/ui/save.svg")
    } else if action.is::<QuitApplication>() {
        Some("icon/ui/power.svg")
    } else if action.is::<Undo>() {
        Some("icon/ui/undo.svg")
    } else if action.is::<Redo>() {
        Some("icon/ui/redo.svg")
    } else if action.is::<Cut>() {
        Some("icon/ui/scissors.svg")
    } else if action.is::<Copy>() || action.is::<CopyAsMarkdown>() || action.is::<DuplicateLine>() {
        Some("icon/ui/copy.svg")
    } else if action.is::<DeleteLine>() {
        Some("icon/ui/trash.svg")
    } else if action.is::<MoveLineUp>() {
        Some("icon/ui/arrow-up.svg")
    } else if action.is::<MoveLineDown>() {
        Some("icon/ui/arrow-down.svg")
    } else if action.is::<Paste>() || action.is::<PasteAsPlainText>() {
        Some("icon/ui/clipboard.svg")
    } else if action.is::<SelectAll>() {
        Some("icon/ui/check.svg")
    } else if action.is::<FindInDocument>()
        || action.is::<ReplaceInDocument>()
        || action.is::<FindNext>()
        || action.is::<FindPrevious>()
    {
        Some("icon/ui/search.svg")
    } else if action.is::<QuickOpen>() {
        Some("icon/ui/files.svg")
    } else if action.is::<CommandPalette>() {
        Some("icon/ui/search.svg")
    } else if action.is::<PreviousTab>() {
        Some("icon/ui/arrow-left.svg")
    } else if action.is::<NextTab>() {
        Some("icon/ui/arrow-right.svg")
    } else if action.is::<ToggleViewMode>() {
        Some("icon/ui/live.svg")
    } else if action.is::<NormalizeLineEndingsLf>()
        || action.is::<NormalizeLineEndingsCrLf>()
        || action.is::<NormalizeLineEndingsCr>()
    {
        Some("icon/ui/source.svg")
    } else if action.is::<ExportHtml>() || action.is::<ExportImage>() || action.is::<ExportPdf>() {
        Some("icon/ui/file-output.svg")
    } else if action.is::<SelectLanguage>()
        || action.is::<AddLanguageConfig>()
        || action.is::<InstallCliTool>()
        || action.is::<UninstallCliTool>()
    {
        Some("icon/ui/keyboard.svg")
    } else if action.is::<ToggleWorkspace>() {
        Some("icon/ui/panel-left.svg")
    } else if action.is::<ToggleFocusMode>() || action.is::<ToggleTypewriterMode>() {
        Some("icon/ui/type.svg")
    } else if action.is::<OpenPrivacyPolicy>() {
        Some("icon/ui/shield.svg")
    } else if action.is::<ShowAbout>() {
        Some("icon/ui/info.svg")
    } else {
        None
    }
}

/// Global app-menu state for platform menu lifecycle hooks.
#[derive(Default)]
pub(crate) struct AppMenuState {
    window_closed_subscription: Option<Subscription>,
    /// Windows' native menu bridge may expose only the launcher menu back to GPUI.
    /// Keep the authoritative owned snapshot for the custom in-window renderer.
    pub(crate) in_window_menus: Vec<OwnedMenu>,
    /// Menu rebuilding consumes this snapshot so the UI thread never rereads history storage.
    pub(crate) recent_files: Vec<PathBuf>,
}

impl Global for AppMenuState {}

use crate::components::{
    CancelFormatting, CollapseAllFolds, CollapseFold, ExpandAllFolds, ExpandFold, FormatDocument,
    FormatSelection,
};

mod cli_support;
mod cli_tool;
mod command_support;
mod dispatch;
mod file_prompts;
mod initialization;
mod menu_build;
mod quit;
mod recent;
mod windows;

#[cfg(any(target_os = "macos", test))]
use cli_support::applescript_string_literal;
#[cfg(target_os = "macos")]
use cli_support::is_cli_symlink_current_app;
use command_support::{
    open_crash_reports, open_recent_file, open_recent_file_with_error_window,
    request_update_check_on_active_editor, show_info_dialog_on_active_editor, show_window_prompt,
    with_active_editor,
};
use file_prompts::{
    prompt_and_import_language_config, prompt_and_import_language_config_with_error_window,
    prompt_and_open_files, prompt_and_open_files_with_error_window, prompt_and_open_safe_source,
    prompt_and_open_safe_source_with_error_window,
};
pub(crate) use recent::{
    load_recent_files_in_background, record_recent_file_and_refresh, remove_recent_file_and_refresh,
};

pub(crate) use cli_tool::{install_cli_tool, uninstall_cli_tool};
pub(crate) use command_support::record_recent_file_from_editor;
pub(crate) use dispatch::{
    abort_pending_quit, continue_pending_quit, dispatch_menu_action,
    dispatch_menu_action_for_editor, request_quit_application, request_update_quit_application,
};
pub(crate) use initialization::init;
pub(crate) use menu_build::{install_menus, install_menus_with_recent_files};
// 原因：测试与过渡调用方仍从 app_menu 根访问退出类型；当全部迁移到 quit 子模块后移除。
#[allow(unused_imports)]
pub(crate) use quit::{QuitCoordinator, QuitIntent, QuitPhase, QuitRequestOutcome};
pub(crate) use windows::*;

#[cfg(test)]
use dispatch::is_window_context_menu_action;
#[cfg(test)]
use menu_build::build_menus;
#[cfg(test)]
#[path = "../../../tests/unit/app_menu.rs"]
mod tests;
