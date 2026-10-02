// @author kongweiguang

// 把窗格实体、关闭和保存生命周期集中管理，避免与布局树操作互相耦合。

use super::*;

#[path = "pane_lifecycle/close.rs"]
mod close;

#[cfg(test)]
#[path = "../../../../tests/unit/editor/pane_lifecycle_private.rs"]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PaneCanvasKind {
    Markdown,
    DocumentHost,
    ReadOnly,
}

/// Limits structural host detachment to the canvas whose live lease must move
/// back into the pane model; Markdown and read-only entities keep their view
/// state and focus surface mounted across tree rebuilds.
fn pane_canvas_kind_requires_detach(kind: PaneCanvasKind) -> bool {
    matches!(kind, PaneCanvasKind::DocumentHost)
}

/// Identifies pane-tree mutations that must survive a crash through a session
/// snapshot; close-tab has a separate dirty/lease lifecycle and schedules on
/// its accepted close path instead.
fn pane_event_persists_workspace(event: &crate::editor::panes::PaneEvent) -> bool {
    matches!(
        event,
        crate::editor::panes::PaneEvent::Split { .. }
            | crate::editor::panes::PaneEvent::CopyTab { .. }
            | crate::editor::panes::PaneEvent::Close { .. }
            | crate::editor::panes::PaneEvent::MoveTab { .. }
            | crate::editor::panes::PaneEvent::Balance
    )
}

/// Requires both a structural event and a successful model mutation so an
/// invalid close/move cannot create a needless session generation or snapshot.
fn should_persist_pane_event(event: &crate::editor::panes::PaneEvent, succeeded: bool) -> bool {
    succeeded && pane_event_persists_workspace(event)
}

impl Editor {
    /// Moves only an active host lease back to its pane model while preserving
    /// the view snapshot needed to recreate the host after a tree mutation.
    pub(in crate::editor) fn detach_pane_canvas(
        &mut self,
        pane: crate::editor::panes::PaneId,
        cx: &mut Context<Self>,
    ) {
        let Some((tab_id, view_id, canvas)) = self.pane_canvas_entities.borrow_mut().remove(&pane)
        else {
            return;
        };
        let snapshot = match &canvas {
            crate::editor::panes::PaneCanvasEntity::Markdown(entity) => {
                Some(entity.read(cx).pane_view_state_snapshot(cx))
            }
            crate::editor::panes::PaneCanvasEntity::DocumentHost(entity) => {
                Some(entity.read(cx).pane_view_state_snapshot(cx))
            }
            crate::editor::panes::PaneCanvasEntity::ReadOnly(_) => None,
        };
        let detached = match &canvas {
            crate::editor::panes::PaneCanvasEntity::Markdown(_) => None,
            crate::editor::panes::PaneCanvasEntity::DocumentHost(entity) => {
                entity.update(cx, |canvas, cx| canvas.detach(cx))
            }
            crate::editor::panes::PaneCanvasEntity::ReadOnly(_) => None,
        };
        if detached.is_none()
            && matches!(
                &canvas,
                crate::editor::panes::PaneCanvasEntity::DocumentHost(_)
            )
        {
            self.pane_canvas_entities
                .borrow_mut()
                .insert(pane, (tab_id, view_id, canvas));
            return;
        }
        if let Some(workspace) = self.pane_workspace.clone() {
            workspace.update(cx, |workspace, _cx| {
                if let Some(tab) = workspace.workspace_mut().find_tab_mut(tab_id) {
                    if let Some(snapshot) = snapshot {
                        tab.view().set_view_state_snapshot(snapshot);
                    }
                    if let Some(detached) = detached {
                        let _ = tab.view().put_detached_host(detached);
                    }
                }
            });
        }
    }

    /// Detaches host canvases before structural tree rewrites because their
    /// linear runtime token cannot be cloned while Markdown/read-only entities
    /// must remain mounted to preserve focus and presentation state.
    pub(super) fn detach_all_pane_host_canvases(&mut self, cx: &mut Context<Self>) {
        let panes = self
            .pane_canvas_entities
            .borrow()
            .iter()
            .filter_map(|(pane, (_, _, canvas))| {
                let kind = match canvas {
                    crate::editor::panes::PaneCanvasEntity::Markdown(_) => PaneCanvasKind::Markdown,
                    crate::editor::panes::PaneCanvasEntity::DocumentHost(_) => {
                        PaneCanvasKind::DocumentHost
                    }
                    crate::editor::panes::PaneCanvasEntity::ReadOnly(_) => PaneCanvasKind::ReadOnly,
                };
                pane_canvas_kind_requires_detach(kind).then_some(*pane)
            })
            .collect::<Vec<_>>();
        for pane in panes {
            self.detach_pane_canvas(pane, cx);
        }
    }

    pub(super) fn sync_pane_canvas_entities(&mut self, cx: &mut Context<Self>) {
        let Some(workspace_entity) = self.pane_workspace.clone() else {
            return;
        };
        let focused_pane = workspace_entity.read(cx).workspace().focused_pane();
        let desired = workspace_entity
            .read(cx)
            .workspace()
            .pane_ids()
            .into_iter()
            .filter_map(|pane| {
                let tab = workspace_entity
                    .read(cx)
                    .workspace()
                    .pane(pane)
                    .and_then(|state| state.active_tab())?;
                Some((pane, tab.id(), tab.view().clone()))
            })
            .collect::<Vec<_>>();
        let stale = self
            .pane_canvas_entities
            .borrow()
            .iter()
            .filter_map(|(pane, (tab_id, view_id, _))| {
                desired
                    .iter()
                    .find(|(desired_pane, _desired_tab, _)| desired_pane == pane)
                    .is_none_or(|(_, desired_tab, document)| {
                        desired_tab != tab_id || document.view_id() != *view_id
                    })
                    .then_some(*pane)
            })
            .collect::<Vec<_>>();
        for pane in stale {
            self.detach_pane_canvas(pane, cx);
        }
        for (pane, tab_id, document) in desired {
            let same = self.pane_canvas_entities.borrow().get(&pane).is_some_and(
                |(existing_tab, view_id, _)| {
                    *existing_tab == tab_id && *view_id == document.view_id()
                },
            );
            if !same {
                let canvas = match document.kind() {
                    crate::editor::panes::PaneDocumentKind::Markdown => {
                        let Some(lease) = document.lease_arc().cloned() else {
                            continue;
                        };
                        let Ok(session) = EditorDocumentSession::from_lease_arc_with_view_id(
                            lease,
                            document.view_id(),
                        ) else {
                            continue;
                        };
                        let file_path = document.path().cloned();
                        let view_state = document.view_state_snapshot();
                        crate::editor::panes::PaneCanvasEntity::Markdown(cx.new(move |cx| {
                            crate::editor::panes::create_pane_editor_canvas(
                                cx,
                                session,
                                file_path,
                                tab_id.as_uuid(),
                                view_state,
                            )
                        }))
                    }
                    crate::editor::panes::PaneDocumentKind::DocumentHost => {
                        let Some(detached) = document.take_detached_host() else {
                            continue;
                        };
                        let Some(path) = document.host_path().cloned() else {
                            continue;
                        };
                        let Some(probe) = document.host_probe().cloned() else {
                            continue;
                        };
                        crate::editor::panes::PaneCanvasEntity::DocumentHost(cx.new(move |cx| {
                            crate::editor::panes::create_pane_document_host_canvas(
                                cx, path, probe, detached,
                            )
                        }))
                    }
                    crate::editor::panes::PaneDocumentKind::Image
                    | crate::editor::panes::PaneDocumentKind::Error => {
                        let Some(kind) = document.readonly_kind().cloned() else {
                            continue;
                        };
                        crate::editor::panes::PaneCanvasEntity::ReadOnly(cx.new(move |cx| {
                            crate::editor::panes::create_pane_readonly_canvas(cx, kind)
                        }))
                    }
                };
                self.pane_canvas_entities
                    .borrow_mut()
                    .insert(pane, (tab_id, document.view_id(), canvas));
            }
            let markdown_entity = self.pane_canvas_entities.borrow().get(&pane).and_then(
                |(_, _, entity)| match entity {
                    crate::editor::panes::PaneCanvasEntity::Markdown(entity) => {
                        Some(entity.clone())
                    }
                    crate::editor::panes::PaneCanvasEntity::DocumentHost(_)
                    | crate::editor::panes::PaneCanvasEntity::ReadOnly(_) => None,
                },
            );
            if let Some(entity) = markdown_entity {
                entity.update(cx, |canvas, cx| {
                    canvas.set_view_mode(self.view_mode, cx);
                    canvas.set_focus_enabled(pane == focused_pane, cx);
                });
            }
            // The factory is stable for the lifetime of this canvas. Replacing
            // it on every parent render invalidates the outer PaneView, which
            // detaches the focused editor subtree while the user is typing.
            if same {
                continue;
            }
            let registry = Rc::clone(&self.pane_canvas_entities);
            let factory: crate::editor::panes::PaneContentFactory =
                Rc::new(move |document, _window, _cx| {
                    registry
                        .borrow()
                        .get(&pane)
                        .filter(|(_, view_id, _)| *view_id == document.view_id())
                        .map(|(_, _, entity)| match entity {
                            crate::editor::panes::PaneCanvasEntity::Markdown(entity) => {
                                entity.clone().into_any_element()
                            }
                            crate::editor::panes::PaneCanvasEntity::DocumentHost(entity) => {
                                entity.clone().into_any_element()
                            }
                            crate::editor::panes::PaneCanvasEntity::ReadOnly(entity) => {
                                entity.clone().into_any_element()
                            }
                        })
                        .unwrap_or_else(|| div().size_full().into_any_element())
                });
            workspace_entity.update(cx, |workspace, cx| {
                workspace.set_content_factory(pane, Some(factory));
                // Installing a factory invalidates the cached PaneView for this
                // leaf. Notify the workspace itself so the replacement entity
                // is mounted even when the parent Editor does not otherwise
                // need another render pass.
                cx.notify();
            });
        }
    }

    /// Read-only close/quit inventory for all pane tabs.  The result contains
    /// no GPUI Entity, source text, or task; callers can merge it with legacy
    /// tab inventory without taking ownership of a pane view.
    pub(crate) fn pane_document_close_states(
        &self,
        cx: &App,
    ) -> Vec<crate::editor::panes::PaneDocumentCloseState> {
        let Some(workspace) = self.pane_workspace.as_ref() else {
            return Vec::new();
        };
        let view = workspace.read(cx);
        let mut states = BTreeMap::<
            gmark_document_runtime::DocumentId,
            crate::editor::panes::PaneDocumentCloseState,
        >::new();
        for pane in view.workspace().pane_ids() {
            let Some(pane_state) = view.workspace().pane(pane) else {
                continue;
            };
            for tab in pane_state.tabs() {
                let document_id = *tab.document();
                let mut dirty = false;
                let mut lease_count = 0;
                match tab.view().kind() {
                    crate::editor::panes::PaneDocumentKind::Markdown => {
                        if let Some((
                            _,
                            _,
                            crate::editor::panes::PaneCanvasEntity::Markdown(entity),
                        )) = self.pane_canvas_entities.borrow().get(&pane)
                        {
                            if let Some((_, markdown_dirty, markdown_leases)) =
                                entity.read(cx).close_state(cx)
                            {
                                dirty = markdown_dirty;
                                lease_count = markdown_leases;
                            }
                        } else if let Some(lease) = tab.view().lease() {
                            let handle = lease.handle();
                            dirty = handle
                                .lock()
                                .map(|controller| controller.session().dirty)
                                .unwrap_or(false);
                            lease_count = handle.lease_count();
                        }
                    }
                    crate::editor::panes::PaneDocumentKind::DocumentHost => {
                        if let Some(handle) = tab.view().host_handle() {
                            dirty = handle
                                .lock()
                                .map(|controller| controller.session().dirty)
                                .unwrap_or(false);
                            lease_count = tab.view().host_lease_count().unwrap_or_default();
                        } else if let Some((
                            _,
                            _,
                            crate::editor::panes::PaneCanvasEntity::DocumentHost(entity),
                        )) = self.pane_canvas_entities.borrow().get(&pane)
                        {
                            if let Some((_, host_dirty, host_leases)) =
                                entity.read(cx).close_state(cx)
                            {
                                dirty = host_dirty;
                                lease_count = host_leases;
                            }
                        }
                    }
                    crate::editor::panes::PaneDocumentKind::Image
                    | crate::editor::panes::PaneDocumentKind::Error => {}
                }
                let entry = states.entry(document_id).or_insert_with(|| {
                    crate::editor::panes::PaneDocumentCloseState {
                        document_id,
                        dirty,
                        global_lease_count: lease_count,
                        window_view_count: 0,
                    }
                });
                entry.dirty |= dirty;
                entry.global_lease_count = entry.global_lease_count.max(lease_count);
                entry.window_view_count = entry.window_view_count.saturating_add(1);
            }
        }
        states.into_values().collect()
    }

    /// Applies one pane command, then persists only successful structural
    /// mutations so rejected operations cannot alter session generations.
    pub(in crate::editor) fn handle_pane_event(
        &mut self,
        event: crate::editor::panes::PaneEvent,
        mut window: Option<&mut Window>,
        cx: &mut Context<Self>,
    ) {
        if self.has_active_ime_composition(cx) {
            self.queue_ime_operation(
                crate::editor::ime_lifecycle::DeferredImeOperation::Pane(event),
                cx,
            );
            if let Some(window) = window.as_deref_mut() {
                self.wait_for_ime_completion(window, cx);
            }
            cx.notify();
            return;
        }
        let Some(workspace_entity) = self.pane_workspace.clone() else {
            return;
        };
        let structural = matches!(
            event,
            crate::editor::panes::PaneEvent::Split { .. }
                | crate::editor::panes::PaneEvent::CopyTab { .. }
                | crate::editor::panes::PaneEvent::Close { .. }
                | crate::editor::panes::PaneEvent::MoveTab { .. }
        );
        // A pane-local tab close has its own dirty-document lifecycle.  Do not
        // eagerly detach the canvas here: a pending close prompt must leave the
        // exact pane/tab view untouched until Cancel, Discard, or Save accepts
        // the operation.
        if let crate::editor::panes::PaneEvent::CloseTab { pane, tab } = event {
            self.request_close_pane_tab(&workspace_entity, pane, tab, window, cx);
            return;
        }
        self.invalidate_pane_close_for_event(&event, cx);
        // A host canvas owns the only live lease. Move it only for structural
        // tree operations; focus and divider updates must not tear down a
        // live input surface and cause a visible focus flash.
        if structural {
            self.detach_all_pane_host_canvases(cx);
        }
        let result = match event {
            crate::editor::panes::PaneEvent::Split { pane, direction } => {
                self.split_pane_with_active_view(&workspace_entity, pane, direction, cx)
            }
            crate::editor::panes::PaneEvent::CopyTab {
                source,
                target,
                tab,
            } => self.copy_pane_tab_with_fork(&workspace_entity, source, target, tab, cx),
            crate::editor::panes::PaneEvent::Close { pane } => workspace_entity
                .update(cx, |workspace, _cx| {
                    workspace.workspace_mut().close_pane(pane).map(|_| ())
                }),
            crate::editor::panes::PaneEvent::CloseTab { .. } => Ok(()),
            crate::editor::panes::PaneEvent::ActivateTab { pane, tab } => {
                workspace_entity.update(cx, |workspace, _cx| {
                    workspace.workspace_mut().focus(pane)?;
                    workspace.workspace_mut().set_active_tab(pane, tab)
                })
            }
            crate::editor::panes::PaneEvent::OpenNewTabMenu { pane, x, y } => {
                let focus = workspace_entity
                    .update(cx, |workspace, _cx| workspace.workspace_mut().focus(pane));
                if focus.is_ok() {
                    self.open_new_tab_menu(point(px(x), px(y)), Some(pane), cx);
                }
                focus
            }
            crate::editor::panes::PaneEvent::OpenSplitMenu { pane, x, y } => {
                let focus = workspace_entity
                    .update(cx, |workspace, _cx| workspace.workspace_mut().focus(pane));
                if focus.is_ok() {
                    self.open_split_pane_menu(point(px(x), px(y)), window, cx);
                }
                focus
            }
            crate::editor::panes::PaneEvent::DismissMenus => {
                self.tabs.dismiss_new_or_split_menu();
                cx.notify();
                Ok(())
            }
            crate::editor::panes::PaneEvent::Focus { pane } => {
                workspace_entity.update(cx, |workspace, _cx| workspace.workspace_mut().focus(pane))
            }
            crate::editor::panes::PaneEvent::FocusAdjacent { from, direction } => workspace_entity
                .update(cx, |workspace, _cx| {
                    workspace
                        .workspace_mut()
                        .focus_adjacent_from(from, direction)
                        .map(|_| ())
                }),
            crate::editor::panes::PaneEvent::MoveTab {
                source,
                target,
                tab,
            } => workspace_entity.update(cx, |workspace, _cx| {
                workspace.workspace_mut().move_tab(source, target, tab)
            }),
            crate::editor::panes::PaneEvent::Balance => {
                workspace_entity.update(cx, |workspace, _cx| {
                    workspace.workspace_mut().balance();
                    Ok(())
                })
            }
        };
        let succeeded = result.is_ok();
        if let Err(error) = result {
            // The model intentionally leaves all state untouched on illegal
            // transfers (duplicate identity, last-pane close, etc.). Keep the
            // rejection non-blocking and visible without stealing focus.
            let strings = cx.global::<crate::ui::i18n::I18nManager>().strings();
            let message = match &error {
                crate::editor::panes::PaneError::DuplicateDocument => format!(
                    "{}: {}",
                    strings.pane_notice_duplicate_document_label,
                    strings.pane_notice_duplicate_document_description,
                ),
                crate::editor::panes::PaneError::TooManyPanes => format!(
                    "{}: {}",
                    strings.pane_notice_pane_limit_label,
                    strings.pane_notice_pane_limit_description,
                ),
                _ => error.to_string(),
            };
            self.show_pane_notice(message, cx);
        }
        self.sync_pane_canvas_entities(cx);
        if should_persist_pane_event(&event, succeeded) {
            self.schedule_workspace_session_save(cx);
        }
        cx.notify();
    }

    pub(super) fn pane_tab_close_state(
        &self,
        workspace: &Entity<crate::editor::panes::PaneWorkspaceView>,
        pane: crate::editor::panes::PaneId,
        tab: crate::editor::panes::TabId,
        cx: &App,
    ) -> Option<(bool, usize)> {
        let tab_view = workspace.read(cx).workspace().tab(pane, tab)?;
        let view = tab_view.view();
        let mut dirty = view.is_dirty();
        let mut leases = match view.kind() {
            crate::editor::panes::PaneDocumentKind::Markdown => view
                .lease()
                .map(|lease| lease.handle().lease_count())
                .unwrap_or_default(),
            crate::editor::panes::PaneDocumentKind::DocumentHost => {
                view.host_lease_count().unwrap_or_default()
            }
            crate::editor::panes::PaneDocumentKind::Image
            | crate::editor::panes::PaneDocumentKind::Error => 0,
        };
        // An active host's detached token lives in its canvas, not in the tab
        // cell.  Read the canvas snapshot before deciding whether this is the
        // final dirty view.
        if workspace
            .read(cx)
            .workspace()
            .pane(pane)
            .and_then(|state| state.active_tab_id())
            == Some(tab)
        {
            if let Some((_, _, canvas)) = self.pane_canvas_entities.borrow().get(&pane) {
                match canvas {
                    crate::editor::panes::PaneCanvasEntity::Markdown(entity) => {
                        if let Some((_, canvas_dirty, canvas_leases)) =
                            entity.read(cx).close_state(cx)
                        {
                            dirty = canvas_dirty;
                            leases = canvas_leases;
                        }
                    }
                    crate::editor::panes::PaneCanvasEntity::DocumentHost(entity) => {
                        if let Some((_, canvas_dirty, canvas_leases)) =
                            entity.read(cx).close_state(cx)
                        {
                            dirty = canvas_dirty;
                            leases = canvas_leases;
                        }
                    }
                    crate::editor::panes::PaneCanvasEntity::ReadOnly(_) => {}
                }
            }
        }
        Some((dirty, leases))
    }

    /// Defers a dirty tab close until its own document save reports a terminal
    /// result, keeping the mounted canvas stable while the prompt is visible.
    pub(super) fn request_close_pane_tab(
        &mut self,
        workspace: &Entity<crate::editor::panes::PaneWorkspaceView>,
        pane: crate::editor::panes::PaneId,
        tab: crate::editor::panes::TabId,
        window: Option<&mut Window>,
        cx: &mut Context<Self>,
    ) {
        let Some((dirty, leases)) = self.pane_tab_close_state(workspace, pane, tab, cx) else {
            return;
        };
        if dirty && leases <= 1 {
            self.begin_pane_close_request(pane, tab, cx);
            self.show_pane_tab_close_prompt(cx);
            return;
        }
        // A direct close supersedes any older save request for another tab;
        // drop that task before mutating the pane model synchronously.
        self.invalidate_pane_close_save(cx);
        self.close_pane_tab_now(workspace, pane, tab, cx);
        let _ = window;
    }

    /// Applies a validated pane-model close and retires its request identity so
    /// late callbacks cannot target a replacement tab.
    pub(in crate::editor) fn close_pane_tab_now(
        &mut self,
        workspace: &Entity<crate::editor::panes::PaneWorkspaceView>,
        pane: crate::editor::panes::PaneId,
        tab: crate::editor::panes::TabId,
        cx: &mut Context<Self>,
    ) -> bool {
        // Only the active tab owns the mounted canvas.  Closing an inactive tab
        // must leave that entity (and its focus/bounds) untouched; detaching it
        // would flash/rebuild the user's current editor for no reason.
        let active = workspace
            .read(cx)
            .workspace()
            .pane(pane)
            .and_then(|state| state.active_tab_id())
            == Some(tab);
        if active {
            // Detach first so an active document host returns its linear lease
            // to the model before the tab value is dropped.
            self.detach_pane_canvas(pane, cx);
        }
        let closed = workspace
            .update(cx, |workspace, _cx| {
                workspace.workspace_mut().close_tab(pane, tab)
            })
            .ok();
        let Some(closed) = closed else {
            return false;
        };
        let _ = self.view_state.close_tab(tab.as_uuid());
        let empty_last_pane = workspace.read(cx).workspace().pane_count() == 1
            && workspace
                .read(cx)
                .workspace()
                .pane(workspace.read(cx).workspace().focused_pane())
                .is_some_and(crate::editor::panes::PaneState::is_empty);
        if empty_last_pane {
            let target = workspace.read(cx).workspace().focused_pane();
            let _ = self.new_document_tab_in_pane(target, DocumentKind::Markdown, cx);
        }
        drop(closed);
        self.finish_pane_close_request(cx);
        self.sync_pane_canvas_entities(cx);
        self.schedule_workspace_session_save(cx);
        cx.notify();
        true
    }
}
