// @author kongweiguang

//! 根 Source Host 与隐藏标签保留真实视图租约，关闭策略不能读取空 Markdown 适配器。

use super::*;

impl Editor {
    /// 两类关闭提示共用焦点句柄，但按各自状态选择目标；只在实际焦点偏离时纠正，避免渲染反复抢焦点。
    pub(in crate::editor) fn sync_close_dialog_focus(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.show_unsaved_changes_dialog {
            let focus = &self.close_dialog_focus_handles[self.close_dialog_keyboard_index];
            if !focus.is_focused(window) {
                focus.focus(window);
            }
        } else if self.tabs.is_close_dialog_open() {
            let has_dialog_focus = self
                .close_dialog_focus_handles
                .iter()
                .any(|focus| focus.is_focused(window));
            if !has_dialog_focus && self.close_dialog_restore_input_focus.is_none() {
                self.close_dialog_restore_input_focus = window.focused(cx);
                self.close_dialog_keyboard_index = 2;
            }
            let focus = &self.close_dialog_focus_handles[self.close_dialog_keyboard_index];
            if !focus.is_focused(window) {
                focus.focus(window);
            }
        } else if let Some(focus) = self.close_dialog_restore_input_focus.take() {
            focus.focus(window);
        }
    }

    /// 只收集实际持有租约的根 Host；工作区挂载后由 pane 清单负责，避免重复计数。
    pub(super) fn root_host_close_states(&self, cx: &App) -> Vec<EditorDocumentCloseState> {
        if self.pane_workspace.is_some() {
            return Vec::new();
        }
        std::iter::once(self.document_host.as_ref())
            .chain(self.tabs.records.iter().map(|record| {
                record
                    .snapshot
                    .as_ref()
                    .and_then(|snapshot| snapshot.document_host.as_ref())
            }))
            .flatten()
            .filter_map(|host| {
                let host = host.read(cx);
                let document_id = host.document_id()?;
                Some(EditorDocumentCloseState {
                    document_id,
                    dirty: host.is_dirty(),
                    global_lease_count: host.lease_count(),
                    window_lease_count: 1,
                })
            })
            .collect()
    }

    /// 已安装 Host 的身份失败时保留未知状态，不退回与正文无关的兼容 session。
    pub(super) fn active_close_document_id(&self, cx: &App) -> Option<DocumentId> {
        if self.pane_workspace.is_some() {
            return self.focused_pane_document_id(cx);
        }
        if let Some(host) = self.document_host.as_ref() {
            return host.read(cx).document_id();
        }
        self.source_document.document_id().ok()
    }

    /// 同一清单服务关窗与退出：关窗只拦截最后租约，退出则由协调器去重全部 dirty 文档。
    pub(super) fn inactive_root_close_target(&self, quitting: bool, cx: &App) -> Option<usize> {
        let states = self.document_close_states(cx);
        self.tabs
            .records
            .iter()
            .enumerate()
            .find_map(|(index, record)| {
                if index == self.tabs.active {
                    return None;
                }
                let snapshot = record.snapshot.as_ref()?;
                let document_id = if let Some(host) = snapshot.document_host.as_ref() {
                    host.read(cx).document_id()
                } else {
                    snapshot.source_document.document_id().ok()
                };
                let Some(document_id) = document_id else {
                    return Some(index);
                };
                states
                    .iter()
                    .find(|state| state.document_id == document_id)
                    .is_none_or(|state| {
                        state.dirty
                            && if quitting {
                                !crate::app_menu::QuitCoordinator::is_document_handled(
                                    cx,
                                    document_id,
                                )
                            } else {
                                state.closes_last_lease()
                            }
                    })
                    .then_some(index)
            })
    }
}
