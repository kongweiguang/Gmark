// @author kongweiguang

use super::super::DeferredImeOperation;
use super::{Editor, init};
use crate::components::block::BlockImeInteraction;
use crate::components::{BlockEvent, BlockRecord};
use crate::editor::ViewMode;
use gpui::{AppContext, CompositionEnd, Context, EntityInputHandler, TestAppContext, Window};

/// Creates a real source-mode undo entry so deferred navigation tests distinguish the two tabs.
fn append_source_text(
    editor: &mut Editor,
    window: &mut Window,
    cx: &mut Context<Editor>,
    text: &str,
) {
    editor.set_view_mode(ViewMode::Source, cx);
    let block = editor
        .document
        .first_root()
        .expect("source mode keeps an editable root")
        .clone();
    let end = block.read(cx).display_text().len();
    editor.refresh_stable_document_snapshot(cx);
    block.update(cx, |block, cx| {
        block.focus_handle.focus(window);
        block.selected_range = end..end;
        block.replace_text_in_range(None, text, window, cx);
    });
}

/// Keeps a delayed pointer hit bound to its source block and tab until IME commits.
#[gpui::test]
async fn ime_defers_pointer_target_selection_across_tab_reorder(cx: &mut TestAppContext) {
    init(cx);
    let visual = cx.add_empty_window();
    let editor = visual.new(|cx| Editor::from_markdown(cx, "alpha\n\nbeta".into(), None));

    visual.update(|_window, cx| {
        editor.update(cx, |editor, cx| {
            assert!(editor.new_untitled_tab(cx));
            assert!(editor.switch_to_tab_index(0, cx));
        });
    });
    visual.run_until_parked();
    let (first_tab_id, other_tab_id) = editor.read_with(visual, |editor, _cx| {
        (editor.tabs.records[0].id, editor.tabs.records[1].id)
    });

    visual.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            assert_eq!(editor.tabs.records[editor.tabs.active].id, first_tab_id);
            let visible = editor.document.visible_blocks();
            assert_eq!(visible.len(), 2);
            let input = visible[0].entity.clone();
            let target = visible[1].entity.clone();
            let revision = editor.source_document.revision();
            let source = editor.source_document.text();

            editor.refresh_stable_document_snapshot(cx);
            input.update(cx, |block, cx| {
                block.focus_handle.focus(window);
                block.selected_range = 1..1;
                block.replace_and_mark_text_in_range(None, "n", Some(1..1), window, cx);
            });
            editor.on_block_event(
                target.clone(),
                &BlockEvent::RequestImeInteraction {
                    interaction: BlockImeInteraction::PointerSelection {
                        clean_offset: 1,
                        click_count: 2,
                        shift: false,
                    },
                },
                cx,
            );
            editor.on_block_event(
                target.clone(),
                &BlockEvent::RequestImeInteraction {
                    interaction: BlockImeInteraction::PointerSelectionEnd,
                },
                cx,
            );

            assert!(input.read(cx).has_ime_composition());
            assert!(input.read(cx).focus_handle.is_focused(window));
            assert!(!target.read(cx).focus_handle.is_focused(window));
            assert_eq!(target.read(cx).selected_range, 0..0);
            assert_eq!(editor.source_document.text(), source);
            assert_eq!(editor.source_document.revision(), revision);
            assert!(!editor.document_dirty);
            assert_eq!(editor.pending_ime_operations.len(), 2);
            assert!(matches!(
                editor.pending_ime_operations.front(),
                Some(DeferredImeOperation::InputInteraction {
                    tab: Some(tab),
                    block: queued_block,
                    interaction: BlockImeInteraction::PointerSelection {
                        click_count: 2,
                        ..
                    },
                    ..
                }) if *tab == first_tab_id && queued_block.entity_id() == target.entity_id()
            ));
            assert!(matches!(
                editor.pending_ime_operations.get(1),
                Some(DeferredImeOperation::InputInteraction {
                    tab: Some(tab),
                    block: queued_block,
                    interaction: BlockImeInteraction::PointerSelectionEnd,
                    ..
                }) if *tab == first_tab_id && queued_block.entity_id() == target.entity_id()
            ));

            assert!(editor.reorder_tab(1, 0, cx));
            assert_eq!(editor.tabs.records[editor.tabs.active].id, first_tab_id);

            input.update(cx, |block, cx| {
                block.replace_text_in_range(None, "你", window, cx);
                block.composition_ended(CompositionEnd::Committed, window, cx);
            });
        });
    });
    visual.run_until_parked();

    visual.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.sync_pending_ime_operations(window, cx)
        });
    });
    visual.run_until_parked();

    visual.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            assert_eq!(editor.tabs.records[editor.tabs.active].id, first_tab_id);
            let target = editor.document.visible_blocks()[1].entity.clone();
            assert_eq!(target.read(cx).selected_range, 0..4);
            assert!(target.read(cx).focus_handle.is_focused(window));
            assert!(!target.read(cx).is_selecting);
            assert!(editor.pending_ime_operations.is_empty());
            assert_eq!(editor.source_document.text(), "a你lpha\n\nbeta");

            let other_index = editor
                .tabs
                .records
                .iter()
                .position(|record| record.id == other_tab_id)
                .expect("other tab remains present after reorder");
            assert!(editor.switch_to_tab_index(other_index, cx));
            assert_eq!(editor.tabs.records[editor.tabs.active].id, other_tab_id);
            assert!(!editor.document_dirty);
            assert!(editor.switch_to_tab_index(1, cx));
            assert_eq!(editor.tabs.records[editor.tabs.active].id, first_tab_id);
            assert_eq!(editor.source_document.text(), "a你lpha\n\nbeta");
            assert_eq!(
                editor.document.visible_blocks()[1]
                    .entity
                    .read(cx)
                    .selected_range,
                0..4
            );
        });
    });
}

/// 脱树目标须在真实结构提交后丢弃；同槽位的新实体不能继承旧指针意图或旧源码权限。
#[gpui::test]
async fn ime_drops_pointer_target_after_block_detaches(cx: &mut TestAppContext) {
    init(cx);
    let visual = cx.add_empty_window();
    let editor = visual.new(|cx| Editor::from_markdown(cx, "alpha\n\nbeta".into(), None));

    visual.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            let visible = editor.document.visible_blocks();
            assert_eq!(visible.len(), 2);
            let input = visible[0].entity.clone();
            let target = visible[1].entity.clone();
            editor.refresh_stable_document_snapshot(cx);
            input.update(cx, |block, cx| {
                block.focus_handle.focus(window);
                block.selected_range = 1..1;
                block.replace_and_mark_text_in_range(None, "n", Some(1..1), window, cx);
            });
            editor.on_block_event(
                target.clone(),
                &BlockEvent::RequestImeInteraction {
                    interaction: BlockImeInteraction::PointerSelection {
                        clean_offset: 1,
                        click_count: 2,
                        shift: false,
                    },
                },
                cx,
            );
            assert_eq!(editor.pending_ime_operations.len(), 1);
            assert!(matches!(
                editor.pending_ime_operations.front(),
                Some(DeferredImeOperation::InputInteraction {
                    block: queued_block,
                    interaction: BlockImeInteraction::PointerSelection {
                        click_count: 2,
                        ..
                    },
                    ..
                }) if queued_block.entity_id() == target.entity_id()
            ));

            let replacement = editor.document.with_structure_mutation(cx, |document, cx| {
                let (_, location) = document
                    .remove_block_by_id_raw(target.entity_id(), cx)
                    .expect("queued pointer target is attached before removal");
                let replacement =
                    Editor::new_block(cx, BlockRecord::paragraph(String::from("beta")));
                document.insert_blocks_at_raw(
                    location.parent,
                    location.index,
                    vec![replacement.clone()],
                    cx,
                );
                replacement
            });
            editor.mark_dirty(cx);
            assert!(editor.document.source_commit_error().is_none());
            editor.refresh_stable_document_snapshot(cx);
            assert_ne!(replacement.entity_id(), target.entity_id());

            input.update(cx, |block, cx| {
                block.replace_text_in_range(None, "你", window, cx);
                block.composition_ended(CompositionEnd::Committed, window, cx);
            });
        });
    });
    visual.run_until_parked();

    visual.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.sync_pending_ime_operations(window, cx)
        });
    });
    visual.run_until_parked();

    visual.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            assert!(editor.pending_ime_operations.is_empty());
            let current_target = editor.document.visible_blocks()[1].entity.clone();
            assert_eq!(current_target.read(cx).display_text(), "beta");
            assert_eq!(current_target.read(cx).selected_range, 0..0);
            assert!(!current_target.read(cx).focus_handle.is_focused(window));
            assert_eq!(editor.source_document.text(), "a你lpha\n\nbeta");
        });
    });
}

/// Verifies a rejected finish leaves preedit visible and save intent retryable without saving it.
#[gpui::test]
async fn ime_finish_failure_preserves_preedit_and_explicit_save_retries(cx: &mut TestAppContext) {
    init(cx);
    let visual = cx.add_empty_window();
    let editor = visual.new(|cx| Editor::from_markdown(cx, "abcd".into(), None));

    visual.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.set_view_mode(ViewMode::Source, cx);
            let block = editor.document.first_root().unwrap().clone();
            let revision = editor.source_document.revision();
            block.update(cx, |block, cx| {
                block.focus_handle.focus(window);
                block.selected_range = 1..3;
                block.replace_and_mark_text_in_range(None, "ni", Some(2..2), window, cx);
            });

            editor.request_save_document(cx);
            editor.sync_pending_ime_operations(window, cx);
            assert!(editor.ime_completion_failed);
            assert_eq!(editor.pending_ime_operations.len(), 1);
            assert!(matches!(
                editor.pending_ime_operations.front(),
                Some(DeferredImeOperation::Action { action, .. })
                    if action.as_any().is::<crate::components::SaveDocument>()
            ));
            assert_eq!(editor.source_document.text(), "abcd");
            assert_eq!(editor.source_document.revision(), revision);
            assert_eq!(block.read(cx).display_text_with_ime().to_string(), "anid");

            editor.sync_pending_ime_operations(window, cx);
            assert!(editor.ime_completion_failed);
            assert_eq!(editor.pending_ime_operations.len(), 1);

            editor.request_save_document(cx);
            assert!(!editor.ime_completion_failed);
            assert_eq!(editor.pending_ime_operations.len(), 1);
            editor.sync_pending_ime_operations(window, cx);
            assert!(editor.ime_completion_failed);
            assert_eq!(editor.pending_ime_operations.len(), 1);
            assert_eq!(editor.source_document.text(), "abcd");
            assert_eq!(block.read(cx).display_text_with_ime().to_string(), "anid");
        });
    });
}

/// Verifies an OS result remains staged until the platform sends its terminal commit notification.
#[gpui::test]
async fn ime_result_is_published_only_after_terminal_commit(cx: &mut TestAppContext) {
    init(cx);
    let visual = cx.add_empty_window();
    let editor = visual.new(|cx| Editor::from_markdown(cx, "ab".into(), None));

    visual.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.set_view_mode(ViewMode::Source, cx);
            let block = editor.document.first_root().unwrap().clone();
            let revision = editor.source_document.revision();
            block.update(cx, |block, cx| {
                block.focus_handle.focus(window);
                block.selected_range = 1..1;
                block.replace_and_mark_text_in_range(None, "n", Some(1..1), window, cx);
                block.replace_text_in_range(None, "你", window, cx);
            });

            assert_eq!(editor.source_document.text(), "ab");
            assert_eq!(editor.source_document.revision(), revision);
            assert!(editor.undo_history.is_empty());
            assert_eq!(block.read(cx).display_text_with_ime().to_string(), "a你b");
            block.update(cx, |block, cx| {
                block.composition_ended(CompositionEnd::Committed, window, cx);
            });
        });
    });

    visual.run_until_parked();
    editor.update(visual, |editor, _cx| {
        assert_eq!(editor.source_document.text(), "a你b");
        assert_eq!(editor.undo_history.len(), 1);
    });
}

/// Verifies queued undo executes on its source tab and UUID targeting survives a tab reorder.
#[gpui::test]
async fn ime_queued_undo_is_applied_to_original_tab_before_switch(cx: &mut TestAppContext) {
    init(cx);
    let visual = cx.add_empty_window();
    let editor = visual.new(|cx| Editor::from_markdown(cx, "first".into(), None));

    visual.update(|window, cx| {
        editor.update(cx, |editor, cx| append_source_text(editor, window, cx, "!"));
    });
    visual.run_until_parked();

    visual.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            assert_eq!(editor.source_document.text(), "first!");
            assert_eq!(editor.undo_history.len(), 1);
            assert!(editor.new_untitled_tab(cx));
            append_source_text(editor, window, cx, "second");
        });
    });
    visual.run_until_parked();
    let (first_tab_id, second_tab_id) = editor.read_with(visual, |editor, _cx| {
        (editor.tabs.records[0].id, editor.tabs.records[1].id)
    });

    visual.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            assert_eq!(editor.source_document.text(), "second");
            assert_eq!(editor.undo_history.len(), 1);
            assert!(editor.switch_to_tab_index(0, cx));
            let block = editor.document.first_root().unwrap().clone();
            block.update(cx, |block, cx| {
                block.focus_handle.focus(window);
                block.selected_range = 6..6;
                block.replace_and_mark_text_in_range(None, "n", Some(1..1), window, cx);
            });
            editor.on_undo(&crate::components::Undo, window, cx);
            assert!(!editor.switch_to_tab_index(1, cx));
            assert_eq!(editor.source_document.text(), "first!");
            assert!(editor.reorder_tab(1, 0, cx));
            assert_eq!(editor.tabs.records[editor.tabs.active].id, first_tab_id);
            assert_eq!(editor.tabs.records[0].id, second_tab_id);
            assert_eq!(editor.source_document.text(), "first!");
            block.update(cx, |block, cx| {
                block.replace_text_in_range(None, "你", window, cx);
                block.composition_ended(CompositionEnd::Committed, window, cx);
            });
        });
    });

    visual.run_until_parked();
    visual.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.sync_pending_ime_operations(window, cx)
        });
    });
    visual.run_until_parked();

    visual.update(|_window, cx| {
        editor.update(cx, |editor, cx| {
            assert_eq!(editor.tabs.active, 0);
            assert_eq!(editor.tabs.records[editor.tabs.active].id, second_tab_id);
            assert_eq!(editor.source_document.text(), "second");
            assert!(editor.switch_to_tab_index(1, cx));
            assert_eq!(editor.tabs.records[editor.tabs.active].id, first_tab_id);
            assert_eq!(editor.source_document.text(), "first!");
            assert_eq!(editor.undo_history.len(), 1);
            assert!(editor.switch_to_tab_index(0, cx));
            assert_eq!(editor.tabs.records[editor.tabs.active].id, second_tab_id);
            assert_eq!(editor.source_document.text(), "second");
            assert_eq!(editor.undo_history.len(), 1);
        });
    });
}

/// Verifies both history intents survive composition and replay in the order the user issued them.
#[gpui::test]
async fn ime_queued_undo_then_redo_replays_fifo(cx: &mut TestAppContext) {
    init(cx);
    let visual = cx.add_empty_window();
    let editor = visual.new(|cx| Editor::from_markdown(cx, "first".into(), None));

    visual.update(|window, cx| {
        editor.update(cx, |editor, cx| append_source_text(editor, window, cx, "!"));
    });
    visual.run_until_parked();

    visual.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            assert_eq!(editor.source_document.text(), "first!");
            assert_eq!(editor.undo_history.len(), 1);
            let block = editor.document.first_root().unwrap().clone();
            block.update(cx, |block, cx| {
                block.focus_handle.focus(window);
                block.selected_range = 6..6;
                block.replace_and_mark_text_in_range(None, "n", Some(1..1), window, cx);
            });
            editor.on_undo(&crate::components::Undo, window, cx);
            editor.on_redo(&crate::components::Redo, window, cx);
            assert_eq!(editor.source_document.text(), "first!");
            assert_eq!(editor.pending_ime_operations.len(), 2);
            assert!(matches!(
                editor.pending_ime_operations.front(),
                Some(DeferredImeOperation::Action { action, .. })
                    if action.as_any().is::<crate::components::Undo>()
            ));
            assert!(matches!(
                editor.pending_ime_operations.get(1),
                Some(DeferredImeOperation::Action { action, .. })
                    if action.as_any().is::<crate::components::Redo>()
            ));
            block.update(cx, |block, cx| {
                block.replace_text_in_range(None, "你", window, cx);
                block.composition_ended(CompositionEnd::Committed, window, cx);
            });
        });
    });

    visual.run_until_parked();
    visual.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.sync_pending_ime_operations(window, cx);
        });
    });
    visual.run_until_parked();

    editor.update(visual, |editor, _cx| {
        assert_eq!(editor.source_document.text(), "first!你");
        assert_eq!(editor.undo_history.len(), 2);
        assert!(editor.redo_history.is_empty());
    });
}

/// Verifies multiple confirmed result chunks form one transaction and one reversible history step.
#[gpui::test]
async fn ime_result_chunks_create_one_undo_step(cx: &mut TestAppContext) {
    init(cx);
    let visual = cx.add_empty_window();
    let editor = visual.new(|cx| Editor::from_markdown(cx, "ab".into(), None));

    visual.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.set_view_mode(ViewMode::Source, cx);
            let block = editor.document.first_root().unwrap().clone();
            let revision = editor.source_document.revision();
            block.update(cx, |block, cx| {
                block.focus_handle.focus(window);
                block.selected_range = 1..1;
                block.replace_and_mark_text_in_range(None, "n", Some(1..1), window, cx);
                block.replace_text_in_range(None, "你", window, cx);
                block.replace_text_in_range(None, "好", window, cx);
            });

            assert_eq!(editor.source_document.text(), "ab");
            assert_eq!(editor.source_document.revision(), revision);
            assert!(editor.undo_history.is_empty());
            assert_eq!(block.read(cx).display_text_with_ime().to_string(), "a你好b");
            block.update(cx, |block, cx| {
                block.composition_ended(CompositionEnd::Committed, window, cx);
            });
        });
    });

    visual.run_until_parked();
    editor.update(visual, |editor, cx| {
        assert_eq!(editor.source_document.text(), "a你好b");
        assert_eq!(editor.undo_history.len(), 1);
        editor.undo_document(cx);
        assert_eq!(editor.source_document.text(), "ab");
        assert!(editor.undo_history.is_empty());
        assert_eq!(editor.redo_history.len(), 1);
    });
}
