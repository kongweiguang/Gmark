// @author kongweiguang

use super::*;
use crate::ui::motion::MotionTokens;
use crate::ui::visual_preferences::VisualPreferencesManager;

impl Editor {
    pub(in crate::editor) fn workspace_search_active(&self) -> bool {
        self.workspace.active_tab == WorkspaceTab::Search
    }

    pub(in crate::editor) fn workspace_docked_open_preference(&self) -> bool {
        self.workspace.docked_open_preference.unwrap_or(true)
    }

    pub(in crate::editor) fn restore_workspace_docked_open_preference(
        &mut self,
        open: Option<bool>,
    ) {
        let Some(open) = open else {
            return;
        };
        self.workspace.docked_open_preference = Some(open);
        if self.workspace.compact_layout != Some(true) {
            self.workspace.is_open = open;
        }
    }

    pub(in crate::editor) fn document_sidebar_docked_open_preference(&self) -> bool {
        self.workspace
            .document_sidebar_docked_open_preference
            .unwrap_or(false)
    }

    pub(in crate::editor) fn restore_document_sidebar_docked_open_preference(
        &mut self,
        open: Option<bool>,
    ) {
        let Some(open) = open else {
            return;
        };
        self.workspace.document_sidebar_docked_open_preference = Some(open);
        if self.workspace.document_sidebar_compact_layout != Some(true) {
            self.workspace.document_sidebar_open = open;
        }
    }

    pub(in crate::editor) fn sync_document_sidebar_visibility_for_viewport(
        &mut self,
        viewport_width: f32,
    ) {
        let compact = workspace_uses_overlay(viewport_width);
        if self.workspace.document_sidebar_compact_layout == Some(compact) {
            return;
        }
        self.workspace.document_sidebar_compact_layout = Some(compact);
        self.workspace.document_sidebar_open = if compact {
            false
        } else {
            self.document_sidebar_docked_open_preference()
        };
    }

    pub(in crate::editor) fn document_sidebar_panel_width(&self) -> Option<f32> {
        self.workspace.document_sidebar_panel_width
    }

    pub(in crate::editor) fn restore_document_sidebar_panel_width(&mut self, width: Option<f32>) {
        self.workspace.document_sidebar_panel_width =
            width.filter(|width| width.is_finite()).map(|width| {
                width.clamp(
                    DOCUMENT_SIDEBAR_PANEL_MIN_WIDTH,
                    DOCUMENT_SIDEBAR_PANEL_MAX_WIDTH,
                )
            });
        self.workspace.document_sidebar_resize_session = None;
    }

    pub(super) fn start_document_sidebar_resize(
        &mut self,
        pointer_x: Pixels,
        panel_width: f32,
        cx: &mut Context<Self>,
    ) {
        self.workspace.document_sidebar_resize_session = Some(WorkspaceResizeSession {
            start_x: pointer_x,
            start_width: panel_width,
            direction: -1.0,
        });
        cx.notify();
    }

    pub(super) fn ensure_document_sidebar_resize_focus_handle(
        &mut self,
        cx: &mut Context<Self>,
    ) -> FocusHandle {
        self.workspace
            .document_sidebar_resize_focus_handle
            .get_or_insert_with(|| cx.focus_handle())
            .clone()
    }

    pub(in crate::editor) fn ensure_document_sidebar_focus_handle(
        &mut self,
        cx: &mut Context<Self>,
    ) -> FocusHandle {
        self.workspace
            .document_sidebar_focus_handle
            .get_or_insert_with(|| cx.focus_handle())
            .clone()
    }

    pub(super) fn on_document_sidebar_resize_key_down(
        &mut self,
        event: &KeyDownEvent,
        rendered_width: f32,
        cx: &mut Context<Self>,
    ) {
        let step = if event.keystroke.modifiers.shift {
            WORKSPACE_RESIZE_KEYBOARD_LARGE_STEP
        } else {
            WORKSPACE_RESIZE_KEYBOARD_STEP
        };
        let current = self
            .workspace
            .document_sidebar_panel_width
            .unwrap_or(rendered_width);
        let next = match event.keystroke.key.as_str() {
            "left" => Some((current + step).clamp(
                DOCUMENT_SIDEBAR_PANEL_MIN_WIDTH,
                DOCUMENT_SIDEBAR_PANEL_MAX_WIDTH,
            )),
            "right" => Some((current - step).clamp(
                DOCUMENT_SIDEBAR_PANEL_MIN_WIDTH,
                DOCUMENT_SIDEBAR_PANEL_MAX_WIDTH,
            )),
            "home" => Some(DOCUMENT_SIDEBAR_PANEL_MIN_WIDTH),
            "end" => Some(DOCUMENT_SIDEBAR_PANEL_MAX_WIDTH),
            "enter" => None,
            _ => return,
        };
        if self.workspace.document_sidebar_panel_width != next {
            self.workspace.document_sidebar_panel_width = next;
            self.workspace.document_sidebar_resize_session = None;
            self.schedule_workspace_session_save(cx);
            cx.notify();
        }
        cx.stop_propagation();
    }

    pub(in crate::editor) fn on_document_sidebar_resize_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(session) = self.workspace.document_sidebar_resize_session else {
            return false;
        };
        let width = (session.start_width
            + f32::from(event.position.x - session.start_x) * session.direction)
            .clamp(
                DOCUMENT_SIDEBAR_PANEL_MIN_WIDTH,
                DOCUMENT_SIDEBAR_PANEL_MAX_WIDTH,
            );
        if self.workspace.document_sidebar_panel_width != Some(width) {
            self.workspace.document_sidebar_panel_width = Some(width);
            cx.notify();
        }
        cx.stop_propagation();
        true
    }

    pub(in crate::editor) fn on_document_sidebar_resize_mouse_up(
        &mut self,
        cx: &mut Context<Self>,
    ) -> bool {
        if self
            .workspace
            .document_sidebar_resize_session
            .take()
            .is_some()
        {
            self.schedule_workspace_session_save(cx);
            cx.notify();
            cx.stop_propagation();
            return true;
        }
        false
    }

    pub(in crate::editor) fn sync_workspace_visibility_for_viewport(
        &mut self,
        viewport_width: f32,
    ) {
        let compact = workspace_uses_overlay(viewport_width);
        if self.workspace.compact_layout == Some(compact) {
            return;
        }

        self.workspace.compact_layout = Some(compact);
        self.workspace.is_open = if compact {
            false
        } else {
            self.workspace_docked_open_preference()
        };
    }

    pub(in crate::editor) fn workspace_panel_width(&self) -> Option<f32> {
        self.workspace.panel_width
    }

    pub(in crate::editor) fn restore_workspace_panel_width(&mut self, width: Option<f32>) {
        self.workspace.panel_width = width
            .filter(|width| width.is_finite())
            .map(|width| width.clamp(WORKSPACE_PANEL_MIN_WIDTH, WORKSPACE_PANEL_MAX_WIDTH));
        self.workspace.resize_session = None;
    }

    pub(super) fn start_workspace_resize(
        &mut self,
        pointer_x: Pixels,
        panel_width: f32,
        cx: &mut Context<Self>,
    ) {
        self.workspace.resize_session = Some(WorkspaceResizeSession {
            start_x: pointer_x,
            start_width: panel_width,
            direction: 1.0,
        });
        cx.notify();
    }

    pub(super) fn ensure_workspace_resize_focus_handle(
        &mut self,
        cx: &mut Context<Self>,
    ) -> FocusHandle {
        self.workspace
            .resize_focus_handle
            .get_or_insert_with(|| cx.focus_handle())
            .clone()
    }

    pub(super) fn on_workspace_resize_key_down(
        &mut self,
        event: &KeyDownEvent,
        rendered_width: f32,
        cx: &mut Context<Self>,
    ) {
        let step = if event.keystroke.modifiers.shift {
            WORKSPACE_RESIZE_KEYBOARD_LARGE_STEP
        } else {
            WORKSPACE_RESIZE_KEYBOARD_STEP
        };
        let current = self.workspace.panel_width.unwrap_or(rendered_width);
        let next = match event.keystroke.key.as_str() {
            "left" => {
                Some((current - step).clamp(WORKSPACE_PANEL_MIN_WIDTH, WORKSPACE_PANEL_MAX_WIDTH))
            }
            "right" => {
                Some((current + step).clamp(WORKSPACE_PANEL_MIN_WIDTH, WORKSPACE_PANEL_MAX_WIDTH))
            }
            "home" => Some(WORKSPACE_PANEL_MIN_WIDTH),
            "end" => Some(WORKSPACE_PANEL_MAX_WIDTH),
            // 自动宽度继续由 viewport ratio 算法决定，窗口缩放时能自然适配。
            "enter" => None,
            _ => return,
        };
        if self.workspace.panel_width != next {
            self.workspace.panel_width = next;
            self.workspace.resize_session = None;
            self.schedule_workspace_session_save(cx);
            cx.notify();
        }
        cx.stop_propagation();
    }

    pub(in crate::editor) fn on_workspace_resize_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.workspace.resize_session else {
            return;
        };
        let width = (session.start_width
            + f32::from(event.position.x - session.start_x) * session.direction)
            .clamp(WORKSPACE_PANEL_MIN_WIDTH, WORKSPACE_PANEL_MAX_WIDTH);
        if self.workspace.panel_width != Some(width) {
            self.workspace.panel_width = Some(width);
            cx.notify();
        }
        cx.stop_propagation();
    }

    pub(in crate::editor) fn on_workspace_resize_mouse_up(
        &mut self,
        _event: &MouseUpEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.workspace.resize_session.take().is_some() {
            // 拖动只更新内存；释放时保存一次，避免高频写入会话文件。
            self.schedule_workspace_session_save(cx);
            cx.notify();
            cx.stop_propagation();
        }
    }

    pub(super) fn set_workspace_tooltip_hover(
        &mut self,
        id: &'static str,
        hovered: bool,
        cx: &mut Context<Self>,
    ) {
        self.workspace.tooltip_task = None;
        self.workspace.tooltip_hovered = hovered.then_some(id);
        self.workspace.tooltip_visible = None;
        if hovered {
            let reduced_motion = cx
                .try_global::<VisualPreferencesManager>()
                .map(VisualPreferencesManager::current)
                .unwrap_or_default()
                .reduced_motion;
            let tooltip_delay = if MotionTokens::default()
                .resolved(reduced_motion)
                .feedback_enter
                .is_zero()
            {
                Duration::ZERO
            } else {
                TOOLTIP_DELAY
            };
            self.workspace.tooltip_task = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(tooltip_delay).await;
                let _ = this.update(cx, |editor, cx| {
                    if editor.workspace.tooltip_hovered == Some(id) {
                        editor.workspace.tooltip_visible = Some(id);
                        editor.workspace.tooltip_task = None;
                        cx.notify();
                    }
                });
            }));
        }
        cx.notify();
    }

    pub(in crate::editor) fn workspace_can_undo_file_operation(&self) -> bool {
        self.workspace.undo_file_operation.is_some() && self.workspace.file_operation_task.is_none()
    }

    pub(in crate::editor) fn workspace_context_target_is_root(&self) -> bool {
        matches!(
            &self.context_menu,
            Some(ContextMenuState::Workspace { path, .. })
                if self.workspace.root.as_ref() == Some(path)
        )
    }

    /// 只从扫描快照读取文件类型，避免菜单可用性判断在慢盘上同步触发 metadata。
    pub(in crate::editor) fn workspace_context_target_is_file(&self) -> bool {
        matches!(
            self.context_menu.as_ref(),
            Some(ContextMenuState::Workspace { path, .. })
                if self.workspace.snapshot_path_is_file(path)
        )
    }

    pub(crate) fn toggle_workspace_drawer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let compact = workspace_uses_overlay(f32::from(window.viewport_size().width));
        if self.workspace.is_open {
            self.workspace.is_open = false;
            self.focus_editor_after_workspace(window, cx);
        } else {
            self.close_menu_bar(cx);
            self.dismiss_contextual_overlays(cx);
            self.workspace.is_open = true;
            // The Files/Search controls live in the status bar. Reopening the
            // workspace from its sidebar button always returns to the file
            // tree, while preserving any search query and result cache.
            self.workspace.active_tab = WorkspaceTab::Files;
            self.workspace.keyboard_zone = WorkspaceKeyboardZone::Body;
            self.sync_workspace_models(cx);
            self.ensure_workspace_focus_handle(cx).focus(window);
            self.ensure_workspace_resize_focus_handle(cx);
        }
        if !compact {
            self.workspace.docked_open_preference = Some(self.workspace.is_open);
            self.schedule_workspace_session_save(cx);
        }
        cx.notify();
    }

    /// Toggle the workspace search view from the status bar. Search is a
    /// view inside the existing left drawer, so cancelling it keeps the drawer
    /// open and only switches back to the Files tree.
    pub(crate) fn toggle_workspace_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let was_closed = !self.workspace.is_open;
        let compact = workspace_uses_overlay(f32::from(window.viewport_size().width));
        if was_closed {
            self.close_menu_bar(cx);
            self.dismiss_contextual_overlays(cx);
            self.workspace.is_open = true;
            self.sync_workspace_models(cx);
            self.ensure_workspace_focus_handle(cx);
            self.ensure_workspace_resize_focus_handle(cx);
            if !compact {
                self.workspace.docked_open_preference = Some(true);
                self.schedule_workspace_session_save(cx);
            }
        }

        if !was_closed && self.workspace.active_tab == WorkspaceTab::Search {
            self.workspace.active_tab = WorkspaceTab::Files;
            self.workspace.keyboard_zone = WorkspaceKeyboardZone::Body;
            self.ensure_workspace_focus_handle(cx).focus(window);
        } else {
            self.workspace.active_tab = WorkspaceTab::Search;
            self.workspace.keyboard_zone = WorkspaceKeyboardZone::Body;
            self.sync_workspace_models(cx);
            let input = self.ensure_workspace_search_input(cx);
            self.schedule_workspace_search(cx);
            input.read(cx).focus_handle.focus(window);
        }
        cx.notify();
    }

    /// Leave workspace search without closing the workspace drawer.
    pub(super) fn cancel_workspace_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.workspace.active_tab = WorkspaceTab::Files;
        self.workspace.keyboard_zone = WorkspaceKeyboardZone::Body;
        self.ensure_workspace_focus_handle(cx).focus(window);
        cx.notify();
    }

    pub(crate) fn toggle_document_sidebar_drawer(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let compact = workspace_uses_overlay(f32::from(window.viewport_size().width));
        if self.workspace.document_sidebar_open {
            self.workspace.document_sidebar_open = false;
            self.focus_editor_after_workspace(window, cx);
        } else {
            self.close_menu_bar(cx);
            self.dismiss_contextual_overlays(cx);
            self.workspace.document_sidebar_open = true;
            self.ensure_document_sidebar_focus_handle(cx).focus(window);
        }
        if !compact {
            self.workspace.document_sidebar_docked_open_preference =
                Some(self.workspace.document_sidebar_open);
            self.schedule_workspace_session_save(cx);
        }
        cx.notify();
    }

    pub(crate) fn open_document_sidebar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.workspace.document_sidebar_open = true;
        self.workspace.document_sidebar_docked_open_preference = Some(true);
        self.sync_workspace_outline(cx);
        self.ensure_document_sidebar_focus_handle(cx).focus(window);
        self.schedule_workspace_session_save(cx);
        cx.notify();
    }

    pub(crate) fn on_toggle_document_sidebar_action(
        &mut self,
        _: &crate::components::ToggleDocumentSidebar,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_document_sidebar_drawer(window, cx);
    }

    pub(crate) fn ensure_workspace_focus_handle(&mut self, cx: &mut Context<Self>) -> FocusHandle {
        self.workspace
            .focus_handle
            .get_or_insert_with(|| cx.focus_handle())
            .clone()
    }

    /// 工作区与状态菜单复用既有活动输入实体，仅归还焦点，不改变正文或选区。
    pub(in crate::editor) fn focus_editor_after_workspace(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = self
            .active_entity_id
            .and_then(|id| self.focusable_entity_by_id(id))
            .or_else(|| {
                self.first_focusable_entity_id(cx)
                    .and_then(|id| self.focusable_entity_by_id(id))
            });
        if let Some(target) = target {
            target.read(cx).focus_handle.focus(window);
        }
    }

    pub(crate) fn on_toggle_workspace_action(
        &mut self,
        _: &crate::components::ToggleWorkspace,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_workspace_drawer(window, cx);
    }

    pub(crate) fn on_quick_open_action(
        &mut self,
        _: &crate::components::QuickOpen,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_menu_bar(cx);
        self.dismiss_contextual_overlays(cx);
        self.sync_workspace_file_tree(cx);
        let input = cx.new(|cx| {
            let mut block = Block::with_record(cx, BlockRecord::paragraph(String::new()));
            block.set_source_raw_mode();
            block
        });
        cx.subscribe(&input, Self::on_quick_open_input_event)
            .detach();
        input.read(cx).focus_handle.focus(window);
        self.workspace.quick_open = Some(QuickOpenState {
            input,
            results: Vec::new(),
            selected: 0,
            running: false,
            generation: 0,
            task: None,
        });
        self.schedule_quick_open(cx);
        cx.notify();
    }

    pub(super) fn on_quick_open_input_event(
        &mut self,
        _: Entity<Block>,
        event: &BlockEvent,
        cx: &mut Context<Self>,
    ) {
        if matches!(event, BlockEvent::Changed) {
            self.schedule_quick_open(cx);
        }
    }

    /// 只消费扫描阶段生成的平面索引，避免 Quick Open 输入时再次递归工作区树。
    pub(super) fn schedule_quick_open(&mut self, cx: &mut Context<Self>) {
        let Some(input) = self
            .workspace
            .quick_open
            .as_ref()
            .map(|state| state.input.clone())
        else {
            return;
        };
        let query = input.read(cx).display_text().trim().to_owned();
        let Some(root) = self.workspace.root.clone() else {
            if let Some(state) = self.workspace.quick_open.as_mut() {
                state.generation = state.generation.wrapping_add(1);
                state.task = None;
                state.results.clear();
                state.running = false;
            }
            cx.notify();
            return;
        };
        let paths = self.workspace.quick_open_paths.clone();
        let scanning = matches!(
            self.workspace.file_scan_state,
            WorkspaceScanState::Scanning { .. }
        );
        let Some(state) = self.workspace.quick_open.as_mut() else {
            return;
        };
        state.generation = state.generation.wrapping_add(1);
        state.task = None;
        state.running = true;
        let generation = state.generation;
        if paths.is_empty() && scanning {
            cx.notify();
            return;
        }
        state.task = Some(cx.spawn(async move |this: WeakEntity<Self>, cx| {
            cx.background_executor().timer(QUICK_OPEN_DEBOUNCE).await;
            let results = cx
                .background_spawn(async move { rank_quick_open_paths(&root, paths, &query) })
                .await;
            let _ = this.update(cx, |editor, cx| {
                let Some(state) = editor.workspace.quick_open.as_mut() else {
                    return;
                };
                if state.generation != generation {
                    return;
                }
                state.task = None;
                state.running = false;
                state.results = results;
                state.selected = 0;
                cx.notify();
                cx.refresh_windows();
            });
        }));
        cx.notify();
    }

    pub(in crate::editor) fn handle_quick_open_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(state) = self.workspace.quick_open.as_mut() else {
            return false;
        };
        match event.keystroke.key.as_str() {
            "up" => state.selected = state.selected.saturating_sub(1),
            "down" => {
                state.selected = (state.selected + 1).min(state.results.len().saturating_sub(1));
            }
            "enter" => {
                let path = state
                    .results
                    .get(state.selected)
                    .map(|result| result.path.clone());
                self.workspace.quick_open = None;
                if let Some(path) = path {
                    self.open_workspace_file(path, window, cx);
                }
            }
            "escape" => self.workspace.quick_open = None,
            _ => return false,
        }
        cx.notify();
        true
    }

    pub(in crate::editor) fn handle_workspace_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.workspace.is_open || self.workspace.operation_dialog.is_some() {
            return false;
        }
        let panel_focused = self
            .workspace
            .focus_handle
            .as_ref()
            .is_some_and(|focus| focus.is_focused(window));
        let search_focused = self
            .workspace
            .search_input
            .as_ref()
            .is_some_and(|input| input.read(cx).focus_handle.is_focused(window));
        if !panel_focused && !search_focused {
            return false;
        }

        let modifiers = event.keystroke.modifiers;
        if modifiers.control || modifiers.platform || modifiers.alt || modifiers.function {
            return false;
        }
        let key = event.keystroke.key.as_str();
        if search_focused {
            return self.handle_workspace_search_input_key(key, modifiers.shift, window, cx);
        }

        let handled = match self.workspace.keyboard_zone {
            WorkspaceKeyboardZone::Tabs => {
                self.handle_workspace_tabs_key(key, modifiers.shift, window, cx)
            }
            WorkspaceKeyboardZone::Body => self.handle_workspace_body_key(key, window, cx),
            WorkspaceKeyboardZone::SearchOptions => {
                self.handle_workspace_search_options_key(key, modifiers.shift, window, cx)
            }
            WorkspaceKeyboardZone::SearchResults => {
                self.handle_workspace_search_results_key(key, modifiers.shift, window, cx)
            }
        };
        if handled {
            cx.notify();
        }
        handled
    }

    pub(in crate::editor) fn handle_document_sidebar_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let focused = self
            .workspace
            .document_sidebar_focus_handle
            .as_ref()
            .is_some_and(|focus| focus.is_focused(window));
        if !focused || !self.workspace.document_sidebar_open {
            return false;
        }
        match event.keystroke.key.as_str() {
            "escape" => {
                if !workspace_uses_overlay(f32::from(window.viewport_size().width)) {
                    return false;
                }
                self.workspace.document_sidebar_open = false;
                self.focus_editor_after_workspace(window, cx);
                cx.notify();
                true
            }
            "tab" | "shift-tab" => false,
            _ => false,
        }
    }
}
