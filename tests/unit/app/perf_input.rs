// @author kongweiguang

use super::{
    InputPaintKind, InputPaintSnapshot, InputPaintSurface, PendingPaintTrace,
    store_input_to_gpui_paint, take_input_to_gpui_paint_for_target,
};
use gmark_document::Revision;
use gpui::EntityId;
use std::time::{Duration, Instant};

/// Keeps event names and fixed array slots stable for low-cardinality trace aggregation.
#[test]
fn input_paint_kinds_have_distinct_stable_events_and_slots() {
    let kinds = [
        InputPaintKind::Typing,
        InputPaintKind::SelectionDrag,
        InputPaintKind::ImePreedit,
        InputPaintKind::ImeCommit,
    ];
    let names = kinds.map(InputPaintKind::event_name);
    let slots = kinds.map(InputPaintKind::slot);

    assert_eq!(
        names,
        [
            "typing_to_gpui_text_paint",
            "selection_drag_to_gpui_text_paint",
            "ime_preedit_to_gpui_text_paint",
            "ime_commit_to_gpui_text_paint",
        ]
    );
    assert_eq!(slots, [0, 1, 2, 3]);
}

/// Builds a content-free identity with enough state to reject samples from older edits.
fn snapshot(
    surface: InputPaintSurface,
    revision: u64,
    selection: (usize, usize),
    composition_generation: Option<u64>,
) -> InputPaintSnapshot {
    InputPaintSnapshot {
        surface,
        revision: Revision::from_u64(revision),
        selection_start: selection.0,
        selection_end: selection.1,
        selection_reversed: false,
        editor_selection: None,
        composition_generation,
    }
}

/// Verifies one bounded slot per kind keeps the latest event without overwriting other kinds.
#[test]
fn begin_replaces_latest_sample_for_the_same_kind() {
    let target = EntityId::from(1);
    let identity = snapshot(InputPaintSurface::BlockText, 1, (3, 3), None);
    let first_started = Instant::now();
    let latest_started = first_started + Duration::from_millis(1);
    let mut pending = [None; 4];

    store_input_to_gpui_paint(
        &mut pending,
        InputPaintKind::Typing,
        target,
        identity,
        first_started,
    );
    store_input_to_gpui_paint(
        &mut pending,
        InputPaintKind::SelectionDrag,
        target,
        identity,
        first_started,
    );
    store_input_to_gpui_paint(
        &mut pending,
        InputPaintKind::Typing,
        target,
        identity,
        latest_started,
    );

    assert_eq!(
        pending[0].map(|trace| trace.kind),
        Some(InputPaintKind::Typing)
    );
    assert_eq!(pending[0].map(|trace| trace.started), Some(latest_started));
    assert_eq!(
        pending[1].map(|trace| trace.kind),
        Some(InputPaintKind::SelectionDrag)
    );
}

/// Verifies a paint consumes only its entity and input surface, preserving unrelated ownership.
#[test]
fn take_returns_only_samples_owned_by_the_painted_surface() {
    let painted_target = EntityId::from(1);
    let other_target = EntityId::from(2);
    let started = Instant::now();
    let text = snapshot(InputPaintSurface::BlockText, 1, (0, 1), None);
    let language = snapshot(InputPaintSurface::CodeLanguage, 1, (0, 1), None);
    let mut pending = [None; 4];
    store_input_to_gpui_paint(
        &mut pending,
        InputPaintKind::Typing,
        painted_target,
        text,
        started,
    );
    store_input_to_gpui_paint(
        &mut pending,
        InputPaintKind::SelectionDrag,
        other_target,
        text,
        started,
    );
    store_input_to_gpui_paint(
        &mut pending,
        InputPaintKind::ImePreedit,
        painted_target,
        language,
        started,
    );

    let ready = take_input_to_gpui_paint_for_target(&mut pending, painted_target, text);

    assert!(ready[0].is_some());
    assert!(ready[1].is_none());
    assert!(ready[2].is_none());
    assert!(pending[0].is_none());
    assert!(pending[1].is_some());
    assert!(pending[2].is_some());
}

/// Prevents later text paint from reporting a stale revision or selection as an input response.
#[test]
fn take_discards_stale_state_on_the_same_surface() {
    let target = EntityId::from(1);
    let started = Instant::now();
    let old_state = snapshot(InputPaintSurface::BlockText, 1, (1, 4), None);
    let new_state = snapshot(InputPaintSurface::BlockText, 2, (4, 4), None);
    let unrelated_surface = snapshot(InputPaintSurface::CodeLanguage, 2, (4, 4), None);
    let mut pending = [None; 4];
    store_input_to_gpui_paint(
        &mut pending,
        InputPaintKind::Typing,
        target,
        old_state,
        started,
    );
    store_input_to_gpui_paint(
        &mut pending,
        InputPaintKind::SelectionDrag,
        target,
        unrelated_surface,
        started,
    );

    let ready = take_input_to_gpui_paint_for_target(&mut pending, target, new_state);

    assert!(ready.iter().all(Option::is_none));
    assert!(pending[0].is_none());
    assert!(pending[1].is_some());
}

/// Matches the post-edit selection captured before Block::Changed publishes the next revision.
#[test]
fn typing_sample_matches_the_committed_revision_at_text_paint() {
    let target = EntityId::from(1);
    let started = Instant::now();
    let captured_after_edit = snapshot(InputPaintSurface::BlockText, 8, (5, 5), None);
    let paint_after_commit = snapshot(InputPaintSurface::BlockText, 9, (5, 5), None);
    let mut pending = [None; 4];
    store_input_to_gpui_paint(
        &mut pending,
        InputPaintKind::Typing,
        target,
        captured_after_edit,
        started,
    );

    let ready = take_input_to_gpui_paint_for_target(&mut pending, target, paint_after_commit);

    assert!(ready[InputPaintKind::Typing.slot()].is_some());

    store_input_to_gpui_paint(
        &mut pending,
        InputPaintKind::ImeCommit,
        target,
        captured_after_edit,
        started,
    );
    let ready = take_input_to_gpui_paint_for_target(&mut pending, target, paint_after_commit);
    assert!(ready[InputPaintKind::ImeCommit.slot()].is_some());
}

/// Rejects a delayed sample when more than its own single local commit advanced the block.
#[test]
fn typing_sample_does_not_match_after_a_later_revision() {
    let target = EntityId::from(1);
    let started = Instant::now();
    let before_edit = snapshot(InputPaintSurface::BlockText, 8, (4, 4), None);
    let later_edit = snapshot(InputPaintSurface::BlockText, 10, (4, 4), None);
    let mut pending = [None; 4];
    store_input_to_gpui_paint(
        &mut pending,
        InputPaintKind::Typing,
        target,
        before_edit,
        started,
    );

    let ready = take_input_to_gpui_paint_for_target(&mut pending, target, later_edit);

    assert!(ready[InputPaintKind::Typing.slot()].is_none());
}

/// Uses composition generations because consecutive IME candidates can share one baseline selection.
#[test]
fn ime_sample_requires_the_current_composition_generation() {
    let target = EntityId::from(1);
    let started = Instant::now();
    let previous = snapshot(InputPaintSurface::BlockText, 4, (6, 6), Some(7));
    let current = snapshot(InputPaintSurface::BlockText, 4, (6, 6), Some(8));
    let mut pending = [None; 4];
    store_input_to_gpui_paint(
        &mut pending,
        InputPaintKind::ImePreedit,
        target,
        previous,
        started,
    );

    let ready = take_input_to_gpui_paint_for_target(&mut pending, target, current);

    assert!(ready[InputPaintKind::ImePreedit.slot()].is_none());
    assert!(pending[InputPaintKind::ImePreedit.slot()].is_none());
}

/// Terminal cleanup invalidates only the matching surface's temporary IME samples.
#[test]
fn ime_terminal_invalidation_keeps_regular_input_samples() {
    let target = EntityId::from(1);
    let started = Instant::now();
    let state = snapshot(InputPaintSurface::BlockText, 5, (2, 2), Some(11));
    let language = snapshot(InputPaintSurface::CodeLanguage, 5, (2, 2), Some(12));
    let mut pending: [Option<PendingPaintTrace>; 4] = [None; 4];
    store_input_to_gpui_paint(&mut pending, InputPaintKind::Typing, target, state, started);
    store_input_to_gpui_paint(
        &mut pending,
        InputPaintKind::ImePreedit,
        target,
        state,
        started,
    );
    store_input_to_gpui_paint(
        &mut pending,
        InputPaintKind::ImeCommit,
        target,
        language,
        started,
    );

    super::invalidate_ime_pending_for_surface(&mut pending, target, InputPaintSurface::BlockText);

    assert!(pending[InputPaintKind::Typing.slot()].is_some());
    assert!(pending[InputPaintKind::ImePreedit.slot()].is_none());
    assert!(pending[InputPaintKind::ImeCommit.slot()].is_some());
}
