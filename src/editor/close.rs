// @author kongweiguang

//! Unsaved-changes dialog and window-close interception.
//!
//! When the document is dirty, `Editor::on_window_should_close` returns
//! false and shows an overlay offering three choices: save-and-close,
//! discard-and-close, or keep editing.  Focus is restored to the
//! previously active block when the dialog is dismissed without closing.

use std::collections::BTreeMap;

use gmark_document_runtime::DocumentId;
use gpui::*;

use super::Editor;

/// The close/quit policy consumes controller-owned identity and lease state,
/// rather than a tab snapshot's cached dirty bit.  A single Editor can expose
/// several views of one document, so the per-window count is kept alongside
/// the process-wide count for last-lease decisions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct EditorDocumentCloseState {
    pub(crate) document_id: DocumentId,
    pub(crate) dirty: bool,
    pub(crate) global_lease_count: usize,
    pub(crate) window_lease_count: usize,
}

impl EditorDocumentCloseState {
    pub(crate) fn closes_last_lease(self) -> bool {
        self.global_lease_count == self.window_lease_count
    }
}

impl Editor {
    /// Returns one authoritative state row per document in this window.  Pane
    /// tabs (Markdown and source-backed hosts) are merged with legacy tabs by
    /// DocumentId; no UI snapshot dirty flag participates in the result.
    pub(crate) fn document_close_states(&self, cx: &App) -> Vec<EditorDocumentCloseState> {
        let mut states = BTreeMap::<DocumentId, EditorDocumentCloseState>::new();

        let mut add_source = |source: &super::document_session::EditorDocumentSession| {
            let Ok(document_id) = source.document_id() else {
                return;
            };
            let global_lease_count = source.lease_count();
            let entry = states
                .entry(document_id)
                .or_insert(EditorDocumentCloseState {
                    document_id,
                    dirty: false,
                    global_lease_count,
                    window_lease_count: 0,
                });
            // A poisoned Controller cannot prove cleanliness.  Keep the
            // close path conservative instead of silently dropping a dirty
            // document from the inventory.
            entry.dirty |= source.try_is_dirty().unwrap_or(true);
            entry.global_lease_count = entry.global_lease_count.max(global_lease_count);
            entry.window_lease_count = entry.window_lease_count.saturating_add(1);
        };

        // Once the pane workspace is mounted it owns the live view leases;
        // the shell's source session is only an empty compatibility adapter.
        if self.pane_workspace.is_none() {
            for (_, source) in self.markdown_tab_sources() {
                add_source(&source);
            }
        }
        for pane in self.pane_document_close_states(cx) {
            let entry = states
                .entry(pane.document_id)
                .or_insert(EditorDocumentCloseState {
                    document_id: pane.document_id,
                    dirty: false,
                    global_lease_count: pane.global_lease_count,
                    window_lease_count: 0,
                });
            entry.dirty |= pane.dirty;
            entry.global_lease_count = entry.global_lease_count.max(pane.global_lease_count);
            entry.window_lease_count = entry
                .window_lease_count
                .saturating_add(pane.window_view_count);
        }

        states.into_values().collect()
    }

    fn focused_pane_document_id(&self, cx: &App) -> Option<DocumentId> {
        let workspace = self.pane_workspace.as_ref()?.read(cx).workspace();
        let pane = workspace.focused_pane();
        let tab = workspace.pane(pane)?.active_tab_id()?;
        Some(workspace.tab(pane, tab)?.view().document_id())
    }

    fn active_last_lease_dirty(&self, cx: &App) -> bool {
        let document_id = if let Some(document_id) = self.focused_pane_document_id(cx) {
            document_id
        } else if let Ok(document_id) = self.source_document.document_id() {
            document_id
        } else {
            // A poisoned/partially mounted view cannot prove cleanliness;
            // fail closed and retain the unsaved-changes interception.
            return true;
        };
        self.document_close_states(cx)
            .into_iter()
            .find(|state| state.document_id == document_id)
            .map_or_else(
                || self.is_document_dirty(),
                |state| {
                    if state.dirty && state.closes_last_lease() {
                        return true;
                    }
                    self.pane_workspace.as_ref().is_some_and(|_| {
                        self.document_close_states(cx)
                            .into_iter()
                            .any(|other| other.dirty && other.closes_last_lease())
                    })
                },
            )
    }

    fn activate_last_lease_dirty_for_window_close(&mut self, cx: &mut Context<Self>) -> bool {
        if self.active_last_lease_dirty(cx) {
            return true;
        }
        if self.pane_workspace.is_some() {
            return false;
        }
        let target = self
            .markdown_tab_sources()
            .into_iter()
            .find_map(|(index, source)| {
                if index == self.active_tab_index() {
                    return None;
                }
                let document_id = match source.document_id() {
                    Ok(document_id) => document_id,
                    Err(_) => {
                        return source
                            .try_is_dirty()
                            .map_or(true, |dirty| dirty)
                            .then_some(index);
                    }
                };
                self.document_close_states(cx)
                    .into_iter()
                    .find(|state| state.document_id == document_id)
                    .is_some_and(|state| state.dirty && state.closes_last_lease())
                    .then_some(index)
            });
        target.is_some_and(|index| self.switch_to_tab_index(index, cx))
    }

    fn active_quit_dirty(&self, cx: &App) -> bool {
        let Some(document_id) = self
            .focused_pane_document_id(cx)
            .or_else(|| self.source_document.document_id().ok())
        else {
            return true;
        };
        self.document_close_states(cx)
            .into_iter()
            .find(|state| state.document_id == document_id)
            .map_or_else(
                || self.is_document_dirty(),
                |state| {
                    state.dirty
                        && !crate::app_menu::QuitCoordinator::is_document_handled(cx, document_id)
                },
            )
    }

    fn mark_quit_document_pending(&self, cx: &mut App) {
        if let Some(document_id) = self
            .focused_pane_document_id(cx)
            .or_else(|| self.source_document.document_id().ok())
        {
            let _ = crate::app_menu::QuitCoordinator::mark_document_pending(cx, document_id);
        }
    }

    /// Activates the next dirty document for process quit.  Unlike explicit
    /// window close, every dirty DocumentId participates even while another
    /// window still holds a lease; the coordinator deduplicates the prompt.
    fn activate_dirty_tab_for_quit(&mut self, cx: &mut Context<Self>) -> bool {
        if self.active_quit_dirty(cx) {
            self.mark_quit_document_pending(cx);
            return true;
        }
        if self.pane_workspace.is_some() {
            let Some(document_id) = self
                .document_close_states(cx)
                .into_iter()
                .find(|state| {
                    state.dirty
                        && !crate::app_menu::QuitCoordinator::is_document_handled(
                            cx,
                            state.document_id,
                        )
                })
                .map(|state| state.document_id)
            else {
                return false;
            };
            let _ = crate::app_menu::QuitCoordinator::mark_document_pending(cx, document_id);
            return true;
        }
        let target = self
            .markdown_tab_sources()
            .into_iter()
            .find_map(|(index, source)| {
                if index == self.active_tab_index() {
                    return None;
                }
                if !source.try_is_dirty().unwrap_or(true) {
                    return None;
                }
                let document_id = match source.document_id() {
                    Ok(document_id) => document_id,
                    Err(_) => return Some(index),
                };
                if crate::app_menu::QuitCoordinator::is_document_handled(cx, document_id) {
                    return None;
                }
                Some(index)
            });
        let switched = target.is_some_and(|index| self.switch_to_tab_index(index, cx));
        if switched {
            self.mark_quit_document_pending(cx);
        }
        switched
    }

    pub(in crate::editor) fn discard_current_document_changes(
        &mut self,
        cx: &mut Context<Self>,
    ) -> bool {
        if let Some(document_host) = self.document_host.clone() {
            document_host.update(cx, |host, cx| host.discard_unsaved_changes(cx));
            self.document_dirty = false;
            return true;
        }
        if self.source_document.lease_count() != 1 {
            return false;
        }
        match self.source_document.try_discard_changes() {
            Ok(_) => {
                self.checkpoint_recovery_journal();
                self.document_dirty = false;
                true
            }
            Err(error) => {
                eprintln!("failed to discard shared document changes: {error}");
                false
            }
        }
    }

    /// Discard the dirty documents owned by a pane workspace before closing
    /// its window.  Pane tabs can expose several views of one document, so the
    /// operation is keyed by DocumentId and uses the authoritative lease
    /// inventory rather than each tab's cached dirty bit.
    fn discard_pane_document_changes_for_window_close(&mut self, cx: &mut Context<Self>) -> bool {
        let states = self
            .document_close_states(cx)
            .into_iter()
            .filter(|state| state.dirty && state.closes_last_lease())
            .map(|state| (state.document_id, state.window_lease_count))
            .collect::<BTreeMap<_, _>>();
        if states.is_empty() {
            return true;
        }

        // Keep one model reference per DocumentId.  A cloned PaneDocumentRef
        // shares the same lease and detached host state, so processing every
        // tab would either repeat the discard or race the first operation.
        let mut documents = BTreeMap::new();
        if let Some(workspace_entity) = self.pane_workspace.clone() {
            let workspace = workspace_entity.read(cx);
            for pane in workspace.workspace().pane_ids() {
                let Some(pane_state) = workspace.workspace().pane(pane) else {
                    continue;
                };
                for tab in pane_state.tabs() {
                    let document = tab.view().clone();
                    documents
                        .entry(document.document_id())
                        .or_insert((pane, document));
                }
            }
        }

        let mut all_clean = true;
        for (document_id, expected_owned_leases) in states {
            let Some((pane, document)) = documents.remove(&document_id) else {
                // A partially mounted workspace cannot prove that this dirty
                // document was discarded; retain the dialog conservatively.
                all_clean = false;
                continue;
            };
            let discarded = match document.kind() {
                crate::editor::panes::PaneDocumentKind::Markdown => match document.lease() {
                    Some(lease) => match lease
                        .handle()
                        .discard_current_changes_for_owned_leases(expected_owned_leases)
                    {
                        Ok(_) => {
                            let mounted = self
                                .pane_canvas_entities
                                .borrow()
                                .values()
                                .filter_map(|(_, _, canvas)| match canvas {
                                    crate::editor::panes::PaneCanvasEntity::Markdown(entity) => {
                                        Some(entity.clone())
                                    }
                                    crate::editor::panes::PaneCanvasEntity::DocumentHost(_)
                                    | crate::editor::panes::PaneCanvasEntity::ReadOnly(_) => None,
                                })
                                .collect::<Vec<_>>();
                            for canvas in mounted {
                                if canvas
                                    .read(cx)
                                    .close_state(cx)
                                    .is_some_and(|(mounted_id, _, _)| mounted_id == document_id)
                                {
                                    canvas.update(cx, |canvas, cx| {
                                        canvas.checkpoint_discarded_recovery(cx)
                                    });
                                }
                            }
                            true
                        }
                        Err(error) => {
                            eprintln!(
                                "failed to discard pane markdown changes for {document_id:?}: {error}"
                            );
                            false
                        }
                    },
                    None => false,
                },
                crate::editor::panes::PaneDocumentKind::DocumentHost => {
                    if let Some(handle) = document.host_handle() {
                        match handle.discard_current_changes_for_owned_leases(expected_owned_leases)
                        {
                            Ok(_) => true,
                            Err(error) => {
                                eprintln!(
                                    "failed to discard detached pane host changes for {document_id:?}: {error}"
                                );
                                false
                            }
                        }
                    } else {
                        let mounted_host = self.pane_canvas_entities.borrow().get(&pane).and_then(
                            |(_, _, canvas)| match canvas {
                                crate::editor::panes::PaneCanvasEntity::DocumentHost(entity) => {
                                    Some(entity.clone())
                                }
                                crate::editor::panes::PaneCanvasEntity::Markdown(_)
                                | crate::editor::panes::PaneCanvasEntity::ReadOnly(_) => None,
                            },
                        );
                        if let Some(mounted_host) = mounted_host {
                            let host = mounted_host.read(cx).host();
                            let mut discarded = false;
                            host.update(cx, |host, cx| {
                                discarded = host.discard_unsaved_changes_for_owned_leases(
                                    expected_owned_leases,
                                    cx,
                                );
                            });
                            discarded
                        } else {
                            false
                        }
                    }
                }
                // Image and error panes are read-only and therefore never
                // participate in the dirty-discard operation.
                crate::editor::panes::PaneDocumentKind::Image
                | crate::editor::panes::PaneDocumentKind::Error => true,
            };
            all_clean &= discarded;
        }

        all_clean
    }

    /// 窗口级“放弃并关闭”承诺丢弃整个窗口的编辑，不能逐标签重复同一弹窗。
    pub(in crate::editor) fn discard_all_document_changes_for_window_close(
        &mut self,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.pane_workspace.is_some() {
            return self.discard_pane_document_changes_for_window_close(cx);
        }

        let mut all_clean = true;
        if let Some(document_host) = self.document_host.clone() {
            document_host.update(cx, |host, cx| host.discard_unsaved_changes(cx));
            self.document_dirty = document_host.read(cx).is_dirty();
            all_clean &= !self.document_dirty;
        } else if self.source_document.lease_count() == 1 {
            match self.source_document.try_discard_changes() {
                Ok(_) => self.checkpoint_recovery_journal(),
                Err(error) => {
                    eprintln!("failed to discard shared document changes: {error}");
                    all_clean = false;
                }
            }
            self.document_dirty = self.source_document.try_is_dirty().unwrap_or(true);
            all_clean &= !self.document_dirty;
        } else {
            self.document_dirty = self.source_document.try_is_dirty().unwrap_or(true);
            all_clean &= !self.document_dirty;
        }

        // Inactive snapshots own their own view lease and therefore must be
        // resolved through the same Controller command.  Their cached
        // `document_dirty` bit is only a presentation mirror; never use it to
        // decide whether a document was discarded.
        for record in &mut self.tabs.records {
            let Some(snapshot) = record.snapshot.as_mut() else {
                continue;
            };
            if let Some(document_host) = snapshot.document_host.clone() {
                document_host.update(cx, |host, cx| host.discard_unsaved_changes(cx));
                snapshot.document_dirty = document_host.read(cx).is_dirty();
            } else if snapshot.source_document.lease_count() == 1 {
                match snapshot.source_document.try_discard_changes() {
                    Ok(_) => {
                        if let Err(error) = snapshot.checkpoint_recovery_journal() {
                            eprintln!(
                                "failed to checkpoint inactive tab recovery journal: {error}"
                            );
                            all_clean = false;
                        }
                    }
                    Err(error) => {
                        eprintln!("failed to discard inactive shared document changes: {error}");
                        all_clean = false;
                    }
                }
                snapshot.document_dirty = snapshot.source_document.try_is_dirty().unwrap_or(true);
            } else {
                snapshot.document_dirty = snapshot.source_document.try_is_dirty().unwrap_or(true);
            }
            if snapshot.document_dirty {
                all_clean = false;
            }
        }

        all_clean
    }

    pub(crate) fn request_close_current_window(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_menu_bar(cx);
        self.hide_info_dialog(cx);
        self.pending_close_after_save = false;

        if self.on_window_should_close(window, cx) {
            self.close_dialog_restore_focus = None;
            window.remove_window();
        }
    }

    pub(crate) fn restore_focus_after_close_dialog(&mut self, cx: &mut Context<Self>) {
        if let Some(focus_id) = self.close_dialog_restore_focus.take() {
            self.pending_focus = Some(focus_id);
            self.pending_scroll_active_block_into_view = true;
            cx.notify();
        }
    }

    pub(crate) fn hide_unsaved_changes_dialog(&mut self, cx: &mut Context<Self>) {
        if self.show_unsaved_changes_dialog {
            self.show_unsaved_changes_dialog = false;
            cx.notify();
        }
    }

    pub(crate) fn abort_pending_close_after_save(&mut self, cx: &mut Context<Self>) {
        let had_pending_close = self.pending_close_after_save;
        self.pending_close_after_save = false;
        self.cancel_explicit_window_close();
        self.abort_window_close_tab_sequence(cx);
        self.close_menu_bar(cx);
        self.hide_unsaved_changes_dialog(cx);
        if had_pending_close {
            crate::app_menu::abort_pending_quit(cx);
            crate::updater::UpdateCoordinator::cancel_pending_install(cx);
            self.restore_focus_after_close_dialog(cx);
        } else {
            self.close_dialog_restore_focus = None;
        }
    }

    /// 系统关窗同样等待输入法终态，随后重走既有最后租约保存门禁。
    pub(crate) fn on_window_should_close(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.defer_action_for_ime(&crate::components::CloseWindow, window, cx) {
            return false;
        }
        self.mark_explicit_window_close(true);
        let should_close = self.evaluate_window_should_close(window, cx);
        if should_close {
            self.remove_workspace_session_for_explicit_close(cx);
        }
        should_close
    }

    pub(crate) fn on_window_should_close_for_quit(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        self.mark_explicit_window_close(false);
        self.last_selection_snapshot = self.capture_source_selection_snapshot(cx);
        self.persist_workspace_session_before_quit(cx);
        self.evaluate_window_should_close_for_quit(window, cx)
    }

    fn evaluate_window_should_close_for_quit(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.show_external_conflict_dialog {
            if crate::app_menu::QuitCoordinator::is_pending_apply_update(cx) {
                crate::app_menu::abort_pending_quit(cx);
                crate::updater::UpdateCoordinator::cancel_pending_install(cx);
            }
            return false;
        }
        if !self.activate_dirty_tab_for_quit(cx) {
            return true;
        }

        self.close_menu_bar(cx);
        self.hide_info_dialog(cx);
        if !self.show_unsaved_changes_dialog {
            self.close_dialog_restore_focus = self.document.focused_block_entity_id(window, cx);
            self.show_unsaved_changes_dialog = true;
            window.blur();
            cx.notify();
        }
        false
    }

    fn evaluate_window_should_close(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.show_external_conflict_dialog {
            if crate::app_menu::QuitCoordinator::is_pending_apply_update(cx) {
                // An existing external-file conflict has no automatic safe
                // resolution. Cancel only the update intent and keep the
                // verified artifact ready for a later explicit retry.
                crate::app_menu::abort_pending_quit(cx);
                crate::updater::UpdateCoordinator::cancel_pending_install(cx);
            }
            return false;
        }
        if !self.activate_last_lease_dirty_for_window_close(cx) {
            return true;
        }

        self.close_menu_bar(cx);
        self.hide_info_dialog(cx);
        if !self.show_unsaved_changes_dialog {
            self.close_dialog_restore_focus = self.document.focused_block_entity_id(window, cx);
            self.show_unsaved_changes_dialog = true;
            window.blur();
            cx.notify();
        }

        false
    }

    pub(crate) fn on_cancel_close_dialog(
        &mut self,
        _: &ClickEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        crate::app_menu::abort_pending_quit(cx);
        crate::updater::UpdateCoordinator::cancel_pending_install(cx);
        self.pending_close_after_save = false;
        self.cancel_explicit_window_close();
        self.abort_window_close_tab_sequence(cx);
        self.close_menu_bar(cx);
        self.hide_unsaved_changes_dialog(cx);
        self.restore_focus_after_close_dialog(cx);
    }

    pub(crate) fn on_discard_and_close(
        &mut self,
        _: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.pending_close_after_save = false;
        self.close_dialog_restore_focus = None;
        self.close_menu_bar(cx);
        self.hide_unsaved_changes_dialog(cx);
        self.pending_window_edited = false;
        if self.discard_all_document_changes_for_window_close(cx) {
            self.remove_workspace_session_for_explicit_close(cx);
            window.remove_window();
            cx.defer(crate::app_menu::continue_pending_quit);
        } else {
            self.show_unsaved_changes_dialog = true;
            self.close_dialog_restore_focus = None;
            window.blur();
            cx.notify();
        }
    }

    pub(crate) fn on_save_and_close(
        &mut self,
        _: &ClickEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.pending_close_after_save = self.prepare_window_close_save();
        self.close_menu_bar(cx);
        self.hide_unsaved_changes_dialog(cx);
        self.pending_save = true;
        cx.notify();
    }
}
