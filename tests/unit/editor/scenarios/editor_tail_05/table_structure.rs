// @author kongweiguang

#[gpui::test]
async fn parsed_table_runtime_installs_column_alignment_on_cells(cx: &mut TestAppContext) {
    let markdown = [
        "| Left | Center | Right |",
        "| :--- | :---: | ---: |",
        "| a | b | c |",
    ]
    .join("\n");
    let editor = cx.new(|cx| Editor::from_markdown(cx, markdown, None));

    editor.read_with(cx, |editor, cx| {
        let table = editor.document.first_root().expect("table root").clone();
        assert_eq!(table.read(cx).kind(), BlockKind::Table);
        let runtime = table
            .read(cx)
            .table_runtime
            .as_ref()
            .expect("table runtime");
        assert_eq!(
            runtime.header[0].read(cx).table_cell_alignment(),
            Some(TableColumnAlignment::Left)
        );
        assert_eq!(
            runtime.header[1].read(cx).table_cell_alignment(),
            Some(TableColumnAlignment::Center)
        );
        assert_eq!(
            runtime.rows[0][2].read(cx).table_cell_alignment(),
            Some(TableColumnAlignment::Right)
        );
    });
}

#[gpui::test]
async fn append_column_updates_table_and_focuses_new_header_cell(cx: &mut TestAppContext) {
    let markdown = ["| A | B |", "| --- | ---: |", "| 1 | 2 |"].join("\n");
    let editor = cx.new(|cx| Editor::from_markdown(cx, markdown, None));

    editor.update(cx, |editor, cx| {
        let table = editor.document.first_root().expect("table root").clone();
        editor.append_table_column(&table, cx);

        let record = table
            .read(cx)
            .record
            .table
            .as_ref()
            .expect("table record after append");
        assert_eq!(record.header.len(), 3);
        assert_eq!(record.rows[0].len(), 3);
        assert_eq!(
            record.alignments,
            vec![
                TableColumnAlignment::Default,
                TableColumnAlignment::Right,
                TableColumnAlignment::Right,
            ]
        );

        let runtime = table
            .read(cx)
            .table_runtime
            .as_ref()
            .expect("rebuilt runtime");
        let focused = runtime.header[2].entity_id();
        assert_eq!(editor.pending_focus, Some(focused));
    });
}

#[gpui::test]
async fn append_row_updates_table_and_focuses_first_cell_of_new_row(cx: &mut TestAppContext) {
    let markdown = ["| A | B |", "| --- | :---: |", "| 1 | 2 |"].join("\n");
    let editor = cx.new(|cx| Editor::from_markdown(cx, markdown, None));

    editor.update(cx, |editor, cx| {
        let table = editor.document.first_root().expect("table root").clone();
        editor.append_table_row(&table, cx);

        let record = table
            .read(cx)
            .record
            .table
            .as_ref()
            .expect("table record after append");
        assert_eq!(record.rows.len(), 2);
        assert_eq!(record.rows[1].len(), 2);
        assert!(
            record.rows[1]
                .iter()
                .all(|cell| cell.serialize_markdown().is_empty())
        );

        let runtime = table
            .read(cx)
            .table_runtime
            .as_ref()
            .expect("rebuilt runtime");
        let focused = runtime.rows[1][0].entity_id();
        assert_eq!(editor.pending_focus, Some(focused));
    });
}

/// Keeps direct structural APIs from mutating a Preview projection or capturing empty undo state.
#[gpui::test]
async fn read_only_table_rejects_direct_structural_edits(cx: &mut TestAppContext) {
    let markdown = ["| A | B |", "| --- | --- |", "| 1 | 2 |"].join("\n");
    let editor = cx.new(|cx| Editor::from_markdown(cx, markdown, None));

    editor.update(cx, |editor, cx| {
        let table = editor.document.first_root().expect("table root").clone();
        editor.set_view_mode(ViewMode::Preview, cx);
        let original = table.read(cx).record.table.clone().expect("table record");
        let source_before = editor.serialized_document_text(cx);
        let revision_before = editor.source_document.revision();
        let dirty_before = editor.document_dirty;
        let undo_before = editor.undo_history.len();
        let runtime = table.read(cx).table_runtime.clone().expect("table runtime");
        runtime.header[0].update(cx, |cell, _cx| {
            cell.record.set_title(InlineTextTree::plain("runtime only"));
        });

        editor.sync_table_record_from_runtime(&table, cx);
        editor.append_table_column(&table, cx);
        editor.append_table_row(&table, cx);
        editor.set_table_column_alignment(&table, 0, TableColumnAlignment::Right, cx);
        editor.insert_table_row(&table, 1, cx);
        editor.duplicate_table_row(&table, 0, cx);
        editor.insert_table_column(&table, 1, cx);
        editor.duplicate_table_column(&table, 0, cx);
        editor.move_table_row(&table, 0, 1, cx);
        editor.move_table_column(&table, 0, 1, cx);
        editor.delete_table_row(&table, 0, cx);
        editor.delete_table_header_row(&table, cx);
        editor.delete_table_column(&table, 0, cx);
        editor.remove_table_block(&table, cx);

        assert_eq!(table.read(cx).record.table.as_ref(), Some(&original));
        assert_eq!(editor.serialized_document_text(cx), source_before);
        assert_eq!(editor.source_document.revision(), revision_before);
        assert_eq!(editor.document_dirty, dirty_before);
        assert_eq!(editor.undo_history.len(), undo_before);
    });
}

/// Refuses stale table append events before they create undo history or dirty the source document.
#[gpui::test]
async fn read_only_table_append_events_leave_editor_state_unchanged(cx: &mut TestAppContext) {
    let markdown = ["| A | B |", "| --- | --- |", "| 1 | 2 |"].join("\n");
    let editor = cx.new(|cx| Editor::from_markdown(cx, markdown, None));

    editor.update(cx, |editor, cx| {
        let table = editor.document.first_root().expect("table root").clone();
        let original = table.read(cx).record.table.clone().expect("table record");
        let source_before = editor.serialized_document_text(cx);
        let revision_before = editor.source_document.revision();
        let dirty_before = editor.document_dirty;
        let undo_before = editor.undo_history.len();
        table.update(cx, |block, _cx| block.set_read_only(true));

        editor.on_block_event(table.clone(), &BlockEvent::RequestAppendTableColumn, cx);
        editor.on_block_event(table.clone(), &BlockEvent::RequestAppendTableRow, cx);

        assert_eq!(table.read(cx).record.table.as_ref(), Some(&original));
        assert_eq!(editor.serialized_document_text(cx), source_before);
        assert_eq!(editor.source_document.revision(), revision_before);
        assert_eq!(editor.document_dirty, dirty_before);
        assert_eq!(editor.undo_history.len(), undo_before);
    });
}

/// Verifies Split Preview rejects table writes while the Source input surface remains editable.
#[gpui::test]
async fn split_preview_table_rejects_writes_while_source_remains_editable(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let source = ["| A | B |", "| --- | --- |", "| 1 | 2 |"].join("\n");
    let (editor, visual_cx) =
        cx.add_window_view(|_window, cx| Editor::from_markdown(cx, source.clone(), None));

    editor.update(visual_cx, |editor, cx| {
        editor.set_view_mode(ViewMode::Split, cx);
    });

    let (markdown_before, revision_before, dirty_before, undo_before) =
        editor.read_with(visual_cx, |editor, cx| {
            (
                editor.serialized_document_text(cx),
                editor.source_document.revision(),
                editor.document_dirty,
                editor.undo_history.len(),
            )
        });

    editor.update(visual_cx, |editor, cx| {
        let preview_table = editor
            .split_preview
            .as_ref()
            .expect("Split Preview")
            .document
            .first_root()
            .expect("Preview table")
            .clone();
        let preview_original = preview_table
            .read(cx)
            .record
            .table
            .clone()
            .expect("Preview table record");

        assert!(preview_table.read(cx).is_read_only());
        editor.append_table_row(&preview_table, cx);
        editor.on_block_event(
            preview_table.clone(),
            &BlockEvent::RequestAppendTableColumn,
            cx,
        );
        editor.on_block_event(
            preview_table.clone(),
            &BlockEvent::RequestAppendTableRow,
            cx,
        );

        assert_eq!(
            preview_table.read(cx).record.table.as_ref(),
            Some(&preview_original)
        );
        assert_eq!(editor.serialized_document_text(cx), markdown_before);
        assert_eq!(editor.source_document.revision(), revision_before);
        assert_eq!(editor.document_dirty, dirty_before);
        assert_eq!(editor.undo_history.len(), undo_before);
    });

    redraw(visual_cx);
    visual_cx.simulate_input("!");
    redraw(visual_cx);
    editor.read_with(visual_cx, |editor, cx| {
        assert_ne!(editor.serialized_document_text(cx), markdown_before);
        assert!(editor.source_document.revision() > revision_before);
        assert!(editor.document_dirty);
        assert_eq!(editor.undo_history.len(), undo_before + 1);
    });
}

#[gpui::test]
async fn setting_column_alignment_updates_record_and_selection(cx: &mut TestAppContext) {
    let markdown = ["| A | B |", "| --- | --- |", "| 1 | 2 |"].join("\n");
    let editor = cx.new(|cx| Editor::from_markdown(cx, markdown, None));

    editor.update(cx, |editor, cx| {
        let table = editor.document.first_root().expect("table root").clone();
        editor.set_table_column_alignment(&table, 1, TableColumnAlignment::Right, cx);

        let record = table.read(cx).record.table.as_ref().expect("table record");
        assert_eq!(
            record.alignments,
            vec![TableColumnAlignment::Default, TableColumnAlignment::Right]
        );
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
async fn moving_table_row_updates_focus_and_selection(cx: &mut TestAppContext) {
    let markdown = ["| A | B |", "| --- | --- |", "| 1 | 2 |", "| 3 | 4 |"].join("\n");
    let editor = cx.new(|cx| Editor::from_markdown(cx, markdown, None));

    editor.update(cx, |editor, cx| {
        let table = editor.document.first_root().expect("table root").clone();
        // Visual row 2 is the second body row; move it up above the first.
        editor.move_table_row(&table, 2, -1, cx);

        let record = table.read(cx).record.table.as_ref().expect("table record");
        assert_eq!(record.rows[0][0].serialize_markdown(), "3");
        assert_eq!(
            editor.table_axis_selection,
            Some(super::TableAxisSelection {
                table_block_id: table.entity_id(),
                kind: crate::components::TableAxisKind::Row,
                index: 1,
            })
        );

        let runtime = table
            .read(cx)
            .table_runtime
            .as_ref()
            .expect("rebuilt runtime");
        assert_eq!(editor.pending_focus, Some(runtime.rows[0][0].entity_id()));
    });
}

#[gpui::test]
async fn moving_first_body_row_up_swaps_with_header(cx: &mut TestAppContext) {
    let markdown = ["| A | B |", "| --- | --- |", "| 1 | 2 |", "| 3 | 4 |"].join("\n");
    let editor = cx.new(|cx| Editor::from_markdown(cx, markdown, None));

    editor.update(cx, |editor, cx| {
        let table = editor.document.first_root().expect("table root").clone();
        // Visual row 1 (first body row) moves up into the header position.
        editor.move_table_row(&table, 1, -1, cx);

        let record = table.read(cx).record.table.as_ref().expect("table record");
        assert_eq!(record.header[0].serialize_markdown(), "1");
        assert_eq!(record.rows[0][0].serialize_markdown(), "A");
        assert_eq!(
            editor.table_axis_selection,
            Some(super::TableAxisSelection {
                table_block_id: table.entity_id(),
                kind: crate::components::TableAxisKind::Row,
                index: 0,
            })
        );
    });
}

#[gpui::test]
async fn moving_header_row_down_swaps_with_first_body(cx: &mut TestAppContext) {
    let markdown = ["| A | B |", "| --- | --- |", "| 1 | 2 |", "| 3 | 4 |"].join("\n");
    let editor = cx.new(|cx| Editor::from_markdown(cx, markdown, None));

    editor.update(cx, |editor, cx| {
        let table = editor.document.first_root().expect("table root").clone();
        // Visual row 0 (header) moves down, swapping with the first body row.
        editor.move_table_row(&table, 0, 1, cx);

        let record = table.read(cx).record.table.as_ref().expect("table record");
        assert_eq!(record.header[0].serialize_markdown(), "1");
        assert_eq!(record.rows[0][0].serialize_markdown(), "A");
        assert_eq!(
            editor.table_axis_selection,
            Some(super::TableAxisSelection {
                table_block_id: table.entity_id(),
                kind: crate::components::TableAxisKind::Row,
                index: 1,
            })
        );
    });
}
