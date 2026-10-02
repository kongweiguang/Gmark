// @author kongweiguang

//! Document mode, encoding, and local-change actions.

use super::*;

impl DocumentHost {
    pub(crate) fn is_dirty(&self) -> bool {
        document_dirty_state(&self.document)
    }

    /// “不保存”是终止当前恢复会话，不只是隐藏窗口级 dirty 标记。
    /// Controller 只允许最终 lease 执行 discard，避免一个 pane 清掉共享文档
    /// 的 dirty 基线；成功后 recovery journal 也必须一并清理。
    pub(crate) fn discard_unsaved_changes(&mut self, cx: &mut Context<Self>) {
        let _ = self.discard_unsaved_changes_for_owned_leases(1, cx);
    }

    /// Discard changes when the caller owns every active view lease for this
    /// host's document.  A split window can hold multiple host leases, so the
    /// final-lease-only compatibility method above is insufficient for window
    /// close.  Keep the journal cleanup and event publication identical to the
    /// single-lease path, while returning success to the close coordinator.
    pub(crate) fn discard_unsaved_changes_for_owned_leases(
        &mut self,
        expected_owned_leases: usize,
        cx: &mut Context<Self>,
    ) -> bool {
        let discard_succeeded = match self.document.as_ref() {
            Some(document) => match document
                .handle()
                .discard_current_changes_for_owned_leases(expected_owned_leases)
            {
                Ok(_) => true,
                Err(error) => {
                    self.coordinator.recovery_error = Some(error.to_string().into());
                    false
                }
            },
            None => true,
        };
        if discard_succeeded && let Some(document) = self.document.clone() {
            // Discard changes only mutates the Controller baseline here; the
            // journal removal is queued so a slow or failing filesystem never
            // blocks the close/UI path and the old log remains recoverable.
            self.enqueue_recovery_checkpoint(&document, None, cx);
        }
        cx.emit(DocumentHostEvent::StateChanged);
        cx.notify();
        discard_succeeded
    }

    pub(crate) fn encoding_label(&self) -> String {
        text_encoding_label(&self.probe.encoding)
    }

    pub(crate) fn cursor_position(&self, cx: &App) -> (usize, usize) {
        if let Some(active) = &self.active_edit {
            let block = active.block.read(cx);
            let offset = block.selected_range.end.min(block.display_text().len());
            let column = block.display_text()[..offset]
                .chars()
                .count()
                .saturating_add(1);
            return (active.line.saturating_add(1), column);
        }
        let line = self
            .selected_lines
            .as_ref()
            .map_or(0, |selection| selection.start)
            .saturating_add(1);
        (line, 1)
    }

    pub(super) fn accessibility_caret(&self, cx: &App) -> (u64, usize) {
        if let Some(active) = &self.active_edit {
            let block = active.block.read(cx);
            let offset = block.selected_range.end.min(block.display_text().len());
            let column = unicode_segmentation::UnicodeSegmentation::graphemes(
                &block.display_text()[..offset],
                true,
            )
            .count();
            return (active.line as u64, column);
        }
        (
            self.selected_lines
                .as_ref()
                .map_or(0, |selection| selection.start) as u64,
            0,
        )
    }
    pub(crate) fn has_registered_structure_view(&self) -> bool {
        (self.probe.format == DocumentFormat::Json || self.structured_index.is_some())
            && self
                .selected_projection_view
                .as_ref()
                .and_then(|id| {
                    self.view_registry
                        .available_provider(id, &self.probe.format)
                })
                .is_some()
    }

    pub(crate) fn is_json_document(&self) -> bool {
        self.probe.format == DocumentFormat::Json
    }

    pub(crate) fn is_delimited_document(&self) -> bool {
        matches!(self.probe.format, DocumentFormat::Delimited { .. })
    }

    pub(crate) fn supports_tabular_modes(&self) -> bool {
        self.is_json_document() || self.is_delimited_document()
    }

    pub(crate) fn source_is_utf8(&self) -> bool {
        matches!(self.probe.encoding, TextEncoding::Utf8 { .. })
    }

    pub(crate) fn convert_source_encoding_to_utf8(&mut self, cx: &mut Context<Self>) {
        if self.source_is_utf8() {
            return;
        }
        let Some(document) = self.document.as_ref() else {
            return;
        };
        if let Err(error) = document.set_encoding(TextEncoding::Utf8 { bom: false }) {
            self.error = Some(error.to_string().into());
            cx.notify();
            return;
        }
        self.probe.encoding = TextEncoding::Utf8 { bom: false };
        cx.emit(DocumentHostEvent::StateChanged);
        cx.notify();
    }

    pub(crate) fn set_json_split_ratio(&mut self, ratio: f32, cx: &mut Context<Self>) {
        let ratio = ratio.clamp(0.3, 0.7);
        if (self.json_split_ratio - ratio).abs() < f32::EPSILON {
            return;
        }
        self.json_split_ratio = ratio;
        cx.notify();
    }

    pub(crate) fn show_source_view(&mut self, cx: &mut Context<Self>) {
        self.dismiss_view_context_menus();
        self.mode_notice = None;
        self.set_view_mode(DocumentHostViewMode::Source, cx);
        cx.emit(DocumentHostEvent::StateChanged);
        cx.emit(DocumentHostEvent::ViewModeChanged(DocumentHostMode::Source));
    }

    /// 切入结构视图时同步终止 Source 拖选，JSON 的专用分支也会替换输入坐标空间。
    pub(crate) fn show_structure_view(&mut self, cx: &mut Context<Self>) {
        self.dismiss_view_context_menus();
        self.mode_notice = None;
        self.request_registered_projection(cx);
        if self.probe.format == DocumentFormat::Json {
            self.end_source_pointer_selection();
            self.active_edit = None;
            self.graph_needs_fit |= self.view_mode != DocumentHostViewMode::Structure;
            self.view_mode = DocumentHostViewMode::Structure;
            self.sync_tab_active_view();
            cx.notify();
        } else {
            self.set_view_mode(DocumentHostViewMode::Structure, cx);
        }
        cx.emit(DocumentHostEvent::StateChanged);
        cx.emit(DocumentHostEvent::ViewModeChanged(
            DocumentHostMode::Preview,
        ));
    }

    pub(crate) fn show_live_view(&mut self, cx: &mut Context<Self>) {
        self.dismiss_view_context_menus();
        self.mode_notice = None;
        if !self.is_delimited_document() {
            self.show_structure_view(cx);
            return;
        }
        self.request_registered_projection(cx);
        self.set_view_mode(DocumentHostViewMode::Live, cx);
        cx.emit(DocumentHostEvent::StateChanged);
        cx.emit(DocumentHostEvent::ViewModeChanged(DocumentHostMode::Live));
    }

    /// 切入 Split 前结束 Source 拖选，避免另一侧开始接收事件后旧滚动任务仍运行。
    pub(crate) fn show_split_view(&mut self, cx: &mut Context<Self>) {
        self.dismiss_view_context_menus();
        self.mode_notice = None;
        if self.probe.format == DocumentFormat::Json || self.structured_index.is_some() {
            self.request_registered_projection(cx);
            self.end_source_pointer_selection();
            self.active_edit = None;
            self.graph_needs_fit |= self.view_mode != DocumentHostViewMode::Split;
            self.view_mode = DocumentHostViewMode::Split;
            self.sync_tab_active_view();
            cx.emit(DocumentHostEvent::StateChanged);
            cx.emit(DocumentHostEvent::ViewModeChanged(DocumentHostMode::Split));
            cx.notify();
        } else {
            self.show_source_view(cx);
        }
    }

    /// 视图切换会替换菜单所属的局部坐标空间，旧菜单不能跨模式继续存在。
    fn dismiss_view_context_menus(&mut self) {
        self.source_context_menu = None;
        self.graph_context_menu = None;
        self.structured_context_target = None;
    }

    pub(crate) fn structure_view_active(&self) -> bool {
        matches!(
            self.view_mode,
            DocumentHostViewMode::Live | DocumentHostViewMode::Structure
        )
    }

    pub(crate) fn structured_split_active(&self) -> bool {
        self.view_mode == DocumentHostViewMode::Split
    }

    /// 模式不可用时仍切回稳定 Source 表面，因此先结束旧视图的拖选任务。
    pub(crate) fn show_mode_unavailable(&mut self, mode: &'static str, cx: &mut Context<Self>) {
        self.end_source_pointer_selection();
        self.view_mode = DocumentHostViewMode::Source;
        self.sync_tab_active_view();
        self.mode_notice = Some(
            format!(
                "{mode} needs a resident Markdown projection; Source remains available for this file size"
            )
            .into(),
        );
        cx.emit(DocumentHostEvent::StateChanged);
        cx.notify();
    }

    pub(crate) fn follow_enabled(&self) -> bool {
        self.tail_enabled
    }

    pub(crate) fn line_endings_visible(&self) -> bool {
        self.show_line_endings
    }

    pub(crate) fn toggle_follow(&mut self, cx: &mut Context<Self>) {
        let strings = cx.global::<I18nManager>().strings_arc();
        if document_dirty_state(&self.document) {
            self.coordinator.external_status =
                Some(strings.large_document_text("follow_dirty_error").into());
        } else {
            self.tail_enabled = !self.tail_enabled;
            self.coordinator.external_status = Some(
                if self.tail_enabled {
                    strings.large_document_text("following_appended")
                } else {
                    strings.large_document_text("log_following_paused")
                }
                .into(),
            );
        }
        cx.emit(DocumentHostEvent::StateChanged);
        cx.notify();
    }

    pub(crate) fn toggle_line_endings(&mut self, cx: &mut Context<Self>) {
        self.show_line_endings = !self.show_line_endings;
        cx.emit(DocumentHostEvent::StateChanged);
        cx.notify();
    }
}
