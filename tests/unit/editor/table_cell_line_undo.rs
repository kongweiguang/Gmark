// @author kongweiguang

/// 用可见文字选区固定光标，确保源码偏移变化时能检查撤销后的实际选中位置。
fn select_alpha_table_cell_for_line_undo(
    editor: &gpui::Entity<Editor>,
    visual: &mut gpui::VisualTestContext,
) {
    editor.update_in(visual, |editor, window, cx| {
        let cell = editor
            .table_cells
            .values()
            .find(|binding| binding.cell.read(cx).display_text() == "alpha")
            .expect("alpha table cell")
            .cell
            .clone();
        cell.update(cx, |block, _cx| {
            block.selected_range = 0..5;
            block.selection_reversed = false;
            block.focus_handle.focus(window);
        });
    });
    redraw_line_operation_test_window(visual);
}

/// 验证重复操作不会把表格之前的非规范源码写入历史，并保留单词选区。
#[gpui::test]
async fn table_cell_duplicate_undo_restores_noncanonical_source_and_selection(
    cx: &mut TestAppContext,
) {
    init_line_operation_test_app(cx);
    let source = "alpha_beta\n\n| H | B |\n| --- | --- |\n| alpha | beta |";
    let (editor, visual) = cx.add_window_view(move |_window, cx| {
        Editor::from_markdown(cx, source.to_owned(), None)
    });
    redraw_line_operation_test_window(visual);
    select_alpha_table_cell_for_line_undo(&editor, visual);

    visual.dispatch_action(crate::components::DuplicateLine);
    redraw_line_operation_test_window(visual);
    editor.read_with(visual, |editor, cx| {
        assert!(editor
            .table_cells
            .values()
            .any(|binding| binding.cell.read(cx).display_text() == "alpha alpha"));
    });

    visual.dispatch_action(crate::components::Undo);
    redraw_line_operation_test_window(visual);
    editor.read_with(visual, |editor, cx| {
        assert_eq!(editor.source_document.text(), source);
        let cell = editor
            .table_cells
            .values()
            .find(|binding| binding.cell.read(cx).display_text() == "alpha")
            .expect("restored alpha table cell")
            .cell
            .read(cx);
        assert_eq!(cell.selected_range, 0..5);
        assert!(!cell.selection_reversed);
    });
}

/// 单独覆盖删除替换路径，防止它绕过重复操作共用的权威源码历史边界。
#[gpui::test]
async fn table_cell_delete_undo_restores_noncanonical_source_and_selection(
    cx: &mut TestAppContext,
) {
    init_line_operation_test_app(cx);
    let source = "alpha_beta\n\n| H | B |\n| --- | --- |\n| alpha | beta |";
    let (editor, visual) = cx.add_window_view(move |_window, cx| {
        Editor::from_markdown(cx, source.to_owned(), None)
    });
    redraw_line_operation_test_window(visual);
    select_alpha_table_cell_for_line_undo(&editor, visual);

    visual.dispatch_action(crate::components::DeleteLine);
    redraw_line_operation_test_window(visual);
    editor.read_with(visual, |editor, cx| {
        assert!(editor
            .table_cells
            .values()
            .any(|binding| binding.cell.read(cx).display_text().is_empty()));
    });

    visual.dispatch_action(crate::components::Undo);
    redraw_line_operation_test_window(visual);
    editor.read_with(visual, |editor, cx| {
        assert_eq!(editor.source_document.text(), source);
        let cell = editor
            .table_cells
            .values()
            .find(|binding| binding.cell.read(cx).display_text() == "alpha")
            .expect("restored alpha table cell")
            .cell
            .read(cx);
        assert_eq!(cell.selected_range, 0..5);
        assert!(!cell.selection_reversed);
    });
}
