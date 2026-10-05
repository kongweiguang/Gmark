// @author kongweiguang

//! GPUI [`EntityInputHandler`] implementation for Block.
//!
//! Bridges between GPUI's UTF-16-based IME subsystem and the block's
//! internal UTF-8 representation.  All range arguments from GPUI arrive
//! as UTF-16 offsets and are converted through `range_from_utf16` before
//! operating on the block's title.

use std::ops::Range;

use gmark_math_edit::{
    MathCursor2D, MathEditCommand, MathEditError, MathSelection, MathSlot, MathVisualProjection,
};
use gpui::*;

use super::Block;
use super::element;
use crate::components::{BlockEvent, UndoCaptureKind};
use crate::editor::math_edit::MathEditSession;

/// Maps only owners with a dedicated text paint boundary so slot preedit is never attributed to block paint.
fn input_paint_surface_for_ime_owner(
    owner: &BlockImeCompositionOwner,
) -> Option<crate::perf::InputPaintSurface> {
    match owner {
        BlockImeCompositionOwner::BlockText => Some(crate::perf::InputPaintSurface::BlockText),
        BlockImeCompositionOwner::CodeLanguage => {
            Some(crate::perf::InputPaintSurface::CodeLanguage)
        }
        BlockImeCompositionOwner::MathSource => Some(crate::perf::InputPaintSurface::MathSource),
        BlockImeCompositionOwner::MathSlot(_) => None,
    }
}

#[path = "input_parts/composition.rs"]
mod composition;
#[path = "input_parts/composition_owner.rs"]
mod composition_owner;
#[path = "input_parts/composition_view.rs"]
mod composition_view;
#[cfg(not(target_os = "windows"))]
#[path = "input_compat.rs"]
mod input_compat;
pub(crate) use composition::{
    BlockImeComposition, BlockImeCompositionOwner, BlockImeOriginalSelection,
};

impl Block {
    /// Captures only the owner, revision, and visible selection needed to reject stale paint traces.
    pub(crate) fn input_paint_snapshot(
        &self,
        surface: crate::perf::InputPaintSurface,
    ) -> crate::perf::InputPaintSnapshot {
        let (selection, reversed, editor_selection) = match surface {
            crate::perf::InputPaintSurface::BlockText => (
                &self.selected_range,
                self.selection_reversed,
                self.editor_selection_range.as_ref(),
            ),
            crate::perf::InputPaintSurface::CodeLanguage => (
                &self.code_language_selected_range,
                self.code_language_selection_reversed,
                None,
            ),
            crate::perf::InputPaintSurface::MathSource => (
                &self.math_source_selected_range,
                self.math_source_selection_reversed,
                None,
            ),
        };
        let composition_generation = self
            .ime_composition
            .as_ref()
            .filter(|composition| {
                input_paint_surface_for_ime_owner(&composition.owner) == Some(surface)
            })
            .and_then(|composition| composition.paint_generation);

        crate::perf::InputPaintSnapshot {
            surface,
            revision: self.document_revision,
            selection_start: selection.start,
            selection_end: selection.end,
            selection_reversed: reversed,
            editor_selection: editor_selection.map(|range| (range.start, range.end)),
            composition_generation,
        }
    }

    /// Starts a drag trace only after its Block selection has reached the rendered state.
    pub(crate) fn begin_selection_input_trace(&self, cx: &mut Context<Self>) {
        crate::perf::begin_input_to_gpui_paint(
            crate::perf::InputPaintKind::SelectionDrag,
            cx.entity().entity_id(),
            self.input_paint_snapshot(crate::perf::InputPaintSurface::BlockText),
        );
    }

    /// Completes a timed input with the post-edit selection snapshot for its actual text surface.
    pub(crate) fn finish_input_to_gpui_paint(
        &self,
        started: Option<crate::perf::InputPaintStart>,
        kind: crate::perf::InputPaintKind,
        surface: crate::perf::InputPaintSurface,
        cx: &mut Context<Self>,
    ) {
        crate::perf::finish_input_to_gpui_paint(
            started,
            kind,
            cx.entity().entity_id(),
            self.input_paint_snapshot(surface),
        );
    }

    fn math_source_range_from_utf16(&self, text: &str, range_utf16: &Range<usize>) -> Range<usize> {
        let range = Self::utf16_range_to_utf8_in(text, range_utf16);
        let start = range.start.min(text.len());
        let end = range.end.min(text.len());
        start.min(end)..start.max(end)
    }

    /// Places the system candidate window against the same overlaid source text shown on screen.
    fn math_source_bounds_for_range(
        &self,
        range_utf16: &Range<usize>,
        bounds: Bounds<Pixels>,
    ) -> Option<Bounds<Pixels>> {
        let text = self
            .ime_visible_text(&BlockImeCompositionOwner::MathSource)
            .unwrap_or_else(|| self.math_source_text());
        let range = Self::utf16_range_to_utf8_in(&text, range_utf16);
        let line = self.math_source_last_layout.as_ref()?;
        let layout_bounds = self.math_source_last_bounds.unwrap_or(bounds);
        let left = layout_bounds.left() + line.x_for_index(range.start);
        let right = layout_bounds.left() + line.x_for_index(range.end);
        Some(Bounds::from_corners(
            point(left, bounds.top()),
            point(right.max(left + px(1.0)), bounds.bottom()),
        ))
    }

    /// Return the slot currently owned by the structured formula editor.
    ///
    /// The semantic editor keeps the cursor and selection in slot-local UTF-8
    /// coordinates.  EntityInputHandler must never fall back to the block's
    /// Markdown-visible projection while this focus target is active: that
    /// projection includes delimiters and can belong to a different inline
    /// fragment entirely.
    fn math_input_context(&self) -> Option<(MathSlot, String)> {
        let session = self.math_edit_session.as_ref()?;
        let slot = session.editor().cursor().slot().clone();
        let text = Self::math_slot_text(session, &slot)?;
        Some((slot, text))
    }

    /// Resolve a slot's source without making the domain model expose its
    /// internal `slot_source` helper.  Probing with `usize::MAX` is safe: the
    /// domain cursor validates the offset and reports the slot length in the
    /// typed error.  This works for regular AST nodes and environment cells.
    fn math_slot_text(session: &MathEditSession, slot: &MathSlot) -> Option<String> {
        let document = session.document();
        let len = match MathCursor2D::at(document, slot.clone(), usize::MAX) {
            Ok(cursor) => cursor.offset(),
            Err(MathEditError::InvalidCursorOffset { len, .. }) => len,
            Err(_) => return None,
        };
        let start = MathCursor2D::at(document, slot.clone(), 0).ok()?;
        let end = MathCursor2D::at(document, slot.clone(), len).ok()?;
        MathSelection::new(start, end).selected_text(document)
    }

    fn math_selection_range(
        session: &MathEditSession,
        slot: &MathSlot,
        text_len: usize,
    ) -> (Range<usize>, bool) {
        let selection = session.editor().selection();
        if selection.is_structural() {
            return (0..text_len, false);
        }
        if selection.slot().is_some_and(|selected| selected == slot) {
            let range = selection.range().unwrap_or_else(|| {
                session.editor().cursor().offset()..session.editor().cursor().offset()
            });
            let start = range.start.min(text_len);
            let end = range.end.min(text_len);
            let reversed = selection.anchor().offset() > selection.focus().offset();
            return (start.min(end)..start.max(end), reversed);
        }
        let offset = session.editor().cursor().offset().min(text_len);
        (offset..offset, false)
    }

    fn math_clamped_range(text: &str, range: Range<usize>) -> Range<usize> {
        let start = range.start.min(text.len());
        let end = range.end.min(text.len());
        if start <= end { start..end } else { end..end }
    }

    fn set_math_selection(&mut self, slot: MathSlot, range: Range<usize>) -> bool {
        let Some(session) = self.math_edit_session.as_mut() else {
            return false;
        };
        let document = session.document().clone();
        let Ok(anchor) = MathCursor2D::at(&document, slot.clone(), range.start) else {
            return false;
        };
        let Ok(focus) = MathCursor2D::at(&document, slot, range.end) else {
            return false;
        };
        session
            .editor_mut()
            .set_selection(MathSelection::new(anchor, focus))
            .is_ok()
    }

    fn math_input_text(new_text: &str) -> String {
        // A formula slot is single-line.  IMEs and clipboard providers may
        // still send CRLF or raw line breaks; preserving them would make the
        // domain cursor and the visual slot disagree about their source.
        new_text.replace("\r\n", " ").replace(['\r', '\n'], " ")
    }

    /// Maps native formula-slot ranges through the disposable composition preview when present.
    fn math_bounds_for_range(
        &self,
        range_utf16: &Range<usize>,
        bounds: Bounds<Pixels>,
    ) -> Option<Bounds<Pixels>> {
        let (slot, base_text) = self.math_input_context()?;
        let owner = BlockImeCompositionOwner::MathSlot(slot.clone());
        let text = self.ime_visible_text(&owner).unwrap_or(base_text);
        let range =
            Self::math_clamped_range(&text, Self::utf16_range_to_utf8_in(&text, range_utf16));
        let preview = self.math_ime_preview_session();
        let session = preview.as_ref().or(self.math_edit_session.as_ref())?;
        let document = session.document();
        let anchor = MathCursor2D::at(document, slot.clone(), range.start).ok()?;
        let focus = MathCursor2D::at(document, slot, range.end).ok()?;
        let selection = MathSelection::new(anchor, focus);
        let projection = MathVisualProjection::from_document(document);
        let rect = projection
            .selection_rect(&selection)
            .or_else(|| projection.caret_rect(selection.focus()))?;
        let formula = projection.geometry().bounds();
        let formula_width = formula.width().max(f64::EPSILON);
        let formula_height = formula.height().max(f64::EPSILON);
        let scale_x = f64::from(bounds.size.width).max(0.0) / formula_width;
        let scale_y = f64::from(bounds.size.height).max(0.0) / formula_height;
        let left = bounds.left() + px((rect.x * scale_x) as f32);
        let top = bounds.top() + px((rect.y * scale_y) as f32);
        let right = left + px((rect.w * scale_x).max(0.0) as f32);
        let bottom = top + px((rect.h * scale_y).max(1.0) as f32);
        Some(Bounds::from_corners(point(left, top), point(right, bottom)))
    }
}

impl EntityInputHandler for Block {
    /// Pins the selection before platforms that report only a committed result can replace it.
    fn composition_started(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.begin_ime_composition(window, cx);
    }

    /// Exposes the virtual preedit view to the pinned OS target without publishing it.
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        if self.ime_reject_until_end {
            return None;
        }
        if let Some(owner) = self.ime_composition_owner()
            && let Some(text) = self.ime_visible_text(&owner)
        {
            let range = Self::utf16_range_to_utf8_in(&text, &range_utf16);
            actual_range.replace(Self::utf8_range_to_utf16_in(&text, &range));
            return text.get(range).map(ToOwned::to_owned);
        }

        if self.math_source_focus_handle.is_focused(_window) {
            let text = self.math_source_text();
            let range = self.math_source_range_from_utf16(&text, &range_utf16);
            actual_range.replace(Self::utf8_range_to_utf16_in(&text, &range));
            return text.get(range).map(ToOwned::to_owned);
        }

        if self.math_structure_focus_handle.is_focused(_window) && self.math_edit_session.is_some()
        {
            let (_, text) = self.math_input_context()?;
            let range =
                Self::math_clamped_range(&text, Self::utf16_range_to_utf8_in(&text, &range_utf16));
            actual_range.replace(Self::utf8_range_to_utf16_in(&text, &range));
            return text.get(range).map(ToOwned::to_owned);
        }

        if self.code_language_focus_handle.is_focused(_window) {
            let range = self.code_language_range_from_utf16(&range_utf16);
            actual_range.replace(self.code_language_range_to_utf16(&range));
            return Some(self.code_language_text()[range].to_string());
        }

        let range = self.range_from_utf16(&range_utf16);
        actual_range.replace(self.range_to_utf16(&range));
        Some(self.display_text()[range].to_string())
    }

    /// Keeps native selection queries in the same coordinate space as staged preedit text.
    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        if self.ime_reject_until_end {
            return None;
        }
        if let Some(owner) = self.ime_composition_owner() {
            return self.ime_selected_text_range(&owner);
        }

        if self.math_source_focus_handle.is_focused(_window) {
            let text = self.math_source_text();
            let (range, reversed) = self.math_source_selection();
            return Some(UTF16Selection {
                range: Self::utf8_range_to_utf16_in(&text, &range),
                reversed,
            });
        }

        if self.math_structure_focus_handle.is_focused(_window) && self.math_edit_session.is_some()
        {
            let (_, text) = self.math_input_context()?;
            let session = self.math_edit_session.as_ref()?;
            let (range, reversed) = Self::math_selection_range(
                session,
                &session.editor().cursor().slot().clone(),
                text.len(),
            );
            return Some(UTF16Selection {
                range: Self::utf8_range_to_utf16_in(&text, &range),
                reversed,
            });
        }

        if self.code_language_focus_handle.is_focused(_window) {
            return Some(UTF16Selection {
                range: self.code_language_range_to_utf16(&self.code_language_selected_range),
                reversed: self.code_language_selection_reversed,
            });
        }

        Some(UTF16Selection {
            range: self.range_to_utf16(&self.selected_range),
            reversed: self.selection_reversed,
        })
    }

    /// Reports the virtual marked range while preserving the distinction from terminal state.
    fn marked_text_range(
        &self,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        if self.ime_reject_until_end {
            return None;
        }
        if let Some(owner) = self.ime_composition_owner() {
            return self.ime_marked_text_range(&owner);
        }

        if self.math_source_focus_handle.is_focused(window) {
            let text = self.math_source_text();
            return self.math_source_marked_range.as_ref().map(|range| {
                let start = range.start.min(text.len());
                let end = range.end.min(text.len());
                Self::utf8_range_to_utf16_in(&text, &(start.min(end)..start.max(end)))
            });
        }

        if self.math_structure_focus_handle.is_focused(window) && self.math_edit_session.is_some() {
            let (_, text) = self.math_input_context()?;
            return self.math_marked_range.as_ref().map(|range| {
                let range = Self::math_clamped_range(&text, range.clone());
                Self::utf8_range_to_utf16_in(&text, &range)
            });
        }

        if self.code_language_focus_handle.is_focused(window) {
            return self
                .code_language_marked_range
                .as_ref()
                .map(|range| self.code_language_range_to_utf16(range));
        }

        self.marked_range
            .as_ref()
            .map(|range| self.range_to_utf16(range))
    }

    /// Treats unmark as a visual hint only; explicit composition termination owns rollback.
    fn unmark_text(&mut self, window: &mut Window, _cx: &mut Context<Self>) {
        if self.ime_composition.is_some() || self.ime_reject_until_end {
            return;
        }

        if self.math_source_focus_handle.is_focused(window) {
            self.math_source_marked_range = None;
            return;
        }

        if self.math_structure_focus_handle.is_focused(window) {
            self.math_marked_range = None;
            if self.math_edit_session.is_some() {
                return;
            }
        }

        if self.code_language_focus_handle.is_focused(window) {
            self.code_language_marked_range = None;
            return;
        }

        self.marked_range = None;
    }

    /// 普通 Source 文字先核对 Host 实际焦点；已暂存的 IME 结果仍由原组合 owner 收尾。
    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.ime_reject_until_end {
            return;
        }
        if self.ime_composition.is_some() && self.commit_ime_result(new_text, cx) {
            return;
        }
        if !self.source_host_input_is_focused(_window) {
            return;
        }
        if self.is_read_only() {
            return;
        }
        let paint_started = crate::perf::start_input_to_gpui_paint();
        if self.math_source_focus_handle.is_focused(_window) {
            let text = self.math_source_text();
            let visible_range = range_utf16
                .as_ref()
                .map(|range| self.math_source_range_from_utf16(&text, range))
                .or_else(|| self.math_source_marked_range.clone())
                .unwrap_or_else(|| self.math_source_selection().0);
            let was_marked = self.math_source_marked_range.is_some();
            let changed = self.replace_math_source_text_in_range(
                visible_range,
                new_text,
                None,
                false,
                if was_marked {
                    UndoCaptureKind::ImeCompositionCommit
                } else {
                    UndoCaptureKind::CoalescibleText
                },
                cx,
            );
            if changed || was_marked {
                self.math_source_marked_range = None;
            }
            if changed {
                self.finish_input_to_gpui_paint(
                    paint_started,
                    crate::perf::InputPaintKind::Typing,
                    crate::perf::InputPaintSurface::MathSource,
                    cx,
                );
            }
            return;
        }

        if self.math_structure_focus_handle.is_focused(_window) && self.math_edit_session.is_some()
        {
            let Some((slot, text)) = self.math_input_context() else {
                return;
            };
            let visible_range = range_utf16
                .as_ref()
                .map(|range| Self::utf16_range_to_utf8_in(&text, range))
                .or_else(|| self.math_marked_range.clone())
                .or_else(|| {
                    self.math_edit_session
                        .as_ref()
                        .map(|session| Self::math_selection_range(session, &slot, text.len()).0)
                })
                .unwrap_or(0..0);
            let visible_range = Self::math_clamped_range(&text, visible_range);
            let was_marked = self.math_marked_range.is_some();
            let _ = self.set_math_selection(slot, visible_range);
            let sanitized = Self::math_input_text(new_text);
            let changed = self.execute_math_command_live(
                MathEditCommand::InsertText(sanitized),
                if was_marked {
                    UndoCaptureKind::ImeCompositionCommit
                } else {
                    UndoCaptureKind::CoalescibleText
                },
                cx,
            );
            if !changed && was_marked {
                // `execute_math_command_live` only captures after a changed
                // command.  A provider may still send an identical final
                // commit; emit the sealing category so the open composition
                // is not left coalescing with subsequent typing.
                self.prepare_undo_capture(UndoCaptureKind::ImeCompositionCommit, cx);
            }
            // A commit seals the composition even when the provider sends an
            // empty final update.  The live command owns document publication;
            // this field only tracks GPUI's marked-text slice.
            if changed || was_marked {
                self.math_marked_range = None;
            }
            return;
        }

        let committing_composition = self.marked_range.is_some();
        if self.code_language_focus_handle.is_focused(_window) {
            let undo_kind = if self.code_language_marked_range.is_some() {
                UndoCaptureKind::ImeCompositionCommit
            } else {
                UndoCaptureKind::CoalescibleText
            };
            let visible_range = range_utf16
                .as_ref()
                .map(|range| self.code_language_range_from_utf16(range))
                .or(self.code_language_marked_range.clone())
                .unwrap_or(self.code_language_selected_range.clone());
            let original_text = self.code_language_text().to_owned();
            let original_selection = self.code_language_selected_range.clone();
            self.prepare_undo_capture(undo_kind, cx);
            self.replace_code_language_text_in_range(visible_range, new_text, None, false, cx);
            if self.code_language_text() != original_text
                || self.code_language_selected_range != original_selection
            {
                self.finish_input_to_gpui_paint(
                    paint_started,
                    crate::perf::InputPaintKind::Typing,
                    crate::perf::InputPaintSurface::CodeLanguage,
                    cx,
                );
            }
            return;
        }

        if self.editor_selection_range.is_some() {
            cx.emit(BlockEvent::RequestReplaceCrossBlockSelection {
                text: new_text.to_string(),
                selected_range_relative: None,
                mark_inserted_text: false,
                undo_kind: if committing_composition {
                    UndoCaptureKind::ImeCompositionCommit
                } else {
                    UndoCaptureKind::CoalescibleText
                },
            });
            return;
        }

        let visible_range = range_utf16
            .as_ref()
            .map(|range| self.range_from_utf16(range))
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());
        let should_trace_input = !new_text.is_empty() || !visible_range.is_empty();
        if self.try_apply_auto_pair_input(visible_range.clone(), new_text, cx) {
            if should_trace_input {
                self.finish_input_to_gpui_paint(
                    paint_started,
                    crate::perf::InputPaintKind::Typing,
                    crate::perf::InputPaintSurface::BlockText,
                    cx,
                );
            }
            return;
        }
        self.prepare_undo_capture(
            if committing_composition {
                UndoCaptureKind::ImeCompositionCommit
            } else {
                UndoCaptureKind::CoalescibleText
            },
            cx,
        );
        self.replace_text_in_visible_range(visible_range, new_text, None, false, cx);
        if should_trace_input {
            self.finish_input_to_gpui_paint(
                paint_started,
                crate::perf::InputPaintKind::Typing,
                crate::perf::InputPaintSurface::BlockText,
                cx,
            );
        }
    }

    /// 新组合只从仍聚焦的 Host 输入桥启动，已开始的组合继续由原 owner 接收候选终态。
    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.ime_reject_until_end {
            return;
        }
        if !self.source_host_input_is_focused(_window) && !self.has_ime_composition() {
            return;
        }
        #[cfg(target_os = "windows")]
        self.stage_ime_preedit(range_utf16, new_text, new_selected_range_utf16, _window, cx);
        #[cfg(not(target_os = "windows"))]
        self.replace_and_mark_text_compat(
            range_utf16,
            new_text,
            new_selected_range_utf16,
            _window,
            cx,
        );
    }

    /// Delivers the terminal result first, then replays queued unmanaged commands after Changed listeners run.
    fn composition_ended(
        &mut self,
        end: CompositionEnd,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.end_ime_composition(end, cx);
        let block = cx.entity().downgrade();
        window.defer(cx, move |window, cx| {
            let _ = block.update(cx, |block, cx| {
                block.replay_unmanaged_input_commands(window, cx);
            });
        });
    }

    /// Preserves the active preedit after IMM rejects a deferred completion request.
    fn composition_finish_failed(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.ime_composition_finish_failed(cx);
    }

    /// Resolves candidate geometry from the text surface that owns the native composition.
    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        if self.ime_reject_until_end {
            return None;
        }
        let owner = self
            .ime_composition_owner()
            .or_else(|| self.focused_ime_owner(_window));
        if let Some(owner) = owner {
            match owner {
                BlockImeCompositionOwner::MathSource => {
                    return self.math_source_bounds_for_range(&range_utf16, bounds);
                }
                BlockImeCompositionOwner::MathSlot(_) => {
                    return self.math_bounds_for_range(&range_utf16, bounds);
                }
                BlockImeCompositionOwner::CodeLanguage => {
                    let text = self.ime_visible_text(&BlockImeCompositionOwner::CodeLanguage)?;
                    let line = self.code_language_last_layout.as_ref()?;
                    let range = Self::utf16_range_to_utf8_in(&text, &range_utf16);
                    let start_x = line.x_for_index(range.start);
                    let end_x = line.x_for_index(range.end);
                    return Some(Bounds::from_corners(
                        point(bounds.left() + start_x, bounds.top()),
                        point(bounds.left() + end_x, bounds.bottom()),
                    ));
                }
                BlockImeCompositionOwner::BlockText => {}
            }
        }

        if self.code_language_focus_handle.is_focused(_window) {
            let line = self.code_language_last_layout.as_ref()?;
            let range = self.code_language_range_from_utf16(&range_utf16);
            let start_x = line.x_for_index(range.start);
            let end_x = line.x_for_index(range.end);
            return Some(Bounds::from_corners(
                point(bounds.left() + start_x, bounds.top()),
                point(bounds.left() + end_x, bounds.bottom()),
            ));
        }

        let lines = self.last_layout.as_ref()?;
        let line_height = self.last_line_height;
        let text = self
            .ime_visible_text(&BlockImeCompositionOwner::BlockText)
            .unwrap_or_else(|| self.display_text().to_owned());
        let range = Self::utf16_range_to_utf8_in(&text, &range_utf16);
        element::range_bounds(lines, bounds, line_height, &text, range, self.text_align())
    }

    /// Maps pointer hit tests against candidate text to UTF-16 coordinates in the pinned surface.
    fn character_index_for_point(
        &mut self,
        pt: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        if self.ime_reject_until_end {
            return None;
        }
        let owner = self
            .ime_composition_owner()
            .or_else(|| self.focused_ime_owner(_window));
        match owner {
            Some(BlockImeCompositionOwner::MathSource) => {
                let text = self.ime_visible_text(&BlockImeCompositionOwner::MathSource)?;
                let bounds = self.math_source_last_bounds?;
                let line = self.math_source_last_layout.as_ref()?;
                let index = line
                    .closest_index_for_x(pt.x - bounds.left())
                    .min(text.len());
                return Some(Self::utf8_to_utf16_in(&text, index));
            }
            Some(BlockImeCompositionOwner::CodeLanguage) => {
                let text = self.ime_visible_text(&BlockImeCompositionOwner::CodeLanguage)?;
                let bounds = self.code_language_last_bounds?;
                let line = self.code_language_last_layout.as_ref()?;
                let index = if pt.x <= bounds.left() {
                    0
                } else if pt.x >= bounds.right() {
                    text.len()
                } else {
                    line.closest_index_for_x(pt.x - bounds.left())
                        .min(text.len())
                };
                return Some(Self::utf8_to_utf16_in(&text, index));
            }
            Some(BlockImeCompositionOwner::MathSlot(slot)) => {
                let text = self.ime_visible_text(&BlockImeCompositionOwner::MathSlot(slot))?;
                let bounds = self.last_bounds?;
                let lines = self.last_layout.as_ref()?;
                let ranges = element::hard_line_ranges(&text);
                let relative = Point {
                    x: pt.x - bounds.left(),
                    y: pt.y - bounds.top(),
                };
                let (line_idx, y_in_line) =
                    element::wrapped_line_for_y(lines, self.last_line_height, relative.y)?;
                let layout = &lines[line_idx];
                let origin_x = element::aligned_line_left(layout, bounds, self.text_align());
                let offset = match layout.closest_index_for_position(
                    point(pt.x - origin_x, y_in_line),
                    self.last_line_height,
                ) {
                    Ok(index) | Err(index) => index,
                };
                return Some(Self::utf8_to_utf16_in(
                    &text,
                    ranges[line_idx].start + offset,
                ));
            }
            Some(BlockImeCompositionOwner::BlockText) | None => {}
        }

        let bounds = self.last_bounds?;
        let lines = self.last_layout.as_ref()?;
        let text = self
            .ime_visible_text(&BlockImeCompositionOwner::BlockText)
            .unwrap_or_else(|| self.display_text().to_owned());
        let ranges = element::hard_line_ranges(&text);
        let relative = Point {
            x: pt.x - bounds.left(),
            y: pt.y - bounds.top(),
        };
        let (line_idx, y_in_line) =
            element::wrapped_line_for_y(lines, self.last_line_height, relative.y)?;
        let layout = &lines[line_idx];
        let origin_x = element::aligned_line_left(layout, bounds, self.text_align());
        let utf8_offset_in_line = match layout
            .closest_index_for_position(point(pt.x - origin_x, y_in_line), self.last_line_height)
        {
            Ok(idx) | Err(idx) => idx,
        };
        let utf8_index = ranges[line_idx].start + utf8_offset_in_line;
        Some(Self::utf8_to_utf16_in(&text, utf8_index))
    }
}
