// @author kongweiguang

#[gpui::test]
async fn ctrl_a_selects_entire_source_document_in_source_mode(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let (editor, cx) = cx.add_window_view(|_window, cx| {
        Editor::from_markdown(cx, "alpha\n\nbeta".to_string(), None)
    });

    editor.update(cx, |editor, cx| {
        editor.toggle_view_mode(cx);
        assert!(matches!(editor.view_mode, ViewMode::Source));
        let source = editor.document.visible_blocks()[0].entity.clone();
        editor.focus_block(source.entity_id());
        source.update(cx, |block, _cx| {
            block.selected_range = 1..3;
        });
    });
    redraw(cx);

    cx.simulate_keystrokes("ctrl-a");
    redraw(cx);

    editor.read_with(cx, |editor, cx| {
        let source = editor.document.visible_blocks()[0].entity.read(cx);
        assert_eq!(source.selected_range, 0..source.visible_len());
        assert!(editor.cross_block_selection.is_none());
    });
}

/// Source 编辑历史通过当前平台的 Redo 默认键验收，确保键位契约变化后仍走同一事务历史。
#[gpui::test]
async fn source_mode_keyboard_copy_cut_paste_and_history_match_text_editor(
    cx: &mut TestAppContext,
) {
    init_editor_test_app(cx);
    let (editor, visual) =
        cx.add_window_view(|_window, cx| Editor::from_markdown(cx, "alpha beta".to_string(), None));

    editor.update(visual, |editor, cx| {
        editor.set_view_mode(ViewMode::Source, cx);
        let source = editor.document.visible_blocks()[0].entity.clone();
        editor.focus_block(source.entity_id());
        source.update(cx, |block, _cx| block.selected_range = 0..5);
    });
    redraw(visual);

    visual.simulate_keystrokes("ctrl-c");
    assert_eq!(
        visual.read_from_clipboard().and_then(|item| item.text()),
        Some("alpha".to_owned())
    );

    visual.simulate_keystrokes("ctrl-x");
    redraw(visual);
    editor.read_with(visual, |editor, cx| {
        assert_eq!(editor.document.raw_source_text(cx), " beta");
        assert_eq!(editor.source_document.text(), " beta");
    });

    visual.write_to_clipboard(gpui::ClipboardItem::new_string("gamma\nline".to_owned()));
    visual.simulate_keystrokes("ctrl-v");
    redraw(visual);
    editor.read_with(visual, |editor, cx| {
        assert_eq!(editor.document.raw_source_text(cx), "gamma\nline beta");
        assert_eq!(editor.source_document.text(), "gamma\nline beta");
    });

    visual.simulate_keystrokes("ctrl-z");
    redraw(visual);
    editor.read_with(visual, |editor, cx| {
        assert_eq!(editor.document.raw_source_text(cx), " beta");
        assert_eq!(editor.source_document.text(), " beta");
    });

    let redo_shortcut = if cfg!(target_os = "macos") {
        "cmd-shift-z"
    } else {
        "ctrl-shift-z"
    };
    visual.simulate_keystrokes(redo_shortcut);
    redraw(visual);
    editor.read_with(visual, |editor, cx| {
        assert_eq!(editor.document.raw_source_text(cx), "gamma\nline beta");
        assert_eq!(editor.source_document.text(), "gamma\nline beta");
    });
}

#[gpui::test]
async fn ctrl_a_selects_only_focused_block_text_in_rendered_mode(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let (editor, cx) = cx.add_window_view(|_window, cx| {
        Editor::from_markdown(cx, "alpha\n\nbeta".to_string(), None)
    });

    editor.update(cx, |editor, cx| {
        let block = editor.document.visible_blocks()[1].entity.clone();
        editor.focus_block(block.entity_id());
        block.update(cx, |block, _cx| {
            block.selected_range = 1..1;
        });
    });
    redraw(cx);

    cx.simulate_keystrokes("ctrl-a");
    redraw(cx);

    editor.read_with(cx, |editor, cx| {
        let first = editor.document.visible_blocks()[0].entity.read(cx);
        let second = editor.document.visible_blocks()[1].entity.read(cx);
        assert_eq!(first.selected_range, 0..0);
        assert_eq!(second.selected_range, 0..second.visible_len());
        assert!(editor.cross_block_selection.is_none());
    });
}

#[gpui::test]
async fn repeated_ctrl_a_selects_all_rendered_blocks(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let markdown =
        "alpha\n\n| a | b |\n| --- | --- |\n| 1 | 2 |\n\n```rust\nfn main() {}\n```\n\ngamma";
    let (editor, cx) = cx.add_window_view({
        let markdown = markdown.to_string();
        move |_window, cx| Editor::from_markdown(cx, markdown.clone(), None)
    });

    editor.update(cx, |editor, cx| {
        let block = editor.document.visible_blocks()[0].entity.clone();
        editor.focus_block(block.entity_id());
        block.update(cx, |block, block_cx| {
            block.move_to(0, block_cx);
        });
    });
    redraw(cx);

    cx.simulate_keystrokes("ctrl-a");
    redraw(cx);

    editor.read_with(cx, |editor, cx| {
        let first = editor.document.visible_blocks()[0].entity.read(cx);
        assert_eq!(first.selected_range, 0..first.visible_len());
        assert!(editor.cross_block_selection.is_none());
    });

    cx.simulate_keystrokes("ctrl-a");
    redraw(cx);

    editor.read_with(cx, |editor, cx| {
        let visible = editor.document.visible_blocks();
        let first_id = visible[0].entity.entity_id();
        let last = visible.last().expect("visible blocks");
        let last_id = last.entity.entity_id();
        let last_len = last.entity.read(cx).visible_len();
        let selection = editor
            .cross_block_selection
            .expect("second Ctrl+A should select the rendered document");
        assert_eq!(selection.anchor.entity_id, first_id);
        assert_eq!(selection.anchor.offset, 0);
        assert_eq!(selection.focus.entity_id, last_id);
        assert_eq!(selection.focus.offset, last_len);
        for visible in visible {
            let block = visible.entity.read(cx);
            let len = block.visible_len();
            if len > 0 {
                assert_eq!(block.editor_selection_range, Some(0..len));
            }
        }
    });

    let selected_after_second = editor.read_with(cx, |editor, _cx| editor.cross_block_selection);
    cx.simulate_keystrokes("ctrl-a");
    redraw(cx);

    editor.read_with(cx, |editor, cx| {
        assert_eq!(
            editor.cross_block_selection, selected_after_second,
            "third Ctrl+A should keep the full rendered document selected"
        );
        for visible in editor.document.visible_blocks() {
            let block = visible.entity.read(cx);
            let len = block.visible_len();
            if len > 0 {
                assert_eq!(block.editor_selection_range, Some(0..len));
            }
        }
    });
}

#[gpui::test]
async fn rendered_ctrl_a_cycle_expires_before_second_press(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let (editor, cx) = cx.add_window_view(|_window, cx| {
        Editor::from_markdown(cx, "alpha\n\nbeta".to_string(), None)
    });

    editor.update(cx, |editor, cx| {
        let block = editor.document.visible_blocks()[1].entity.clone();
        editor.focus_block(block.entity_id());
        block.update(cx, |block, block_cx| {
            block.move_to(1, block_cx);
        });
    });
    redraw(cx);

    cx.simulate_keystrokes("ctrl-a");
    redraw(cx);

    editor.update(cx, |editor, cx| {
        let block = editor.document.visible_blocks()[1].entity.clone();
        block.update(cx, |block, _cx| {
            block.selected_range = 1..1;
        });
        let cycle = editor
            .rendered_select_all_cycle
            .as_mut()
            .expect("first Ctrl+A should arm the rendered select-all cycle");
        cycle.last_pressed_at =
            Instant::now() - (Editor::RENDERED_SELECT_ALL_CYCLE_WINDOW + Duration::from_millis(1));
    });

    cx.simulate_keystrokes("ctrl-a");
    redraw(cx);

    editor.read_with(cx, |editor, cx| {
        let second = editor.document.visible_blocks()[1].entity.read(cx);
        assert_eq!(second.selected_range, 0..second.visible_len());
        assert!(editor.cross_block_selection.is_none());
        assert_eq!(
            editor
                .rendered_select_all_cycle
                .expect("cycle should be reset by expired second press")
                .count,
            1
        );
    });
}

#[gpui::test]
async fn tab_key_inserts_tab_in_focused_paragraph(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let (editor, cx) =
        cx.add_window_view(|_window, cx| Editor::from_markdown(cx, "ab".to_string(), None));

    editor.update(cx, |editor, cx| {
        let block = editor.document.visible_blocks()[0].entity.clone();
        editor.focus_block(block.entity_id());
        block.update(cx, |block, block_cx| {
            block.move_to(1, block_cx);
        });
    });
    redraw(cx);

    cx.simulate_keystrokes("tab");
    redraw(cx);

    editor.update(cx, |editor, cx| {
        let block = editor.document.visible_blocks()[0].entity.clone();
        assert_eq!(block.read(cx).display_text(), "a    b");
        assert_eq!(editor.document.markdown_text(cx), "a    b");
    });
}

#[gpui::test]
async fn tab_key_inserts_tab_in_focused_code_block(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let (editor, cx) = cx.add_window_view(|_window, cx| {
        Editor::from_markdown(cx, "```rust\nab\n```".to_string(), None)
    });

    editor.update(cx, |editor, cx| {
        let block = editor.document.visible_blocks()[0].entity.clone();
        editor.focus_block(block.entity_id());
        block.update(cx, |block, block_cx| {
            block.move_to(1, block_cx);
        });
    });
    redraw(cx);

    cx.simulate_keystrokes("tab");
    redraw(cx);

    editor.update(cx, |editor, cx| {
        let block = editor.document.visible_blocks()[0].entity.clone();
        assert_eq!(block.read(cx).display_text(), "a    b");
        assert_eq!(editor.document.markdown_text(cx), "```rust\na    b\n```");
    });
}

#[gpui::test]
async fn captured_tab_key_inserts_visible_indent_in_paragraph(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let (editor, cx) =
        cx.add_window_view(|_window, cx| Editor::from_markdown(cx, "ab".to_string(), None));

    editor.update(cx, |editor, cx| {
        let block = editor.document.visible_blocks()[0].entity.clone();
        editor.focus_block(block.entity_id());
        block.update(cx, |block, block_cx| {
            block.move_to(1, block_cx);
        });
    });
    redraw(cx);

    let event = KeyDownEvent {
        keystroke: Keystroke::parse("tab").expect("valid tab keystroke"),
        is_held: false,
    };
    editor.update_in(cx, |editor, window, cx| {
        editor.on_editor_key_down_capture(&event, window, cx);
    });
    redraw(cx);

    editor.update(cx, |editor, cx| {
        let block = editor.document.visible_blocks()[0].entity.clone();
        assert_eq!(block.read(cx).display_text(), "a    b");
    });
}
