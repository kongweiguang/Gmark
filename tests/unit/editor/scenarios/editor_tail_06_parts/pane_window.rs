// @author kongweiguang

//! Window-close behavior for dirty documents mounted in multiple panes.

use super::*;

// Reason: Keep shared-document discard coverage grouped with the pane-close scenarios so the
// main tail file stays readable without changing the behavior exercised by the test.
#[gpui::test]
async fn pane_window_discard_clears_shared_dirty_document(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let (editor, visual) =
        cx.add_window_view(|_window, cx| Editor::from_markdown(cx, "draft".to_owned(), None));

    editor.update(visual, |editor, cx| {
        editor.split_pane_toward(crate::editor::panes::PaneSplitDirection::Right, cx)
    });
    visual.run_until_parked();
    redraw(visual);

    let pane_editors = editor.read_with(visual, |editor, cx| {
        editor
            .pane_canvas_entities
            .borrow()
            .values()
            .filter_map(|(_, _, canvas)| match canvas {
                crate::editor::panes::PaneCanvasEntity::Markdown(canvas) => {
                    Some(canvas.read(cx).editor())
                }
                crate::editor::panes::PaneCanvasEntity::DocumentHost(_)
                | crate::editor::panes::PaneCanvasEntity::ReadOnly(_) => None,
            })
            .collect::<Vec<_>>()
    });
    assert_eq!(pane_editors.len(), 2);
    pane_editors[0].update(visual, |pane_editor, _cx| {
        pane_editor.set_document_dirty_for_test(true);
    });
    visual.run_until_parked();

    visual.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            assert!(!editor.on_window_should_close(window, cx));
            assert!(editor.show_unsaved_changes_dialog);
            editor.on_discard_and_close(&gpui::ClickEvent::default(), window, cx);
        });
    });

    for pane_editor in pane_editors {
        assert!(!pane_editor.read_with(visual, |pane_editor, _cx| {
            pane_editor.source_document.is_dirty()
        }));
    }
}

// Reason: Preserve the focused-pane distinction while checking that a background dirty tab still
// participates in the window-close confirmation and discard path.
#[gpui::test]
async fn pane_window_close_prompts_for_background_dirty_document(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let (editor, visual) =
        cx.add_window_view(|_window, cx| Editor::from_markdown(cx, "focused".to_owned(), None));

    editor.update(visual, |editor, cx| {
        editor.split_pane_toward(crate::editor::panes::PaneSplitDirection::Right, cx)
    });
    visual.run_until_parked();
    redraw(visual);

    let (focused_pane, background_pane) = editor.read_with(visual, |editor, cx| {
        let workspace = editor.pane_workspace.as_ref().unwrap().read(cx);
        let focused = workspace.workspace().focused_pane();
        let other = workspace
            .workspace()
            .pane_ids()
            .into_iter()
            .find(|pane| *pane != focused)
            .expect("background pane");
        (focused, other)
    });
    editor.update(visual, |editor, cx| {
        assert!(editor.new_document_tab_in_pane(background_pane, DocumentKind::Markdown, cx));
        editor
            .pane_workspace
            .as_ref()
            .expect("pane workspace")
            .update(cx, |workspace, _cx| {
                workspace.workspace_mut().focus(focused_pane).unwrap();
            });
    });
    visual.run_until_parked();
    redraw(visual);

    let background_editor = editor.read_with(visual, |editor, cx| {
        let canvases = editor.pane_canvas_entities.borrow();
        let (_, _, canvas) = canvases.get(&background_pane).expect("background canvas");
        match canvas {
            crate::editor::panes::PaneCanvasEntity::Markdown(canvas) => canvas.read(cx).editor(),
            crate::editor::panes::PaneCanvasEntity::DocumentHost(_)
            | crate::editor::panes::PaneCanvasEntity::ReadOnly(_) => {
                panic!("background markdown canvas expected")
            }
        }
    });
    background_editor.update(visual, |pane_editor, _cx| {
        pane_editor.set_document_dirty_for_test(true);
    });
    visual.run_until_parked();

    visual.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            assert_eq!(
                editor
                    .pane_workspace
                    .as_ref()
                    .unwrap()
                    .read(cx)
                    .workspace()
                    .focused_pane(),
                focused_pane
            );
            assert!(!editor.on_window_should_close(window, cx));
            assert!(editor.show_unsaved_changes_dialog);
            editor.on_discard_and_close(&gpui::ClickEvent::default(), window, cx);
        });
    });
    assert!(!background_editor.read_with(visual, |pane_editor, _cx| {
        pane_editor.source_document.is_dirty()
    }));
}

// Reason: Validate that discarding a window clears every dirty document collected by the close
// coordinator, not only the currently focused pane.
#[gpui::test]
async fn pane_window_discard_clears_multiple_dirty_documents(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let (editor, visual) =
        cx.add_window_view(|_window, cx| Editor::from_markdown(cx, "first".to_owned(), None));

    editor.update(visual, |editor, cx| {
        editor.split_pane_toward(crate::editor::panes::PaneSplitDirection::Right, cx)
    });
    visual.run_until_parked();
    redraw(visual);
    let background_pane = editor.read_with(visual, |editor, cx| {
        let workspace = editor.pane_workspace.as_ref().unwrap().read(cx);
        let focused = workspace.workspace().focused_pane();
        workspace
            .workspace()
            .pane_ids()
            .into_iter()
            .find(|pane| *pane != focused)
            .expect("background pane")
    });

    editor.update(visual, |editor, cx| {
        assert!(editor.new_document_tab_in_pane(background_pane, DocumentKind::Markdown, cx));
    });
    visual.run_until_parked();
    redraw(visual);

    let markdown_canvases = editor.read_with(visual, |editor, cx| {
        editor
            .pane_canvas_entities
            .borrow()
            .values()
            .filter_map(|(_, _, canvas)| match canvas {
                crate::editor::panes::PaneCanvasEntity::Markdown(canvas) => {
                    Some(canvas.read(cx).editor())
                }
                crate::editor::panes::PaneCanvasEntity::DocumentHost(_)
                | crate::editor::panes::PaneCanvasEntity::ReadOnly(_) => None,
            })
            .collect::<Vec<_>>()
    });
    assert_eq!(markdown_canvases.len(), 2);
    for canvas in &markdown_canvases {
        canvas.update(visual, |pane_editor, _cx| {
            pane_editor.set_document_dirty_for_test(true);
        });
    }
    visual.run_until_parked();

    editor.update(visual, |editor, cx| {
        let states = editor.document_close_states(cx);
        assert_eq!(states.iter().filter(|state| state.dirty).count(), 2);
        assert!(editor.discard_all_document_changes_for_window_close(cx));
        assert!(
            editor
                .document_close_states(cx)
                .into_iter()
                .all(|state| !state.dirty)
        );
    });
}

// Reason: Keep an external lease alive to prove discard honors ownership boundaries instead of
// silently clearing a document that another view still references.
#[gpui::test]
async fn pane_window_discard_respects_external_document_lease(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let (editor, visual) =
        cx.add_window_view(|_window, cx| Editor::from_markdown(cx, "draft".to_owned(), None));

    editor.update(visual, |editor, cx| {
        editor.split_pane_toward(crate::editor::panes::PaneSplitDirection::Right, cx)
    });
    visual.run_until_parked();
    let external_lease = editor.read_with(visual, |editor, cx| {
        let canvases = editor.pane_canvas_entities.borrow();
        let (_, _, canvas) = canvases.values().next().expect("mounted markdown canvas");
        match canvas {
            crate::editor::panes::PaneCanvasEntity::Markdown(canvas) => canvas
                .read(cx)
                .editor()
                .read(cx)
                .source_document
                .handle()
                .expect("document handle")
                .lease(),
            crate::editor::panes::PaneCanvasEntity::DocumentHost(_)
            | crate::editor::panes::PaneCanvasEntity::ReadOnly(_) => {
                panic!("markdown canvas expected")
            }
        }
    });
    let pane_editor = editor.read_with(visual, |editor, cx| {
        let canvases = editor.pane_canvas_entities.borrow();
        let (_, _, canvas) = canvases.values().next().expect("mounted markdown canvas");
        match canvas {
            crate::editor::panes::PaneCanvasEntity::Markdown(canvas) => canvas.read(cx).editor(),
            crate::editor::panes::PaneCanvasEntity::DocumentHost(_)
            | crate::editor::panes::PaneCanvasEntity::ReadOnly(_) => {
                panic!("markdown canvas expected")
            }
        }
    });
    pane_editor.update(visual, |pane_editor, _cx| {
        pane_editor.set_document_dirty_for_test(true);
    });
    visual.run_until_parked();

    editor.update(visual, |editor, cx| {
        assert!(editor.discard_all_document_changes_for_window_close(cx));
        assert!(
            editor
                .document_close_states(cx)
                .into_iter()
                .any(|state| { state.dirty && !state.closes_last_lease() })
        );
    });
    drop(external_lease);
}

/// Keeps Find, Replace, and match navigation on the focused pane's Markdown editor.
#[gpui::test]
async fn pane_window_find_targets_focused_markdown_editor(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let (editor, visual) = cx.add_window_view(|_window, cx| {
        Editor::from_markdown(cx, "alpha beta alpha gamma alpha".to_owned(), None)
    });

    editor.update(visual, |editor, cx| {
        editor.split_pane_toward(crate::editor::panes::PaneSplitDirection::Right, cx)
    });
    visual.run_until_parked();
    redraw(visual);

    let pane_editor = editor.read_with(visual, |editor, cx| {
        let workspace = editor.pane_workspace.as_ref().expect("pane workspace");
        let pane = workspace.read(cx).workspace().focused_pane();
        let canvases = editor.pane_canvas_entities.borrow();
        let (_, _, canvas) = canvases.get(&pane).expect("focused pane canvas");
        match canvas {
            crate::editor::panes::PaneCanvasEntity::Markdown(canvas) => canvas.read(cx).editor(),
            crate::editor::panes::PaneCanvasEntity::DocumentHost(_)
            | crate::editor::panes::PaneCanvasEntity::ReadOnly(_) => {
                panic!("focused Markdown pane expected")
            }
        }
    });

    visual.update(|window, cx| {
        pane_editor.update(cx, |editor, cx| {
            let block = editor.document.first_root().expect("root block").clone();
            editor.active_entity_id = Some(block.entity_id());
            block.update(cx, |block, _cx| {
                block.focus_handle.focus(window);
                block.selected_range = 0..5;
            });
        });
    });
    visual.run_until_parked();
    redraw(visual);

    visual.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.on_find_in_document_action(&crate::components::FindInDocument, window, cx);
        });
    });

    assert!(pane_editor.read_with(visual, |editor, _cx| editor.find_panel.is_some()));
    assert!(editor.read_with(visual, |editor, _cx| editor.find_panel.is_none()));
    visual.executor().advance_clock(Duration::from_millis(40));
    visual.run_until_parked();
    redraw(visual);
    pane_editor.read_with(visual, |editor, cx| {
        let state = editor.find_panel.as_ref().expect("pane Find panel");
        assert_eq!(state.query.read(cx).display_text(), "alpha");
        assert_eq!(state.matches.len(), 3);
        assert_eq!(state.selected, 1);
    });

    visual.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.on_replace_in_document_action(&crate::components::ReplaceInDocument, window, cx);
        });
    });
    pane_editor.read_with(visual, |editor, _cx| {
        assert!(
            editor
                .find_panel
                .as_ref()
                .is_some_and(|state| state.show_replace)
        );
    });

    // 初次搜索先标记原选区之后的待定位匹配；第一次 F3 应到第二个 alpha。
    visual.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.on_find_next_action(&crate::components::FindNext, window, cx);
        });
    });
    pane_editor.read_with(visual, |editor, cx| {
        assert_eq!(editor.capture_source_selection_snapshot(cx).range(), 11..16);
    });
    visual.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.on_find_next_action(&crate::components::FindNext, window, cx);
        });
    });
    pane_editor.read_with(visual, |editor, _cx| {
        assert_eq!(
            editor
                .find_panel
                .as_ref()
                .expect("pane Find panel")
                .selected,
            2,
            "Next must advance to the match after the selected source text"
        );
    });
    visual.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.on_find_previous_action(&crate::components::FindPrevious, window, cx);
        });
    });
    pane_editor.read_with(visual, |editor, _cx| {
        assert_eq!(
            editor
                .find_panel
                .as_ref()
                .expect("pane Find panel")
                .selected,
            1,
            "Previous must return to the match selected when Find opened"
        );
    });
    assert!(editor.read_with(visual, |editor, _cx| editor.find_panel.is_none()));
}

/// Makes sure a pane-owned Find state has a visible input surface in that pane.
#[gpui::test]
async fn pane_window_find_panel_renders_on_child_surface(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let (editor, visual) =
        cx.add_window_view(|_window, cx| Editor::from_markdown(cx, "alpha beta".to_owned(), None));

    editor.update(visual, |editor, cx| {
        editor.split_pane_toward(crate::editor::panes::PaneSplitDirection::Right, cx)
    });
    visual.run_until_parked();
    redraw(visual);
    let pane_editor = editor.read_with(visual, |editor, cx| {
        let workspace = editor.pane_workspace.as_ref().expect("pane workspace");
        let pane = workspace.read(cx).workspace().focused_pane();
        let canvases = editor.pane_canvas_entities.borrow();
        let (_, _, canvas) = canvases.get(&pane).expect("focused pane canvas");
        match canvas {
            crate::editor::panes::PaneCanvasEntity::Markdown(canvas) => canvas.read(cx).editor(),
            crate::editor::panes::PaneCanvasEntity::DocumentHost(_)
            | crate::editor::panes::PaneCanvasEntity::ReadOnly(_) => {
                panic!("focused Markdown pane expected")
            }
        }
    });

    visual.update(|window, cx| {
        pane_editor.update(cx, |editor, cx| {
            editor.on_find_in_document_action(&crate::components::FindInDocument, window, cx);
        });
    });
    redraw(visual);

    assert!(
        visual.debug_bounds("document-find-input").is_some(),
        "pane-owned Find state must render its input surface"
    );
}

/// Keeps both Split surfaces mounted within the bounds of their pane content.
#[gpui::test]
async fn pane_window_split_surfaces_fit_inside_pane_content(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let (editor, visual) =
        cx.add_window_view(|_window, cx| Editor::from_markdown(cx, "alpha beta".to_owned(), None));

    editor.update(visual, |editor, cx| {
        editor.split_pane_toward(crate::editor::panes::PaneSplitDirection::Right, cx)
    });
    visual.run_until_parked();
    redraw(visual);
    editor.update(visual, |editor, cx| {
        editor.set_view_mode(ViewMode::Split, cx)
    });
    visual.run_until_parked();
    redraw(visual);
    flush_split_projection(visual);
    redraw(visual);

    let pane = visual
        .debug_bounds("pane-content")
        .expect("pane content bounds");
    let source = visual
        .debug_bounds("split-source-pane-shell")
        .expect("Split source bounds");
    let preview = visual
        .debug_bounds("split-preview-pane")
        .expect("Split preview bounds");
    assert!(
        source.left() >= pane.left()
            && source.right() <= pane.right()
            && source.top() >= pane.top()
            && source.bottom() <= pane.bottom(),
        "Split source must stay inside pane content"
    );
    assert!(
        preview.left() >= pane.left()
            && preview.right() <= pane.right()
            && preview.top() >= pane.top()
            && preview.bottom() <= pane.bottom(),
        "Split preview must stay inside pane content"
    );
    assert!(
        source.right() <= preview.left(),
        "Split surfaces must not overlap"
    );
}

/// Ensures a pane child consumes Save requests during its lightweight canvas render path.
#[gpui::test]
async fn pane_window_save_action_is_consumed_and_written(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let path = temp_markdown_path("pane-window-save");
    fs::write(&path, "alpha").expect("write initial Markdown");
    let cleanup_path = path.clone();
    cx.on_quit(move || {
        let _ = fs::remove_file(&cleanup_path);
    });
    let (editor, visual) = cx.add_window_view({
        let path = path.clone();
        move |_window, cx| Editor::from_markdown(cx, "alpha".to_owned(), Some(path))
    });

    editor.update(visual, |editor, cx| {
        editor.split_pane_toward(crate::editor::panes::PaneSplitDirection::Right, cx)
    });
    visual.run_until_parked();
    redraw(visual);
    let pane_editor = editor.read_with(visual, |editor, cx| {
        let workspace = editor.pane_workspace.as_ref().expect("pane workspace");
        let pane = workspace.read(cx).workspace().focused_pane();
        let canvases = editor.pane_canvas_entities.borrow();
        let (_, _, canvas) = canvases.get(&pane).expect("focused pane canvas");
        match canvas {
            crate::editor::panes::PaneCanvasEntity::Markdown(canvas) => canvas.read(cx).editor(),
            crate::editor::panes::PaneCanvasEntity::DocumentHost(_)
            | crate::editor::panes::PaneCanvasEntity::ReadOnly(_) => {
                panic!("focused Markdown pane expected")
            }
        }
    });
    visual.update(|window, cx| {
        pane_editor.update(cx, |editor, cx| {
            let block = editor.document.first_root().expect("root block").clone();
            editor.active_entity_id = Some(block.entity_id());
            block.update(cx, |block, cx| {
                block.focus_handle.focus(window);
                block.selected_range = 5..5;
                block.prepare_undo_capture(crate::components::UndoCaptureKind::CoalescibleText, cx);
                block.replace_text_in_visible_range(5..5, " saved", None, false, cx);
            });
        });
    });
    visual.run_until_parked();
    redraw(visual);
    let expected = pane_editor.read_with(visual, |editor, cx| {
        assert!(editor.source_document.is_dirty());
        editor.document.markdown_text(cx)
    });
    assert_ne!(expected, "alpha");

    visual.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.on_save_document(&crate::components::SaveDocument, window, cx);
        });
    });
    redraw(visual);

    assert_eq!(
        fs::read_to_string(&path).expect("read saved Markdown"),
        expected
    );
    pane_editor.read_with(visual, |editor, _cx| {
        assert!(
            !editor.pending_save,
            "pane render must consume pending Save"
        );
        assert!(!editor.source_document.is_dirty());
    });
}

/// Saves an actual pane edit after its idle deadline and keeps a later window Save conflict-free.
#[gpui::test]
async fn pane_window_auto_save_then_ctrl_s_uses_refreshed_file_identity(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    cx.update(|cx| {
        crate::config::EditorSettings::init(
            cx,
            true,
            crate::config::AutoSavePreference::AfterDelay,
            true,
        );
    });
    let path = temp_markdown_path("pane-window-auto-save");
    fs::write(&path, "alpha").expect("write initial Markdown");
    let cleanup_path = path.clone();
    cx.on_quit(move || {
        let _ = fs::remove_file(&cleanup_path);
    });
    let (editor, visual) = cx.add_window_view({
        let path = path.clone();
        move |_window, cx| Editor::from_markdown(cx, "alpha".to_owned(), Some(path))
    });

    editor.update(visual, |editor, cx| {
        editor.split_pane_toward(crate::editor::panes::PaneSplitDirection::Right, cx)
    });
    visual.run_until_parked();
    redraw(visual);
    let pane_editor = editor.read_with(visual, |editor, cx| {
        let workspace = editor.pane_workspace.as_ref().expect("pane workspace");
        let pane = workspace.read(cx).workspace().focused_pane();
        let canvases = editor.pane_canvas_entities.borrow();
        let (_, _, canvas) = canvases.get(&pane).expect("focused pane canvas");
        match canvas {
            crate::editor::panes::PaneCanvasEntity::Markdown(canvas) => canvas.read(cx).editor(),
            crate::editor::panes::PaneCanvasEntity::DocumentHost(_)
            | crate::editor::panes::PaneCanvasEntity::ReadOnly(_) => {
                panic!("focused Markdown pane expected")
            }
        }
    });

    visual.update(|window, cx| {
        pane_editor.update(cx, |editor, cx| {
            let block = editor.document.first_root().expect("root block").clone();
            editor.active_entity_id = Some(block.entity_id());
            block.update(cx, |block, cx| {
                block.focus_handle.focus(window);
                block.selected_range = 5..5;
                block.prepare_undo_capture(crate::components::UndoCaptureKind::CoalescibleText, cx);
                block.replace_text_in_visible_range(5..5, " idle", None, false, cx);
            });
        });
    });
    visual.run_until_parked();
    redraw(visual);
    let expected = pane_editor.read_with(visual, |editor, cx| {
        assert!(editor.source_document.is_dirty());
        assert!(
            editor.auto_save_task.is_some(),
            "pane edits must schedule auto-save"
        );
        editor.document.markdown_text(cx)
    });

    visual.executor().advance_clock(Duration::from_secs(1));
    visual.run_until_parked();
    redraw(visual);
    visual.run_until_parked();
    redraw(visual);
    assert_eq!(
        fs::read_to_string(&path).expect("read auto-saved Markdown"),
        expected
    );
    pane_editor.read_with(visual, |editor, _cx| {
        assert!(!editor.source_document.is_dirty());
        assert!(editor.auto_save_task.is_none());
        assert!(!editor.external_file_conflict);
    });

    visual.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.on_save_document(&crate::components::SaveDocument, window, cx);
        });
    });
    visual.run_until_parked();
    redraw(visual);
    visual.run_until_parked();
    redraw(visual);
    pane_editor.read_with(visual, |editor, _cx| {
        assert!(!editor.external_file_conflict);
        assert!(!editor.show_external_conflict_dialog);
        assert!(!editor.source_document.is_dirty());
    });
    assert_eq!(
        fs::read_to_string(&path).expect("read Markdown after Ctrl+S"),
        expected
    );
}

/// Confines a pane-owned conflict dialog and its actions to the pane content bounds.
#[gpui::test]
async fn pane_window_conflict_dialog_renders_inside_child_bounds(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    for direction in [
        crate::editor::panes::PaneSplitDirection::Right,
        crate::editor::panes::PaneSplitDirection::Down,
    ] {
        let (editor, visual) =
            cx.add_window_view(|_window, cx| Editor::from_markdown(cx, "alpha".to_owned(), None));

        editor.update(visual, |editor, cx| editor.split_pane_toward(direction, cx));
        visual.run_until_parked();
        redraw(visual);
        let pane_editor = editor.read_with(visual, |editor, cx| {
            let workspace = editor.pane_workspace.as_ref().expect("pane workspace");
            let pane = workspace.read(cx).workspace().focused_pane();
            let canvases = editor.pane_canvas_entities.borrow();
            let (_, _, canvas) = canvases.get(&pane).expect("focused pane canvas");
            match canvas {
                crate::editor::panes::PaneCanvasEntity::Markdown(canvas) => {
                    canvas.read(cx).editor()
                }
                crate::editor::panes::PaneCanvasEntity::DocumentHost(_)
                | crate::editor::panes::PaneCanvasEntity::ReadOnly(_) => {
                    panic!("focused Markdown pane expected")
                }
            }
        });
        pane_editor.update(visual, |editor, cx| {
            editor.show_external_conflict_dialog = true;
            editor.external_conflict_preview = Some(crate::editor::ExternalConflictPreview {
                path: "notes/alpha.md".to_owned(),
                first_difference_line: Some(1),
                local_line: "local text".to_owned(),
                disk_line: "disk text".to_owned(),
                local_line_count: 1,
                disk_line_count: 1,
                local_bytes: 10,
                disk_bytes: 9,
                disk_error: None,
            });
            cx.notify();
        });
        redraw(visual);

        let pane = visual
            .debug_bounds("pane-content")
            .expect("pane content bounds");
        let overlay = visual
            .debug_bounds("external-conflict-overlay")
            .expect("pane-owned conflict overlay");
        let dialog = visual
            .debug_bounds("external-conflict-dialog")
            .expect("pane-owned conflict dialog");
        assert!(overlay.left() >= pane.left() && overlay.right() <= pane.right());
        assert!(overlay.top() >= pane.top() && overlay.bottom() <= pane.bottom());
        assert!(dialog.left() >= pane.left() && dialog.right() <= pane.right());
        assert!(dialog.top() >= pane.top() && dialog.bottom() <= pane.bottom());
        for selector in [
            "cancel-external-conflict",
            "reload-external-conflict",
            "overwrite-external-conflict",
            "save-as-external-conflict",
        ] {
            let action = visual
                .debug_bounds(selector)
                .unwrap_or_else(|| panic!("missing pane conflict action: {selector}"));
            assert!(action.left() >= dialog.left(), "{selector} escaped left");
            assert!(action.right() <= dialog.right(), "{selector} escaped right");
            assert!(action.top() >= dialog.top(), "{selector} escaped top");
            assert!(
                action.bottom() <= dialog.bottom(),
                "{selector} escaped bottom"
            );
        }
    }
}

/// Keeps the original selection direction when Undo restores a pane child's pre-edit selection.
#[gpui::test]
async fn pane_window_undo_restores_original_selection(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let (editor, visual) =
        cx.add_window_view(|_window, cx| Editor::from_markdown(cx, "alpha".to_owned(), None));

    editor.update(visual, |editor, cx| {
        editor.split_pane_toward(crate::editor::panes::PaneSplitDirection::Right, cx)
    });
    visual.run_until_parked();
    redraw(visual);
    let pane_editor = editor.read_with(visual, |editor, cx| {
        let workspace = editor.pane_workspace.as_ref().expect("pane workspace");
        let pane = workspace.read(cx).workspace().focused_pane();
        let canvases = editor.pane_canvas_entities.borrow();
        let (_, _, canvas) = canvases.get(&pane).expect("focused pane canvas");
        match canvas {
            crate::editor::panes::PaneCanvasEntity::Markdown(canvas) => canvas.read(cx).editor(),
            crate::editor::panes::PaneCanvasEntity::DocumentHost(_)
            | crate::editor::panes::PaneCanvasEntity::ReadOnly(_) => {
                panic!("focused Markdown pane expected")
            }
        }
    });

    visual.update(|window, cx| {
        pane_editor.update(cx, |editor, cx| {
            let block = editor.document.first_root().expect("root block").clone();
            editor.active_entity_id = Some(block.entity_id());
            block.update(cx, |block, _cx| {
                block.focus_handle.focus(window);
                block.selected_range = 1..4;
                block.selection_reversed = true;
            });
        });
    });
    visual.run_until_parked();
    redraw(visual);

    visual.update(|window, cx| {
        pane_editor.update(cx, |editor, cx| {
            let block = editor.document.first_root().expect("root block").clone();
            block.update(cx, |block, cx| {
                block.focus_handle.focus(window);
                block.prepare_undo_capture(crate::components::UndoCaptureKind::CoalescibleText, cx);
                block.replace_text_in_visible_range(1..4, "X", None, false, cx);
            });
        });
    });
    visual.run_until_parked();
    redraw(visual);

    visual.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.on_undo(&crate::components::Undo, window, cx);
        });
    });
    redraw(visual);

    pane_editor.read_with(visual, |editor, cx| {
        assert_eq!(editor.document.markdown_text(cx), "alpha");
        let block = editor.document.first_root().expect("restored root block");
        assert_eq!(block.read(cx).selected_range, 1..4);
        assert!(block.read(cx).selection_reversed);
    });
}
