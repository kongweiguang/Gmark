// @author kongweiguang

//! A document-only child entity used by [`super::PaneWorkspaceView`].
//!
//! The root [`Editor`] remains responsible for the window shell, menus, side
//! bars, tabs, and close lifecycle. Each active pane receives one child Editor
//! entity whose render path is explicitly reduced to `render_document_content`.
//! The child session is a short-lived adapter over the pane tab's shared lease
//! and view identity; it never owns a second source body or a duplicate
//! filesystem watcher.

use std::path::PathBuf;

use gmark_paged_document::OpenProbe;
use gpui::{
    AppContext, Context, Entity, InteractiveElement, IntoElement, ParentElement, Render, Styled,
    Window, div,
};

use crate::document_host::{DetachedDocumentHostView, DocumentHost};
use crate::editor::Editor;
use crate::editor::document_session::EditorDocumentSession;

/// Mounted content entity for one active pane tab.
pub struct PaneEditorCanvas {
    editor: Entity<Editor>,
    initial_layout_refresh_requested: bool,
}

impl PaneEditorCanvas {
    pub(in crate::editor) fn new(
        cx: &mut Context<Self>,
        session: EditorDocumentSession,
        file_path: Option<PathBuf>,
        pane_tab_id: uuid::Uuid,
        view_state: crate::editor::panes::PaneViewStateSnapshot,
    ) -> Self {
        let editor = cx.new(move |cx| {
            Editor::from_pane_session(cx, session, file_path, pane_tab_id, view_state)
        });
        Self {
            editor,
            initial_layout_refresh_requested: false,
        }
    }

    /// 渲染逐帧同步模式；模式未变时不重走切换收尾，保留所属窗格的浮层和输入状态。
    pub(in crate::editor) fn set_view_mode(
        &mut self,
        mode: crate::editor::ViewMode,
        cx: &mut Context<Self>,
    ) {
        if self.editor.read(cx).view_mode == mode {
            return;
        }
        self.editor
            .update(cx, |editor, cx| editor.set_view_mode(mode, cx));
    }

    /// 焦点与标题共用活动窗格身份；切回来时必须重新发布标题，不能沿用兄弟窗格的窗口状态。
    pub(in crate::editor) fn set_focus_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.editor.update(cx, |editor, cx| {
            let changed = editor.pane_canvas_focus_enabled != enabled;
            editor.pane_canvas_focus_enabled = enabled;
            if enabled {
                if changed {
                    editor.pending_window_title_refresh = true;
                }
                if changed && editor.pending_focus.is_none() {
                    editor.pending_focus = editor
                        .active_entity_id
                        .filter(|entity_id| editor.focusable_entity_by_id(*entity_id).is_some())
                        .or_else(|| editor.first_focusable_entity_id(cx));
                }
            } else {
                // Projection publication may enqueue focus after this pane lost
                // activation. Drop it so a later-painted sibling cannot replace
                // the active pane's platform input handler.
                editor.pending_focus = None;
            }
            if changed {
                cx.notify();
            }
        });
    }

    pub(in crate::editor) fn editor(&self) -> Entity<Editor> {
        self.editor.clone()
    }

    pub(crate) fn pane_view_state_snapshot(
        &self,
        cx: &gpui::App,
    ) -> crate::editor::panes::PaneViewStateSnapshot {
        self.editor.read(cx).pane_view_state_snapshot(cx)
    }

    pub(crate) fn accessibility_snapshot(
        &self,
        cx: &gpui::App,
    ) -> crate::accessibility::EditorAccessibilitySnapshot {
        self.editor.read(cx).accessibility_snapshot(cx)
    }

    pub(crate) fn accessibility_revision(&self, cx: &gpui::App) -> u64 {
        self.editor.read(cx).current_accessibility_revision(cx)
    }

    pub(crate) fn fork_view(
        &self,
        cx: &gpui::App,
    ) -> Result<EditorDocumentSession, crate::editor::document_session::EditorDocumentSessionError>
    {
        self.editor.read(cx).source_document.fork_view()
    }

    pub(crate) fn close_state(
        &self,
        cx: &gpui::App,
    ) -> Option<(gmark_document_runtime::DocumentId, bool, usize)> {
        let editor = self.editor.read(cx);
        let document_id = editor.source_document.document_id().ok()?;
        Some((
            document_id,
            editor.is_document_dirty(),
            editor.source_document.lease_count(),
        ))
    }

    /// Clear the pane editor's recovery snapshot after the shared document
    /// was discarded by the window-close coordinator.  Every mounted view has
    /// its own recovery journal, so a shared document must update all canvases.
    pub(crate) fn checkpoint_discarded_recovery(&self, cx: &mut Context<Self>) {
        self.editor.update(cx, |editor, _cx| {
            editor.checkpoint_recovery_journal();
            editor.document_dirty = false;
        });
    }
}

impl Render for PaneEditorCanvas {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl gpui::IntoElement {
        if !self.initial_layout_refresh_requested {
            self.initial_layout_refresh_requested = true;
            let editor = self.editor.clone();
            window.defer(cx, move |_window, cx| {
                editor.update(cx, |_editor, cx| cx.notify());
            });
        }
        // The child Editor owns the real block/entity tree and renders the
        // solid editor surface. This wrapper deliberately adds no duplicate
        // title, menu, sidebar, or projection work.
        div()
            .id("pane-editor-canvas")
            .w_full()
            .h_full()
            .min_w(gpui::px(0.0))
            .min_h(gpui::px(0.0))
            .child(self.editor.clone())
            .into_any_element()
    }
}

/// Mounted canvas for one active source-backed `DocumentHost` tab.  The
/// detached host token is moved into the child Entity exactly once; dropping
/// this canvas after deactivation is preceded by `detach_view` in the editor
/// lifecycle synchronizer.
pub struct PaneDocumentHostCanvas {
    host: Entity<DocumentHost>,
    focus_enabled: bool,
    published_window_title: Option<String>,
}

impl PaneDocumentHostCanvas {
    /// Host 通知同时刷新外层标题；正文仍只由所属实体绘制，不建立第二套文档状态。
    pub(in crate::editor) fn new(
        cx: &mut Context<Self>,
        path: PathBuf,
        probe: OpenProbe,
        detached: DetachedDocumentHostView,
    ) -> Self {
        let host = cx.new(move |cx| DocumentHost::from_detached(path, probe, detached, cx));
        cx.observe(&host, |_canvas, _host, cx| cx.notify()).detach();
        Self {
            host,
            focus_enabled: false,
            published_window_title: None,
        }
    }

    /// 标题归属沿用活动窗格身份；重新激活时强制发布，防止保留兄弟 Markdown 的标题。
    pub(in crate::editor) fn set_focus_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.focus_enabled == enabled {
            return;
        }
        self.focus_enabled = enabled;
        self.published_window_title = None;
        cx.notify();
    }

    pub fn host(&self) -> Entity<DocumentHost> {
        self.host.clone()
    }

    pub(crate) fn close_state(
        &self,
        cx: &gpui::App,
    ) -> Option<(gmark_document_runtime::DocumentId, bool, usize)> {
        let host = self.host.read(cx);
        Some((host.document_id()?, host.is_dirty(), host.lease_count()))
    }

    pub(crate) fn pane_view_state_snapshot(
        &self,
        cx: &gpui::App,
    ) -> crate::editor::panes::PaneViewStateSnapshot {
        let host = self.host.read(cx);
        crate::editor::panes::host_presentation_to_pane_view_state(
            &host.view_presentation_snapshot(cx),
        )
    }

    pub(crate) fn accessibility_snapshot(
        &self,
        cx: &gpui::App,
    ) -> crate::accessibility::EditorAccessibilitySnapshot {
        self.host.read(cx).accessibility_snapshot(cx)
    }

    /// 与单窗使用同一输入表面身份，避免子窗格读屏内容滞后于候选或当前文字。
    pub(crate) fn accessibility_revision(&self, cx: &gpui::App) -> u64 {
        self.host.read(cx).accessibility_revision(cx)
    }

    pub(in crate::editor) fn detach(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Option<DetachedDocumentHostView> {
        self.host.update(cx, |host, cx| host.detach_view(cx))
    }
}

impl Render for PaneDocumentHostCanvas {
    /// 仅活动 Host 发布路径和 dirty，标题未变时跳过系统调用；后台窗格不得覆盖窗口状态。
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl gpui::IntoElement {
        if self.focus_enabled {
            let host = self.host.read(cx);
            let strings = cx.global::<crate::i18n::I18nManager>().strings_arc();
            let title = Editor::window_title(Some(host.path()), host.is_dirty(), &strings);
            if self.published_window_title.as_ref() != Some(&title) {
                window.set_window_title(&title);
                self.published_window_title = Some(title);
            }
        }
        div()
            .id("pane-document-host-canvas")
            .w_full()
            .h_full()
            .min_w(gpui::px(0.0))
            .min_h(gpui::px(0.0))
            .child(self.host.clone())
            .into_any_element()
    }
}
