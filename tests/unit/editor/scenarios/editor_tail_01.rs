// @author kongweiguang

#[gpui::test]
async fn code_language_menu_supports_keyboard_selection_and_undo(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let (editor, visual_cx) = cx.add_window_view(|_window, cx| {
        Editor::from_markdown(cx, "```rust\nlet x = 1;\n```".to_owned(), None)
    });
    redraw(visual_cx);
    let button = visual_cx
        .debug_bounds("code-language-menu-button")
        .expect("language menu button");
    visual_cx.simulate_mouse_move(button.center(), None, Modifiers::default());
    visual_cx
        .executor()
        .advance_clock(Duration::from_millis(520));
    visual_cx.run_until_parked();
    redraw(visual_cx);
    assert!(visual_cx.debug_bounds("ui-tooltip").is_some());
    visual_cx.simulate_click(button.center(), Modifiers::default());
    redraw(visual_cx);
    editor.update_in(visual_cx, |editor, window, cx| {
        let block = editor.document.first_root().expect("code block").clone();
        assert!(block.read(cx).code_language_focus_handle.is_focused(window));
    });
    for viewport in [size(px(720.0), px(520.0)), size(px(1180.0), px(780.0))] {
        visual_cx.simulate_resize(viewport);
        redraw(visual_cx);
        visual_cx.update(|window, _cx| assert_eq!(window.scale_factor(), 2.0));
        let menu = visual_cx.debug_bounds("code-language-menu").unwrap();
        let content = visual_cx.debug_bounds("editor-content").unwrap();
        assert!(menu.left() >= content.left());
        assert!(menu.right() <= content.right());
        assert!(menu.top() >= content.top());
        assert!(menu.bottom() <= content.bottom());
    }

    let down = KeyDownEvent {
        keystroke: Keystroke::parse("down").expect("valid down keystroke"),
        is_held: false,
    };
    let enter = KeyDownEvent {
        keystroke: Keystroke::parse("enter").expect("valid enter keystroke"),
        is_held: false,
    };
    editor.update_in(visual_cx, |editor, window, cx| {
        editor.on_editor_key_down_capture(&down, window, cx);
        editor.on_editor_key_down_capture(&enter, window, cx);
    });
    redraw(visual_cx);
    editor.update(visual_cx, |editor, cx| {
        let block = editor.document.first_root().expect("code block").clone();
        assert_eq!(block.read(cx).code_language_text(), "javascript");
        assert_eq!(
            editor.source_document.text(),
            "```javascript\nlet x = 1;\n```"
        );
        editor.undo_document(cx);
        assert_eq!(editor.source_document.text(), "```rust\nlet x = 1;\n```");
    });
    editor.update_in(visual_cx, |editor, window, cx| {
        let block = editor.document.first_root().expect("code block").clone();
        let handled = block.update(cx, |block, block_cx| {
            block.handle_code_language_menu_key(&down, window, block_cx)
        });
        assert!(!handled, "closed language menu must not consume Down");
    });
}

#[gpui::test]
async fn down_from_code_content_focuses_language_input(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let (editor, cx) = cx.add_window_view(|_window, cx| {
        Editor::from_markdown(cx, "```rust\nab\n```".to_string(), None)
    });

    // Settle focus on the code content first (and clear any pending focus that a
    // later redraw would otherwise re-apply and steal back).
    editor.update_in(cx, |editor, _window, _cx| {
        let block = editor.document.visible_blocks()[0].entity.clone();
        editor.focus_block(block.entity_id());
    });
    redraw(cx);

    editor.update_in(cx, |editor, window, cx| {
        let block = editor.document.visible_blocks()[0].entity.clone();
        block.update(cx, |block, block_cx| {
            block.move_to(block.visible_len(), block_cx);
            block.on_focus_next(&FocusNext, window, block_cx);
        });
        assert!(
            block.read(cx).code_language_focus_handle.is_focused(window),
            "Down from the last code line should focus the language field"
        );
    });
}

#[gpui::test]
async fn down_from_code_language_at_document_end_creates_trailing_paragraph(
    cx: &mut TestAppContext,
) {
    init_editor_test_app(cx);
    let (editor, cx) = cx.add_window_view(|_window, cx| {
        Editor::from_markdown(cx, "```rust\nab\n```".to_string(), None)
    });

    editor.update_in(cx, |editor, window, cx| {
        let block = editor.document.visible_blocks()[0].entity.clone();
        editor.focus_block(block.entity_id());
        block.update(cx, |block, block_cx| {
            block.code_language_focus_handle.focus(window);
            block.on_code_language_focus_next(&FocusNext, window, block_cx);
        });
    });
    redraw(cx);

    editor.update(cx, |editor, cx| {
        let roots = editor.document.root_blocks();
        assert_eq!(roots.len(), 2, "a trailing paragraph should be created");
        assert_eq!(roots[1].read(cx).kind(), BlockKind::Paragraph);
        assert_eq!(roots[1].read(cx).display_text(), "");
    });
}

#[gpui::test]
async fn enter_in_code_language_does_not_exit_block(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let (editor, cx) = cx.add_window_view(|_window, cx| {
        Editor::from_markdown(cx, "```rust\nab\n```".to_string(), None)
    });

    editor.update_in(cx, |editor, window, cx| {
        let block = editor.document.visible_blocks()[0].entity.clone();
        editor.focus_block(block.entity_id());
        block.update(cx, |block, block_cx| {
            block.code_language_focus_handle.focus(window);
            block.on_code_language_newline(&Newline, window, block_cx);
        });
    });
    redraw(cx);

    editor.update(cx, |editor, _cx| {
        // Enter must not leave the block, so no trailing paragraph appears.
        assert_eq!(editor.document.root_count(), 1);
    });
}

#[gpui::test]
async fn captured_tab_key_does_not_modify_code_language_input(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let (editor, cx) = cx.add_window_view(|_window, cx| {
        Editor::from_markdown(cx, "```rust\nab\n```".to_string(), None)
    });

    editor.update_in(cx, |editor, window, cx| {
        let block = editor.document.visible_blocks()[0].entity.clone();
        editor.focus_block(block.entity_id());
        block.update(cx, |block, block_cx| {
            block.move_to(1, block_cx);
        });
        block.update(cx, |block, _cx| {
            block.code_language_focus_handle.focus(window);
        });
    });
    redraw(cx);

    editor.update_in(cx, |editor, window, cx| {
        let block = editor.document.visible_blocks()[0].entity.clone();
        block.update(cx, |block, _cx| {
            block.code_language_focus_handle.focus(window);
        });
    });

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
        let block = block.read(cx);
        assert_eq!(block.code_language_text(), "rust");
        assert_eq!(block.display_text(), "ab");
    });
}

#[gpui::test]
async fn tab_key_keeps_list_indent_semantics(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let (editor, cx) =
        cx.add_window_view(|_window, cx| Editor::from_markdown(cx, "- a\n- b".to_string(), None));

    editor.update(cx, |editor, cx| {
        let second = editor.document.visible_blocks()[1].entity.clone();
        editor.focus_block(second.entity_id());
        second.update(cx, |block, block_cx| {
            block.move_to(block.visible_len(), block_cx);
        });
    });
    redraw(cx);

    cx.simulate_keystrokes("tab");
    redraw(cx);

    editor.update(cx, |editor, cx| {
        let visible = editor.document.visible_blocks();
        assert_eq!(visible.len(), 2);
        assert_eq!(visible[1].entity.read(cx).render_depth, 1);
        assert_eq!(editor.document.markdown_text(cx), "- a\n  - b");
    });
}

#[gpui::test]
async fn tab_key_keeps_table_cell_navigation(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let markdown = ["| A | B |", "| --- | --- |", "| 1 | 2 |"].join("\n");
    let (editor, cx) =
        cx.add_window_view(move |_window, cx| Editor::from_markdown(cx, markdown, None));

    let second_cell_id = editor.update(cx, |editor, cx| {
        let table = editor.document.first_root().expect("table root").clone();
        let runtime = table
            .read(cx)
            .table_runtime
            .as_ref()
            .expect("table runtime")
            .clone();
        let first = runtime.rows[0][0].clone();
        let second = runtime.rows[0][1].clone();
        editor.focus_block(first.entity_id());
        first.update(cx, |block, block_cx| {
            block.move_to(block.visible_len(), block_cx);
        });
        second.entity_id()
    });
    redraw(cx);

    cx.simulate_keystrokes("tab");
    redraw(cx);

    editor.update(cx, |editor, _cx| {
        assert_eq!(editor.active_entity_id, Some(second_cell_id));
    });
}

#[gpui::test]
async fn right_arrow_at_cell_end_moves_to_next_cell(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let markdown = ["| A | B |", "| --- | --- |", "| 1 | 2 |"].join("\n");
    let (editor, cx) =
        cx.add_window_view(move |_window, cx| Editor::from_markdown(cx, markdown, None));

    let second_cell_id = editor.update(cx, |editor, cx| {
        let table = editor.document.first_root().expect("table root").clone();
        let runtime = table
            .read(cx)
            .table_runtime
            .as_ref()
            .expect("table runtime")
            .clone();
        let first = runtime.rows[0][0].clone();
        let second = runtime.rows[0][1].clone();
        editor.focus_block(first.entity_id());
        first.update(cx, |block, block_cx| {
            block.move_to(block.visible_len(), block_cx);
        });
        second.entity_id()
    });
    redraw(cx);

    cx.simulate_keystrokes("right");
    redraw(cx);

    editor.update(cx, |editor, _cx| {
        assert_eq!(editor.active_entity_id, Some(second_cell_id));
    });
}

#[gpui::test]
async fn left_arrow_at_cell_start_moves_to_previous_cell(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let markdown = ["| A | B |", "| --- | --- |", "| 1 | 2 |"].join("\n");
    let (editor, cx) =
        cx.add_window_view(move |_window, cx| Editor::from_markdown(cx, markdown, None));

    let first_cell_id = editor.update(cx, |editor, cx| {
        let table = editor.document.first_root().expect("table root").clone();
        let runtime = table
            .read(cx)
            .table_runtime
            .as_ref()
            .expect("table runtime")
            .clone();
        let first = runtime.rows[0][0].clone();
        let second = runtime.rows[0][1].clone();
        editor.focus_block(second.entity_id());
        second.update(cx, |block, block_cx| {
            block.move_to(0, block_cx);
        });
        first.entity_id()
    });
    redraw(cx);

    cx.simulate_keystrokes("left");
    redraw(cx);

    editor.update(cx, |editor, _cx| {
        assert_eq!(editor.active_entity_id, Some(first_cell_id));
    });
}

#[gpui::test]
async fn inserting_table_at_document_end_adds_trailing_paragraph(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let editor = cx.new(|cx| Editor::from_markdown(cx, String::new(), None));

    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.table_insert_dialog = Some(super::context_menu::TableInsertDialogState {
                target: super::context_menu::TableInsertTarget::Append,
                body_rows: 2,
                columns: 2,
            });
            editor.on_confirm_table_insert_dialog(&ClickEvent::default(), window, cx);
        });
    });

    editor.update(cx, |editor, cx| {
        let roots = editor.document.visible_blocks();
        let kinds = roots
            .iter()
            .map(|visible| visible.entity.read(cx).kind())
            .collect::<Vec<_>>();
        let table_index = kinds
            .iter()
            .position(|kind| *kind == BlockKind::Table)
            .expect("table inserted");
        // The table is the last meaningful block, so an empty paragraph is
        // appended after it to give the caret somewhere to land.
        assert_eq!(kinds.get(table_index + 1), Some(&BlockKind::Paragraph));
        assert_eq!(table_index + 1, kinds.len() - 1);
        assert_eq!(roots[table_index + 1].entity.read(cx).display_text(), "");
    });
}

#[gpui::test]
async fn clicking_inserted_table_cell_focuses_it_for_text_input(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let (editor, visual) =
        cx.add_window_view(|_window, cx| Editor::from_markdown(cx, String::new(), None));

    visual.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.table_insert_dialog = Some(super::context_menu::TableInsertDialogState {
                target: super::context_menu::TableInsertTarget::Append,
                body_rows: 2,
                columns: 2,
            });
            editor.on_confirm_table_insert_dialog(&ClickEvent::default(), window, cx);
        });
    });
    redraw(visual);

    let target_cell = editor.read_with(visual, |editor, cx| {
        editor.document.visible_blocks()[0]
            .entity
            .read(cx)
            .table_runtime
            .as_ref()
            .expect("inserted table runtime")
            .rows[1][1]
            .clone()
    });
    let table = visual.debug_bounds("table-surface").expect("table surface");
    let target = point(
        table.left() + table.size.width * 0.75,
        table.top() + table.size.height * (5.0 / 6.0),
    );
    visual.simulate_mouse_down(target, MouseButton::Left, Modifiers::default());
    visual.simulate_mouse_up(target, MouseButton::Left, Modifiers::default());
    redraw(visual);

    editor.read_with(visual, |editor, _cx| {
        assert_eq!(editor.active_entity_id, Some(target_cell.entity_id()));
    });
    visual.simulate_keystrokes("x");
    redraw(visual);
    target_cell.read_with(visual, |cell, _cx| {
        assert_eq!(cell.display_text(), "x");
    });
}

#[gpui::test]
async fn ctrl_enter_exits_focused_math_block(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let (editor, cx) =
        cx.add_window_view(|_window, cx| Editor::from_markdown(cx, "$$n^2$$".to_string(), None));

    editor.update(cx, |editor, cx| {
        let block = editor.document.visible_blocks()[0].entity.clone();
        editor.focus_block(block.entity_id());
        block.update(cx, |block, block_cx| {
            block.move_to(block.visible_len(), block_cx);
        });
    });
    redraw(cx);

    cx.simulate_keystrokes("ctrl-enter");
    redraw(cx);

    editor.update(cx, |editor, cx| {
        let visible = editor.document.visible_blocks();
        assert_eq!(visible.len(), 2);
        assert_eq!(visible[0].entity.read(cx).kind(), BlockKind::MathBlock);
        assert_eq!(visible[0].entity.read(cx).display_text(), "$$n^2$$");
        assert_eq!(visible[1].entity.read(cx).kind(), BlockKind::Paragraph);
        assert_eq!(visible[1].entity.read(cx).display_text(), "");
        assert_eq!(editor.document.markdown_text(cx), "$$n^2$$\n\n");
    });
}

include!("editor_tail_01/complex_render_recovery.rs");

#[gpui::test]
async fn ctrl_enter_exits_focused_table_cell(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let markdown = ["| A | B |", "| --- | --- |", "| 1 | 2 |"].join("\n");
    let (editor, cx) =
        cx.add_window_view(move |_window, cx| Editor::from_markdown(cx, markdown, None));

    editor.update(cx, |editor, cx| {
        let table = editor.document.first_root().expect("table root").clone();
        let cell = table
            .read(cx)
            .table_runtime
            .as_ref()
            .expect("table runtime")
            .rows[0][0]
            .clone();
        editor.focus_block(cell.entity_id());
        cell.update(cx, |block, block_cx| {
            block.move_to(block.visible_len(), block_cx);
        });
    });
    redraw(cx);

    cx.simulate_keystrokes("ctrl-enter");
    redraw(cx);

    editor.update(cx, |editor, cx| {
        let visible = editor.document.visible_blocks();
        assert_eq!(visible.len(), 2);
        assert_eq!(visible[0].entity.read(cx).kind(), BlockKind::Table);
        assert_eq!(visible[1].entity.read(cx).kind(), BlockKind::Paragraph);
        assert_eq!(visible[1].entity.read(cx).display_text(), "");
        assert_eq!(editor.active_entity_id, Some(visible[1].entity.entity_id()));
    });
}

#[gpui::test]
async fn ending_editor_pointer_selection_sessions_keeps_normal_selection(cx: &mut TestAppContext) {
    let editor =
        cx.new(|cx| Editor::from_markdown(cx, "```rust\nfn main() {}\n```".to_string(), None));

    editor.update(cx, |editor, cx| {
        let target = editor.document.visible_blocks()[0].entity.clone();
        target.update(cx, |block, _cx| {
            block.selected_range = 3..7;
            block.marked_range = Some(4..6);
            block.is_selecting = true;
        });
        editor.active_entity_id = Some(target.entity_id());

        assert!(editor.end_block_pointer_selection_sessions(cx));
        target.read_with(cx, |block, _cx| {
            assert!(!block.is_selecting);
            assert_eq!(block.selected_range, 3..7);
            assert_eq!(block.marked_range, Some(4..6));
        });

        assert!(!editor.end_block_pointer_selection_sessions(cx));
    });
}

#[gpui::test]
async fn toggle_view_mode_preserves_table_cell_position(cx: &mut TestAppContext) {
    let markdown = ["| Name | Value |", "| --- | --- |", "| alpha | beta |"].join("\n");
    let editor = cx.new(|cx| Editor::from_markdown(cx, markdown, None));

    editor.update(cx, |editor, cx| {
        let table = editor.document.first_root().expect("table root").clone();
        let cell = table
            .read(cx)
            .table_runtime
            .as_ref()
            .expect("table runtime")
            .rows[0][1]
            .clone();
        cell.update(cx, |block, _cx| {
            block.selected_range = 2..2;
        });
        editor.active_entity_id = Some(cell.entity_id());

        editor.toggle_view_mode(cx);
        assert!(matches!(editor.view_mode, ViewMode::Source));

        editor.toggle_view_mode(cx);
        assert!(matches!(editor.view_mode, ViewMode::Rendered));
        let restored_table = editor
            .document
            .first_root()
            .expect("restored table")
            .clone();
        let restored_cell = restored_table
            .read(cx)
            .table_runtime
            .as_ref()
            .expect("restored runtime")
            .rows[0][1]
            .clone();
        assert_eq!(restored_cell.read(cx).display_text(), "beta");
        assert_eq!(restored_cell.read(cx).selected_range, 2..2);
        assert_eq!(editor.pending_focus, Some(restored_cell.entity_id()));
    });
}

#[gpui::test]
async fn toggle_view_mode_preserves_callout_table_cell_position(cx: &mut TestAppContext) {
    let markdown = [
        "> [!NOTE]",
        "> | Name | Value |",
        "> | --- | --- |",
        "> | alpha | beta |",
    ]
    .join("\n");
    let editor = cx.new(|cx| Editor::from_markdown(cx, markdown, None));

    editor.update(cx, |editor, cx| {
        let callout = editor.document.first_root().expect("callout root").clone();
        let table = callout
            .read(cx)
            .children
            .iter()
            .find(|child| child.read(cx).kind() == BlockKind::Table)
            .expect("nested table child")
            .clone();
        let cell = table
            .read(cx)
            .table_runtime
            .as_ref()
            .expect("table runtime")
            .rows[0][1]
            .clone();
        cell.update(cx, |block, _cx| {
            block.selected_range = 2..2;
        });
        editor.active_entity_id = Some(cell.entity_id());

        editor.toggle_view_mode(cx);
        assert!(matches!(editor.view_mode, ViewMode::Source));

        editor.toggle_view_mode(cx);
        assert!(matches!(editor.view_mode, ViewMode::Rendered));
        let restored_callout = editor
            .document
            .first_root()
            .expect("restored callout")
            .clone();
        let restored_table = restored_callout
            .read(cx)
            .children
            .iter()
            .find(|child| child.read(cx).kind() == BlockKind::Table)
            .expect("restored nested table")
            .clone();
        let restored_cell = restored_table
            .read(cx)
            .table_runtime
            .as_ref()
            .expect("restored runtime")
            .rows[0][1]
            .clone();
        assert_eq!(restored_cell.read(cx).display_text(), "beta");
        assert_eq!(restored_cell.read(cx).selected_range, 2..2);
        assert_eq!(editor.pending_focus, Some(restored_cell.entity_id()));
    });
}
