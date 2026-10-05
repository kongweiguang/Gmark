// @author kongweiguang

//! Resolves the stable native input target and its UTF-8 source coordinates.

use std::borrow::Cow;

use gpui::Window;

use super::{Block, BlockImeCompositionOwner, BlockImeOriginalSelection};

impl Block {
    /// Resolves the active focus target without changing the composition owner.
    pub(super) fn focused_ime_owner(&self, window: &Window) -> Option<BlockImeCompositionOwner> {
        if self.math_source_focus_handle.is_focused(window) {
            return Some(BlockImeCompositionOwner::MathSource);
        }
        if self.math_structure_focus_handle.is_focused(window) {
            return self
                .math_input_context()
                .map(|(slot, _)| BlockImeCompositionOwner::MathSlot(slot));
        }
        if self.code_language_focus_handle.is_focused(window) {
            return Some(BlockImeCompositionOwner::CodeLanguage);
        }
        Some(BlockImeCompositionOwner::BlockText)
    }

    /// Routes delayed native callbacks to the pinned owner after focus has moved.
    pub(super) fn current_ime_owner(&self, window: &Window) -> Option<BlockImeCompositionOwner> {
        self.ime_composition_owner()
            .or_else(|| self.focused_ime_owner(window))
    }

    /// Provides the model text for one input surface, refusing stale formula slots.
    pub(super) fn ime_base_text(&self, owner: &BlockImeCompositionOwner) -> Option<String> {
        self.ime_base_text_view(owner).map(Cow::into_owned)
    }

    /// Borrows large ordinary surfaces while materializing only derived math input text.
    pub(super) fn ime_base_text_view<'a>(
        &'a self,
        owner: &BlockImeCompositionOwner,
    ) -> Option<Cow<'a, str>> {
        match owner {
            BlockImeCompositionOwner::BlockText => Some(Cow::Borrowed(self.display_text())),
            BlockImeCompositionOwner::CodeLanguage => {
                Some(Cow::Borrowed(self.code_language_text()))
            }
            BlockImeCompositionOwner::MathSource => Some(Cow::Owned(self.math_source_text())),
            BlockImeCompositionOwner::MathSlot(slot) => self
                .math_input_context()
                .filter(|(active, _)| active == slot)
                .map(|(_, text)| Cow::Owned(text)),
        }
    }

    /// Captures normalized selection direction in the owner's UTF-8 coordinate space.
    pub(super) fn ime_selection_snapshot(
        &self,
        owner: &BlockImeCompositionOwner,
    ) -> Option<BlockImeOriginalSelection> {
        let text_len = if matches!(owner, BlockImeCompositionOwner::MathSlot(_)) {
            self.ime_base_text(owner)?.len()
        } else {
            0
        };
        self.ime_selection_snapshot_with_text_len(owner, text_len)
    }

    /// Reuses the resolved input length; compact Source rows take their visible shared-selection overlay as the IME range.
    pub(super) fn ime_selection_snapshot_with_text_len(
        &self,
        owner: &BlockImeCompositionOwner,
        text_len: usize,
    ) -> Option<BlockImeOriginalSelection> {
        let (range, reversed) = match owner {
            BlockImeCompositionOwner::BlockText if self.compact_source_host() => (
                self.editor_selection_range
                    .clone()
                    .unwrap_or_else(|| self.selected_range.clone()),
                self.selection_reversed,
            ),
            BlockImeCompositionOwner::BlockText => {
                (self.selected_range.clone(), self.selection_reversed)
            }
            BlockImeCompositionOwner::CodeLanguage => (
                self.code_language_selected_range.clone(),
                self.code_language_selection_reversed,
            ),
            BlockImeCompositionOwner::MathSource => self.math_source_selection(),
            BlockImeCompositionOwner::MathSlot(slot) => {
                let session = self.math_edit_session.as_ref()?;
                Self::math_selection_range(session, slot, text_len)
            }
        };
        Some(BlockImeOriginalSelection { range, reversed })
    }
}
