// @author kongweiguang

//! Document save operations.
//!
//! 所有视图都从源码优先文档写出，避免块树投影在未编辑时改写用户语法。

use std::path::{Path, PathBuf};

use gpui::*;

use super::{DocumentKind, Editor, ExternalConflictPreview};
use crate::i18n::I18nManager;
use crate::perf;

enum ExistingSaveOutcome {
    Saved {
        source: String,
        source_format: gmark_document::SourceFormatSnapshot,
        revision: gmark_document::Revision,
        identity: gmark_document_runtime::FileIdentity,
    },
    Conflict {
        revision: gmark_document::Revision,
        preview: ExternalConflictPreview,
    },
    Failed {
        revision: gmark_document::Revision,
        detail: String,
        target_may_have_changed: bool,
    },
}

fn longest_marker_run(text: &str, marker: char) -> usize {
    let mut longest = 0usize;
    let mut current = 0usize;

    for ch in text.chars() {
        if ch == marker {
            current += 1;
            longest = longest.max(current);
        } else {
            current = 0;
        }
    }

    longest
}

pub(super) fn safe_code_fence(content: &str) -> String {
    let longest_backticks = longest_marker_run(content, '`');
    if longest_backticks < 3 {
        return "```".to_string();
    }

    let longest_tildes = longest_marker_run(content, '~');
    "~".repeat(longest_tildes.max(2) + 1)
}

pub(super) fn safe_code_fence_with_info(content: &str, info: Option<&str>) -> String {
    if info.is_some_and(|info| info.contains('`')) {
        let longest_tildes = longest_marker_run(content, '~');
        return "~".repeat(longest_tildes.max(2) + 1);
    }

    safe_code_fence(content)
}

impl Editor {
    /// Reserve a Save As target in the process-wide resident registry before
    /// any bytes are written.  The registry key is based on the normalized
    /// target path (not the source snapshot identity), so a second open
    /// document cannot be silently overwritten or merged into this save.
    pub(super) fn reserve_save_as_target(
        &self,
        path: &Path,
        cx: &App,
    ) -> Result<crate::app::document_service::SaveAsTargetReservation, String> {
        let Some(service) = cx.try_global::<crate::app::document_service::DocumentService>() else {
            return Err("document registry is not initialized".to_owned());
        };
        let handle = self
            .source_document
            .handle()
            .map_err(|error| error.to_string())?;
        service
            .reserve_save_as_target(&handle, path)
            .map_err(|error| error.to_string())
    }

    pub(super) fn switch_to_shared_save_as_target(
        &mut self,
        target: crate::app::document_service::SharedSaveAsTarget,
        cx: &mut Context<Self>,
    ) -> Result<bool, String> {
        match target
            .into_existing_open()
            .map_err(|error| error.to_string())?
        {
            crate::app::document_service::SharedExistingOpen::Resident(open) => {
                let source_document =
                    super::document_session::EditorDocumentSession::from_lease(open.lease)
                        .map_err(|error| error.to_string())?;
                Ok(self.open_shared_document_tab(
                    source_document,
                    open.file_path,
                    open.encoding,
                    cx,
                ))
            }
            crate::app::document_service::SharedExistingOpen::Host(open) => {
                Ok(self.open_shared_document_host_tab(open, cx))
            }
        }
    }

    /// 派生输入未提交时拒绝旧快照保存；成功输入已在区域事务中发布，保存继续沿用共享 Controller。
    pub(super) fn prepare_background_save(
        &mut self,
        _cx: &App,
    ) -> Result<
        Option<(
            gmark_document_runtime::DocumentSaveSnapshot,
            gmark_document::SourceFormatSnapshot,
        )>,
        String,
    > {
        if let Some(error) = self.document.source_commit_error() {
            return Err(error.to_owned());
        }
        // The shared Controller is the save authority even in rendered mode.
        // A completed block edit records the exact projection in
        // `pending_dirty_source`; publish that text before taking an
        // immutable snapshot.  Format-only changes (for example an explicit
        // legacy-encoding conversion) deliberately leave this marker empty:
        // rebuilding a rendered projection would strip Markdown syntax such
        // as heading prefixes from an otherwise unchanged source document.
        if let Some(source) = self.pending_dirty_source.clone() {
            self.sync_source_document_from_projection(&source);
        }
        self.source_document
            .try_request_save_context()
            .map_err(|error| error.to_string())
    }

    pub(super) fn serialized_document_text(&self, cx: &App) -> String {
        if matches!(
            self.view_mode,
            super::ViewMode::Source | super::ViewMode::Split
        ) {
            // IME/输入事件提交前也可能触发导出，直接读取 Source 投影可避免旧快照。
            self.document.raw_source_text(cx)
        } else {
            self.source_document.text()
        }
    }

    pub(super) fn save_dialog_defaults(&self) -> (PathBuf, Option<String>) {
        if let Some(path) = self.file_path.as_ref() {
            let directory = path
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
            let suggested_name = path
                .file_name()
                .map(|name| name.to_string_lossy().to_string());
            (directory, suggested_name)
        } else {
            (
                std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
                Some(self.document_kind.untitled_name().to_owned()),
            )
        }
    }

    pub(super) fn apply_successful_save(
        &mut self,
        path: PathBuf,
        saved_source: String,
        saved_format: gmark_document::SourceFormatSnapshot,
        cx: &mut Context<Self>,
    ) {
        let path_changed = self.file_path.as_ref() != Some(&path);
        let removed_workspace_session = self.remove_workspace_session_after_final_save(cx);
        self.auto_save_task = None;
        self.saved_file_fingerprint = crate::recovery::fingerprint_file(&path).ok();
        self.external_file_conflict = false;
        self.recovered_session = false;
        self.show_external_conflict_dialog = false;
        self.external_conflict_preview = None;
        self.external_conflict_restore_focus = None;
        self.allow_external_overwrite_once = false;
        self.document_kind = DocumentKind::from_path(&path);
        self.file_path = Some(path.clone());
        if path_changed {
            let tab_id = self.tabs.active_id();
            let _ = self.view_state.rekey_tab(
                tab_id,
                super::markdown_view_state::MarkdownTabIdentity::saved(&path, tab_id),
            );
        }
        if path_changed {
            self.restart_file_watcher(cx);
        }
        self.document_dirty = false;
        self.pending_window_edited = false;
        self.pending_window_title_refresh = true;
        self.pending_close_after_save = false;
        self.close_dialog_restore_focus = None;
        self.checkpoint_recovery_journal_with_snapshot(saved_source, saved_format);
        self.sync_workspace_after_document_path_change(cx);
        if !removed_workspace_session {
            self.schedule_workspace_session_save(cx);
        }
        self.resume_pending_resource_insertion(cx);
        self.continue_window_close_after_save(cx);
        self.finish_pending_tab_close_after_save(cx);
        cx.notify();
    }

    pub(super) fn apply_background_save_success(
        &mut self,
        path: PathBuf,
        saved_source: String,
        saved_format: gmark_document::SourceFormatSnapshot,
        saved_revision: gmark_document::Revision,
        saved_document_epoch: u64,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.document_epoch != saved_document_epoch {
            return false;
        }
        if self.source_document.revision() == saved_revision
            && self.source_document.source_format() == saved_format
        {
            self.apply_successful_save(path, saved_source, saved_format, cx);
            return true;
        }

        // 后台写入期间可能继续输入。磁盘和恢复基线更新为已写快照，但当前 revision
        // 仍保持 dirty，绝不能用旧保存结果清除标题标记或关闭窗口。
        let path_changed = self.file_path.as_ref() != Some(&path);
        self.saved_file_fingerprint = crate::recovery::fingerprint_file(&path).ok();
        self.external_file_conflict = false;
        self.recovered_session = false;
        self.show_external_conflict_dialog = false;
        self.external_conflict_preview = None;
        self.external_conflict_restore_focus = None;
        self.allow_external_overwrite_once = false;
        self.document_kind = DocumentKind::from_path(&path);
        self.file_path = Some(path.clone());
        if path_changed {
            let tab_id = self.tabs.active_id();
            let _ = self.view_state.rekey_tab(
                tab_id,
                super::markdown_view_state::MarkdownTabIdentity::saved(&path, tab_id),
            );
        }
        if path_changed {
            self.restart_file_watcher(cx);
        }
        self.checkpoint_recovery_journal_with_snapshot(saved_source, saved_format);
        self.document_dirty = true;
        self.pending_window_edited = true;
        self.pending_window_title_refresh = true;
        self.schedule_recovery_journal(cx);
        self.schedule_auto_save(cx);
        self.sync_workspace_after_document_path_change(cx);
        cx.notify();
        false
    }
}

#[path = "persistence_parts/state_machine.rs"]
mod state_machine;

/// 比较 CRLF 与 LF 文本时只忽略行尾 CR，保留尾随空行语义且不复制全文。
fn build_external_conflict_preview(
    path: &Path,
    local: &str,
    disk: &str,
    disk_bytes: usize,
    disk_error: Option<String>,
) -> ExternalConflictPreview {
    let mut first_difference = None;
    if disk_error.is_none() {
        let mut local_lines = local
            .split('\n')
            .map(|line| line.strip_suffix('\r').unwrap_or(line));
        let mut disk_lines = disk
            .split('\n')
            .map(|line| line.strip_suffix('\r').unwrap_or(line));
        let mut line = 1usize;
        loop {
            match (local_lines.next(), disk_lines.next()) {
                (None, None) => break,
                (left, right) if left == right => line += 1,
                (left, right) => {
                    first_difference = Some((
                        line,
                        truncate_conflict_line(left.unwrap_or_default()),
                        truncate_conflict_line(right.unwrap_or_default()),
                    ));
                    break;
                }
            }
        }
    }
    let (first_difference_line, local_line, disk_line) = first_difference
        .map(|(line, local, disk)| (Some(line), local, disk))
        .unwrap_or_else(|| (None, String::new(), String::new()));
    ExternalConflictPreview {
        path: path.display().to_string(),
        first_difference_line,
        local_line,
        disk_line,
        local_line_count: text_line_count(local),
        disk_line_count: disk_error
            .as_ref()
            .map_or_else(|| text_line_count(disk), |_| 0),
        local_bytes: local.len(),
        disk_bytes,
        disk_error,
    }
}

fn text_line_count(text: &str) -> usize {
    if text.is_empty() {
        0
    } else {
        text.as_bytes()
            .iter()
            .filter(|byte| **byte == b'\n')
            .count()
            + 1
    }
}

fn truncate_conflict_line(line: &str) -> String {
    const MAX_CHARS: usize = 240;
    let mut chars = line.chars();
    let truncated = chars.by_ref().take(MAX_CHARS).collect::<String>();
    if chars.next().is_some() {
        format!("{truncated}...")
    } else {
        truncated
    }
}

#[cfg(test)]
#[path = "../../tests/unit/editor/persistence.rs"]
mod tests;
