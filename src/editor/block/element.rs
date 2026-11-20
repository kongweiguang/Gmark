// @author kongweiguang

use std::cell::{Cell, RefCell};
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use gpui::*;
use smallvec::SmallVec;

use super::{
    Block, BlockImeCompositionOwner, InlineFootnoteHit, InlineLinkHit, SourceLayoutCacheKey,
    SourceLayoutMeasurement, code_highlight_color,
};
use crate::components::HtmlCssColor;
use crate::theme::{ThemeColors, ThemeManager};

const SOURCE_LINE_NUMBER_MIN_DIGITS: usize = 2;
const SOURCE_LINE_NUMBER_GAP: f32 = 12.0;
const SOURCE_LINE_NUMBER_DIGIT_WIDTH_RATIO: f32 = 0.62;

/// 仅构造当前可见硬行的绘制片段，避免高亮更新导致全文重新排版。
fn source_line_decorations(
    line_range: Range<usize>,
    spans: &[(Range<usize>, Hsla)],
    base: Hsla,
    marked_range: Option<&Range<usize>>,
    underline_thickness: Pixels,
) -> SmallVec<[DecorationRun; 8]> {
    let mut runs = SmallVec::<[DecorationRun; 8]>::new();
    let mut start = line_range.start;
    let mut push = |range: Range<usize>, color| {
        let mut boundaries = [range.start, range.end, range.end, range.end];
        if let Some(marked) = marked_range {
            boundaries[1] = marked.start.clamp(range.start, range.end);
            boundaries[2] = marked.end.clamp(range.start, range.end);
            boundaries.sort_unstable();
        }
        for pair in boundaries.windows(2).filter(|pair| pair[0] < pair[1]) {
            let is_marked =
                marked_range.is_some_and(|marked| pair[0] < marked.end && marked.start < pair[1]);
            runs.push(DecorationRun {
                len: (pair[1] - pair[0]) as u32,
                color,
                background_color: None,
                underline: is_marked.then_some(UnderlineStyle {
                    color: Some(color),
                    thickness: underline_thickness,
                    wavy: false,
                }),
                strikethrough: None,
            });
        }
    };
    for (span, color) in spans {
        let span_start = start.max(span.start);
        let span_end = line_range.end.min(span.end);
        if span_end > span_start {
            push(start..span_start, base);
            push(span_start..span_end, *color);
            start = span_end;
        }
    }
    push(start..line_range.end, base);
    runs
}

/// 语法颜色与候选下划线仅覆盖可见硬行；候选替换后的偏移沿用输入法的源码映射。
fn decorate_source_line(
    line: &WrappedLine,
    byte_start: usize,
    highlight: &super::CodeHighlightResult,
    input: &Block,
    base: Hsla,
    colors: &ThemeColors,
    marked_range: Option<&Range<usize>>,
    underline_thickness: Pixels,
) -> Option<WrappedLine> {
    let end = byte_start + line.text.len();
    let first = highlight.spans.partition_point(|span| {
        input
            .ime_visible_range(&BlockImeCompositionOwner::BlockText, span.range.clone())
            .end
            <= byte_start
    });
    let mut spans = SmallVec::<[(Range<usize>, Hsla); 8]>::new();
    for span in &highlight.spans[first..] {
        let range =
            input.ime_visible_range(&BlockImeCompositionOwner::BlockText, span.range.clone());
        if range.start >= end {
            break;
        }
        if range.end > byte_start && !range.is_empty() {
            spans.push((range, code_highlight_color(colors, span.class)));
        }
    }
    if spans.is_empty() {
        return None;
    }
    let runs = source_line_decorations(
        byte_start..end,
        &spans,
        base,
        marked_range,
        underline_thickness,
    );
    Some(line.with_decoration_runs(runs))
}

fn source_line_count(text: &str) -> usize {
    text.split('\n').count().max(1)
}

pub(crate) fn source_line_number_gutter_width(line_count: usize, font_size: Pixels) -> Pixels {
    let digits = line_count
        .max(1)
        .to_string()
        .len()
        .max(SOURCE_LINE_NUMBER_MIN_DIGITS);
    px(digits as f32 * f32::from(font_size) * SOURCE_LINE_NUMBER_DIGIT_WIDTH_RATIO)
        + px(SOURCE_LINE_NUMBER_GAP)
}

fn source_text_bounds(bounds: Bounds<Pixels>, gutter_width: Pixels) -> Bounds<Pixels> {
    if gutter_width <= px(0.0) {
        return bounds;
    }

    let max_gutter = (f32::from(bounds.size.width) - 1.0).max(0.0);
    let gutter_width = px(f32::from(gutter_width).min(max_gutter));
    Bounds::new(
        point(bounds.origin.x + gutter_width, bounds.origin.y),
        size(
            (bounds.size.width - gutter_width).max(px(1.0)),
            bounds.size.height,
        ),
    )
}

#[cfg(test)]
/// 为换行回归提供完整硬行坐标，生产绘制只处理裁剪内的行。
fn source_line_number_tops(lines: &[WrappedLine], line_height: Pixels) -> Vec<Pixels> {
    let mut tops = Vec::with_capacity(lines.len());
    let mut y = Pixels::default();
    for line in lines {
        tops.push(y);
        y += wrapped_line_height(line, line_height);
    }
    tops
}

struct VisibleHardLine {
    index: usize,
    top: Pixels,
    byte_start: usize,
    number_visible: bool,
}

/// 按真实裁剪范围保留相交硬行；软换行续行露出时不重复显示行号。
fn visible_hard_lines(
    lines: &[WrappedLine],
    line_height: Pixels,
    origin: Pixels,
    clip_top: Pixels,
    clip_bottom: Pixels,
) -> SmallVec<[VisibleHardLine; 4]> {
    let mut visible = SmallVec::new();
    if clip_top >= clip_bottom {
        return visible;
    }
    let mut relative_top = px(0.0);
    let mut byte_start = 0;
    for (index, line) in lines.iter().enumerate() {
        let top = origin + relative_top;
        let next_relative_top = relative_top + wrapped_line_height(line, line_height);
        let bottom = origin + next_relative_top;
        if top >= clip_bottom {
            break;
        }
        if bottom > clip_top {
            visible.push(VisibleHardLine {
                index,
                top,
                byte_start,
                number_visible: top + line_height > clip_top,
            });
        }
        relative_top = next_relative_top;
        byte_start += line.text.len() + 1;
    }
    visible
}

/// Builds inline styling with candidate ranges resolved in the same virtual text coordinates as layout.
fn build_text_runs(
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

/// Builds syntax runs while applying the staged preedit underline in virtual source coordinates.
fn build_code_text_runs(
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

/// Compute byte ranges of each hard-line (`\n`-separated) segment in the
/// visible text.  Index `i` in the returned Vec corresponds to the `i`-th
/// WrappedLine produced by `shape_text`.
pub(super) fn hard_line_ranges(text: &str) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let mut start = 0;
    for (idx, _) in text.match_indices('\n') {
        ranges.push(start..idx);
        start = idx + 1;
    }
    ranges.push(start..text.len());
    ranges
}

/// Map a flat visible-text offset to `(line_index, offset_within_line)`.
pub(super) fn line_index_for_offset(ranges: &[Range<usize>], offset: usize) -> (usize, usize) {
    let clamped = offset.min(ranges.last().map(|r| r.end).unwrap_or(0));
    for (i, range) in ranges.iter().enumerate() {
        if clamped <= range.end {
            return (i, clamped.saturating_sub(range.start));
        }
    }
    let last = ranges.len() - 1;
    (last, ranges[last].len())
}

pub(crate) fn aligned_line_left(
    line: &WrappedLine,
    bounds: Bounds<Pixels>,
    align: TextAlign,
) -> Pixels {
    let slack = (bounds.size.width - line.width()).max(px(0.0));
    match align {
        TextAlign::Left => bounds.left(),
        TextAlign::Center => bounds.left() + slack / 2.0,
        TextAlign::Right => bounds.left() + slack,
    }
}

pub(super) fn wrapped_line_height(line: &WrappedLine, line_height: Pixels) -> Pixels {
    line.size(line_height).height
}

pub(super) fn wrapped_line_top(
    lines: &[WrappedLine],
    line_height: Pixels,
    line_idx: usize,
) -> Pixels {
    lines.iter().take(line_idx).fold(px(0.0), |height, line| {
        height + wrapped_line_height(line, line_height)
    })
}

pub(super) fn wrapped_line_for_y(
    lines: &[WrappedLine],
    line_height: Pixels,
    relative_y: Pixels,
) -> Option<(usize, Pixels)> {
    if lines.is_empty() {
        return None;
    }

    let mut top = px(0.0);
    for (line_idx, line) in lines.iter().enumerate() {
        let height = wrapped_line_height(line, line_height);
        if relative_y < top + height || line_idx + 1 == lines.len() {
            return Some((line_idx, (relative_y - top).max(px(0.0))));
        }
        top += height;
    }

    Some((lines.len() - 1, px(0.0)))
}

fn wrap_boundary_offset(line: &WrappedLine, wrap_idx: usize) -> Option<usize> {
    let boundary = line.wrap_boundaries().get(wrap_idx)?;
    let run = line.unwrapped_layout.runs.get(boundary.run_ix)?;
    let glyph = run.glyphs.get(boundary.glyph_ix)?;
    Some(glyph.index)
}

fn wrapped_row_offsets(line: &WrappedLine) -> Vec<usize> {
    let mut offsets = Vec::with_capacity(line.wrap_boundaries().len() + 2);
    offsets.push(0);
    for wrap_idx in 0..line.wrap_boundaries().len() {
        if let Some(offset) = wrap_boundary_offset(line, wrap_idx) {
            offsets.push(offset.min(line.len()));
        }
    }
    offsets.push(line.len());
    offsets.dedup();
    offsets
}

/// Resolves the visual row containing a caret, treating a wrap boundary as the next row's start.
pub(super) fn visual_row_range_for_offset(
    line: &WrappedLine,
    offset: usize,
) -> Option<Range<usize>> {
    let offsets = wrapped_row_offsets(line);
    if offsets.len() < 2 {
        return Some(0..line.len());
    }

    let offset = offset.min(line.len());
    let row_count = offsets.len() - 1;
    let row = (0..row_count)
        .find(|row| offset < offsets[row + 1])
        .unwrap_or(row_count - 1);
    Some(offsets[row]..offsets[row + 1])
}

fn wrapped_row_origin_x(
    line: &WrappedLine,
    bounds: Bounds<Pixels>,
    align: TextAlign,
    row_start: usize,
    row_end: usize,
) -> Pixels {
    let row_width =
        line.unwrapped_layout.x_for_index(row_end) - line.unwrapped_layout.x_for_index(row_start);
    let align_width = line.width();
    let slack = (align_width - row_width).max(px(0.0));
    let line_left = aligned_line_left(line, bounds, align);
    match align {
        TextAlign::Left => line_left,
        TextAlign::Center => line_left + slack / 2.0,
        TextAlign::Right => line_left + slack,
    }
}

pub(super) fn position_for_offset(
    line: &WrappedLine,
    offset: usize,
    line_height: Pixels,
    prefer_next_wrap_start: bool,
) -> Option<Point<Pixels>> {
    let offsets = wrapped_row_offsets(line);
    for row_idx in 0..offsets.len().saturating_sub(1) {
        let row_start = offsets[row_idx];
        let row_end = offsets[row_idx + 1];
        let is_start_of_wrapped_row = prefer_next_wrap_start && row_idx > 0 && offset == row_start;
        if is_start_of_wrapped_row || (offset >= row_start && offset < row_end) {
            let row_start_x = line.unwrapped_layout.x_for_index(row_start);
            let x = line.unwrapped_layout.x_for_index(offset) - row_start_x;
            return Some(point(x, line_height * row_idx as f32));
        }
    }

    line.position_for_index(offset, line_height)
}

pub(super) fn cursor_bounds_for_offset(
    lines: &[WrappedLine],
    bounds: Bounds<Pixels>,
    line_height: Pixels,
    text: &str,
    offset: usize,
    align: TextAlign,
    cursor_width: Pixels,
) -> Option<Bounds<Pixels>> {
    let ranges = hard_line_ranges(text);
    let (line_idx, offset_in_line) = line_index_for_offset(&ranges, offset);
    let layout = lines.get(line_idx)?;
    cursor_bounds_in_hard_line(
        layout,
        bounds,
        line_height,
        bounds.top() + wrapped_line_top(lines, line_height, line_idx),
        offset_in_line,
        align,
        cursor_width,
    )
}

/// 视口外的光标无须绘制，复用硬行的字节与纵坐标避免每次闪烁重扫全文。
fn visible_cursor_bounds(
    lines: &[WrappedLine],
    visible: &[VisibleHardLine],
    bounds: Bounds<Pixels>,
    line_height: Pixels,
    offset: usize,
    align: TextAlign,
    cursor_width: Pixels,
) -> Option<Bounds<Pixels>> {
    for row in visible {
        let layout = lines.get(row.index)?;
        if let Some(local) = offset.checked_sub(row.byte_start)
            && local <= layout.text.len()
        {
            return cursor_bounds_in_hard_line(
                layout,
                bounds,
                line_height,
                row.top,
                local,
                align,
                cursor_width,
            );
        }
    }
    None
}

/// 屏幕绘制与 IME 几何共用软换行边界和对齐规则。
fn cursor_bounds_in_hard_line(
    layout: &WrappedLine,
    bounds: Bounds<Pixels>,
    line_height: Pixels,
    line_top: Pixels,
    offset: usize,
    align: TextAlign,
    cursor_width: Pixels,
) -> Option<Bounds<Pixels>> {
    let origin_x = aligned_line_left(layout, bounds, align);
    let cursor_pos = position_for_offset(layout, offset, line_height, true)?;
    Some(Bounds::new(
        point(origin_x + cursor_pos.x, line_top + cursor_pos.y),
        size(cursor_width, line_height),
    ))
}

pub(super) fn range_bounds(
    lines: &[WrappedLine],
    bounds: Bounds<Pixels>,
    line_height: Pixels,
    text: &str,
    range: Range<usize>,
    align: TextAlign,
) -> Option<Bounds<Pixels>> {
    let segments = range_segment_bounds(lines, bounds, line_height, text, range.clone(), align);
    if segments.is_empty() {
        return cursor_bounds_for_offset(
            lines,
            bounds,
            line_height,
            text,
            range.start,
            align,
            px(1.0),
        );
    }

    let mut union = segments[0];
    for segment in segments.iter().skip(1) {
        union = Bounds::from_corners(
            point(
                union.left().min(segment.left()),
                union.top().min(segment.top()),
            ),
            point(
                union.right().max(segment.right()),
                union.bottom().max(segment.bottom()),
            ),
        );
    }
    Some(union)
}

/// 复用调用方已有的行坐标，避免长选区为每行重新累计前面的行高。
fn extend_range_segments_for_hard_line(
    segments: &mut Vec<Bounds<Pixels>>,
    line: &WrappedLine,
    bounds: Bounds<Pixels>,
    line_height: Pixels,
    line_top: Pixels,
    range: Range<usize>,
    align: TextAlign,
    visual_rows: Range<usize>,
) {
    if range.is_empty() {
        return;
    }
    for row_idx in visual_rows {
        let row_start = if row_idx == 0 {
            0
        } else if let Some(offset) = wrap_boundary_offset(line, row_idx - 1) {
            offset
        } else {
            continue;
        };
        let row_end = wrap_boundary_offset(line, row_idx).unwrap_or_else(|| line.len());
        let seg_start = range.start.max(row_start).min(row_end);
        let seg_end = range.end.min(row_end).max(row_start);
        if seg_start >= seg_end {
            continue;
        }

        let row_start_x = line.unwrapped_layout.x_for_index(row_start);
        let start_x = line.unwrapped_layout.x_for_index(seg_start) - row_start_x;
        let end_x = line.unwrapped_layout.x_for_index(seg_end) - row_start_x;
        let origin_x = wrapped_row_origin_x(line, bounds, align, row_start, row_end);
        let y = line_top + line_height * row_idx as f32;
        segments.push(Bounds::from_corners(
            point(origin_x + start_x, y),
            point(origin_x + end_x, y + line_height),
        ));
    }
}

/// 描绘选区只需要当前视口，保留完整字节选区供编辑、复制与辅助功能读取。
fn visible_selection_bounds(
    lines: &[WrappedLine],
    visible: &[VisibleHardLine],
    bounds: Bounds<Pixels>,
    line_height: Pixels,
    range: Range<usize>,
    align: TextAlign,
    viewport: Range<Pixels>,
) -> Vec<Bounds<Pixels>> {
    if visible.is_empty() || range.is_empty() {
        return Vec::new();
    }

    let mut segments = Vec::new();
    for row in visible {
        let Some(line) = lines.get(row.index) else {
            break;
        };
        let start = range.start.max(row.byte_start);
        let end = range.end.min(row.byte_start + line.text.len());
        if start < end {
            let start_row = ((viewport.start - row.top) / line_height).floor().max(0.0) as usize;
            let end_row = (((viewport.end - row.top) / line_height).ceil().max(0.0) as usize)
                .min(line.wrap_boundaries().len() + 1);
            extend_range_segments_for_hard_line(
                &mut segments,
                line,
                bounds,
                line_height,
                row.top,
                start - row.byte_start..end - row.byte_start,
                align,
                start_row..end_row,
            );
        }
    }
    segments
}

/// 输入法等调用方仍可取得完整几何，连续硬行共用累计行高以控制大选区开销。
pub(super) fn range_segment_bounds(
    lines: &[WrappedLine],
    bounds: Bounds<Pixels>,
    line_height: Pixels,
    text: &str,
    range: Range<usize>,
    align: TextAlign,
) -> Vec<Bounds<Pixels>> {
    if range.start >= range.end || lines.is_empty() {
        return Vec::new();
    }

    let ranges = hard_line_ranges(text);
    let (start_line, start_offset) = line_index_for_offset(&ranges, range.start);
    let (end_line, end_offset) = line_index_for_offset(&ranges, range.end);
    let mut segments = Vec::new();
    let mut relative_top = wrapped_line_top(lines, line_height, start_line);

    for (line_idx, (line, hard_range)) in lines
        .iter()
        .zip(&ranges)
        .enumerate()
        .take(end_line + 1)
        .skip(start_line)
    {
        let line_start = if line_idx == start_line {
            start_offset
        } else {
            0
        };
        let line_end = if line_idx == end_line {
            end_offset
        } else {
            hard_range.len()
        };
        extend_range_segments_for_hard_line(
            &mut segments,
            line,
            bounds,
            line_height,
            bounds.top() + relative_top,
            line_start..line_end,
            align,
            0..line.wrap_boundaries().len() + 1,
        );
        relative_top += wrapped_line_height(line, line_height);
    }

    segments
}

fn point_inside_bounds(bounds: Bounds<Pixels>, position: Point<Pixels>) -> bool {
    position.x >= bounds.left()
        && position.x < bounds.right()
        && position.y >= bounds.top()
        && position.y < bounds.bottom()
}

pub(crate) fn link_at_position<'a>(
    input: &'a Block,
    lines: &[WrappedLine],
    bounds: Bounds<Pixels>,
    line_height: Pixels,
    position: Point<Pixels>,
) -> Option<&'a InlineLinkHit> {
    if input.is_source_raw_mode()
        || input.display_text().is_empty()
        || lines.is_empty()
        || position.y < bounds.top()
        || position.y >= bounds.bottom()
    {
        return None;
    }

    let text = input.display_text();
    let align = input.text_align();

    for span in input.inline_spans() {
        let Some(link) = span.link.as_ref() else {
            continue;
        };
        if span.range.is_empty() {
            continue;
        }

        for link_bounds in
            range_segment_bounds(lines, bounds, line_height, text, span.range.clone(), align)
        {
            if point_inside_bounds(link_bounds, position) {
                return Some(link);
            }
        }
    }

    None
}

pub(crate) fn footnote_at_position<'a>(
    input: &'a Block,
    lines: &[WrappedLine],
    bounds: Bounds<Pixels>,
    line_height: Pixels,
    position: Point<Pixels>,
) -> Option<&'a InlineFootnoteHit> {
    if input.is_source_raw_mode()
        || input.display_text().is_empty()
        || lines.is_empty()
        || position.y < bounds.top()
        || position.y >= bounds.bottom()
    {
        return None;
    }

    let text = input.display_text();
    let align = input.text_align();

    for span in input.inline_spans() {
        let Some(footnote) = span.footnote.as_ref() else {
            continue;
        };
        if span.range.is_empty() {
            continue;
        }

        for footnote_bounds in
            range_segment_bounds(lines, bounds, line_height, text, span.range.clone(), align)
        {
            if point_inside_bounds(footnote_bounds, position) {
                return Some(footnote);
            }
        }
    }

    None
}
#[path = "element_parts/text.rs"]
mod text;
pub use text::{BlockTextElement, CodeLanguageInputElement};
#[cfg(test)]
#[path = "../../../tests/unit/components/block/element.rs"]
mod tests;
