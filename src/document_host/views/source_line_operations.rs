// @author kongweiguang

//! Transactional line operations for the large-file Source editor.

use super::source_ime::DeferredSourceAction;
use super::*;
use crate::components::{LineOperation, plan_line_operation};

impl DocumentHost {
    /// 复用行操作规划器原子提交正文和方向选区；保存使用旧快照，不锁住新的行操作。
    pub(super) fn apply_source_line_operation(
        &mut self,
        operation: LineOperation,
        cx: &mut Context<Self>,
    ) {
        if self.reloading || self.document.is_none() {
            return;
        }
        let Some(document) = self.document.clone() else {
            return;
        };
        let base_revision = document.revision();
        let selection_before = document.source_selection();
        let selected = selection_before.range();
        let selected_end = if selected.is_empty() {
            selected.end
        } else {
            selected.end.saturating_sub(1)
        };
        let Some(first_line) = document
            .line_for_offset(selected.start)
            .and_then(|line| usize::try_from(line).ok())
        else {
            return;
        };
        let Some(last_line) = document
            .line_for_offset(selected_end)
            .and_then(|line| usize::try_from(line).ok())
        else {
            return;
        };
        let line_count = usize::try_from(document.line_count()).unwrap_or(usize::MAX);
        let context_first = if operation == LineOperation::MoveUp {
            first_line.saturating_sub(1)
        } else {
            first_line
        };
        let context_last = if operation == LineOperation::MoveDown {
            last_line
                .saturating_add(1)
                .min(line_count.saturating_sub(1))
        } else {
            last_line
        };
        let Some(context_range) = document
            .line_range(context_first as u64)
            .zip(document.line_range(context_last as u64))
            .map(|(first, last)| first.start..last.end)
        else {
            return;
        };
        let bytes = match document.read_range(context_range.clone()) {
            Ok(bytes) => bytes,
            Err(error) => {
                self.error = Some(localized_document_error(&error, cx));
                cx.notify();
                return;
            }
        };
        let text = match String::from_utf8(bytes) {
            Ok(text) => text,
            Err(error) => {
                self.error = Some(error.to_string().into());
                cx.notify();
                return;
            }
        };
        let local_selection = match (
            usize::try_from(selected.start.saturating_sub(context_range.start)),
            usize::try_from(selected.end.saturating_sub(context_range.start)),
        ) {
            (Ok(start), Ok(end)) => start..end,
            _ => {
                self.error = Some("选区超出当前平台可处理的范围。".into());
                cx.notify();
                return;
            }
        };
        if document.revision() != base_revision {
            self.error = Some("源码已在其他视图修改，行操作已取消。请重试。".into());
            self.reject_stale_source_edit(cx);
            cx.notify();
            return;
        }
        let reversed = selection_before.anchor.byte_offset > selection_before.head.byte_offset;
        let Some(edit) = plan_line_operation(&text, local_selection, reversed, operation) else {
            return;
        };
        let Some(edit_start_offset) = u64::try_from(edit.range.start).ok() else {
            return;
        };
        let Some(edit_end_offset) = u64::try_from(edit.range.end).ok() else {
            return;
        };
        let Some(edit_start) = context_range.start.checked_add(edit_start_offset) else {
            return;
        };
        let Some(edit_end) = context_range.start.checked_add(edit_end_offset) else {
            return;
        };
        let edit_range = edit_start..edit_end;
        let Some(selection_start) = u64::try_from(edit.selection.start)
            .ok()
            .and_then(|offset| context_range.start.checked_add(offset))
        else {
            return;
        };
        let Some(selection_end) = u64::try_from(edit.selection.end)
            .ok()
            .and_then(|offset| context_range.start.checked_add(offset))
        else {
            return;
        };
        let selection_after =
            SourceSelection::from_range(selection_start..selection_end, edit.reversed);
        let transaction_id = match document.next_transaction_id() {
            Ok(transaction_id) => transaction_id,
            Err(error) => {
                self.error = Some(error.to_string().into());
                cx.notify();
                return;
            }
        };
        let transaction = Transaction::new(
            DocumentRevision(base_revision),
            vec![SourceEdit::new(
                edit_range.clone(),
                edit.replacement.clone(),
            )],
        );
        if let Err(error) = document.apply_transaction(
            transaction_id,
            transaction,
            selection_before,
            selection_after,
        ) {
            if document.revision() != base_revision {
                self.error = Some("源码已在其他视图修改，行操作已取消。请重试。".into());
                self.reject_stale_source_edit(cx);
            } else {
                self.error = Some(error.to_string().into());
            }
            cx.notify();
            return;
        }
        self.install_source_replacement_with_selection(
            edit_range,
            &edit.replacement,
            Some(selection_after),
            false,
            false,
            false,
            cx,
        );
    }

    /// Routes line actions only when Source text owns focus, then waits for any native composition to finish.
    pub(super) fn on_source_line_operation(
        &mut self,
        operation: LineOperation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.view_mode != DocumentHostViewMode::Source
            || !self.source_text_surface_has_focus(window, cx)
        {
            return;
        }
        self.apply_source_line_operation_after_ime(operation, window, cx);
    }

    /// Applies focused Source row commands, including the Source half of Split mode, without using a Preview selection.
    pub(super) fn on_source_row_line_operation(
        &mut self,
        operation: LineOperation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !matches!(
            self.view_mode,
            DocumentHostViewMode::Source | DocumentHostViewMode::Split
        ) || !self.source_text_surface_has_focus(window, cx)
        {
            return;
        }
        self.apply_source_line_operation_after_ime(operation, window, cx);
    }

    /// Resolves Source command ownership while excluding Host tools and the independently focused structured grid.
    pub(super) fn source_text_surface_has_focus(&self, window: &Window, cx: &App) -> bool {
        let independent_field_focused = self.structured_focus_handle.is_focused(window)
            || [
                &self.search_input,
                &self.navigation_input,
                &self.structured_filter_input,
                &self.structured_cell_input,
                &self.graph_edit_input,
            ]
            .iter()
            .any(|block| block.read(cx).focus_handle.is_focused(window));
        !independent_field_focused
            && (self.focus_handle.is_focused(window)
                || self
                    .active_edit
                    .as_ref()
                    .is_some_and(|active| active.block.read(cx).focus_handle.is_focused(window)))
    }

    /// Defers the line edit behind native composition before applying its transaction.
    fn apply_source_line_operation_after_ime(
        &mut self,
        operation: LineOperation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.defer_source_action_for_ime(DeferredSourceAction::Line(operation), window, cx) {
            return;
        }
        self.apply_source_line_operation(operation, cx);
    }

    /// Connects duplicate-line to the single-transaction Source planner.
    pub(super) fn on_duplicate_line(
        &mut self,
        _: &DuplicateLine,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.on_source_line_operation(LineOperation::Duplicate, window, cx);
    }

    /// Connects delete-line to the single-transaction Source planner.
    pub(super) fn on_delete_line(
        &mut self,
        _: &DeleteLine,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.on_source_line_operation(LineOperation::Delete, window, cx);
    }

    /// Connects moving a line up to the single-transaction Source planner.
    pub(super) fn on_move_line_up(
        &mut self,
        _: &MoveLineUp,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.on_source_line_operation(LineOperation::MoveUp, window, cx);
    }

    /// Connects moving a line down to the single-transaction Source planner.
    pub(super) fn on_move_line_down(
        &mut self,
        _: &MoveLineDown,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.on_source_line_operation(LineOperation::MoveDown, window, cx);
    }

    /// Indents the selected Source lines as one undoable transaction.
    pub(super) fn on_indent_source(
        &mut self,
        _: &IndentBlock,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.on_source_line_operation(LineOperation::Indent, window, cx);
    }

    /// Outdents the selected Source lines as one undoable transaction.
    pub(super) fn on_outdent_source(
        &mut self,
        _: &OutdentBlock,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.on_source_line_operation(LineOperation::Outdent, window, cx);
    }
}
