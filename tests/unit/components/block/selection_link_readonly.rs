// @author kongweiguang

use gpui::{AppContext, EntityInputHandler, TestAppContext};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use crate::components::{
    Block, BlockEvent, BlockHostAction, BlockKind, BlockRecord, InlineTextTree,
};

/// 迟到命中可能仍携带旧链接投影；最终写入口不能因纯链接快速路径跳过只读检查。
#[gpui::test]
async fn readonly_link_label_replacement_preserves_text_and_revision(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let block = cx.new(|cx| {
        let mut title = InlineTextTree::plain("alpha beta");
        assert!(title.set_inline_link_destination(0..5, Some("https://example.org".to_owned())));
        Block::with_record(cx, BlockRecord::new(BlockKind::Paragraph, title))
    });
    let (range, original_title, original_text, original_revision) =
        block.update(cx, |block, _cx| {
            block.set_read_only(true);
            block.selected_range = 0..5;
            // 模拟旧布局仍用原焦点展开链接，直接验证最终写入口，而非依赖正常切换先清投影。
            block.sync_inline_projection_for_focus(true);
            assert!(block.display_text().starts_with("[alpha]("));
            let start = block.display_text().find("alpha").expect("projected label");
            let range = start..start + "alpha".len();
            (
                range,
                block.record.title.clone(),
                block.display_text().to_owned(),
                block.document_revision,
            )
        });
    let changes = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&changes);
    let _subscription = cx.update(|_, cx| {
        cx.subscribe(&block, move |_, event: &BlockEvent, _| {
            if matches!(event, BlockEvent::Changed | BlockEvent::PrepareUndo { .. }) {
                observed.fetch_add(1, Ordering::Relaxed);
            }
        })
    });
    block.update(cx, |block, cx| {
        block.replace_text_in_visible_range(range, "bravo", None, false, cx);
        assert_eq!(block.record.title, original_title);
        assert_eq!(block.display_text(), original_text);
        assert_eq!(block.document_revision, original_revision);
    });
    assert_eq!(changes.load(Ordering::Relaxed), 0);
}

/// 正常浮层提交后返回原文字选区，确保只读防护不破坏日常链接编辑。
#[gpui::test]
async fn selection_link_input_submits_destination_and_returns_focus(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let block = cx.new(|cx| Block::with_record(cx, BlockRecord::paragraph("alpha beta")));
    let input = cx.update(|window, cx| {
        block.update(cx, |block, cx| {
            block.selected_range = 0..5;
            block.open_selection_link_editor(window, cx);
        });
        block
            .read(cx)
            .selection_toolbar_link_input
            .clone()
            .expect("opening the link editor creates its floating input")
    });

    cx.update(|window, cx| {
        input.update(cx, |input, input_cx| {
            <Block as EntityInputHandler>::replace_text_in_range(
                input,
                None,
                "https://example.org",
                window,
                input_cx,
            );
        });
        let destination = input.read(cx).display_text().to_owned();
        let handler = input
            .read(cx)
            .host_action_handler()
            .expect("floating input routes Submit to its owning block");
        handler(BlockHostAction::Submit(destination.into()), window, cx);

        let block = block.read(cx);
        assert_eq!(
            block.record.title.serialize_markdown(),
            "[alpha](https://example.org) beta"
        );
        assert_eq!(block.selection_clean_range(), 0..5);
        assert!(block.selection_toolbar_link_input.is_none());
        assert!(block.focus_handle.is_focused(window));
    });
}

/// 切到只读须立即移除可写浮层；迟到提交不能改正文、产生撤销项或抢走新输入的焦点。
#[gpui::test]
async fn late_link_submit_after_read_only_transition_is_discarded(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let block = cx.new(|cx| Block::with_record(cx, BlockRecord::paragraph("alpha beta")));
    let other_input = cx.new(|cx| Block::with_record(cx, BlockRecord::paragraph("find query")));
    let undo_captures = Arc::new(AtomicUsize::new(0));
    let observed_captures = Arc::clone(&undo_captures);
    let _subscription = cx.update(|_, cx| {
        cx.subscribe(&block, move |_, event: &BlockEvent, _| {
            if matches!(event, BlockEvent::PrepareUndo { .. }) {
                observed_captures.fetch_add(1, Ordering::Relaxed);
            }
        })
    });
    let (original_title, original_revision, _retained_input, handler, destination) =
        cx.update(|window, cx| {
            let (title, revision) = {
                let block = block.read(cx);
                (block.record.title.clone(), block.document_revision)
            };
            block.update(cx, |block, cx| {
                block.selected_range = 0..5;
                block.open_selection_link_editor(window, cx);
            });
            let input = block
                .read(cx)
                .selection_toolbar_link_input
                .clone()
                .expect("opening the link editor creates its floating input");
            input.update(cx, |input, input_cx| {
                <Block as EntityInputHandler>::replace_text_in_range(
                    input,
                    None,
                    "https://stale.example",
                    window,
                    input_cx,
                );
            });
            let destination = input.read(cx).display_text().to_owned();
            let handler = input
                .read(cx)
                .host_action_handler()
                .expect("floating input keeps its Submit route after unmount");
            (title, revision, input, handler, destination)
        });

    cx.update(|window, cx| {
        block.update(cx, |block, _cx| block.set_read_only(true));
        assert!(block.read(cx).selection_toolbar_link_input.is_none());
        assert!(block.read(cx).selection_toolbar_link_focus.is_none());
        assert!(block.read(cx).selection_toolbar_link_range.is_none());
        other_input.read(cx).focus_handle.focus(window);
        block.update(cx, |block, cx| block.open_selection_link_editor(window, cx));
        assert!(other_input.read(cx).focus_handle.is_focused(window));
        let block = block.read(cx);
        assert!(block.selection_toolbar_link_input.is_none());
        assert!(block.selection_toolbar_link_range.is_none());
    });

    cx.update(|window, cx| {
        handler(BlockHostAction::Submit(destination.into()), window, cx);
        block.update(cx, |block, cx| {
            block.selection_toolbar_link_range = Some(0..5);
            block.commit_selection_link_destination(
                Some("https://stale.example".to_owned()),
                window,
                cx,
            );
        });

        let block = block.read(cx);
        assert_eq!(block.record.title, original_title);
        assert_eq!(block.document_revision, original_revision);
        assert_eq!(block.selected_range, 0..5);
        assert!(block.selection_toolbar_link_input.is_none());
        assert!(block.selection_toolbar_link_range.is_none());
        assert!(other_input.read(cx).focus_handle.is_focused(window));
    });
    assert_eq!(undo_captures.load(Ordering::Relaxed), 0);
}
