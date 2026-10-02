// @author kongweiguang

//! Native composition ownership and ordered deferred actions for Source rows.

use super::*;
use crate::components::{BlockImeCompositionOwner, LineOperation};
use gpui::AnyWindowHandle;

#[derive(Clone)]
pub(super) enum DeferredSourceAction {
    Block(BlockHostAction),
    /// Pins a deferred submit to the input that emitted it, even when another field owns the composition.
    HostInputSubmit {
        input: Entity<Block>,
    },
    StructuredCellSelect(StructuredCellEdit),
    Line(LineOperation),
    Copy,
    Cut,
    Paste,
    Delete,
    DeleteBack,
    SelectAll,
    FormatDocument,
    FormatSelection,
    SaveAs,
    Vertical {
        direction: i32,
        extend: bool,
    },
    Page {
        direction: i32,
        extend: bool,
    },
    DocumentBoundary {
        at_end: bool,
        extend: bool,
    },
    Pointer(DeferredSourcePointerAction),
}

#[derive(Clone)]
pub(super) enum DeferredSourcePointerAction {
    Down {
        snapshot: SourcePointerSnapshot,
        click_count: usize,
        shift: bool,
    },
    Move {
        snapshot: SourcePointerSnapshot,
        position: Point<Pixels>,
    },
    End {
        revision: u64,
    },
}

#[derive(Clone)]
pub(super) struct SourcePointerSnapshot {
    pub(super) row_entity: Entity<Block>,
    pub(super) line: usize,
    pub(super) row_range: Range<u64>,
    pub(super) selection: SourceSelection,
    pub(super) hit: SourceAnchor,
    pub(super) revision: u64,
}

pub(super) struct PendingSourceImeAction {
    pub(super) owner: Entity<Block>,
    pub(super) owner_kind: BlockImeCompositionOwner,
    pub(super) actions: VecDeque<DeferredSourceAction>,
    pub(super) ready: bool,
    pub(super) request_in_flight: bool,
    pub(super) replay_scheduled: bool,
    pub(super) window: AnyWindowHandle,
}

impl DocumentHost {
    /// Includes every mounted Host input owner so saves and focus changes wait for its native terminal event.
    pub(crate) fn has_active_ime_composition(&self, cx: &App) -> bool {
        self.pending_source_ime_action
            .as_ref()
            .is_some_and(|pending| pending.owner.read(cx).has_ime_composition())
            || self
                .source_row_blocks
                .values()
                .any(|block| block.read(cx).has_ime_composition())
            || self.structured_cell_input.read(cx).has_ime_composition()
            || self.graph_edit_input.read(cx).has_ime_composition()
            || self.search_input.read(cx).has_ime_composition()
            || self.navigation_input.read(cx).has_ime_composition()
            || self.structured_filter_input.read(cx).has_ime_composition()
    }

    /// Sends terminal events from non-row fields through the same identity-pinned action queue.
    pub(super) fn on_host_input_ime_event(
        &mut self,
        block: Entity<Block>,
        event: &BlockEvent,
        cx: &mut Context<Self>,
    ) {
        self.on_source_ime_terminal(&block, event, cx);
    }

    /// Reports whether a cached Block must remain addressable until its native terminal event.
    pub(super) fn is_pending_source_ime_owner(&self, block: &Entity<Block>) -> bool {
        self.pending_source_ime_action
            .as_ref()
            .is_some_and(|pending| pending.owner == *block)
    }

    /// Clears stale row entities but keeps the Block that still owns a native composition.
    pub(super) fn clear_source_row_blocks_except_ime_owner(&mut self) {
        let owner = self
            .pending_source_ime_action
            .as_ref()
            .map(|pending| pending.owner.clone());
        self.source_row_blocks
            .retain(|_, block| owner.as_ref().is_some_and(|owner| owner == block));
    }

    /// Pins a Source action to the Block currently owning native composition until the OS reports its terminal result.
    pub(super) fn defer_source_action_for_ime(
        &mut self,
        action: DeferredSourceAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let host_fields = [
            &self.structured_cell_input,
            &self.graph_edit_input,
            &self.search_input,
            &self.navigation_input,
            &self.structured_filter_input,
        ];
        let source_target = self
            .source_row_blocks
            .values()
            .chain(host_fields)
            .find_map(|block| {
                let block_state = block.read(cx);
                block_state.has_ime_composition().then(|| {
                    (
                        block.clone(),
                        block_state
                            .ime_composition_owner()
                            .unwrap_or(BlockImeCompositionOwner::BlockText),
                    )
                })
            });
        let Some((owner, owner_kind)) = source_target else {
            if let Some(pending) = self.pending_source_ime_action.as_mut() {
                enqueue_source_ime_action(&mut pending.actions, action);
                pending.window = window.window_handle();
                if !pending.owner.read(cx).has_ime_composition() {
                    pending.request_in_flight = false;
                    pending.ready = true;
                    self.schedule_source_ime_replay(cx);
                }
                return true;
            }
            return false;
        };

        if let Some(pending) = self.pending_source_ime_action.as_mut() {
            enqueue_source_ime_action(&mut pending.actions, action);
            if pending.owner != owner {
                pending.owner = owner;
                pending.owner_kind = owner_kind;
                pending.ready = false;
                pending.request_in_flight = false;
            }
            pending.window = window.window_handle();
            if pending.ready || pending.request_in_flight {
                return true;
            }
        } else {
            self.pending_source_ime_action = Some(PendingSourceImeAction {
                owner,
                owner_kind,
                actions: VecDeque::from([action]),
                ready: false,
                request_in_flight: false,
                replay_scheduled: false,
                window: window.window_handle(),
            });
        }
        self.request_source_ime_finish(window, cx);
        true
    }

    /// Repositions deferred pointer endpoints through each contiguous shared-document revision.
    pub(super) fn rebase_pending_source_pointer_actions(
        &mut self,
        view_id: DocumentViewInstanceId,
        revision: DocumentRevision,
        mutation: &DocumentMutationMap,
        cx: &mut Context<Self>,
    ) {
        let Some(document) = self.document.clone() else {
            return;
        };
        let mut conflicted = false;
        if let Some(pending) = self.pending_source_ime_action.as_mut() {
            for action in &mut pending.actions {
                if !rebase_deferred_source_pointer_action(
                    action,
                    revision.0,
                    view_id == document.view_id(),
                    mutation,
                    &document,
                ) {
                    conflicted = true;
                    break;
                }
            }
            if conflicted {
                pending
                    .actions
                    .retain(|action| !matches!(action, DeferredSourceAction::Pointer(_)));
            }
        }
        if conflicted {
            self.end_source_pointer_selection();
            self.error = Some("源码在拖选期间发生变化，已取消本次鼠标选择。".into());
            cx.notify();
        } else {
            self.schedule_source_ime_replay(cx);
        }
    }

    /// Requests native composition completion while retaining the queue on failure for retry.
    pub(super) fn request_source_ime_finish(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(pending) = self.pending_source_ime_action.as_mut() else {
            return;
        };
        if pending.request_in_flight || pending.ready {
            return;
        }
        if !pending.owner.read(cx).has_ime_composition() {
            pending.ready = true;
            self.schedule_source_ime_replay(cx);
            return;
        }

        match window.finish_ime_composition() {
            Ok(true) => pending.request_in_flight = true,
            Ok(false) | Err(_) => {
                pending.request_in_flight = false;
                self.error = Some("输入法仍在编辑候选文本。请确认或取消候选后重试操作。".into());
                cx.notify();
            }
        }
    }

    /// Releases queued actions only after the original input Block reports a terminal result.
    pub(super) fn on_source_ime_terminal(
        &mut self,
        block: &Entity<Block>,
        event: &BlockEvent,
        cx: &mut Context<Self>,
    ) {
        let Some(pending) = self
            .pending_source_ime_action
            .as_mut()
            .filter(|pending| pending.owner == *block)
        else {
            return;
        };
        match event {
            BlockEvent::ImeCompositionEnded { .. } if !block.read(cx).has_ime_composition() => {
                pending.request_in_flight = false;
                pending.ready = true;
                self.schedule_source_ime_replay(cx);
            }
            BlockEvent::ImeCompositionFinishFailed => {
                pending.request_in_flight = false;
                pending.ready = false;
                pending.replay_scheduled = false;
                self.error = Some("输入法尚未结束组合输入。请确认或取消候选后重试操作。".into());
                cx.notify();
            }
            _ => {}
        }
    }

    /// Defers action replay until Block event propagation has committed the final text.
    pub(super) fn schedule_source_ime_replay(&mut self, cx: &mut Context<Self>) {
        if !self
            .pending_source_ime_action
            .as_ref()
            .is_some_and(|pending| pending.ready)
        {
            return;
        }
        let Some(pending) = self.pending_source_ime_action.as_mut() else {
            return;
        };
        if pending.replay_scheduled {
            return;
        }
        pending.replay_scheduled = true;
        let host = cx.entity().downgrade();
        let window_handle = pending.window;
        cx.defer(move |cx| {
            let _ = cx.update_window(window_handle, move |_view, window, cx| {
                if let Some(host) = host.upgrade() {
                    host.update(cx, |view, cx| {
                        view.replay_source_ime_action(window, cx);
                    });
                }
            });
        });
    }

    /// Rechecks the pinned owner before replay so late actions cannot overtake a new composition.
    fn replay_source_ime_action(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mut pending) = self.pending_source_ime_action.take() else {
            return;
        };
        pending.replay_scheduled = false;
        if !pending.ready {
            self.pending_source_ime_action = Some(pending);
            return;
        }
        if pending.owner.read(cx).has_ime_composition() {
            pending.ready = false;
            pending.request_in_flight = false;
            self.pending_source_ime_action = Some(pending);
            self.request_source_ime_finish(window, cx);
            return;
        }
        let current_revision = self.document.as_ref().map_or(0, SharedDocument::revision);
        if pending
            .actions
            .front()
            .and_then(deferred_source_pointer_revision)
            .is_some_and(|_| self.displayed_screen_lines.document_revision < current_revision)
        {
            // Wait for the immutable viewport snapshot matching the committed offsets before hit-tested selection resumes.
            self.pending_source_ime_action = Some(pending);
            return;
        }
        if pending
            .actions
            .front()
            .and_then(deferred_source_pointer_revision)
            .is_some_and(|revision| revision != current_revision)
        {
            // Controller revisions arrive asynchronously; old hit-test offsets must wait
            // for their mutation map instead of being interpreted against the new layout.
            pending.replay_scheduled = false;
            self.pending_source_ime_action = Some(pending);
            return;
        }
        let Some(action) = pending.actions.pop_front() else {
            return;
        };
        match action {
            DeferredSourceAction::Block(action) => {
                self.on_line_edit_host_action(action, window, cx);
            }
            DeferredSourceAction::HostInputSubmit { input } => {
                let value = input.read(cx).display_text().to_owned();
                if input == self.structured_cell_input {
                    self.on_structured_cell_host_action(
                        BlockHostAction::Submit(value.into()),
                        window,
                        cx,
                    );
                } else if input == self.search_input {
                    self.on_search_host_action(BlockHostAction::Submit(value.into()), window, cx);
                } else if input == self.navigation_input {
                    self.on_navigation_host_action(
                        BlockHostAction::Submit(value.into()),
                        window,
                        cx,
                    );
                }
            }
            DeferredSourceAction::StructuredCellSelect(target) => {
                self.select_structured_cell(target, window, cx);
            }
            DeferredSourceAction::Line(operation) => {
                self.apply_source_line_operation(operation, cx);
            }
            DeferredSourceAction::Copy => self.on_copy(&Copy, window, cx),
            DeferredSourceAction::Cut => self.on_cut(&Cut, window, cx),
            DeferredSourceAction::Paste => self.on_paste(&Paste, window, cx),
            DeferredSourceAction::Delete => self.on_delete(&Delete, window, cx),
            DeferredSourceAction::DeleteBack => self.on_delete_back(&DeleteBack, window, cx),
            DeferredSourceAction::SelectAll => self.on_select_all(&SelectAll, window, cx),
            DeferredSourceAction::FormatDocument => {
                self.on_format_document(&FormatDocument, window, cx);
            }
            DeferredSourceAction::FormatSelection => {
                self.on_format_selection(&FormatSelection, window, cx);
            }
            DeferredSourceAction::SaveAs => {
                self.on_save_document_as(&SaveDocumentAs, window, cx);
            }
            DeferredSourceAction::Vertical { direction, extend } => {
                self.move_source_caret_by_visual_lines(direction, 1, extend, window, cx);
            }
            DeferredSourceAction::Page { direction, extend } => {
                self.move_source_page(direction, extend, window, cx);
            }
            DeferredSourceAction::DocumentBoundary { at_end, extend } => {
                self.on_source_document_boundary(at_end, extend, window, cx);
            }
            DeferredSourceAction::Pointer(action) => {
                self.replay_source_pointer_action(action, window, cx);
            }
        }
        if !pending.actions.is_empty() {
            pending.ready = true;
            pending.request_in_flight = false;
            self.pending_source_ime_action = Some(pending);
            if self.has_active_ime_composition(cx) {
                if let Some(pending) = self.pending_source_ime_action.as_mut() {
                    pending.ready = false;
                }
                self.request_source_ime_finish(window, cx);
            } else {
                self.schedule_source_ime_replay(cx);
            }
        }
    }
}

/// Bounds queued pointer traffic by replacing only the final adjacent Move snapshot.
fn enqueue_source_ime_action(
    actions: &mut VecDeque<DeferredSourceAction>,
    action: DeferredSourceAction,
) {
    match action {
        DeferredSourceAction::Pointer(current @ DeferredSourcePointerAction::Move { .. }) => {
            if let Some(DeferredSourceAction::Pointer(
                previous @ DeferredSourcePointerAction::Move { .. },
            )) = actions.back_mut()
            {
                *previous = current;
            } else {
                actions.push_back(DeferredSourceAction::Pointer(current));
            }
        }
        action => actions.push_back(action),
    }
}

/// Maps one captured gesture step and rejects stale revision gaps or edits to its target row.
fn rebase_deferred_source_pointer_action(
    action: &mut DeferredSourceAction,
    revision: u64,
    owner_view_mutation: bool,
    mutation: &DocumentMutationMap,
    document: &SharedDocument,
) -> bool {
    let DeferredSourceAction::Pointer(pointer) = action else {
        return true;
    };
    match pointer {
        DeferredSourcePointerAction::Down { snapshot, .. }
        | DeferredSourcePointerAction::Move { snapshot, .. } => {
            if snapshot.revision >= revision {
                return true;
            }
            if snapshot.revision.checked_add(1) != Some(revision)
                || source_pointer_mutation_conflicts(snapshot, mutation, owner_view_mutation)
            {
                return false;
            }
            snapshot.selection = mutation.map_selection(snapshot.selection);
            snapshot.hit = mutation.map_anchor(snapshot.hit);
            let start = mutation
                .map_anchor(SourceAnchor::new(
                    snapshot.row_range.start,
                    SourceAffinity::After,
                ))
                .byte_offset;
            let end = mutation
                .map_anchor(SourceAnchor::new(
                    snapshot.row_range.end,
                    SourceAffinity::Before,
                ))
                .byte_offset;
            if start > end {
                return false;
            }
            snapshot.row_range = start..end;
            snapshot.line = document
                .line_for_offset(snapshot.row_range.start.min(document.len()))
                .and_then(|line| usize::try_from(line).ok())
                .unwrap_or(snapshot.line);
            snapshot.revision = revision;
            true
        }
        DeferredSourcePointerAction::End {
            revision: source_revision,
        } => {
            if *source_revision >= revision {
                return true;
            }
            if source_revision.checked_add(1) != Some(revision) {
                return false;
            }
            *source_revision = revision;
            true
        }
    }
}

/// Treats changed hit text, selected bytes, or the captured row as a stale pointer target.
fn source_pointer_mutation_conflicts(
    snapshot: &SourcePointerSnapshot,
    mutation: &DocumentMutationMap,
    owner_view_mutation: bool,
) -> bool {
    let selection = snapshot.selection.range();
    mutation.edits().iter().any(|edit| {
        let point_touched = if edit.range.is_empty() {
            edit.range.start == snapshot.hit.byte_offset
        } else {
            edit.range.start <= snapshot.hit.byte_offset
                && snapshot.hit.byte_offset < edit.range.end
        };
        let selection_touched = if selection.is_empty() {
            !edit.range.is_empty()
                && edit.range.start <= selection.start
                && selection.start < edit.range.end
        } else if edit.range.is_empty() {
            selection.start < edit.range.start && edit.range.start < selection.end
        } else {
            edit.range.start < selection.end && selection.start < edit.range.end
        };
        let row_touched = if edit.range.is_empty() {
            snapshot.row_range.start <= edit.range.start
                && edit.range.start <= snapshot.row_range.end
        } else {
            edit.range.start < snapshot.row_range.end && snapshot.row_range.start < edit.range.end
        };
        point_touched || (selection_touched && !owner_view_mutation) || row_touched
    })
}

/// Returns the source revision guarding the first pointer action in the replay queue.
fn deferred_source_pointer_revision(action: &DeferredSourceAction) -> Option<u64> {
    match action {
        DeferredSourceAction::Pointer(DeferredSourcePointerAction::Down { snapshot, .. })
        | DeferredSourceAction::Pointer(DeferredSourcePointerAction::Move { snapshot, .. }) => {
            Some(snapshot.revision)
        }
        DeferredSourceAction::Pointer(DeferredSourcePointerAction::End { revision }) => {
            Some(*revision)
        }
        _ => None,
    }
}
