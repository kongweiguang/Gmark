// @author kongweiguang

#[gpui::test]
async fn json_graph_invalid_edit_keeps_source_and_draft_open(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let temp = tempfile::tempdir().expect("JSON invalid graph edit tempdir");
    let path = temp.path().join("invalid-graph-edit.json");
    fs::write(&path, r#"{"nested":{"value":1}}"#).expect("JSON fixture");
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
    visual.update(|window, cx| {
        large_view.update(cx, |view, cx| {
            view.begin_json_graph_node_edit_for_test("node:$/nested#0", window, cx)
        });
    });
    redraw(visual);
    let input = large_view.read_with(visual, |view, _cx| view.json_graph_edit_input_for_test());
    input.update(visual, |block, cx| {
        let len = block.display_text().len();
        block.replace_text_in_visible_range(0..len, r#"{"value":}"#, None, false, cx);
    });
    redraw(visual);
    let save = visual
        .debug_bounds("json-graph-edit-save")
        .expect("graph edit save");
    visual.simulate_click(save.center(), Modifiers::default());
    redraw(visual);
    assert!(visual.debug_bounds("json-graph-edit-error").is_some());
    assert!(visual.debug_bounds("json-graph-edit-panel").is_some());
    assert_eq!(
        large_view.read_with(visual, |view, _cx| view.source_text_for_test()),
        r#"{"nested":{"value":1}}"#
    );
}

#[gpui::test]
async fn stale_json_graph_edit_is_rejected_and_can_reload_current_value(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let temp = tempfile::tempdir().expect("stale JSON graph edit tempdir");
    let path = temp.path().join("stale-graph-edit.json");
    fs::write(&path, r#"{"nested":{"value":1}}"#).expect("JSON fixture");
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
    visual.update(|window, cx| {
        large_view.update(cx, |view, cx| {
            view.begin_json_graph_node_edit_for_test("node:$/nested#0", window, cx)
        });
    });
    let graph_input =
        large_view.read_with(visual, |view, _cx| view.json_graph_edit_input_for_test());
    graph_input.update(visual, |block, cx| {
        let len = block.display_text().len();
        block.replace_text_in_visible_range(0..len, r#"{"value":9}"#, None, false, cx);
    });

    visual.update(|window, cx| {
        large_view.update(cx, |view, cx| view.begin_line_edit_for_test(0, window, cx));
    });
    let (_, source_input) = large_view
        .read_with(visual, |view, _cx| view.active_edit_for_test())
        .expect("source edit");
    source_input.update(visual, |block, cx| {
        let len = block.display_text().len();
        block.replace_text_in_visible_range(0..len, r#"{"nested":{"value":2}}"#, None, false, cx);
    });
    visual.executor().advance_clock(Duration::from_millis(300));
    visual.run_until_parked();
    redraw(visual);

    let save = visual
        .debug_bounds("json-graph-edit-save")
        .expect("stale graph edit save");
    visual.simulate_click(save.center(), Modifiers::default());
    redraw(visual);
    assert!(visual.debug_bounds("json-graph-edit-reload").is_some());
    assert_eq!(
        large_view.read_with(visual, |view, _cx| view.source_text_for_test()),
        r#"{"nested":{"value":2}}"#
    );

    let reload = visual
        .debug_bounds("json-graph-edit-reload")
        .expect("reload latest graph value");
    visual.simulate_click(reload.center(), Modifiers::default());
    assert_eq!(
        graph_input.read_with(visual, |block, _cx| block.display_text().to_owned()),
        r#"{"value":2}"#
    );
}

/// 普通结构化编辑场景只验证内存行为，不得借测试进程身份写入真实用户恢复目录。
#[gpui::test]
async fn json_graph_edit_in_split_updates_source_without_closing_split(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let temp = tempfile::tempdir().expect("split JSON graph edit tempdir");
    let path = temp.path().join("split-graph-edit.json");
    fs::write(&path, r#"{"nested":{"value":1}}"#).expect("JSON fixture");
    let probe =
        gmark_paged_document::probe_file(&path, gmark_paged_document::ProbeOptions::default())
            .expect("JSON probe");
    let source = gmark_paged_document::FileSource::open(&path).expect("JSON source");
    let (editor, visual) = cx.add_window_view(move |_window, cx| {
        Editor::from_source_backed_file(cx, path, probe, source)
    });
    visual.run_until_parked();
    editor.update(visual, |editor, cx| {
        editor.set_view_mode(ViewMode::Split, cx)
    });
    visual.run_until_parked();
    redraw(visual);
    let large_view = editor
        .read_with(visual, |editor, _cx| editor.document_host.clone())
        .expect("JSON SourceBacked view");
    assert!(
        !large_view.read_with(visual, |view, _cx| view.has_recovery_journal_for_test()),
        "ordinary unit scenarios must not attach durable recovery to the user's profile"
    );
    visual.update(|window, cx| {
        large_view.update(cx, |view, cx| {
            view.begin_json_graph_node_edit_for_test("node:$/nested#0", window, cx)
        });
    });
    redraw(visual);
    let input = large_view.read_with(visual, |view, _cx| view.json_graph_edit_input_for_test());
    input.update(visual, |block, cx| {
        let len = block.display_text().len();
        block.replace_text_in_visible_range(0..len, r#"{"value":2}"#, None, false, cx);
    });
    redraw(visual);
    let save = visual
        .debug_bounds("json-graph-edit-save")
        .expect("split graph edit save");
    visual.simulate_click(save.center(), Modifiers::default());
    assert_eq!(
        large_view.read_with(visual, |view, _cx| view.source_text_for_test()),
        r#"{"nested":{"value":2}}"#
    );
    assert!(editor.read_with(visual, |editor, _cx| editor.view_mode == ViewMode::Split));
    redraw(visual);
    assert!(visual.debug_bounds("json-graph-split-source").is_some());
    assert!(visual.debug_bounds("json-graph-split-preview").is_some());
}

#[gpui::test]
async fn oversized_json_graph_value_routes_to_source_without_materializing_editor(
    cx: &mut TestAppContext,
) {
    init_editor_test_app(cx);
    let temp = tempfile::tempdir().expect("oversized JSON graph edit tempdir");
    let path = temp.path().join("oversized-graph-edit.json");
    let source_text = format!(r#"{{"blob":"{}"}}"#, "x".repeat(300 * 1024));
    fs::write(&path, source_text).expect("large JSON fixture");
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
    visual.update(|window, cx| {
        large_view.update(cx, |view, cx| {
            view.begin_json_graph_item_edit_for_test("field:$/blob#0", window, cx)
        });
    });
    redraw(visual);
    assert!(visual.debug_bounds("json-graph-edit-source").is_some());
    assert!(visual.debug_bounds("json-graph-edit-save").is_none());
    assert!(visual.debug_bounds("json-graph-edit-error").is_some());
}

#[gpui::test]
async fn json_split_edit_keeps_last_valid_graph_until_repaired(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let temp = tempfile::tempdir().expect("JSON stale graph tempdir");
    let path = temp.path().join("stale.json");
    fs::write(&path, r#"{"value":1}"#).expect("JSON fixture");
    let probe =
        gmark_paged_document::probe_file(&path, gmark_paged_document::ProbeOptions::default())
            .expect("JSON probe");
    let source = gmark_paged_document::FileSource::open(&path).expect("JSON source");
    let (editor, visual) = cx.add_window_view(move |_window, cx| {
        Editor::from_source_backed_file(cx, path, probe, source)
    });
    visual.run_until_parked();
    editor.update(visual, |editor, cx| {
        editor.set_view_mode(ViewMode::Split, cx)
    });
    visual.run_until_parked();
    redraw(visual);
    let large_view = editor
        .read_with(visual, |editor, _cx| editor.document_host.clone())
        .expect("JSON disk view");

    visual.update(|window, cx| {
        large_view.update(cx, |view, cx| view.begin_line_edit_for_test(0, window, cx));
    });
    let (_, edit) = large_view
        .read_with(visual, |view, _cx| view.active_edit_for_test())
        .expect("JSON line edit");
    edit.update(visual, |block, cx| {
        let len = block.display_text().len();
        block.replace_text_in_visible_range(0..len, r#"{"value":}"#, None, false, cx);
    });
    visual.executor().advance_clock(Duration::from_millis(300));
    visual.run_until_parked();
    redraw(visual);
    assert!(editor.read_with(visual, |editor, _cx| editor.view_mode == ViewMode::Split));
    assert!(visual.debug_bounds("json-graph-stale-banner").is_some());
    assert!(
        large_view
            .read_with(visual, |view, _cx| view.json_graph_state_for_test())
            .is_some_and(|(_, _, _, stale, error)| stale && error.is_some())
    );

    let (_, edit) = large_view
        .read_with(visual, |view, _cx| view.active_edit_for_test())
        .expect("JSON line edit remains active");
    edit.update(visual, |block, cx| {
        let len = block.display_text().len();
        block.replace_text_in_visible_range(0..len, r#"{"value":2}"#, None, false, cx);
    });
    visual.executor().advance_clock(Duration::from_millis(300));
    visual.run_until_parked();
    redraw(visual);
    let repaired_state = large_view.read_with(visual, |view, _cx| view.json_graph_state_for_test());
    assert!(
        repaired_state.is_some_and(|(_, _, _, stale, error)| !stale && error.is_none()),
        "repaired graph state: {repaired_state:?}"
    );
    assert!(
        large_view
            .read_with(visual, |view, _cx| view.json_graph_state_for_test())
            .is_some_and(|(_, _, _, stale, error)| !stale && error.is_none())
    );
}

#[gpui::test]
async fn json_tabs_keep_independent_modes_and_persist_split_ratio(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let temp = tempfile::tempdir().expect("JSON tab tempdir");
    let first_path = temp.path().join("first.json");
    let second_path = temp.path().join("second.json");
    fs::write(&first_path, r#"{"first":1}"#).expect("first JSON");
    fs::write(&second_path, r#"{"second":{"value":2}}"#).expect("second JSON");
    let first_probe = gmark_paged_document::probe_file(
        &first_path,
        gmark_paged_document::ProbeOptions::default(),
    )
    .expect("first probe");
    let first_source = gmark_paged_document::FileSource::open(&first_path).expect("first source");
    let (editor, visual) = cx.add_window_view(move |_window, cx| {
        Editor::from_source_backed_file(cx, first_path, first_probe, first_source)
    });
    visual.run_until_parked();

    let second_probe = gmark_paged_document::probe_file(
        &second_path,
        gmark_paged_document::ProbeOptions::default(),
    )
    .expect("second probe");
    let second_source =
        gmark_paged_document::FileSource::open(&second_path).expect("second source");
    editor.update(visual, |editor, cx| {
        editor.install_new_source_backed_tab(second_path, second_probe, second_source, cx);
        editor.split_pane_ratio = 0.61;
        editor.set_view_mode(ViewMode::Split, cx);
    });
    visual.run_until_parked();
    redraw(visual);
    assert!(editor.read_with(visual, |editor, _cx| editor.view_mode == ViewMode::Split));
    assert!(visual.debug_bounds("json-graph-split-divider").is_some());
    let divider_line = visual
        .debug_bounds("json-graph-split-divider-line")
        .expect("JSON split must render exactly one visible divider line");
    assert_eq!(f32::from(divider_line.size.width), 1.0);

    editor.update(visual, |editor, cx| {
        assert!(editor.switch_to_tab_index(0, cx));
        assert_eq!(editor.view_mode, ViewMode::Preview);
        assert!(editor.switch_to_tab_index(1, cx));
        assert_eq!(editor.view_mode, ViewMode::Split);
        let session = editor
            .workspace_session_snapshot_result(cx)
            .expect("canonical workspace session snapshot");
        let focused = session.focused().expect("focused pane");
        assert_eq!(focused.tabs[1].state.view_mode.as_deref(), Some("split"));
        assert_eq!(session.split_pane_ratio, Some(0.61));
    });
}
