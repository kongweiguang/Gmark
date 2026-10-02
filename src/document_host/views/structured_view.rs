// @author kongweiguang

//! Composition root for the virtualized structured-document panel.

use super::*;
use crate::theme::workbench::SurfaceKind;
use crate::ui::visual_preferences::VisualPreferencesManager;

impl DocumentHost {
    /// 构建结构化 CSV/JSON/Markdown 表格面板；源码滚动面保持独立所有权。
    pub(super) fn render_structured_panel(&mut self, cx: &mut Context<Self>) -> Stateful<Div> {
        let theme = cx.global::<ThemeManager>().current_arc();
        let strings = cx.global::<I18nManager>().strings_arc();
        let colors = &theme.colors;
        let visual_preferences = cx
            .try_global::<VisualPreferencesManager>()
            .map(VisualPreferencesManager::current)
            .unwrap_or_default();
        let content_material = colors
            .workbench
            .material(SurfaceKind::Solid, visual_preferences);
        let divider_material = colors
            .workbench
            .material(SurfaceKind::Glass, visual_preferences);
        let layout = self.structured_panel_layout(&strings);
        let structured_width = layout.width;
        let structured_list = self.render_structured_list(&layout, colors, cx);
        let structured_scrollbar = self.render_structured_scrollbar(layout.row_count, colors, cx);
        let structured_header = self.render_structured_header(&layout, colors, cx);
        let structured_column_pager =
            self.render_structured_column_pager(&layout, colors, &strings, cx);
        let markdown_table_switcher =
            self.render_markdown_table_switcher(structured_width, colors, &strings, cx);
        let structured_operation_bar = self.render_structured_operation_bar(colors, &strings, cx);
        let add_row = self.render_structured_add_row(layout.structured_live, colors, &strings, cx);
        let context_menu = self.render_structured_context_menu(colors, &strings, cx);
        let content = div()
            .id("document-host-structured-content")
            .debug_selector(|| "document-host-structured-content".to_owned())
            .tab_index(0)
            .track_focus(&self.structured_focus_handle)
            .on_action(cx.listener(Self::on_structured_grid_indent))
            .on_action(cx.listener(Self::on_structured_grid_outdent))
            .capture_key_down(cx.listener(Self::on_structured_table_key_down))
            .on_key_down(cx.listener(Self::on_structured_table_key_down))
            .h_full()
            .w(px(structured_width))
            .relative()
            .flex()
            .flex_col()
            .children(markdown_table_switcher)
            .children(structured_column_pager)
            .children(structured_operation_bar)
            .child(structured_header)
            .child(div().flex_1().min_h(px(0.0)).child(structured_list))
            .children(add_row)
            .children(context_menu);
        if self.view_mode == DocumentHostViewMode::Split {
            let mut horizontal_scroll = div()
                .id("document-host-split-structure-horizontal-scroll")
                .size_full()
                .overflow_x_scroll()
                .track_scroll(&self.structured_horizontal_scroll_handle)
                .on_scroll_wheel(cx.listener(Self::on_horizontal_container_scroll_wheel))
                .child(content);
            // GPUI 默认会把纯纵向滚轮转成横向；表格嵌在纵向列表时必须禁用该回退。
            horizontal_scroll.style().restrict_scroll_to_axis = Some(true);
            div()
                .id("document-host-split-structure")
                .debug_selector(|| "document-host-split-structure".to_owned())
                .w(relative(0.5))
                .h_full()
                .min_w(px(0.0))
                .relative()
                .overflow_hidden()
                .border_l(px(1.0))
                .border_color(divider_material.border)
                .bg(content_material.background)
                .child(horizontal_scroll)
                .children(structured_scrollbar)
        } else {
            let mut horizontal_scroll = div()
                .id("document-host-structured-horizontal-scroll")
                .size_full()
                .overflow_x_scroll()
                .track_scroll(&self.structured_horizontal_scroll_handle)
                .on_scroll_wheel(cx.listener(Self::on_horizontal_container_scroll_wheel))
                .child(content);
            horizontal_scroll.style().restrict_scroll_to_axis = Some(true);
            div()
                .id("document-host-structured-scroll")
                .debug_selector(|| "document-host-structured-scroll".to_owned())
                .flex_1()
                .min_h(px(0.0))
                .relative()
                .overflow_hidden()
                .bg(content_material.background)
                .child(horizontal_scroll)
                .children(structured_scrollbar)
        }
    }
}
