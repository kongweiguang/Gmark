// @author kongweiguang

use super::{Editor, ViewMode, ime_lifecycle::DeferredImeOperation, render::DocumentToolbarAction};
use gpui::{Action as _, AppContext, CompositionEnd, EntityInputHandler, TestAppContext};

/// Initializes the real block/editor event path used by the simulated IME regression tests.
fn init(cx: &mut TestAppContext) {
    cx.update(|cx| {
        crate::i18n::I18nManager::init(cx);
        crate::theme::ThemeManager::init(cx);
        crate::components::init(cx);
    });
}

/// Toolbar Find must not take focus until the original block has delivered its composition terminal.
#[gpui::test]
async fn toolbar_find_waits_for_ime_terminal_before_opening_panel(cx: &mut TestAppContext) {
    init(cx);
    let visual = cx.add_empty_window();
    let editor = visual.new(|cx| Editor::from_markdown(cx, "alpha".into(), None));

    let block = visual.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.set_view_mode(ViewMode::Source, cx);
            let block = editor.document.first_root().unwrap().clone();
            editor.refresh_stable_document_snapshot(cx);
            block.update(cx, |block, cx| {
                block.focus_handle.focus(window);
                block.selected_range = 1..1;
                block.replace_and_mark_text_in_range(None, "n", Some(1..1), window, cx);
            });

            assert!(editor.defer_document_toolbar_action_for_ime(
                DocumentToolbarAction::Find,
                window,
                cx,
            ));
            assert!(block.read(cx).focus_handle.is_focused(window));
            assert!(!editor.document_toolbar_focus_handles[2].is_focused(window));
            assert!(matches!(
                editor.pending_ime_operations.front(),
                Some(DeferredImeOperation::ToolFocus { .. })
            ));
            assert!(editor.find_panel.is_none());
            block
        })
    });

    visual.update(|window, cx| {
        block.update(cx, |block, cx| {
            block.replace_text_in_range(None, "你", window, cx);
            block.composition_ended(CompositionEnd::Committed, window, cx);
        });
    });
    visual.run_until_parked();

    visual.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.sync_pending_ime_operations(window, cx);
            let query = editor
                .find_panel
                .as_ref()
                .expect("Find opens after the owner terminates composition")
                .query
                .clone();
            assert!(query.read(cx).focus_handle.is_focused(window));
            assert_eq!(editor.source_document.text(), "a你lpha");
        });
    });
}

/// Global Find actions remain queued while composition owns the current editor field.
#[gpui::test]
async fn find_action_waits_for_ime_terminal_before_creating_input(cx: &mut TestAppContext) {
    init(cx);
    let visual = cx.add_empty_window();
    let editor = visual.new(|cx| Editor::from_markdown(cx, "alpha".into(), None));

    visual.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.set_view_mode(ViewMode::Source, cx);
            let block = editor.document.first_root().unwrap().clone();
            editor.refresh_stable_document_snapshot(cx);
            block.update(cx, |block, cx| {
                block.focus_handle.focus(window);
                block.selected_range = 1..1;
                block.replace_and_mark_text_in_range(None, "n", Some(1..1), window, cx);
            });

            editor.on_find_in_document_action(&crate::components::FindInDocument, window, cx);
            assert!(editor.find_panel.is_none());
            assert!(matches!(
                editor.pending_ime_operations.front(),
                Some(DeferredImeOperation::Action { action, .. })
                    if action.as_any().is::<crate::components::FindInDocument>()
            ));

            block.update(cx, |block, cx| {
                block.composition_ended(CompositionEnd::Cancelled, window, cx);
            });
            editor.sync_pending_ime_operations(window, cx);
            assert!(editor.find_panel.is_some());
            assert_eq!(editor.source_document.text(), "alpha");
        });
    });
}
