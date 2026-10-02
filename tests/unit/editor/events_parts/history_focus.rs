// @author kongweiguang

// 回归使用宿主默认修饰键，避免仅在 Windows/Linux 绑定上触发历史动作。
const HISTORY_SELECT_ALL_KEY: &str = if cfg!(target_os = "macos") { "cmd-a" } else { "ctrl-a" };
const HISTORY_UNDO_KEY: &str = if cfg!(target_os = "macos") { "cmd-z" } else { "ctrl-z" };
const HISTORY_REDO_KEY: &str = if cfg!(target_os = "macos") { "cmd-shift-z" } else { "ctrl-shift-z" };

/// Confirms the code-language input keeps Ctrl+A local while Undo/Redo use document history.
#[gpui::test]
async fn code_language_focus_keeps_select_all_local_and_uses_document_history(
    cx: &mut TestAppContext,
) {
    init_line_operation_test_app(cx);
    let original = "```rust\nfn main() {}\n```";
    let (editor, visual) = cx.add_window_view(|_window, cx| {
        Editor::from_markdown(cx, original.to_owned(), None)
    });
    redraw_line_operation_test_window(visual);

    editor.update_in(visual, |editor, window, cx| {
        let code = editor.document.first_root().expect("code block").clone();
        code.update(cx, |block, _block_cx| {
            block.selected_range = 2..2;
            block.code_language_selected_range = 1..2;
            block.code_language_focus_handle.focus(window);
            assert!(block.code_language_focus_handle.is_focused(window));
            assert!(!block.focus_handle.is_focused(window));
        });
    });
    redraw_line_operation_test_window(visual);
    visual.simulate_keystrokes(HISTORY_SELECT_ALL_KEY);
    redraw_line_operation_test_window(visual);

    editor.read_with(visual, |editor, cx| {
        let code = editor.document.first_root().expect("code block");
        assert_eq!(code.read(cx).code_language_selected_range, 0..4);
        assert_eq!(code.read(cx).selected_range, 2..2);
    });

    editor.update_in(visual, |editor, _window, cx| {
        let code = editor.document.first_root().expect("code block").clone();
        code.update(cx, |block, block_cx| {
            block.replace_code_language_text_in_range(0..4, "python", None, false, block_cx);
        });
    });
    redraw_line_operation_test_window(visual);
    assert_eq!(
        editor.read_with(visual, |editor, _cx| editor.source_document.text()),
        "```python\nfn main() {}\n```"
    );

    visual.simulate_keystrokes(HISTORY_UNDO_KEY);
    redraw_line_operation_test_window(visual);
    assert_eq!(
        editor.read_with(visual, |editor, _cx| editor.source_document.text()),
        original
    );

    editor.update_in(visual, |editor, window, cx| {
        editor
            .document
            .first_root()
            .expect("restored code block")
            .read(cx)
            .code_language_focus_handle
            .focus(window);
    });
    redraw_line_operation_test_window(visual);
    visual.simulate_keystrokes(HISTORY_REDO_KEY);
    redraw_line_operation_test_window(visual);
    assert_eq!(
        editor.read_with(visual, |editor, _cx| editor.source_document.text()),
        "```python\nfn main() {}\n```"
    );
}

/// Verifies formula source and structured math each dispatch history through the bound Editor action.
#[gpui::test]
async fn formula_source_and_structure_focus_use_document_undo_redo(cx: &mut TestAppContext) {
    init_line_operation_test_app(cx);
    let source_original = "$$\nx^2\n$$";
    let (source_editor, source_visual) = cx.add_window_view(|_window, cx| {
        Editor::from_markdown(cx, source_original.to_owned(), None)
    });
    redraw_line_operation_test_window(source_visual);

    source_editor.update_in(source_visual, |editor, window, cx| {
        let formula = editor.document.first_root().expect("formula").clone();
        formula.update(cx, |block, block_cx| {
            block.focus_handle.focus(window);
            block.sync_math_edit_focus(true, window, block_cx);
            block.math_source_focus_handle.focus(window);
            assert!(block.math_source_focus_handle.is_focused(window));
            assert!(!block.focus_handle.is_focused(window));
            assert!(block.replace_math_source_text_in_range(
                0..3,
                "y+1",
                None,
                false,
                crate::components::UndoCaptureKind::NonCoalescible,
                block_cx,
            ));
        });
    });
    redraw_line_operation_test_window(source_visual);
    assert_eq!(
        source_editor.read_with(source_visual, |editor, _cx| editor.source_document.text()),
        "$$\ny+1\n$$"
    );
    source_visual.simulate_keystrokes(HISTORY_UNDO_KEY);
    redraw_line_operation_test_window(source_visual);
    assert_eq!(
        source_editor.read_with(source_visual, |editor, _cx| editor.source_document.text()),
        source_original
    );
    source_editor.update_in(source_visual, |editor, window, cx| {
        editor
            .document
            .first_root()
            .expect("restored formula")
            .read(cx)
            .math_source_focus_handle
            .focus(window);
    });
    redraw_line_operation_test_window(source_visual);
    source_visual.simulate_keystrokes(HISTORY_REDO_KEY);
    redraw_line_operation_test_window(source_visual);
    assert_eq!(
        source_editor.read_with(source_visual, |editor, _cx| editor.source_document.text()),
        "$$\ny+1\n$$"
    );

    let structure_original = "$$\nx^2\n$$";
    let (structure_editor, structure_visual) = cx.add_window_view(|_window, cx| {
        Editor::from_markdown(cx, structure_original.to_owned(), None)
    });
    redraw_line_operation_test_window(structure_visual);
    structure_editor.update_in(structure_visual, |editor, window, cx| {
        let formula = editor.document.first_root().expect("formula").clone();
        formula.update(cx, |block, block_cx| {
            block.focus_handle.focus(window);
            block.sync_math_edit_focus(true, window, block_cx);
            assert!(block.execute_math_command_live(
                gmark_math_edit::MathEditCommand::InsertText("+1".to_owned()),
                crate::components::UndoCaptureKind::NonCoalescible,
                block_cx,
            ));
            block.math_structure_focus_handle.focus(window);
            assert!(block.math_structure_focus_handle.is_focused(window));
            assert!(!block.focus_handle.is_focused(window));
        });
    });
    redraw_line_operation_test_window(structure_visual);
    let structure_edited = "$$\n+1x^2\n$$";
    assert_eq!(
        structure_editor.read_with(structure_visual, |editor, _cx| editor.source_document.text()),
        structure_edited
    );
    structure_visual.simulate_keystrokes(HISTORY_UNDO_KEY);
    redraw_line_operation_test_window(structure_visual);
    assert_eq!(
        structure_editor.read_with(structure_visual, |editor, _cx| editor.source_document.text()),
        structure_original
    );
    structure_editor.update_in(structure_visual, |editor, window, cx| {
        editor
            .document
            .first_root()
            .expect("restored formula")
            .read(cx)
            .math_structure_focus_handle
            .focus(window);
    });
    redraw_line_operation_test_window(structure_visual);
    structure_visual.simulate_keystrokes(HISTORY_REDO_KEY);
    redraw_line_operation_test_window(structure_visual);
    assert_eq!(
        structure_editor.read_with(structure_visual, |editor, _cx| editor.source_document.text()),
        structure_edited
    );
}
