// @author kongweiguang

//! View-mode selection and structured runtime invalidation.

use super::*;

impl DocumentHost {
    pub(super) fn emit_viewport_cancellation_trace(&self) {
        let profile = self.probe.profile();
        let plan = session_plan(&profile, &self.probe, self.probe.strategy, false);
        crate::perf::emit_document_value(
            "document_viewport_cancelled",
            self.metrics.viewport_cancellations,
            &profile.format,
            &plan,
        );
    }

    /// 切换 Host 视图时同步结束 Source 拖选，并清除只属于旧输入目标的编辑焦点。
    pub(super) fn set_view_mode(&mut self, mode: DocumentHostViewMode, cx: &mut Context<Self>) {
        if matches!(
            mode,
            DocumentHostViewMode::Live | DocumentHostViewMode::Structure
        ) && self.structured_index.is_none()
            && !self.is_delimited_document()
        {
            return;
        }
        if self.view_mode != mode {
            self.end_source_pointer_selection();
        }
        self.active_edit = None;
        self.view_mode = mode;
        self.sync_tab_active_view();
        cx.notify();
    }

    pub(super) fn select_markdown_table(&mut self, table: usize, cx: &mut Context<Self>) {
        let changed = match self.structured_index.as_mut() {
            Some(StructuredIndex::MarkdownTables { tables, selected })
                if table < tables.len() && table != *selected =>
            {
                *selected = table;
                true
            }
            _ => false,
        };
        if !changed {
            return;
        }
        // 视口行以表内相对序号为 key；切表必须整体失效，否则会短暂展示上一张表的同序号行。
        self.invalidate_structured_runtime();
        self.structured_filter_column = None;
        cx.notify();
    }

    pub(super) fn set_structure_error(
        &mut self,
        error: gmark_paged_document::PagedDocumentError,
        cx: &App,
    ) {
        self.structure_error_byte = match &error {
            gmark_paged_document::PagedDocumentError::InvalidJson { offset, .. } => Some(*offset),
            gmark_paged_document::PagedDocumentError::InvalidDelimited { offset, .. } => {
                Some(*offset)
            }
            _ => None,
        };
        self.structure_error = Some(
            cx.global::<I18nManager>()
                .strings()
                .large_document_error(&error)
                .into(),
        );
    }

    pub(super) fn clear_structure_error(&mut self) {
        self.structure_error = None;
        self.structure_error_byte = None;
    }

    /// 结构索引、视口行、JSON 子树和筛选结果共享同一份磁盘基线。
    /// 基线变化时必须整体失效，避免后台旧任务把过期行重新发布到新文档。
    pub(super) fn invalidate_structured_runtime(&mut self) {
        self.derived_projection_generation = self.derived_projection_generation.wrapping_add(1);
        if let Some(cancellation) = self.derived_projection_cancellation.take() {
            cancellation.cancel();
        }
        if self.probe.format == DocumentFormat::Json {
            self.derived_projection_stale = self.derived_projection_snapshot.is_some();
        } else {
            self.derived_projection_snapshot = None;
            self.derived_projection_stale = false;
        }
        self.derived_projection_task = Task::ready(());
        self.structured_generation = self.structured_generation.wrapping_add(1);
        if let Some(cancellation) = self.structured_cancellation.take() {
            cancellation.cancel();
        }
        self.structured_column_progress = None;
        self.structured_progress_task = Task::ready(());
        self.structured_pending = None;
        self.structured_rows.clear();
        self.structured_cell_overrides.clear();
        self.structured_cell_source_edits.clear();

        self.structured_filter_generation = self.structured_filter_generation.wrapping_add(1);
        if let Some(cancellation) = self.structured_filter_cancellation.take() {
            cancellation.cancel();
        }
        self.structured_filter_running = false;
        self.structured_filtered_rows.clear();
        self.hidden_structured_columns.clear();
        self.structured_column_window_start = 0;

        self.json_expand_generation = self.json_expand_generation.wrapping_add(1);
        if let Some(cancellation) = self.json_expand_cancellation.take() {
            cancellation.cancel();
        }
        self.json_child_indexes.clear();
        self.json_expanded_nodes.clear();
        self.json_rows.clear();
    }
}
