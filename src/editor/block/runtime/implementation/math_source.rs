// @author kongweiguang

//! The compact source-side input used while a formula is active in Live view.
//!
//! The visual formula editor has its own two-dimensional focus target. This
//! element deliberately registers the same Block entity with GPUI's input
//! bridge under a separate focus handle, so IME and clipboard requests keep
//! their UTF-16 contract while the source field remains a normal text input.

use super::*;
use crate::components::block::BlockImeCompositionOwner;
use crate::components::{Copy, Cut, End, Home, Paste, SelectAll};
use crate::theme::ThemeManager;
use crate::ui::actions::ExitCodeBlock;

fn compact_math_source_display_text(source: &str) -> String {
    // 紧凑源码输入始终按单行塑形；逐个替换 CR/LF 而不折叠 CRLF，保证显示文本与
    // 原始 LaTeX 的 UTF-8 字节偏移一致，光标、选区和文档编辑仍可共享同一套范围。
    source
        .chars()
        .map(|character| match character {
            '\r' | '\n' => ' ',
            _ => character,
        })
        .collect()
}

pub(crate) struct MathSourceInputElement {
    input: Entity<Block>,
}

impl MathSourceInputElement {
    pub(crate) fn new(input: Entity<Block>) -> Self {
        Self { input }
    }
}

pub(crate) struct MathSourceInputPrepaintState {
    line: Option<ShapedLine>,
    line_origin: Point<Pixels>,
    cursor: Option<PaintQuad>,
    selection: Option<PaintQuad>,
    hitbox: Hitbox,
}

impl IntoElement for MathSourceInputElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for MathSourceInputElement {
    type RequestLayoutState = ();
    type PrepaintState = MathSourceInputPrepaintState;

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
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = px(28.0).max(window.line_height()).into();
        (window.request_layout(style, [], cx), ())
    }

    /// Shapes virtual MathSource preedit and derives caret geometry in the same UTF-8 space.
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
        let owner = BlockImeCompositionOwner::MathSource;
        let text = input
            .ime_visible_text(&owner)
            .unwrap_or_else(|| input.math_source_text());
        let display_text: SharedString = compact_math_source_display_text(&text).into();
        debug_assert_eq!(display_text.len(), text.len());
        let focused = input.math_source_focus_handle.is_focused(window);
        let style = window.text_style();
        let base_run = TextRun {
            len: display_text.len(),
            font: style.font(),
            color: colors.code_language_input_text,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let marked_range = input.ime_marked_range(&owner);
        let runs = if let Some(range) = marked_range.as_ref().filter(|_| !display_text.is_empty()) {
            vec![
                TextRun {
                    len: range.start,
                    ..base_run.clone()
                },
                TextRun {
                    len: range.end.saturating_sub(range.start),
                    underline: Some(UnderlineStyle {
                        color: Some(colors.code_language_input_text),
                        thickness: px(theme.dimensions.underline_thickness),
                        wavy: false,
                    }),
                    ..base_run.clone()
                },
                TextRun {
                    len: display_text.len().saturating_sub(range.end),
                    ..base_run
                },
            ]
            .into_iter()
            .filter(|run| run.len > 0)
            .collect::<Vec<_>>()
        } else {
            vec![base_run]
        };
        let font_size = style.font_size.to_pixels(window.rem_size());
        let line = window
            .text_system()
            .shape_line(display_text, font_size, &runs, None);
        let ime_selection = input.ime_render_selection(&owner);
        let source_selection = input.math_source_selection();
        let selected = ime_selection
            .as_ref()
            .map(|(range, _)| range.clone())
            .unwrap_or(source_selection.0);
        let reversed = ime_selection
            .as_ref()
            .map(|(_, reversed)| *reversed)
            .unwrap_or(source_selection.1);
        let focus_index = if reversed {
            selected.start
        } else {
            selected.end
        };
        let reveal_width =
            (bounds.size.width - px(theme.dimensions.cursor_width + 4.0)).max(px(1.0));
        let maximum_scroll = (line.width - bounds.size.width).max(px(0.0));
        let scroll_x = (line.x_for_index(focus_index) - reveal_width)
            .max(px(0.0))
            .min(maximum_scroll);
        let line_origin = point(bounds.left() - scroll_x, bounds.top());
        let selection = (focused && selected.start < selected.end).then(|| {
            fill(
                Bounds::from_corners(
                    point(
                        line_origin.x + line.x_for_index(selected.start),
                        bounds.top(),
                    ),
                    point(
                        line_origin.x + line.x_for_index(selected.end),
                        bounds.bottom(),
                    ),
                ),
                colors.selection,
            )
        });
        let cursor = (focused && selected.is_empty()).then(|| {
            let mut cursor_color = colors.cursor;
            cursor_color.a *= input.cursor_opacity();
            let left = line_origin.x + line.x_for_index(focus_index.min(text.len()));
            fill(
                Bounds::from_corners(
                    point(left, bounds.top()),
                    point(left + px(theme.dimensions.cursor_width), bounds.bottom()),
                ),
                cursor_color,
            )
        });

        MathSourceInputPrepaintState {
            line: Some(line),
            line_origin,
            cursor,
            selection,
            hitbox: window.insert_hitbox(bounds, HitboxBehavior::Normal),
        }
    }

    /// Paints the compact source line and drains only this block's native composition trace.
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
        if prepaint.hitbox.is_hovered(window) {
            window.set_cursor_style(CursorStyle::IBeam, &prepaint.hitbox);
        }

        let (focus_handle, input_snapshot) = {
            let input = self.input.read(cx);
            (
                input.math_source_focus_handle.clone(),
                input.input_paint_snapshot(crate::perf::InputPaintSurface::MathSource),
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
        let layout = prepaint.line.take();
        if let Some(line) = layout.as_ref() {
            line.paint(prepaint.line_origin, bounds.size.height, window, cx)
                .ok();
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
            input.math_source_last_layout = layout;
            // Store the shaped line's scrolled origin for pointer hit testing and
            // IME candidate positioning; the element hitbox remains `bounds`.
            input.math_source_last_bounds = Some(Bounds::new(prepaint.line_origin, bounds.size));
        });
    }
}

impl Block {
    /// Lets bound history actions reach the Editor first so nested source focus uses the owning
    /// document transaction without a duplicate Block-to-host dispatch from the raw key event.
    pub(crate) fn on_math_source_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.math_source_focus_handle.is_focused(window) {
            return;
        }
        let modifiers = event.keystroke.modifiers;
        let key = event.keystroke.key.as_str();
        if key == "escape" {
            if self.math_edit_session.is_some() {
                self.math_structure_focus_handle.focus(window);
            }
            cx.stop_propagation();
            cx.notify();
            return;
        }
        if key == "enter" && (modifiers.platform || modifiers.control) && !modifiers.alt {
            self.finish_math_edit(cx);
            self.focus_handle.focus(window);
            self.on_exit_code_block(&ExitCodeBlock, window, cx);
            cx.stop_propagation();
        }
    }

    pub(crate) fn on_math_source_home(
        &mut self,
        _action: &Home,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_math_source_to_edge(false, window, cx);
    }

    pub(crate) fn on_math_source_end(
        &mut self,
        _action: &End,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_math_source_to_edge(true, window, cx);
    }

    fn move_math_source_to_edge(&mut self, end: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !self.math_source_focus_handle.is_focused(window) {
            return;
        }
        let offset = if end {
            self.math_source_text().len()
        } else {
            0
        };
        self.set_math_source_selection(offset..offset, false);
        cx.stop_propagation();
        cx.notify();
    }

    pub(crate) fn on_math_source_select_all(
        &mut self,
        _action: &SelectAll,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.math_source_focus_handle.is_focused(window) {
            return;
        }
        self.set_math_source_selection(0..self.math_source_text().len(), false);
        cx.stop_propagation();
        cx.notify();
    }

    pub(crate) fn on_math_source_copy(
        &mut self,
        _action: &Copy,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.math_source_focus_handle.is_focused(window) {
            return;
        }
        let (range, _) = self.math_source_selection();
        if !range.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.math_source_text()[range].to_owned(),
            ));
        }
        cx.stop_propagation();
    }

    pub(crate) fn on_math_source_cut(
        &mut self,
        _action: &Cut,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.math_source_focus_handle.is_focused(window) {
            return;
        }
        let (range, _) = self.math_source_selection();
        if range.is_empty() {
            cx.stop_propagation();
            return;
        }
        cx.write_to_clipboard(ClipboardItem::new_string(
            self.math_source_text()[range.clone()].to_owned(),
        ));
        self.replace_math_source_text_in_range(
            range,
            "",
            None,
            false,
            UndoCaptureKind::NonCoalescible,
            cx,
        );
        cx.stop_propagation();
    }

    pub(crate) fn on_math_source_paste(
        &mut self,
        _action: &Paste,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.math_source_focus_handle.is_focused(window) {
            return;
        }
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            let (range, _) = self.math_source_selection();
            self.replace_math_source_text_in_range(
                range,
                &text,
                None,
                false,
                UndoCaptureKind::NonCoalescible,
                cx,
            );
        }
        cx.stop_propagation();
    }

    pub(crate) fn math_source_index_for_point(&self, pt: Point<Pixels>) -> usize {
        let Some(bounds) = self.math_source_last_bounds else {
            return self.math_source_text().len();
        };
        let Some(line) = self.math_source_last_layout.as_ref() else {
            return self.math_source_text().len();
        };
        let text = self.math_source_text();
        line.closest_index_for_x(pt.x - bounds.left())
            .min(text.len())
    }
}
