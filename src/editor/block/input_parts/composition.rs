// @author kongweiguang

//! Stages native IME preedit on the owning input surface until the platform
//! reports a final result or cancellation.

use std::ops::Range;

use gmark_document::Revision;
use gmark_math_edit::{MathEditCommand, MathSlot};
use gpui::{CompositionEnd, Context, Window};
use unicode_segmentation::UnicodeSegmentation;

use super::Block;
use crate::components::{BlockEvent, UndoCaptureKind};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum BlockImeCompositionOwner {
    BlockText,
    CodeLanguage,
    MathSource,
    MathSlot(MathSlot),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BlockImeOriginalSelection {
    pub(crate) range: Range<usize>,
    pub(crate) reversed: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BlockImeComposition {
    pub(crate) owner: BlockImeCompositionOwner,
    pub(crate) replacement_range: Range<usize>,
    pub(crate) baseline_fragment: String,
    pub(crate) committed_prefix: String,
    pub(crate) preedit: String,
    pub(crate) has_preedit_update: bool,
    pub(crate) has_confirmed_result: bool,
    pub(crate) selection_in_preedit: Range<usize>,
    pub(crate) original_selection: BlockImeOriginalSelection,
    pub(crate) base_revision: Revision,
    pub(crate) replace_cross_block: bool,
    pub(crate) result_committed: bool,
    /// Distinguishes successive candidate redraws that share one source selection snapshot.
    pub(crate) paint_generation: Option<u64>,
}

impl Block {
    /// Returns the original input target while a native composition is active.
    pub(crate) fn ime_composition_owner(&self) -> Option<BlockImeCompositionOwner> {
        self.ime_composition
            .as_ref()
            .map(|composition| composition.owner.clone())
    }

    /// Lets document hosts defer writes and navigation until staged preedit ends.
    pub(crate) fn has_ime_composition(&self) -> bool {
        self.ime_awaiting_end || self.ime_composition.is_some() || self.ime_reject_until_end
    }

    /// Transfers only an uncommitted preedit to shared rebase; terminal-pending commits stay pinned.
    pub(crate) fn take_ime_composition(&mut self) -> Option<BlockImeComposition> {
        if self
            .ime_composition
            .as_ref()
            .is_some_and(|composition| composition.result_committed)
        {
            return None;
        }
        self.ime_composition.take()
    }

    /// Restores a rebased session only when its original text still matches.
    pub(crate) fn restore_ime_composition(
        &mut self,
        mut composition: BlockImeComposition,
        cx: &mut Context<Self>,
    ) -> bool {
        if matches!(
            &composition.owner,
            BlockImeCompositionOwner::MathSource | BlockImeCompositionOwner::MathSlot(_)
        ) && composition.base_revision != self.document_revision
        {
            return false;
        }
        if !self.ime_baseline_matches(&composition) {
            return false;
        }
        composition.base_revision = self.document_revision;
        self.ime_reject_until_end = false;
        self.ime_awaiting_end = true;
        self.ime_composition = Some(composition);
        cx.notify();
        true
    }

    /// Captures the owner and replacement baseline at native START, including result-only IMEs.
    pub(super) fn begin_ime_composition(&mut self, window: &Window, cx: &mut Context<Self>) {
        if self.ime_reject_until_end || self.is_read_only() {
            return;
        }
        let Some(owner) = self.focused_ime_owner(window) else {
            return;
        };
        if let Some(active) = self.ime_composition.as_ref() {
            if active.owner == owner && self.ime_baseline_matches(active) {
                self.ime_awaiting_end = true;
                return;
            }
            self.reject_stale_ime_composition();
            cx.notify();
            return;
        }
        let Some(base_text) = self.ime_base_text_view(&owner) else {
            return;
        };
        let selection = self
            .ime_selection_snapshot_with_text_len(&owner, base_text.len())
            .unwrap_or(BlockImeOriginalSelection {
                range: 0..0,
                reversed: false,
            });
        let Some(composition) = Self::new_ime_composition(
            self,
            owner,
            base_text.as_ref(),
            selection.clone(),
            selection.range,
        ) else {
            return;
        };
        self.ime_composition = Some(composition);
        self.ime_awaiting_end = true;
        cx.notify();
    }

    /// Replaces the current virtual composition range without mutating the document model.
    pub(super) fn stage_ime_preedit(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        selected_range_utf16: Option<Range<usize>>,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.ime_reject_until_end || self.is_read_only() {
            return true;
        }
        let Some(owner) = self.current_ime_owner(window) else {
            return false;
        };
        let paint_started = crate::perf::start_input_to_gpui_paint();
        let preedit = Self::sanitize_ime_text(&owner, new_text);
        let selection_in_preedit = selected_range_utf16
            .as_ref()
            .map(|range| Self::utf16_range_to_utf8_in(&preedit, range))
            .unwrap_or_else(|| preedit.len()..preedit.len());
        if let Some(active) = self.ime_composition.as_ref() {
            let stale_revision = matches!(
                &active.owner,
                BlockImeCompositionOwner::MathSource | BlockImeCompositionOwner::MathSlot(_)
            ) && active.base_revision != self.document_revision;
            if active.owner != owner || stale_revision || !self.ime_baseline_matches(active) {
                self.reject_stale_ime_composition();
                return true;
            }
        } else {
            let Some(base_text) = self.ime_base_text_view(&owner) else {
                return true;
            };
            let selection = self
                .ime_selection_snapshot_with_text_len(&owner, base_text.len())
                .unwrap_or(BlockImeOriginalSelection {
                    range: 0..0,
                    reversed: false,
                });
            let replacement_range = range_utf16
                .as_ref()
                .map(|range| Self::utf16_range_to_utf8_in(base_text.as_ref(), range))
                .unwrap_or_else(|| selection.range.clone());
            let Some(composition) = Self::new_ime_composition(
                self,
                owner.clone(),
                base_text.as_ref(),
                selection,
                replacement_range,
            ) else {
                return true;
            };
            self.ime_composition = Some(composition);
        }

        let Some(composition) = self.ime_composition.as_mut() else {
            return true;
        };
        composition.preedit = preedit;
        composition.has_preedit_update = true;
        composition.selection_in_preedit =
            Self::clamp_ime_range_preserving_direction(&composition.preedit, selection_in_preedit);
        composition.result_committed = false;
        composition.paint_generation = crate::perf::next_input_paint_generation();

        self.ime_awaiting_end = true;
        let paint_surface = super::input_paint_surface_for_ime_owner(&owner);
        if let Some(surface) = paint_surface {
            self.finish_input_to_gpui_paint(
                paint_started,
                crate::perf::InputPaintKind::ImePreedit,
                surface,
                cx,
            );
        }
        cx.notify();
        true
    }

    /// Commits only the platform's final result and never substitutes staged phonetic preedit.
    pub(super) fn commit_ime_result(&mut self, result_text: &str, cx: &mut Context<Self>) -> bool {
        let Some(mut composition) = self.ime_composition.clone() else {
            return false;
        };
        if self.ime_reject_until_end || composition.result_committed {
            return true;
        }
        if !self.ime_baseline_matches(&composition) {
            self.reject_stale_ime_composition();
            return true;
        }
        if matches!(
            &composition.owner,
            BlockImeCompositionOwner::MathSource | BlockImeCompositionOwner::MathSlot(_)
        ) && composition.base_revision != self.document_revision
        {
            self.reject_stale_ime_composition();
            return true;
        }

        let paint_started = crate::perf::start_input_to_gpui_paint();
        composition
            .committed_prefix
            .push_str(&Self::sanitize_ime_text(&composition.owner, result_text));
        composition.has_confirmed_result = true;
        composition.preedit.clear();
        composition.selection_in_preedit = 0..0;
        composition.paint_generation = crate::perf::next_input_paint_generation();
        let paint_surface = super::input_paint_surface_for_ime_owner(&composition.owner);
        self.ime_composition = Some(composition);
        if let Some(surface) = paint_surface {
            self.finish_input_to_gpui_paint(
                paint_started,
                crate::perf::InputPaintKind::ImeCommit,
                surface,
                cx,
            );
        }
        cx.notify();
        true
    }

    /// Resolves queued offsets before publication, including a confirmed prefix whose remainder was cancelled.
    /// Native cancellation discards only pending preedit; it does not undo results already confirmed by the IME.
    pub(super) fn end_ime_composition(&mut self, end: CompositionEnd, cx: &mut Context<Self>) {
        let paint_started = crate::perf::start_input_to_gpui_paint();
        self.ime_interaction_rebase = None;
        let composition = self.ime_composition.take();
        if let Some(composition) = composition.as_ref()
            && let Some(surface) = super::input_paint_surface_for_ime_owner(&composition.owner)
        {
            crate::perf::invalidate_ime_input_to_gpui_paint(cx.entity().entity_id(), surface);
        }
        let mut committed_surface = None;
        let mut committed = end == CompositionEnd::Committed && !self.ime_reject_until_end;
        if let Some(mut composition) = composition {
            if composition.has_confirmed_result && !composition.committed_prefix.is_empty() {
                if !self.ime_reject_until_end && self.ime_baseline_matches(&composition) {
                    let should_trace_commit = !composition.replace_cross_block
                        && composition.committed_prefix != composition.baseline_fragment
                        && !self.is_read_only();
                    let paint_surface =
                        super::input_paint_surface_for_ime_owner(&composition.owner);
                    composition.result_committed = true;
                    self.ime_composition = Some(composition.clone());
                    self.record_ime_interaction_rebase(&composition, true);
                    self.publish_ime_result(&composition, cx);
                    self.ime_composition = None;
                    if should_trace_commit {
                        committed_surface = paint_surface;
                    }
                } else {
                    self.reject_stale_ime_composition();
                    committed = false;
                }
            } else {
                self.restore_ime_selection(&composition);
                committed = false;
            }
        }
        self.clear_ime_marked_ranges();
        self.ime_reject_until_end = false;
        self.ime_awaiting_end = false;
        if let Some(surface) = committed_surface {
            self.finish_input_to_gpui_paint(
                paint_started,
                crate::perf::InputPaintKind::ImeCommit,
                surface,
                cx,
            );
        }
        cx.emit(BlockEvent::ImeCompositionEnded { committed });
        cx.notify();
    }

    /// Keeps a rejected native finish request retryable without committing its current preedit.
    pub(super) fn ime_composition_finish_failed(&mut self, cx: &mut Context<Self>) {
        if self.ime_composition.is_some() {
            cx.emit(BlockEvent::ImeCompositionFinishFailed);
        }
    }

    /// Checks the captured fragment before any final write or concurrent restore.
    fn ime_baseline_matches(&self, composition: &BlockImeComposition) -> bool {
        self.ime_base_text_view(&composition.owner)
            .and_then(|text| {
                text.get(composition.replacement_range.clone())
                    .map(|fragment| fragment == composition.baseline_fragment.as_str())
            })
            .unwrap_or(false)
    }

    /// Converts platform line breaks only for input targets whose grammar is single-line.
    fn sanitize_ime_text(owner: &BlockImeCompositionOwner, text: &str) -> String {
        if matches!(
            owner,
            BlockImeCompositionOwner::CodeLanguage
                | BlockImeCompositionOwner::MathSource
                | BlockImeCompositionOwner::MathSlot(_)
        ) {
            text.replace("\r\n", " ").replace(['\r', '\n'], " ")
        } else {
            text.to_owned()
        }
    }

    /// Clamps endpoints to grapheme boundaries while keeping selection direction.
    fn clamp_ime_range_preserving_direction(text: &str, range: Range<usize>) -> Range<usize> {
        if range.start == range.end {
            let offset = Self::floor_ime_grapheme_boundary(text, range.start);
            return offset..offset;
        }
        if range.start <= range.end {
            Self::floor_ime_grapheme_boundary(text, range.start)
                ..Self::ceil_ime_grapheme_boundary(text, range.end)
        } else {
            Self::ceil_ime_grapheme_boundary(text, range.start)
                ..Self::floor_ime_grapheme_boundary(text, range.end)
        }
    }

    /// Normalizes an incoming replacement range without splitting a Unicode grapheme.
    pub(super) fn clamp_ime_range(text: &str, range: Range<usize>) -> Range<usize> {
        let start = range.start.min(range.end);
        let end = range.start.max(range.end);
        if start == end {
            let offset = Self::floor_ime_grapheme_boundary(text, start);
            return offset..offset;
        }
        Self::floor_ime_grapheme_boundary(text, start)..Self::ceil_ime_grapheme_boundary(text, end)
    }

    /// Finds the preceding grapheme boundary for an untrusted platform offset.
    fn clamp_ime_offset(text: &str, offset: usize) -> usize {
        Self::floor_ime_grapheme_boundary(text, offset)
    }

    /// Floors an arbitrary UTF-8 offset so native ranges cannot divide a visible grapheme.
    fn floor_ime_grapheme_boundary(text: &str, offset: usize) -> usize {
        let offset = offset.min(text.len());
        let mut boundary = 0;
        for (start, grapheme) in text.grapheme_indices(true) {
            if start >= offset {
                return start;
            }
            let end = start + grapheme.len();
            if offset < end {
                return start;
            }
            boundary = end;
        }
        boundary
    }

    /// Ceils an arbitrary UTF-8 offset so selected ranges include a whole intersected grapheme.
    fn ceil_ime_grapheme_boundary(text: &str, offset: usize) -> usize {
        let offset = offset.min(text.len());
        for (start, grapheme) in text.grapheme_indices(true) {
            let end = start + grapheme.len();
            if offset <= start {
                return start;
            }
            if offset < end {
                return end;
            }
        }
        text.len()
    }

    /// Builds the immutable baseline captured at START or the first marked-text update.
    fn new_ime_composition(
        block: &Self,
        owner: BlockImeCompositionOwner,
        base_text: &str,
        original_selection: BlockImeOriginalSelection,
        replacement_range: Range<usize>,
    ) -> Option<BlockImeComposition> {
        let replacement_range = Self::clamp_ime_range(base_text, replacement_range);
        let baseline_fragment = base_text.get(replacement_range.clone())?.to_owned();
        Some(BlockImeComposition {
            replace_cross_block: owner == BlockImeCompositionOwner::BlockText
                && block.editor_selection_range.is_some(),
            owner,
            replacement_range,
            baseline_fragment,
            committed_prefix: String::new(),
            preedit: String::new(),
            has_preedit_update: false,
            has_confirmed_result: false,
            selection_in_preedit: 0..0,
            original_selection,
            base_revision: block.document_revision,
            result_committed: false,
            paint_generation: None,
        })
    }

    /// Cancels stale staging locally and drops later native results until its terminal callback.
    fn reject_stale_ime_composition(&mut self) {
        if let Some(composition) = self.ime_composition.take() {
            self.restore_ime_selection(&composition);
        }
        self.clear_ime_marked_ranges();
        self.ime_reject_until_end = true;
        self.ime_awaiting_end = true;
    }

    /// Restores the directional selection for the original owner after cancellation.
    fn restore_ime_selection(&mut self, composition: &BlockImeComposition) {
        let selection = &composition.original_selection;
        match &composition.owner {
            BlockImeCompositionOwner::BlockText => {
                let text = self.display_text().to_owned();
                let len = text.len();
                let range = Self::clamp_ime_range(&text, selection.range.clone());
                self.selected_range = range.start.min(len)..range.end.min(len);
                self.selection_reversed = selection.reversed;
                self.sync_collapsed_caret_affinity();
            }
            BlockImeCompositionOwner::CodeLanguage => {
                let len = self.code_language_text().len();
                self.code_language_selected_range =
                    selection.range.start.min(len)..selection.range.end.min(len);
                self.code_language_selection_reversed = selection.reversed;
            }
            BlockImeCompositionOwner::MathSource => {
                self.set_math_source_selection(selection.range.clone(), selection.reversed);
            }
            BlockImeCompositionOwner::MathSlot(slot) => {
                let _ = self.set_math_selection(slot.clone(), selection.range.clone());
            }
        }
    }

    /// Clears compatibility mark fields so legacy commands cannot mistake preedit for source text.
    fn clear_ime_marked_ranges(&mut self) {
        self.marked_range = None;
        self.code_language_marked_range = None;
        self.math_source_marked_range = None;
        self.math_marked_range = None;
    }

    /// Applies the final confirmed text through the ordinary transactional mutation for its owner.
    fn publish_ime_result(&mut self, composition: &BlockImeComposition, cx: &mut Context<Self>) {
        let inserted = &composition.committed_prefix;
        if inserted == &composition.baseline_fragment {
            return;
        }
        if self.is_read_only() {
            return;
        }
        if composition.replace_cross_block {
            if !inserted.is_empty() || !composition.baseline_fragment.is_empty() {
                cx.emit(BlockEvent::RequestReplaceCrossBlockSelection {
                    text: inserted.clone(),
                    selected_range_relative: None,
                    mark_inserted_text: false,
                    undo_kind: UndoCaptureKind::ImeCompositionCommit,
                });
            }
            return;
        }
        match &composition.owner {
            BlockImeCompositionOwner::BlockText => {
                self.prepare_undo_capture(UndoCaptureKind::ImeCompositionCommit, cx);
                self.replace_text_in_visible_range(
                    composition.replacement_range.clone(),
                    inserted,
                    None,
                    false,
                    cx,
                );
            }
            BlockImeCompositionOwner::CodeLanguage => {
                self.replace_code_language_text_in_range_with_undo(
                    composition.replacement_range.clone(),
                    inserted,
                    None,
                    false,
                    UndoCaptureKind::ImeCompositionCommit,
                    cx,
                );
            }
            BlockImeCompositionOwner::MathSource => {
                self.replace_math_source_text_in_range(
                    composition.replacement_range.clone(),
                    inserted,
                    None,
                    false,
                    UndoCaptureKind::ImeCompositionCommit,
                    cx,
                );
            }
            BlockImeCompositionOwner::MathSlot(slot) => {
                if !self.ime_baseline_matches(composition)
                    || !self.set_math_selection(slot.clone(), composition.replacement_range.clone())
                {
                    return;
                }
                self.execute_math_command_live(
                    MathEditCommand::InsertText(inserted.clone()),
                    UndoCaptureKind::ImeCompositionCommit,
                    cx,
                );
            }
        }
    }
}

#[cfg(test)]
#[path = "../../../../tests/unit/components/block/ime_composition.rs"]
mod tests;
