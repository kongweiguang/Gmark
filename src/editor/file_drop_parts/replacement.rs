// @author kongweiguang

use std::path::{Path, PathBuf};

use gpui::*;

use super::{DocumentKind, Editor, ViewMode};

impl Editor {
    /// 安装打开时冻结的 session 身份，避免解码后的正文与另一轮磁盘探测结果错配。
    pub(super) fn replace_document_from_opened_markdown(
        &mut self,
        opened: crate::document_io::OpenedMarkdown,
        path: PathBuf,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<()> {
        let encoding = opened.encoding.clone();
        let source_document =
            crate::editor::document_session::EditorDocumentSession::try_new_with_open_context(
                gmark_document::SourceDocument::new(&opened.text),
                opened.loading_limits,
                opened.text_encoding,
                opened.file_identity,
            )?;
        self.replace_document_with_session(Some(path.clone()), source_document, cx);
        self.finish_opened_document_replacement(&path, encoding, cx);
        Ok(())
    }

    /// 读取前固定共享版本，只允许本次明确确认的 reload 丢弃 dirty 内容。
    pub(in crate::editor) fn reload_document_from_path_after_confirmation(
        &mut self,
        path: &Path,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<()> {
        let (expected_revision, expected_identity) =
            self.source_document.try_external_reload_baseline()?;
        let opened = crate::document_io::read_markdown_file(path)?;
        let encoding = opened.encoding.clone();
        let prepared =
            crate::editor::document_session::EditorDocumentSession::try_prepare_session_with_open_context_and_dirty(
                gmark_document::SourceDocument::new(&opened.text),
                opened.loading_limits,
                opened.text_encoding,
                opened.file_identity,
                false,
            )?;
        self.source_document
            .try_reload_prepared_document_after_confirmation(
                expected_revision,
                expected_identity,
                prepared,
            )?;
        let source_document = self.source_document.clone();
        self.replace_document_with_session(Some(path.to_path_buf()), source_document, cx);
        self.finish_opened_document_replacement(path, encoding, cx);
        Ok(())
    }

    /// 新开与原位重载使用同一编码、模式和最近文件收尾，避免两条路径逐渐分叉。
    fn finish_opened_document_replacement(
        &mut self,
        path: &Path,
        encoding: crate::document_io::DocumentEncoding,
        cx: &mut Context<Self>,
    ) {
        self.source_encoding = encoding;
        if !self.source_encoding.is_utf8() {
            self.set_view_mode(ViewMode::Preview, cx);
            self.show_encoding_conversion_dialog = true;
        }
        if !crate::document_io::is_markdown_path(path) {
            self.set_view_mode(ViewMode::Source, cx);
        }
        crate::app_menu::record_recent_file_from_editor(path, cx);
    }

    /// 无文件读取上下文的生成式替换继续使用内存 session 初始化。
    pub(in crate::editor) fn replace_document_from_markdown(
        &mut self,
        markdown: String,
        file_path: Option<PathBuf>,
        cx: &mut Context<Self>,
    ) {
        let source_document = crate::editor::document_session::EditorDocumentSession::new(
            gmark_document::SourceDocument::new(&markdown),
        );
        self.replace_document_with_session(file_path, source_document, cx);
    }

    /// 同步重置 UI 派生状态、恢复基线和 watcher，使它们都观察新安装的 session。
    fn replace_document_with_session(
        &mut self,
        file_path: Option<PathBuf>,
        source_document: crate::editor::document_session::EditorDocumentSession,
        cx: &mut Context<Self>,
    ) {
        // A document replacement invalidates every renderer-owned generation,
        // not only standalone image-preview tiles. Cancel the old document's
        // decode tasks before advancing the epoch so completions cannot retain
        // or publish payloads into the new document.
        self.release_render_assets_for_active_document(cx);
        self.document_epoch = self.document_epoch.wrapping_add(1);
        self.reset_markdown_view_state_identity(file_path.as_deref());
        self.image_preview_path = None;
        self.source_encoding = crate::document_io::DocumentEncoding::Utf8;
        self.show_encoding_conversion_dialog = false;
        self.saved_file_fingerprint = file_path
            .as_deref()
            .and_then(|path| crate::recovery::fingerprint_file(path).ok());
        self.external_file_conflict = false;
        self.recovered_session = false;
        self.show_external_conflict_dialog = false;
        self.external_conflict_preview = None;
        self.external_conflict_restore_focus = None;
        self.allow_external_overwrite_once = false;
        self.document_kind = file_path
            .as_deref()
            .map(DocumentKind::from_path)
            .unwrap_or(DocumentKind::Markdown);
        self.file_path = file_path;
        self.image_preview_zoom = 1.0;
        self.view_mode = ViewMode::Rendered;
        self.split_preview = None;
        self.projection_cache_task = None;
        self.projection_cache_scheduled_revision = None;
        self.split_projection_task = None;
        self.split_projection_scheduled_revision = None;
        self.source_document = source_document;
        self.projection_cache = None;
        self.table_cells.clear();
        self.rebuild_primary_projection_from_source(cx);

        self.document_dirty = false;
        self.pending_window_edited = false;
        self.pending_window_title_refresh = true;
        self.pending_save = false;
        self.pending_save_as = false;
        self.pending_resource_insertion = None;
        self.save_task = None;
        self.save_queued = false;
        self.auto_save_task = None;
        self.pending_open_link = None;
        self.pending_close_after_save = false;
        self.close_dialog_restore_focus = None;
        self.show_unsaved_changes_dialog = false;
        self.clear_pending_drop_replace_state(cx);
        self.dismiss_contextual_overlays(cx);
        self.close_menu_bar(cx);
        self.table_axis_preview = None;
        self.table_axis_selection = None;
        self.sync_table_axis_visuals(cx);
        self.clear_cross_block_selection(cx);

        self.pending_scroll_active_block_into_view = true;
        self.pending_scroll_recheck_after_layout = true;
        self.last_scroll_viewport_size = None;
        self.scroll_handle.set_offset(point(px(0.0), px(0.0)));
        self.pending_focus = self.first_focusable_entity_id(cx);
        self.active_entity_id = self.pending_focus;

        self.undo_history.clear();
        self.redo_history.clear();
        self.pending_undo_capture = None;
        self.last_selection_snapshot = Self::empty_selection_snapshot();
        self.history_restore_in_progress = false;
        self.checkpoint_recovery_journal();
        self.refresh_stable_document_snapshot(cx);
        self.sync_workspace_after_document_path_change(cx);
        self.restart_file_watcher(cx);
        self.apply_pending_workspace_navigation(cx);
        cx.notify();
    }
}
