// @author kongweiguang

use super::{PointerSelectionGranularity, PointerSelectionSession, ime_display_offset_to_baseline};
use crate::ui::text_editing::logical_line_range_at;

/// Keep a word drag anchored to its original unit when crossing either direction.
#[test]
fn word_drag_extends_from_the_original_word_in_both_directions() {
    let text = "alpha beta gamma";
    let session = PointerSelectionSession::new(6..10, PointerSelectionGranularity::Word);

    assert_eq!(session.selection_for_offset(text, 1), (0..10, true));
    assert_eq!(session.selection_for_offset(text, 13), (6..16, false));
}

/// Preserve CRLF separators outside selected source-line content while reversing direction.
#[test]
fn line_drag_extends_across_crlf_without_splitting_line_content() {
    let text = "first\r\nsecond\r\nthird";
    let session = PointerSelectionSession::new(
        logical_line_range_at(text, 8),
        PointerSelectionGranularity::Line,
    );

    assert_eq!(session.anchor_range, 7..13);
    assert_eq!(session.selection_for_offset(text, 2), (0..13, true));
    assert_eq!(session.selection_for_offset(text, 17), (7..20, false));
}

/// Keeps virtual candidate hits at Unicode grapheme boundaries before resolving the replacement edge.
#[test]
fn ime_pointer_hits_inside_candidate_map_to_nearest_baseline_edge() {
    let base = "abX!cd";
    let candidate = "甲👨‍👩‍👧‍👦乙";
    let family_end = "甲👨‍👩‍👧‍👦".len();

    assert_eq!(
        ime_display_offset_to_baseline(base, &(2..4), "X!", "", candidate, 2 + 3),
        Some(2),
    );
    assert_eq!(
        ime_display_offset_to_baseline(base, &(2..4), "X!", "", candidate, 2 + family_end,),
        Some(4),
    );
}

/// Uses right affinity at an exact grapheme midpoint so empty-range insertion cannot land before committed text.
#[test]
fn ime_pointer_candidate_midpoint_prefers_the_right_replacement_edge() {
    let base = "ab!cd";
    assert_eq!(
        ime_display_offset_to_baseline(base, &(2..3), "!", "", "甲乙", 2 + 3),
        Some(3),
    );
    assert_eq!(
        ime_display_offset_to_baseline(base, &(2..2), "", "", "甲乙", 2 + 3),
        Some(2),
    );
}

/// Preserves hits before and after a virtual splice despite a candidate length different from the replaced text.
#[test]
fn ime_pointer_hits_outside_candidate_keep_baseline_suffix_positions() {
    let base = "zero-old-tail";
    assert_eq!(
        ime_display_offset_to_baseline(base, &(5..8), "old", "A", "B", 3),
        Some(3),
    );
    assert_eq!(
        ime_display_offset_to_baseline(base, &(5..8), "old", "A", "B", 9),
        Some(10),
    );
}
