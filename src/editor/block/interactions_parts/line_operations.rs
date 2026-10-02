// @author kongweiguang

use super::*;

impl Block {
    /// Source 交给所属输入目标；代码与表格单元格共用文字行事务，避免单元格快捷键改变表格结构。
    pub(crate) fn apply_raw_line_operation(
        &mut self,
        operation: crate::components::block::LineOperation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.compact_source_host() {
            if self.has_host_action_handler() {
                self.dispatch_host_action(BlockHostAction::LineOperation(operation), window, cx);
            } else {
                cx.stop_propagation();
            }
            return true;
        }

        if self.code_language_focus_handle.is_focused(window) {
            cx.stop_propagation();
            return true;
        }
        if self.is_read_only() {
            return self.uses_raw_text_editing()
                || self.kind().is_code_block()
                || self.is_table_cell();
        }
        if !self.uses_raw_text_editing() && !self.kind().is_code_block() && !self.is_table_cell() {
            return false;
        }
        if self.has_ime_composition() {
            return false;
        }

        let text = self.display_text().to_owned();
        let Some(mut edit) = crate::components::block::plan_line_operation(
            &text,
            self.selected_range.clone(),
            self.selection_reversed,
            operation,
        ) else {
            cx.stop_propagation();
            return true;
        };
        if self.is_table_cell() {
            // GFM 单元格以一个逻辑行保存；衔接文字时保持即时显示与保存、撤销结果一致。
            edit.replacement = edit.replacement.replace('\n', " ");
        }
        if text.get(edit.range.clone()) == Some(edit.replacement.as_str())
            && edit.selection == self.selected_range
        {
            cx.stop_propagation();
            return true;
        }
        let Some(selection_start) = edit.selection.start.checked_sub(edit.range.start) else {
            cx.stop_propagation();
            return true;
        };
        let Some(selection_end) = edit.selection.end.checked_sub(edit.range.start) else {
            cx.stop_propagation();
            return true;
        };
        if selection_end > edit.replacement.len()
            || !edit.replacement.is_char_boundary(selection_start)
            || !edit.replacement.is_char_boundary(selection_end)
        {
            cx.stop_propagation();
            return true;
        }

        let selected_relative = selection_start..selection_end;
        self.prepare_undo_capture(UndoCaptureKind::NonCoalescible, cx);
        self.replace_text_in_visible_range_with_direction(
            edit.range,
            &edit.replacement,
            Some(selected_relative),
            false,
            Some(edit.reversed),
            cx,
        );
        cx.stop_propagation();
        true
    }
}
