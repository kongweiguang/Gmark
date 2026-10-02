// @author kongweiguang

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use super::super::runtime::math_edit::{MathPaletteDrag, MathPaletteDragPreview};
use super::math_visual::render_math_editing_svg_content;
use super::*;
use crate::components::MathPalettePage;
use crate::ui::visual_preferences::VisualPreferencesManager;

fn preview_key(source: &str, parameters: impl Hash) -> u64 {
    let mut hasher = DefaultHasher::new();
    source.hash(&mut hasher);
    parameters.hash(&mut hasher);
    hasher.finish()
}

fn math_palette_panel_height(page: MathPalettePage, structured: bool) -> f32 {
    if !structured {
        return 66.0;
    }
    match page {
        MathPalettePage::Symbols => 382.0,
        MathPalettePage::Structures => 178.0,
    }
}

const MATH_PALETTE_WIDTH: f32 = 200.0;
const MATH_PALETTE_GAP: f32 = 6.0;
const MATH_PALETTE_VIEWPORT_INSET: f32 = 8.0;
const MATH_FORMULA_ANCHOR_HEIGHT: f32 = 56.0;
const MATH_PALETTE_CONTROL_RADIUS: f32 = 8.0;

/// Keeps the formula palette in the same floating-surface radius range as other transient
/// editor actions while allowing a theme's larger dialog radius to remain bounded.
pub(super) fn math_palette_radius(dialog_radius: f32) -> f32 {
    dialog_radius.clamp(14.0, 18.0)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct MathPalettePlacement {
    pub(super) left: f32,
    pub(super) top: f32,
    pub(super) above: bool,
}

#[cfg(test)]
pub(super) fn math_palette_placement(
    bounds: Bounds<Pixels>,
    pointer_anchor_y: Option<Pixels>,
    viewport: Size<Pixels>,
    page: MathPalettePage,
    drag_offset: Point<Pixels>,
) -> MathPalettePlacement {
    math_palette_placement_for_mode(bounds, pointer_anchor_y, viewport, page, true, drag_offset)
}

fn math_palette_placement_for_mode(
    bounds: Bounds<Pixels>,
    pointer_anchor_y: Option<Pixels>,
    viewport: Size<Pixels>,
    page: MathPalettePage,
    structured: bool,
    drag_offset: Point<Pixels>,
) -> MathPalettePlacement {
    let viewport_width = f32::from(viewport.width);
    let viewport_height = f32::from(viewport.height);
    let panel_height = math_palette_panel_height(page, structured);
    let stable_height = math_palette_panel_height(MathPalettePage::Symbols, structured);
    let anchor_top = pointer_anchor_y
        .map(f32::from)
        .map(|anchor| anchor - MATH_FORMULA_ANCHOR_HEIGHT / 2.0)
        .unwrap_or_else(|| f32::from(bounds.top()));
    let anchor_bottom = pointer_anchor_y
        .map(f32::from)
        .map(|anchor| anchor + MATH_FORMULA_ANCHOR_HEIGHT / 2.0)
        .unwrap_or_else(|| f32::from(bounds.bottom()));
    let available_below = viewport_height - anchor_bottom - MATH_PALETTE_GAP;
    let available_above = anchor_top - MATH_PALETTE_GAP;
    // Use the taller symbols page for the side decision so switching to the
    // compact structures page never makes the palette jump across the formula.
    let above = available_below < stable_height && available_above > available_below;
    let unclamped_left = f32::from(bounds.left())
        + (f32::from(bounds.size.width) - MATH_PALETTE_WIDTH) / 2.0
        + f32::from(drag_offset.x);
    let max_left = (viewport_width - MATH_PALETTE_WIDTH - MATH_PALETTE_VIEWPORT_INSET)
        .max(MATH_PALETTE_VIEWPORT_INSET);
    let left = unclamped_left.clamp(MATH_PALETTE_VIEWPORT_INSET, max_left);
    let unclamped_top = if above {
        anchor_top - MATH_PALETTE_GAP - panel_height
    } else {
        anchor_bottom + MATH_PALETTE_GAP
    } + f32::from(drag_offset.y);
    let max_top = (viewport_height - panel_height - MATH_PALETTE_VIEWPORT_INSET)
        .max(MATH_PALETTE_VIEWPORT_INSET);
    let top = unclamped_top.clamp(MATH_PALETTE_VIEWPORT_INSET, max_top);
    MathPalettePlacement { left, top, above }
}

pub(super) fn math_caret_scroll_offset(
    current: f32,
    maximum: f32,
    viewport_width: f32,
    caret_left: f32,
    caret_right: f32,
) -> f32 {
    const MARGIN: f32 = 12.0;
    if viewport_width <= MARGIN * 2.0 || maximum <= 0.0 {
        return 0.0;
    }
    let mut next = current.clamp(-maximum, 0.0);
    if caret_left + next < MARGIN {
        next = MARGIN - caret_left;
    } else if caret_right + next > viewport_width - MARGIN {
        next = viewport_width - MARGIN - caret_right;
    }
    next.clamp(-maximum, 0.0)
}

impl Block {
    pub(super) fn render_math_visual_editor(
        &mut self,
        theme: &Theme,
        viewport: Size<Pixels>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let preview = self.render_math_content(theme, cx);
        let math_focus = self.math_structure_focus_handle.clone();
        if let Some(session) = self.math_edit_session.as_ref() {
            let projection =
                gmark_math_edit::MathVisualProjection::from_document(session.document());
            if let Some(caret) = projection.caret_rect(session.editor().cursor()) {
                let font_size = display_math_font_size(theme.typography.text_size);
                let padding = (font_size * 0.35).max(4.0);
                let viewport_width = f32::from(self.math_visual_scroll_handle.bounds().size.width);
                let maximum = f32::from(self.math_visual_scroll_handle.max_offset().width);
                let current = f32::from(self.math_visual_scroll_handle.offset().x);
                let next = math_caret_scroll_offset(
                    current,
                    maximum,
                    viewport_width,
                    caret.x as f32 * font_size + padding,
                    caret.right() as f32 * font_size + padding,
                );
                let mut offset = self.math_visual_scroll_handle.offset();
                offset.x = px(next);
                self.math_visual_scroll_handle.set_offset(offset);
            }
        }
        let scroll_handle = self.math_visual_scroll_handle.clone();
        let surface = div()
            .id("math-visual-editor-surface")
            .debug_selector(|| "math-visual-editor-surface".to_owned())
            .w_full()
            .min_w(px(0.0))
            .overflow_x_scroll()
            .track_scroll(&scroll_handle)
            .tab_index(0)
            .key_context(BLOCK_EDITOR_CONTEXT)
            .track_focus(&math_focus)
            .on_click(cx.listener({
                let math_focus = math_focus.clone();
                move |_block, _event, window, cx| {
                    math_focus.focus(window);
                    cx.stop_propagation();
                }
            }))
            .on_action(cx.listener(Self::on_math_structure_delete_back))
            .on_action(cx.listener(Self::on_math_structure_delete))
            .on_action(cx.listener(Self::on_exit_code_block))
            .on_key_down(cx.listener(Self::on_math_structure_key_down))
            .child(preview)
            .into_any_element();
        let root = div()
            .id("math-visual-editor")
            .debug_selector(|| "math-visual-editor".to_owned())
            .w_full()
            .flex()
            .flex_col()
            .items_center()
            .key_context(BLOCK_EDITOR_CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::on_exit_code_block))
            .relative();
        root.child(surface)
            .child(self.render_math_palette_overlay(theme, viewport, cx))
            .into_any_element()
    }

    pub(super) fn render_math_palette_overlay(
        &self,
        theme: &Theme,
        viewport: Size<Pixels>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let bounds = self
            .last_bounds
            .unwrap_or_else(|| Bounds::new(point(px(0.0), px(0.0)), viewport));
        let structured = self.math_edit_session.is_some();
        let placement = math_palette_placement_for_mode(
            bounds,
            self.math_palette_anchor_y,
            viewport,
            self.math_palette_page,
            structured,
            point(px(0.0), px(0.0)),
        );
        let dragged_placement = math_palette_placement_for_mode(
            bounds,
            self.math_palette_anchor_y,
            viewport,
            self.math_palette_page,
            structured,
            self.math_palette_offset,
        );
        let base_left =
            f32::from(bounds.left()) + (f32::from(bounds.size.width) - MATH_PALETTE_WIDTH) / 2.0;
        let effective_offset = point(
            px(dragged_placement.left - base_left),
            self.math_palette_offset.y,
        );
        let toolbar = self.render_math_structure_toolbar(theme, effective_offset, cx);
        let panel_height = math_palette_panel_height(self.math_palette_page, structured);
        let panel = div()
            .id("math-palette-overlay")
            .absolute()
            .left_0()
            .w(px(MATH_PALETTE_WIDTH))
            .h(px(panel_height))
            .occlude()
            .when(placement.above, |panel| panel.bottom(px(MATH_PALETTE_GAP)))
            .when(!placement.above, |panel| panel.top(px(MATH_PALETTE_GAP)))
            .child(toolbar);
        let hit_container = div()
            .absolute()
            .left(relative(0.5))
            .ml(px(-MATH_PALETTE_WIDTH / 2.0))
            .w(px(MATH_PALETTE_WIDTH))
            .h(px(panel_height + MATH_PALETTE_GAP))
            .when(placement.above, |container| container.bottom(relative(1.0)))
            .when(!placement.above, |container| container.top(relative(1.0)))
            .child(panel);
        deferred(hit_container).with_priority(30).into_any_element()
    }

    /// Renders structured IME candidates from a disposable session so preview never mutates source.
    pub(super) fn render_math_content(
        &mut self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let c = &theme.colors;
        let wb = &c.workbench;
        let d = &theme.dimensions;
        let t = &theme.typography;
        let ime_preview = self.math_ime_preview_session();
        let visible_session = ime_preview.as_ref().or(self.math_edit_session.as_ref());
        let ime_marked_selection = ime_preview
            .as_ref()
            .and_then(|preview| self.math_ime_marked_selection(preview));
        let raw = visible_session
            .map(crate::editor::math_edit::MathEditSession::visual_preview_raw)
            .unwrap_or_else(|| {
                self.record
                    .raw_fallback
                    .as_deref()
                    .unwrap_or_else(|| self.display_text())
                    .to_owned()
            });
        let text_color = c.text_default;
        let font_size = display_math_font_size(t.text_size);
        let editing_geometry = visible_session.map(|session| {
            (
                gmark_math_edit::MathVisualProjection::from_document(session.document()),
                session.editor().cursor().clone(),
                session.editor().selection().clone(),
            )
        });
        let key = preview_key(&raw, (format!("{text_color:?}"), font_size.to_bits()));
        if self.math_preview_key != Some(key) {
            self.math_preview_key = Some(key);
            self.math_render_error = None;
            let source = raw.clone();
            self.math_preview_task = Some(cx.spawn(
                async move |this: WeakEntity<Block>, cx: &mut AsyncApp| {
                    cx.background_executor()
                        .timer(Duration::from_millis(100))
                        .await;
                    let result = cx
                        .background_spawn(async move {
                            parse_display_math_source(&source)
                                .ok_or_else(|| "invalid display math source".to_owned())
                                .and_then(|parsed| {
                                    render_display_math_svg(&parsed, text_color, font_size)
                                        .map_err(|error| error.to_string())
                                })
                        })
                        .await;
                    let _ = this.update(cx, |block, cx| {
                        if block.math_preview_key != Some(key) {
                            return;
                        }
                        block.math_preview_task = None;
                        match result {
                            Ok(rendered) => {
                                block.last_successful_math_render = Some(rendered);
                                block.math_render_error = None;
                            }
                            Err(error) => block.math_render_error = Some(error),
                        }
                        cx.notify();
                    });
                },
            ));
        }

        match (
            self.last_successful_math_render.as_ref(),
            self.math_render_error.as_ref(),
        ) {
            (Some(rendered), None) => editing_geometry
                .as_ref()
                .map(|(projection, cursor, selection)| {
                    render_math_editing_svg_content(
                        rendered,
                        theme,
                        projection,
                        cursor,
                        selection,
                        ime_marked_selection.as_ref(),
                    )
                })
                .unwrap_or_else(|| render_math_svg_content(rendered, theme)),
            (Some(rendered), Some(error)) => div()
                .id("math-render-fallback")
                .debug_selector(|| "math-render-fallback".to_owned())
                .w_full()
                .min_w(px(0.0))
                .max_w(Length::Definite(relative(1.0)))
                .overflow_hidden()
                .flex()
                .flex_col()
                .gap(px(4.0))
                .child(render_math_svg_content(rendered, theme))
                .child(render_complex_warning(
                    format!("LaTeX render error: {error}"),
                    theme,
                    "math-render-warning",
                ))
                .into_any_element(),
            (None, Some(error)) => div()
                .w_full()
                .flex()
                .flex_col()
                .gap(px(4.0))
                .rounded_sm()
                .bg(wb.solid_surface)
                .px(px(d.block_padding_x))
                .py(px(d.block_padding_y))
                .text_size(px(t.text_size))
                .line_height(rems(t.text_line_height))
                .text_color(c.text_default)
                .child(SharedString::from(raw))
                .child(render_complex_warning(
                    format!("LaTeX render error: {error}"),
                    theme,
                    "math-render-warning",
                ))
                .into_any_element(),
            (None, None) => div()
                .id("math-render-pending")
                .debug_selector(|| "math-render-pending".to_owned())
                .w_full()
                .min_h(px(64.0))
                .rounded_sm()
                .bg(wb.editor_surface)
                .into_any_element(),
        }
    }

    /// Floating 200 px formula palette. Graphic labels stay compact while
    /// localized names are exposed through tooltips and the accessibility tree.
    /// Toolbar commands retain the source input's IME owner until its terminal event before focusing the diagram.
    pub(super) fn render_math_structure_toolbar(
        &self,
        theme: &Theme,
        palette_offset: Point<Pixels>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        use gmark_math_edit::{MathDelimiterPair, MathEditCommand};

        let palette = &theme.colors.workbench;
        let visual_preferences = cx
            .try_global::<VisualPreferencesManager>()
            .map(VisualPreferencesManager::current)
            .unwrap_or_default();
        let material = palette.material(
            crate::theme::workbench::SurfaceKind::GlassStrong,
            visual_preferences,
        );
        let panel_radius = math_palette_radius(theme.dimensions.dialog_radius);
        let strings = cx.global::<I18nManager>().strings_arc();
        let block = cx.entity().downgrade();
        let drag_target = block.clone();
        let page = self.math_palette_page;
        let structured = self.math_edit_session.is_some();
        let items: Vec<(&'static str, &'static str, MathEditCommand)> = match page {
            MathPalettePage::Symbols => vec![
                ("fraction", "x/y", MathEditCommand::InsertFraction),
                ("sqrt", "√", MathEditCommand::InsertSquareRoot),
                ("nth_root", "ⁿ√", MathEditCommand::InsertNthRoot),
                (
                    "matrix",
                    "▦",
                    MathEditCommand::InsertMatrix {
                        rows: 2,
                        columns: 2,
                    },
                ),
                (
                    "paren",
                    "()",
                    MathEditCommand::InsertDelimiter(MathDelimiterPair::Parentheses),
                ),
                (
                    "bracket",
                    "[]",
                    MathEditCommand::InsertDelimiter(MathDelimiterPair::Brackets),
                ),
                (
                    "brace",
                    "{}",
                    MathEditCommand::InsertDelimiter(MathDelimiterPair::Braces),
                ),
                (
                    "abs",
                    "||",
                    MathEditCommand::InsertDelimiter(MathDelimiterPair::AbsoluteValue),
                ),
                (
                    "norm",
                    "‖‖",
                    MathEditCommand::InsertDelimiter(MathDelimiterPair::Norm),
                ),
                (
                    "angle",
                    "⟨⟩",
                    MathEditCommand::InsertDelimiter(MathDelimiterPair::Angle),
                ),
                (
                    "floor",
                    "⌊⌋",
                    MathEditCommand::InsertDelimiter(MathDelimiterPair::Floor),
                ),
                (
                    "ceil",
                    "⌈⌉",
                    MathEditCommand::InsertDelimiter(MathDelimiterPair::Ceil),
                ),
                (
                    "integral",
                    "∫",
                    MathEditCommand::InsertOperatorWithLimits("int".into()),
                ),
                (
                    "sum",
                    "∑",
                    MathEditCommand::InsertOperatorWithLimits("sum".into()),
                ),
                (
                    "product",
                    "∏",
                    MathEditCommand::InsertOperatorWithLimits("prod".into()),
                ),
                (
                    "infinity",
                    "∞",
                    MathEditCommand::InsertText(r"\infty".into()),
                ),
                ("pi", "π", MathEditCommand::InsertText(r"\pi".into())),
                ("theta", "θ", MathEditCommand::InsertText(r"\theta".into())),
                ("alpha", "α", MathEditCommand::InsertText(r"\alpha".into())),
                ("beta", "β", MathEditCommand::InsertText(r"\beta".into())),
                ("gamma", "γ", MathEditCommand::InsertText(r"\gamma".into())),
                ("delta", "δ", MathEditCommand::InsertText(r"\delta".into())),
                (
                    "lambda",
                    "λ",
                    MathEditCommand::InsertText(r"\lambda".into()),
                ),
                ("mu", "μ", MathEditCommand::InsertText(r"\mu".into())),
                ("sigma", "σ", MathEditCommand::InsertText(r"\sigma".into())),
                ("phi", "φ", MathEditCommand::InsertText(r"\phi".into())),
                ("omega", "ω", MathEditCommand::InsertText(r"\omega".into())),
                (
                    "uppercase_delta",
                    "Δ",
                    MathEditCommand::InsertText(r"\Delta".into()),
                ),
                (
                    "less_or_equal",
                    "≤",
                    MathEditCommand::InsertText(r"\le".into()),
                ),
                (
                    "greater_or_equal",
                    "≥",
                    MathEditCommand::InsertText(r"\ge".into()),
                ),
                ("not_equal", "≠", MathEditCommand::InsertText(r"\ne".into())),
                (
                    "approximately",
                    "≈",
                    MathEditCommand::InsertText(r"\approx".into()),
                ),
                ("times", "×", MathEditCommand::InsertText(r"\times".into())),
                ("divide", "÷", MathEditCommand::InsertText(r"\div".into())),
                ("dot", "·", MathEditCommand::InsertText(r"\cdot".into())),
                (
                    "plus_minus",
                    "±",
                    MathEditCommand::InsertText(r"\pm".into()),
                ),
                (
                    "right_arrow",
                    "→",
                    MathEditCommand::InsertText(r"\to".into()),
                ),
                (
                    "partial",
                    "∂",
                    MathEditCommand::InsertText(r"\partial".into()),
                ),
                ("nabla", "∇", MathEditCommand::InsertText(r"\nabla".into())),
                ("member", "∈", MathEditCommand::InsertText(r"\in".into())),
            ],
            MathPalettePage::Structures => vec![
                ("fraction", "x/y", MathEditCommand::InsertFraction),
                ("sqrt", "√", MathEditCommand::InsertSquareRoot),
                ("superscript", "x²", MathEditCommand::InsertSuperscript),
                ("subscript", "x₂", MathEditCommand::InsertSubscript),
                (
                    "matrix",
                    "▦",
                    MathEditCommand::InsertMatrix {
                        rows: 2,
                        columns: 2,
                    },
                ),
                ("cases", "{⋮", MathEditCommand::InsertCases { rows: 2 }),
                (
                    "aligned",
                    "=⋮",
                    MathEditCommand::InsertAligned {
                        rows: 2,
                        columns: 2,
                    },
                ),
                ("text_mode", "T", MathEditCommand::InsertTextMode),
                ("alpha", "α", MathEditCommand::InsertText(r"\alpha".into())),
                (
                    "sum",
                    "∑",
                    MathEditCommand::InsertOperatorWithLimits("sum".into()),
                ),
            ],
        };
        if page == MathPalettePage::Symbols {
            debug_assert!(
                items
                    .iter()
                    .map(|(key, _, _)| *key)
                    .eq(gmark_math_edit::MATH_SYMBOL_PALETTE_KEYS)
            );
        }

        let tab = |id: &'static str, label_key: &'static str, target_page: MathPalettePage| {
            let target = block.clone();
            let pointer_target = block.clone();
            let active = page == target_page;
            let tooltip = SharedString::from(strings.math_palette_text(label_key));
            div()
                .id(id)
                .debug_selector(move || id.to_owned())
                .h(px(24.0))
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(MATH_PALETTE_CONTROL_RADIUS))
                .text_size(px(11.0))
                .text_color(if active {
                    palette.text_primary
                } else {
                    palette.text_secondary
                })
                .border_1()
                .border_color(if active {
                    palette.accent
                } else {
                    palette.border_subtle
                })
                .when(active, |this| this.bg(palette.control_pressed))
                .hover(|this| this.bg(palette.control_hover))
                .active(|this| this.bg(palette.control_pressed))
                .focus(|this| this.border_color(palette.focus_ring))
                .tab_index(0)
                .cursor_pointer()
                .tooltip(move |_window, cx| crate::ui::ui_tooltip(tooltip.clone(), cx))
                .on_mouse_down(MouseButton::Left, move |_event, _window, cx| {
                    let _ = pointer_target.update(cx, |block, cx| {
                        block.math_palette_page = target_page;
                        cx.notify();
                    });
                    cx.stop_propagation();
                })
                .on_click(move |_event, _window, cx| {
                    let _ = target.update(cx, |block, cx| {
                        block.math_palette_page = target_page;
                        cx.notify();
                    });
                    cx.stop_propagation();
                })
                .child(SharedString::from(strings.math_palette_text(label_key)))
        };

        let rows = items.chunks(5).map(|row| {
            let buttons = row.iter().map(|(key, glyph, command)| {
                let target = block.clone();
                let command = command.clone();
                let tooltip = strings.math_palette_text(key);
                // The command key is the stable identity. Row/column are a
                // presentation detail and must not change selectors when
                // the panel density or page layout is tuned.
                let element_id = SharedString::from(format!("math-palette-item-{key}"));
                div()
                    .id(element_id)
                    .debug_selector(move || format!("math-palette-item-{key}"))
                    .size(px(32.0))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(MATH_PALETTE_CONTROL_RADIUS))
                    .border_1()
                    .border_color(palette.border_subtle)
                    .bg(palette.control_surface)
                    .hover(|this| {
                        this.bg(palette.control_hover)
                            .border_color(palette.focus_ring)
                    })
                    .active(|this| this.bg(palette.control_pressed))
                    .focus(|this| this.border_color(palette.focus_ring))
                    .tab_index(0)
                    .text_color(palette.text_primary)
                    .text_size(px(16.0))
                    .cursor_pointer()
                    .tooltip(move |_window, cx| crate::ui::ui_tooltip(tooltip.clone(), cx))
                    .on_mouse_down(MouseButton::Left, move |_event, _window, cx| {
                        cx.stop_propagation();
                    })
                    .on_click(move |_event, window, cx| {
                        let command = command.clone();
                        let _ = target.update(cx, |block, cx| {
                            block.request_ime_interaction(
                                crate::components::block::BlockImeInteraction::MathPaletteCommand(
                                    command,
                                ),
                                window,
                                cx,
                            );
                        });
                        cx.stop_propagation();
                    })
                    .child(*glyph)
            });
            div().w_full().flex().gap(px(4.0)).children(buttons)
        });

        let panel = div()
            .id("math-palette-panel")
            .w_full()
            .occlude()
            .p(px(8.0))
            .flex()
            .flex_col()
            .gap(px(4.0))
            .rounded(px(panel_radius))
            .overflow_hidden()
            .border_1()
            .border_color(material.border)
            .bg(material.background)
            .child(
                div()
                    .id("math-palette-drag-handle")
                    .debug_selector(|| "math-palette-drag-handle".to_owned())
                    .h(px(16.0))
                    .w_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(palette.text_tertiary)
                    .text_size(px(12.0))
                    .bg(palette.control_surface)
                    .hover(|this| this.bg(palette.control_hover))
                    .active(|this| this.bg(palette.control_pressed))
                    .cursor_pointer()
                    .tooltip({
                        let tooltip = SharedString::from(strings.math_palette_text("drag_handle"));
                        move |_window, cx| crate::ui::ui_tooltip(tooltip.clone(), cx)
                    })
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(Self::begin_math_palette_drag),
                    )
                    .on_drag(MathPaletteDrag, |_payload, _offset, _window, cx| {
                        cx.new(|_| MathPaletteDragPreview)
                    })
                    .on_drag_move::<MathPaletteDrag>(move |event, window, cx| {
                        let _ = drag_target.update(cx, |block, cx| {
                            block.update_math_palette_drag(&event.event, window, cx);
                        });
                    })
                    .child("⠿"),
            )
            .child(self.render_math_source_editor(theme, cx));
        let panel = if structured {
            panel
                .child(div().w_full().flex().gap(px(4.0)).children([
                    tab(
                        "math-palette-symbols",
                        "symbols_tab",
                        MathPalettePage::Symbols,
                    ),
                    tab(
                        "math-palette-structures",
                        "structures_tab",
                        MathPalettePage::Structures,
                    ),
                ]))
                .children(rows)
        } else {
            panel
        };

        div()
            .id("math-structure-toolbar")
            .debug_selector(|| "math-structure-toolbar".to_owned())
            .w(px(200.0))
            .relative()
            .left(palette_offset.x)
            .top(palette_offset.y)
            .flex()
            .flex_col()
            .items_center()
            .gap(px(4.0))
            .shadow(vec![BoxShadow {
                color: material.shadow,
                offset: point(px(0.0), px(8.0)),
                blur_radius: px(20.0),
                spread_radius: px(-6.0),
            }])
            .on_mouse_move(cx.listener(Self::update_math_palette_drag))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::end_math_palette_drag))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::end_math_palette_drag))
            .child(panel)
            .into_any_element()
    }
}
