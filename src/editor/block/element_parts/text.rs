// @author kongweiguang

use super::*;

/// 大型常驻源码在绘制阶段叠加语法颜色，后台结果无需改变已有布局。
fn shared_source_paint_syntax(bytes: usize) -> bool {
    bytes >= 1024 * 1024
}

/// Custom low-level [`Element`] that renders a block's inline-formatted
/// text with selection highlights and a blinking cursor.
///
/// Supports multi-line text (used by code blocks) via hard `\n` breaks.
/// Each `\n` produces a separate `WrappedLine` from the text shaper.
pub struct BlockTextElement {
    input: Entity<Block>,
    is_placeholder: bool,
    placeholder_text: Option<SharedString>,
    placeholder_color: Option<Hsla>,
}

#[path = "../elements/code_language_input.rs"]
mod code_language_input;
pub use code_language_input::CodeLanguageInputElement;
#[cfg(test)]
pub(super) use code_language_input::code_language_cursor_bounds;

impl BlockTextElement {
    pub fn new(input: Entity<Block>, is_placeholder: bool) -> Self {
        Self {
            input,
            is_placeholder,
            placeholder_text: None,
            placeholder_color: None,
        }
    }

    pub fn with_placeholder(
        input: Entity<Block>,
        is_placeholder: bool,
        placeholder_text: SharedString,
        placeholder_color: Option<Hsla>,
    ) -> Self {
        Self {
            input,
            is_placeholder,
            placeholder_text: Some(placeholder_text),
            placeholder_color,
        }
    }
}

/// Prepared text layout and paint geometry for one `BlockTextElement` frame.
pub struct PrepaintState {
    lines: Arc<Vec<WrappedLine>>,
    text_bounds: Bounds<Pixels>,
    source_horizontal_offset: Pixels,
    source_layout_cache_key: Option<SourceLayoutCacheKey>,
    source_layout_cache_size: Option<Size<Pixels>>,
    source_layout_measurements: Vec<SourceLayoutMeasurement>,
    source_layout_cache_hit: bool,
    source_paint_only_syntax: bool,
    source_text_color: Hsla,
    source_line_numbers: Vec<(Pixels, ShapedLine)>,
    visible_lines: SmallVec<[VisibleHardLine; 4]>,
    source_gutter_separator: Option<PaintQuad>,
    active_source_line: Option<PaintQuad>,
    cursor: Option<PaintQuad>,
    selection: Vec<PaintQuad>,
    code_backgrounds: Vec<PaintQuad>,
    line_height: Pixels,
    hitbox: Hitbox,
}

pub struct BlockTextRequestLayoutState {
    lines: Rc<RefCell<Option<Arc<Vec<WrappedLine>>>>>,
    source_layout_cache_key: Rc<RefCell<Option<SourceLayoutCacheKey>>>,
    source_layout_cache_size: Rc<RefCell<Option<Size<Pixels>>>>,
    source_layout_measurements: Rc<RefCell<Vec<SourceLayoutMeasurement>>>,
    source_layout_cache_hit: Rc<Cell<bool>>,
}

/// 有界源码行仍可超过可见宽度；用实际裁剪边界跟随插入点，绘制、命中与候选坐标共用平移。
/// 拖选期间保留原位置，避免按下点在同一手势里漂移；非 Source 表面完全沿用原排版。
fn source_text_viewport_bounds(
    input: &Block,
    lines: &[WrappedLine],
    bounds: Bounds<Pixels>,
    cursor: usize,
    line_height: Pixels,
    window: &Window,
) -> (Bounds<Pixels>, Pixels) {
    if !input.compact_source_host() {
        return (bounds, px(0.0));
    }
    let right = bounds
        .right()
        .min(window.content_mask().bounds.right())
        .min(window.viewport_size().width);
    let width = (right - bounds.left()).max(px(1.0));
    let margin = px(8.0).min(width / 4.0);
    let text_width = lines
        .first()
        .map_or(px(0.0), |line| line.size(line_height).width);
    let maximum = (text_width - width + margin).max(px(0.0));
    let mut offset = input.source_horizontal_offset.max(px(0.0)).min(maximum);
    if input.text_input_focus_handle().is_focused(window) && !input.is_selecting {
        if let Some(position) = lines
            .first()
            .and_then(|line| line.position_for_index(cursor, line_height))
        {
            if position.x - offset < margin {
                offset = (position.x - margin).max(px(0.0));
            } else if position.x - offset > width - margin {
                offset = (position.x - width + margin).min(maximum);
            }
        }
    }
    let mut translated = bounds;
    translated.origin.x -= offset;
    (translated, offset)
}

impl IntoElement for BlockTextElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

#[path = "text_element.rs"]
mod text_element;

/// 纯源码的硬行独立成形。编辑后保留未变的首尾行，避免单字输入重新塑形百万字。
pub(super) fn reshape_plain_source(
    text_system: &WindowTextSystem,
    previous: &[WrappedLine],
    text: &str,
    line_count: usize,
    run: &TextRun,
    font_size: Pixels,
    wrap_width: Option<Pixels>,
) -> Result<Vec<WrappedLine>> {
    let prefix = previous
        .iter()
        .zip(text.split('\n'))
        .take_while(|(line, next)| line.text.as_ref() == *next)
        .count();
    let suffix = previous[prefix..]
        .iter()
        .rev()
        .zip(text.rsplit('\n'))
        .take(line_count.saturating_sub(prefix))
        .take_while(|(line, next)| line.text.as_ref() == *next)
        .count();
    let mut result = Vec::with_capacity(line_count);
    result.extend_from_slice(&previous[..prefix]);
    if prefix + suffix < line_count {
        let start = previous[..prefix]
            .iter()
            .map(|line| line.text.len() + 1)
            .sum::<usize>();
        let tail = previous[previous.len() - suffix..]
            .iter()
            .map(|line| line.text.len() + 1)
            .sum::<usize>();
        let changed: SharedString = text[start..text.len() - tail].to_owned().into();
        let mut run = run.clone();
        run.len = changed.len();
        result.extend(text_system.shape_text(changed, font_size, &[run], wrap_width, None)?);
    }
    result.extend_from_slice(&previous[previous.len() - suffix..]);
    Ok(result)
}

fn measured_text_size(
    lines: &[WrappedLine],
    line_height: Pixels,
    source_line_number_gutter_width: Pixels,
) -> Size<Pixels> {
    let mut total_size: Size<Pixels> = Size::default();
    for line in lines {
        let size = line.size(line_height);
        total_size.height += size.height;
        total_size.width = total_size.width.max(size.width);
    }
    total_size.width += source_line_number_gutter_width;
    total_size
}
