// @author kongweiguang

use gpui::{AppContext, EntityInputHandler, TestAppContext};

use super::Block;
use crate::components::BlockRecord;

/// Platforms without a composition terminal must keep marked updates in the existing eager edit path.
#[gpui::test]
async fn marked_updates_replace_the_previous_candidate_without_staging(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let block = cx.new(|cx| Block::with_record(cx, BlockRecord::paragraph("before")));

    cx.update(|window, cx| {
        block.update(cx, |block, block_cx| {
            block.selected_range = 1..4;
            <Block as EntityInputHandler>::replace_and_mark_text_in_range(
                block,
                None,
                "first",
                Some(5..5),
                window,
                block_cx,
            );
            assert_eq!(block.display_text(), "bfirstre");
            assert!(!block.has_ime_composition());

            <Block as EntityInputHandler>::replace_and_mark_text_in_range(
                block,
                None,
                "second",
                Some(6..6),
                window,
                block_cx,
            );
            assert_eq!(block.display_text(), "bsecondre");
            assert!(!block.has_ime_composition());

            <Block as EntityInputHandler>::unmark_text(block, window, block_cx);
            assert_eq!(block.display_text(), "bsecondre");
            assert!(!block.has_ime_composition());
        });
    });
}
