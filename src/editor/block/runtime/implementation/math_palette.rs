// @author kongweiguang

use super::*;

impl Block {
    pub(crate) fn begin_math_palette_drag(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.math_structure_focus_handle.focus(window);
        self.math_palette_drag_anchor = Some(event.position);
        cx.stop_propagation();
    }

    pub(crate) fn update_math_palette_drag(
        &mut self,
        event: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(previous) = self.math_palette_drag_anchor.replace(event.position) else {
            return;
        };
        self.math_palette_offset.x += event.position.x - previous.x;
        self.math_palette_offset.y += event.position.y - previous.y;
        let viewport = window.viewport_size();
        let horizontal_limit = (viewport.width - px(200.0)).max(px(0.0));
        let vertical_limit = (viewport.height - px(64.0)).max(px(0.0));
        self.math_palette_offset.x = self
            .math_palette_offset
            .x
            .clamp(-horizontal_limit, horizontal_limit);
        self.math_palette_offset.y = self
            .math_palette_offset
            .y
            .clamp(-vertical_limit, vertical_limit);
        cx.notify();
    }

    pub(crate) fn end_math_palette_drag(
        &mut self,
        _event: &MouseUpEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.math_palette_drag_anchor = None;
        cx.stop_propagation();
    }

    /// Leaves history shortcuts to the bound Editor action, avoiding a second host dispatch from
    /// the raw key event and keeping configurable shortcut ownership at the document surface.
    pub(crate) fn on_math_structure_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = event.keystroke.key.as_str();
        let modifiers = event.keystroke.modifiers;
        if key == "escape" {
            self.finish_math_edit(cx);
            self.focus_handle.focus(window);
            cx.stop_propagation();
            return;
        }
        if key == "enter" && (modifiers.platform || modifiers.control) && !modifiers.alt {
            self.finish_math_edit(cx);
            self.focus_handle.focus(window);
            self.on_exit_code_block(&ExitCodeBlock, window, cx);
            cx.stop_propagation();
            return;
        }
        let navigated = if let Some(session) = self.math_edit_session.as_mut() {
            match key {
                "left" => session
                    .move_cursor_horizontal_with_selection(-1, modifiers.shift)
                    .unwrap_or(false),
                "right" => session
                    .move_cursor_horizontal_with_selection(1, modifiers.shift)
                    .unwrap_or(false),
                "up" => session
                    .move_cursor_vertical_with_selection(-1, modifiers.shift)
                    .unwrap_or(false),
                "down" => session
                    .move_cursor_vertical_with_selection(1, modifiers.shift)
                    .unwrap_or(false),
                "tab" => session
                    .move_cursor_environment_slot_with_selection(
                        if modifiers.shift { -1 } else { 1 },
                        modifiers.shift,
                    )
                    .unwrap_or(false),
                _ => false,
            }
        } else {
            false
        };
        let edited = match key {
            "backspace" => self.execute_math_command_live(
                gmark_math_edit::MathEditCommand::DeleteBackward,
                UndoCaptureKind::CoalescibleText,
                cx,
            ),
            "delete" => self.execute_math_command_live(
                gmark_math_edit::MathEditCommand::DeleteForward,
                UndoCaptureKind::CoalescibleText,
                cx,
            ),
            "v" if modifiers.platform || modifiers.control => cx
                .read_from_clipboard()
                .and_then(|item| item.text())
                .is_some_and(|text| {
                    self.execute_math_command_live(
                        gmark_math_edit::MathEditCommand::InsertText(text),
                        UndoCaptureKind::NonCoalescible,
                        cx,
                    )
                }),
            "/" if !modifiers.control && !modifiers.platform && !modifiers.alt => self
                .execute_math_palette_command(gmark_math_edit::MathEditCommand::InsertFraction, cx),
            "^" if !modifiers.control && !modifiers.platform && !modifiers.alt => self
                .execute_math_palette_command(
                    gmark_math_edit::MathEditCommand::InsertSuperscript,
                    cx,
                ),
            "_" if !modifiers.control && !modifiers.platform && !modifiers.alt => self
                .execute_math_palette_command(
                    gmark_math_edit::MathEditCommand::InsertSubscript,
                    cx,
                ),
            "(" if !modifiers.control && !modifiers.platform && !modifiers.alt => self
                .execute_math_palette_command(
                    gmark_math_edit::MathEditCommand::InsertDelimiter(
                        gmark_math_edit::MathDelimiterPair::Parentheses,
                    ),
                    cx,
                ),
            "[" if !modifiers.control && !modifiers.platform && !modifiers.alt => self
                .execute_math_palette_command(
                    gmark_math_edit::MathEditCommand::InsertDelimiter(
                        gmark_math_edit::MathDelimiterPair::Brackets,
                    ),
                    cx,
                ),
            "{" if !modifiers.control && !modifiers.platform && !modifiers.alt => self
                .execute_math_palette_command(
                    gmark_math_edit::MathEditCommand::InsertDelimiter(
                        gmark_math_edit::MathDelimiterPair::Braces,
                    ),
                    cx,
                ),
            "|" if !modifiers.control && !modifiers.platform && !modifiers.alt => self
                .execute_math_palette_command(
                    gmark_math_edit::MathEditCommand::InsertDelimiter(
                        gmark_math_edit::MathDelimiterPair::AbsoluteValue,
                    ),
                    cx,
                ),
            value
                if value.chars().count() == 1
                    && !modifiers.control
                    && !modifiers.platform
                    && !modifiers.alt
                    && !modifiers.function =>
            {
                self.execute_math_command_live(
                    gmark_math_edit::MathEditCommand::InsertText(value.to_owned()),
                    UndoCaptureKind::CoalescibleText,
                    cx,
                )
            }
            _ => false,
        };
        if edited || navigated || matches!(key, "left" | "right" | "up" | "down" | "tab") {
            cx.stop_propagation();
            cx.notify();
        }
    }

    pub(crate) fn on_math_structure_delete_back(
        &mut self,
        _action: &DeleteBack,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.math_structure_focus_handle.is_focused(window) {
            return;
        }
        self.execute_math_command_live(
            gmark_math_edit::MathEditCommand::DeleteBackward,
            UndoCaptureKind::CoalescibleText,
            cx,
        );
        cx.stop_propagation();
        cx.notify();
    }

    pub(crate) fn on_math_structure_delete(
        &mut self,
        _action: &Delete,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.math_structure_focus_handle.is_focused(window) {
            return;
        }
        self.execute_math_command_live(
            gmark_math_edit::MathEditCommand::DeleteForward,
            UndoCaptureKind::CoalescibleText,
            cx,
        );
        cx.stop_propagation();
        cx.notify();
    }
}
