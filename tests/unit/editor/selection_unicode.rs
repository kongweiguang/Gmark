// @author kongweiguang

/// 检验复制、删除及一次撤销共用完整字素范围，覆盖方向和常驻/虚拟投影，避免高亮与写入分歧。
fn assert_cross_block_unicode_edit_boundaries(leading: &str, trailing: &str, offset: usize) {
    for virtualized in [false, true] {
        for reversed in [false, true] {
            let mut cx = TestAppContext::single();
            init_editor_test_app(&mut cx);
            let source = format!("{leading}a\n\n{trailing}b");
            let selected = format!("{leading}a\n\n{trailing}");
            let editor = cx.new(|cx| {
                if virtualized {
                    Editor::from_markdown_virtualized(cx, source.clone(), None)
                } else {
                    Editor::from_markdown(cx, source.clone(), None)
                }
            });
            editor.update(&mut cx, |editor, cx| {
                if reversed {
                    set_selection(editor, 1, offset, 0, offset, cx);
                } else {
                    set_selection(editor, 0, offset, 1, offset, cx);
                }
                assert_eq!(
                    editor.cross_block_selected_markdown(cx).as_deref(),
                    Some(selected.as_str())
                );
                assert!(editor.delete_cross_block_selection(cx));
                assert_eq!(editor.source_document.text(), "b");
                editor.undo_document(cx);
                assert_eq!(editor.source_document.text(), source);
                assert_eq!(
                    editor.cross_block_selected_markdown(cx).as_deref(),
                    Some(selected.as_str())
                );
            });
            cx.quit();
        }
    }
}

/// 平台或命中投影产生汉字内部字节时，删除应与复制同样覆盖完整字符。
#[test]
fn cross_block_unicode_endpoints_expand_to_complete_characters() {
    assert_cross_block_unicode_edit_boundaries("中", "尾", 1);
}

/// 有效 UTF-8 字节位置仍可能拆开组合音标、旗帜或家庭 emoji；完整字素才是编辑边界。
#[test]
fn cross_block_grapheme_endpoints_expand_to_complete_clusters() {
    for (leading, trailing, offset) in [
        ("e\u{301}", "a\u{301}", 1),
        ("🇨🇳", "🇯🇵", 4),
        ("👨‍👩‍👧‍👦", "👩‍👩‍👧‍👧", 4),
    ] {
        assert_cross_block_unicode_edit_boundaries(leading, trailing, offset);
    }
}
