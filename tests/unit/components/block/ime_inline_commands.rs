// @author kongweiguang

use gpui::{AppContext, TestAppContext};
#[cfg(target_os = "windows")]
use gpui::{CompositionEnd, EntityInputHandler};
use std::sync::{Arc, Mutex};

use super::*;
use crate::components::block::InlineFormat;
use crate::components::{BlockEvent, BlockRecord, UndoCaptureKind};

/// A read-only text surface must not alter its Markdown, revision, or undo stream.
#[gpui::test]
async fn read_only_inline_format_does_not_mutate_or_capture_undo(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let block = cx.new(|cx| {
        let mut block = Block::with_record(cx, BlockRecord::paragraph("alpha"));
        block.set_read_only(true);
        block
    });
    let captures = Arc::new(Mutex::new(Vec::new()));
    let observed_captures = captures.clone();
    let _subscription = cx.update(|_window, cx| {
        cx.subscribe(&block, move |_, event: &BlockEvent, _| {
            if let BlockEvent::PrepareUndo { kind } = event {
                observed_captures.lock().unwrap().push(*kind);
            }
        })
    });
    let (original_title, original_revision) = cx.update(|_window, cx| {
        let block = block.read(cx);
        (block.record.title.clone(), block.document_revision)
    });

    cx.update(|_window, cx| {
        block.update(cx, |block, block_cx| {
            block.selected_range = 0..block.display_text().len();
            block.toggle_inline_format(InlineFormat::Bold, block_cx);
        });
    });

    cx.update(|_window, cx| {
        let block = block.read(cx);
        assert_eq!(block.record.title, original_title);
        assert_eq!(block.document_revision, original_revision);
    });
    assert!(captures.lock().unwrap().is_empty());
}

/// A queued format command must run after the IME commit as its own undo transaction.
#[cfg(target_os = "windows")]
#[gpui::test]
async fn queued_inline_format_commits_after_ime_in_a_separate_transaction(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let block = cx.new(|cx| Block::with_record(cx, BlockRecord::paragraph("alpha")));
    let captures = Arc::new(Mutex::new(Vec::new()));
    let observed_captures = captures.clone();
    let _subscription = cx.update(|_window, cx| {
        cx.subscribe(&block, move |_, event: &BlockEvent, _| {
            if let BlockEvent::PrepareUndo { kind } = event {
                observed_captures.lock().unwrap().push(*kind);
            }
        })
    });

    cx.update(|window, cx| {
        block.update(cx, |block, block_cx| {
            block.selected_range = 0..block.display_text().len();
            <Block as EntityInputHandler>::composition_started(block, window, block_cx);
            block.toggle_inline_format(InlineFormat::Bold, block_cx);
            <Block as EntityInputHandler>::replace_text_in_range(
                block, None, "你好", window, block_cx,
            );
            assert_eq!(block.display_text(), "alpha");
            assert_eq!(block.record.title.serialize_markdown(), "alpha");

            <Block as EntityInputHandler>::composition_ended(
                block,
                CompositionEnd::Committed,
                window,
                block_cx,
            );
            assert_eq!(block.display_text(), "你好");
            block.replay_unmanaged_input_commands(window, block_cx);
            assert_eq!(block.display_text(), "你好");
            assert_eq!(block.record.title.serialize_markdown(), "**你好**");
        });
    });

    assert_eq!(
        *captures.lock().unwrap(),
        vec![
            UndoCaptureKind::ImeCompositionCommit,
            UndoCaptureKind::NonCoalescible,
        ]
    );
}
