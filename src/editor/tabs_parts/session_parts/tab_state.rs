// @author kongweiguang

use super::*;

impl Editor {
    pub(in crate::editor) fn mark_explicit_window_close(&mut self, explicit: bool) {
        self.tabs.remove_session_after_window_close = explicit;
    }

    pub(in crate::editor) fn cancel_explicit_window_close(&mut self) {
        self.tabs.remove_session_after_window_close = false;
    }

    /// Invalidates older session writers before removing explicit-close state,
    /// so a queued save cannot recreate the session after removal.
    pub(in crate::editor) fn remove_workspace_session_for_explicit_close(
        &mut self,
        cx: &mut Context<Self>,
    ) {
        if !self.tabs.remove_session_after_window_close {
            return;
        }
        self.tabs.remove_session_after_window_close = false;
        #[cfg(test)]
        {
            let _ = cx;
        }
        #[cfg(not(test))]
        {
            let id = self.tabs.session_id;
            let generation = self.tabs.session_generation.wrapping_add(1);
            self.tabs.session_generation = generation;
            if let Err(error) = SESSION_WRITE_GENERATIONS
                .get_or_init(SessionWriteGenerationRegistry::default)
                .set(id, generation)
            {
                eprintln!("failed to record workspace session generation before removal: {error}");
                return;
            }
            cx.background_spawn(async move {
                super::persistence::run_current_session_write(
                    &SESSION_WRITE_LOCK,
                    SESSION_WRITE_GENERATIONS.get_or_init(SessionWriteGenerationRegistry::default),
                    id,
                    generation,
                    || crate::config::workspace_session::remove_workspace_session(id),
                )
                .map(|_| ())
            })
            .detach();
        }
    }

    pub(in crate::editor) fn remove_workspace_session_after_final_save(
        &mut self,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.pending_close_after_save {
            let should_remove = self.tabs.remove_session_after_window_close;
            self.remove_workspace_session_for_explicit_close(cx);
            return should_remove;
        }
        false
    }

    pub(crate) fn install_detached_tab(&mut self, detached: DetachedTab, cx: &mut Context<Self>) {
        self.install_tab_snapshot(detached.snapshot, cx);
        self.schedule_workspace_session_save(cx);
    }

    pub(crate) fn reattach_detached_tab(
        &mut self,
        detached: DetachedTab,
        cx: &mut Context<Self>,
    ) -> bool {
        self.new_tab_from_snapshot(detached.snapshot, cx)
    }

    pub(crate) fn detach_tab_by_id(
        &mut self,
        id: uuid::Uuid,
        cx: &mut Context<Self>,
    ) -> Option<DetachedTab> {
        if self.tabs.records.len() < 2 || !self.can_switch_tabs() {
            return None;
        }
        let index = self
            .tabs
            .records
            .iter()
            .position(|record| record.id == id)?;
        let snapshot = if index == self.tabs.active {
            let snapshot = self.capture_active_tab(cx);
            self.tabs.records.remove(index);
            self.tabs.active = index.min(self.tabs.records.len() - 1);
            let target = self.tabs.records[self.tabs.active]
                .snapshot
                .take()
                .expect("inactive tab must own a snapshot");
            self.install_tab_snapshot(target, cx);
            snapshot
        } else {
            self.tabs.records.remove(index).snapshot?
        };
        if index < self.tabs.active {
            self.tabs.active -= 1;
        }
        self.schedule_workspace_session_save(cx);
        cx.notify();
        Some(DetachedTab { snapshot })
    }

    /// 异步保存与路径对话框都绑定原标签，完成前不得拆卸其 Controller。
    pub(crate) fn can_switch_tabs(&self) -> bool {
        self.save_task.is_none() && self.save_prompt_task.is_none() && self.export_task.is_none()
    }

    pub(crate) fn capture_active_tab(&mut self, cx: &mut Context<Self>) -> DocumentTabSnapshot {
        // Snapshots outlive the active renderer. Release the active document's
        // GPUI assets before moving its path/source into the snapshot, so the
        // parent image key and every in-flight generation remain removable.
        self.release_render_assets_for_active_document(cx);
        if matches!(self.view_mode, ViewMode::Source | ViewMode::Split) {
            let source = self.document.raw_source_text(cx);
            self.sync_source_document_from_projection(&source);
        }
        let selection = self.capture_source_selection_snapshot(cx);

        // 文档后台任务绝不能跨标签完成后污染另一份活动状态。保存和导出在入口被阻止，
        // 其余 debounce/task 可安全取消并在目标标签恢复后重新调度。
        self.auto_save_task = None;
        self.spellcheck_task = None;
        self.projection_cache_task = None;
        self.split_projection_task = None;
        self.recovery_task = None;
        self.file_watch_task = None;
        self.file_watch_guard = None;
        self.shared_event_task = None;
        self.last_stable_source = HistorySource::empty();
        let shared_document = self.shared_document;
        self.shared_document = false;

        DocumentTabSnapshot {
            document_host: self.document_host.take(),
            source_document: mem::replace(
                &mut self.source_document,
                EditorDocumentSession::shell(),
            ),
            shared_document,
            source_encoding: mem::replace(
                &mut self.source_encoding,
                crate::document_io::DocumentEncoding::Utf8,
            ),
            document_kind: self.document_kind,
            file_path: self.file_path.take(),
            image_preview_path: self.image_preview_path.take(),
            image_preview_zoom: self.image_preview_zoom,
            file_open_failure: self.file_open_failure.take(),
            saved_file_fingerprint: self.saved_file_fingerprint.take(),
            document_dirty: self.is_document_dirty(),
            view_mode: self.view_mode,
            selection,
            scroll_offset: self.scroll_handle.offset(),
            undo_history: mem::take(&mut self.undo_history),
            redo_history: mem::take(&mut self.redo_history),
            pending_undo_capture: self.pending_undo_capture.take(),
            virtual_undo_selections: mem::take(&mut self.virtual_undo_selections),
            virtual_redo_selections: mem::take(&mut self.virtual_redo_selections),
            pending_virtual_undo_selection: self.pending_virtual_undo_selection.take(),
            recovery_journal: self.recovery_journal.take(),
            external_file_conflict: self.external_file_conflict,
            recovered_session: self.recovered_session,
            show_encoding_conversion_dialog: self.show_encoding_conversion_dialog,
            external_conflict_preview: self.external_conflict_preview.take(),
            allow_external_overwrite_once: self.allow_external_overwrite_once,
        }
    }

    pub(crate) fn install_tab_snapshot(
        &mut self,
        snapshot: DocumentTabSnapshot,
        cx: &mut Context<Self>,
    ) {
        // A dynamic format menu is derived from the active snapshot. Closing
        // its panels before replacing the snapshot prevents old actions from
        // being activated during the transition frame.
        self.close_menu_bar(cx);
        self.accessibility_revision = None;
        self.document_host = snapshot.document_host.clone();
        if let Some(document_host) = self.document_host.as_ref() {
            document_host.update(cx, |view, cx| view.resume_after_closed_tab(cx));
        }
        let source = snapshot.source_document.text();
        let mut target_mode = snapshot.view_mode;
        let target_path = snapshot.file_path.clone();
        if target_mode == ViewMode::Rendered
            && target_path
                .as_deref()
                .is_some_and(crate::document_io::is_svg_path)
        {
            target_mode = ViewMode::Preview;
        }
        // 搜索结果导航属于即将安装的目标标签。replace_document 会提前消费该请求，
        // 因此先暂存，待快照选择恢复完毕后再执行，避免目标行被旧光标覆盖。
        let pending_navigation = self.take_pending_workspace_navigation();
        self.replace_document_from_markdown(source.clone(), target_path, cx);

        let authoritative_dirty = if self.document_host.is_some() {
            snapshot.document_dirty
        } else {
            snapshot.source_document.is_dirty()
        };
        self.source_document = snapshot.source_document;
        self.shared_document = snapshot.shared_document;
        self.source_encoding = snapshot.source_encoding;
        self.document_kind = snapshot.document_kind;
        self.image_preview_path = snapshot.image_preview_path;
        self.image_preview_zoom = snapshot.image_preview_zoom;
        self.svg_preview_cache = None;
        self.file_open_failure = snapshot.file_open_failure;
        self.saved_file_fingerprint = snapshot.saved_file_fingerprint;
        self.document_dirty = authoritative_dirty;
        self.pending_window_edited = authoritative_dirty;
        self.pending_window_title_refresh = true;
        self.undo_history = snapshot.undo_history;
        self.redo_history = snapshot.redo_history;
        self.pending_undo_capture = snapshot.pending_undo_capture;
        self.virtual_undo_selections = snapshot.virtual_undo_selections;
        self.virtual_redo_selections = snapshot.virtual_redo_selections;
        self.pending_virtual_undo_selection = snapshot.pending_virtual_undo_selection;
        self.last_stable_source = HistorySource::capture(self.source_document.snapshot(), source);
        self.recovery_journal = snapshot.recovery_journal;
        self.external_file_conflict = snapshot.external_file_conflict;
        self.recovered_session = snapshot.recovered_session;
        self.show_encoding_conversion_dialog = snapshot.show_encoding_conversion_dialog;
        self.external_conflict_preview = snapshot.external_conflict_preview;
        self.allow_external_overwrite_once = snapshot.allow_external_overwrite_once;

        // replace_document 建出的 projection 属于临时 revision；重新发布目标 Rope 的 snapshot，
        // 确保虚拟 surface、Split cache 和保存 revision 使用同一个真值。
        self.projection_cache = None;
        self.rebuild_primary_projection_from_source(cx);
        if target_mode != ViewMode::Rendered {
            self.set_view_mode(target_mode, cx);
        }
        self.apply_selection_snapshot_in_current_mode(&snapshot.selection, cx);
        self.last_selection_snapshot = snapshot.selection;
        self.scroll_handle.set_offset(snapshot.scroll_offset);
        self.pending_scroll_active_block_into_view = false;
        self.pending_scroll_recheck_after_layout = false;
        self.restore_pending_workspace_navigation(pending_navigation);
        self.apply_pending_workspace_navigation(cx);
        if !self.shared_document {
            self.restart_file_watcher(cx);
        } else {
            self.start_shared_event_pump(cx);
        }
        if self.is_document_dirty() {
            self.schedule_recovery_journal(cx);
            self.schedule_auto_save(cx);
        }
        #[cfg(target_os = "macos")]
        self.schedule_platform_document_menu_refresh(cx);
        cx.notify();
    }

    /// 标签安装会拆卸输入实体；候选终态前只记录意图，不抓取带预编辑的快照。
    pub(in crate::editor) fn switch_to_tab_index(
        &mut self,
        target: usize,
        cx: &mut Context<Self>,
    ) -> bool {
        if target != self.tabs.active && self.has_active_ime_composition(cx) {
            if let Some(tab) = self.tabs.records.get(target).map(|tab| tab.id) {
                self.queue_ime_operation(
                    crate::editor::ime_lifecycle::DeferredImeOperation::SwitchTab(tab),
                    cx,
                );
            }
            return false;
        }
        if target == self.tabs.active
            || target >= self.tabs.records.len()
            || !self.can_switch_tabs()
        {
            return false;
        }
        let Some(target_snapshot) = self.tabs.records[target].snapshot.take() else {
            return false;
        };
        let current_snapshot = self.capture_active_tab(cx);
        self.tabs.records[self.tabs.active].snapshot = Some(current_snapshot);
        self.tabs.active = target;
        self.install_tab_snapshot(target_snapshot, cx);
        if self.find_panel.is_some() {
            self.schedule_find(cx);
        }
        self.schedule_workspace_session_save(cx);
        true
    }
}
