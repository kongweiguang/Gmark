// @author kongweiguang

/// 窗格恢复使用子编辑器，真实多击与键盘派发必须沿同一个焦点祖先接通工具栏。
#[gpui::test]
async fn pane_selection_toolbar_keeps_word_selection_during_keyboard_navigation(
    cx: &mut TestAppContext,
) {
    init_editor_test_app(cx);
    let (editor, visual) =
        cx.add_window_view(|_window, cx| Editor::from_markdown(cx, "alpha beta".into(), None));
    editor.update(visual, |editor, cx| {
        editor.split_pane_toward(crate::editor::panes::PaneSplitDirection::Right, cx);
    });
    visual.run_until_parked();
    redraw(visual);
    let pane_editor = editor.read_with(visual, |editor, cx| {
        let pane = editor
            .pane_workspace
            .as_ref()
            .expect("workspace")
            .read(cx)
            .workspace()
            .focused_pane();
        let canvases = editor.pane_canvas_entities.borrow();
        let (_, _, canvas) = canvases.get(&pane).expect("focused canvas");
        match canvas {
            crate::editor::panes::PaneCanvasEntity::Markdown(canvas) => canvas.read(cx).editor(),
            _ => panic!("Markdown canvas"),
        }
    });
    let block = pane_editor.read_with(visual, |editor, _cx| {
        editor.document.first_root().expect("word block").clone()
    });
    let position = block.read_with(visual, |block, _cx| {
        let bounds = block.last_bounds.expect("word layout");
        point(bounds.left() + px(12.0), bounds.center().y)
    });
    visual.simulate_click(position, Modifiers::default());
    visual.simulate_event(gpui::MouseDownEvent {
        position,
        modifiers: Modifiers::default(),
        button: MouseButton::Left,
        click_count: 2,
        first_mouse: false,
    });
    visual.simulate_mouse_up(position, MouseButton::Left, Modifiers::default());
    visual.run_until_parked();
    redraw(visual);
    assert_eq!(block.read_with(visual, |block, _cx| block.selected_range.clone()), 0..5);
    visual.simulate_keystrokes("alt-f10");
    redraw(visual);
    block.read_with(visual, |block, _cx| {
        assert!(block.selection_toolbar_keyboard_active, "focused pane toolbar");
        assert_eq!(block.selection_toolbar_keyboard_index, 0);
    });
    visual.simulate_keystrokes("right");
    redraw(visual);
    block.read_with(visual, |block, _cx| {
        assert_eq!(block.selected_range, 0..5);
        assert!(block.selection_toolbar_keyboard_active);
        assert_eq!(block.selection_toolbar_keyboard_index, 1);
    });
    visual.simulate_keystrokes("escape");
    redraw(visual);
    block.read_with(visual, |block, _cx| {
        assert_eq!(block.selected_range, 0..5);
        assert!(!block.selection_toolbar_keyboard_active);
    });
    editor.update(visual, |editor, cx| editor.open_menu_bar(0, cx));
    redraw(visual);
    visual.simulate_keystrokes("escape");
    redraw(visual);
    assert!(editor.read_with(visual, |editor, _cx| editor.menu_bar_open.is_none()));
}

/// 验证选区工具栏随可用空间切换上下位置，同时保持选区与撤销状态稳定。
#[gpui::test]
async fn selection_toolbar_formats_without_losing_selection_and_undo_restores_source(
    cx: &mut TestAppContext,
) {
    init_editor_test_app(cx);
    let (editor, visual_cx) =
        cx.add_window_view(|_window, cx| Editor::from_markdown(cx, "alpha beta".to_string(), None));
    editor.update(visual_cx, |editor, cx| {
        let block = editor.document.first_root().expect("root").clone();
        editor.focus_block(block.entity_id());
        block.update(cx, |block, block_cx| {
            block.selected_range = 0..5;
            block.selection_reversed = false;
            block.refresh_selection_toolbar();
            block_cx.notify();
        });
    });
    for viewport in [size(px(720.0), px(520.0)), size(px(1180.0), px(780.0))] {
        visual_cx.simulate_resize(viewport);
        redraw(visual_cx);
        let toolbar = visual_cx.debug_bounds("selection-toolbar").unwrap();
        let content = visual_cx.debug_bounds("editor-content").unwrap();
        let selection = editor.update(visual_cx, |editor, cx| {
            editor
                .document
                .first_root()
                .expect("root")
                .read(cx)
                .active_range_or_cursor_bounds()
                .expect("selection bounds")
        });
        assert_eq!(f32::from(toolbar.size.height), 32.0);
        assert!(toolbar.left() >= content.left());
        assert!(toolbar.right() <= content.right());
        assert!(toolbar.top() >= content.top());
        assert!(toolbar.bottom() <= content.bottom());
        assert!(
            toolbar.bottom() <= selection.top() || toolbar.top() >= selection.bottom(),
            "toolbar={toolbar:?} selection={selection:?}"
        );
    }

    let bold_tooltip_target = visual_cx
        .debug_bounds("selection-toolbar-bold")
        .expect("bold button");
    visual_cx.simulate_mouse_move(bold_tooltip_target.center(), None, Modifiers::default());
    visual_cx
        .executor()
        .advance_clock(Duration::from_millis(520));
    visual_cx.run_until_parked();
    redraw(visual_cx);
    let tooltip = visual_cx.debug_bounds("ui-tooltip").unwrap();
    let content = visual_cx.debug_bounds("editor-content").unwrap();
    assert!(tooltip.left() >= content.left());
    assert!(tooltip.right() <= content.right());

    let overflow_button = visual_cx
        .debug_bounds("selection-toolbar-overflow")
        .expect("overflow button");
    visual_cx.simulate_click(overflow_button.center(), Modifiers::default());
    redraw(visual_cx);
    let overflow = visual_cx
        .debug_bounds("selection-toolbar-overflow-menu")
        .expect("overflow menu");
    let content = visual_cx.debug_bounds("editor-content").unwrap();
    assert!(overflow.left() >= content.left());
    assert!(overflow.right() <= content.right());
    assert!(overflow.top() >= content.top());
    assert!(overflow.bottom() <= content.bottom());
    let underline = visual_cx
        .debug_bounds("selection-toolbar-underline")
        .expect("underline button");
    visual_cx.simulate_click(underline.center(), Modifiers::default());
    redraw(visual_cx);
    editor.update(visual_cx, |editor, cx| {
        let block = editor.document.first_root().expect("root").clone();
        assert_eq!(editor.source_document.text(), "<u>alpha</u> beta");
        assert_eq!(block.read(cx).selection_clean_range(), 0..5);
        editor.undo_document(cx);
        assert_eq!(editor.source_document.text(), "alpha beta");
    });
    redraw(visual_cx);

    let bold = visual_cx
        .debug_bounds("selection-toolbar-bold")
        .expect("bold button");
    visual_cx.simulate_click(bold.center(), Modifiers::default());
    redraw(visual_cx);

    editor.update(visual_cx, |editor, cx| {
        let block = editor.document.first_root().expect("root").clone();
        assert_eq!(editor.source_document.text(), "**alpha** beta");
        assert_eq!(block.read(cx).selection_clean_range(), 0..5);
        assert!(block.read(cx).selection_toolbar_visible());
        editor.undo_document(cx);
        assert_eq!(editor.source_document.text(), "alpha beta");
    });
}

#[gpui::test]
async fn selection_toolbar_roving_focus_intercepts_real_key_dispatch(
    cx: &mut TestAppContext,
) {
    init_editor_test_app(cx);
    let (editor, visual_cx) =
        cx.add_window_view(|_window, cx| Editor::from_markdown(cx, "alpha beta".to_string(), None));
    editor.update(visual_cx, |editor, cx| {
        let block = editor.document.first_root().expect("root").clone();
        editor.focus_block(block.entity_id());
        block.update(cx, |block, block_cx| {
            block.selected_range = 0..5;
            block.refresh_selection_toolbar();
            block_cx.notify();
        });
    });
    redraw(visual_cx);

    visual_cx.simulate_keystrokes("alt-f10");
    redraw(visual_cx);
    editor.update(visual_cx, |editor, cx| {
        let block = editor.document.first_root().expect("root").read(cx);
        assert_eq!(block.selected_range, 0..5);
        assert!(block.selection_toolbar_keyboard_active);
        assert_eq!(block.selection_toolbar_keyboard_index, 0);
    });

    visual_cx.simulate_keystrokes("right");
    redraw(visual_cx);

    editor.update(visual_cx, |editor, cx| {
        let block = editor.document.first_root().expect("root").read(cx);
        assert_eq!(block.selected_range, 0..5);
        assert!(block.selection_toolbar_keyboard_active);
        assert_eq!(block.selection_toolbar_keyboard_index, 1);
        assert!(block.selection_toolbar_visible());
    });

    visual_cx.simulate_keystrokes("enter");
    redraw(visual_cx);
    editor.update(visual_cx, |editor, cx| {
        let block = editor.document.first_root().expect("root").read(cx);
        assert_eq!(editor.source_document.text(), "**alpha** beta");
        assert_eq!(block.selection_clean_range(), 0..5);
        assert!(block.selection_toolbar_visible());
    });
}

#[gpui::test]
async fn selection_toolbar_escape_dismisses_only_until_selection_changes(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let (editor, visual_cx) =
        cx.add_window_view(|_window, cx| Editor::from_markdown(cx, "alpha beta".to_string(), None));
    editor.update(visual_cx, |editor, cx| {
        let block = editor.document.first_root().expect("root").clone();
        editor.focus_block(block.entity_id());
        block.update(cx, |block, block_cx| {
            block.selected_range = 0..5;
            block.refresh_selection_toolbar();
            block_cx.notify();
        });
    });
    redraw(visual_cx);
    assert!(visual_cx.debug_bounds("selection-toolbar").is_some());

    let escape = KeyDownEvent {
        keystroke: Keystroke::parse("escape").expect("valid escape keystroke"),
        is_held: false,
    };
    editor.update_in(visual_cx, |editor, window, cx| {
        editor.on_editor_key_down_capture(&escape, window, cx);
    });
    editor.update(visual_cx, |editor, cx| {
        let block = editor.document.first_root().expect("root").read(cx);
        assert_eq!(block.selection_toolbar_dismissed_range, Some(0..5));
        assert!(!block.selection_toolbar_visible());
    });
    redraw(visual_cx);
    redraw(visual_cx);
    editor.update(visual_cx, |editor, cx| {
        let block = editor.document.first_root().expect("root").clone();
        assert_eq!(block.read(cx).selected_range, 0..5);
        assert!(!block.read(cx).selection_toolbar_visible());
        block.update(cx, |block, block_cx| {
            block.selected_range = 0..4;
            block.refresh_selection_toolbar();
            assert!(block.selection_toolbar_visible());
            block_cx.notify();
        });
    });
    redraw(visual_cx);
    assert!(visual_cx.debug_bounds("selection-toolbar").is_some());
}

#[gpui::test]
async fn code_toolbar_copies_body_without_mutating_document_and_clears_feedback(
    cx: &mut TestAppContext,
) {
    init_editor_test_app(cx);
    let source = "```rust\nfn main() {}\n```";
    let (editor, visual_cx) =
        cx.add_window_view(move |_window, cx| Editor::from_markdown(cx, source.to_owned(), None));
    for viewport in [
        size(px(720.0), px(520.0)),
        size(px(1180.0), px(780.0)),
        size(px(1440.0), px(780.0)),
    ] {
        visual_cx.simulate_resize(viewport);
        redraw(visual_cx);
        let toolbar = visual_cx.debug_bounds("code-block-toolbar").unwrap();
        let control = visual_cx.debug_bounds("code-language-control").unwrap();
        let copy = visual_cx.debug_bounds("code-block-copy").unwrap();
        let content = visual_cx.debug_bounds("editor-content").unwrap();
        assert_eq!(f32::from(toolbar.size.height), 32.0);
        assert!(toolbar.left() >= content.left());
        assert!(toolbar.right() <= content.right());
        assert!(control.left() >= toolbar.left());
        assert!(
            copy.right() <= toolbar.right(),
            "toolbar={toolbar:?} copy={copy:?} control={control:?} viewport={viewport:?}"
        );
        assert!(control.right() <= copy.left());
    }

    let (revision, dirty) = editor.read_with(visual_cx, |editor, _cx| {
        (editor.source_document.revision(), editor.document_dirty)
    });
    let copy = visual_cx.debug_bounds("code-block-copy").unwrap();
    visual_cx.simulate_mouse_move(copy.center(), None, Modifiers::default());
    visual_cx
        .executor()
        .advance_clock(Duration::from_millis(520));
    visual_cx.run_until_parked();
    redraw(visual_cx);
    let tooltip = visual_cx.debug_bounds("ui-tooltip").unwrap();
    let content = visual_cx.debug_bounds("editor-content").unwrap();
    assert!(tooltip.left() >= content.left());
    assert!(tooltip.right() <= content.right());
    visual_cx.simulate_click(copy.center(), Modifiers::default());
    redraw(visual_cx);
    assert_eq!(
        visual_cx
            .read_from_clipboard()
            .and_then(|item| item.text())
            .as_deref(),
        Some("fn main() {}")
    );
    editor.update(visual_cx, |editor, cx| {
        let block = editor.document.first_root().expect("code block").read(cx);
        assert!(block.code_copy_feedback);
        assert_eq!(editor.source_document.revision(), revision);
        assert_eq!(editor.document_dirty, dirty);
    });
    visual_cx
        .executor()
        .advance_clock(Duration::from_millis(1_200));
    visual_cx.run_until_parked();
    editor.update(visual_cx, |editor, cx| {
        assert!(
            !editor
                .document
                .first_root()
                .expect("code block")
                .read(cx)
                .code_copy_feedback
        );
    });

    visual_cx.simulate_keystrokes("enter");
    redraw(visual_cx);
    assert_eq!(
        visual_cx
            .read_from_clipboard()
            .and_then(|item| item.text())
            .as_deref(),
        Some("fn main() {}")
    );
}
