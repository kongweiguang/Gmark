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
/// 文档表面一次全选，局部光标状态不代替跨块全文选区。
async fn ctrl_a_selects_entire_rendered_document_from_any_focused_block(cx: &mut TestAppContext) {
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
        let selection = editor
            .cross_block_selection
            .expect("one Ctrl+A selects the document");
        assert_eq!(selection.anchor.offset, 0);
        assert_eq!(selection.focus.offset, second.visible_len());
        assert_eq!(first.editor_selection_range, Some(0..first.visible_len()));
        assert_eq!(second.editor_selection_range, Some(0..second.visible_len()));
    });
}

#[gpui::test]
/// 连续全选始终包含表格与代码，结束选区时同步清除 cell 文字镜像。
async fn repeated_ctrl_a_keeps_all_rendered_blocks_selected(cx: &mut TestAppContext) {
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
        assert_eq!(first.editor_selection_range, Some(0..first.visible_len()));
        assert!(editor.cross_block_selection.is_some());
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
            .expect("Ctrl+A keeps the rendered document selected");
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
        let table = editor
            .document
            .visible_blocks()
            .iter()
            .find(|visible| visible.entity.read(cx).kind() == BlockKind::Table)
            .expect("selected table")
            .entity
            .clone();
        let runtime = table
            .read(cx)
            .table_runtime
            .clone()
            .expect("rendered table cells");
        for cell in runtime.header.iter().chain(runtime.rows.iter().flatten()) {
            let cell = cell.read(cx);
            assert_eq!(
                cell.editor_selection_range,
                Some(0..cell.visible_len()),
                "document-wide selection highlights each table cell's text"
            );
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

    editor.update(cx, |editor, cx| editor.clear_cross_block_selection(cx));
    redraw(cx);
    editor.read_with(cx, |editor, cx| {
        let table = editor
            .document
            .visible_blocks()
            .iter()
            .find(|visible| visible.entity.read(cx).kind() == BlockKind::Table)
            .expect("selected table")
            .entity
            .clone();
        let runtime = table
            .read(cx)
            .table_runtime
            .clone()
            .expect("rendered table cells");
        for cell in runtime.header.iter().chain(runtime.rows.iter().flatten()) {
            assert_eq!(cell.read(cx).editor_selection_range, None);
        }
    });
}

#[gpui::test]
/// 表格父 block 是空文字锚点，停在该边界不能把局部选区扩成整表文字选区。
async fn partial_cross_block_table_endpoint_does_not_select_all_cells(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let markdown = "alpha\n\n| a | b |\n| --- | --- |\n| 1 | 2 |";
    let editor = cx.new(|cx| Editor::from_markdown(cx, markdown.to_owned(), None));

    editor.update(cx, |editor, cx| {
        let visible = editor.document.visible_blocks();
        let start = visible[0].entity.clone();
        let table = visible
            .iter()
            .find(|visible| visible.entity.read(cx).kind() == BlockKind::Table)
            .expect("table block")
            .entity
            .clone();
        editor.set_cross_block_selection_for_surface(
            crate::editor::selection_surface::SelectionSurface::Main,
            Some(CrossBlockSelection {
                anchor: CrossBlockSelectionEndpoint {
                    entity_id: start.entity_id(),
                    offset: start.read(cx).visible_len(),
                },
                focus: CrossBlockSelectionEndpoint {
                    entity_id: table.entity_id(),
                    offset: 0,
                },
                source_anchor: None,
                source_focus: None,
            }),
        );
        editor.sync_cross_block_selection_visuals_for_surface(
            crate::editor::selection_surface::SelectionSurface::Main,
            cx,
        );

        let runtime = table.read(cx).table_runtime.clone().expect("table cells");
        for cell in runtime.header.iter().chain(runtime.rows.iter().flatten()) {
            assert_eq!(cell.read(cx).editor_selection_range, None);
        }
    });

    editor.update(cx, |editor, cx| editor.set_view_mode(ViewMode::Split, cx));
    editor.update(cx, |editor, cx| {
        let surface = crate::editor::selection_surface::SelectionSurface::SplitPreview;
        let visible = editor.selection_surface_entities(surface);
        let start = visible.first().expect("Split Preview start block").clone();
        let table = visible
            .iter()
            .find(|visible| visible.read(cx).kind() == BlockKind::Table)
            .expect("Split Preview table")
            .clone();
        editor.set_cross_block_selection_for_surface(
            surface,
            Some(CrossBlockSelection {
                anchor: CrossBlockSelectionEndpoint {
                    entity_id: start.entity_id(),
                    offset: start.read(cx).visible_len(),
                },
                focus: CrossBlockSelectionEndpoint {
                    entity_id: table.entity_id(),
                    offset: 0,
                },
                source_anchor: None,
                source_focus: None,
            }),
        );
        editor.sync_cross_block_selection_visuals_for_surface(surface, cx);

        let runtime = table
            .read(cx)
            .table_runtime
            .clone()
            .expect("Split Preview cells");
        for cell in runtime.header.iter().chain(runtime.rows.iter().flatten()) {
            assert_eq!(cell.read(cx).editor_selection_range, None);
        }
    });
}

#[gpui::test]
/// 先完成模式切换的首帧焦点恢复，再聚焦 Split Preview，避免测试按键与待处理焦点竞争。
async fn ctrl_a_selects_table_cell_text_on_preview_surfaces(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let markdown = "alpha\n\n| a | b |\n| --- | --- |\n| 1 | 2 |\n\nomega";
    let (editor, visual) = cx.add_window_view({
        let markdown = markdown.to_string();
        move |_window, cx| Editor::from_markdown(cx, markdown.clone(), None)
    });

    editor.update_in(visual, |editor, window, cx| {
        editor.set_view_mode(ViewMode::Preview, cx);
        let first = editor.document.visible_blocks()[0].entity.clone();
        first.read(cx).focus_handle.focus(window);
    });
    redraw(visual);
    visual.simulate_keystrokes("ctrl-a");
    redraw(visual);

    editor.read_with(visual, |editor, cx| {
        let table = editor
            .document
            .visible_blocks()
            .iter()
            .find(|visible| visible.entity.read(cx).kind() == BlockKind::Table)
            .expect("Preview table")
            .entity
            .clone();
        let runtime = table
            .read(cx)
            .table_runtime
            .clone()
            .expect("Preview table cells");
        for cell in runtime.header.iter().chain(runtime.rows.iter().flatten()) {
            let cell = cell.read(cx);
            assert_eq!(cell.editor_selection_range, Some(0..cell.visible_len()));
        }
    });

    editor.update(visual, |editor, cx| {
        editor.set_view_mode(ViewMode::Split, cx);
    });
    redraw(visual);
    editor.update_in(visual, |editor, window, cx| {
        let first = editor
            .split_preview
            .as_ref()
            .expect("Split Preview")
            .document
            .visible_blocks()[0]
            .entity
            .clone();
        editor.active_selection_surface =
            crate::editor::selection_surface::SelectionSurface::SplitPreview;
        first.read(cx).focus_handle.focus(window);
        assert!(
            first.read(cx).focus_handle.is_focused(window),
            "Split Preview must own window focus before keyboard input"
        );
    });
    redraw(visual);
    visual.simulate_keystrokes("ctrl-a");
    redraw(visual);

    editor.read_with(visual, |editor, cx| {
        let selection = editor
            .split_preview_cross_block_selection
            .expect("Split Preview Ctrl+A selection");
        let revision = editor.source_document.snapshot().revision();
        assert_eq!(
            selection.source_anchor.map(|anchor| anchor.byte_offset),
            Some(0)
        );
        assert_eq!(
            selection.source_focus.map(|anchor| anchor.byte_offset),
            Some(editor.source_document.len())
        );
        assert_eq!(
            selection.source_anchor.map(|anchor| anchor.revision),
            Some(revision)
        );
        assert_eq!(
            selection.source_focus.map(|anchor| anchor.revision),
            Some(revision)
        );

        let table = editor
            .split_preview
            .as_ref()
            .expect("Split Preview")
            .document
            .visible_blocks()
            .iter()
            .find(|visible| visible.entity.read(cx).kind() == BlockKind::Table)
            .expect("Split Preview table")
            .entity
            .clone();
        let runtime = table
            .read(cx)
            .table_runtime
            .clone()
            .expect("Split Preview table cells");
        for cell in runtime.header.iter().chain(runtime.rows.iter().flatten()) {
            let cell = cell.read(cx);
            assert_eq!(cell.editor_selection_range, Some(0..cell.visible_len()));
        }
        assert!(
            editor.table_cells.is_empty(),
            "Split Source has no cell projection"
        );
    });
}

#[gpui::test]
/// 移动光标及等待后，全选仍然一次选择全文，不需要临时循环状态。
async fn rendered_ctrl_a_selects_full_document_after_cursor_move(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let (editor, cx) = cx.add_window_view(|_window, cx| {
        Editor::from_markdown(cx, "alpha\n\nbeta".to_string(), None)
    });
    redraw(cx);
    cx.simulate_keystrokes("ctrl-a");
    redraw(cx);
    editor.update(cx, |editor, cx| {
        editor.clear_cross_block_selection(cx);
        let block = editor.document.visible_blocks()[1].entity.clone();
        editor.focus_block(block.entity_id());
        block.update(cx, |block, cx| block.move_to(1, cx));
    });
    cx.executor().advance_clock(Duration::from_secs(2));
    redraw(cx);
    cx.simulate_keystrokes("ctrl-a");
    redraw(cx);
    editor.read_with(cx, |editor, cx| {
        let second = editor.document.visible_blocks()[1].entity.read(cx);
        assert_eq!(second.editor_selection_range, Some(0..second.visible_len()));
        assert!(editor.cross_block_selection.is_some());
        assert!(editor.rendered_select_all_cycle.is_none());
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
