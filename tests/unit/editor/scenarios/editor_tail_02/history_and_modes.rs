// @author kongweiguang

#[gpui::test]
async fn redo_restores_text_reverted_by_undo(cx: &mut TestAppContext) {
    let editor = cx.new(|cx| Editor::from_markdown(cx, "alpha".to_string(), None));

    editor.update(cx, |editor, cx| {
        let block = editor.document.first_root().expect("root").clone();
        editor.active_entity_id = Some(block.entity_id());
        block.update(cx, |block, cx| {
            block.prepare_undo_capture(crate::components::UndoCaptureKind::CoalescibleText, cx);
            block.replace_text_in_visible_range(5..5, " beta", None, false, cx);
        });
    });

    editor.update(cx, |editor, cx| {
        editor.undo_document(cx);
        assert_eq!(editor.document.markdown_text(cx), "alpha");
        assert_eq!(editor.redo_history.len(), 1);

        editor.redo_document(cx);
        assert_eq!(editor.document.markdown_text(cx), "alpha beta");
        assert!(editor.redo_history.is_empty());
    });
}

#[gpui::test]
async fn fresh_edit_clears_pending_redo_history(cx: &mut TestAppContext) {
    let editor = cx.new(|cx| Editor::from_markdown(cx, "alpha".to_string(), None));

    editor.update(cx, |editor, cx| {
        let block = editor.document.first_root().expect("root").clone();
        editor.active_entity_id = Some(block.entity_id());
        block.update(cx, |block, cx| {
            block.prepare_undo_capture(crate::components::UndoCaptureKind::CoalescibleText, cx);
            block.replace_text_in_visible_range(5..5, " beta", None, false, cx);
        });
    });

    editor.update(cx, |editor, cx| {
        editor.undo_document(cx);
        assert_eq!(editor.redo_history.len(), 1);

        // A new edit invalidates the redo stack so it cannot revive stale text.
        let block = editor.document.first_root().expect("root").clone();
        block.update(cx, |block, cx| {
            block.prepare_undo_capture(crate::components::UndoCaptureKind::CoalescibleText, cx);
            block.replace_text_in_visible_range(5..5, " gamma", None, false, cx);
        });
    });

    editor.update(cx, |editor, cx| {
        editor.finalize_pending_undo_capture(cx);
        assert!(editor.redo_history.is_empty());

        editor.redo_document(cx);
        assert_eq!(editor.document.markdown_text(cx), "alpha gamma");
    });
}

#[gpui::test]
async fn toggle_view_mode_preserves_paragraph_caret_position(cx: &mut TestAppContext) {
    let editor = cx.new(|cx| Editor::from_markdown(cx, "alpha\n\nbeta".to_string(), None));

    editor.update(cx, |editor, cx| {
        let target = editor.document.visible_blocks()[1].entity.clone();
        target.update(cx, |block, _cx| {
            block.selected_range = 2..2;
        });
        editor.active_entity_id = Some(target.entity_id());

        editor.toggle_view_mode(cx);
        assert!(matches!(editor.view_mode, ViewMode::Source));
        let source = editor.document.first_root().expect("source root").clone();
        assert_eq!(source.read(cx).selected_range, 9..9);
        assert!(source.read(cx).show_source_line_numbers());

        editor.toggle_view_mode(cx);
        assert!(matches!(editor.view_mode, ViewMode::Rendered));
        let visible = editor.document.visible_blocks();
        assert_eq!(visible.len(), 2);
        assert!(
            visible
                .iter()
                .all(|visible| !visible.entity.read(cx).show_source_line_numbers())
        );
        assert_eq!(visible[1].entity.read(cx).display_text(), "beta");
        assert_eq!(visible[1].entity.read(cx).selected_range, 2..2);
        assert_eq!(editor.pending_focus, Some(visible[1].entity.entity_id()));
    });
}

#[gpui::test]
async fn toggle_view_mode_ends_stale_code_block_pointer_selection(cx: &mut TestAppContext) {
    let editor =
        cx.new(|cx| Editor::from_markdown(cx, "```rust\nfn main() {}\n```".to_string(), None));

    editor.update(cx, |editor, cx| {
        let target = editor.document.visible_blocks()[0].entity.clone();
        target.update(cx, |block, _cx| {
            block.selected_range = 3..7;
            block.is_selecting = true;
            block.code_language_is_selecting = true;
        });
        editor.active_entity_id = Some(target.entity_id());

        editor.toggle_view_mode(cx);

        assert!(matches!(editor.view_mode, ViewMode::Source));
        target.read_with(cx, |block, _cx| {
            assert!(!block.is_selecting);
            assert!(!block.code_language_is_selecting);
            assert_eq!(block.selected_range, 3..7);
        });
    });
}

/// 视图切换必须由当前平台产品键位触发，避免继续把已移除的 Ctrl+Tab 当作契约。
#[gpui::test]
async fn platform_toggle_shortcut_toggles_view_mode(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let (editor, cx) =
        cx.add_window_view(|_window, cx| Editor::from_markdown(cx, "alpha".to_string(), None));
    let toggle_shortcut = if cfg!(target_os = "macos") {
        "cmd-/"
    } else {
        "ctrl-/"
    };

    redraw(cx);
    cx.simulate_keystrokes(toggle_shortcut);
    redraw(cx);

    editor.update(cx, |editor, _cx| {
        assert!(matches!(editor.view_mode, ViewMode::Source));
    });

    cx.simulate_keystrokes(toggle_shortcut);
    redraw(cx);

    editor.update(cx, |editor, _cx| {
        assert!(matches!(editor.view_mode, ViewMode::Rendered));
    });
}
