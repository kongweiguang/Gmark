// @author kongweiguang

//! Events emitted by the pane view layer.
//!
//! The view owns input hit testing, focus, and drag-session lifetime.  It does
//! not own editor commands such as moving a tab or closing a document.  Those
//! commands are represented by [`PaneEvent`] and dispatched to the editor
//! through [`PaneWorkspaceController`].

use std::fmt;
use std::rc::Rc;

use gmark_document_runtime::DocumentId;
use gpui::App;

use super::{FocusDirection, PaneId, PaneSplitDirection, TabId};

/// Canonical, GPUI-free presentation snapshot retained by each pane tab.
///
/// The alias deliberately uses the versioned workspace-session DTO so the
/// pane model and session restore path cannot drift into two subtly different
/// selection/scroll/fold/table/history representations.
pub type PaneViewStateSnapshot = crate::config::workspace_session::WorkspaceSessionPaneViewState;

/// Drag payload for moving or copying the active tab between pane leaves.
/// The document identity is included for diagnostics/acceptance checks; the
/// model remains authoritative and rejects duplicates atomically.
#[derive(Clone, Debug)]
pub struct PaneTabDragPayload {
    pub source: PaneId,
    pub tab: TabId,
    pub document_id: DocumentId,
}

/// Close/quit inventory entry contributed by the recursive pane workspace.
/// It intentionally contains no Entity, body text, or host worker state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaneDocumentCloseState {
    pub document_id: DocumentId,
    pub dirty: bool,
    pub global_lease_count: usize,
    pub window_view_count: usize,
}

/// Commands and notifications that the editor integration layer can handle.
#[derive(Clone, Debug, PartialEq)]
pub enum PaneEvent {
    /// Open the new-document menu for one pane-local tab bar.
    OpenNewTabMenu { pane: PaneId, x: f32, y: f32 },
    /// Open the four-direction split menu for one pane-local tab bar.
    OpenSplitMenu { pane: PaneId, x: f32, y: f32 },
    /// Dismiss pane-local menus without changing focus or pane geometry.
    DismissMenus,
    /// Request a new pane in one of the four directions around the source.
    Split {
        pane: PaneId,
        direction: PaneSplitDirection,
    },
    /// Request that a pane be closed and its tabs merged by the model.
    Close { pane: PaneId },
    /// Request that one pane-local tab be closed without changing the
    /// pane's active tab before the lifecycle layer has accepted the close.
    /// Dirty-document prompting is deliberately handled by the root editor.
    CloseTab { pane: PaneId, tab: TabId },
    /// Activate one existing tab without rebuilding the pane tree.
    ActivateTab { pane: PaneId, tab: TabId },
    /// Request focus for a pane (including a hidden pane in compact layout).
    Focus { pane: PaneId },
    /// Request deterministic geometric focus movement.
    FocusAdjacent {
        from: PaneId,
        direction: FocusDirection,
    },
    /// Request a tab move.  The model performs duplicate-document checks.
    MoveTab {
        source: PaneId,
        target: PaneId,
        tab: TabId,
    },
    /// Request a tab copy.  The model allocates the new durable tab id.
    CopyTab {
        source: PaneId,
        target: PaneId,
        tab: TabId,
    },
    /// Request balancing all split ratios according to leaf counts.
    Balance,
}

/// Callback bridge from GPUI input handlers to the owning editor.
///
/// `Rc` is intentional: GPUI callbacks run on the UI thread and the editor
/// owns their lifetime.  No document body or runtime task crosses this bridge.
type PaneEventCallback = dyn Fn(PaneEvent, &mut gpui::Window, &mut App);
type PaneWorkspaceChangedCallback = dyn Fn(&mut App);

#[derive(Clone)]
pub struct PaneWorkspaceController {
    callback: Rc<PaneEventCallback>,
    workspace_changed: Rc<PaneWorkspaceChangedCallback>,
}

impl fmt::Debug for PaneWorkspaceController {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PaneWorkspaceController")
            .field("callback", &"<ui callback>")
            .finish()
    }
}

impl PaneWorkspaceController {
    /// Build a callback-backed controller.  The callback should forward the
    /// event to the editor's command executor, not mutate a detached copy.
    pub fn new(callback: impl Fn(PaneEvent, &mut gpui::Window, &mut App) + 'static) -> Self {
        Self {
            callback: Rc::new(callback),
            workspace_changed: Rc::new(|_| {}),
        }
    }

    /// Attaches persistence notification separately from command dispatch so
    /// divider changes can schedule a session snapshot without inventing a
    /// window-bound `PaneEvent` or mutating the pane model twice.
    pub fn with_workspace_changed(mut self, callback: impl Fn(&mut App) + 'static) -> Self {
        self.workspace_changed = Rc::new(callback);
        self
    }

    /// Notifies the owning editor after a ratio change has been committed.
    ///
    /// The notification is deferred because the persistence callback snapshots
    /// this same workspace entity.  Running it synchronously would read the
    /// entity while the divider handler still holds its update, which panics
    /// before the new ratio can be saved.  Deferring keeps the callback
    /// UI-thread-only while letting GPUI finish the current effect cycle.
    pub(super) fn notify_workspace_changed(&self, cx: &mut App) {
        let callback = self.workspace_changed.clone();
        cx.defer(move |cx| callback(cx));
    }

    /// Construct a no-op controller for isolated view tests and previews.
    pub fn noop() -> Self {
        Self::new(|_, _, _| {})
    }

    /// Dispatch an event to the integration layer.
    pub fn emit(&self, event: PaneEvent, window: &mut gpui::Window, cx: &mut App) {
        (self.callback)(event, window, cx);
    }

    pub fn focus(&self, pane: PaneId, window: &mut gpui::Window, cx: &mut App) {
        self.emit(PaneEvent::Focus { pane }, window, cx);
    }

    pub fn move_tab(
        &self,
        source: PaneId,
        target: PaneId,
        tab: TabId,
        window: &mut gpui::Window,
        cx: &mut App,
    ) {
        self.emit(
            PaneEvent::MoveTab {
                source,
                target,
                tab,
            },
            window,
            cx,
        );
    }

    pub fn copy_tab(
        &self,
        source: PaneId,
        target: PaneId,
        tab: TabId,
        window: &mut gpui::Window,
        cx: &mut App,
    ) {
        self.emit(
            PaneEvent::CopyTab {
                source,
                target,
                tab,
            },
            window,
            cx,
        );
    }
}
