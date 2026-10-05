// @author kongweiguang

// 把 Render 实现独立出来，降低 facade 文件的结构复杂度而不改变渲染树。

use super::*;

impl Render for Editor {
    /// 先处理平台终态与共享事件，并在应用待处理导航后同步模态焦点，以保存真实输入目标。
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _trace = crate::perf::span("editor_render_sync");
        self.sync_shared_document_events(cx);
        self.sync_pending_ime_operations(window, cx);
        self.ensure_markdown_view_state();
        if self.pane_canvas {
            // 子窗格只省略窗口装饰，文档选区、查找与保存仍由实际输入目标收尾。
            if self.pane_canvas_focus_enabled {
                self.apply_pending_focus(window, cx);
            } else {
                self.pending_focus = None;
            }
            self.apply_pending_scroll_into_view(window, cx);
            self.sync_document_frame(window, cx);
            return self.render_pane_document_canvas(window, cx);
        }
        self.drain_pane_events(cx);
        // Keep the existing single-document editor path lazy.  A normal
        // render must not migrate the root session into a pane workspace;
        // migration is performed by split/restore actions only.  Once a
        // workspace is active, synchronize its active leaf entities here.
        if self.pane_workspace.is_some() {
            self.sync_pane_canvas_entities(cx);
        }
        if let Some(document_host) = self.document_host.as_ref() {
            let dirty = document_host.read(cx).is_dirty();
            self.document_dirty = dirty;
            self.pending_window_edited = dirty;
        }
        let source_bytes = self.source_document.len();
        if let Some(started) = self.first_render_started.take() {
            super::perf::emit(
                "editor_first_render",
                started,
                Some(source_bytes),
                Some(true),
                Some("GPUI render boundary; not draw or platform present"),
            );
        }
        self.install_close_guard(cx, window);
        self.install_menu_window_activation_observer(window, cx);
        self.apply_pending_focus(window, cx);
        self.sync_close_dialog_focus(window, cx);
        self.apply_pending_scroll_into_view(window, cx);
        self.sync_document_frame(window, cx);
        self.sync_workspace_session_view_state(cx);
        self.sync_window_edited_state(window);

        let content_area = if self.pane_workspace.is_some() {
            div().size_full().into_any_element()
        } else {
            self.render_document_content(window, cx)
        };
        // Rendered Markdown presentation state (fold keys, collapsed flags,
        // and table layout) is synchronized while building the content tree.
        // Publish the accessibility snapshot afterwards so the first frame
        // exposes the same fold semantics as the pixels on screen.
        self.sync_accessibility_bridge(window, cx);
        let theme = cx.global::<ThemeManager>().current_arc();
        let visual_preferences = cx
            .try_global::<VisualPreferencesManager>()
            .map(VisualPreferencesManager::current)
            .unwrap_or_default();
        let editor_material = theme
            .colors
            .workbench
            .material(SurfaceKind::Editor, visual_preferences);
        let compact_navigation_material = theme
            .colors
            .workbench
            .material(SurfaceKind::Navigation, visual_preferences);
        let strings = cx.global::<I18nManager>().strings_arc();
        let d = &theme.dimensions;
        let status_bar_height =
            if !self.focus_mode && EditorSettings::status_bar_preferences(cx).enabled {
                d.status_bar_height
            } else {
                0.0
            };
        let has_menus = cx
            .get_menus()
            .map(|menus| !menus.is_empty())
            .unwrap_or(false);
        let titlebar_height = custom_titlebar_height(window, d);
        let menu_chrome = in_window_menu_chrome_layout(
            std::env::consts::OS,
            has_menus,
            self.focus_mode,
            titlebar_height,
            d,
        );
        let menu_bar_height = menu_chrome.content_height;

        // Repaint when the Cmd/Ctrl follow modifier toggles so a hovered link's
        // hand cursor updates without moving the pointer. `ModifiersChanged` is
        // dispatched along the focused element's path to the root, and this root
        // is an ancestor of every block, so one listener here covers a link in any
        // block while editing. Gated to the secondary modifier so Shift during
        // selection does not repaint.
        let follow_modifier_active = window.modifiers().secondary();

        let base = div()
            .w_full()
            .h_full()
            .flex()
            .flex_col()
            .relative()
            .bg(editor_material.background)
            .on_modifiers_changed(move |event, window, _| {
                if event.modifiers.secondary() != follow_modifier_active {
                    window.refresh();
                }
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(Self::on_editor_surface_mouse_down),
            )
            .capture_action(cx.listener(Self::on_copy_capture))
            .capture_action(cx.listener(Self::on_copy_as_markdown_capture))
            .capture_action(cx.listener(Self::on_cut_capture))
            .capture_action(cx.listener(Self::on_paste_capture))
            .capture_action(cx.listener(Self::on_delete_capture))
            .capture_action(cx.listener(Self::on_delete_back_capture))
            .capture_key_down(cx.listener(Self::on_editor_key_down_capture))
            .can_drop(|dragged, _window, _cx| dragged.is::<ExternalPaths>())
            .on_drop::<ExternalPaths>(cx.listener(Self::on_external_paths_drop))
            .on_action(cx.listener(Self::on_undo))
            .on_action(cx.listener(Self::on_redo))
            .on_action(cx.listener(Self::on_save_document))
            .on_action(cx.listener(Self::on_save_document_as))
            .on_action(cx.listener(Self::on_export_html))
            .on_action(cx.listener(Self::on_export_image))
            .on_action(cx.listener(Self::on_export_pdf))
            .on_action(cx.listener(Self::on_show_document_info))
            .on_action(cx.listener(Self::on_show_document_outline))
            .on_action(cx.listener(Self::on_show_structure_view))
            .on_action(cx.listener(Self::on_show_structured_inspector))
            .on_action(cx.listener(Self::on_focus_structured_filter))
            .on_action(cx.listener(Self::on_focus_structured_columns))
            .on_action(cx.listener(Self::on_normalize_line_endings_lf))
            .on_action(cx.listener(Self::on_normalize_line_endings_crlf))
            .on_action(cx.listener(Self::on_normalize_line_endings_cr))
            .on_action(cx.listener(Self::on_quit_application))
            .on_action(cx.listener(Self::on_close_window))
            .on_action(cx.listener(Self::on_new_tab_action))
            .on_action(cx.listener(Self::on_close_tab_action))
            .on_action(cx.listener(Self::on_reopen_closed_tab_action))
            .on_action(cx.listener(Self::on_previous_tab_action))
            .on_action(cx.listener(Self::on_next_tab_action))
            .on_action(cx.listener(Self::on_toggle_view_mode_action))
            .on_action(cx.listener(Self::on_toggle_workspace_action))
            .on_action(cx.listener(Self::on_toggle_document_sidebar_action))
            .on_action(cx.listener(Self::on_quick_open_action))
            .on_action(cx.listener(Self::on_command_palette_action))
            .on_action(cx.listener(Self::on_go_to_line_action))
            .on_action(cx.listener(Self::on_find_in_document_action))
            .on_action(cx.listener(Self::on_replace_in_document_action))
            .on_action(cx.listener(Self::on_find_next_action))
            .on_action(cx.listener(Self::on_find_previous_action))
            .on_action(cx.listener(Self::on_open_folder_action))
            .on_action(cx.listener(Self::on_toggle_focus_mode_action))
            .on_action(cx.listener(Self::on_toggle_typewriter_mode_action))
            .on_action(cx.listener(Self::on_page_up))
            .on_action(cx.listener(Self::on_page_down))
            .on_action(cx.listener(Self::on_select_page_up))
            .on_action(cx.listener(Self::on_select_page_down))
            .on_action(cx.listener(Self::on_move_to_document_start))
            .on_action(cx.listener(Self::on_move_to_document_end))
            .on_action(cx.listener(Self::on_select_to_document_start))
            .on_action(cx.listener(Self::on_select_to_document_end))
            .on_action(cx.listener(Self::on_select_up))
            .on_action(cx.listener(Self::on_select_down))
            .capture_action(cx.listener(Self::on_duplicate_line))
            .capture_action(cx.listener(Self::on_delete_line))
            .capture_action(cx.listener(Self::on_move_line_up))
            .capture_action(cx.listener(Self::on_move_line_down))
            .capture_action(cx.listener(Self::on_indent_block))
            .capture_action(cx.listener(Self::on_outdent_block))
            .on_action(cx.listener(Self::on_jump_to_top))
            .on_action(cx.listener(Self::on_jump_to_bottom))
            .on_action(cx.listener(Self::on_dismiss_transient_ui))
            .on_action(cx.listener(Self::on_install_cli_tool))
            .on_action(cx.listener(Self::on_uninstall_cli_tool))
            .on_action(cx.listener(Self::on_split_right_action))
            .on_action(cx.listener(Self::on_split_down_action))
            .on_action(cx.listener(Self::on_close_pane_action))
            .on_action(cx.listener(Self::on_focus_pane_left_action))
            .on_action(cx.listener(Self::on_focus_pane_right_action))
            .on_action(cx.listener(Self::on_focus_pane_up_action))
            .on_action(cx.listener(Self::on_focus_pane_down_action))
            .on_action(cx.listener(Self::on_move_tab_to_pane_left_action))
            .on_action(cx.listener(Self::on_move_tab_to_pane_right_action))
            .on_action(cx.listener(Self::on_move_tab_to_pane_up_action))
            .on_action(cx.listener(Self::on_move_tab_to_pane_down_action))
            .on_action(cx.listener(Self::on_balance_panes_action));
        let base = base
            .on_action(cx.listener(Self::on_collapse_fold_action))
            .on_action(cx.listener(Self::on_expand_fold_action))
            .on_action(cx.listener(Self::on_collapse_all_folds_action))
            .on_action(cx.listener(Self::on_expand_all_folds_action))
            .on_action(cx.listener(Self::on_format_document_action))
            .on_action(cx.listener(Self::on_format_selection_action))
            .on_action(cx.listener(Self::on_cancel_formatting_action));
        // Fetch menus + collect labels once for both renderers; previously each
        // of render_in_window_menu_bar / render_in_window_menu_panel called
        // cx.get_menus() and walked menus.iter().map(|m| m.name.to_string())
        // independently — two redundant Vec<OwnedMenu> + two redundant
        // Vec<String>-of-N-allocations per frame.
        let menus = supports_in_window_menu()
            .then(|| {
                let snapshot = cx
                    .try_global::<crate::app_menu::AppMenuState>()
                    .map(|state| state.in_window_menus.clone())
                    .unwrap_or_default();
                let platform_menus = cx.get_menus().unwrap_or_default();
                // Windows 的原生菜单桥在窄标题栏可能只回传前几个菜单。
                // 快照来自 install_menus，是完整的应用命令模型，优先保证可达性。
                if snapshot.len() > platform_menus.len() {
                    snapshot
                } else {
                    platform_menus
                }
            })
            .map(|menus| self.contextual_menus(menus, cx))
            .filter(|menus| !menus.is_empty());
        let menu_labels: Vec<SharedString> = menus
            .as_ref()
            .map(|m| m.iter().map(|menu| menu.name.clone()).collect())
            .unwrap_or_default();
        let base = if let Some(titlebar) = render_custom_titlebar(
            "editor-titlebar",
            None,
            None,
            &theme,
            window,
            cx,
            Self::on_titlebar_close,
        ) {
            base.child(titlebar)
        } else {
            base
        };
        let base = if self.focus_mode {
            base
        } else if let Some(menu_bar) = self.render_in_window_menu_bar(
            &theme,
            cx,
            menus.as_deref(),
            &menu_labels,
            menu_chrome.bar_top,
            menu_chrome.bar_height,
            menu_chrome.origin_x,
            menu_chrome.integrated,
        ) {
            base.child(menu_bar)
        } else {
            base
        };
        let tab_strip_height = self.tab_strip_height();
        let viewport_width = f32::from(window.viewport_size().width);
        let compact_workspace = workspace_uses_overlay(viewport_width);
        self.sync_document_sidebar_visibility_for_viewport(viewport_width);
        let compact_both_sidebars =
            compact_workspace && self.workspace.is_open && self.workspace.document_sidebar_open;
        let compact_side_width = (viewport_width * 0.5).max(1.0);
        let workspace_width =
            workspace_panel_width_for_viewport(viewport_width, self.workspace_panel_width()).min(
                if compact_both_sidebars {
                    compact_side_width
                } else {
                    viewport_width.max(1.0)
                },
            );
        let workspace_panel =
            self.render_workspace_panel(&theme, &strings, workspace_width, !compact_workspace, cx);
        let effective_workspace_width = if workspace_panel.is_some() && !compact_workspace {
            workspace_width
        } else {
            0.0
        };
        let document_sidebar_width = document_sidebar_panel_width_for_viewport(
            viewport_width,
            self.document_sidebar_panel_width(),
        )
        .min(if compact_both_sidebars {
            compact_side_width
        } else {
            viewport_width.max(1.0)
        });
        let document_sidebar_can_dock = !compact_workspace
            && viewport_width - effective_workspace_width - document_sidebar_width
                >= SPLIT_PANE_MIN_WIDTH * 2.0 + SPLIT_DIVIDER_HIT_WIDTH;
        let document_sidebar_panel = self.render_document_sidebar(
            &theme,
            &strings,
            document_sidebar_width,
            document_sidebar_can_dock,
            cx,
        );
        let effective_document_sidebar_width =
            if document_sidebar_panel.is_some() && document_sidebar_can_dock {
                document_sidebar_width
            } else {
                0.0
            };
        let (docked_workspace_panel, overlay_workspace_panel) = if compact_workspace {
            (None, workspace_panel)
        } else {
            (workspace_panel, None)
        };
        let (docked_document_sidebar, overlay_document_sidebar) = if document_sidebar_can_dock {
            (document_sidebar_panel, None)
        } else {
            (None, document_sidebar_panel)
        };
        let left_workspace_panel = docked_workspace_panel;
        let (tab_strip_left, _) = editor_tab_strip_insets(effective_workspace_width);
        let tab_strip_right = effective_document_sidebar_width;
        let base = if let Some(tab_strip) = self.render_tab_strip(
            &theme,
            titlebar_height + menu_bar_height,
            tab_strip_left,
            tab_strip_right,
            cx,
        ) {
            base.child(tab_strip)
        } else {
            base
        };
        let main_content = div()
            .id("editor-main-content")
            .debug_selector(|| "editor-main-content".to_owned())
            .w_full()
            .flex_1()
            .min_h(px(0.0))
            .pt(px(titlebar_height + menu_bar_height))
            .flex()
            .min_w(px(0.0))
            .capture_any_mouse_down(cx.listener(Self::on_editor_main_content_capture_mouse_down))
            .on_mouse_move(cx.listener(Self::on_editor_layout_resize_mouse_move))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(Self::on_editor_layout_resize_mouse_up),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(Self::on_editor_layout_resize_mouse_up),
            );
        let main_content = if let Some(workspace_panel) = left_workspace_panel {
            main_content.child(workspace_panel)
        } else {
            main_content
        };
        let resident_content = self.render_resident_surface(
            content_area.into_any_element(),
            &theme,
            (f32::from(window.viewport_size().width)
                - effective_workspace_width
                - effective_document_sidebar_width
                - SPLIT_DIVIDER_HIT_WIDTH)
                .max(1.0),
            (f32::from(window.viewport_size().height)
                - titlebar_height
                - menu_bar_height
                - tab_strip_height
                - status_bar_height)
                .max(1.0),
            window,
            cx,
        );
        let editor_content = if let Some(pane_workspace) = self.pane_workspace.as_ref() {
            let pane_viewport = crate::editor::panes::PaneViewport::new(
                (viewport_width - effective_workspace_width - effective_document_sidebar_width)
                    .max(1.0),
                (f32::from(window.viewport_size().height)
                    - titlebar_height
                    - menu_bar_height
                    - tab_strip_height
                    - status_bar_height)
                    .max(1.0),
            );
            pane_workspace.update(cx, |workspace, _cx| {
                workspace.set_viewport(pane_viewport);
            });
            div()
                .id("pane-workspace-content")
                .debug_selector(|| "pane-workspace-content".to_owned())
                .size_full()
                .min_w(px(0.0))
                .min_h(px(0.0))
                .child(pane_workspace.clone())
                .into_any_element()
        } else if let Some(document_host) = self.document_host.as_ref() {
            div()
                .id("document-host-tab-content")
                .debug_selector(|| "document-host-tab-content".to_owned())
                .size_full()
                .child(document_host.clone())
                .into_any_element()
        } else {
            resident_content
        };
        let editor_content = div()
            .id("editor-content")
            .debug_selector(|| "editor-content".to_owned())
            .w_full()
            .flex_1()
            .min_h(px(0.0))
            .min_w(px(0.0))
            .font(editor_text_font(cx))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(Self::on_editor_content_mouse_down),
            )
            .child(editor_content);
        // Tab 属于编辑器列而非整个窗口；工作区从导航下方直接开始，右侧内容则在 Tab 下方开始。
        let editor_pane = div()
            .id("editor-pane")
            .debug_selector(|| "editor-pane".to_owned())
            .h_full()
            .flex_1()
            .min_h(px(0.0))
            .min_w(px(0.0))
            .pt(px(tab_strip_height))
            .flex()
            .flex_col()
            .child(editor_content);
        let main_content = main_content.child(editor_pane);
        let main_content = if let Some(document_sidebar) = docked_document_sidebar {
            main_content.child(document_sidebar)
        } else {
            main_content
        };
        let base = base.child(main_content);
        let base = if let Some(find_panel) = self.render_find_panel(
            &theme,
            &strings,
            titlebar_height + menu_bar_height + tab_strip_height,
            cx,
        ) {
            base.child(find_panel)
        } else {
            base
        };
        let base = if let Some(workspace_panel) = overlay_workspace_panel {
            let overlay = div()
                .id("compact-workspace-overlay")
                .debug_selector(|| "compact-workspace-overlay".to_owned())
                .absolute()
                .top(px(titlebar_height + menu_bar_height))
                .bottom(px(status_bar_height))
                .w(px(workspace_width))
                .bg(compact_navigation_material.background)
                .border_r(px(theme.dimensions.dialog_border_width))
                .border_color(compact_navigation_material.border)
                .shadow_lg()
                .child(workspace_panel);
            let overlay = overlay.left(px(0.0));
            base.child(overlay)
        } else {
            base
        };
        let base = if let Some(document_sidebar) = overlay_document_sidebar {
            let overlay = div()
                .id("compact-document-sidebar-overlay")
                .debug_selector(|| "compact-document-sidebar-overlay".to_owned())
                .absolute()
                .top(px(titlebar_height + menu_bar_height))
                .bottom(px(status_bar_height))
                .right(px(0.0))
                .w(px(document_sidebar_width))
                .bg(compact_navigation_material.background)
                .border_l(px(theme.dimensions.dialog_border_width))
                .border_color(compact_navigation_material.border)
                .shadow_lg()
                .child(document_sidebar);
            base.child(overlay)
        } else {
            base
        };
        let base = if let Some(status_bar) = self.render_status_bar(&theme, &strings, window, cx) {
            base.child(status_bar)
        } else {
            base
        };
        let base = if self.focus_mode {
            base
        } else if let Some(menu_panel) = self.render_in_window_menu_panel(
            &theme,
            window,
            cx,
            menus.as_deref(),
            &menu_labels,
            menu_chrome.panel_top_offset,
            menu_chrome.origin_x,
            f32::from(window.viewport_size().height.max(px(1.0))),
        ) {
            base.child(menu_panel)
        } else {
            base
        };
        let base = if let Some(context_menu) = self.render_context_menu_overlay(&theme, window, cx)
        {
            base.child(context_menu)
        } else {
            base
        };
        let base = if let Some(table_dialog) = self.render_table_insert_dialog_overlay(&theme, cx) {
            base.child(table_dialog)
        } else {
            base
        };
        let base =
            if let Some(prompt) = self.render_table_fragment_merge_prompt(&theme, &strings, cx) {
                base.child(prompt)
            } else {
                base
            };
        let base =
            if let Some(completion) = self.render_workspace_link_completion(&theme, &strings, cx) {
                base.child(completion)
            } else {
                base
            };
        let base = if let Some(workspace_dialog) =
            self.render_workspace_operation_dialog_overlay(&theme, &strings, cx)
        {
            base.child(workspace_dialog)
        } else {
            base
        };
        let base = if let Some(quick_open) = self.render_quick_open_overlay(&theme, &strings, cx) {
            base.child(quick_open)
        } else {
            base
        };
        let base = if let Some(command_palette) =
            self.render_command_palette_overlay(&theme, &strings, cx)
        {
            base.child(command_palette)
        } else {
            base
        };
        let base = if let Some(tab_context_menu) =
            self.render_tab_context_menu_overlay(&theme, &strings, window, cx)
        {
            base.child(tab_context_menu)
        } else {
            base
        };
        let base = if let Some(new_tab_menu) =
            self.render_new_tab_menu_overlay(&theme, &strings, window, cx)
        {
            base.child(new_tab_menu)
        } else {
            base
        };
        let base = if let Some(split_pane_menu) =
            self.render_split_pane_menu_overlay(&theme, window, cx)
        {
            base.child(split_pane_menu)
        } else {
            base
        };
        let base = if self.export_in_progress {
            base.child(self.render_export_progress(&theme, status_bar_height, cx))
        } else {
            base
        };
        let base =
            if let Some(update_panel) = self.render_update_panel(&theme, status_bar_height, cx) {
                base.child(update_panel)
            } else {
                base
            };
        let base = if let Some(overlay) = self.render_diagram_overlay(&theme, &strings, window, cx)
        {
            base.child(overlay)
        } else {
            base
        };
        let base = base.children(self.render_resource_title_dialog_overlay(&theme, cx));
        let base = if let Some(tab_close_dialog) =
            self.render_tab_close_dialog_overlay(&theme, &strings, cx)
        {
            base.child(tab_close_dialog)
        } else {
            base
        };
        self.render_document_dialogs(base, &theme, window, cx)
    }
}
