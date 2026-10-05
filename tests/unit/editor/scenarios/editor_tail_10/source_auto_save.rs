// @author kongweiguang

use super::*;

/// Waits for the real Source save path to atomically publish the expected body, including its background I/O.
fn wait_for_paged_source_text(
    path: &std::path::Path,
    expected: &str,
    visual: &mut gpui::VisualTestContext,
) {
    for _ in 0..5_000 {
        visual.run_until_parked();
        if fs::read_to_string(path).is_ok_and(|saved| saved == expected) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    let actual = fs::read_to_string(path).ok();
    panic!("Paged Source auto-save did not persist the expected body: {actual:?}");
}

/// Verifies an actual Paged Source keyboard edit enters the existing AfterDelay persistence path.
#[gpui::test]
async fn paged_source_keyboard_edit_is_auto_saved_after_idle(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    cx.update(|cx| {
        crate::config::EditorSettings::init(
            cx,
            true,
            crate::config::AutoSavePreference::AfterDelay,
            true,
        );
    });

    let temp = tempfile::tempdir().expect("Paged Source auto-save tempdir");
    let path = temp.path().join("paged-source-auto-save.txt");
    let original = (0..40)
        .map(|line| format!("source row {line:03}\n"))
        .collect::<String>();
    fs::write(&path, &original).expect("Paged Source auto-save fixture");
    let probe = gmark_paged_document::probe_file(
        &path,
        gmark_paged_document::ProbeOptions {
            max_resident_bytes: 1,
            ..gmark_paged_document::ProbeOptions::default()
        },
    )
    .expect("Paged Source auto-save probe");
    assert_eq!(probe.strategy, gmark_paged_document::OpenStrategy::Paged);
    let source =
        gmark_paged_document::FileSource::open(&path).expect("Paged Source auto-save source");
    let task_path = path.clone();
    let (editor, visual) = cx.add_window_view(move |_window, cx| {
        Editor::from_source_backed_file(cx, task_path, probe, source)
    });
    visual.simulate_resize(size(px(760.0), px(520.0)));
    visual.run_until_parked();
    redraw(visual);
    let host = editor
        .read_with(visual, |editor, _cx| editor.document_host.clone())
        .expect("Paged Source auto-save Host");

    let row = visual
        .debug_bounds("document-host-line-body-0")
        .expect("first Paged Source row");
    let target = point(row.left() + px(45.0), row.center().y);
    visual.simulate_mouse_down(target, MouseButton::Left, Modifiers::default());
    visual.simulate_mouse_move(target, MouseButton::Left, Modifiers::default());
    visual.simulate_mouse_up(target, MouseButton::Left, Modifiers::default());
    visual.run_until_parked();
    visual.simulate_input(" auto-saved");
    visual.run_until_parked();

    let edited = host.read_with(visual, |view, _cx| view.source_text_for_test());
    assert_ne!(edited, original);
    assert!(host.read_with(visual, |view, _cx| view.is_dirty()));
    assert_eq!(
        fs::read_to_string(&path).expect("Source file before auto-save deadline"),
        original,
        "the edit must be dirty before the idle deadline"
    );

    visual
        .executor()
        .advance_clock(std::time::Duration::from_secs(1));
    visual.run_until_parked();
    wait_for_paged_source_text(&path, &edited, visual);

    assert!(!host.read_with(visual, |view, _cx| view.is_dirty()));
}

/// 恢复内容先保留给用户确认；显式保存建立新磁盘基线后，后续编辑才启用延迟写盘。
#[gpui::test]
async fn paged_recovery_requires_explicit_save_before_delayed_auto_save(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    cx.update(|cx| {
        crate::config::EditorSettings::init(
            cx,
            true,
            crate::config::AutoSavePreference::AfterDelay,
            true,
        );
    });

    let temp = tempfile::tempdir().expect("Paged recovery auto-save tempdir");
    let path = temp.path().join("paged-recovery-auto-save.txt");
    let original = "alpha\nbeta\n";
    let recovered = "ALPHA\nbeta\n";
    fs::write(&path, original).expect("Paged recovery auto-save fixture");
    let source =
        gmark_paged_document::FileSource::open(&path).expect("Paged recovery auto-save source");
    let mut journal = gmark_paged_document::PagedRecoveryJournal::create(
        temp.path().join("recovery"),
        &source,
        gmark_paged_document::TextEncoding::Utf8 { bom: false },
    )
    .expect("Paged recovery auto-save journal");
    journal
        .record_replace(0..5, "ALPHA", None, "source")
        .expect("Paged recovery auto-save edit");
    let journal_path = journal.path().to_path_buf();
    let probe = gmark_paged_document::probe_file(
        &path,
        gmark_paged_document::ProbeOptions {
            max_resident_bytes: 1,
            ..gmark_paged_document::ProbeOptions::default()
        },
    )
    .expect("Paged recovery auto-save probe");
    assert_eq!(probe.strategy, gmark_paged_document::OpenStrategy::Paged);
    let task_path = path.clone();
    let (editor, visual) = cx.add_window_view(move |_window, cx| {
        Editor::from_paged_recovery(cx, task_path, probe, source, journal_path)
    });
    visual.simulate_resize(size(px(760.0), px(520.0)));
    visual.run_until_parked();
    redraw(visual);
    let host = editor
        .read_with(visual, |editor, _cx| editor.document_host.clone())
        .expect("Paged recovery auto-save Host");

    let mut restored = false;
    for _ in 0..5_000 {
        visual.run_until_parked();
        restored = host.read_with(visual, |view, _cx| {
            view.is_dirty()
                && view.recovered_text_for_test().as_deref() == Some(recovered.as_bytes())
        });
        if restored {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert!(
        restored,
        "Paged recovery must install its dirty recovered body"
    );

    visual
        .executor()
        .advance_clock(std::time::Duration::from_millis(1_100));
    visual.run_until_parked();
    assert_eq!(
        fs::read_to_string(&path).expect("file before explicit recovered save"),
        original,
        "delayed auto-save must not replace the original file with recovered content"
    );

    visual.update(|window, cx| {
        host.update(cx, |view, cx| {
            view.on_save_document(&SaveDocument, window, cx);
        });
    });
    wait_for_paged_source_text(&path, recovered, visual);
    for _ in 0..5_000 {
        visual.run_until_parked();
        if !host.read_with(visual, |view, _cx| view.is_dirty()) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert!(!host.read_with(visual, |view, _cx| view.is_dirty()));

    let row = visual
        .debug_bounds("document-host-line-body-0")
        .expect("first recovered Paged Source row");
    let target = point(row.left() + px(45.0), row.center().y);
    visual.simulate_mouse_down(target, MouseButton::Left, Modifiers::default());
    visual.simulate_mouse_move(target, MouseButton::Left, Modifiers::default());
    visual.simulate_mouse_up(target, MouseButton::Left, Modifiers::default());
    visual.run_until_parked();
    visual.simulate_input(" after-save");
    visual.run_until_parked();
    let edited = host.read_with(visual, |view, _cx| view.source_text_for_test());
    assert!(edited.contains("after-save"));
    assert!(host.read_with(visual, |view, _cx| view.is_dirty()));

    visual
        .executor()
        .advance_clock(std::time::Duration::from_secs(1));
    visual.run_until_parked();
    wait_for_paged_source_text(&path, &edited, visual);
    assert!(!host.read_with(visual, |view, _cx| view.is_dirty()));
}

/// Ensures a pending Windows candidate pauses the dirty snapshot, cancellation resumes it, and commit restarts it.
#[cfg(target_os = "windows")]
#[gpui::test]
async fn paged_source_auto_save_waits_for_native_composition_end(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    cx.update(|cx| {
        crate::config::EditorSettings::init(
            cx,
            true,
            crate::config::AutoSavePreference::AfterDelay,
            true,
        );
    });

    let temp = tempfile::tempdir().expect("Paged Source IME auto-save tempdir");
    let path = temp.path().join("paged-source-ime-auto-save.txt");
    let original = (0..40)
        .map(|line| format!("source row {line:03}\n"))
        .collect::<String>();
    fs::write(&path, &original).expect("Paged Source IME auto-save fixture");
    let probe = gmark_paged_document::probe_file(
        &path,
        gmark_paged_document::ProbeOptions {
            max_resident_bytes: 1,
            ..gmark_paged_document::ProbeOptions::default()
        },
    )
    .expect("Paged Source IME auto-save probe");
    assert_eq!(probe.strategy, gmark_paged_document::OpenStrategy::Paged);
    let source =
        gmark_paged_document::FileSource::open(&path).expect("Paged Source IME auto-save source");
    let task_path = path.clone();
    let (editor, visual) = cx.add_window_view(move |_window, cx| {
        Editor::from_source_backed_file(cx, task_path, probe, source)
    });
    visual.simulate_resize(size(px(760.0), px(520.0)));
    visual.run_until_parked();
    redraw(visual);
    let host = editor
        .read_with(visual, |editor, _cx| editor.document_host.clone())
        .expect("Paged Source IME auto-save Host");

    let row_bounds = visual
        .debug_bounds("document-host-line-body-0")
        .expect("first Paged Source IME row");
    let target = point(row_bounds.left() + px(45.0), row_bounds.center().y);
    visual.simulate_mouse_down(target, MouseButton::Left, Modifiers::default());
    visual.simulate_mouse_move(target, MouseButton::Left, Modifiers::default());
    visual.simulate_mouse_up(target, MouseButton::Left, Modifiers::default());
    visual.run_until_parked();
    visual.simulate_input(" 已确认");
    visual.run_until_parked();
    let confirmed_text = host.read_with(visual, |view, _cx| view.source_text_for_test());
    assert!(host.read_with(visual, |view, _cx| view.is_dirty()));

    let (_, input) = host
        .read_with(visual, |view, _cx| view.active_edit_for_test())
        .expect("focused Paged Source row");
    let cancelled_candidate = "拼音候选";
    let candidate_end = cancelled_candidate.encode_utf16().count();
    visual.update(|window, cx| {
        input.update(cx, |block, block_cx| {
            <crate::components::Block as gpui::EntityInputHandler>::replace_and_mark_text_in_range(
                block,
                None,
                cancelled_candidate,
                Some(candidate_end..candidate_end),
                window,
                block_cx,
            );
        });
    });
    visual.run_until_parked();
    input.read_with(visual, |block, _cx| assert!(block.has_ime_composition()));

    visual
        .executor()
        .advance_clock(std::time::Duration::from_millis(1_100));
    visual.run_until_parked();
    input.read_with(visual, |block, _cx| assert!(block.has_ime_composition()));
    assert_eq!(
        host.read_with(visual, |view, _cx| view.source_text_for_test()),
        confirmed_text,
        "an unconfirmed native candidate must not enter the Source body"
    );
    assert_eq!(
        fs::read_to_string(&path).expect("file while candidate is pending"),
        original,
        "the dirty confirmed text must not auto-save while the IME candidate is unresolved"
    );

    visual.update(|window, cx| {
        input.update(cx, |block, block_cx| {
            <crate::components::Block as gpui::EntityInputHandler>::composition_ended(
                block,
                gpui::CompositionEnd::Cancelled,
                window,
                block_cx,
            );
        });
    });
    visual.run_until_parked();
    assert_eq!(
        host.read_with(visual, |view, _cx| view.source_text_for_test()),
        confirmed_text,
        "cancelling preedit must retain the earlier confirmed dirty edit"
    );
    assert!(host.read_with(visual, |view, _cx| view.is_dirty()));
    visual
        .executor()
        .advance_clock(std::time::Duration::from_secs(1));
    visual.run_until_parked();
    wait_for_paged_source_text(&path, &confirmed_text, visual);
    assert!(!host.read_with(visual, |view, _cx| view.is_dirty()));

    let committed_candidate = "新词";
    let candidate_end = committed_candidate.encode_utf16().count();
    visual.update(|window, cx| {
        input.update(cx, |block, block_cx| {
            <crate::components::Block as gpui::EntityInputHandler>::replace_and_mark_text_in_range(
                block,
                None,
                committed_candidate,
                Some(candidate_end..candidate_end),
                window,
                block_cx,
            );
        });
    });
    visual.run_until_parked();
    visual
        .executor()
        .advance_clock(std::time::Duration::from_millis(1_100));
    visual.run_until_parked();
    assert_eq!(
        fs::read_to_string(&path).expect("file while new candidate is pending"),
        confirmed_text,
        "a later candidate must also remain outside the saved Source body"
    );

    visual.update(|window, cx| {
        input.update(cx, |block, block_cx| {
            <crate::components::Block as gpui::EntityInputHandler>::replace_text_in_range(
                block,
                None,
                committed_candidate,
                window,
                block_cx,
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
    let committed_text = host.read_with(visual, |view, _cx| view.source_text_for_test());
    assert!(committed_text.contains(committed_candidate));
    assert!(host.read_with(visual, |view, _cx| view.is_dirty()));

    visual
        .executor()
        .advance_clock(std::time::Duration::from_secs(1));
    visual.run_until_parked();
    wait_for_paged_source_text(&path, &committed_text, visual);
    assert!(!host.read_with(visual, |view, _cx| view.is_dirty()));
}
