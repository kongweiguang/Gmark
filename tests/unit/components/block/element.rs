// @author kongweiguang

use super::text::code_language_cursor_bounds;
use super::{
    link_at_position, source_line_number_gutter_width, source_line_number_tops, source_text_bounds,
    wrapped_line_height,
};
use crate::components::{Block, BlockKind, BlockRecord, InlineTextTree, TableCellPosition};
use gpui::{
    AppContext, Bounds, Hsla, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    SharedString, TestAppContext, TextAlign, TextRun, VisualTestContext, font, point, px, rgba,
    size,
};

fn shaped_lines(
    text: &str,
    width: gpui::Pixels,
    cx: &mut VisualTestContext,
) -> Vec<gpui::WrappedLine> {
    cx.update(|window, _app| {
        window
            .text_system()
            .shape_text(
                text.to_string().into(),
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
            .expect("text should shape")
            .into_vec()
    })
}

#[test]
fn source_line_number_gutter_grows_with_digit_count() {
    let one_digit = source_line_number_gutter_width(9, px(16.0));
    let two_digits = source_line_number_gutter_width(10, px(16.0));
    let three_digits = source_line_number_gutter_width(100, px(16.0));

    assert_eq!(one_digit, two_digits);
    assert!(three_digits > two_digits);
}

#[test]
fn source_text_bounds_are_offset_by_gutter_width() {
    let bounds = Bounds::new(point(px(10.0), px(20.0)), size(px(300.0), px(120.0)));
    let text_bounds = source_text_bounds(bounds, px(48.0));

    assert_eq!(text_bounds.left(), px(58.0));
    assert_eq!(text_bounds.top(), px(20.0));
    assert_eq!(text_bounds.size.width, px(252.0));
    assert_eq!(text_bounds.size.height, px(120.0));
}

#[test]
fn code_language_cursor_uses_font_height_and_stays_centered() {
    let input_bounds = Bounds::new(point(px(10.0), px(20.0)), size(px(156.0), px(24.0)));
    let cursor = code_language_cursor_bounds(input_bounds, px(42.0), px(14.0), px(2.0));

    assert_eq!(cursor.left(), px(52.0));
    assert_eq!(cursor.top(), px(25.0));
    assert_eq!(cursor.size, size(px(2.0), px(14.0)));
    assert!(cursor.top() >= input_bounds.top());
    assert!(cursor.bottom() <= input_bounds.bottom());
}

#[gpui::test]
async fn source_line_number_tops_follow_soft_wrapped_hard_lines(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let lines = shaped_lines(
        "this line should wrap before the next hard line\nsecond",
        px(92.0),
        cx,
    );
    assert!(
        !lines[0].wrap_boundaries().is_empty(),
        "first hard line should soft-wrap"
    );

    let tops = source_line_number_tops(&lines, px(20.0));
    assert_eq!(tops[0], px(0.0));
    assert_eq!(tops[1], wrapped_line_height(&lines[0], px(20.0)));
}

#[gpui::test]
async fn link_hit_matches_only_rendered_link_text(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::Paragraph,
                InlineTextTree::from_markdown("[link](https://example.com)"),
            ),
        )
    });

    let display_text = block.read_with(cx, |block, _cx| block.display_text().to_string());
    let lines = shaped_lines(&display_text, px(320.0), cx);
    let (hit, miss_right) = block.read_with(cx, |block, _cx| {
        let bounds = Bounds::new(point(px(0.0), px(0.0)), size(px(320.0), px(20.0)));
        let span = block
            .inline_spans()
            .iter()
            .find(|span| span.link.is_some())
            .expect("link span should exist");
        let layout = &lines[0];
        let start = layout
            .position_for_index(span.range.start, px(20.0))
            .expect("start position");
        let end = layout
            .position_for_index(span.range.end, px(20.0))
            .expect("end position");
        let hit = point((start.x + end.x) / 2.0, px(10.0));
        let miss_right = point(end.x + px(24.0), px(10.0));
        (
            link_at_position(block, &lines, bounds, px(20.0), hit)
                .map(|link| link.open_target.clone()),
            link_at_position(block, &lines, bounds, px(20.0), miss_right)
                .map(|link| link.open_target.clone()),
        )
    });

    assert_eq!(hit, Some("https://example.com".to_string()));
    assert_eq!(miss_right, None);
}

/// Begin a drag from an unfocused source block on the first press and select its exact text range.
#[gpui::test]
async fn source_document_mouse_drag_selects_exact_text_range(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let block = cx.new(|cx| {
        let mut block = Block::with_record(cx, BlockRecord::paragraph("alpha beta"));
        block.set_source_document_mode();
        block
    });
    let text = block.read_with(cx, |block, _cx| block.display_text().to_owned());
    let lines = shaped_lines(&text, px(320.0), cx);
    let bounds = Bounds::new(point(px(0.0), px(0.0)), size(px(320.0), px(20.0)));
    let start = lines[0]
        .position_for_index(1, px(20.0))
        .expect("source selection start");
    let end = lines[0]
        .position_for_index(6, px(20.0))
        .expect("source selection end");
    block.update(cx, |block, _cx| {
        block.last_layout = Some(lines);
        block.last_bounds = Some(bounds);
        block.last_line_height = px(20.0);
    });

    cx.update(|window, app| {
        block.update(app, |block, block_cx| {
            block.on_mouse_down(
                &MouseDownEvent {
                    button: MouseButton::Left,
                    position: point(start.x, px(10.0)),
                    modifiers: Modifiers::default(),
                    click_count: 1,
                    first_mouse: false,
                },
                window,
                block_cx,
            );
            block.on_mouse_move(
                &MouseMoveEvent {
                    position: point(end.x, px(10.0)),
                    pressed_button: Some(MouseButton::Left),
                    modifiers: Modifiers::default(),
                },
                window,
                block_cx,
            );
            block.on_mouse_up(
                &MouseUpEvent {
                    button: MouseButton::Left,
                    position: point(end.x, px(10.0)),
                    modifiers: Modifiers::default(),
                    click_count: 1,
                },
                window,
                block_cx,
            );
        });
    });

    block.read_with(cx, |block, _cx| {
        assert_eq!(block.selected_range, 1..6);
        assert!(!block.is_selecting);
    });
}

/// Double-clicking rendered link text selects its word; only Ctrl+click follows the link.
#[gpui::test]
async fn secondary_click_follows_link_while_plain_click_edits(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::Paragraph,
                InlineTextTree::from_markdown("a [link](https://example.com) bbbb"),
            ),
        )
    });

    let display_text = block.read_with(cx, |block, _cx| block.display_text().to_string());
    let lines = shaped_lines(&display_text, px(320.0), cx);
    let bounds = Bounds::new(point(px(0.0), px(0.0)), size(px(320.0), px(20.0)));

    let link_position = block.read_with(cx, |block, _cx| {
        let span = block
            .inline_spans()
            .iter()
            .find(|span| span.link.is_some())
            .expect("link span should exist");
        let layout = &lines[0];
        let start = layout
            .position_for_index(span.range.start, px(20.0))
            .expect("start position");
        let end = layout
            .position_for_index(span.range.end, px(20.0))
            .expect("end position");
        point((start.x + end.x) / 2.0, px(10.0))
    });

    block.update(cx, |block, _cx| {
        block.last_layout = Some(lines.clone());
        block.last_bounds = Some(bounds);
        block.last_line_height = px(20.0);
        block.selected_range = 0..0;
    });

    let mut event = MouseDownEvent {
        button: MouseButton::Left,
        position: link_position,
        modifiers: Modifiers::default(),
        click_count: 1,
        first_mouse: false,
    };

    // A plain click on the link moves the caret into the text for editing.
    cx.update(|window, app| {
        block.update(app, |block, cx| block.on_mouse_down(&event, window, cx));
    });
    block.read_with(cx, |block, _cx| {
        assert_ne!(block.selected_range, 0..0);
    });

    let link_range = block.read_with(cx, |block, _cx| {
        block
            .inline_spans()
            .iter()
            .find(|span| span.link.is_some())
            .expect("link span should exist")
            .range
            .clone()
    });

    // A plain double click selects the label's word rather than activating it.
    block.update(cx, |block, _cx| block.selected_range = 0..0);
    event.click_count = 2;
    cx.update(|window, app| {
        block.update(app, |block, block_cx| {
            block.on_mouse_down(&event, window, block_cx);
            block.on_mouse_up(
                &MouseUpEvent {
                    button: MouseButton::Left,
                    position: link_position,
                    modifiers: Modifiers::default(),
                    click_count: 2,
                },
                window,
                block_cx,
            );
        });
    });
    block.read_with(cx, |block, _cx| {
        assert_eq!(block.selected_range, link_range)
    });

    // Ctrl+click leaves text selection untouched and follows the link on release.
    block.update(cx, |block, _cx| block.selected_range = 0..0);
    event.click_count = 1;
    event.modifiers = Modifiers::secondary_key();
    cx.update(|window, app| {
        block.update(app, |block, block_cx| {
            block.on_mouse_down(&event, window, block_cx);
            block.on_mouse_up(
                &MouseUpEvent {
                    button: MouseButton::Left,
                    position: link_position,
                    modifiers: Modifiers::secondary_key(),
                    click_count: 1,
                },
                window,
                block_cx,
            );
        });
    });
    block.read_with(cx, |block, _cx| {
        assert_eq!(block.selected_range, 0..0);
        assert!(!block.is_selecting);
    });
}

#[gpui::test]
async fn link_hit_respects_center_alignment(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let block = cx.new(|cx| {
        let mut block = Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::Paragraph,
                InlineTextTree::from_markdown("[link](https://example.com)"),
            ),
        );
        block.set_table_cell_mode(
            TableCellPosition { row: 0, column: 0 },
            crate::components::TableColumnAlignment::Center,
        );
        block
    });

    let display_text = block.read_with(cx, |block, _cx| block.display_text().to_string());
    let lines = shaped_lines(&display_text, px(240.0), cx);
    let (miss_left, hit_center) = block.read_with(cx, |block, _cx| {
        let bounds = Bounds::new(point(px(0.0), px(0.0)), size(px(240.0), px(20.0)));
        let span = block
            .inline_spans()
            .iter()
            .find(|span| span.link.is_some())
            .expect("link span should exist");
        let layout = &lines[0];
        let origin_x = super::aligned_line_left(layout, bounds, block.text_align());
        let start = layout
            .position_for_index(span.range.start, px(20.0))
            .expect("start position");
        let end = layout
            .position_for_index(span.range.end, px(20.0))
            .expect("end position");
        let miss_left = point(origin_x - px(12.0), px(10.0));
        let hit_center = point(origin_x + (start.x + end.x) / 2.0, px(10.0));
        (
            link_at_position(block, &lines, bounds, px(20.0), miss_left)
                .map(|link| link.open_target.clone()),
            link_at_position(block, &lines, bounds, px(20.0), hit_center)
                .map(|link| link.open_target.clone()),
        )
    });

    assert_eq!(miss_left, None);
    assert_eq!(hit_center, Some("https://example.com".to_string()));
}

#[gpui::test]
async fn text_runs_apply_inline_html_color_and_background(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::Paragraph,
                InlineTextTree::from_markdown(
                    "before <span style='color:blue;background-color:#ff0'>marked</span>",
                ),
            ),
        )
    });

    block.read_with(cx, |block, _cx| {
        let display_text: SharedString = block.display_text().to_string().into();
        let base_run = TextRun {
            len: display_text.len(),
            font: font(".SystemUIFont"),
            color: Hsla::from(rgba(0xffffffff)),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let runs = super::build_text_runs(
            block,
            &display_text,
            &base_run,
            px(1.0),
            Hsla::from(rgba(0x0066ccff)),
            Hsla::from(rgba(0x111111ff)),
            Hsla::from(rgba(0xdc2626ff)),
            true,
        );
        let marked_run = runs.last().expect("styled text should create a final run");

        assert_eq!(block.display_text(), "before marked");
        assert_eq!(marked_run.len, "marked".len());
        assert_eq!(marked_run.color, Hsla::from(rgba(0x0000ffff)));
        assert_eq!(
            marked_run.background_color,
            Some(Hsla::from(rgba(0xffff00ff)))
        );
    });
}

#[gpui::test]
async fn spelling_diagnostic_uses_wavy_danger_underline(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let danger = Hsla::from(rgba(0xdc2626ff));
    let block = cx.new(|cx| {
        let mut block = Block::with_record(cx, BlockRecord::paragraph("bad wrd".to_owned()));
        block.spelling_diagnostics = vec![crate::spellcheck::SpellingDiagnostic {
            range: 4..7,
            original: "wrd".to_owned(),
            message: "Unknown word".to_owned(),
            replacements: vec!["word".to_owned()],
        }]
        .into();
        block
    });

    block.read_with(cx, |block, _cx| {
        let display_text: SharedString = block.display_text().to_string().into();
        let base_run = TextRun {
            len: display_text.len(),
            font: font(".SystemUIFont"),
            color: Hsla::from(rgba(0xffffffff)),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let runs = super::build_text_runs(
            block,
            &display_text,
            &base_run,
            px(1.0),
            Hsla::from(rgba(0x0066ccff)),
            Hsla::from(rgba(0x111111ff)),
            danger,
            true,
        );
        let underline = runs
            .iter()
            .find(|run| run.len == 3 && run.underline.is_some())
            .and_then(|run| run.underline)
            .expect("misspelling run should be underlined");
        assert!(underline.wavy);
        assert_eq!(underline.color, Some(danger));
    });
}

#[gpui::test]
async fn text_runs_reject_stale_non_utf8_composition_boundaries(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let block = cx.new(|cx| {
        let mut block = Block::with_record(cx, BlockRecord::paragraph("中文".to_owned()));
        block.marked_range = Some(1..2);
        block
    });

    block.read_with(cx, |block, _cx| {
        let display_text: SharedString = block.display_text().to_string().into();
        let base_run = TextRun {
            len: display_text.len(),
            font: font(".SystemUIFont"),
            color: Hsla::from(rgba(0xffffffff)),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let runs = super::build_text_runs(
            block,
            &display_text,
            &base_run,
            px(1.0),
            Hsla::from(rgba(0x0066ccff)),
            Hsla::from(rgba(0x111111ff)),
            Hsla::from(rgba(0xdc2626ff)),
            true,
        );

        let mut boundary = 0usize;
        for run in runs {
            boundary += run.len;
            assert!(display_text.is_char_boundary(boundary));
        }
        assert_eq!(boundary, display_text.len());
    });
}

#[gpui::test]
async fn soft_wrapped_range_segments_stay_within_wrap_width(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let text = "abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyz";
    let lines = shaped_lines(text, px(80.0), cx);
    assert!(
        !lines[0].wrap_boundaries().is_empty(),
        "test text should soft-wrap"
    );

    let bounds = Bounds::new(point(px(0.0), px(0.0)), size(px(80.0), px(120.0)));
    let segments = super::range_segment_bounds(
        &lines,
        bounds,
        px(20.0),
        text,
        0..text.len(),
        TextAlign::Left,
    );

    assert!(segments.len() > 1);
    for segment in segments {
        assert!(segment.left() >= bounds.left());
        assert!(segment.right() <= bounds.right() + px(0.5));
    }
}

#[gpui::test]
async fn wrapped_link_hit_matches_only_visible_segments(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let label = "abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyz";
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::Paragraph,
                InlineTextTree::from_markdown(&format!("[{label}](https://example.com)")),
            ),
        )
    });

    let display_text = block.read_with(cx, |block, _cx| block.display_text().to_string());
    let lines = shaped_lines(&display_text, px(80.0), cx);
    assert!(
        !lines[0].wrap_boundaries().is_empty(),
        "link text should soft-wrap"
    );

    let (hit, miss_right) = block.read_with(cx, |block, _cx| {
        let bounds = Bounds::new(point(px(0.0), px(0.0)), size(px(80.0), px(120.0)));
        let span = block
            .inline_spans()
            .iter()
            .find(|span| span.link.is_some())
            .expect("link span should exist");
        let segments = super::range_segment_bounds(
            &lines,
            bounds,
            px(20.0),
            &display_text,
            span.range.clone(),
            block.text_align(),
        );
        assert!(segments.len() > 1);
        let second_segment = segments[1];
        let hit = point(
            (second_segment.left() + second_segment.right()) / 2.0,
            (second_segment.top() + second_segment.bottom()) / 2.0,
        );
        let miss_right = point(second_segment.right() + px(24.0), hit.y);
        (
            link_at_position(block, &lines, bounds, px(20.0), hit)
                .map(|link| link.open_target.clone()),
            link_at_position(block, &lines, bounds, px(20.0), miss_right)
                .map(|link| link.open_target.clone()),
        )
    });

    assert_eq!(hit, Some("https://example.com".to_string()));
    assert_eq!(miss_right, None);
}

#[gpui::test]
async fn wrapped_hard_line_top_accumulates_soft_wrap_height(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let text = "abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyz\nnext";
    let lines = shaped_lines(text, px(80.0), cx);
    assert_eq!(lines.len(), 2);
    assert!(
        !lines[0].wrap_boundaries().is_empty(),
        "first hard line should soft-wrap"
    );

    let first_height = lines[0].size(px(20.0)).height;
    assert!(first_height > px(20.0));
    assert_eq!(super::wrapped_line_top(&lines, px(20.0), 1), first_height);
}
