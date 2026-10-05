// @author kongweiguang

//! 视图级保存请求等待共享 IO；后台保存队列仍只持有真正执行中的快照。

use super::coordinator::PendingHostSave;
use super::*;

impl DocumentHost {
    /// 关闭只等待保存相关工作；索引、查找和无障碍刷新不能冒充保存终态或阻止关闭。
    pub(crate) fn has_pending_save(&self) -> bool {
        self.saving
            || self.save_after_format.is_some()
            || !self.coordinator.save.pending_requests.is_empty()
    }

    /// 连续保存同一目标可折叠；不同另存为目标按用户顺序保留，不覆盖已确认的路径。
    pub(super) fn enqueue_pending_save(
        &mut self,
        path: PathBuf,
        save_as: bool,
        window: gpui::AnyWindowHandle,
        cx: &mut Context<Self>,
    ) {
        let pending = &mut self.coordinator.save.pending_requests;
        if pending.back().is_some_and(|request| {
            request.document_epoch == self.document_epoch
                && request.save_as == save_as
                && request.path == path
        }) {
            return;
        }
        pending.push_back(PendingHostSave {
            document_epoch: self.document_epoch,
            path,
            save_as,
            window,
        });
        cx.notify();
    }

    /// 已确认请求等所属输入与 IO 结束再捕获快照；渲染不替输入法确认候选。
    pub(super) fn flush_pending_save_requests(&mut self, cx: &mut Context<Self>) {
        if self.coordinator.save.pending_requests.is_empty() {
            return;
        }
        if self.closed_suspended || self.reloading {
            self.coordinator.save.pending_requests.clear();
            return;
        }
        if self.saving || self.has_active_ime_composition(cx) || self.has_pending_source_input() {
            return;
        }
        let Some(document) = self.document.as_ref() else {
            self.coordinator.save.pending_requests.clear();
            return;
        };
        let in_flight = match document.lock() {
            Ok(controller) => controller.save_in_flight_revision().is_some(),
            Err(error) => {
                self.error = Some(error.to_string().into());
                self.coordinator.save.pending_requests.clear();
                return;
            }
        };
        if in_flight {
            return;
        }
        while let Some(request) = self.coordinator.save.pending_requests.pop_front() {
            if request.document_epoch != self.document_epoch {
                self.error = Some("文档已重新载入，请重新保存。".into());
                continue;
            }
            if !request.save_as && !document_dirty_state(&self.document) {
                continue;
            }
            // 普通保存跟随本视图的另存为结果；显式另存为继续使用用户确认的独立目标。
            let path = if request.save_as {
                request.path
            } else {
                self.path.clone()
            };
            self.start_save(path, request.save_as, request.window, cx);
            break;
        }
        if !self.has_pending_save() {
            // 同修订的排队保存会被折叠；清空队列也要发布终态，关闭不能一直等不存在的第二次 IO。
            cx.emit(DocumentHostEvent::StateChanged);
        }
    }
}
