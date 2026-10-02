// @author kongweiguang

use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation;

use super::Block;
use crate::ui::text_editing::{logical_line_range_at, word_range_at};

/// Maps a virtual IME hit back to the unmodified text, keeping suffix positions stable after replacement.
fn ime_display_offset_to_baseline(
    base_text: &str,
    replacement_range: &Range<usize>,
    baseline_fragment: &str,
    committed_prefix: &str,
    preedit: &str,
    display_offset: usize,
) -> Option<usize> {
    if base_text.get(replacement_range.clone())? != baseline_fragment {
        return None;
    }

    let mut inserted = String::with_capacity(committed_prefix.len().saturating_add(preedit.len()));
    inserted.push_str(committed_prefix);
    inserted.push_str(preedit);
    let virtual_end = replacement_range.start.checked_add(inserted.len())?;
    let virtual_len = base_text
        .len()
        .saturating_sub(baseline_fragment.len())
        .saturating_add(inserted.len());
    let offset = display_offset.min(virtual_len);

    if offset < replacement_range.start {
        return Some(offset);
    }
    if offset > virtual_end {
        return Some(
            replacement_range
                .end
                .saturating_add(offset.saturating_sub(virtual_end))
                .min(base_text.len()),
        );
    }
    if inserted.is_empty() || offset == virtual_end {
        return Some(replacement_range.end);
    }
    if offset == replacement_range.start {
        return Some(replacement_range.start);
    }

    let local_offset = offset.saturating_sub(replacement_range.start);
    let snapped_offset = nearest_grapheme_boundary(&inserted, local_offset);
    let graphemes_before = inserted[..snapped_offset].graphemes(true).count();
    let total_graphemes = inserted.graphemes(true).count();
    Some(if graphemes_before.saturating_mul(2) < total_graphemes {
        replacement_range.start
    } else {
        replacement_range.end
    })
}

/// Keeps pointer and caret offsets outside combining marks and joined emoji intact.
fn nearest_grapheme_boundary(text: &str, offset: usize) -> usize {
    let mut offset = offset.min(text.len());
    while !text.is_char_boundary(offset) {
        offset = offset.saturating_sub(1);
    }
    if offset == 0 || offset == text.len() {
        return offset;
    }

    let range = crate::ui::text_editing::grapheme_range_at(text, offset);
    if offset == range.start || range.is_empty() {
        return offset;
    }
    if offset.saturating_sub(range.start) < range.end.saturating_sub(offset) {
        range.start
    } else {
        range.end
    }
}

/// Granularity captured on pointer-down so later drag events keep the same
/// selection unit even if the pointer crosses several words or source lines.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PointerSelectionGranularity {
    Word,
    Line,
    Paragraph,
}

/// Stable clean-text anchor for a multi-click drag; display offsets can change
/// when focusing a rich-text block rebuilds its inline projection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PointerSelectionSession {
    pub(crate) anchor_range: Range<usize>,
    pub(crate) granularity: PointerSelectionGranularity,
}

impl Block {
    /// Shares Block's double/triple-click policy with the editor's pre-Block pointer capture.
    pub(crate) fn pointer_selection_granularity_for_click_count(
        &self,
        click_count: usize,
    ) -> Option<PointerSelectionGranularity> {
        match click_count {
            2 => Some(PointerSelectionGranularity::Word),
            3.. if self.uses_raw_text_editing() => Some(PointerSelectionGranularity::Line),
            3.. => Some(PointerSelectionGranularity::Paragraph),
            _ => None,
        }
    }

    /// Converts layout hits through the temporary IME splice before mapping rich text to clean text.
    pub(crate) fn pointer_clean_offset(&self, baseline_display_offset: usize) -> usize {
        let display_text = self.display_text();
        let baseline_display_offset =
            nearest_grapheme_boundary(display_text, baseline_display_offset);
        let clean_text = self.render_cache.visible_text();
        nearest_grapheme_boundary(
            clean_text,
            self.current_to_clean_offset(baseline_display_offset),
        )
    }

    /// Reverses only the BlockText preedit projection so parent surfaces hit the same baseline bytes.
    pub(crate) fn pointer_layout_offset_to_baseline(&self, layout_offset: usize) -> usize {
        let base_text = self.display_text();
        let Some(composition) = self.ime_composition.as_ref().filter(|composition| {
            composition.owner == super::input::BlockImeCompositionOwner::BlockText
                && !composition.result_committed
                && (composition.has_preedit_update || composition.has_confirmed_result)
        }) else {
            return nearest_grapheme_boundary(base_text, layout_offset);
        };

        ime_display_offset_to_baseline(
            base_text,
            &composition.replacement_range,
            &composition.baseline_fragment,
            &composition.committed_prefix,
            &composition.preedit,
            layout_offset,
        )
        .map(|offset| nearest_grapheme_boundary(base_text, offset))
        .unwrap_or_else(|| nearest_grapheme_boundary(base_text, layout_offset))
    }

    /// Exposes the original clean-text multi-click unit so the editor can extend it across blocks.
    pub(crate) fn pointer_selection_session(&self) -> Option<PointerSelectionSession> {
        self.pointer_selection.clone()
    }

    /// Resolves a target unit in clean text while keeping word and logical-line rules shared with Block.
    pub(crate) fn pointer_selection_unit_range(
        &self,
        clean_offset: usize,
        granularity: PointerSelectionGranularity,
    ) -> Range<usize> {
        let text = self.render_cache.visible_text();
        match granularity {
            PointerSelectionGranularity::Word => word_range_at(text, clean_offset),
            PointerSelectionGranularity::Line => logical_line_range_at(text, clean_offset),
            PointerSelectionGranularity::Paragraph => 0..text.len(),
        }
    }

    /// Captures the fixed endpoint before a Block mouse handler can replace its local selection.
    pub(crate) fn pointer_selection_clean_anchor(&self) -> usize {
        let anchor = if self.selected_range.is_empty() || !self.selection_reversed {
            self.selected_range.start
        } else {
            self.selected_range.end
        };
        self.current_to_clean_offset(anchor)
    }
}

impl PointerSelectionSession {
    /// Capture the initial selected unit in the block's stable clean text.
    pub(super) fn new(
        anchor_range: Range<usize>,
        granularity: PointerSelectionGranularity,
    ) -> Self {
        Self {
            anchor_range,
            granularity,
        }
    }

    /// Extend from the original selected unit and return selection direction
    /// separately so callers can preserve reverse drags in the Block model.
    pub(super) fn selection_for_offset(&self, text: &str, offset: usize) -> (Range<usize>, bool) {
        let start = self.anchor_range.start.min(text.len());
        let end = self.anchor_range.end.min(text.len()).max(start);
        let anchor = start..end;
        let focus = match self.granularity {
            PointerSelectionGranularity::Word => word_range_at(text, offset),
            PointerSelectionGranularity::Line => logical_line_range_at(text, offset),
            PointerSelectionGranularity::Paragraph => 0..text.len(),
        };

        if self.granularity == PointerSelectionGranularity::Paragraph {
            return (anchor, false);
        }
        if offset < anchor.start {
            (focus.start..anchor.end, true)
        } else if offset >= anchor.end {
            (anchor.start..focus.end, false)
        } else {
            (anchor, false)
        }
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/components/block/pointer_selection.rs"]
mod tests;
