// @author kongweiguang

//! Small, identity-bound UI intents that must wait for an IME terminal event.

use super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum ToolImeIntent {
    FindCycle(bool),
    FindFocus(find_replace::FindKeyboardTarget),
    FindReplace {
        all: bool,
        query: EntityId,
        replacement: EntityId,
    },
    FindClose,
    ResourceTitleOpen {
        entity_id: EntityId,
        previous: crate::components::ResourceRecord,
    },
    ResourceTitleConfirm {
        entity_id: EntityId,
        input_id: EntityId,
        previous: crate::components::ResourceRecord,
    },
    ResourceTitleCancel {
        entity_id: EntityId,
        input_id: EntityId,
        previous: crate::components::ResourceRecord,
    },
    PaletteOpen,
    PaletteClose,
}

impl Editor {
    /// Keeps the current input entity mounted and focused until the platform resolves its composition.
    pub(in crate::editor) fn defer_tool_ime_intent(
        &mut self,
        intent: ToolImeIntent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.has_active_ime_composition(cx) {
            return false;
        }
        let tab = Some(self.tabs.active_id());
        let retry = self.ime_completion_failed
            && self.pending_ime_operations.back().is_some_and(|pending| {
                matches!(pending,
                    ime_lifecycle::DeferredImeOperation::ToolFocus {
                        tab: previous_tab,
                        intent: previous_intent,
                    } if *previous_tab == tab && previous_intent == &intent)
            });
        if self.ime_completion_failed {
            self.ime_completion_failed = false;
            self.ime_completion_requested = false;
        }
        if !retry {
            self.queue_ime_operation(
                ime_lifecycle::DeferredImeOperation::ToolFocus { tab, intent },
                cx,
            );
        }
        let _ = self.wait_for_ime_completion(window, cx);
        true
    }

    /// Pins replacement to the two Find fields that owned the candidate being composed.
    pub(in crate::editor) fn defer_find_replace_for_ime(
        &mut self,
        all: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(state) = self.find_panel.as_ref() else {
            return false;
        };
        let intent = ToolImeIntent::FindReplace {
            all,
            query: state.query.entity_id(),
            replacement: state.replacement.entity_id(),
        };
        self.defer_tool_ime_intent(intent, window, cx)
    }

    /// Defers only toolbar actions that transfer focus into an editor-owned text surface.
    pub(in crate::editor) fn defer_document_toolbar_action_for_ime(
        &mut self,
        action: render::DocumentToolbarAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let intent = match action {
            render::DocumentToolbarAction::Find => ToolImeIntent::FindCycle(false),
            render::DocumentToolbarAction::CommandPalette => ToolImeIntent::PaletteOpen,
            render::DocumentToolbarAction::SplitPane | render::DocumentToolbarAction::QuickOpen => {
                return false;
            }
        };
        self.defer_tool_ime_intent(intent, window, cx)
    }

    /// Replays the focused UI request only after the original input owner reports a terminal event.
    pub(in crate::editor) fn replay_tool_ime_intent(
        &mut self,
        intent: ToolImeIntent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match intent {
            ToolImeIntent::FindCycle(show_replace) => {
                if show_replace {
                    self.on_replace_in_document_action(
                        &crate::components::ReplaceInDocument,
                        window,
                        cx,
                    );
                } else {
                    self.on_find_in_document_action(&crate::components::FindInDocument, window, cx);
                }
            }
            ToolImeIntent::FindFocus(target) => {
                self.focus_find_keyboard_target(target, window, cx);
            }
            ToolImeIntent::FindReplace {
                all,
                query,
                replacement,
            } => {
                let fields_match = self.find_panel.as_ref().is_some_and(|state| {
                    state.query.entity_id() == query && state.replacement.entity_id() == replacement
                });
                if fields_match {
                    if all {
                        self.replace_all_find_matches(window, cx);
                    } else {
                        self.replace_current_find_match(window, cx);
                    }
                }
            }
            ToolImeIntent::FindClose => self.close_find_panel(window, cx),
            ToolImeIntent::ResourceTitleOpen {
                entity_id,
                previous,
            } => self.request_resource_title_dialog(entity_id, previous, window, cx),
            ToolImeIntent::ResourceTitleConfirm {
                entity_id,
                input_id,
                previous,
            } => {
                self.request_resource_title_confirmation(entity_id, input_id, &previous, window, cx)
            }
            ToolImeIntent::ResourceTitleCancel {
                entity_id,
                input_id,
                previous,
            } => {
                self.request_resource_title_cancellation(entity_id, input_id, &previous, window, cx)
            }
            ToolImeIntent::PaletteOpen => {
                self.on_command_palette_action(&crate::components::CommandPalette, window, cx);
            }
            ToolImeIntent::PaletteClose => {
                self.request_command_palette_close(window, cx);
            }
        }
    }

    /// Leaves the palette input mounted when an outside click arrives during pre-edit.
    pub(super) fn request_command_palette_close(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.defer_tool_ime_intent(ToolImeIntent::PaletteClose, window, cx) {
            return true;
        }
        let dismissed = self.dismiss_command_palette();
        if dismissed {
            cx.notify();
        }
        dismissed
    }
}
