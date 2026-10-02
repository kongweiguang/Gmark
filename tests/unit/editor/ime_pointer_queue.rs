// @author kongweiguang

use super::super::{
    DeferredImeOperation, DeferredSurfacePointerEndpoint, DeferredSurfacePointerInteraction, Editor,
};
use crate::editor::selection_surface::SelectionSurface;
use crate::i18n::I18nManager;
use gpui::{AppContext, TestAppContext};

/// Initializes the globals needed by Editor entities in focused IME queue tests.
fn init(cx: &mut TestAppContext) {
    cx.update(|cx| {
        I18nManager::init(cx);
        crate::theme::ThemeManager::init(cx);
        crate::components::init(cx);
    });
}

#[gpui::test]
/// Coalesces same-gesture surface and Block moves while retaining each move stream's FIFO position.
async fn ime_pointer_moves_coalesce_within_their_surface_gesture(cx: &mut TestAppContext) {
    init(cx);
    let visual = cx.add_empty_window();
    let editor = visual.new(|cx| Editor::from_markdown(cx, "alpha beta".into(), None));
    visual.update(|_window, cx| {
        editor.update(cx, |editor, cx| {
            let block = editor.document.first_root().unwrap().clone();
            let entity_id = block.entity_id();
            let tab = editor
                .tabs
                .records
                .get(editor.tabs.active)
                .map(|tab| tab.id);
            let revision = editor.source_document.revision();
            let endpoint = |clean_offset| {
                DeferredSurfacePointerEndpoint::new(entity_id, clean_offset, revision)
            };
            let owner = block.clone();

            editor.queue_ime_operation(
                DeferredImeOperation::SurfacePointerSelection {
                    tab,
                    surface: SelectionSurface::Main,
                    owner: owner.clone(),
                    interaction: DeferredSurfacePointerInteraction::Begin {
                        anchor: endpoint(0),
                        target_entity_id: entity_id,
                        extend: false,
                    },
                },
                cx,
            );
            editor.queue_ime_operation(
                DeferredImeOperation::SurfacePointerSelection {
                    tab,
                    surface: SelectionSurface::Main,
                    owner: owner.clone(),
                    interaction: DeferredSurfacePointerInteraction::Move { focus: endpoint(1) },
                },
                cx,
            );
            editor.queue_ime_operation(
                DeferredImeOperation::InputInteraction {
                    tab,
                    surface: SelectionSurface::Main,
                    block: block.clone(),
                    interaction:
                        crate::components::block::BlockImeInteraction::PointerSelectionMove {
                            clean_offset: 2,
                        },
                },
                cx,
            );
            editor.queue_ime_operation(
                DeferredImeOperation::SurfacePointerSelection {
                    tab,
                    surface: SelectionSurface::Main,
                    owner,
                    interaction: DeferredSurfacePointerInteraction::Move { focus: endpoint(3) },
                },
                cx,
            );
            editor.queue_ime_operation(
                DeferredImeOperation::InputInteraction {
                    tab,
                    surface: SelectionSurface::Main,
                    block,
                    interaction:
                        crate::components::block::BlockImeInteraction::PointerSelectionMove {
                            clean_offset: 4,
                        },
                },
                cx,
            );

            assert_eq!(editor.pending_ime_operations.len(), 3);
            assert!(matches!(
                editor.pending_ime_operations.front(),
                Some(DeferredImeOperation::SurfacePointerSelection {
                    interaction: DeferredSurfacePointerInteraction::Begin {
                        target_entity_id: queued_target,
                        ..
                    },
                    ..
                }) if queued_target == &entity_id
            ));
            assert!(matches!(
                editor.pending_ime_operations.get(1),
                Some(DeferredImeOperation::SurfacePointerSelection {
                    interaction: DeferredSurfacePointerInteraction::Move {
                        focus: DeferredSurfacePointerEndpoint {
                            clean_offset: 3,
                            ..
                        }
                    },
                    ..
                })
            ));
            assert!(matches!(
                editor.pending_ime_operations.get(2),
                Some(DeferredImeOperation::InputInteraction {
                    interaction:
                        crate::components::block::BlockImeInteraction::PointerSelectionMove {
                            clean_offset: 4,
                        },
                    ..
                })
            ));
        });
    });
}

#[gpui::test]
/// A release terminates a move run, so later movement cannot replace an earlier gesture endpoint.
async fn ime_pointer_end_is_a_move_coalescing_barrier(cx: &mut TestAppContext) {
    init(cx);
    let visual = cx.add_empty_window();
    let editor = visual.new(|cx| Editor::from_markdown(cx, "alpha beta".into(), None));
    visual.update(|_window, cx| {
        editor.update(cx, |editor, cx| {
            let block = editor.document.first_root().unwrap().clone();
            let entity_id = block.entity_id();
            let tab = editor
                .tabs
                .records
                .get(editor.tabs.active)
                .map(|tab| tab.id);
            let revision = editor.source_document.revision();
            let endpoint = |clean_offset| {
                DeferredSurfacePointerEndpoint::new(entity_id, clean_offset, revision)
            };
            let owner = block.clone();
            for interaction in [
                DeferredSurfacePointerInteraction::Begin {
                    anchor: endpoint(0),
                    target_entity_id: entity_id,
                    extend: false,
                },
                DeferredSurfacePointerInteraction::Move { focus: endpoint(1) },
                DeferredSurfacePointerInteraction::End,
                DeferredSurfacePointerInteraction::Begin {
                    anchor: endpoint(3),
                    target_entity_id: entity_id,
                    extend: false,
                },
            ] {
                editor.queue_ime_operation(
                    DeferredImeOperation::SurfacePointerSelection {
                        tab,
                        surface: SelectionSurface::Main,
                        owner: owner.clone(),
                        interaction,
                    },
                    cx,
                );
            }

            assert_eq!(editor.pending_ime_operations.len(), 4);
            assert!(matches!(
                editor.pending_ime_operations.get(1),
                Some(DeferredImeOperation::SurfacePointerSelection {
                    interaction: DeferredSurfacePointerInteraction::Move {
                        focus: DeferredSurfacePointerEndpoint {
                            clean_offset: 1,
                            ..
                        }
                    },
                    ..
                })
            ));
            assert!(matches!(
                editor.pending_ime_operations.get(2),
                Some(DeferredImeOperation::SurfacePointerSelection {
                    interaction: DeferredSurfacePointerInteraction::End,
                    ..
                })
            ));
        });
    });
}
