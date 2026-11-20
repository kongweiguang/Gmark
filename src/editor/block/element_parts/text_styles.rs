// @author kongweiguang

use super::*;

/// 输入法候选范围与布局共用虚拟文本坐标，避免样式边界错位。
pub(super) fn build_text_runs(
    input: &Block,
    display_text: &SharedString,
    base_run: &TextRun,
    underline_thickness: Pixels,
    link_color: Hsla,
    code_bg: Hsla,
    spelling_color: Hsla,
    show_inline_code_backgrounds: bool,
) -> Vec<TextRun> {
    let spans = input
        .inline_spans()
        .iter()
        .map(|span| {
            (
                input.ime_visible_range(&BlockImeCompositionOwner::BlockText, span.range.clone()),
                span,
            )
        })
        .collect::<Vec<_>>();
    let ime_marked_range = input.ime_marked_range(&BlockImeCompositionOwner::BlockText);
    let marked_range = ime_marked_range.as_ref().or(input.marked_range.as_ref());
    let mut boundaries = vec![0, display_text.len()];
    for (range, _) in &spans {
        boundaries.push(range.start);
        boundaries.push(range.end);
    }
    if let Some(marked_range) = marked_range {
        boundaries.push(marked_range.start);
        boundaries.push(marked_range.end);
    }
    for diagnostic in input.spelling_diagnostics.iter() {
        boundaries.push(diagnostic.range.start.min(display_text.len()));
        boundaries.push(diagnostic.range.end.min(display_text.len()));
    }
    // IME composition and a sibling split view can briefly expose a stale
    // marked/style range while the shared UTF-8 text has already advanced.
    // GPUI requires every TextRun boundary to be a valid byte boundary; passing
    // a UTF-16-style or out-of-range offset reaches DirectWrite as an invalid
    // string slice and panics the whole window.
    boundaries.retain(|offset| display_text.is_char_boundary(*offset));
    boundaries.sort_unstable();
    boundaries.dedup();

    let mut runs = Vec::new();
    let mut span_idx = 0usize;
    for boundary_pair in boundaries.windows(2) {
        let start = boundary_pair[0];
        let end = boundary_pair[1];
        if start >= end {
            continue;
        }

        // Spans are stored in ascending order and boundaries are sorted, so
        // we can advance a single index instead of re-scanning per boundary.
        while span_idx < spans.len() && spans[span_idx].0.end <= start {
            span_idx += 1;
        }
        let active_span = spans
            .get(span_idx)
            .filter(|(range, _)| range.start <= start && start < range.end)
            .map(|(_, span)| *span);

        let inline_style = active_span.map(|s| s.style).unwrap_or_default();
        let html_style = active_span.and_then(|s| s.html_style);
        let is_link = active_span.map(|s| s.link.is_some()).unwrap_or(false);
        let is_footnote = active_span.map(|s| s.footnote.is_some()).unwrap_or(false);
        let is_marked = marked_range
            .map(|range| start < range.end && range.start < end)
            .unwrap_or(false);
        let is_misspelled = input
            .spelling_diagnostics
            .iter()
            .any(|diagnostic| start < diagnostic.range.end && diagnostic.range.start < end);

        let mut font = base_run.font.clone();
        if inline_style.bold && font.weight < FontWeight::BOLD {
            font.weight = FontWeight::BOLD;
        }
        if inline_style.italic {
            font.style = FontStyle::Italic;
        }

        let mut run_color = if is_link || is_footnote {
            link_color
        } else {
            base_run.color
        };
        if let Some(style) = html_style
            && let Some(color) = style.color
        {
            run_color = html_css_color_to_hsla(color, run_color);
        }
        let underline = if inline_style.underline || is_marked || is_link || is_footnote {
            Some(UnderlineStyle {
                color: Some(run_color),
                thickness: underline_thickness,
                wavy: false,
            })
        } else if is_misspelled {
            Some(UnderlineStyle {
                color: Some(spelling_color),
                thickness: underline_thickness,
                wavy: true,
            })
        } else {
            None
        };
        let strikethrough = inline_style.strikethrough.then_some(StrikethroughStyle {
            color: Some(run_color),
            thickness: underline_thickness,
        });

        let mut background_color = if show_inline_code_backgrounds && inline_style.code {
            Some(code_bg)
        } else if inline_style.highlight {
            // 高亮是内容语义而非临时选区；沿用主题感知的行内代码底色，保证
            // 自定义浅/深主题下仍有可读对比度，后续主题令牌可无损替换。
            Some(code_bg.opacity(0.72))
        } else {
            base_run.background_color
        };
        if let Some(style) = html_style
            && let Some(color) = style.background_color
        {
            background_color = Some(html_css_color_to_hsla(color, run_color));
        }

        runs.push(TextRun {
            len: end - start,
            font,
            color: run_color,
            background_color,
            underline,
            strikethrough,
        });
    }

    if runs.is_empty() {
        vec![base_run.clone()]
    } else {
        runs
    }
}

/// 保留 currentColor 的继承语义，避免 HTML 样式脱离基础文字颜色。
fn html_css_color_to_hsla(color: HtmlCssColor, current_color: Hsla) -> Hsla {
    match color {
        HtmlCssColor::CurrentColor => current_color,
        HtmlCssColor::Rgba(color) => Hsla::from(Rgba {
            r: color.red as f32 / 255.0,
            g: color.green as f32 / 255.0,
            b: color.blue as f32 / 255.0,
            a: color.alpha.clamp(0.0, 1.0),
        }),
    }
}

/// 语法颜色与输入法预编辑下划线共用虚拟源码坐标。
pub(super) fn build_code_text_runs(
    input: &Block,
    display_text: &SharedString,
    base_run: &TextRun,
    underline_thickness: Pixels,
    colors: &ThemeColors,
) -> Vec<TextRun> {
    let highlight_spans = input
        .code_highlight_result()
        .map(|result| {
            result
                .spans
                .iter()
                .map(|span| {
                    (
                        input.ime_visible_range(
                            &BlockImeCompositionOwner::BlockText,
                            span.range.clone(),
                        ),
                        span,
                    )
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let mut boundaries = vec![0, display_text.len()];
    for (range, _) in &highlight_spans {
        boundaries.push(range.start);
        boundaries.push(range.end);
    }
    let ime_marked_range = input.ime_marked_range(&BlockImeCompositionOwner::BlockText);
    let marked_range = ime_marked_range.as_ref().or(input.marked_range.as_ref());
    if let Some(marked_range) = marked_range {
        boundaries.push(marked_range.start);
        boundaries.push(marked_range.end);
    }
    boundaries.retain(|offset| display_text.is_char_boundary(*offset));
    boundaries.sort_unstable();
    boundaries.dedup();

    let mut runs = Vec::new();
    let mut span_idx = 0usize;
    for boundary_pair in boundaries.windows(2) {
        let start = boundary_pair[0];
        let end = boundary_pair[1];
        if start >= end {
            continue;
        }

        let is_marked = marked_range
            .map(|range| start < range.end && range.start < end)
            .unwrap_or(false);
        while span_idx < highlight_spans.len() && highlight_spans[span_idx].0.end <= start {
            span_idx += 1;
        }
        let run_color = highlight_spans
            .get(span_idx)
            .filter(|(range, _)| range.start <= start && start < range.end)
            .map(|(_, span)| *span)
            .map(|span| code_highlight_color(colors, span.class))
            .unwrap_or(base_run.color);

        runs.push(TextRun {
            len: end - start,
            font: base_run.font.clone(),
            color: run_color,
            background_color: base_run.background_color,
            underline: is_marked.then_some(UnderlineStyle {
                color: Some(run_color),
                thickness: underline_thickness,
                wavy: false,
            }),
            strikethrough: None,
        });
    }

    if runs.is_empty() {
        vec![base_run.clone()]
    } else {
        runs
    }
}
