// @author kongweiguang

//! Keeps marked-text updates eager on platforms whose GPUI adapters lack an IME terminal callback.

use super::*;

impl Block {
    /// Preserves existing per-platform marked-range behavior until its adapter reports composition end.
    pub(super) fn replace_and_mark_text_compat(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.is_read_only() {
            return;
        }
        if self.math_source_focus_handle.is_focused(window) {
            let text = self.math_source_text();
            let visible_range = range_utf16
                .as_ref()
                .map(|range| self.math_source_range_from_utf16(&text, range))
                .or_else(|| self.math_source_marked_range.clone())
                .unwrap_or_else(|| self.math_source_selection().0);
            let selected_range_relative = new_selected_range_utf16
                .as_ref()
                .map(|range| Self::utf16_range_to_utf8_in(new_text, range));
            let changed = self.replace_math_source_text_in_range(
                visible_range,
                new_text,
                selected_range_relative,
                !new_text.is_empty(),
                UndoCaptureKind::ImeComposition,
                cx,
            );
            if !changed {
                self.math_source_marked_range = None;
            }
            return;
        }

        if self.math_structure_focus_handle.is_focused(window) && self.math_edit_session.is_some() {
            let Some((slot, text)) = self.math_input_context() else {
                return;
            };
            let visible_range = range_utf16
                .as_ref()
                .map(|range| Self::utf16_range_to_utf8_in(&text, range))
                .or_else(|| self.math_marked_range.clone())
                .unwrap_or_else(|| {
                    self.math_edit_session
                        .as_ref()
                        .map(|session| Self::math_selection_range(session, &slot, text.len()).0)
                        .unwrap_or(0..0)
                });
            let visible_range = Self::math_clamped_range(&text, visible_range);
            let _ = self.set_math_selection(slot, visible_range.clone());
            let sanitized = Self::math_input_text(new_text);
            let selected_range_relative = new_selected_range_utf16
                .as_ref()
                .map(|range| Self::utf16_range_to_utf8_in(&sanitized, range))
                .map(|range| Self::math_clamped_range(&sanitized, range));
            let inserted_end = visible_range
                .start
                .saturating_add(sanitized.len())
                .min(text.len().saturating_add(sanitized.len()));
            let changed = self.execute_math_command_live(
                MathEditCommand::InsertText(sanitized.clone()),
                UndoCaptureKind::ImeComposition,
                cx,
            );

            if !changed {
                self.math_marked_range = None;
                return;
            }

            if let Some(relative) = selected_range_relative {
                if let Some((next_slot, _)) = self.math_input_context() {
                    let absolute = visible_range.start.saturating_add(relative.start)
                        ..visible_range.start.saturating_add(relative.end);
                    let _ = self.set_math_selection(next_slot, absolute);
                }
            }
            self.math_marked_range =
                (!sanitized.is_empty()).then_some(visible_range.start..inserted_end);
            return;
        }

        if self.code_language_focus_handle.is_focused(window) {
            let visible_range = range_utf16
                .as_ref()
                .map(|range| self.code_language_range_from_utf16(range))
                .or(self.code_language_marked_range.clone())
                .unwrap_or(self.code_language_selected_range.clone());
            let sanitized_new_text = new_text.replace("\r\n", " ").replace(['\r', '\n'], " ");
            let selected_range_relative = new_selected_range_utf16
                .as_ref()
                .map(|range| Self::utf16_range_to_utf8_in(&sanitized_new_text, range))
                .map(|relative| relative.start..relative.end);

            self.prepare_undo_capture(UndoCaptureKind::ImeComposition, cx);
            self.replace_code_language_text_in_range(
                visible_range,
                &sanitized_new_text,
                selected_range_relative,
                !sanitized_new_text.is_empty(),
                cx,
            );
            return;
        }

        if self.editor_selection_range.is_some() {
            let selected_range_relative = new_selected_range_utf16
                .as_ref()
                .map(|range| Self::utf16_range_to_utf8_in(new_text, range))
                .map(|relative| relative.start..relative.end);
            cx.emit(BlockEvent::RequestReplaceCrossBlockSelection {
                text: new_text.to_string(),
                selected_range_relative,
                mark_inserted_text: !new_text.is_empty(),
                undo_kind: UndoCaptureKind::ImeComposition,
            });
            return;
        }

        self.prepare_undo_capture(UndoCaptureKind::ImeComposition, cx);
        let visible_range = range_utf16
            .as_ref()
            .map(|range| self.range_from_utf16(range))
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());
        let selected_range_relative = new_selected_range_utf16
            .as_ref()
            .map(|range| Self::utf16_range_to_utf8_in(new_text, range))
            .map(|relative| relative.start..relative.end);

        self.replace_text_in_visible_range(
            visible_range,
            new_text,
            selected_range_relative,
            !new_text.is_empty(),
            cx,
        );
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/components/block/ime_platform_compat.rs"]
mod tests;
