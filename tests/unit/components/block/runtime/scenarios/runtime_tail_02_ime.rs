// @author kongweiguang

use super::*;

#[gpui::test]
/// Keeps table-cell IME candidates virtual until terminal commit replaces the reversed range.
async fn ime_replace_and_mark_text_replaces_right_to_left_selection_in_table_cell(
    cx: &mut TestAppContext,
) {
    let cx = cx.add_empty_window();
    let block = cx.new(|cx| {
        let mut block = Block::with_record(
            cx,
            BlockRecord::new(BlockKind::Paragraph, InlineTextTree::from_markdown("alpha")),
        );
        block.set_table_cell_mode(
            TableCellPosition { row: 0, column: 0 },
            crate::components::TableColumnAlignment::Left,
        );
        block
    });

    block.update(cx, |block, _cx| {
        block.selected_range = 1..4;
        block.selection_reversed = true;
    });

    cx.update(|window, cx| {
        block.update(cx, |block, block_cx| {
            <Block as EntityInputHandler>::composition_started(block, window, block_cx);
            <Block as EntityInputHandler>::replace_and_mark_text_in_range(
                block,
                None,
                "XY",
                Some(0..1),
                window,
                block_cx,
            );
        });
    });

    cx.update(|window, cx| {
        block.update(cx, |block, block_cx| {
            assert_eq!(block.display_text(), "alpha");
            assert_eq!(block.display_text_with_ime().as_ref(), "aXYa");
            assert!(block.selection_reversed);
            let selection =
                <Block as EntityInputHandler>::selected_text_range(block, false, window, block_cx)
                    .expect("virtual candidate selection");
            assert_eq!(selection.range, 1..2);
            assert!(!selection.reversed);

            <Block as EntityInputHandler>::replace_text_in_range(
                block, None, "XY", window, block_cx,
            );
            assert_eq!(block.display_text(), "alpha");
            assert_eq!(block.display_text_with_ime().as_ref(), "aXYa");
            <Block as EntityInputHandler>::composition_ended(
                block,
                gpui::CompositionEnd::Committed,
                window,
                block_cx,
            );
            assert_eq!(block.display_text(), "aXYa");
            assert_eq!(block.selected_range, 3..3);
            assert!(!block.selection_reversed);
        });
    });
}

#[gpui::test]
/// Publishes the final IME text through the normal inline-code-aware source transaction.
async fn ime_commit_inside_inline_code_preserves_code_style(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::Paragraph,
                InlineTextTree::from_markdown("aaa`hello world`aaa"),
            ),
        )
    });

    block.update(cx, |block, _cx| {
        let cursor = "aaahello".len();
        block.selected_range = cursor..cursor;
    });

    cx.update(|window, cx| {
        block.update(cx, |block, block_cx| {
            <Block as EntityInputHandler>::composition_started(block, window, block_cx);
            <Block as EntityInputHandler>::replace_and_mark_text_in_range(
                block,
                None,
                "ni",
                Some(2..2),
                window,
                block_cx,
            );
            assert_eq!(block.display_text(), "aaahello worldaaa");
            assert!(block.display_text_with_ime().contains("ni"));
            assert_eq!(
                block.record.title.serialize_markdown(),
                "aaa`hello world`aaa"
            );
            <Block as EntityInputHandler>::replace_text_in_range(
                block, None, "你", window, block_cx,
            );
            assert_eq!(block.display_text(), "aaahello worldaaa");
            assert!(block.display_text_with_ime().contains("你"));
            assert_eq!(
                block.record.title.serialize_markdown(),
                "aaa`hello world`aaa"
            );
            <Block as EntityInputHandler>::composition_ended(
                block,
                gpui::CompositionEnd::Committed,
                window,
                block_cx,
            );
        });
    });

    block.read_with(cx, |block, _cx| {
        assert_eq!(block.display_text(), "aaahello你 worldaaa");
        assert_eq!(
            block.record.title.serialize_markdown(),
            "aaa`hello你 world`aaa"
        );
        assert_only_code_range(block, "aaa".len().."aaahello你 world".len());
    });
}

#[gpui::test]
/// Confirms projected code preedit stays transient and terminal commit preserves its style.
async fn ime_commit_inside_projected_inline_code_preserves_code_style(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::Paragraph,
                InlineTextTree::from_markdown("aaa`hello world`aaa"),
            ),
        )
    });

    block.update(cx, |block, _cx| {
        let cursor = "aaahello".len();
        block.selected_range = cursor..cursor;
        block.sync_inline_projection_for_focus(true);
        assert_eq!(block.display_text(), "aaa`hello world`aaa");
    });

    cx.update(|window, cx| {
        block.update(cx, |block, block_cx| {
            <Block as EntityInputHandler>::composition_started(block, window, block_cx);
            <Block as EntityInputHandler>::replace_and_mark_text_in_range(
                block,
                None,
                "ni",
                Some(2..2),
                window,
                block_cx,
            );
            assert_eq!(block.display_text(), "aaa`hello world`aaa");
            assert!(block.display_text_with_ime().contains("ni"));
            assert_eq!(
                block.record.title.serialize_markdown(),
                "aaa`hello world`aaa"
            );
            <Block as EntityInputHandler>::replace_text_in_range(
                block, None, "你", window, block_cx,
            );
            assert_eq!(block.display_text(), "aaa`hello world`aaa");
            assert!(block.display_text_with_ime().contains("你"));
            assert_eq!(
                block.record.title.serialize_markdown(),
                "aaa`hello world`aaa"
            );
            <Block as EntityInputHandler>::composition_ended(
                block,
                gpui::CompositionEnd::Committed,
                window,
                block_cx,
            );
        });
    });

    block.update(cx, |block, _cx| {
        assert_eq!(
            block.record.title.serialize_markdown(),
            "aaa`hello你 world`aaa"
        );
        block.clear_inline_projection();
        assert_eq!(block.display_text(), "aaahello你 worldaaa");
        assert_only_code_range(block, "aaa".len().."aaahello你 world".len());
    });
}
