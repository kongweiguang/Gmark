// @author kongweiguang

use super::*;

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
    lines: Vec<WrappedLine>,
    text_bounds: Bounds<Pixels>,
    source_horizontal_offset: Pixels,
    source_layout_cache_key: Option<SourceLayoutCacheKey>,
    source_layout_cache_hit: bool,
    source_line_numbers: Vec<ShapedLine>,
    source_gutter_separator: Option<PaintQuad>,
    active_source_line: Option<PaintQuad>,
    cursor: Option<PaintQuad>,
    selection: Vec<PaintQuad>,
    code_backgrounds: Vec<PaintQuad>,
    line_height: Pixels,
    hitbox: Hitbox,
}

pub struct BlockTextRequestLayoutState {
    lines: Rc<RefCell<Option<Vec<WrappedLine>>>>,
    source_layout_cache_key: Rc<RefCell<Option<SourceLayoutCacheKey>>>,
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

impl Element for BlockTextElement {
    type RequestLayoutState = BlockTextRequestLayoutState;
    type PrepaintState = PrepaintState;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    /// Shapes the owning block's virtual IME text so paint and native range queries share offsets.
    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let theme = cx.global::<ThemeManager>().current_arc();
        let wb = &theme.colors.workbench;
        let input = self.input.read(cx);
        let shared_text = input.display_text_with_ime();
        let is_placeholder = self.is_placeholder && shared_text.is_empty();
        let show_inline_code_backgrounds = !input.is_source_raw_mode();
        let show_source_line_numbers =
            input.show_source_line_numbers() || input.kind().is_code_block();
        let compact_source_host = input.compact_source_host();
        let source_layout_identity = input.source_layout_identity.clone();
        let cached_source_layout_key = input.source_layout_cache_key.clone();
        let cached_source_layout = input.last_layout.clone();
        let source_line_count = source_line_count(shared_text.as_ref());
        let style = window.text_style();

        let (display_text, text_color): (SharedString, Hsla) = if is_placeholder {
            (
                self.placeholder_text
                    .clone()
                    .unwrap_or_else(|| theme.placeholders.empty_editing.clone().into()),
                self.placeholder_color
                    .unwrap_or(theme.colors.text_placeholder),
            )
        } else {
            (shared_text, style.color)
        };

        let run = TextRun {
            len: display_text.len(),
            font: style.font(),
            color: text_color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };

        let runs: Vec<TextRun> = if !is_placeholder {
            if input.kind().is_code_block() || input.code_highlight_result().is_some() {
                build_code_text_runs(
                    input,
                    &display_text,
                    &run,
                    px(theme.dimensions.underline_thickness),
                    &theme.colors,
                )
            } else {
                build_text_runs(
                    input,
                    &display_text,
                    &run,
                    px(theme.dimensions.underline_thickness),
                    theme.colors.text_link,
                    theme.colors.code_bg,
                    wb.danger,
                    show_inline_code_backgrounds,
                )
            }
        } else {
            vec![run]
        };

        let font_size = style.font_size.to_pixels(window.rem_size());
        let line_height = window.line_height();
        let source_font = style.font();
        let theme_identity = Arc::as_ptr(&theme) as usize;
        let scale_bits = f32::from(window.rem_size()).to_bits();
        let marked_range = input
            .ime_marked_range(&super::BlockImeCompositionOwner::BlockText)
            .or_else(|| input.marked_range.clone());
        let source_line_number_gutter_width = show_source_line_numbers
            .then(|| source_line_number_gutter_width(source_line_count, font_size))
            .unwrap_or(px(0.0));

        let shared_lines = Rc::new(RefCell::new(None));
        let shared_lines_clone = shared_lines.clone();
        let shared_source_layout_cache_key = Rc::new(RefCell::new(None));
        let shared_source_layout_cache_key_clone = shared_source_layout_cache_key.clone();
        let shared_source_layout_cache_hit = Rc::new(Cell::new(false));
        let shared_source_layout_cache_hit_clone = shared_source_layout_cache_hit.clone();

        let mut layout_style = Style::default();
        layout_style.size.width = relative(1.).into();
        layout_style.min_size.width = px(0.0).into();
        layout_style.max_size.width = relative(1.).into();

        let layout_id = window.request_measured_layout(
            layout_style,
            move |known_dimensions, available_space, window, _cx| {
                let wrap_width = known_dimensions.width.or(match available_space.width {
                    AvailableSpace::Definite(x) => Some(x),
                    AvailableSpace::MinContent => Some(px(1.0)),
                    AvailableSpace::MaxContent => Some(window.viewport_size().width.max(px(1.0))),
                });
                let text_wrap_width = (!compact_source_host).then(|| {
                    wrap_width
                        .map(|width| (width - source_line_number_gutter_width).max(px(1.0)))
                        .unwrap_or(px(1.0))
                });

                let source_layout_cache_key =
                    source_layout_identity
                        .clone()
                        .map(|identity| SourceLayoutCacheKey {
                            identity,
                            text: display_text.clone(),
                            marked_range: marked_range.clone(),
                            theme_identity,
                            font: source_font.clone(),
                            font_size_bits: f32::from(font_size).to_bits(),
                            line_height_bits: f32::from(line_height).to_bits(),
                            scale_bits,
                            wrap_width_bits: text_wrap_width
                                .map(|width| f32::from(width).to_bits()),
                            soft_wrap: text_wrap_width.is_some(),
                        });

                if source_layout_cache_key.is_some()
                    && source_layout_cache_key == cached_source_layout_key
                    && let Some(lines) = cached_source_layout.clone()
                {
                    let total_size =
                        measured_text_size(&lines, line_height, source_line_number_gutter_width);
                    *shared_lines_clone.borrow_mut() = Some(lines);
                    *shared_source_layout_cache_key_clone.borrow_mut() = source_layout_cache_key;
                    shared_source_layout_cache_hit_clone.set(true);
                    return total_size;
                }

                match window.text_system().shape_text(
                    display_text.clone(),
                    font_size,
                    &runs,
                    text_wrap_width,
                    None,
                ) {
                    Ok(lines) => {
                        let total_size = measured_text_size(
                            &lines,
                            line_height,
                            source_line_number_gutter_width,
                        );
                        *shared_lines_clone.borrow_mut() = Some(lines.into_vec());
                        *shared_source_layout_cache_key_clone.borrow_mut() =
                            source_layout_cache_key;
                        total_size
                    }
                    Err(_) => Size::default(),
                }
            },
        );

        (
            layout_id,
            BlockTextRequestLayoutState {
                lines: shared_lines,
                source_layout_cache_key: shared_source_layout_cache_key,
                source_layout_cache_hit: shared_source_layout_cache_hit,
            },
        )
    }

    /// Computes selection and caret geometry against the visible candidate text rather than source.
    /// 源码平移在排版完成后计算；所有选区、光标与平台候选框使用同一帧的实际文字坐标。
    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let theme = cx.global::<ThemeManager>().current_arc();
        let wb = &theme.colors.workbench;
        let input = self.input.read(cx);
        let display_text = input.display_text_with_ime();
        let is_placeholder = self.is_placeholder && display_text.is_empty();
        let editor_selection_range = input
            .editor_selection_range
            .as_ref()
            .filter(|range| !range.is_empty())
            .cloned();
        let ime_selection = input.ime_render_selection(&super::BlockImeCompositionOwner::BlockText);
        let selected_range = ime_selection
            .as_ref()
            .map(|(range, _)| range.clone())
            .or_else(|| editor_selection_range.clone())
            .unwrap_or_else(|| input.selected_range.clone());
        let cursor = ime_selection
            .as_ref()
            .map(|(range, reversed)| if *reversed { range.start } else { range.end })
            .unwrap_or_else(|| input.cursor_offset());
        let line_height = window.line_height();
        let focused = input.text_input_focus_handle().is_focused(window);
        let show_inline_code_backgrounds = !input.is_source_raw_mode();
        let show_source_line_numbers =
            input.show_source_line_numbers() || input.kind().is_code_block();
        let style = window.text_style();
        let font_size = style.font_size.to_pixels(window.rem_size());

        let lines = request_layout.lines.borrow_mut().take().unwrap_or_default();
        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        let source_line_number_gutter_width = show_source_line_numbers
            .then(|| source_line_number_gutter_width(lines.len().max(1), font_size))
            .unwrap_or(px(0.0));
        let text_bounds = source_text_bounds(bounds, source_line_number_gutter_width);
        let (text_bounds, source_horizontal_offset) =
            source_text_viewport_bounds(input, &lines, text_bounds, cursor, line_height, window);
        let source_line_numbers = if show_source_line_numbers {
            let run_color = if input.kind().is_code_block() {
                theme.colors.code_syntax_comment
            } else {
                wb.text_tertiary
            };
            (1..=lines.len().max(1))
                .map(|line_number| {
                    let label = line_number.to_string();
                    window.text_system().shape_line(
                        SharedString::from(label.clone()),
                        font_size,
                        &[TextRun {
                            len: label.len(),
                            font: style.font(),
                            color: run_color,
                            background_color: None,
                            underline: None,
                            strikethrough: None,
                        }],
                        None,
                    )
                })
                .collect()
        } else {
            Vec::new()
        };
        let source_gutter_separator = show_source_line_numbers.then(|| {
            fill(
                Bounds::new(
                    point(text_bounds.left() - px(1.0), bounds.top()),
                    size(px(1.0), bounds.size.height),
                ),
                wb.border_subtle.opacity(0.7),
            )
        });

        let cursor_opacity = input.cursor_opacity();
        let cursor_color = {
            let mut c = theme.colors.cursor;
            c.a *= cursor_opacity;
            c
        };
        let cursor_width = theme.dimensions.cursor_width;
        let selection_color = wb.selection;
        let text_align = input.text_align();

        let (selection_quads, cursor_quad) =
            if (focused || editor_selection_range.is_some()) && !lines.is_empty() {
                if is_placeholder {
                    // Placeholder: cursor after the placeholder text
                    let layout = &lines[0];
                    let origin_x = aligned_line_left(layout, text_bounds, text_align);
                    let cursor_pos = layout
                        .position_for_index(0, line_height)
                        .unwrap_or_default();
                    (
                        vec![],
                        Some(fill(
                            Bounds::new(
                                point(origin_x + cursor_pos.x, text_bounds.top() + cursor_pos.y),
                                size(px(cursor_width), line_height),
                            ),
                            cursor_color,
                        )),
                    )
                } else if selected_range.is_empty() {
                    // No selection: just draw the cursor
                    (
                        vec![],
                        cursor_bounds_for_offset(
                            &lines,
                            text_bounds,
                            line_height,
                            display_text.as_ref(),
                            cursor,
                            text_align,
                            px(cursor_width),
                        )
                        .map(|bounds| fill(bounds, cursor_color)),
                    )
                } else {
                    let quads = range_segment_bounds(
                        &lines,
                        text_bounds,
                        line_height,
                        display_text.as_ref(),
                        selected_range,
                        text_align,
                    )
                    .into_iter()
                    .map(|bounds| fill(bounds, selection_color))
                    .collect();
                    (quads, None)
                }
            } else {
                (vec![], None)
            };

        let active_source_line =
            (focused && input.is_source_raw_mode() && !is_placeholder && !lines.is_empty())
                .then(|| {
                    cursor_bounds_for_offset(
                        &lines,
                        text_bounds,
                        line_height,
                        display_text.as_ref(),
                        cursor,
                        text_align,
                        px(cursor_width),
                    )
                    .map(|caret| {
                        fill(
                            Bounds::new(
                                point(bounds.left(), caret.top()),
                                size(bounds.size.width, caret.size.height),
                            ),
                            wb.accent_soft,
                        )
                    })
                })
                .flatten();

        // Compute code-span background quads with rounded corners and padding.
        let mut code_quads = Vec::new();
        if show_inline_code_backgrounds && !is_placeholder {
            let text = display_text.as_ref();
            let code_color = theme.colors.code_bg;
            let pad_x = px(theme.dimensions.code_bg_pad_x);
            let pad_y = px(theme.dimensions.code_bg_pad_y);
            let radius = px(theme.dimensions.code_bg_radius);
            for span in input.inline_spans() {
                if !span.style.code || span.range.is_empty() {
                    continue;
                }
                let visible_range = input.ime_visible_range(
                    &super::BlockImeCompositionOwner::BlockText,
                    span.range.clone(),
                );
                if visible_range.is_empty() {
                    continue;
                }
                for segment in range_segment_bounds(
                    &lines,
                    text_bounds,
                    line_height,
                    text,
                    visible_range.clone(),
                    text_align,
                ) {
                    let quad_bounds = Bounds::from_corners(
                        point(segment.left() - pad_x, segment.top() - pad_y),
                        point(segment.right() + pad_x, segment.bottom() + pad_y),
                    );
                    code_quads.push({
                        let mut q = fill(quad_bounds, code_color);
                        q.corner_radii = Corners::all(radius);
                        q
                    });
                }
            }
        }

        PrepaintState {
            lines,
            text_bounds,
            source_horizontal_offset,
            source_layout_cache_key: request_layout.source_layout_cache_key.borrow().clone(),
            source_layout_cache_hit: request_layout.source_layout_cache_hit.get(),
            source_line_numbers,
            source_gutter_separator,
            active_source_line,
            cursor: cursor_quad,
            selection: selection_quads,
            code_backgrounds: code_quads,
            line_height,
            hitbox,
        }
    }

    /// 复用预绘制的文字平移，确保可见文字、命中与 IME 坐标一致；采样只消费当前输入快照。
    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let (focus_handle, hovering_link, input_snapshot) = {
            let input = self.input.read(cx);
            let text_bounds = prepaint.text_bounds;
            let hovering_link = !self.is_placeholder
                && input.ime_composition_owner().is_none()
                && !input.is_source_raw_mode()
                && prepaint.hitbox.is_hovered(window)
                && link_at_position(
                    input,
                    &prepaint.lines,
                    text_bounds,
                    prepaint.line_height,
                    window.mouse_position(),
                )
                .is_some();
            (
                input.text_input_focus_handle().clone(),
                hovering_link,
                input.input_paint_snapshot(crate::perf::InputPaintSurface::BlockText),
            )
        };

        if hovering_link {
            // The hand cursor only appears while the Cmd/Ctrl follow modifier is
            // held (matching the gesture that opens the link); a plain hover keeps
            // the text cursor. The editor root repaints on follow-modifier
            // toggles, so this re-evaluates even when the pointer stays still.
            if window.modifiers().secondary() {
                window.set_cursor_style(CursorStyle::PointingHand, &prepaint.hitbox);
            }
        }

        if focus_handle.is_focused(window) {
            let text_bounds = prepaint.text_bounds;
            window.handle_input(
                &focus_handle,
                ElementInputHandler::new(text_bounds, self.input.clone()),
                cx,
            );
        }

        if let Some(active_source_line) = prepaint.active_source_line.take() {
            window.paint_quad(active_source_line);
        }
        if let Some(source_gutter_separator) = prepaint.source_gutter_separator.take() {
            window.paint_quad(source_gutter_separator);
        }

        // Paint code backgrounds behind text.
        for code_bg in prepaint.code_backgrounds.drain(..) {
            window.paint_quad(code_bg);
        }

        for selection in prepaint.selection.drain(..) {
            window.paint_quad(selection);
        }

        let line_height = prepaint.line_height;
        let lines = std::mem::take(&mut prepaint.lines);
        let text_align = self.input.read(cx).text_align();
        let text_bounds = prepaint.text_bounds;
        let line_number_tops = source_line_number_tops(&lines, line_height);
        let line_number_gap = px(SOURCE_LINE_NUMBER_GAP);
        let line_numbers = std::mem::take(&mut prepaint.source_line_numbers);
        for (line_number, y_offset) in line_numbers.iter().zip(line_number_tops.iter()) {
            let line_number_width = line_number.x_for_index(line_number.len());
            line_number
                .paint(
                    point(
                        text_bounds.left() - line_number_gap - line_number_width,
                        bounds.origin.y + *y_offset,
                    ),
                    line_height,
                    window,
                    cx,
                )
                .ok();
        }

        let mut y_offset = Pixels::default();
        for line in &lines {
            let origin_x = aligned_line_left(line, text_bounds, text_align);
            line.paint(
                point(origin_x, text_bounds.origin.y + y_offset),
                line_height,
                TextAlign::Left,
                None,
                window,
                cx,
            )
            .ok();
            y_offset += wrapped_line_height(line, line_height);
        }

        for trace in crate::perf::take_input_to_gpui_paint(self.input.entity_id(), input_snapshot)
            .into_iter()
            .flatten()
        {
            trace.record_gpui_text_paint();
        }

        if focus_handle.is_focused(window)
            && let Some(cursor) = prepaint.cursor.take()
        {
            window.paint_quad(cursor);
        }

        self.input.update(cx, |input, _cx| {
            input.last_layout = Some(lines);
            input.source_layout_cache_key = prepaint.source_layout_cache_key.clone();
            if prepaint.source_layout_cache_key.is_some() {
                if prepaint.source_layout_cache_hit {
                    input.source_layout_cache_hits =
                        input.source_layout_cache_hits.saturating_add(1);
                } else {
                    input.source_layout_cache_misses =
                        input.source_layout_cache_misses.saturating_add(1);
                }
            }
            input.last_bounds = Some(text_bounds);
            input.source_horizontal_offset = prepaint.source_horizontal_offset;
            input.last_line_height = line_height;
        });
    }
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
