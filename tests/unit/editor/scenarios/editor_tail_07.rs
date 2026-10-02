// @author kongweiguang

#[gpui::test]
async fn live_source_and_preview_share_revision_projection_and_entity_identity(
    cx: &mut TestAppContext,
) {
    init_editor_test_app(cx);
    let source = "# Title\n\nparagraph";
    let (editor, _cx) =
        cx.add_window_view(|_window, cx| Editor::from_markdown(cx, source.to_string(), None));

    let initial_entity = editor.read_with(cx, |editor, cx| {
        let projection = editor
            .projection_cache
            .as_ref()
            .expect("shared projection should exist");
        let block = editor.document.first_root().expect("live root");
        assert_eq!(
            block.read(cx).record.id,
            projection.nodes[0].as_ref().unwrap()[0].record.id
        );
        block.entity_id()
    });

    let edited = "# Updated\n\nparagraph";
    editor.update(cx, |editor, cx| {
        editor.set_view_mode(ViewMode::Source, cx);
        editor.sync_source_document_from_projection(edited);
        editor.set_view_mode(ViewMode::Rendered, cx);
    });

    let rendered_entity = editor.read_with(cx, |editor, cx| {
        let projection = editor
            .projection_cache
            .as_ref()
            .expect("shared projection should exist");
        assert_eq!(projection.revision, editor.source_document.revision());
        assert_eq!(editor.document.markdown_text(cx), edited);
        let block = editor.document.first_root().expect("rendered root");
        assert_eq!(
            block.read(cx).record.id,
            projection.nodes[0].as_ref().unwrap()[0].record.id
        );
        assert_ne!(block.entity_id(), initial_entity);
        block.entity_id()
    });

    editor.update(cx, |editor, cx| editor.set_view_mode(ViewMode::Preview, cx));
    editor.read_with(cx, |editor, cx| {
        let block = editor.document.first_root().expect("preview root");
        assert_eq!(block.entity_id(), rendered_entity);
        assert!(block.read(cx).is_read_only());
    });
    editor.update(cx, |editor, cx| {
        editor.set_view_mode(ViewMode::Rendered, cx)
    });
    editor.read_with(cx, |editor, cx| {
        let block = editor.document.first_root().expect("live root");
        assert_eq!(block.entity_id(), rendered_entity);
        assert!(!block.read(cx).is_read_only());
    });
}

#[gpui::test]
async fn live_projection_cache_refresh_coalesces_without_rebuilding_entities(
    cx: &mut TestAppContext,
) {
    init_editor_test_app(cx);
    let source = (0..40)
        .map(|index| format!("## Section {index}\n\nparagraph {index}"))
        .collect::<Vec<_>>()
        .join("\n\n");
    let (editor, cx) = cx.add_window_view({
        let source = source.clone();
        move |_window, cx| Editor::from_markdown(cx, source, None)
    });
    let tail = editor.read_with(cx, |editor, _cx| {
        editor
            .document
            .visible_blocks()
            .last()
            .unwrap()
            .entity
            .clone()
    });
    let tail_entity_id = tail.entity_id();

    editor.update(cx, |editor, cx| {
        tail.update(cx, |block, _cx| {
            block
                .record
                .set_title(InlineTextTree::from_markdown("paragraph 39 changed"));
        });
        editor.mark_dirty(cx);
        tail.update(cx, |block, _cx| {
            block
                .record
                .set_title(InlineTextTree::from_markdown("paragraph 39 changed twice"));
        });
        editor.mark_dirty(cx);
        assert_eq!(
            editor.projection_cache_scheduled_revision,
            Some(editor.source_document.revision())
        );
        assert!(editor.projection_cache_task.is_some());
    });

    cx.executor().advance_clock(Duration::from_millis(30));
    cx.run_until_parked();

    editor.read_with(cx, |editor, _cx| {
        let cache = editor
            .projection_cache
            .as_ref()
            .expect("projection cache should publish");
        assert_eq!(cache.revision, editor.source_document.revision());
        assert!(cache.source.ends_with("paragraph 39 changed twice"));
        assert!(cache.reused_prefix_regions > 60);
        assert!(editor.projection_cache_task.is_none());
        assert!(editor.projection_cache_scheduled_revision.is_none());
        assert_eq!(
            editor
                .document
                .visible_blocks()
                .last()
                .unwrap()
                .entity
                .entity_id(),
            tail_entity_id
        );
    });
}

#[gpui::test]
async fn entering_split_cancels_pending_live_cache_publication(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let (editor, cx) =
        cx.add_window_view(|_window, cx| Editor::from_markdown(cx, "alpha".to_string(), None));

    editor.update(cx, |editor, cx| {
        let block = editor.document.first_root().unwrap().clone();
        block.update(cx, |block, _cx| {
            block
                .record
                .set_title(InlineTextTree::from_markdown("alpha changed"));
        });
        editor.mark_dirty(cx);
        assert!(editor.projection_cache_task.is_some());
        editor.set_view_mode(ViewMode::Split, cx);
        assert!(editor.projection_cache_task.is_none());
        assert!(editor.projection_cache_scheduled_revision.is_none());
    });
    let installed_cache = editor.read_with(cx, |editor, _cx| {
        let cache = editor.projection_cache.as_ref().unwrap();
        assert_eq!(cache.revision, editor.source_document.revision());
        assert_eq!(
            editor.split_preview.as_ref().unwrap().revision,
            cache.revision
        );
        Arc::as_ptr(cache)
    });

    cx.executor().advance_clock(Duration::from_millis(30));
    cx.run_until_parked();
    editor.read_with(cx, |editor, _cx| {
        assert_eq!(
            Arc::as_ptr(editor.projection_cache.as_ref().unwrap()),
            installed_cache
        );
    });
}

#[gpui::test]
async fn preview_mode_does_not_apply_undo_or_redo(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let (editor, cx) =
        cx.add_window_view(|_window, cx| Editor::from_markdown(cx, "alpha".to_string(), None));

    cx.simulate_input(" beta");
    redraw(cx);
    let edited = editor.read_with(cx, |editor, cx| editor.serialized_document_text(cx));
    assert_ne!(edited, "alpha");

    editor.update(cx, |editor, cx| {
        editor.set_view_mode(ViewMode::Preview, cx);
        editor.undo_document(cx);
        editor.redo_document(cx);
    });

    editor.read_with(cx, |editor, cx| {
        assert_eq!(editor.serialized_document_text(cx), edited);
        assert!(matches!(editor.view_mode, ViewMode::Preview));
    });
}

#[gpui::test]
async fn split_mode_owns_editable_source_and_read_only_preview(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let source = "# Title\n\n| A | B |\n| --- | --- |\n| 1 | 2 |";
    let (editor, _cx) =
        cx.add_window_view(|_window, cx| Editor::from_markdown(cx, source.to_string(), None));

    editor.update(cx, |editor, cx| {
        editor.set_view_mode(ViewMode::Split, cx);
    });

    editor.read_with(cx, |editor, cx| {
        assert!(matches!(editor.view_mode, ViewMode::Split));
        assert_eq!(editor.document.root_count(), 1);
        assert_eq!(editor.document.raw_source_text(cx), source);
        assert!(
            !editor
                .document
                .first_root()
                .unwrap()
                .read(cx)
                .is_read_only()
        );

        let preview = editor
            .split_preview
            .as_ref()
            .expect("split preview state should exist");
        assert_eq!(preview.document.markdown_text(cx), source);
        assert!(!preview.table_cells.is_empty());
        assert!(
            preview
                .document
                .visible_blocks()
                .iter()
                .all(|visible| visible.entity.read(cx).is_read_only())
        );
        assert!(
            preview
                .table_cells
                .values()
                .all(|binding| binding.cell.read(cx).is_read_only())
        );
        assert!(preview.document.visible_blocks().iter().all(|visible| {
            preview
                .source_ranges
                .contains_key(&visible.entity.entity_id())
        }));
    });
}

#[gpui::test]
async fn large_split_preview_virtualizes_scroll_and_incremental_projection(
    cx: &mut TestAppContext,
) {
    init_editor_test_app(cx);
    let source = (0..10_000)
        .map(|index| format!("paragraph {index}"))
        .collect::<Vec<_>>()
        .join("\n\n");
    let expected_suffix = "paragraph 9999";
    let (editor, visual_cx) =
        cx.add_window_view(move |_window, cx| Editor::from_markdown_virtualized(cx, source, None));
    redraw(visual_cx);
    editor.update(visual_cx, |editor, cx| {
        editor.set_view_mode(ViewMode::Split, cx);
    });
    redraw(visual_cx);

    editor.read_with(visual_cx, |editor, cx| {
        let preview = editor.split_preview.as_ref().expect("split preview");
        assert!(preview.virtual_surface.is_some());
        assert!(preview.document.visible_blocks().len() < 200);
        assert!(
            preview
                .document
                .visible_blocks()
                .iter()
                .all(|visible| visible.entity.read(cx).is_read_only())
        );
        assert!(
            editor
                .projection_cache
                .as_ref()
                .expect("projection")
                .nodes
                .iter()
                .all(Option::is_none)
        );
    });

    editor.update(visual_cx, |editor, _cx| {
        let preview = editor.split_preview.as_ref().expect("split preview");
        let max_y = f32::from(preview.scroll_handle.max_offset().height.max(px(0.0)));
        assert!(max_y > 100_000.0);
        preview
            .scroll_handle
            .set_offset(point(px(0.0), px(-max_y * 0.75)));
    });
    redraw(visual_cx);
    editor.read_with(visual_cx, |editor, _cx| {
        let preview = editor.split_preview.as_ref().expect("split preview");
        let surface = preview.virtual_surface.as_ref().expect("virtual surface");
        assert!(surface.top_spacer_height() > 100_000.0);
        assert!(preview.document.visible_blocks().len() < 250);
        assert!(
            preview
                .source_ranges
                .values()
                .all(|range| range.start > 10_000)
        );
        assert!(editor.source_document.text().ends_with(expected_suffix));
    });

    editor.update(visual_cx, |editor, cx| {
        let mut edited = editor.source_document.text();
        edited.push_str(" changed");
        editor.sync_source_document_from_projection(&edited);
        editor.schedule_split_preview_projection(cx);
    });
    flush_split_projection(visual_cx);
    editor.read_with(visual_cx, |editor, _cx| {
        let preview = editor.split_preview.as_ref().expect("split preview");
        assert!(preview.virtual_surface.is_some());
        assert_eq!(preview.revision, editor.source_document.revision());
        assert!(
            editor
                .projection_cache
                .as_ref()
                .expect("projection")
                .nodes
                .iter()
                .all(Option::is_none)
        );
        assert!(
            editor
                .source_document
                .text()
                .ends_with("paragraph 9999 changed")
        );
    });

    editor.update(visual_cx, |editor, cx| {
        editor.set_view_mode(ViewMode::Rendered, cx);
        assert!(editor.virtual_surface.is_some());
        assert!(editor.split_preview.is_none());
    });
}

#[gpui::test]
async fn split_source_edit_rebuilds_preview_and_returns_to_live(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let (editor, cx) =
        cx.add_window_view(|_window, cx| Editor::from_markdown(cx, "alpha".to_string(), None));

    editor.update(cx, |editor, cx| {
        editor.set_view_mode(ViewMode::Split, cx);
    });
    redraw(cx);
    cx.simulate_input(" beta");
    flush_split_projection(cx);

    let edited = editor.read_with(cx, |editor, cx| {
        let source = editor.document.raw_source_text(cx);
        assert_ne!(source, "alpha");
        assert_eq!(editor.source_document.text(), source);
        assert_eq!(
            editor
                .split_preview
                .as_ref()
                .expect("preview should exist")
                .document
                .markdown_text(cx),
            source
        );
        source
    });

    editor.update(cx, |editor, cx| {
        editor.set_view_mode(ViewMode::Rendered, cx);
    });
    editor.read_with(cx, |editor, cx| {
        assert!(matches!(editor.view_mode, ViewMode::Rendered));
        assert!(editor.split_preview.is_none());
        assert_eq!(editor.document.markdown_text(cx), edited);
        assert!(
            editor
                .document
                .visible_blocks()
                .iter()
                .all(|visible| !visible.entity.read(cx).is_read_only())
        );
    });
}

#[gpui::test]
async fn split_scheduler_reuses_cached_region_prefix_for_tail_edit(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let source = (0..40)
        .map(|index| format!("## Section {index}\n\nparagraph {index}"))
        .collect::<Vec<_>>()
        .join("\n\n");
    let (editor, cx) = cx.add_window_view({
        let source = source.clone();
        move |_window, cx| Editor::from_markdown(cx, source, None)
    });
    editor.update(cx, |editor, cx| editor.set_view_mode(ViewMode::Split, cx));
    let (unchanged_entity_id, changed_entity_id) = editor.read_with(cx, |editor, _cx| {
        let visible = editor
            .split_preview
            .as_ref()
            .expect("preview should exist")
            .document
            .visible_blocks();
        (
            visible
                .first()
                .expect("first preview block")
                .entity
                .entity_id(),
            visible
                .last()
                .expect("last preview block")
                .entity
                .entity_id(),
        )
    });
    editor.update(cx, |editor, _cx| {
        let preview = editor.split_preview.as_mut().expect("preview should exist");
        preview.row_stride_cache.insert(unchanged_entity_id, 42.0);
        preview.row_stride_cache.insert(changed_entity_id, 43.0);
    });

    let edited = format!("{source} changed");
    editor.update(cx, |editor, cx| {
        editor.sync_source_document_from_projection(&edited);
        editor.schedule_split_preview_projection(cx);
    });
    flush_split_projection(cx);

    editor.read_with(cx, |editor, cx| {
        let preview = editor.split_preview.as_ref().expect("preview should exist");
        let projection = editor
            .projection_cache
            .as_ref()
            .expect("shared projection should exist");
        assert_eq!(projection.revision, editor.source_document.revision());
        assert!(projection.reused_prefix_regions > 60);
        assert_eq!(preview.document.markdown_text(cx), edited);
        let visible = preview.document.visible_blocks();
        assert_eq!(
            visible.first().unwrap().entity.entity_id(),
            unchanged_entity_id
        );
        assert_ne!(
            visible.last().unwrap().entity.entity_id(),
            changed_entity_id
        );
        assert!(preview.row_stride_cache.contains_key(&unchanged_entity_id));
        assert!(!preview.row_stride_cache.contains_key(&changed_entity_id));
    });
}

#[gpui::test]
async fn split_entity_reuse_rebuilds_table_runtime_and_source_mapping(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let source = "| A | B |\n| --- | --- |\n| 1 | 2 |\n\n## Tail\n\nparagraph";
    let (editor, cx) =
        cx.add_window_view(move |_window, cx| Editor::from_markdown(cx, source.to_string(), None));
    editor.update(cx, |editor, cx| editor.set_view_mode(ViewMode::Split, cx));
    let table_entity_id = editor.read_with(cx, |editor, cx| {
        let preview = editor.split_preview.as_ref().expect("preview should exist");
        assert_eq!(preview.table_cells.len(), 4);
        preview
            .document
            .visible_blocks()
            .iter()
            .find(|visible| visible.entity.read(cx).kind() == BlockKind::Table)
            .expect("preview table")
            .entity
            .entity_id()
    });

    let edited = format!("{source} changed");
    editor.update(cx, |editor, cx| {
        editor.sync_source_document_from_projection(&edited);
        editor.schedule_split_preview_projection(cx);
    });
    flush_split_projection(cx);

    editor.read_with(cx, |editor, cx| {
        let preview = editor.split_preview.as_ref().expect("preview should exist");
        let table = preview
            .document
            .visible_blocks()
            .iter()
            .find(|visible| visible.entity.read(cx).kind() == BlockKind::Table)
            .expect("preview table");
        assert_eq!(table.entity.entity_id(), table_entity_id);
        assert_eq!(preview.table_cells.len(), 4);
        assert!(
            preview
                .table_cells
                .values()
                .all(|binding| binding.cell.read(cx).is_read_only())
        );
        assert!(preview.source_ranges.contains_key(&table_entity_id));
        assert_eq!(preview.document.markdown_text(cx), edited);
    });
}

#[gpui::test]
async fn split_undo_rebuilds_preview_from_restored_source(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let (editor, cx) =
        cx.add_window_view(|_window, cx| Editor::from_markdown(cx, "alpha".to_string(), None));
    editor.update(cx, |editor, cx| editor.set_view_mode(ViewMode::Split, cx));
    redraw(cx);
    cx.simulate_input(" beta");
    flush_split_projection(cx);

    editor.update(cx, |editor, cx| editor.undo_document(cx));
    flush_split_projection(cx);

    editor.read_with(cx, |editor, cx| {
        assert_eq!(editor.document.raw_source_text(cx), "alpha");
        assert_eq!(
            editor
                .split_preview
                .as_ref()
                .expect("preview should exist")
                .document
                .markdown_text(cx),
            "alpha"
        );
    });
}

#[gpui::test]
async fn split_scroll_uses_source_anchor_after_layout(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let source = (0..160)
        .map(|index| format!("## Section {index}\n\nparagraph {index}"))
        .collect::<Vec<_>>()
        .join("\n\n");
    let (editor, cx) =
        cx.add_window_view(move |_window, cx| Editor::from_markdown(cx, source, None));
    editor.update(cx, |editor, cx| editor.set_view_mode(ViewMode::Split, cx));
    redraw(cx);

    editor.read_with(cx, |editor, _cx| {
        let preview = editor.split_preview.as_ref().expect("preview should exist");
        let (start, end) = preview
            .previous_render_window
            .expect("preview render window should be recorded");
        assert!(end.saturating_sub(start) < preview.previous_visible_ids.len());
    });

    editor.update(cx, |editor, cx| {
        editor
            .scroll_handle
            .set_offset(gpui::point(gpui::px(0.0), gpui::px(-320.0)));
        editor
            .split_preview
            .as_mut()
            .expect("preview should exist")
            .scroll_driver = Some(super::SplitScrollDriver::Source);
        editor.sync_split_scroll_handles(cx);
    });

    editor.read_with(cx, |editor, _cx| {
        let preview_y = editor
            .split_preview
            .as_ref()
            .expect("preview should exist")
            .scroll_handle
            .offset()
            .y;
        assert!(preview_y < gpui::px(0.0));
    });
}
#[cfg(target_os = "windows")]
#[path = "editor_tail_07_ime.rs"]
mod ime;
