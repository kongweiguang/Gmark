// @author kongweiguang

use crate::components::{
    DeleteLine, DuplicateLine, FindInDocument, IndentBlock, MoveLineDown, MoveLineUp,
    OutdentBlock, SelectDown,
};
use crate::editor::{CrossBlockSelection, CrossBlockSelectionEndpoint, ViewMode};
use crate::i18n::I18nManager;
use crate::theme::ThemeManager;
use gpui::VisualTestContext;

/// Installs the UI globals required to render and dispatch actions through an Editor window.
fn init_line_operation_test_app(cx: &mut TestAppContext) {
    cx.update(|cx| {
        I18nManager::init(cx);
        ThemeManager::init(cx);
        crate::components::init(cx);
    });
}

/// Draws the window so focus tracking and action listeners match the real dispatch path.
fn redraw_line_operation_test_window(cx: &mut VisualTestContext) {
    cx.update(|window, cx| window.draw(cx).clear());
    cx.run_until_parked();
}

/// Keeps multi-block Live row commands aligned with source spans and a single undo boundary.
#[gpui::test]
async fn selected_live_rows_duplicate_delete_and_move_as_one_transaction(cx: &mut TestAppContext) {
    let visual = cx.add_empty_window();
    let duplicate =
        visual.new(|cx| Editor::from_markdown(cx, "one\n\ntwo\n\nthree".to_owned(), None));
    visual.update(|window, cx| {
        duplicate.update(cx, |editor, cx| {
            let visible = editor.document.visible_blocks().to_vec();
            editor.cross_block_selection = Some(CrossBlockSelection {
                anchor: CrossBlockSelectionEndpoint {
                    entity_id: visible[0].entity.entity_id(),
                    offset: 1,
                },
                focus: CrossBlockSelectionEndpoint {
                    entity_id: visible[2].entity.entity_id(),
                    offset: 2,
                },
                source_anchor: None,
                source_focus: None,
            });
            visible[2].entity.read(cx).focus_handle.focus(window);
            editor.on_duplicate_line(&DuplicateLine, window, cx);

            assert_eq!(
                editor.source_document.text(),
                "one\n\ntwo\n\nthree\n\none\n\ntwo\n\nthree"
            );
            assert_eq!(editor.undo_history.len(), 1);
            assert_eq!(
                editor
                    .cross_block_selection
                    .map(|selection| selection.focus.entity_id),
                Some(editor.document.visible_blocks()[5].entity.entity_id())
            );
            editor.undo_document(cx);
            assert_eq!(editor.source_document.text(), "one\n\ntwo\n\nthree");
        });
    });

    let move_rows =
        visual.new(|cx| Editor::from_markdown(cx, "one\n\ntwo\n\nthree\n\nfour".to_owned(), None));
    visual.update(|window, cx| {
        move_rows.update(cx, |editor, cx| {
            let visible = editor.document.visible_blocks().to_vec();
            editor.cross_block_selection = Some(CrossBlockSelection {
                anchor: CrossBlockSelectionEndpoint {
                    entity_id: visible[1].entity.entity_id(),
                    offset: 0,
                },
                focus: CrossBlockSelectionEndpoint {
                    entity_id: visible[2].entity.entity_id(),
                    offset: visible[2].entity.read(cx).visible_len(),
                },
                source_anchor: None,
                source_focus: None,
            });
            visible[2].entity.read(cx).focus_handle.focus(window);
            editor.on_move_line_down(&MoveLineDown, window, cx);

            assert_eq!(editor.source_document.text(), "one\n\nfour\n\ntwo\n\nthree");
            assert_eq!(editor.undo_history.len(), 1);
            editor.undo_document(cx);
            assert_eq!(editor.source_document.text(), "one\n\ntwo\n\nthree\n\nfour");

            let visible = editor.document.visible_blocks().to_vec();
            editor.cross_block_selection = Some(CrossBlockSelection {
                anchor: CrossBlockSelectionEndpoint {
                    entity_id: visible[2].entity.entity_id(),
                    offset: 0,
                },
                focus: CrossBlockSelectionEndpoint {
                    entity_id: visible[3].entity.entity_id(),
                    offset: visible[3].entity.read(cx).visible_len(),
                },
                source_anchor: None,
                source_focus: None,
            });
            visible[3].entity.read(cx).focus_handle.focus(window);
            editor.on_move_line_up(&MoveLineUp, window, cx);

            assert_eq!(editor.source_document.text(), "one\n\nthree\n\nfour\n\ntwo");
            assert_eq!(editor.undo_history.len(), 1);
            editor.undo_document(cx);
            assert_eq!(editor.source_document.text(), "one\n\ntwo\n\nthree\n\nfour");
        });
    });

    let delete_rows =
        visual.new(|cx| Editor::from_markdown(cx, "one\n\ntwo\n\nthree\n\nfour".to_owned(), None));
    visual.update(|window, cx| {
        delete_rows.update(cx, |editor, cx| {
            let visible = editor.document.visible_blocks().to_vec();
            editor.cross_block_selection = Some(CrossBlockSelection {
                anchor: CrossBlockSelectionEndpoint {
                    entity_id: visible[1].entity.entity_id(),
                    offset: 0,
                },
                focus: CrossBlockSelectionEndpoint {
                    entity_id: visible[2].entity.entity_id(),
                    offset: visible[2].entity.read(cx).visible_len(),
                },
                source_anchor: None,
                source_focus: None,
            });
            visible[2].entity.read(cx).focus_handle.focus(window);
            editor.on_delete_line(&DeleteLine, window, cx);

            assert_eq!(editor.source_document.text(), "one\n\nfour");
            assert_eq!(editor.undo_history.len(), 1);
            editor.undo_document(cx);
            assert_eq!(editor.source_document.text(), "one\n\ntwo\n\nthree\n\nfour");
        });
    });
}

/// Keeps the caret column and selection anchor when vertical movement crosses multiple blocks.
#[gpui::test]
async fn vertical_selection_preserves_column_across_blocks(cx: &mut TestAppContext) {
    let visual = cx.add_empty_window();
    let editor =
        visual.new(|cx| Editor::from_markdown(cx, "alpha\n\nbeta\n\ngamma".to_owned(), None));

    visual.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            let blocks = editor.document.visible_blocks().to_vec();
            let preferred_x = px(42.0);
            blocks[0].entity.update(cx, |block, _cx| {
                block.vertical_motion_x = Some(preferred_x);
                block.focus_handle.focus(window);
            });

            editor.on_select_down(&SelectDown, window, cx);
            editor.on_select_down(&SelectDown, window, cx);

            let selection = editor
                .cross_block_selection
                .expect("selection should cross all three blocks");
            assert_eq!(selection.anchor.entity_id, blocks[0].entity.entity_id());
            assert_eq!(selection.focus.entity_id, blocks[2].entity.entity_id());
            assert_eq!(
                blocks[1].entity.read(cx).vertical_motion_x,
                Some(preferred_x)
            );
            assert_eq!(
                blocks[2].entity.read(cx).vertical_motion_x,
                Some(preferred_x)
            );
        });
    });
}

/// Exercises the public action path so Source line edits use the same undo transaction as typing.
#[gpui::test]
async fn source_line_action_dispatch_edits_and_undoes_one_logical_row(cx: &mut TestAppContext) {
    init_line_operation_test_app(cx);
    let original = "alpha\nbeta";
    let (editor, visual) = cx.add_window_view(|_window, cx| {
        Editor::from_markdown(cx, original.to_owned(), None)
    });
    redraw_line_operation_test_window(visual);

    editor.update_in(visual, |editor, window, cx| {
        editor.set_view_mode(ViewMode::Source, cx);
        let source = editor.document.first_root().expect("Source text block").clone();
        source.update(cx, |block, _cx| {
            block.selected_range = "alpha\n".len().."alpha\n".len();
            block.focus_handle.focus(window);
        });
    });
    redraw_line_operation_test_window(visual);

    visual.dispatch_action(DuplicateLine);
    redraw_line_operation_test_window(visual);
    assert_eq!(
        editor.read_with(visual, |editor, _cx| editor.source_document.text()),
        "alpha\nbeta\nbeta"
    );
    assert_eq!(editor.read_with(visual, |editor, _cx| editor.undo_history.len()), 1);

    visual.dispatch_action(crate::components::Undo);
    redraw_line_operation_test_window(visual);
    assert_eq!(
        editor.read_with(visual, |editor, _cx| editor.source_document.text()),
        original
    );
}

/// Creates real Editor history so read-only assertions exercise populated undo and redo stacks.
fn seed_line_operation_history(
    editor: &mut Editor,
    window: &mut gpui::Window,
    cx: &mut gpui::Context<Editor>,
) -> String {
    let first = editor.document.first_root().expect("first text block").clone();
    first.read(cx).focus_handle.focus(window);
    editor.on_duplicate_line(&DuplicateLine, window, cx);
    assert_eq!(editor.undo_history.len(), 1);
    assert!(editor.redo_history.is_empty());
    editor.source_document.text()
}

/// Verifies read-only Preview projections cannot change source, revision, dirty state, or history.
fn assert_read_only_commands_preserve_document(
    editor: &mut Editor,
    window: &mut gpui::Window,
    cx: &mut gpui::Context<Editor>,
) {
    let source = editor.source_document.text();
    let revision = editor.source_document.revision();
    let dirty = editor.document_dirty;
    let undo_count = editor.undo_history.len();
    let redo_count = editor.redo_history.len();

    editor.on_duplicate_line(&DuplicateLine, window, cx);
    editor.on_delete_line(&DeleteLine, window, cx);
    editor.on_move_line_up(&MoveLineUp, window, cx);
    editor.on_move_line_down(&MoveLineDown, window, cx);
    editor.on_indent_block(&IndentBlock, window, cx);
    editor.on_outdent_block(&OutdentBlock, window, cx);
    editor.on_undo(&crate::components::Undo, window, cx);
    editor.on_redo(&crate::components::Redo, window, cx);

    assert_eq!(editor.source_document.text(), source);
    assert_eq!(editor.source_document.revision(), revision);
    assert_eq!(editor.document_dirty, dirty);
    assert_eq!(editor.undo_history.len(), undo_count);
    assert_eq!(editor.redo_history.len(), redo_count);
}

/// Keeps row operations and history read-only from Preview and Split's right projection.
#[gpui::test]
async fn preview_and_split_preview_commands_preserve_document_history(cx: &mut TestAppContext) {
    init_line_operation_test_app(cx);
    let (editor, visual) = cx.add_window_view(|_window, cx| {
        Editor::from_markdown(cx, "first\n\nsecond".to_owned(), None)
    });
    redraw_line_operation_test_window(visual);

    let edited = editor.update_in(visual, seed_line_operation_history);
    editor.update_in(visual, |editor, _window, cx| {
        editor.set_view_mode(ViewMode::Preview, cx);
    });
    redraw_line_operation_test_window(visual);
    editor.update_in(visual, |editor, window, cx| {
        let preview = editor.document.first_root().expect("Preview block").clone();
        preview.read(cx).focus_handle.focus(window);
        assert!(!editor.undo_history.is_empty(), "Preview must exercise Undo protection");
        assert_read_only_commands_preserve_document(editor, window, cx);
    });

    editor.update_in(visual, |editor, window, cx| {
        editor.set_view_mode(ViewMode::Rendered, cx);
        let body = editor.document.first_root().expect("Live block").clone();
        body.read(cx).focus_handle.focus(window);
        editor.on_undo(&crate::components::Undo, window, cx);
        assert_eq!(editor.source_document.text(), "first\n\nsecond");
        assert!(editor.undo_history.is_empty());
        assert_eq!(editor.redo_history.len(), 1);
        editor.set_view_mode(ViewMode::Split, cx);
    });
    redraw_line_operation_test_window(visual);
    editor.update_in(visual, |editor, window, cx| {
        let preview = editor
            .split_preview
            .as_ref()
            .expect("Split preview should be installed")
            .document
            .visible_blocks()
            .first()
            .expect("Split preview block")
            .entity
            .clone();
        preview.read(cx).focus_handle.focus(window);
        assert_eq!(editor.source_document.text(), "first\n\nsecond");
        assert_eq!(editor.redo_history.len(), 1, "Split must exercise Redo protection");
        assert_read_only_commands_preserve_document(editor, window, cx);
    });

    assert_ne!(edited, "first\n\nsecond");
}

/// Keeps tool-field Undo/Redo local while allowing the same actions from a focused document block.
#[gpui::test]
async fn find_field_history_does_not_borrow_stale_document_focus(cx: &mut TestAppContext) {
    init_line_operation_test_app(cx);
    let (editor, visual) = cx.add_window_view(|_window, cx| {
        Editor::from_markdown(cx, "first\n\nsecond".to_owned(), None)
    });
    redraw_line_operation_test_window(visual);

    let edited = editor.update_in(visual, seed_line_operation_history);
    editor.update_in(visual, |editor, window, cx| {
        let body = editor.document.first_root().expect("Live block").clone();
        body.read(cx).focus_handle.focus(window);
        editor.on_find_in_document_action(&FindInDocument, window, cx);
        let query = editor
            .find_panel
            .as_ref()
            .expect("Find panel should open")
            .query
            .clone();
        assert!(query.read(cx).focus_handle.is_focused(window));

        let source = editor.source_document.text();
        let revision = editor.source_document.revision();
        let dirty = editor.document_dirty;
        let undo_count = editor.undo_history.len();
        let redo_count = editor.redo_history.len();
        editor.on_undo(&crate::components::Undo, window, cx);
        editor.on_redo(&crate::components::Redo, window, cx);
        assert_eq!(editor.source_document.text(), source);
        assert_eq!(editor.source_document.revision(), revision);
        assert_eq!(editor.document_dirty, dirty);
        assert_eq!(editor.undo_history.len(), undo_count);
        assert_eq!(editor.redo_history.len(), redo_count);

        body.read(cx).focus_handle.focus(window);
        editor.on_undo(&crate::components::Undo, window, cx);
        assert_eq!(editor.source_document.text(), "first\n\nsecond");
        assert!(editor.undo_history.is_empty());
        assert_eq!(editor.redo_history.len(), 1);

        let body = editor.document.first_root().expect("restored Live block").clone();
        body.read(cx).focus_handle.focus(window);
        editor.on_redo(&crate::components::Redo, window, cx);
        assert_eq!(editor.source_document.text(), edited);
        assert_eq!(editor.undo_history.len(), 1);
        assert!(editor.redo_history.is_empty());
    });
}
