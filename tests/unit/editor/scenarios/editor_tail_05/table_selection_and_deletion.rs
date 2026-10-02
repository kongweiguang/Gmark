// @author kongweiguang

#[gpui::test]
async fn selecting_first_body_row_does_not_highlight_header(cx: &mut TestAppContext) {
    use crate::components::{TableAxisHighlight, TableAxisKind};
    let markdown = ["| A | B |", "| --- | --- |", "| 1 | 2 |", "| 3 | 4 |"].join("\n");
    let editor = cx.new(|cx| Editor::from_markdown(cx, markdown, None));

    editor.update(cx, |editor, cx| {
        let table = editor.document.first_root().expect("table root").clone();
        // Visual row 1 is the first body row; the header (row 0) must stay clear.
        editor.select_table_axis(table.entity_id(), TableAxisKind::Row, 1, cx);

        let runtime = table.read(cx).table_runtime.clone().expect("runtime");
        for cell in &runtime.header {
            assert_eq!(
                cell.read(cx).table_axis_highlight,
                TableAxisHighlight::None,
                "header should not be highlighted"
            );
        }
        for cell in &runtime.rows[0] {
            assert_eq!(
                cell.read(cx).table_axis_highlight,
                TableAxisHighlight::Selected
            );
        }
        for cell in &runtime.rows[1] {
            assert_eq!(cell.read(cx).table_axis_highlight, TableAxisHighlight::None);
        }
    });
}

#[gpui::test]
async fn selecting_header_row_highlights_only_header(cx: &mut TestAppContext) {
    use crate::components::{TableAxisHighlight, TableAxisKind};
    let markdown = ["| A | B |", "| --- | --- |", "| 1 | 2 |"].join("\n");
    let editor = cx.new(|cx| Editor::from_markdown(cx, markdown, None));

    editor.update(cx, |editor, cx| {
        let table = editor.document.first_root().expect("table root").clone();
        editor.select_table_axis(table.entity_id(), TableAxisKind::Row, 0, cx);

        let runtime = table.read(cx).table_runtime.clone().expect("runtime");
        for cell in &runtime.header {
            assert_eq!(
                cell.read(cx).table_axis_highlight,
                TableAxisHighlight::Selected
            );
        }
        for cell in &runtime.rows[0] {
            assert_eq!(cell.read(cx).table_axis_highlight, TableAxisHighlight::None);
        }
    });
}

#[gpui::test]
async fn body_row_preview_survives_stale_header_leave(cx: &mut TestAppContext) {
    use crate::components::TableAxisKind;
    let markdown = ["| A | B |", "| --- | --- |", "| 1 | 2 |"].join("\n");
    let editor = cx.new(|cx| Editor::from_markdown(cx, markdown, None));

    editor.update(cx, |editor, cx| {
        let table = editor.document.first_root().expect("table root").clone();
        let id = table.entity_id();

        // Pointer crosses from the header handle down onto the first body row.
        // The body handle's enter arrives first, then the header handle's leave;
        // the stale leave must not clear the preview the pointer moved onto.
        editor.preview_table_axis(id, TableAxisKind::Row, 1, true, cx);
        editor.preview_table_axis(id, TableAxisKind::Row, 0, false, cx);
        assert_eq!(
            editor.table_axis_preview,
            Some(super::TableAxisSelection {
                table_block_id: id,
                kind: TableAxisKind::Row,
                index: 1,
            }),
            "body row preview must survive the header's stale leave"
        );

        // Leaving the body handle that owns the preview still clears it.
        editor.preview_table_axis(id, TableAxisKind::Row, 1, false, cx);
        assert_eq!(editor.table_axis_preview, None);
    });
}

#[gpui::test]
async fn deleting_table_column_moves_selection_to_nearest_survivor(cx: &mut TestAppContext) {
    let markdown = ["| A | B | C |", "| --- | --- | --- |", "| 1 | 2 | 3 |"].join("\n");
    let editor = cx.new(|cx| Editor::from_markdown(cx, markdown, None));

    editor.update(cx, |editor, cx| {
        let table = editor.document.first_root().expect("table root").clone();
        editor.delete_table_column(&table, 2, cx);

        let record = table.read(cx).record.table.as_ref().expect("table record");
        assert_eq!(record.header.len(), 2);
        assert_eq!(
            editor.table_axis_selection,
            Some(super::TableAxisSelection {
                table_block_id: table.entity_id(),
                kind: crate::components::TableAxisKind::Column,
                index: 1,
            })
        );
    });
}

#[gpui::test]
async fn deleting_table_header_promotes_next_row(cx: &mut TestAppContext) {
    let markdown = ["| A | B |", "| --- | --- |", "| 1 | 2 |"].join("\n");
    let editor = cx.new(|cx| Editor::from_markdown(cx, markdown, None));

    editor.update(cx, |editor, cx| {
        let table = editor.document.first_root().expect("table root").clone();
        editor.delete_table_header_row(&table, cx);

        let record = table.read(cx).record.table.as_ref().expect("table record");
        assert_eq!(record.header[0].serialize_markdown(), "1");
        assert_eq!(record.header[1].serialize_markdown(), "2");
        assert!(record.rows.is_empty());

        let runtime = table
            .read(cx)
            .table_runtime
            .as_ref()
            .expect("rebuilt runtime");
        assert_eq!(editor.pending_focus, Some(runtime.header[0].entity_id()));
    });
}

#[gpui::test]
async fn deleting_last_body_row_leaves_header_only_table(cx: &mut TestAppContext) {
    let markdown = ["| A | B |", "| --- | --- |", "| 1 | 2 |"].join("\n");
    let editor = cx.new(|cx| Editor::from_markdown(cx, markdown, None));

    editor.update(cx, |editor, cx| {
        let table = editor.document.first_root().expect("table root").clone();
        // Deleting the only body row used to be blocked; now it leaves a
        // header-only table behind.
        editor.delete_table_row(&table, 0, cx);

        let record = table.read(cx).record.table.as_ref().expect("table record");
        assert!(record.rows.is_empty());
        assert_eq!(record.header[0].serialize_markdown(), "A");
        assert_eq!(editor.document.root_count(), 1);
        assert_eq!(table.read(cx).kind(), BlockKind::Table);
    });
}

#[gpui::test]
async fn removing_table_block_replaces_it_with_empty_paragraph(cx: &mut TestAppContext) {
    let markdown = [
        "intro",
        "",
        "| A | B |",
        "| --- | --- |",
        "| 1 | 2 |",
        "",
        "outro",
    ]
    .join("\n");
    let editor = cx.new(|cx| Editor::from_markdown(cx, markdown, None));

    editor.update(cx, |editor, cx| {
        let table = editor.document.root_blocks()[1].clone();
        assert_eq!(table.read(cx).kind(), BlockKind::Table);
        editor.remove_table_block(&table, cx);

        let roots = editor.document.root_blocks();
        assert_eq!(roots.len(), 3);
        assert_eq!(roots[0].read(cx).display_text(), "intro");
        assert_eq!(roots[1].read(cx).kind(), BlockKind::Paragraph);
        assert_eq!(roots[1].read(cx).display_text(), "");
        assert_eq!(roots[2].read(cx).display_text(), "outro");
        assert_eq!(editor.pending_focus, Some(roots[1].entity_id()));
    });
}

#[gpui::test]
async fn removing_the_only_table_leaves_one_empty_paragraph(cx: &mut TestAppContext) {
    let markdown = ["| A | B |", "| --- | --- |", "| 1 | 2 |"].join("\n");
    let editor = cx.new(|cx| Editor::from_markdown(cx, markdown, None));

    editor.update(cx, |editor, cx| {
        let table = editor.document.first_root().expect("table root").clone();
        editor.remove_table_block(&table, cx);

        let roots = editor.document.root_blocks();
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].read(cx).kind(), BlockKind::Paragraph);
        assert_eq!(roots[0].read(cx).display_text(), "");
    });
}

#[gpui::test]
async fn table_insert_and_duplicate_commands_are_single_undo_steps(cx: &mut TestAppContext) {
    let markdown = ["| A | B |", "| --- | --- |", "| 1 | 2 |"].join("\n");
    let editor = cx.new(|cx| Editor::from_markdown(cx, markdown, None));

    editor.update(cx, |editor, cx| {
        let table = editor.document.first_root().expect("table root").clone();
        let initial_history = editor.undo_history.len();
        editor.insert_table_row(&table, 1, cx);
        assert_eq!(editor.undo_history.len(), initial_history + 1);
        assert_eq!(table.read(cx).record.table.as_ref().unwrap().rows.len(), 2);

        editor.duplicate_table_column(&table, 0, cx);
        assert_eq!(editor.undo_history.len(), initial_history + 2);
        let data = table.read(cx).record.table.as_ref().unwrap();
        assert_eq!(data.column_count(), 3);
        assert_eq!(data.header[0].visible_text(), data.header[1].visible_text());
    });
}

/// Entering rectangle mode clears cell-local ranges so later clipboard commands have one selection owner.
#[gpui::test]
async fn escape_enters_rectangular_table_selection_and_delete_clears_cells(
    cx: &mut TestAppContext,
) {
    let markdown = ["| A | B |", "| --- | --- |", "| 1 | 2 |"].join("\n");
    let editor = cx.new(|cx| Editor::from_markdown(cx, markdown, None));
    let escape = KeyDownEvent {
        keystroke: Keystroke::parse("escape").expect("valid escape"),
        is_held: false,
    };
    let extend = KeyDownEvent {
        keystroke: Keystroke::parse("shift-right").expect("valid shifted arrow"),
        is_held: false,
    };
    let delete = KeyDownEvent {
        keystroke: Keystroke::parse("delete").expect("valid delete"),
        is_held: false,
    };

    editor.update(cx, |editor, cx| {
        let table = editor.document.first_root().expect("table root").clone();
        let first = table.read(cx).table_runtime.as_ref().unwrap().header[0].clone();
        first.update(cx, |block, _cx| {
            block.selected_range = 0..1;
        });
        editor.active_entity_id = Some(first.entity_id());
        assert!(editor.handle_table_cell_selection_key(&escape, cx));
        assert!(first.read(cx).selected_range.is_empty());
        assert!(editor.handle_table_cell_selection_key(&extend, cx));
        let selection = editor.table_cell_rectangle.expect("rectangle selection");
        assert_eq!(selection.columns(), 0..=1);

        assert!(editor.handle_table_cell_selection_key(&delete, cx));
        let table = table.read(cx).record.table.as_ref().unwrap();
        assert!(
            table
                .header
                .iter()
                .all(|cell| cell.visible_text().is_empty())
        );
        assert_eq!(editor.undo_history.len(), 1);
    });
}

#[gpui::test]
async fn tsv_paste_expands_table_once_without_tiling(cx: &mut TestAppContext) {
    let markdown = ["| A |", "| --- |", "| 1 |"].join("\n");
    let editor = cx.new(|cx| Editor::from_markdown(cx, markdown, None));
    editor.update(cx, |editor, cx| {
        let table = editor.document.first_root().expect("table root").clone();
        editor.table_cell_rectangle = Some(crate::editor::table_selection::TableCellRectangle {
            table_block_id: table.entity_id(),
            anchor: crate::components::TableCellPosition { row: 1, column: 0 },
            focus: crate::components::TableCellPosition { row: 1, column: 0 },
        });
        assert!(editor.paste_table_cells_tsv("x\ty\nz\tw", cx));
        let data = table.read(cx).record.table.as_ref().unwrap();
        assert_eq!(data.rows.len(), 2);
        assert_eq!(data.column_count(), 2);
        assert_eq!(data.rows[0][0].visible_text(), "x");
        assert_eq!(data.rows[0][1].visible_text(), "y");
        assert_eq!(data.rows[1][0].visible_text(), "z");
        assert_eq!(data.rows[1][1].visible_text(), "w");
        assert_eq!(editor.undo_history.len(), 1);
    });
}
