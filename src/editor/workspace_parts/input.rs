// @author kongweiguang

use super::*;
use crate::i18n::I18nManager;

impl Editor {
    pub(super) fn handle_workspace_search_input_key(
        &mut self,
        key: &str,
        shift: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        match key {
            "escape" => {
                self.cancel_workspace_search(window, cx);
            }
            "tab" => {
                if shift {
                    self.cancel_workspace_search(window, cx);
                } else {
                    self.workspace.keyboard_zone = WorkspaceKeyboardZone::SearchOptions;
                    self.ensure_workspace_focus_handle(cx).focus(window);
                }
            }
            "up" | "down" => {
                if self.workspace.search_results.is_empty() {
                    return false;
                }
                self.workspace.search_selected = if key == "up" {
                    self.workspace.search_results.len() - 1
                } else {
                    0
                };
                self.workspace.keyboard_zone = WorkspaceKeyboardZone::SearchResults;
                self.ensure_workspace_focus_handle(cx).focus(window);
                self.ensure_workspace_keyboard_item_visible(
                    self.workspace.search_selected,
                    58.0,
                    80.0,
                );
            }
            "enter" => {
                self.open_selected_workspace_search_result(window, cx);
            }
            _ => return false,
        }
        cx.notify();
        true
    }

    pub(super) fn handle_workspace_tabs_key(
        &mut self,
        key: &str,
        _shift: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        // The former workspace header is no longer rendered. If a restored
        // focus state still points at the old tab zone, continue in the
        // visible body rather than routing keyboard navigation through hidden
        // controls.
        self.workspace.keyboard_zone = WorkspaceKeyboardZone::Body;
        self.handle_workspace_body_key(key, window, cx)
    }

    pub(super) fn handle_workspace_body_key(
        &mut self,
        key: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let nodes = self.visible_workspace_keyboard_nodes();
        if nodes.is_empty() {
            return match key {
                "tab" => {
                    self.workspace.keyboard_zone = WorkspaceKeyboardZone::Body;
                    true
                }
                "escape" => {
                    self.workspace.is_open = false;
                    self.focus_editor_after_workspace(window, cx);
                    true
                }
                _ => false,
            };
        }
        let current = self.selected_workspace_node_index(&nodes).unwrap_or(0);
        match key {
            "up" => self.select_workspace_keyboard_node(&nodes[current.saturating_sub(1)]),
            "down" => self.select_workspace_keyboard_node(
                &nodes[(current + 1).min(nodes.len().saturating_sub(1))],
            ),
            "home" => self.select_workspace_keyboard_node(&nodes[0]),
            "end" => self.select_workspace_keyboard_node(&nodes[nodes.len() - 1]),
            "right" => {
                let node = &nodes[current];
                if node.has_children && !self.workspace.expanded.contains(&node.id) {
                    self.workspace.expanded.insert(node.id.clone());
                } else if let Some(child) = nodes
                    .iter()
                    .skip(current + 1)
                    .find(|candidate| candidate.parent_id.as_ref() == Some(&node.id))
                {
                    self.select_workspace_keyboard_node(child);
                }
            }
            "left" => {
                let node = &nodes[current];
                if node.has_children && self.workspace.expanded.remove(&node.id) {
                    return true;
                }
                if let Some(parent_id) = node.parent_id.as_ref()
                    && let Some(parent) = nodes.iter().find(|node| &node.id == parent_id)
                {
                    self.select_workspace_keyboard_node(parent);
                }
            }
            "enter" | "space" => {
                self.activate_workspace_keyboard_node(nodes[current].clone(), window, cx)
            }
            "tab" => self.workspace.keyboard_zone = WorkspaceKeyboardZone::Body,
            "escape" => {
                self.workspace.is_open = false;
                self.focus_editor_after_workspace(window, cx);
            }
            _ => return false,
        }
        let refreshed = self.visible_workspace_keyboard_nodes();
        if let Some(index) = self.selected_workspace_node_index(&refreshed) {
            self.ensure_workspace_keyboard_item_visible(index, WORKSPACE_NODE_HEIGHT, 0.0);
        }
        true
    }

    pub(super) fn handle_workspace_search_options_key(
        &mut self,
        key: &str,
        shift: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let selected = self.workspace.search_selected.min(2);
        match key {
            "left" | "up" => self.workspace.search_selected = selected.saturating_sub(1),
            "right" | "down" => self.workspace.search_selected = (selected + 1).min(2),
            "home" => self.workspace.search_selected = 0,
            "end" => self.workspace.search_selected = 2,
            "enter" | "space" => {
                let option: fn(&mut WorkspaceSearchOptions) -> &mut bool = match selected {
                    0 => |options| &mut options.case_sensitive,
                    1 => |options| &mut options.whole_word,
                    _ => |options| &mut options.regex,
                };
                self.toggle_workspace_search_option(option, cx);
            }
            "tab" if shift => {
                self.ensure_workspace_search_input(cx)
                    .read(cx)
                    .focus_handle
                    .focus(window);
            }
            "tab" => {
                self.workspace.keyboard_zone = if self.workspace.search_results.is_empty() {
                    WorkspaceKeyboardZone::SearchOptions
                } else {
                    self.workspace.search_selected = 0;
                    self.ensure_workspace_keyboard_item_visible(0, 58.0, 80.0);
                    WorkspaceKeyboardZone::SearchResults
                };
            }
            "escape" => self.cancel_workspace_search(window, cx),
            _ => return false,
        }
        true
    }

    pub(super) fn handle_workspace_search_results_key(
        &mut self,
        key: &str,
        shift: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let last = self.workspace.search_results.len().saturating_sub(1);
        match key {
            "up" => {
                self.workspace.search_selected = self.workspace.search_selected.saturating_sub(1)
            }
            "down" => {
                self.workspace.search_selected = (self.workspace.search_selected + 1).min(last)
            }
            "home" => self.workspace.search_selected = 0,
            "end" => self.workspace.search_selected = last,
            "enter" | "space" => self.open_selected_workspace_search_result(window, cx),
            "tab" if shift => self.workspace.keyboard_zone = WorkspaceKeyboardZone::SearchOptions,
            "tab" => self.workspace.keyboard_zone = WorkspaceKeyboardZone::SearchOptions,
            "escape" => {
                self.ensure_workspace_search_input(cx)
                    .read(cx)
                    .focus_handle
                    .focus(window);
            }
            _ => return false,
        }
        if self.workspace.keyboard_zone == WorkspaceKeyboardZone::SearchResults {
            self.ensure_workspace_keyboard_item_visible(self.workspace.search_selected, 58.0, 80.0);
        }
        true
    }

    pub(super) fn ensure_workspace_keyboard_item_visible(
        &self,
        index: usize,
        row_height: f32,
        top_padding: f32,
    ) {
        let viewport_height = f32::from(self.workspace.panel_scroll.bounds().size.height);
        if viewport_height <= 0.0 {
            return;
        }
        let current_top = -f32::from(self.workspace.panel_scroll.offset().y);
        let item_top = top_padding + index as f32 * row_height;
        let item_bottom = item_top + row_height;
        let target_top = if item_top < current_top {
            item_top
        } else if item_bottom > current_top + viewport_height {
            item_bottom - viewport_height
        } else {
            current_top
        };
        let max_offset = f32::from(self.workspace.panel_scroll.max_offset().height).max(0.0);
        self.workspace
            .panel_scroll
            .set_offset(point(px(0.0), px(-target_top.clamp(0.0, max_offset))));
    }

    pub(super) fn open_selected_workspace_search_result(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let result = self
            .workspace
            .search_results
            .get(self.workspace.search_selected)
            .cloned();
        if let Some(result) = result {
            self.open_workspace_search_result(result.path, result.line, window, cx);
        }
    }

    /// 文档路径变化只重置依赖模型；目录扫描由显式代次任务继续负责，避免同步 IO。
    pub(in crate::editor) fn sync_workspace_after_document_path_change(
        &mut self,
        cx: &mut Context<Self>,
    ) {
        self.workspace.search_generation = self.workspace.search_generation.wrapping_add(1);
        self.workspace.search_task = None;
        self.workspace.search_running = false;
        self.workspace.search_results.clear();
        self.workspace.search_error = None;
        let next_root = self.workspace_root_for_current_file();
        let scan_request_matches =
            self.workspace.file_scan_requested_root.as_ref() == next_root.as_ref();
        if self.workspace.root != next_root && !scan_request_matches {
            self.invalidate_workspace_file_tree();
        }
        self.workspace.outline_source = None;
        self.workspace.outline_revision = None;
        self.workspace.outline_requested_source = None;
        self.workspace.outline_requested_revision = None;
        self.workspace.outline_generation = self.workspace.outline_generation.wrapping_add(1);
        self.workspace.outline_task = None;
        self.workspace.outline_running = false;
        if self.workspace.is_open {
            self.sync_workspace_models(cx);
            if self.workspace.active_tab == WorkspaceTab::Search {
                self.schedule_workspace_search(cx);
            }
        }
    }

    pub(super) fn sync_workspace_models(&mut self, cx: &mut Context<Self>) {
        self.sync_workspace_file_tree(cx);
        self.sync_workspace_outline(cx);
    }

    pub(super) fn workspace_root_for_current_file(&self) -> Option<PathBuf> {
        // “打开文件”只建立文档会话；只有用户明确打开文件夹或恢复工作区时才展示目录树。
        self.workspace.explicit_root.clone()
    }

    /// 取消并隔离旧扫描结果，使后续目录请求不会被过期任务覆盖。
    pub(super) fn invalidate_workspace_file_tree(&mut self) {
        if let Some(cancelled) = self.workspace.file_scan_cancel.take() {
            cancelled.store(true, std::sync::atomic::Ordering::Release);
        }
        self.workspace.file_scan_generation = self.workspace.file_scan_generation.wrapping_add(1);
        self.workspace.file_scan_task = None;
        self.workspace.file_scanning = false;
        self.workspace.file_scan_state = WorkspaceScanState::Idle;
        self.workspace.file_scan_requested_root = None;
        self.workspace.root = None;
        self.workspace.file_tree = None;
        self.workspace.file_error = None;
        self.workspace.quick_open_paths.clear();
    }

    pub(in crate::editor) fn explicit_workspace_root(&self) -> Option<PathBuf> {
        self.workspace.explicit_root.clone()
    }

    pub(in crate::editor) fn restore_explicit_workspace_root(
        &mut self,
        root: PathBuf,
        cx: &mut Context<Self>,
    ) {
        self.set_explicit_workspace_root(root, cx);
    }

    pub(crate) fn on_open_folder_action(
        &mut self,
        _: &crate::components::OpenFolder,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let prompt_title = cx
            .global::<crate::i18n::I18nManager>()
            .strings()
            .open_workspace_folder_prompt
            .clone();
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(prompt_title.into()),
        });
        cx.spawn(async move |this: WeakEntity<Self>, cx| match prompt.await {
            Ok(Ok(Some(paths))) => {
                if let Some(path) = paths.into_iter().next() {
                    let _ = this.update(cx, |editor, cx| {
                        editor.set_explicit_workspace_root(path, cx);
                    });
                }
            }
            Ok(Err(error)) => {
                let _ = this.update(cx, |editor, cx| {
                    let error = error.to_string();
                    // The native picker may fail while an older scan is still running. Advance
                    // its generation so that late completion cannot erase this visible failure.
                    if let Some(cancelled) = editor.workspace.file_scan_cancel.take() {
                        cancelled.store(true, std::sync::atomic::Ordering::Release);
                    }
                    editor.workspace.file_scan_generation =
                        editor.workspace.file_scan_generation.wrapping_add(1);
                    let generation = editor.workspace.file_scan_generation;
                    editor.workspace.file_scan_task = None;
                    editor.workspace.file_scan_requested_root = None;
                    editor.workspace.file_error = Some(error.clone());
                    editor.workspace.file_scan_state =
                        WorkspaceScanState::Failed { generation, error };
                    editor.workspace.file_scanning = false;
                    editor.workspace.is_open = true;
                    cx.notify();
                    cx.refresh_windows();
                });
            }
            Ok(Ok(None)) | Err(_) => {}
        })
        .detach();
    }

    /// 只提交用户选择的目录请求，让后台任务完成规范化并驱动首帧 Ready/Failed。
    pub(super) fn set_explicit_workspace_root(&mut self, root: PathBuf, cx: &mut Context<Self>) {
        // 目录选择回调必须只提交请求；canonicalize 和所有磁盘遍历都放入后台，
        // 否则 Windows 原生选择器返回后会在 UI 线程同步卡住且无法及时重绘。
        self.workspace.explicit_root = Some(root);
        self.workspace.is_open = true;
        self.workspace
            .focus_handle
            .get_or_insert_with(|| cx.focus_handle());
        self.workspace
            .resize_focus_handle
            .get_or_insert_with(|| cx.focus_handle());
        self.sync_workspace_after_document_path_change(cx);
        cx.notify();
        cx.refresh_windows();
    }

    /// 为当前显式目录启动唯一代次的可取消后台扫描，防止渲染重复启动或卡住窗口。
    pub(super) fn sync_workspace_file_tree(&mut self, cx: &mut Context<Self>) {
        let next_root = self.workspace_root_for_current_file();
        let show_hidden_files = crate::config::EditorSettings::show_hidden_files(cx);
        if self.workspace.root == next_root
            && self.workspace.file_tree.is_some()
            && self.workspace.file_scan_requested_root.is_none()
            && self.workspace.file_scan_task.is_none()
        {
            // 测试/恢复路径可能已经安装快照；模型版本未变化时不能重复扫描。
            self.workspace.file_scan_requested_root = next_root.clone();
            self.workspace.file_scan_state = WorkspaceScanState::Ready {
                generation: self.workspace.file_scan_generation,
            };
            return;
        }
        if self.workspace.file_scan_requested_root.as_ref() == next_root.as_ref()
            && (matches!(
                self.workspace.file_scan_state,
                WorkspaceScanState::Scanning { .. }
            ) || (matches!(
                self.workspace.file_scan_state,
                WorkspaceScanState::Ready { .. }
            ) && self.workspace.file_tree.is_some()))
        {
            // 只有仍在运行或拥有完整树快照的请求才复用；Failed 必须允许用户对同一路径显式重试。
            return;
        }

        self.workspace.root = next_root.clone();
        self.workspace.file_tree = None;
        self.workspace.file_error = None;
        self.workspace.quick_open_paths.clear();
        if let Some(cancelled) = self.workspace.file_scan_cancel.take() {
            cancelled.store(true, std::sync::atomic::Ordering::Release);
        }
        self.workspace.file_scan_task = None;
        self.workspace.file_scan_generation = self.workspace.file_scan_generation.wrapping_add(1);
        let generation = self.workspace.file_scan_generation;

        let Some(root) = next_root else {
            self.workspace.selected = None;
            self.workspace.file_scanning = false;
            self.workspace.file_scan_requested_root = None;
            self.workspace.file_scan_state = WorkspaceScanState::Idle;
            return;
        };

        let pinned_empty_directories = self
            .workspace
            .pinned_empty_directories
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        self.workspace.file_scan_requested_root = Some(root.clone());
        self.workspace.file_scan_show_hidden_files = show_hidden_files;
        self.workspace.file_scan_cancel = Some(cancelled.clone());
        self.workspace.file_scan_state = WorkspaceScanState::Scanning { generation };
        self.workspace.file_scanning = true;
        self.workspace.file_scan_task = Some(cx.spawn(async move |this: WeakEntity<Self>, cx| {
            let scan_root = root.clone();
            let worker_cancelled = cancelled.clone();
            let result = cx
                .background_spawn(async move {
                    scan_workspace(
                        &scan_root,
                        &pinned_empty_directories,
                        show_hidden_files,
                        &worker_cancelled,
                    )
                })
                .await;
            let _ = this.update(cx, |editor, cx| {
                if editor.workspace.file_scan_generation != generation
                    || editor.workspace.file_scan_requested_root.as_ref() != Some(&root)
                {
                    return;
                }
                editor.workspace.file_scan_task = None;
                editor.workspace.file_scan_cancel = None;
                editor.workspace.file_scanning = false;
                if cancelled.load(std::sync::atomic::Ordering::Acquire) {
                    editor.workspace.file_scan_state = WorkspaceScanState::Idle;
                    // 取消也是终态，必须主动唤醒窗口，否则原生模态边界可能吞掉这次状态变化。
                    cx.notify();
                    cx.refresh_windows();
                    return;
                }
                match result {
                    Ok(scan) => {
                        editor.workspace.explicit_root = Some(scan.root.clone());
                        editor.workspace.root = Some(scan.root.clone());
                        editor.workspace.file_scan_requested_root = Some(scan.root.clone());
                        editor.workspace.expanded.insert(scan.tree.id.clone());
                        editor.workspace.file_tree = Some(scan.tree);
                        editor.workspace.quick_open_paths = scan.quick_open_paths;
                        editor.workspace.file_error = None;
                        editor.workspace.file_scan_state = WorkspaceScanState::Ready { generation };
                        editor.schedule_workspace_session_save(cx);
                        editor.workspace.selected = editor
                            .file_path
                            .as_ref()
                            .map(|path| WorkspaceSelection::File(path.clone()));
                    }
                    Err(error) => {
                        editor.workspace.file_tree = None;
                        let error = error.to_string();
                        editor.workspace.file_error = Some(error.clone());
                        editor.workspace.file_scan_state =
                            WorkspaceScanState::Failed { generation, error };
                    }
                }
                if editor.workspace.quick_open.is_some() {
                    editor.schedule_quick_open(cx);
                }
                cx.notify();
                cx.refresh_windows();
            });
        }));
        cx.notify();
        cx.refresh_windows();
    }

    pub(in crate::editor) fn sync_workspace_hidden_file_preference(
        &mut self,
        cx: &mut Context<Self>,
    ) {
        let show_hidden_files = crate::config::EditorSettings::show_hidden_files(cx);
        if self.workspace.explicit_root.is_none()
            || self.workspace.file_scan_show_hidden_files == show_hidden_files
        {
            return;
        }
        self.invalidate_workspace_file_tree();
        self.sync_workspace_file_tree(cx);
        self.workspace.search_generation = self.workspace.search_generation.wrapping_add(1);
        self.workspace.search_task = None;
        self.workspace.search_results.clear();
        if self.workspace.active_tab == WorkspaceTab::Search {
            self.schedule_workspace_search(cx);
        }
    }

    pub(in crate::editor) fn sync_workspace_outline(&mut self, cx: &mut Context<Self>) {
        let revision = (self.document_epoch, self.source_document.revision());
        if self.workspace.outline_revision == Some(revision)
            || self.workspace.outline_requested_revision == Some(revision)
        {
            return;
        }
        let source = self.serialized_document_text(cx);
        if self.workspace.outline_source.as_deref() == Some(source.as_str())
            || self.workspace.outline_requested_source.as_deref() == Some(source.as_str())
        {
            return;
        }
        self.workspace.outline_generation = self.workspace.outline_generation.wrapping_add(1);
        let generation = self.workspace.outline_generation;
        self.workspace.outline_task = None;
        self.workspace.outline_requested_source = Some(source.clone());
        self.workspace.outline_requested_revision = Some(revision);
        self.workspace.outline_running = true;
        self.workspace.outline_task = Some(cx.spawn(async move |this: WeakEntity<Self>, cx| {
            let worker_source = source.clone();
            let outline = cx
                .background_spawn(async move { build_outline_tree(&worker_source) })
                .await;
            let _ = this.update(cx, |editor, cx| {
                if editor.workspace.outline_generation != generation
                    || editor.workspace.outline_requested_revision != Some(revision)
                    || editor.workspace.outline_requested_source.as_deref() != Some(source.as_str())
                {
                    return;
                }
                editor.workspace.outline_task = None;
                editor.workspace.outline_running = false;
                editor.workspace.outline_requested_source = None;
                editor.workspace.outline_requested_revision = None;
                prune_outline_state(&mut editor.workspace, &outline);
                editor.workspace.outline_tree = outline;
                editor.workspace.outline_source = Some(source);
                editor.workspace.outline_revision = Some(revision);
                cx.notify();
                cx.refresh_windows();
            });
        }));
    }

    pub(super) fn set_workspace_tab(&mut self, tab: WorkspaceTab, cx: &mut Context<Self>) {
        if self.workspace.active_tab != tab {
            self.workspace.active_tab = tab;
            self.sync_workspace_models(cx);
            if tab == WorkspaceTab::Search {
                self.ensure_workspace_search_input(cx);
                self.schedule_workspace_search(cx);
            }
            cx.notify();
        }
    }

    pub(super) fn ensure_workspace_search_input(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Entity<Block> {
        let placeholder = cx
            .global::<crate::i18n::I18nManager>()
            .strings()
            .workspace_search_prompt
            .clone();
        if let Some(input) = self.workspace.search_input.as_ref() {
            input.update(cx, |input, _cx| {
                input.set_input_placeholder(placeholder);
            });
            return input.clone();
        }
        let input = cx.new(move |cx| {
            let mut block = Block::with_record(cx, BlockRecord::paragraph(String::new()));
            block.set_source_raw_mode();
            block.set_input_placeholder(placeholder);
            block
        });
        cx.subscribe(&input, Self::on_workspace_search_input_event)
            .detach();
        self.workspace.search_input = Some(input.clone());
        input
    }

    pub(super) fn on_workspace_search_input_event(
        &mut self,
        _input: Entity<Block>,
        event: &BlockEvent,
        cx: &mut Context<Self>,
    ) {
        if matches!(event, BlockEvent::Changed) {
            self.schedule_workspace_search(cx);
        }
    }

    pub(super) fn toggle_workspace_search_option(
        &mut self,
        option: fn(&mut WorkspaceSearchOptions) -> &mut bool,
        cx: &mut Context<Self>,
    ) {
        let value = option(&mut self.workspace.search_options);
        *value = !*value;
        self.schedule_workspace_search(cx);
    }

    pub(super) fn schedule_workspace_search(&mut self, cx: &mut Context<Self>) {
        self.workspace.search_generation = self.workspace.search_generation.wrapping_add(1);
        self.workspace.search_task = None;
        self.workspace.search_error = None;
        let query = self
            .workspace
            .search_input
            .as_ref()
            .map(|input| input.read(cx).display_text().trim().to_owned())
            .unwrap_or_default();
        let Some(root) = self.workspace.root.clone() else {
            self.workspace.search_results.clear();
            self.workspace.search_running = false;
            return;
        };
        if query.is_empty() {
            self.workspace.search_results.clear();
            self.workspace.search_selected = 0;
            self.workspace.search_running = false;
            cx.notify();
            return;
        }

        self.workspace.search_running = true;
        let generation = self.workspace.search_generation;
        let options = self.workspace.search_options;
        let show_hidden_files = self.workspace.file_scan_show_hidden_files;
        self.workspace.search_task = Some(cx.spawn(async move |this: WeakEntity<Self>, cx| {
            cx.background_executor().timer(SEARCH_DEBOUNCE).await;
            let result = cx
                .background_spawn(async move {
                    search_workspace(&root, &query, options, show_hidden_files)
                })
                .await;
            let _ = this.update(cx, |editor, cx| {
                if editor.workspace.search_generation != generation {
                    return;
                }
                editor.workspace.search_task = None;
                editor.workspace.search_running = false;
                match result {
                    Ok(results) => {
                        editor.workspace.search_results = results;
                        editor.workspace.search_selected = editor
                            .workspace
                            .search_selected
                            .min(editor.workspace.search_results.len().saturating_sub(1));
                        editor.workspace.search_error = None;
                    }
                    Err(error) => {
                        editor.workspace.search_results.clear();
                        editor.workspace.search_error = Some(error);
                    }
                }
                cx.notify();
                cx.refresh_windows();
            });
        }));
        cx.notify();
    }
}
