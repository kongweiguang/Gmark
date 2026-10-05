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

/// Guards Escape after the Find field's select/copy keys without involving IME state.
#[gpui::test]
async fn find_panel_closes_on_escape_after_keyboard_open(cx: &mut TestAppContext) {
    init_find_surface_app(cx);
    let (editor, visual) =
        cx.add_window_view(|_window, cx| Editor::from_markdown(cx, "alpha".to_owned(), None));

    visual.simulate_keystrokes("ctrl-f");
    visual.run_until_parked();
    assert!(editor.read_with(visual, |editor, _cx| editor.find_panel.is_some()));

    visual.simulate_keystrokes("ctrl-a");
    visual.simulate_keystrokes("ctrl-c");
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();
    assert!(
        editor.read_with(visual, |editor, _cx| editor.find_panel.is_none()),
        "Escape should close a focused Find panel when no IME candidate is active"
    );
}

/// Leaves Escape with a focused Find input's IME when a candidate composition is active.
#[cfg(target_os = "windows")]
#[gpui::test]
async fn find_panel_escape_preserves_active_ime_candidate(cx: &mut TestAppContext) {
    init_find_surface_app(cx);
    let (editor, visual) =
        cx.add_window_view(|_window, cx| Editor::from_markdown(cx, "alpha".to_owned(), None));
    let query = editor.update_in(visual, |editor, window, cx| {
        editor.open_find_panel(false, window, cx);
        let query = editor
            .find_panel
            .as_ref()
            .expect("Find panel")
            .query
            .clone();
        query.update(cx, |input, cx| {
            input.focus_handle.focus(window);
            input.replace_and_mark_text_in_range(None, "n", Some(0..0), window, cx);
        });
        assert!(query.read(cx).has_ime_composition());
        query
    });

    visual.simulate_keystrokes("escape");
    visual.run_until_parked();
    editor.read_with(visual, |editor, cx| {
        assert!(editor.find_panel.is_some());
        assert!(query.read(cx).has_ime_composition());
    });
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

/// Preview's visible selection must beat its older source snapshot; Escape must restore the read-only target.
#[gpui::test]
async fn find_prefills_preview_visible_selection_and_escape_restores_it(cx: &mut TestAppContext) {
    init_find_surface_app(cx);
    let (editor, visual) = cx
        .add_window_view(|_window, cx| Editor::from_markdown(cx, "alpha\\_beta".to_owned(), None));

    let preview = editor.update_in(visual, |editor, window, cx| {
        editor.set_view_mode(ViewMode::Source, cx);
        let source = editor.document.first_root().expect("source block").clone();
        source.update(cx, |block, _cx| {
            block.selected_range = 0.."alpha\\_beta".len();
            block.focus_handle.focus(window);
        });
        editor.set_view_mode(ViewMode::Preview, cx);

        let preview = editor
            .document
            .visible_blocks()
            .first()
            .expect("preview block")
            .entity
            .clone();
        let visible_len = preview.read(cx).visible_len();
        preview.update(cx, |block, _cx| {
            block.selected_range = 0..visible_len;
            block.focus_handle.focus(window);
        });
        editor.active_selection_surface = SelectionSurface::Main;
        editor.open_find_panel(false, window, cx);
        assert_eq!(
            editor
                .find_panel
                .as_ref()
                .expect("Find panel")
                .query
                .read(cx)
                .display_text(),
            "alpha_beta"
        );
        preview
    });

    visual.run_until_parked();
    visual.update(|window, cx| window.draw(cx).clear());
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();
    editor.update_in(visual, |editor, window, cx| {
        assert!(editor.find_panel.is_none());
        assert_eq!(editor.active_selection_surface, SelectionSurface::Main);
        let preview = preview.read(cx);
        assert!(preview.focus_handle.is_focused(window));
        assert_eq!(preview.selected_range, 0.."alpha_beta".len());
    });
}

/// The focused Split right pane owns Find's query while Main keeps its older multiline source selection.
#[gpui::test]
async fn find_prefills_split_right_selection_instead_of_stale_main_selection(
    cx: &mut TestAppContext,
) {
    init_find_surface_app(cx);
    let (editor, visual) = cx.add_window_view(|_window, cx| {
        Editor::from_markdown(cx, "old left\n\nold right\n\nalpha\\_beta".to_owned(), None)
    });
    editor.update_in(visual, |editor, _window, cx| {
        editor.set_view_mode(ViewMode::Split, cx);
    });
    visual.run_until_parked();

    let (preview, visible_len, stale_main_range) =
        editor.update_in(visual, |editor, window, cx| {
            let main = editor
                .document
                .first_root()
                .expect("Main Source block")
                .clone();
            let stale_main_range = 0.."old left\n\nold right".len();
            main.update(cx, |block, _cx| {
                block.selected_range = stale_main_range.clone();
            });

            let preview = editor
                .split_preview
                .as_ref()
                .expect("Split right pane")
                .document
                .visible_blocks()
                .last()
                .expect("right preview block")
                .entity
                .clone();
            let visible_len = preview.read(cx).visible_len();
            preview.update(cx, |block, _cx| {
                block.selected_range = 0..visible_len;
                block.focus_handle.focus(window);
            });
            editor.active_selection_surface = SelectionSurface::SplitPreview;
            editor.open_find_panel(false, window, cx);
            assert_eq!(
                editor
                    .find_panel
                    .as_ref()
                    .expect("Find panel")
                    .query
                    .read(cx)
                    .display_text(),
                "alpha_beta"
            );
            assert_eq!(main.read(cx).selected_range, stale_main_range);
            (preview, visible_len, stale_main_range)
        });

    visual.run_until_parked();
    visual.update(|window, cx| window.draw(cx).clear());
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();
    editor.update_in(visual, |editor, window, cx| {
        assert!(editor.find_panel.is_none());
        assert_eq!(
            editor.active_selection_surface,
            SelectionSurface::SplitPreview
        );
        let preview = preview.read(cx);
        assert!(preview.focus_handle.is_focused(window));
        assert_eq!(preview.selected_range, 0..visible_len);
        assert_eq!(
            editor
                .document
                .first_root()
                .expect("Main Source block")
                .read(cx)
                .selected_range,
            stale_main_range
        );
    });
}

/// Source-mode Find must preserve Markdown escaping instead of searching the rendered spelling.
#[gpui::test]
async fn find_prefills_source_selection_with_raw_markdown(cx: &mut TestAppContext) {
    init_find_surface_app(cx);
    let (editor, visual) = cx
        .add_window_view(|_window, cx| Editor::from_markdown(cx, "alpha\\_beta".to_owned(), None));

    let source = editor.update_in(visual, |editor, window, cx| {
        editor.set_view_mode(ViewMode::Source, cx);
        let source = editor.document.first_root().expect("source block").clone();
        source.update(cx, |block, _cx| {
            block.selected_range = 0.."alpha\\_beta".len();
            block.focus_handle.focus(window);
        });
        editor.open_find_panel(false, window, cx);
        assert_eq!(
            editor
                .find_panel
                .as_ref()
                .expect("Find panel")
                .query
                .read(cx)
                .display_text(),
            "alpha\\_beta"
        );
        source
    });

    visual.run_until_parked();
    visual.update(|window, cx| window.draw(cx).clear());
    editor.update_in(visual, |editor, window, cx| {
        let query = &editor.find_panel.as_ref().expect("Find panel").query;
        assert!(
            query.read(cx).focus_handle.is_focused(window),
            "Find query should keep focus after the source-mode transition is drawn"
        );
    });
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();
    editor.update_in(visual, |editor, window, cx| {
        assert!(editor.find_panel.is_none());
        assert!(source.read(cx).focus_handle.is_focused(window));
        assert_eq!(source.read(cx).selected_range, 0.."alpha\\_beta".len());
    });
}

/// Live Find uses the selected inline text, not Markdown delimiters from the editor's display range.
#[gpui::test]
async fn find_prefills_live_selection_without_markdown_delimiters(cx: &mut TestAppContext) {
    init_find_surface_app(cx);
    let (editor, visual) = cx.add_window_view(|_window, cx| {
        Editor::from_markdown(cx, "**alpha_beta**".to_owned(), None)
    });

    editor.update_in(visual, |editor, window, cx| {
        let live = editor.document.visible_blocks()[0].entity.clone();
        live.update(cx, |block, _cx| {
            block.selected_range = 0..block.display_text().len();
            block.focus_handle.focus(window);
        });
        editor.active_selection_surface = SelectionSurface::Main;
        editor.open_find_panel(false, window, cx);
        assert_eq!(
            editor
                .find_panel
                .as_ref()
                .expect("Find panel")
                .query
                .read(cx)
                .display_text(),
            "alpha_beta"
        );
    });
}
