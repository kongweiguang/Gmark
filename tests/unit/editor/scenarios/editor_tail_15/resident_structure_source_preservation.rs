// @author kongweiguang

use super::*;

/// 结构编辑后紧接普通输入，不等待后台解析；只校验未编辑首尾与两次独立撤销的用户结果。
fn structure_then_type_preserves_source(
    cx: &mut TestAppContext,
    key: &str,
    target: &str,
    caret: usize,
) {
    init_editor_test_app(cx);
    let path = temp_markdown_path(&format!("resident-structure-{key}-source"));
    let original = "before __untouched__ alpha_beta\r\n\r\nalpha omega\r\n\r\ndelta_gamma __middle__\r\n\r\nafter ~~untouched~~ alpha_beta\n\n| A | B |\n|---|---|\n| 一 | 🙂 |";
    let prefix = "before __untouched__ alpha_beta\r\n\r\n";
    let suffix = "\r\n\r\nafter ~~untouched~~ alpha_beta\n\n| A | B |\n|---|---|\n| 一 | 🙂 |";
    std::fs::write(&path, original).unwrap();
    let editor_path = path.clone();
    let (editor, visual) = cx.add_window_view(move |_window, cx| {
        Editor::from_markdown(cx, original.to_owned(), Some(editor_path))
    });
    redraw(visual);
    editor.update_in(visual, |editor, window, cx| {
        let block = editor
            .document
            .root_blocks()
            .iter()
            .find(|block| block.read(cx).display_text() == target)
            .expect("target paragraph")
            .clone();
        editor.focus_block(block.entity_id());
        block.update(cx, |block, cx| {
            block.selected_range = caret..caret;
            block.selection_reversed = false;
            block.focus_handle.focus(window);
            cx.notify();
        });
    });
    redraw(visual);
    visual.simulate_keystrokes(key);
    redraw(visual);
    let after_structure = editor.read_with(visual, |editor, _cx| editor.source_document.text());
    assert_ne!(after_structure, original.replace("\r\n", "\n"));
    visual.simulate_input("7");
    redraw(visual);
    editor.read_with(visual, |editor, _cx| {
        assert!(editor.document.source_commit_error().is_none());
        assert!(editor.source_document.text().contains('7'));
    });
    assert!(visual.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.save_to_existing_path(&path, window, cx)
        })
    }));
    let saved = std::fs::read(&path).unwrap();
    assert!(saved.starts_with(prefix.as_bytes()));
    assert!(saved.ends_with(suffix.as_bytes()));
    if key == "enter" {
        assert!(
            String::from_utf8(saved)
                .unwrap()
                .contains("delta_gamma __middle__\r\n\r\n")
        );
    }
    visual.simulate_keystrokes("ctrl-z");
    redraw(visual);
    editor.read_with(visual, |editor, _cx| {
        assert_eq!(editor.source_document.text(), after_structure);
    });
    visual.simulate_keystrokes("ctrl-z");
    redraw(visual);
    assert!(visual.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.save_to_existing_path(&path, window, cx)
        })
    }));
    assert_eq!(std::fs::read(&path).unwrap(), original.as_bytes());
    let _ = std::fs::remove_file(path);
}

/// Enter 拆出的新根必须立即取得原源码归属，不能把后续文字留在无法保存的投影中。
#[gpui::test]
async fn resident_structure_newline_then_input_preserves_neighbor_source(cx: &mut TestAppContext) {
    structure_then_type_preserves_source(cx, "enter", "alpha omega", 5);
}

/// 行末新增空段只拥有插入间隙，相邻段落的拼写与原有分隔不能变成新增根的写入范围。
#[gpui::test]
async fn resident_structure_insert_between_regions_preserves_neighbor_source(
    cx: &mut TestAppContext,
) {
    structure_then_type_preserves_source(cx, "enter", "alpha omega", "alpha omega".len());
}

/// 合并会移走根槽位；紧随输入与一次撤销必须使用更新后的同一文档范围。
#[gpui::test]
async fn resident_structure_merge_then_input_preserves_neighbor_source(cx: &mut TestAppContext) {
    structure_then_type_preserves_source(cx, "backspace", "delta_gamma middle", 0);
}

/// 引用快捷转换会重解析树，但不能把首尾原源码作为派生投影一同写回。
#[gpui::test]
async fn resident_structure_quote_reparse_preserves_neighbor_source(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("quote-source-preservation.md");
    let prefix = "before __untouched__ alpha_beta\r\n\r\n";
    let suffix = "\r\n\r\nafter __untouched__ alpha_beta";
    let original = format!("{prefix}alpha omega{suffix}");
    std::fs::write(&path, original.as_bytes()).unwrap();
    let editor_path = path.clone();
    let (editor, visual) = cx
        .add_window_view(move |_window, cx| Editor::from_markdown(cx, original, Some(editor_path)));
    redraw(visual);
    editor.update_in(visual, |editor, window, cx| {
        let block = editor.document.root_blocks()[1].clone();
        editor.focus_block(block.entity_id());
        block.update(cx, |block, cx| {
            block.selected_range = 0..0;
            block.selection_reversed = false;
            block.focus_handle.focus(window);
            cx.notify();
        });
    });
    redraw(visual);
    visual.simulate_input("> ");
    redraw(visual);
    visual.simulate_input("7");
    redraw(visual);
    editor.read_with(visual, |editor, _cx| {
        assert!(editor.document.source_commit_error().is_none());
        assert!(editor.source_document.text().contains("> 7alpha omega"));
    });
    assert!(visual.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.save_to_existing_path(&path, window, cx)
        })
    }));
    let saved = std::fs::read(path).unwrap();
    assert!(saved.starts_with(prefix.as_bytes()));
    assert!(saved.ends_with(suffix.as_bytes()));
}
