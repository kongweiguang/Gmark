// @author kongweiguang

use super::Block;
use crate::components::BlockEvent;
use crate::components::block::BlockImeComposition;
use crate::components::block::BlockImeCompositionOwner;
use crate::components::block::BlockImeInteraction;
use crate::components::block::BlockInputCommand;
use gpui::{Context, Window};
use std::ops::Range;

/// A one-shot owner-local edit map shared by pointer hits and queued input selections.
pub(crate) enum BlockImeInteractionRebase {
    /// The BlockText owner replaced this visible-text range with committed UTF-8 text.
    LocalReplacement {
        range: Range<usize>,
        committed_len: usize,
        base_revision: gmark_document::Revision,
    },
    /// The IME result replaced an editor-wide selection and has no local Block coordinate map.
    RejectLocalPointerIntent {
        base_revision: gmark_document::Revision,
    },
}

impl Block {
    /// Marks blocks whose focus-changing interactions must pass through Editor's IME queue.
    pub(crate) fn set_ime_interactions_managed(&mut self, managed: bool) {
        self.ime_interactions_managed = managed;
    }

    /// Keeps the composition owner available to rebase a parent surface's cross-block drag.
    pub(crate) fn set_ime_surface_selection_pending(&mut self, pending: bool) {
        self.ime_surface_selection_pending = pending;
    }

    /// Routes an interaction through Editor only when that owner can serialize its IME lifecycle.
    pub(crate) fn request_ime_interaction(
        &mut self,
        interaction: BlockImeInteraction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.ime_interactions_managed {
            match &interaction {
                BlockImeInteraction::PointerSelection { .. } => {
                    self.ime_pointer_selection_pending = true;
                }
                BlockImeInteraction::PointerSelectionEnd => {}
                BlockImeInteraction::PointerSelectionMove { .. } => {}
                BlockImeInteraction::MathPaletteCommand(_) => {}
                BlockImeInteraction::InputCommand { .. } => {}
                BlockImeInteraction::InlineCommand { .. } => {}
            }
            cx.emit(BlockEvent::RequestImeInteraction { interaction });
        } else {
            self.apply_ime_interaction(interaction, window, cx);
        }
    }

    /// Replays a stable intent after composition resolution without revisiting pointer hit testing.
    pub(crate) fn apply_ime_interaction(
        &mut self,
        interaction: BlockImeInteraction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match interaction {
            BlockImeInteraction::PointerSelection {
                clean_offset,
                click_count,
                shift,
            } => {
                let Some(clean_offset) = self.rebase_pointer_interaction_offset(clean_offset)
                else {
                    return;
                };
                self.focus_handle.focus(window);
                self.apply_pointer_selection_interaction(clean_offset, click_count, shift, cx);
            }
            BlockImeInteraction::PointerSelectionMove { clean_offset } => {
                if let Some(clean_offset) = self.rebase_pointer_interaction_offset(clean_offset) {
                    self.apply_pointer_selection_move_interaction(clean_offset, cx);
                }
            }
            BlockImeInteraction::PointerSelectionEnd => {
                self.ime_pointer_selection_pending = false;
                self.clear_ime_interaction_rebase_if_idle();
                self.is_selecting = false;
                self.pointer_selection = None;
            }
            BlockImeInteraction::MathPaletteCommand(command) => {
                if !self.is_read_only() && self.math_edit_session.is_some() {
                    self.math_structure_focus_handle.focus(window);
                    self.execute_math_palette_command(command, cx);
                }
            }
            BlockImeInteraction::InputCommand {
                command,
                selection,
                reversed,
                base_revision,
            } => {
                if self.restore_ime_command_selection(selection, reversed, base_revision) {
                    self.focus_handle.focus(window);
                    self.replay_input_command(command, window, cx);
                }
            }
            BlockImeInteraction::InlineCommand {
                command,
                selection,
                reversed,
                base_revision,
            } => {
                if self.restore_ime_command_selection(selection, reversed, base_revision) {
                    self.focus_handle.focus(window);
                    self.replay_inline_command(command, cx);
                }
            }
        }
    }

    /// Queues a rich-text command against its original selection until native preedit resolves.
    pub(crate) fn defer_inline_command_if_composing(
        &mut self,
        command: super::super::EditingCommandId,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.has_ime_composition() {
            return false;
        }
        let interaction = BlockImeInteraction::InlineCommand {
            command,
            selection: self.current_to_clean_range(self.selected_range.clone()),
            reversed: self.selection_reversed,
            base_revision: self.document_revision,
        };
        self.ime_input_command_pending = self.ime_input_command_pending.saturating_add(1);
        if self.ime_interactions_managed {
            cx.emit(BlockEvent::RequestImeInteraction { interaction });
        } else {
            self.ime_unmanaged_input_commands.push_back(interaction);
        }
        true
    }

    /// Replays only inline commands representable by Block's existing rich-text actions.
    fn replay_inline_command(
        &mut self,
        command: super::super::EditingCommandId,
        cx: &mut Context<Self>,
    ) {
        match command {
            super::super::EditingCommandId::Bold => {
                self.toggle_inline_format(super::InlineFormat::Bold, cx)
            }
            super::super::EditingCommandId::Italic => {
                self.toggle_inline_format(super::InlineFormat::Italic, cx)
            }
            super::super::EditingCommandId::Strikethrough => {
                self.toggle_inline_format(super::InlineFormat::Strikethrough, cx)
            }
            super::super::EditingCommandId::Underline => {
                self.toggle_inline_format(super::InlineFormat::Underline, cx)
            }
            super::super::EditingCommandId::Highlight => {
                self.toggle_inline_format(super::InlineFormat::Highlight, cx)
            }
            super::super::EditingCommandId::Superscript => {
                self.toggle_inline_format(super::InlineFormat::Superscript, cx)
            }
            super::super::EditingCommandId::Subscript => {
                self.toggle_inline_format(super::InlineFormat::Subscript, cx)
            }
            super::super::EditingCommandId::InlineCode => {
                self.toggle_inline_format(super::InlineFormat::Code, cx)
            }
            super::super::EditingCommandId::InlineMath => self.insert_inline_math(cx),
            super::super::EditingCommandId::Link => self.toggle_inline_link(cx),
            super::super::EditingCommandId::ClearFormatting => self.clear_inline_formatting(cx),
            _ => {}
        }
    }

    /// Queues document commands behind the native terminal event using clean-text selection coordinates.
    pub(crate) fn defer_input_command_if_composing(
        &mut self,
        command: BlockInputCommand,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.has_ime_composition() {
            return false;
        }

        let interaction = BlockImeInteraction::InputCommand {
            command,
            selection: self.current_to_clean_range(self.selected_range.clone()),
            reversed: self.selection_reversed,
            base_revision: self.document_revision,
        };
        self.ime_input_command_pending = self.ime_input_command_pending.saturating_add(1);
        if self.ime_interactions_managed {
            cx.emit(BlockEvent::RequestImeInteraction { interaction });
        } else {
            self.ime_unmanaged_input_commands.push_back(interaction);
        }
        true
    }

    /// Replays one local command and defers the next until its Changed event reaches the host.
    pub(crate) fn replay_unmanaged_input_commands(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.ime_interactions_managed || self.has_ime_composition() {
            return;
        }
        let Some(interaction) = self.ime_unmanaged_input_commands.pop_front() else {
            return;
        };
        self.apply_ime_interaction(interaction, window, cx);

        if !self.ime_unmanaged_input_commands.is_empty() {
            let block = cx.entity().downgrade();
            window.defer(cx, move |window, cx| {
                let _ = block.update(cx, |block, cx| {
                    if block.ime_interactions_managed || block.has_ime_composition() {
                        return;
                    }
                    block.update_unmanaged_input_command_snapshots();
                    block.replay_unmanaged_input_commands(window, cx);
                });
            });
        }
    }

    /// Advances later local commands to the text and caret produced by the preceding FIFO action.
    fn update_unmanaged_input_command_snapshots(&mut self) {
        let selection = self.current_to_clean_range(self.selected_range.clone());
        let reversed = self.selection_reversed;
        let base_revision = self.document_revision;
        for interaction in &mut self.ime_unmanaged_input_commands {
            match interaction {
                BlockImeInteraction::InputCommand {
                    selection: pending_selection,
                    reversed: pending_reversed,
                    base_revision: pending_revision,
                    ..
                }
                | BlockImeInteraction::InlineCommand {
                    selection: pending_selection,
                    reversed: pending_reversed,
                    base_revision: pending_revision,
                    ..
                } => {
                    *pending_selection = selection.clone();
                    *pending_reversed = reversed;
                    *pending_revision = base_revision;
                }
                _ => {}
            }
        }
    }

    /// Restores a queued command range only when the captured revision has a trustworthy local rebase.
    pub(crate) fn restore_ime_command_selection(
        &mut self,
        selection: Range<usize>,
        reversed: bool,
        base_revision: gmark_document::Revision,
    ) -> bool {
        self.ime_input_command_pending = self.ime_input_command_pending.saturating_sub(1);
        let rebased = match self.ime_interaction_rebase.as_ref() {
            Some(BlockImeInteractionRebase::LocalReplacement {
                base_revision: map_revision,
                ..
            }) => {
                if base_revision == *map_revision {
                    let exactly_one_local_commit = map_revision
                        .get()
                        .checked_add(1)
                        .is_some_and(|revision| revision == self.document_revision.get());
                    if self.document_revision != *map_revision && !exactly_one_local_commit {
                        None
                    } else {
                        match (
                            self.rebase_pointer_interaction_offset(selection.start),
                            self.rebase_pointer_interaction_offset(selection.end),
                        ) {
                            (Some(start), Some(end)) => Some(start..end),
                            _ => None,
                        }
                    }
                } else if base_revision == self.document_revision {
                    Some(selection)
                } else {
                    None
                }
            }
            Some(BlockImeInteractionRebase::RejectLocalPointerIntent {
                base_revision: map_revision,
            }) if base_revision == self.document_revision && base_revision != *map_revision => {
                Some(selection)
            }
            Some(BlockImeInteractionRebase::RejectLocalPointerIntent { .. }) => None,
            None if base_revision == self.document_revision => Some(selection),
            _ => None,
        };

        let Some(selection) = rebased else {
            self.clear_ime_interaction_rebase_if_idle();
            return false;
        };
        let selection = selection.start.min(selection.end)..selection.start.max(selection.end);
        if self
            .render_cache
            .visible_text()
            .get(selection.clone())
            .is_none()
        {
            self.clear_ime_interaction_rebase_if_idle();
            return false;
        }

        self.selected_range = self.clean_to_current_range(selection);
        self.selection_reversed = !self.selected_range.is_empty() && reversed;
        self.clear_vertical_motion();
        self.clear_ime_interaction_rebase_if_idle();
        true
    }

    /// Refreshes only command snapshots that the owner-local FIFO is about to execute next.
    pub(crate) fn refresh_ime_command_snapshot(&self, interaction: &mut BlockImeInteraction) {
        let selection = self.current_to_clean_range(self.selected_range.clone());
        let reversed = self.selection_reversed;
        let base_revision = self.document_revision;
        match interaction {
            BlockImeInteraction::InputCommand {
                selection: pending_selection,
                reversed: pending_reversed,
                base_revision: pending_revision,
                ..
            }
            | BlockImeInteraction::InlineCommand {
                selection: pending_selection,
                reversed: pending_reversed,
                base_revision: pending_revision,
                ..
            } => {
                *pending_selection = selection;
                *pending_reversed = reversed;
                *pending_revision = base_revision;
            }
            _ => {}
        }
    }

    /// Keeps the one-shot map until every queued command and pointer intent has finished replaying.
    pub(crate) fn clear_ime_interaction_rebase_if_idle(&mut self) {
        if self.ime_input_command_pending == 0
            && !self.ime_pointer_selection_pending
            && !self.ime_surface_selection_pending
        {
            self.ime_interaction_rebase = None;
        }
    }

    /// Captures a clean-text replacement map before commit rebuilds the current inline projection.
    pub(crate) fn record_ime_interaction_rebase(
        &mut self,
        composition: &BlockImeComposition,
        committed: bool,
    ) {
        self.ime_interaction_rebase = None;
        if !(self.ime_pointer_selection_pending
            || self.ime_surface_selection_pending
            || self.ime_input_command_pending > 0)
            || !committed
        {
            return;
        }
        if composition.replace_cross_block {
            self.ime_interaction_rebase =
                Some(BlockImeInteractionRebase::RejectLocalPointerIntent {
                    base_revision: composition.base_revision,
                });
            return;
        }
        if composition.owner != BlockImeCompositionOwner::BlockText
            || composition.committed_prefix == composition.baseline_fragment
            || self.is_read_only()
        {
            return;
        }
        let range = self.current_to_clean_range(composition.replacement_range.clone());
        self.ime_interaction_rebase = Some(BlockImeInteractionRebase::LocalReplacement {
            range,
            committed_len: composition.committed_prefix.len(),
            base_revision: composition.base_revision,
        });
    }

    /// Converts captured baseline clean-text offsets before the current projection maps them.
    pub(crate) fn rebase_pointer_interaction_offset(&self, clean_offset: usize) -> Option<usize> {
        match self.ime_interaction_rebase.as_ref() {
            Some(BlockImeInteractionRebase::RejectLocalPointerIntent { .. }) => None,
            Some(BlockImeInteractionRebase::LocalReplacement {
                range,
                committed_len,
                ..
            }) => {
                let start = range.start;
                let end = range.end;
                let offset = if clean_offset < start {
                    clean_offset
                } else if clean_offset > end {
                    clean_offset
                        .saturating_sub(end.saturating_sub(start))
                        .saturating_add(*committed_len)
                } else if clean_offset == start && start != end {
                    start
                } else {
                    start.saturating_add(*committed_len)
                };
                Some(offset)
            }
            None => Some(clean_offset),
        }
    }

    /// Distinguishes a captured local text replacement from both no map and an unsafe cross-block commit.
    pub(crate) fn has_local_pointer_interaction_rebase(&self) -> bool {
        matches!(
            self.ime_interaction_rebase.as_ref(),
            Some(BlockImeInteractionRebase::LocalReplacement { .. })
        )
    }

    /// Lets a host record movement while its queued mouse-down has not rendered yet.
    pub(crate) fn ime_pointer_selection_pending(&self) -> bool {
        self.ime_pointer_selection_pending
    }
}
