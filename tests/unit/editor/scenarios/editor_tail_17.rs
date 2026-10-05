// @author kongweiguang

include!("editor_tail_17/smooth_scroll.rs");

/// 合并是独立事务，撤销须精确回到已粘贴正文，而非撤销粘贴或重写其拼写。
#[gpui::test]
async fn table_fragment_merge_is_explicit_and_one_undo_step(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let source = "| A | B |\n| --- | --- |\n| 1 | 2 |\n\n";
    let (editor, visual) =
        cx.add_window_view(move |_window, cx| Editor::from_markdown(cx, source.to_owned(), None));
    let paragraph = editor.read_with(visual, |editor, _cx| {
        editor
            .document
            .visible_blocks()
            .last()
            .unwrap()
            .entity
            .clone()
    });

    editor.update(visual, |editor, cx| {
        editor.on_block_event(
            paragraph,
            &BlockEvent::RequestPasteMultiline {
                leading: InlineTextTree::plain(String::new()),
                lines: vec!["| 3 | 4 |".to_owned()],
                trailing: InlineTextTree::plain(String::new()),
                split_physical_lines: false,
            },
            cx,
        );
        assert!(editor.table_fragment_merge.is_some());
        assert!(editor.source_document.text().contains("| 3 | 4 |"));
        let pasted_source = editor.source_document.text();
        assert!(pasted_source.contains("| 1 | 2 |\n\n| 3 | 4 |"));
        assert!(editor.document.source_commit_error().is_none());
        editor.confirm_table_fragment_merge(0, cx);
        assert!(editor.table_fragment_merge.is_none());
        assert!(editor.document.source_commit_error().is_none(), "{:?}", editor.document.source_commit_error());
        assert_eq!(
            editor.source_document.text(),
            "| A | B |\n| --- | --- |\n| 1 | 2 |\n| 3 | 4 |"
        );
        editor.undo_document(cx);
        assert_eq!(editor.source_document.text(), pasted_source);
        assert_eq!(
            editor.document.visible_blocks()[0].entity.read(cx).kind(),
            BlockKind::Table
        );
        editor.redo_document(cx);
        assert_eq!(editor.source_document.text(), "| A | B |\n| --- | --- |\n| 1 | 2 |\n| 3 | 4 |");
        assert!(editor.document.source_commit_error().is_none());
    });
}

#[gpui::test]
async fn stale_table_fragment_candidate_preserves_pasted_rows(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let source = "| A | B |\n| --- | --- |\n\n";
    let (editor, visual) =
        cx.add_window_view(move |_window, cx| Editor::from_markdown(cx, source.to_owned(), None));
    let paragraph = editor.read_with(visual, |editor, _cx| {
        editor
            .document
            .visible_blocks()
            .last()
            .unwrap()
            .entity
            .clone()
    });
    editor.update(visual, |editor, cx| {
        editor.on_block_event(
            paragraph,
            &BlockEvent::RequestPasteMultiline {
                leading: InlineTextTree::plain(String::new()),
                lines: vec!["| x | y |".to_owned()],
                trailing: InlineTextTree::plain(String::new()),
                split_physical_lines: false,
            },
            cx,
        );
        let fragment = editor.document.visible_blocks()[1].entity.clone();
        fragment.update(cx, |block, cx| {
            let end = block.display_text().len();
            block.prepare_undo_capture(crate::components::UndoCaptureKind::NonCoalescible, cx);
            block.replace_text_in_visible_range(end..end, "!", None, false, cx);
        });
    });
    visual.run_until_parked();
    editor.update(visual, |editor, cx| {
        editor.confirm_table_fragment_merge(0, cx);
        assert!(editor.table_fragment_merge.is_none());
        assert!(editor.source_document.text().contains("| x | y |!"));
    });
}

#[gpui::test]
async fn workspace_link_completion_writes_standard_relative_markdown_and_undoes_once(
    cx: &mut TestAppContext,
) {
    init_editor_test_app(cx);
    let root = std::env::temp_dir().join("gmark-link-completion-workspace");
    let current = root.join("notes").join("Current.md");
    let target = root.join("guides").join("Guide.md");
    let current_for_editor = current.clone();
    let (editor, visual) = cx.add_window_view(move |_window, cx| {
        Editor::from_markdown(cx, String::new(), Some(current_for_editor))
    });
    editor.update(visual, |editor, cx| {
        editor
            .workspace
            .install_markdown_snapshot_for_test(root, vec![current, target]);
        let block = editor.document.first_root().unwrap().clone();
        block.update(cx, |block, cx| {
            block.prepare_undo_capture(crate::components::UndoCaptureKind::NonCoalescible, cx);
            block.replace_text_in_visible_range(0..0, "[[gu", None, false, cx);
        });
    });
    visual.run_until_parked();
    editor.update(visual, |editor, cx| {
        assert!(editor.workspace_link_completion.is_some());
        editor.accept_workspace_link_completion(0, cx);
    });
    visual.run_until_parked();
    editor.update(visual, |editor, cx| {
        assert_eq!(editor.source_document.text(), "[Guide](../guides/Guide.md)");
        editor.undo_document(cx);
        assert_eq!(editor.source_document.text(), "[[gu");
    });
}

#[gpui::test]
async fn focused_math_block_shows_source_and_visual_editors(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let (editor, visual) =
        cx.add_window_view(|_window, cx| Editor::from_markdown(cx, "$$\nx^2\n$$".to_owned(), None));
    editor.update(visual, |editor, _cx| {
        let math = editor.document.first_root().unwrap().clone();
        editor.focus_block(math.entity_id());
    });
    redraw(visual);
    assert!(visual.debug_bounds("math-source-editor").is_some());
    assert!(visual.debug_bounds("math-visual-editor-surface").is_some());
    editor.read_with(visual, |editor, _cx| assert!(!editor.document_dirty));
}

#[gpui::test]
async fn mermaid_overlay_is_read_only_and_escape_restores_block_focus(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let source = "```mermaid\nflowchart LR\nA --> B\n```";
    let (editor, visual) =
        cx.add_window_view(move |_window, cx| Editor::from_markdown(cx, source.to_owned(), None));
    redraw(visual);
    visual.executor().advance_clock(Duration::from_millis(300));
    redraw(visual);
    visual.run_until_parked();
    redraw(visual);
    let open = visual
        .debug_bounds("mermaid-open-overlay")
        .expect("Mermaid enlarged-view button");
    let before = editor.read_with(visual, |editor, _cx| {
        (editor.source_document.text(), editor.document_dirty)
    });
    visual.simulate_click(open.center(), Modifiers::default());
    editor.read_with(visual, |editor, _cx| {
        assert!(editor.diagram_overlay.is_some());
    });
    redraw(visual);
    visual.run_until_parked();
    redraw(visual);
    assert!(visual.debug_bounds("diagram-overlay").is_some());
    editor.update_in(visual, |editor, window, _cx| {
        assert!(
            editor
                .diagram_overlay
                .as_ref()
                .expect("overlay state")
                .close_focus_handle
                .is_focused(window),
            "overlay close control must own keyboard focus"
        );
    });
    assert!(visual.debug_bounds("diagram-overlay-scale").is_some());
    assert!(visual.debug_bounds("diagram-overlay-close").is_some());
    let panel = visual
        .debug_bounds("diagram-overlay-panel")
        .expect("diagram overlay panel");
    let scale = visual
        .debug_bounds("diagram-overlay-scale")
        .expect("diagram scale control");
    let close = visual
        .debug_bounds("diagram-overlay-close")
        .expect("diagram close control");
    let button_height = crate::theme::Theme::default_theme()
        .dimensions
        .dialog_button_height;
    assert_eq!(f32::from(scale.size.height), button_height);
    assert_eq!(f32::from(close.size.height), button_height);
    assert!(scale.left() >= panel.left());
    assert!(scale.right() <= panel.right());
    assert!(close.left() >= panel.left());
    assert!(close.right() <= panel.right());
    assert!(scale.top() >= panel.top());
    assert!(close.bottom() <= panel.bottom());
    editor.update_in(visual, |editor, window, _cx| {
        editor
            .diagram_overlay
            .as_ref()
            .expect("overlay state")
            .scale_focus_handle
            .focus(window);
    });
    visual.simulate_keystrokes("space");
    editor.read_with(visual, |editor, _cx| {
        assert!(
            editor
                .diagram_overlay
                .as_ref()
                .expect("overlay state")
                .manual_scale
                .is_some(),
            "keyboard activation must toggle the overlay scale"
        );
    });
    visual.simulate_keystrokes("escape");
    editor.read_with(visual, |editor, _cx| {
        assert!(
            editor.diagram_overlay.is_none(),
            "Escape must clear overlay state"
        );
    });
    redraw(visual);
    visual.run_until_parked();
    redraw(visual);
    editor.read_with(visual, |editor, cx| {
        assert_eq!(
            (editor.source_document.text(), editor.document_dirty),
            before
        );
        let block = editor.document.first_root().unwrap();
        assert_eq!(block.read(cx).kind(), BlockKind::MermaidBlock);
    });
    editor.update_in(visual, |editor, window, cx| {
        assert!(
            editor
                .document
                .first_root()
                .unwrap()
                .read(cx)
                .focus_handle
                .is_focused(window)
        );
    });
}

#[gpui::test]
async fn mermaid_overlay_wheel_zooms_without_scrolling_source(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let source = format!(
        "```mermaid\nflowchart LR\nA --> B\n```\n\n{}",
        "Supporting paragraph.\n\n".repeat(80)
    );
    let (editor, visual) =
        cx.add_window_view(move |_window, cx| Editor::from_markdown(cx, source, None));
    redraw(visual);
    visual.executor().advance_clock(Duration::from_millis(300));
    visual.run_until_parked();
    redraw(visual);
    let open = visual
        .debug_bounds("mermaid-open-overlay")
        .expect("Mermaid enlarged-view button");
    visual.simulate_click(open.center(), Modifiers::default());
    redraw(visual);
    visual.run_until_parked();
    redraw(visual);
    let diagram_bounds = visual
        .debug_bounds("diagram-overlay-canvas")
        .expect("Mermaid enlarged-view canvas");
    let source_scroll_before =
        editor.read_with(visual, |editor, _cx| editor.scroll_handle.offset());
    editor.read_with(visual, |editor, _cx| {
        assert!(
            editor.scroll_handle.max_offset().height > px(0.0),
            "test document must be vertically scrollable"
        );
    });
    visual.simulate_event(gpui::ScrollWheelEvent {
        position: diagram_bounds.center(),
        delta: gpui::ScrollDelta::Pixels(point(px(0.0), px(120.0))),
        ..Default::default()
    });
    redraw(visual);
    let zoomed_bounds = visual
        .debug_bounds("diagram-overlay-canvas")
        .expect("zoomed Mermaid canvas");
    assert!(
        zoomed_bounds.size.width > diagram_bounds.size.width,
        "wheel up must enlarge the rendered diagram"
    );
    let zoomed_scale = editor.read_with(visual, |editor, _cx| {
        let state = editor.diagram_overlay.as_ref().expect("overlay state");
        assert_eq!(
            editor.scroll_handle.offset(),
            source_scroll_before,
            "diagram wheel input must not scroll the document behind the overlay"
        );
        state.manual_scale.expect("wheel up must zoom the diagram")
    });
    visual.simulate_event(gpui::ScrollWheelEvent {
        position: zoomed_bounds.center(),
        delta: gpui::ScrollDelta::Pixels(point(px(0.0), px(-120.0))),
        ..Default::default()
    });
    redraw(visual);
    let zoomed_out_bounds = visual
        .debug_bounds("diagram-overlay-canvas")
        .expect("zoomed-out Mermaid canvas");
    assert!(
        zoomed_out_bounds.size.width < zoomed_bounds.size.width,
        "wheel down must shrink the rendered diagram"
    );
    editor.read_with(visual, |editor, _cx| {
        let scale = editor
            .diagram_overlay
            .as_ref()
            .and_then(|state| state.manual_scale)
            .expect("overlay keeps its explicit zoom level");
        assert!(scale < zoomed_scale, "wheel down must zoom the diagram out");
        assert_eq!(
            editor.scroll_handle.offset(),
            source_scroll_before,
            "diagram wheel input must never scroll the document behind the overlay"
        );
    });
}

#[gpui::test]
async fn mermaid_workbench_uses_explicit_modes_and_adapts_its_content_height(
    cx: &mut TestAppContext,
) {
    init_editor_test_app(cx);
    let source = "```mermaid\nflowchart LR\nA --> B\n```";
    let (editor, visual) =
        cx.add_window_view(move |_window, cx| Editor::from_markdown(cx, source.to_owned(), None));
    visual.simulate_resize(size(px(960.0), px(720.0)));
    redraw(visual);
    visual.executor().advance_clock(Duration::from_millis(300));
    visual.run_until_parked();
    redraw(visual);

    let preview_frame = visual
        .debug_bounds("mermaid-workbench-frame")
        .expect("Mermaid workbench frame");
    assert!(visual.debug_bounds("mermaid-preview-pane").is_some());
    assert!(visual.debug_bounds("mermaid-source-pane").is_none());

    let diagram = visual
        .debug_bounds("mermaid-rendered-content")
        .expect("rendered Mermaid diagram");
    visual.simulate_click(diagram.center(), Modifiers::default());
    redraw(visual);
    assert!(
        visual.debug_bounds("mermaid-preview-pane").is_some(),
        "clicking the diagram must not implicitly enter source editing"
    );

    let source_mode = visual
        .debug_bounds("mermaid-view-source")
        .expect("source mode button");
    visual.simulate_click(source_mode.center(), Modifiers::default());
    redraw(visual);
    let source_frame = visual
        .debug_bounds("mermaid-workbench-frame")
        .expect("source-mode frame");
    assert_eq!(source_frame.size, preview_frame.size);
    assert!(visual.debug_bounds("mermaid-source-pane").is_some());
    assert!(visual.debug_bounds("mermaid-source-editor").is_some());

    let split_mode = visual
        .debug_bounds("mermaid-view-split")
        .expect("split mode button");
    visual.simulate_click(split_mode.center(), Modifiers::default());
    redraw(visual);
    let split_frame = visual
        .debug_bounds("mermaid-workbench-frame")
        .expect("split-mode frame");
    assert_eq!(split_frame.size.width, preview_frame.size.width);
    assert!(
        split_frame.size.height > preview_frame.size.height,
        "wide Split reserves its readable 420px content minimum"
    );
    assert!(visual.debug_bounds("mermaid-split-pane").is_some());
    assert!(visual.debug_bounds("mermaid-source-editor").is_some());
    assert!(visual.debug_bounds("mermaid-rendered-content").is_some());

    let copy = visual
        .debug_bounds("mermaid-copy-source")
        .expect("Mermaid source copy button");
    visual.simulate_click(copy.center(), Modifiers::default());
    redraw(visual);
    assert_eq!(
        visual.read_from_clipboard().and_then(|item| item.text()).as_deref(),
        Some(source)
    );
    editor.read_with(visual, |editor, cx| {
        assert!(
            editor
                .document
                .first_root()
                .expect("Mermaid block")
                .read(cx)
                .mermaid_copy_feedback
        );
        assert_eq!(editor.source_document.text(), source);
        assert!(!editor.document_dirty);
    });
    visual.executor().advance_clock(Duration::from_millis(1_200));
    visual.run_until_parked();
    editor.read_with(visual, |editor, cx| {
        assert!(
            !editor
                .document
                .first_root()
                .expect("Mermaid block")
                .read(cx)
                .mermaid_copy_feedback
        );
    });

    visual.simulate_resize(size(px(600.0), px(720.0)));
    visual.run_until_parked();
    redraw(visual);
    let narrow_preview = visual
        .debug_bounds("mermaid-split-preview-narrow")
        .expect("narrow split preview pane");
    let narrow_source = visual
        .debug_bounds("mermaid-source-editor")
        .expect("narrow split source pane");
    assert!(
        narrow_source.bottom() <= narrow_preview.top(),
        "narrow split must stack source above the preview"
    );
    assert!(
        narrow_source.size.height >= px(280.0),
        "narrow split source must remain readable"
    );
    assert!(
        narrow_preview.size.height >= px(280.0),
        "narrow split preview must remain readable"
    );
}

#[gpui::test]
async fn mermaid_preview_wheel_continues_scrolling_the_document(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let source = format!(
        "```mermaid\nflowchart TD\nA -->|yes| B\n```\n\n{}",
        "Supporting paragraph.\n\n".repeat(80)
    );
    let (editor, visual) =
        cx.add_window_view(move |_window, cx| Editor::from_markdown(cx, source, None));
    redraw(visual);
    visual.executor().advance_clock(Duration::from_millis(300));
    visual.run_until_parked();
    redraw(visual);

    let preview = visual
        .debug_bounds("mermaid-preview-pane")
        .expect("Mermaid preview pane");
    editor.read_with(visual, |editor, _cx| {
        assert!(editor.scroll_handle.max_offset().height > px(0.0));
        assert_eq!(editor.scroll_handle.offset().y, px(0.0));
    });
    visual.simulate_event(gpui::ScrollWheelEvent {
        position: preview.center(),
        delta: gpui::ScrollDelta::Pixels(point(px(0.0), px(-120.0))),
        ..Default::default()
    });
    redraw(visual);

    editor.read_with(visual, |editor, _cx| {
        assert!(
            editor.scroll_handle.offset().y < px(0.0),
            "wheel input over a non-scrollable Mermaid preview must continue scrolling the document"
        );
    });
}
