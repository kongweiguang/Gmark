// @author kongweiguang

//! Delayed persistence for the shared Paged Source document.

use std::time::Duration;

use gpui::*;

use crate::config::AutoSavePreference;
use crate::preferences::EditorSettings;

use super::coordinator::RecoveredAutoSaveGate;
use super::*;

const AUTO_SAVE_IDLE_DELAY: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, PartialEq, Eq)]
enum AutoSaveDecision {
    Disabled,
    Clean,
    Wait,
    Ready,
}

impl DocumentHost {
    /// 渲染准备会频繁重复；只有正文修订变化才重置期限，避免选择与滚动延迟写盘。
    pub(crate) fn schedule_auto_save(&mut self, window: &Window, cx: &mut Context<Self>) {
        let Some(document) = self.document.as_ref().cloned() else {
            self.coordinator.auto_save.reset();
            return;
        };
        if !document.dirty() {
            self.coordinator.auto_save.reset();
            return;
        }

        let revision = document.revision_doc();
        self.coordinator.auto_save.prepare_revision(revision);
        match self.auto_save_decision(&document, cx) {
            AutoSaveDecision::Disabled => {
                self.cancel_auto_save_deadline();
            }
            AutoSaveDecision::Clean => {
                self.coordinator.auto_save.reset();
            }
            AutoSaveDecision::Wait => {
                self.cancel_auto_save_deadline();
                self.coordinator.auto_save.waiting_revision = Some(revision);
            }
            AutoSaveDecision::Ready => {
                let auto_save = &mut self.coordinator.auto_save;
                auto_save.waiting_revision = None;
                if auto_save.attempted_revision == Some(revision)
                    || auto_save.scheduled_revision == Some(revision)
                {
                    return;
                }
                auto_save.generation = auto_save.generation.wrapping_add(1);
                let generation = auto_save.generation;
                auto_save.scheduled_revision = Some(revision);
                let window_handle = window.window_handle();
                auto_save.task = Some(cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(AUTO_SAVE_IDLE_DELAY).await;
                    let _ = this.update(cx, |view, cx| {
                        let auto_save = &mut view.coordinator.auto_save;
                        if auto_save.generation != generation
                            || auto_save.scheduled_revision != Some(revision)
                        {
                            return;
                        }
                        auto_save.task = None;
                        auto_save.scheduled_revision = None;

                        let Some(document) = view.document.as_ref().cloned() else {
                            view.coordinator.auto_save.reset();
                            return;
                        };
                        if document.revision_doc() != revision {
                            return;
                        }
                        match view.auto_save_decision(&document, cx) {
                            AutoSaveDecision::Disabled => (),
                            AutoSaveDecision::Clean => {
                                view.coordinator.auto_save.reset();
                            }
                            AutoSaveDecision::Wait => {
                                view.coordinator.auto_save.waiting_revision = Some(revision);
                            }
                            AutoSaveDecision::Ready => {
                                let auto_save = &mut view.coordinator.auto_save;
                                if auto_save.attempted_revision == Some(revision) {
                                    return;
                                }
                                // 同一修订自动保存失败后等待新编辑或显式保存，避免错误状态触发逐帧重试。
                                auto_save.attempted_revision = Some(revision);
                                let path = view.path.clone();
                                if crate::source_tools::format_on_save_for_file(
                                    &path,
                                    EditorSettings::format_on_save(cx),
                                ) && view.probe.strategy != OpenStrategy::Paged
                                {
                                    view.start_format_before_save(window_handle, cx);
                                } else {
                                    view.start_save(path, false, window_handle, cx);
                                }
                            }
                        }
                    });
                }));
            }
        }
    }

    /// 暂态输入结束后重新等待空闲间隔，后台导航期间尚未发布的确认文字不能被自动保存遗漏。
    fn auto_save_decision(&self, document: &SharedDocument, cx: &App) -> AutoSaveDecision {
        if !document.dirty() {
            return AutoSaveDecision::Clean;
        }
        if EditorSettings::auto_save(cx) != AutoSavePreference::AfterDelay
            || self.path.as_os_str().is_empty()
            || self.coordinator.external_monitor_paused
            || RecoveredAutoSaveGate::blocks_auto_save(document)
        {
            return AutoSaveDecision::Disabled;
        }

        let save_in_flight = document
            .handle()
            .save_in_flight_revision()
            .map_or(true, |revision| revision.is_some());
        if self.saving
            || self.reloading
            || self.closed_suspended
            || !self.coordinator.save.pending_requests.is_empty()
            || self.has_active_ime_composition(cx)
            || self.has_pending_source_input()
            || save_in_flight
        {
            AutoSaveDecision::Wait
        } else {
            AutoSaveDecision::Ready
        }
    }

    /// 只取消活动计时，避免暂态阻塞期间每帧推进任务代次。
    fn cancel_auto_save_deadline(&mut self) {
        if self.coordinator.auto_save.scheduled_revision.is_some() {
            self.coordinator.auto_save.cancel_task();
        }
    }
}
