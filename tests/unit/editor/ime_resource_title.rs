// @author kongweiguang

use std::ops::Range;
use std::path::Path;

use gpui::{AppContext, CompositionEnd, Entity, EntityInputHandler, TestAppContext, Window};

use super::{Editor, resource_records_match};
use crate::components::Block;

/// Initializes the real editor, theme, and input services used by title-dialog tests.
fn init_resource_title_ime_app(cx: &mut TestAppContext) {
    cx.update(|cx| {
        crate::i18n::I18nManager::init(cx);
        crate::theme::ThemeManager::init(cx);
        crate::components::init(cx);
    });
}

/// Starts a marked-text update over a known title range without committing source text.
fn begin_preedit(
    input: &Entity<Block>,
    range: Range<usize>,
    text: &str,
    window: &mut Window,
    cx: &mut gpui::Context<Editor>,
) {
    input.update(cx, |input, cx| {
        input.focus_handle.focus(window);
        input.selected_range = range.clone();
        input.replace_and_mark_text_in_range(None, text, Some(range), window, cx);
    });
}

/// Delivers the platform's final candidate and terminal event to the original input entity.
fn finish_preedit(
    input: &Entity<Block>,
    text: &str,
    end: CompositionEnd,
    window: &mut Window,
    cx: &mut gpui::Context<Editor>,
) {
    input.update(cx, |input, cx| {
        input.replace_text_in_range(None, text, window, cx);
        input.composition_ended(end, window, cx);
    });
}

/// Keeps title editing queued while another editor field owns uncommitted IME text.
#[gpui::test]
async fn resource_title_open_waits_for_other_field_commit(cx: &mut TestAppContext) {
    init_resource_title_ime_app(cx);
    let (editor, visual) = cx.add_window_view(|_window, cx| {
        Editor::from_markdown(
            cx,
            "[Spec](https://example.com/spec.pdf \"gmark:resource\")\n\nplain".to_owned(),
            None,
        )
    });

    let (owner, resource_id, previous, source_before, revision_before) =
        editor.update_in(visual, |editor, window, cx| {
            let visible = editor.document.visible_blocks().to_vec();
            assert_eq!(visible.len(), 2);
            let resource = visible[0].entity.clone();
            let owner = visible[1].entity.clone();
            let previous = resource
                .read(cx)
                .record
                .resource
                .clone()
                .expect("resource record");
            let source_before = editor.source_document.text().to_owned();
            let revision_before = editor.source_document.revision();

            editor.refresh_stable_document_snapshot(cx);
            owner.update(cx, |block, cx| {
                block.focus_handle.focus(window);
                block.selected_range = 0..5;
                block.replace_and_mark_text_in_range(None, "ni", Some(0..5), window, cx);
            });
            assert!(owner.read(cx).has_ime_composition());

            editor.request_resource_title_dialog(
                resource.entity_id(),
                previous.clone(),
                window,
                cx,
            );
            assert!(editor.resource_title_dialog.is_none());
            assert_eq!(editor.pending_ime_operations.len(), 1);
            assert_eq!(editor.source_document.text(), source_before);
            assert_eq!(editor.source_document.revision(), revision_before);
            (
                owner,
                resource.entity_id(),
                previous,
                source_before,
                revision_before,
            )
        });

    editor.update_in(visual, |_editor, window, cx| {
        finish_preedit(&owner, "你", CompositionEnd::Committed, window, cx);
    });
    visual.run_until_parked();
    editor.update_in(visual, |editor, window, cx| {
        editor.sync_pending_ime_operations(window, cx);
        let dialog = editor
            .resource_title_dialog
            .as_ref()
            .expect("title dialog opens after the unrelated owner commits");
        assert_eq!(dialog.entity_id, resource_id);
        assert_eq!(dialog.previous.label, previous.label);
        assert_ne!(editor.source_document.text(), source_before);
        assert!(editor.source_document.revision() > revision_before);
    });
}

/// Applies the committed IME candidate as the dialog's sole resource-title transaction.
#[gpui::test]
async fn resource_title_confirmation_uses_committed_candidate(cx: &mut TestAppContext) {
    init_resource_title_ime_app(cx);
    let (editor, visual) = cx.add_window_view(|_window, cx| {
        Editor::from_markdown(
            cx,
            "[Old](https://example.com/old.pdf \"gmark:resource\")".to_owned(),
            None,
        )
    });

    let (entity_id, input, previous, source_before, revision_before, undo_before) = editor
        .update_in(visual, |editor, window, cx| {
            let resource = editor.document.first_root().expect("resource").clone();
            let previous = resource
                .read(cx)
                .record
                .resource
                .clone()
                .expect("resource record");
            let entity_id = resource.entity_id();
            let source_before = editor.source_document.text().to_owned();
            let revision_before = editor.source_document.revision();
            let undo_before = editor.undo_history.len();

            editor.request_resource_title_dialog(entity_id, previous.clone(), window, cx);
            let input = editor
                .resource_title_dialog
                .as_ref()
                .expect("title dialog opened")
                .input
                .clone();
            begin_preedit(&input, 0..3, "xinh", window, cx);
            assert!(input.read(cx).has_ime_composition());

            editor.request_resource_title_confirmation(
                entity_id,
                input.entity_id(),
                &previous,
                window,
                cx,
            );
            assert!(editor.resource_title_dialog.is_some());
            assert_eq!(editor.pending_ime_operations.len(), 1);
            assert_eq!(editor.source_document.text(), source_before);
            assert_eq!(editor.source_document.revision(), revision_before);
            (
                entity_id,
                input,
                previous,
                source_before,
                revision_before,
                undo_before,
            )
        });

    editor.update_in(visual, |_editor, window, cx| {
        finish_preedit(&input, "新标题", CompositionEnd::Committed, window, cx);
    });
    visual.run_until_parked();
    editor.update_in(visual, |editor, window, cx| {
        editor.sync_pending_ime_operations(window, cx);
        assert!(editor.resource_title_dialog.is_none());
        assert!(editor.source_document.text().contains("[新标题]"));
        assert!(editor.source_document.revision() > revision_before);
        assert_eq!(editor.undo_history.len(), undo_before + 1);
        assert!(editor.document_dirty);
        assert!(editor.focusable_entity_by_id(entity_id).is_some());
        assert!(source_before.contains("[Old]"));
        assert_ne!(editor.source_document.text(), source_before);
        assert_eq!(previous.destination, "https://example.com/old.pdf");
    });
}

/// Cancels the dialog only after the IME reports cancellation, without creating document history.
#[gpui::test]
async fn resource_title_cancellation_waits_for_composition_terminal(cx: &mut TestAppContext) {
    init_resource_title_ime_app(cx);
    let (editor, visual) = cx.add_window_view(|_window, cx| {
        Editor::from_markdown(
            cx,
            "[Old](https://example.com/old.pdf \"gmark:resource\")".to_owned(),
            None,
        )
    });

    let (input, previous, source_before, revision_before, undo_before) =
        editor.update_in(visual, |editor, window, cx| {
            let resource = editor.document.first_root().expect("resource").clone();
            let entity_id = resource.entity_id();
            let previous = resource
                .read(cx)
                .record
                .resource
                .clone()
                .expect("resource record");
            let source_before = editor.source_document.text().to_owned();
            let revision_before = editor.source_document.revision();
            let undo_before = editor.undo_history.len();
            editor.request_resource_title_dialog(entity_id, previous.clone(), window, cx);
            let input = editor
                .resource_title_dialog
                .as_ref()
                .expect("title dialog opened")
                .input
                .clone();
            begin_preedit(&input, 0..3, "xin", window, cx);
            editor.request_resource_title_cancellation(
                entity_id,
                input.entity_id(),
                &previous,
                window,
                cx,
            );
            assert!(editor.resource_title_dialog.is_some());
            assert_eq!(editor.pending_ime_operations.len(), 1);
            (input, previous, source_before, revision_before, undo_before)
        });

    editor.update_in(visual, |_editor, window, cx| {
        finish_preedit(&input, "Old", CompositionEnd::Cancelled, window, cx);
    });
    visual.run_until_parked();
    editor.update_in(visual, |editor, window, cx| {
        editor.sync_pending_ime_operations(window, cx);
        assert!(editor.resource_title_dialog.is_none());
        assert_eq!(input.read(cx).display_text(), "Old");
        assert_eq!(editor.source_document.text(), source_before);
        assert_eq!(editor.source_document.revision(), revision_before);
        assert_eq!(editor.undo_history.len(), undo_before);
        assert_eq!(previous.label, "Old");
    });
}

/// Allows runtime base-directory projection while rejecting a changed visible label or destination.
#[::core::prelude::v1::test]
fn resource_snapshot_uses_domain_identity_instead_of_markdown_spelling() {
    let source =
        crate::components::ResourceRecord::parse("[Spec](./spec.pdf \"gmark:resource\")", None)
            .expect("resource record");
    let resolved = source.with_base_dir(Some(Path::new(r"C:\docs")));
    let moved = crate::components::ResourceRecord::from_parts(
        source.label.clone(),
        "./other.pdf".to_owned(),
        source.explicit_kind,
        None,
    );

    assert!(resource_records_match(&source, &resolved));
    assert!(!resource_records_match(&source, &moved));
}
