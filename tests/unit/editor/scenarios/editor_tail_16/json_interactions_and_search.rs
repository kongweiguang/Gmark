// @author kongweiguang

#[gpui::test]
async fn json_graph_split_click_and_keyboard_enter_keep_details_open(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let temp = tempfile::tempdir().expect("JSON graph interaction tempdir");
    let path = temp.path().join("interaction.json");
    let source_text = r#"{"nested":{"value":1},"tail":true}"#;
    fs::write(&path, source_text).expect("JSON interaction fixture");
    let probe =
        gmark_paged_document::probe_file(&path, gmark_paged_document::ProbeOptions::default())
            .expect("JSON probe");
    let source = gmark_paged_document::FileSource::open(&path).expect("JSON source");
    let (editor, visual) = cx.add_window_view(move |_window, cx| {
        Editor::from_source_backed_file(cx, path, probe, source)
    });
    visual.simulate_resize(size(px(1100.0), px(720.0)));
    visual.run_until_parked();
    editor.update(visual, |editor, cx| {
        editor.set_view_mode(ViewMode::Split, cx)
    });
    visual.run_until_parked();
    redraw(visual);

    let fit = visual
        .debug_bounds("json-graph-fit")
        .expect("JSON graph fit control");
    visual.simulate_click(fit.center(), Modifiers::default());
    visual.run_until_parked();
    redraw(visual);
    let child = visual
        .debug_bounds("json-graph-node-node:$/nested#0")
        .expect("nested graph card");
    let canvas = visual
        .debug_bounds("json-graph-canvas")
        .expect("JSON graph canvas");
    assert!(child.left() >= canvas.left() && child.right() <= canvas.right());
    assert!(child.top() >= canvas.top() && child.bottom() <= canvas.bottom());
    let child_click = point(child.left() + px(12.0), child.top() + px(12.0));
    visual.simulate_event(MouseDownEvent {
        position: child_click,
        modifiers: Modifiers::default(),
        button: MouseButton::Left,
        click_count: 1,
        first_mouse: false,
    });
    visual.simulate_event(MouseUpEvent {
        position: child_click,
        modifiers: Modifiers::default(),
        button: MouseButton::Left,
        click_count: 1,
    });
    visual.run_until_parked();
    redraw(visual);
    assert!(visual.debug_bounds("json-graph-node-details").is_some());
    let large_view = editor
        .read_with(visual, |editor, _cx| editor.document_host.clone())
        .expect("JSON SourceBacked view");
    assert_eq!(
        large_view.read_with(visual, |view, _cx| view.graph_selected_item_for_test()),
        Some("node:$/nested#0".to_owned()),
        "nested card bounds: {child:?}"
    );

    let selection = large_view
        .read_with(visual, |view, _cx| view.source_selection_for_test())
        .expect("source selection after split click");
    let nested_start = source_text.find(r#"{"value":1}"#).unwrap() as u64;
    assert_eq!(
        selection.range(),
        nested_start..nested_start + r#"{"value":1}"#.len() as u64
    );
    assert!(editor.read_with(visual, |editor, _cx| editor.view_mode == ViewMode::Split));

    redraw(visual);
    let focus = visual
        .debug_bounds("json-graph-focus-subtree")
        .expect("focus selected subtree control");
    visual.simulate_click(focus.center(), Modifiers::default());
    visual.run_until_parked();
    assert_eq!(
        large_view.read_with(visual, |view, _cx| view.json_graph_root_identity_for_test()),
        Some(("$/nested#0".to_owned(), "nested".to_owned()))
    );

    editor.update(visual, |editor, cx| {
        editor.set_view_mode(ViewMode::Preview, cx)
    });
    visual.run_until_parked();
    redraw(visual);
    let root = visual
        .debug_bounds("json-graph-node-node:$/nested#0")
        .expect("root graph card");
    let root_click = point(root.left() + px(12.0), root.top() + px(12.0));
    visual.simulate_event(MouseDownEvent {
        position: root_click,
        modifiers: Modifiers::default(),
        button: MouseButton::Left,
        click_count: 1,
        first_mouse: false,
    });
    visual.simulate_event(MouseUpEvent {
        position: root_click,
        modifiers: Modifiers::default(),
        button: MouseButton::Left,
        click_count: 1,
    });
    visual.simulate_keystrokes("enter");
    visual.run_until_parked();
    redraw(visual);
    assert!(editor.read_with(visual, |editor, _cx| editor.view_mode == ViewMode::Preview));
    assert!(visual.debug_bounds("json-graph-node-details").is_some());
}

#[gpui::test]
async fn json_graph_starts_expanded_and_search_selects_a_deep_match(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let temp = tempfile::tempdir().expect("JSON search tempdir");
    let path = temp.path().join("search.json");
    fs::write(
        &path,
        r#"{"level1":{"level2":{"level3":{"target":"needle"}}}}"#,
    )
    .expect("JSON search fixture");
    let probe =
        gmark_paged_document::probe_file(&path, gmark_paged_document::ProbeOptions::default())
            .expect("JSON probe");
    let source = gmark_paged_document::FileSource::open(&path).expect("JSON source");
    let (editor, visual) = cx.add_window_view(move |_window, cx| {
        Editor::from_source_backed_file(cx, path, probe, source)
    });
    visual.simulate_resize(size(px(900.0), px(620.0)));
    visual.run_until_parked();
    redraw(visual);
    let large_view = editor
        .read_with(visual, |editor, _cx| editor.document_host.clone())
        .expect("JSON SourceBacked view");
    let target_selector = "json-graph-node-node:$/level1#0/level2#0/level3#0";
    assert!(
        visual.debug_bounds(target_selector).is_some(),
        "projected JSON containers start fully expanded"
    );

    let search = large_view.read_with(visual, |view, _cx| view.json_search_input_for_test());
    search.update(visual, |block, cx| {
        block.replace_text_in_visible_range(
            0..block.display_text().len(),
            "needle",
            None,
            false,
            cx,
        );
    });
    visual.run_until_parked();
    redraw(visual);

    assert_eq!(
        large_view.read_with(visual, |view, _cx| view.graph_selected_item_for_test()),
        Some("node:$/level1#0/level2#0/level3#0".to_owned())
    );
    assert!(visual.debug_bounds(target_selector).is_some());
    assert!(visual.debug_bounds("json-graph-search-count").is_some());
    assert!(visual.debug_bounds("json-graph-search-previous").is_some());
    assert!(visual.debug_bounds("json-graph-search-next").is_some());
}

#[gpui::test]
async fn json_graph_search_next_cycles_all_loaded_matches(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let temp = tempfile::tempdir().expect("JSON search navigation tempdir");
    let path = temp.path().join("search-navigation.json");
    fs::write(
        &path,
        r#"{"left":{"value":"needle"},"right":{"value":"needle"}}"#,
    )
    .expect("JSON search fixture");
    let probe =
        gmark_paged_document::probe_file(&path, gmark_paged_document::ProbeOptions::default())
            .expect("JSON probe");
    let source = gmark_paged_document::FileSource::open(&path).expect("JSON source");
    let (editor, visual) = cx.add_window_view(move |_window, cx| {
        Editor::from_source_backed_file(cx, path, probe, source)
    });
    visual.run_until_parked();
    redraw(visual);
    let large_view = editor
        .read_with(visual, |editor, _cx| editor.document_host.clone())
        .expect("JSON SourceBacked view");
    let search = large_view.read_with(visual, |view, _cx| view.json_search_input_for_test());
    search.update(visual, |block, cx| {
        block.replace_text_in_visible_range(
            0..block.display_text().len(),
            "needle",
            None,
            false,
            cx,
        );
    });
    visual.run_until_parked();
    redraw(visual);
    assert_eq!(
        large_view.read_with(visual, |view, _cx| view.json_graph_search_state_for_test()),
        (2, 0)
    );
    let next = visual
        .debug_bounds("json-graph-search-next")
        .expect("next JSON graph search match");
    visual.simulate_click(next.center(), Modifiers::default());
    assert_eq!(
        large_view.read_with(visual, |view, _cx| view.json_graph_search_state_for_test()),
        (2, 1)
    );
    assert_eq!(
        large_view.read_with(visual, |view, _cx| view.graph_selected_item_for_test()),
        Some("node:$/right#1".to_owned())
    );
}

/// 小文件可能在首帧前已完成加载；按当前结构状态验证，避免依赖已消失的加载提示索引。
#[gpui::test]
async fn empty_json_stays_in_preview_while_loading_then_installs_one_empty_root(
    cx: &mut TestAppContext,
) {
    init_editor_test_app(cx);
    let temp = tempfile::tempdir().expect("empty JSON tempdir");
    let path = temp.path().join("empty.json");
    fs::write(&path, "{}").expect("empty JSON fixture");
    let probe =
        gmark_paged_document::probe_file(&path, gmark_paged_document::ProbeOptions::default())
            .expect("empty JSON probe");
    let source = gmark_paged_document::FileSource::open(&path).expect("empty JSON source");
    let (editor, visual) = cx.add_window_view(move |_window, cx| {
        Editor::from_source_backed_file(cx, path, probe, source)
    });

    visual.update(|window, cx| window.draw(cx).clear());
    assert!(editor.read_with(visual, |editor, _cx| editor.view_mode == ViewMode::Preview));
    let initial_graph = editor.read_with(visual, |editor, cx| {
        editor.document_host.as_ref().unwrap().read(cx).json_graph_state_for_test()
    });
    if let Some((nodes, edges, _, _, _)) = initial_graph {
        assert_eq!((nodes, edges), (1, 0));
        assert!(visual.debug_bounds("json-graph-canvas").is_some());
    } else {
        assert!(visual.debug_bounds("json-graph-empty-state").is_some());
    }

    visual.run_until_parked();
    redraw(visual);
    assert!(visual.debug_bounds("json-graph-canvas").is_some());
    let large_view = editor
        .read_with(visual, |editor, _cx| editor.document_host.clone())
        .expect("empty JSON SourceBacked view");
    assert!(
        large_view
            .read_with(visual, |view, _cx| view.json_graph_state_for_test())
            .is_some_and(|(nodes, edges, truncated, stale, error)| {
                nodes == 1 && edges == 0 && !truncated && !stale && error.is_none()
            })
    );
}
