// @author kongweiguang

//! Focus-owner routing for document navigation actions.

use super::super::Editor;
use crate::components::{
    DeleteLine, DuplicateLine, IndentBlock, LineOperation, MoveLineDown, MoveLineUp,
    MoveToDocumentEnd, MoveToDocumentStart, OutdentBlock, PageDown, PageUp, SelectDown,
    SelectPageDown, SelectPageUp, SelectToDocumentEnd, SelectToDocumentStart, SelectUp,
};
use gpui::{Context, Window};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::editor) enum EditorActionRoute {
    Local,
    FocusedEditor,
    DocumentHost,
    PassThrough,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::editor) enum FocusedNavigationAction {
    MoveToDocumentStart,
    MoveToDocumentEnd,
    SelectToDocumentStart,
    SelectToDocumentEnd,
    SelectUp,
    SelectDown,
    SelectPageUp,
    SelectPageDown,
    PageUp,
    PageDown,
}

impl Editor {
    /// Lets PageUp/PageDown move the focused caret first; callers retain scrolling as a fallback.
    pub(in crate::editor) fn move_focused_by_page(
        &mut self,
        direction: i32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.document_host.is_some() {
            return false;
        }
        let lines = self.page_line_count(window, cx);
        self.move_selection_by_visual_lines(direction, lines, false, window, cx)
    }

    /// Routes to the focused text owner; unhandled actions bubble so Host and utility controls can receive them.
    pub(in crate::editor) fn route_navigation_action_to_focused_owner(
        &mut self,
        action: FocusedNavigationAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> EditorActionRoute {
        if self.pane_canvas {
            return if self.focused_document_target(window, cx).is_some() {
                EditorActionRoute::Local
            } else {
                cx.propagate();
                EditorActionRoute::PassThrough
            };
        }

        if self.pane_workspace.is_some() {
            let (markdown, host) = self.focused_pane_entities(cx);
            if let Some(editor) = markdown {
                if !editor.read_with(cx, |editor, cx| {
                    editor.focused_document_target(window, cx).is_some()
                }) {
                    cx.propagate();
                    return EditorActionRoute::PassThrough;
                }
                editor.update(cx, |editor, cx| {
                    editor.dispatch_navigation_action_to_self(action, window, cx)
                });
                cx.stop_propagation();
                return EditorActionRoute::FocusedEditor;
            }
            if host.is_some() {
                cx.propagate();
                return EditorActionRoute::DocumentHost;
            }
            cx.propagate();
            return EditorActionRoute::PassThrough;
        }

        if self.document_host.is_some() {
            cx.propagate();
            return EditorActionRoute::DocumentHost;
        }
        if self.focused_document_target(window, cx).is_some() {
            EditorActionRoute::Local
        } else {
            cx.propagate();
            EditorActionRoute::PassThrough
        }
    }

    /// Reuses normal handlers after pane routing so IME gates remain the single action boundary.
    fn dispatch_navigation_action_to_self(
        &mut self,
        action: FocusedNavigationAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            FocusedNavigationAction::MoveToDocumentStart => {
                self.on_move_to_document_start(&MoveToDocumentStart, window, cx)
            }
            FocusedNavigationAction::MoveToDocumentEnd => {
                self.on_move_to_document_end(&MoveToDocumentEnd, window, cx)
            }
            FocusedNavigationAction::SelectToDocumentStart => {
                self.on_select_to_document_start(&SelectToDocumentStart, window, cx)
            }
            FocusedNavigationAction::SelectToDocumentEnd => {
                self.on_select_to_document_end(&SelectToDocumentEnd, window, cx)
            }
            FocusedNavigationAction::SelectUp => self.on_select_up(&SelectUp, window, cx),
            FocusedNavigationAction::SelectDown => self.on_select_down(&SelectDown, window, cx),
            FocusedNavigationAction::SelectPageUp => {
                self.on_select_page_up(&SelectPageUp, window, cx)
            }
            FocusedNavigationAction::SelectPageDown => {
                self.on_select_page_down(&SelectPageDown, window, cx)
            }
            FocusedNavigationAction::PageUp => self.on_page_up(&PageUp, window, cx),
            FocusedNavigationAction::PageDown => self.on_page_down(&PageDown, window, cx),
        }
    }

    /// Resolves the actual focused pane before row-editability gates run, so stale surface state cannot authorize a write.
    pub(in crate::editor) fn route_line_operation_to_focused_owner(
        &mut self,
        operation: LineOperation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> EditorActionRoute {
        if self.pane_canvas {
            if let Some((surface, _)) = self.focused_document_target(window, cx) {
                self.active_selection_surface = surface;
                return EditorActionRoute::Local;
            }
            cx.propagate();
            return EditorActionRoute::PassThrough;
        }

        if self.pane_workspace.is_some() {
            let (markdown, host) = self.focused_pane_entities(cx);
            if let Some(editor) = markdown {
                if !editor.read_with(cx, |editor, cx| {
                    editor.focused_document_target(window, cx).is_some()
                }) {
                    cx.propagate();
                    return EditorActionRoute::PassThrough;
                }
                editor.update(cx, |editor, cx| {
                    editor.dispatch_line_operation_to_self(operation, window, cx)
                });
                cx.stop_propagation();
                return EditorActionRoute::FocusedEditor;
            }
            if host.is_some() {
                cx.propagate();
                return EditorActionRoute::DocumentHost;
            }
            cx.propagate();
            return EditorActionRoute::PassThrough;
        }

        if self.document_host.is_some() {
            cx.propagate();
            return EditorActionRoute::DocumentHost;
        }
        if let Some((surface, _)) = self.focused_document_target(window, cx) {
            self.active_selection_surface = surface;
            return EditorActionRoute::Local;
        }
        cx.propagate();
        EditorActionRoute::PassThrough
    }

    /// Reuses normal handlers after pane routing so IME and editability gates stay authoritative.
    fn dispatch_line_operation_to_self(
        &mut self,
        operation: LineOperation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match operation {
            LineOperation::Duplicate => self.on_duplicate_line(&DuplicateLine, window, cx),
            LineOperation::Delete => self.on_delete_line(&DeleteLine, window, cx),
            LineOperation::MoveUp => self.on_move_line_up(&MoveLineUp, window, cx),
            LineOperation::MoveDown => self.on_move_line_down(&MoveLineDown, window, cx),
            LineOperation::Indent => self.on_indent_block(&IndentBlock, window, cx),
            LineOperation::Outdent => self.on_outdent_block(&OutdentBlock, window, cx),
        }
    }
}
