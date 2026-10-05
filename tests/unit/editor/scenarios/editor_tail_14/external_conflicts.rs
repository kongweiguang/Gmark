// @author kongweiguang

/// 验证显式重载保留共享 Controller，让 peer view 看到新内容且后续保存使用新身份。
#[gpui::test]
async fn external_conflict_reload_replaces_local_document_with_disk_version(
    cx: &mut TestAppContext,
) {
    init_editor_test_app(cx);
    let path = temp_markdown_path("external-reload");
    fs::write(&path, b"base\r\n").unwrap();
    let editor_path = path.clone();
    let (editor, visual_cx) = cx.add_window_view(move |_window, cx| {
        Editor::from_markdown(cx, "base".to_owned(), Some(editor_path))
    });
    redraw(visual_cx);
    let peer = visual_cx.update(|_window, cx| {
        editor.update(cx, |editor, _cx| {
            editor
                .source_document
                .fork_view()
                .expect("open a second view of the same document")
        })
    });
    fs::write(&path, b"disk version\r\n").unwrap();
    visual_cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.sync_source_document_from_projection("local version");
            editor.set_document_dirty_for_test(true);
            assert!(!editor.save_to_existing_path(&path, window, cx));
            editor.on_reload_external_conflict(&ClickEvent::default(), window, cx);
        });
    });
    editor.read_with(visual_cx, |editor, _cx| {
        assert_eq!(editor.source_document.text(), "disk version\n");
        assert!(!editor.document_dirty);
        assert!(!editor.external_file_conflict);
        assert!(!editor.show_external_conflict_dialog);
    });
    assert_eq!(
        peer.text(),
        "disk version\n",
        "reload must update every view attached to the document Controller"
    );

    let saved = visual_cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.sync_source_document_from_projection("disk version edited");
            editor.set_document_dirty_for_test(true);
            editor.save_to_existing_path(&path, window, cx)
        })
    });
    assert!(
        saved,
        "saving after reload must use the reloaded file identity"
    );
    assert_eq!(
        fs::read_to_string(&path).unwrap().replace("\r\n", "\n"),
        "disk version edited"
    );
    assert_eq!(
        peer.text(),
        "disk version edited",
        "subsequent edits must remain on the shared Controller"
    );
    let _ = fs::remove_file(path);
}

#[gpui::test]
async fn plain_text_keeps_all_status_modes_visible_and_opens_in_source(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    cx.update(|cx| crate::updater::UpdateCoordinator::init(false, None, cx));
    let temp = tempfile::tempdir().expect("plain-text mode switch tempdir");
    let path = temp.path().join("sample.txt");
    fs::write(&path, "alpha\nbeta\n").expect("plain-text fixture");
    let probe =
        gmark_paged_document::probe_file(&path, gmark_paged_document::ProbeOptions::default())
            .expect("plain-text probe");
    let source = gmark_paged_document::FileSource::open(&path).expect("plain-text source");
    let (editor, visual) = cx.add_window_view(move |_window, cx| {
        Editor::from_source_backed_file(cx, path, probe, source)
    });

    visual.run_until_parked();
    redraw(visual);

    assert_eq!(
        editor.read_with(visual, |editor, _cx| editor.view_mode),
        ViewMode::Source
    );
    assert!(visual.debug_bounds("status-bar-mode-menu").is_none());
    let mode_button = visual.debug_bounds("status-bar-mode-switch").unwrap();
    visual.simulate_click(mode_button.center(), Modifiers::default());
    redraw(visual);
    assert!(visual.debug_bounds("status-bar-mode-menu").is_some());
    for selector in [
        "status-bar-mode-Rendered",
        "status-bar-mode-Source",
        "status-bar-mode-Split",
        "status-bar-mode-Preview",
    ] {
        assert!(
            visual.debug_bounds(selector).is_some(),
            "{selector} must remain visible for plain-text files"
        );
    }
}

#[gpui::test]
async fn external_conflict_save_as_and_cancel_preserve_disk_and_close_intent(
    cx: &mut TestAppContext,
) {
    init_editor_test_app(cx);
    let path = temp_markdown_path("external-save-as-cancel");
    fs::write(&path, "base").unwrap();
    let editor_path = path.clone();
    let (editor, visual_cx) = cx.add_window_view(move |_window, cx| {
        Editor::from_markdown(cx, "base".to_owned(), Some(editor_path))
    });
    redraw(visual_cx);
    fs::write(&path, "disk version").unwrap();
    visual_cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.sync_source_document_from_projection("local version");
            editor.set_document_dirty_for_test(true);
            editor.pending_close_after_save = true;
            editor.save_document(window, cx);
        });
    });
    visual_cx.run_until_parked();
    visual_cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            assert!(editor.show_external_conflict_dialog);
            assert!(editor.pending_close_after_save);
            editor.on_cancel_external_conflict(&ClickEvent::default(), window, cx);
            assert!(!editor.pending_close_after_save);

            editor.save_document(window, cx);
        });
    });
    visual_cx.run_until_parked();
    visual_cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.on_save_as_external_conflict(&ClickEvent::default(), window, cx);
            assert!(editor.pending_save_as);
            assert!(!editor.show_external_conflict_dialog);
        });
    });
    assert_eq!(fs::read_to_string(&path).unwrap(), "disk version");
    let _ = fs::remove_file(path);
}

#[gpui::test]
async fn external_conflict_handles_deleted_and_invalid_utf8_disk_files(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let path = temp_markdown_path("external-missing-invalid-utf8");
    fs::write(&path, "base").unwrap();
    let editor_path = path.clone();
    let (editor, visual_cx) = cx.add_window_view(move |_window, cx| {
        Editor::from_markdown(cx, "base".to_owned(), Some(editor_path))
    });
    redraw(visual_cx);

    fs::remove_file(&path).unwrap();
    visual_cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.sync_source_document_from_projection("local version");
            editor.set_document_dirty_for_test(true);
            assert!(!editor.save_to_existing_path(&path, window, cx));
            let preview = editor.external_conflict_preview.as_ref().unwrap();
            assert!(preview.disk_error.is_some());
            assert_eq!(preview.disk_line_count, 0);
            editor.cancel_external_conflict(cx);
        });
    });

    fs::write(&path, [0xff, b'a', b'\n', 0xfe]).unwrap();
    visual_cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            assert!(!editor.save_to_existing_path(&path, window, cx));
            let preview = editor.external_conflict_preview.as_ref().unwrap();
            assert_eq!(preview.first_difference_line, Some(1));
            assert!(preview.disk_line.contains('\u{fffd}'));
            assert_eq!(preview.disk_bytes, 4);
        });
    });
    assert_eq!(fs::read(&path).unwrap(), [0xff, b'a', b'\n', 0xfe]);
    let _ = fs::remove_file(path);
}

#[gpui::test]
async fn external_conflict_overwrite_completes_pending_close_save(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let path = temp_markdown_path("external-overwrite-close");
    fs::write(&path, "base").unwrap();
    let editor_path = path.clone();
    let (editor, visual_cx) = cx.add_window_view(move |_window, cx| {
        Editor::from_markdown(cx, "base".to_owned(), Some(editor_path))
    });
    redraw(visual_cx);
    fs::write(&path, "disk version").unwrap();

    visual_cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.sync_source_document_from_projection("local version");
            editor.set_document_dirty_for_test(true);
            editor.pending_close_after_save = true;
            assert!(!editor.save_to_existing_path(&path, window, cx));
            assert!(editor.pending_close_after_save);
            editor.on_overwrite_external_conflict(&ClickEvent::default(), window, cx);
            assert!(editor.allow_external_overwrite_once);
            assert!(editor.save_to_existing_path(&path, window, cx));
            assert!(!editor.pending_close_after_save);
            assert!(!editor.allow_external_overwrite_once);
        });
    });

    assert_eq!(fs::read_to_string(&path).unwrap(), "local version");
    let _ = fs::remove_file(path);
}

#[gpui::test]
async fn external_conflict_dialog_stays_within_small_and_large_window_bounds(
    cx: &mut TestAppContext,
) {
    init_editor_test_app(cx);
    let path = std::env::temp_dir()
        .join("gmark-external-conflict-layout")
        .join("a-very-long-directory-name-without-spaces".repeat(4))
        .join("document-with-a-very-long-name.md");
    let editor_path = path.clone();
    let (editor, visual_cx) = cx.add_window_view(move |_window, cx| {
        Editor::from_markdown(cx, "base".to_owned(), Some(editor_path))
    });
    editor.update(visual_cx, |editor, cx| {
        editor.show_external_conflict_dialog = true;
        editor.external_conflict_preview = Some(super::ExternalConflictPreview {
            path: path.display().to_string(),
            first_difference_line: Some(1),
            local_line: "local ".repeat(80),
            disk_line: "disk ".repeat(80),
            local_line_count: 20,
            disk_line_count: 22,
            local_bytes: 1_024,
            disk_bytes: 1_120,
            disk_error: None,
        });
        cx.notify();
    });

    for viewport in [size(px(720.0), px(520.0)), size(px(1180.0), px(780.0))] {
        visual_cx.simulate_resize(viewport);
        redraw(visual_cx);

        let overlay = visual_cx.debug_bounds("external-conflict-overlay").unwrap();
        let dialog = visual_cx.debug_bounds("external-conflict-dialog").unwrap();
        assert_dialog_title_icon(
            visual_cx,
            "external-conflict-dialog",
            "external-conflict-title-icon",
            "external-conflict-title-label",
        );
        assert!(
            dialog.left() >= overlay.left(),
            "dialog={dialog:?} overlay={overlay:?}"
        );
        assert!(
            dialog.right() <= overlay.right(),
            "dialog={dialog:?} overlay={overlay:?}"
        );
        assert!(
            dialog.top() >= overlay.top(),
            "dialog={dialog:?} overlay={overlay:?}"
        );
        assert!(
            dialog.bottom() <= overlay.bottom(),
            "dialog={dialog:?} overlay={overlay:?}"
        );

        for selector in [
            "external-conflict-path",
            "external-conflict-summary",
            "external-conflict-local",
            "external-conflict-disk",
            "cancel-external-conflict",
            "reload-external-conflict",
            "overwrite-external-conflict",
            "save-as-external-conflict",
        ] {
            let bounds = visual_cx.debug_bounds(selector).unwrap();
            assert!(bounds.left() >= dialog.left(), "{selector} escaped left");
            assert!(bounds.right() <= dialog.right(), "{selector} escaped right");
            assert!(bounds.top() >= dialog.top(), "{selector} escaped top");
        }
        for selector in [
            "cancel-external-conflict",
            "reload-external-conflict",
            "overwrite-external-conflict",
            "save-as-external-conflict",
        ] {
            let action = visual_cx.debug_bounds(selector).unwrap();
            assert!(f32::from(action.size.width) >= 72.0, "{selector}");
            assert_eq!(f32::from(action.size.height), 36.0, "{selector}");
            assert!(action.bottom() <= dialog.bottom(), "{selector}");
        }
    }
}

#[cfg(target_os = "windows")]
/// 禁止目标句柄共享删除来稳定复现原子替换提交前失败，避免依赖文件权限和竞态时序。
fn lock_save_target_for_atomic_replace(path: &std::path::Path) -> std::fs::File {
    use std::os::windows::fs::OpenOptionsExt;

    std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0x0000_0001 | 0x0000_0002)
        .open(path)
        .expect("open save target without delete sharing")
}

#[cfg(target_os = "windows")]
/// 有界等待真实后台回调结束，避免用固定延时掩盖保存状态未收敛。
fn wait_for_existing_target_save(
    editor: &gpui::Entity<Editor>,
    visual: &mut gpui::VisualTestContext,
) {
    for _ in 0..5_000 {
        visual.run_until_parked();
        if !editor.read_with(visual, |editor, _cx| editor.save_task.is_some()) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    panic!("existing-target save did not finish within the bounded test wait");
}

#[cfg(target_os = "windows")]
#[gpui::test]
/// 用真实共享拒绝覆盖失败与 Ctrl+S 重试闭环，确保编辑、焦点和历史状态不受反馈机制影响。
async fn existing_target_save_failure_keeps_edit_non_modal_and_retry_clears_notice(
    cx: &mut TestAppContext,
) {
    init_editor_test_app(cx);
    let path = temp_markdown_path("save-failure-feedback");
    fs::write(&path, "base").expect("write save feedback fixture");
    let editor_path = path.clone();
    let (editor, visual) = cx.add_window_view(move |_window, cx| {
        Editor::from_markdown(cx, "base".to_owned(), Some(editor_path))
    });
    redraw(visual);
    editor.update_in(visual, |editor, window, cx| {
        let block = editor
            .document
            .first_root()
            .expect("editable paragraph")
            .clone();
        editor.focus_block(block.entity_id());
        block.update(cx, |block, cx| {
            block.selected_range = 0..0;
            block.focus_handle.focus(window);
            cx.notify();
        });
    });
    visual.simulate_input("!");
    redraw(visual);

    let (edited_source, undo_count) = editor.read_with(visual, |editor, _cx| {
        assert!(editor.document_dirty, "typed input must remain unsaved");
        (editor.source_document.text(), editor.undo_history.len())
    });
    assert_ne!(edited_source, "base");
    let lock = lock_save_target_for_atomic_replace(&path);

    visual.simulate_keystrokes("ctrl-s");
    wait_for_existing_target_save(&editor, visual);
    redraw(visual);
    assert_eq!(
        fs::read(&path).expect("read target after failed save"),
        b"base"
    );
    assert!(
        visual.debug_bounds("status-bar-pane-notice").is_some(),
        "the short failure notice must be visible in the existing status bar"
    );
    assert!(
        !visual.has_pending_prompt(),
        "save failure feedback must not enqueue a native modal prompt"
    );
    editor.update_in(visual, |editor, window, cx| {
        assert_eq!(editor.source_document.text(), edited_source);
        assert!(editor.document_dirty);
        assert_eq!(editor.undo_history.len(), undo_count);
        assert!(editor.info_dialog.is_none());
        assert!(!editor.show_external_conflict_dialog);
        assert_eq!(
            editor.pane_notice.as_ref().map(|notice| notice.as_ref()),
            Some("保存失败，修改已保留；按 Ctrl+S 重试")
        );
        let block = editor.document.first_root().expect("editable paragraph");
        assert!(
            block.read(cx).focus_handle.is_focused(window),
            "save failure feedback must not steal editor focus"
        );
    });

    drop(lock);
    visual.simulate_keystrokes("ctrl-s");
    wait_for_existing_target_save(&editor, visual);
    redraw(visual);
    assert_eq!(
        fs::read_to_string(&path).expect("read retried save"),
        edited_source
    );
    editor.read_with(visual, |editor, _cx| {
        assert!(!editor.document_dirty);
        assert_eq!(editor.undo_history.len(), undo_count);
        assert!(editor.pane_notice.is_none());
        assert!(editor.pane_notice_task.is_none());
    });
}
