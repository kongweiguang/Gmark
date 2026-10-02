// @author kongweiguang

#[cfg(target_os = "windows")]
use std::time::Duration;

use gpui::{AppContext, TestAppContext};
#[cfg(target_os = "windows")]
use gpui::{CompositionEnd, EntityInputHandler, Window};

use super::super::selection_surface::SelectionSurface;
use super::{Editor, FindMatchMetadata, FindRestoreFocus, Replaceability, ViewMode};

/// Initializes the editor services used by the Find panel and its block inputs.
fn init_find_surface_app(cx: &mut TestAppContext) {
    cx.update(|cx| {
        crate::i18n::I18nManager::init(cx);
        crate::theme::ThemeManager::init(cx);
        crate::components::init(cx);
    });
}

/// Delivers the final IME candidate and terminal event to the replacement field.
#[cfg(target_os = "windows")]
fn commit_candidate(
    input: &mut crate::components::Block,
    text: &str,
    window: &mut Window,
    cx: &mut gpui::Context<crate::components::Block>,
) {
    input.replace_text_in_range(None, text, window, cx);
    input.composition_ended(CompositionEnd::Committed, window, cx);
}

/// Waits for replacement-field pre-edit to commit before applying one Find Replace transaction.
#[cfg(target_os = "windows")]
#[gpui::test]
async fn find_replace_all_uses_committed_ime_candidate(cx: &mut TestAppContext) {
    init_find_surface_app(cx);
    let (editor, visual) =
        cx.add_window_view(|_window, cx| Editor::from_markdown(cx, "alpha".to_owned(), None));

    editor.update_in(visual, |editor, _window, cx| {
        editor.open_find_panel(true, _window, cx);
        let query = editor
            .find_panel
            .as_ref()
            .expect("Find panel")
            .query
            .clone();
        query.update(cx, |input, cx| {
            input.replace_text_in_visible_range(0..0, "alpha", None, false, cx);
        });
    });
    visual.executor().advance_clock(Duration::from_millis(40));
    visual.run_until_parked();

    let replacement = editor.update_in(visual, |editor, window, cx| {
        let state = editor.find_panel.as_ref().expect("Find panel");
        assert_eq!(state.matches.len(), 1);
        let replacement = state.replacement.clone();
        replacement.update(cx, |input, cx| {
            input.focus_handle.focus(window);
            input.selected_range = 0..0;
            input.replace_and_mark_text_in_range(None, "x", Some(0..0), window, cx);
        });
        assert!(replacement.read(cx).has_ime_composition());

        let source_before = editor.source_document.text().to_owned();
        let revision_before = editor.source_document.revision();
        let undo_before = editor.undo_history.len();
        editor.replace_all_find_matches(window, cx);
        assert_eq!(editor.pending_ime_operations.len(), 1);
        assert_eq!(editor.source_document.text(), source_before);
        assert_eq!(editor.source_document.revision(), revision_before);
        assert_eq!(editor.undo_history.len(), undo_before);
        replacement
    });

    editor.update_in(visual, |_editor, window, cx| {
        replacement.update(cx, |input, cx| commit_candidate(input, "omega", window, cx));
    });
    visual.run_until_parked();
    editor.update_in(visual, |editor, window, cx| {
        editor.sync_pending_ime_operations(window, cx);
    });
    visual.run_until_parked();
    editor.update_in(visual, |editor, _window, cx| {
        assert_eq!(editor.source_document.text(), "omega");
        assert_eq!(editor.undo_history.len(), 1);
        editor.undo_document(cx);
        assert_eq!(editor.source_document.text(), "alpha");
    });
}

/// Restores Split Preview focus and rejects replacement commands on that read-only surface.
/// Keeps the split preview active while replacement operates on one direct source match.
#[gpui::test]
async fn find_close_restores_split_preview_and_replace_stays_read_only(cx: &mut TestAppContext) {
    init_find_surface_app(cx);
    let (editor, visual) =
        cx.add_window_view(|_window, cx| Editor::from_markdown(cx, "alpha".to_owned(), None));
    editor.update_in(visual, |editor, _window, cx| {
        editor.set_view_mode(ViewMode::Split, cx);
    });
    visual.run_until_parked();

    editor.update_in(visual, |editor, window, cx| {
        let preview = editor
            .split_preview
            .as_ref()
            .expect("Split Preview")
            .document
            .visible_blocks()[0]
            .entity
            .clone();
        editor.active_selection_surface = SelectionSurface::SplitPreview;
        preview.read(cx).focus_handle.focus(window);
        editor.open_find_panel(false, window, cx);
        assert_eq!(
            editor
                .find_panel
                .as_ref()
                .and_then(|state| state.restore_focus),
            Some(FindRestoreFocus {
                surface: SelectionSurface::SplitPreview,
                entity_id: preview.entity_id(),
            })
        );
        editor.close_find_panel(window, cx);
        assert_eq!(
            editor.active_selection_surface,
            SelectionSurface::SplitPreview
        );
        assert!(preview.read(cx).focus_handle.is_focused(window));

        editor.open_find_panel(true, window, cx);
        let revision = editor.source_document.revision();
        let source = editor.source_document.text().to_owned();
        let undo_len = editor.undo_history.len();
        let state = editor.find_panel.as_mut().expect("Replace panel");
        state.matches = std::iter::once(0..5).collect();
        state.match_metadata = vec![FindMatchMetadata {
            visible: 0..5,
            source: Some(0..5),
            replaceability: Replaceability::Direct,
        }];
        state.revision = revision;
        state.selected = 0;

        editor.replace_current_find_match(window, cx);
        editor.replace_all_find_matches(window, cx);
        assert_eq!(editor.source_document.text(), source);
        assert_eq!(editor.source_document.revision(), revision);
        assert_eq!(editor.undo_history.len(), undo_len);
        assert!(
            editor
                .find_panel
                .as_ref()
                .is_some_and(|state| state.replace_task.is_none())
        );
    });
}
