// @author kongweiguang

//! GPUI composition root for the normalized document host.

use super::*;
use crate::theme::workbench::SurfaceKind;
use crate::ui::visual_preferences::VisualPreferencesManager;

#[path = "render/context_menu.rs"]
mod context_menu;
#[path = "render/main_panel.rs"]
mod main_panel;
#[path = "render/navigation.rs"]
mod navigation;
#[path = "render/notices.rs"]
mod notices;
#[path = "render/search.rs"]
mod search;
#[path = "render/source.rs"]
mod source;

impl DocumentHost {
    /// 渲染拥有真实窗口句柄；持久化调度仍按正文 revision 去重，选择和滚动不会重启空闲计时。
    fn prepare_document_render(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.flush_pending_save_requests(cx);
        self.schedule_auto_save(window, cx);
        if self.graph_focus_subscription.is_none() {
            let focus_handle = self.graph_focus_handle.clone();
            self.graph_focus_subscription =
                Some(cx.on_blur(&focus_handle, window, |this, _window, cx| {
                    if this.graph_selected_item.is_some() {
                        this.dismiss_json_graph_details();
                        cx.notify();
                    }
                }));
        }
        if !self.displayed_screen_lines.rows.is_empty()
            && let Some(started) = self.first_render_started.take()
        {
            let profile = self.probe.profile();
            let plan = session_plan(&profile, &self.probe, self.probe.strategy, false);
            crate::perf::emit_document(
                "document_first_source_frame",
                started,
                usize::try_from(self.probe.len).ok(),
                Some(true),
                &profile.format,
                &plan,
                Some("GPUI render boundary; not platform present"),
            );
        }
        if self.soak_ready_published && self.displayed_screen_lines.rows.is_empty() {
            self.metrics.blank_frames_after_content =
                self.metrics.blank_frames_after_content.saturating_add(1);
        }
        let (layout_hits, layout_misses, layout_entries) = self.source_row_blocks.values().fold(
            (0u64, 0u64, 0usize),
            |(hits, misses, entries), block| {
                let block = block.read(cx);
                (
                    hits.saturating_add(block.source_layout_cache_hits),
                    misses.saturating_add(block.source_layout_cache_misses),
                    entries + usize::from(block.source_layout_cache_key.is_some()),
                )
            },
        );
        self.metrics.layout_cache_hits = layout_hits;
        self.metrics.layout_cache_misses = layout_misses;
        if layout_entries > self.metrics.max_layout_cache_entries {
            self.metrics.max_layout_cache_entries = layout_entries;
            let profile = self.probe.profile();
            let plan = session_plan(&profile, &self.probe, self.probe.strategy, false);
            crate::perf::emit_document_value(
                "document_layout_cache_peak",
                layout_entries as u64,
                &profile.format,
                &plan,
            );
        }
    }
}

impl Render for DocumentHost {
    /// 将文档级键盘命令注册在 Host 焦点面上，使虚拟 Source 与结构视图共用稳定入口。
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.prepare_document_render(window, cx);
        let viewport_width = f32::from(window.viewport_size().width).max(1.0);
        let viewport_height = f32::from(window.viewport_size().height).max(1.0);
        let document_host_bounds = self.document_host_bounds.clone();
        let document_host_bounds_tracker = canvas(
            move |bounds, _, _| {
                if let Ok(mut current) = document_host_bounds.lock() {
                    *current = Some(bounds);
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .top_0()
        .left_0()
        .right_0()
        .bottom_0();
        let body = self.render_main_panel(window, cx);
        let search_panel = self.render_search_panel(cx);
        let navigation_panel = self.render_navigation_panel(cx);
        let structure_banner = self.render_structure_banner(cx);
        let oversized_selection_banner = self.render_oversized_selection_banner(cx);
        let external_banner = self.render_external_change_banner(cx);
        let source_context_menu =
            self.render_source_context_menu(viewport_width, viewport_height, cx);
        let graph_edit_overlay = (self.probe.format == DocumentFormat::Json)
            .then(|| self.render_json_graph_edit_overlay(viewport_width, viewport_height, cx));
        let colors = &cx.global::<ThemeManager>().current_arc().colors;
        let visual_preferences = cx
            .try_global::<VisualPreferencesManager>()
            .map(VisualPreferencesManager::current)
            .unwrap_or_default();
        let editor_material = colors
            .workbench
            .material(SurfaceKind::Editor, visual_preferences);
        let content = div()
            .size_full()
            .flex()
            .flex_col()
            // 宿主接管活动行焦点后仍需沿用文本编辑快捷键上下文，否则 Ctrl+Y 等
            // 仅绑定在 BlockEditor 的动作无法到达这里。
            .key_context(DOCUMENT_HOST_KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            // 右键打开菜单时，焦点路径里可能仍包含行内 Block；在捕获阶段关闭
            // 瞬态菜单，避免 Block 先消费 Escape 导致菜单残留。
            .capture_key_down(cx.listener(Self::on_source_surface_key_down))
            .capture_action(cx.listener(Self::on_select_all_capture))
            .capture_action(cx.listener(Self::on_source_boundary_copy_capture))
            .capture_action(cx.listener(|this, _: &MoveLeft, window, cx| {
                if this.route_source_horizontal(-1, false, false, window, cx) {
                    cx.stop_propagation();
                } else {
                    cx.propagate();
                }
            }))
            .capture_action(cx.listener(|this, _: &MoveRight, window, cx| {
                if this.route_source_horizontal(1, false, false, window, cx) {
                    cx.stop_propagation();
                } else {
                    cx.propagate();
                }
            }))
            .capture_action(cx.listener(|this, _: &SelectLeft, window, cx| {
                if this.route_source_horizontal(-1, false, true, window, cx) {
                    cx.stop_propagation();
                } else {
                    cx.propagate();
                }
            }))
            .capture_action(cx.listener(|this, _: &SelectRight, window, cx| {
                if this.route_source_horizontal(1, false, true, window, cx) {
                    cx.stop_propagation();
                } else {
                    cx.propagate();
                }
            }))
            .capture_action(cx.listener(|this, _: &WordMoveLeft, window, cx| {
                if this.route_source_horizontal(-1, true, false, window, cx) {
                    cx.stop_propagation();
                } else {
                    cx.propagate();
                }
            }))
            .capture_action(cx.listener(|this, _: &WordMoveRight, window, cx| {
                if this.route_source_horizontal(1, true, false, window, cx) {
                    cx.stop_propagation();
                } else {
                    cx.propagate();
                }
            }))
            .capture_action(cx.listener(|this, _: &WordSelectLeft, window, cx| {
                if this.route_source_horizontal(-1, true, true, window, cx) {
                    cx.stop_propagation();
                } else {
                    cx.propagate();
                }
            }))
            .capture_action(cx.listener(|this, _: &WordSelectRight, window, cx| {
                if this.route_source_horizontal(1, true, true, window, cx) {
                    cx.stop_propagation();
                } else {
                    cx.propagate();
                }
            }))
            .capture_action(cx.listener(|this, _: &Home, window, cx| {
                if this.route_source_visual_line_boundary(false, false, window, cx) {
                    cx.stop_propagation();
                } else {
                    cx.propagate();
                }
            }))
            .capture_action(cx.listener(|this, _: &End, window, cx| {
                if this.route_source_visual_line_boundary(true, false, window, cx) {
                    cx.stop_propagation();
                } else {
                    cx.propagate();
                }
            }))
            .capture_action(cx.listener(|this, _: &SelectHome, window, cx| {
                if this.route_source_visual_line_boundary(false, true, window, cx) {
                    cx.stop_propagation();
                } else {
                    cx.propagate();
                }
            }))
            .capture_action(cx.listener(|this, _: &SelectEnd, window, cx| {
                if this.route_source_visual_line_boundary(true, true, window, cx) {
                    cx.stop_propagation();
                } else {
                    cx.propagate();
                }
            }))
            .on_action(cx.listener(Self::on_undo))
            .on_action(cx.listener(Self::on_redo))
            .on_action(cx.listener(Self::on_save_document))
            .on_action(cx.listener(Self::on_find_in_document))
            .on_action(cx.listener(Self::on_go_to_line))
            .on_action(cx.listener(Self::on_find_next))
            .on_action(cx.listener(Self::on_find_previous))
            .on_action(cx.listener(Self::on_dismiss_transient_ui))
            .on_action(cx.listener(Self::on_copy))
            .on_action(cx.listener(Self::on_cut))
            .on_action(cx.listener(Self::on_paste))
            .on_action(cx.listener(Self::on_delete))
            .on_action(cx.listener(Self::on_delete_back))
            .on_action(cx.listener(Self::on_select_all))
            .on_action(cx.listener(Self::on_select_up))
            .on_action(cx.listener(Self::on_select_down))
            .on_action(cx.listener(Self::on_select_page_up))
            .on_action(cx.listener(Self::on_select_page_down))
            .on_action(cx.listener(Self::on_move_to_document_start))
            .on_action(cx.listener(Self::on_move_to_document_end))
            .on_action(cx.listener(Self::on_select_to_document_start))
            .on_action(cx.listener(Self::on_select_to_document_end))
            .on_action(cx.listener(Self::on_export_selection))
            .on_action(cx.listener(Self::on_page_up))
            .on_action(cx.listener(Self::on_page_down))
            .on_action(cx.listener(Self::on_jump_to_top))
            .on_action(cx.listener(Self::on_jump_to_bottom))
            .on_action(cx.listener(Self::on_collapse_fold))
            .on_action(cx.listener(Self::on_expand_fold))
            .on_action(cx.listener(Self::on_collapse_all_folds))
            .on_action(cx.listener(Self::on_expand_all_folds))
            .on_action(cx.listener(Self::on_duplicate_line))
            .on_action(cx.listener(Self::on_delete_line))
            .on_action(cx.listener(Self::on_move_line_up))
            .on_action(cx.listener(Self::on_move_line_down))
            .on_action(cx.listener(Self::on_indent_source))
            .on_action(cx.listener(Self::on_outdent_source))
            .on_action(cx.listener(Self::on_format_document))
            .on_action(cx.listener(Self::on_format_selection))
            .on_action(cx.listener(Self::on_cancel_formatting))
            .bg(editor_material.background)
            .children(external_banner)
            .children(oversized_selection_banner)
            .children(structure_banner)
            .child(body);
        div()
            .size_full()
            .relative()
            .key_context(DOCUMENT_HOST_KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::on_dismiss_transient_ui))
            .capture_key_down(cx.listener(Self::on_source_surface_key_down))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(Self::dismiss_source_context_menu_on_mouse_down),
            )
            .child(document_host_bounds_tracker)
            .child(content)
            .children(source_context_menu)
            .children(search_panel)
            .children(navigation_panel)
            .children(graph_edit_overlay.flatten())
    }
}
