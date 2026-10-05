// @author kongweiguang

use super::DocumentKind;

#[gpui::test]
async fn split_projection_coalesces_revisions_and_rejects_stale_work(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let (editor, cx) =
        cx.add_window_view(|_window, cx| Editor::from_markdown(cx, "alpha".to_string(), None));
    editor.update(cx, |editor, cx| editor.set_view_mode(ViewMode::Split, cx));
    redraw(cx);

    let initial_revision = editor.read_with(cx, |editor, _cx| {
        editor.split_preview.as_ref().unwrap().revision
    });
    cx.simulate_input(" one");
    let stale_revision = editor.read_with(cx, |editor, _cx| editor.source_document.revision());
    cx.simulate_input(" two");

    editor.update(cx, |editor, cx| {
        let latest = editor.source_document.revision();
        assert!(latest > stale_revision);
        assert_eq!(editor.split_projection_scheduled_revision, Some(latest));
        assert_eq!(
            editor.split_preview.as_ref().unwrap().revision,
            initial_revision
        );
        assert!(!editor.apply_split_preview_projection_revision(stale_revision, cx));
        assert_eq!(
            editor.split_preview.as_ref().unwrap().revision,
            initial_revision
        );
    });

    flush_split_projection(cx);
    editor.read_with(cx, |editor, cx| {
        let latest = editor.source_document.revision();
        let preview = editor.split_preview.as_ref().unwrap();
        assert_eq!(preview.revision, latest);
        assert_eq!(
            preview.document.markdown_text(cx),
            editor.document.raw_source_text(cx)
        );
        assert!(editor.split_projection_scheduled_revision.is_none());
        assert!(editor.split_projection_task.is_none());
    });
}

#[gpui::test]
async fn leaving_split_cancels_pending_projection(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let (editor, cx) =
        cx.add_window_view(|_window, cx| Editor::from_markdown(cx, "alpha".to_string(), None));
    editor.update(cx, |editor, cx| editor.set_view_mode(ViewMode::Split, cx));
    redraw(cx);
    cx.simulate_input(" pending");

    editor.update(cx, |editor, cx| {
        assert!(editor.split_projection_task.is_some());
        editor.set_view_mode(ViewMode::Source, cx);
        assert!(editor.split_projection_task.is_none());
        assert!(editor.split_projection_scheduled_revision.is_none());
        assert!(editor.split_preview.is_none());
    });

    cx.executor().advance_clock(Duration::from_millis(30));
    cx.run_until_parked();
    editor.read_with(cx, |editor, _cx| {
        assert!(matches!(editor.view_mode, ViewMode::Source));
        assert!(editor.split_preview.is_none());
    });
}

#[gpui::test]
async fn window_save_action_saves_current_editor_without_global_menu_route(
    cx: &mut TestAppContext,
) {
    init_editor_test_app(cx);

    let path = temp_markdown_path("window-action-save");
    fs::write(&path, "alpha").expect("write initial markdown");
    let cleanup_path = path.clone();
    cx.on_quit(move || {
        let _ = fs::remove_file(&cleanup_path);
    });

    let (editor, cx) = cx.add_window_view({
        let path = path.clone();
        move |_window, cx| Editor::from_markdown(cx, "alpha".to_string(), Some(path))
    });

    cx.simulate_input(" action");
    redraw(cx);
    let expected = editor.read_with(cx, |editor, cx| {
        assert!(editor.document_dirty);
        editor.document.markdown_text(cx)
    });
    assert_ne!(expected, "alpha");

    cx.dispatch_action(SaveDocument);
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
/// 导出不得清除真实文字修改的 dirty；无变化的 mark_dirty 不代表用户已编辑。
async fn export_html_writes_rendered_document_without_changing_editor_state(
    cx: &mut TestAppContext,
) {
    init_editor_test_app(cx);

    let export_path = temp_export_path("rendered-export-html", "html");
    let cleanup_path = export_path.clone();
    cx.on_quit(move || {
        let _ = fs::remove_file(&cleanup_path);
    });

    let (editor, cx) = cx.add_window_view(|_window, cx| {
        Editor::from_markdown(cx, "# Title\n\nbody".to_string(), None)
    });

    editor.update(cx, |editor, cx| {
        let block = editor.document.root_blocks()[1].clone();
        block.update(cx, |block, cx| {
            block.record.set_title(InlineTextTree::plain("body edited"));
            block.sync_render_cache();
            cx.notify();
        });
        editor.mark_dirty(cx);
        assert!(editor.document_dirty);
        assert!(editor.file_path.is_none());
        editor
            .export_document_to_path(ExportFormat::Html, &export_path, cx)
            .expect("html export should write");
        assert!(editor.document_dirty);
        assert!(editor.file_path.is_none());
    });

    let html = fs::read_to_string(&export_path).expect("read exported html");
    assert!(html.contains("<h1 id=\"title\">Title</h1>"));
    assert!(html.contains("<p>body edited</p>"));
}

#[gpui::test]
async fn export_progress_is_non_modal_and_cancel_sets_worker_token(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let (editor, visual) =
        cx.add_window_view(|_window, cx| Editor::from_markdown(cx, "# Doc".to_owned(), None));
    let cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));
    editor.update(visual, |editor, cx| {
        editor.export_cancel = Some(Arc::clone(&cancelled));
        editor.export_in_progress = true;
        cx.notify();
    });
    let (revision, dirty) = editor.read_with(visual, |editor, _cx| {
        (editor.source_document.revision(), editor.document_dirty)
    });
    let mut cancel_size = None;
    for viewport in [size(px(720.0), px(520.0)), size(px(1180.0), px(780.0))] {
        visual.simulate_resize(viewport);
        redraw(visual);
        visual.update(|window, _cx| assert_eq!(window.scale_factor(), 2.0));
        let main = visual.debug_bounds("editor-main-content").unwrap();
        let progress = visual.debug_bounds("export-progress").unwrap();
        let icon = visual.debug_bounds("export-progress-icon").unwrap();
        let label = visual.debug_bounds("export-progress-label").unwrap();
        let cancel = visual.debug_bounds("cancel-export").unwrap();
        let cancel_icon = visual.debug_bounds("cancel-export-icon").unwrap();
        assert_eq!(progress.size.height, px(36.0));
        assert_eq!(icon.size, size(px(18.0), px(18.0)));
        assert_eq!(cancel_icon.size, size(px(13.0), px(13.0)));
        assert!(progress.left() >= main.left());
        assert!(progress.right() <= main.right());
        assert!(icon.left() >= progress.left());
        assert!(icon.right() <= label.left());
        assert!(label.right() <= cancel.left());
        assert!(cancel.right() <= progress.right());
        cancel_size = Some(cancel.size);
    }

    let cancel = visual.debug_bounds("cancel-export").unwrap();
    visual.simulate_click(cancel.center(), Modifiers::default());
    visual.run_until_parked();
    redraw(visual);
    editor.read_with(visual, |editor, _cx| {
        assert!(editor.export_cancel_requested);
        assert_eq!(editor.source_document.revision(), revision);
        assert_eq!(editor.document_dirty, dirty);
    });
    assert!(cancelled.load(std::sync::atomic::Ordering::Acquire));
    assert_eq!(
        visual.debug_bounds("cancel-export").unwrap().size,
        cancel_size.unwrap()
    );

    cancelled.store(false, std::sync::atomic::Ordering::Release);
    let disabled_cancel = visual.debug_bounds("cancel-export").unwrap();
    visual.simulate_click(disabled_cancel.center(), Modifiers::default());
    visual.run_until_parked();
    assert!(!cancelled.load(std::sync::atomic::Ordering::Acquire));
}

#[gpui::test]
async fn export_html_uses_source_mode_raw_text(cx: &mut TestAppContext) {
    init_editor_test_app(cx);

    let export_path = temp_export_path("source-export-html", "html");
    let cleanup_path = export_path.clone();
    cx.on_quit(move || {
        let _ = fs::remove_file(&cleanup_path);
    });

    let (editor, cx) =
        cx.add_window_view(|_window, cx| Editor::from_markdown(cx, "rendered".to_string(), None));

    editor.update(cx, |editor, cx| {
        editor.toggle_view_mode(cx);
        let source_block = editor
            .document
            .first_root()
            .expect("source mode should keep one root block")
            .clone();
        source_block.update(cx, |block, _cx| {
            block.record.set_title(InlineTextTree::plain(
                "# Source\n\n<!--\n<strong>visible</strong>\n-->".to_string(),
            ));
            block.sync_render_cache();
        });
        editor
            .export_document_to_path(ExportFormat::Html, &export_path, cx)
            .expect("source html export should write");
    });

    let html = fs::read_to_string(&export_path).expect("read exported html");
    assert!(html.contains("<h1 id=\"source\">Source</h1>"));
    assert!(html.contains("class=\"vlt-comment\""));
    assert!(html.contains("&lt;strong&gt;visible&lt;/strong&gt;"));
}

#[gpui::test]
async fn dropped_markdown_opens_new_tab_without_replacing_current(cx: &mut TestAppContext) {
    init_editor_test_app(cx);

    let dropped_path = temp_markdown_path("drop-new-tab");
    fs::write(
        &dropped_path,
        "# Dropped\n\n| A | B |\n| --- | --- |\n| 1 | 2 |\n",
    )
    .expect("write dropped markdown");
    let cleanup_path = dropped_path.clone();
    cx.on_quit(move || {
        let _ = fs::remove_file(&cleanup_path);
    });

    let (editor, cx) =
        cx.add_window_view(|_window, cx| Editor::from_markdown(cx, "old".to_string(), None));

    editor.update(cx, |editor, cx| {
        editor.set_document_dirty_for_test(true);
        editor.open_dropped_markdown_in_tab(dropped_path.clone(), cx);
    });
    cx.run_until_parked();

    editor.read_with(cx, |editor, cx| {
        assert_eq!(editor.file_path.as_ref(), Some(&dropped_path));
        assert!(editor.view_mode == ViewMode::Rendered);
        assert!(!editor.document_dirty);
        assert!(!editor.show_drop_replace_dialog);
        assert_eq!(editor.document.root_count(), 2);
        assert_eq!(
            editor
                .document
                .root_blocks()
                .last()
                .expect("table block")
                .read(cx)
                .kind(),
            BlockKind::Table
        );
        assert!(editor.document.markdown_text(cx).contains("# Dropped"));
    });

    editor.update(cx, |editor, cx| {
        assert!(editor.switch_to_tab_index(0, cx));
    });
    editor.read_with(cx, |editor, cx| {
        assert!(editor.document_dirty);
        assert_eq!(editor.file_path, None);
        assert_eq!(editor.document.markdown_text(cx), "old");
    });
    assert_eq!(cx.cx.windows().len(), 1);
}

#[gpui::test]
async fn dropped_paths_pick_first_supported_document_or_image(cx: &mut TestAppContext) {
    init_editor_test_app(cx);

    let text_path = temp_export_path("drop-ignore-non-markdown", "txt");
    let image_path = temp_export_path("drop-pick-image", "png");
    let markdown_path = temp_export_path("drop-pick-markdown", "markdown");
    fs::write(&text_path, "plain").expect("write text");
    fs::write(&image_path, "image").expect("write image");
    fs::write(&markdown_path, "markdown").expect("write markdown");
    let cleanup_text = text_path.clone();
    let cleanup_image = image_path.clone();
    let cleanup_markdown = markdown_path.clone();
    cx.on_quit(move || {
        let _ = fs::remove_file(&cleanup_text);
        let _ = fs::remove_file(&cleanup_image);
        let _ = fs::remove_file(&cleanup_markdown);
    });

    assert_eq!(
        Editor::first_dropped_openable_path(&[text_path, image_path.clone(), markdown_path,]),
        Some(image_path)
    );
}

#[gpui::test]
/// 替换提示保护真实未提交到磁盘的文字，取消必须保留该正文和所属 dirty。
async fn dirty_drop_waits_for_replace_decision_and_cancel_preserves_document(
    cx: &mut TestAppContext,
) {
    init_editor_test_app(cx);

    let dropped_path = temp_markdown_path("drop-dirty-cancel");
    fs::write(&dropped_path, "dropped").expect("write dropped markdown");
    let cleanup_path = dropped_path.clone();
    cx.on_quit(move || {
        let _ = fs::remove_file(&cleanup_path);
    });

    let (editor, cx) =
        cx.add_window_view(|_window, cx| Editor::from_markdown(cx, "current".to_string(), None));
    editor.update(cx, |editor, cx| {
        let block = editor.document.first_root().unwrap().clone();
        block.update(cx, |block, cx| {
            block.record.set_title(InlineTextTree::plain("current edited"));
            block.sync_render_cache();
            cx.notify();
        });
        editor.mark_dirty(cx);
    });

    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.request_dropped_markdown_replace(dropped_path, window, cx);
        });
    });
    redraw(cx);

    editor.read_with(cx, |editor, cx| {
        assert!(editor.document_dirty);
        assert!(editor.show_drop_replace_dialog);
        assert_eq!(editor.document.markdown_text(cx), "current edited");
        assert!(editor.pending_drop_replace_path.is_some());
    });

    editor.update(cx, |editor, cx| editor.cancel_drop_replace_dialog(cx));

    editor.read_with(cx, |editor, cx| {
        assert!(editor.document_dirty);
        assert!(!editor.show_drop_replace_dialog);
        assert!(editor.pending_drop_replace_path.is_none());
        assert_eq!(editor.document.markdown_text(cx), "current edited");
    });
}

#[gpui::test]
async fn dirty_drop_can_replace_without_saving(cx: &mut TestAppContext) {
    init_editor_test_app(cx);

    let dropped_path = temp_markdown_path("drop-dirty-discard");
    fs::write(&dropped_path, "dropped").expect("write dropped markdown");
    let cleanup_path = dropped_path.clone();
    cx.on_quit(move || {
        let _ = fs::remove_file(&cleanup_path);
    });

    let (editor, cx) =
        cx.add_window_view(|_window, cx| Editor::from_markdown(cx, "current".to_string(), None));
    editor.update(cx, |editor, cx| editor.mark_dirty(cx));

    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.request_dropped_markdown_replace(dropped_path.clone(), window, cx);
            editor.discard_pending_drop_replace(window, cx);
        });
    });
    redraw(cx);

    editor.read_with(cx, |editor, cx| {
        assert_eq!(editor.file_path.as_ref(), Some(&dropped_path));
        assert_eq!(editor.document.markdown_text(cx), "dropped");
        assert!(!editor.document_dirty);
        assert!(!editor.show_drop_replace_dialog);
    });
}

#[gpui::test]
async fn dirty_drop_saves_existing_document_before_replace(cx: &mut TestAppContext) {
    init_editor_test_app(cx);

    let current_path = temp_markdown_path("drop-save-current");
    let dropped_path = temp_markdown_path("drop-save-replace");
    fs::write(&current_path, "original").expect("write current markdown");
    fs::write(&dropped_path, "dropped").expect("write dropped markdown");
    let cleanup_current = current_path.clone();
    let cleanup_dropped = dropped_path.clone();
    cx.on_quit(move || {
        let _ = fs::remove_file(&cleanup_current);
        let _ = fs::remove_file(&cleanup_dropped);
    });

    let (editor, cx) = cx.add_window_view({
        let current_path = current_path.clone();
        move |_window, cx| Editor::from_markdown(cx, "original".to_string(), Some(current_path))
    });

    editor.update(cx, |editor, cx| {
        let first = editor.document.first_root().expect("current root").clone();
        first.update(cx, |block, _cx| {
            block
                .record
                .set_title(InlineTextTree::plain("edited".to_string()));
            block.sync_render_cache();
        });
        editor.mark_dirty(cx);
    });

    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.request_dropped_markdown_replace(dropped_path.clone(), window, cx);
            editor.save_and_replace_pending_drop(window, cx);
        });
    });
    redraw(cx);

    assert_eq!(
        fs::read_to_string(&current_path).expect("read saved current"),
        "edited"
    );
    editor.read_with(cx, |editor, cx| {
        assert_eq!(editor.file_path.as_ref(), Some(&dropped_path));
        assert_eq!(editor.document.markdown_text(cx), "dropped");
        assert!(!editor.document_dirty);
        assert!(!editor.pending_drop_replace_after_save);
    });
}

#[gpui::test]
async fn close_window_menu_action_closes_only_active_editor_window(cx: &mut TestAppContext) {
    init_editor_test_app(cx);

    let (_first_editor, cx) =
        cx.add_window_view(|_window, cx| Editor::from_markdown(cx, "first".to_string(), None));
    let first_window = activate_visual_window(cx);

    let (_second_editor, cx) = cx
        .cx
        .add_window_view(|_window, cx| Editor::from_markdown(cx, "second".to_string(), None));
    let second_window = activate_visual_window(cx);

    assert_ne!(first_window.window_id(), second_window.window_id());
    assert_eq!(cx.cx.windows().len(), 2);

    cx.cx.update(|cx| {
        crate::app_menu::dispatch_menu_action(&CloseWindow, cx);
    });
    cx.run_until_parked();

    let remaining = cx.cx.windows();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].window_id(), first_window.window_id());
    assert_ne!(remaining[0].window_id(), second_window.window_id());
}

#[gpui::test]
async fn app_menu_opened_windows_activate_and_close_independently(cx: &mut TestAppContext) {
    init_editor_test_app(cx);

    let first_window = cx
        .update(|cx| crate::app_menu::open_editor_window(cx, "first".to_string(), None))
        .unwrap();
    cx.run_until_parked();
    let second_window = cx
        .update(|cx| crate::app_menu::open_editor_window(cx, "second".to_string(), None))
        .unwrap();
    cx.run_until_parked();

    let active_window = cx.update(|cx| cx.active_window().expect("window should be active"));
    assert_eq!(active_window.window_id(), second_window.window_id());
    assert_ne!(first_window.window_id(), second_window.window_id());
    assert_eq!(cx.update(|cx| cx.windows().len()), 2);

    assert!(
        second_window
            .update(cx, |editor, _window, _cx| editor.close_guard_installed)
            .expect("second editor window should be open")
    );

    cx.update(|cx| {
        crate::app_menu::dispatch_menu_action(&CloseWindow, cx);
    });
    cx.run_until_parked();

    let remaining = cx.update(|cx| cx.windows());
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].window_id(), first_window.window_id());

    cx.update(|cx| {
        crate::app_menu::dispatch_menu_action(&CloseWindow, cx);
    });
    cx.run_until_parked();

    assert!(cx.update(|cx| cx.windows().is_empty()));
}

#[gpui::test]
async fn app_menu_opened_file_window_reinstalls_close_guard_after_registration(
    cx: &mut TestAppContext,
) {
    init_editor_test_app(cx);

    let opened_path = temp_markdown_path("app-menu-opened-file-window-close");
    fs::write(&opened_path, "opened from file").expect("write opened markdown");

    let first_window = cx
        .update(|cx| crate::app_menu::open_editor_window(cx, "first".to_string(), None))
        .unwrap();
    cx.run_until_parked();
    let second_window = cx
        .update(|cx| {
            crate::app_menu::open_editor_window(
                cx,
                fs::read_to_string(&opened_path).expect("read opened markdown"),
                Some(opened_path.clone()),
            )
        })
        .unwrap();
    cx.run_until_parked();

    let active_window = cx.update(|cx| cx.active_window().expect("window should be active"));
    assert_eq!(active_window.window_id(), second_window.window_id());
    assert_ne!(first_window.window_id(), second_window.window_id());

    second_window
        .update(cx, |editor, window, cx| {
            assert!(editor.close_guard_installed);
            assert!(editor.on_window_should_close(window, cx));
        })
        .expect("second editor window should be open");

    cx.update(|cx| {
        crate::app_menu::dispatch_menu_action(&CloseWindow, cx);
    });
    cx.run_until_parked();

    let remaining = cx.update(|cx| cx.windows());
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].window_id(), first_window.window_id());
    assert_ne!(remaining[0].window_id(), second_window.window_id());

    let _ = fs::remove_file(opened_path);
}

#[gpui::test]
async fn app_menu_opened_dirty_file_window_prompts_only_that_window(cx: &mut TestAppContext) {
    init_editor_test_app(cx);

    let opened_path = temp_markdown_path("app-menu-opened-dirty-file-window-close");
    fs::write(&opened_path, "opened from file").expect("write opened markdown");

    let first_window = cx
        .update(|cx| crate::app_menu::open_editor_window(cx, "first".to_string(), None))
        .unwrap();
    let second_window = cx
        .update(|cx| {
            crate::app_menu::open_editor_window(
                cx,
                fs::read_to_string(&opened_path).expect("read opened markdown"),
                Some(opened_path.clone()),
            )
        })
        .unwrap();
    cx.run_until_parked();

    second_window
        .update(cx, |editor, window, cx| {
            editor.set_document_dirty_for_test(true);
            assert!(!editor.on_window_should_close(window, cx));
        })
        .expect("second editor window should be open");

    first_window
        .update(cx, |editor, _window, _cx| {
            assert!(!editor.show_unsaved_changes_dialog);
        })
        .expect("first editor window should be open");
    second_window
        .update(cx, |editor, _window, _cx| {
            assert!(editor.show_unsaved_changes_dialog);
        })
        .expect("second editor window should be open");

    let _ = fs::remove_file(opened_path);
}

#[gpui::test]
async fn discard_and_close_clears_resident_document_dirty_state(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    cx.update(|cx| crate::updater::UpdateCoordinator::init(false, None, cx));
    let (editor, visual) =
        cx.add_window_view(|_window, cx| Editor::from_markdown(cx, "draft".to_owned(), None));

    editor.update(visual, |editor, _cx| {
        editor.set_document_dirty_for_test(true)
    });
    visual.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            assert!(!editor.on_window_should_close(window, cx));
            assert!(editor.show_unsaved_changes_dialog);
            assert!(editor.source_document.is_dirty());

            editor.on_discard_and_close(&gpui::ClickEvent::default(), window, cx);

            assert!(!editor.source_document.is_dirty());
            assert!(!editor.show_unsaved_changes_dialog);
        });
    });
}

#[gpui::test]
async fn app_menu_opened_dirty_window_close_guard_prompts_only_that_window(
    cx: &mut TestAppContext,
) {
    init_editor_test_app(cx);

    let first_window = cx
        .update(|cx| crate::app_menu::open_editor_window(cx, "first".to_string(), None))
        .unwrap();
    let second_window = cx
        .update(|cx| crate::app_menu::open_editor_window(cx, "second".to_string(), None))
        .unwrap();
    cx.run_until_parked();

    second_window
        .update(cx, |editor, window, cx| {
            editor.set_document_dirty_for_test(true);
            assert!(!editor.on_window_should_close(window, cx));
        })
        .expect("second editor window should be open");

    first_window
        .update(cx, |editor, _window, _cx| {
            assert!(!editor.show_unsaved_changes_dialog);
        })
        .expect("first editor window should be open");
    second_window
        .update(cx, |editor, _window, _cx| {
            assert!(editor.show_unsaved_changes_dialog);
        })
        .expect("second editor window should be open");
}

#[gpui::test]
async fn windows_fallback_close_window_dispatch_closes_target_editor_window(
    cx: &mut TestAppContext,
) {
    init_editor_test_app(cx);

    let (editor, cx) =
        cx.add_window_view(|_window, cx| Editor::from_markdown(cx, "target".to_string(), None));
    let target_window = activate_visual_window(cx);

    cx.update(|window, cx| {
        let editor = editor.downgrade();
        crate::app_menu::dispatch_menu_action_for_editor(&CloseWindow, &editor, window, cx);
    });
    cx.run_until_parked();

    assert!(
        cx.cx
            .windows()
            .iter()
            .all(|window| window.window_id() != target_window.window_id())
    );
}

#[gpui::test]
async fn windows_fallback_edit_menu_dispatches_to_target_editor(cx: &mut TestAppContext) {
    init_editor_test_app(cx);

    let (editor, cx) =
        cx.add_window_view(|_window, cx| Editor::from_markdown(cx, "target".to_string(), None));

    cx.update(|window, cx| {
        let target = editor.downgrade();
        crate::app_menu::dispatch_menu_action_for_editor(&FindInDocument, &target, window, cx);
    });

    editor.read_with(cx, |editor, _cx| {
        assert!(editor.find_panel.is_some());
    });
}

#[gpui::test]
async fn window_close_action_closes_current_editor_before_global_menu_route(
    cx: &mut TestAppContext,
) {
    init_editor_test_app(cx);

    let (_first_editor, cx) =
        cx.add_window_view(|_window, cx| Editor::from_markdown(cx, "first".to_string(), None));
    let first_window = activate_visual_window(cx);

    let (_second_editor, cx) = cx
        .cx
        .add_window_view(|_window, cx| Editor::from_markdown(cx, "second".to_string(), None));
    let second_window = activate_visual_window(cx);

    cx.dispatch_action(CloseWindow);
    cx.run_until_parked();

    let remaining = cx.cx.windows();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].window_id(), first_window.window_id());
    assert_ne!(remaining[0].window_id(), second_window.window_id());
}

#[path = "editor_tail_06_parts/pane_window.rs"]
mod pane_window;
