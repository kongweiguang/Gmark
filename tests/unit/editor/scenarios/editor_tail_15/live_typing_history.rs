// @author kongweiguang

use super::*;

/// 点击与选区刷新不得把规范化投影当成原正文；文字替换撤销必须保留原 Markdown 拼写。
#[gpui::test]
async fn live_typing_undo_restores_original_source_spelling_and_selected_word(
    cx: &mut TestAppContext,
) {
    init_editor_test_app(cx);
    let original = "alpha_beta\n\nuntouched_word and __bold__\n\n| A | B |\n|---|---|\n| one | two |";
    let (editor, visual) = cx.add_window_view(move |_window, cx| {
        Editor::from_markdown(cx, original.to_owned(), None)
    });
    redraw(visual);
    editor.update_in(visual, |editor, window, cx| {
        let block = editor.document.first_root().expect("first word").clone();
        editor.focus_block(block.entity_id());
        block.update(cx, |block, cx| {
            block.selected_range = 0..10;
            block.selection_reversed = false;
            block.focus_handle.focus(window);
            cx.notify();
        });
    });
    redraw(visual);
    visual.simulate_input("你");
    redraw(visual);
    assert!(editor.read_with(visual, |editor, _cx| editor.source_document.text().starts_with("你")));
    visual.simulate_keystrokes("ctrl-z");
    redraw(visual);
    editor.read_with(visual, |editor, cx| {
        assert_eq!(editor.source_document.text(), original);
        let block = editor.document.first_root().expect("restored word").read(cx);
        assert_eq!(block.selected_range, 0..10);
        assert!(!block.selection_reversed);
    });
    visual.simulate_keystrokes("ctrl-shift-z");
    redraw(visual);
    assert!(editor.read_with(visual, |editor, _cx| editor.source_document.text().starts_with("你")));
}

/// 非规范前缀会改变序列化偏移；撤销必须把原反向选区投回原正文中的第二段。
#[gpui::test]
async fn live_typing_undo_restores_reverse_selection_after_noncanonical_prefix(
    cx: &mut TestAppContext,
) {
    init_editor_test_app(cx);
    let original = "alpha_beta\n\nuntouched_word and __bold__";
    let (editor, visual) = cx.add_window_view(move |_window, cx| {
        Editor::from_markdown(cx, original.to_owned(), None)
    });
    redraw(visual);
    editor.update_in(visual, |editor, window, cx| {
        let block = editor.document.root_blocks()[1].clone();
        editor.focus_block(block.entity_id());
        block.update(cx, |block, cx| {
            block.selected_range = 0..14;
            block.selection_reversed = true;
            block.focus_handle.focus(window);
            cx.notify();
        });
    });
    redraw(visual);
    visual.simulate_input("你");
    redraw(visual);
    visual.simulate_keystrokes("ctrl-z");
    redraw(visual);
    editor.read_with(visual, |editor, cx| {
        assert_eq!(editor.source_document.text(), original);
        let block = editor.document.root_blocks()[1].read(cx);
        assert_eq!(block.selected_range, 0..14);
        assert!(block.selection_reversed);
    });
    editor.update(visual, |editor, cx| editor.set_view_mode(ViewMode::Source, cx));
    redraw(visual);
    editor.read_with(visual, |editor, cx| {
        assert_eq!(editor.source_document.text(), original);
        let block = editor.document.first_root().expect("source root").read(cx);
        assert_eq!(block.selected_range, 12..26);
        assert!(block.selection_reversed);
    });
    editor.update(visual, |editor, cx| editor.set_view_mode(ViewMode::Rendered, cx));
    redraw(visual);
    editor.read_with(visual, |editor, cx| {
        let block = editor.document.root_blocks()[1].read(cx);
        assert_eq!(block.selected_range, 0..14);
        assert!(block.selection_reversed);
    });
}

/// 跨模式和 Preview 中转均保留原源码锚点，覆盖格式拼写、表格前缀及多字节字素。
#[gpui::test]
async fn source_live_preview_selection_roundtrip_preserves_original_unicode_coordinates(
    cx: &mut TestAppContext,
) {
    init_editor_test_app(cx);
    for (original, selected) in [
        ("alpha_beta\n\n中文👨‍👩‍👧‍👦 e\u{301} untouched_word", "中文👨‍👩‍👧‍👦 e\u{301}"),
        ("| A | B |\n|---|---|\n| one | two |\n\nuntouched_word", "untouched_word"),
        ("Title\n=====\n\n__bold__ and alpha_beta", "bold"),
    ] {
        let (editor, visual) = cx.add_window_view(move |_window, cx| {
            Editor::from_markdown(cx, original.to_owned(), None)
        });
        redraw(visual);
        let start = original.find(selected).expect("selected literal");
        let range = start..start + selected.len();
        editor.update(visual, |editor, cx| {
            editor.set_view_mode(ViewMode::Source, cx);
            let block = editor.document.first_root().expect("source root").clone();
            block.update(cx, |block, cx| {
                block.selected_range = range.clone();
                block.selection_reversed = true;
                cx.notify();
            });
            editor.set_view_mode(ViewMode::Rendered, cx);
        });
        redraw(visual);
        editor.update(visual, |editor, cx| editor.set_view_mode(ViewMode::Preview, cx));
        redraw(visual);
        editor.update(visual, |editor, cx| editor.set_view_mode(ViewMode::Source, cx));
        redraw(visual);
        editor.read_with(visual, |editor, cx| {
            assert_eq!(editor.source_document.text(), original);
            let block = editor.document.first_root().expect("source root").read(cx);
            assert_eq!(block.selected_range, range, "{original}");
            assert!(block.selection_reversed);
        });
    }
}

/// Live 创建的历史在 Source 撤销时仍使用原源码范围，而不是保存时所在投影的字节偏移。
#[gpui::test]
async fn source_undo_of_live_typing_restores_original_selection_coordinates(
    cx: &mut TestAppContext,
) {
    init_editor_test_app(cx);
    let original = "alpha_beta\n\nuntouched_word and __bold__";
    let (editor, visual) = cx.add_window_view(move |_window, cx| {
        Editor::from_markdown(cx, original.to_owned(), None)
    });
    redraw(visual);
    editor.update_in(visual, |editor, window, cx| {
        let block = editor.document.root_blocks()[1].clone();
        editor.focus_block(block.entity_id());
        block.update(cx, |block, cx| {
            block.selected_range = 0..14;
            block.selection_reversed = true;
            block.focus_handle.focus(window);
            cx.notify();
        });
    });
    redraw(visual);
    visual.simulate_input("你");
    redraw(visual);
    editor.update(visual, |editor, cx| editor.set_view_mode(ViewMode::Source, cx));
    redraw(visual);
    visual.simulate_keystrokes("ctrl-z");
    redraw(visual);
    editor.read_with(visual, |editor, cx| {
        assert_eq!(editor.source_document.text(), original);
        let block = editor.document.first_root().expect("source root").read(cx);
        assert_eq!(block.selected_range, 12..26);
        assert!(block.selection_reversed);
    });
}
