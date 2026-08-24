// @author kongweiguang

#[gpui::test]
/// Split 右侧复用 Live 的代码表面和高亮，但它没有 SourceDocument 事务所有权；
/// 所有语言编辑入口都必须保持只读，避免切回 Live 后看到不同结果。
async fn split_preview_code_language_cannot_diverge_from_live(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let source = "```rust\nlet x = 1;\n```";
    let (editor, visual_cx) = cx.add_window_view(move |_window, cx| {
        Editor::from_markdown(cx, source.to_owned(), None)
    });
    let live_render = editor.read_with(visual_cx, |editor, cx| {
        let block = editor.document.first_root().expect("Live code block");
        let block = block.read(cx);
        (
            block.display_text().to_string(),
            block.code_language_text().to_owned(),
            block
                .code_highlight_result()
                .cloned()
                .expect("Live code highlight"),
        )
    });
    editor.update(visual_cx, |editor, cx| {
        editor.set_view_mode(ViewMode::Split, cx);
    });
    redraw(visual_cx);

    editor.update_in(visual_cx, |editor, window, cx| {
        let preview = editor
            .split_preview
            .as_ref()
            .expect("Split preview runtime")
            .document
            .first_root()
            .expect("preview code block")
            .clone();
        preview.update(cx, |block, block_cx| {
            assert!(block.is_read_only());
            assert_eq!(block.display_text(), live_render.0);
            assert_eq!(block.code_language_text(), live_render.1);
            assert_eq!(block.code_highlight_result(), Some(&live_render.2));
            block.toggle_code_language_menu(window, block_cx);
            block.select_code_language_menu_item(1, block_cx);
            block.replace_code_language_text_in_range(0..4, "javascript", None, false, block_cx);
            assert!(!block.code_language_menu_open);
            assert_eq!(block.code_language_text(), "rust");
        });
        assert_eq!(editor.source_document.text(), source);
    });
}
