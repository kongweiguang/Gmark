// @author kongweiguang

//! Existing-file auto-save scheduling. The normal Save path still owns conflict checks and writes.

use std::time::Duration;

use gpui::*;

use super::super::Editor;
use crate::config::{AutoSavePreference, EditorSettings};

impl Editor {
    const AUTO_SAVE_IDLE_DELAY: Duration = Duration::from_secs(1);

    /// 空闲计时不能强制结束候选；终态事件会为仍 dirty 的正文重新启动计时。
    pub(in crate::editor) fn schedule_auto_save(&mut self, cx: &mut Context<Self>) {
        self.auto_save_task = None;
        if EditorSettings::auto_save(cx) != AutoSavePreference::AfterDelay
            || self.file_path.is_none()
            || self.recovered_session
            || self.external_file_conflict
        {
            return;
        }

        self.auto_save_task = Some(cx.spawn(async move |this: WeakEntity<Self>, cx| {
            cx.background_executor()
                .timer(Self::AUTO_SAVE_IDLE_DELAY)
                .await;
            let _ = this.update(cx, |editor, cx| {
                editor.auto_save_task = None;
                // 计时期间 watcher、恢复或手动保存都可能改变资格，触发前必须重新校验。
                if EditorSettings::auto_save(cx) == AutoSavePreference::AfterDelay
                    && editor.is_document_dirty()
                    && editor.file_path.is_some()
                    && !editor.recovered_session
                    && !editor.external_file_conflict
                    && !editor.pending_save
                    && !editor.pending_save_as
                    && editor.save_task.is_none()
                    && !editor.has_active_ime_composition(cx)
                {
                    editor.pending_save = true;
                    cx.notify();
                }
            });
        }));
    }
}
