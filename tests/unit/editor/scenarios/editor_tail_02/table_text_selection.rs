// @author kongweiguang

/// 从实际布局寻找词内命中，避免字体或缩放差异把多击测试落到段落的空白区域。
fn table_mirror_word_hit(
    block: &gpui::Entity<crate::components::Block>,
    visual: &mut VisualTestContext,
) -> gpui::Point<gpui::Pixels> {
    let bounds = block
        .read_with(visual, |block, _cx| block.last_bounds)
        .expect("rendered text bounds");
    (0..f32::from(bounds.size.width) as usize)
        .map(|x| point(bounds.left() + px(x as f32), bounds.center().y))
        .find(|position| {
            (1..3).contains(&block.read_with(visual, |block, _cx| {
                block.index_for_mouse_position(*position)
            }))
        })
        .expect("a hit inside the first word")
}

/// 多击必须经完整窗口冒泡路径；直接清选区无法覆盖树外 cell 镜像遗漏的缺陷。
fn table_mirror_click(
    visual: &mut VisualTestContext,
    position: gpui::Point<gpui::Pixels>,
    click_count: usize,
) {
    visual.simulate_event(MouseDownEvent {
        position,
        modifiers: Modifiers::default(),
        button: MouseButton::Left,
        click_count,
        first_mouse: false,
    });
    visual.simulate_mouse_move(position, MouseButton::Left, Modifiers::default());
    visual.simulate_event(MouseUpEvent {
        position,
        modifiers: Modifiers::default(),
        button: MouseButton::Left,
        click_count,
    });
    redraw(visual);
}

/// 各投影共享同一鼠标行为验收，同时检查清除全文镜像不会清掉 cell 自己的局部选择。
fn assert_document_select_all_pointer_clears_table_mirrors(
    cx: &mut TestAppContext,
    mode: ViewMode,
) {
    init_editor_test_app(cx);
    let original = "alpha start\n\nbeta phrase end\n\n| head | other |\n| --- | --- |\n| cell text | last |\n\ngamma end";
    let (editor, visual) =
        cx.add_window_view(move |_window, cx| Editor::from_markdown(cx, original.to_owned(), None));
    visual.simulate_resize(size(px(1180.0), px(780.0)));
    editor.update(visual, |editor, cx| editor.set_view_mode(mode, cx));
    redraw(visual);
    let surface = if mode == ViewMode::Split {
        crate::editor::selection_surface::SelectionSurface::SplitPreview
    } else {
        crate::editor::selection_surface::SelectionSurface::Main
    };
    let (paragraph, cells, baseline) = editor.read_with(visual, |editor, cx| {
        let visible = editor.selection_surface_entities(surface);
        let paragraph = visible
            .iter()
            .find(|block| block.read(cx).record.title.visible_text() == "beta phrase end")
            .expect("second paragraph")
            .clone();
        let table = visible
            .iter()
            .find(|block| block.read(cx).kind() == BlockKind::Table)
            .expect("native table");
        let runtime = table.read(cx).table_runtime.as_ref().expect("native cells");
        let cells = runtime
            .header
            .iter()
            .chain(runtime.rows.iter().flatten())
            .cloned()
            .collect::<Vec<_>>();
        let baseline = (
            editor.source_document.revision(),
            editor.document_dirty,
            editor.undo_history.len(),
            editor.redo_history.len(),
        );
        (paragraph, cells, baseline)
    });
    let position = table_mirror_word_hit(&paragraph, visual);
    visual.simulate_click(position, Modifiers::default());
    redraw(visual);

    for click_count in 1..=3 {
        visual.simulate_keystrokes("ctrl-a");
        redraw(visual);
        editor.read_with(visual, |editor, cx| {
            assert!(editor.cross_block_selection_for_surface(surface).is_some());
            for cell in &cells {
                let cell = cell.read(cx);
                assert_eq!(
                    cell.editor_selection_range,
                    Some(0..cell.visible_len()),
                    "Ctrl+A highlights the native table's visible text"
                );
            }
        });

        table_mirror_click(visual, position, click_count);
        editor.read_with(visual, |editor, cx| {
            assert!(editor.cross_block_selection_for_surface(surface).is_none());
            for cell in &cells {
                assert_eq!(
                    cell.read(cx).editor_selection_range,
                    None,
                    "a fresh {click_count}-click gesture must clear the old table text mirror"
                );
            }
            let paragraph = paragraph.read(cx);
            match click_count {
                1 => assert!(paragraph.selected_range.is_empty()),
                2 => assert_eq!(paragraph.selected_range, 0..4),
                3 => assert_eq!(paragraph.selected_range, 0..paragraph.visible_len()),
                _ => unreachable!(),
            }
        });
        if click_count > 1 {
            visual.simulate_keystrokes("ctrl-c");
            assert_eq!(
                visual.read_from_clipboard().and_then(|item| item.text()),
                Some(
                    if click_count == 2 {
                        "beta"
                    } else {
                        "beta phrase end"
                    }
                    .to_owned()
                )
            );
        }
    }

    let cell = &cells[0];
    let cell_position = table_mirror_word_hit(cell, visual);
    table_mirror_click(visual, cell_position, 2);
    visual.simulate_keystrokes("ctrl-c");
    assert_eq!(
        visual.read_from_clipboard().and_then(|item| item.text()),
        Some("head".to_owned()),
        "local cell word selection remains owned by its Block"
    );
    editor.read_with(visual, |editor, cx| {
        assert_eq!(cell.read(cx).selected_range, 0..4);
        assert!(
            cells
                .iter()
                .all(|cell| cell.read(cx).editor_selection_range.is_none())
        );
        assert_eq!(editor.source_document.text(), original);
        assert_eq!(
            (
                editor.source_document.revision(),
                editor.document_dirty,
                editor.undo_history.len(),
                editor.redo_history.len(),
            ),
            baseline,
            "selection and copying must preserve source and editing history"
        );
    });
}

/// Live 的段落新手势要结束全文镜像，保持局部选择与屏幕高亮一致。
#[gpui::test]
async fn live_document_select_all_pointer_clears_table_mirrors(cx: &mut TestAppContext) {
    assert_document_select_all_pointer_clears_table_mirrors(cx, ViewMode::Rendered);
}

/// Preview 的选择清理只影响展示状态，正文、revision 与历史均保持只读。
#[gpui::test]
async fn preview_document_select_all_pointer_clears_table_mirrors(cx: &mut TestAppContext) {
    assert_document_select_all_pointer_clears_table_mirrors(cx, ViewMode::Preview);
}

/// Split 右侧使用自己的 cell 运行时，不能依赖左侧 Source 的树清理文字高亮。
#[gpui::test]
async fn split_document_select_all_pointer_clears_table_mirrors(cx: &mut TestAppContext) {
    assert_document_select_all_pointer_clears_table_mirrors(cx, ViewMode::Split);
}

/// 非规范表格拼写不能影响全选边界；没有可见文字长度的尾部原子块仍属于整个文档。
fn assert_document_select_all_includes_eof_table(
    cx: &mut TestAppContext,
    mode: ViewMode,
    table_only: bool,
) {
    init_editor_test_app(cx);
    let original = if table_only {
        "| head | other |\n|---|---|\n| cell text | last |"
    } else {
        "alpha_beta\n\n| head | other |\n|---|---|\n| cell text | last |"
    };
    let (editor, visual) =
        cx.add_window_view(move |_window, cx| Editor::from_markdown(cx, original.to_owned(), None));
    visual.simulate_resize(size(px(1180.0), px(780.0)));
    editor.update(visual, |editor, cx| editor.set_view_mode(mode, cx));
    redraw(visual);
    let surface = if mode == ViewMode::Split {
        crate::editor::selection_surface::SelectionSurface::SplitPreview
    } else {
        crate::editor::selection_surface::SelectionSurface::Main
    };
    let (first, cells, baseline) = editor.read_with(visual, |editor, cx| {
        let visible = editor.selection_surface_entities(surface);
        let last = visible.last().expect("EOF table");
        assert_eq!(last.read(cx).kind(), BlockKind::Table);
        let runtime = last
            .read(cx)
            .table_runtime
            .as_ref()
            .expect("EOF table cells");
        let cells = runtime
            .header
            .iter()
            .chain(runtime.rows.iter().flatten())
            .cloned()
            .collect::<Vec<_>>();
        let first = if table_only {
            cells.first().expect("initial header cell").clone()
        } else {
            visible.first().expect("initial paragraph").clone()
        };
        let baseline = (
            editor.source_document.revision(),
            editor.document_dirty,
            editor.undo_history.len(),
            editor.redo_history.len(),
        );
        (first, cells, baseline)
    });
    let position = table_mirror_word_hit(&first, visual);
    visual.simulate_click(position, Modifiers::default());
    redraw(visual);
    visual.simulate_keystrokes("ctrl-a");
    redraw(visual);
    editor.read_with(visual, |editor, cx| {
        assert!(editor.cross_block_selection_for_surface(surface).is_some());
        for cell in &cells {
            let cell = cell.read(cx);
            assert_eq!(
                cell.editor_selection_range,
                Some(0..cell.visible_len()),
                "the table at EOF must be fully highlighted by one Ctrl+A"
            );
        }
        if mode == ViewMode::Rendered {
            assert_eq!(
                editor.capture_source_selection_snapshot(cx).range(),
                0..original.len(),
                "the full selection resolves to original source bytes"
            );
        }
    });
    visual.simulate_keystrokes("ctrl-c");
    assert_eq!(
        visual.read_from_clipboard().and_then(|item| item.text()),
        Some(
            if table_only {
                "head\tother\ncell text\tlast"
            } else {
                "alpha_beta\n\nhead\tother\ncell text\tlast"
            }
            .to_owned()
        ),
        "ordinary Copy includes the EOF table's visible cells"
    );
    let (html, _plain) = Editor::take_rich_clipboard_write_for_test()
        .expect("ordinary Copy preserves a rich payload");
    assert!(
        html.contains("<table"),
        "the rich payload retains native table structure"
    );
    editor.read_with(visual, |editor, _cx| {
        assert_eq!(editor.source_document.text(), original);
        assert_eq!(
            (
                editor.source_document.revision(),
                editor.document_dirty,
                editor.undo_history.len(),
                editor.redo_history.len(),
            ),
            baseline
        );
    });
}

/// Live 全选必须包含尾部表格，原始下划线及紧凑分隔符只参与源码坐标映射。
#[gpui::test]
async fn live_document_select_all_includes_eof_table(cx: &mut TestAppContext) {
    assert_document_select_all_includes_eof_table(cx, ViewMode::Rendered, false);
}

/// Preview 的 EOF 全选与复制需要完整原子表格，但不得形成任何正文事务。
#[gpui::test]
async fn preview_document_select_all_includes_eof_table(cx: &mut TestAppContext) {
    assert_document_select_all_includes_eof_table(cx, ViewMode::Preview, false);
}

/// Split 右侧 EOF 全选持续使用原源码范围，保持左右投影坐标契约独立。
#[gpui::test]
async fn split_document_select_all_includes_eof_table(cx: &mut TestAppContext) {
    assert_document_select_all_includes_eof_table(cx, ViewMode::Split, false);
}

/// 只有原子表格的 Live 文档仍有真实源码跨度，父块的零文字长度不能让全文选择消失。
#[gpui::test]
async fn live_document_select_all_includes_single_table(cx: &mut TestAppContext) {
    assert_document_select_all_includes_eof_table(cx, ViewMode::Rendered, true);
}

/// 纯表格 Preview 从实际 cell 输入目标全选，不能把读者焦点移到空父块。
#[gpui::test]
async fn preview_document_select_all_includes_single_table(cx: &mut TestAppContext) {
    assert_document_select_all_includes_eof_table(cx, ViewMode::Preview, true);
}

/// 纯表格 Split 右侧全文选择依赖非空源码锚点，不能以显示端点相同为由丢掉选区。
#[gpui::test]
async fn split_document_select_all_includes_single_table(cx: &mut TestAppContext) {
    assert_document_select_all_includes_eof_table(cx, ViewMode::Split, true);
}

/// 全选的规范投影范围必须编辑同一坐标空间；一次撤销还要恢复原拼写与整个表格的选区。
fn assert_live_document_select_all_mutation_roundtrip(
    cx: &mut TestAppContext,
    command: &str,
    table_only: bool,
    unicode_tail: bool,
) {
    init_editor_test_app(cx);
    let original = if table_only {
        "| head | other |\n|---|---|\n| cell text | last |"
    } else if unicode_tail {
        "alpha_beta\n\n| head | other |\n|---|---|\n| cell text | 末尾🤝 |"
    } else {
        "alpha_beta\n\n| head | other |\n|---|---|\n| cell text | last |"
    };
    let plain = if table_only {
        "head\tother\ncell text\tlast"
    } else if unicode_tail {
        "alpha_beta\n\nhead\tother\ncell text\t末尾🤝"
    } else {
        "alpha_beta\n\nhead\tother\ncell text\tlast"
    };
    let expected = if command == "input" { "替换🙂" } else { "" };
    let (editor, visual) =
        cx.add_window_view(move |_window, cx| Editor::from_markdown(cx, original.to_owned(), None));
    visual.simulate_resize(size(px(1180.0), px(780.0)));
    redraw(visual);
    let first = editor.read_with(visual, |editor, cx| {
        let visible = editor.document.visible_blocks();
        let first = visible.first().expect("initial content").entity.clone();
        if table_only {
            first
                .read(cx)
                .table_runtime
                .as_ref()
                .expect("native table cells")
                .header[0]
                .clone()
        } else {
            first
        }
    });
    let position = table_mirror_word_hit(&first, visual);
    visual.simulate_click(position, Modifiers::default());
    redraw(visual);
    visual.simulate_keystrokes("ctrl-a");
    redraw(visual);
    let initial_revision = editor.read_with(visual, |editor, cx| {
        assert_eq!(
            editor.capture_source_selection_snapshot(cx).range(),
            0..original.len()
        );
        assert!(editor.cross_block_selection.is_some());
        editor.source_document.revision()
    });

    if command == "input" {
        visual.simulate_input(expected);
    } else {
        visual.simulate_keystrokes(command);
    }
    redraw(visual);
    if command == "ctrl-x" {
        assert_eq!(
            visual.read_from_clipboard().and_then(|item| item.text()),
            Some(plain.to_owned()),
            "Cut writes the complete visible text before deleting the original selection"
        );
        let (html, _) = Editor::take_rich_clipboard_write_for_test().expect("rich Cut payload");
        assert!(html.contains("<table"));
    }
    editor.read_with(visual, |editor, _cx| {
        assert_eq!(
            editor.source_document.text(),
            expected,
            "full selection leaves no canonical tail"
        );
        assert!(editor.source_document.revision() > initial_revision);
        assert!(editor.document_dirty);
        assert_eq!(editor.undo_history.len(), 1);
        assert!(editor.redo_history.is_empty());
    });

    visual.simulate_keystrokes("ctrl-z");
    redraw(visual);
    editor.read_with(visual, |editor, cx| {
        assert_eq!(
            editor.source_document.text(),
            original,
            "Undo preserves every original Markdown byte"
        );
        assert_eq!(
            editor.capture_source_selection_snapshot(cx).range(),
            0..original.len()
        );
        assert!(
            editor.cross_block_selection.is_some(),
            "Undo restores the full atomic table selection"
        );
        assert!(editor.undo_history.is_empty());
        assert_eq!(editor.redo_history.len(), 1);
    });
    visual.simulate_keystrokes("ctrl-c");
    assert_eq!(
        visual.read_from_clipboard().and_then(|item| item.text()),
        Some(plain.to_owned()),
        "restored selection copies the same complete visible content"
    );
    visual.simulate_keystrokes("ctrl-shift-z");
    redraw(visual);
    editor.read_with(visual, |editor, cx| {
        assert_eq!(editor.source_document.text(), expected);
        assert_eq!(
            editor.capture_source_selection_snapshot(cx).range(),
            expected.len()..expected.len()
        );
        assert_eq!(editor.undo_history.len(), 1);
        assert!(editor.redo_history.is_empty());
    });
}

/// 尾部表格的完整删除不能按原拼写长度截断规范化范围，撤销恢复正文和全文选区。
#[gpui::test]
async fn live_document_select_all_delete_eof_table_roundtrip(cx: &mut TestAppContext) {
    assert_live_document_select_all_mutation_roundtrip(cx, "delete", false, false);
}

/// 剪切先生成完整 TSV 与 HTML，再与删除共用同一个正文事务和历史恢复边界。
#[gpui::test]
async fn live_document_select_all_cut_eof_table_roundtrip(cx: &mut TestAppContext) {
    assert_live_document_select_all_mutation_roundtrip(cx, "ctrl-x", false, false);
}

/// 单一原子表格的零显示端点仍代表全文，Delete 和 Undo 不能退化成单元格编辑。
#[gpui::test]
async fn live_document_select_all_delete_single_table_roundtrip(cx: &mut TestAppContext) {
    assert_live_document_select_all_mutation_roundtrip(cx, "delete", true, false);
}

/// 纯表格从单元格获得焦点后，Cut 与 Undo 仍归属于整个文档表面。
#[gpui::test]
async fn live_document_select_all_cut_single_table_roundtrip(cx: &mut TestAppContext) {
    assert_live_document_select_all_mutation_roundtrip(cx, "ctrl-x", true, false);
}

/// 非 ASCII 尾部验证写入范围属于同一 UTF-8 字符串，错误截断不能导致主线程 panic。
#[gpui::test]
async fn live_document_select_all_delete_unicode_eof_table_roundtrip(cx: &mut TestAppContext) {
    assert_live_document_select_all_mutation_roundtrip(cx, "delete", false, true);
}

/// 系统文字输入替换全文也要复用连续规范范围，保留新字素并完整恢复原文和选区。
#[gpui::test]
async fn live_document_select_all_input_unicode_eof_table_roundtrip(cx: &mut TestAppContext) {
    assert_live_document_select_all_mutation_roundtrip(cx, "input", false, true);
}
