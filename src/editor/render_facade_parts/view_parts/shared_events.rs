// @author kongweiguang

// 把共享文档事件泵留在独立边界，确保多视图同步仍由同一入口驱动。

use super::*;

impl Editor {
    pub(in crate::editor) fn start_shared_event_pump(&mut self, cx: &mut Context<Self>) {
        if !self.shared_document || self.shared_event_task.is_some() {
            return;
        }
        self.shared_event_task = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(16))
                    .await;
                let Ok(keep_running) = this.update(cx, |editor, cx| {
                    if !editor.shared_document {
                        return false;
                    }
                    let pending = match editor.source_document.has_pending_events() {
                        Ok(pending) => pending,
                        Err(_) => return true,
                    };
                    if !pending {
                        return true;
                    }
                    let Ok(polled) = editor.source_document.poll_events() else {
                        return true;
                    };
                    if polled.events.is_empty() && polled.snapshot.is_none() {
                        return true;
                    }
                    editor.source_document.queue_events(polled);
                    cx.notify();
                    true
                }) else {
                    break;
                };
                if !keep_running {
                    break;
                }
            }
        }));
    }

    /// 未提交输入保留所属树直到剪贴板恢复；其余视图继续按共享事件更新，不写回旧投影。
    /// Drain the per-view Controller cursor before building a frame.  A
    /// sibling view can mutate the shared document without touching this
    /// Editor entity; RevisionChanged therefore rebuilds only this view's
    /// projection from the immutable adapter snapshot, never by issuing a
    /// compensating transaction.
    pub(in crate::editor) fn sync_shared_document_events(&mut self, cx: &mut Context<Self>) {
        if self.document.source_commit_error().is_some() {
            return;
        }
        let mut polled = self.source_document.take_queued_events().unwrap_or(
            crate::editor::document_session::DocumentEventPoll {
                events: Vec::new(),
                snapshot: None,
            },
        );
        let Ok(immediate) = self.source_document.poll_events() else {
            return;
        };
        polled.merge(immediate);
        let resynchronized = polled.snapshot.is_some();
        let events = polled.events;
        let foreign_revision = events.iter().any(|event| {
            matches!(event,
            gmark_document_runtime::DocumentEvent::RevisionChanged { view_id, .. }
                if *view_id != self.source_document.view_id())
        });
        let mut ime_sessions =
            if (resynchronized || foreign_revision) && self.has_active_ime_composition(cx) {
                self.capture_ime_for_shared_rebase(cx)
            } else {
                Vec::new()
            };
        if resynchronized {
            Self::invalidate_shared_ime_sessions(&mut ime_sessions);
        }
        if let Some(snapshot) = polled.snapshot {
            self.document_dirty = snapshot.dirty;
            self.pending_window_edited = snapshot.dirty;
        }
        let mut projection_revision_changed = resynchronized;
        for event in events {
            match event {
                gmark_document_runtime::DocumentEvent::RevisionChanged {
                    view_id,
                    revision,
                    dirty,
                    mutation,
                    ..
                } => {
                    self.document_dirty = dirty;
                    self.pending_window_edited = dirty;
                    let revision = Revision::from_u64(revision.0);
                    // The originating view has already applied its edit to the
                    // mounted projection before submitting the Controller
                    // transaction. Rebuilding that same view from its own event
                    // would replace the focused Entity between consecutive IME,
                    // formula, and pointer-input callbacks. Other views still
                    // rebuild on the next frame, which is the shared-document
                    // synchronization contract.
                    let originated_here = view_id == self.source_document.view_id();
                    if !originated_here {
                        Self::map_ime_shared_mutation(&mut ime_sessions, &mutation);
                    }
                    if !originated_here
                        && self
                            .projection_cache
                            .as_ref()
                            .is_none_or(|projection| projection.revision != revision)
                    {
                        projection_revision_changed = true;
                    }
                }
                gmark_document_runtime::DocumentEvent::DirtyChanged { dirty, .. } => {
                    self.document_dirty = dirty;
                    self.pending_window_edited = dirty;
                }
                gmark_document_runtime::DocumentEvent::Saved {
                    dirty, identity, ..
                } => {
                    self.document_dirty = dirty;
                    self.pending_window_edited = dirty;
                    if !identity.canonical_path.as_os_str().is_empty() {
                        self.file_path = Some(identity.canonical_path.clone());
                    }
                }
                gmark_document_runtime::DocumentEvent::IdentityChanged { identity, .. } => {
                    if !identity.canonical_path.as_os_str().is_empty() {
                        self.file_path = Some(identity.canonical_path.clone());
                    }
                }
                gmark_document_runtime::DocumentEvent::ExternalConflict { .. } => {
                    self.external_file_conflict = true;
                }
            }
        }
        if !projection_revision_changed {
            self.restore_ime_after_shared_rebase(ime_sessions, cx);
            return;
        }

        match self.view_mode {
            ViewMode::Source | ViewMode::Split => {
                if let Some(block) = self.document.first_root().cloned() {
                    let source = self.source_document.text();
                    block.update(cx, |block, cx| {
                        block
                            .record
                            .set_title(crate::components::InlineTextTree::plain(source));
                        block.sync_render_cache();
                        cx.notify();
                    });
                    self.document.rebuild_metadata_and_snapshot(cx);
                } else {
                    self.document = self.source_view_document(cx);
                }
                self.virtual_surface = None;
                self.table_cells.clear();
                if self.view_mode == ViewMode::Split {
                    self.rebuild_split_preview_projection(cx);
                }
            }
            ViewMode::Rendered | ViewMode::Preview => {
                self.rebuild_primary_projection_from_source_reusing(cx);
            }
        }
        if let Ok(selection) = self.source_document.try_source_selection() {
            let snapshot = UndoSelectionSnapshot::from_source_selection(selection);
            self.apply_selection_snapshot_in_current_mode(&snapshot, cx);
            self.last_selection_snapshot = snapshot;
        }
        self.restore_ime_after_shared_rebase(ime_sessions, cx);
        self.pending_window_title_refresh = true;
        self.refresh_stable_document_snapshot(cx);
    }
}
