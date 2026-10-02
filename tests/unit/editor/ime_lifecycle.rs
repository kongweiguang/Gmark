// @author kongweiguang

#[cfg(target_os = "windows")]
use super::{DeferredImeOperation, Editor};
#[cfg(target_os = "windows")]
use crate::editor::ViewMode;
#[cfg(target_os = "windows")]
use crate::i18n::I18nManager;
#[cfg(target_os = "windows")]
use gpui::{AppContext, CompositionEnd, EntityInputHandler, TestAppContext};

/// 测试使用公开输入协议而不把中文粘贴误称真实系统输入法验收。
#[cfg(target_os = "windows")]
fn init(cx: &mut TestAppContext) {
    cx.update(|cx| {
        I18nManager::init(cx);
        crate::theme::ThemeManager::init(cx);
        crate::components::init(cx);
    });
}

#[cfg(target_os = "windows")]
#[gpui::test]
/// 用户动作不能借渲染帧绕过候选终态，取消还要保留原反向选择。
async fn ime_preedit_defers_mode_and_save_without_changing_source(cx: &mut TestAppContext) {
    init(cx);
    let visual = cx.add_empty_window();
    let editor = visual.new(|cx| Editor::from_markdown(cx, "alpha".into(), None));
    visual.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.set_view_mode(ViewMode::Source, cx);
            let block = editor.document.first_root().unwrap().clone();
            let revision = editor.source_document.revision();
            block.update(cx, |block, cx| {
                block.focus_handle.focus(window);
                block.selected_range = 1..3;
                block.selection_reversed = true;
                block.replace_and_mark_text_in_range(None, "ni", Some(2..2), window, cx);
            });
            assert_eq!(editor.source_document.text(), "alpha");
            assert_eq!(editor.source_document.revision(), revision);
            assert!(!editor.document_dirty);
            assert!(editor.undo_history.is_empty());
            editor.set_view_mode(ViewMode::Preview, cx);
            editor.request_save_document(cx);
            editor.sync_pending_ime_operations(window, cx);
            assert_eq!(editor.view_mode, ViewMode::Source);
            assert!(editor.ime_completion_failed);
            assert_eq!(editor.pending_ime_operations.len(), 2);
            assert!(matches!(
                editor.pending_ime_operations.front(),
                Some(DeferredImeOperation::Mode {
                    mode: ViewMode::Preview,
                    ..
                })
            ));
            assert!(matches!(
                editor.pending_ime_operations.get(1),
                Some(DeferredImeOperation::Action { action, .. })
                    if action.as_any().is::<crate::components::SaveDocument>()
            ));
            block.update(cx, |block, cx| {
                block.composition_ended(CompositionEnd::Cancelled, window, cx)
            });
            assert_eq!(block.read(cx).selected_range, 1..3);
            assert!(block.read(cx).selection_reversed);
            assert_eq!(editor.source_document.text(), "alpha");
        });
    });
}

#[cfg(target_os = "windows")]
#[gpui::test]
/// 多次候选更新属于一次输入意图，确认与撤销都只影响该意图。
async fn ime_commits_one_source_transaction_and_one_undo(cx: &mut TestAppContext) {
    init(cx);
    let visual = cx.add_empty_window();
    let editor = visual.new(|cx| Editor::from_markdown(cx, "ab".into(), None));
    visual.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.set_view_mode(ViewMode::Source, cx);
            let block = editor.document.first_root().unwrap().clone();
            block.update(cx, |block, _cx| {
                block.focus_handle.focus(window);
                block.selected_range = 1..1;
            });
            editor.refresh_stable_document_snapshot(cx);
            block.update(cx, |block, cx| {
                block.replace_and_mark_text_in_range(None, "n", Some(1..1), window, cx);
                block.replace_and_mark_text_in_range(None, "ni", Some(2..2), window, cx);
                block.replace_text_in_range(None, "你", window, cx);
                block.composition_ended(CompositionEnd::Committed, window, cx);
            });
        });
    });
    visual.run_until_parked();
    editor.update(visual, |editor, cx| {
        assert_eq!(editor.source_document.text(), "a你b");
        assert_eq!(editor.undo_history.len(), 1);
        editor.undo_document(cx);
        assert_eq!(editor.source_document.text(), "ab");
        assert_eq!(
            editor
                .document
                .first_root()
                .unwrap()
                .read(cx)
                .selected_range,
            1..1
        );
    });
}

#[cfg(target_os = "windows")]
#[gpui::test]
/// 无交集共享事务应保留系统绑定的实体身份并正确重定位候选。
async fn ime_rebases_foreign_edit_and_keeps_original_input_entity(cx: &mut TestAppContext) {
    init(cx);
    let visual = cx.add_empty_window();
    let editor = visual.new(|cx| Editor::from_markdown(cx, "alpha beta".into(), None));
    visual.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.set_view_mode(ViewMode::Source, cx);
            let original = editor.document.first_root().unwrap().clone();
            original.update(cx, |block, cx| {
                block.focus_handle.focus(window);
                block.selected_range = 6..6;
                block.replace_and_mark_text_in_range(None, "ni", Some(2..2), window, cx);
            });
            let peer = editor.source_document.fork_view().unwrap();
            peer.apply_transaction(gmark_document::Transaction::new(
                peer.revision(),
                vec![gmark_document::TextEdit::new(0..0, "x ")],
            ))
            .unwrap();
            editor.sync_shared_document_events(cx);
            assert_eq!(
                editor.document.first_root().unwrap().entity_id(),
                original.entity_id()
            );
            assert!(original.read(cx).has_ime_composition());
            original.update(cx, |block, cx| {
                block.replace_text_in_range(None, "你", window, cx);
                block.composition_ended(CompositionEnd::Committed, window, cx);
            });
        });
    });
    visual.run_until_parked();
    assert_eq!(
        editor.read_with(visual, |editor, _cx| editor.source_document.text()),
        "x alpha 你beta"
    );
}

#[cfg(target_os = "windows")]
#[gpui::test]
/// 覆盖原替换片段后，迟到系统结果不能覆盖其他视图的新正文。
async fn ime_conflict_drops_late_result_without_overwriting_peer(cx: &mut TestAppContext) {
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
            assert!(original.read(cx).has_ime_composition());
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

#[cfg(target_os = "windows")]
#[path = "ime_deferred_actions.rs"]
mod deferred_actions;

/// Partial native results remain unsaved until the terminal, then share one document undo boundary.
/// The host owns revision and disk assertions that a standalone Block fixture cannot make.
#[cfg(target_os = "windows")]
#[gpui::test]
async fn partial_ime_result_then_remaining_preedit_has_one_saved_document_transaction(
    cx: &mut TestAppContext,
) {
    init(cx);
    let directory = tempfile::tempdir().expect("isolated partial IME save directory");
    for (end, expected, file_name) in [
        (CompositionEnd::Committed, "b日本re", "committed.md"),
        (CompositionEnd::Cancelled, "b日re", "cancelled.md"),
    ] {
        let path = directory.path().join(file_name);
        std::fs::write(&path, "before").expect("isolated saved baseline");
        let visual = cx.add_empty_window();
        let editor =
            visual.new(|cx| Editor::from_markdown(cx, "before".into(), Some(path.clone())));
        let (block, revision) = visual.update(|window, cx| {
            editor.update(cx, |editor, cx| {
                editor.set_view_mode(ViewMode::Source, cx);
                let block = editor.document.first_root().expect("source block").clone();
                block.update(cx, |block, _| {
                    block.focus_handle.focus(window);
                    block.selected_range = 1..4;
                    block.selection_reversed = true;
                });
                editor.refresh_stable_document_snapshot(cx);
                (block, editor.source_document.revision())
            })
        });
        visual.update(|window, cx| {
            block.update(cx, |block, cx| {
                block.composition_started(window, cx);
                block.replace_and_mark_text_in_range(None, "kana", Some(4..4), window, cx);
                block.replace_text_in_range(None, "日", window, cx);
                block.replace_and_mark_text_in_range(None, "候補", Some(2..2), window, cx);
            });
        });
        visual.run_until_parked();
        editor.read_with(visual, |editor, _| {
            assert_eq!(editor.source_document.text(), "before");
            assert_eq!(editor.source_document.revision(), revision);
            assert!(!editor.document_dirty);
            assert!(editor.undo_history.is_empty());
        });
        assert_eq!(
            std::fs::read_to_string(&path).expect("saved baseline"),
            "before"
        );

        visual.update(|window, cx| {
            block.update(cx, |block, cx| {
                if end == CompositionEnd::Committed {
                    block.replace_text_in_range(None, "本", window, cx);
                }
                block.composition_ended(end, window, cx);
            });
        });
        visual.run_until_parked();
        visual.update(|window, cx| {
            editor.update(cx, |editor, cx| {
                assert_eq!(editor.source_document.text(), expected);
                assert_ne!(editor.source_document.revision(), revision);
                assert_eq!(editor.undo_history.len(), 1);
                assert!(editor.document_dirty);
                assert!(editor.save_to_existing_path(&path, window, cx));
            });
        });
        assert_eq!(
            std::fs::read_to_string(&path).expect("confirmed IME save"),
            expected
        );
        editor.update(visual, |editor, cx| {
            editor.undo_document(cx);
            assert_eq!(editor.source_document.text(), "before");
            let block = editor
                .document
                .first_root()
                .expect("restored source block")
                .read(cx);
            assert_eq!(block.selected_range, 1..4);
            assert!(block.selection_reversed);
            assert!(editor.undo_history.is_empty());
            editor.redo_document(cx);
            assert_eq!(editor.source_document.text(), expected);
        });
    }
}

#[path = "ime_pointer_queue.rs"]
mod pointer_queue;
