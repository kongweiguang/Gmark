// @author kongweiguang

use std::fs;

use super::*;

#[path = "document_runtime/document_host_lifecycle.rs"]
mod document_host_lifecycle;
#[path = "document_runtime/source_boundary_requests.rs"]
mod source_boundary_requests;
#[path = "document_runtime/source_boundary_unicode.rs"]
mod source_boundary_unicode;
#[path = "document_runtime/source_horizontal_navigation.rs"]
mod source_horizontal_navigation;
#[path = "document_runtime/source_pointer_boundaries.rs"]
mod source_pointer_boundaries;

fn open_document(text: &str) -> (tempfile::TempDir, DocumentSession) {
    let temp = tempfile::tempdir().expect("bounded line tempdir");
    let path = temp.path().join("long-line.txt");
    fs::write(&path, text).expect("bounded line fixture");
    let source = FileSource::open(&path).expect("bounded line source");
    let identity = source.identity().expect("bounded line identity");
    let probe =
        gmark_paged_document::probe_file(&path, gmark_paged_document::ProbeOptions::default())
            .expect("bounded line probe");
    let index = LineIndex::build(&source).expect("bounded line index");
    let piece = PieceDocument::open(source, index).expect("bounded line document");
    let document = build_paged_session(&probe, piece, identity).expect("bounded line session");
    (temp, document)
}

#[test]
fn far_single_line_window_is_bounded_and_keeps_search_anchor_visible() {
    let target = MAX_RENDERED_LINE_BYTES + 12_345;
    let mut text = "a".repeat(target as usize);
    text.push_str("NEEDLE");
    text.push_str(&"z".repeat(MAX_RENDERED_LINE_BYTES as usize));
    text.push('\n');
    let (_temp, document) = open_document(&text);
    let start = source_window_start_for_anchor(text.len() as u64, target);
    let window = read_bounded_line_window(&document, 0, start)
        .expect("bounded read")
        .expect("first line");

    assert!(window.content_range.start > 0);
    assert!(window.content_range.end - window.content_range.start <= MAX_RENDERED_LINE_BYTES);
    assert!(window.content_range.contains(&target));
    assert!(window.text.contains("NEEDLE"));
    assert!(window.leading_truncated && window.trailing_truncated);
    let rendered = rendered_line_window_text(&window, false);
    assert!(rendered.starts_with("… ") && rendered.ends_with(" …"));
}

#[test]
fn source_list_window_keeps_f32_scroll_height_bounded_at_tens_of_millions_of_lines() {
    let total = 24_412_160;
    let last = total - 1;
    let origin = source_list_origin_for_target(total, last);
    assert_eq!(origin, total - SOURCE_LIST_WINDOW_ROWS);
    assert_eq!(last - origin, SOURCE_LIST_WINDOW_ROWS - 1);
    assert_eq!(source_list_origin_for_target(total, 0), 0);
    assert!(SOURCE_LIST_WINDOW_ROWS as f32 * FALLBACK_SOURCE_ROW_HEIGHT < 2_f32.powi(22));
}

#[test]
fn source_surface_uses_compact_horizontal_padding() {
    let theme = crate::theme::Theme::default_theme();

    assert_eq!(theme.dimensions.editor_padding, 24.0);
    assert_eq!(source_surface_padding(&theme.dimensions), (12.0, 24.0));
}

#[test]
fn global_scrollbar_pointer_uses_line_space_instead_of_giant_pixel_offsets() {
    let top = px(100.0);
    assert_eq!(
        source_line_from_scrollbar_pointer(px(100.0), top, 800.0, 28.0, 24_000_000),
        0
    );
    assert_eq!(
        source_line_from_scrollbar_pointer(px(900.0), top, 800.0, 28.0, 24_000_000),
        24_000_000
    );
}

#[test]
fn bounded_edit_replaces_only_the_visible_window_and_preserves_suffixes() {
    let target = MAX_RENDERED_LINE_BYTES + 4_096;
    let mut original = "p".repeat(target as usize);
    original.push_str("NEEDLE");
    original.push_str(&"s".repeat(MAX_RENDERED_LINE_BYTES as usize));
    let (_temp, mut document) = open_document(&original);
    let start = source_window_start_for_anchor(original.len() as u64, target);
    let window = read_bounded_line_window(&document, 0, start)
        .expect("bounded read")
        .expect("first line");
    let local = window.text.find("NEEDLE").expect("target in window");
    let mut replacement = window.text.to_string();
    replacement.replace_range(local..local + "NEEDLE".len(), "EDITED");
    replacement.push_str(&window.ending);

    document
        .replace_text(window.replace_range, &replacement)
        .expect("bounded edit");
    let mut expected = original;
    expected.replace_range(target as usize..target as usize + 6, "EDITED");
    assert_eq!(
        document.read_range(0..document.len()).expect("edited text"),
        expected.as_bytes()
    );
}

#[test]
fn save_readback_verifier_rejects_same_length_content_mismatch() {
    let (_temp, expected) = open_document("alpha\nbeta\n");
    let actual = expected.clone();
    assert!(
        verify_saved_session_readback(&expected, &actual, &SearchCancellation::default()).is_ok()
    );

    let mut mismatched = actual;
    mismatched.replace_text(0..5, "ALPHA").unwrap();
    assert!(matches!(
        verify_saved_session_readback(&expected, &mismatched, &SearchCancellation::default()),
        Err(PagedDocumentError::InvalidTransaction(_))
    ));
}

#[test]
fn recovery_cleanup_failure_keeps_resident_journal_retryable_while_replacement_records()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("resident-recovery.txt");
    fs::write(&path, "one")?;
    let source = FileSource::open(&path)?;
    let probe =
        gmark_paged_document::probe_file(&path, gmark_paged_document::ProbeOptions::default())?;
    let index = LineIndex::build(&source)?;
    let mut document = build_document_session(&probe, &source, source.clone(), index, false)?;
    let mut retired =
        DocumentRecoveryJournal::create(temp.path(), &source, probe.encoding.clone(), &document)?;
    document.replace_text(0..3, "two")?;
    retired.record_after_change(
        &document,
        &RecoveryRecord {
            action: RecoveryAction::Transaction(Transaction::new(
                gmark_document_core::DocumentRevision(0),
                vec![SourceEdit::new(0..3, "two")],
            )),
            selection: Some(SourceSelection::collapsed(3, SourceAffinity::After)),
            view_id: DocumentViewId::source(),
        },
    )?;
    let retired_path = retired.path().to_path_buf();
    let replacement =
        DocumentRecoveryJournal::create(temp.path(), &source, probe.encoding.clone(), &document)?;
    let replacement_path = replacement.path().to_path_buf();
    let mut coordinator = DocumentCoordinator::new(SearchCancellation::default());
    coordinator.recovery_journal = Some(retired);

    DocumentRecoveryJournal::fail_next_checkpoint_for_test();
    DocumentRecoveryJournal::fail_next_rename_for_test();
    assert!(
        coordinator
            .replace_recovery_journal_after_persistence(Some(replacement), &document)
            .is_err()
    );
    assert_eq!(coordinator.retired_recovery_journals.len(), 1);
    assert_eq!(
        coordinator
            .recovery_journal
            .as_ref()
            .ok_or_else(|| std::io::Error::other("replacement resident journal was not installed"))?
            .path(),
        replacement_path
    );

    let suppression_marker = retired_path.with_extension("journal.suppressed");
    assert!(suppression_marker.exists());
    assert!(retired_path.exists());

    document.replace_text(3..3, "!")?;
    coordinator
        .recovery_journal
        .as_mut()
        .ok_or_else(|| std::io::Error::other("replacement resident journal was lost"))?
        .record_after_change(
            &document,
            &RecoveryRecord {
                action: RecoveryAction::Transaction(Transaction::new(
                    gmark_document_core::DocumentRevision(1),
                    vec![SourceEdit::new(3..3, "!")],
                )),
                selection: Some(SourceSelection::collapsed(4, SourceAffinity::After)),
                view_id: DocumentViewId::source(),
            },
        )?;
    drop(coordinator);

    let recovered = gmark_document_runtime::load_resident_recovery_journals(temp.path())?;
    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].document.source, "two!");
    assert!(!suppression_marker.exists());
    assert!(!retired_path.exists());
    Ok(())
}

#[test]
fn recovery_cleanup_failure_keeps_paged_journal_retryable_while_replacement_records()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("paged-recovery.txt");
    fs::write(&path, "paged")?;
    let source = FileSource::open(&path)?;
    let mut probe =
        gmark_paged_document::probe_file(&path, gmark_paged_document::ProbeOptions::default())?;
    probe.strategy = OpenStrategy::Paged;
    let index = LineIndex::build(&source)?;
    let mut document = build_document_session(&probe, &source, source.clone(), index, false)?;
    let mut retired =
        DocumentRecoveryJournal::create(temp.path(), &source, probe.encoding.clone(), &document)?;
    document.replace_text(0..5, "saved")?;
    retired.record_after_change(
        &document,
        &RecoveryRecord {
            action: RecoveryAction::Transaction(Transaction::new(
                gmark_document_core::DocumentRevision(0),
                vec![SourceEdit::new(0..5, "saved")],
            )),
            selection: Some(SourceSelection::collapsed(5, SourceAffinity::After)),
            view_id: DocumentViewId::source(),
        },
    )?;
    let retired_path = retired.path().to_path_buf();
    let replacement =
        DocumentRecoveryJournal::create(temp.path(), &source, probe.encoding.clone(), &document)?;
    let replacement_path = replacement.path().to_path_buf();
    let mut coordinator = DocumentCoordinator::new(SearchCancellation::default());
    coordinator.recovery_journal = Some(retired);

    DocumentRecoveryJournal::fail_next_checkpoint_for_test();
    DocumentRecoveryJournal::fail_next_rename_for_test();
    assert!(
        coordinator
            .replace_recovery_journal_after_persistence(Some(replacement), &document)
            .is_err()
    );
    assert_eq!(coordinator.retired_recovery_journals.len(), 1);
    assert_eq!(
        coordinator
            .recovery_journal
            .as_ref()
            .ok_or_else(|| std::io::Error::other("replacement paged journal was not installed"))?
            .path(),
        replacement_path
    );

    let suppression_marker = retired_path.with_extension("large-journal.suppressed");
    assert!(suppression_marker.exists());
    assert!(retired_path.exists());

    document.replace_text(5..5, "!")?;
    coordinator
        .recovery_journal
        .as_mut()
        .ok_or_else(|| std::io::Error::other("replacement paged journal was lost"))?
        .record_after_change(
            &document,
            &RecoveryRecord {
                action: RecoveryAction::Transaction(Transaction::new(
                    gmark_document_core::DocumentRevision(1),
                    vec![SourceEdit::new(5..5, "!")],
                )),
                selection: Some(SourceSelection::collapsed(6, SourceAffinity::After)),
                view_id: DocumentViewId::source(),
            },
        )?;
    assert!(gmark_paged_document::paged_recovery_has_edits(
        &replacement_path
    )?);
    drop(coordinator);

    assert_eq!(
        gmark_paged_document::list_paged_recovery_journals(temp.path())?,
        vec![replacement_path]
    );
    assert!(!suppression_marker.exists());
    assert!(!retired_path.exists());
    Ok(())
}

#[test]
fn suppression_marker_creation_and_cleanup_retry_keep_the_original_before_marker()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("suppression-retry.txt");
    fs::write(&path, "saved")?;
    let source = FileSource::open(&path)?;
    let mut probe =
        gmark_paged_document::probe_file(&path, gmark_paged_document::ProbeOptions::default())?;
    probe.strategy = OpenStrategy::Paged;
    let index = LineIndex::build(&source)?;
    let document = build_document_session(&probe, &source, source.clone(), index, false)?;
    let retired =
        DocumentRecoveryJournal::create(temp.path(), &source, probe.encoding.clone(), &document)?;
    let retired_path = retired.path().to_path_buf();
    let replacement =
        DocumentRecoveryJournal::create(temp.path(), &source, probe.encoding.clone(), &document)?;
    let mut coordinator = DocumentCoordinator::new(SearchCancellation::default());
    coordinator.recovery_journal = Some(retired);

    DocumentRecoveryJournal::fail_next_checkpoint_for_test();
    DocumentRecoveryJournal::fail_next_rename_for_test();
    DocumentRecoveryJournal::fail_next_suppression_marker_for_test();
    assert!(matches!(
        coordinator.replace_recovery_journal_after_persistence(Some(replacement), &document),
        Err(PagedDocumentError::Io { .. })
    ));
    let suppression_marker = retired_path.with_extension("large-journal.suppressed");
    assert!(retired_path.exists());
    assert!(!suppression_marker.exists());

    DocumentRecoveryJournal::fail_next_remove_for_test();
    assert!(coordinator.retry_retired_recovery_journals().is_err());
    assert!(retired_path.exists());
    assert!(suppression_marker.exists());

    coordinator.retry_retired_recovery_journals()?;
    assert!(coordinator.retired_recovery_journals.is_empty());
    assert!(!retired_path.exists());
    assert!(!suppression_marker.exists());
    Ok(())
}

#[test]
fn bounded_window_never_starts_inside_utf8_codepoint() {
    let prefix = "a".repeat(MAX_RENDERED_LINE_BYTES as usize + 7);
    let unicode_start = prefix.len() as u64;
    let text = format!(
        "{prefix}中文{}",
        "b".repeat(MAX_RENDERED_LINE_BYTES as usize)
    );
    let (_temp, document) = open_document(&text);
    let window = read_bounded_line_window(&document, 0, unicode_start + 1)
        .expect("bounded read")
        .expect("first line");

    assert!(window.content_range.start >= unicode_start + '中'.len_utf8() as u64);
    assert!(std::str::from_utf8(window.text.as_bytes()).is_ok());
    assert!(window.text.len() <= MAX_RENDERED_LINE_BYTES as usize);
}

#[test]
fn unicode_crlf_windows_are_always_valid_edit_transactions() {
    let unit = "中😀e\u{301}かな";
    let first_line = unit.repeat(12_000);
    let text = format!("{first_line}\r\n尾行\r\n");
    let (_temp, document) = open_document(&text);
    let requested_starts = [
        0,
        1,
        2,
        3,
        MAX_RENDERED_LINE_BYTES.saturating_sub(3),
        MAX_RENDERED_LINE_BYTES,
        first_line.len().saturating_sub(1) as u64,
    ];

    for requested_start in requested_starts {
        let window = read_bounded_line_window(&document, 0, requested_start)
            .expect("Unicode bounded read")
            .expect("first Unicode line");
        assert!(std::str::from_utf8(window.text.as_bytes()).is_ok());
        let mut replacement = window.text.to_string();
        replacement.push('✓');
        replacement.push_str(&window.ending);
        let mut candidate = document.clone();
        candidate
            .replace_text(window.replace_range.clone(), &replacement)
            .unwrap_or_else(|error| {
                panic!(
                    "window {:?} from requested byte {requested_start} must be editable: {error}",
                    window.replace_range
                )
            });
    }
}

#[test]
fn provisional_utf16_viewport_decodes_before_shadow_index_is_ready() {
    let temp = tempfile::tempdir().expect("UTF-16 viewport tempdir");
    let path = temp.path().join("large-utf16.txt");
    let mut bytes = vec![0xff, 0xfe];
    for unit in "alpha\n世界\nomega\n".encode_utf16() {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    fs::write(&path, bytes).expect("UTF-16 fixture");
    let source = FileSource::open(&path).expect("UTF-16 source");
    let rows = read_provisional_source_rows(
        &source,
        3,
        0..3,
        0,
        &TextEncoding::Utf16Le,
        &SearchCancellation::default(),
    )
    .expect("provisional viewport");

    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].1.text.as_ref(), "alpha");
    assert!(rows.iter().all(|(_, row)| !row.text.contains('\0')));
    assert!(rows.iter().all(|(_, row)| !row.text.is_empty()));
}

#[test]
fn screen_lines_top_anchor_tracks_the_visible_source_row() {
    let mut rows = BTreeMap::new();
    rows.insert(
        42,
        Arc::new(BoundedLineWindow::new(
            12_345..12_350,
            12_345..12_351,
            "alpha".to_owned(),
            "\n".to_owned(),
            false,
            false,
        )),
    );
    let screen = ScreenLines {
        visible: 42..43,
        rows: Arc::new(rows),
        ..ScreenLines::default()
    };

    assert_eq!(
        screen.top_source_anchor(),
        Some(SourceAnchor::new(12_345, SourceAffinity::Before))
    );
}

/// provisional 与精确后端复用有效当前帧；测试不能用默认旧缓存代次绕过窗口身份校验。
#[gpui::test]
async fn provisional_source_row_is_read_only_until_piece_document_is_installed(
    cx: &mut gpui::TestAppContext,
) {
    let temp = tempfile::tempdir().expect("provisional Source tempdir");
    let path = temp.path().join("provisional.txt");
    fs::write(&path, "alpha\nbeta\n").expect("provisional Source fixture");
    let source = FileSource::open(&path).expect("provisional Source handle");
    let probe =
        gmark_paged_document::probe_file(&path, gmark_paged_document::ProbeOptions::default())
            .expect("provisional Source probe");
    let view = cx.new(|cx| DocumentHost::new(path.clone(), probe, source, cx));

    let provisional_block = view.update(cx, |view, cx| {
        view.suspend_for_closed_tab();
        view.displayed_screen_lines = Arc::new(ScreenLines {
            cache_epoch: view.source_cache_epoch,
            column_window_start: view.source_window_start,
            visible: 0..1,
            rows: Arc::new(BTreeMap::from([(
                0,
                Arc::new(BoundedLineWindow::new(
                    0..5,
                    0..6,
                    "alpha".to_owned(),
                    "\n".to_owned(),
                    false,
                    false,
                )),
            )])),
            ..ScreenLines::default()
        });
        view.ensure_source_row_block(0, cx)
            .expect("provisional Source block")
    });
    assert!(provisional_block.read_with(cx, |block, _cx| block.is_read_only()));

    let editable_block = view.update(cx, |view, cx| {
        let source = FileSource::open(&path).expect("exact Source handle");
        let identity = source.identity().expect("exact Source identity");
        let probe =
            gmark_paged_document::probe_file(&path, gmark_paged_document::ProbeOptions::default())
                .expect("exact Source probe");
        let index = LineIndex::build(&source).expect("exact Source index");
        let piece = PieceDocument::open(source, index).expect("exact PieceTree document");
        let document = build_paged_session(&probe, piece, identity).expect("exact Paged session");
        let controller = DocumentController::new(DocumentId::new(), document);
        view.document =
            Some(SharedDocument::from_controller(controller).expect("exact shared Paged session"));
        view.ensure_source_row_block(0, cx)
            .expect("editable Source block")
    });

    assert_eq!(
        provisional_block.entity_id(),
        editable_block.entity_id(),
        "the visible row entity must survive provisional-to-exact installation"
    );
    assert!(!editable_block.read_with(cx, |block, _cx| block.is_read_only()));
}

#[gpui::test]
async fn paged_workspace_restore_keeps_source_selection_and_vertical_position(
    cx: &mut gpui::TestAppContext,
) {
    let temp = tempfile::tempdir().expect("workspace restore tempdir");
    let path = temp.path().join("restore.txt");
    fs::write(&path, "alpha\nbeta\n").expect("workspace restore fixture");
    let source = FileSource::open(&path).expect("workspace restore source");
    let mut probe =
        gmark_paged_document::probe_file(&path, gmark_paged_document::ProbeOptions::default())
            .expect("workspace restore probe");
    probe.strategy = OpenStrategy::Paged;
    let host = cx.new(|cx| DocumentHost::new(path, probe, source, cx));

    host.update(cx, |host, cx| {
        host.suspend_for_closed_tab();
        host.restore_workspace_source_state(
            SourceSelection {
                anchor: SourceAnchor::new(2, SourceAffinity::Before),
                head: SourceAnchor::new(u64::MAX, SourceAffinity::After),
            },
            -42.0,
            cx,
        );
        let (selection, scroll_y) = host.workspace_source_state_for_test();
        assert_eq!(selection.anchor.byte_offset, 2);
        assert_eq!(selection.head.byte_offset, host.probe.len);
        assert_eq!(scroll_y, -42.0);
        let (persisted_selection, persisted_scroll) = host.workspace_source_state();
        assert_eq!(persisted_selection, selection);
        assert!(f32::from(persisted_scroll.y) < 0.0);
    });
}

#[gpui::test]
async fn paged_jsonl_never_registers_projection_while_resident_jsonl_does(
    cx: &mut gpui::TestAppContext,
) {
    let temp = tempfile::tempdir().expect("JSONL registry tempdir");
    let path = temp.path().join("records.jsonl");
    fs::write(&path, "{\"id\":1}\n").expect("JSONL registry fixture");
    let source = FileSource::open(&path).expect("JSONL registry source");
    let resident_probe =
        gmark_paged_document::probe_file(&path, gmark_paged_document::ProbeOptions::default())
            .expect("resident JSONL probe");
    let resident = cx.new(|cx| DocumentHost::new(path.clone(), resident_probe, source.clone(), cx));
    resident.update(cx, |host, _cx| {
        host.suspend_for_closed_tab();
        assert_eq!(
            host.view_registry
                .available(&DocumentFormat::JsonLines)
                .len(),
            1
        );
    });

    let mut paged_probe =
        gmark_paged_document::probe_file(&path, gmark_paged_document::ProbeOptions::default())
            .expect("paged JSONL probe");
    paged_probe.strategy = OpenStrategy::Paged;
    let paged = cx.new(|cx| DocumentHost::new(path, paged_probe, source, cx));
    paged.update(cx, |host, _cx| {
        host.suspend_for_closed_tab();
        assert!(
            host.view_registry
                .available(&DocumentFormat::JsonLines)
                .is_empty()
        );
        assert!(host.selected_projection_view.is_none());
    });
}

#[test]
fn logical_horizontal_offsets_clamp_at_both_edges() {
    assert_eq!(shift_source_window_start(10, -50, 1_000), 0);
    assert_eq!(shift_source_window_start(900, 500, 1_000), 1_000);
    assert_eq!(source_window_start_for_anchor(10_000, 9_000), 0);
    let long = MAX_RENDERED_LINE_BYTES * 4;
    let anchored = source_window_start_for_anchor(long, long - 10);
    assert!(anchored <= long - MAX_RENDERED_LINE_BYTES);
    assert!(long - 10 >= anchored && long - 10 <= anchored + MAX_RENDERED_LINE_BYTES);
}

#[test]
fn structured_wheel_only_maps_vertical_delta_with_an_explicit_modifier() {
    assert_eq!(
        modifier_horizontal_wheel_delta(false, false, 0.0, -26.0),
        None
    );
    assert_eq!(
        modifier_horizontal_wheel_delta(true, false, 0.0, -26.0),
        Some(-26.0)
    );
    assert_eq!(
        modifier_horizontal_wheel_delta(false, true, 0.0, 26.0),
        Some(26.0)
    );
    assert_eq!(
        modifier_horizontal_wheel_delta(false, false, 18.0, 2.0),
        None
    );
}

#[test]
fn paged_strategy_disables_all_derived_views() {
    assert!(derived_views_enabled(OpenStrategy::Resident));
    assert!(!derived_views_enabled(OpenStrategy::Paged));
}

#[test]
fn bounded_row_reuses_plain_text_and_builds_line_endings_lazily() {
    let row = BoundedLineWindow::new(
        0..5,
        0..6,
        "alpha".to_owned(),
        "\n".to_owned(),
        false,
        false,
    );

    assert_eq!(row.text.as_ptr(), row.display.as_ptr());
    assert!(row.display_with_endings.get().is_none());
    assert_eq!(row.rendered(false).as_ref(), "alpha");
    assert!(row.display_with_endings.get().is_none());
    assert_eq!(row.rendered(true).as_ref(), "alpha␊");
    assert!(row.display_with_endings.get().is_some());
}

#[test]
fn screen_lines_retain_the_previous_frame_only_for_a_fully_disjoint_jump() {
    let rows = BTreeMap::from([
        (
            10,
            Arc::new(BoundedLineWindow::new(
                100..105,
                100..106,
                "alpha".to_owned(),
                "\n".to_owned(),
                false,
                false,
            )),
        ),
        (
            11,
            Arc::new(BoundedLineWindow::new(
                106..110,
                106..111,
                "beta".to_owned(),
                "\n".to_owned(),
                false,
                false,
            )),
        ),
    ]);
    let screen = ScreenLines {
        visible: 10..12,
        rows: Arc::new(rows),
        ..ScreenLines::default()
    };

    let requested = 100..102;
    let retain = screen.should_retain_previous_frame(&requested);
    assert!(retain);
    let retained = screen.retained_rows(false);
    let (display_line, row) = retained.first().expect("retained first old row");
    assert_eq!(*display_line, 10);
    assert_eq!(row.as_ref(), "alpha");

    let mut mixed = (*screen.rows).clone();
    mixed.insert(
        100,
        Arc::new(BoundedLineWindow::new(
            1_000..1_003,
            1_000..1_004,
            "new".to_owned(),
            "\n".to_owned(),
            false,
            false,
        )),
    );
    let mixed = ScreenLines {
        rows: Arc::new(mixed),
        ..screen
    };
    let retain = mixed.should_retain_previous_frame(&requested);
    assert!(!retain);
}

#[test]
fn built_in_derived_descriptors_publish_their_resource_limits() {
    assert!(RegisteredStructuredProvider::for_format(&DocumentFormat::Json).is_none());
    let json = JsonGraphProjectionProvider::new(Arc::new(Mutex::new(HashMap::new())));
    assert_eq!(
        json.descriptor().max_items,
        Some(DEFAULT_JSON_GRAPH_ITEM_LIMIT)
    );

    let delimited =
        RegisteredStructuredProvider::for_format(&DocumentFormat::Delimited { delimiter: b',' })
            .expect("delimited derived provider");
    assert_eq!(
        delimited.descriptor().max_items,
        Some(DEFAULT_DELIMITED_ROW_WINDOW * DEFAULT_DELIMITED_COLUMN_WINDOW)
    );

    let json_lines = RegisteredStructuredProvider::for_format(&DocumentFormat::JsonLines)
        .expect("resident JSONL structure provider");
    assert_eq!(json_lines.descriptor().id, DocumentViewId::json_structure());
    assert_eq!(
        json_lines.descriptor().max_items,
        Some(DEFAULT_JSON_GRAPH_ITEM_LIMIT)
    );
}

#[test]
fn structured_row_cache_keeps_the_requested_window_while_pruning_far_rows() {
    let mut rows = (0..32).map(|row| (row, row)).collect::<BTreeMap<_, _>>();

    prune_structured_row_cache(&mut rows, 24, 8);

    assert_eq!(rows.len(), 8);
    assert!(rows.contains_key(&24));
    let first = rows.first_key_value().map(|(row, _)| *row).unwrap();
    let last = rows.last_key_value().map(|(row, _)| *row).unwrap();
    assert!(24 - first <= 4);
    assert!(last - 24 <= 4);
}
