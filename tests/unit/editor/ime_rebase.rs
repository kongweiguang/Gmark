// @author kongweiguang

use crate::components::BlockRecord;
use crate::editor::{
    CrossBlockDrag, CrossBlockSelection, CrossBlockSelectionEndpoint, Editor, ViewMode,
};
use crate::i18n::I18nManager;
use gpui::{AppContext, CompositionEnd, EntityInputHandler, TestAppContext};

/// Each independent rebase case initializes the UI globals it renders through.
fn init(cx: &mut TestAppContext) {
    cx.update(|cx| {
        I18nManager::init(cx);
        crate::theme::ThemeManager::init(cx);
        crate::components::init(cx);
    });
}

#[gpui::test]
/// A projection replacement must preserve the native input owner and remap editor-held IDs.
async fn shared_rebase_keeps_original_input_entity_and_references(cx: &mut TestAppContext) {
    init(cx);
    let visual = cx.add_empty_window();
    let editor = visual.new(|cx| Editor::from_markdown(cx, "alpha".into(), None));

    visual.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            let original = editor.document.first_root().unwrap().clone();
            let original_id = original.entity_id();
            original.update(cx, |block, cx| {
                block.focus_handle.focus(window);
                block.selected_range = 5..5;
                block.replace_and_mark_text_in_range(None, "n", Some(1..1), window, cx);
            });

            let replacement = Editor::new_block(cx, BlockRecord::paragraph("alpha"));
            let replacement_id = replacement.entity_id();
            editor.document.replace_roots(vec![replacement.clone()], cx);
            editor.active_entity_id = Some(replacement_id);
            editor.pending_focus = Some(replacement_id);
            editor.cross_block_selection = Some(CrossBlockSelection {
                anchor: CrossBlockSelectionEndpoint {
                    entity_id: replacement_id,
                    offset: 0,
                },
                focus: CrossBlockSelectionEndpoint {
                    entity_id: replacement_id,
                    offset: 5,
                },
                source_anchor: None,
                source_focus: None,
            });
            editor.cross_block_drag = Some(CrossBlockDrag {
                anchor: CrossBlockSelectionEndpoint {
                    entity_id: replacement_id,
                    offset: 0,
                },
                source_anchor: None,
            });

            assert!(editor.retain_shared_ime_input_entity(&replacement, &original, cx));
            assert_eq!(
                editor.document.first_root().unwrap().entity_id(),
                original_id
            );
            assert_eq!(editor.active_entity_id, Some(original_id));
            assert_eq!(editor.pending_focus, Some(original_id));
            let selection = editor.cross_block_selection.unwrap();
            assert_eq!(selection.anchor.entity_id, original_id);
            assert_eq!(selection.focus.entity_id, original_id);
            assert_eq!(
                editor.cross_block_drag.unwrap().anchor.entity_id,
                original_id
            );
            assert!(original.read(cx).has_ime_composition());
        });
    });
}

#[gpui::test]
/// Rebuilding a native table must rebind its runtime cell and registry to the pinned input entity.
async fn shared_rebase_rebinds_table_cell_runtime_and_registry(cx: &mut TestAppContext) {
    init(cx);
    let visual = cx.add_empty_window();
    let editor =
        visual.new(|cx| Editor::from_markdown(cx, "| name |\n| --- |\n| a |".into(), None));

    visual.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            let table = editor.document.first_root().unwrap().clone();
            let cell = table.read(cx).table_runtime.as_ref().unwrap().rows[0][0].clone();
            let cell_id = cell.entity_id();
            cell.update(cx, |block, cx| {
                block.focus_handle.focus(window);
                let end = block.display_text().len();
                block.selected_range = end..end;
                block.replace_and_mark_text_in_range(None, "n", Some(1..1), window, cx);
            });

            let peer = editor.source_document.fork_view().unwrap();
            let end = editor.source_document.text().len();
            peer.apply_transaction(gmark_document::Transaction::new(
                peer.revision(),
                vec![gmark_document::TextEdit::new(end..end, "\nfooter")],
            ))
            .unwrap();
            editor.sync_shared_document_events(cx);

            let current_table = editor.document.first_root().unwrap().clone();
            let current_cell =
                current_table.read(cx).table_runtime.as_ref().unwrap().rows[0][0].clone();
            assert_eq!(current_cell.entity_id(), cell_id);
            assert_eq!(
                editor.table_cells.get(&cell_id).unwrap().cell.entity_id(),
                cell_id
            );
            assert!(cell.read(cx).has_ime_composition());

            cell.update(cx, |block, cx| {
                block.replace_text_in_range(None, "你", window, cx);
                block.composition_ended(CompositionEnd::Committed, window, cx);
            });
        });
    });
    visual.run_until_parked();

    let source = editor.read_with(visual, |editor, _cx| editor.source_document.text());
    assert!(source.contains("a你"));
    assert!(source.contains("footer"));
}

#[gpui::test]
/// A peer edit over the replacement range must reject the late native result without changing it.
async fn shared_rebase_rejects_conflicted_late_ime_commit(cx: &mut TestAppContext) {
    init(cx);
    let visual = cx.add_empty_window();
    let editor = visual.new(|cx| Editor::from_markdown(cx, "alpha beta".into(), None));

    visual.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.set_view_mode(ViewMode::Source, cx);
            let original = editor.document.first_root().unwrap().clone();
            original.update(cx, |block, cx| {
                block.focus_handle.focus(window);
                block.selected_range = 6..8;
                block.replace_and_mark_text_in_range(None, "ni", Some(2..2), window, cx);
            });

            let peer = editor.source_document.fork_view().unwrap();
            peer.apply_transaction(gmark_document::Transaction::new(
                peer.revision(),
                vec![gmark_document::TextEdit::new(6..10, "new")],
            ))
            .unwrap();
            editor.sync_shared_document_events(cx);

            assert!(original.read(cx).ime_reject_until_end);
            original.update(cx, |block, cx| {
                block.replace_text_in_range(None, "你", window, cx);
                block.composition_ended(CompositionEnd::Committed, window, cx);
            });
            assert_eq!(editor.source_document.text(), "alpha new");
        });
    });
    visual.run_until_parked();

    assert_eq!(
        editor.read_with(visual, |editor, _cx| editor.source_document.text()),
        "alpha new"
    );
}
