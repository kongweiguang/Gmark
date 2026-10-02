// @author kongweiguang

use super::super::ResourceTitleDialogState;
use super::{ContextMenuCommand, ContextMenuState, Editor, TableInsertTarget};
use gpui::{AppContext, ClickEvent, KeyDownEvent, Keystroke, Point, TestAppContext, px, size};

fn init_context_menu_test_app(cx: &mut TestAppContext) {
    cx.update(|cx| {
        crate::i18n::I18nManager::init(cx);
        crate::theme::ThemeManager::init(cx);
        crate::components::init(cx);
    });
}

fn key_event(key: &str) -> KeyDownEvent {
    KeyDownEvent {
        keystroke: Keystroke::parse(key).expect("valid context-menu key"),
        is_held: false,
    }
}

#[gpui::test]
async fn context_menu_keyboard_enters_submenu_and_preserves_caret_focus(cx: &mut TestAppContext) {
    init_context_menu_test_app(cx);
    let (editor, visual) = cx.add_window_view(|window, cx| {
        let editor = Editor::from_markdown(cx, "paragraph".to_owned(), None);
        editor
            .document
            .first_root()
            .expect("paragraph")
            .read(cx)
            .focus_handle
            .focus(window);
        editor
    });
    editor.update_in(visual, |editor, window, cx| {
        editor.open_insert_context_menu(
            Point {
                x: px(710.0),
                y: px(510.0),
            },
            TableInsertTarget::Append,
            cx,
        );
        assert!(editor.handle_context_menu_key(&key_event("down"), window, cx));
        assert_eq!(editor.context_menu_keyboard_item, Some(0));
        assert!(editor.handle_context_menu_key(&key_event("right"), window, cx));
        assert!(editor.insert_context_submenu_open());
        assert_eq!(editor.context_menu_keyboard_submenu_item, Some(0));
        assert!(editor.handle_context_menu_key(&key_event("left"), window, cx));
        assert!(!editor.insert_context_submenu_open());
        assert!(editor.handle_context_menu_key(&key_event("escape"), window, cx));
        assert!(editor.context_menu.is_none());
        assert!(
            editor
                .document
                .first_root()
                .expect("paragraph")
                .read(cx)
                .focus_handle
                .is_focused(window)
        );
    });
}

#[gpui::test]
async fn shift_f10_opens_the_selected_resource_context_menu(cx: &mut TestAppContext) {
    init_context_menu_test_app(cx);
    let (editor, visual) = cx.add_window_view(|window, cx| {
        let editor =
            Editor::from_markdown(cx, "[Spec](./spec.pdf \"gmark:resource\")".to_owned(), None);
        let resource = editor.document.first_root().expect("resource");
        resource.update(cx, |block, _cx| block.resource_selected = true);
        resource.read(cx).focus_handle.focus(window);
        editor
    });

    editor.update_in(visual, |editor, window, cx| {
        assert!(editor.open_resource_context_menu_from_keyboard(
            &key_event("shift-f10"),
            window,
            cx,
        ));
        assert!(matches!(
            editor.context_menu,
            Some(ContextMenuState::Resource { .. })
        ));
        assert_eq!(editor.context_menu_keyboard_item, Some(0));
    });
}

#[gpui::test]
async fn context_menu_keyboard_skips_disabled_table_edges(cx: &mut TestAppContext) {
    init_context_menu_test_app(cx);
    let markdown = "| A | B |\n| --- | --- |\n| 1 | 2 |".to_owned();
    let (editor, visual) =
        cx.add_window_view(move |_window, cx| Editor::from_markdown(cx, markdown, None));
    editor.update_in(visual, |editor, window, cx| {
        let table = editor.document.first_root().expect("table");
        editor.context_menu = Some(ContextMenuState::TableAxis {
            position: Point {
                x: px(710.0),
                y: px(510.0),
            },
            selection: crate::editor::TableAxisSelection {
                table_block_id: table.entity_id(),
                kind: crate::components::TableAxisKind::Column,
                index: 0,
            },
        });
        editor.context_menu_keyboard_item = Some(5);
        assert!(editor.handle_context_menu_key(&key_event("down"), window, cx));
        assert_eq!(
            editor.context_menu_keyboard_item,
            Some(7),
            "disabled move-left command must be skipped"
        );
    });
}

#[gpui::test]
async fn context_menu_keyboard_executes_spelling_suggestion(cx: &mut TestAppContext) {
    init_context_menu_test_app(cx);
    let (spelling_editor, spelling_visual) =
        cx.add_window_view(|_window, cx| Editor::from_markdown(cx, "bad sentnce".to_owned(), None));
    let entity_id = spelling_editor.read_with(spelling_visual, |editor, _cx| {
        editor.document.first_root().expect("root").entity_id()
    });
    spelling_editor.update_in(spelling_visual, |editor, window, cx| {
        editor.context_menu = Some(ContextMenuState::Spelling {
            position: Point {
                x: px(20.0),
                y: px(20.0),
            },
            entity_id,
            diagnostic: crate::spellcheck::SpellingDiagnostic {
                range: 4..11,
                original: "sentnce".to_owned(),
                message: "Unknown word".to_owned(),
                replacements: vec!["sentence".to_owned()],
            },
        });
        assert!(editor.handle_context_menu_key(&key_event("down"), window, cx));
        assert!(editor.handle_context_menu_key(&key_event("enter"), window, cx));
        assert_eq!(
            editor
                .document
                .first_root()
                .expect("root")
                .read(cx)
                .display_text(),
            "bad sentence"
        );
        assert!(editor.context_menu.is_none());
    });
}

#[gpui::test]
async fn insert_context_menu_uses_stable_semantic_icon_slots(cx: &mut TestAppContext) {
    init_context_menu_test_app(cx);
    let (editor, visual) =
        cx.add_window_view(|_window, cx| Editor::from_markdown(cx, "paragraph".to_owned(), None));
    visual.simulate_resize(size(px(720.0), px(520.0)));
    editor.update(visual, |editor, cx| {
        editor.open_insert_context_menu(
            Point {
                x: px(710.0),
                y: px(510.0),
            },
            TableInsertTarget::Append,
            cx,
        );
        editor.set_context_menu_hover_state(true, true, cx);
    });
    visual.update(|window, cx| window.draw(cx).clear());

    for (row_selector, icon_selector) in [
        (
            "editor-context-menu-insert",
            "editor-context-menu-insert-icon",
        ),
        (
            "editor-context-menu-insert-table",
            "editor-context-menu-insert-table-icon",
        ),
        (
            "editor-context-menu-insert-image",
            "editor-context-menu-insert-image-icon",
        ),
        (
            "editor-context-menu-insert-math",
            "editor-context-menu-insert-math-icon",
        ),
        (
            "editor-context-menu-insert-horizontal_rule",
            "editor-context-menu-insert-horizontal_rule-icon",
        ),
    ] {
        let row = visual.debug_bounds(row_selector).unwrap();
        let icon = visual.debug_bounds(icon_selector).unwrap();
        assert_eq!(f32::from(row.size.height), 28.0, "{row_selector}");
        assert_eq!(f32::from(icon.size.width), 18.0, "{icon_selector}");
        assert_eq!(f32::from(icon.size.height), 18.0, "{icon_selector}");
        assert!(icon.left() >= row.left(), "{icon_selector}");
        assert!(icon.right() <= row.right(), "{icon_selector}");
    }
    let panel = visual.debug_bounds("editor-context-menu-panel").unwrap();
    let submenu = visual.debug_bounds("editor-context-menu-submenu").unwrap();
    for (name, bounds) in [("panel", panel), ("submenu", submenu)] {
        assert!(f32::from(bounds.left()) >= 8.0, "{name}");
        assert!(f32::from(bounds.top()) >= 8.0, "{name}");
        assert!(f32::from(bounds.right()) <= 712.0, "{name}");
        assert!(f32::from(bounds.bottom()) <= 512.0, "{name}");
    }
    assert!(submenu.right() <= panel.left());
    visual.update(|window, _cx| assert_eq!(window.scale_factor(), 2.0));
}

#[gpui::test]
async fn context_insert_submenu_executes_every_shared_command_with_undo(cx: &mut TestAppContext) {
    init_context_menu_test_app(cx);
    for (command, expected_kind) in [
        (
            crate::components::EditingCommandId::Image,
            crate::components::BlockKind::Paragraph,
        ),
        (
            crate::components::EditingCommandId::Math,
            crate::components::BlockKind::MathBlock,
        ),
        (
            crate::components::EditingCommandId::HorizontalRule,
            crate::components::BlockKind::Separator,
        ),
    ] {
        let (editor, visual) =
            cx.add_window_view(|_window, cx| Editor::from_markdown(cx, "anchor".to_owned(), None));
        editor.update_in(visual, |editor, window, cx| {
            let anchor = editor.document.first_root().expect("anchor").entity_id();
            editor.open_insert_context_menu(
                Point {
                    x: px(20.0),
                    y: px(20.0),
                },
                TableInsertTarget::After(anchor),
                cx,
            );
            editor.on_context_menu_insert_command(command, &ClickEvent::default(), window, cx);
            let inserted = editor.document.visible_blocks()[1].entity.clone();
            assert_eq!(inserted.read(cx).kind(), expected_kind, "{command:?}");
            assert_eq!(editor.undo_history.len(), 1, "{command:?}");
            editor.undo_document(cx);
            assert_eq!(editor.source_document.text(), "anchor", "{command:?}");
        });
    }
}

#[gpui::test]
async fn context_insert_keyboard_model_matches_shared_command_order(cx: &mut TestAppContext) {
    init_context_menu_test_app(cx);
    let (editor, visual) =
        cx.add_window_view(|_window, cx| Editor::from_markdown(cx, "anchor".to_owned(), None));
    editor.update_in(visual, |editor, window, cx| {
        editor.open_insert_context_menu(
            Point {
                x: px(20.0),
                y: px(20.0),
            },
            TableInsertTarget::Append,
            cx,
        );
        assert!(editor.handle_context_menu_key(&key_event("down"), window, cx));
        assert!(editor.handle_context_menu_key(&key_event("right"), window, cx));
        assert!(editor.handle_context_menu_key(&key_event("end"), window, cx));
        assert_eq!(editor.context_menu_keyboard_submenu_item, Some(6));
        assert!(editor.handle_context_menu_key(&key_event("enter"), window, cx));
        assert!(
            editor.document.visible_blocks().iter().any(
                |block| block.entity.read(cx).kind() == crate::components::BlockKind::Separator
            )
        );
    });
}

#[gpui::test]
async fn table_axis_context_menu_uses_align_move_and_delete_icons(cx: &mut TestAppContext) {
    init_context_menu_test_app(cx);
    let markdown = "| A | B |\n| --- | --- |\n| 1 | 2 |".to_owned();
    let (editor, visual) =
        cx.add_window_view(move |_window, cx| Editor::from_markdown(cx, markdown, None));
    visual.simulate_resize(size(px(720.0), px(520.0)));
    editor.update(visual, |editor, cx| {
        let table = editor.document.first_root().expect("table");
        editor.context_menu = Some(ContextMenuState::TableAxis {
            position: Point {
                x: px(710.0),
                y: px(510.0),
            },
            selection: crate::editor::TableAxisSelection {
                table_block_id: table.entity_id(),
                kind: crate::components::TableAxisKind::Column,
                index: 0,
            },
        });
        cx.notify();
    });
    visual.update(|window, cx| window.draw(cx).clear());

    for selector in [
        "table-axis-align-column-left-icon",
        "table-axis-align-column-center-icon",
        "table-axis-align-column-right-icon",
        "table-axis-move-column-left-icon",
        "table-axis-move-column-right-icon",
        "table-axis-delete-column-icon",
    ] {
        let icon = visual.debug_bounds(selector).unwrap();
        assert_eq!(f32::from(icon.size.width), 18.0, "{selector}");
        assert_eq!(f32::from(icon.size.height), 18.0, "{selector}");
    }
    let panel = visual
        .debug_bounds("table-axis-context-menu-panel")
        .unwrap();
    assert!(f32::from(panel.left()) >= 8.0);
    assert!(f32::from(panel.top()) >= 8.0);
    assert!(f32::from(panel.right()) <= 712.0);
    assert!(f32::from(panel.bottom()) <= 512.0);
}

#[gpui::test]
async fn long_spelling_menu_scrolls_inside_minimum_viewport(cx: &mut TestAppContext) {
    init_context_menu_test_app(cx);
    let (editor, visual) =
        cx.add_window_view(|_window, cx| Editor::from_markdown(cx, "misspelled".to_owned(), None));
    visual.simulate_resize(size(px(720.0), px(520.0)));
    let entity_id = editor.read_with(visual, |editor, _cx| {
        editor.document.first_root().expect("root").entity_id()
    });
    editor.update(visual, |editor, cx| {
        editor.context_menu = Some(ContextMenuState::Spelling {
            position: Point {
                x: px(710.0),
                y: px(510.0),
            },
            entity_id,
            diagnostic: crate::spellcheck::SpellingDiagnostic {
                range: 0..10,
                original: "misspelled".to_owned(),
                message: "Unknown word".to_owned(),
                replacements: (0..40).map(|index| format!("suggestion-{index}")).collect(),
            },
        });
        cx.notify();
    });
    visual.update(|window, cx| window.draw(cx).clear());

    let panel = visual.debug_bounds("editor-spelling-menu-panel").unwrap();
    assert!(f32::from(panel.left()) >= 8.0);
    assert!(f32::from(panel.top()) >= 8.0);
    assert!(f32::from(panel.right()) <= 712.0);
    assert!(f32::from(panel.bottom()) <= 512.0);
    assert!(f32::from(panel.size.height) <= 504.0);
    editor.update_in(visual, |editor, window, cx| {
        assert!(editor.handle_context_menu_key(&key_event("end"), window, cx));
        assert_eq!(editor.context_menu_keyboard_item, Some(39));
    });
    visual.update(|window, cx| window.draw(cx).clear());
    assert!(editor.read_with(visual, |editor, _cx| {
        editor.context_menu_scroll_handle.offset().y < px(0.0)
    }));
    let last = visual
        .debug_bounds("editor-spelling-suggestion-39")
        .unwrap();
    let panel = visual.debug_bounds("editor-spelling-menu-panel").unwrap();
    assert!(last.top() >= panel.top());
    assert!(last.bottom() <= panel.bottom());
}

#[gpui::test]
async fn context_submenu_stays_open_while_crossing_hover_gap(cx: &mut TestAppContext) {
    let editor = cx.new(|cx| Editor::from_markdown(cx, "alpha".to_string(), None));

    editor.update(cx, |editor, cx| {
        editor.open_insert_context_menu(
            Point {
                x: px(24.0),
                y: px(24.0),
            },
            TableInsertTarget::Append,
            cx,
        );

        editor.set_context_menu_hover_state(true, false, cx);
        let Some(ContextMenuState::Insert { submenu_open, .. }) = editor.context_menu.as_ref()
        else {
            panic!("expected insert context menu");
        };
        assert!(*submenu_open);
        assert!(editor.context_menu_submenu_close_task.is_none());

        editor.set_context_menu_hover_state(false, false, cx);
        let Some(ContextMenuState::Insert { submenu_open, .. }) = editor.context_menu.as_ref()
        else {
            panic!("expected insert context menu");
        };
        assert!(*submenu_open);
        assert!(editor.context_menu_submenu_close_task.is_some());

        editor.set_context_menu_hover_state(true, true, cx);
        let Some(ContextMenuState::Insert { submenu_open, .. }) = editor.context_menu.as_ref()
        else {
            panic!("expected insert context menu");
        };
        assert!(*submenu_open);
        assert!(editor.context_menu_submenu_close_task.is_none());
    });
}

#[gpui::test]
async fn spelling_suggestion_replaces_text_as_undoable_block_edit(cx: &mut TestAppContext) {
    cx.update(|cx| {
        crate::i18n::I18nManager::init(cx);
        crate::theme::ThemeManager::init(cx);
        crate::components::init(cx);
    });
    let (editor, visual_cx) =
        cx.add_window_view(|_window, cx| Editor::from_markdown(cx, "bad sentnce".to_owned(), None));
    let entity_id = editor.read_with(visual_cx, |editor, _cx| {
        editor.document.first_root().expect("root").entity_id()
    });

    visual_cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.context_menu = Some(ContextMenuState::Spelling {
                position: Point {
                    x: px(20.0),
                    y: px(20.0),
                },
                entity_id,
                diagnostic: crate::spellcheck::SpellingDiagnostic {
                    range: 4..11,
                    original: "sentnce".to_owned(),
                    message: "Unknown word".to_owned(),
                    replacements: vec!["sentence".to_owned()],
                },
            });
            editor.apply_spelling_suggestion(0, &ClickEvent::default(), window, cx);
        });
    });

    editor.read_with(visual_cx, |editor, cx| {
        assert_eq!(
            editor
                .document
                .first_root()
                .expect("root")
                .read(cx)
                .display_text(),
            "bad sentence"
        );
        assert!(editor.context_menu.is_none());
    });
}

/// Uses the live resource snapshot while checking path canonicalization and undo restoration.
#[gpui::test]
async fn resource_title_dialog_canonicalizes_and_participates_in_undo_redo(
    cx: &mut TestAppContext,
) {
    init_context_menu_test_app(cx);
    let original_source = crate::components::ResourceRecord::from_parts(
        "Old".to_owned(),
        r"dir\spec.pdf".to_owned(),
        None,
        None,
    )
    .to_markdown();
    let editor = cx.new(|cx| Editor::from_markdown(cx, original_source.clone(), None));

    editor.update(cx, |editor, cx| {
        let entity_id = editor.document.first_root().expect("resource").entity_id();
        let previous = editor
            .document
            .first_root()
            .expect("resource")
            .read(cx)
            .record
            .resource
            .clone()
            .expect("resource metadata");
        let input = cx.new(|cx| {
            let mut input = crate::components::Block::with_record(
                cx,
                crate::components::BlockRecord::paragraph(String::new()),
            );
            input.set_source_raw_mode();
            input
        });
        editor.resource_title_dialog = Some(ResourceTitleDialogState {
            entity_id,
            previous,
            input,
        });

        editor.confirm_resource_title_dialog(cx);
        assert_eq!(
            editor.source_document.text(),
            "[spec.pdf](dir/spec.pdf \"gmark:resource\")"
        );
        assert_eq!(editor.undo_history.len(), 1);

        editor.undo_document(cx);
        assert_eq!(editor.source_document.text(), original_source);
        editor.redo_document(cx);
        assert_eq!(
            editor.source_document.text(),
            "[spec.pdf](dir/spec.pdf \"gmark:resource\")"
        );
    });
}

/// Keeps resource editing commands inert on Preview while leaving the title dialog available to cancel.
#[gpui::test]
async fn resource_context_mutations_are_disabled_on_preview_surface(cx: &mut TestAppContext) {
    init_context_menu_test_app(cx);
    let (editor, visual) = cx.add_window_view(|_window, cx| {
        let mut editor =
            Editor::from_markdown(cx, "[Spec](./spec.pdf \"gmark:resource\")".to_owned(), None);
        editor.set_view_mode(super::super::ViewMode::Preview, cx);
        editor
    });

    editor.update_in(visual, |editor, window, cx| {
        let block = editor.document.first_root().expect("resource").clone();
        let entity_id = block.entity_id();
        editor.context_menu = Some(ContextMenuState::Resource {
            position: Point {
                x: px(24.0),
                y: px(24.0),
            },
            entity_id,
        });

        let model = editor.context_menu_command_model(cx);
        for command in [
            ContextMenuCommand::ResourceEditTitle,
            ContextMenuCommand::ResourceReplace,
            ContextMenuCommand::ResourceConvertLink,
            ContextMenuCommand::ResourceDelete,
            ContextMenuCommand::ResourceRelocate,
        ] {
            let entry = model
                .main
                .iter()
                .find(|entry| entry.command == command)
                .expect("resource command should remain visible");
            assert!(!entry.enabled, "{command:?} must be disabled on Preview");
        }
        assert!(model.main.iter().any(|entry| {
            entry.command == ContextMenuCommand::ResourceCopyAddress && entry.enabled
        }));

        let before = (
            editor.source_document.text().to_owned(),
            editor.source_document.revision(),
            editor.undo_history.len(),
        );
        let click = ClickEvent::default();
        editor.edit_resource_title_from_context_menu(&click, window, cx);
        assert!(editor.resource_title_dialog.is_none());
        editor.convert_resource_to_link_from_context_menu(&click, window, cx);
        editor.delete_resource_from_context_menu(&click, window, cx);
        editor.complete_resource_replacement(
            entity_id,
            crate::components::ResourceRecord::from_parts(
                "Spec".to_owned(),
                "./spec.pdf".to_owned(),
                None,
                None,
            ),
            std::path::PathBuf::from("unused.pdf"),
            cx,
        );
        assert!(matches!(
            editor.context_menu.as_ref(),
            Some(ContextMenuState::Resource { .. })
        ));

        let input = cx.new(|cx| {
            let mut input = crate::components::Block::with_record(
                cx,
                crate::components::BlockRecord::paragraph("Updated"),
            );
            input.set_source_raw_mode();
            input
        });
        editor.resource_title_dialog = Some(ResourceTitleDialogState {
            entity_id,
            previous: crate::components::ResourceRecord::from_parts(
                "Spec".to_owned(),
                "./spec.pdf".to_owned(),
                None,
                None,
            ),
            input: input.clone(),
        });
        editor.confirm_resource_title_dialog(cx);
        assert_eq!(
            input.read(cx).display_text(),
            "Updated",
            "a read-only confirmation must preserve the user's title input"
        );
        assert!(editor.resource_title_dialog.is_some());
        assert_eq!(
            (
                editor.source_document.text().to_owned(),
                editor.source_document.revision(),
                editor.undo_history.len(),
            ),
            before
        );
    });
}
