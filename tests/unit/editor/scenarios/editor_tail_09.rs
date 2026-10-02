// @author kongweiguang

#[test]
fn resource_links_inside_lists_and_callouts_keep_their_projection_container() {
    let source = concat!(
        "- [spec](./spec.pdf \"gmark:resource\")\n\n",
        "> [!NOTE] [clip](./clip.mp4 \"gmark:resource;type=video\")"
    );
    let prepared = PreparedSplitProjection::from_snapshot(
        gmark_document::SourceDocument::new(source).snapshot(),
    );

    assert_eq!(prepared.regions[0].kind, ProjectionRegionKind::List);
    assert_eq!(prepared.regions[2].kind, ProjectionRegionKind::Quote);
    let list_node = &prepared.nodes[0].as_ref().expect("list node")[0].record;
    let callout_node = &prepared.nodes[2].as_ref().expect("callout node")[0].record;
    assert!(list_node.resource.is_some());
    assert_eq!(list_node.kind, BlockKind::BulletedListItem);
    assert!(callout_node.resource.is_some());
    assert!(matches!(callout_node.kind, BlockKind::Callout(_)));
}

#[test]
fn incremental_projection_reuses_unchanged_region_prefix_for_tail_edit() {
    let previous = "# A\n\nalpha\n\n## B\n\nbeta\n\n## C\n\n中文尾部";
    let current = "# A\n\nalpha\n\n## B\n\nbeta\n\n## C\n\n中文结尾";
    let old = PreparedSplitProjection::from_snapshot(
        gmark_document::SourceDocument::new(previous).snapshot(),
    );
    let prepared = PreparedSplitProjection::from_snapshot_incremental(
        gmark_document::SourceDocument::new(current).snapshot(),
        &old,
    );
    let full = PreparedSplitProjection::from_snapshot(
        gmark_document::SourceDocument::new(current).snapshot(),
    );
    assert_eq!(prepared.regions, full.regions);
    assert!(prepared.reused_prefix_regions >= 4);
    for index in 0..prepared.reused_prefix_regions {
        let old_ids = old.nodes[index]
            .as_ref()
            .map(|nodes| nodes.iter().map(|node| node.record.id).collect::<Vec<_>>());
        let new_ids = prepared.nodes[index]
            .as_ref()
            .map(|nodes| nodes.iter().map(|node| node.record.id).collect::<Vec<_>>());
        assert_eq!(new_ids, old_ids);
    }
}

#[test]
fn prepared_projection_builds_non_recursive_semantics_before_ui_installation() {
    let source = "# **Title**\n\nparagraph with *style*\n\n```rust\nfn main() {}\n```\n\n| A | B |\n| --- | --- |\n| 1 | 2 |\n\n<div>safe</div>\n\n$$\nx + y\n$$";
    let prepared = PreparedSplitProjection::from_snapshot(
        gmark_document::SourceDocument::new(source).snapshot(),
    );
    for (region, nodes) in prepared.regions.iter().zip(&prepared.nodes) {
        if region.kind == ProjectionRegionKind::Blank {
            assert!(nodes.is_none());
        } else {
            assert!(
                nodes.as_ref().is_some_and(|nodes| !nodes.is_empty()),
                "missing prepared semantics for {:?}",
                region.kind
            );
        }
    }

    let table = prepared
        .nodes
        .iter()
        .flatten()
        .flat_map(|nodes| nodes.iter())
        .find(|node| node.record.kind == BlockKind::Table)
        .expect("table semantics should be prepared");
    assert!(table.record.table.is_some());
    let html = prepared
        .nodes
        .iter()
        .flatten()
        .flat_map(|nodes| nodes.iter())
        .find(|node| node.record.kind == BlockKind::HtmlBlock)
        .expect("HTML semantics should be prepared");
    assert!(html.record.html.is_some());
}

#[test]
fn prepared_projection_keeps_section_html_as_native_html() {
    let source = "# title\n\n<section><h2>Heading</h2><p><strong>safe</strong></p></section>";
    let prepared = PreparedSplitProjection::from_snapshot(
        gmark_document::SourceDocument::new(source).snapshot(),
    );
    let html = prepared
        .nodes
        .iter()
        .flatten()
        .flat_map(|nodes| nodes.iter())
        .find(|node| node.record.kind == BlockKind::HtmlBlock)
        .expect("section should be a native HTML block");
    assert!(html.record.html.as_ref().is_some_and(|html| html.is_semantic()));
}

#[test]
fn prepared_projection_builds_common_recursive_blocks_as_pure_nodes() {
    let source = "- one\n  - nested\n- two\n\n> quoted line\n\n> [!NOTE] Title\n> callout body\n\n[^n]: footnote *body*";
    let prepared = PreparedSplitProjection::from_snapshot(
        gmark_document::SourceDocument::new(source).snapshot(),
    );
    let semantic_nodes = prepared
        .regions
        .iter()
        .zip(&prepared.nodes)
        .filter(|(region, _)| region.kind != ProjectionRegionKind::Blank)
        .map(|(_, nodes)| {
            nodes
                .as_ref()
                .expect("common recursive block should prepare")
        })
        .collect::<Vec<_>>();

    assert_eq!(semantic_nodes[0].len(), 2);
    assert!(
        semantic_nodes[0]
            .iter()
            .all(|node| node.record.kind.is_list_item())
    );
    assert_eq!(semantic_nodes[0][0].children.len(), 1);
    assert_eq!(semantic_nodes[1][0].record.kind, BlockKind::Quote);
    assert_eq!(
        semantic_nodes[2][0].record.kind,
        BlockKind::Callout(crate::components::CalloutVariant::Note)
    );
    assert_eq!(semantic_nodes[2][0].children.len(), 1);
    assert_eq!(
        semantic_nodes[3][0].record.kind,
        BlockKind::FootnoteDefinition
    );
    assert_eq!(semantic_nodes[3][0].children.len(), 1);
}

#[test]
fn rendered_find_uses_visible_text_and_marks_derived_matches_nonreplaceable() {
    let result = super::find_replace::find_matches_for_view(
        "[label](https://example.test) &amp;\n\n```mermaid\nflowchart LR\nA-->B\n```",
        "example",
        super::find_replace::FindOptions::default(),
        gmark_document::Revision::INITIAL,
        true,
    );
    assert!(result.matches.is_empty(), "link destinations are hidden");

    let result = super::find_replace::find_matches_for_view(
        "[label](https://example.test) &amp;\n\n```mermaid\nflowchart LR\nA-->B\n```",
        "flowchart",
        super::find_replace::FindOptions::default(),
        gmark_document::Revision::INITIAL,
        true,
    );
    assert_eq!(result.matches.len(), 1);
    assert_eq!(result.match_metadata[0].replaceability, gmark_markdown::Replaceability::Derived);
}

#[test]
fn prepared_projection_defers_complex_recursive_blocks_to_entity_builder() {
    let source = "- parent\n  continuation\n\n> quote\n> | A | B |\n> | --- | --- |";
    let prepared = PreparedSplitProjection::from_snapshot(
        gmark_document::SourceDocument::new(source).snapshot(),
    );
    let deferred_kinds = prepared
        .regions
        .iter()
        .zip(&prepared.nodes)
        .filter_map(|(region, nodes)| nodes.is_none().then_some(region.kind))
        .collect::<Vec<_>>();
    assert!(deferred_kinds.contains(&ProjectionRegionKind::List));
    assert!(deferred_kinds.contains(&ProjectionRegionKind::Quote));
}

#[gpui::test]
async fn per_region_materialization_matches_full_projection_import(cx: &mut TestAppContext) {
    let source = "# heading\n\nparagraph 中文 🚀\n\n- simple\n  - nested\n\n- loose\n  continuation\n\n> [!NOTE] title\n> body\n\n| A | B |\n| --- | --- |\n| 1 | 2 |\n\n```rust\nfn main() {}\n```\n\n[^n]: note";
    let projection = PreparedSplitProjection::from_snapshot(
        gmark_document::SourceDocument::new(source).snapshot(),
    );
    let editor = cx.new(|cx| Editor::from_markdown(cx, String::new(), None));

    editor.update(cx, |_editor, cx| {
        let full = Editor::build_blocks_from_projection_reusing(
            cx,
            &projection,
            &mut std::collections::HashMap::new(),
        );
        let mut regional = Vec::new();
        let mut reusable = std::collections::HashMap::new();
        for region_index in 0..projection.regions.len() {
            regional.extend(Editor::materialize_projection_region(
                cx,
                &projection,
                region_index,
                &mut reusable,
            ));
        }

        let mut full_tree = super::tree::DocumentTree::new(full);
        full_tree.rebuild_metadata_and_snapshot(cx);
        let mut regional_tree = super::tree::DocumentTree::new(regional);
        regional_tree.rebuild_metadata_and_snapshot(cx);
        assert_eq!(regional_tree.markdown_text(cx), full_tree.markdown_text(cx));
    });
}

fn mixed_projection_fixture(target_bytes: usize) -> String {
    let mut source = String::with_capacity(target_bytes + 2048);
    let mut section = 0usize;
    while source.len() < target_bytes {
        source.push_str(&format!(
            "## Section {section} 中文标题 🚀\n\nParagraph **bold** and *italic* with [link](https://example.com/{section})、中文和 emoji 👨‍💻.\n\n- item {section}\n  - nested 中文\n    - deep emoji ✅\n\n| Name | Value | 状态 |\n| --- | ---: | :---: |\n| alpha | {section} | ✅ |\n| beta | 123456 | 处理中 |\n\n```rust\nfn section_{section}() {{ let value = \"{}\"; }}\n```\n\n![image {section}](assets/image-{section}.png)\n\n> [!NOTE] Note {section}\n> Mixed 中文 body with $x^2 + y^2$.\n\n[^note-{section}]: footnote body\n\n<div data-section=\"{section}\">semantic html</div>\n\n$$\nx_{{{section}}} + y_{{{section}}}\n$$\n\n",
            "x".repeat(512)
        ));
        section += 1;
    }
    source
}

fn percentile_micros(samples: &mut [u128], percentile: usize) -> u128 {
    samples.sort_unstable();
    let index = (samples.len().saturating_sub(1) * percentile).div_ceil(100);
    samples[index.min(samples.len().saturating_sub(1))]
}

fn current_process_rss_mib() -> Option<f64> {
    let pid = get_current_pid().ok()?;
    let mut system = System::new();
    system.refresh_processes(ProcessesToUpdate::Some(&[pid]));
    system
        .process(pid)
        .map(|process| process.memory() as f64 / (1024.0 * 1024.0))
}

fn benchmark_percentiles(samples: &mut [u128]) -> (f64, f64, f64) {
    (
        percentile_micros(samples, 50) as f64 / 1000.0,
        percentile_micros(samples, 95) as f64 / 1000.0,
        percentile_micros(samples, 99) as f64 / 1000.0,
    )
}

include!("editor_tail_09_benchmark.rs");

#[gpui::test]
async fn far_focused_row_does_not_expand_virtualized_scroll_window(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let markdown = (0..super::render::RENDER_ROW_VIRTUALIZATION_THRESHOLD + 250)
        .map(|index| format!("paragraph {index}"))
        .collect::<Vec<_>>()
        .join("\n\n");
    let (editor, visual_cx) =
        cx.add_window_view(move |_window, cx| Editor::from_markdown(cx, markdown, None));
    redraw(visual_cx);

    editor.update(visual_cx, |editor, _cx| {
        let max_y = f32::from(editor.scroll_handle.max_offset().height.max(px(0.0)));
        assert!(max_y > 0.0);
        editor
            .scroll_handle
            .set_offset(point(px(0.0), px(-max_y * 0.75)));
    });
    redraw(visual_cx);

    editor.read_with(visual_cx, |editor, _cx| {
        let (start, end) = editor.prev_render_window.expect("render window");
        assert!(
            start > 0,
            "far focus must not pin the mounted run to row zero; offset={:?}, max={:?}, window={start}..{end}",
            editor.scroll_handle.offset(),
            editor.scroll_handle.max_offset(),
        );
        assert!(end - start < 200, "scroll window must remain bounded");
    });
    visual_cx.update(|window, cx| {
        editor.read_with(cx, |editor, cx| {
            let first = editor.document.first_root().expect("first block");
            assert!(first.read(cx).focus_handle.is_focused(window));
        });
    });
}

#[test]
fn performance_trace_environment_accepts_explicit_truthy_values_only() {
    for value in ["1", "true", "TRUE", " yes ", "on"] {
        assert!(super::perf::env_value_enables_trace(value));
    }
    for value in ["", "0", "false", "enabled", "2"] {
        assert!(!super::perf::env_value_enables_trace(value));
    }
}

#[test]
fn virtual_region_index_keeps_large_mount_window_bounded() {
    let source = (0..20_000)
        .map(|index| format!("paragraph {index} 中文 🚀"))
        .collect::<Vec<_>>()
        .join("\n\n");
    let projection = PreparedSplitProjection::from_snapshot(
        gmark_document::SourceDocument::new(&source).snapshot(),
    );
    let index = VirtualRegionIndex::from_projection(&projection);
    assert_eq!(index.len(), projection.regions.len());
    assert!(index.total_height() > 400_000.0);

    let window = index.mount_window(index.total_height() * 0.75, 720.0, 800.0, Some(0));
    assert!(window.regions.start > 10_000);
    assert!(window.regions.len() < 200);
    assert_eq!(window.pinned_region, Some(0));
}

#[test]
fn virtual_blank_region_keeps_only_minimum_index_height() {
    let source = format!("alpha\n{}beta", "\n".repeat(2_000));
    let projection = PreparedSplitProjection::from_snapshot(
        gmark_document::SourceDocument::new(&source).snapshot(),
    );
    let blank_region = projection
        .regions
        .iter()
        .position(|region| region.kind == ProjectionRegionKind::Blank)
        .expect("blank region");
    let index = VirtualRegionIndex::from_projection(&projection);

    assert_eq!(index.height(blank_region), Some(1.0));
}

#[test]
fn virtual_region_height_update_preserves_prefix_lookup_and_anchor() {
    let source = "# 标题 🚀\n\nalpha\n\nbeta";
    let projection = PreparedSplitProjection::from_snapshot(
        gmark_document::SourceDocument::new(source).snapshot(),
    );
    let mut index = VirtualRegionIndex::from_projection(&projection);
    let old_total = index.total_height();
    let old_second_top = index.top(1).expect("second region");
    assert!(index.update_height(0, 240.0));
    assert!(index.total_height() > old_total);
    assert!(index.top(1).expect("second region") > old_second_top);

    let anchor = index.source_anchor_at_y(120.0).expect("source anchor");
    assert!(source.is_char_boundary(anchor.source_offset));
    assert_eq!(
        index.region_for_source_offset(anchor.source_offset),
        Some(0)
    );
    assert!((index.y_for_source_anchor(anchor).expect("anchor y") - 120.0).abs() < 0.01);
}

#[test]
fn virtual_region_source_edit_shifts_only_current_end_and_following_ranges() {
    let source = "alpha\n\nbeta\n\ngamma";
    let projection = PreparedSplitProjection::from_snapshot(
        gmark_document::SourceDocument::new(source).snapshot(),
    );
    let mut index = VirtualRegionIndex::from_projection(&projection);
    let before = (0..index.len())
        .map(|region| index.source_range(region).expect("source range"))
        .collect::<Vec<_>>();
    let edited_region = 2usize;
    let old_len = before[edited_region].len();
    assert!(index.apply_region_source_len(edited_region, old_len + 7));

    for region in 0..edited_region {
        assert_eq!(index.source_range(region), Some(before[region].clone()));
    }
    let edited = index.source_range(edited_region).expect("edited range");
    assert_eq!(edited.start, before[edited_region].start);
    assert_eq!(edited.end, before[edited_region].end + 7);
    for region in edited_region + 1..index.len() {
        let shifted = index.source_range(region).expect("shifted range");
        assert_eq!(shifted.start, before[region].start + 7);
        assert_eq!(shifted.end, before[region].end + 7);
    }
}

#[gpui::test]
async fn virtual_surface_materializes_only_viewport_and_pinned_regions(cx: &mut TestAppContext) {
    let source = (0..5_000)
        .map(|index| format!("paragraph {index}"))
        .collect::<Vec<_>>()
        .join("\n\n");
    let projection = Arc::new(PreparedSplitProjection::from_snapshot(
        gmark_document::SourceDocument::new(&source).snapshot(),
    ));
    let editor = cx.new(|cx| Editor::from_markdown(cx, String::new(), None));
    editor.update(cx, |_editor, cx| {
        let mut surface = VirtualSurfaceState::new(projection);
        let target = surface.desired_window(80_000.0, 720.0, 800.0, Some(0));
        surface.reconcile_mounts(target, cx);

        assert!(surface.mounted_region_count() < 200);
        assert!(surface.mounted_entity_count() < 300);
        assert!(surface.top_spacer_height() > 0.0);
        assert!(surface.bottom_spacer_height() > 0.0);
        assert!(!surface.viewport_roots().is_empty());
        assert!(!surface.pinned_roots().is_empty());
        for root in surface.flattened_roots() {
            assert!(surface.region_for_entity(root.entity_id()).is_some());
        }
    });
}

#[gpui::test]
async fn virtualized_editor_keeps_rope_as_full_document_truth(cx: &mut TestAppContext) {
    let source = (0..10_000)
        .map(|index| format!("paragraph {index} 中文"))
        .collect::<Vec<_>>()
        .join("\n\n");
    let expected = source.clone();
    let editor = cx.new(move |cx| Editor::from_markdown(cx, source, None));
    editor.read_with(cx, |editor, cx| {
        assert!(editor.virtual_surface.is_some());
        assert!(editor.document.visible_blocks().len() < 200);
        assert_eq!(editor.current_document_source(cx), expected);
        assert_eq!(editor.serialized_document_text(cx), expected);
        assert_eq!(editor.source_document.len(), expected.len());
        assert!(
            editor
                .projection_cache
                .as_ref()
                .expect("projection cache")
                .nodes
                .iter()
                .all(Option::is_none)
        );
    });
}

#[test]
fn regions_only_incremental_projection_matches_full_region_boundaries() {
    let previous_source = (0..10_000)
        .map(|index| format!("paragraph {index}"))
        .collect::<Vec<_>>()
        .join("\n\n");
    let previous_document = gmark_document::SourceDocument::new(&previous_source);
    let previous = PreparedSplitProjection::from_snapshot_adaptive(
        previous_document.snapshot(),
        Editor::VIRTUAL_SURFACE_REGION_THRESHOLD,
    );
    assert!(previous.nodes.iter().all(Option::is_none));

    let edited = format!("changed\n\n{previous_source}");
    let edited_document = gmark_document::SourceDocument::new(&edited);
    let regions_only = PreparedSplitProjection::from_snapshot_incremental_regions_only(
        edited_document.snapshot(),
        &previous,
    );
    let full = PreparedSplitProjection::from_snapshot(edited_document.snapshot());
    assert_eq!(regions_only.source, edited);
    assert_eq!(regions_only.regions, full.regions);
    assert!(regions_only.nodes.iter().all(Option::is_none));
}

#[gpui::test]
async fn small_document_keeps_full_entity_tree_below_virtual_threshold(cx: &mut TestAppContext) {
    let source = (0..100)
        .map(|index| format!("paragraph {index}"))
        .collect::<Vec<_>>()
        .join("\n\n");
    let editor = cx.new(move |cx| Editor::from_markdown(cx, source, None));
    editor.read_with(cx, |editor, _cx| {
        assert!(editor.virtual_surface.is_none());
        assert!(editor.document.visible_blocks().len() >= 100);
    });
}

#[gpui::test]
async fn virtualized_block_edit_preserves_unmounted_rope_suffix(cx: &mut TestAppContext) {
    let source = (0..10_000)
        .map(|index| format!("paragraph {index} 中文"))
        .collect::<Vec<_>>()
        .join("\n\n");
    let suffix = "paragraph 9999 中文";
    let original_len = source.len();
    let editor = cx.new(move |cx| Editor::from_markdown_virtualized(cx, source, None));

    editor.update(cx, |editor, cx| {
        let first = editor.document.first_root().expect("first root").clone();
        let end = first.read(cx).display_text().len();
        first.update(cx, |block, cx| {
            block.prepare_undo_capture(crate::components::UndoCaptureKind::CoalescibleText, cx);
            block.replace_text_in_visible_range(end..end, "!", None, false, cx);
        });
    });

    editor.read_with(cx, |editor, _cx| {
        let current = editor.source_document.text();
        assert_eq!(current.len(), original_len + 1);
        assert!(current.starts_with("paragraph 0 中文!"));
        assert!(current.ends_with(suffix));
        assert!(editor.document_dirty);
    });
}

#[gpui::test]
async fn virtualized_editor_scroll_replaces_only_viewport_and_keeps_pinned_focus(
    cx: &mut TestAppContext,
) {
    init_editor_test_app(cx);
    let source = (0..10_000)
        .map(|index| format!("paragraph {index} 中文"))
        .collect::<Vec<_>>()
        .join("\n\n");
    let expected_len = source.len();
    let (editor, visual_cx) =
        cx.add_window_view(move |_window, cx| Editor::from_markdown_virtualized(cx, source, None));
    redraw(visual_cx);
    let pinned_id = editor.read_with(visual_cx, |editor, _cx| {
        editor.active_entity_id.expect("active entity")
    });

    editor.update(visual_cx, |editor, _cx| {
        let max_y = f32::from(editor.scroll_handle.max_offset().height.max(px(0.0)));
        assert!(max_y > 100_000.0);
        editor
            .scroll_handle
            .set_offset(point(px(0.0), px(-max_y * 0.75)));
    });
    redraw(visual_cx);

    editor.read_with(visual_cx, |editor, cx| {
        let surface = editor.virtual_surface.as_ref().expect("virtual surface");
        assert!(surface.top_spacer_height() > 100_000.0);
        assert!(surface.mounted_region_count() < 250);
        assert!(surface.entity_by_id(pinned_id).is_some());
        assert!(editor.document.block_entity_by_id(pinned_id).is_none());
        assert_eq!(editor.source_document.len(), expected_len);
        let first_viewport = editor.document.first_root().expect("viewport root");
        assert_ne!(first_viewport.entity_id(), pinned_id);
        assert!(
            first_viewport
                .read(cx)
                .display_text()
                .starts_with("paragraph ")
        );
    });
    visual_cx.update(|window, cx| {
        let pinned = editor.read_with(cx, |editor, _cx| {
            editor
                .virtual_surface
                .as_ref()
                .and_then(|surface| surface.entity_by_id(pinned_id))
                .expect("pinned entity")
        });
        assert!(pinned.read(cx).focus_handle.is_focused(window));
    });
}

#[gpui::test]
async fn virtualized_projection_publish_reuses_active_edited_entity(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let source = (0..10_000)
        .map(|index| format!("paragraph {index}"))
        .collect::<Vec<_>>()
        .join("\n\n");
    let (editor, visual_cx) =
        cx.add_window_view(move |_window, cx| Editor::from_markdown_virtualized(cx, source, None));
    redraw(visual_cx);
    let active = editor.update(visual_cx, |editor, cx| {
        let block = editor.document.first_root().expect("first root").clone();
        let end = block.read(cx).display_text().len();
        block.update(cx, |block, cx| {
            block.prepare_undo_capture(crate::components::UndoCaptureKind::CoalescibleText, cx);
            block.replace_text_in_visible_range(end..end, "!", None, false, cx);
        });
        block.entity_id()
    });

    visual_cx
        .executor()
        .advance_clock(Duration::from_millis(749));
    visual_cx.run_until_parked();
    editor.read_with(visual_cx, |editor, _cx| {
        assert!(editor.projection_cache_task.is_some());
    });
    visual_cx.executor().advance_clock(Duration::from_millis(1));
    visual_cx.run_until_parked();
    redraw(visual_cx);

    editor.read_with(visual_cx, |editor, cx| {
        assert_eq!(editor.active_entity_id, Some(active));
        let active_block = editor
            .virtual_surface
            .as_ref()
            .and_then(|surface| surface.entity_by_id(active))
            .expect("active entity must survive projection publish");
        assert_eq!(active_block.read(cx).display_text(), "paragraph 0!");
        assert_eq!(
            editor.projection_cache.as_ref().map(|cache| cache.revision),
            Some(editor.source_document.revision())
        );
    });
}

#[gpui::test]
async fn virtualized_undo_redo_uses_rope_inverse_without_full_editor_snapshot(
    cx: &mut TestAppContext,
) {
    let source = (0..10_000)
        .map(|index| format!("paragraph {index}"))
        .collect::<Vec<_>>()
        .join("\n\n");
    let original = source.clone();
    let editor = cx.new(move |cx| Editor::from_markdown(cx, source, None));
    editor.update(cx, |editor, cx| {
        let block = editor.document.first_root().expect("first root").clone();
        let end = block.read(cx).display_text().len();
        block.update(cx, |block, cx| {
            block.prepare_undo_capture(crate::components::UndoCaptureKind::CoalescibleText, cx);
            block.replace_text_in_visible_range(end..end, "!", None, false, cx);
        });
    });
    editor.update(cx, |editor, cx| {
        assert!(editor.last_stable_source.is_empty());
        assert_eq!(editor.virtual_undo_selections.len(), 1);

        editor.undo_document(cx);
        assert_eq!(editor.source_document.text(), original);
        assert_eq!(editor.virtual_redo_selections.len(), 1);

        editor.redo_document(cx);
        let redone = editor.source_document.text();
        assert!(redone.starts_with("paragraph 0!"));
        assert!(redone.ends_with("paragraph 9999"));
        assert!(editor.last_stable_source.is_empty());
    });
}

#[gpui::test]
/// Checks source anchoring and inverse edits while replacement endpoints live in separate regions.
async fn virtualized_cross_region_replace_preserves_unmounted_source_and_undo_redo(
    cx: &mut TestAppContext,
) {
    let source = (0..10_000)
        .map(|index| format!("paragraph {index}"))
        .collect::<Vec<_>>()
        .join("\n\n");
    let original = source.clone();
    let mut expected = source;
    // `paragraph 0` 的 byte 2 到 `paragraph 2` 的 byte 2。
    expected.replace_range(2..28, "替换");
    let constructor_source = original.clone();
    let editor = cx.new(move |cx| Editor::from_markdown_virtualized(cx, constructor_source, None));

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
        assert!(editor.replace_cross_block_selection_with_text(
            "替换",
            None,
            false,
            crate::components::UndoCaptureKind::NonCoalescible,
            cx,
        ));
        assert_eq!(editor.source_document.text(), expected);
        assert!(editor.source_document.text().ends_with("paragraph 9999"));
        assert_eq!(editor.virtual_undo_selections.len(), 1);

        editor.undo_document(cx);
        assert_eq!(editor.source_document.text(), original);
        editor.redo_document(cx);
        assert_eq!(editor.source_document.text(), expected);
    });
}
