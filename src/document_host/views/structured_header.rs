// @author kongweiguang

//! Structured table headers and column-window controls.

use super::*;
use crate::theme::{ThemeColors, workbench::SurfaceKind};
use crate::ui::visual_preferences::VisualPreferencesManager;

impl DocumentHost {
    /// Keeps header commands under the structured focus owner so Source shortcuts cannot consume cell navigation.
    pub(super) fn render_structured_header(
        &mut self,
        layout: &StructuredPanelLayout,
        colors: &ThemeColors,
        cx: &mut Context<Self>,
    ) -> Div {
        let structured_width = layout.width;
        let visible_structured_headers = layout.visible_headers.clone();
        let structured_column_widths = layout.column_widths.clone();
        let structured_live = layout.structured_live;
        let structured_selection_color = colors.table_axis_selected_bg;
        let visual_preferences = cx
            .try_global::<VisualPreferencesManager>()
            .map(VisualPreferencesManager::current)
            .unwrap_or_default();
        let header_material = colors
            .workbench
            .material(SurfaceKind::Elevated, visual_preferences);

        div()
            .h(px(30.0))
            .w(px(structured_width))
            .flex()
            .items_center()
            .bg(header_material.background)
            .border_b(px(1.0))
            .border_color(header_material.border)
            .text_size(px(11.0))
            .text_color(colors.workbench.text_primary)
            .child(div().w(px(76.0)).px(px(10.0)).child("#"))
            .children(
                visible_structured_headers
                    .into_iter()
                    .map(|(column, header)| {
                        let editing = self.structured_cell_edit
                            == Some(StructuredCellEdit {
                                record: None,
                                column,
                            });
                        let selected = self.structured_selected_cell
                            == Some(StructuredCellEdit {
                                record: None,
                                column,
                            });
                        let selection_target = StructuredCellEdit {
                            record: None,
                            column,
                        };
                        let edit_header = header.clone();
                        div()
                            .id(("document-host-structured-header", column))
                            .debug_selector(move || {
                                format!("document-host-structured-header-{column}")
                            })
                            .w(px(structured_column_widths
                                .get(column)
                                .copied()
                                .unwrap_or(STRUCTURED_CELL_WIDTH)))
                            .h_full()
                            .px(px(10.0))
                            .flex()
                            .items_center()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .tab_index(0)
                            .border_l(px(1.0))
                            .border_color(header_material.border)
                            .focus(|header_view| {
                                header_view.border_color(colors.workbench.focus_ring)
                            })
                            .when(selected, |header_view| {
                                header_view
                                    .bg(structured_selection_color)
                                    .border_color(colors.workbench.focus_ring)
                            })
                            .capture_key_down(cx.listener(Self::on_structured_table_key_down))
                            .on_key_down(cx.listener(Self::on_structured_table_key_down))
                            .child(if editing {
                                div()
                                    .id("document-host-structured-cell-editor")
                                    .debug_selector(|| {
                                        "document-host-structured-cell-editor".to_owned()
                                    })
                                    .size_full()
                                    .min_w(px(0.0))
                                    .overflow_hidden()
                                    .child(self.structured_cell_input.clone())
                                    .into_any_element()
                            } else {
                                div().child(header).into_any_element()
                            })
                            .cursor_text()
                            .on_click(cx.listener(move |this, _, window, cx| {
                                if this.structured_cell_edit == Some(selection_target) {
                                    return;
                                }
                                this.select_structured_cell(selection_target, window, cx)
                            }))
                            .when(structured_live, |header_view| {
                                header_view
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(
                                            move |this, event: &MouseDownEvent, window, cx| {
                                                if event.click_count >= 2 {
                                                    this.begin_structured_cell_edit(
                                                        None,
                                                        column,
                                                        edit_header.clone(),
                                                        window,
                                                        cx,
                                                    );
                                                    cx.stop_propagation();
                                                }
                                            },
                                        ),
                                    )
                                    .on_mouse_down(
                                        MouseButton::Right,
                                        cx.listener(move |this, _, _, cx| {
                                            this.structured_context_target =
                                                Some(StructuredMenuTarget::Column(column));
                                            cx.stop_propagation();
                                            cx.notify();
                                        }),
                                    )
                            })
                            .when(!structured_live, |header_view| {
                                header_view.on_mouse_down(
                                    MouseButton::Right,
                                    cx.listener(move |this, _, _, cx| {
                                        this.hidden_structured_columns.insert(column);
                                        cx.stop_propagation();
                                        cx.notify();
                                    }),
                                )
                            })
                    }),
            )
    }

    pub(super) fn render_structured_column_pager(
        &mut self,
        layout: &StructuredPanelLayout,
        colors: &ThemeColors,
        strings: &I18nStrings,
        cx: &mut Context<Self>,
    ) -> Option<Stateful<Div>> {
        let structured_width = layout.width;
        let structured_column_count = layout.column_count;
        let visual_preferences = cx
            .try_global::<VisualPreferencesManager>()
            .map(VisualPreferencesManager::current)
            .unwrap_or_default();
        let control_material = colors
            .workbench
            .material(SurfaceKind::Glass, visual_preferences);

        (structured_column_count > STRUCTURED_COLUMN_WINDOW).then(|| {
            let start = self.structured_column_window_start;
            let end = start
                .saturating_add(STRUCTURED_COLUMN_WINDOW)
                .min(structured_column_count);
            let previous = start.saturating_sub(STRUCTURED_COLUMN_WINDOW);
            let next = start
                .saturating_add(STRUCTURED_COLUMN_WINDOW)
                .min(structured_column_count.saturating_sub(1));
            div()
                .id("document-host-structured-column-pager")
                .debug_selector(|| "document-host-structured-column-pager".to_owned())
                .h(px(32.0))
                .w(px(structured_width))
                .px(px(8.0))
                .flex()
                .items_center()
                .gap(px(6.0))
                .border_b(px(1.0))
                .border_color(control_material.border)
                .text_size(px(12.0))
                .text_color(colors.workbench.text_tertiary)
                .child(
                    div()
                        .id("document-host-structured-columns-previous")
                        .debug_selector(|| "document-host-structured-columns-previous".to_owned())
                        .size(px(24.0))
                        .tab_index(0)
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(4.0))
                        .cursor_pointer()
                        .text_color(if start == 0 {
                            colors.workbench.text_tertiary
                        } else {
                            colors.workbench.text_primary
                        })
                        .bg(control_material.background)
                        .hover(|button| button.bg(colors.workbench.control_hover))
                        .focus(|button| button.border_color(colors.workbench.focus_ring))
                        .child("‹")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.set_structured_column_window_start(previous, cx);
                        })),
                )
                .child(
                    strings
                        .large_document_text("columns_window_template")
                        .replace("{start}", &(start + 1).to_string())
                        .replace("{end}", &end.to_string())
                        .replace("{total}", &structured_column_count.to_string()),
                )
                .child(
                    div()
                        .id("document-host-structured-columns-next")
                        .debug_selector(|| "document-host-structured-columns-next".to_owned())
                        .size(px(24.0))
                        .tab_index(0)
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(4.0))
                        .cursor_pointer()
                        .text_color(if end == structured_column_count {
                            colors.workbench.text_tertiary
                        } else {
                            colors.workbench.text_primary
                        })
                        .bg(control_material.background)
                        .hover(|button| button.bg(colors.workbench.control_hover))
                        .focus(|button| button.border_color(colors.workbench.focus_ring))
                        .child("›")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.set_structured_column_window_start(next, cx);
                        })),
                )
        })
    }
}
