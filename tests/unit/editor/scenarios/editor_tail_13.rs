// @author kongweiguang

#[gpui::test]
async fn disabled_status_bar_releases_compact_workspace_bottom_space(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    cx.update(|cx| {
        crate::config::EditorSettings::init(cx, true, crate::config::AutoSavePreference::Off, true);
        crate::config::EditorSettings::set_status_bar_preferences_for_test(
            cx,
            crate::preferences::StatusBarPreferences {
                enabled: false,
                ..crate::preferences::StatusBarPreferences::default()
            },
        );
    });
    let (editor, visual) = cx.add_window_view(|_window, cx| {
        Editor::from_markdown(cx, "# Layout\n\nBody\n".to_owned(), None)
    });
    visual.simulate_resize(size(px(720.0), px(520.0)));
    editor.update(visual, |editor, cx| {
        editor.workspace.is_open = true;
        editor.export_in_progress = true;
        cx.notify();
    });
    redraw(visual);

    assert!(visual.debug_bounds("status-bar").is_none());
    let main = visual.debug_bounds("editor-main-content").unwrap();
    let workspace = visual.debug_bounds("compact-workspace-overlay").unwrap();
    let export = visual.debug_bounds("export-progress").unwrap();
    assert_eq!(workspace.bottom(), main.bottom());
    assert_eq!(f32::from(main.bottom() - export.bottom()), 10.0);
}

#[gpui::test]
async fn split_divider_resizes_resets_and_preserves_document_state(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let (editor, visual) = cx.add_window_view(|_window, cx| {
        Editor::from_markdown(cx, "# Heading\n\nBody\n".to_owned(), None)
    });
    visual.simulate_resize(size(px(1180.0), px(780.0)));
    editor.update(visual, |editor, cx| {
        editor.set_view_mode(ViewMode::Split, cx);
    });
    redraw(visual);

    let content = visual.debug_bounds("editor-content").unwrap();
    let source_before = visual.debug_bounds("split-source-pane-shell").unwrap();
    let preview_before = visual.debug_bounds("split-preview-pane").unwrap();
    let divider_before = visual.debug_bounds("split-divider").unwrap();
    let divider_line = visual.debug_bounds("split-divider-line").unwrap();
    assert_eq!(f32::from(divider_before.size.width), 7.0);
    assert_eq!(f32::from(divider_line.size.width), 1.0);
    assert_eq!(source_before.right(), divider_before.left());
    assert_eq!(divider_before.right(), preview_before.left());
    assert!(source_before.left() >= content.left());
    assert!(
        preview_before.right() <= content.right(),
        "preview={preview_before:?} content={content:?} source={source_before:?} divider={divider_before:?}"
    );

    let source = editor.read_with(visual, |editor, _cx| editor.source_document.text());
    let revision = editor.read_with(visual, |editor, _cx| editor.source_document.revision());
    let dirty = editor.read_with(visual, |editor, _cx| editor.document_dirty);
    let drag_target = point(
        divider_before.center().x + px(150.0),
        divider_before.center().y,
    );
    visual.simulate_mouse_down(
        divider_before.center(),
        MouseButton::Left,
        Modifiers::default(),
    );
    visual.simulate_mouse_move(drag_target, MouseButton::Left, Modifiers::default());
    redraw(visual);
    let source_after = visual.debug_bounds("split-source-pane-shell").unwrap();
    let preview_after = visual.debug_bounds("split-preview-pane").unwrap();
    assert!(source_after.size.width > source_before.size.width + px(120.0));
    assert!(preview_after.size.width < preview_before.size.width - px(120.0));
    editor.update(visual, |editor, _cx| {
        assert!(editor.split_resize_session.is_some());
    });
    visual.simulate_mouse_up(drag_target, MouseButton::Left, Modifiers::default());
    visual.run_until_parked();
    editor.update(visual, |editor, cx| {
        assert!(editor.split_resize_session.is_none());
        assert_eq!(editor.source_document.text(), source);
        assert_eq!(editor.source_document.revision(), revision);
        assert_eq!(editor.document_dirty, dirty);
        assert!(
            editor
                .workspace_session_snapshot_result(cx)
                .expect("canonical workspace session snapshot")
                .split_pane_ratio
                .is_some_and(|ratio| ratio > 0.5)
        );
    });

    redraw(visual);
    let divider_after = visual.debug_bounds("split-divider").unwrap();
    visual.simulate_event(MouseDownEvent {
        position: divider_after.center(),
        modifiers: Modifiers::default(),
        button: MouseButton::Left,
        click_count: 2,
        first_mouse: false,
    });
    visual.simulate_event(MouseUpEvent {
        position: divider_after.center(),
        modifiers: Modifiers::default(),
        button: MouseButton::Left,
        click_count: 2,
    });
    visual.run_until_parked();
    redraw(visual);
    let source_reset = visual.debug_bounds("split-source-pane-shell").unwrap();
    let preview_reset = visual.debug_bounds("split-preview-pane").unwrap();
    assert!((f32::from(source_reset.size.width - preview_reset.size.width)).abs() <= 1.0);
    editor.update(visual, |editor, cx| {
        assert_eq!(
            editor
                .workspace_session_snapshot_result(cx)
                .expect("canonical workspace session snapshot")
                .split_pane_ratio,
            Some(0.5)
        );
    });
    visual.update(|window, _cx| assert_eq!(window.scale_factor(), 2.0));
}

#[gpui::test]
async fn split_divider_keyboard_controls_are_focused_bounded_and_non_destructive(
    cx: &mut TestAppContext,
) {
    init_editor_test_app(cx);
    let (editor, visual) = cx.add_window_view(|_window, cx| {
        Editor::from_markdown(cx, "# Heading\n\nBody\n".to_owned(), None)
    });
    visual.simulate_resize(size(px(1180.0), px(780.0)));
    editor.update(visual, |editor, cx| {
        editor.set_view_mode(ViewMode::Split, cx)
    });
    redraw(visual);

    let source = editor.read_with(visual, |editor, _cx| editor.source_document.text());
    let revision = editor.read_with(visual, |editor, _cx| editor.source_document.revision());
    let dirty = editor.read_with(visual, |editor, _cx| editor.document_dirty);
    let divider = visual.debug_bounds("split-divider").unwrap();
    visual.simulate_click(divider.center(), Modifiers::default());
    redraw(visual);
    editor.update_in(visual, |editor, window, _cx| {
        assert!(editor.split_divider_focus_handle.is_focused(window));
    });

    visual.simulate_keystrokes("right shift-right");
    visual.run_until_parked();
    editor.update(visual, |editor, _cx| {
        assert!((editor.split_pane_ratio - 0.56).abs() < 0.001);
    });

    visual.simulate_keystrokes("home");
    redraw(visual);
    editor.update(visual, |editor, _cx| {
        assert!((0.3..=0.31).contains(&editor.split_pane_ratio));
    });
    visual.simulate_keystrokes("end");
    redraw(visual);
    editor.update(visual, |editor, _cx| {
        assert!((0.69..=0.7).contains(&editor.split_pane_ratio));
    });
    visual.simulate_keystrokes("enter");
    visual.run_until_parked();
    editor.update(visual, |editor, cx| {
        assert!((editor.split_pane_ratio - 0.5).abs() < 0.001);
        assert_eq!(editor.source_document.text(), source);
        assert_eq!(editor.source_document.revision(), revision);
        assert_eq!(editor.document_dirty, dirty);
        assert_eq!(
            editor
                .workspace_session_snapshot_result(cx)
                .expect("canonical workspace session snapshot")
                .split_pane_ratio,
            Some(0.5)
        );
    });

    visual.simulate_resize(size(px(720.0), px(520.0)));
    redraw(visual);
    visual.simulate_keystrokes("home");
    redraw(visual);
    let preview_at_minimum = visual.debug_bounds("split-preview-pane").unwrap();
    assert!(f32::from(preview_at_minimum.size.width) >= 280.0);
    visual.simulate_keystrokes("end");
    redraw(visual);
    let source_at_maximum = visual.debug_bounds("split-source-pane-shell").unwrap();
    let preview_at_maximum = visual.debug_bounds("split-preview-pane").unwrap();
    assert!(f32::from(source_at_maximum.size.width) >= 280.0);
    assert!(f32::from(preview_at_maximum.size.width) >= 280.0);
    visual.update(|window, _cx| assert_eq!(window.scale_factor(), 2.0));
}

#[gpui::test]
async fn split_preview_uses_same_reading_top_padding_without_mutating_document(
    cx: &mut TestAppContext,
) {
    init_editor_test_app(cx);
    let (editor, visual) = cx.add_window_view(|_window, cx| {
        Editor::from_markdown(cx, "# Heading\n\nBody\n".to_owned(), None)
    });
    let source = editor.read_with(visual, |editor, _cx| editor.source_document.text());
    let revision = editor.read_with(visual, |editor, _cx| editor.source_document.revision());
    let dirty = editor.read_with(visual, |editor, _cx| editor.document_dirty);
    editor.update(visual, |editor, cx| {
        editor.set_view_mode(ViewMode::Split, cx)
    });

    for viewport in [size(px(720.0), px(520.0)), size(px(1180.0), px(780.0))] {
        visual.simulate_resize(viewport);
        redraw(visual);
        let scroll = visual.debug_bounds("split-preview-scroll").unwrap();
        let content = visual.debug_bounds("split-preview-content").unwrap();
        let theme = Theme::default_theme();
        assert!(
            (f32::from(content.top() - scroll.top()) - theme.dimensions.editor_padding).abs()
                <= 0.5,
            "viewport={viewport:?} scroll={scroll:?} content={content:?}"
        );
        assert!(content.left() >= scroll.left());
        assert!(content.right() <= scroll.right());
    }
    editor.update(visual, |editor, _cx| {
        assert_eq!(editor.source_document.text(), source);
        assert_eq!(editor.source_document.revision(), revision);
        assert_eq!(editor.document_dirty, dirty);
    });
    visual.update(|window, _cx| assert_eq!(window.scale_factor(), 2.0));
}

#[gpui::test]
async fn split_preview_scrollbar_is_stable_draggable_and_non_destructive(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let markdown = (0..160)
        .map(|index| format!("# Heading {index}\n\nParagraph {index} with enough text to scroll."))
        .collect::<Vec<_>>()
        .join("\n\n");
    let (editor, visual) =
        cx.add_window_view(move |_window, cx| Editor::from_markdown(cx, markdown, None));
    visual.simulate_resize(size(px(720.0), px(520.0)));
    editor.update(visual, |editor, cx| {
        editor.set_view_mode(ViewMode::Split, cx);
        editor.split_preview_scrollbar_visible_until = Instant::now() + Duration::from_secs(1);
        cx.notify();
    });
    redraw(visual);
    redraw(visual);

    let source = editor.read_with(visual, |editor, _cx| editor.source_document.text());
    let revision = editor.read_with(visual, |editor, _cx| editor.source_document.revision());
    let dirty = editor.read_with(visual, |editor, _cx| editor.document_dirty);
    let undo_len = editor.read_with(visual, |editor, _cx| editor.undo_history.len());
    let pane = visual.debug_bounds("split-preview-pane").unwrap();
    let hitbox = visual
        .debug_bounds("split-preview-scrollbar-hitbox")
        .unwrap();
    let idle_thumb = visual
        .debug_bounds("split-preview-scrollbar-thumb")
        .unwrap();
    assert_eq!(f32::from(hitbox.size.width), 14.0);
    assert_eq!(f32::from(idle_thumb.size.width), 6.0);
    assert_eq!(idle_thumb.right(), hitbox.right());
    assert!(hitbox.left() >= pane.left());
    assert!(hitbox.right() <= pane.right());

    visual.simulate_mouse_move(hitbox.center(), None, Modifiers::default());
    redraw(visual);
    let hovered_hitbox = visual
        .debug_bounds("split-preview-scrollbar-hitbox")
        .unwrap();
    let hovered_thumb = visual
        .debug_bounds("split-preview-scrollbar-thumb")
        .unwrap();
    assert_eq!(f32::from(hovered_hitbox.size.width), 14.0);
    assert_eq!(f32::from(hovered_thumb.size.width), 10.0);
    assert_eq!(hovered_thumb.right(), hovered_hitbox.right());

    let start = hovered_hitbox.center();
    visual.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
    visual.simulate_mouse_move(
        point(start.x, start.y + px(120.0)),
        MouseButton::Left,
        Modifiers::default(),
    );
    editor.update(visual, |editor, _cx| {
        assert!(editor.split_preview_scrollbar_drag.is_some());
        let preview = editor.split_preview.as_ref().expect("preview should exist");
        assert!(preview.scroll_handle.offset().y < px(0.0));
    });
    redraw(visual);
    editor.update(visual, |editor, _cx| {
        assert!(editor.scroll_handle.offset().y < px(0.0));
    });
    visual.simulate_mouse_up(
        point(start.x, start.y + px(120.0)),
        MouseButton::Left,
        Modifiers::default(),
    );
    visual.run_until_parked();

    visual.simulate_resize(size(px(1180.0), px(780.0)));
    editor.update(visual, |editor, cx| {
        editor.split_preview_scrollbar_visible_until = Instant::now() + Duration::from_secs(1);
        cx.notify();
    });
    redraw(visual);
    let pane = visual.debug_bounds("split-preview-pane").unwrap();
    let hitbox = visual
        .debug_bounds("split-preview-scrollbar-hitbox")
        .unwrap();
    assert!(hitbox.left() >= pane.left());
    assert!(hitbox.right() <= pane.right());
    visual.update(|window, _cx| assert_eq!(window.scale_factor(), 2.0));

    editor.update(visual, |editor, cx| {
        editor.set_view_mode(ViewMode::Rendered, cx)
    });
    editor.update(visual, |editor, _cx| {
        assert!(editor.split_preview_scrollbar_drag.is_none());
        assert!(!editor.split_preview_scrollbar_hovered);
        assert!(editor.split_preview_scrollbar_fade_task.is_none());
        assert_eq!(editor.source_document.text(), source);
        assert_eq!(editor.source_document.revision(), revision);
        assert_eq!(editor.document_dirty, dirty);
        assert_eq!(editor.undo_history.len(), undo_len);
    });
}

#[gpui::test]
async fn compact_status_custom_button_dispatches_registered_action(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    cx.update(|cx| {
        crate::config::EditorSettings::init(cx, true, crate::config::AutoSavePreference::Off, true);
        crate::config::EditorSettings::set_status_bar_preferences_for_test(
            cx,
            crate::preferences::StatusBarPreferences {
                custom_buttons: vec![crate::preferences::StatusBarButton {
                    id: "mode".into(),
                    label: "Mode".into(),
                    action_id: "toggle_view_mode".into(),
                }],
                ..crate::preferences::StatusBarPreferences::default()
            },
        );
    });
    let (editor, visual) =
        cx.add_window_view(|_window, cx| Editor::from_markdown(cx, "# Heading\n".to_owned(), None));
    visual.simulate_resize(size(px(720.0), px(520.0)));
    editor.update(visual, |editor, cx| {
        editor.set_view_mode(ViewMode::Split, cx)
    });
    redraw(visual);

    let overflow = visual
        .debug_bounds("status-bar-format-overflow-button")
        .unwrap();
    visual.simulate_click(overflow.center(), Modifiers::default());
    redraw(visual);
    let custom = visual
        .debug_bounds("status-bar-custom-button-mode")
        .unwrap();
    assert!(custom.size.height > px(0.0));
    editor.update_in(visual, |editor, window, _cx| {
        let handle = editor
            .status_bar
            .custom_button_focus_handles
            .get("mode")
            .expect("custom status button focus");
        handle.focus(window);
        assert!(handle.is_focused(window));
    });
    visual.simulate_keystrokes("enter");
    visual.run_until_parked();
    editor.update(visual, |editor, _cx| {
        assert!(!editor.status_bar.format_overflow_open);
        assert_ne!(editor.view_mode, ViewMode::Split);
    });

    visual.update(|_window, cx| {
        crate::config::EditorSettings::set_status_bar_preferences_for_test(
            cx,
            crate::preferences::StatusBarPreferences::default(),
        );
        editor.update(cx, |_editor, cx| cx.notify());
    });
    redraw(visual);
    editor.update(visual, |editor, _cx| {
        assert!(editor.status_bar.custom_button_focus_handles.is_empty());
    });
}

#[gpui::test]
async fn slash_menu_stays_within_compact_and_standard_editor_bounds(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let (editor, visual_cx) =
        cx.add_window_view(|_window, cx| Editor::from_markdown(cx, "/".to_owned(), None));
    editor.update(visual_cx, |editor, cx| {
        let block = editor.document.first_root().expect("root").clone();
        editor.focus_block(block.entity_id());
        block.update(cx, |block, block_cx| {
            block.move_to(block.visible_len(), block_cx);
            block.refresh_slash_menu(block_cx);
        });
    });

    for viewport in [size(px(720.0), px(520.0)), size(px(1180.0), px(780.0))] {
        visual_cx.simulate_resize(viewport);
        redraw(visual_cx);
        let content = visual_cx.debug_bounds("editor-content").unwrap();
        let menu = visual_cx.debug_bounds("slash-command-menu").unwrap();
        assert!(menu.left() >= content.left());
        assert!(menu.right() <= content.right());
        assert!(menu.top() >= content.top());
        assert!(menu.bottom() <= content.bottom());
        assert!(f32::from(menu.size.width) <= 292.0);
        assert!(f32::from(menu.size.height) <= 304.0);
        for selector in [
            "slash-command-icon-heading_1",
            "slash-command-icon-heading_2",
            "slash-command-icon-heading_3",
            "slash-command-icon-bulleted_list",
            "slash-command-icon-numbered_list",
            "slash-command-icon-task_list",
            "slash-command-icon-quote",
            "slash-command-icon-code_block",
            "slash-command-icon-table",
            "slash-command-icon-image",
            "slash-command-icon-math",
        ] {
            let icon = visual_cx
                .debug_bounds(selector)
                .unwrap_or_else(|| panic!("missing {selector}"));
            assert_eq!(icon.size, size(px(24.0), px(24.0)));
            assert!(icon.left() >= menu.left());
            assert!(icon.right() <= menu.right());
        }
    }
}

/// 验证窄窗口中的工具栏和附着菜单都约束在所属编辑视口内。
#[gpui::test]
async fn selection_toolbar_popovers_stay_inside_narrow_editor_viewport(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let (editor, visual) =
        cx.add_window_view(|_window, cx| Editor::from_markdown(cx, "alpha beta".to_owned(), None));
    visual.simulate_resize(size(px(320.0), px(280.0)));
    editor.update_in(visual, |editor, window, cx| {
        let block = editor.document.first_root().expect("root").clone();
        editor.focus_block(block.entity_id());
        block.update(cx, |block, block_cx| {
            block.focus_handle.focus(window);
            block.selected_range = 0..5;
            block.refresh_selection_toolbar();
            block_cx.notify();
        });
    });
    redraw(visual);

    let content = visual.debug_bounds("editor-content").unwrap();
    let toolbar = visual.debug_bounds("selection-toolbar").unwrap();
    let owner_viewport = editor.update(visual, |editor, cx| {
        editor
            .document
            .first_root()
            .expect("root")
            .read(cx)
            .selection_toolbar_viewport
    });
    assert!(
        toolbar.left() >= content.left(),
        "toolbar={toolbar:?} content={content:?} owner_viewport={owner_viewport:?}"
    );
    assert!(toolbar.right() <= content.right());
    assert!(
        toolbar.top() >= content.top(),
        "toolbar={toolbar:?} content={content:?}"
    );
    assert!(toolbar.bottom() <= content.bottom());

    let block_type = visual.debug_bounds("selection-toolbar-block-type").unwrap();
    visual.simulate_click(block_type.center(), Modifiers::default());
    visual.run_until_parked();
    redraw(visual);
    let type_menu = visual
        .debug_bounds("selection-toolbar-block-type-menu")
        .unwrap();
    assert!(type_menu.left() >= content.left());
    assert!(type_menu.right() <= content.right());
    assert!(type_menu.top() >= content.top());
    assert!(type_menu.bottom() <= content.bottom());

    visual.simulate_click(block_type.center(), Modifiers::default());
    visual.run_until_parked();
    redraw(visual);
    let link = visual.debug_bounds("selection-toolbar-link").unwrap();
    visual.simulate_click(link.center(), Modifiers::default());
    visual.run_until_parked();
    redraw(visual);
    if let Some(link_editor) = visual.debug_bounds("selection-toolbar-link-editor") {
        // The narrow test backend may keep the link editor deferred while a
        // previous popover relinquishes focus; validate it whenever mounted.
        assert!(link_editor.left() >= content.left());
        assert!(link_editor.right() <= content.right());
        assert!(link_editor.top() >= content.top());
        assert!(link_editor.bottom() <= content.bottom());
    }
}

#[gpui::test]
async fn task_checkbox_uses_stable_svg_and_preview_remains_read_only(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let source = "- [x] completed task";
    let (editor, visual_cx) =
        cx.add_window_view(move |_window, cx| Editor::from_markdown(cx, source.to_owned(), None));

    for viewport in [size(px(720.0), px(520.0)), size(px(1180.0), px(780.0))] {
        visual_cx.simulate_resize(viewport);
        redraw(visual_cx);
        visual_cx.update(|window, _cx| assert_eq!(window.scale_factor(), 2.0));
        let content = visual_cx.debug_bounds("editor-content").unwrap();
        let checkbox = visual_cx.debug_bounds("task-checkbox").unwrap();
        let check = visual_cx.debug_bounds("task-checkbox-check").unwrap();
        assert_eq!(checkbox.size, size(px(14.0), px(14.0)));
        assert_eq!(check.size, size(px(10.0), px(10.0)));
        assert!(checkbox.left() >= content.left());
        assert!(checkbox.right() <= content.right());
        assert!(check.left() >= checkbox.left());
        assert!(check.right() <= checkbox.right());
        assert!(check.top() >= checkbox.top());
        assert!(check.bottom() <= checkbox.bottom());
    }

    let checkbox = visual_cx.debug_bounds("task-checkbox").unwrap();
    visual_cx.simulate_click(checkbox.center(), Modifiers::default());
    visual_cx.run_until_parked();
    redraw(visual_cx);
    editor.update(visual_cx, |editor, cx| {
        assert_eq!(editor.source_document.text(), "- [ ] completed task");
        assert!(editor.document_dirty);
        editor.undo_document(cx);
        assert_eq!(editor.source_document.text(), source);
    });
    redraw(visual_cx);
    assert!(visual_cx.debug_bounds("task-checkbox-check").is_some());

    editor.update(visual_cx, |editor, cx| {
        editor.set_view_mode(ViewMode::Preview, cx);
    });
    redraw(visual_cx);
    let (revision, dirty) = editor.read_with(visual_cx, |editor, _cx| {
        (editor.source_document.revision(), editor.document_dirty)
    });
    let checkbox = visual_cx.debug_bounds("task-checkbox").unwrap();
    visual_cx.simulate_click(checkbox.center(), Modifiers::default());
    visual_cx.run_until_parked();
    editor.read_with(visual_cx, |editor, _cx| {
        assert_eq!(editor.source_document.text(), source);
        assert_eq!(editor.source_document.revision(), revision);
        assert_eq!(editor.document_dirty, dirty);
    });
}

#[gpui::test]
async fn nested_bulleted_lists_use_centered_font_independent_markers(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let source = "- root\n  - nested\n    - deep";
    let (editor, visual_cx) =
        cx.add_window_view(move |_window, cx| Editor::from_markdown(cx, source.to_owned(), None));
    let (revision, dirty) = editor.read_with(visual_cx, |editor, _cx| {
        (editor.source_document.revision(), editor.document_dirty)
    });

    for viewport in [size(px(720.0), px(520.0)), size(px(1180.0), px(780.0))] {
        visual_cx.simulate_resize(viewport);
        redraw(visual_cx);
        visual_cx.update(|window, _cx| assert_eq!(window.scale_factor(), 2.0));
        let content = visual_cx.debug_bounds("editor-content").unwrap();
        let mut previous_left = None;
        for (slot_selector, marker_selector, marker_size) in [
            (
                "bulleted-list-marker-slot-filled",
                "bulleted-list-marker-filled",
                5.0,
            ),
            (
                "bulleted-list-marker-slot-hollow",
                "bulleted-list-marker-hollow",
                6.0,
            ),
            (
                "bulleted-list-marker-slot-square",
                "bulleted-list-marker-square",
                6.0,
            ),
        ] {
            let slot = visual_cx.debug_bounds(slot_selector).unwrap();
            let marker = visual_cx.debug_bounds(marker_selector).unwrap();
            assert_eq!(slot.size.width, px(12.0));
            assert_eq!(marker.size, size(px(marker_size), px(marker_size)));
            assert!((f32::from(slot.center().x - marker.center().x)).abs() <= 0.5);
            assert!((f32::from(slot.center().y - marker.center().y)).abs() <= 0.5);
            assert!(slot.left() >= content.left());
            assert!(slot.right() <= content.right());
            if let Some(previous_left) = previous_left {
                assert!(slot.left() > previous_left);
            }
            previous_left = Some(slot.left());
        }
    }

    editor.read_with(visual_cx, |editor, _cx| {
        assert_eq!(editor.source_document.text(), source);
        assert_eq!(editor.source_document.revision(), revision);
        assert_eq!(editor.document_dirty, dirty);
    });
}
