// @author kongweiguang

//! Window-level editor state such as scrolling, mode switching, and menus.

use super::*;

#[path = "render/window/layout.rs"]
mod layout;

impl Editor {
    pub(super) fn focused_pane_entities(
        &self,
        cx: &App,
    ) -> (
        Option<Entity<Editor>>,
        Option<Entity<crate::document_host::DocumentHost>>,
    ) {
        let Some(workspace) = self.pane_workspace.as_ref() else {
            return (None, None);
        };
        let view = workspace.read(cx);
        let pane = view.workspace().focused_pane();
        let Some(tab) = view
            .workspace()
            .pane(pane)
            .and_then(|state| state.active_tab())
        else {
            return (None, None);
        };
        let canvases = self.pane_canvas_entities.borrow();
        let Some((tab_id, view_id, canvas)) = canvases.get(&pane) else {
            return (None, None);
        };
        if *tab_id != tab.id() || *view_id != tab.view().view_id() {
            return (None, None);
        }
        (canvas.markdown_editor(cx), canvas.document_host(cx))
    }

    /// Builds the OS window title, including the dirty marker when the
    /// document has unsaved changes.
    pub(super) fn window_title(
        file_path: Option<&Path>,
        is_dirty: bool,
        strings: &crate::i18n::I18nStrings,
    ) -> String {
        let base_title = if let Some(path) = file_path {
            format!(
                "Gmark - {}",
                path.file_name().map_or_else(
                    || path.to_string_lossy().to_string(),
                    |name| name.to_string_lossy().to_string()
                )
            )
        } else {
            "Gmark".to_string()
        };

        if is_dirty && !strings.dirty_title_marker.is_empty() {
            format!("{} {}", strings.dirty_title_marker, base_title)
        } else {
            base_title
        }
    }

    /// 快捷键与状态栏使用同一窗口模式来源，避免子窗格在下一帧被旧模式覆盖。
    pub(crate) fn on_toggle_view_mode_action(
        &mut self,
        _: &crate::components::ToggleViewMode,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_view_mode_from_ui(cx);
    }

    /// Markdown 模式由窗口统一下发；set_view_mode 的门控仍等待活动子窗格的组合终态。
    pub(super) fn toggle_view_mode_from_ui(&mut self, cx: &mut Context<Self>) {
        if !self.pane_canvas {
            let (markdown, host) = self.focused_pane_entities(cx);
            if markdown.is_some() {
                self.toggle_view_mode(cx);
                return;
            }
            if let Some(host) = host {
                let target = match self.view_mode {
                    ViewMode::Source => ViewMode::Preview,
                    ViewMode::Preview | ViewMode::Rendered => ViewMode::Source,
                    ViewMode::Split => ViewMode::Source,
                };
                host.update(cx, |host, cx| match target {
                    ViewMode::Source => host.show_source_view(cx),
                    ViewMode::Preview => host.show_structure_view(cx),
                    ViewMode::Split => host.show_split_view(cx),
                    ViewMode::Rendered => host.show_live_view(cx),
                });
                self.view_mode = target;
                return;
            }
        }
        self.end_block_pointer_selection_sessions(cx);
        self.last_selection_snapshot = self.capture_source_selection_snapshot(cx);
        self.toggle_view_mode(cx);
    }

    /// 正文与文档内公式/语言字段共用所属文档事务历史；工具字段不借用正文历史。
    pub(crate) fn on_undo(
        &mut self,
        action: &crate::components::Undo,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.pane_canvas {
            let (markdown, host) = self.focused_pane_entities(cx);
            if let Some(editor) = markdown {
                editor.update(cx, |editor, cx| editor.on_undo(action, window, cx));
                return;
            }
            if let Some(host) = host {
                host.update(cx, |host, cx| host.on_undo(action, window, cx));
                return;
            }
        }
        if let Some(document_host) = self.document_host.clone() {
            document_host.update(cx, |document_host, cx| {
                document_host.on_undo(action, window, cx);
            });
            return;
        }
        let Some((surface, _)) = self.focused_document_history_target(window, cx) else {
            return;
        };
        self.active_selection_surface = surface;
        if !self.document_surface_is_editable_for(surface)
            || self.defer_action_for_ime(action, window, cx)
        {
            return;
        }
        self.undo_document(cx);
        self.apply_pending_focus(window, cx);
    }

    /// 正文与文档内公式/语言字段共用所属文档事务历史；工具字段不借用正文历史。
    pub(crate) fn on_redo(
        &mut self,
        action: &crate::components::Redo,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.pane_canvas {
            let (markdown, host) = self.focused_pane_entities(cx);
            if let Some(editor) = markdown {
                editor.update(cx, |editor, cx| editor.on_redo(action, window, cx));
                return;
            }
            if let Some(host) = host {
                host.update(cx, |host, cx| host.on_redo(action, window, cx));
                return;
            }
        }
        if let Some(document_host) = self.document_host.clone() {
            document_host.update(cx, |document_host, cx| {
                document_host.on_redo(action, window, cx);
            });
            return;
        }
        let Some((surface, _)) = self.focused_document_history_target(window, cx) else {
            return;
        };
        self.active_selection_surface = surface;
        if !self.document_surface_is_editable_for(surface)
            || self.defer_action_for_ime(action, window, cx)
        {
            return;
        }
        self.redo_document(cx);
        self.apply_pending_focus(window, cx);
    }

    pub(crate) fn on_save_document(
        &mut self,
        action: &crate::components::SaveDocument,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.pane_canvas {
            let (markdown, host) = self.focused_pane_entities(cx);
            if let Some(editor) = markdown {
                editor.update(cx, |editor, cx| editor.on_save_document(action, window, cx));
                return;
            }
            if let Some(host) = host {
                host.update(cx, |host, cx| host.on_save_document(action, window, cx));
                return;
            }
        }
        if self.document_host.is_some() && self.file_path.is_none() {
            self.save_document_as(window, cx);
            return;
        }
        if let Some(document_host) = self.document_host.clone() {
            document_host.update(cx, |document_host, cx| {
                document_host.on_save_document(action, window, cx);
            });
            return;
        }
        self.request_save_document(cx);
    }

    /// 另存为对话框也等待原输入终态，不能在新路径窗口抢走候选焦点。
    pub(crate) fn on_save_document_as(
        &mut self,
        _action: &crate::components::SaveDocumentAs,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.pane_canvas {
            let (markdown, host) = self.focused_pane_entities(cx);
            if let Some(editor) = markdown {
                editor.update(cx, |editor, cx| {
                    editor.on_save_document_as(_action, window, cx)
                });
                return;
            }
            if let Some(host) = host {
                host.update(cx, |host, cx| {
                    host.on_save_document_as(_action, window, cx);
                });
                return;
            }
        }
        if self.defer_action_for_ime(_action, window, cx) {
            return;
        }
        self.request_save_document_as(cx);
    }

    /// 路由集中在实际格式入口，保证菜单和快捷动作使用相同文档、只读与历史边界。
    pub(crate) fn on_normalize_line_endings_lf(
        &mut self,
        _: &crate::components::NormalizeLineEndingsLf,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.normalize_line_endings(gmark_document::LineEnding::Lf, cx);
    }

    /// 复用实际格式入口的目标判定，避免动作与底栏菜单各自维护窗格规则。
    pub(crate) fn on_normalize_line_endings_crlf(
        &mut self,
        _: &crate::components::NormalizeLineEndingsCrLf,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.normalize_line_endings(gmark_document::LineEnding::CrLf, cx);
    }

    /// 同一入口负责活动文档和原子格式历史，CR 仅作为目标样式传入。
    pub(crate) fn on_normalize_line_endings_cr(
        &mut self,
        _: &crate::components::NormalizeLineEndingsCr,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.normalize_line_endings(gmark_document::LineEnding::Cr, cx);
    }

    pub(crate) fn on_export_html(
        &mut self,
        _: &crate::components::ExportHtml,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.pane_canvas {
            let (markdown, host) = self.focused_pane_entities(cx);
            if let Some(editor) = markdown {
                editor.update(cx, |editor, cx| {
                    editor.export_document_via_prompt(crate::export::ExportFormat::Html, window, cx)
                });
                return;
            }
            if host.is_some() {
                return;
            }
        }
        self.export_document_via_prompt(crate::export::ExportFormat::Html, window, cx);
    }

    pub(crate) fn on_export_pdf(
        &mut self,
        _: &crate::components::ExportPdf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.pane_canvas {
            let (markdown, host) = self.focused_pane_entities(cx);
            if let Some(editor) = markdown {
                editor.update(cx, |editor, cx| {
                    editor.export_document_via_prompt(crate::export::ExportFormat::Pdf, window, cx)
                });
                return;
            }
            if host.is_some() {
                return;
            }
        }
        self.export_document_via_prompt(crate::export::ExportFormat::Pdf, window, cx);
    }

    pub(crate) fn on_export_image(
        &mut self,
        _: &crate::components::ExportImage,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.pane_canvas {
            let (markdown, host) = self.focused_pane_entities(cx);
            if let Some(editor) = markdown {
                editor.update(cx, |editor, cx| {
                    editor.export_document_via_prompt(crate::export::ExportFormat::Png, window, cx)
                });
                return;
            }
            if host.is_some() {
                return;
            }
        }
        self.export_document_via_prompt(crate::export::ExportFormat::Png, window, cx);
    }

    /// 退出申请等待系统组合输入终态，再进入应用级最后租约门禁。
    pub(crate) fn on_quit_application(
        &mut self,
        action: &crate::components::QuitApplication,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.defer_action_for_ime(action, window, cx) {
            return;
        }
        crate::app_menu::request_quit_application(cx);
    }

    pub(crate) fn on_close_window(
        &mut self,
        _: &crate::components::CloseWindow,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.request_close_current_window(window, cx);
    }

    pub(crate) fn on_install_cli_tool(
        &mut self,
        _: &crate::components::InstallCliTool,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        crate::app_menu::install_cli_tool(cx);
    }

    pub(crate) fn on_uninstall_cli_tool(
        &mut self,
        _: &crate::components::UninstallCliTool,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        crate::app_menu::uninstall_cli_tool(cx);
    }

    pub(crate) fn toggle_view_mode(&mut self, cx: &mut Context<Self>) {
        let tabular_document = self
            .document_host
            .as_ref()
            .is_some_and(|view| view.read(cx).supports_tabular_modes());
        let target = if self.is_svg_document() {
            match self.view_mode {
                ViewMode::Source => ViewMode::Preview,
                ViewMode::Preview | ViewMode::Split | ViewMode::Rendered => ViewMode::Source,
            }
        } else {
            match self.view_mode {
                ViewMode::Rendered | ViewMode::Preview => ViewMode::Source,
                ViewMode::Source if tabular_document => ViewMode::Preview,
                ViewMode::Source => ViewMode::Rendered,
                ViewMode::Split if tabular_document => ViewMode::Source,
                ViewMode::Split => ViewMode::Rendered,
            }
        };
        self.set_view_mode(target, cx);
    }

    /// 模式切换更换输入投影，等待原候选与分页确认文字收尾，避免旧坐标写入新表面。
    pub(crate) fn set_view_mode(&mut self, target: ViewMode, cx: &mut Context<Self>) {
        if target != self.view_mode
            && (self.has_active_ime_composition(cx)
                || self.has_pending_source_input_for_active_target(cx))
        {
            let tab = self.tabs.records.get(self.tabs.active).map(|tab| tab.id);
            self.queue_ime_operation(
                ime_lifecycle::DeferredImeOperation::Mode { tab, mode: target },
                cx,
            );
            return;
        }
        // 模式改变会替换内容坐标空间；父编辑器的菜单也不能跨视图继续存在。
        self.dismiss_contextual_overlays(cx);
        if self.is_svg_document() && target == ViewMode::Rendered {
            return;
        }
        if let Some(document_host) = self.document_host.clone() {
            let json_document = document_host.read(cx).is_json_document();
            let delimited_document = document_host.read(cx).is_delimited_document();
            let tabular_document = json_document || delimited_document;
            if delimited_document
                && target == ViewMode::Rendered
                && !document_host.read(cx).source_is_utf8()
            {
                self.request_encoding_conversion(cx);
                return;
            }
            let target = if json_document && target == ViewMode::Rendered {
                ViewMode::Preview
            } else {
                target
            };
            let split_ratio = self.split_pane_ratio;
            document_host.update(cx, |view, cx| {
                if tabular_document {
                    view.set_json_split_ratio(split_ratio, cx);
                    match target {
                        ViewMode::Source => view.show_source_view(cx),
                        ViewMode::Split => view.show_split_view(cx),
                        ViewMode::Preview => view.show_structure_view(cx),
                        ViewMode::Rendered if delimited_document => view.show_live_view(cx),
                        ViewMode::Rendered => view.show_structure_view(cx),
                    }
                } else {
                    match target {
                        ViewMode::Source => view.show_source_view(cx),
                        ViewMode::Rendered => view.show_mode_unavailable("Live", cx),
                        ViewMode::Split => view.show_mode_unavailable("Split", cx),
                        ViewMode::Preview => view.show_mode_unavailable("Preview", cx),
                    }
                }
            });
            if tabular_document {
                self.view_mode = target;
                self.status_bar.format_overflow_open = false;
                self.schedule_workspace_session_save(cx);
                cx.notify();
                return;
            }
            // 大文件增强尚未产生 resident Markdown projection 时，模式控件必须保持
            // Source 选中，不能让 Live/Preview 标签与实际源码画布相互矛盾。
            self.view_mode = ViewMode::Source;
            self.status_bar.format_overflow_open = false;
            cx.notify();
            return;
        }
        if target != ViewMode::Preview && !self.source_encoding.is_utf8() {
            self.request_encoding_conversion(cx);
            return;
        }
        if self.view_mode == target {
            return;
        }

        // Formula structure editing is an ephemeral transaction. Changing the
        // rendered/source coordinate space must cancel every active session
        // before rebuilding or reusing blocks; every command has already been
        // published, so switching views only ends the visual session.
        for visible in self.document.flatten_visible_blocks() {
            visible.entity.update(cx, |block, cx| {
                if block.math_edit_session.is_some() {
                    block.finish_math_edit(cx);
                }
            });
        }

        self.end_block_pointer_selection_sessions(cx);
        let selection_snapshot = if self.view_mode == ViewMode::Preview {
            self.last_selection_snapshot
        } else {
            self.capture_source_selection_snapshot(cx)
        };
        self.clear_cross_block_selection(cx);
        self.rendered_select_all_cycle = None;
        if target == ViewMode::Preview {
            self.projection_cache_task = None;
            self.projection_cache_scheduled_revision = None;
        }
        if target == ViewMode::Split {
            self.enter_split_view(cx);
        } else if self.view_mode == ViewMode::Split {
            self.exit_split_view(target, cx);
        } else {
            match (self.view_mode, target) {
                (ViewMode::Source, ViewMode::Rendered | ViewMode::Preview) => {
                    self.rebuild_primary_projection_from_source(cx);
                }
                (ViewMode::Rendered | ViewMode::Preview, ViewMode::Source) => {
                    // 切换视图不能触发源码规范化；Source 视图直接读取 Rope 真值。
                    let markdown = self.source_document.text();
                    let block = Self::new_block(cx, BlockRecord::paragraph(markdown));
                    let language = if self.is_svg_document() {
                        Some("html")
                    } else {
                        self.document_kind.source_syntax_language()
                    };
                    block.update(cx, move |block, _cx| {
                        block.set_source_document_mode_with_language(language)
                    });
                    self.document.replace_roots(vec![block], cx);
                    self.table_cells.clear();
                    self.virtual_surface = None;
                }
                (ViewMode::Rendered, ViewMode::Preview)
                | (ViewMode::Preview, ViewMode::Rendered) => {}
                _ => return,
            }
        }

        self.view_mode = target;
        self.render_row_cache = None;
        self.set_projection_read_only(target == ViewMode::Preview, cx);

        if target == ViewMode::Preview {
            self.last_selection_snapshot = selection_snapshot;
            self.pending_focus = None;
            self.active_entity_id = None;
        } else {
            self.apply_selection_snapshot_in_current_mode(&selection_snapshot, cx);
        }
        self.pending_scroll_active_block_into_view = true;
        self.pending_scroll_recheck_after_layout = true;
        self.last_scroll_viewport_size = None;
        self.pending_window_title_refresh = true;
        self.close_dialog_restore_focus = None;
        self.table_axis_preview = None;
        self.table_axis_selection = None;
        self.dismiss_contextual_overlays(cx);
        self.sync_table_axis_visuals(cx);
        self.refresh_stable_document_snapshot(cx);
        cx.notify();
    }

    /// 将当前投影及原生表格单元格统一切换为只读或可编辑状态。
    pub(super) fn set_projection_read_only(&mut self, read_only: bool, cx: &mut Context<Self>) {
        Self::set_document_read_only(&self.document, &self.table_cells, read_only, cx);
    }

    fn set_document_read_only(
        document: &DocumentTree,
        table_cells: &HashMap<EntityId, TableCellBinding>,
        read_only: bool,
        cx: &mut Context<Self>,
    ) {
        let mut blocks: Vec<Entity<Block>> = document
            .visible_blocks()
            .iter()
            .map(|visible| visible.entity.clone())
            .collect();
        blocks.extend(table_cells.values().map(|binding| binding.cell.clone()));
        blocks.sort_by_key(Entity::entity_id);
        blocks.dedup_by_key(|block| block.entity_id());
        for block in blocks {
            block.update(cx, move |block, cx| {
                block.set_read_only(read_only || block.record.is_yaml_frontmatter());
                cx.notify();
            });
        }
    }
}

#[path = "window_state_parts/controller.rs"]
mod controller;
#[path = "window_state_parts/projection.rs"]
mod projection;
