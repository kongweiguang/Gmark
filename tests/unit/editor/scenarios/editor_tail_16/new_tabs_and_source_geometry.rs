// @author kongweiguang

#[gpui::test]
async fn new_tab_button_offers_untyped_markdown_json_and_csv_document_types(
    cx: &mut TestAppContext,
) {
    init_editor_test_app(cx);
    let (_editor, visual) =
        cx.add_window_view(|_window, cx| Editor::from_markdown(cx, String::new(), None));
    visual.simulate_resize(size(px(900.0), px(620.0)));
    redraw(visual);

    let add = visual
        .debug_bounds("document-new-tab")
        .expect("new tab button");
    visual.simulate_click(add.center(), Modifiers::default());
    redraw(visual);

    assert!(visual.debug_bounds("new-tab-type-menu").is_some());
    assert!(visual.debug_bounds("new-tab-untyped").is_some());
    assert!(visual.debug_bounds("new-tab-markdown").is_some());
    assert!(visual.debug_bounds("new-tab-json").is_some());
    assert!(visual.debug_bounds("new-tab-csv").is_some());
}

#[gpui::test]
async fn json_new_tab_choice_creates_an_unsaved_document_without_a_path(
    cx: &mut TestAppContext,
) {
    init_editor_test_app(cx);
    let (editor, visual) =
        cx.add_window_view(|_window, cx| Editor::from_markdown(cx, String::new(), None));
    visual.simulate_resize(size(px(900.0), px(620.0)));
    redraw(visual);

    let add = visual.debug_bounds("document-new-tab").unwrap();
    visual.simulate_click(add.center(), Modifiers::default());
    redraw(visual);
    let json = visual.debug_bounds("new-tab-json").unwrap();
    visual.simulate_click(json.center(), Modifiers::default());

    editor.update(visual, |editor, cx| {
        assert!(editor.file_path.is_none());
        assert_eq!(editor.source_document.text(), "{\n}\n");
        assert_eq!(editor.document_kind, super::DocumentKind::Json);
        assert_eq!(editor.view_mode, ViewMode::Source);
        assert_eq!(editor.document_kind.icon(), "icon/ui/code.svg");
        assert_eq!(editor.save_dialog_defaults().1.as_deref(), Some("Untitled.json"));
        assert!(editor.switch_to_tab_index(0, cx));
        assert_eq!(editor.document_kind, super::DocumentKind::Markdown);
        assert!(editor.switch_to_tab_index(1, cx));
        assert_eq!(editor.document_kind, super::DocumentKind::Json);
    });
}

#[gpui::test]
async fn csv_new_tab_choice_creates_an_unsaved_document_without_a_path(
    cx: &mut TestAppContext,
) {
    init_editor_test_app(cx);
    let (editor, visual) =
        cx.add_window_view(|_window, cx| Editor::from_markdown(cx, String::new(), None));
    visual.simulate_resize(size(px(900.0), px(620.0)));
    redraw(visual);

    let add = visual.debug_bounds("document-new-tab").unwrap();
    visual.simulate_click(add.center(), Modifiers::default());
    redraw(visual);
    let csv = visual.debug_bounds("new-tab-csv").unwrap();
    visual.simulate_click(csv.center(), Modifiers::default());

    editor.update(visual, |editor, _cx| {
        assert!(editor.file_path.is_none());
        assert_eq!(editor.source_document.text(), "Column 1,Column 2\n");
        assert_eq!(editor.document_kind, super::DocumentKind::Csv);
        assert_eq!(editor.view_mode, ViewMode::Source);
        assert_eq!(editor.document_kind.icon(), "icon/ui/table.svg");
        assert_eq!(editor.save_dialog_defaults().1.as_deref(), Some("Untitled.csv"));
    });
}

#[test]
fn new_document_kind_controls_only_missing_save_extensions() {
    let mut untyped = PathBuf::from("Untitled");
    super::DocumentKind::Unspecified.apply_default_extension(&mut untyped);
    assert_eq!(untyped, PathBuf::from("Untitled"));

    let mut json = PathBuf::from("Untitled");
    super::DocumentKind::Json.apply_default_extension(&mut json);
    assert_eq!(json, PathBuf::from("Untitled.json"));

    let mut csv = PathBuf::from("report");
    super::DocumentKind::Csv.apply_default_extension(&mut csv);
    assert_eq!(csv, PathBuf::from("report.csv"));

    let mut explicit = PathBuf::from("report.txt");
    super::DocumentKind::Csv.apply_default_extension(&mut explicit);
    assert_eq!(explicit, PathBuf::from("report.txt"));
}

/// 分页源码在窄窗口为正文保留空间，跳过行号位数变化时也保持正文列对齐。
#[gpui::test]
async fn million_line_source_jump_keeps_local_scroll_geometry_exact(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let temp = tempfile::tempdir().expect("million-line Source tempdir");
    let path = temp.path().join("million-lines.md");
    let mut text = String::from("# Heading\n");
    text.push_str(&"x\n".repeat(999_998));
    text.push('x');
    fs::write(&path, text).expect("million-line Source fixture");
    let probe = gmark_paged_document::probe_file(
        &path,
        gmark_paged_document::ProbeOptions {
            max_resident_bytes: 1,
            ..gmark_paged_document::ProbeOptions::default()
        },
    )
    .expect("million-line Source probe");
    let source = gmark_paged_document::FileSource::open(&path).expect("million-line Source");
    let (editor, visual) = cx.add_window_view(move |_window, cx| {
        Editor::from_source_backed_file(cx, path, probe, source)
    });
    visual.simulate_resize(size(px(720.0), px(520.0)));
    visual.run_until_parked();
    redraw(visual);
    let surface = visual
        .debug_bounds("document-host-source-horizontal-scroll")
        .expect("Source surface");
    let first = visual
        .debug_bounds("document-host-line-body-0")
        .expect("first global Source line");
    assert!(
        first.left() - surface.left() <= px(100.0),
        "the seven-digit gutter must leave space for the text: {first:?} in {surface:?}"
    );
    let large_view = editor
        .read_with(visual, |editor, _cx| editor.document_host.clone())
        .expect("million-line large view");

    visual.update(|window, cx| {
        large_view.update(cx, |view, cx| view.jump_bottom_for_test(window, cx));
    });
    visual.run_until_parked();
    redraw(visual);

    let (origin, window_len, total_lines) =
        large_view.read_with(visual, |view, _cx| view.source_list_window_for_test());
    assert!(
        origin > 0,
        "a million lines must use a non-zero local origin: total={total_lines}, window={window_len}"
    );
    assert_eq!(window_len, crate::document_host::SOURCE_LIST_WINDOW_ROWS);
    let last = visual
        .debug_bounds("document-host-line-body-999999")
        .expect("last global Source line");
    let previous = visual
        .debug_bounds("document-host-line-body-999998")
        .expect("previous global Source line");
    assert_eq!(last.left(), first.left());
    assert_eq!(previous.left(), last.left());
    let row_height = large_view.read_with(visual, |view, _cx| view.source_row_height_for_test());
    assert!(
        (f32::from(last.top() - previous.top()) - row_height).abs() < 0.5,
        "local scroll window must not quantize or overlap rows at the global file tail"
    );
}
