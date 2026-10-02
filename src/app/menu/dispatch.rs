// @author kongweiguang

//! Application and editor menu action dispatch.

use super::*;

fn is_editor_scoped_menu_action(action: &dyn Action) -> bool {
    is_editor_dispatch_action(action)
        || action.as_any().is::<SaveDocument>()
        || action.as_any().is::<SaveDocumentAs>()
        || action.as_any().is::<ExportHtml>()
        || action.as_any().is::<ExportImage>()
        || action.as_any().is::<ExportPdf>()
        || action.as_any().is::<ExportSelection>()
        || action.as_any().is::<ShowDocumentInfo>()
        || action.as_any().is::<ShowDocumentOutline>()
        || action.as_any().is::<ShowStructureView>()
        || action.as_any().is::<ShowStructuredInspector>()
        || action.as_any().is::<FocusStructuredFilter>()
        || action.as_any().is::<FocusStructuredColumns>()
        || action.as_any().is::<NormalizeLineEndingsLf>()
        || action.as_any().is::<NormalizeLineEndingsCrLf>()
        || action.as_any().is::<NormalizeLineEndingsCr>()
        || action.as_any().is::<QuitApplication>()
        || action.as_any().is::<CloseWindow>()
        || action.as_any().is::<NewTab>()
        || action.as_any().is::<CloseTab>()
        || action.as_any().is::<ReopenClosedTab>()
        || action.as_any().is::<CheckForUpdates>()
        || action.as_any().is::<ShowAbout>()
        || action.as_any().is::<InstallCliTool>()
        || action.as_any().is::<UninstallCliTool>()
        || action.as_any().is::<ToggleWorkspace>()
        || action.as_any().is::<ToggleDocumentSidebar>()
        || action.as_any().is::<OpenFolder>()
        || action.as_any().is::<InsertResource>()
        || action.as_any().is::<ToggleFocusMode>()
        || action.as_any().is::<ToggleTypewriterMode>()
}

/// 这些 action 必须沿窗口焦点路径分发，才能由当前 Block 或 Editor 处理。
/// 全局菜单与 Windows/Linux 自绘菜单共用同一判定，避免两条入口能力漂移。
fn is_editor_dispatch_action(action: &dyn Action) -> bool {
    action.as_any().is::<Undo>()
        || action.as_any().is::<Redo>()
        || action.as_any().is::<Cut>()
        || action.as_any().is::<Copy>()
        || action.as_any().is::<CopyAsMarkdown>()
        || action.as_any().is::<Paste>()
        || action.as_any().is::<PasteAsPlainText>()
        || action.as_any().is::<SelectAll>()
        || action.as_any().is::<DuplicateLine>()
        || action.as_any().is::<DeleteLine>()
        || action.as_any().is::<MoveLineUp>()
        || action.as_any().is::<MoveLineDown>()
        || action.as_any().is::<FindInDocument>()
        || action.as_any().is::<ReplaceInDocument>()
        || action.as_any().is::<FindNext>()
        || action.as_any().is::<FindPrevious>()
        || action.as_any().is::<BoldSelection>()
        || action.as_any().is::<ItalicSelection>()
        || action.as_any().is::<StrikethroughSelection>()
        || action.as_any().is::<UnderlineSelection>()
        || action.as_any().is::<HighlightSelection>()
        || action.as_any().is::<SuperscriptSelection>()
        || action.as_any().is::<SubscriptSelection>()
        || action.as_any().is::<InlineMathSelection>()
        || action.as_any().is::<CodeSelection>()
        || action.as_any().is::<LinkSelection>()
}

pub(super) fn is_window_context_menu_action(action: &dyn Action) -> bool {
    action.as_any().is::<NewWindow>()
        || action.as_any().is::<OpenFile>()
        || action.as_any().is::<OpenSafeSource>()
        || action.as_any().is::<OpenPreferences>()
        || action.as_any().is::<OpenRecentFile>()
        || action.as_any().is::<NoRecentFiles>()
        || action.as_any().is::<AddLanguageConfig>()
        || action.as_any().is::<InstallCliTool>()
        || action.as_any().is::<UninstallCliTool>()
        || action.as_any().is::<OpenCrashReports>()
        || action.as_any().is::<OpenPrivacyPolicy>()
        || is_editor_scoped_menu_action(action)
}

pub(super) fn current_window_candidates(cx: &mut App) -> Vec<AnyWindowHandle> {
    let mut candidates = Vec::new();
    let mut push_unique = |window: AnyWindowHandle| {
        if candidates
            .iter()
            .all(|candidate: &AnyWindowHandle| candidate.window_id() != window.window_id())
        {
            candidates.push(window);
        }
    };

    if let Some(window) = cx.active_window() {
        push_unique(window);
    }
    if let Some(windows) = cx.window_stack() {
        for window in windows {
            push_unique(window);
        }
    }
    for window in cx.windows() {
        push_unique(window);
    }

    candidates
}

fn request_close_editor_window(window: AnyWindowHandle, cx: &mut App) -> bool {
    let Some(window) = window.downcast::<Editor>() else {
        return false;
    };

    window
        .update(cx, |editor, window, cx| {
            editor.request_close_current_window(window, cx);
        })
        .is_ok()
}

fn request_close_current_editor_window(cx: &mut App) {
    let candidates = current_window_candidates(cx);
    if candidates.is_empty() {
        cx.quit();
        return;
    }

    for window in candidates {
        if request_close_editor_window(window, cx) {
            return;
        }
    }
}

pub(crate) fn request_quit_application(cx: &mut App) -> QuitRequestOutcome {
    request_quit_with_intent(QuitIntent::UserQuit, cx)
}

/// Starts the update restart quit flow.  Installation is deliberately not
/// prepared here: this function only records the intent and defers the window
/// veto checks until the current editor callback has returned to GPUI.
pub(crate) fn request_update_quit_application(cx: &mut App) -> QuitRequestOutcome {
    request_quit_with_intent(QuitIntent::ApplyUpdate, cx)
}

fn request_quit_with_intent(intent: QuitIntent, cx: &mut App) -> QuitRequestOutcome {
    let outcome = QuitCoordinator::begin(cx, intent);
    if outcome == QuitRequestOutcome::Scheduled {
        cx.defer(evaluate_quit_application);
    }
    outcome
}

/// Re-runs a previously vetoed request after an unsaved dialog saves/discards
/// its document or after a window is removed.  The second scheduling boundary
/// is intentional: the close button itself is an editor callback and may have
/// just removed that callback's window.
pub(crate) fn continue_pending_quit(cx: &mut App) {
    if QuitCoordinator::schedule_continuation(cx) {
        cx.defer(evaluate_quit_application);
    }
}

/// Cancels the current intent when the user keeps editing, a save fails, or an
/// external-file conflict remains unresolved.  Calling this for a normal close
/// is harmless because the coordinator is idle.
pub(crate) fn abort_pending_quit(cx: &mut App) {
    let _ = QuitCoordinator::abort(cx);
}

fn evaluate_quit_application(cx: &mut App) {
    let Some(intent) = QuitCoordinator::begin_evaluation(cx) else {
        return;
    };

    let mut vetoed = false;
    for candidate in current_window_candidates(cx) {
        let Some(window) = candidate.downcast::<Editor>() else {
            continue;
        };

        // A window may disappear between collecting handles and evaluating it
        // (for example, a discard-and-close action).  A stale handle is not a
        // veto; treating it as one was the source of the old indefinite wait.
        match window.update(cx, |editor, window, cx| {
            editor.on_window_should_close_for_quit(window, cx)
        }) {
            Ok(true) => {}
            Ok(false) => {
                vetoed = true;
                break;
            }
            Err(_) => {}
        }
    }

    if vetoed {
        QuitCoordinator::veto(cx);
        return;
    }

    match intent {
        QuitIntent::UserQuit => {
            QuitCoordinator::complete(cx);
            cx.quit();
        }
        QuitIntent::ApplyUpdate => {
            QuitCoordinator::mark_handoff(cx);
            if crate::updater::UpdateCoordinator::handoff_install_after_quit_approval(cx) {
                QuitCoordinator::complete(cx);
                cx.quit();
            } else {
                // No helper/plan is left behind when the handoff cannot be
                // prepared; the verified artifact remains ready for retry.
                QuitCoordinator::abort(cx);
            }
        }
    }
}

fn menu_block_kind(action: &dyn Action) -> Option<BlockKind> {
    if action.as_any().is::<SetHeading1>() {
        Some(BlockKind::Heading { level: 1 })
    } else if action.as_any().is::<SetHeading2>() {
        Some(BlockKind::Heading { level: 2 })
    } else if action.as_any().is::<SetHeading3>() {
        Some(BlockKind::Heading { level: 3 })
    } else if action.as_any().is::<SetHeading4>() {
        Some(BlockKind::Heading { level: 4 })
    } else if action.as_any().is::<SetHeading5>() {
        Some(BlockKind::Heading { level: 5 })
    } else if action.as_any().is::<SetHeading6>() {
        Some(BlockKind::Heading { level: 6 })
    } else if action.as_any().is::<SetParagraph>() {
        Some(BlockKind::Paragraph)
    } else if action.as_any().is::<SetBulletedList>() {
        Some(BlockKind::BulletedListItem)
    } else if action.as_any().is::<SetNumberedList>() {
        Some(BlockKind::NumberedListItem)
    } else if action.as_any().is::<SetTaskList>() {
        Some(BlockKind::TaskListItem { checked: false })
    } else if action.as_any().is::<SetQuote>() {
        Some(BlockKind::Quote)
    } else if action.as_any().is::<SetCodeBlock>() {
        Some(BlockKind::CodeBlock { language: None })
    } else {
        None
    }
}

/// Executes one of the app-menu actions against the current application state.
pub(crate) fn dispatch_menu_action(action: &dyn Action, cx: &mut App) {
    if action.as_any().is::<NewWindow>() {
        if let Err(error) = open_editor_window(cx, String::new(), None) {
            eprintln!("failed to open editor window: {error}");
        }
    } else if action.as_any().is::<NewTab>() {
        let _ = with_active_editor(cx, |editor, _window, cx| {
            editor.new_untitled_tab(cx);
        });
    } else if action.as_any().is::<OpenFile>() {
        prompt_and_open_files(cx);
    } else if action.as_any().is::<OpenSafeSource>() {
        prompt_and_open_safe_source(cx);
    } else if action.as_any().is::<OpenFolder>() {
        let _ = with_active_editor(cx, |editor, window, cx| {
            editor.on_open_folder_action(&OpenFolder, window, cx);
        });
    } else if action.as_any().is::<OpenPreferences>() {
        if let Err(error) = open_preferences_window(cx) {
            eprintln!("failed to open preferences: {error}");
        }
    } else if let Some(action) = action.as_any().downcast_ref::<OpenRecentFile>() {
        open_recent_file(cx, PathBuf::from(&action.path));
    } else if action.as_any().is::<NoRecentFiles>() {
    } else if action.as_any().is::<AddLanguageConfig>() {
        prompt_and_import_language_config(cx);
    } else if action.as_any().is::<InsertResource>() {
        let _ = with_active_editor(cx, |editor, window, cx| {
            editor.on_insert_resource_action(&InsertResource, window, cx);
        });
    } else if is_editor_dispatch_action(action) {
        let action = action.boxed_clone();
        let _ = with_active_editor(cx, |_editor, window, cx| {
            window.dispatch_action(action, cx);
        });
    } else if let Some(kind) = menu_block_kind(action) {
        let _ = with_active_editor(cx, |editor, _window, cx| {
            editor.set_active_block_kind(kind, cx)
        });
    } else if action.as_any().is::<SaveDocument>() {
        let _ = with_active_editor(cx, |editor, window, cx| editor.save_document(window, cx));
    } else if action.as_any().is::<SaveDocumentAs>() {
        let _ = with_active_editor(cx, |editor, window, cx| editor.save_document_as(window, cx));
    } else if action.as_any().is::<ExportHtml>() {
        let _ = with_active_editor(cx, |editor, window, cx| {
            editor.export_document_via_prompt(ExportFormat::Html, window, cx)
        });
    } else if action.as_any().is::<ExportImage>() {
        let _ = with_active_editor(cx, |editor, window, cx| {
            editor.export_document_via_prompt(ExportFormat::Png, window, cx)
        });
    } else if action.as_any().is::<ExportPdf>() {
        let _ = with_active_editor(cx, |editor, window, cx| {
            editor.export_document_via_prompt(ExportFormat::Pdf, window, cx)
        });
    } else if action.as_any().is::<ExportSelection>() {
        let _ = with_active_editor(cx, |editor, window, cx| {
            editor.on_export_selection_action(&ExportSelection, window, cx)
        });
    } else if action.as_any().is::<ShowDocumentInfo>() {
        let _ = with_active_editor(cx, |editor, window, cx| {
            editor.on_show_document_info(&ShowDocumentInfo, window, cx)
        });
    } else if action.as_any().is::<ShowDocumentOutline>() {
        let _ = with_active_editor(cx, |editor, window, cx| {
            editor.on_show_document_outline(&ShowDocumentOutline, window, cx)
        });
    } else if action.as_any().is::<ShowStructureView>() {
        let _ = with_active_editor(cx, |editor, window, cx| {
            editor.on_show_structure_view(&ShowStructureView, window, cx)
        });
    } else if action.as_any().is::<ShowStructuredInspector>() {
        let _ = with_active_editor(cx, |editor, window, cx| {
            editor.on_show_structured_inspector(&ShowStructuredInspector, window, cx)
        });
    } else if action.as_any().is::<FocusStructuredFilter>() {
        let _ = with_active_editor(cx, |editor, window, cx| {
            editor.on_focus_structured_filter(&FocusStructuredFilter, window, cx)
        });
    } else if action.as_any().is::<FocusStructuredColumns>() {
        let _ = with_active_editor(cx, |editor, window, cx| {
            editor.on_focus_structured_columns(&FocusStructuredColumns, window, cx)
        });
    } else if action.as_any().is::<NormalizeLineEndingsLf>() {
        let _ = with_active_editor(cx, |editor, _window, cx| {
            editor.normalize_line_endings(gmark_document::LineEnding::Lf, cx);
        });
    } else if action.as_any().is::<NormalizeLineEndingsCrLf>() {
        let _ = with_active_editor(cx, |editor, _window, cx| {
            editor.normalize_line_endings(gmark_document::LineEnding::CrLf, cx);
        });
    } else if action.as_any().is::<NormalizeLineEndingsCr>() {
        let _ = with_active_editor(cx, |editor, _window, cx| {
            editor.normalize_line_endings(gmark_document::LineEnding::Cr, cx);
        });
    } else if let Some(action) = action.as_any().downcast_ref::<SelectLanguage>() {
        match apply_configured_language(cx, &action.language_id) {
            Ok(changed) => {
                if changed {
                    install_menus(cx);
                    cx.refresh_windows();
                }
            }
            Err(err) => {
                let title = cx
                    .global::<I18nManager>()
                    .strings()
                    .preferences_save_failed_title
                    .clone();
                show_window_prompt(cx.active_window(), &title, &err.to_string(), cx);
            }
        }
    } else if action.as_any().is::<CheckForUpdates>() {
        request_update_check_on_active_editor(cx);
    } else if action.as_any().is::<OpenCrashReports>() {
        open_crash_reports(cx);
    } else if action.as_any().is::<OpenPrivacyPolicy>() {
        cx.open_url(PRIVACY_POLICY_URL);
    } else if action.as_any().is::<ShowAbout>() {
        show_info_dialog_on_active_editor(cx, InfoDialogKind::About);
    } else if action.as_any().is::<InstallCliTool>() {
        install_cli_tool(cx);
    } else if action.as_any().is::<UninstallCliTool>() {
        uninstall_cli_tool(cx);
    } else if action.as_any().is::<ToggleWorkspace>() {
        let _ = with_active_editor(cx, |editor, window, cx| {
            editor.toggle_workspace_drawer(window, cx);
        });
    } else if action.as_any().is::<ToggleDocumentSidebar>() {
        let _ = with_active_editor(cx, |editor, window, cx| {
            editor.toggle_document_sidebar_drawer(window, cx);
        });
    } else if action.as_any().is::<ToggleFocusMode>() {
        let _ = with_active_editor(cx, |editor, window, cx| {
            editor.on_toggle_focus_mode_action(&ToggleFocusMode, window, cx);
        });
    } else if action.as_any().is::<ToggleTypewriterMode>() {
        let _ = with_active_editor(cx, |editor, window, cx| {
            editor.on_toggle_typewriter_mode_action(&ToggleTypewriterMode, window, cx);
        });
    } else if action.as_any().is::<CloseTab>() {
        let _ = with_active_editor(cx, |editor, window, cx| {
            editor.on_close_tab_action(&CloseTab, window, cx);
        });
    } else if action.as_any().is::<ReopenClosedTab>() {
        let _ = with_active_editor(cx, |editor, window, cx| {
            editor.on_reopen_closed_tab_action(&ReopenClosedTab, window, cx);
        });
    } else if action.as_any().is::<QuitApplication>() {
        request_quit_application(cx);
    } else if action.as_any().is::<CloseWindow>() {
        request_close_current_editor_window(cx);
    }
}

/// Executes a menu action against a specific editor when the action is
/// editor-scoped, falling back to app-wide behavior for global actions.
pub(crate) fn dispatch_menu_action_for_editor(
    action: &dyn Action,
    target: &WeakEntity<Editor>,
    window: &mut Window,
    cx: &mut App,
) {
    if !is_window_context_menu_action(action) {
        let deferred_action = action.boxed_clone();
        cx.defer(move |cx| {
            dispatch_menu_action(deferred_action.as_ref(), cx);
        });
        return;
    }

    window.activate_window();
    let current_window = Some(window.window_handle());

    if action.as_any().is::<NewWindow>() {
        if let Err(error) = open_editor_window(cx, String::new(), None) {
            eprintln!("failed to open editor window: {error}");
        }
    } else if action.as_any().is::<NewTab>() {
        let _ = target.update(cx, |editor, cx| {
            editor.new_untitled_tab(cx);
        });
    } else if action.as_any().is::<OpenFile>() {
        prompt_and_open_files_with_error_window(cx, current_window);
    } else if action.as_any().is::<OpenSafeSource>() {
        prompt_and_open_safe_source_with_error_window(cx, current_window);
    } else if action.as_any().is::<OpenFolder>() {
        let _ = target.update(cx, |editor, cx| {
            editor.on_open_folder_action(&OpenFolder, window, cx);
        });
    } else if action.as_any().is::<OpenPreferences>() {
        if let Err(error) = open_preferences_window(cx) {
            eprintln!("failed to open preferences: {error}");
        }
    } else if let Some(action) = action.as_any().downcast_ref::<OpenRecentFile>() {
        open_recent_file_with_error_window(cx, PathBuf::from(&action.path), current_window);
    } else if action.as_any().is::<NoRecentFiles>() {
    } else if action.as_any().is::<AddLanguageConfig>() {
        prompt_and_import_language_config_with_error_window(cx, current_window);
    } else if action.as_any().is::<InsertResource>() {
        let _ = target.update(cx, |editor, cx| {
            editor.on_insert_resource_action(&InsertResource, window, cx);
        });
    } else if is_editor_dispatch_action(action) {
        window.dispatch_action(action.boxed_clone(), cx);
    } else if action.as_any().is::<SaveDocument>() {
        let _ = target.update(cx, |editor, cx| editor.request_save_document(cx));
    } else if action.as_any().is::<SaveDocumentAs>() {
        let _ = target.update(cx, |editor, cx| editor.request_save_document_as(cx));
    } else if action.as_any().is::<ExportHtml>() {
        let _ = target.update(cx, |editor, cx| {
            editor.export_document_via_prompt(ExportFormat::Html, window, cx);
        });
    } else if action.as_any().is::<ExportImage>() {
        let _ = target.update(cx, |editor, cx| {
            editor.export_document_via_prompt(ExportFormat::Png, window, cx);
        });
    } else if action.as_any().is::<ExportPdf>() {
        let _ = target.update(cx, |editor, cx| {
            editor.export_document_via_prompt(ExportFormat::Pdf, window, cx);
        });
    } else if action.as_any().is::<ExportSelection>() {
        let _ = target.update(cx, |editor, cx| {
            editor.on_export_selection_action(&ExportSelection, window, cx);
        });
    } else if action.as_any().is::<ShowDocumentInfo>() {
        let _ = target.update(cx, |editor, cx| {
            editor.on_show_document_info(&ShowDocumentInfo, window, cx);
        });
    } else if action.as_any().is::<ShowDocumentOutline>() {
        let _ = target.update(cx, |editor, cx| {
            editor.on_show_document_outline(&ShowDocumentOutline, window, cx);
        });
    } else if action.as_any().is::<ShowStructureView>() {
        let _ = target.update(cx, |editor, cx| {
            editor.on_show_structure_view(&ShowStructureView, window, cx);
        });
    } else if action.as_any().is::<ShowStructuredInspector>() {
        let _ = target.update(cx, |editor, cx| {
            editor.on_show_structured_inspector(&ShowStructuredInspector, window, cx);
        });
    } else if action.as_any().is::<FocusStructuredFilter>() {
        let _ = target.update(cx, |editor, cx| {
            editor.on_focus_structured_filter(&FocusStructuredFilter, window, cx);
        });
    } else if action.as_any().is::<FocusStructuredColumns>() {
        let _ = target.update(cx, |editor, cx| {
            editor.on_focus_structured_columns(&FocusStructuredColumns, window, cx);
        });
    } else if action.as_any().is::<NormalizeLineEndingsLf>() {
        let _ = target.update(cx, |editor, cx| {
            editor.normalize_line_endings(gmark_document::LineEnding::Lf, cx);
        });
    } else if action.as_any().is::<NormalizeLineEndingsCrLf>() {
        let _ = target.update(cx, |editor, cx| {
            editor.normalize_line_endings(gmark_document::LineEnding::CrLf, cx);
        });
    } else if action.as_any().is::<NormalizeLineEndingsCr>() {
        let _ = target.update(cx, |editor, cx| {
            editor.normalize_line_endings(gmark_document::LineEnding::Cr, cx);
        });
    } else if action.as_any().is::<QuitApplication>() {
        request_quit_application(cx);
    } else if action.as_any().is::<CloseWindow>() {
        let _ = target.update(cx, |editor, cx| {
            editor.request_close_current_window(window, cx);
        });
    } else if action.as_any().is::<CloseTab>() {
        let _ = target.update(cx, |editor, cx| {
            editor.on_close_tab_action(&CloseTab, window, cx);
        });
    } else if action.as_any().is::<ReopenClosedTab>() {
        let _ = target.update(cx, |editor, cx| {
            editor.on_reopen_closed_tab_action(&ReopenClosedTab, window, cx);
        });
    } else if action.as_any().is::<CheckForUpdates>() {
        let _ = target.update(cx, |editor, cx| {
            editor.request_check_updates(window, cx);
        });
    } else if action.as_any().is::<OpenCrashReports>() {
        open_crash_reports(cx);
    } else if action.as_any().is::<OpenPrivacyPolicy>() {
        cx.open_url(PRIVACY_POLICY_URL);
    } else if action.as_any().is::<ShowAbout>() {
        let _ = target.update(cx, |editor, cx| {
            editor.show_info_dialog(InfoDialogKind::About, cx)
        });
    } else if action.as_any().is::<InstallCliTool>() {
        install_cli_tool(cx);
    } else if action.as_any().is::<UninstallCliTool>() {
        uninstall_cli_tool(cx);
    } else if action.as_any().is::<ToggleWorkspace>() {
        let _ = target.update(cx, |editor, cx| {
            editor.toggle_workspace_drawer(window, cx);
        });
    } else if action.as_any().is::<ToggleDocumentSidebar>() {
        let _ = target.update(cx, |editor, cx| {
            editor.toggle_document_sidebar_drawer(window, cx);
        });
    } else if action.as_any().is::<ToggleFocusMode>() {
        let _ = target.update(cx, |editor, cx| {
            editor.on_toggle_focus_mode_action(&ToggleFocusMode, window, cx);
        });
    } else if action.as_any().is::<ToggleTypewriterMode>() {
        let _ = target.update(cx, |editor, cx| {
            editor.on_toggle_typewriter_mode_action(&ToggleTypewriterMode, window, cx);
        });
    }
}
