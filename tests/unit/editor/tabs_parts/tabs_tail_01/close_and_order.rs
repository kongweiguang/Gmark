// @author kongweiguang

#[gpui::test]
async fn closing_only_tab_creates_fresh_untitled_and_can_reopen(cx: &mut gpui::TestAppContext) {
    init_test_app(cx);
    let (editor, visual) = cx.add_window_view(|_window, cx| {
        super::Editor::from_markdown(cx, "only".to_owned(), Some(PathBuf::from("only.md")))
    });
    editor.update(visual, |editor, cx| {
        editor.request_close_tab_index(0, cx);
        assert_eq!(editor.tabs.records.len(), 1);
        assert_eq!(editor.source_document.text(), "");
        assert!(editor.file_path.is_none());
        assert!(!editor.document_dirty);
        assert_eq!(editor.tabs.closed.len(), 1);
    });
}

#[gpui::test]
async fn pending_save_close_only_finishes_after_document_is_clean(cx: &mut gpui::TestAppContext) {
    init_test_app(cx);
    let (editor, visual) = cx
        .add_window_view(|_window, cx| super::Editor::from_markdown(cx, "dirty".to_owned(), None));
    editor.update(visual, |editor, cx| {
        add_inactive_tab(editor, "survivor", "survivor.md");
        editor.set_document_dirty_for_test(true);
        editor.tabs.close_after_save = true;
        editor.finish_pending_tab_close_after_save(cx);
        assert_eq!(editor.tabs.records.len(), 2);

        editor.set_document_dirty_for_test(false);
        editor.finish_pending_tab_close_after_save(cx);
        assert_eq!(editor.tabs.records.len(), 1);
        assert_eq!(editor.source_document.text(), "survivor");
        assert!(!editor.tabs.close_after_save);
    });
}

#[gpui::test]
async fn window_close_activates_background_dirty_tab(cx: &mut gpui::TestAppContext) {
    init_test_app(cx);
    let (editor, visual) = cx
        .add_window_view(|_window, cx| super::Editor::from_markdown(cx, "clean".to_owned(), None));
    editor.update(visual, |editor, cx| {
        add_inactive_tab(editor, "dirty", "dirty.md");
        editor.tabs.records[1]
            .snapshot
            .as_ref()
            .unwrap()
            .source_document
            .set_dirty_for_test(true);

        assert!(editor.activate_dirty_tab_for_window_close(cx));
        assert_eq!(editor.tabs.active, 1);
        assert_eq!(editor.source_document.text(), "dirty");
        assert!(editor.document_dirty);
    });
}

#[gpui::test]
async fn window_close_save_advances_to_next_dirty_tab(cx: &mut gpui::TestAppContext) {
    init_test_app(cx);
    let (editor, visual) = cx.add_window_view(|_window, cx| {
        super::Editor::from_markdown(cx, "first dirty".to_owned(), None)
    });
    editor.update(visual, |editor, cx| {
        editor.set_document_dirty_for_test(true);
        add_inactive_tab(editor, "second dirty", "second.md");
        editor.tabs.records[1]
            .snapshot
            .as_ref()
            .unwrap()
            .source_document
            .set_dirty_for_test(true);

        assert!(!editor.prepare_window_close_save());
        assert!(editor.tabs.continue_window_close_after_save);
        editor.set_document_dirty_for_test(false);
        editor.continue_window_close_after_save(cx);

        assert_eq!(editor.tabs.active, 1);
        assert_eq!(editor.source_document.text(), "second dirty");
        assert!(editor.document_dirty);
        assert!(editor.show_unsaved_changes_dialog);
        assert!(!editor.tabs.continue_window_close_after_save);
    });
}

/// 窗口级放弃覆盖活动与后台 Tab，并以磁盘不可再恢复作为完成条件。
#[gpui::test]
async fn window_close_discard_clears_every_dirty_tab_in_one_action(cx: &mut gpui::TestAppContext) {
    init_test_app(cx);
    let recovery_dir = tempfile::tempdir().expect("inactive recovery fixture");
    let mut recovery_journal = crate::recovery::RecoveryJournal::create(
        recovery_dir.path(),
        Some(PathBuf::from("first.md")),
        "base".to_owned(),
    )
    .expect("create inactive recovery journal");
    recovery_journal
        .record_formatted(
            "first dirty",
            SourceDocument::new("first dirty").source_format(),
            crate::recovery::RecoverySelection {
                start: 0,
                end: 0,
                reversed: false,
                anchor_affinity: None,
                head_affinity: None,
            },
            "rendered",
        )
        .expect("persist inactive recovery journal");
    assert_eq!(
        crate::recovery::load_recovery_documents(recovery_dir.path())
            .expect("load recovery fixture before discard")
            .len(),
        1,
        "fixture must contain one durable recovery document"
    );
    let (editor, visual) = cx.add_window_view(|_window, cx| {
        super::Editor::from_markdown(cx, "first dirty".to_owned(), None)
    });
    editor.update(visual, |editor, cx| {
        editor.set_document_dirty_for_test(true);
        add_inactive_tab(editor, "second dirty", "second.md");
        add_inactive_tab(editor, "third dirty", "third.md");
        editor.recovery_journal = Some(std::sync::Arc::new(std::sync::Mutex::new(
            recovery_journal,
        )));
        assert!(editor.switch_to_tab_index(1, cx));
        editor.set_document_dirty_for_test(true);
        for record in &editor.tabs.records {
            if let Some(snapshot) = record.snapshot.as_ref() {
                snapshot.source_document.set_dirty_for_test(true);
            }
        }

        assert!(editor.discard_all_document_changes_for_window_close(cx));
        assert!(!editor.is_document_dirty());
        assert!(editor.tabs.records.iter().all(|record| {
            record
                .snapshot
                .as_ref()
                .is_none_or(|snapshot| !snapshot.source_document.is_dirty())
        }));
    });
    assert_eq!(
        crate::recovery::load_recovery_documents(recovery_dir.path())
            .expect("load recovery fixture after discard")
            .len(),
        0,
        "window discard must consume inactive tab recovery journals"
    );
}

#[gpui::test]
async fn pinning_and_reordering_preserve_active_snapshot_and_partitions(
    cx: &mut gpui::TestAppContext,
) {
    init_test_app(cx);
    let (editor, visual) = cx
        .add_window_view(|_window, cx| super::Editor::from_markdown(cx, "first".to_owned(), None));
    editor.update(visual, |editor, cx| {
        add_inactive_tab(editor, "second", "second.md");
        add_inactive_tab(editor, "third", "third.md");
        let active_id = editor.tabs.records[0].id;

        assert!(editor.toggle_pin_tab(2, cx));
        assert!(editor.tabs.records[0].pinned);
        assert_eq!(editor.tabs.active, 1);
        assert_eq!(editor.tabs.records[1].id, active_id);

        assert!(editor.toggle_pin_tab(1, cx));
        assert_eq!(editor.pinned_tab_count(), 2);
        assert!(editor.reorder_tab(1, 0, cx));
        assert_eq!(editor.tabs.active, 0);
        assert_eq!(editor.tabs.records[0].id, active_id);

        // 未固定标签不能越过固定前缀，跨分区 drop 会被钳制到合法位置。
        assert!(!editor.reorder_tab(2, 0, cx));
        assert_eq!(editor.pinned_tab_count(), 2);
    });
}

#[gpui::test]
async fn close_other_tabs_prompts_dirty_tabs_and_keeps_requested_tab(
    cx: &mut gpui::TestAppContext,
) {
    init_test_app(cx);
    let (editor, visual) = cx
        .add_window_view(|_window, cx| super::Editor::from_markdown(cx, "first".to_owned(), None));
    let keep_id = editor.update(visual, |editor, cx| {
        add_inactive_tab(editor, "keep", "keep.md");
        add_inactive_tab(editor, "dirty", "dirty.md");
        editor.tabs.records[2]
            .snapshot
            .as_ref()
            .unwrap()
            .source_document
            .set_dirty_for_test(true);
        let keep_id = editor.tabs.records[1].id;
        editor.request_close_other_tabs(1, cx);
        assert_eq!(editor.tabs.records.len(), 2);
        assert!(editor.tabs.show_close_dialog);
        assert_eq!(editor.source_document.text(), "dirty");
        keep_id
    });
    visual.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.on_discard_tab_close(&gpui::ClickEvent::default(), window, cx);
            assert_eq!(editor.tabs.records.len(), 1);
            assert_eq!(editor.tabs.records[0].id, keep_id);
            assert_eq!(editor.source_document.text(), "keep");
            assert!(editor.tabs.close_others_keep.is_none());
        });
    });
}
