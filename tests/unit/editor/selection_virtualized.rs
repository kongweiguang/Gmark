// @author kongweiguang

use gpui::{AppContext, TestAppContext, VisualTestContext, point, px};

use super::{CrossBlockSelectionEndpoint, Editor, SelectionSurface};
use crate::editor::ViewMode;
use crate::i18n::I18nManager;
use crate::theme::ThemeManager;
use crate::ui::actions::{
    MoveToDocumentEnd, MoveToDocumentStart, SelectToDocumentEnd, SelectToDocumentStart,
};

/// Initializes the same globals used by rendered editor tests without pulling in the app shell.
fn init_editor_test_app(cx: &mut TestAppContext) {
    cx.update(|cx| {
        I18nManager::init(cx);
        ThemeManager::init(cx);
        crate::components::init(cx);
    });
}

/// 后台投影尚未追上正文时，显式跳转仍须定位最新全文；不能吞掉这次按键。
#[gpui::test]
async fn virtual_document_boundary_navigation_refreshes_stale_projection(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let source = (0..600)
        .map(|index| format!("paragraph {index:04}"))
        .collect::<Vec<_>>()
        .join("\n\n");
    let (editor, visual) =
        cx.add_window_view(move |_window, cx| Editor::from_markdown_virtualized(cx, source, None));
    redraw(visual);
    editor.update_in(visual, |editor, window, cx| {
        let first = editor.document.first_root().expect("first").clone();
        first.read(cx).focus_handle.focus(window);
        let snapshot = editor.source_document.snapshot();
        editor
            .source_document
            .apply_transaction(gmark_document::Transaction::new(
                snapshot.revision(),
                vec![gmark_document::TextEdit::new(
                    editor.source_document.len()..editor.source_document.len(),
                    "\n\nfresh tail".to_owned(),
                )],
            ))
            .expect("source edit");
        assert_ne!(
            editor
                .virtual_surface
                .as_ref()
                .expect("surface")
                .projection_revision(),
            editor.source_document.revision()
        );
        editor.on_move_to_document_end(&MoveToDocumentEnd, window, cx);
    });
    redraw(visual);
    editor.update_in(visual, |editor, window, cx| {
        let (_, focused) = editor
            .focused_document_target(window, cx)
            .expect("latest end");
        assert_eq!(focused.read(cx).display_text(), "fresh tail");
        assert_eq!(
            editor
                .virtual_surface
                .as_ref()
                .expect("surface")
                .projection_revision(),
            editor.source_document.revision()
        );
    });
}

/// 文档首尾必须按完整源码定位；虚拟视口末尾不能冒充文档末尾，Preview 也应能扩选全文。
#[gpui::test]
async fn virtual_document_boundary_navigation_and_selection_reach_full_source(
    cx: &mut TestAppContext,
) {
    init_editor_test_app(cx);
    for mode in [ViewMode::Rendered, ViewMode::Preview] {
        let source = (0..600)
            .map(|index| format!("paragraph {index:04}"))
            .collect::<Vec<_>>()
            .join("\n\n");
        let source_len = source.len();
        let expected = source.clone();
        let (editor, visual) = cx.add_window_view(move |_window, cx| {
            Editor::from_markdown_virtualized(cx, source, None)
        });
        redraw(visual);
        editor.update_in(visual, |editor, window, cx| {
            editor.view_mode = mode;
            if mode == ViewMode::Preview {
                editor.set_projection_read_only(true, cx);
            }
            let first = editor.document.first_root().expect("first block").clone();
            first.read(cx).focus_handle.focus(window);
            editor.on_move_to_document_end(&MoveToDocumentEnd, window, cx);
        });
        redraw(visual);
        editor.update_in(visual, |editor, window, cx| {
            let (_, focused) = editor
                .focused_document_target(window, cx)
                .expect("document end focus");
            assert_eq!(
                focused.read(cx).display_text(),
                "paragraph 0599",
                "Ctrl+End reaches the full document"
            );
            assert_eq!(
                focused.read(cx).cursor_offset(),
                focused.read(cx).visible_len()
            );
            editor.on_select_to_document_start(&SelectToDocumentStart, window, cx);
        });
        redraw(visual);
        editor.update_in(visual, |editor, window, cx| {
            let selection = editor
                .cross_block_selection
                .expect("reverse document selection");
            assert_eq!(
                selection
                    .source_anchor
                    .expect("end source anchor")
                    .byte_offset,
                source_len
            );
            assert_eq!(
                selection
                    .source_focus
                    .expect("start source focus")
                    .byte_offset,
                0
            );
            assert_eq!(editor.source_document.text(), expected);
            editor.on_move_to_document_start(&MoveToDocumentStart, window, cx);
        });
        redraw(visual);
        editor.update_in(visual, |editor, window, cx| {
            let (_, focused) = editor
                .focused_document_target(window, cx)
                .expect("document start focus");
            assert_eq!(focused.read(cx).display_text(), "paragraph 0000");
            assert_eq!(focused.read(cx).cursor_offset(), 0);
            editor.on_select_to_document_end(&SelectToDocumentEnd, window, cx);
        });
        redraw(visual);
        editor.read_with(visual, |editor, _cx| {
            let selection = editor
                .cross_block_selection
                .expect("forward document selection");
            assert_eq!(
                selection
                    .source_anchor
                    .expect("start source anchor")
                    .byte_offset,
                0
            );
            assert_eq!(
                selection
                    .source_focus
                    .expect("end source focus")
                    .byte_offset,
                source_len
            );
            assert_eq!(editor.source_document.text(), expected);
            assert!(
                !editor.document_dirty,
                "navigation does not modify the document"
            );
        });
    }
}

/// Reconciles the virtual viewport after each scroll so the regression crosses a real mount boundary.
fn redraw(cx: &mut VisualTestContext) {
    cx.update(|window, cx| window.draw(cx).clear());
    cx.run_until_parked();
}

/// Keeps a drag's source anchor usable after its block leaves the viewport, including reverse selection and copy.
#[gpui::test]
async fn virtualized_cross_block_drag_survives_anchor_unmount_in_both_directions_and_copies_all_text(
    cx: &mut TestAppContext,
) {
    init_editor_test_app(cx);
    let source = (0..10_000)
        .map(|index| format!("paragraph {index:05}"))
        .collect::<Vec<_>>()
        .join("\n\n");
    let expected_source = source.clone();
    let (editor, visual) =
        cx.add_window_view(move |_window, cx| Editor::from_markdown_virtualized(cx, source, None));
    redraw(visual);

    let first_id = editor.update(visual, |editor, _cx| {
        let first = editor
            .document
            .first_root()
            .expect("first viewport block")
            .clone();
        let first_id = first.entity_id();
        editor.active_entity_id = Some(first_id);
        let anchor = CrossBlockSelectionEndpoint {
            entity_id: first_id,
            offset: 0,
        };
        editor.cross_block_drag =
            Some(editor.cross_block_drag_from_endpoint(SelectionSurface::Main, anchor, _cx));
        let max_y = f32::from(editor.scroll_handle.max_offset().height.max(px(0.0)));
        editor
            .scroll_handle
            .set_offset(point(px(0.0), px(-max_y * 0.75)));
        first_id
    });
    redraw(visual);

    editor.update(visual, |editor, cx| {
        let surface = editor.virtual_surface.as_ref().expect("virtual surface");
        assert!(
            surface.entity_by_id(first_id).is_some(),
            "the active anchor remains pinned"
        );
        assert!(editor.document.block_entity_by_id(first_id).is_none());

        let focus = editor
            .document
            .visible_blocks()
            .last()
            .expect("far viewport focus")
            .entity
            .clone();
        let focus_text = focus.read(cx).display_text().to_owned();
        let far_end = focus.read(cx).visible_len();
        editor.apply_surface_pointer_selection_to_endpoint(
            SelectionSurface::Main,
            CrossBlockSelectionEndpoint {
                entity_id: focus.entity_id(),
                offset: far_end,
            },
            cx,
        );

        assert!(editor.cross_block_selection.is_some());
        let selection = editor
            .cross_block_selection
            .expect("captured source selection");
        let revision = editor.source_document.snapshot().revision();
        assert_eq!(
            selection.source_anchor.map(|anchor| anchor.byte_offset),
            Some(0)
        );
        assert!(
            selection
                .source_focus
                .is_some_and(|anchor| anchor.revision == revision)
        );
        let copied = editor
            .selected_visible_text_for_target(SelectionSurface::Main, &focus, cx)
            .expect("copy should include the complete virtualized selection");
        assert!(copied.starts_with("paragraph 00000"));
        assert!(copied.contains("paragraph 03000"));
        assert!(copied.ends_with(&focus_text));
    });

    editor.update(visual, |editor, _cx| {
        editor.scroll_handle.set_offset(point(px(0.0), px(0.0)));
    });
    redraw(visual);
    editor.update(visual, |editor, cx| {
        let visible = editor.document.visible_blocks();
        assert!(!visible.is_empty());
        assert!(
            visible
                .iter()
                .all(|block| block.entity.read(cx).editor_selection_range.is_some()),
            "scrolling a virtualized selection to a new mount window must restore its highlights"
        );
    });

    // 反向手势必须从实际可见的文字重新按下；高亮验收已卸载此前的远端实体。
    editor.update(visual, |editor, _cx| {
        let max_y = f32::from(editor.scroll_handle.max_offset().height.max(px(0.0)));
        editor
            .scroll_handle
            .set_offset(point(px(0.0), px(-max_y * 0.75)));
    });
    redraw(visual);
    let (far_id, far_end, far_text) = editor.read_with(visual, |editor, cx| {
        let far = editor
            .document
            .visible_blocks()
            .last()
            .expect("visible reverse anchor");
        let block = far.entity.read(cx);
        (
            far.entity.entity_id(),
            block.visible_len(),
            block.display_text().to_owned(),
        )
    });

    editor.update(visual, |editor, cx| {
        editor.clear_cross_block_selection(cx);
        editor.active_entity_id = Some(far_id);
        let anchor = CrossBlockSelectionEndpoint {
            entity_id: far_id,
            offset: far_end,
        };
        editor.cross_block_drag =
            Some(editor.cross_block_drag_from_endpoint(SelectionSurface::Main, anchor, cx));
        editor.scroll_handle.set_offset(point(px(0.0), px(0.0)));
    });
    redraw(visual);

    editor.update(visual, |editor, cx| {
        let surface = editor.virtual_surface.as_ref().expect("virtual surface");
        assert!(
            surface.entity_by_id(far_id).is_some(),
            "the reverse-drag anchor remains pinned"
        );
        assert!(editor.document.block_entity_by_id(far_id).is_none());

        let focus = editor
            .document
            .first_root()
            .expect("top viewport focus")
            .clone();
        editor.apply_surface_pointer_selection_to_endpoint(
            SelectionSurface::Main,
            CrossBlockSelectionEndpoint {
                entity_id: focus.entity_id(),
                offset: 0,
            },
            cx,
        );

        assert!(editor.cross_block_selection.is_some());
        let selection = editor
            .cross_block_selection
            .expect("captured reverse selection");
        let revision = editor.source_document.snapshot().revision();
        let source_anchor = selection.source_anchor.expect("reverse drag source anchor");
        let source_focus = selection.source_focus.expect("reverse focus source anchor");
        assert!(source_anchor.byte_offset > source_focus.byte_offset);
        assert_eq!(source_anchor.revision, revision);
        assert_eq!(source_focus.revision, revision);
        let copied = editor
            .selected_visible_text_for_target(SelectionSurface::Main, &focus, cx)
            .expect("reverse copy should preserve the original source interval");
        assert!(copied.starts_with("paragraph 00000"));
        assert!(copied.contains("paragraph 03000"));
        assert!(copied.ends_with(&far_text));

        let selection = editor
            .normalized_cross_block_selection(cx)
            .expect("normalized selection");
        let source_range = editor
            .cross_block_source_range_for_normalized(selection, cx)
            .expect("revision-checked source range");
        let before_delete = editor.source_document.text();
        assert_eq!(before_delete, expected_source);
        let after_delete = format!(
            "{}{}",
            &before_delete[..source_range.start],
            &before_delete[source_range.end..]
        );

        assert!(editor.delete_cross_block_selection(cx));
        assert_eq!(editor.source_document.text(), after_delete);
        editor.undo_document(cx);
        assert_eq!(editor.source_document.text(), before_delete);
        let restored = editor
            .cross_block_selection
            .expect("undo restores source selection");
        let revision = editor.source_document.snapshot().revision();
        assert_eq!(
            restored.source_anchor.map(|anchor| anchor.byte_offset),
            Some(source_range.end)
        );
        assert_eq!(
            restored.source_focus.map(|anchor| anchor.byte_offset),
            Some(source_range.start)
        );
        assert!(
            restored
                .source_anchor
                .is_some_and(|anchor| anchor.revision == revision)
        );
        assert!(
            restored
                .source_focus
                .is_some_and(|anchor| anchor.revision == revision)
        );
        editor.redo_document(cx);
        assert_eq!(editor.source_document.text(), after_delete);
    });
}

/// Keeps formatting independent of endpoint mounting, while one undo restores the whole source interval.
#[gpui::test]
async fn virtualized_inline_formatting_edits_unmounted_blocks_as_one_undo_group(
    cx: &mut TestAppContext,
) {
    init_editor_test_app(cx);
    let source = (0..1_200)
        .map(|index| format!("paragraph {index:05}"))
        .collect::<Vec<_>>()
        .join("\n\n");
    let original = source.clone();
    let (editor, visual) =
        cx.add_window_view(move |_window, cx| Editor::from_markdown_virtualized(cx, source, None));
    redraw(visual);

    let first_id = editor.update(visual, |editor, cx| {
        let first = editor
            .document
            .first_root()
            .expect("first viewport block")
            .clone();
        let first_id = first.entity_id();
        let max_y = f32::from(editor.scroll_handle.max_offset().height.max(px(0.0)));
        editor.active_entity_id = Some(first_id);
        editor.cross_block_drag = Some(editor.cross_block_drag_from_endpoint(
            SelectionSurface::Main,
            CrossBlockSelectionEndpoint {
                entity_id: first_id,
                offset: 0,
            },
            cx,
        ));
        editor
            .scroll_handle
            .set_offset(point(px(0.0), px(-max_y * 0.75)));
        first_id
    });
    redraw(visual);

    let far_id = editor.update(visual, |editor, cx| {
        let surface = editor.virtual_surface.as_ref().expect("virtual surface");
        assert!(surface.entity_by_id(first_id).is_some());
        assert!(editor.document.block_entity_by_id(first_id).is_none());
        let focus = editor
            .document
            .visible_blocks()
            .last()
            .expect("far viewport focus")
            .entity
            .clone();
        editor.apply_surface_pointer_selection_to_endpoint(
            SelectionSurface::Main,
            CrossBlockSelectionEndpoint {
                entity_id: focus.entity_id(),
                offset: focus.read(cx).visible_len(),
            },
            cx,
        );
        editor.cross_block_drag = None;
        editor.scroll_handle.set_offset(point(px(0.0), px(0.0)));
        focus.entity_id()
    });
    redraw(visual);

    let formatted = editor.update(visual, |editor, cx| {
        assert!(
            editor
                .virtual_surface
                .as_ref()
                .expect("virtual surface")
                .entity_by_id(far_id)
                .is_none(),
            "the original far endpoint must actually unmount before formatting"
        );
        assert!(
            editor.apply_cross_block_inline_command(crate::components::EditingCommandId::Bold, cx,)
        );
        editor.source_document.text()
    });

    assert!(formatted.starts_with("**paragraph 00000**"));
    assert!(formatted.contains("**paragraph 00300**"));
    editor.update(visual, |editor, cx| {
        editor.undo_document(cx);
        assert_eq!(editor.source_document.text(), original);
        editor.redo_document(cx);
        assert_eq!(editor.source_document.text(), formatted);
        // 重做后的第二次命令仍必须覆盖原选区，不能缩到重建投影的当前视口。
        assert!(
            editor.apply_cross_block_inline_command(crate::components::EditingCommandId::Bold, cx)
        );
        assert_eq!(editor.source_document.text(), original);
        editor.undo_document(cx);
        assert_eq!(editor.source_document.text(), formatted);
    });
}
