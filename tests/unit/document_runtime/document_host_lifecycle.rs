// @author kongweiguang

//! DocumentHost opening, backend selection, and recovery lifecycle tests.

use super::*;
use gpui::EntityInputHandler;

#[test]
fn source_font_uses_a_real_direct_write_family_on_windows() {
    #[cfg(target_os = "windows")]
    assert_eq!(source_monospace_font_family(), "Consolas");
    #[cfg(target_os = "macos")]
    assert_eq!(source_monospace_font_family(), "Menlo");
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    assert_eq!(source_monospace_font_family(), "monospace");
}

/// Keep the probe/edit race on the external test surface so production indexing has no inline test module.
#[gpui::test]
async fn empty_probe_growth_falls_back_to_background_reprobe(cx: &mut gpui::TestAppContext) {
    let temp = tempfile::tempdir().expect("empty probe race tempdir");
    let path = temp.path().join("grown-after-probe.txt");
    fs::write(&path, []).expect("empty probe race fixture");
    let source = FileSource::open(&path).expect("empty probe race source");
    let stale_probe =
        gmark_paged_document::probe_file(&path, gmark_paged_document::ProbeOptions::default())
            .expect("empty probe race probe");
    fs::write(&path, "grown after probe\n").expect("grow after probe");

    let view = cx.new(|cx| DocumentHost::new(path, stale_probe, source, cx));
    cx.run_until_parked();

    view.update(cx, |view, _cx| {
        assert_eq!(view.probe.len, "grown after probe\n".len() as u64);
        assert_eq!(view.source_text_for_test(), "grown after probe\n");
        assert!(view.document.is_some());
        assert!(view.error.is_none());
    });
}

/// 锁定 Paged Source 的真实按键入口：Host 拥有焦点时 Right 必须折叠当前词选区。
#[gpui::test]
async fn paged_source_right_collapses_word_selection_to_its_end(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        crate::i18n::I18nManager::init(cx);
        crate::theme::ThemeManager::init(cx);
        crate::components::init(cx);
    });
    let temp = tempfile::tempdir().expect("horizontal navigation tempdir");
    let path = temp.path().join("horizontal-navigation.txt");
    fs::write(&path, "alpha_beta next\n").expect("horizontal navigation fixture");
    let probe = gmark_paged_document::probe_file(
        &path,
        gmark_paged_document::ProbeOptions {
            max_resident_bytes: 1,
            ..gmark_paged_document::ProbeOptions::default()
        },
    )
    .expect("Paged Source probe");
    assert_eq!(probe.strategy, OpenStrategy::Paged);
    let source = FileSource::open(&path).expect("Paged Source file");
    let (host, visual) =
        cx.add_window_view(move |_window, cx| DocumentHost::new(path, probe, source, cx));
    visual.run_until_parked();
    visual.update(|window, cx| {
        host.update(cx, |host, _cx| {
            host.select_source_range_and_focus_for_test(0..10, false, window);
        });
    });

    visual.simulate_keystrokes("right");

    assert_eq!(
        host.read_with(visual, |host, _cx| host.source_selection_for_test())
            .expect("Source selection"),
        SourceSelection::collapsed(10, SourceAffinity::After),
        "Right must collapse a word selection at its end instead of leaving it active"
    );
}

/// Source 水平移动按完整字素跨 CRLF，Shift 扩选继续保留原源码锚点。
#[gpui::test]
async fn paged_source_horizontal_navigation_preserves_graphemes_and_anchor(
    cx: &mut gpui::TestAppContext,
) {
    cx.update(|cx| {
        crate::i18n::I18nManager::init(cx);
        crate::theme::ThemeManager::init(cx);
        crate::components::init(cx);
    });
    let temp = tempfile::tempdir().expect("horizontal navigation tempdir");
    let path = temp.path().join("horizontal-graphemes.txt");
    let family = "👨‍👩‍👧‍👦";
    let source = format!("e\u{301}{family}x\r\nnext");
    fs::write(&path, &source).expect("horizontal navigation fixture");
    let probe = gmark_paged_document::probe_file(
        &path,
        gmark_paged_document::ProbeOptions {
            max_resident_bytes: 1,
            ..gmark_paged_document::ProbeOptions::default()
        },
    )
    .expect("Paged Source probe");
    assert_eq!(probe.strategy, OpenStrategy::Paged);
    let source_file = FileSource::open(&path).expect("Paged Source file");
    let (host, visual) =
        cx.add_window_view(move |_window, cx| DocumentHost::new(path, probe, source_file, cx));
    visual.run_until_parked();

    visual.update(|window, cx| {
        host.update(cx, |host, _cx| {
            host.select_source_range_and_focus_for_test(0..0, false, window);
        });
    });
    visual.simulate_keystrokes("right");
    let combining_end = "e\u{301}".len() as u64;
    assert_eq!(
        host.read_with(visual, |host, _cx| host.source_selection_for_test())
            .expect("combining grapheme selection"),
        SourceSelection::collapsed(combining_end, SourceAffinity::After)
    );

    visual.simulate_keystrokes("right");
    let family_end = combining_end + family.len() as u64;
    assert_eq!(
        host.read_with(visual, |host, _cx| host.source_selection_for_test())
            .expect("family emoji selection"),
        SourceSelection::collapsed(family_end, SourceAffinity::After)
    );

    visual.update(|window, cx| {
        host.update(cx, |host, _cx| {
            host.select_source_range_and_focus_for_test(family_end..family_end, false, window);
        });
    });
    visual.simulate_keystrokes("shift-right");
    assert_eq!(
        host.read_with(visual, |host, _cx| host.source_selection_for_test())
            .expect("shift grapheme selection"),
        SourceSelection {
            anchor: SourceAnchor::new(family_end, SourceAffinity::Before),
            head: SourceAnchor::new(family_end + 1, SourceAffinity::After),
        }
    );

    let x_end = source.find("x").expect("x fixture") as u64 + 1;
    let next_line_start = source.find("next").expect("next fixture") as u64;
    visual.update(|window, cx| {
        host.update(cx, |host, _cx| {
            host.select_source_range_and_focus_for_test(x_end..x_end, false, window);
        });
    });
    visual.simulate_keystrokes("right");
    assert_eq!(
        host.read_with(visual, |host, _cx| host.source_selection_for_test())
            .expect("CRLF forward navigation"),
        SourceSelection::collapsed(next_line_start, SourceAffinity::After)
    );
    visual.simulate_keystrokes("left");
    assert_eq!(
        host.read_with(visual, |host, _cx| host.source_selection_for_test())
            .expect("CRLF backward navigation"),
        SourceSelection::collapsed(x_end, SourceAffinity::After)
    );
}

/// Keep the exact UTF-8 paste boundary stable so callers cannot reject valid 64 MiB input.
#[test]
fn source_paste_limit_accepts_the_64_mib_boundary() {
    let limit = gmark_paged_document::MAX_SYSTEM_CLIPBOARD_BYTES as usize;
    assert!(!DocumentHost::source_paste_exceeds_limit(
        &"x".repeat(limit)
    ));
    assert!(DocumentHost::source_paste_exceeds_limit(
        &"x".repeat(limit + 1)
    ));
}

/// 锁定同步完成请求被拒绝后的重试语义，避免剪贴板和另存为动作重复入队。
#[gpui::test]
async fn source_ime_sync_finish_failure_retry_does_not_duplicate_actions(
    cx: &mut gpui::TestAppContext,
) {
    cx.update(|cx| {
        crate::i18n::I18nManager::init(cx);
        crate::theme::ThemeManager::init(cx);
        crate::components::init(cx);
    });
    let (host, visual) = cx.add_window_view(|_window, cx| {
        DocumentHost::new_untitled(
            PathBuf::from("Untitled-source-ime-sync.txt"),
            DocumentFormat::PlainText,
            "candidate",
            cx,
        )
    });

    visual.update(|window, cx| {
        host.update(cx, |host, cx| {
            host.begin_line_edit_for_test(0, window, cx);
            let (_, input) = host
                .active_edit_for_test()
                .expect("Source input must be mounted before IME starts");
            input.update(cx, |block, cx| block.composition_started(window, cx));

            host.paste_for_test(window, cx);
            assert_eq!(
                host.pending_source_ime_action
                    .as_ref()
                    .expect("paste must wait for IME")
                    .actions
                    .len(),
                1
            );
            assert!(
                !host
                    .pending_source_ime_action
                    .as_ref()
                    .expect("paste must remain queued after refusal")
                    .request_in_flight
            );

            host.cut_for_test(window, cx);
            host.on_save_document_as(&SaveDocumentAs, window, cx);
            assert_eq!(
                host.pending_source_ime_action
                    .as_ref()
                    .expect("all requested actions must stay queued")
                    .actions
                    .len(),
                3
            );

            // 测试平台已拒绝组合输入完成；同一命令的显式重试不能再增加队列项。
            host.paste_for_test(window, cx);
            host.cut_for_test(window, cx);
            host.on_save_document_as(&SaveDocumentAs, window, cx);
            assert_eq!(
                host.pending_source_ime_action
                    .as_ref()
                    .expect("failed actions must remain retryable")
                    .actions
                    .len(),
                3
            );
        });
    });
}

/// 锁定异步完成拒绝后的去重边界，并保留已受理在途请求中的用户重复操作。
#[gpui::test]
async fn source_ime_async_finish_failure_retry_does_not_duplicate_actions(
    cx: &mut gpui::TestAppContext,
) {
    cx.update(|cx| {
        crate::i18n::I18nManager::init(cx);
        crate::theme::ThemeManager::init(cx);
        crate::components::init(cx);
    });
    let (host, visual) = cx.add_window_view(|_window, cx| {
        DocumentHost::new_untitled(
            PathBuf::from("Untitled-source-ime-async.txt"),
            DocumentFormat::PlainText,
            "candidate",
            cx,
        )
    });

    visual.update(|window, cx| {
        host.update(cx, |host, cx| {
            host.begin_line_edit_for_test(0, window, cx);
            let (_, input) = host
                .active_edit_for_test()
                .expect("Source input must be mounted before IME starts");
            input.update(cx, |block, cx| block.composition_started(window, cx));

            host.paste_for_test(window, cx);
            let pending = host
                .pending_source_ime_action
                .as_mut()
                .expect("paste must wait for IME");
            // GPUI TestPlatform 不能受理原生请求；这里模拟已受理状态，再调用实际 Host 终态处理器覆盖异步拒绝。
            pending.request_in_flight = true;

            host.cut_for_test(window, cx);
            host.on_save_document_as(&SaveDocumentAs, window, cx);
            let owner = host
                .active_edit_for_test()
                .expect("IME owner must stay mounted until terminal event")
                .1;
            host.on_source_ime_terminal(&owner, &BlockEvent::ImeCompositionFinishFailed, cx);

            assert_eq!(
                host.pending_source_ime_action
                    .as_ref()
                    .expect("asynchronous refusal must preserve queued work")
                    .actions
                    .len(),
                3
            );
            host.paste_for_test(window, cx);
            host.cut_for_test(window, cx);
            host.on_save_document_as(&SaveDocumentAs, window, cx);
            assert_eq!(
                host.pending_source_ime_action
                    .as_ref()
                    .expect("failed actions must remain retryable")
                    .actions
                    .len(),
                3
            );

            // 后续请求已受理后属于新的等待区间；用户重复操作仍是独立动作。
            host.pending_source_ime_action
                .as_mut()
                .expect("pending source request")
                .request_in_flight = true;
            host.paste_for_test(window, cx);
            assert_eq!(
                host.pending_source_ime_action
                    .as_ref()
                    .expect("in-flight repeated action must be queued")
                    .actions
                    .len(),
                4
            );
        });
    });
}

/// Verify an oversized source paste reports an error before the shared document transaction.
#[gpui::test]
async fn source_paste_over_limit_does_not_mutate_the_document(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        crate::i18n::I18nManager::init(cx);
        crate::theme::ThemeManager::init(cx);
        crate::components::init(cx);
    });
    let (host, visual) = cx.add_window_view(|_window, cx| {
        DocumentHost::new_untitled(
            PathBuf::from("Untitled-paste-limit.txt"),
            DocumentFormat::PlainText,
            "before",
            cx,
        )
    });
    let over_limit = "x".repeat((gmark_paged_document::MAX_SYSTEM_CLIPBOARD_BYTES + 1) as usize);
    visual.write_to_clipboard(gpui::ClipboardItem::new_string(over_limit));
    visual.update(|window, cx| {
        host.update(cx, |view, cx| {
            view.select_source_range_for_test(0..6, false);
            view.paste_for_test(window, cx);
        });
    });

    host.read_with(visual, |view, _cx| {
        assert_eq!(view.source_text_for_test(), "before");
        assert!(view.error.is_some());
    });
}

#[gpui::test]
async fn untitled_json_and_csv_install_resident_format_capabilities(cx: &mut gpui::TestAppContext) {
    let json = cx.new(|cx| {
        DocumentHost::new_untitled(
            PathBuf::from("Untitled.json"),
            DocumentFormat::Json,
            "{\n  \"name\": \"x\"\n}\n",
            cx,
        )
    });
    json.update(cx, |host, _cx| {
        assert!(host.is_json_document());
        assert!(host.has_registered_structure_view());
        assert!(host.source_view_for_test());
        assert_eq!(host.source_text_for_test(), "{\n  \"name\": \"x\"\n}\n");
        assert!(host.provisional_source.is_none());
        let snapshot = host.document_sidebar_snapshot();
        assert_eq!(snapshot.format, DocumentMenuFormat::Json);
        assert_eq!(snapshot.metadata.encoding, "UTF-8");
        assert!(snapshot.metadata.lines >= 1);
        assert!(snapshot.document_epoch > 0);
    });

    let csv = cx.new(|cx| {
        DocumentHost::new_untitled(
            PathBuf::from("Untitled.csv"),
            DocumentFormat::Delimited { delimiter: b',' },
            "Column 1,Column 2\n",
            cx,
        )
    });
    csv.update(cx, |host, _cx| {
        assert!(host.is_delimited_document());
        assert!(host.has_registered_structure_view());
        assert!(host.source_view_for_test());
        assert_eq!(host.source_text_for_test(), "Column 1,Column 2\n");
        assert!(matches!(
            host.structured_index,
            Some(StructuredIndex::Delimited(_))
        ));
        assert!(host.provisional_source.is_none());
        let snapshot = host.document_sidebar_snapshot();
        assert_eq!(snapshot.format, DocumentMenuFormat::Csv);
        assert_eq!(snapshot.metadata.length, "Column 1,Column 2\n".len() as u64);
        assert_eq!(snapshot.nodes.len(), 2);
        assert!(matches!(
            snapshot.nodes[0].target,
            DocumentSidebarTarget::Column { column: 0 }
        ));
    });
}

#[gpui::test]
async fn empty_file_mount_is_editable_before_any_viewport_worker_turn(
    cx: &mut gpui::TestAppContext,
) {
    cx.update(|cx| {
        crate::i18n::I18nManager::init(cx);
        crate::theme::ThemeManager::init(cx);
        crate::components::init(cx);
    });
    let temp = tempfile::tempdir().expect("empty file first-edit tempdir");
    let path = temp.path().join("first-edit.txt");
    fs::write(&path, []).expect("empty first-edit fixture");
    let source = FileSource::open(&path).expect("empty first-edit source");
    let probe =
        gmark_paged_document::probe_file(&path, gmark_paged_document::ProbeOptions::default())
            .expect("empty first-edit probe");
    let path_for_view = path.clone();
    let (host, visual) =
        cx.add_window_view(move |_window, cx| DocumentHost::new(path_for_view, probe, source, cx));
    let recovery_dir = temp.path().join("recovery");
    fs::create_dir_all(&recovery_dir).expect("empty first-edit recovery dir");
    let recovery_source = FileSource::open(&path).expect("empty first-edit recovery source");
    let mut journal_path = None;
    let mut pending_journal = None;

    visual.update(|window, cx| {
        host.update(cx, |host, cx| {
            assert!(
                host.document.is_some(),
                "empty file must have a session at mount"
            );
            assert!(host.provisional_source.is_none());
            assert!(host.external_monitor_owned);
            assert!(!host.external_monitor_paused_for_test());
            assert_eq!(host.line_count(), 1);
            assert_eq!(host.displayed_screen_lines.visible, 0..1);
            assert!(host.displayed_screen_lines.row(0).is_some());
            // 生产构建会启用恢复日志；测试显式补齐它，锁定首次输入不得在
            // `with_session` 内再次读取 Controller revision 而自锁。
            let document = host.document.as_ref().expect("empty document").clone();
            let journal = document
                .with_session(|session| {
                    DocumentRecoveryJournal::create(
                        &recovery_dir,
                        &recovery_source,
                        host.probe.encoding.clone(),
                        session,
                    )
                })
                .expect("empty first-edit recovery controller")
                .expect("empty first-edit recovery journal");
            journal_path = Some(journal.path().to_path_buf());
            // Hold the already-created journal outside the host to model the
            // production background callback arriving after the first edit.
            pending_journal = Some(journal);
            host.coordinator.recovery_enabled = true;
            host.begin_line_edit_for_test(0, window, cx);
            let (_, row) = host
                .active_edit_for_test()
                .expect("empty Source row must accept the first edit");
            row.update(cx, |block, cx| {
                block.replace_text_in_visible_range(0..0, "hello", None, false, cx);
            });
        });
    });
    visual.run_until_parked();
    host.read_with(visual, |host, _cx| {
        assert!(
            host.coordinator.pending_recovery_record.is_some(),
            "the first edit must survive until journal installation"
        );
    });
    let journal = pending_journal
        .take()
        .expect("journal callback fixture must remain pending");
    visual.update(|_window, cx| {
        host.update(cx, |host, cx| {
            host.install_recovery_journal(journal, cx);
        });
    });
    visual.run_until_parked();
    visual
        .executor()
        .advance_clock(std::time::Duration::from_millis(1_100));
    visual.run_until_parked();

    host.read_with(visual, |host, _cx| {
        assert_eq!(host.source_text_for_test(), "hello");
        assert!(host.is_dirty());
        assert_eq!(host.pending_external_change_for_test(), None);
    });
    assert_eq!(
        fs::read(&path).expect("read untouched empty file"),
        Vec::<u8>::new()
    );
    let journal_path = journal_path.expect("empty first-edit journal path");
    let recovered = gmark_document_runtime::replay_resident_recovery_journal(&journal_path)
        .expect("background recovery must produce a replayable journal");
    assert_eq!(recovered.source, "hello");
}

#[gpui::test]
async fn empty_json_and_csv_hosts_do_not_inherit_untitled_templates(cx: &mut gpui::TestAppContext) {
    for (name, expected_format) in [
        ("empty.json", DocumentFormat::Json),
        ("empty.csv", DocumentFormat::Delimited { delimiter: b',' }),
    ] {
        let temp = tempfile::tempdir().expect("empty format tempdir");
        let path = temp.path().join(name);
        fs::write(&path, []).expect("empty format fixture");
        let source = FileSource::open(&path).expect("empty format source");
        let probe =
            gmark_paged_document::probe_file(&path, gmark_paged_document::ProbeOptions::default())
                .expect("empty format probe");
        assert_eq!(probe.format, expected_format);
        let host = cx.new(|cx| DocumentHost::new(path, probe, source, cx));
        host.update(cx, |host, _cx| {
            assert_eq!(host.source_text_for_test(), "");
            assert!(host.document.is_some());
            assert_eq!(host.line_count(), 1);
        });
    }
}

#[test]
fn stale_resident_probe_is_rejected_and_fresh_probe_replans_to_paged() {
    let temp = tempfile::tempdir().expect("probe race tempdir");
    let path = temp.path().join("probe-race.csv");
    fs::write(&path, "a,b\n").expect("small probe fixture");
    let options = gmark_paged_document::ProbeOptions {
        max_resident_bytes: 8,
        ..gmark_paged_document::ProbeOptions::default()
    };
    let stale_probe = gmark_paged_document::probe_file(&path, options).expect("resident probe");
    assert_eq!(stale_probe.strategy, OpenStrategy::Resident);

    fs::write(&path, "a,b\n1,2\n3,4\n").expect("grow fixture after probe");
    let changed_source = FileSource::open(&path).expect("changed source");
    let changed_index = LineIndex::build(&changed_source).expect("changed index");
    assert!(matches!(
        build_document_session(
            &stale_probe,
            &changed_source,
            changed_source.clone(),
            changed_index,
            false,
        ),
        Err(PagedDocumentError::SourceChanged)
    ));

    let fresh_probe = gmark_paged_document::probe_file(&path, options).expect("fresh probe");
    assert_eq!(fresh_probe.strategy, OpenStrategy::Paged);
    let fresh_source = FileSource::open(&path).expect("fresh source");
    let fresh_index = LineIndex::build(&fresh_source).expect("fresh index");
    let session = build_document_session(
        &fresh_probe,
        &fresh_source,
        fresh_source.clone(),
        fresh_index,
        false,
    )
    .expect("fresh paged session");
    assert_eq!(
        session.store.kind(),
        gmark_document_core::DocumentBackendKind::Paged
    );
}

#[test]
fn same_length_replacement_invalidates_a_stale_probe() {
    let temp = tempfile::tempdir().expect("same-length probe race tempdir");
    let path = temp.path().join("same-length.json");
    fs::write(&path, br#"{"a":1}"#).expect("initial same-length fixture");
    let options = gmark_paged_document::ProbeOptions::default();
    let stale_probe = gmark_paged_document::probe_file(&path, options).expect("initial probe");
    let stale_source = FileSource::open(&path).expect("initial stable source");

    let replacement = temp.path().join("same-length.replacement");
    fs::write(&replacement, br#"{"b":2}"#).expect("replacement fixture");
    fs::remove_file(&path).expect("remove original fixture");
    fs::rename(&replacement, &path).expect("install same-length replacement");
    assert_eq!(
        fs::metadata(&path).expect("replacement metadata").len(),
        stale_probe.len
    );

    let replacement_source = FileSource::open(&path).expect("replacement source");
    let replacement_index = LineIndex::build(&replacement_source).expect("replacement index");
    assert_ne!(
        replacement_source.identity().expect("replacement identity"),
        stale_probe.identity,
        "identity must detect a replacement even when byte length is unchanged"
    );
    assert!(matches!(
        build_document_session(
            &stale_probe,
            &stale_source,
            replacement_source,
            replacement_index,
            false,
        ),
        Err(PagedDocumentError::SourceChanged)
    ));
}

#[test]
fn resident_host_session_keeps_probe_limits_and_only_marks_runtime_growth() {
    let temp = tempfile::tempdir().expect("resident growth tempdir");
    let path = temp.path().join("growth.csv");
    fs::write(&path, "a,b\n").expect("resident growth fixture");
    let options = gmark_paged_document::ProbeOptions {
        max_resident_bytes: 5,
        ..gmark_paged_document::ProbeOptions::default()
    };
    let probe = gmark_paged_document::probe_file(&path, options).expect("resident growth probe");
    assert_eq!(probe.strategy, OpenStrategy::Resident);
    let source = FileSource::open(&path).expect("resident growth source");
    let index = LineIndex::build(&source).expect("resident growth index");
    let mut session = build_document_session(&probe, &source, source.clone(), index, false)
        .expect("resident growth session");

    assert_eq!(session.loading_limits.max_resident_bytes, 5);
    session
        .replace_text(4..4, "12")
        .expect("grow resident source");
    assert_eq!(
        session.resident_growth_reason(),
        Some(gmark_document_core::OpenReason::ByteLimitExceeded)
    );
    assert_eq!(
        session.store.kind(),
        gmark_document_core::DocumentBackendKind::Resident
    );
}

#[test]
fn resident_recovery_contract_records_the_resulting_undo_snapshot() {
    let temp = tempfile::tempdir().expect("resident recovery tempdir");
    let path = temp.path().join("recovery.csv");
    fs::write(&path, "one").expect("resident recovery fixture");
    let source = FileSource::open(&path).expect("resident recovery source");
    let probe =
        gmark_paged_document::probe_file(&path, gmark_paged_document::ProbeOptions::default())
            .expect("resident recovery probe");
    assert_eq!(probe.strategy, OpenStrategy::Resident);
    let index = LineIndex::build(&source).expect("resident recovery index");
    let mut document = build_document_session(&probe, &source, source.clone(), index, false)
        .expect("resident recovery session");
    let mut journal =
        DocumentRecoveryJournal::create(temp.path(), &source, probe.encoding.clone(), &document)
            .expect("resident recovery journal");
    assert!(matches!(&journal, DocumentRecoveryJournal::Resident(_)));

    document
        .replace_text(0..3, "two")
        .expect("replace resident source");
    journal
        .record_after_change(
            &document,
            &RecoveryRecord {
                action: RecoveryAction::Transaction(Transaction::new(
                    gmark_document_core::DocumentRevision(0),
                    vec![SourceEdit::new(0..3, "two")],
                )),
                selection: Some(SourceSelection::collapsed(3, SourceAffinity::After)),
                view_id: DocumentViewId::source(),
            },
        )
        .expect("record resident replacement");
    assert!(document.undo(), "resident edit must be undoable");
    journal
        .record_after_change(
            &document,
            &RecoveryRecord {
                action: RecoveryAction::Undo,
                selection: Some(SourceSelection::collapsed(0, SourceAffinity::Before)),
                view_id: DocumentViewId::source(),
            },
        )
        .expect("record resident undo snapshot");

    let recovered = gmark_document_runtime::replay_resident_recovery_journal(journal.path())
        .expect("replay resident runtime journal");
    assert_eq!(recovered.source, "one");
    assert_eq!(
        recovered.selection,
        SourceSelection::collapsed(0, SourceAffinity::Before)
    );
}

#[test]
fn paged_recovery_contract_keeps_the_large_journal_path() {
    let temp = tempfile::tempdir().expect("paged recovery tempdir");
    let path = temp.path().join("paged.txt");
    fs::write(&path, "paged source").expect("paged recovery fixture");
    let source = FileSource::open(&path).expect("paged recovery source");
    let mut probe =
        gmark_paged_document::probe_file(&path, gmark_paged_document::ProbeOptions::default())
            .expect("paged recovery probe");
    probe.strategy = OpenStrategy::Paged;
    let identity = source.identity().expect("paged recovery identity");
    let index = LineIndex::build(&source).expect("paged recovery index");
    let piece = PieceDocument::open(source.clone(), index).expect("paged recovery document");
    let document = build_paged_session(&probe, piece, identity).expect("paged recovery session");
    let journal =
        DocumentRecoveryJournal::create(temp.path(), &source, probe.encoding.clone(), &document)
            .expect("paged recovery journal");

    assert!(matches!(&journal, DocumentRecoveryJournal::Paged(_)));
    assert_eq!(
        journal
            .path()
            .extension()
            .and_then(|extension| extension.to_str()),
        Some("large-journal")
    );
}

/// Keep the pre-install handoff Resident-only so Paged undo/redo commands can
/// never be silently coalesced while their ordered worker is unavailable.
#[test]
fn pending_recovery_does_not_coalesce_paged_commands() {
    let temp = tempfile::tempdir().expect("paged pending recovery tempdir");
    let path = temp.path().join("pending-paged.txt");
    fs::write(&path, "paged source").expect("paged pending recovery fixture");
    let source = FileSource::open(&path).expect("paged pending recovery source");
    let mut probe =
        gmark_paged_document::probe_file(&path, gmark_paged_document::ProbeOptions::default())
            .expect("paged pending recovery probe");
    probe.strategy = OpenStrategy::Paged;
    let identity = source.identity().expect("paged pending recovery identity");
    let index = LineIndex::build(&source).expect("paged pending recovery index");
    let piece = PieceDocument::open(source, index).expect("paged pending recovery document");
    let document =
        build_paged_session(&probe, piece, identity).expect("paged pending recovery session");
    let snapshot = document.save_snapshot();
    assert!(snapshot.source_format.is_none());

    let mut coordinator = DocumentCoordinator::new(SearchCancellation::default());
    assert!(!coordinator.stage_pending_recovery(
        snapshot,
        RecoveryRecord {
            action: RecoveryAction::Undo,
            selection: None,
            view_id: DocumentViewId::source(),
        },
    ));
    assert!(coordinator.pending_recovery_record.is_none());
}
