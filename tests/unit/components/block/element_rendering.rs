// @author kongweiguang

use super::*;

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
        let runs = super::super::build_text_runs(
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
        let runs = super::super::build_text_runs(
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
        let runs = super::super::build_text_runs(
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
    let segments = super::super::range_segment_bounds(
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
        let segments = super::super::range_segment_bounds(
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
    assert_eq!(
        super::super::wrapped_line_top(&lines, px(20.0), 1),
        first_height
    );
}
