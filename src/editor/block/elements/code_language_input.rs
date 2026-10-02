// @author kongweiguang

use super::*;

/// Single-line text element used to edit a fenced code block's info string.
pub struct CodeLanguageInputElement {
    input: Entity<Block>,
    placeholder: SharedString,
}

impl CodeLanguageInputElement {
    pub fn new(input: Entity<Block>, placeholder: SharedString) -> Self {
        Self { input, placeholder }
    }
}

pub struct CodeLanguageInputPrepaintState {
    line: Option<ShapedLine>,
    cursor: Option<PaintQuad>,
    selection: Option<PaintQuad>,
    hitbox: Option<Hitbox>,
}

pub(in super::super) fn code_language_cursor_bounds(
    bounds: Bounds<Pixels>,
    cursor_x: Pixels,
    font_size: Pixels,
    cursor_width: Pixels,
) -> Bounds<Pixels> {
    // 语言输入是紧凑控件；光标跟随字面高度并垂直居中，不能把编辑器行距
    // 当成光标高度，否则大行距主题会让光标越过控件的视觉边界。
    let cursor_height = font_size.min(bounds.size.height);
    let cursor_top = bounds.top() + (bounds.size.height - cursor_height) / 2.0;
    Bounds::new(
        point(bounds.left() + cursor_x, cursor_top),
        size(cursor_width, cursor_height),
    )
}

impl IntoElement for CodeLanguageInputElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for CodeLanguageInputElement {
    type RequestLayoutState = ();
    type PrepaintState = CodeLanguageInputPrepaintState;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let theme = cx.global::<ThemeManager>().current_arc();
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = px(theme.dimensions.code_language_input_height)
            .max(window.line_height())
            .into();
        (window.request_layout(style, [], cx), ())
    }

    /// Shapes the pinned virtual language text so its candidate and native offsets stay aligned.
    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let theme = cx.global::<ThemeManager>().current_arc();
        let colors = &theme.colors;
        let input = self.input.read(cx);
        let owner = BlockImeCompositionOwner::CodeLanguage;
        let content = input
            .ime_visible_text(&owner)
            .unwrap_or_else(|| input.code_language_text().to_owned());
        let is_placeholder = content.is_empty();
        let display_text: SharedString = if is_placeholder {
            self.placeholder.clone()
        } else {
            content.into()
        };
        let focused = input.code_language_focus_handle.is_focused(window);
        let style = window.text_style();
        let run_color = if is_placeholder {
            colors.code_language_input_placeholder
        } else {
            colors.code_language_input_text
        };
        let base_run = TextRun {
            len: display_text.len(),
            font: style.font(),
            color: run_color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };

        let marked_range = input
            .ime_marked_range(&owner)
            .or_else(|| input.code_language_marked_range.clone());
        let runs = if let Some(marked_range) = marked_range.as_ref().filter(|_| !is_placeholder) {
            vec![
                TextRun {
                    len: marked_range.start,
                    ..base_run.clone()
                },
                TextRun {
                    len: marked_range.end - marked_range.start,
                    underline: Some(UnderlineStyle {
                        color: Some(run_color),
                        thickness: px(theme.dimensions.underline_thickness),
                        wavy: false,
                    }),
                    ..base_run.clone()
                },
                TextRun {
                    len: display_text.len() - marked_range.end,
                    ..base_run
                },
            ]
            .into_iter()
            .filter(|run| run.len > 0)
            .collect()
        } else {
            vec![base_run]
        };

        let font_size = style.font_size.to_pixels(window.rem_size());
        let line = window
            .text_system()
            .shape_line(display_text, font_size, &runs, None);
        let ime_selection = input.ime_render_selection(&owner);
        let selected_range = ime_selection
            .as_ref()
            .map(|(range, _)| range.clone())
            .unwrap_or_else(|| input.code_language_selected_range.clone());
        let selection = if focused && !selected_range.is_empty() {
            let start = line.x_for_index(selected_range.start);
            let end = line.x_for_index(selected_range.end);
            Some(fill(
                Bounds::from_corners(
                    point(bounds.left() + start, bounds.top()),
                    point(bounds.left() + end, bounds.bottom()),
                ),
                colors.selection,
            ))
        } else {
            None
        };
        let cursor = if focused && selected_range.is_empty() {
            let cursor_offset = ime_selection
                .as_ref()
                .map(|(range, reversed)| if *reversed { range.start } else { range.end })
                .unwrap_or_else(|| input.code_language_cursor_offset());
            let cursor_x = line.x_for_index(cursor_offset);
            let mut cursor_color = colors.cursor;
            cursor_color.a *= input.cursor_opacity();
            Some(fill(
                code_language_cursor_bounds(
                    bounds,
                    cursor_x,
                    font_size,
                    px(theme.dimensions.cursor_width),
                ),
                cursor_color,
            ))
        } else {
            None
        };
        let hitbox = Some(window.insert_hitbox(bounds, HitboxBehavior::Normal));

        CodeLanguageInputPrepaintState {
            line: Some(line),
            cursor,
            selection,
            hitbox,
        }
    }

    /// Paints the language field and consumes only traces owned by its current selection state.
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
        if let Some(hitbox) = prepaint.hitbox.as_ref()
            && hitbox.is_hovered(window)
        {
            window.set_cursor_style(CursorStyle::IBeam, hitbox);
        }

        let (focus_handle, input_snapshot) = {
            let input = self.input.read(cx);
            (
                input.code_language_focus_handle.clone(),
                input.input_paint_snapshot(crate::perf::InputPaintSurface::CodeLanguage),
            )
        };
        if focus_handle.is_focused(window) {
            window.handle_input(
                &focus_handle,
                ElementInputHandler::new(bounds, self.input.clone()),
                cx,
            );
        }

        if let Some(selection) = prepaint.selection.take() {
            window.paint_quad(selection);
        }

        let line = prepaint.line.take().expect("line should be shaped");
        line.paint(bounds.origin, bounds.size.height, window, cx)
            .ok();

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
            input.code_language_last_layout = Some(line);
            input.code_language_last_bounds = Some(bounds);
        });
    }
}
