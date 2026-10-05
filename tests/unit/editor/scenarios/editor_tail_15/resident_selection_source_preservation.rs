// @author kongweiguang

use super::*;

/// 从真实布局取得指定插入点，避免固定像素坐标在字体或 DPI 改变后选到相邻字素。
fn resident_selection_hit(
    block: &gpui::Entity<crate::components::Block>,
    offset: usize,
    visual: &mut VisualTestContext,
) -> gpui::Point<gpui::Pixels> {
    let bounds = block
        .read_with(visual, |block, _cx| block.last_bounds)
        .expect("rendered target bounds");
    (0..f32::from(bounds.size.width) as usize)
        .map(|x| point(bounds.left() + px(x as f32), bounds.center().y))
        .find(|position| {
            block.read_with(visual, |block, _cx| {
                block.index_for_mouse_position(*position)
            }) == offset
        })
        .expect("target caret hit")
}

/// 用实际跨块拖选和键盘命令检查原源码边界，包含未选表格、混合换行与反向选区的历史恢复。
fn assert_resident_partial_cross_block_source_preservation(
    cx: &mut TestAppContext,
    command: &str,
    reversed: bool,
) {
    init_editor_test_app(cx);
    let original = "before __untouched__ alpha_beta\r\n\r\nfirst alpha\r\n\r\nsecond omega\r\n\r\n| head | other |\n|---|---|\n| cell text | 末尾🤝 |\r\n\r\nafter ~~untouched~~ alpha_beta";
    let normalized = original.replace("\r\n", "\n");
    let replacement = if matches!(command, "delete" | "ctrl-x") {
        ""
    } else {
        "替换🙂"
    };
    let first_start = original.find("first alpha").expect("first target");
    let second_end = original.find("second omega").expect("second target") + "second omega".len();
    let expected = format!(
        "{}first {replacement} omega{}",
        &original[..first_start],
        &original[second_end..]
    );
    let selected_start =
        normalized.find("first alpha").expect("source first target") + "first ".len();
    let selected_end = normalized
        .find("second omega")
        .expect("source second target")
        + "second".len();
    let path = temp_markdown_path(&format!("resident-selection-{command}-{reversed}"));
    std::fs::write(&path, original.as_bytes()).expect("original file");
    let editor_path = path.clone();
    let (editor, visual) = cx.add_window_view(move |_window, cx| {
        Editor::from_markdown(cx, original.to_owned(), Some(editor_path))
    });
    visual.simulate_resize(size(px(1180.0), px(900.0)));
    redraw(visual);
    let (first, second, initial_revision) = editor.read_with(visual, |editor, cx| {
        assert!(editor.virtual_surface.is_none());
        let target = |text| {
            editor
                .document
                .root_blocks()
                .iter()
                .find(|block| block.read(cx).record.title.visible_text() == text)
                .expect("target paragraph")
                .clone()
        };
        (
            target("first alpha"),
            target("second omega"),
            editor.source_document.revision(),
        )
    });
    let first_hit = resident_selection_hit(&first, "first ".len(), visual);
    let second_hit = resident_selection_hit(&second, "second".len(), visual);
    let (anchor, focus) = if reversed {
        (second_hit, first_hit)
    } else {
        (first_hit, second_hit)
    };
    visual.simulate_event(MouseDownEvent {
        position: anchor,
        modifiers: Modifiers::default(),
        button: MouseButton::Left,
        click_count: 1,
        first_mouse: false,
    });
    visual.simulate_mouse_move(focus, MouseButton::Left, Modifiers::default());
    visual.simulate_event(MouseUpEvent {
        position: focus,
        modifiers: Modifiers::default(),
        button: MouseButton::Left,
        click_count: 1,
    });
    redraw(visual);
    editor.read_with(visual, |editor, cx| {
        let selection = editor.capture_source_selection_snapshot(cx);
        assert_eq!(selection.range(), selected_start..selected_end);
        assert_eq!(selection.reversed(), reversed);
        assert!(editor.cross_block_selection.is_some());
        assert_eq!(
            editor.source_document.serialized_bytes(),
            original.as_bytes()
        );
        assert!(editor.undo_history.is_empty());
    });
    match command {
        "input" => {
            // 一次平台确认结果是一个 payload；simulate_input 会逐字符发送独立输入，不能代表 IME 终态。
            visual.update(|window, cx| {
                editor.update(cx, |editor, cx| {
                    let target = editor.current_edit_target_from_state(cx).unwrap();
                    target.update(cx, |block, cx| {
                        block.replace_text_in_range(None, replacement, window, cx);
                    });
                });
            });
        }
        "ctrl-v" => {
            visual.write_to_clipboard(gpui::ClipboardItem::new_string(replacement.to_owned()));
            visual.simulate_keystrokes(command);
        }
        _ => visual.simulate_keystrokes(command),
    }
    redraw(visual);
    if command == "ctrl-x" {
        assert_eq!(
            visual.read_from_clipboard().and_then(|item| item.text()),
            Some("alpha\n\nsecond".to_owned())
        );
    }
    editor.read_with(visual, |editor, cx| {
        assert_eq!(
            editor.source_document.serialized_bytes(),
            expected.as_bytes(),
            "partial edit must preserve every byte outside its source region group"
        );
        assert_eq!(
            editor.source_document.revision().get(),
            initial_revision.get() + 1
        );
        assert_eq!(editor.undo_history.len(), 1);
        let cursor = expected
            .replace("\r\n", "\n")
            .find("first ")
            .expect("merged paragraph")
            + "first ".len()
            + replacement.len();
        assert_eq!(
            editor.capture_source_selection_snapshot(cx).range(),
            cursor..cursor
        );
    });
    visual.update(|window, cx| {
        assert!(editor.update(cx, |editor, cx| {
            editor.save_to_existing_path(&path, window, cx)
        }));
    });
    assert_eq!(
        std::fs::read(&path).expect("saved file"),
        expected.as_bytes()
    );

    visual.simulate_keystrokes("ctrl-z");
    redraw(visual);
    editor.read_with(visual, |editor, cx| {
        assert_eq!(
            editor.source_document.serialized_bytes(),
            original.as_bytes()
        );
        let selection = editor.capture_source_selection_snapshot(cx);
        assert_eq!(selection.range(), selected_start..selected_end);
        assert_eq!(selection.reversed(), reversed);
        assert!(editor.undo_history.is_empty());
        assert_eq!(editor.redo_history.len(), 1);
    });
    visual.simulate_keystrokes("ctrl-c");
    assert_eq!(
        visual.read_from_clipboard().and_then(|item| item.text()),
        Some("alpha\n\nsecond".to_owned())
    );
    visual.simulate_keystrokes("ctrl-shift-z");
    redraw(visual);
    editor.read_with(visual, |editor, _cx| {
        assert_eq!(
            editor.source_document.serialized_bytes(),
            expected.as_bytes()
        );
        assert_eq!(editor.undo_history.len(), 1);
        assert!(editor.redo_history.is_empty());
    });
    let _ = std::fs::remove_file(path);
}

/// 删除两个段落之间的选区，只修改被选中的连续区域，未选根不参与规范化。
#[gpui::test]
async fn resident_partial_cross_block_delete_preserves_original_source(cx: &mut TestAppContext) {
    assert_resident_partial_cross_block_source_preservation(cx, "delete", false);
}

/// 反向剪切要保持正文原拼写和原选区方向，复制成功后再形成一个历史项。
#[gpui::test]
async fn resident_partial_cross_block_cut_preserves_original_source(cx: &mut TestAppContext) {
    assert_resident_partial_cross_block_source_preservation(cx, "ctrl-x", true);
}

/// 系统确认文字替换跨块选区时，原区域之外的粗体、下划线、表格及换行均保持原字节。
#[gpui::test]
async fn resident_partial_cross_block_input_preserves_original_source(cx: &mut TestAppContext) {
    assert_resident_partial_cross_block_source_preservation(cx, "input", false);
}

/// 粘贴与确认文字使用同一写入授权，不能从规范投影重建整份未选文档。
#[gpui::test]
async fn resident_partial_cross_block_paste_preserves_original_source(cx: &mut TestAppContext) {
    assert_resident_partial_cross_block_source_preservation(cx, "ctrl-v", true);
}

/// 长文输入仍须保留原拼写与一次撤销；宽松耗时门槛防止重新引入每键数秒的全文映射回退。
#[gpui::test]
async fn resident_long_input_preserves_source_without_full_parse(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let mut original = "alpha_beta **keep**\r\n\r\n".to_owned();
    for index in 0..1800 {
        original.push_str(&format!(
            "段落 {index} alpha_beta 中文 emoji 👨‍👩‍👧‍👦 🇨🇳 与组合音标 é __原拼写__\r\n\r\n"
        ));
    }
    let initial = original.clone();
    let (editor, visual) =
        cx.add_window_view(move |_window, cx| Editor::from_markdown(cx, initial, None));
    redraw(visual);
    editor.update_in(visual, |editor, window, cx| {
        assert!(editor.virtual_surface.is_none());
        let block = editor.document.first_root().unwrap().clone();
        editor.focus_block(block.entity_id());
        block.update(cx, |block, cx| {
            block.selected_range = 0.."alpha_beta".len();
            block.focus_handle.focus(window);
            cx.notify();
        });
    });
    redraw(visual);
    let started = std::time::Instant::now();
    visual.simulate_input("7");
    redraw(visual);
    assert!(
        started.elapsed() < std::time::Duration::from_secs(1),
        "one local input must not synchronously reparse the full document: {:?}",
        started.elapsed()
    );
    editor.read_with(visual, |editor, cx| {
        assert_eq!(
            editor.source_document.serialized_bytes(),
            original.replacen("alpha_beta", "7", 1).as_bytes()
        );
        assert_eq!(editor.capture_source_selection_snapshot(cx).range(), 1..1);
    });
    visual.simulate_keystrokes("ctrl-z");
    redraw(visual);
    editor.read_with(visual, |editor, _cx| {
        assert_eq!(editor.source_document.serialized_bytes(), original.as_bytes());
    });
}
