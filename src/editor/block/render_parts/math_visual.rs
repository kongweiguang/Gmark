// @author kongweiguang

//! Paints the RaTeX image, semantic selection, and caret for the active formula.

use super::*;

/// Paints semantic selection, IME preedit underline, and caret over the generated formula image.
pub(super) fn render_math_editing_svg_content(
    rendered: &crate::components::LatexSvgRender,
    theme: &Theme,
    projection: &gmark_math_edit::MathVisualProjection,
    cursor: &gmark_math_edit::MathCursor2D,
    selection: &gmark_math_edit::MathSelection,
    ime_marked_selection: Option<&gmark_math_edit::MathSelection>,
) -> AnyElement {
    let font_size = display_math_font_size(theme.typography.text_size);
    let padding = (font_size * 0.35).max(4.0);
    let width = (projection.width() as f32 * font_size + padding * 2.0).max(24.0);
    let height =
        ((projection.height() + projection.depth()) as f32 * font_size + padding * 2.0).max(28.0);
    let selection_rect = projection.selection_rect(selection);
    let marked_rect = ime_marked_selection.and_then(|marked| projection.selection_rect(marked));
    let caret_rect = projection.caret_rect(cursor);
    let wb = &theme.colors.workbench;

    div()
        .id("math-rendered-content")
        .debug_selector(|| "math-rendered-content".to_owned())
        .w_full()
        .min_w(px(0.0))
        .flex()
        .justify_center()
        .py(px(theme.dimensions.block_padding_y.max(6.0)))
        .child(
            div()
                .relative()
                .w(px(width))
                .h(px(height))
                .when_some(selection_rect, |this, rect| {
                    this.child(
                        div()
                            .absolute()
                            .left(px(padding + rect.x as f32 * font_size))
                            .top(px(padding + rect.y as f32 * font_size))
                            .w(px((rect.w as f32 * font_size).max(2.0)))
                            .h(px((rect.h as f32 * font_size).max(font_size)))
                            .rounded(px(2.0))
                            .bg(wb.selection),
                    )
                })
                .child(
                    img(rendered.path.clone())
                        .w(px(width))
                        .h(px(height))
                        .object_fit(ObjectFit::Contain),
                )
                .when_some(marked_rect, |this, rect| {
                    this.child(
                        div()
                            .absolute()
                            .left(px(padding + rect.x as f32 * font_size))
                            .top(px(padding + (rect.y + rect.h) as f32 * font_size - 1.0))
                            .w(px((rect.w as f32 * font_size).max(2.0)))
                            .h(px(1.5))
                            .bg(wb.accent),
                    )
                })
                .when_some(caret_rect, |this, rect| {
                    this.child(
                        div()
                            .absolute()
                            .left(px(padding + rect.x as f32 * font_size))
                            .top(px(padding + rect.y as f32 * font_size))
                            .w(px(2.0))
                            .h(px((rect.h as f32 * font_size).max(font_size)))
                            .rounded(px(1.0))
                            .bg(wb.accent),
                    )
                }),
        )
        .into_any_element()
}
