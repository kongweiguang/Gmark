// @author kongweiguang

use crate::components::{Paste, SelectAll, Undo};
use crate::editor::{Editor, ViewMode};
use gpui::{AppContext, ClipboardItem, CompositionEnd, EntityInputHandler, TestAppContext};

/// 用真实 Editor 订阅链验证暂存、命令回放和历史，避免仅验证 Block 内部标记。
fn init(cx: &mut TestAppContext) {
    cx.update(|cx| {
        crate::i18n::I18nManager::init(cx);
        crate::theme::ThemeManager::init(cx);
        crate::components::init(cx);
    });
}

/// 普通粘贴必须是独立事务，候选阶段不能将剪贴板内容混进 IME 的确认前缀。
#[gpui::test]
async fn ime_paste_waits_for_terminal_and_keeps_two_undo_steps(cx: &mut TestAppContext) {
    init(cx);
    let visual = cx.add_empty_window();
    let editor = visual.new(|cx| Editor::from_markdown(cx, "ab".into(), None));
    visual.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.set_view_mode(ViewMode::Source, cx);
            editor.refresh_stable_document_snapshot(cx);
            let target = editor.document.first_root().unwrap().clone();
            target.update(cx, |block, cx| {
                block.focus_handle.focus(window);
                block.selected_range = 1..1;
                block.replace_and_mark_text_in_range(None, "ni", Some(2..2), window, cx);
            });
            cx.write_to_clipboard(ClipboardItem::new_string(String::from("X")));
            editor.on_paste_capture(&Paste, window, cx);
            assert_eq!(editor.source_document.text(), "ab");
            assert!(!editor.document_dirty);
            assert!(editor.undo_history.is_empty());
            assert_eq!(target.read(cx).display_text_with_ime().to_string(), "anib");
        });
    });
    visual.run_until_parked();
    visual.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            assert_eq!(editor.pending_ime_operations.len(), 1);
            let target = editor.document.first_root().unwrap().clone();
            target.update(cx, |block, cx| {
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
            assert_eq!(editor.source_document.text(), "a你Xb");
            assert_eq!(editor.undo_history.len(), 2);
            editor.on_undo(&Undo, window, cx);
            assert_eq!(editor.source_document.text(), "a你b");
            editor.on_undo(&Undo, window, cx);
            assert_eq!(editor.source_document.text(), "ab");
        });
    });
}

/// 多个候选期间的粘贴必须按顺序执行，各自可撤销，不能被旧 revision 吞掉。
#[gpui::test]
async fn ime_two_pastes_replay_in_order_with_separate_history(cx: &mut TestAppContext) {
    init(cx);
    let visual = cx.add_empty_window();
    let editor = visual.new(|cx| Editor::from_markdown(cx, "ab".into(), None));
    visual.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.set_view_mode(ViewMode::Source, cx);
            editor.refresh_stable_document_snapshot(cx);
            let target = editor.document.first_root().unwrap().clone();
            target.update(cx, |block, cx| {
                block.focus_handle.focus(window);
                block.selected_range = 1..1;
                block.replace_and_mark_text_in_range(None, "ni", Some(2..2), window, cx);
            });
            cx.write_to_clipboard(ClipboardItem::new_string(String::from("X")));
            editor.on_paste_capture(&Paste, window, cx);
            editor.on_paste_capture(&Paste, window, cx);
            assert_eq!(editor.source_document.text(), "ab");
            assert!(!editor.document_dirty);
            assert!(editor.undo_history.is_empty());
            assert_eq!(target.read(cx).display_text_with_ime().to_string(), "anib");
        });
    });
    visual.run_until_parked();
    visual.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            assert_eq!(editor.pending_ime_operations.len(), 2);
            let target = editor.document.first_root().unwrap().clone();
            target.update(cx, |block, cx| {
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
            assert_eq!(editor.source_document.text(), "a你XXb");
            assert_eq!(editor.undo_history.len(), 3);
            editor.on_undo(&Undo, window, cx);
            assert_eq!(editor.source_document.text(), "a你Xb");
            editor.on_undo(&Undo, window, cx);
            assert_eq!(editor.source_document.text(), "a你b");
            editor.on_undo(&Undo, window, cx);
            assert_eq!(editor.source_document.text(), "ab");
        });
    });
}

/// 文档全选不能先改候选替换范围；终态后一次 Ctrl+A 应覆盖完整确认文字。
#[gpui::test]
async fn ime_select_all_preserves_preedit_then_selects_committed_document(cx: &mut TestAppContext) {
    init(cx);
    let visual = cx.add_empty_window();
    let editor = visual.new(|cx| Editor::from_markdown(cx, "alpha\n\nbeta".into(), None));
    visual.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.refresh_stable_document_snapshot(cx);
            let target = editor.document.first_root().unwrap().clone();
            target.update(cx, |block, cx| {
                block.focus_handle.focus(window);
                block.selected_range = 1..1;
                block.replace_and_mark_text_in_range(None, "ni", Some(2..2), window, cx);
                block.on_select_all(&SelectAll, window, cx);
                assert_eq!(block.selected_range, 1..1);
                assert!(block.has_ime_composition());
            });
            assert!(editor.cross_block_selection.is_none());
        });
    });
    visual.run_until_parked();
    visual.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            let target = editor.document.first_root().unwrap().clone();
            target.update(cx, |block, cx| {
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
    editor.update(visual, |editor, cx| {
        assert_eq!(editor.source_document.text(), "a你lpha\n\nbeta");
        assert_eq!(
            editor.selected_markdown_text(cx).as_deref(),
            Some("a你lpha\n\nbeta")
        );
        assert!(editor.pending_ime_operations.is_empty());
    });
}
