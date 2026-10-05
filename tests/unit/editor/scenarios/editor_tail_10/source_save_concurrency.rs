// @author kongweiguang

use super::*;

/// Bounds save completion polling so the test verifies the real async callback without guessing worker timing.
fn wait_for_source_save(
    host: &gpui::Entity<crate::document_host::DocumentHost>,
    visual: &mut gpui::VisualTestContext,
) {
    for _ in 0..5_000 {
        visual.run_until_parked();
        if !host.read_with(visual, |host, _cx| host.is_saving_for_test()) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    let error = host.read_with(visual, |host, _cx| host.error_for_test());
    panic!("Source save did not finish within the bounded test wait: {error:?}");
}

/// 保存应保留鼠标定位的输入目标和插入点，后续文字不能被移到行尾或要求用户重新点击。
#[gpui::test]
async fn large_source_save_keeps_mouse_focused_input_for_followup_typing(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let temp = tempfile::tempdir().expect("focused Source save tempdir");
    let path = temp.path().join("focused-source-save.txt");
    let original = (0..40)
        .map(|line| format!("source row {line:03}\n"))
        .collect::<String>();
    fs::write(&path, &original).expect("focused Source save fixture");
    let probe = gmark_paged_document::probe_file(
        &path,
        gmark_paged_document::ProbeOptions {
            max_resident_bytes: 1,
            ..gmark_paged_document::ProbeOptions::default()
        },
    )
    .expect("focused Source save probe");
    assert_eq!(probe.strategy, gmark_paged_document::OpenStrategy::Paged);
    let source = gmark_paged_document::FileSource::open(&path).expect("focused Source save source");
    let task_path = path.clone();
    let (editor, visual) = cx.add_window_view(move |_window, cx| {
        Editor::from_source_backed_file(cx, task_path, probe, source)
    });
    visual.simulate_resize(size(px(760.0), px(520.0)));
    visual.run_until_parked();
    redraw(visual);
    let host = editor
        .read_with(visual, |editor, _cx| editor.document_host.clone())
        .expect("focused Source save Host");

    let row_bounds = visual
        .debug_bounds("document-host-line-body-0")
        .expect("first Source row body");
    let target = point(row_bounds.left() + px(45.0), row_bounds.center().y);
    visual.simulate_mouse_down(target, MouseButton::Left, Modifiers::default());
    visual.simulate_mouse_move(target, MouseButton::Left, Modifiers::default());
    visual.simulate_mouse_up(target, MouseButton::Left, Modifiers::default());
    visual.run_until_parked();
    visual.simulate_input(" first-edit");
    visual.run_until_parked();
    let saved_snapshot = host.read_with(visual, |view, _cx| view.source_text_for_test());
    assert_ne!(saved_snapshot, original);
    let saved_selection = host
        .read_with(visual, |view, _cx| view.source_selection_for_test())
        .expect("Source caret before save");
    assert!(saved_selection.range().is_empty());
    let mut expected = saved_snapshot.clone();
    expected.insert_str(saved_selection.head.byte_offset as usize, " after-save");

    visual.update(|window, cx| {
        host.update(cx, |view, cx| {
            view.on_save_document(&SaveDocument, window, cx);
            assert!(view.is_saving_for_test());
        });
    });
    wait_for_source_save(&host, visual);
    assert_eq!(
        fs::read_to_string(&path).expect("first focused Source save"),
        saved_snapshot,
        "the first save must persist the snapshot captured before the follow-up keystroke"
    );
    assert_eq!(
        host.read_with(visual, |view, _cx| view.source_selection_for_test()),
        Some(saved_selection),
        "save must preserve the user's caret as well as its input owner"
    );

    visual.simulate_input(" after-save");
    visual.run_until_parked();
    let latest = host.read_with(visual, |view, _cx| view.source_text_for_test());
    assert_eq!(
        latest, expected,
        "typing after save must still target the row focused before save, without another click"
    );
    assert!(host.read_with(visual, |view, _cx| view.is_dirty()));

    visual.update(|window, cx| {
        host.update(cx, |view, cx| {
            view.on_save_document(&SaveDocument, window, cx)
        });
    });
    wait_for_source_save(&host, visual);
    assert_eq!(
        fs::read_to_string(&path).expect("follow-up focused Source save"),
        latest
    );
    assert!(!host.read_with(visual, |view, _cx| view.is_dirty()));
}

/// Keeps edits and viewport movement made during an immutable save, then checks that the next save persists them.
#[gpui::test]
async fn large_source_save_keeps_concurrent_edit_dirty_and_viewport(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let temp = tempfile::tempdir().expect("concurrent Source save tempdir");
    let path = temp.path().join("concurrent-source-save.txt");
    let original = (0..120)
        .map(|line| format!("source row {line:03}\n"))
        .collect::<String>();
    fs::write(&path, &original).expect("concurrent Source save fixture");
    let probe = gmark_paged_document::probe_file(
        &path,
        gmark_paged_document::ProbeOptions {
            max_resident_bytes: 1,
            ..gmark_paged_document::ProbeOptions::default()
        },
    )
    .expect("concurrent Source save probe");
    assert_eq!(probe.strategy, gmark_paged_document::OpenStrategy::Paged);
    let source =
        gmark_paged_document::FileSource::open(&path).expect("concurrent Source save file");
    let task_path = path.clone();
    let (editor, visual) = cx.add_window_view(move |_window, cx| {
        Editor::from_source_backed_file(cx, task_path, probe, source)
    });
    visual.simulate_resize(size(px(760.0), px(520.0)));
    visual.run_until_parked();
    redraw(visual);
    let host = editor
        .read_with(visual, |editor, _cx| editor.document_host.clone())
        .expect("concurrent Source save Host");

    visual.update(|window, cx| {
        host.update(cx, |view, cx| view.begin_line_edit_for_test(0, window, cx));
    });
    let (_, first_row) = host
        .read_with(visual, |view, _cx| view.active_edit_for_test())
        .expect("first row input before save");
    first_row.update(visual, |block, cx| {
        let end = block.display_text().len();
        block.replace_text_in_visible_range(end..end, " before-save", None, false, cx);
    });
    visual.run_until_parked();
    let saved_snapshot_text = host.read_with(visual, |view, _cx| view.source_text_for_test());
    assert!(host.read_with(visual, |view, _cx| view.is_dirty()));

    let row_bounds = visual
        .debug_bounds("document-host-line-body-1")
        .expect("second visible Source row");
    visual.update(|window, cx| {
        host.update(cx, |view, cx| {
            view.on_save_document(&SaveDocument, window, cx);
            assert!(view.is_saving_for_test());
            view.activate_source_pointer_for_test(
                1,
                point(row_bounds.center().x, row_bounds.center().y),
                1,
                window,
                cx,
            );
            let (_, row) = view
                .active_edit_for_test()
                .expect("second row input during save");
            row.update(cx, |block, cx| {
                let end = block.display_text().len();
                block.replace_text_in_visible_range(end..end, " during-save", None, false, cx);
            });
            view.scroll_to_line_for_test(60);
        });
        window.draw(cx).clear();
    });
    let edited_during_save = host.read_with(visual, |view, _cx| view.source_text_for_test());
    let scroll_during_save = host.read_with(visual, |view, _cx| view.scroll_top_line_for_test());
    assert_ne!(edited_during_save, saved_snapshot_text);
    assert!(
        scroll_during_save > 20,
        "the user moved the Source viewport during save"
    );

    wait_for_source_save(&host, visual);
    assert_eq!(
        host.read_with(visual, |view, _cx| view.source_text_for_test()),
        edited_during_save,
        "completing the older immutable snapshot must not roll back newer source input"
    );
    assert!(
        host.read_with(visual, |view, _cx| view.is_dirty()),
        "the revision created during save remains newer than the saved baseline"
    );
    assert_eq!(
        fs::read_to_string(&path).expect("first saved snapshot"),
        saved_snapshot_text,
        "the first save must persist exactly its captured revision"
    );
    let scroll_after_save = host.read_with(visual, |view, _cx| view.scroll_top_line_for_test());
    assert!(
        scroll_after_save.abs_diff(scroll_during_save) <= 2,
        "save completion must preserve the user's newer viewport: before={scroll_during_save}, after={scroll_after_save}"
    );

    visual.update(|window, cx| {
        host.update(cx, |view, cx| {
            view.on_save_document(&SaveDocument, window, cx)
        });
    });
    wait_for_source_save(&host, visual);
    assert_eq!(
        fs::read_to_string(&path).expect("second saved snapshot"),
        edited_during_save,
        "a subsequent save must persist the newer revision"
    );
    assert!(!host.read_with(visual, |view, _cx| view.is_dirty()));
}

/// 保存期间再次确认保存必须等待旧快照结束后捕获新正文，不能静默丢弃用户的第二次请求。
#[gpui::test]
async fn large_source_second_save_during_io_persists_the_newer_edit(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let temp = tempfile::tempdir().expect("queued Source save tempdir");
    let path = temp.path().join("queued-source-save.txt");
    let original = (0..40)
        .map(|line| format!("source row {line:03}\n"))
        .collect::<String>();
    fs::write(&path, &original).expect("queued Source save fixture");
    let probe = gmark_paged_document::probe_file(
        &path,
        gmark_paged_document::ProbeOptions {
            max_resident_bytes: 1,
            ..gmark_paged_document::ProbeOptions::default()
        },
    )
    .expect("queued Source save probe");
    let source = gmark_paged_document::FileSource::open(&path).expect("queued Source source");
    let task_path = path.clone();
    let (editor, visual) = cx.add_window_view(move |_window, cx| {
        Editor::from_source_backed_file(cx, task_path, probe, source)
    });
    visual.simulate_resize(size(px(760.0), px(520.0)));
    visual.run_until_parked();
    redraw(visual);
    let host = editor
        .read_with(visual, |editor, _cx| editor.document_host.clone())
        .expect("queued Source Host");
    visual.update(|window, cx| {
        host.update(cx, |view, cx| view.begin_line_edit_for_test(0, window, cx));
    });
    let (_, row) = host
        .read_with(visual, |view, _cx| view.active_edit_for_test())
        .expect("queued Source input");
    row.update(visual, |block, cx| {
        let end = block.display_text().len();
        block.replace_text_in_visible_range(end..end, " before-save", None, false, cx);
    });
    visual.run_until_parked();
    let first_snapshot = host.read_with(visual, |view, _cx| view.source_text_for_test());

    visual.update(|window, cx| {
        host.update(cx, |view, cx| {
            view.on_save_document(&SaveDocument, window, cx);
            assert!(view.is_saving_for_test());
        });
        row.update(cx, |block, cx| {
            let end = block.display_text().len();
            block.replace_text_in_visible_range(end..end, " during-save", None, false, cx);
        });
        host.update(cx, |view, cx| {
            view.on_save_document(&SaveDocument, window, cx)
        });
    });
    visual.run_until_parked();
    let expected = host.read_with(visual, |view, _cx| view.source_text_for_test());
    assert_ne!(expected, first_snapshot);
    let mut persisted = false;
    for _ in 0..5_000 {
        visual.run_until_parked();
        if fs::read_to_string(&path).is_ok_and(|text| text == expected)
            && !host.read_with(visual, |view, _cx| view.is_dirty())
        {
            persisted = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert!(
        persisted,
        "the second Save request must persist the newer body without a third request; actual={:?}",
        fs::read_to_string(&path).ok()
    );
    assert_eq!(
        host.read_with(visual, |view, _cx| view.source_text_for_test()),
        expected
    );
}

/// 滚动已请求的新视口必须在保存期间完成加载，不能取消读取后遗留永远等待的 pending 标记。
#[gpui::test]
async fn large_source_save_keeps_an_in_flight_viewport_loading(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let temp = tempfile::tempdir().expect("save viewport tempdir");
    let path = temp.path().join("save-viewport.txt");
    let original = (0..2_000)
        .map(|line| format!("source row {line:04}\n"))
        .collect::<String>();
    fs::write(&path, original).expect("save viewport fixture");
    let probe = gmark_paged_document::probe_file(
        &path,
        gmark_paged_document::ProbeOptions {
            max_resident_bytes: 1,
            ..gmark_paged_document::ProbeOptions::default()
        },
    )
    .expect("save viewport probe");
    let source = gmark_paged_document::FileSource::open(&path).expect("save viewport source");
    let task_path = path.clone();
    let (editor, visual) = cx.add_window_view(move |_window, cx| {
        Editor::from_source_backed_file(cx, task_path, probe, source)
    });
    visual.run_until_parked();
    redraw(visual);
    let host = editor
        .read_with(visual, |editor, _cx| editor.document_host.clone())
        .expect("save viewport Host");
    visual.update(|window, cx| {
        host.update(cx, |view, cx| view.begin_line_edit_for_test(0, window, cx));
    });
    let (_, row) = host
        .read_with(visual, |view, _cx| view.active_edit_for_test())
        .expect("save viewport input");
    row.update(visual, |block, cx| {
        let end = block.display_text().len();
        block.replace_text_in_visible_range(end..end, " changed", None, false, cx);
    });
    visual.run_until_parked();
    visual.update(|window, cx| {
        host.update(cx, |view, cx| {
            view.scroll_to_line_for_test(1_500);
            cx.notify();
        });
        window.draw(cx).clear();
        host.update(cx, |view, cx| {
            view.on_save_document(&SaveDocument, window, cx)
        });
    });
    for _ in 0..5_000 {
        visual.run_until_parked();
        redraw(visual);
        if visual
            .debug_bounds("document-host-line-body-1500")
            .is_some()
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert!(
        visual
            .debug_bounds("document-host-line-body-1500")
            .is_some(),
        "saving must not strand the requested viewport in a permanent pending state"
    );
}

/// 关闭保存发起视图必须取消或完成共享 IO；不能 panic 或让剩余视图永远等待在途快照。
#[gpui::test]
async fn large_source_closing_save_owner_releases_shared_save_queue(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let temp = tempfile::tempdir().expect("closed save owner tempdir");
    let path = temp.path().join("closed-save-owner.txt");
    fs::write(&path, "alpha\nbeta\n").expect("closed save owner fixture");
    let probe = gmark_paged_document::probe_file(
        &path,
        gmark_paged_document::ProbeOptions {
            max_resident_bytes: 1,
            ..gmark_paged_document::ProbeOptions::default()
        },
    )
    .expect("closed save owner probe");
    let source = gmark_paged_document::FileSource::open(&path).expect("closed save owner source");
    let task_path = path.clone();
    let (editor, visual) = cx.add_window_view(move |_window, cx| {
        Editor::from_source_backed_file(cx, task_path, probe, source)
    });
    visual.run_until_parked();
    redraw(visual);
    let host = editor
        .read_with(visual, |editor, _cx| editor.document_host.clone())
        .expect("closed save owner Host");
    visual.update(|window, cx| {
        host.update(cx, |view, cx| view.begin_line_edit_for_test(0, window, cx));
    });
    let (_, row) = host
        .read_with(visual, |view, _cx| view.active_edit_for_test())
        .expect("closed save owner input");
    row.update(visual, |block, cx| {
        let end = block.display_text().len();
        block.replace_text_in_visible_range(end..end, " changed", None, false, cx);
    });
    visual.run_until_parked();
    let handle = host
        .read_with(visual, |view, _cx| view.document_handle_for_test())
        .expect("shared save handle");
    let peer_lease = handle.lease();
    visual.update(|window, cx| {
        host.update(cx, |view, cx| {
            view.on_save_document(&SaveDocument, window, cx);
            assert!(
                handle
                    .save_in_flight_revision()
                    .expect("in-flight save")
                    .is_some()
            );
            view.suspend_for_closed_tab();
        });
    });
    for _ in 0..5_000 {
        visual.run_until_parked();
        if handle
            .save_in_flight_revision()
            .expect("shared save queue")
            .is_none()
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert!(
        handle
            .save_in_flight_revision()
            .expect("shared save queue")
            .is_none()
    );
    assert!(host.read_with(visual, |view, _cx| view.is_closed_suspended_for_test()));
    assert!(
        handle
            .request_save_snapshot()
            .expect("peer retry after owner close")
            .is_some()
    );
    drop(peer_lease);
}

/// Starts a new row edit in the same UI turn as Save As so completion cannot be mistaken for a document replacement.
#[gpui::test]
async fn large_source_save_as_keeps_active_row_input(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let temp = tempfile::tempdir().expect("concurrent Source Save As tempdir");
    let path = temp.path().join("source-save-as.txt");
    let original = (0..80)
        .map(|line| format!("source row {line:03}\n"))
        .collect::<String>();
    fs::write(&path, &original).expect("concurrent Source Save As fixture");
    let probe = gmark_paged_document::probe_file(
        &path,
        gmark_paged_document::ProbeOptions {
            max_resident_bytes: 1,
            ..gmark_paged_document::ProbeOptions::default()
        },
    )
    .expect("concurrent Source Save As probe");
    let source =
        gmark_paged_document::FileSource::open(&path).expect("concurrent Source Save As file");
    let task_path = path.clone();
    let (editor, visual) = cx.add_window_view(move |_window, cx| {
        Editor::from_source_backed_file(cx, task_path, probe, source)
    });
    visual.simulate_resize(size(px(760.0), px(520.0)));
    visual.run_until_parked();
    redraw(visual);
    let host = editor
        .read_with(visual, |editor, _cx| editor.document_host.clone())
        .expect("concurrent Source Save As Host");

    visual.update(|window, cx| {
        host.update(cx, |view, cx| view.begin_line_edit_for_test(0, window, cx));
    });
    let (_, first_row) = host
        .read_with(visual, |view, _cx| view.active_edit_for_test())
        .expect("first row input before Save As");
    first_row.update(visual, |block, cx| {
        let end = block.display_text().len();
        block.replace_text_in_visible_range(end..end, " before-save", None, false, cx);
    });
    visual.run_until_parked();
    let saved_snapshot_text = host.read_with(visual, |view, _cx| view.source_text_for_test());
    let saved_as = temp.path().join("source-save-as-copy.txt");
    let row_bounds = visual
        .debug_bounds("document-host-line-body-1")
        .expect("second visible Source row before Save As");

    visual.update(|window, cx| {
        host.update(cx, |view, cx| {
            view.save_as_path(saved_as.clone(), window.window_handle(), cx);
            assert!(view.is_saving_for_test());
            view.activate_source_pointer_for_test(
                1,
                point(row_bounds.center().x, row_bounds.center().y),
                1,
                window,
                cx,
            );
            let (_, row) = view
                .active_edit_for_test()
                .expect("second row input during Save As");
            row.update(cx, |block, cx| {
                let end = block.display_text().len();
                block.replace_text_in_visible_range(end..end, " during-save", None, false, cx);
            });
            #[cfg(target_os = "windows")]
            row.update(cx, |block, block_cx| {
                let composing = "拼音🙂";
                let composing_end = composing.encode_utf16().count();
                <crate::components::Block as gpui::EntityInputHandler>::replace_and_mark_text_in_range(
                    block,
                    None,
                    composing,
                    Some(composing_end..composing_end),
                    window,
                    block_cx,
                );
            });
            view.scroll_to_line_for_test(50);
        });
        window.draw(cx).clear();
    });
    let scroll_during_save = host.read_with(visual, |view, _cx| view.scroll_top_line_for_test());
    assert!(scroll_during_save > 20);
    wait_for_source_save(&host, visual);

    let expected_during_save =
        saved_snapshot_text.replacen("source row 001\n", "source row 001 during-save\n", 1);
    assert_eq!(
        host.read_with(visual, |view, _cx| view.source_text_for_test()),
        expected_during_save,
        "Save As completion must retain a newer edit made through its Source row"
    );
    assert!(host.read_with(visual, |view, _cx| view.is_dirty()));
    assert_eq!(
        fs::read_to_string(&saved_as).expect("Save As captured snapshot"),
        saved_snapshot_text,
        "Save As writes the immutable revision captured before the in-flight edit"
    );
    assert_eq!(
        editor.read_with(visual, |editor, _cx| editor.file_path.clone()),
        Some(saved_as.clone())
    );
    assert!(
        host.read_with(visual, |view, _cx| view.scroll_top_line_for_test())
            .abs_diff(scroll_during_save)
            <= 2,
        "Save As must keep the user's current viewport"
    );
    #[cfg(target_os = "windows")]
    row_for_save_as_ime_commit(host.clone(), visual);

    let (_, active_row) = host
        .read_with(visual, |view, _cx| view.active_edit_for_test())
        .expect("Source row input remains active after Save As");
    active_row.update(visual, |block, cx| {
        let end = block.display_text().len();
        block.replace_text_in_visible_range(end..end, " after-save", None, false, cx);
    });
    visual.run_until_parked();
    let latest = host.read_with(visual, |view, _cx| view.source_text_for_test());
    #[cfg(target_os = "windows")]
    let expected_second_line = "source row 001 during-save汉🙂 after-save";
    #[cfg(not(target_os = "windows"))]
    let expected_second_line = "source row 001 during-save after-save";
    assert_eq!(
        latest.lines().nth(1),
        Some(expected_second_line),
        "continued input must remain on the edited Source row"
    );
    visual.update(|window, cx| {
        host.update(cx, |view, cx| {
            view.on_save_document(&SaveDocument, window, cx)
        });
    });
    wait_for_source_save(&host, visual);
    assert_eq!(
        fs::read_to_string(&saved_as).expect("follow-up save after Save As"),
        latest
    );
    assert!(!host.read_with(visual, |view, _cx| view.is_dirty()));
}

/// Commits a candidate on the original input owner after Save As; the same entity must survive path projection refresh.
#[cfg(target_os = "windows")]
fn row_for_save_as_ime_commit(
    host: gpui::Entity<crate::document_host::DocumentHost>,
    visual: &mut gpui::VisualTestContext,
) {
    let (_, row) = host
        .read_with(visual, |view, _cx| view.active_edit_for_test())
        .expect("IME row remains active after Save As");
    row.read_with(visual, |block, _cx| assert!(block.has_ime_composition()));
    visual.update(|window, cx| {
        row.update(cx, |block, block_cx| {
            <crate::components::Block as gpui::EntityInputHandler>::replace_text_in_range(
                block, None, "汉🙂", window, block_cx,
            );
            <crate::components::Block as gpui::EntityInputHandler>::composition_ended(
                block,
                gpui::CompositionEnd::Committed,
                window,
                block_cx,
            );
        });
    });
    visual.run_until_parked();
    assert!(
        host.read_with(visual, |view, _cx| view.source_text_for_test())
            .contains("during-save汉🙂")
    );
}
