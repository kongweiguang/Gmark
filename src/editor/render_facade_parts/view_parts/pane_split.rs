// @author kongweiguang

// 把拆分、聚焦、移动和均衡动作集中管理，保持输入动作的现有顺序。

use super::*;

impl Editor {
    pub(super) fn split_pane_with_active_view(
        &mut self,
        workspace: &Entity<crate::editor::panes::PaneWorkspaceView>,
        pane: crate::editor::panes::PaneId,
        direction: crate::editor::panes::PaneSplitDirection,
        cx: &mut Context<Self>,
    ) -> Result<(), crate::editor::panes::PaneError> {
        let source_document = workspace
            .read(cx)
            .workspace()
            .pane(pane)
            .and_then(|state| state.active_tab())
            .map(|tab| tab.view().clone());
        let Some(source_document) = source_document else {
            workspace.update(cx, |workspace, _cx| {
                workspace.workspace_mut().split_toward(pane, direction)
            })?;
            return Ok(());
        };
        let (tab_id, document) = if source_document.kind()
            == crate::editor::panes::PaneDocumentKind::Markdown
        {
            let source_lease = source_document
                .lease_arc()
                .ok_or(crate::editor::panes::PaneError::IdCollision)?;
            let handle = source_lease.handle();
            let lease = Arc::new(handle.lease());
            let view_id = gmark_document_core::DocumentViewInstanceId::new();
            let document_id = source_document.document_id();
            let tab_id = crate::editor::panes::TabId::from_uuid(view_id.uuid());
            let document =
                crate::editor::panes::PaneDocumentRef::from_lease_arc_with_title_and_path(
                    document_id,
                    lease,
                    view_id,
                    source_document.display_title(),
                    source_document.path().cloned(),
                );
            document.set_view_state_snapshot(source_document.view_state_snapshot());
            (tab_id, document)
        } else if source_document.kind() == crate::editor::panes::PaneDocumentKind::DocumentHost {
            let Some(original) = source_document.take_detached_host() else {
                return Ok(());
            };
            let handle = original.handle().clone();
            let view_id = gmark_document_core::DocumentViewInstanceId::new();
            let fork =
                match crate::document_host::DetachedDocumentHostView::from_shared_with_view_id(
                    handle.clone(),
                    handle.lease(),
                    view_id,
                    original.presentation_snapshot(),
                ) {
                    Ok(fork) => fork,
                    Err(_) => {
                        let _ = source_document.put_detached_host(original);
                        return Err(crate::editor::panes::PaneError::IdCollision);
                    }
                };
            let _ = source_document.put_detached_host(original);
            let Some(path) = source_document.host_path().cloned() else {
                return Err(crate::editor::panes::PaneError::IdCollision);
            };
            let Some(probe) = source_document.host_probe().cloned() else {
                return Err(crate::editor::panes::PaneError::IdCollision);
            };
            let tab_id = crate::editor::panes::TabId::from_uuid(fork.view_id().uuid());
            let document = crate::editor::panes::PaneDocumentRef::from_detached_host(
                source_document.document_id(),
                fork,
                path,
                probe,
                source_document.title().to_owned(),
            );
            (tab_id, document)
        } else {
            let Some(readonly) = source_document.readonly_kind().cloned() else {
                return Err(crate::editor::panes::PaneError::IdCollision);
            };
            let view_id = gmark_document_core::DocumentViewInstanceId::new();
            let tab_id = crate::editor::panes::TabId::from_uuid(view_id.uuid());
            let document = match readonly {
                crate::editor::panes::PaneReadOnlyKind::Image { path } => {
                    crate::editor::panes::PaneDocumentRef::from_image(
                        source_document.document_id(),
                        path,
                        view_id,
                        source_document.title().to_owned(),
                    )
                }
                crate::editor::panes::PaneReadOnlyKind::Error { path, message } => {
                    crate::editor::panes::PaneDocumentRef::from_error(
                        source_document.document_id(),
                        path,
                        message,
                        view_id,
                        source_document.title().to_owned(),
                    )
                }
            };
            document.set_view_state_snapshot(source_document.view_state_snapshot());
            (tab_id, document)
        };
        let document_id = source_document.document_id();
        let new_pane = workspace.update(cx, |workspace, _cx| {
            workspace.workspace_mut().split_toward(pane, direction)
        })?;
        let insert_result = workspace.update(cx, |workspace, _cx| {
            workspace.workspace_mut().insert_tab(
                new_pane,
                crate::editor::panes::TabView::new(tab_id, document_id, document),
            )?;
            Ok(())
        });
        if let Err(error) = insert_result {
            // Splitting is user-visible state.  If the prepared view cannot be
            // attached, collapse the newly-created empty leaf so a failed
            // split never leaves behind a blank pane or changes the old tree.
            let _ = workspace.update(cx, |workspace, _cx| {
                workspace.workspace_mut().close_pane(new_pane)
            });
            return Err(error);
        }
        Ok(())
    }

    pub(super) fn copy_pane_tab_with_fork(
        &mut self,
        workspace: &Entity<crate::editor::panes::PaneWorkspaceView>,
        source: crate::editor::panes::PaneId,
        target: crate::editor::panes::PaneId,
        tab: crate::editor::panes::TabId,
        cx: &mut Context<Self>,
    ) -> Result<(), crate::editor::panes::PaneError> {
        let Some(source_document) = workspace
            .read(cx)
            .workspace()
            .tab(source, tab)
            .map(|tab| tab.view().clone())
        else {
            return Err(crate::editor::panes::PaneError::TabNotFound(tab));
        };
        let (new_tab, document) = if source_document.kind()
            == crate::editor::panes::PaneDocumentKind::Markdown
        {
            let source_lease = source_document
                .lease_arc()
                .ok_or(crate::editor::panes::PaneError::IdCollision)?;
            let handle = source_lease.handle();
            let lease = Arc::new(handle.lease());
            let view_id = gmark_document_core::DocumentViewInstanceId::new();
            let document_id = source_document.document_id();
            let new_tab = crate::editor::panes::TabId::from_uuid(view_id.uuid());
            let document =
                crate::editor::panes::PaneDocumentRef::from_lease_arc_with_title_and_path(
                    document_id,
                    lease,
                    view_id,
                    source_document.display_title(),
                    source_document.path().cloned(),
                );
            document.set_view_state_snapshot(source_document.view_state_snapshot());
            (new_tab, document)
        } else if source_document.kind() == crate::editor::panes::PaneDocumentKind::DocumentHost {
            let Some(original) = source_document.take_detached_host() else {
                return Err(crate::editor::panes::PaneError::TabNotFound(tab));
            };
            let handle = original.handle().clone();
            let view_id = gmark_document_core::DocumentViewInstanceId::new();
            let fork =
                match crate::document_host::DetachedDocumentHostView::from_shared_with_view_id(
                    handle.clone(),
                    handle.lease(),
                    view_id,
                    original.presentation_snapshot(),
                ) {
                    Ok(fork) => fork,
                    Err(_) => {
                        let _ = source_document.put_detached_host(original);
                        return Err(crate::editor::panes::PaneError::IdCollision);
                    }
                };
            let _ = source_document.put_detached_host(original);
            let Some(path) = source_document.host_path().cloned() else {
                return Err(crate::editor::panes::PaneError::IdCollision);
            };
            let Some(probe) = source_document.host_probe().cloned() else {
                return Err(crate::editor::panes::PaneError::IdCollision);
            };
            let new_tab = crate::editor::panes::TabId::from_uuid(fork.view_id().uuid());
            let document = crate::editor::panes::PaneDocumentRef::from_detached_host(
                source_document.document_id(),
                fork,
                path,
                probe,
                source_document.title().to_owned(),
            );
            (new_tab, document)
        } else {
            let Some(readonly) = source_document.readonly_kind().cloned() else {
                return Err(crate::editor::panes::PaneError::IdCollision);
            };
            let view_id = gmark_document_core::DocumentViewInstanceId::new();
            let new_tab = crate::editor::panes::TabId::from_uuid(view_id.uuid());
            let document = match readonly {
                crate::editor::panes::PaneReadOnlyKind::Image { path } => {
                    crate::editor::panes::PaneDocumentRef::from_image(
                        source_document.document_id(),
                        path,
                        view_id,
                        source_document.title().to_owned(),
                    )
                }
                crate::editor::panes::PaneReadOnlyKind::Error { path, message } => {
                    crate::editor::panes::PaneDocumentRef::from_error(
                        source_document.document_id(),
                        path,
                        message,
                        view_id,
                        source_document.title().to_owned(),
                    )
                }
            };
            document.set_view_state_snapshot(source_document.view_state_snapshot());
            (new_tab, document)
        };
        let document_id = source_document.document_id();
        workspace.update(cx, |workspace, _cx| {
            workspace.workspace_mut().insert_tab(
                target,
                crate::editor::panes::TabView::new(new_tab, document_id, document),
            )
        })?;
        Ok(())
    }

    pub(super) fn drain_pane_events(&mut self, cx: &mut Context<Self>) {
        let events = std::mem::take(&mut *self.pane_events.borrow_mut());
        for event in events {
            self.handle_pane_event(event, None, cx);
        }
    }

    /// 短暂错误复用既有底栏反馈，不改变正文布局或夺取当前输入焦点。
    pub(in crate::editor) fn show_pane_notice(
        &mut self,
        message: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) {
        self.pane_notice = Some(message.into());
        let weak = cx.entity().downgrade();
        self.pane_notice_task = Some(cx.spawn(async move |_this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_secs(3))
                .await;
            let _ = weak.update(cx, |editor, cx| {
                editor.pane_notice = None;
                editor.pane_notice_task = None;
                cx.notify();
            });
        }));
        cx.notify();
    }

    pub(super) fn emit_pane_event(
        &mut self,
        event: crate::editor::panes::PaneEvent,
        cx: &mut Context<Self>,
    ) {
        self.pane_events.borrow_mut().push(event);
        self.drain_pane_events(cx);
    }

    pub(in crate::editor) fn split_pane_toward(
        &mut self,
        direction: crate::editor::panes::PaneSplitDirection,
        cx: &mut Context<Self>,
    ) {
        self.ensure_pane_workspace(cx);
        let Some(workspace) = self.pane_workspace.as_ref() else {
            return;
        };
        let pane = workspace.read(cx).workspace().focused_pane();
        self.emit_pane_event(
            crate::editor::panes::PaneEvent::Split { pane, direction },
            cx,
        );
    }

    pub(super) fn focused_pane_tab_and_target(
        &self,
        direction: crate::editor::panes::FocusDirection,
        cx: &App,
    ) -> Option<(
        crate::editor::panes::PaneId,
        crate::editor::panes::PaneId,
        crate::editor::panes::TabId,
    )> {
        let workspace = self.pane_workspace.as_ref()?.read(cx).workspace();
        let source = workspace.focused_pane();
        let tab = workspace.pane(source)?.active_tab_id()?;
        let target = workspace.adjacent_pane(source, direction).ok()?;
        Some((source, target, tab))
    }

    pub(crate) fn on_split_right_action(
        &mut self,
        _: &crate::components::SplitRight,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.split_pane_toward(crate::editor::panes::PaneSplitDirection::Right, cx);
    }

    pub(crate) fn on_split_down_action(
        &mut self,
        _: &crate::components::SplitDown,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.split_pane_toward(crate::editor::panes::PaneSplitDirection::Down, cx);
    }

    pub(crate) fn on_close_pane_action(
        &mut self,
        _: &crate::components::ClosePane,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.pane_workspace.as_ref() else {
            return;
        };
        let pane = workspace.read(cx).workspace().focused_pane();
        self.emit_pane_event(crate::editor::panes::PaneEvent::Close { pane }, cx);
    }

    pub(super) fn on_focus_pane_action(
        &mut self,
        direction: crate::editor::panes::FocusDirection,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.pane_workspace.as_ref() else {
            return;
        };
        let source = workspace.read(cx).workspace().focused_pane();
        self.emit_pane_event(
            crate::editor::panes::PaneEvent::FocusAdjacent {
                from: source,
                direction,
            },
            cx,
        );
    }

    pub(crate) fn on_focus_pane_left_action(
        &mut self,
        _: &crate::components::FocusPaneLeft,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.on_focus_pane_action(crate::editor::panes::FocusDirection::Left, cx);
    }

    pub(crate) fn on_focus_pane_right_action(
        &mut self,
        _: &crate::components::FocusPaneRight,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.on_focus_pane_action(crate::editor::panes::FocusDirection::Right, cx);
    }

    pub(crate) fn on_focus_pane_up_action(
        &mut self,
        _: &crate::components::FocusPaneUp,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.on_focus_pane_action(crate::editor::panes::FocusDirection::Up, cx);
    }

    pub(crate) fn on_focus_pane_down_action(
        &mut self,
        _: &crate::components::FocusPaneDown,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.on_focus_pane_action(crate::editor::panes::FocusDirection::Down, cx);
    }

    pub(super) fn on_move_tab_to_pane_action(
        &mut self,
        direction: crate::editor::panes::FocusDirection,
        cx: &mut Context<Self>,
    ) {
        let Some((source, target, tab)) = self.focused_pane_tab_and_target(direction, cx) else {
            return;
        };
        self.emit_pane_event(
            crate::editor::panes::PaneEvent::MoveTab {
                source,
                target,
                tab,
            },
            cx,
        );
    }

    pub(crate) fn on_move_tab_to_pane_left_action(
        &mut self,
        _: &crate::components::MoveTabToPaneLeft,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.on_move_tab_to_pane_action(crate::editor::panes::FocusDirection::Left, cx);
    }

    pub(crate) fn on_move_tab_to_pane_right_action(
        &mut self,
        _: &crate::components::MoveTabToPaneRight,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.on_move_tab_to_pane_action(crate::editor::panes::FocusDirection::Right, cx);
    }

    pub(crate) fn on_move_tab_to_pane_up_action(
        &mut self,
        _: &crate::components::MoveTabToPaneUp,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.on_move_tab_to_pane_action(crate::editor::panes::FocusDirection::Up, cx);
    }

    pub(crate) fn on_move_tab_to_pane_down_action(
        &mut self,
        _: &crate::components::MoveTabToPaneDown,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.on_move_tab_to_pane_action(crate::editor::panes::FocusDirection::Down, cx);
    }

    pub(crate) fn on_balance_panes_action(
        &mut self,
        _: &crate::components::BalancePanes,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.pane_workspace.is_some() {
            self.emit_pane_event(crate::editor::panes::PaneEvent::Balance, cx);
        }
    }
}
