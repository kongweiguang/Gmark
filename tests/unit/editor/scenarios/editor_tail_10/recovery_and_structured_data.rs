// @author kongweiguang

#[gpui::test]
async fn paged_recovery_replays_inside_the_standard_editor_shell(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let temp = tempfile::tempdir().expect("large recovery tempdir");
    let path = temp.path().join("recovered-large.md");
    fs::write(&path, "alpha\nbeta\n").expect("large recovery source");
    let source = gmark_paged_document::FileSource::open(&path).expect("recovery source");
    let mut journal = gmark_paged_document::PagedRecoveryJournal::create(
        temp.path().join("recovery"),
        &source,
        gmark_paged_document::TextEncoding::Utf8 { bom: false },
    )
    .expect("large recovery journal");
    journal
        .record_replace(0..5, "ALPHA", None, "source")
        .expect("recovery edit");
    let journal_path = journal.path().to_path_buf();
    let probe = gmark_paged_document::probe_file(
        &path,
        gmark_paged_document::ProbeOptions {
            max_resident_bytes: 1,
            ..gmark_paged_document::ProbeOptions::default()
        },
    )
    .expect("large recovery probe");
    let (editor, visual) = cx.add_window_view(move |_window, cx| {
        Editor::from_paged_recovery(cx, path, probe, source, journal_path)
    });
    visual.simulate_resize(size(px(960.0), px(640.0)));
    redraw(visual);

    let large_view = editor
        .read_with(visual, |editor, _cx| editor.document_host.clone())
        .expect("large recovery view");
    assert_eq!(
        large_view.read_with(visual, |view, _cx| view.recovered_text_for_test()),
        Some(b"ALPHA\nbeta\n".to_vec())
    );
    assert!(large_view.read_with(visual, |view, _cx| view.has_recovery_journal_for_test()));
    assert!(editor.read_with(visual, |editor, _cx| editor.document_dirty));
    assert!(visual.debug_bounds("editor-titlebar").is_some());
    assert!(visual.debug_bounds("document-tab-strip").is_some());
    assert!(visual.debug_bounds("document-host-tab-content").is_some());
    assert!(visual.debug_bounds("status-bar").is_some());
}

#[gpui::test]
async fn discard_and_close_checkpoints_paged_recovery_and_clears_host_dirty(
    cx: &mut TestAppContext,
) {
    init_editor_test_app(cx);
    let temp = tempfile::tempdir().expect("discard recovery tempdir");
    let path = temp.path().join("discard-recovered.md");
    fs::write(&path, "alpha\nbeta\n").expect("discard recovery source");
    let source = gmark_paged_document::FileSource::open(&path).expect("discard recovery source");
    let mut journal = gmark_paged_document::PagedRecoveryJournal::create(
        temp.path().join("recovery"),
        &source,
        gmark_paged_document::TextEncoding::Utf8 { bom: false },
    )
    .expect("discard recovery journal");
    journal
        .record_replace(0..5, "ALPHA", None, "source")
        .expect("discard recovery edit");
    let journal_path = journal.path().to_path_buf();
    let probe = gmark_paged_document::probe_file(
        &path,
        gmark_paged_document::ProbeOptions {
            max_resident_bytes: 1,
            ..gmark_paged_document::ProbeOptions::default()
        },
    )
    .expect("discard recovery probe");
    let journal_path_for_editor = journal_path.clone();
    let (editor, visual) = cx.add_window_view(move |_window, cx| {
        Editor::from_paged_recovery(cx, path, probe, source, journal_path_for_editor)
    });
    visual.run_until_parked();
    let document_host = editor
        .read_with(visual, |editor, _cx| editor.document_host.clone())
        .expect("discard recovery host");
    assert!(document_host.read_with(visual, |host, _cx| host.is_dirty()));

    visual.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            assert!(!editor.on_window_should_close(window, cx));
            editor.on_discard_and_close(&gpui::ClickEvent::default(), window, cx);
            assert!(!editor.is_document_dirty());
            assert!(!document_host.read(cx).is_dirty());
        });
    });

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while journal_path.exists() && std::time::Instant::now() < deadline {
        // Discard deliberately checkpoints on the recovery worker so closing a
        // window never waits for filesystem deletion; let that worker publish
        // its durable terminal state before asserting the disk contract.
        visual.run_until_parked();
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert!(!journal_path.exists());
}

#[gpui::test]
async fn recovered_resident_csv_can_return_to_the_live_table(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let temp = tempfile::tempdir().expect("CSV recovery tempdir");
    let path = temp.path().join("recovered.csv");
    fs::write(&path, "name,value\nalpha,1\n").expect("CSV recovery source");
    let source = gmark_paged_document::FileSource::open(&path).expect("CSV recovery file source");
    let mut journal = gmark_paged_document::PagedRecoveryJournal::create(
        temp.path().join("recovery"),
        &source,
        gmark_paged_document::TextEncoding::Utf8 { bom: false },
    )
    .expect("CSV recovery journal");
    journal
        .record_replace(17..18, "2", None, "live")
        .expect("CSV recovery edit");
    let journal_path = journal.path().to_path_buf();
    let probe =
        gmark_paged_document::probe_file(&path, gmark_paged_document::ProbeOptions::default())
            .expect("CSV recovery probe");
    let (editor, visual) = cx.add_window_view(move |_window, cx| {
        Editor::from_paged_recovery(cx, path, probe, source, journal_path)
    });
    visual.simulate_resize(size(px(960.0), px(640.0)));
    visual.run_until_parked();
    redraw(visual);

    let document_host = editor
        .read_with(visual, |editor, _cx| editor.document_host.clone())
        .expect("recovered CSV DocumentHost");
    assert!(document_host.read_with(visual, |view, _cx| view.has_structure_view()));
    assert!(document_host.read_with(visual, |view, _cx| {
        view.structure_error_for_test().is_none()
    }));

    editor.update(visual, |editor, cx| {
        editor.set_view_mode(ViewMode::Rendered, cx)
    });
    visual.run_until_parked();
    redraw(visual);
    assert!(document_host.read_with(visual, |view, _cx| view.delimited_live_for_test()));
    assert!(
        visual
            .debug_bounds("document-host-structured-content")
            .is_some()
    );
}

#[gpui::test]
async fn large_jsonl_follow_stays_source_only_while_appended_bytes_remain_visible(
    cx: &mut TestAppContext,
) {
    use std::io::Write as _;

    init_editor_test_app(cx);
    let temp = tempfile::tempdir().expect("JSONL follow tempdir");
    let path = temp.path().join("follow.jsonl");
    fs::write(&path, "{\"id\":1}\n").expect("JSONL fixture");
    let probe = gmark_paged_document::probe_file(
        &path,
        gmark_paged_document::ProbeOptions {
            max_resident_bytes: 1,
            ..gmark_paged_document::ProbeOptions::default()
        },
    )
    .expect("JSONL follow probe");
    let source = gmark_paged_document::FileSource::open(&path).expect("JSONL follow source");
    let (editor, visual) = cx.add_window_view(move |_window, cx| {
        Editor::from_source_backed_file(cx, path, probe, source)
    });
    visual.run_until_parked();
    redraw(visual);
    let large_view = editor
        .read_with(visual, |editor, _cx| editor.document_host.clone())
        .expect("JSONL follow view");
    assert!(!large_view.read_with(visual, |view, _cx| view.has_structure_view()));
    large_view.update(visual, |view, cx| view.toggle_follow(cx));

    let mut writer = fs::OpenOptions::new()
        .append(true)
        .open(temp.path().join("follow.jsonl"))
        .expect("open JSONL append");
    writer
        .write_all(b"{\"id\":2}\n")
        .expect("append valid JSONL record");
    writer.sync_all().expect("sync valid JSONL append");
    visual
        .executor()
        .advance_clock(Duration::from_millis(1_100));
    visual.run_until_parked();
    redraw(visual);
    assert!(!large_view.read_with(visual, |view, _cx| view.has_structure_view()));

    writer
        .write_all(b"{\"broken\":]}\n")
        .expect("append invalid JSONL record");
    writer.sync_all().expect("sync invalid JSONL append");
    visual
        .executor()
        .advance_clock(Duration::from_millis(1_100));
    visual.run_until_parked();
    redraw(visual);

    assert!(
        large_view
            .read_with(visual, |view, _cx| view.structure_error_for_test())
            .is_none()
    );
    assert!(
        large_view
            .read_with(visual, |view, _cx| view.recovered_text_for_test())
            .is_some_and(|text| text.ends_with(b"{\"broken\":]}\n"))
    );
    assert!(
        visual
            .debug_bounds("document-host-structure-error-jump")
            .is_none()
    );
}

#[gpui::test]
async fn resident_strategy_json_uses_bounded_graph_and_refreshes_after_source_edits(
    cx: &mut TestAppContext,
) {
    init_editor_test_app(cx);
    let temp = tempfile::tempdir().expect("large json tempdir");
    let path = temp.path().join("large-tree.json");
    fs::write(&path, r#"[{"id":1}, [2, 3, {"nested":true}], "tail"]"#).expect("large json fixture");
    let probe =
        gmark_paged_document::probe_file(&path, gmark_paged_document::ProbeOptions::default())
            .expect("large json probe");
    let source = gmark_paged_document::FileSource::open(&path).expect("large json source");
    let (editor, visual) = cx.add_window_view(move |_window, cx| {
        Editor::from_source_backed_file(cx, path, probe, source)
    });
    redraw(visual);
    let large_view = editor
        .read_with(visual, |editor, _cx| editor.document_host.clone())
        .expect("large json view");
    visual.run_until_parked();
    assert!(editor.read_with(visual, |editor, _cx| editor.view_mode == ViewMode::Preview));
    assert!(
        large_view
            .read_with(visual, |view, _cx| view.json_graph_state_for_test())
            .is_some_and(|(nodes, edges, truncated, stale, error)| {
                nodes == 4 && edges == 3 && !truncated && !stale && error.is_none()
            })
    );
    editor.update(visual, |editor, cx| {
        editor.set_view_mode(ViewMode::Source, cx);
    });
    assert!(large_view.read_with(visual, |view, _cx| view.source_view_for_test()));
    editor.update(visual, |editor, cx| {
        editor.set_view_mode(ViewMode::Rendered, cx);
    });
    visual.run_until_parked();
    assert!(editor.read_with(visual, |editor, _cx| editor.view_mode == ViewMode::Preview));
    let (epoch, revision, generation) = large_view
        .read_with(visual, |view, _cx| view.installed_projection_for_test())
        .expect("registered projection snapshot");
    assert!(epoch > 0);
    assert_eq!(revision, 0);
    assert!(generation > 0);
    assert!(visual.debug_bounds("document-host-tab-content").is_some());

    editor.update(visual, |editor, cx| {
        editor.set_view_mode(ViewMode::Split, cx);
    });
    visual.run_until_parked();
    visual.update(|window, cx| {
        large_view.update(cx, |view, cx| view.begin_line_edit_for_test(0, window, cx));
    });
    let (_, edit_block) = large_view
        .read_with(visual, |view, _cx| view.active_edit_for_test())
        .expect("active JSON source edit");
    edit_block.update(visual, |block, cx| {
        let end = block.display_text().len();
        block.replace_text_in_visible_range(end..end, " ", None, false, cx);
    });
    visual.executor().advance_clock(Duration::from_millis(300));
    visual.run_until_parked();
    redraw(visual);
    assert!(editor.read_with(visual, |editor, _cx| editor.document_dirty));
    assert!(
        large_view
            .read_with(visual, |view, _cx| view.json_graph_state_for_test())
            .is_some_and(|(_, _, _, stale, error)| !stale && error.is_none())
    );

    visual.update(|window, cx| {
        large_view.update(cx, |view, cx| view.on_undo(&Undo, window, cx));
    });
    visual.executor().advance_clock(Duration::from_millis(300));
    visual.run_until_parked();
    redraw(visual);
    assert!(!editor.read_with(visual, |editor, _cx| editor.document_dirty));
    assert!(
        large_view
            .read_with(visual, |view, _cx| view.json_graph_state_for_test())
            .is_some_and(|(_, _, _, stale, error)| !stale && error.is_none())
    );
}

/// JSON 图错误跳转应把 Source 光标放到对应逻辑行，并允许异步列号暂未就绪。
#[gpui::test]
async fn invalid_resident_json_reports_the_byte_and_jumps_back_to_source(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let temp = tempfile::tempdir().expect("invalid JSON tempdir");
    let path = temp.path().join("invalid-large.json");
    let text = "{\n  \"ok\": 1,\n  \"broken\": ]\n}\n";
    fs::write(&path, text).expect("invalid JSON fixture");
    let probe =
        gmark_paged_document::probe_file(&path, gmark_paged_document::ProbeOptions::default())
            .expect("invalid JSON probe");
    let source = gmark_paged_document::FileSource::open(&path).expect("invalid JSON source");
    let (editor, visual) = cx.add_window_view(move |_window, cx| {
        Editor::from_source_backed_file(cx, path, probe, source)
    });
    visual.simulate_resize(size(px(960.0), px(640.0)));
    visual.run_until_parked();
    redraw(visual);

    let large_view = editor
        .read_with(visual, |editor, _cx| editor.document_host.clone())
        .expect("invalid JSON large view");
    let (message, byte_offset) = large_view
        .read_with(visual, |view, _cx| view.json_graph_error_for_test())
        .expect("JSON graph error");
    let expected_message = large_view.read_with(visual, |_view, cx| {
        cx.global::<I18nManager>()
            .strings()
            .large_document_text("error_invalid_json_location")
            .replace("{line}", "3")
            .replace("{column}", "13")
    });
    assert_eq!(message, expected_message);
    let byte_offset = byte_offset.expect("JSON error byte offset");
    assert!(visual.debug_bounds("json-graph-empty-state").is_some());
    let jump = visual
        .debug_bounds("json-graph-error-jump")
        .expect("JSON error jump action");
    visual.simulate_click(jump.center(), Modifiers::default());
    visual.run_until_parked();
    redraw(visual);

    let expected_line = text.as_bytes()[..byte_offset as usize]
        .iter()
        .filter(|byte| **byte == b'\n')
        .count()
        + 1;
    assert_eq!(
        large_view.read_with(visual, |view, cx| {
            view.cursor_position(cx).expect("JSON Source jump caret").0
        }),
        expected_line
    );
    assert!(editor.read_with(visual, |editor, _cx| editor.view_mode == ViewMode::Source));
    assert!(visual.debug_bounds("editor-titlebar").is_some());
    assert!(visual.debug_bounds("status-bar").is_some());
}

/// JSONL 结构错误跳转仍按全局行定位，不依赖状态栏列号缓存完成时机。
#[gpui::test]
async fn invalid_resident_jsonl_record_reports_global_byte_and_jumps_to_source(
    cx: &mut TestAppContext,
) {
    init_editor_test_app(cx);
    let temp = tempfile::tempdir().expect("invalid JSONL tempdir");
    let path = temp.path().join("invalid-large.jsonl");
    let text = "{\"ok\":1}\n[1,2,3]\n{\"broken\":]}\n";
    fs::write(&path, text).expect("invalid JSONL fixture");
    let probe =
        gmark_paged_document::probe_file(&path, gmark_paged_document::ProbeOptions::default())
            .expect("invalid JSONL probe");
    let source = gmark_paged_document::FileSource::open(&path).expect("invalid JSONL source");
    let (editor, visual) = cx.add_window_view(move |_window, cx| {
        Editor::from_source_backed_file(cx, path, probe, source)
    });
    visual.simulate_resize(size(px(960.0), px(640.0)));
    visual.run_until_parked();
    redraw(visual);

    let large_view = editor
        .read_with(visual, |editor, _cx| editor.document_host.clone())
        .expect("invalid JSONL large view");
    let (message, byte_offset) = large_view
        .read_with(visual, |view, _cx| view.structure_error_for_test())
        .expect("structured JSONL error");
    let byte_offset = byte_offset.expect("JSONL error byte offset");
    let expected_message = large_view.read_with(visual, |_view, cx| {
        cx.global::<I18nManager>()
            .strings()
            .large_document_text("error_invalid_json")
            .replace("{offset}", &byte_offset.to_string())
    });
    assert_eq!(message, expected_message);
    assert_eq!(byte_offset, text.rfind(']').expect("invalid token") as u64);

    let jump = visual
        .debug_bounds("document-host-structure-error-jump")
        .expect("JSONL error jump action");
    visual.simulate_click(jump.center(), Modifiers::default());
    redraw(visual);
    assert_eq!(
        large_view.read_with(visual, |view, cx| {
            view.cursor_position(cx).expect("JSONL Source jump caret").0
        }),
        3
    );
    assert!(large_view.read_with(visual, |view, _cx| view.source_view_for_test()));
}
