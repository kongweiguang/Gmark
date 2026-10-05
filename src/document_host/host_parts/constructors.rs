// @author kongweiguang

use super::*;

impl DocumentHost {
    /// Pane integration may carry constructor metadata across detach without
    /// borrowing any Controller body or backend save plan.
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn probe(&self) -> &OpenProbe {
        &self.probe
    }

    /// Read-only identity seams for pane close/detach bookkeeping.  These
    /// never expose the Controller body or its save state.
    pub(crate) fn document_id(&self) -> Option<DocumentId> {
        self.document
            .as_ref()
            .and_then(|document| document.document_id().ok())
    }

    /// Return the Controller-owned view identity for the active Host.  This
    /// is metadata-only: it neither registers another view nor acquires a
    /// lease, so workspace snapshots can persist the exact identity used by
    /// the current Entity.
    pub(crate) fn view_id(&self) -> Option<DocumentViewInstanceId> {
        self.document.as_ref().map(SharedDocument::view_id)
    }

    pub(crate) fn lease_count(&self) -> usize {
        self.document
            .as_ref()
            .map(SharedDocument::lease_count)
            .unwrap_or_default()
    }

    /// Return the active pane's pure presentation state without detaching the
    /// Entity or stopping any view-scoped work.  Pane/session code can use the
    /// same DTO as a detached view.
    pub(crate) fn view_presentation_snapshot(&self, cx: &App) -> DocumentHostViewPresentation {
        DocumentHostViewPresentation::bounded(
            self.capture_presentation(cx),
            self.view_back_history.iter().cloned(),
            self.view_forward_history.iter().cloned(),
        )
    }

    /// Installs the first runtime session for the compatibility/indexing path.
    /// Once a shared handle exists, only its Controller session is updated; no
    /// second body is retained by the host.
    pub(super) fn install_document_session(&mut self, document: DocumentSession) {
        if self.document.is_none()
            && let Ok(shared) = SharedDocument::from_controller(DocumentController::new(
                DocumentId::new(),
                document,
            ))
        {
            self.document = Some(shared);
        } else if self.document.is_none() {
            self.error = Some("document controller initialization failed".into());
        }
    }

    pub(crate) fn from_shared(
        path: PathBuf,
        probe: OpenProbe,
        handle: DocumentHandle,
        lease: DocumentLease,
        cx: &mut Context<Self>,
    ) -> Self {
        let fallback_path = path.clone();
        let fallback_probe = probe.clone();
        match Self::from_shared_with_view_id(
            path,
            probe,
            handle,
            lease,
            DocumentViewInstanceId::new(),
            DocumentHostViewPresentation::default(),
            cx,
        ) {
            Ok(view) => view,
            Err(error) => {
                let mut view = Self::new_with_source(fallback_path, fallback_probe, None, cx);
                view.error = Some(error.to_string().into());
                view
            }
        }
    }

    /// Restore a service-owned Host using the persisted pane view identity and
    /// pure presentation snapshot.  A duplicate or nil identity fails closed
    /// in `SharedDocument::from_handle_with_view_id`; it never falls back to a
    /// random Controller view or creates a second body.
    pub(crate) fn from_shared_with_view_id(
        path: PathBuf,
        probe: OpenProbe,
        handle: DocumentHandle,
        lease: DocumentLease,
        view_id: DocumentViewInstanceId,
        presentation: DocumentHostViewPresentation,
        cx: &mut Context<Self>,
    ) -> Result<Self, ControllerError> {
        let mut view = Self::new_with_source(path, probe, None, cx);
        let shared = SharedDocument::from_handle_with_view_id(handle, lease, view_id)?;
        view.view_back_history = VecDeque::from_iter(presentation.back_history.clone());
        view.view_forward_history = VecDeque::from_iter(presentation.forward_history.clone());
        view.document = Some(shared);
        // Shared production opens must attach recovery to the Controller
        // lifetime before a detached/split pane can receive its first edit.
        view.start_shared_recovery(cx);
        view.restore_presentation(presentation.current, cx);
        if let Some(document) = view.document.as_ref() {
            let _ = document.set_source_selection(view.tab_view_state.source.selection);
        }
        view.index = view.document.as_ref().and_then(SharedDocument::line_index);
        view.provisional_source = None;
        view.external_monitor_owned = false;
        view.closed_suspended = false;
        view.start_controller_event_subscription(cx);
        if view.structured_index.is_none()
            && !document_dirty_state(&view.document)
            && view.document.is_some()
        {
            view.rebuild_clean_structured_index(cx);
        }
        if view.view_mode != DocumentHostViewMode::Source {
            view.request_registered_projection(cx);
        }
        if !view.search_input.read(cx).display_text().is_empty() {
            view.schedule_search(cx);
        }
        if !view
            .structured_filter_input
            .read(cx)
            .display_text()
            .is_empty()
        {
            view.schedule_structured_filter(cx);
        }
        Ok(view)
    }

    /// GPUI Entity-friendly compatibility wrapper for callers whose mount
    /// closure must return `Self`.  New restore code should call
    /// [`Self::from_shared_with_view_id`] first when it needs to branch on a
    /// duplicate/nil identity; this wrapper records the typed failure only
    /// after that strict path has rejected it.
    pub(crate) fn from_shared_with_view_id_or_error(
        path: PathBuf,
        probe: OpenProbe,
        handle: DocumentHandle,
        lease: DocumentLease,
        view_id: DocumentViewInstanceId,
        presentation: DocumentHostViewPresentation,
        cx: &mut Context<Self>,
    ) -> Self {
        let fallback_path = path.clone();
        let fallback_probe = probe.clone();
        match Self::from_shared_with_view_id(path, probe, handle, lease, view_id, presentation, cx)
        {
            Ok(view) => view,
            Err(error) => {
                let mut view = Self::new_with_source(fallback_path, fallback_probe, None, cx);
                view.error = Some(error.to_string().into());
                view
            }
        }
    }

    /// Rebuild an active host Entity from pane-owned pure view state. The
    /// detached lease is moved into the rebuilt host; no clone can extend the
    /// registry lifetime.
    pub(crate) fn from_detached(
        path: PathBuf,
        probe: OpenProbe,
        detached: DetachedDocumentHostView,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut view = Self::new_with_source(path, probe, None, cx);
        match detached
            .activate()
            .and_then(|(handle, lease, view_id, presentation)| {
                SharedDocument::from_handle(handle, lease, view_id)
                    .map(|document| (document, presentation))
            }) {
            Ok((document, presentation)) => {
                view.view_back_history = VecDeque::from_iter(presentation.back_history.clone());
                view.view_forward_history =
                    VecDeque::from_iter(presentation.forward_history.clone());
                view.document = Some(document);
                // Reattached views reuse the handle-anchored worker; this call
                // only starts setup when the shared document has no journal.
                view.start_shared_recovery(cx);
                view.restore_presentation(presentation.current, cx);
                view.index = view.document.as_ref().and_then(SharedDocument::line_index);
                view.provisional_source = None;
                view.external_monitor_owned = false;
                view.closed_suspended = false;
                view.start_controller_event_subscription(cx);
                if view.structured_index.is_none()
                    && !document_dirty_state(&view.document)
                    && view.document.is_some()
                {
                    view.rebuild_clean_structured_index(cx);
                }
                if view.view_mode != DocumentHostViewMode::Source {
                    view.request_registered_projection(cx);
                }
                if !view.search_input.read(cx).display_text().is_empty() {
                    view.schedule_search(cx);
                }
                if !view
                    .structured_filter_input
                    .read(cx)
                    .display_text()
                    .is_empty()
                {
                    view.schedule_structured_filter(cx);
                }
            }
            Err(error) => view.error = Some(error.to_string().into()),
        }
        view
    }

    /// 拆卸仅转移纯展示状态；待定位文字仍属于输入实体，必须先完成或复制恢复，不能随租约转移丢失。
    pub(crate) fn detach_view(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Option<DetachedDocumentHostView> {
        if self.saving || self.reloading || self.has_pending_source_input() {
            return None;
        }
        let presentation = self.capture_presentation(cx);
        self.suspend_for_closed_tab();
        let document = self.document.take()?;
        let (handle, lease, view_id) = document.detach_parts()?;
        Some(DetachedDocumentHostView::from_state(
            DocumentHostViewState::with_presentation(
                handle,
                lease,
                view_id,
                DocumentHostViewPresentation::bounded(
                    presentation,
                    self.view_back_history.iter().cloned(),
                    self.view_forward_history.iter().cloned(),
                ),
            ),
        ))
    }
}
