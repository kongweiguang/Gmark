// @author kongweiguang

#[path = "editor_tail_15/live_typing_history.rs"]
mod live_typing_history;

#[path = "editor_tail_15/live_input_source_preservation.rs"]
mod live_input_source_preservation;

#[path = "editor_tail_15/resident_selection_source_preservation.rs"]
mod resident_selection_source_preservation;

#[path = "editor_tail_15/resident_structure_source_preservation.rs"]
mod resident_structure_source_preservation;

#[path = "editor_tail_15/resident_source_commit_recovery.rs"]
mod resident_source_commit_recovery;

#[test]
fn history_source_uses_rope_snapshot_and_falls_back_only_when_projection_differs() {
    let document = gmark_document::SourceDocument::new("alpha\n世界🙂");
    let shared = crate::editor::HistorySource::capture(document.snapshot(), document.text());
    assert!(matches!(
        shared,
        crate::editor::HistorySource::Snapshot(_)
    ));
    assert!(shared.matches_text("alpha\n世界🙂"));

    let divergent =
        crate::editor::HistorySource::capture(document.snapshot(), "projection lag".to_owned());
    assert!(matches!(
        divergent,
        crate::editor::HistorySource::Materialized(_)
    ));
    assert!(divergent.matches_text("projection lag"));
}

#[test]
fn two_hundred_large_document_history_entries_retain_rope_snapshots() {
    let mut document = gmark_document::SourceDocument::new(&"x".repeat(256 * 1024));
    let mut history = Vec::with_capacity(200);

    for _ in 0..200 {
        history.push(crate::editor::HistorySource::capture(
            document.snapshot(),
            document.text(),
        ));
        let end = document.len();
        document
            .apply_transaction(gmark_document::Transaction::new(
                document.revision(),
                vec![gmark_document::TextEdit::new(end..end, "x")],
            ))
            .expect("append transaction should succeed");
    }

    assert_eq!(history.len(), 200);
    assert!(history
        .iter()
        .all(|source| matches!(source, crate::editor::HistorySource::Snapshot(_))));
}

#[gpui::test]
async fn dirty_editor_flushes_replayable_recovery_and_save_checkpoints_it(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let temp = tempfile::tempdir().unwrap();
    let journal =
        crate::recovery::RecoveryJournal::create(temp.path(), None, "alpha".to_owned()).unwrap();
    let journal_path = journal.path().to_path_buf();
    let editor = cx.new(|cx| Editor::from_markdown(cx, "alpha".to_owned(), None));
    editor.update(cx, |editor, cx| {
        editor.recovery_journal = Some(Arc::new(Mutex::new(journal)));
        let block = editor.document.first_root().expect("root").clone();
        let end = block.read(cx).visible_len();
        block.update(cx, |block, cx| {
            block.prepare_undo_capture(crate::components::UndoCaptureKind::CoalescibleText, cx);
            block.replace_text_in_visible_range(end..end, " 中文", None, false, cx);
        });
        editor.mark_block_dirty(block.entity_id(), cx);
        assert!(editor.document_dirty);
        assert!(editor.flush_recovery_journal_now(cx).unwrap());
    });

    let recovered = crate::recovery::replay_journal(&journal_path).unwrap();
    assert_eq!(recovered.source, "alpha 中文");
    let saved_path = temp.path().join("saved.md");
    fs::write(&saved_path, &recovered.source).unwrap();
    editor.update(cx, |editor, cx| {
        editor.apply_successful_save(saved_path, recovered.source, recovered.source_format, cx);
        assert!(!editor.document_dirty);
        assert!(!editor.recovered_session);
    });
    assert!(!journal_path.exists());
}

#[gpui::test]
async fn recovered_editor_restores_source_mode_selection_and_dirty_state(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let temp = tempfile::tempdir().unwrap();
    let mut journal =
        crate::recovery::RecoveryJournal::create(temp.path(), None, "alpha".to_owned()).unwrap();
    journal
        .record(
            "alpha 中文",
            crate::recovery::RecoverySelection {
                start: 6,
                end: 12,
                reversed: false,
                anchor_affinity: None,
                head_affinity: None,
            },
            "source",
        )
        .unwrap();
    let recovered = crate::recovery::replay_journal(journal.path()).unwrap();
    let editor = cx.new(move |cx| Editor::from_recovered(cx, recovered));

    editor.read_with(cx, |editor, cx| {
        assert_eq!(editor.source_document.text(), "alpha 中文");
        assert_eq!(editor.view_mode, ViewMode::Source);
        assert!(editor.document_dirty);
        assert_eq!(
            editor
                .document
                .first_root()
                .expect("source root")
                .read(cx)
                .selected_range,
            6..12
        );
        assert!(editor.recovery_journal.is_some());
    });
}

#[gpui::test]
async fn no_edit_save_preserves_bom_and_mixed_line_endings_byte_for_byte(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let path = temp_markdown_path("source-format-round-trip");
    let original = "\u{feff}alpha\r\nbeta\ngamma\rdelta";
    fs::write(&path, original.as_bytes()).unwrap();
    let editor_path = path.clone();
    let (editor, visual_cx) = cx.add_window_view(move |_window, cx| {
        Editor::from_markdown(cx, original.to_owned(), Some(editor_path))
    });

    let saved = visual_cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.set_view_mode(ViewMode::Source, cx);
            editor.save_to_existing_path(&path, window, cx)
        })
    });
    assert!(saved);
    assert_eq!(fs::read(&path).unwrap(), original.as_bytes());
    let _ = fs::remove_file(path);
}

#[gpui::test]
async fn legacy_encoding_opens_read_only_and_requires_explicit_utf8_conversion(
    cx: &mut TestAppContext,
) {
    init_editor_test_app(cx);
    let path = temp_markdown_path("legacy-encoding-conversion");
    let original = [0xff, 0xfe, b'#', 0, b' ', 0, 0x2d, 0x4e, 0x87, 0x65];
    fs::write(&path, original).unwrap();
    let opened = crate::document_io::read_markdown_file(&path).unwrap();
    let editor_path = path.clone();
    let (editor, visual) = cx.add_window_view(move |_window, cx| {
        Editor::from_opened_markdown(cx, opened, Some(editor_path))
    });
    redraw(visual);

    editor.read_with(visual, |editor, cx| {
        assert_eq!(editor.source_document.text(), "# 中文");
        assert_eq!(editor.view_mode, ViewMode::Preview);
        assert!(editor.show_encoding_conversion_dialog);
        assert!(
            editor
                .document
                .visible_blocks()
                .iter()
                .all(|block| block.entity.read(cx).is_read_only())
        );
    });
    assert!(visual.debug_bounds("encoding-conversion-dialog").is_some());

    visual.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.on_keep_legacy_encoding_read_only(&ClickEvent::default(), window, cx);
            assert!(!editor.show_encoding_conversion_dialog);
            editor.set_view_mode(ViewMode::Rendered, cx);
            assert_eq!(editor.view_mode, ViewMode::Preview);
            assert!(editor.show_encoding_conversion_dialog);
            editor.on_convert_encoding_to_utf8(&ClickEvent::default(), window, cx);
            assert!(editor.source_encoding.is_utf8());
            assert_eq!(editor.view_mode, ViewMode::Rendered);
            assert!(editor.document_dirty);
            editor.save_document(window, cx);
        });
    });
    visual.run_until_parked();

    assert_eq!(fs::read(&path).unwrap(), "# 中文".as_bytes());
    editor.read_with(visual, |editor, _cx| assert!(!editor.document_dirty));
    let _ = fs::remove_file(path);
}

#[gpui::test]
async fn source_content_edit_preserves_untouched_mixed_endings(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let path = temp_markdown_path("source-format-edit");
    let original = "alpha\r\nbeta\ngamma\rdelta";
    fs::write(&path, original.as_bytes()).unwrap();
    let editor_path = path.clone();
    let (editor, visual_cx) = cx.add_window_view(move |_window, cx| {
        Editor::from_markdown(cx, original.to_owned(), Some(editor_path))
    });

    let saved = visual_cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.sync_source_document_from_projection("alpha\nBETA\ngamma\ndelta");
            editor.set_document_dirty_for_test(true);
            editor.save_to_existing_path(&path, window, cx)
        })
    });
    assert!(saved);
    assert_eq!(fs::read(&path).unwrap(), b"alpha\r\nBETA\ngamma\rdelta");
    let _ = fs::remove_file(path);
}

#[gpui::test]
async fn completed_old_save_snapshot_does_not_clear_newer_revision_dirty_state(
    cx: &mut TestAppContext,
) {
    init_editor_test_app(cx);
    let path = temp_markdown_path("stale-background-save");
    fs::write(&path, "base").unwrap();
    let editor_path = path.clone();
    let editor = cx.new(move |cx| Editor::from_markdown(cx, "base".to_owned(), Some(editor_path)));

    editor.update(cx, |editor, cx| {
        let saved_revision = editor.source_document.revision();
        let saved_format = editor.source_document.source_format();
        editor.sync_source_document_from_projection("newer edit");
        editor.set_document_dirty_for_test(true);

        assert!(!editor.apply_background_save_success(
            path.clone(),
            "base".to_owned(),
            saved_format,
            saved_revision,
            editor.document_epoch,
            cx,
        ));
        assert_eq!(editor.source_document.text(), "newer edit");
        assert!(editor.document_dirty);
        assert!(editor.pending_window_edited);
    });
    let _ = fs::remove_file(path);
}

#[gpui::test]
async fn completed_save_from_replaced_document_cannot_rebind_editor_state(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let old_path = temp_markdown_path("old-save-epoch");
    let new_path = temp_markdown_path("new-save-epoch");
    fs::write(&old_path, "old").unwrap();
    fs::write(&new_path, "new").unwrap();
    let editor_path = old_path.clone();
    let editor = cx.new(move |cx| Editor::from_markdown(cx, "old".to_owned(), Some(editor_path)));

    editor.update(cx, |editor, cx| {
        let revision = editor.source_document.revision();
        let format = editor.source_document.source_format();
        let old_epoch = editor.document_epoch;
        editor.replace_document_from_markdown("new".to_owned(), Some(new_path.clone()), cx);

        assert!(!editor.apply_background_save_success(
            old_path.clone(),
            "old".to_owned(),
            format,
            revision,
            old_epoch,
            cx,
        ));
        assert_eq!(editor.file_path.as_ref(), Some(&new_path));
        assert_eq!(editor.source_document.text(), "new");
        assert!(!editor.document_dirty);
    });
    let _ = fs::remove_file(old_path);
    let _ = fs::remove_file(new_path);
}

#[gpui::test]
async fn nonvirtual_history_restore_recovers_original_line_ending_map(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let original = "a\r\nb\nc\rd";
    let editor = cx.new(|cx| Editor::from_markdown(cx, original.to_owned(), None));

    editor.update(cx, |editor, cx| {
        let history =
            editor.capture_history_entry(crate::components::UndoCaptureKind::NonCoalescible, cx);
        editor.sync_source_document_from_projection("a\nB\nX\nc\nd");
        editor.restore_history_entry(&history, cx);
        assert_eq!(
            editor.source_document.serialized_bytes(),
            original.as_bytes()
        );
    });
}

#[gpui::test]
async fn malformed_history_line_endings_are_repaired_without_panicking(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let editor = cx.new(|cx| Editor::from_markdown(cx, "current".to_owned(), None));

    editor.update(cx, |editor, cx| {
        let mut history =
            editor.capture_history_entry(crate::components::UndoCaptureKind::NonCoalescible, cx);
        history.source =
            crate::editor::HistorySource::Materialized(std::sync::Arc::from("a\nb"));
        history.source_format = gmark_document::SourceFormatSnapshot {
            utf8_bom: false,
            endings: Vec::new(),
            dominant: gmark_document::LineEnding::CrLf,
        };

        editor.restore_history_entry(&history, cx);
        assert_eq!(editor.source_document.text(), "a\nb");
        assert_eq!(editor.source_document.serialized_bytes(), b"a\r\nb");
    });
}

#[gpui::test]
async fn line_ending_command_is_dirty_undoable_and_saves_real_bytes(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let path = temp_markdown_path("line-ending-normalize");
    let original = "\u{feff}a\r\nb\nc\rd";
    fs::write(&path, original.as_bytes()).unwrap();
    let editor_path = path.clone();
    let (editor, visual_cx) = cx.add_window_view(move |_window, cx| {
        Editor::from_markdown(cx, original.to_owned(), Some(editor_path))
    });

    editor.update(visual_cx, |editor, cx| {
        editor.normalize_line_endings(gmark_document::LineEnding::CrLf, cx);
        assert!(editor.document_dirty);
        assert_eq!(editor.undo_history.len(), 1);
        assert_eq!(
            editor.source_document.serialized_bytes(),
            b"\xef\xbb\xbfa\r\nb\r\nc\r\nd"
        );

        editor.undo_document(cx);
        assert_eq!(
            editor.source_document.serialized_bytes(),
            original.as_bytes()
        );
        editor.redo_document(cx);
        assert_eq!(
            editor.source_document.serialized_bytes(),
            b"\xef\xbb\xbfa\r\nb\r\nc\r\nd"
        );
    });

    let saved = visual_cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.save_to_existing_path(&path, window, cx)
        })
    });
    assert!(saved);
    assert_eq!(fs::read(&path).unwrap(), b"\xef\xbb\xbfa\r\nb\r\nc\r\nd");
    let _ = fs::remove_file(path);
}

#[gpui::test]
async fn line_ending_command_noop_and_preview_do_not_create_edits(cx: &mut TestAppContext) {
    let editor = cx.new(|cx| Editor::from_markdown(cx, "a\nb".to_owned(), None));

    editor.update(cx, |editor, cx| {
        editor.normalize_line_endings(gmark_document::LineEnding::Lf, cx);
        assert!(!editor.document_dirty);
        assert!(editor.undo_history.is_empty());

        editor.set_view_mode(ViewMode::Preview, cx);
        editor.normalize_line_endings(gmark_document::LineEnding::CrLf, cx);
        assert!(!editor.document_dirty);
        assert!(editor.undo_history.is_empty());
        assert_eq!(editor.source_document.serialized_bytes(), b"a\nb");
    });
}

#[gpui::test]
async fn format_only_edit_is_replayable_from_recovery_journal(cx: &mut TestAppContext) {
    let temp = tempfile::tempdir().unwrap();
    let original = gmark_document::SourceDocument::new("a\r\nb\nc");
    let journal = crate::recovery::RecoveryJournal::create_formatted(
        temp.path(),
        None,
        original.text(),
        original.source_format(),
    )
    .unwrap();
    let journal_path = journal.path().to_path_buf();
    let editor = cx.new(|cx| Editor::from_markdown(cx, "a\r\nb\nc".to_owned(), None));

    editor.update(cx, |editor, cx| {
        editor.recovery_journal = Some(Arc::new(Mutex::new(journal)));
        editor.normalize_line_endings(gmark_document::LineEnding::Cr, cx);
        assert!(editor.flush_recovery_journal_now(cx).unwrap());
    });

    let recovered = crate::recovery::replay_journal(&journal_path).unwrap();
    let restored = gmark_document::SourceDocument::from_normalized(
        &recovered.source,
        recovered.source_format,
        gmark_document::SourceDocument::DEFAULT_HISTORY_LIMIT,
    )
    .unwrap();
    assert_eq!(restored.serialized_bytes(), b"a\rb\rc");
}

#[gpui::test]
async fn virtual_line_ending_command_keeps_selection_history_aligned(cx: &mut TestAppContext) {
    let mut source = String::new();
    for index in 0..9_000 {
        if index > 0 {
            source.push_str(if index % 2 == 0 { "\r\n\r\n" } else { "\n\n" });
        }
        source.push_str(&format!("paragraph {index}"));
    }
    let original = source.as_bytes().to_vec();
    let editor = cx.new(move |cx| Editor::from_markdown(cx, source, None));

    editor.update(cx, |editor, cx| {
        assert!(editor.virtual_surface.is_some());
        editor.normalize_line_endings(gmark_document::LineEnding::CrLf, cx);
        assert_eq!(editor.virtual_undo_selections.len(), 1);
        assert_eq!(editor.virtual_redo_selections.len(), 0);
        assert!(
            editor
                .source_document
                .serialized_bytes()
                .windows(2)
                .filter(|bytes| *bytes == b"\r\n")
                .count()
                >= 17_998
        );

        editor.undo_document(cx);
        assert_eq!(editor.source_document.serialized_bytes(), original);
        assert!(editor.virtual_undo_selections.is_empty());
        assert_eq!(editor.virtual_redo_selections.len(), 1);

        editor.redo_document(cx);
        assert_eq!(editor.virtual_undo_selections.len(), 1);
        assert!(editor.virtual_redo_selections.is_empty());
    });
}

#[gpui::test]
async fn auto_save_after_idle_uses_normal_existing_file_save_path(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    cx.update(|cx| {
        crate::config::EditorSettings::init(
            cx,
            true,
            crate::config::AutoSavePreference::AfterDelay,
            true,
        );
    });
    let path = temp_markdown_path("auto-save-idle");
    fs::write(&path, "base").unwrap();
    let editor_path = path.clone();
    let (editor, visual_cx) = cx.add_window_view(move |_window, cx| {
        Editor::from_markdown(cx, "base".to_owned(), Some(editor_path))
    });
    redraw(visual_cx);

    editor.update(visual_cx, |editor, cx| {
        editor.sync_source_document_from_projection("edited");
        editor.set_document_dirty_for_test(true);
        editor.schedule_auto_save(cx);
        assert!(editor.auto_save_task.is_some());
    });
    visual_cx.executor().advance_clock(Duration::from_secs(1));
    visual_cx.run_until_parked();
    redraw(visual_cx);

    assert_eq!(fs::read_to_string(&path).unwrap(), "edited");
    editor.read_with(visual_cx, |editor, _cx| {
        assert!(!editor.document_dirty);
        assert!(editor.auto_save_task.is_none());
        assert!(!editor.external_file_conflict);
    });
    let _ = fs::remove_file(path);
}

#[gpui::test]
/// 保留真实 CRLF 源格式，验证自动保存刷新后的文件身份也能通过 Ctrl+S 再次保存。
async fn ctrl_s_after_crlf_auto_save_uses_refreshed_file_fingerprint(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    cx.update(|cx| {
        crate::config::EditorSettings::init(
            cx,
            true,
            crate::config::AutoSavePreference::AfterDelay,
            true,
        );
    });
    let path = temp_markdown_path("crlf-auto-save-then-ctrl-s");
    fs::write(&path, b"alpha\r\nbeta\r\nomega").unwrap();
    let opened = crate::document_io::read_markdown_file(&path).unwrap();
    let editor_path = path.clone();
    let (editor, visual_cx) = cx.add_window_view(move |_window, cx| {
        Editor::from_opened_markdown(cx, opened, Some(editor_path))
    });
    redraw(visual_cx);

    editor.update(visual_cx, |editor, cx| {
        editor.sync_source_document_from_projection("edited\nbeta\nomega");
        editor.set_document_dirty_for_test(true);
        editor.schedule_auto_save(cx);
        assert!(editor.auto_save_task.is_some());
    });
    visual_cx.executor().advance_clock(Duration::from_secs(1));
    visual_cx.run_until_parked();
    redraw(visual_cx);

    let expected = b"edited\r\nbeta\r\nomega";
    assert_eq!(fs::read(&path).unwrap(), expected);
    editor.read_with(visual_cx, |editor, _cx| {
        assert!(!editor.document_dirty);
        assert!(!editor.external_file_conflict);
        assert!(editor.save_task.is_none());
        assert_eq!(
            editor.saved_file_fingerprint,
            crate::recovery::fingerprint_file(&path).ok(),
            "the automatic save callback must install the identity of the written CRLF file"
        );
        let active_path = editor.file_path.as_deref().expect("saved file path");
        assert_eq!(
            editor.saved_file_fingerprint,
            crate::recovery::fingerprint_file(active_path).ok(),
            "the baseline must use the same canonical path spelling as the active save target"
        );
    });

    editor.update(visual_cx, |editor, cx| editor.request_save_document(cx));
    redraw(visual_cx);
    visual_cx.run_until_parked();
    redraw(visual_cx);

    assert_eq!(fs::read(&path).unwrap(), expected);
    editor.read_with(visual_cx, |editor, _cx| {
        assert!(!editor.external_file_conflict);
        assert!(!editor.show_external_conflict_dialog);
        assert!(!editor.document_dirty);
        assert!(editor.save_task.is_none());
    });
    let _ = fs::remove_file(path);
}

#[gpui::test]
async fn auto_save_never_schedules_untitled_recovered_or_conflicted_documents(
    cx: &mut TestAppContext,
) {
    init_editor_test_app(cx);
    cx.update(|cx| {
        crate::config::EditorSettings::init(
            cx,
            true,
            crate::config::AutoSavePreference::AfterDelay,
            true,
        );
    });
    let editor = cx.new(|cx| Editor::from_markdown(cx, "dirty".to_owned(), None));
    editor.update(cx, |editor, cx| {
        editor.set_document_dirty_for_test(true);
        editor.schedule_auto_save(cx);
        assert!(editor.auto_save_task.is_none());

        editor.file_path = Some(temp_markdown_path("auto-save-guard"));
        editor.recovered_session = true;
        editor.schedule_auto_save(cx);
        assert!(editor.auto_save_task.is_none());

        editor.recovered_session = false;
        editor.external_file_conflict = true;
        editor.schedule_auto_save(cx);
        assert!(editor.auto_save_task.is_none());
    });
}

#[gpui::test]
async fn later_edit_resets_auto_save_idle_deadline(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    cx.update(|cx| {
        crate::config::EditorSettings::init(
            cx,
            true,
            crate::config::AutoSavePreference::AfterDelay,
            true,
        );
    });
    let path = temp_markdown_path("auto-save-debounce");
    fs::write(&path, "base").unwrap();
    let editor_path = path.clone();
    let editor = cx.new(move |cx| Editor::from_markdown(cx, "base".to_owned(), Some(editor_path)));

    editor.update(cx, |editor, cx| {
        editor.set_document_dirty_for_test(true);
        editor.schedule_auto_save(cx);
    });
    cx.executor().advance_clock(Duration::from_millis(500));
    cx.run_until_parked();
    editor.update(cx, |editor, cx| editor.schedule_auto_save(cx));
    cx.executor().advance_clock(Duration::from_millis(500));
    cx.run_until_parked();
    editor.read_with(cx, |editor, _cx| assert!(!editor.pending_save));

    cx.executor().advance_clock(Duration::from_millis(500));
    cx.run_until_parked();
    editor.read_with(cx, |editor, _cx| assert!(editor.pending_save));
    let _ = fs::remove_file(path);
}

#[gpui::test]
async fn active_block_spellcheck_runs_off_thread_and_publishes_utf8_ranges(
    cx: &mut TestAppContext,
) {
    init_editor_test_app(cx);
    cx.update(|cx| {
        crate::config::EditorSettings::init(cx, true, crate::config::AutoSavePreference::Off, true);
    });
    let editor = cx.new(|cx| Editor::from_markdown(cx, "中文 sentnce".to_owned(), None));
    editor.update(cx, |editor, cx| {
        editor.schedule_active_block_spellcheck(cx);
        assert!(editor.spellcheck_task.is_some());
    });
    cx.executor().advance_clock(Duration::from_millis(250));
    cx.run_until_parked();

    editor.read_with(cx, |editor, cx| {
        let block = editor.document.first_root().expect("root").read(cx);
        let diagnostic = block
            .spelling_diagnostics
            .iter()
            .find(|diagnostic| &block.display_text()[diagnostic.range.clone()] == "sentnce")
            .expect("misspelling should publish to active block");
        assert!(
            diagnostic
                .replacements
                .iter()
                .any(|value| value == "sentence")
        );
        assert!(editor.spellcheck_task.is_none());
    });
}

#[gpui::test]
async fn disabled_spellcheck_clears_existing_diagnostics_without_scheduling(
    cx: &mut TestAppContext,
) {
    init_editor_test_app(cx);
    cx.update(|cx| {
        crate::config::EditorSettings::init(
            cx,
            true,
            crate::config::AutoSavePreference::Off,
            false,
        );
    });
    let editor = cx.new(|cx| Editor::from_markdown(cx, "sentnce".to_owned(), None));
    editor.update(cx, |editor, cx| {
        let block = editor.document.first_root().expect("root").clone();
        block.update(cx, |block, _cx| {
            block.spelling_diagnostics = vec![crate::spellcheck::SpellingDiagnostic {
                range: 0..7,
                original: "sentnce".to_owned(),
                message: "Unknown word".to_owned(),
                replacements: vec!["sentence".to_owned()],
            }]
            .into();
        });
        editor.schedule_active_block_spellcheck(cx);
        assert!(editor.spellcheck_task.is_none());
        assert!(block.read(cx).spelling_diagnostics.is_empty());
    });
}

#[gpui::test]
/// 模拟指纹预检完成后磁盘又发生变化：原子写的 SourceChanged 必须进入版本比较
/// 对话框，不能显示无恢复动作的系统级保存失败。
async fn save_identity_race_opens_external_conflict_recovery(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let path = temp_markdown_path("save-identity-race");
    fs::write(&path, "base").unwrap();
    let editor_path = path.clone();
    let (editor, visual_cx) = cx.add_window_view(move |_window, cx| {
        Editor::from_markdown(cx, "base".to_owned(), Some(editor_path))
    });
    redraw(visual_cx);

    fs::write(&path, "external edit").unwrap();
    let saved = visual_cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            // 指纹已经观察到新文件，但共享 Controller 仍持有打开时身份，精确复现
            // 外部更新落在预检与原子替换之间的竞态窗口。
            editor.saved_file_fingerprint = crate::recovery::fingerprint_file(&path).ok();
            editor.sync_source_document_from_projection("local edit");
            editor.set_document_dirty_for_test(true);
            editor.save_to_existing_path(&path, window, cx)
        })
    });

    editor.read_with(visual_cx, |editor, _cx| {
        assert!(!saved);
        assert!(editor.external_file_conflict);
        assert!(editor.document_dirty);
        assert!(editor.show_external_conflict_dialog);
        let preview = editor
            .external_conflict_preview
            .as_ref()
            .expect("race conflict comparison");
        assert_eq!(preview.local_line, "local edit");
        assert_eq!(preview.disk_line, "external edit");
    });
    assert_eq!(fs::read_to_string(&path).unwrap(), "external edit");
    let _ = fs::remove_file(path);
}

#[gpui::test]
async fn save_blocks_external_file_overwrite_and_keeps_document_dirty(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let path = temp_markdown_path("external-save-conflict");
    fs::write(&path, "base").unwrap();
    let editor_path = path.clone();
    let (editor, visual_cx) = cx.add_window_view(move |_window, cx| {
        Editor::from_markdown(cx, "base".to_owned(), Some(editor_path))
    });
    redraw(visual_cx);
    fs::write(&path, "external edit").unwrap();
    let saved = visual_cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.sync_source_document_from_projection("local edit");
            editor.set_document_dirty_for_test(true);
            editor.pending_window_edited = true;
            editor.save_to_existing_path(&path, window, cx)
        })
    });
    editor.read_with(visual_cx, |editor, _cx| {
        assert!(!saved);
        assert!(editor.external_file_conflict);
        assert!(editor.document_dirty);
        assert!(editor.show_external_conflict_dialog);
        let preview = editor
            .external_conflict_preview
            .as_ref()
            .expect("comparison preview");
        assert_eq!(preview.first_difference_line, Some(1));
        assert_eq!(preview.local_line, "local edit");
        assert_eq!(preview.disk_line, "external edit");
    });
    assert_eq!(fs::read_to_string(&path).unwrap(), "external edit");
    visual_cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.on_overwrite_external_conflict(&ClickEvent::default(), window, cx);
            editor.save_document(window, cx);
        });
    });
    visual_cx.run_until_parked();
    editor.read_with(visual_cx, |editor, _cx| {
        assert!(!editor.document_dirty);
        assert!(!editor.external_file_conflict);
        assert!(!editor.show_external_conflict_dialog);
    });
    assert_eq!(fs::read_to_string(&path).unwrap(), "local edit");
    let _ = fs::remove_file(path);
}
