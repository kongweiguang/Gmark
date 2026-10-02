// @author kongweiguang

use crate::components::{
    Block, BlockKind, BlockRecord, End, Home, InlineTextTree, SelectEnd, SelectHome,
};
#[cfg(target_os = "windows")]
use gpui::EntityInputHandler;
use gpui::{AppContext, Hsla, TestAppContext, TextRun, VisualTestContext, font, px, rgba};
use unicode_segmentation::UnicodeSegmentation;

/// Shapes a Block line with GPUI so navigation uses the same wrap boundaries as the rendered surface.
fn shape_line(
    text: &str,
    width: gpui::Pixels,
    cx: &mut VisualTestContext,
) -> Vec<gpui::WrappedLine> {
    cx.update(|window, _app| {
        window
            .text_system()
            .shape_text(
                text.to_owned().into(),
                px(16.0),
                &[TextRun {
                    len: text.len(),
                    font: font(".SystemUIFont"),
                    color: Hsla::from(rgba(0xffffffff)),
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                }],
                Some(width),
                None,
            )
            .expect("line should shape")
            .into_vec()
    })
}

/// Confirms collapsed and extended Home/End stay on the active soft-wrapped row and retain anchor direction.
#[gpui::test]
async fn home_end_and_shift_variants_use_the_current_visual_row(cx: &mut TestAppContext) {
    let visual = cx.add_empty_window();
    let text = "alpha bravo charlie delta echo";
    let layout = shape_line(text, px(66.0), visual);
    assert!(
        !layout[0].wrap_boundaries().is_empty(),
        "text should soft-wrap"
    );
    let block = visual.new(|cx| Block::with_record(cx, BlockRecord::paragraph(text)));

    let (row_start, caret, row_end) = block.update(visual, |block, _cx| {
        block.last_layout = Some(layout);
        let wrap_start = (1..text.len())
            .find(|offset| {
                block.selected_range = *offset..*offset;
                block.current_visual_line_boundary(false) > 0
            })
            .expect("a soft-wrapped row should have a nonzero start");
        let caret = wrap_start + 1;
        block.selected_range = caret..caret;
        let row_start = block.current_visual_line_boundary(false);
        let row_end = block.current_visual_line_boundary(true);
        assert!(row_start > 0);
        assert!(caret < row_end);
        (row_start, caret, row_end)
    });

    visual.update(|window, app| {
        block.update(app, |block, block_cx| {
            block.selected_range = caret..caret;
            block.selection_reversed = false;
            block.on_home(&Home, window, block_cx);
            assert_eq!(block.selected_range, row_start..row_start);

            block.selected_range = caret..caret;
            block.on_end(&End, window, block_cx);
            assert_eq!(block.selected_range, row_end..row_end);

            block.selected_range = caret..caret + 1;
            block.selection_reversed = true;
            block.on_select_home(&SelectHome, window, block_cx);
            assert_eq!(block.selected_range, row_start..caret + 1);
            assert!(
                block.selection_reversed,
                "Shift+Home must preserve its reverse anchor"
            );

            block.selected_range = caret..caret + 1;
            block.selection_reversed = true;
            block.on_select_end(&SelectEnd, window, block_cx);
            assert_eq!(block.selected_range, caret + 1..row_end);
            assert!(
                !block.selection_reversed,
                "crossing the anchor must update selection direction"
            );
        });
    });
}

/// Keeps CRLF out of the line endpoint and makes Home/End destinations complete grapheme boundaries.
#[gpui::test]
async fn home_end_preserve_crlf_and_emoji_graphemes(cx: &mut TestAppContext) {
    let visual = cx.add_empty_window();
    let family = "👨‍👩‍👧‍👦";
    let line = format!("x{family}y");
    let text = format!("first\r\n{line}\r\nlast");
    let line_start = "first\r\n".len();
    let line_end = line_start + line.len();
    let inside_family = line_start + 1 + "👨‍".len();
    let block = visual.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(BlockKind::Paragraph, InlineTextTree::plain(text.clone())),
        )
    });

    block.read_with(visual, |block, _cx| {
        assert_eq!(block.display_text(), text);
    });
    visual.update(|window, app| {
        block.update(app, |block, block_cx| {
            block.selected_range = inside_family..inside_family;
            block.on_home(&Home, window, block_cx);
            assert_eq!(block.cursor_offset(), line_start);
            assert!(is_grapheme_boundary(&text, block.cursor_offset()));

            block.selected_range = inside_family..inside_family;
            block.on_end(&End, window, block_cx);
            assert_eq!(block.cursor_offset(), line_end);
            assert!(is_grapheme_boundary(&text, block.cursor_offset()));
        });
    });
}

/// Keeps native preedit coordinates stable when Home/End actions reach the Block during candidate handling.
#[cfg(target_os = "windows")]
#[gpui::test]
async fn home_end_do_not_move_the_selection_during_ime_preedit(cx: &mut TestAppContext) {
    let visual = cx.add_empty_window();
    let block = visual.new(|cx| Block::with_record(cx, BlockRecord::paragraph("alpha")));

    visual.update(|window, app| {
        block.update(app, |block, block_cx| {
            block.focus_handle.focus(window);
            block.selected_range = 2..4;
            <Block as EntityInputHandler>::composition_started(block, window, block_cx);
            assert!(block.has_ime_composition());
            let selection = block.selected_range.clone();

            block.on_home(&Home, window, block_cx);
            block.on_end(&End, window, block_cx);
            block.on_select_home(&SelectHome, window, block_cx);
            block.on_select_end(&SelectEnd, window, block_cx);

            assert_eq!(block.selected_range, selection);
            assert!(block.has_ime_composition());
        });
    });
}

/// Checks byte offsets against Unicode grapheme starts plus the valid text-end sentinel.
fn is_grapheme_boundary(text: &str, offset: usize) -> bool {
    offset == text.len()
        || text
            .grapheme_indices(true)
            .any(|(start, _)| start == offset)
}
