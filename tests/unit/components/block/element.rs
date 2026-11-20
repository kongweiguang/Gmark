// @author kongweiguang

use super::text::{code_language_cursor_bounds, reshape_plain_source};
use super::{
    link_at_position, source_line_number_gutter_width, source_line_number_tops, source_text_bounds,
    visible_hard_lines, wrapped_line_height,
};
use crate::components::{Block, BlockKind, BlockRecord, InlineTextTree, TableCellPosition};
use gpui::{
    AppContext, Bounds, Hsla, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    SharedString, TestAppContext, TextAlign, TextRun, VisualTestContext, font, point, px, rgba,
    size,
};

/// 候选跨语法 token 时，颜色与下划线必须按虚拟文本字节边界同时保留。
#[test]
fn source_line_ime_decorations_keep_colors_and_marked_range() {
    let base = Hsla::from(rgba(0xffffffff));
    let keyword = Hsla::from(rgba(0xff0000ff));
    let string = Hsla::from(rgba(0x00ff00ff));
    let runs = super::source_line_decorations(
        10..20,
        &[(10..15, keyword), (17..20, string)],
        base,
        Some(&(13..18)),
        px(1.0),
    );
    let actual = runs
        .iter()
        .map(|run| (run.len, run.color, run.underline.is_some()))
        .collect::<Vec<_>>();
    assert_eq!(
        actual,
        vec![
            (3, keyword, false),
            (2, keyword, true),
            (2, base, true),
            (1, string, true),
            (2, string, false),
        ]
    );
}

/// 多字节候选替换源码后，后续硬行仍按虚拟字节坐标查找已有语法颜色。
#[cfg(target_os = "windows")]
#[gpui::test]
async fn source_line_ime_decorations_follow_multibyte_replacement(cx: &mut TestAppContext) {
    use crate::components::markdown::code_highlight::{
        CodeHighlightClass, CodeHighlightResult, CodeHighlightSpan, CodeLanguageKey,
    };
    use gpui::EntityInputHandler;

    let cx = cx.add_empty_window();
    let lines = shaped_lines("漢字\nbb\ncc", px(500.0), cx);
    let block = cx.new(|cx| Block::with_record(cx, BlockRecord::paragraph("aa\nbb\ncc")));
    let highlight = CodeHighlightResult {
        language: CodeLanguageKey::Markdown,
        spans: vec![
            CodeHighlightSpan {
                range: 3..5,
                class: CodeHighlightClass::Keyword,
            },
            CodeHighlightSpan {
                range: 6..8,
                class: CodeHighlightClass::String,
            },
        ],
    };
    cx.update(|window, cx| {
        crate::theme::ThemeManager::init(cx);
        let theme = cx.global::<crate::theme::ThemeManager>().current_arc();
        block.update(cx, |block, block_cx| {
            block.selected_range = 0..2;
            <Block as EntityInputHandler>::composition_started(block, window, block_cx);
            <Block as EntityInputHandler>::replace_and_mark_text_in_range(
                block,
                None,
                "漢字",
                Some(2..2),
                window,
                block_cx,
            );
            let marked = block.ime_marked_range(&super::BlockImeCompositionOwner::BlockText);
            let decorate = |line: usize, start: usize| {
                super::decorate_source_line(
                    &lines[line],
                    start,
                    &highlight,
                    block,
                    Hsla::from(rgba(0xffffffff)),
                    &theme.colors,
                    marked.as_ref(),
                    px(1.0),
                )
            };
            assert!(decorate(1, 3).is_none());
            assert!(decorate(1, 7).is_some());
            assert!(decorate(2, 10).is_some());
        });
    });
}

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

/// 比较整篇重新排版，确保编辑复用字形时不丢 Unicode、重复硬行及末尾空行。
#[gpui::test]
async fn incremental_source_layout_matches_full_shape_after_line_edits(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let width = px(70.0);
    for (before, after) in [
        (
            "首行\n重复\n重复\n尾行\n",
            "首行\n重复\n插入字行\n重复\n尾行\n",
        ),
        ("首行\n重复\n重复\n尾行\n", "首行\n重复\n尾行\n"),
        ("首行\n行尾\n", "首行行尾"),
        ("首行", "首行\n"),
        ("\n", ""),
        ("尾行\n", "首行\n尾行\n"),
    ] {
        let previous = shaped_lines(before, width, cx);
        let expected = shaped_lines(after, width, cx);
        let actual = cx.update(|window, _app| {
            reshape_plain_source(
                window.text_system(),
                &previous,
                after,
                expected.len(),
                &TextRun {
                    len: after.len(),
                    font: font(".SystemUIFont"),
                    color: Hsla::from(rgba(0xffffffff)),
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                },
                px(16.0),
                Some(width),
            )
            .expect("changed lines should shape")
        });
        assert_eq!(actual.len(), expected.len());
        for (actual, expected) in actual.iter().zip(&expected) {
            assert_eq!(actual.text, expected.text);
            assert_eq!(actual.size(px(21.0)), expected.size(px(21.0)));
        }
    }
}

/// 裁剪跨越软换行时仍需保留完整硬行，但只给首个可见的硬行位置画行号。
#[gpui::test]
async fn visible_hard_lines_keep_wrapped_text_but_hide_clipped_numbers(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let lines = shaped_lines(
        "long long long long long long long\nsecond\nthird",
        px(50.0),
        cx,
    );
    let line_height = px(20.0);
    let first_height = wrapped_line_height(&lines[0], line_height);
    assert!(first_height > line_height);
    let visible = visible_hard_lines(
        &lines,
        line_height,
        px(0.0),
        line_height,
        first_height + line_height,
    );
    assert_eq!(
        visible.iter().map(|line| line.index).collect::<Vec<_>>(),
        vec![0, 1]
    );
    assert!(!visible[0].number_visible);
    assert!(visible[1].number_visible);
    assert_eq!(visible[1].top, first_height);
}

/// 多行文档只产生视口中的绘制任务；行号仍从全文的硬行序号计算。
#[gpui::test]
async fn visible_hard_lines_limit_work_and_keep_document_line_numbers(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let source = "line\n".repeat(10_000);
    let lines = shaped_lines(&source, px(500.0), cx);
    let visible = visible_hard_lines(&lines, px(20.0), px(0.0), px(100_000.0), px(100_060.0));
    assert_eq!(
        visible.iter().map(|line| line.index).collect::<Vec<_>>(),
        vec![5_000, 5_001, 5_002]
    );
    assert!(visible.iter().all(|line| line.number_visible));
    assert!(visible_hard_lines(&lines, px(20.0), px(0.0), px(300_000.0), px(300_020.0)).is_empty());
    assert!(visible_hard_lines(&lines, px(20.0), px(0.0), px(20.0), px(20.0)).is_empty());
}

/// 深滚动时绘制使用与光标相同的相对坐标累加顺序，防止浮点舍入让文字偏离插入点。
#[gpui::test]
async fn visible_hard_lines_match_cursor_geometry_at_large_offsets(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let sample = shaped_lines("line", px(500.0), cx);
    let lines = vec![sample[0].clone(); 200_000];
    let line_height = px(20.3);
    let index = 190_000;
    let mut relative = px(0.0);
    for line in &lines[..index] {
        relative += wrapped_line_height(line, line_height);
    }
    let origin = -relative + px(50.0);
    let expected = origin + relative;
    let visible = visible_hard_lines(&lines, line_height, origin, expected, expected + px(1.0));
    let line = visible
        .iter()
        .find(|line| line.index == index)
        .expect("target line must be visible");
    assert_eq!(line.top, expected);
}

/// 全选必须保留真实字节选区，但源码重绘只应生成视口相交硬行的几何。
#[gpui::test]
async fn select_all_paints_only_visible_source_lines(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let source = "中文 line\n".repeat(10_000);
    let lines = shaped_lines(&source, px(500.0), cx);
    let line_height = px(20.0);
    let bounds = Bounds::new(
        point(px(0.0), px(-100_000.0)),
        size(px(500.0), px(200_020.0)),
    );
    let visible = visible_hard_lines(&lines, line_height, bounds.top(), px(0.0), px(60.0));
    let selection = super::visible_selection_bounds(
        &lines,
        &visible,
        bounds,
        line_height,
        0..source.len(),
        TextAlign::Left,
        px(0.0)..px(60.0),
    );
    assert_eq!(selection.len(), 3);
    assert_eq!(selection[0].top(), px(0.0));
    assert_eq!(selection[2].bottom(), px(60.0));
    let caret_offset = "中文 line\n".len() * 5_001 + "中文".len();
    let caret = super::visible_cursor_bounds(
        &lines,
        &visible,
        bounds,
        line_height,
        caret_offset,
        TextAlign::Left,
        px(1.0),
    );
    assert_eq!(
        caret,
        super::cursor_bounds_for_offset(
            &lines,
            bounds,
            line_height,
            &source,
            caret_offset,
            TextAlign::Left,
            px(1.0),
        ),
    );
    assert!(
        super::visible_cursor_bounds(
            &lines,
            &visible,
            bounds,
            line_height,
            0,
            TextAlign::Left,
            px(1.0),
        )
        .is_none()
    );
}

/// 一行有大量软换行时，全选也只能为视口中的软换行行生成矩形。
#[gpui::test]
async fn source_selection_does_not_expand_offscreen_soft_wraps(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let source = "中文 words ".repeat(200);
    let lines = shaped_lines(&source, px(60.0), cx);
    let line_height = px(20.0);
    let bounds = Bounds::new(point(px(0.0), px(-100.0)), size(px(60.0), px(5_000.0)));
    let visible = visible_hard_lines(&lines, line_height, bounds.top(), px(0.0), px(60.0));
    assert_eq!(visible.len(), 1);
    let selection = super::visible_selection_bounds(
        &lines,
        &visible,
        bounds,
        line_height,
        0..source.len(),
        TextAlign::Left,
        px(0.0)..px(60.0),
    );
    assert_eq!(selection.len(), 3);
    assert_eq!(selection[0].top(), px(0.0));
    assert_eq!(selection[2].bottom(), px(60.0));
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
/// 布局共享后仍需保留精确的鼠标命中与选区坐标。
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
        block.last_layout = Some(std::sync::Arc::new(lines));
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
/// 链接命中复用共享布局，不能改变编辑与跟随链接的手势区分。
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
        block.last_layout = Some(std::sync::Arc::new(lines.clone()));
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

#[path = "element_rendering.rs"]
mod element_rendering;
