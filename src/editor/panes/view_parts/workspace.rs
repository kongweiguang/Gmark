// @author kongweiguang

//! Workspace entity state, active-view synchronization, and ratio input.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use gmark_document_runtime::DocumentId;
use gpui::{
    AppContext, Context, Entity, FocusHandle, KeyDownEvent, MouseDownEvent, MouseMoveEvent, Window,
};

use super::document::PaneDocumentRef;
use super::layout::{PaneDivider, PaneLayout, PaneViewport, compute_pane_layout};
use super::pane::{PaneContentFactory, PaneView};
use crate::editor::panes::{
    MAX_SPLIT_RATIO, MIN_SPLIT_RATIO, PaneId, PaneWorkspace, PaneWorkspaceController, SplitAxis,
};

#[derive(Clone, Debug)]
pub(super) struct RatioDrag {
    pub(super) path: Vec<bool>,
    pub(super) axis: SplitAxis,
    pub(super) start_pointer: f32,
    pub(super) start_ratio: f32,
    pub(super) span: f32,
    pub(super) changed: bool,
}

/// Recursive pane workspace entity.
///
/// The generic model is specialized to runtime document references here so
/// Editor integration receives a concrete, lease-safe API.  Command-like
/// actions are emitted through [`PaneWorkspaceController`]; only ratio drag
/// state is committed directly because the divider is owned by this entity.
pub struct PaneWorkspaceView {
    pub(super) workspace: PaneWorkspace<DocumentId, PaneDocumentRef>,
    pub(super) controller: PaneWorkspaceController,
    pub(super) viewport: PaneViewport,
    pub(super) layout: PaneLayout,
    pub(super) active_views: BTreeMap<PaneId, Entity<PaneView>>,
    pub(super) content_factories: BTreeMap<PaneId, PaneContentFactory>,
    pub(super) divider_focus_handles: BTreeMap<Vec<bool>, FocusHandle>,
    pub(super) new_tab_focus_handles: BTreeMap<PaneId, FocusHandle>,
    pub(super) split_focus_handles: BTreeMap<PaneId, FocusHandle>,
    pub(super) drag: Option<RatioDrag>,
}

impl fmt::Debug for PaneWorkspaceView {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PaneWorkspaceView")
            .field("workspace", &self.workspace)
            .field("viewport", &self.viewport)
            .field("layout", &self.layout)
            .field("active_view_count", &self.active_views.len())
            .field("drag", &self.drag)
            .finish()
    }
}

impl PaneWorkspaceView {
    pub fn new(
        workspace: PaneWorkspace<DocumentId, PaneDocumentRef>,
        controller: PaneWorkspaceController,
    ) -> Self {
        let focused = workspace.focused_pane();
        let layout = compute_pane_layout(workspace.root(), PaneViewport::default(), focused);
        Self {
            workspace,
            controller,
            viewport: PaneViewport::default(),
            layout,
            active_views: BTreeMap::new(),
            content_factories: BTreeMap::new(),
            divider_focus_handles: BTreeMap::new(),
            new_tab_focus_handles: BTreeMap::new(),
            split_focus_handles: BTreeMap::new(),
            drag: None,
        }
    }

    pub fn workspace(&self) -> &PaneWorkspace<DocumentId, PaneDocumentRef> {
        &self.workspace
    }

    pub fn workspace_mut(&mut self) -> &mut PaneWorkspace<DocumentId, PaneDocumentRef> {
        &mut self.workspace
    }

    pub fn replace_workspace(&mut self, workspace: PaneWorkspace<DocumentId, PaneDocumentRef>) {
        self.workspace = workspace;
        self.active_views.clear();
        self.new_tab_focus_handles.clear();
        self.split_focus_handles.clear();
        self.drag = None;
    }

    /// Set or replace the renderable content for a pane's active tab.
    ///
    /// The factory is retained across tab switches for that pane and called
    /// only when the corresponding active `PaneView` renders.
    pub fn set_content_factory(&mut self, pane: PaneId, factory: Option<PaneContentFactory>) {
        if let Some(factory) = factory {
            self.content_factories.insert(pane, factory);
        } else {
            self.content_factories.remove(&pane);
        }
        self.active_views.remove(&pane);
    }

    pub fn viewport(&self) -> PaneViewport {
        self.viewport
    }

    /// The root editor supplies the actual content-column dimensions. Pane
    /// geometry must never be derived from the full OS window because docked
    /// sidebars, tab chrome, and status chrome are outside this entity.
    pub fn set_viewport(&mut self, viewport: PaneViewport) {
        self.viewport = PaneViewport::new(viewport.width.max(0.0), viewport.height.max(0.0));
    }

    pub fn layout(&self) -> &PaneLayout {
        &self.layout
    }

    /// Exposes the persistence notification to pane tests without making the
    /// controller field part of the view API.  Tests need to invoke it while
    /// this entity is being updated to prove the callback never re-enters the
    /// workspace, which is otherwise only observable in release builds because
    /// session capture is disabled under `cfg(test)`.
    #[cfg(test)]
    pub(in crate::editor::panes) fn notify_workspace_changed_for_test(&self, cx: &mut gpui::App) {
        self.controller.notify_workspace_changed(cx);
    }

    /// Number of active tab entities mounted in the latest render pass.
    pub fn mounted_view_count(&self) -> usize {
        self.active_views.len()
    }

    pub fn mounted_view(&self, pane: PaneId) -> Option<&Entity<PaneView>> {
        self.active_views.get(&pane)
    }

    pub(super) fn sync_active_views(&mut self, cx: &mut Context<Self>) {
        let desired = self
            .workspace
            .pane_ids()
            .into_iter()
            .filter_map(|pane| {
                let state = self.workspace.pane(pane)?;
                let tab = state.active_tab()?;
                Some((pane, tab.id(), *tab.document(), tab.view().clone()))
            })
            .collect::<Vec<_>>();
        let desired_panes = desired
            .iter()
            .map(|(pane, _, _, _)| *pane)
            .collect::<BTreeSet<_>>();
        self.active_views.retain(|pane, entity| {
            desired_panes.contains(pane) && entity.read(cx).pane_id() == *pane
        });

        for (pane, tab, document_id, document) in desired {
            let should_replace = self.active_views.get(&pane).is_none_or(|entity| {
                let view = entity.read(cx);
                view.tab_id() != tab
                    || view.document_id() != document_id
                    || view.view_id() != document.view_id()
            });
            if should_replace {
                let content_factory = self.content_factories.get(&pane).cloned();
                let entity = cx.new(|cx| {
                    if let Some(factory) = content_factory {
                        PaneView::with_content_factory(pane, tab, document, factory, cx)
                    } else {
                        PaneView::new(pane, tab, document, cx)
                    }
                });
                entity.update(cx, |view, _cx| view.set_controller(self.controller.clone()));
                self.active_views.insert(pane, entity);
            }
        }
    }

    pub(super) fn start_ratio_drag(
        &mut self,
        divider: &PaneDivider,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(focus) = self.divider_focus_handles.get(divider.path()) {
            focus.focus(window);
        }
        let start_pointer = match divider.axis() {
            SplitAxis::Horizontal => f32::from(event.position.x),
            SplitAxis::Vertical => f32::from(event.position.y),
        };
        self.drag = Some(RatioDrag {
            path: divider.path().to_vec(),
            axis: divider.axis(),
            start_pointer,
            start_ratio: divider.ratio(),
            span: divider.span().max(1.0),
            changed: false,
        });
        cx.notify();
        cx.stop_propagation();
    }

    /// Commits a live divider ratio without saving every pointer sample; the
    /// completed drag emits one persistence notification from `end_ratio_drag`.
    pub(super) fn update_ratio_drag(
        &mut self,
        event: &MouseMoveEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(drag) = self.drag.clone() else {
            return;
        };
        if !event.dragging() {
            return;
        }
        let pointer = match drag.axis {
            SplitAxis::Horizontal => f32::from(event.position.x),
            SplitAxis::Vertical => f32::from(event.position.y),
        };
        let delta = (pointer - drag.start_pointer) / drag.span;
        let next = (drag.start_ratio + delta).clamp(MIN_SPLIT_RATIO, MAX_SPLIT_RATIO);
        if self
            .workspace
            .set_split_ratio_at_path(&drag.path, next)
            .is_ok()
        {
            if let Some(active_drag) = self.drag.as_mut() {
                active_drag.changed = true;
            }
            cx.notify();
        }
        cx.stop_propagation();
    }

    /// Ends a divider drag and schedules one snapshot only when its ratio moved,
    /// avoiding a disk task for a click that did not change layout.
    pub(super) fn end_ratio_drag(&mut self, cx: &mut Context<Self>) {
        let drag = self.drag.take();
        let changed = drag.as_ref().is_some_and(|drag| drag.changed);
        if changed {
            self.controller.notify_workspace_changed(cx);
            cx.notify();
        } else if drag.is_some() {
            cx.notify();
        }
    }

    /// Persists keyboard ratio changes immediately because each key event is a
    /// complete user operation rather than a stream of drag samples.
    pub(super) fn adjust_ratio_from_key(
        &mut self,
        path: &[bool],
        axis: SplitAxis,
        event: &KeyDownEvent,
        cx: &mut Context<Self>,
    ) -> bool {
        let increase = match (axis, event.keystroke.key.as_str()) {
            (SplitAxis::Horizontal, "right") | (SplitAxis::Vertical, "down") => true,
            (SplitAxis::Horizontal, "left") | (SplitAxis::Vertical, "up") => false,
            _ => return false,
        };
        let shift = event.keystroke.modifiers.shift;
        if self
            .workspace
            .adjust_split_ratio_at_path(path, increase, shift)
            .is_ok()
        {
            self.controller.notify_workspace_changed(cx);
            cx.notify();
        }
        true
    }
}
