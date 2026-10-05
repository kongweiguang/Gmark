// @author kongweiguang

use super::*;
use crate::{
    self as gpui, FontId, GlyphId, ShapedGlyph, ShapedRun, Styled, TestAppContext, canvas, point,
    px, size,
};
use std::{cell::Cell, rc::Rc};

/// Uses fractional glyph advances so repeated f32 placement exposes long-line drift.
fn long_fractional_line(glyph_count: usize) -> (LineLayout, Pixels) {
    let mut x = px(0.0);
    let glyphs = (0..glyph_count)
        .map(|index| {
            let glyph = ShapedGlyph {
                id: GlyphId(1),
                position: point(x, px(0.0)),
                index,
                is_emoji: false,
            };
            x += px(7.2);
            glyph
        })
        .collect();
    let tail_x = x - px(7.2);
    (
        LineLayout {
            font_size: px(16.0),
            width: x,
            ascent: px(12.0),
            descent: px(4.0),
            runs: vec![ShapedRun {
                font_id: FontId(0),
                glyphs,
            }],
            len: glyph_count,
        },
        tail_x,
    )
}

/// Marks only the final glyph so scene bounds reveal where the renderer placed the line tail.
fn tail_decorations(glyph_count: usize) -> Vec<DecorationRun> {
    vec![
        DecorationRun {
            len: glyph_count.saturating_sub(1) as u32,
            color: black(),
            background_color: None,
            underline: None,
            strikethrough: None,
        },
        DecorationRun {
            len: 1,
            color: black(),
            background_color: Some(black()),
            underline: Some(UnderlineStyle {
                thickness: px(1.0),
                color: Some(black()),
                wavy: false,
            }),
            strikethrough: None,
        },
    ]
}

/// 在 Canvas 绘制回调内读取 Scene，避免后续空窗口刷新清空被验证的输出。
fn assert_tail_position(
    cx: &mut TestAppContext,
    layout: LineLayout,
    origin: Point<Pixels>,
    align: TextAlign,
    align_width: Option<Pixels>,
    wraps: Vec<WrapBoundary>,
    expected_x: f32,
) {
    let decorations = tail_decorations(layout.len);
    let geometry = Rc::new(Cell::new(None));
    let painted_geometry = geometry.clone();
    let cx = cx.add_empty_window();

    cx.draw(
        point(px(0.0), px(0.0)),
        size(px(800.0), px(64.0)),
        |_, _| {
            canvas(
                |_, _, _| (),
                move |_, _, window, cx| {
                    paint_line(
                        origin,
                        &layout,
                        px(20.0),
                        align,
                        align_width,
                        &decorations,
                        &wraps,
                        window,
                        cx,
                    )
                    .expect("synthetic glyph line should paint");
                    paint_line_background(
                        origin,
                        &layout,
                        px(20.0),
                        align,
                        align_width,
                        &decorations,
                        &wraps,
                        window,
                        cx,
                    )
                    .expect("synthetic glyph backgrounds should paint");
                    let scene = &window.next_frame.scene;
                    painted_geometry.set(Some((
                        scene.underlines.last().map(|line| line.bounds.origin.x.0),
                        scene.quads.last().map(|quad| quad.bounds.origin.x.0),
                        window.scale_factor(),
                    )));
                },
            )
            .w(px(800.0))
            .h(px(64.0))
        },
    );

    let (underline_x, background_x, scale_factor) =
        geometry.get().expect("canvas painted geometry");
    let underline_x = underline_x.expect("tail underline remains inside viewport");
    let background_x = background_x.expect("tail background remains inside viewport");
    assert!(
        (underline_x / scale_factor - expected_x).abs() < 0.5,
        "tail underline began at {}, expected shaped x {}",
        underline_x / scale_factor,
        expected_x
    );
    assert!(
        (background_x / scale_factor - expected_x).abs() < 0.5,
        "tail background began at {}, expected shaped x {}",
        background_x / scale_factor,
        expected_x
    );
}

/// 大幅横向平移后的尾字装饰必须与 shaped 光标位置吻合，不能随字数漂移。
#[gpui::test]
fn long_negative_origin_keeps_tail_decoration_at_shaped_position(cx: &mut TestAppContext) {
    let (layout, tail_x) = long_fractional_line(65_536);
    let origin = point(px(48.0) - layout.width, px(12.0));
    let expected = f32::from(origin.x + tail_x);
    assert_tail_position(cx, layout, origin, TextAlign::Left, None, vec![], expected);
}

/// 第二显示行重新建立原点；不等长软换行仍须保持左、中、右对齐的原有位置。
#[gpui::test]
fn wrapped_tail_decorations_keep_alignment(cx: &mut TestAppContext) {
    for (align, expected) in [
        (TextAlign::Left, 46.0),
        (TextAlign::Center, 74.4),
        (TextAlign::Right, 102.8),
    ] {
        let (layout, _) = long_fractional_line(10);
        assert_tail_position(
            cx,
            layout,
            point(px(10.0), px(12.0)),
            align,
            Some(px(100.0)),
            vec![WrapBoundary {
                run_ix: 0,
                glyph_ix: 4,
            }],
            expected,
        );
    }
}
