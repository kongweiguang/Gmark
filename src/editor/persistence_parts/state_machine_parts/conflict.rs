// @author kongweiguang

// 把外部文件冲突状态与用户决策隔离，避免提示流程改变保存入口。

use super::*;

impl Editor {
    pub(super) fn existing_path_has_external_change(&mut self, path: &Path) -> bool {
        if std::mem::take(&mut self.allow_external_overwrite_once) {
            return false;
        }
        if self.external_file_conflict {
            return true;
        }
        let Some(expected) = self.saved_file_fingerprint.as_ref() else {
            return false;
        };
        let changed = crate::recovery::fingerprint_file(path)
            .map(|current| current != *expected)
            .unwrap_or(true);
        self.external_file_conflict = changed;
        changed
    }

    pub(in crate::editor) fn present_external_file_conflict(
        &mut self,
        path: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let local = self.serialized_document_text(cx);
        let (disk, disk_bytes, disk_error) = match std::fs::read(path) {
            Ok(bytes) => (
                String::from_utf8_lossy(&bytes).into_owned(),
                bytes.len(),
                None,
            ),
            Err(error) => (String::new(), 0, Some(error.to_string())),
        };
        self.external_conflict_preview = Some(build_external_conflict_preview(
            path, &local, &disk, disk_bytes, disk_error,
        ));
        if self.external_conflict_restore_focus.is_none()
            && self.close_dialog_restore_focus.is_none()
        {
            self.external_conflict_restore_focus =
                self.document.focused_block_entity_id(window, cx);
        }
        self.show_external_conflict_dialog = true;
        self.close_menu_bar(cx);
        self.hide_info_dialog(cx);
        window.blur();
        cx.notify();
    }

    pub(super) fn hide_external_file_conflict(&mut self, cx: &mut Context<Self>) {
        self.show_external_conflict_dialog = false;
        self.external_conflict_preview = None;
        cx.notify();
    }

    pub(crate) fn on_cancel_external_conflict(
        &mut self,
        _: &ClickEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cancel_external_conflict(cx);
    }

    pub(in crate::editor) fn cancel_external_conflict(&mut self, cx: &mut Context<Self>) {
        self.hide_external_file_conflict(cx);
        self.abort_window_close_tab_sequence(cx);
        if self.pending_close_after_save {
            self.abort_pending_close_after_save(cx);
            self.external_conflict_restore_focus = None;
            return;
        }
        if let Some(entity_id) = self.external_conflict_restore_focus.take() {
            self.pending_focus = Some(entity_id);
            self.pending_scroll_active_block_into_view = true;
        }
    }

    pub(crate) fn on_save_as_external_conflict(
        &mut self,
        _: &ClickEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.hide_external_file_conflict(cx);
        self.external_conflict_restore_focus = None;
        self.pending_save_as = true;
        cx.notify();
    }

    pub(crate) fn on_overwrite_external_conflict(
        &mut self,
        _: &ClickEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.hide_external_file_conflict(cx);
        self.external_conflict_restore_focus = None;
        self.allow_external_overwrite_once = true;
        self.pending_save = true;
        cx.notify();
    }

    /// 对用户明确选择的 reload 使用共享会话替换，避免同一文档的 peer 留在旧 Controller。
    pub(crate) fn on_reload_external_conflict(
        &mut self,
        _: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(path) = self.file_path.clone() else {
            self.hide_external_file_conflict(cx);
            self.external_conflict_restore_focus = None;
            return;
        };
        self.pending_close_after_save = false;
        self.abort_window_close_tab_sequence(cx);
        self.hide_external_file_conflict(cx);
        self.external_conflict_restore_focus = None;
        match self.reload_document_from_path_after_confirmation(&path, cx) {
            Ok(()) => window.set_window_edited(false),
            Err(error) => {
                let strings = cx.global::<I18nManager>().strings().clone();
                let buttons = [strings.info_dialog_ok.as_str()];
                let _ = window.prompt(
                    PromptLevel::Critical,
                    &strings.open_failed_title,
                    Some(&error.to_string()),
                    &buttons,
                    cx,
                );
            }
        }
    }
}
