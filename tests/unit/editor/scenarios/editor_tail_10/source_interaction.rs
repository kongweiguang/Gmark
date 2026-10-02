// @author kongweiguang

#[gpui::test]
async fn large_source_shaped_layout_cache_reuses_and_invalidates_complete_keys(
    cx: &mut TestAppContext,
) {
    init_editor_test_app(cx);
    let temp = tempfile::tempdir().expect("large layout cache tempdir");
    let path = temp.path().join("layout-cache.txt");
    fs::write(&path, "alpha\n世界🙂\nomega\n".repeat(128)).expect("layout cache fixture");
    let probe = gmark_paged_document::probe_file(
        &path,
        gmark_paged_document::ProbeOptions {
            max_resident_bytes: 1,
            ..gmark_paged_document::ProbeOptions::default()
        },
    )
    .expect("layout cache probe");
    let source = gmark_paged_document::FileSource::open(&path).expect("layout cache source");
    let (editor, visual) = cx.add_window_view(move |_window, cx| {
        Editor::from_source_backed_file(cx, path, probe, source)
    });
    visual.run_until_parked();
    redraw(visual);
    let large_view = editor
        .read_with(visual, |editor, _cx| editor.document_host.clone())
        .expect("large layout cache view");
    let (initial_hits, initial_misses, initial_entries) = large_view
        .read_with(visual, |view, cx| {
            view.source_layout_cache_metrics_for_test(cx)
        });
    assert!(initial_misses > 0);
    assert!((1..=512).contains(&initial_entries));

    visual.update(|_window, cx| cx.refresh_windows());
    redraw(visual);
    let (reused_hits, reused_misses, reused_entries) = large_view.read_with(visual, |view, cx| {
        view.source_layout_cache_metrics_for_test(cx)
    });
    assert!(reused_hits > initial_hits);
    assert_eq!(reused_misses, initial_misses);
    assert_eq!(reused_entries, initial_entries);

    visual.update(|_window, cx| {
        let platform_appearance = cx.window_appearance();
        assert!(cx.update_global::<ThemeManager, _>(|manager, _cx| {
            manager.set_theme_preference(
                ThemeAppearance::Light,
                ThemePalette::Xcode,
                platform_appearance,
            )
        }));
        cx.refresh_windows();
    });
    redraw(visual);
    let (_theme_hits, theme_misses, theme_entries) = large_view.read_with(visual, |view, cx| {
        view.source_layout_cache_metrics_for_test(cx)
    });
    assert!(theme_misses > reused_misses);
    assert_eq!(theme_entries, reused_entries);
}

#[gpui::test]
async fn large_source_pointer_selection_is_character_precise_cross_line_and_reversible(
    cx: &mut TestAppContext,
) {
    init_editor_test_app(cx);
    let temp = tempfile::tempdir().expect("large pointer selection tempdir");
    let path = temp.path().join("pointer-selection.txt");
    fs::write(&path, "alpha bravo\n世界🙂 tail\nthird line\n")
        .expect("large pointer selection fixture");
    let probe = gmark_paged_document::probe_file(
        &path,
        gmark_paged_document::ProbeOptions {
            max_resident_bytes: 1,
            ..gmark_paged_document::ProbeOptions::default()
        },
    )
    .expect("large pointer selection probe");
    let source =
        gmark_paged_document::FileSource::open(&path).expect("large pointer selection source");
    let (editor, visual) = cx.add_window_view(move |_window, cx| {
        Editor::from_source_backed_file(cx, path, probe, source)
    });
    visual.run_until_parked();
    redraw(visual);
    let large_view = editor
        .read_with(visual, |editor, _cx| editor.document_host.clone())
        .expect("large pointer selection view");

    let first = visual
        .debug_bounds("document-host-line-body-0")
        .expect("first source row bounds");
    let third = visual
        .debug_bounds("document-host-line-body-2")
        .expect("third source row bounds");
    let forward_start = point(first.left() + px(28.0), first.center().y);
    let forward_end = point(third.left() + px(42.0), third.center().y);
    visual.simulate_mouse_down(forward_start, MouseButton::Left, Modifiers::default());
    visual.simulate_mouse_move(forward_end, MouseButton::Left, Modifiers::default());
    visual.simulate_mouse_up(forward_end, MouseButton::Left, Modifiers::default());
    visual.run_until_parked();

    let forward = large_view
        .read_with(visual, |view, _cx| view.source_selection_for_test())
        .expect("forward source selection");
    assert!(!forward.reversed());
    assert!(forward.range().start > 0 && forward.range().start < 11);
    assert!(forward.range().end > 28 && forward.range().end < 38);

    visual.simulate_mouse_down(forward_end, MouseButton::Left, Modifiers::default());
    visual.simulate_mouse_move(forward_start, MouseButton::Left, Modifiers::default());
    visual.simulate_mouse_up(forward_start, MouseButton::Left, Modifiers::default());
    visual.run_until_parked();
    let reversed = large_view
        .read_with(visual, |view, _cx| view.source_selection_for_test())
        .expect("reversed source selection");
    assert!(reversed.reversed());
    assert!(reversed.range().start > 0 && reversed.range().start < 11);
    assert!(reversed.range().end > 28 && reversed.range().end < 38);
    assert!(large_view.read_with(visual, |view, _cx| {
        view.source_row_block_count_for_test() <= 512
    }));
    let (_revision, generation, _epoch, column, visible, rows, epochs_match, revision_matches) =
        large_view.read_with(visual, |view, _cx| view.screen_lines_contract_for_test());
    assert!(generation > 0);
    assert_eq!(column, 0);
    assert!(!visible.is_empty());
    assert!((1..=512).contains(&rows));
    assert!(epochs_match);
    assert!(revision_matches);
    let metrics = large_view.read_with(visual, |view, _cx| view.metrics_for_test());
    assert!(metrics.viewport_requests > 0 && metrics.viewport_installs > 0);
    assert!((1..=512).contains(&metrics.max_cached_rows));
    assert_eq!(metrics.blank_frames_after_content, 0);

    visual.simulate_mouse_down(forward_start, MouseButton::Right, Modifiers::default());
    visual.simulate_mouse_up(forward_start, MouseButton::Right, Modifiers::default());
    assert!(large_view.read_with(visual, |view, _cx| {
        view.source_context_menu_open_for_test()
    }));
    redraw(visual);
    let context_menu = visual
        .debug_bounds("document-host-source-context-menu")
        .expect("source context menu bounds");
    assert!(
        f32::from(context_menu.left() - forward_start.x).abs() <= 12.0,
        "source context menu should stay near the pointer horizontally"
    );
    assert!(
        f32::from(context_menu.top() - forward_start.y).abs() <= 12.0,
        "source context menu should stay near the pointer vertically"
    );
    assert!(
        visual
            .debug_bounds("large-source-context-export-utf8")
            .is_some()
    );
    large_view.update(visual, |view, cx| view.show_source_view(cx));
    redraw(visual);
    assert!(!large_view.read_with(visual, |view, _cx| {
        view.source_context_menu_open_for_test()
    }));

    visual.simulate_mouse_down(forward_start, MouseButton::Right, Modifiers::default());
    visual.simulate_mouse_up(forward_start, MouseButton::Right, Modifiers::default());
    redraw(visual);
    visual.update(|window, cx| {
        assert!(
            large_view
                .read(cx)
                .source_context_menu_is_focused_for_test(window)
        );
    });
    visual.simulate_keystrokes("escape");
    redraw(visual);
    assert!(!large_view.read_with(visual, |view, _cx| {
        view.source_context_menu_open_for_test()
    }));
    visual.update(|window, cx| {
        assert!(large_view.read(cx).host_is_focused_for_test(window));
    });
}

/// Keeps selection expansion tied to the newly mounted rows while the edge timer advances.
#[gpui::test]
async fn large_source_drag_autoscroll_extends_selection_beyond_mounted_viewport(
    cx: &mut TestAppContext,
) {
    init_editor_test_app(cx);
    let temp = tempfile::tempdir().expect("large drag autoscroll tempdir");
    let path = temp.path().join("drag-autoscroll.txt");
    let text = (0..400)
        .map(|line| format!("source line {line:04} with selectable text\n"))
        .collect::<String>();
    fs::write(&path, text).expect("large drag autoscroll fixture");
    let probe = gmark_paged_document::probe_file(
        &path,
        gmark_paged_document::ProbeOptions {
            max_resident_bytes: 1,
            ..gmark_paged_document::ProbeOptions::default()
        },
    )
    .expect("large drag autoscroll probe");
    let source = gmark_paged_document::FileSource::open(&path).expect("drag autoscroll source");
    let (editor, visual) = cx.add_window_view(move |_window, cx| {
        Editor::from_source_backed_file(cx, path, probe, source)
    });
    visual.simulate_resize(size(px(720.0), px(520.0)));
    visual.run_until_parked();
    redraw(visual);
    let large_view = editor
        .read_with(visual, |editor, _cx| editor.document_host.clone())
        .expect("large drag autoscroll view");
    let first = visual
        .debug_bounds("document-host-line-body-0")
        .expect("first source row");
    let viewport = visual
        .debug_bounds("document-host-source-horizontal-scroll")
        .expect("source viewport");
    let start = point(first.left() + px(8.0), first.center().y);
    let edge = point(first.left() + px(48.0), viewport.bottom() - px(2.0));
    visual.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
    large_view.update(visual, |view, cx| {
        view.start_drag_autoscroll_for_test(1, cx);
    });

    for _ in 0..24 {
        large_view.update(visual, |view, cx| {
            assert!(view.drag_autoscroll_tick_for_test(cx));
        });
        visual.run_until_parked();
        redraw(visual);
    }
    visual.simulate_mouse_up(edge, MouseButton::Left, Modifiers::default());
    visual.run_until_parked();

    assert!(large_view.read_with(visual, |view, _cx| view.scroll_top_line_for_test()) > 0);
    let selection = large_view
        .read_with(visual, |view, _cx| view.source_selection_for_test())
        .expect("autoscroll Source selection");
    assert!(selection.range().end > 200, "selection={selection:?}");
}

/// A drag at either file edge must reach the edge once and stop refreshing; its anchor
/// stays intact so reversing the drag still selects from the original mouse-down position.
#[gpui::test]
async fn large_source_drag_autoscroll_stops_at_document_edges(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let temp = tempfile::tempdir().expect("source edge drag tempdir");
    let path = temp.path().join("drag-edges.txt");
    let text = "first\r\nmiddle\r\nlast";
    fs::write(&path, text).expect("source edge drag fixture");
    let probe = gmark_paged_document::probe_file(
        &path,
        gmark_paged_document::ProbeOptions {
            max_resident_bytes: 1,
            ..gmark_paged_document::ProbeOptions::default()
        },
    )
    .expect("source edge drag probe");
    let source = gmark_paged_document::FileSource::open(&path).expect("source edge drag source");
    let (editor, visual) = cx.add_window_view(move |_window, cx| {
        Editor::from_source_backed_file(cx, path, probe, source)
    });
    visual.simulate_resize(size(px(720.0), px(520.0)));
    visual.run_until_parked();
    redraw(visual);
    let host = editor
        .read_with(visual, |editor, _cx| editor.document_host.clone())
        .expect("source edge drag Host");
    host.update(visual, |host, _cx| {
        host.select_source_range_for_test(4..5, false);
    });

    for (direction, expected_head) in [(-1, 0), (1, text.len() as u64)] {
        host.update(visual, |host, cx| {
            host.start_drag_autoscroll_for_test(direction, cx);
            assert!(
                !host.drag_autoscroll_tick_for_test(cx),
                "a drag already at the file edge must stop its timer"
            );
            let selection = host.source_selection_for_test().expect("edge selection");
            assert_eq!(selection.anchor.byte_offset, 4, "preserve the drag anchor");
            assert_eq!(selection.head.byte_offset, expected_head);
            assert!(
                !host.drag_autoscroll_tick_for_test(cx),
                "a stopped timer must stay idle until the pointer changes direction"
            );
            assert_eq!(host.source_selection_for_test(), Some(selection));
            assert_eq!(host.scroll_top_line_for_test(), 0);
            assert_eq!(host.source_text_for_test(), text);
        });
        redraw(visual);
    }
}

/// Verifies the first Ctrl+A waits for Source IME completion and selects committed document bytes.
#[cfg(target_os = "windows")]
#[gpui::test]
async fn large_source_ime_composition_commits_one_piece_tree_undo_transaction(
    cx: &mut TestAppContext,
) {
    init_editor_test_app(cx);
    let temp = tempfile::tempdir().expect("large IME tempdir");
    let path = temp.path().join("ime-source.txt");
    fs::write(&path, "alpha\n").expect("large IME fixture");
    let probe = gmark_paged_document::probe_file(
        &path,
        gmark_paged_document::ProbeOptions {
            max_resident_bytes: 1,
            ..gmark_paged_document::ProbeOptions::default()
        },
    )
    .expect("large IME probe");
    let source = gmark_paged_document::FileSource::open(&path).expect("large IME source");
    let (editor, visual) = cx.add_window_view(move |_window, cx| {
        Editor::from_source_backed_file(cx, path, probe, source)
    });
    visual.run_until_parked();
    redraw(visual);
    let large_view = editor
        .read_with(visual, |editor, _cx| editor.document_host.clone())
        .expect("large IME view");
    visual.update(|window, cx| {
        large_view.update(cx, |view, cx| view.begin_line_edit_for_test(0, window, cx));
    });
    redraw(visual);
    let block = large_view
        .read_with(visual, |view, _cx| view.active_edit_for_test())
        .expect("active large source block")
        .1;

    visual.update(|window, cx| {
        block.update(cx, |block, block_cx| {
            let composing = "拼音🙂";
            let utf16_end = composing.encode_utf16().count();
            <crate::components::Block as EntityInputHandler>::replace_and_mark_text_in_range(
                block,
                None,
                composing,
                Some(utf16_end..utf16_end),
                window,
                block_cx,
            );
        });
    });
    visual.run_until_parked();
    block.read_with(visual, |block, _cx| {
        assert_eq!(block.display_text(), "alpha");
        assert!(block.has_ime_composition());
        assert_eq!(
            block
                .ime_visible_text(&crate::components::BlockImeCompositionOwner::BlockText)
                .as_deref(),
            Some("alpha拼音🙂"),
            "unconfirmed candidate text should remain a virtual Source surface"
        );
    });
    assert_eq!(
        large_view.read_with(visual, |view, _cx| view.recovered_text_for_test()),
        Some(b"alpha\n".to_vec()),
        "marked text must stay transient until IME commit"
    );

    visual.update(|window, cx| {
        block.update(cx, |block, block_cx| {
            <crate::components::Block as EntityInputHandler>::replace_text_in_range(
                block,
                None,
                "中文🙂",
                window,
                block_cx,
            );
        });
    });
    visual.run_until_parked();
    block.read_with(visual, |block, _cx| {
        assert_eq!(block.display_text(), "alpha");
        assert!(block.has_ime_composition());
        assert_eq!(
            block
                .ime_visible_text(&crate::components::BlockImeCompositionOwner::BlockText)
                .as_deref(),
            Some("alpha中文🙂"),
            "confirmed result remains staged until the native terminal event"
        );
    });
    assert_eq!(
        large_view.read_with(visual, |view, _cx| view.recovered_text_for_test()),
        Some(b"alpha\n".to_vec()),
        "PieceTree must not receive IME text before composition end"
    );
    visual.simulate_keystrokes("ctrl-a");

    visual.update(|window, cx| {
        block.update(cx, |block, block_cx| {
            <crate::components::Block as EntityInputHandler>::composition_ended(
                block,
                gpui::CompositionEnd::Committed,
                window,
                block_cx,
            );
        });
    });
    visual.run_until_parked();
    assert_eq!(
        large_view.read_with(visual, |view, _cx| view.recovered_text_for_test()),
        Some("alpha中文🙂\n".as_bytes().to_vec())
    );
    let selection = large_view
        .read_with(visual, |view, _cx| view.source_selection_for_test())
        .expect("whole-document selection after IME terminal");
    assert_eq!(selection.range(), 0.."alpha中文🙂\n".len() as u64);

    visual.update(|window, cx| {
        large_view.update(cx, |view, cx| view.undo_for_test(window, cx));
    });
    assert_eq!(
        large_view.read_with(visual, |view, _cx| view.recovered_text_for_test()),
        Some(b"alpha\n".to_vec())
    );
    visual.update(|window, cx| {
        large_view.update(cx, |view, cx| view.redo_for_test(window, cx));
    });
    assert_eq!(
        large_view.read_with(visual, |view, _cx| view.recovered_text_for_test()),
        Some("alpha中文🙂\n".as_bytes().to_vec())
    );
}

#[gpui::test]
async fn large_source_cross_line_paste_is_one_reversible_source_transaction(
    cx: &mut TestAppContext,
) {
    init_editor_test_app(cx);
    let temp = tempfile::tempdir().expect("large paste tempdir");
    let path = temp.path().join("paste-source.txt");
    fs::write(&path, "alpha\nbeta\ngamma\n").expect("large paste fixture");
    let probe = gmark_paged_document::probe_file(
        &path,
        gmark_paged_document::ProbeOptions {
            max_resident_bytes: 1,
            ..gmark_paged_document::ProbeOptions::default()
        },
    )
    .expect("large paste probe");
    let source = gmark_paged_document::FileSource::open(&path).expect("large paste source");
    let (editor, visual) = cx.add_window_view(move |_window, cx| {
        Editor::from_source_backed_file(cx, path, probe, source)
    });
    visual.run_until_parked();
    redraw(visual);
    let large_view = editor
        .read_with(visual, |editor, _cx| editor.document_host.clone())
        .expect("large paste view");
    visual.write_to_clipboard(gpui::ClipboardItem::new_string("中\n🙂".to_owned()));
    visual.update(|window, cx| {
        large_view.update(cx, |view, cx| {
            view.select_source_range_for_test(3..13, true);
            view.paste_for_test(window, cx);
        });
    });
    visual.run_until_parked();
    assert_eq!(
        large_view.read_with(visual, |view, _cx| view.recovered_text_for_test()),
        Some("alp中\n🙂mma\n".as_bytes().to_vec())
    );
    let pasted_selection = large_view
        .read_with(visual, |view, _cx| view.source_selection_for_test())
        .expect("selection after paste");

    visual.update(|window, cx| {
        large_view.update(cx, |view, cx| view.undo_for_test(window, cx));
    });
    assert_eq!(
        large_view.read_with(visual, |view, _cx| view.recovered_text_for_test()),
        Some(b"alpha\nbeta\ngamma\n".to_vec())
    );
    assert_eq!(
        large_view.read_with(visual, |view, _cx| view.source_selection_for_test()),
        Some(gmark_document_core::SourceSelection::from_range(
            3..13,
            true
        ))
    );
    visual.update(|window, cx| {
        large_view.update(cx, |view, cx| view.redo_for_test(window, cx));
    });
    assert_eq!(
        large_view.read_with(visual, |view, _cx| view.recovered_text_for_test()),
        Some("alp中\n🙂mma\n".as_bytes().to_vec())
    );
    assert_eq!(
        large_view.read_with(visual, |view, _cx| view.source_selection_for_test()),
        Some(pasted_selection)
    );
}

/// Verifies the first Ctrl+A selects the entire virtual Source document.
#[gpui::test]
async fn large_source_first_ctrl_a_selects_lazy_document_range(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let temp = tempfile::tempdir().expect("large select-all tempdir");
    let path = temp.path().join("select-all-source.txt");
    let source_text = "alpha\n世界🙂\nomega\n";
    fs::write(&path, source_text).expect("large select-all fixture");
    let probe = gmark_paged_document::probe_file(
        &path,
        gmark_paged_document::ProbeOptions {
            max_resident_bytes: 1,
            ..gmark_paged_document::ProbeOptions::default()
        },
    )
    .expect("large select-all probe");
    let source = gmark_paged_document::FileSource::open(&path).expect("large select-all source");
    let (editor, visual) = cx.add_window_view(move |_window, cx| {
        Editor::from_source_backed_file(cx, path, probe, source)
    });
    visual.run_until_parked();
    redraw(visual);
    let large_view = editor
        .read_with(visual, |editor, _cx| editor.document_host.clone())
        .expect("large select-all view");
    visual.update(|window, cx| {
        large_view.update(cx, |view, cx| view.begin_line_edit_for_test(0, window, cx));
    });
    redraw(visual);

    visual.simulate_keystrokes("ctrl-a");
    let document_selection = large_view
        .read_with(visual, |view, _cx| view.source_selection_for_test())
        .expect("whole-document selection");
    assert_eq!(document_selection.range(), 0..source_text.len() as u64);
    assert!(
        large_view
            .read_with(visual, |view, _cx| view.active_edit_for_test())
            .is_none()
    );
}
