// @author kongweiguang

/// 大文件沿用标准编辑壳与当前平台历史键位；测试同时锁定虚拟化、保存和重做后的脏状态。
#[gpui::test]
async fn large_document_uses_the_standard_editor_shell(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let temp = tempfile::tempdir().expect("large document tempdir");
    let path = temp.path().join("large-shell.md");
    let text = (0..5_000)
        .map(|line| format!("large document line {line}\n"))
        .collect::<String>();
    fs::write(&path, text).expect("large document fixture");
    let probe = gmark_paged_document::probe_file(
        &path,
        gmark_paged_document::ProbeOptions {
            max_resident_bytes: 1,
            ..gmark_paged_document::ProbeOptions::default()
        },
    )
    .expect("large document probe");
    assert_eq!(probe.strategy, gmark_paged_document::OpenStrategy::Paged);
    let source = gmark_paged_document::FileSource::open(&path).expect("large document source");
    let (editor, visual) = cx.add_window_view(move |_window, cx| {
        Editor::from_source_backed_file(cx, path, probe, source)
    });

    for viewport in [size(px(1180.0), px(780.0)), size(px(720.0), px(520.0))] {
        visual.simulate_resize(viewport);
        redraw(visual);
        visual.update(|window, _cx| {
            assert_eq!(
                window.scale_factor(),
                2.0,
                "large-file visual coverage runs at 200% scale"
            )
        });
        let shell = visual.debug_bounds("editor-main-content").unwrap();
        let content = visual.debug_bounds("editor-content").unwrap();
        let large_content = visual.debug_bounds("document-host-tab-content").unwrap();
        let tab_strip = visual.debug_bounds("document-tab-strip").unwrap();
        let status_bar = visual.debug_bounds("status-bar").unwrap();
        let large_status = visual
            .debug_bounds("status-bar-document-host-status")
            .unwrap();

        if cfg!(any(target_os = "windows", target_os = "macos")) {
            assert!(visual.debug_bounds("editor-titlebar").is_some());
        }
        assert!(visual.debug_bounds("status-bar-mode-switch").is_some());
        let fixed_mode = visual.debug_bounds("status-bar-mode-switch").unwrap();
        visual.simulate_click(fixed_mode.center(), Modifiers::default());
        redraw(visual);
        assert!(
            visual.debug_bounds("status-bar-mode-menu").is_none(),
            "a fixed Paged Source status must not open a misleading mode menu"
        );
        assert!(visual.debug_bounds("status-bar-mode-Source").is_some());
        for unavailable in [
            "status-bar-mode-Rendered",
            "status-bar-mode-Split",
            "status-bar-mode-Preview",
        ] {
            assert!(
                visual.debug_bounds(unavailable).is_none(),
                "large documents expose one fixed Source mode, not a misleading switch"
            );
        }
        assert!(
            visual
                .debug_bounds("status-bar-format-overflow-button")
                .is_some()
        );
        assert!(visual.debug_bounds("document-host-source-mode").is_none());
        assert!(large_content.left() >= content.left());
        assert!(large_content.right() <= content.right());
        assert_eq!(tab_strip.bottom(), content.top());
        assert_eq!(status_bar.top(), shell.bottom());
        assert!(large_status.right() <= status_bar.right());
        if let Some(first_body) = visual.debug_bounds("document-host-line-body-0") {
            let document_host = visual
                .debug_bounds("document-host-source-horizontal-scroll")
                .expect("large Source surface");
            let expected_inset = Theme::default_theme().dimensions.editor_padding;
            assert!(
                first_body.top() >= document_host.top() + px(expected_inset - 1.0),
                "large Source keeps the same reading top inset as ordinary Source"
            );
        }
    }

    for appearance in [ThemeAppearance::Light, ThemeAppearance::Dark] {
        visual.update(|_window, cx| {
            let platform_appearance = cx.window_appearance();
            assert!(cx.update_global::<ThemeManager, _>(|manager, _cx| {
                manager.set_theme_preference(appearance, ThemePalette::Xcode, platform_appearance)
            }));
            cx.refresh_windows();
        });
        redraw(visual);
        assert!(visual.debug_bounds("document-host-tab-content").is_some());
        assert!(visual.debug_bounds("status-bar-mode-switch").is_some());
        assert!(visual.debug_bounds("document-host-scrollbar").is_some());
    }

    visual.executor().advance_clock(Duration::from_millis(50));
    redraw(visual);
    let large_view = editor
        .read_with(visual, |editor, _cx| editor.document_host.clone())
        .expect("large document view");
    let initial_scroll_top =
        large_view.read_with(visual, |view, _cx| view.scroll_top_line_for_test());
    large_view.read_with(visual, |view, _cx| view.scroll_to_line_for_test(4_000));
    visual.update(|window, cx| window.draw(cx).clear());
    assert!(
        visual
            .debug_bounds("document-host-retained-frame-progress")
            .is_some(),
        "a disjoint jump must retain the previous ScreenLines instead of painting a blank frame"
    );
    let distant_scroll_top =
        large_view.read_with(visual, |view, _cx| view.scroll_top_line_for_test());
    assert!(distant_scroll_top > initial_scroll_top);
    large_view.read_with(visual, |view, _cx| {
        view.scroll_to_line_for_test(initial_scroll_top)
    });
    redraw(visual);
    assert_eq!(
        large_view.read_with(visual, |view, _cx| view.scroll_top_line_for_test()),
        initial_scroll_top
    );
    assert!(
        large_view.read_with(visual, |view, _cx| view.viewport_cancellations_for_test()) > 0,
        "a disjoint jump supersedes the in-flight viewport read"
    );

    let inactive_body = visual
        .debug_bounds("document-host-line-body-0")
        .expect("inactive large source row body");
    visual.update(|window, cx| {
        large_view.update(cx, |view, cx| view.begin_line_edit_for_test(0, window, cx));
    });
    redraw(visual);
    let active_body = visual
        .debug_bounds("document-host-line-body-0")
        .expect("active large source row body");
    assert_eq!(active_body, inactive_body);
    assert!(
        large_view.read_with(visual, |view, _cx| view.source_row_height_for_test()) > 24.0,
        "large Source must inherit ordinary editor typography instead of the old 22 px row"
    );
    assert!(large_view.read_with(visual, |view, _cx| {
        view.active_edit_for_test()
            .is_some_and(|(_, block)| block.read(_cx).compact_source_host())
    }));
    visual.simulate_keystrokes("ctrl-g");
    redraw(visual);
    assert!(
        visual
            .debug_bounds("document-host-navigation-panel")
            .is_some()
    );
    large_view.update(visual, |view, cx| view.close_navigation_for_test(cx));
    redraw(visual);

    visual.update(|window, cx| {
        large_view.update(cx, |view, cx| view.begin_line_edit_for_test(0, window, cx));
    });
    redraw(visual);
    let focused_scroll_top =
        large_view.read_with(visual, |view, _cx| view.scroll_top_line_for_test());
    visual.simulate_keystrokes("pagedown");
    redraw(visual);
    assert!(
        large_view.read_with(visual, |view, _cx| view.scroll_top_line_for_test())
            > focused_scroll_top
    );
    visual.simulate_keystrokes("pageup");
    redraw(visual);
    assert_eq!(
        large_view.read_with(visual, |view, _cx| view.scroll_top_line_for_test()),
        focused_scroll_top
    );
    assert!(large_view.read_with(visual, |view, _cx| view.source_view_for_test()));
    assert!(editor.read_with(visual, |editor, _cx| editor.view_mode == ViewMode::Source));
    editor.update(visual, |editor, cx| {
        editor.set_view_mode(ViewMode::Preview, cx)
    });
    assert!(editor.read_with(visual, |editor, _cx| editor.view_mode == ViewMode::Source));
    assert!(
        large_view
            .read_with(visual, |view, cx| {
                view.status_text(cx.global::<crate::i18n::I18nManager>().strings())
                    .to_string()
            })
            .contains("Preview needs a resident Markdown projection")
    );
    editor.update(visual, |editor, cx| {
        editor.set_view_mode(ViewMode::Source, cx)
    });
    assert!(visual.debug_bounds("document-host-find-panel").is_none());
    visual.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.on_find_in_document_action(&crate::components::FindInDocument, window, cx);
        });
    });
    redraw(visual);
    assert!(visual.debug_bounds("document-host-find-panel").is_some());
    assert!(visual.debug_bounds("document-host-search-input").is_some());
    assert!(visual.debug_bounds("document-host-scrollbar").is_some());
    assert!(editor.read_with(visual, |editor, _cx| editor.document_host.is_some()));
    visual.simulate_input("stale query");
    visual.simulate_keystrokes("ctrl-a");
    visual.simulate_input("line 400");
    visual.run_until_parked();
    assert_eq!(
        large_view.read_with(visual, |view, cx| view.search_text_for_test(cx)),
        "line 400"
    );

    visual.simulate_keystrokes("escape");
    visual.simulate_keystrokes("ctrl-g");
    redraw(visual);
    assert!(
        visual
            .debug_bounds("document-host-navigation-panel")
            .is_some()
    );
    assert!(
        visual
            .debug_bounds("document-host-navigation-input")
            .is_some()
    );
    visual.simulate_input("400");
    visual.run_until_parked();
    assert_eq!(
        large_view.read_with(visual, |view, cx| view.cursor_position(cx)),
        (400, 1)
    );
    visual.simulate_keystrokes("enter");
    visual.run_until_parked();
    redraw(visual);
    assert!(!large_view.read_with(visual, |view, _cx| view.navigation_visible_for_test()));

    let overflow = visual
        .debug_bounds("status-bar-format-overflow-button")
        .unwrap();
    visual.simulate_click(overflow.center(), Modifiers::default());
    redraw(visual);
    assert!(
        visual
            .debug_bounds("status-bar-large-reopen-utf16-le")
            .is_some(),
        "manual encoding reopen must be reachable from the standard status overflow"
    );
    let line_endings = visual
        .debug_bounds("status-bar-large-line-endings")
        .unwrap();
    visual.simulate_click(line_endings.center(), Modifiers::default());
    redraw(visual);
    assert!(large_view.read_with(visual, |view, _cx| view.line_endings_visible()));

    let overflow = visual
        .debug_bounds("status-bar-format-overflow-button")
        .unwrap();
    visual.simulate_click(overflow.center(), Modifiers::default());
    redraw(visual);
    let follow = visual.debug_bounds("status-bar-large-follow").unwrap();
    visual.simulate_click(follow.center(), Modifiers::default());
    redraw(visual);
    assert!(large_view.read_with(visual, |view, _cx| view.follow_enabled()));

    visual.update(|window, cx| {
        large_view.update(cx, |view, cx| view.begin_line_edit_for_test(0, window, cx));
    });
    let (_, edit_block) = large_view
        .read_with(visual, |view, _cx| view.active_edit_for_test())
        .expect("active large line edit");
    let cached_before_edit =
        large_view.read_with(visual, |view, _cx| view.source_cache_len_for_test());
    assert!(cached_before_edit > 0);
    let (unchanged_line, unchanged_row_block) = large_view
        .read_with(visual, |view, _cx| {
            view.inactive_source_row_block_for_test()
        })
        .expect("unchanged source row block");
    let line_end = edit_block.read_with(visual, |block, _cx| block.display_text().len());
    edit_block.update(visual, |block, cx| {
        block.replace_text_in_visible_range(line_end..line_end, "x", None, false, cx);
    });
    visual.run_until_parked();
    assert_eq!(
        large_view.read_with(visual, |view, _cx| {
            view.source_row_block_for_test(unchanged_line)
        }),
        Some(unchanged_row_block),
        "byte-range shifts must retain Block entities for rows whose visible text is unchanged"
    );
    visual.update(|window, cx| {
        large_view.update(cx, |view, cx| view.undo_for_test(window, cx));
    });
    visual.run_until_parked();
    visual.update(|window, cx| {
        large_view.update(cx, |view, cx| view.begin_line_edit_for_test(0, window, cx));
    });
    visual.run_until_parked();
    let (_, edit_block) = large_view
        .read_with(visual, |view, _cx| view.active_edit_for_test())
        .expect("reanchored large line edit after undo");
    edit_block.update(visual, |block, cx| {
        block.replace_text_in_visible_range(5..5, "\n", None, false, cx);
    });
    assert!(
        large_view.read_with(visual, |view, _cx| view.source_cache_len_for_test()) > 0,
        "typing must retain the last painted viewport until replacement rows arrive"
    );
    visual.run_until_parked();
    assert!(large_view.read_with(visual, |view, _cx| view.source_cache_len_for_test()) <= 1_024);
    assert!(large_view.read_with(visual, |view, _cx| view.source_row_is_current_for_test(0)));
    assert!(!large_view.read_with(visual, |view, _cx| view.follow_enabled()));
    assert_eq!(
        large_view
            .read_with(visual, |view, _cx| view.active_edit_for_test())
            .map(|(line, _)| line),
        Some(1)
    );
    assert!(
        large_view
            .read_with(visual, |view, _cx| view.recovered_text_for_test())
            .is_some_and(|text| text.starts_with(b"large\n document line 0\n"))
    );
    assert!(large_view.read_with(visual, |view, _cx| view.error_for_test().is_none()));
    assert!(
        large_view.read_with(visual, |view, _cx| view
            .structure_error_for_test()
            .is_none()),
        "successful Source editing must not show a structured-view warning as an error banner"
    );

    redraw(visual);
    let stable_active_body = visual
        .debug_bounds("document-host-line-body-1")
        .expect("reanchored active row body");
    for _ in 0..3 {
        redraw(visual);
        assert_eq!(
            visual
                .debug_bounds("document-host-line-body-1")
                .expect("stable active row body"),
            stable_active_body,
            "settled viewport rows must not alternate geometry between frames"
        );
    }
    assert!(editor.read_with(visual, |editor, _cx| editor.document_dirty));
    visual.simulate_keystrokes("ctrl-z");
    visual.run_until_parked();
    redraw(visual);
    assert!(!editor.read_with(visual, |editor, _cx| editor.document_dirty));
    assert!(
        large_view
            .read_with(visual, |view, _cx| view.recovered_text_for_test())
            .is_some_and(|text| text.starts_with(b"large document line 0\n"))
    );
    assert!(visual.update(|window, cx| { large_view.read(cx).host_is_focused_for_test(window) }));

    let redo_shortcut = if cfg!(target_os = "macos") {
        "cmd-shift-z"
    } else {
        "ctrl-shift-z"
    };
    visual.simulate_keystrokes(redo_shortcut);
    visual.run_until_parked();
    redraw(visual);
    assert!(editor.read_with(visual, |editor, _cx| editor.document_dirty));
    visual.update(|window, cx| {
        large_view.update(cx, |view, cx| {
            view.begin_line_edit_for_test(400, window, cx)
        });
    });
    large_view.read_with(visual, |view, _cx| view.scroll_to_line_for_test(400));
    redraw(visual);
    let scroll_top_before_save =
        large_view.read_with(visual, |view, _cx| view.scroll_top_line_for_test());
    assert!(scroll_top_before_save > 0);
    visual.simulate_keystrokes("ctrl-s");
    wait_for_large_document_save(&large_view, visual);
    redraw(visual);
    assert!(!editor.read_with(visual, |editor, _cx| editor.document_dirty));
    let scroll_top_after_save =
        large_view.read_with(visual, |view, _cx| view.scroll_top_line_for_test());
    assert!(
        scroll_top_after_save.abs_diff(scroll_top_before_save) <= 2,
        "saving a rebuilt large-file baseline must preserve the visible line anchor: before={scroll_top_before_save}, after={scroll_top_after_save}"
    );
    assert!(
        fs::read(editor.read_with(visual, |editor, _cx| {
            editor.file_path.clone().expect("large source path")
        }))
        .expect("overwritten large document")
        .starts_with(b"large\n document line 0\n")
    );
    visual
        .executor()
        .advance_clock(Duration::from_millis(1_100));
    visual.run_until_parked();
    assert!(
        large_view
            .read_with(visual, |view, _cx| view.pending_external_change_for_test())
            .is_none(),
        "the external monitor must discard a pre-save snapshot"
    );

    visual.update(|window, cx| {
        large_view.update(cx, |view, cx| view.begin_line_edit_for_test(0, window, cx));
    });
    let (_, edit_block) = large_view
        .read_with(visual, |view, _cx| view.active_edit_for_test())
        .expect("active large line edit after save");
    edit_block.update(visual, |block, cx| {
        block.replace_text_in_visible_range(0..0, "saved-as ", None, false, cx);
    });
    visual.run_until_parked();

    let saved_as = temp.path().join("large-shell-saved-as.md");
    let saved_as_for_action = saved_as.clone();
    visual.update(|window, cx| {
        let window_handle = window.window_handle();
        large_view.update(cx, move |view, cx| {
            view.save_as_path(saved_as_for_action, window_handle, cx);
        });
    });
    wait_for_large_document_save(&large_view, visual);
    redraw(visual);
    assert!(!editor.read_with(visual, |editor, _cx| editor.document_dirty));
    assert_eq!(
        editor.read_with(visual, |editor, _cx| editor.file_path.clone()),
        Some(saved_as.clone())
    );
    assert!(
        fs::read(&saved_as)
            .expect("saved large document")
            .starts_with(b"saved-as large\n document line 0\n")
    );
}

fn wait_for_large_document_save(
    view: &gpui::Entity<crate::document_host::DocumentHost>,
    cx: &mut gpui::VisualTestContext,
) {
    for _ in 0..5_000 {
        cx.run_until_parked();
        let (saving, dirty) = view.read_with(cx, |view, _cx| {
            (view.is_saving_for_test(), view.is_dirty())
        });
        if !saving && !dirty {
            return;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    let (saving, dirty, error) = view.read_with(cx, |view, _cx| {
        (view.is_saving_for_test(), view.is_dirty(), view.error_for_test())
    });
    panic!("large document save did not finish within the bounded test wait (saving={saving}, dirty={dirty}, error={error:?})");
}
