// @author kongweiguang

use super::*;

#[gpui::test]
/// Keeps Unicode preedit virtual across redraws and commits one source edit before mode/history checks.
async fn ime_composition_preserves_unicode_source_across_all_view_modes_and_history(
    cx: &mut TestAppContext,
) {
    init_editor_test_app(cx);
    let original = "开头 e\u{301} 😀 かな";
    let composing = "拼音👩‍💻";
    let committed = "中文👩‍💻";
    let expected_composing = format!("{original}{composing}");
    let expected_committed = format!("{original}{committed}");
    let (editor, visual) = cx
        .add_window_view(move |_window, cx| Editor::from_markdown(cx, original.to_string(), None));
    let block = editor.read_with(visual, |editor, _cx| {
        editor.document.first_root().expect("live root").clone()
    });
    let initial_revision =
        editor.read_with(visual, |editor, _cx| editor.source_document.revision());

    visual.update(|window, cx| {
        block.update(cx, |block, block_cx| {
            block.focus_handle.focus(window);
            let end = block.display_text().len();
            block.selected_range = end..end;
            let composing_utf16_len = composing.encode_utf16().count();
            <crate::components::Block as EntityInputHandler>::composition_started(
                block, window, block_cx,
            );
            <crate::components::Block as EntityInputHandler>::replace_and_mark_text_in_range(
                block,
                None,
                composing,
                Some(composing_utf16_len..composing_utf16_len),
                window,
                block_cx,
            );
        });
    });
    redraw(visual);
    let composing_revision = editor.read_with(visual, |editor, _cx| {
        assert_eq!(editor.source_document.text(), original);
        assert_eq!(editor.source_document.revision(), initial_revision);
        assert_eq!(
            block.read(_cx).display_text_with_ime().as_ref(),
            expected_composing
        );
        assert!(!editor.document_dirty);
        editor.source_document.revision()
    });
    assert_eq!(composing_revision, initial_revision);

    visual.update(|window, cx| {
        block.update(cx, |block, block_cx| {
            <crate::components::Block as EntityInputHandler>::replace_text_in_range(
                block, None, committed, window, block_cx,
            );
            assert_eq!(block.display_text(), original);
            assert_eq!(block.display_text_with_ime().as_ref(), expected_committed);
            <crate::components::Block as EntityInputHandler>::composition_ended(
                block,
                gpui::CompositionEnd::Committed,
                window,
                block_cx,
            );
        });
    });
    redraw(visual);
    let committed_revision = editor.read_with(visual, |editor, _cx| {
        assert_eq!(editor.source_document.text(), expected_committed);
        assert_eq!(editor.undo_history.len(), 1);
        editor.source_document.revision()
    });
    assert_ne!(committed_revision, initial_revision);

    editor.update(visual, |editor, cx| {
        for mode in [
            ViewMode::Source,
            ViewMode::Split,
            ViewMode::Preview,
            ViewMode::Rendered,
        ] {
            editor.set_view_mode(mode, cx);
            assert_eq!(editor.source_document.text(), expected_committed);
        }
    });

    let revision_after_history = editor.update(visual, |editor, cx| {
        editor.undo_document(cx);
        assert_eq!(editor.source_document.text(), original);
        editor.redo_document(cx);
        assert_eq!(editor.source_document.text(), expected_committed);
        editor.source_document.revision()
    });
    assert_ne!(revision_after_history, committed_revision);

    editor.update(visual, |editor, cx| {
        editor.set_view_mode(ViewMode::Preview, cx);
    });
    let preview = editor.read_with(visual, |editor, _cx| {
        editor.document.first_root().expect("preview root").clone()
    });
    visual.update(|window, cx| {
        preview.update(cx, |block, block_cx| {
            <crate::components::Block as EntityInputHandler>::replace_text_in_range(
                block,
                None,
                "不应写入",
                window,
                block_cx,
            );
        });
    });
    redraw(visual);
    editor.read_with(visual, |editor, _cx| {
        assert_eq!(editor.source_document.text(), expected_committed);
        assert_eq!(editor.source_document.revision(), revision_after_history);
    });
}

#[gpui::test]
/// Holds candidate updates past autosave intervals and seals each composition as one undo step.
async fn slow_ime_composition_is_one_undo_transaction_after_commit(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let original = "前缀 ";
    let (editor, visual) =
        cx.add_window_view(move |_window, cx| Editor::from_markdown(cx, original.to_owned(), None));
    let block = editor.read_with(visual, |editor, _cx| {
        editor.document.first_root().expect("live root").clone()
    });
    let initial_revision =
        editor.read_with(visual, |editor, _cx| editor.source_document.revision());

    visual.update(|window, cx| {
        block.update(cx, |block, block_cx| {
            block.focus_handle.focus(window);
            let end = block.display_text().len();
            block.selected_range = end..end;
            <crate::components::Block as EntityInputHandler>::composition_started(
                block, window, block_cx,
            );
            <crate::components::Block as EntityInputHandler>::replace_and_mark_text_in_range(
                block,
                None,
                "z",
                Some(1..1),
                window,
                block_cx,
            );
        });
    });
    visual.executor().advance_clock(Duration::from_secs(2));
    visual.run_until_parked();
    editor.read_with(visual, |editor, _cx| {
        assert_eq!(editor.source_document.text(), original);
        assert_eq!(editor.source_document.revision(), initial_revision);
        assert!(editor.undo_history.is_empty());
        assert!(!editor.document_dirty);
    });

    visual.update(|window, cx| {
        block.update(cx, |block, block_cx| {
            <crate::components::Block as EntityInputHandler>::replace_and_mark_text_in_range(
                block,
                None,
                "zhongwen",
                Some(9..9),
                window,
                block_cx,
            );
        });
    });
    visual.executor().advance_clock(Duration::from_secs(2));
    visual.run_until_parked();
    editor.read_with(visual, |editor, _cx| {
        assert_eq!(editor.source_document.text(), original);
        assert_eq!(editor.source_document.revision(), initial_revision);
        assert!(!editor.document_dirty);
        assert_eq!(
            block.read(_cx).display_text_with_ime().as_ref(),
            "前缀 zhongwen"
        );
    });

    visual.update(|window, cx| {
        block.update(cx, |block, block_cx| {
            <crate::components::Block as EntityInputHandler>::replace_text_in_range(
                block, None, "中文", window, block_cx,
            );
            assert_eq!(block.display_text(), original);
            assert_eq!(block.display_text_with_ime().as_ref(), "前缀 中文");
            <crate::components::Block as EntityInputHandler>::composition_ended(
                block,
                gpui::CompositionEnd::Committed,
                window,
                block_cx,
            );
        });
    });
    redraw(visual);
    editor.read_with(visual, |editor, _cx| {
        assert_eq!(editor.source_document.text(), "前缀 中文");
        assert_eq!(editor.undo_history.len(), 1);
    });

    visual.update(|window, cx| {
        block.update(cx, |block, block_cx| {
            <crate::components::Block as EntityInputHandler>::composition_started(
                block, window, block_cx,
            );
            <crate::components::Block as EntityInputHandler>::replace_and_mark_text_in_range(
                block,
                None,
                "c",
                Some(1..1),
                window,
                block_cx,
            );
            <crate::components::Block as EntityInputHandler>::replace_text_in_range(
                block, None, "测", window, block_cx,
            );
            assert_eq!(block.display_text(), "前缀 中文");
            assert_eq!(block.display_text_with_ime().as_ref(), "前缀 中文测");
            <crate::components::Block as EntityInputHandler>::composition_ended(
                block,
                gpui::CompositionEnd::Committed,
                window,
                block_cx,
            );
        });
    });
    redraw(visual);
    editor.update(visual, |editor, cx| {
        assert_eq!(editor.source_document.text(), "前缀 中文测");
        assert_eq!(editor.undo_history.len(), 2);
        editor.undo_document(cx);
        assert_eq!(editor.source_document.text(), "前缀 中文");
        editor.undo_document(cx);
        assert_eq!(editor.source_document.text(), original);
        editor.redo_document(cx);
        assert_eq!(editor.source_document.text(), "前缀 中文");
        editor.redo_document(cx);
        assert_eq!(editor.source_document.text(), "前缀 中文测");
    });
}
