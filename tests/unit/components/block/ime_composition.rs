// @author kongweiguang

#[cfg(target_os = "windows")]
use gpui::{AppContext, CompositionEnd, EntityInputHandler, TestAppContext};

use super::*;
#[cfg(target_os = "windows")]
use crate::components::block::BlockImeInteraction;
#[cfg(target_os = "windows")]
use crate::components::{BlockEvent, UndoCaptureKind};
#[cfg(target_os = "windows")]
use crate::components::{BlockKind, BlockRecord, InlineTextTree};
#[cfg(target_os = "windows")]
use std::sync::{Arc, Mutex};

#[path = "ime_inline_commands.rs"]
mod ime_inline_commands;

/// A result-only native cycle must replace its captured selection as one deferred transaction.
#[cfg(target_os = "windows")]
#[gpui::test]
async fn result_only_ime_publishes_once_after_terminal(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let block = cx.new(|cx| Block::with_record(cx, BlockRecord::paragraph("before")));

    cx.update(|window, cx| {
        block.update(cx, |block, block_cx| {
            block.selected_range = 1..4;
            <Block as EntityInputHandler>::composition_started(block, window, block_cx);
            let revision = block.document_revision;
            assert!(block.has_ime_composition());
            assert_eq!(block.display_text(), "before");

            <Block as EntityInputHandler>::replace_text_in_range(
                block, None, "X", window, block_cx,
            );
            <Block as EntityInputHandler>::replace_text_in_range(
                block, None, "Y", window, block_cx,
            );
            assert_eq!(block.display_text(), "before");
            assert_eq!(
                block
                    .ime_visible_text(&BlockImeCompositionOwner::BlockText)
                    .as_deref(),
                Some("bXYre")
            );
            assert_eq!(
                block.ime_visible_range(&BlockImeCompositionOwner::BlockText, 4..6),
                3..5
            );
            assert_eq!(block.document_revision, revision);

            <Block as EntityInputHandler>::composition_ended(
                block,
                CompositionEnd::Committed,
                window,
                block_cx,
            );
            assert_eq!(block.display_text(), "bXYre");
            assert!(!block.has_ime_composition());
        });
    });
}

/// Repeated candidate redraws remain virtual and publish only the final confirmed result.
#[cfg(target_os = "windows")]
#[gpui::test]
async fn repeated_ime_preedit_updates_publish_once_after_confirmation(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let block = cx.new(|cx| Block::with_record(cx, BlockRecord::paragraph("before")));

    cx.update(|window, cx| {
        block.update(cx, |block, block_cx| {
            block.selected_range = 1..4;
            let revision = block.document_revision;
            <Block as EntityInputHandler>::composition_started(block, window, block_cx);
            <Block as EntityInputHandler>::replace_and_mark_text_in_range(
                block,
                None,
                "kana",
                Some(4..4),
                window,
                block_cx,
            );
            <Block as EntityInputHandler>::replace_and_mark_text_in_range(
                block,
                None,
                "漢字",
                Some(2..2),
                window,
                block_cx,
            );

            assert_eq!(block.display_text(), "before");
            assert_eq!(block.document_revision, revision);
            assert_eq!(
                block
                    .ime_visible_text(&BlockImeCompositionOwner::BlockText)
                    .as_deref(),
                Some("b漢字re")
            );

            <Block as EntityInputHandler>::replace_text_in_range(
                block, None, "漢字", window, block_cx,
            );
            <Block as EntityInputHandler>::composition_ended(
                block,
                CompositionEnd::Committed,
                window,
                block_cx,
            );

            assert_eq!(block.display_text(), "b漢字re");
            assert!(!block.has_ime_composition());
        });
    });
}

/// Observes public edit events because a standalone Block does not own document revisions or history.
/// A confirmed prefix and later candidate must still publish one edit at the native terminal.
#[cfg(target_os = "windows")]
#[gpui::test]
async fn partial_ime_result_then_remaining_preedit_commits_only_results(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let block = cx.new(|cx| Block::with_record(cx, BlockRecord::paragraph("before")));
    let publications = Arc::new(Mutex::new((0usize, Vec::new())));
    let observed = publications.clone();
    let _subscription = cx.update(|_, cx| {
        cx.subscribe(&block, move |_, event: &BlockEvent, _| {
            let mut publications = observed.lock().expect("publication observations");
            match event {
                BlockEvent::Changed => publications.0 += 1,
                BlockEvent::PrepareUndo { kind } => publications.1.push(*kind),
                _ => {}
            }
        })
    });

    cx.update(|window, cx| {
        block.update(cx, |block, block_cx| {
            block.selected_range = 1..4;
            let revision = block.document_revision;
            <Block as EntityInputHandler>::composition_started(block, window, block_cx);
            <Block as EntityInputHandler>::replace_and_mark_text_in_range(
                block,
                None,
                "kana",
                Some(4..4),
                window,
                block_cx,
            );
            <Block as EntityInputHandler>::replace_text_in_range(
                block, None, "日", window, block_cx,
            );
            <Block as EntityInputHandler>::replace_and_mark_text_in_range(
                block,
                None,
                "候補",
                Some(2..2),
                window,
                block_cx,
            );

            assert_eq!(block.display_text(), "before");
            assert_eq!(block.document_revision, revision);
            assert_eq!(
                block
                    .ime_visible_text(&BlockImeCompositionOwner::BlockText)
                    .as_deref(),
                Some("b日候補re")
            );

            <Block as EntityInputHandler>::replace_text_in_range(
                block, None, "本", window, block_cx,
            );
            assert_eq!(block.display_text(), "before");
            assert_eq!(block.document_revision, revision);
            assert_eq!(
                block
                    .ime_visible_text(&BlockImeCompositionOwner::BlockText)
                    .as_deref(),
                Some("b日本re")
            );
        });
    });
    assert_eq!(
        *publications.lock().expect("publication observations"),
        (0, Vec::<UndoCaptureKind>::new())
    );

    cx.update(|window, cx| {
        block.update(cx, |block, block_cx| {
            <Block as EntityInputHandler>::composition_ended(
                block,
                CompositionEnd::Committed,
                window,
                block_cx,
            );

            assert_eq!(block.display_text(), "b日本re");
            assert!(!block.has_ime_composition());
        });
    });
    assert_eq!(
        *publications.lock().expect("publication observations"),
        (1, vec![UndoCaptureKind::ImeCompositionCommit])
    );
}

/// Cancelling keeps the confirmed prefix and discards only the live candidate, with one public edit.
/// Revision changes belong to the document host rather than this standalone input fixture.
#[cfg(target_os = "windows")]
#[gpui::test]
async fn partial_ime_result_then_remaining_preedit_cancel_keeps_prefix(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let block = cx.new(|cx| Block::with_record(cx, BlockRecord::paragraph("before")));
    let publications = Arc::new(Mutex::new((0usize, Vec::new())));
    let observed = publications.clone();
    let _subscription = cx.update(|_, cx| {
        cx.subscribe(&block, move |_, event: &BlockEvent, _| {
            let mut publications = observed.lock().expect("publication observations");
            match event {
                BlockEvent::Changed => publications.0 += 1,
                BlockEvent::PrepareUndo { kind } => publications.1.push(*kind),
                _ => {}
            }
        })
    });

    cx.update(|window, cx| {
        block.update(cx, |block, block_cx| {
            block.selected_range = 1..4;
            let revision = block.document_revision;
            <Block as EntityInputHandler>::composition_started(block, window, block_cx);
            <Block as EntityInputHandler>::replace_and_mark_text_in_range(
                block,
                None,
                "kana",
                Some(4..4),
                window,
                block_cx,
            );
            <Block as EntityInputHandler>::replace_text_in_range(
                block, None, "日", window, block_cx,
            );
            <Block as EntityInputHandler>::replace_and_mark_text_in_range(
                block,
                None,
                "候補",
                Some(2..2),
                window,
                block_cx,
            );

            assert_eq!(block.display_text(), "before");
            assert_eq!(block.document_revision, revision);
            assert_eq!(
                block
                    .ime_visible_text(&BlockImeCompositionOwner::BlockText)
                    .as_deref(),
                Some("b日候補re")
            );
        });
    });
    assert_eq!(
        *publications.lock().expect("publication observations"),
        (0, Vec::<UndoCaptureKind>::new())
    );

    cx.update(|window, cx| {
        block.update(cx, |block, block_cx| {
            <Block as EntityInputHandler>::composition_ended(
                block,
                CompositionEnd::Cancelled,
                window,
                block_cx,
            );

            assert_eq!(block.display_text(), "b日re");
            assert!(!block.has_ime_composition());
        });
    });
    assert_eq!(
        *publications.lock().expect("publication observations"),
        (1, vec![UndoCaptureKind::ImeCompositionCommit])
    );
}

/// A cancelled remainder still publishes its confirmed prefix, so a queued click must follow that edit.
#[cfg(target_os = "windows")]
#[gpui::test]
async fn partial_ime_result_then_remaining_preedit_cancel_rebases_queued_pointer(
    cx: &mut TestAppContext,
) {
    let cx = cx.add_empty_window();
    let block = cx.new(|cx| Block::with_record(cx, BlockRecord::paragraph("before")));
    cx.update(|window, cx| {
        block.update(cx, |block, cx| {
            block.focus_handle.focus(window);
            block.set_ime_interactions_managed(true);
            block.selected_range = 1..4;
            block.composition_started(window, cx);
            block.replace_and_mark_text_in_range(None, "kana", Some(4..4), window, cx);
            block.replace_text_in_range(None, "中文", window, cx);
            block.replace_and_mark_text_in_range(None, "候補", Some(2..2), window, cx);
            block.request_ime_interaction(
                BlockImeInteraction::PointerSelection {
                    clean_offset: 5,
                    click_count: 1,
                    shift: false,
                },
                window,
                cx,
            );
            block.composition_ended(CompositionEnd::Cancelled, window, cx);
            assert_eq!(block.display_text(), "b中文re");
            block.apply_ime_interaction(
                BlockImeInteraction::PointerSelection {
                    clean_offset: 5,
                    click_count: 1,
                    shift: false,
                },
                window,
                cx,
            );
            assert_eq!(block.current_to_clean_offset(block.cursor_offset()), 8);
        });
    });
}

/// Queued pointer hits and IME ranges must use the same clean coordinates after rich-text expansion.
/// Verifies that surface replay can distinguish a local commit map from an absent map.
#[cfg(target_os = "windows")]
#[gpui::test]
async fn ime_pointer_rebase_uses_clean_range_from_old_inline_projection(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::Paragraph,
                InlineTextTree::from_markdown("**alpha** beta"),
            ),
        )
    });

    cx.update(|window, cx| {
        block.update(cx, |block, block_cx| {
            block.focus_handle.focus(window);
            block.selected_range = 1..2;
            block.sync_inline_projection_for_focus(true);
            let projected_range = block.selected_range.clone();
            assert_ne!(projected_range, 1..2);

            block.set_ime_interactions_managed(true);
            <Block as EntityInputHandler>::composition_started(block, window, block_cx);
            <Block as EntityInputHandler>::replace_text_in_range(
                block, None, "你好", window, block_cx,
            );
            let queued_hit = block.pointer_clean_offset(projected_range.end);
            assert_eq!(queued_hit, 2);
            block.request_ime_interaction(
                BlockImeInteraction::PointerSelection {
                    clean_offset: queued_hit,
                    click_count: 1,
                    shift: false,
                },
                window,
                block_cx,
            );
            assert!(!block.has_local_pointer_interaction_rebase());

            <Block as EntityInputHandler>::composition_ended(
                block,
                CompositionEnd::Committed,
                window,
                block_cx,
            );
            assert_eq!(block.render_cache.visible_text(), "a你好pha beta");
            assert!(block.has_local_pointer_interaction_rebase());

            block.apply_ime_interaction(
                BlockImeInteraction::PointerSelection {
                    clean_offset: queued_hit,
                    click_count: 1,
                    shift: false,
                },
                window,
                block_cx,
            );
            assert_eq!(block.current_to_clean_offset(block.cursor_offset()), 7);

            block.apply_ime_interaction(BlockImeInteraction::PointerSelectionEnd, window, block_cx);
            assert!(!block.ime_pointer_selection_pending());
            assert!(!block.has_local_pointer_interaction_rebase());
            assert!(block.ime_interaction_rebase.is_none());
        });
    });
}

/// Stale sessions and malformed ranges must not shift styling onto candidate text.
#[cfg(target_os = "windows")]
#[gpui::test]
async fn ime_style_range_mapping_rejects_stale_or_invalid_source_ranges(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let block = cx.new(|cx| Block::with_record(cx, BlockRecord::paragraph("before")));

    cx.update(|window, cx| {
        block.update(cx, |block, block_cx| {
            block.selected_range = 1..4;
            <Block as EntityInputHandler>::composition_started(block, window, block_cx);
            <Block as EntityInputHandler>::replace_and_mark_text_in_range(
                block,
                None,
                "candidate",
                None,
                window,
                block_cx,
            );

            let owner = BlockImeCompositionOwner::BlockText;
            assert_eq!(block.ime_visible_range(&owner, 4..6), 10..12);
            assert_eq!(block.ime_visible_range(&owner, 99..100), 99..100);
            // Keep this reversed range as genuine invalid input; the mapper must not normalize it.
            assert_eq!(
                block.ime_visible_range(&owner, std::ops::Range { start: 4, end: 2 },),
                std::ops::Range { start: 4, end: 2 },
            );

            block
                .ime_composition
                .as_mut()
                .expect("the marked-text callback starts a composition")
                .baseline_fragment
                .push('!');
            assert_eq!(block.ime_visible_range(&owner, 4..6), 4..6);
        });
    });
}

/// Cancellation restores selection while deliberately reversed candidate endpoints stay malformed.
#[cfg(target_os = "windows")]
#[gpui::test]
async fn cancelled_and_empty_ime_cycles_restore_original_selection(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let block = cx.new(|cx| Block::with_record(cx, BlockRecord::paragraph("before")));

    cx.update(|window, cx| {
        block.update(cx, |block, block_cx| {
            block.selected_range = 1..4;
            block.selection_reversed = true;
            <Block as EntityInputHandler>::composition_started(block, window, block_cx);
            <Block as EntityInputHandler>::replace_and_mark_text_in_range(
                block,
                None,
                "kana",
                Some(std::ops::Range { start: 3, end: 1 }),
                window,
                block_cx,
            );
            assert_eq!(
                block
                    .ime_visible_text(&BlockImeCompositionOwner::BlockText)
                    .as_deref(),
                Some("bkanare")
            );
            assert_eq!(
                block.ime_render_selection(&BlockImeCompositionOwner::BlockText),
                Some((2..4, true))
            );
            <Block as EntityInputHandler>::composition_ended(
                block,
                CompositionEnd::Cancelled,
                window,
                block_cx,
            );
            assert_eq!(block.display_text(), "before");
            assert_eq!(block.selected_range, 1..4);
            assert!(block.selection_reversed);

            <Block as EntityInputHandler>::composition_started(block, window, block_cx);
            <Block as EntityInputHandler>::replace_text_in_range(block, None, "", window, block_cx);
            <Block as EntityInputHandler>::composition_ended(
                block,
                CompositionEnd::Committed,
                window,
                block_cx,
            );
            assert_eq!(block.display_text(), "before");
            assert_eq!(block.selected_range, 1..4);
            assert!(block.selection_reversed);
        });
    });
}

/// Native offsets inside an emoji or combining sequence expand to whole grapheme boundaries.
#[test]
fn ime_range_clamping_preserves_graphemes_and_selection_direction() {
    let family = "👨‍👩‍👧‍👦";
    let inside_family = family.find('\u{200d}').unwrap();
    assert_eq!(
        Block::clamp_ime_range(family, inside_family..inside_family + 1),
        0..family.len()
    );
    assert_eq!(
        Block::clamp_ime_range(family, inside_family..inside_family),
        0..0
    );
    assert_eq!(
        Block::clamp_ime_range_preserving_direction(family, inside_family + 1..inside_family,),
        family.len()..0
    );

    let combined = "a\u{301}";
    assert_eq!(Block::clamp_ime_range(combined, 1..2), 0..combined.len());
}
