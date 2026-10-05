// @author kongweiguang

//! Closed-tab suspension and resumption.

use super::*;

impl DocumentHost {
    /// 关闭的标签会暂存在 reopen 栈中，所以不能依赖 Entity Drop 释放任务。
    /// 视图任务取消并推进代次；保存写盘继续到明确终态后释放共享门禁，不能随 UI future 丢弃。
    pub(crate) fn suspend_for_closed_tab(&mut self) {
        if self.closed_suspended {
            return;
        }
        self.closed_suspended = true;
        self.saving = false;
        self.reloading = false;
        self.document_epoch = self.document_epoch.wrapping_add(1);

        // Keep the lease alive for pane reactivation, but close the
        // controller view while this Entity is inactive.
        if let Some(document) = self.document.as_ref() {
            let _ = document.close_view();
        }
        self.controller_events = None;
        self.controller_event_task = Task::ready(());

        self.coordinator.lifetime_cancellation.cancel();
        self.coordinator.external_generation = self.coordinator.external_generation.wrapping_add(1);
        self.coordinator.external_task = Task::ready(());

        self.coordinator.index_generation = self.coordinator.index_generation.wrapping_add(1);
        if let Some(cancellation) = self.coordinator.index_cancellation.take() {
            cancellation.cancel();
        }
        self.coordinator.index_task = Task::ready(());

        self.invalidate_source_rows();
        if let Some(cancellation) = self.fold_cancellation.take() {
            cancellation.cancel();
        }
        self.fold_generation = self.fold_generation.wrapping_add(1);
        self.fold_task = Task::ready(());
        self.cancel_source_formatting();
        self.invalidate_structured_runtime();
        self.derived_projection_generation = self.derived_projection_generation.wrapping_add(1);
        if let Some(cancellation) = self.derived_projection_cancellation.take() {
            cancellation.cancel();
        }
        self.derived_projection_task = Task::ready(());

        self.coordinator.search_generation = self.coordinator.search_generation.wrapping_add(1);
        if let Some(cancellation) = self.coordinator.search_cancellation.take() {
            cancellation.cancel();
        }
        self.search_running = false;
        self.coordinator.search_task = Task::ready(());

        self.coordinator.save.generation = self.coordinator.save.generation.wrapping_add(1);
        if let Some(cancellation) = self.coordinator.save.cancellation.take() {
            cancellation.cancel();
        }
        std::mem::replace(&mut self.coordinator.save.task, Task::ready(())).detach();
        self.coordinator.save.pending_requests.clear();
        self.coordinator.auto_save.reset();

        self.source_drag_anchor = None;
        self.source_drag_autoscroll_direction = 0;
        self.source_drag_autoscroll_task = Task::ready(());
        self.cancel_selection_transfers();

        self.structured_filter_task = Task::ready(());
        if let Some(cancellation) = self.structured_filter_cancellation.take() {
            cancellation.cancel();
        }
        self.json_expand_task = Task::ready(());
        self.clipboard_generation = self.clipboard_generation.wrapping_add(1);
        if let Some(cancellation) = self.clipboard_cancellation.take() {
            cancellation.cancel();
        }
        self.clipboard_task = Task::ready(());
        self.selection_export_generation = self.selection_export_generation.wrapping_add(1);
        if let Some(cancellation) = self.selection_export_cancellation.take() {
            cancellation.cancel();
        }
        self.selection_export_task = Task::ready(());
    }

    /// 只恢复关闭标签时被挂起的任务。普通标签切换仍保留既有实体，不会重复启动 monitor。
    pub(crate) fn resume_after_closed_tab(&mut self, cx: &mut Context<Self>) {
        if !self.closed_suspended {
            return;
        }
        self.closed_suspended = false;
        self.coordinator.lifetime_cancellation = SearchCancellation::default();
        if let Some(document) = self.document.as_ref()
            && let Err(error) = document.register_view()
        {
            self.error = Some(error.to_string().into());
            return;
        }
        if self.document.is_none() {
            self.start_initial_index(cx);
        } else if self.structured_index.is_none() && !document_dirty_state(&self.document) {
            self.rebuild_clean_structured_index(cx);
        }
        if !self.external_monitor_owned {
            self.start_controller_event_subscription(cx);
        }
        if self.view_mode != DocumentHostViewMode::Source {
            self.request_registered_projection(cx);
        }
        self.start_external_monitor(cx);
        if !self.search_input.read(cx).display_text().is_empty() {
            self.schedule_search(cx);
        }
        if !self
            .structured_filter_input
            .read(cx)
            .display_text()
            .is_empty()
        {
            self.schedule_structured_filter(cx);
        }
        cx.notify();
    }
}
