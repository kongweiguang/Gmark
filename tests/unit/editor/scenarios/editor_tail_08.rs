// @author kongweiguang

#[gpui::test]
async fn virtualized_cross_block_format_is_one_transaction_and_preserves_unmounted_source(
    cx: &mut TestAppContext,
) {
    let source = (0..10_000)
        .map(|index| format!("paragraph {index}"))
        .collect::<Vec<_>>()
        .join("\n\n");
    let original = source.clone();
    let editor = cx.new(move |cx| Editor::from_markdown_virtualized(cx, source, None));

    editor.update(cx, |editor, cx| {
        let visible = editor.document.visible_blocks();
        assert!(visible.len() >= 3);
        editor.cross_block_selection = Some(CrossBlockSelection {
            anchor: CrossBlockSelectionEndpoint {
                entity_id: visible[0].entity.entity_id(),
                offset: 2,
            },
            focus: CrossBlockSelectionEndpoint {
                entity_id: visible[2].entity.entity_id(),
                offset: 2,
            },
            source_anchor: None,
            source_focus: None,
        });
        assert!(editor.apply_cross_block_inline_command(EditingCommandId::Bold, cx));
        assert!(
            editor.source_document.text().starts_with(
                "pa**ragraph 0**\n\n**paragraph 1**\n\n**pa**ragraph 2\n\nparagraph 3"
            )
        );
        assert!(editor.source_document.text().ends_with("paragraph 9999"));
        assert!(editor.cross_block_selection.is_some());
        assert_eq!(editor.virtual_undo_selections.len(), 1);

        editor.undo_document(cx);
        assert_eq!(editor.source_document.text(), original);
        editor.redo_document(cx);
        assert!(
            editor
                .source_document
                .text()
                .starts_with("pa**ragraph 0**\n\n**paragraph 1**\n\n**pa**ragraph 2")
        );
    });
}

#[gpui::test]
/// Keeps tables atomic when deletion spans virtual regions and verifies the undo source boundary.
async fn virtualized_cross_region_delete_treats_table_as_atomic_and_restores_on_undo(
    cx: &mut TestAppContext,
) {
    let tail = (0..10_000)
        .map(|index| format!("paragraph {index}"))
        .collect::<Vec<_>>()
        .join("\n\n");
    let source = format!("alpha\n\n| a | b |\n| --- | --- |\n| 1 | 2 |\n\ngamma\n\n{tail}");
    let original = source.clone();
    let editor = cx.new(move |cx| Editor::from_markdown_virtualized(cx, source, None));

    editor.update(cx, |editor, cx| {
        let visible = editor.document.visible_blocks();
        assert!(visible.len() >= 3);
        assert_eq!(visible[1].entity.read(cx).kind(), BlockKind::Table);
        let alpha_len = visible[0].entity.read(cx).visible_len();
        editor.cross_block_selection = Some(CrossBlockSelection {
            anchor: CrossBlockSelectionEndpoint {
                entity_id: visible[0].entity.entity_id(),
                offset: alpha_len,
            },
            focus: CrossBlockSelectionEndpoint {
                entity_id: visible[1].entity.entity_id(),
                offset: 0,
            },
            source_anchor: None,
            source_focus: None,
        });
        assert!(editor.replace_cross_block_selection_with_text(
            "",
            None,
            false,
            crate::components::UndoCaptureKind::NonCoalescible,
            cx,
        ));
        let deleted = editor.source_document.text();
        assert!(!deleted.contains("| a | b |"));
        assert!(deleted.starts_with("alpha\n\ngamma"));
        assert!(deleted.ends_with("paragraph 9999"));

        editor.undo_document(cx);
        assert_eq!(editor.source_document.text(), original);
    });
}

#[gpui::test]
async fn large_document_mode_round_trip_restores_virtual_surface_and_source_selection(
    cx: &mut TestAppContext,
) {
    let source = (0..10_000)
        .map(|index| format!("paragraph {index}"))
        .collect::<Vec<_>>()
        .join("\n\n");
    let expected = source.clone();
    let editor = cx.new(move |cx| Editor::from_markdown_virtualized(cx, source, None));

    editor.update(cx, |editor, cx| {
        let first = editor.document.first_root().expect("first root").clone();
        first.update(cx, |block, _cx| {
            block.selected_range = 4..4;
        });
        editor.active_entity_id = Some(first.entity_id());

        editor.set_view_mode(ViewMode::Source, cx);
        assert!(editor.virtual_surface.is_none());
        assert_eq!(editor.document.raw_source_text(cx), expected);
        assert_eq!(
            editor
                .document
                .first_root()
                .expect("source root")
                .read(cx)
                .selected_range,
            4..4
        );

        editor.set_view_mode(ViewMode::Rendered, cx);
        assert!(editor.virtual_surface.is_some());
        assert!(editor.document.visible_blocks().len() < 200);

        editor.set_view_mode(ViewMode::Split, cx);
        assert!(editor.virtual_surface.is_none());
        assert!(editor.split_preview.is_some());
        editor.set_view_mode(ViewMode::Rendered, cx);
        assert!(editor.virtual_surface.is_some());
        assert!(editor.split_preview.is_none());
        assert!(editor.document.visible_blocks().len() < 200);

        editor.set_view_mode(ViewMode::Preview, cx);
        assert!(editor.virtual_surface.is_some());
        editor.set_view_mode(ViewMode::Source, cx);
        editor.set_view_mode(ViewMode::Preview, cx);
        assert!(editor.virtual_surface.is_some());
        assert_eq!(editor.source_document.text(), expected);
    });
}

#[gpui::test]
async fn virtualized_reference_definition_edit_rebuilds_global_runtime_from_full_rope(
    cx: &mut TestAppContext,
) {
    let body = (0..10_000)
        .map(|index| format!("paragraph {index}"))
        .collect::<Vec<_>>()
        .join("\n\n");
    let source = format!("[ref]: old.png\n\n{body}\n\n![tail][ref]");
    let editor = cx.new(move |cx| Editor::from_markdown(cx, source, None));
    editor.update(cx, |editor, cx| {
        let definition = editor.document.first_root().expect("definition").clone();
        let text = definition.read(cx).display_text().to_string();
        let start = text.find("old.png").expect("old target");
        definition.update(cx, |block, cx| {
            block.prepare_undo_capture(crate::components::UndoCaptureKind::CoalescibleText, cx);
            block.replace_text_in_visible_range(
                start..start + "old.png".len(),
                "new.png",
                None,
                false,
                cx,
            );
        });
    });
    editor.read_with(cx, |editor, _cx| {
        let definition = editor
            .image_reference_definitions
            .get("ref")
            .expect("global definition");
        assert_eq!(definition.src, "new.png");
        let source = editor.source_document.text();
        assert!(source.starts_with("[ref]: new.png"));
        assert!(source.ends_with("![tail][ref]"));
    });
}

#[gpui::test]
async fn virtualized_footnote_ordinals_follow_full_source_order_with_offscreen_definitions(
    cx: &mut TestAppContext,
) {
    let body = (0..10_000)
        .map(|index| format!("paragraph {index}"))
        .collect::<Vec<_>>()
        .join("\n\n");
    let source = format!("intro [^b]\n\n{body}\n\nlater [^a]\n\n[^a]: alpha\n\n[^b]: beta");
    let editor = cx.new(move |cx| Editor::from_markdown(cx, source, None));
    editor.read_with(cx, |editor, cx| {
        let surface = editor.virtual_surface.as_ref().expect("virtual surface");
        assert_eq!(surface.footnote_ordinal("b"), Some(1));
        assert_eq!(surface.footnote_ordinal("a"), Some(2));
        assert!(surface.footnote_definition_region("b").is_some());

        let first = editor.document.first_root().expect("first root");
        let occurrences = editor
            .footnote_registry
            .occurrences_for_block(first.read(cx).record.id)
            .expect("mounted occurrence");
        assert_eq!(occurrences.len(), 1);
        assert_eq!(occurrences[0].id, "b");
        assert_eq!(occurrences[0].ordinal, Some(1));
    });
}

#[gpui::test]
async fn virtualized_footnote_jump_mounts_and_focuses_offscreen_definition(
    cx: &mut TestAppContext,
) {
    init_editor_test_app(cx);
    let body = (0..10_000)
        .map(|index| format!("paragraph {index}"))
        .collect::<Vec<_>>()
        .join("\n\n");
    let source = format!("intro [^note]\n\n{body}\n\n[^note]: definition");
    let (editor, visual_cx) =
        cx.add_window_view(move |_window, cx| Editor::from_markdown(cx, source, None));
    redraw(visual_cx);
    editor.update(visual_cx, |editor, cx| {
        let source_block = editor.document.first_root().expect("reference").clone();
        editor.on_block_event(
            source_block,
            &crate::components::BlockEvent::RequestJumpToFootnoteDefinition {
                id: "note".to_string(),
            },
            cx,
        );
        assert_eq!(
            editor.pending_virtual_footnote_focus.as_deref(),
            Some("note")
        );
    });
    redraw(visual_cx);

    editor.read_with(visual_cx, |editor, cx| {
        assert!(editor.pending_virtual_footnote_focus.is_none());
        let active = editor.active_entity_id.expect("focused definition");
        let definition = editor
            .focusable_entity_by_id(active)
            .expect("mounted definition");
        assert_eq!(definition.read(cx).kind(), BlockKind::FootnoteDefinition);
        assert_eq!(definition.read(cx).display_text(), "note");
        assert!(f32::from(editor.scroll_handle.offset().y) < -100_000.0);
    });

    editor.update(visual_cx, |editor, cx| {
        let definition = editor
            .active_entity_id
            .and_then(|entity_id| editor.focusable_entity_by_id(entity_id))
            .expect("focused definition");
        editor.on_block_event(
            definition,
            &crate::components::BlockEvent::RequestJumpToFootnoteBackref {
                id: "note".to_string(),
            },
            cx,
        );
    });
    redraw(visual_cx);

    editor.read_with(visual_cx, |editor, cx| {
        assert!(editor.pending_virtual_footnote_backref_focus.is_none());
        let active = editor.active_entity_id.expect("focused first reference");
        let reference = editor
            .focusable_entity_by_id(active)
            .expect("mounted first reference");
        assert!(reference.read(cx).display_text().contains("note"));
        assert!(f32::from(editor.scroll_handle.offset().y) > -1_000.0);
    });
}

#[gpui::test]
async fn targeted_source_mapping_matches_full_document_mapping(cx: &mut TestAppContext) {
    let source = "# first\n\n- one\n- two\n\n\nparagraph\n\n> quote\n\n[^n]: note";
    let editor = cx.new(|cx| Editor::from_markdown(cx, source.to_string(), None));
    editor.read_with(cx, |editor, cx| {
        for expected in editor.build_source_target_mappings(cx) {
            let actual = editor
                .build_source_target_mapping_for_entity(expected.entity.entity_id(), cx)
                .expect("targeted mapping");
            assert_eq!(actual.full_source_range, expected.full_source_range);
            assert_eq!(actual.content_to_source, expected.content_to_source);
            assert_eq!(actual.source_to_content, expected.source_to_content);
        }
    });
}

fn prepared_node_estimated_bytes(node: &super::projection::PreparedBlockNode) -> usize {
    let record = &node.record;
    let mut bytes = std::mem::size_of_val(node)
        + record.title.serialize_markdown().len()
        + record.raw_fallback.as_ref().map_or(0, String::capacity);
    if let Some(table) = record.table.as_ref() {
        bytes += table
            .header
            .iter()
            .chain(table.rows.iter().flatten())
            .map(|cell| cell.serialize_markdown().len())
            .sum::<usize>();
    }
    if let Some(html) = record.html.as_ref() {
        bytes += html.raw_source.capacity();
    }
    bytes
        + node
            .children
            .iter()
            .map(prepared_node_estimated_bytes)
            .sum::<usize>()
}

fn projection_estimated_bytes(prepared: &PreparedSplitProjection) -> usize {
    std::mem::size_of_val(prepared)
        + prepared.source.capacity()
        + prepared.lines.iter().map(String::capacity).sum::<usize>()
        + prepared.regions.capacity() * std::mem::size_of::<super::projection::ProjectionRegion>()
        + prepared
            .nodes
            .iter()
            .flatten()
            .flat_map(|nodes| nodes.iter())
            .map(prepared_node_estimated_bytes)
            .sum::<usize>()
}

#[test]
#[ignore = "release-only performance benchmark; run with --release --ignored --nocapture"]
fn projection_pipeline_release_benchmark() {
    const SAMPLES: usize = 30;
    for target_mib in [1usize, 10] {
        let source = mixed_projection_fixture(target_mib * 1024 * 1024);
        let document = gmark_document::SourceDocument::new(&source);
        let mut full_samples = Vec::with_capacity(SAMPLES);
        let mut last_full = None;
        for _ in 0..SAMPLES {
            let started = Instant::now();
            let prepared = PreparedSplitProjection::from_snapshot(document.snapshot());
            full_samples.push(started.elapsed().as_micros());
            std::hint::black_box(prepared.regions.len());
            last_full = Some(prepared);
        }

        let previous = last_full.expect("benchmark should prepare at least once");
        let edited = format!("{source}\n\n## Tail\n\nchanged 中文 🚀");
        let edited_document = gmark_document::SourceDocument::new(&edited);
        let mut incremental_samples = Vec::with_capacity(SAMPLES);
        let mut last_incremental = None;
        for _ in 0..SAMPLES {
            let started = Instant::now();
            let prepared = PreparedSplitProjection::from_snapshot_incremental(
                edited_document.snapshot(),
                &previous,
            );
            incremental_samples.push(started.elapsed().as_micros());
            std::hint::black_box(prepared.regions.len());
            last_incremental = Some(prepared);
        }

        let incremental = last_incremental.expect("incremental benchmark should run");
        let reuse_percent =
            incremental.reused_prefix_regions as f64 / previous.regions.len().max(1) as f64 * 100.0;
        let estimated_mib = projection_estimated_bytes(&previous) as f64 / (1024.0 * 1024.0);
        let full_p50 = percentile_micros(&mut full_samples, 50) as f64 / 1000.0;
        let full_p95 = percentile_micros(&mut full_samples, 95) as f64 / 1000.0;
        let incremental_p50 = percentile_micros(&mut incremental_samples, 50) as f64 / 1000.0;
        let incremental_p95 = percentile_micros(&mut incremental_samples, 95) as f64 / 1000.0;
        println!(
            "projection_benchmark size_bytes={} regions={} full_p50_ms={full_p50:.3} full_p95_ms={full_p95:.3} incremental_p50_ms={incremental_p50:.3} incremental_p95_ms={incremental_p95:.3} reused_regions_pct={reuse_percent:.2} estimated_ir_mib={estimated_mib:.2}",
            source.len(),
            previous.regions.len(),
        );

        assert_eq!(incremental.source, edited);
        assert!(reuse_percent > 95.0);
        assert_eq!(incremental.regions.len(), incremental.nodes.len());
    }
}

#[test]
#[ignore = "10 MiB stress test; run explicitly in release mode"]
fn projection_pipeline_10mib_stress_preserves_utf8_ranges() {
    let source = mixed_projection_fixture(10 * 1024 * 1024);
    let prepared = PreparedSplitProjection::from_snapshot(
        gmark_document::SourceDocument::new(&source).snapshot(),
    );
    assert_eq!(prepared.source, source);
    assert_eq!(prepared.regions.len(), prepared.nodes.len());
    assert!(prepared.regions.len() > 10_000);
    for region in &prepared.regions {
        assert!(prepared.source.is_char_boundary(region.bytes.start));
        assert!(prepared.source.is_char_boundary(region.bytes.end));
        assert_eq!(
            &prepared.source[region.bytes.clone()],
            prepared.lines[region.lines.clone()].join("\n")
        );
    }
}

#[test]
fn incremental_projection_rescans_previous_region_when_block_marker_disappears() {
    let previous = "intro\n\n## Heading\nfollowing";
    let current = "intro\n\nHeading\nfollowing";
    let prepared = assert_incremental_projection_matches_full(previous, current);
    assert_eq!(
        prepared
            .regions
            .iter()
            .map(|region| region.kind)
            .collect::<Vec<_>>(),
        [
            ProjectionRegionKind::Paragraph,
            ProjectionRegionKind::Blank,
            ProjectionRegionKind::Paragraph,
        ]
    );
}

#[test]
fn incremental_projection_keeps_utf8_ranges_after_multibyte_edit() {
    let previous = "# 标题\n\n第一段\n\n第二段旧值";
    let current = "# 标题\n\n第一段\n\n第二段新值";
    let prepared = assert_incremental_projection_matches_full(previous, current);
    for region in &prepared.regions {
        assert!(prepared.source.is_char_boundary(region.bytes.start));
        assert!(prepared.source.is_char_boundary(region.bytes.end));
        assert_eq!(
            &prepared.source[region.bytes.clone()],
            prepared.lines[region.lines.clone()].join("\n")
        );
    }
}

/// Equal-height rows as per-row footprints, the input `rendered_window` takes.
fn uniform_strides(count: usize, height: f32) -> Vec<f32> {
    vec![height; count]
}

#[test]
fn rendered_window_culls_offscreen_rows() {
    // 100 rows of 50px (total 5000). Scroll 2000, viewport 400 -> band [2000, 2400].
    let strides = uniform_strides(100, 50.0);
    let window = Editor::rendered_window(&strides, 2000.0, 400.0, 0.0, None);

    // Row i spans [50i, 50i+50). bottom>=2000 -> i>=39; top<=2400 -> i<=48.
    assert_eq!(window.run_start, 39);
    assert_eq!(window.run_end, 49);
    assert!((window.top_h - 1950.0).abs() < 0.01);
    assert!((window.bottom_h - 2550.0).abs() < 0.01);
}

#[test]
fn rendered_window_keeps_focus_row_mounted() {
    let strides = uniform_strides(100, 50.0);
    // Viewport at the top, caret parked far below at row 80.
    let window = Editor::rendered_window(&strides, 0.0, 400.0, 0.0, Some(80));

    assert_eq!(window.run_start, 0);
    assert_eq!(window.run_end, 81);
}

#[test]
fn rendered_window_tracks_current_scroll_offset() {
    // Scrolling by one row's height shifts the mounted run by exactly one row.
    let strides = uniform_strides(100, 50.0);

    let low = Editor::rendered_window(&strides, 2000.0, 400.0, 0.0, None);
    let high = Editor::rendered_window(&strides, 2050.0, 400.0, 0.0, None);

    assert_eq!(low.run_start, 39);
    assert_eq!(low.run_end, 49);
    assert_eq!(high.run_start, low.run_start + 1);
    assert_eq!(high.run_end, low.run_end + 1);
}

#[test]
fn rendered_window_has_no_spacer_at_document_edges() {
    let strides = uniform_strides(50, 40.0); // total 2000

    let at_top = Editor::rendered_window(&strides, 0.0, 400.0, 0.0, None);
    assert_eq!(at_top.run_start, 0);
    assert_eq!(at_top.top_h, 0.0);
    assert!(at_top.bottom_h > 0.0);

    let at_bottom = Editor::rendered_window(&strides, 1600.0, 400.0, 0.0, None);
    assert_eq!(at_bottom.run_end, 50);
    assert_eq!(at_bottom.bottom_h, 0.0);
    assert!(at_bottom.top_h > 0.0);
}

#[test]
fn rendered_window_preserves_total_height() {
    let strides = uniform_strides(200, 37.0);
    let total: f32 = strides.iter().sum();

    for &(scroll_y, viewport_height, focus) in &[
        (0.0f32, 500.0f32, None),
        (3000.0, 500.0, None),
        (37.0 * 150.0, 37.0 * 5.0, Some(10usize)),
    ] {
        let window = Editor::rendered_window(&strides, scroll_y, viewport_height, 200.0, focus);
        let rendered: f32 = strides[window.run_start..window.run_end].iter().sum();
        assert!(
            (window.top_h + rendered + window.bottom_h - total).abs() < 0.01,
            "height invariant broken at scroll {scroll_y}"
        );
    }
}

#[test]
fn restored_deep_scroll_includes_unmeasured_rows_instead_of_only_the_last_row() {
    let strides = uniform_strides(100, 20.0);
    let estimated = Editor::rendered_window(&strides, 5_000.0, 400.0, 0.0, None);
    assert_eq!(estimated.run_start, 99, "低估总高会触发末行兜底");

    let window = Editor::include_render_measurement_frontier(estimated, &strides, 10);
    assert_eq!(window.run_start, 10);
    assert_eq!(window.run_end, 100);
    assert_eq!(window.top_h, 200.0);
    let rendered: f32 = strides[window.run_start..window.run_end].iter().sum();
    assert_eq!(window.top_h + rendered + window.bottom_h, 2_000.0);
}

#[test]
fn ordinary_deep_scroll_does_not_expand_to_the_measurement_frontier() {
    let strides = uniform_strides(500, 20.0);
    let window =
        Editor::rendered_document_window(&strides, 7_500.0, 400.0, 200.0, 0, false, 128);

    assert!(window.run_start > 0);
    assert!(window.run_end - window.run_start < 128);
}

#[test]
fn small_document_keeps_all_rows_mounted_at_a_deep_scroll_offset() {
    let strides = uniform_strides(100, 20.0);
    let window =
        Editor::rendered_document_window(&strides, 5_000.0, 400.0, 0.0, 0, false, 512);

    assert_eq!(window.run_start, 0);
    assert_eq!(window.run_end, strides.len());
    assert_eq!(window.top_h, 0.0);
    assert_eq!(window.bottom_h, 0.0);
}

#[test]
fn rendered_window_estimated_row_keeps_culling_active() {
    // Row 60 is an estimated (unmeasured) row; it must not disable culling.
    let mut strides = uniform_strides(100, 50.0);
    strides[60] = 20.0;

    let window = Editor::rendered_window(&strides, 0.0, 400.0, 0.0, None);
    assert_eq!(window.run_start, 0);
    assert!(
        window.run_end < strides.len(),
        "a single estimated row must not disable culling"
    );
}

#[test]
fn rendered_window_all_estimated_windows_near_top() {
    // Cold start: all rows estimated. At the top the window still covers the
    // first rows, so the viewport is never blank while heights are learned.
    let strides = uniform_strides(500, 20.0);

    let window = Editor::rendered_window(&strides, 0.0, 400.0, 0.0, None);
    assert_eq!(window.run_start, 0);
    assert!(window.run_end < strides.len());
    // A viewport-plus-band worth of rows, not the whole document.
    assert!(window.run_end >= 20);
}

#[test]
fn about_dialog_body_lines_include_repository_without_star_message() {
    let strings = I18nStrings::zh_cn();
    let lines = Editor::about_dialog_body_lines(&strings);

    // The dialog and native window title share the product's title-case brand,
    // so this contract catches accidental casing drift in the about surface.
    assert_eq!(lines[0], format!("Gmark {}", env!("CARGO_PKG_VERSION")));
    assert_eq!(lines[1], "作者：kongweiguang\n版权所有 © 2026 kongweiguang");
    assert_eq!(
        lines[2],
        format!("GitHub: {}", super::render::ABOUT_GITHUB_URL)
    );
    assert_eq!(lines.len(), 3);
}

#[gpui::test]
async fn about_github_link_uses_gpui_url_opening(cx: &mut TestAppContext) {
    cx.update(|cx| {
        super::render::open_about_github_url(cx);
    });

    assert_eq!(
        cx.opened_url(),
        Some(super::render::ABOUT_GITHUB_URL.to_string())
    );
}

#[gpui::test]
async fn ctrl_s_saves_rendered_mode_edit_to_existing_file(cx: &mut TestAppContext) {
    init_editor_test_app(cx);

    let path = temp_markdown_path("ctrl-s-rendered-save");
    fs::write(&path, "alpha").expect("write initial markdown");
    let cleanup_path = path.clone();
    cx.on_quit(move || {
        let _ = fs::remove_file(&cleanup_path);
    });

    let (editor, cx) = cx.add_window_view({
        let path = path.clone();
        move |_window, cx| Editor::from_markdown(cx, "alpha".to_string(), Some(path))
    });

    cx.simulate_input("!");
    redraw(cx);
    let expected = editor.read_with(cx, |editor, cx| {
        assert!(editor.document_dirty);
        assert!(!editor.pending_save);
        editor.document.markdown_text(cx)
    });
    assert_ne!(expected, "alpha");

    cx.simulate_keystrokes("ctrl-s");
    redraw(cx);

    assert_eq!(
        fs::read_to_string(&path).expect("read saved markdown"),
        expected
    );
    editor.read_with(cx, |editor, _cx| {
        assert!(!editor.document_dirty);
        assert!(!editor.pending_save);
    });
}

#[gpui::test]
async fn source_document_preserves_loaded_markdown_spelling(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let source = "1) one\n2) two";
    let (editor, _cx) =
        cx.add_window_view(|_window, cx| Editor::from_markdown(cx, source.to_string(), None));

    editor.read_with(cx, |editor, cx| {
        assert_eq!(editor.document.markdown_text(cx), "1. one\n2. two");
        assert_eq!(editor.serialized_document_text(cx), source);
    });
}

#[gpui::test]
async fn source_view_uses_source_document_without_normalizing_markers(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let source = "1) one\n2) two";
    let (editor, _cx) =
        cx.add_window_view(|_window, cx| Editor::from_markdown(cx, source.to_string(), None));

    editor.update(cx, |editor, cx| editor.toggle_view_mode(cx));

    editor.read_with(cx, |editor, cx| {
        assert!(matches!(editor.view_mode, ViewMode::Source));
        assert_eq!(editor.document.raw_source_text(cx), source);
        assert_eq!(editor.source_document.text(), source);
    });
}

#[gpui::test]
async fn preview_mode_is_read_only_and_live_mode_restores_editing(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let source = "# Title\n\nbody";
    let (editor, cx) =
        cx.add_window_view(|_window, cx| Editor::from_markdown(cx, source.to_string(), None));

    editor.update(cx, |editor, cx| {
        editor.set_view_mode(ViewMode::Preview, cx);
    });
    redraw(cx);
    editor.read_with(cx, |editor, cx| {
        assert!(matches!(editor.view_mode, ViewMode::Preview));
        assert!(
            editor
                .document
                .visible_blocks()
                .iter()
                .all(|visible| visible.entity.read(cx).is_read_only())
        );
        assert_eq!(editor.serialized_document_text(cx), source);
        assert!(!editor.document_dirty);
    });

    cx.simulate_input(" must-not-appear");
    redraw(cx);
    editor.read_with(cx, |editor, cx| {
        assert_eq!(editor.serialized_document_text(cx), source);
        assert!(!editor.document_dirty);
    });

    editor.update(cx, |editor, cx| {
        editor.set_view_mode(ViewMode::Rendered, cx);
    });
    redraw(cx);
    editor.read_with(cx, |editor, cx| {
        assert!(matches!(editor.view_mode, ViewMode::Rendered));
        assert!(
            editor
                .document
                .visible_blocks()
                .iter()
                .all(|visible| !visible.entity.read(cx).is_read_only())
        );
    });

    cx.simulate_input(" edited");
    redraw(cx);
    editor.read_with(cx, |editor, cx| {
        assert!(editor.document_dirty);
        assert_ne!(editor.serialized_document_text(cx), source);
    });
}

#[gpui::test]
async fn preview_mode_makes_native_table_cells_read_only(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let source = "| A | B |\n| --- | --- |\n| 1 | 2 |";
    let (editor, _cx) =
        cx.add_window_view(|_window, cx| Editor::from_markdown(cx, source.to_string(), None));

    editor.update(cx, |editor, cx| {
        assert!(!editor.table_cells.is_empty());
        editor.set_view_mode(ViewMode::Preview, cx);
    });

    editor.read_with(cx, |editor, cx| {
        assert!(
            editor
                .table_cells
                .values()
                .all(|binding| binding.cell.read(cx).is_read_only())
        );
        assert_eq!(editor.serialized_document_text(cx), source);
    });
}

#[gpui::test]
async fn source_to_preview_builds_read_only_rendered_projection(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let source = "1) one\n2) two";
    let (editor, _cx) =
        cx.add_window_view(|_window, cx| Editor::from_markdown(cx, source.to_string(), None));

    editor.update(cx, |editor, cx| {
        editor.set_view_mode(ViewMode::Source, cx);
        editor.set_view_mode(ViewMode::Preview, cx);
    });

    editor.read_with(cx, |editor, cx| {
        assert!(matches!(editor.view_mode, ViewMode::Preview));
        assert_eq!(editor.document.markdown_text(cx), "1. one\n2. two");
        assert_eq!(editor.serialized_document_text(cx), source);
        assert!(
            editor
                .document
                .visible_blocks()
                .iter()
                .all(|visible| visible.entity.read(cx).is_read_only())
        );
    });
}
