// @author kongweiguang

//! 分页输入属于挂载视图；复用既有延迟意图，生命周期等待与原生 IME 完成请求保持分离。

use super::*;

impl Editor {
    /// 首次拆分尚无 Pane ID，先保留标签身份；已有工作区则保留原窗格，防止迁移抢走待提交输入。
    pub(in crate::editor) fn defer_pane_split_for_input(
        &mut self,
        direction: panes::PaneSplitDirection,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.has_active_ime_composition(cx) && !self.has_pending_source_input_in_window(cx) {
            return false;
        }
        let operation = if let Some(workspace) = self.pane_workspace.as_ref() {
            DeferredImeOperation::Pane(panes::PaneEvent::Split {
                pane: workspace.read(cx).workspace().focused_pane(),
                direction,
            })
        } else {
            DeferredImeOperation::FirstSplit {
                tab: self
                    .tabs
                    .records
                    .get(self.tabs.active)
                    .map(|record| record.id),
                direction,
            }
        };
        self.queue_ime_operation(operation, cx);
        true
    }

    /// Quit 协调器已在等待本窗口时续跑原请求，不能重新 begin 导致退出意图永久停留。
    pub(super) fn resume_quit_after_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if crate::app_menu::QuitCoordinator::is_pending(cx) {
            crate::app_menu::continue_pending_quit(cx);
        } else {
            self.on_quit_application(&crate::ui::actions::QuitApplication, window, cx);
        }
    }

    /// 未提交的常驻文字与分页等待共享退出门禁；根标签与后台窗格不能只检查当前焦点。
    pub(in crate::editor) fn has_pending_source_input_in_window(&self, cx: &App) -> bool {
        if self.document.source_commit_error().is_some()
            || self
                .document_host
                .as_ref()
                .is_some_and(|host| host.read(cx).has_pending_source_input())
            || self.tabs.records.iter().any(|record| {
                record.snapshot.as_ref().is_some_and(|snapshot| {
                    snapshot
                        .document_host
                        .as_ref()
                        .is_some_and(|host| host.read(cx).has_pending_source_input())
                })
            })
        {
            return true;
        }
        self.pane_canvas_entities
            .borrow()
            .values()
            .any(|(_, _, canvas)| match canvas {
                panes::PaneCanvasEntity::DocumentHost(canvas) => {
                    canvas.read(cx).host().read(cx).has_pending_source_input()
                }
                panes::PaneCanvasEntity::Markdown(canvas) => canvas
                    .read(cx)
                    .editor()
                    .read(cx)
                    .has_pending_source_input_in_window(cx),
                panes::PaneCanvasEntity::ReadOnly(_) => false,
            })
    }

    /// 标签关闭既检查拆卸目标，也等待当前输入，避免关闭邻居时抢焦点令旧定位请求失效。
    pub(in crate::editor) fn has_pending_source_input_for_tab(
        &self,
        index: usize,
        cx: &App,
    ) -> bool {
        self.has_pending_source_input_for_active_target(cx)
            || self.tabs.records.get(index).is_some_and(|record| {
                record.snapshot.as_ref().is_some_and(|snapshot| {
                    snapshot
                        .document_host
                        .as_ref()
                        .is_some_and(|host| host.read(cx).has_pending_source_input())
                })
            })
    }

    /// 模式与标签切换不能丢弃未提交的常驻文字；结构性窗格操作另查将拆卸的全部实体。
    pub(in crate::editor) fn has_pending_source_input_for_active_target(&self, cx: &App) -> bool {
        if self.document.source_commit_error().is_some()
            || self
                .document_host
                .as_ref()
                .is_some_and(|host| host.read(cx).has_pending_source_input())
        {
            return true;
        }
        if self.pane_canvas {
            return false;
        }
        let (editor, host) = self.focused_pane_entities(cx);
        host.is_some_and(|host| host.read(cx).has_pending_source_input())
            || editor.is_some_and(|editor| {
                editor
                    .read(cx)
                    .has_pending_source_input_for_active_target(cx)
            })
    }
}
