// @author kongweiguang

#[gpui::test]
async fn large_document_reopens_with_an_explicit_encoding_without_changing_tabs(
    cx: &mut TestAppContext,
) {
    init_editor_test_app(cx);
    let temp = tempfile::tempdir().expect("explicit encoding tempdir");
    let path = temp.path().join("explicit-encoding.md");
    fs::write(&path, "café\n").expect("explicit encoding fixture");
    let probe = gmark_paged_document::probe_file(
        &path,
        gmark_paged_document::ProbeOptions {
            max_resident_bytes: 1,
            ..gmark_paged_document::ProbeOptions::default()
        },
    )
    .expect("explicit encoding probe");
    let source = gmark_paged_document::FileSource::open(&path).expect("explicit encoding source");
    let (editor, visual) =
        cx.add_window_view(move |_window, cx| Editor::from_source_backed_file(cx, path, probe, source));
    visual.run_until_parked();
    redraw(visual);
    let large_view = editor
        .read_with(visual, |editor, _cx| editor.document_host.clone())
        .expect("explicit encoding large view");
    assert_eq!(
        large_view.read_with(visual, |view, _cx| view.encoding_label()),
        "UTF-8"
    );

    visual.update(|window, cx| {
        large_view.update(cx, |view, cx| {
            view.reopen_with_encoding(
                gmark_paged_document::TextEncoding::Legacy("windows-1252".to_owned()),
                window,
                cx,
            )
        });
    });
    visual.run_until_parked();
    redraw(visual);
    assert_eq!(
        large_view.read_with(visual, |view, _cx| view.encoding_label()),
        "WINDOWS-1252"
    );
    assert!(
        large_view
            .read_with(visual, |view, _cx| view.recovered_text_for_test())
            .is_some_and(|text| text == "cafÃ©\n".as_bytes())
    );

    visual.update(|window, cx| {
        large_view.update(cx, |view, cx| view.begin_line_edit_for_test(0, window, cx));
    });
    let (_, edit_block) = large_view
        .read_with(visual, |view, _cx| view.active_edit_for_test())
        .expect("explicit encoding edit block");
    edit_block.update(visual, |block, cx| {
        let end = block.display_text().len();
        block.replace_text_in_visible_range(end..end, "x", None, false, cx);
    });
    visual.run_until_parked();
    visual.update(|window, cx| {
        large_view.update(cx, |view, cx| {
            view.reopen_with_encoding(
                gmark_paged_document::TextEncoding::Utf8 { bom: false },
                window,
                cx,
            )
        });
    });
    assert_eq!(
        large_view.read_with(visual, |view, _cx| view.encoding_label()),
        "WINDOWS-1252",
        "dirty documents must not be discarded by an encoding reopen"
    );
    let expected_error = large_view.read_with(visual, |_view, cx| {
        cx.global::<I18nManager>()
            .strings()
            .large_document_text("reopen_dirty_error")
    });
    assert_eq!(
        large_view.read_with(visual, |view, _cx| view.error_for_test()),
        Some(expected_error)
    );
    visual.simulate_keystrokes("ctrl-z");
    visual.run_until_parked();

    visual.update(|window, cx| {
        large_view.update(cx, |view, cx| {
            view.reopen_with_encoding(
                gmark_paged_document::TextEncoding::Utf8 { bom: false },
                window,
                cx,
            )
        });
    });
    visual.run_until_parked();
    redraw(visual);
    assert_eq!(
        large_view.read_with(visual, |view, _cx| view.encoding_label()),
        "UTF-8"
    );
    assert!(
        large_view
            .read_with(visual, |view, _cx| view.recovered_text_for_test())
            .is_some_and(|text| text == "café\n".as_bytes())
    );
    assert!(editor.read_with(visual, |editor, _cx| editor.view_mode == ViewMode::Source));
    assert!(large_view.read_with(visual, |view, _cx| view.error_for_test().is_none()));
}

/// Opt-in wall-clock soak for the real GPUI input/render pipeline. The fixture is copied to a
/// temporary file, so edit/save cycles never mutate the checked-in performance corpus.
#[gpui::test]
#[ignore = "eight-hour interactive production soak"]
async fn large_document_interactive_soak(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let fixture = PathBuf::from(
        std::env::var_os("GMARK_INTERACTIVE_SOAK_FIXTURE")
            .expect("GMARK_INTERACTIVE_SOAK_FIXTURE must name an existing large Markdown file"),
    );
    let duration = Duration::from_secs(
        std::env::var("GMARK_INTERACTIVE_SOAK_SECONDS")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(28_800),
    );
    let progress_path = std::env::var_os("GMARK_INTERACTIVE_SOAK_PROGRESS").map(PathBuf::from);
    let temp = tempfile::tempdir().expect("interactive soak tempdir");
    let extension = fixture
        .extension()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or("txt");
    let path = temp.path().join(format!("interactive-soak.{extension}"));
    fs::copy(&fixture, &path).expect("copy interactive soak fixture");
    let probe = gmark_paged_document::probe_file(
        &path,
        gmark_paged_document::ProbeOptions {
            max_resident_bytes: 1,
            ..gmark_paged_document::ProbeOptions::default()
        },
    )
    .expect("interactive soak probe");
    let source = gmark_paged_document::FileSource::open(&path).expect("interactive soak source");
    let (editor, visual) =
        cx.add_window_view(move |_window, cx| Editor::from_source_backed_file(cx, path, probe, source));
    visual.simulate_resize(size(px(1180.0), px(780.0)));
    redraw(visual);
    let large_view = editor
        .read_with(visual, |editor, _cx| editor.document_host.clone())
        .expect("interactive soak large view");

    let ready_deadline = Instant::now() + Duration::from_secs(30);
    loop {
        visual.update(|window, cx| {
            large_view.update(cx, |view, cx| view.begin_line_edit_for_test(0, window, cx));
        });
        visual.run_until_parked();
        if large_view.read_with(visual, |view, _cx| view.active_edit_for_test().is_some()) {
            break;
        }
        assert!(
            Instant::now() < ready_deadline,
            "large Source never became editable"
        );
        std::thread::sleep(Duration::from_millis(50));
        redraw(visual);
    }

    let started = Instant::now();
    let baseline_rss = current_process_rss_mib().unwrap_or_default();
    let mut maximum_rss = baseline_rss;
    let mut cycles = 0u64;
    let mut edits = 0u64;
    let mut saves = 0u64;
    let mut searches = 0u64;
    let mut structure_transitions = 0u64;
    let mut next_edit_at = Duration::ZERO;
    let mut next_search_at = Duration::ZERO;
    let mut next_save_at = Duration::ZERO;
    while started.elapsed() < duration {
        large_view.update(visual, |view, cx| view.scroll_page_for_test(true, cx));
        redraw(visual);
        large_view.update(visual, |view, cx| view.scroll_page_for_test(false, cx));
        redraw(visual);

        let elapsed = started.elapsed();
        if elapsed >= next_edit_at {
            next_edit_at += Duration::from_secs(3);
            let line = large_view.read_with(visual, |view, _cx| view.scroll_top_line_for_test());
            visual.update(|window, cx| {
                large_view.update(cx, |view, cx| {
                    view.begin_line_edit_for_test(line, window, cx)
                });
            });
            if let Some((_, block)) =
                large_view.read_with(visual, |view, _cx| view.active_edit_for_test())
            {
                block.update(visual, |block, cx| {
                    let end = block.display_text().len();
                    block.replace_text_in_visible_range(end..end, "x", None, false, cx);
                });
                edits += 1;
            }
            visual.run_until_parked();
            visual.update(|window, cx| {
                large_view.update(cx, |view, cx| view.undo_for_test(window, cx));
            });
            visual.run_until_parked();
            visual.update(|window, cx| {
                large_view.update(cx, |view, cx| view.redo_for_test(window, cx));
            });
            visual.run_until_parked();

            if elapsed >= next_save_at {
                next_save_at += Duration::from_secs(15 * 60);
                visual.update(|window, cx| {
                    large_view.update(cx, |view, cx| {
                        view.on_save_document(&SaveDocument, window, cx)
                    });
                });
                visual.run_until_parked();
                saves += 1;
            } else {
                visual.update(|window, cx| {
                    large_view.update(cx, |view, cx| view.undo_for_test(window, cx));
                });
                visual.run_until_parked();
            }
        }

        if elapsed >= next_search_at {
            next_search_at += Duration::from_secs(45);
            visual.simulate_keystrokes("ctrl-f");
            redraw(visual);
            visual.simulate_keystrokes("ctrl-a");
            visual.simulate_input("生产报告");
            visual.run_until_parked();
            visual.simulate_keystrokes("escape");
            searches += 1;
        }
        if elapsed.as_secs().is_multiple_of(60)
            && large_view.read_with(visual, |view, _cx| view.has_structure_view())
        {
            large_view.update(visual, |view, cx| view.show_structure_view(cx));
            redraw(visual);
            large_view.update(visual, |view, cx| view.show_source_view(cx));
            structure_transitions += 1;
        }

        redraw(visual);
        assert!(large_view.read_with(visual, |view, _cx| view.error_for_test().is_none()));
        assert!(
            large_view.read_with(visual, |view, _cx| view.source_cache_len_for_test())
                <= crate::document_host::MAX_SOURCE_CACHED_ROWS
        );
        maximum_rss = maximum_rss.max(current_process_rss_mib().unwrap_or_default());
        cycles += 1;

        if cycles.is_multiple_of(10)
            && let Some(progress_path) = &progress_path
        {
            let progress = serde_json::json!({
                "schema_version": 1,
                "completed": false,
                "elapsed_seconds": started.elapsed().as_secs_f64(),
                "cycles": cycles,
                "edits": edits,
                "saves": saves,
                "searches": searches,
                "structure_transitions": structure_transitions,
                "baseline_rss_mib": baseline_rss,
                "maximum_rss_mib": maximum_rss,
            });
            fs::write(
                progress_path,
                serde_json::to_vec_pretty(&progress).expect("serialize interactive soak progress"),
            )
            .expect("write interactive soak progress");
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    if let Some(progress_path) = progress_path {
        let progress = serde_json::json!({
            "schema_version": 1,
            "completed": true,
            "elapsed_seconds": started.elapsed().as_secs_f64(),
            "cycles": cycles,
            "edits": edits,
            "saves": saves,
            "searches": searches,
            "structure_transitions": structure_transitions,
            "baseline_rss_mib": baseline_rss,
            "maximum_rss_mib": maximum_rss,
            "rss_growth_mib": (maximum_rss - baseline_rss).max(0.0),
        });
        fs::write(
            progress_path,
            serde_json::to_vec_pretty(&progress)
                .expect("serialize completed interactive soak progress"),
        )
        .expect("write completed interactive soak progress");
    }
}

#[gpui::test]
async fn large_markdown_exposes_only_source_without_table_projection(
    cx: &mut TestAppContext,
) {
    init_editor_test_app(cx);
    let temp = tempfile::tempdir().expect("large Markdown tempdir");
    let path = temp.path().join("multiple-tables.md");
    fs::write(
        &path,
        "# Report\n\n| name | score |\n| --- | ---: |\n| Ada | 10 |\n\nBetween tables.\n\n| city | country | note |\n| :--- | :--- | ---: |\n| Paris | France | first |\n| Tokyo | Japan | second |\n\nDone.\n",
    )
    .expect("multi-table Markdown fixture");
    let probe = gmark_paged_document::probe_file(
        &path,
        gmark_paged_document::ProbeOptions {
            max_resident_bytes: 1,
            ..gmark_paged_document::ProbeOptions::default()
        },
    )
    .expect("large Markdown probe");
    let source = gmark_paged_document::FileSource::open(&path).expect("large Markdown source");
    let (editor, visual) =
        cx.add_window_view(move |_window, cx| Editor::from_source_backed_file(cx, path, probe, source));
    visual.simulate_resize(size(px(1180.0), px(780.0)));
    visual.run_until_parked();
    redraw(visual);

    let large_view = editor
        .read_with(visual, |editor, _cx| editor.document_host.clone())
        .expect("large Markdown view");
    assert!(large_view.read_with(visual, |view, _cx| view.source_view_for_test()));
    assert!(editor.read_with(visual, |editor, _cx| editor.view_mode == ViewMode::Source));
    assert!(!large_view.read_with(visual, |view, _cx| view.has_structure_view()));
    assert!(large_view.read_with(visual, |view, _cx| view.markdown_table_state_for_test()).is_none());
    assert!(visual.debug_bounds("status-bar-large-structure").is_none());
    assert!(visual.debug_bounds("document-host-markdown-table-switcher").is_none());
}

#[gpui::test]
async fn wide_large_csv_exposes_only_source_without_column_projection(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let temp = tempfile::tempdir().expect("wide CSV tempdir");
    let path = temp.path().join("wide.csv");
    let headers = (0..64)
        .map(|column| format!("h{column}"))
        .collect::<Vec<_>>();
    let values = (0..64)
        .map(|column| format!("v{column}"))
        .collect::<Vec<_>>();
    fs::write(
        &path,
        format!("{}\n{}\n", headers.join(","), values.join(",")),
    )
    .expect("wide CSV fixture");
    let probe = gmark_paged_document::probe_file(
        &path,
        gmark_paged_document::ProbeOptions {
            max_resident_bytes: 1,
            ..gmark_paged_document::ProbeOptions::default()
        },
    )
    .expect("wide CSV probe");
    let source = gmark_paged_document::FileSource::open(&path).expect("wide CSV source");
    let (editor, visual) =
        cx.add_window_view(move |_window, cx| Editor::from_source_backed_file(cx, path, probe, source));
    visual.simulate_resize(size(px(960.0), px(640.0)));
    visual.run_until_parked();
    redraw(visual);
    let large_view = editor
        .read_with(visual, |editor, _cx| editor.document_host.clone())
        .expect("wide CSV large view");
    assert!(large_view.read_with(visual, |view, _cx| view.source_view_for_test()));
    assert!(!large_view.read_with(visual, |view, _cx| view.has_structure_view()));
    assert!(visual.debug_bounds("document-host-structured-column-pager").is_none());
    assert!(visual.debug_bounds("document-host-structured-columns-next").is_none());
}

/// Verifies structured-surface Tab stays owned by the table and moves in both directions.
#[gpui::test]
async fn csv_uses_all_four_modes_and_live_cell_edits_rebuild_the_table(
    cx: &mut TestAppContext,
) {
    init_editor_test_app(cx);
    let temp = tempfile::tempdir().expect("CSV mode tempdir");
    let path = temp.path().join("modes.csv");
    fs::write(&path, "name,score\r\nAda,10\r\nBob,20\r\n").expect("CSV fixture");
    let probe = gmark_paged_document::probe_file(
        &path,
        gmark_paged_document::ProbeOptions::default(),
    )
    .expect("CSV probe");
    let source = gmark_paged_document::FileSource::open(&path).expect("CSV source");
    let (editor, visual) = cx.add_window_view(move |_window, cx| {
        Editor::from_source_backed_file(cx, path, probe, source)
    });
    visual.run_until_parked();
    redraw(visual);
    assert!(editor.read_with(visual, |editor, _cx| editor.view_mode == ViewMode::Preview));
    let large_view = editor
        .read_with(visual, |editor, _cx| editor.document_host.clone())
        .expect("CSV SourceBacked view");
    assert!(
        large_view.read_with(visual, |view, _cx| view.has_structure_view()),
        "CSV index did not install"
    );
    assert!(
        large_view.read_with(visual, |view, _cx| view.structure_view_active()),
        "CSV Preview did not activate"
    );
    assert!(visual.debug_bounds("document-host-structured-scroll").is_some());
    assert!(
        visual
            .debug_bounds("document-host-structured-filter-bar")
            .is_none(),
        "CSV Preview must not render the row filter bar"
    );
    let preview_scroll = visual
        .debug_bounds("document-host-structured-scroll")
        .expect("CSV Preview scroll surface");
    let preview_table = visual
        .debug_bounds("document-host-structured-content")
        .expect("CSV Preview table");
    assert!(
        f32::from(preview_table.left() - preview_scroll.left()).abs() <= 1.0,
        "CSV tables must stay left-aligned in Preview"
    );
    let preview_row = visual
        .debug_bounds("document-host-structured-row-0")
        .expect("CSV Preview row");
    visual.simulate_click(preview_row.center(), Modifiers::default());
    assert!(editor.read_with(visual, |editor, _cx| editor.view_mode == ViewMode::Preview));
    assert!(large_view.read_with(visual, |view, _cx| view.structure_view_active()));
    let preview_cell = visual
        .debug_bounds("document-host-structured-cell-0-1")
        .expect("CSV Preview score cell");
    visual.simulate_click(preview_cell.center(), Modifiers::default());
    assert_eq!(
        large_view.read_with(visual, |view, _cx| view.structured_selected_cell_for_test()),
        Some((Some(0), 1))
    );
    visual.update(|window, cx| {
        large_view.update(cx, |view, cx| view.copy_for_test(window, cx));
    });
    assert_eq!(
        visual.read_from_clipboard().and_then(|item| item.text()),
        Some("10".to_owned()),
        "CSV Preview must copy the selected cell content"
    );

    visual.simulate_keystrokes("ctrl-f");
    redraw(visual);
    visual.simulate_input("Bob");
    visual.run_until_parked();
    redraw(visual);
    assert!(editor.read_with(visual, |editor, _cx| editor.view_mode == ViewMode::Preview));
    assert!(
        large_view.read_with(visual, |view, _cx| view.structure_view_active()),
        "CSV search must not replace Preview with Source"
    );
    assert!(visual.debug_bounds("document-host-structured-scroll").is_some());
    visual.simulate_keystrokes("escape");

    editor.update(visual, |editor, cx| {
        editor.set_view_mode(ViewMode::Rendered, cx)
    });
    visual.run_until_parked();
    redraw(visual);
    assert!(large_view.read_with(visual, |view, _cx| view.delimited_live_for_test()));
    assert!(visual.debug_bounds("document-host-structured-add-row").is_some());

    let cell = visual
        .debug_bounds("document-host-structured-cell-0-1")
        .expect("editable score cell");
    visual.simulate_click(cell.center(), Modifiers::default());
    assert_eq!(
        large_view.read_with(visual, |view, _cx| view.structured_selected_cell_for_test()),
        Some((Some(0), 1))
    );
    visual.simulate_keystrokes("tab");
    visual.run_until_parked();
    redraw(visual);
    let selected_after =
        large_view.read_with(visual, |view, _cx| view.structured_selected_cell_for_test());
    assert_eq!(
        selected_after,
        Some((Some(1), 0)),
        "Tab must advance to the next row-major cell"
    );
    visual.simulate_keystrokes("shift-tab");
    assert_eq!(
        large_view.read_with(visual, |view, _cx| view.structured_selected_cell_for_test()),
        Some((Some(0), 1)),
        "Shift+Tab must return to the prior selected cell"
    );
    visual.simulate_click(cell.center(), Modifiers::default());
    visual.simulate_keystrokes("enter");
    redraw(visual);
    let input = large_view.read_with(visual, |view, _cx| view.structured_cell_input_for_test());
    assert_eq!(input.read_with(visual, |block, _cx| block.host_text_size()), Some(12.0));
    let cell_editor = visual
        .debug_bounds("document-host-structured-cell-editor")
        .expect("inline CSV cell editor");
    assert!(
        cell_editor.size.height <= cell.size.height,
        "inline editing must not enlarge the table row"
    );
    visual.simulate_keystrokes("1");
    redraw(visual);
    assert_eq!(
        input.read_with(visual, |block, _cx| block.display_text().to_owned()),
        "101"
    );
    assert!(
        visual
            .debug_bounds("document-host-structured-cell-editor")
            .is_some(),
        "typing in one CSV cell must keep the inline editor mounted"
    );
    assert!(
        visual
            .debug_bounds("document-host-structured-cell-1-1")
            .is_some(),
        "typing in one CSV cell must not replace the table viewport"
    );
    input.update(visual, |block, cx| {
        let len = block.display_text().len();
        block.replace_text_in_visible_range(0..len, "11", None, false, cx);
    });
    visual.simulate_keystrokes("tab");
    visual.run_until_parked();
    redraw(visual);
    let selected_after =
        large_view.read_with(visual, |view, _cx| view.structured_selected_cell_for_test());
    assert_eq!(
        selected_after,
        Some((Some(1), 0)),
        "Tab in a CSV cell editor must move to the next row-major cell"
    );
    assert_eq!(
        large_view.read_with(visual, |view, _cx| view.source_text_for_test()),
        "name,score\r\nAda,11\r\nBob,20\r\n",
        "cell navigation must commit the current field before returning focus to the grid"
    );
    let next_cell = visual
        .debug_bounds("document-host-structured-cell-0-0")
        .expect("next editable CSV cell");
    visual.simulate_click(next_cell.center(), Modifiers::default());
    redraw(visual);
    assert!(
        large_view.read_with(visual, |view, _cx| view.has_structure_view()),
        "committing a CSV cell must keep the existing table visible while its index catches up"
    );
    assert!(visual.debug_bounds("document-host-structured-scroll").is_some());
    visual.run_until_parked();
    redraw(visual);
    assert_eq!(
        large_view.read_with(visual, |view, _cx| view.source_text_for_test()),
        "name,score\r\nAda,11\r\nBob,20\r\n"
    );
    assert_eq!(
        large_view.read_with(visual, |view, _cx| view.structured_selected_cell_for_test()),
        Some((Some(0), 0)),
        "clicking another cell must commit the previous edit before moving selection"
    );
    visual
        .executor()
        .advance_clock(Duration::from_millis(250));
    visual.run_until_parked();
    assert!(
        large_view.read_with(visual, |view, _cx| view
            .structured_loaded_row_count_for_test())
            > 0,
        "installing the refreshed CSV index must retain loaded rows to avoid a blank frame"
    );
    assert!(large_view.read_with(visual, |view, _cx| view.delimited_live_for_test()));

    large_view.update(visual, |view, cx| {
        view.insert_delimited_column_for_test(2, "Column 3", cx)
    });
    visual.run_until_parked();
    assert!(
        large_view.read_with(visual, |view, _cx| view.has_structure_view()),
        "changing CSV columns must keep Live table rendering mounted"
    );
    visual
        .executor()
        .advance_clock(Duration::from_millis(250));
    visual.run_until_parked();
    redraw(visual);
    assert_eq!(
        large_view.read_with(visual, |view, _cx| view.source_text_for_test()),
        "name,score,Column 3\r\nAda,11,\r\nBob,20,\r\n"
    );

    let add_row = visual
        .debug_bounds("document-host-structured-add-row")
        .expect("add row action");
    visual.simulate_click(add_row.center(), Modifiers::default());
    assert!(
        large_view.read_with(visual, |view, _cx| view.delimited_live_for_test()),
        "adding a CSV row must not switch Live editing back to Source"
    );
    assert!(
        large_view.read_with(visual, |view, _cx| view.has_structure_view()),
        "adding a CSV row must keep the table mounted while its index catches up"
    );
    visual.run_until_parked();
    redraw(visual);
    assert_eq!(
        large_view.read_with(visual, |view, _cx| view.source_text_for_test()),
        "name,score,Column 3\r\nAda,11,\r\nBob,20,\r\n,,\r\n"
    );
    visual.update(|window, cx| {
        large_view.update(cx, |view, cx| view.on_undo(&Undo, window, cx));
    });
    assert!(
        large_view.read_with(visual, |view, _cx| view.has_structure_view()),
        "undo in CSV Live editing must not fall back to Source"
    );
    visual.run_until_parked();
    assert_eq!(
        large_view.read_with(visual, |view, _cx| view.source_text_for_test()),
        "name,score,Column 3\r\nAda,11,\r\nBob,20,\r\n"
    );
    visual.update(|window, cx| {
        large_view.update(cx, |view, cx| {
            view.on_redo(&crate::components::Redo, window, cx)
        });
    });
    assert!(
        large_view.read_with(visual, |view, _cx| view.has_structure_view()),
        "redo in CSV Live editing must not fall back to Source"
    );
    visual
        .executor()
        .advance_clock(Duration::from_millis(250));
    visual.run_until_parked();
    assert_eq!(
        large_view.read_with(visual, |view, _cx| view.source_text_for_test()),
        "name,score,Column 3\r\nAda,11,\r\nBob,20,\r\n,,\r\n"
    );

    editor.update(visual, |editor, cx| editor.set_view_mode(ViewMode::Split, cx));
    visual.run_until_parked();
    redraw(visual);
    assert!(visual.debug_bounds("document-host-split-view").is_some());
    let split_source = visual
        .debug_bounds("document-host-split-source")
        .expect("CSV Split source");
    let split_preview = visual
        .debug_bounds("document-host-split-structure")
        .expect("CSV Split preview");
    assert!(
        split_source.right() <= split_preview.left(),
        "CSV Split must place Source on the left and Preview on the right"
    );
    let split_row = visual
        .debug_bounds("document-host-structured-row-0")
        .expect("CSV Split row");
    visual.simulate_click(split_row.center(), Modifiers::default());
    assert!(editor.read_with(visual, |editor, _cx| editor.view_mode == ViewMode::Split));
    assert!(large_view.read_with(visual, |view, _cx| view.structured_split_active()));
    editor.update(visual, |editor, cx| editor.set_view_mode(ViewMode::Source, cx));
    assert!(large_view.read_with(visual, |view, _cx| view.source_view_for_test()));
    editor.update(visual, |editor, cx| editor.set_view_mode(ViewMode::Preview, cx));
    visual.run_until_parked();
    redraw(visual);
    assert!(visual.debug_bounds("document-host-structured-scroll").is_some());
}

#[gpui::test]
async fn large_log_follow_and_external_conflict_preserve_local_edits(cx: &mut TestAppContext) {
    use std::io::Write as _;

    init_editor_test_app(cx);
    let temp = tempfile::tempdir().expect("large log tempdir");
    let path = temp.path().join("follow.log");
    fs::write(&path, "first\n").expect("large log fixture");
    let probe = gmark_paged_document::probe_file(
        &path,
        gmark_paged_document::ProbeOptions {
            max_resident_bytes: 1,
            ..gmark_paged_document::ProbeOptions::default()
        },
    )
    .expect("large log probe");
    let source = gmark_paged_document::FileSource::open(&path).expect("large log source");
    let (editor, visual) =
        cx.add_window_view(move |_window, cx| Editor::from_source_backed_file(cx, path, probe, source));
    visual.run_until_parked();
    redraw(visual);
    let large_view = editor
        .read_with(visual, |editor, _cx| editor.document_host.clone())
        .expect("large log view");
    assert!(large_view.read_with(visual, |view, _cx| view.follow_enabled()));
    editor.update(visual, |editor, cx| {
        editor.external_file_conflict = true;
        editor.restart_file_watcher(cx);
    });
    assert!(!editor.read_with(visual, |editor, _cx| editor.external_file_conflict));

    let mut writer = fs::OpenOptions::new()
        .append(true)
        .open(temp.path().join("follow.log"))
        .expect("append log");
    writer.write_all(b"second\n").expect("append second line");
    writer.sync_all().expect("sync appended log");
    visual
        .executor()
        .advance_clock(Duration::from_millis(1_100));
    visual.run_until_parked();
    redraw(visual);
    assert_eq!(
        large_view.read_with(visual, |view, _cx| view.recovered_text_for_test()),
        Some(b"first\nsecond\n".to_vec())
    );
    assert!(
        large_view
            .read_with(visual, |view, _cx| view.pending_external_change_for_test())
            .is_none()
    );

    visual.update(|window, cx| {
        large_view.update(cx, |view, cx| view.begin_line_edit_for_test(0, window, cx));
    });
    let (_, edit) = large_view
        .read_with(visual, |view, _cx| view.active_edit_for_test())
        .expect("active log edit");
    edit.update(visual, |block, cx| {
        block.replace_text_in_visible_range(0..0, "local ", None, false, cx);
    });
    visual.run_until_parked();
    assert!(editor.read_with(visual, |editor, _cx| editor.document_dirty));
    assert!(!large_view.read_with(visual, |view, _cx| view.follow_enabled()));

    writer.write_all(b"remote\n").expect("append remote line");
    writer.sync_all().expect("sync remote line");
    visual
        .executor()
        .advance_clock(Duration::from_millis(1_100));
    visual.run_until_parked();
    redraw(visual);
    assert!(matches!(
        large_view.read_with(visual, |view, _cx| view.pending_external_change_for_test()),
        Some(gmark_paged_document::ExternalChange::Appended { .. })
    ));
    assert!(
        visual
            .debug_bounds("document-host-external-change-banner")
            .is_some()
    );
    assert!(
        large_view
            .read_with(visual, |view, _cx| view.recovered_text_for_test())
            .is_some_and(|text| {
                text.starts_with(b"local first\nsecond\n") && !text.ends_with(b"remote\n")
            })
    );

    large_view.update(visual, |view, cx| view.keep_local_for_test(cx));
    assert!(
        large_view
            .read_with(visual, |view, _cx| view.pending_external_change_for_test())
            .is_none()
    );
    assert!(large_view.read_with(visual, |view, _cx| view.external_monitor_paused_for_test()));
    assert!(editor.read_with(visual, |editor, _cx| editor.document_dirty));
}
