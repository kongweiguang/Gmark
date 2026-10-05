// @author kongweiguang

use gpui::{AppContext, Bounds, Context, TestAppContext, point, px, size};

use super::{CrossBlockDrag, CrossBlockSelection, CrossBlockSelectionEndpoint, Editor};
use crate::components::{
    BoldSelection, Cut, EditingCommandId, PointerSelectionGranularity, PointerSelectionSession,
    Undo, UndoCaptureKind,
};
use crate::i18n::I18nManager;
use crate::theme::ThemeManager;

fn init_editor_test_app(cx: &mut TestAppContext) {
    cx.update(|cx| {
        I18nManager::init(cx);
        ThemeManager::init(cx);
        crate::components::init(cx);
    });
}

fn redraw(cx: &mut gpui::VisualTestContext) {
    cx.update(|window, cx| window.draw(cx).clear());
    cx.run_until_parked();
}

/// Builds a direction-preserving fixture selection so each case starts from the same endpoint contract.
fn set_selection(
    editor: &mut Editor,
    start_index: usize,
    start_offset: usize,
    end_index: usize,
    end_offset: usize,
    cx: &mut Context<Editor>,
) {
    let visible = editor.document.visible_blocks().to_vec();
    let start = visible[start_index].entity.entity_id();
    let end = visible[end_index].entity.entity_id();
    editor.cross_block_selection = Some(CrossBlockSelection {
        anchor: CrossBlockSelectionEndpoint {
            entity_id: start,
            offset: start_offset,
        },
        focus: CrossBlockSelectionEndpoint {
            entity_id: end,
            offset: end_offset,
        },
        source_anchor: None,
        source_focus: None,
    });
    editor.sync_cross_block_selection_visuals(cx);
}

fn assign_visible_block_bounds(editor: &mut Editor, cx: &mut Context<Editor>) {
    for (index, visible) in editor
        .document
        .visible_blocks()
        .to_vec()
        .into_iter()
        .enumerate()
    {
        visible.entity.update(cx, move |block, _cx| {
            block.last_bounds = Some(Bounds::new(
                point(px(0.0), px(index as f32 * 32.0)),
                size(px(400.0), px(24.0)),
            ));
        });
    }
}

#[test]
fn mouse_down_starts_cross_block_drag_after_clearing_old_selection() {
    let mut cx = TestAppContext::single();
    init_editor_test_app(&mut cx);
    let editor = cx.new(|cx| Editor::from_markdown(cx, "alpha\n\nbeta\n\ngamma".to_string(), None));

    editor.update(&mut cx, |editor, cx| {
        assign_visible_block_bounds(editor, cx);
        set_selection(editor, 0, 0, 2, 2, cx);
        assert!(editor.cross_block_selection.is_some());
        assert!(
            editor
                .document
                .visible_blocks()
                .iter()
                .any(|visible| visible.entity.read(cx).editor_selection_range.is_some())
        );

        editor.begin_cross_block_drag_at_point(point(px(8.0), px(4.0)), cx);

        assert!(editor.cross_block_selection.is_none());
        assert!(editor.cross_block_drag.is_some());
        assert!(
            editor
                .document
                .visible_blocks()
                .iter()
                .all(|visible| visible.entity.read(cx).editor_selection_range.is_none())
        );
    });
    cx.quit();
}

/// Keeps same-block multi-click gestures local so the block owns word expansion.
#[test]
fn same_block_multiclick_drag_remains_owned_by_block_selection() {
    let mut cx = TestAppContext::single();
    init_editor_test_app(&mut cx);
    let editor = cx.new(|cx| Editor::from_markdown(cx, "alpha beta gamma".to_string(), None));

    editor.update(&mut cx, |editor, cx| {
        assign_visible_block_bounds(editor, cx);
        let block = editor.document.visible_blocks()[0].entity.clone();
        let block_id = block.entity_id();
        block.update(cx, |block, _cx| {
            block.pointer_selection = Some(PointerSelectionSession {
                anchor_range: 6..10,
                granularity: PointerSelectionGranularity::Word,
            });
            block.selected_range = 6..10;
            block.is_selecting = true;
        });
        editor.cross_block_drag = Some(CrossBlockDrag {
            anchor: CrossBlockSelectionEndpoint {
                entity_id: block_id,
                offset: 8,
            },
            source_anchor: None,
        });

        editor.update_surface_pointer_selection(
            crate::editor::selection_surface::SelectionSurface::Main,
            point(px(24.0), px(4.0)),
            cx,
        );

        assert!(editor.cross_block_selection.is_none());
        assert_eq!(block.read(cx).selected_range, 6..10);
    });
    cx.quit();
}

/// Preserves the anchor word when a word-granularity drag reverses into another block.
#[test]
fn reverse_cross_block_word_drag_keeps_the_original_word_boundary() {
    let mut cx = TestAppContext::single();
    init_editor_test_app(&mut cx);
    let editor = cx.new(|cx| Editor::from_markdown(cx, "zero one\n\nalpha beta".to_string(), None));

    editor.update(&mut cx, |editor, cx| {
        assign_visible_block_bounds(editor, cx);
        let visible = editor.document.visible_blocks().to_vec();
        let anchor = visible[1].entity.clone();
        let focus = visible[0].entity.clone();
        let anchor_id = anchor.entity_id();
        let focus_id = focus.entity_id();
        anchor.update(cx, |block, _cx| {
            block.pointer_selection = Some(PointerSelectionSession {
                anchor_range: 6..10,
                granularity: PointerSelectionGranularity::Word,
            });
            block.selected_range = 6..10;
            block.is_selecting = true;
        });
        editor.cross_block_drag = Some(CrossBlockDrag {
            anchor: CrossBlockSelectionEndpoint {
                entity_id: anchor_id,
                offset: 8,
            },
            source_anchor: editor.cross_block_source_anchor_for_endpoint(
                crate::editor::selection_surface::SelectionSurface::Main,
                CrossBlockSelectionEndpoint {
                    entity_id: anchor_id,
                    offset: 8,
                },
                cx,
            ),
        });

        editor.update_surface_pointer_selection(
            crate::editor::selection_surface::SelectionSurface::Main,
            point(px(24.0), px(4.0)),
            cx,
        );

        let selection = editor
            .cross_block_selection
            .expect("cross-block word selection");
        assert_eq!(
            (selection.anchor, selection.focus),
            (
                CrossBlockSelectionEndpoint {
                    entity_id: anchor_id,
                    offset: 10,
                },
                CrossBlockSelectionEndpoint {
                    entity_id: focus_id,
                    offset: 0,
                }
            )
        );
        assert!(selection.source_anchor.is_some());
        assert!(selection.source_focus.is_some());
        assert_eq!(focus.read(cx).editor_selection_range, Some(0..8));
        assert_eq!(anchor.read(cx).editor_selection_range, Some(0..10));
    });
    cx.quit();
}

/// Carries an existing focused selection anchor into a cross-block Shift drag.
#[test]
fn shift_drag_from_a_local_selection_keeps_its_focused_anchor_across_blocks() {
    let mut cx = TestAppContext::single();
    init_editor_test_app(&mut cx);
    let (editor, visual) = cx.add_window_view(|_window, cx| {
        Editor::from_markdown(cx, "alpha beta\n\ngamma delta".to_string(), None)
    });
    redraw(visual);

    editor.update_in(visual, |editor, window, cx| {
        assign_visible_block_bounds(editor, cx);
        let visible = editor.document.visible_blocks().to_vec();
        let anchor = visible[0].entity.clone();
        let focus = visible[1].entity.clone();
        let anchor_id = anchor.entity_id();
        let focus_id = focus.entity_id();
        anchor.update(cx, |block, _cx| {
            block.selected_range = 2..4;
            block.selection_reversed = false;
            block.focus_handle.focus(window);
        });

        editor.begin_surface_cross_block_drag_at_point(
            crate::editor::selection_surface::SelectionSurface::Main,
            point(px(0.0), px(36.0)),
            true,
            Some(window),
            cx,
        );
        editor.update_surface_pointer_selection(
            crate::editor::selection_surface::SelectionSurface::Main,
            point(px(0.0), px(36.0)),
            cx,
        );

        let selection = editor
            .cross_block_selection
            .expect("shift cross-block selection");
        assert_eq!(
            (selection.anchor, selection.focus),
            (
                CrossBlockSelectionEndpoint {
                    entity_id: anchor_id,
                    offset: 2,
                },
                CrossBlockSelectionEndpoint {
                    entity_id: focus_id,
                    offset: 0,
                }
            )
        );
        assert!(selection.source_anchor.is_some());
        assert!(selection.source_focus.is_some());
    });
}

/// Keeps ordinary cross-block Copy visible while retaining Markdown for rich table paste.
#[test]
fn cross_block_visible_copy_uses_inline_text_and_table_tsv() {
    let mut cx = TestAppContext::single();
    init_editor_test_app(&mut cx);
    let editor = cx.new(|cx| {
        Editor::from_markdown(
            cx,
            "**bold**\n\n[linked](https://example.test)\n\n```rust\nlet answer = 42;\n```\n\n| label | value |\n| --- | --- |\n| x | 42 |\n\nfinish"
                .to_owned(),
            None,
        )
    });

    editor.update(&mut cx, |editor, cx| {
        let visible = editor.document.visible_blocks().to_vec();
        let last = visible.len() - 1;
        let end = visible[last].entity.read(cx).visible_len();
        set_selection(editor, 0, 0, last, end, cx);

        let plain = editor
            .selected_visible_text_for_target(
                crate::editor::selection_surface::SelectionSurface::Main,
                &visible[0].entity,
                cx,
            )
            .expect("the cross-block selection should have visible text");
        assert_eq!(
            plain,
            "bold\n\nlinked\n\nlet answer = 42;\n\nlabel\tvalue\nx\t42\n\nfinish"
        );
        assert!(!plain.contains("**"));
        assert!(!plain.contains("[linked]"));

        let markdown = editor
            .selected_markdown_text(cx)
            .expect("the selection should retain Markdown for rich paste");
        assert!(markdown.contains("**bold**"));
        assert!(markdown.contains("[linked](https://example.test)"));
        assert!(markdown.contains("```rust"));
        assert!(markdown.contains("| label | value |"));
        let html = crate::adapters::export::render_clipboard_fragment_with_base_dir(
            &markdown,
            cx.global::<ThemeManager>().current(),
            None,
        );
        assert!(html.contains("<table"), "rich clipboard HTML: {html:?}");
    });
    cx.quit();
}

#[test]
fn typing_replaces_cross_block_selection_with_plain_text() {
    let mut cx = TestAppContext::single();
    init_editor_test_app(&mut cx);
    let editor = cx.new(|cx| Editor::from_markdown(cx, "alpha\n\nbeta\n\ngamma".to_string(), None));

    editor.update(&mut cx, |editor, cx| {
        set_selection(editor, 0, 2, 2, 2, cx);
        assert!(editor.replace_cross_block_selection_with_text(
            "X",
            None,
            false,
            UndoCaptureKind::CoalescibleText,
            cx
        ));

        assert_eq!(editor.document.markdown_text(cx), "alXmma");
        assert!(editor.cross_block_selection.is_none());
        assert!(editor.cross_block_drag.is_none());
        let block = editor.document.visible_blocks()[0].entity.read(cx);
        assert_eq!(block.selected_range, 3..3);
        assert!(block.marked_range.is_none());
    });
    cx.quit();
}

#[test]
fn ime_composition_replaces_cross_block_selection_and_marks_inserted_text() {
    let mut cx = TestAppContext::single();
    init_editor_test_app(&mut cx);
    let editor = cx.new(|cx| Editor::from_markdown(cx, "alpha\n\nbeta\n\ngamma".to_string(), None));

    editor.update(&mut cx, |editor, cx| {
        set_selection(editor, 0, 2, 2, 2, cx);
        assert!(editor.replace_cross_block_selection_with_text(
            "ni",
            Some(2..2),
            true,
            UndoCaptureKind::CoalescibleText,
            cx
        ));

        assert_eq!(editor.document.markdown_text(cx), "alnimma");
        let block = editor.document.visible_blocks()[0].entity.read(cx);
        assert_eq!(block.selected_range, 4..4);
        assert_eq!(block.marked_range, Some(2..4));
        assert!(block.editor_selection_range.is_none());
    });
    cx.quit();
}

#[test]
fn cross_block_selection_marks_visual_ranges_and_copies_markdown() {
    let mut cx = TestAppContext::single();
    init_editor_test_app(&mut cx);
    let editor = cx.new(|cx| {
        Editor::from_markdown(
            cx,
            "alpha **bold**\n\n- item\n\n![alt](image.png)".to_string(),
            None,
        )
    });

    editor.update(&mut cx, |editor, cx| {
        let visible = editor.document.visible_blocks().to_vec();
        assert_eq!(visible.len(), 3);
        let end_len = visible[2].entity.read(cx).visible_len();
        set_selection(editor, 0, 0, 2, end_len, cx);

        assert_eq!(
            editor.cross_block_selected_markdown(cx).as_deref(),
            Some("alpha **bold**\n\n- item\n\n![alt](image.png)")
        );
        for visible in visible {
            let block = visible.entity.read(cx);
            assert_eq!(block.editor_selection_range, Some(0..block.visible_len()));
        }
    });
    cx.quit();
}

#[test]
fn cross_block_inline_format_is_atomic_uniform_and_undoable() {
    let mut cx = TestAppContext::single();
    init_editor_test_app(&mut cx);
    let original = "alpha\n\n**beta**\n\ngamma";
    let editor = cx.new(|cx| Editor::from_markdown(cx, original.to_owned(), None));

    editor.update(&mut cx, |editor, cx| {
        let visible = editor.document.visible_blocks().to_vec();
        let end = visible[2].entity.read(cx).visible_len();
        set_selection(editor, 0, 0, 2, end, cx);
        assert!(editor.document.visible_blocks().iter().all(|visible| {
            visible
                .entity
                .read(cx)
                .editor_selection_supports_inline_commands
        }));
        assert!(editor.apply_cross_block_inline_command(EditingCommandId::Bold, cx));
        assert_eq!(
            editor.source_document.text(),
            "**alpha**\n\n**beta**\n\n**gamma**"
        );
        assert!(editor.cross_block_selection.is_some());
        assert_eq!(editor.undo_history.len(), 1);

        assert!(editor.apply_cross_block_inline_command(EditingCommandId::Bold, cx));
        assert_eq!(editor.source_document.text(), "alpha\n\nbeta\n\ngamma");
        assert_eq!(editor.undo_history.len(), 2);
        editor.undo_document(cx);
        assert_eq!(
            editor.source_document.text(),
            "**alpha**\n\n**beta**\n\n**gamma**"
        );
        editor.undo_document(cx);
        assert_eq!(editor.source_document.text(), original);
    });
    cx.quit();
}

#[test]
fn cross_block_format_shortcut_dispatches_one_atomic_editor_transaction() {
    let mut cx = TestAppContext::single();
    init_editor_test_app(&mut cx);
    let (editor, visual) = cx.add_window_view(|_window, cx| {
        Editor::from_markdown(cx, "alpha\n\nbeta\n\ngamma".to_owned(), None)
    });
    redraw(visual);

    editor.update_in(visual, |editor, window, cx| {
        let visible = editor.document.visible_blocks().to_vec();
        let end = visible[2].entity.read(cx).visible_len();
        let focus = visible[2].entity.clone();
        set_selection(editor, 0, 0, 2, end, cx);
        editor.focus_block(focus.entity_id());
        focus.read(cx).focus_handle.focus(window);
    });
    redraw(visual);

    visual.dispatch_action(BoldSelection);
    redraw(visual);

    editor.read_with(visual, |editor, _cx| {
        assert_eq!(
            editor.source_document.text(),
            "**alpha**\n\n**beta**\n\n**gamma**"
        );
        assert_eq!(editor.undo_history.len(), 1);
        assert!(editor.cross_block_selection.is_some());
    });
}

#[test]
fn incompatible_cross_block_inline_format_rejects_the_entire_selection() {
    let mut cx = TestAppContext::single();
    init_editor_test_app(&mut cx);
    let original = "alpha\n\n```rust\ncode\n```\n\ngamma";
    let editor = cx.new(|cx| Editor::from_markdown(cx, original.to_owned(), None));

    editor.update(&mut cx, |editor, cx| {
        let visible = editor.document.visible_blocks().to_vec();
        let end = visible[2].entity.read(cx).visible_len();
        set_selection(editor, 0, 0, 2, end, cx);
        assert!(editor.document.visible_blocks().iter().all(|visible| {
            !visible
                .entity
                .read(cx)
                .editor_selection_supports_inline_commands
        }));
        assert!(!editor.apply_cross_block_inline_command(EditingCommandId::Italic, cx));
        assert_eq!(editor.source_document.text(), original);
        assert!(editor.undo_history.is_empty());
    });
    cx.quit();
}

#[test]
fn cross_block_cut_writes_markdown_deletes_range_and_undo_restores() {
    let mut cx = TestAppContext::single();
    init_editor_test_app(&mut cx);
    let original = "alpha\n\nbeta\n\ngamma";
    let (editor, cx) = cx.add_window_view({
        let original = original.to_string();
        move |_window, cx| Editor::from_markdown(cx, original.clone(), None)
    });
    redraw(cx);

    editor.update(cx, |editor, cx| {
        set_selection(editor, 0, 2, 2, 2, cx);
        assert_eq!(
            editor.cross_block_selected_markdown(cx).as_deref(),
            Some("pha\n\nbeta\n\nga")
        );
    });
    redraw(cx);

    cx.dispatch_action(Cut);
    redraw(cx);

    assert_eq!(
        cx.read_from_clipboard()
            .and_then(|item| item.text())
            .as_deref(),
        Some("pha\n\nbeta\n\nga")
    );
    assert_eq!(
        editor.read_with(cx, |editor, cx| editor.document.markdown_text(cx)),
        "almma"
    );

    cx.dispatch_action(Undo);
    redraw(cx);

    assert_eq!(
        editor.read_with(cx, |editor, cx| editor.document.markdown_text(cx)),
        original
    );
    editor.read_with(cx, |editor, cx| {
        assert_eq!(
            editor.cross_block_selected_markdown(cx).as_deref(),
            Some("pha\n\nbeta\n\nga")
        );
    });
}

/// Keeps document content, revision, and undo history intact when native clipboard acceptance fails.
#[test]
fn cut_preserves_document_when_native_clipboard_write_fails() {
    let mut cx = TestAppContext::single();
    init_editor_test_app(&mut cx);
    let original = "alpha beta";
    let (editor, visual) =
        cx.add_window_view(|_window, cx| Editor::from_markdown(cx, original.to_owned(), None));
    redraw(visual);

    editor.update_in(visual, |editor, window, cx| {
        let block = editor.document.visible_blocks()[0].entity.clone();
        set_selection(editor, 0, 0, 0, 5, cx);
        editor.focus_block(block.entity_id());
        block.read(cx).focus_handle.focus(window);
    });
    redraw(visual);

    let (source_before, revision_before, history_before) =
        editor.read_with(visual, |editor, _cx| {
            (
                editor.source_document.text().to_owned(),
                editor.source_document.revision(),
                editor.undo_history.len(),
            )
        });
    Editor::with_rich_clipboard_write_failure_for_test(|| {
        visual.dispatch_action(Cut);
        redraw(visual);
    });

    editor.read_with(visual, |editor, _cx| {
        assert_eq!(editor.source_document.text(), source_before);
        assert_eq!(editor.source_document.revision(), revision_before);
        assert_eq!(editor.undo_history.len(), history_before);
    });
}

const TABLE_DOC: &str = "alpha\n\n| a | b |\n| --- | --- |\n| 1 | 2 |\n\ngamma";

#[test]
fn delete_selection_spanning_table_removes_table() {
    let mut cx = TestAppContext::single();
    init_editor_test_app(&mut cx);
    let editor = cx.new(|cx| Editor::from_markdown(cx, TABLE_DOC.to_string(), None));

    editor.update(&mut cx, |editor, cx| {
        let visible = editor.document.visible_blocks().to_vec();
        assert_eq!(visible.len(), 3);
        let end_len = visible[2].entity.read(cx).visible_len();
        // The table sits in the interior of the selection.
        set_selection(editor, 0, 0, 2, end_len, cx);
        assert!(editor.delete_cross_block_selection(cx));

        let text = editor.document.markdown_text(cx);
        assert!(!text.contains('|'), "table should be gone: {text:?}");
        assert!(!text.contains("alpha"));
        assert!(!text.contains("gamma"));
    });
    cx.quit();
}

#[test]
fn delete_selection_with_trailing_table_removes_table() {
    let mut cx = TestAppContext::single();
    init_editor_test_app(&mut cx);
    let editor = cx.new(|cx| Editor::from_markdown(cx, TABLE_DOC.to_string(), None));

    editor.update(&mut cx, |editor, cx| {
        assert_eq!(editor.document.visible_blocks().len(), 3);
        // Selection ends at the start of the table block: the previously
        // broken case where the trailing atomic block was left behind.
        set_selection(editor, 0, 0, 1, 0, cx);
        assert!(editor.delete_cross_block_selection(cx));

        // The table is removed in full; only `gamma` survives (deleting from
        // the document start leaves the table's trailing blank line, which
        // reparses to leading empty paragraphs, harmless and trimmable).
        let text = editor.document.markdown_text(cx);
        assert!(
            !text.contains('|'),
            "trailing table should be gone: {text:?}"
        );
        assert_eq!(text.trim(), "gamma");
    });
    cx.quit();
}

#[test]
fn delete_selection_of_only_table_removes_just_the_table() {
    let mut cx = TestAppContext::single();
    init_editor_test_app(&mut cx);
    let editor = cx.new(|cx| Editor::from_markdown(cx, TABLE_DOC.to_string(), None));

    editor.update(&mut cx, |editor, cx| {
        let visible = editor.document.visible_blocks().to_vec();
        assert_eq!(visible.len(), 3);
        let alpha_len = visible[0].entity.read(cx).visible_len();
        // Drag from the end of the paragraph above onto the table: only the
        // table is removed, and re-parse normalizes the spacing.
        set_selection(editor, 0, alpha_len, 1, 0, cx);
        assert!(editor.delete_cross_block_selection(cx));

        assert_eq!(editor.document.markdown_text(cx), "alpha\n\ngamma");
    });
    cx.quit();
}

#[test]
fn cut_selection_including_table_serializes_and_deletes_it() {
    // Exercise cut's two halves directly (the clipboard markdown and the
    // deleted source range) rather than dispatching the action, keeping this
    // a focused unit test of the cut logic.
    let mut cx = TestAppContext::single();
    init_editor_test_app(&mut cx);
    let editor = cx.new(|cx| Editor::from_markdown(cx, TABLE_DOC.to_string(), None));

    editor.update(&mut cx, |editor, cx| {
        let visible = editor.document.visible_blocks().to_vec();
        assert_eq!(visible.len(), 3);
        let end_len = visible[2].entity.read(cx).visible_len();
        set_selection(editor, 0, 0, 2, end_len, cx);

        // The clipboard markdown serializes the full table, matching what
        // delete removes; otherwise cut would drop it from the clipboard.
        let markdown = editor.cross_block_selected_markdown(cx).unwrap();
        assert!(markdown.contains("| a | b |"), "clipboard: {markdown:?}");
        assert!(markdown.contains("| 1 | 2 |"), "clipboard: {markdown:?}");
        assert!(markdown.contains("alpha") && markdown.contains("gamma"));

        assert!(editor.delete_cross_block_selection(cx));
        assert!(
            !editor.document.markdown_text(cx).contains('|'),
            "document should no longer contain the table"
        );
    });
    cx.quit();
}

#[test]
fn delete_selection_spanning_code_block_removes_it() {
    let mut cx = TestAppContext::single();
    init_editor_test_app(&mut cx);
    // Code blocks edit their raw text, so they are deletable as an ordinary
    // text range; this documents that visible_len-based behavior.
    let doc = "alpha\n\n```\ncode\n```\n\ngamma";
    let editor = cx.new(|cx| Editor::from_markdown(cx, doc.to_string(), None));

    editor.update(&mut cx, |editor, cx| {
        let visible = editor.document.visible_blocks().to_vec();
        assert_eq!(visible.len(), 3);
        let end_len = visible[2].entity.read(cx).visible_len();
        set_selection(editor, 0, 0, 2, end_len, cx);
        assert!(editor.delete_cross_block_selection(cx));

        let text = editor.document.markdown_text(cx);
        assert!(
            !text.contains("code"),
            "code block should be gone: {text:?}"
        );
    });
    cx.quit();
}

#[test]
/// 空段插入先完成真实源码事务，再验证跨块删除，避免测试使用尚未发布的结构坐标。
fn delete_selection_ending_on_trailing_empty_paragraph_removes_table() {
    let mut cx = TestAppContext::single();
    init_editor_test_app(&mut cx);
    let doc = "alpha\n\n| a | b |\n| --- | --- |\n| 1 | 2 |";
    let editor = cx.new(|cx| Editor::from_markdown(cx, doc.to_string(), None));

    editor.update(&mut cx, |editor, cx| {
        // Append a trailing empty paragraph, exactly as inserting a table at
        // the end of a document does. Ending the selection on it used to
        // abort deletion because empty roots had no source span.
        let empty = Editor::new_block(cx, crate::components::BlockRecord::paragraph(String::new()));
        let index = editor.document.root_count();
        editor
            .document
            .insert_blocks_at(None, index, vec![empty], cx);
        editor.mark_dirty(cx);
        assert!(editor.document.source_commit_error().is_none());

        let visible = editor.document.visible_blocks().to_vec();
        assert_eq!(visible.len(), 3);
        let alpha_len = visible[0].entity.read(cx).visible_len();
        // From the end of `alpha` onto the trailing empty paragraph.
        set_selection(editor, 0, alpha_len, 2, 0, cx);
        assert!(editor.delete_cross_block_selection(cx));

        let text = editor.document.markdown_text(cx);
        assert!(!text.contains('|'), "table should be gone: {text:?}");
        assert_eq!(text.trim(), "alpha");
    });
    cx.quit();
}

#[test]
/// 首段空白也必须绑定插入事务的源码范围，删除不能凭测试构造的旧树位置授权。
fn delete_selection_starting_on_empty_paragraph_removes_table() {
    let mut cx = TestAppContext::single();
    init_editor_test_app(&mut cx);
    let doc = "| a | b |\n| --- | --- |\n| 1 | 2 |\n\ngamma";
    let editor = cx.new(|cx| Editor::from_markdown(cx, doc.to_string(), None));

    editor.update(&mut cx, |editor, cx| {
        // Prepend a leading empty paragraph; starting the highlight on it used
        // to abort deletion (the user's "drag up from the text below into an
        // empty block above the table" case).
        let empty = Editor::new_block(cx, crate::components::BlockRecord::paragraph(String::new()));
        editor.document.insert_blocks_at(None, 0, vec![empty], cx);
        editor.mark_dirty(cx);
        assert!(editor.document.source_commit_error().is_none());

        let visible = editor.document.visible_blocks().to_vec();
        assert_eq!(visible.len(), 3);
        // From the empty paragraph (index 0) to the start of `gamma`.
        set_selection(editor, 0, 0, 2, 0, cx);
        assert!(editor.delete_cross_block_selection(cx));

        let text = editor.document.markdown_text(cx);
        assert!(!text.contains('|'), "table should be gone: {text:?}");
        assert_eq!(text.trim(), "gamma");
    });
    cx.quit();
}

#[test]
fn source_selection_with_interior_utf8_offsets_does_not_panic() {
    let mut cx = TestAppContext::single();
    init_editor_test_app(&mut cx);
    let editor = cx.new(|cx| Editor::from_markdown(cx, "中a".to_string(), None));

    editor.update(&mut cx, |editor, cx| {
        editor.set_view_mode(super::ViewMode::Source, cx);
        let source = editor.document.visible_blocks()[0].entity.clone();
        source.update(cx, |block, _cx| {
            // 模拟命中测试或投影映射把选择端点落在三字节中文字符内部。
            block.selected_range = 1..2;
        });

        assert_eq!(editor.selected_markdown_text(cx).as_deref(), Some("中"));
        assert_eq!(editor.compute_source_cursor_position(cx), (1, 1));
    });
    cx.quit();
}

include!("selection_unicode.rs");
