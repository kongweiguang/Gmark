// @author kongweiguang

/// 无障碍正文必须同步当前输入表面；候选只进入视图快照，不能污染权威正文。
#[cfg(target_os = "windows")]
#[gpui::test]
async fn large_source_accessibility_tracks_preedit_and_cancellation(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let temp = tempfile::tempdir().expect("Source accessibility tempdir");
    let path = temp.path().join("source-accessibility.txt");
    let original = "alpha\nbeta\n";
    fs::write(&path, original).expect("Source accessibility fixture");
    let probe = gmark_paged_document::probe_file(
        &path,
        gmark_paged_document::ProbeOptions {
            max_resident_bytes: 1,
            ..gmark_paged_document::ProbeOptions::default()
        },
    )
    .expect("Source accessibility probe");
    let source = gmark_paged_document::FileSource::open(&path).expect("Source accessibility file");
    let (editor, visual) = cx.add_window_view(move |_window, cx| {
        Editor::from_source_backed_file(cx, path, probe, source)
    });
    visual.run_until_parked();
    redraw(visual);
    let host = editor
        .read_with(visual, |editor, _cx| editor.document_host.clone())
        .expect("Source accessibility Host");
    visual.update(|window, cx| {
        host.update(cx, |view, cx| view.begin_line_edit_for_test(1, window, cx));
    });
    redraw(visual);
    let owner = host
        .read_with(visual, |view, _cx| view.active_edit_for_test())
        .expect("Source accessibility input owner")
        .1;
    let before = editor.read_with(visual, |editor, cx| {
        editor.current_accessibility_revision(cx)
    });
    visual.update(|window, cx| {
        owner.update(cx, |block, block_cx| {
            <crate::components::Block as gpui::EntityInputHandler>::replace_and_mark_text_in_range(
                block,
                None,
                "拼音",
                Some(2..2),
                window,
                block_cx,
            );
        });
    });
    let preedit = host.read_with(visual, |view, cx| view.accessibility_snapshot(cx));
    assert_eq!(
        preedit
            .lines
            .iter()
            .find(|(line, _)| *line == 1)
            .map(|(_, text)| text.as_str()),
        Some("beta拼音")
    );
    assert_ne!(
        editor.read_with(visual, |editor, cx| editor
            .current_accessibility_revision(cx)),
        before
    );
    assert_eq!(
        host.read_with(visual, |view, _cx| view.source_text_for_test()),
        original
    );
    visual.update(|window, cx| {
        owner.update(cx, |block, block_cx| {
            <crate::components::Block as gpui::EntityInputHandler>::composition_ended(
                block,
                gpui::CompositionEnd::Cancelled,
                window,
                block_cx,
            );
        });
    });
    let cancelled = host.read_with(visual, |view, cx| view.accessibility_snapshot(cx));
    assert_eq!(
        cancelled
            .lines
            .iter()
            .find(|(line, _)| *line == 1)
            .map(|(_, text)| text.as_str()),
        Some("beta")
    );
    assert_eq!(
        host.read_with(visual, |view, _cx| view.source_text_for_test()),
        original
    );
}
