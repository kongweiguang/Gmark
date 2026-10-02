// @author kongweiguang

#[derive(Clone, Copy)]
enum VirtualLineCommand {
    Duplicate,
    Delete,
    MoveUp,
    MoveDown,
    Indent,
}

/// Provides enough projected regions to leave the selection's source anchor outside the viewport.
fn virtual_line_operation_source() -> String {
    (0..600)
        .map(|index| format!("paragraph {index:04}"))
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Pins a reverse selection endpoint, then returns its source range after that endpoint is unmounted.
fn select_virtual_reverse_line_range(
    editor: &gpui::Entity<Editor>,
    visual: &mut gpui::VisualTestContext,
) -> (std::ops::Range<usize>, usize, usize, String, String) {
    editor.update_in(visual, |editor, _window, _cx| {
        let max_y = f32::from(editor.scroll_handle.max_offset().height.max(px(0.0)));
        editor
            .scroll_handle
            .set_offset(point(px(0.0), px(-max_y * 0.75)));
    });
    redraw_line_operation_test_window(visual);

    let anchor_id = editor.update_in(visual, |editor, _window, cx| {
        let anchor_block = editor
            .document
            .visible_blocks()
            .last()
            .expect("far viewport paragraph")
            .entity
            .clone();
        let anchor = CrossBlockSelectionEndpoint {
            entity_id: anchor_block.entity_id(),
            offset: anchor_block.read(cx).visible_len(),
        };
        editor.active_entity_id = Some(anchor.entity_id);
        editor.cross_block_drag = Some(editor.cross_block_drag_from_endpoint(
            crate::editor::selection_surface::SelectionSurface::Main,
            anchor,
            cx,
        ));
        editor.scroll_handle.set_offset(point(px(0.0), px(0.0)));
        anchor.entity_id
    });
    redraw_line_operation_test_window(visual);

    editor.update_in(visual, |editor, window, cx| {
        let virtual_surface = editor.virtual_surface.as_ref().expect("virtual surface");
        assert!(virtual_surface.entity_by_id(anchor_id).is_some());
        assert!(editor.document.block_entity_by_id(anchor_id).is_none());
        let anchor_text = virtual_surface
            .entity_by_id(anchor_id)
            .expect("pinned source anchor")
            .read(cx)
            .display_text()
            .to_owned();

        let focus = editor
            .document
            .visible_blocks()
            .get(10)
            .expect("top viewport paragraph after the previous row")
            .entity
            .clone();
        let focus_text = focus.read(cx).display_text().to_owned();
        focus.read(cx).focus_handle.focus(window);
        editor.apply_surface_pointer_selection_to_endpoint(
            crate::editor::selection_surface::SelectionSurface::Main,
            CrossBlockSelectionEndpoint {
                entity_id: focus.entity_id(),
                offset: 0,
            },
            cx,
        );

        let selection = editor
            .normalized_cross_block_selection_for_surface(
                crate::editor::selection_surface::SelectionSurface::Main,
                cx,
            )
            .expect("revision-bound virtual selection");
        assert!(
            selection.reversed,
            "the source anchor is after the active head"
        );
        assert!(selection.start_index.is_some());
        assert!(
            selection.end_index.is_none(),
            "the anchor endpoint is unmounted"
        );
        let source_range = editor
            .cross_block_source_range_for_normalized(selection, cx)
            .expect("selection source range");
        let stored = editor
            .cross_block_selection
            .expect("stored cross-block selection");
        let anchor = stored.source_anchor.expect("source anchor");
        let focus = stored.source_focus.expect("source focus");
        assert!(anchor.byte_offset > focus.byte_offset);
        (
            source_range,
            anchor.byte_offset,
            focus.byte_offset,
            focus_text,
            anchor_text,
        )
    })
}

/// 独立按 fixture 的完整段落计算期望，避免源码行 planner 把空白行误当作 Live 的移动单位。
fn expected_virtual_line_operation_source(
    source: &str,
    range: std::ops::Range<usize>,
    command: VirtualLineCommand,
) -> String {
    let separator = if source.contains("\r\n") {
        "\r\n\r\n"
    } else {
        "\n\n"
    };
    let first = source[..range.start].matches(separator).count();
    let end = first + source[range].split(separator).count();
    let mut paragraphs = source
        .split(separator)
        .map(str::to_owned)
        .collect::<Vec<_>>();
    match command {
        VirtualLineCommand::Duplicate => {
            let selected = paragraphs[first..end].to_vec();
            paragraphs.splice(end..end, selected);
        }
        VirtualLineCommand::Delete => {
            paragraphs.drain(first..end);
        }
        VirtualLineCommand::MoveUp => paragraphs[first - 1..end].rotate_left(1),
        VirtualLineCommand::MoveDown => paragraphs[first..=end].rotate_right(1),
        VirtualLineCommand::Indent => {
            for paragraph in &mut paragraphs[first..end] {
                *paragraph = format!("    {paragraph}");
            }
        }
    }
    paragraphs.join(separator)
}

/// 使用真实保存快照写入隔离临时文件，验证 LF 真值之外的 CRLF 格式没有被行操作破坏。
fn saved_virtual_line_operation_bytes(editor: &Editor) -> Vec<u8> {
    let temporary = tempfile::tempdir().expect("isolated save directory");
    let path = temporary.path().join("line-operation.md");
    editor
        .source_document
        .try_save_snapshot()
        .expect("save snapshot")
        .save_as_atomic_cancellable(&path, &gmark_paged_document::SearchCancellation::default())
        .expect("save snapshot bytes");
    std::fs::read(path).expect("saved bytes")
}

/// Runs the public action and verifies one Undo restores both source and the original reverse direction.
fn assert_virtual_line_action_undoes_with_direction(
    cx: &mut TestAppContext,
    command: VirtualLineCommand,
) {
    assert_virtual_line_action_with_source(cx, command, virtual_line_operation_source());
}

/// 通过公开编辑动作验证源码边界，允许 CRLF fixture 与同一 undo 契约共用执行器。
fn assert_virtual_line_action_with_source(
    cx: &mut TestAppContext,
    command: VirtualLineCommand,
    original: String,
) {
    init_line_operation_test_app(cx);
    let original_bytes = original.clone();
    let crlf = original.contains("\r\n");
    let editor_source = original.clone();
    let (editor, visual) = cx.add_window_view(move |_window, cx| {
        Editor::from_markdown_virtualized(cx, editor_source, None)
    });
    redraw_line_operation_test_window(visual);
    let original = editor.read_with(visual, |editor, _cx| {
        assert_eq!(
            saved_virtual_line_operation_bytes(editor),
            original_bytes.as_bytes(),
        );
        editor.source_document.text()
    });

    let (source_range, anchor_offset, focus_offset, first_text, anchor_text) =
        select_virtual_reverse_line_range(&editor, visual);
    let expected = expected_virtual_line_operation_source(&original, source_range.clone(), command);
    let first_index = first_text
        .strip_prefix("paragraph ")
        .expect("generated paragraph marker")
        .parse::<usize>()
        .expect("generated paragraph index");
    let anchor_index = anchor_text
        .strip_prefix("paragraph ")
        .expect("generated anchor marker")
        .parse::<usize>()
        .expect("generated anchor index");
    let previous_text = format!("paragraph {:04}", first_index - 1);
    let next_text = format!("paragraph {:04}", anchor_index + 1);
    editor.update_in(visual, |editor, window, cx| {
        assert_eq!(
            editor.cross_block_source_range_for_normalized(
                editor
                    .normalized_cross_block_selection_for_surface(
                        crate::editor::selection_surface::SelectionSurface::Main,
                        cx,
                    )
                    .expect("selection"),
                cx,
            ),
            Some(source_range.clone())
        );
        match command {
            VirtualLineCommand::Duplicate => editor.on_duplicate_line(&DuplicateLine, window, cx),
            VirtualLineCommand::Delete => editor.on_delete_line(&DeleteLine, window, cx),
            VirtualLineCommand::MoveUp => editor.on_move_line_up(&MoveLineUp, window, cx),
            VirtualLineCommand::MoveDown => editor.on_move_line_down(&MoveLineDown, window, cx),
            VirtualLineCommand::Indent => editor.on_indent_block(&IndentBlock, window, cx),
        }

        let actual = editor.source_document.text();
        assert_eq!(
            actual, expected,
            "the complete source selection must be edited"
        );
        let encoded_expected = if crlf {
            expected.replace('\n', "\r\n")
        } else {
            expected.clone()
        };
        assert_eq!(
            saved_virtual_line_operation_bytes(editor),
            encoded_expected.as_bytes(),
            "the save snapshot preserves the original line ending format",
        );
        assert!(
            actual.contains(&previous_text),
            "the unselected preceding paragraph remains"
        );
        assert!(
            actual.contains(&next_text),
            "the unselected following paragraph remains"
        );
        if matches!(command, VirtualLineCommand::Indent) {
            let separator = if original.contains("\r\n") {
                "\r\n\r\n"
            } else {
                "\n\n"
            };
            assert!(actual.contains(&format!("{previous_text}{separator}    {first_text}")));
            assert!(actual.contains(&format!("    {anchor_text}{separator}{next_text}")));
        }
    });
    redraw_line_operation_test_window(visual);

    editor.update_in(visual, |editor, _window, cx| {
        editor.undo_document(cx);
        assert_eq!(editor.source_document.text(), original);
        assert_eq!(
            saved_virtual_line_operation_bytes(editor),
            original_bytes.as_bytes(),
        );

        let restored = editor
            .cross_block_selection
            .expect("Undo restores the selection");
        let restored_anchor = restored.source_anchor.expect("restored source anchor");
        let restored_focus = restored.source_focus.expect("restored source focus");
        let revision = editor.source_document.snapshot().revision();
        assert_eq!(restored_anchor.byte_offset, anchor_offset);
        assert_eq!(restored_focus.byte_offset, focus_offset);
        assert!(restored_anchor.byte_offset > restored_focus.byte_offset);
        assert_eq!(restored_anchor.revision, revision);
        assert_eq!(restored_focus.revision, revision);
        editor.undo_document(cx);
        assert_eq!(
            editor.source_document.text(),
            original,
            "one operation leaves no second partial undo"
        );
        editor.redo_document(cx);
        assert_eq!(editor.source_document.text(), expected);
    });
}

/// Covers virtualized Duplicate, whose selected source rows span beyond mounted block entities.
#[gpui::test]
async fn virtual_duplicate_uses_unmounted_selection_and_undo_restores_direction(
    cx: &mut TestAppContext,
) {
    assert_virtual_line_action_undoes_with_direction(cx, VirtualLineCommand::Duplicate);
}

/// Covers virtualized Delete while ensuring its single Undo restores the reverse source selection.
#[gpui::test]
async fn virtual_delete_uses_unmounted_selection_and_undo_restores_direction(
    cx: &mut TestAppContext,
) {
    assert_virtual_line_action_undoes_with_direction(cx, VirtualLineCommand::Delete);
}

/// Covers moving a reverse virtual selection upward when its source anchor is no longer mounted.
#[gpui::test]
async fn virtual_move_uses_unmounted_selection_and_undo_restores_direction(
    cx: &mut TestAppContext,
) {
    assert_virtual_line_action_undoes_with_direction(cx, VirtualLineCommand::MoveUp);
}

/// Covers indentation across virtual regions and verifies Undo retains the original selection order.
#[gpui::test]
async fn virtual_indent_uses_unmounted_selection_and_undo_restores_direction(
    cx: &mut TestAppContext,
) {
    assert_virtual_line_action_undoes_with_direction(cx, VirtualLineCommand::Indent);
}

/// 向下移动应交换整个相邻段落；跨块反向选区不能退回当前块。
#[gpui::test]
async fn virtual_move_down_uses_unmounted_selection_and_undo_restores_direction(
    cx: &mut TestAppContext,
) {
    assert_virtual_line_action_undoes_with_direction(cx, VirtualLineCommand::MoveDown);
}

/// 行操作必须保留未选正文及 Windows 换行格式，不能在重建选区时规范化整篇源码。
#[gpui::test]
async fn virtual_crlf_duplicate_uses_unmounted_selection_and_undo_restores_direction(
    cx: &mut TestAppContext,
) {
    assert_virtual_line_action_with_source(
        cx,
        VirtualLineCommand::Duplicate,
        virtual_line_operation_source().replace('\n', "\r\n"),
    );
}

/// 常驻 Live 的跨段落缩进必须操作完整段落，并可一次撤销正文与原反向选区。
#[gpui::test]
async fn resident_live_selected_paragraphs_indent_as_one_transaction(cx: &mut TestAppContext) {
    init_line_operation_test_app(cx);
    let source = "before\n\none\n\ntwo\n\nafter";
    let (editor, visual) =
        cx.add_window_view(|_window, cx| Editor::from_markdown(cx, source.to_owned(), None));
    redraw_line_operation_test_window(visual);
    editor.update_in(visual, |editor, window, cx| {
        let blocks = editor
            .selection_surface_entities(crate::editor::selection_surface::SelectionSurface::Main);
        let first = blocks
            .iter()
            .find(|block| block.read(cx).display_text() == "one")
            .expect("first paragraph")
            .clone();
        let last = blocks
            .iter()
            .find(|block| block.read(cx).display_text() == "two")
            .expect("last paragraph")
            .clone();
        let anchor = CrossBlockSelectionEndpoint {
            entity_id: last.entity_id(),
            offset: 2,
        };
        let focus = CrossBlockSelectionEndpoint {
            entity_id: first.entity_id(),
            offset: 1,
        };
        first.read(cx).focus_handle.focus(window);
        editor.cross_block_selection = Some(editor.cross_block_selection_from_endpoints(
            crate::editor::selection_surface::SelectionSurface::Main,
            anchor,
            focus,
            None,
            cx,
        ));
        let original = editor
            .cross_block_source_selection_snapshot(cx)
            .expect("source selection");
        editor.on_indent_block(&IndentBlock, window, cx);
        assert_eq!(
            editor.source_document.text(),
            "before\n\n    one\n\n    two\n\nafter"
        );
        editor.undo_document(cx);
        assert_eq!(editor.source_document.text(), source);
        let restored = editor
            .cross_block_source_selection_snapshot(cx)
            .expect("restored selection");
        assert_eq!(restored.range(), original.range());
        assert_eq!(restored.reversed(), original.reversed());
    });
}

/// 列表选区使用源码锚点，便于缩进重建后仍操作同一组文字，而不是旧实体。
fn select_live_list_test_items(
    editor: &gpui::Entity<Editor>,
    visual: &mut gpui::VisualTestContext,
    first_title: &str,
    last_title: &str,
) {
    editor.update_in(visual, |editor, window, cx| {
        let blocks = editor
            .selection_surface_entities(crate::editor::selection_surface::SelectionSurface::Main);
        let first = blocks
            .iter()
            .find(|block| block.read(cx).display_text() == first_title)
            .expect("first item")
            .clone();
        let last = blocks
            .iter()
            .find(|block| block.read(cx).display_text() == last_title)
            .expect("last item")
            .clone();
        let anchor = CrossBlockSelectionEndpoint {
            entity_id: last.entity_id(),
            offset: last.read(cx).visible_len(),
        };
        let focus = CrossBlockSelectionEndpoint {
            entity_id: first.entity_id(),
            offset: 0,
        };
        first.read(cx).focus_handle.focus(window);
        editor.active_entity_id = Some(first.entity_id());
        editor.cross_block_selection = Some(editor.cross_block_selection_from_endpoints(
            crate::editor::selection_surface::SelectionSurface::Main,
            anchor,
            focus,
            None,
            cx,
        ));
    });
}

/// 多项列表应整体移到前一项下面，再整体反缩进；每次动作只留一条可恢复的历史。
#[gpui::test]
async fn live_selected_list_items_indent_and_outdent_round_trip(cx: &mut TestAppContext) {
    init_line_operation_test_app(cx);
    for virtualized in [false, true] {
        let suffix = if virtualized {
            format!("\n\n{}", virtual_line_operation_source())
        } else {
            String::new()
        };
        let source = format!("- one\n- two\n- three\n- four{suffix}");
        let before = source.clone();
        let (editor, visual) = cx.add_window_view(move |_window, cx| {
            if virtualized {
                Editor::from_markdown_virtualized(cx, source, None)
            } else {
                Editor::from_markdown(cx, source, None)
            }
        });
        redraw_line_operation_test_window(visual);
        select_live_list_test_items(&editor, visual, "two", "three");
        editor.update_in(visual, |editor, window, cx| {
            editor.on_indent_block(&IndentBlock, window, cx)
        });
        redraw_line_operation_test_window(visual);
        let indented = format!("- one\n  - two\n  - three\n- four{suffix}");
        editor.read_with(visual, |editor, _cx| {
            assert_eq!(editor.source_document.text(), indented)
        });
        editor.update_in(visual, |editor, window, cx| {
            editor.on_outdent_block(&OutdentBlock, window, cx)
        });
        redraw_line_operation_test_window(visual);
        editor.update_in(visual, |editor, _window, cx| {
            assert_eq!(editor.source_document.text(), before);
            editor.undo_document(cx);
            assert_eq!(editor.source_document.text(), indented);
            editor.undo_document(cx);
            assert_eq!(editor.source_document.text(), before);
            editor.redo_document(cx);
            assert_eq!(editor.source_document.text(), indented);
            editor.redo_document(cx);
            assert_eq!(editor.source_document.text(), before);
        });
    }
}

/// 第一项没有可缩进的前一项；无效批量缩进不能改源码、dirty 或撤销历史。
#[gpui::test]
async fn live_first_selected_list_items_indent_is_noop(cx: &mut TestAppContext) {
    init_line_operation_test_app(cx);
    let source = "- one\n- two\n- three";
    let (editor, visual) =
        cx.add_window_view(|_window, cx| Editor::from_markdown(cx, source.to_owned(), None));
    redraw_line_operation_test_window(visual);
    select_live_list_test_items(&editor, visual, "one", "two");
    editor.update_in(visual, |editor, window, cx| {
        let revision = editor.source_document.revision();
        editor.on_indent_block(&IndentBlock, window, cx);
        assert_eq!(editor.source_document.text(), source);
        assert_eq!(editor.source_document.revision(), revision);
        assert!(!editor.document_dirty);
        editor.undo_document(cx);
        assert_eq!(editor.source_document.text(), source);
    });
}

/// 根列表项反缩进沿用单项规则转成普通段落，并保持两项次序和未选邻项。
#[gpui::test]
async fn live_top_level_selected_list_items_outdent_to_paragraphs(cx: &mut TestAppContext) {
    init_line_operation_test_app(cx);
    let source = "- one\n- two\n- three\n- four";
    let (editor, visual) =
        cx.add_window_view(|_window, cx| Editor::from_markdown(cx, source.to_owned(), None));
    redraw_line_operation_test_window(visual);
    select_live_list_test_items(&editor, visual, "two", "three");
    editor.update_in(visual, |editor, window, cx| {
        editor.on_outdent_block(&OutdentBlock, window, cx)
    });
    redraw_line_operation_test_window(visual);
    editor.update_in(visual, |editor, _window, cx| {
        assert_eq!(
            editor.source_document.text(),
            "- one\n\ntwo\n\nthree\n\n- four"
        );
        editor.undo_document(cx);
        assert_eq!(editor.source_document.text(), source);
        let selection = editor
            .cross_block_selection
            .expect("reverse list selection restored");
        assert!(
            selection.source_anchor.expect("anchor").byte_offset
                > selection.source_focus.expect("focus").byte_offset
        );
    });
}

/// GFM 单元格保持一个逻辑行；快捷键只编辑文字，保存和撤销不能改变其显示或表格结构。
#[gpui::test]
async fn table_cell_line_operations_edit_text_and_restore_table_on_undo(cx: &mut TestAppContext) {
    init_line_operation_test_app(cx);
    let source = "| H | B |\n| --- | --- |\n| alpha | beta |";
    let (editor, visual) =
        cx.add_window_view(|_window, cx| Editor::from_markdown(cx, source.to_owned(), None));
    redraw_line_operation_test_window(visual);
    let cell = editor.update_in(visual, |editor, window, cx| {
        let cell = editor
            .table_cells
            .values()
            .find(|binding| binding.cell.read(cx).display_text() == "alpha")
            .expect("body cell")
            .cell
            .clone();
        cell.read(cx).focus_handle.focus(window);
        editor.on_duplicate_line(&DuplicateLine, window, cx);
        cell
    });
    redraw_line_operation_test_window(visual);
    editor.read_with(visual, |editor, cx| {
        assert_eq!(cell.read(cx).display_text(), "alpha alpha");
        assert_eq!(editor.table_cells.len(), 4, "table shape remains stable");
    });
    editor.update_in(visual, |editor, window, cx| {
        editor.on_delete_line(&DeleteLine, window, cx)
    });
    redraw_line_operation_test_window(visual);
    editor.read_with(visual, |editor, cx| {
        assert_eq!(cell.read(cx).display_text(), "");
        assert_eq!(editor.table_cells.len(), 4);
    });
    editor.update_in(visual, |editor, _window, cx| {
        editor.undo_document(cx);
        assert!(
            editor
                .table_cells
                .values()
                .any(|binding| binding.cell.read(cx).display_text() == "alpha alpha")
        );
        editor.undo_document(cx);
        assert_eq!(editor.source_document.text(), source);
        assert_eq!(editor.table_cells.len(), 4);
    });
}
