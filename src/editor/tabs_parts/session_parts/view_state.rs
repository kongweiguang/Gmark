// @author kongweiguang

use super::*;

impl Editor {
    pub(crate) fn restore_pane_view_state(
        &mut self,
        snapshot: crate::editor::panes::PaneViewStateSnapshot,
        cx: &mut Context<Self>,
    ) {
        if let Some(mode) = snapshot.view_mode.as_deref().and_then(Self::pane_view_mode) {
            self.set_view_mode(mode, cx);
        }
        if let Some(selection) = snapshot.selection.as_ref() {
            let len = self.source_document.len();
            let start = selection.start.min(len);
            let end = selection.end.min(len);
            let source_selection = selection.source_selection_for_range(start..end);
            let selection = UndoSelectionSnapshot::from_source_selection(source_selection);
            self.apply_selection_snapshot_in_current_mode(&selection, cx);
            self.last_selection_snapshot = selection;
        }
        if let (Some(scroll_x), Some(scroll_y)) = (snapshot.scroll_x, snapshot.scroll_y)
            && scroll_x.is_finite()
            && scroll_y.is_finite()
        {
            self.scroll_handle
                .set_offset(point(px(scroll_x), px(scroll_y)));
        }
        if let Some(split_ratio) = snapshot.split_ratio.filter(|value| value.is_finite()) {
            self.split_pane_ratio = split_ratio.clamp(0.1, 0.9);
        }

        let mut markdown = crate::editor::markdown_view_state::MarkdownViewState::default();
        for fold in &snapshot.markdown_folds {
            let Some(object) = fold.as_object() else {
                continue;
            };
            let Some(key) = object.get("key").and_then(Value::as_str) else {
                continue;
            };
            let collapsed = object
                .get("collapsed")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            match object.get("kind").and_then(Value::as_str) {
                Some("heading") => {
                    markdown
                        .collapsed_headings
                        .insert(key.to_owned(), collapsed);
                }
                Some("callout") => {
                    markdown
                        .collapsed_callouts
                        .insert(key.to_owned(), collapsed);
                }
                _ => {}
            }
        }
        if let Some(table_layout) = snapshot.table_layout.as_ref()
            && let Some(columns) = table_layout.get("columns").and_then(Value::as_object)
        {
            for (key, widths) in columns {
                let Some(widths) = widths.as_array() else {
                    continue;
                };
                let fractions = widths
                    .iter()
                    .filter_map(Value::as_f64)
                    .map(|value| value as f32)
                    .collect();
                markdown.table_column_widths.insert(key.clone(), fractions);
            }
        }
        if let Some(tab_id) = self.pane_tab_id {
            let _ = self.view_state.replace_tab_state(tab_id, markdown);
        }
        self.pane_history_back = snapshot.back;
        self.pane_history_forward = snapshot.forward;
    }

    fn pane_view_mode(value: &str) -> Option<ViewMode> {
        match value {
            "rendered" | "live" => Some(ViewMode::Rendered),
            "source" => Some(ViewMode::Source),
            "preview" | "structure" => Some(ViewMode::Preview),
            "split" => Some(ViewMode::Split),
            _ => None,
        }
    }

    /// Capture the complete pure presentation state for one pane canvas.
    ///
    /// This is intentionally body-free: history entries retain revision,
    /// length, kind, and selection metadata, while the shared Rope remains
    /// owned by the document session.  The returned DTO is the same versioned
    /// shape consumed by workspace-session persistence.
    pub(crate) fn pane_view_state_snapshot(
        &self,
        cx: &App,
    ) -> crate::editor::panes::PaneViewStateSnapshot {
        let tab_id = self.pane_tab_id.unwrap_or_else(|| self.tabs.active_id());
        let presentation = self.view_state.state_for_tab(tab_id).unwrap_or_default();
        let selection =
            crate::config::workspace_session::WorkspaceSessionSelection::from_source_selection(
                self.capture_source_selection_snapshot(cx)
                    .source_selection(),
            );
        let scroll = self.scroll_handle.offset();

        let mut markdown_folds = presentation
            .collapsed_headings
            .iter()
            .map(|(key, collapsed)| {
                json!({
                    "kind": "heading",
                    "key": key,
                    "collapsed": collapsed,
                })
            })
            .chain(
                presentation
                    .collapsed_callouts
                    .iter()
                    .map(|(key, collapsed)| {
                        json!({
                            "kind": "callout",
                            "key": key,
                            "collapsed": collapsed,
                        })
                    }),
            )
            .collect::<Vec<_>>();
        markdown_folds.sort_by_key(|left| left.to_string());

        let table_layout = (!presentation.table_column_widths.is_empty()).then(|| {
            let columns = presentation
                .table_column_widths
                .iter()
                .map(|(key, widths)| (key.clone(), json!(widths)))
                .collect::<std::collections::BTreeMap<_, _>>();
            json!({ "columns": columns })
        });

        let mut back = self.pane_history_back.clone();
        back.extend(
            self.undo_history
                .iter()
                .map(Self::pane_history_value)
                .collect::<Vec<_>>(),
        );
        back.extend(
            self.virtual_undo_selections
                .iter()
                .map(|selection| Self::pane_selection_history_value(*selection)),
        );
        let mut forward = self.pane_history_forward.clone();
        forward.extend(
            self.redo_history
                .iter()
                .map(Self::pane_history_value)
                .collect::<Vec<_>>(),
        );
        forward.extend(
            self.virtual_redo_selections
                .iter()
                .map(|selection| Self::pane_selection_history_value(*selection)),
        );
        if back.len() > 32 {
            back.drain(0..back.len() - 32);
        }
        if forward.len() > 32 {
            forward.drain(0..forward.len() - 32);
        }

        crate::config::workspace_session::WorkspaceSessionPaneViewState {
            selection: Some(selection),
            scroll_x: Some(f32::from(scroll.x)),
            scroll_y: Some(f32::from(scroll.y)),
            view_mode: Some(Self::session_view_mode(self.view_mode).to_owned()),
            split_ratio: Some(self.split_pane_ratio.clamp(0.1, 0.9)),
            markdown_fold: markdown_folds.first().cloned(),
            markdown_folds,
            table_layout,
            forward,
            back,
        }
    }

    fn pane_history_value(entry: &HistoryEntry) -> Value {
        let (revision, length) = match &entry.source {
            HistorySource::Snapshot(snapshot) => (Some(snapshot.revision().get()), snapshot.len()),
            HistorySource::Materialized(source) => (None, source.len()),
        };
        let selection = Self::pane_selection_value(entry.selection);
        json!({
            "revision": revision,
            "length": length,
            "selection": selection,
            "kind": format!("{:?}", entry.kind),
        })
    }

    fn pane_selection_history_value(selection: UndoSelectionSnapshot) -> Value {
        json!({
            "revision": serde_json::Value::Null,
            "length": 0,
            "selection": Self::pane_selection_value(selection),
            "kind": "virtual_selection",
        })
    }

    fn pane_selection_value(selection: UndoSelectionSnapshot) -> Value {
        let selection = selection.source_selection();
        let range = selection.range();
        json!({
            "start": range.start,
            "end": range.end,
            "reversed": selection.reversed(),
            "anchor_affinity": format!("{:?}", selection.anchor.affinity),
            "head_affinity": format!("{:?}", selection.head.affinity),
        })
    }

    /// 正文后端仍是 dirty 真值；尚未发布的视图文字必须额外阻止保存成功和无提示关闭。
    pub(crate) fn is_document_dirty(&self) -> bool {
        if self.document.source_commit_error().is_some() {
            return true;
        }
        let dirty = if self.document_host.is_some() {
            self.document_dirty
        } else {
            self.source_document.is_dirty()
        };
        #[cfg(test)]
        {
            // 旧 UI fixture 可能只设置边沿缓存；生产构建不会把该缓存作为正文真值。
            dirty || self.document_dirty
        }
        #[cfg(not(test))]
        dirty
    }

    #[cfg(test)]
    pub(crate) fn set_document_dirty_for_test(&mut self, dirty: bool) {
        self.source_document.set_dirty_for_test(dirty);
        self.document_dirty = self
            .source_document
            .try_is_dirty()
            .unwrap_or(self.document_dirty);
    }

    pub(in crate::editor) fn dismiss_tab_context_menu(&mut self) -> bool {
        let dismissed = self.tabs.context_menu.take().is_some();
        if dismissed {
            self.context_menu_keyboard_item = None;
            self.context_menu_keyboard_submenu_item = None;
        }
        dismissed
    }

    pub(in crate::editor) fn dismiss_new_tab_menu(&mut self) -> bool {
        self.tabs.new_tab_menu.take().is_some()
    }

    pub(in crate::editor) fn open_new_tab_menu(
        &mut self,
        position: Point<Pixels>,
        pane: Option<crate::editor::panes::PaneId>,
        cx: &mut Context<Self>,
    ) {
        self.tabs.context_menu = None;
        self.tabs.split_pane_menu = None;
        self.tabs.new_tab_menu = Some(NewTabMenu { position, pane });
        cx.notify();
    }

    pub(in crate::editor) fn dismiss_split_pane_menu(&mut self) -> bool {
        self.tabs.split_pane_menu.take().is_some()
    }

    pub(in crate::editor) fn open_split_pane_menu(
        &mut self,
        position: Point<Pixels>,
        window: Option<&mut Window>,
        cx: &mut Context<Self>,
    ) {
        self.tabs.context_menu = None;
        self.tabs.new_tab_menu = None;
        let focus_handle = cx.focus_handle();
        if let Some(window) = window {
            let deferred_focus = focus_handle.clone();
            window.defer(cx, move |window, _cx| deferred_focus.focus(window));
        }
        self.tabs.split_pane_menu = Some(SplitPaneMenu {
            position,
            focus_handle,
        });
        cx.notify();
    }

    #[cfg(test)]
    pub(in crate::editor) fn split_pane_menu_is_focused_for_test(
        &self,
        window: &mut Window,
    ) -> bool {
        self.tabs
            .split_pane_menu
            .as_ref()
            .is_some_and(|menu| menu.focus_handle.is_focused(window))
    }

    #[cfg(test)]
    pub(in crate::editor) fn has_new_or_split_menu_for_test(&self) -> bool {
        self.tabs.has_new_or_split_menu()
    }

    pub(in crate::editor) fn tab_context_menu_info(&self) -> Option<(usize, bool, bool)> {
        let menu = self.tabs.context_menu.as_ref()?;
        let pinned = self
            .tabs
            .records
            .get(menu.index)
            .is_some_and(|record| record.pinned);
        Some((menu.index, pinned, self.tabs.records.len() > 1))
    }

    /// Canonical v10 capture used by all production writes.
    pub(in crate::editor) fn workspace_session_snapshot_result(
        &self,
        cx: &App,
    ) -> anyhow::Result<crate::config::workspace_session::WorkspaceSession> {
        self.capture_active_pane_view_states(cx);
        let Some(workspace_entity) = self.pane_workspace.as_ref() else {
            // The ordinary single-document editor intentionally keeps the
            // pane workspace lazy.  Persist the same canonical v10 shape
            // without mounting a pane Entity or moving the active session;
            // split/restore actions will materialize the runtime workspace
            // when needed.
            let pane_uuid = self
                .tabs
                .records
                .first()
                .map(|record| record.id)
                .filter(|id| !id.is_nil())
                .unwrap_or(self.tabs.session_id);
            let pane_id = crate::config::workspace_session::WorkspaceSessionPaneId::from(pane_uuid);
            let mut tabs = Vec::with_capacity(self.tabs.records.len());
            let mut active_tab = None;
            for (index, record) in self.tabs.records.iter().enumerate() {
                let snapshot = if index == self.tabs.active {
                    self.legacy_workspace_session_tab(record.pinned, cx)?
                } else {
                    let Some(snapshot) = record.snapshot.as_ref() else {
                        continue;
                    };
                    Self::snapshot_workspace_session_tab(record, snapshot, cx)?
                };
                if index == self.tabs.active {
                    active_tab = Some(snapshot.id);
                }
                tabs.push(snapshot);
            }
            let mut session = crate::config::workspace_session::WorkspaceSession::single_pane(
                self.tabs.session_id,
                self.explicit_workspace_root(),
            );
            session.root =
                crate::config::workspace_session::WorkspaceSessionPaneNode::Leaf(pane_id);
            session.focused_pane = pane_id;
            session.panes.clear();
            session.panes.insert(
                pane_id,
                crate::config::workspace_session::WorkspaceSessionPane::new(tabs, active_tab),
            );
            self.apply_workspace_session_window_state(&mut session);
            return Ok(session);
        };
        let workspace = workspace_entity.read(cx).workspace();
        let mut panes = std::collections::BTreeMap::new();
        let root = Self::snapshot_pane_node(self, workspace.root(), workspace, &mut panes, cx)?;
        let focused_pane = crate::config::workspace_session::WorkspaceSessionPaneId::from(
            workspace.focused_pane().as_uuid(),
        );
        if !panes.contains_key(&focused_pane) {
            return Err(anyhow::anyhow!(
                "pane workspace focus references a missing pane"
            ));
        }

        let mut session = crate::config::workspace_session::WorkspaceSession::single_pane(
            self.tabs.session_id,
            self.explicit_workspace_root(),
        );
        session.root = root;
        session.panes = panes;
        session.focused_pane = focused_pane;
        self.apply_workspace_session_window_state(&mut session);
        Ok(session)
    }

    /// Synchronize mounted active canvases into the pure tab snapshots before
    /// persistence/close inventory reads. This does not detach or mutate an
    /// Entity and therefore preserves one active canvas per leaf.
    pub(crate) fn capture_active_pane_view_states(&self, cx: &App) {
        let Some(workspace_entity) = self.pane_workspace.as_ref() else {
            return;
        };
        let workspace = workspace_entity.read(cx).workspace();
        let canvases = self.pane_canvas_entities.borrow();
        for (pane_id, (tab_id, view_id, canvas)) in canvases.iter() {
            let Some(tab) = workspace.pane(*pane_id).and_then(|pane| pane.tab(*tab_id)) else {
                continue;
            };
            if tab.view().view_id() != *view_id {
                continue;
            }
            let snapshot = match canvas {
                crate::editor::panes::PaneCanvasEntity::Markdown(entity) => {
                    entity.read(cx).pane_view_state_snapshot(cx)
                }
                crate::editor::panes::PaneCanvasEntity::DocumentHost(entity) => {
                    entity.read(cx).pane_view_state_snapshot(cx)
                }
                crate::editor::panes::PaneCanvasEntity::ReadOnly(_) => {
                    tab.view().view_state_snapshot()
                }
            };
            tab.view().set_view_state_snapshot(snapshot);
        }
    }
}
