// @author kongweiguang

//! Projects staged candidate text and ranges for rendering and GPUI queries.

use std::ops::Range;

use gmark_math_edit::{MathCursor2D, MathEditCommand, MathSelection};
use gpui::{SharedString, UTF16Selection};

use super::{Block, BlockImeComposition, BlockImeCompositionOwner};
use crate::editor::math_edit::MathEditSession;

impl Block {
    /// Returns the text a given input handler must expose to the operating system.
    pub(crate) fn ime_visible_text(&self, owner: &BlockImeCompositionOwner) -> Option<String> {
        let base = self.ime_base_text(owner)?;
        let Some(composition) = self
            .ime_composition
            .as_ref()
            .filter(|composition| &composition.owner == owner && !composition.result_committed)
        else {
            return Some(base);
        };
        if !composition.has_preedit_update && !composition.has_confirmed_result {
            return Some(base);
        }
        Self::splice_ime_text(&base, composition).or(Some(base))
    }

    /// Supplies temporary text to ordinary block layout without changing cached source.
    pub(crate) fn display_text_with_ime(&self) -> SharedString {
        self.ime_visible_text(&BlockImeCompositionOwner::BlockText)
            .unwrap_or_else(|| self.display_text().to_owned())
            .into()
    }

    /// Maps source styling ranges through the replacement so later runs do not drift.
    pub(crate) fn ime_visible_range(
        &self,
        owner: &BlockImeCompositionOwner,
        range: Range<usize>,
    ) -> Range<usize> {
        let Some(composition) = self.ime_composition.as_ref().filter(|composition| {
            &composition.owner == owner
                && !composition.result_committed
                && (composition.has_preedit_update || composition.has_confirmed_result)
        }) else {
            return range;
        };
        let Some(base_text) = self.ime_base_text_view(owner) else {
            return range;
        };
        if base_text
            .get(composition.replacement_range.clone())
            .is_none_or(|fragment| fragment != composition.baseline_fragment.as_str())
            || range.start > range.end
            || range.end > base_text.len()
            || !base_text.is_char_boundary(range.start)
            || !base_text.is_char_boundary(range.end)
        {
            return range;
        }
        let replacement = &composition.replacement_range;
        let inserted_len = composition
            .committed_prefix
            .len()
            .saturating_add(composition.preedit.len());
        let map_after = |offset: usize| {
            replacement
                .start
                .saturating_add(inserted_len)
                .saturating_add(offset.saturating_sub(replacement.end))
        };
        if range.end <= replacement.start {
            range
        } else if range.start >= replacement.end {
            map_after(range.start)..map_after(range.end)
        } else {
            let start = range.start.min(replacement.start);
            let end = if range.end > replacement.end {
                map_after(range.end)
            } else {
                replacement.start.saturating_add(inserted_len)
            };
            start..end
        }
    }

    /// Returns the virtual marked range used by shaping and native range queries.
    pub(crate) fn ime_marked_range(
        &self,
        owner: &BlockImeCompositionOwner,
    ) -> Option<Range<usize>> {
        let composition = self.ime_composition.as_ref()?;
        if &composition.owner != owner
            || composition.result_committed
            || composition.preedit.is_empty()
        {
            return None;
        }
        let start = composition
            .replacement_range
            .start
            .saturating_add(composition.committed_prefix.len());
        Some(start..start.saturating_add(composition.preedit.len()))
    }

    /// Provides a render-time selection in the virtual text coordinate space.
    pub(crate) fn ime_render_selection(
        &self,
        owner: &BlockImeCompositionOwner,
    ) -> Option<(Range<usize>, bool)> {
        let Some(composition) = self
            .ime_composition
            .as_ref()
            .filter(|composition| &composition.owner == owner && !composition.result_committed)
        else {
            return None;
        };
        let start = composition
            .replacement_range
            .start
            .saturating_add(composition.committed_prefix.len());
        if !composition.has_preedit_update && !composition.has_confirmed_result {
            return None;
        }
        let anchor = start.saturating_add(composition.selection_in_preedit.start);
        let focus = start.saturating_add(composition.selection_in_preedit.end);
        Some((anchor.min(focus)..anchor.max(focus), anchor > focus))
    }

    /// Builds a disposable math model so candidate glyphs and caret geometry follow preedit.
    pub(crate) fn math_ime_preview_session(&self) -> Option<MathEditSession> {
        let composition = self.ime_composition.as_ref()?;
        let BlockImeCompositionOwner::MathSlot(slot) = &composition.owner else {
            return None;
        };
        if composition.result_committed || composition.base_revision != self.document_revision {
            return None;
        }
        let (active_slot, _) = self.math_input_context()?;
        if &active_slot != slot {
            return None;
        }
        let mut preview = self.math_edit_session.as_ref()?.clone();
        let document = preview.document().clone();
        let anchor =
            MathCursor2D::at(&document, slot.clone(), composition.replacement_range.start).ok()?;
        let focus =
            MathCursor2D::at(&document, slot.clone(), composition.replacement_range.end).ok()?;
        preview
            .editor_mut()
            .set_selection(MathSelection::new(anchor, focus))
            .ok()?;
        let candidate = format!("{}{}", composition.committed_prefix, composition.preedit);
        if !candidate.is_empty() {
            preview
                .execute(MathEditCommand::InsertText(candidate))
                .ok()?;
        }
        if composition.has_preedit_update || composition.has_confirmed_result {
            let (selection_start, selection_end) = if composition.has_preedit_update {
                (
                    composition.selection_in_preedit.start,
                    composition.selection_in_preedit.end,
                )
            } else {
                (0, 0)
            };
            let candidate_start = composition
                .replacement_range
                .start
                .saturating_add(composition.committed_prefix.len());
            let document = preview.document().clone();
            let anchor = MathCursor2D::at(
                &document,
                slot.clone(),
                candidate_start.saturating_add(selection_start),
            )
            .ok()?;
            let focus = MathCursor2D::at(
                &document,
                slot.clone(),
                candidate_start.saturating_add(selection_end),
            )
            .ok()?;
            preview
                .editor_mut()
                .set_selection(MathSelection::new(anchor, focus))
                .ok()?;
        }
        Some(preview)
    }

    /// Returns a disposable selection rectangle range for an unconfirmed formula candidate.
    pub(crate) fn math_ime_marked_selection(
        &self,
        preview: &MathEditSession,
    ) -> Option<MathSelection> {
        let composition = self.ime_composition.as_ref()?;
        let BlockImeCompositionOwner::MathSlot(slot) = &composition.owner else {
            return None;
        };
        if !composition.has_preedit_update
            || composition.preedit.is_empty()
            || composition.result_committed
        {
            return None;
        }
        let start = composition
            .replacement_range
            .start
            .saturating_add(composition.committed_prefix.len());
        let document = preview.document().clone();
        let anchor = MathCursor2D::at(&document, slot.clone(), start).ok()?;
        let focus = MathCursor2D::at(
            &document,
            slot.clone(),
            start.saturating_add(composition.preedit.len()),
        )
        .ok()?;
        Some(MathSelection::new(anchor, focus))
    }

    /// Returns a UTF-16 selection for native queries on the pinned input surface.
    pub(crate) fn ime_selected_text_range(
        &self,
        owner: &BlockImeCompositionOwner,
    ) -> Option<UTF16Selection> {
        let visible = self.ime_visible_text(owner)?;
        let (range, reversed) = if let Some(selection) = self.ime_render_selection(owner) {
            selection
        } else {
            self.ime_selection_snapshot(owner)
                .map(|selection| (selection.range, selection.reversed))?
        };
        let normalized = Self::clamp_ime_range(&visible, range);
        Some(UTF16Selection {
            range: Self::utf8_range_to_utf16_in(&visible, &normalized),
            reversed,
        })
    }

    /// Returns the preedit range in the exact text representation exposed to GPUI.
    pub(crate) fn ime_marked_text_range(
        &self,
        owner: &BlockImeCompositionOwner,
    ) -> Option<Range<usize>> {
        let visible = self.ime_visible_text(owner)?;
        let marked = self.ime_marked_range(owner)?;
        Some(Self::utf8_range_to_utf16_in(&visible, &marked))
    }

    /// Splices candidate text over its baseline without publishing a document mutation.
    fn splice_ime_text(base: &str, composition: &BlockImeComposition) -> Option<String> {
        if base.get(composition.replacement_range.clone())?
            != composition.baseline_fragment.as_str()
        {
            return None;
        }
        let mut visible = String::with_capacity(
            base.len()
                .saturating_sub(composition.baseline_fragment.len())
                .saturating_add(composition.committed_prefix.len())
                .saturating_add(composition.preedit.len()),
        );
        visible.push_str(base.get(..composition.replacement_range.start)?);
        visible.push_str(&composition.committed_prefix);
        visible.push_str(&composition.preedit);
        visible.push_str(base.get(composition.replacement_range.end..)?);
        Some(visible)
    }
}
