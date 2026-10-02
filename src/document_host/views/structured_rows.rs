// @author kongweiguang

//! Virtualized structured rows and cell interactions.

use super::*;
use crate::theme::{ThemeColors, workbench::SurfaceKind};
use crate::ui::visual_preferences::VisualPreferencesManager;

impl DocumentHost {
    /// Shares one structured focus owner across virtual rows while each editable cell retains its native input target.
    pub(super) fn render_structured_list(
        &mut self,
        layout: &StructuredPanelLayout,
        colors: &ThemeColors,
        cx: &mut Context<Self>,
    ) -> gpui::UniformList {
        let layout = layout.clone();
        let structured_count = layout.row_count;
        let structured_width = layout.width;
        let structured_row_widths = layout.column_widths.clone();
        let json_structure = layout.json_structure;
        let structured_live = layout.structured_live;
        let loading_text = layout.loading_text.clone();
        let line_text_color = colors.text_default;
        let line_number_color = colors.text_placeholder;
        let visual_preferences = cx
            .try_global::<VisualPreferencesManager>()
            .map(VisualPreferencesManager::current)
            .unwrap_or_default();
        let content_material = colors
            .workbench
            .material(SurfaceKind::Solid, visual_preferences);
        let structured_border_color = content_material.border;
        let structured_selection_color = colors.table_axis_selected_bg;
        let focus_ring_color = colors.workbench.focus_ring;

        uniform_list(
            "document-host-structured-rows",
            structured_count,
            cx.processor(move |this, range: Range<usize>, _window, _cx| {
                this.request_structured_rows(range.clone(), _cx);
                range
                    .map(|row_index| {
                        let json_node = json_structure
                            .then(|| this.json_node_at(row_index as u64))
                            .flatten();
                        let logical_row = if let Some(node) = &json_node {
                            node.item
                        } else {
                            row_index as u64
                        };
                        let row = if let Some(node) = &json_node {
                            this.json_rows.get(&node.path()).cloned()
                        } else {
                            this.structured_rows.get(&logical_row).cloned()
                        };
                        let row_depth = row.as_ref().map_or(0, |row| row.depth);
                        let cells = row
                            .as_ref()
                            .map(|row| row.cells.clone())
                            .unwrap_or_else(|| vec![loading_text.clone()]);
                        let row_column_start = row
                            .as_ref()
                            .map_or(this.structured_column_window_start, |row| row.column_start);
                        div()
                            .id(("document-host-structured-row", row_index))
                            .debug_selector(move || {
                                format!("document-host-structured-row-{row_index}")
                            })
                            .h(px(26.0))
                            .w(px(structured_width))
                            .flex()
                            .items_center()
                            .border_b(px(1.0))
                            .border_color(structured_border_color)
                            .text_size(px(12.0))
                            .text_color(line_text_color)
                            .bg(content_material.background)
                            .child(
                                div()
                                    .id(("document-host-structured-row-number", logical_row))
                                    .w(px(76.0))
                                    .px(px(10.0))
                                    .text_align(gpui::TextAlign::Right)
                                    .text_color(line_number_color)
                                    .child(if json_structure {
                                        String::new()
                                    } else {
                                        (logical_row + 1).to_string()
                                    })
                                    .when(structured_live && !json_structure, |gutter| {
                                        gutter
                                            .cursor_context_menu()
                                            .on_mouse_down(
                                                MouseButton::Right,
                                                _cx.listener(move |this, _, _, cx| {
                                                    this.structured_context_target = Some(
                                                        StructuredMenuTarget::Row(logical_row),
                                                    );
                                                    cx.stop_propagation();
                                                    cx.notify();
                                                }),
                                            )
                                    }),
                            )
                            .children(
                                cells
                                    .into_iter()
                                    .enumerate()
                                    .map(move |(column, cell)| {
                                        (row_column_start.saturating_add(column), cell)
                                    })
                                    .filter(|(column, _)| {
                                        !this.hidden_structured_columns.contains(column)
                                    })
                                    .map(|(column, cell)| {
                                        let cell = this
                                            .structured_cell_overrides
                                            .get(&StructuredCellEdit {
                                                record: Some(logical_row),
                                                column,
                                            })
                                            .cloned()
                                            .unwrap_or(cell);
                                        let json_prefix = if json_structure && column == 0 {
                                            let path = json_node
                                                .as_ref()
                                                .map(JsonNode::path)
                                                .unwrap_or_default();
                                            if this.json_expanded_nodes.contains(&path) {
                                                "▾ "
                                            } else {
                                                "▸ "
                                            }
                                        } else {
                                            ""
                                        };
                                        let editing = this.structured_cell_edit
                                            == Some(StructuredCellEdit {
                                                record: Some(logical_row),
                                                column,
                                            });
                                        let selected = this.structured_selected_cell
                                            == Some(StructuredCellEdit {
                                                record: Some(logical_row),
                                                column,
                                            });
                                        let selection_target = StructuredCellEdit {
                                            record: Some(logical_row),
                                            column,
                                        };
                                        let display_cell = cell.clone();
                                        div()
                                            .id(SharedString::from(format!(
                                                "document-host-structured-cell-{logical_row}-{column}"
                                            )))
                                            .debug_selector(move || {
                                                format!(
                                                    "document-host-structured-cell-{logical_row}-{column}"
                                                )
                                            })
                                            .w(px(structured_row_widths
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
                                            .border_color(structured_border_color)
                                            .focus(|cell_view| {
                                                cell_view.border_color(focus_ring_color)
                                            })
                                            .when(selected, |cell_view| {
                                                cell_view
                                                    .bg(structured_selection_color)
                                                    .border_color(focus_ring_color)
                                            })
                                            .capture_key_down(
                                                _cx.listener(Self::on_structured_table_key_down),
                                            )
                                            .on_key_down(
                                                _cx.listener(Self::on_structured_table_key_down),
                                            )
                                            .pl(px(10.0 + row_depth as f32 * 14.0))
                                            .child(if editing {
                                                div()
                                                    .id("document-host-structured-cell-editor")
                                                    .debug_selector(|| {
                                                        "document-host-structured-cell-editor".to_owned()
                                                    })
                                                    .size_full()
                                                    .min_w(px(0.0))
                                                    .overflow_hidden()
                                                    .child(this.structured_cell_input.clone())
                                                    .into_any_element()
                                            } else {
                                                div()
                                                    .child(format!(
                                                        "{json_prefix}{}",
                                                        truncate_cell(cell)
                                                    ))
                                                    .into_any_element()
                                            })
                                            .cursor_text()
                                            .on_click(_cx.listener(move |this, _, window, cx| {
                                                if this.structured_cell_edit
                                                    == Some(selection_target)
                                                {
                                                    return;
                                                }
                                                this.select_structured_cell(
                                                    selection_target,
                                                    window,
                                                    cx,
                                                )
                                            }))
                                            .when(structured_live && !json_structure, |cell_view| {
                                                cell_view
                                                    .on_mouse_down(
                                                        MouseButton::Left,
                                                        _cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                                                            if event.click_count >= 2 {
                                                                this.begin_structured_cell_edit(
                                                                    Some(logical_row),
                                                                    column,
                                                                    display_cell.clone(),
                                                                    window,
                                                                    cx,
                                                                );
                                                                cx.stop_propagation();
                                                            }
                                                        }),
                                                    )
                                            })
                                    }),
                            )
                            .when(json_structure, |row| {
                                row.on_click(_cx.listener(move |this, _, _window, cx| {
                                    this.activate_json_node(row_index as u64, cx);
                                }))
                            })
                            .when(
                                !json_structure && this.view_mode == DocumentHostViewMode::Split,
                                |row| {
                                    row.on_click(_cx.listener(move |this, _, _window, cx| {
                                        this.reveal_structured_row_in_split(logical_row, cx);
                                    }))
                                },
                            )
                    })
                    .collect::<Vec<_>>()
            }),
        )
        .track_scroll(self.structured_scroll_handle.clone())
        .h_full()
        .w(px(structured_width))
    }
}
