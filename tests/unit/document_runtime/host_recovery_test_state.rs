// @author kongweiguang

//! DocumentHost test-only state inspection.

use super::*;

impl DocumentRecoveryJournal {
    pub(crate) fn path(&self) -> &Path {
        match self {
            Self::Resident(journal) => journal.path(),
            Self::Paged(journal) => journal.path(),
        }
    }
}

impl DocumentHost {
    /// 生命周期回归直接检查共享保存队列；视图暂停不能用丢失本地状态冒充共享 IO 已结束。
    #[cfg(test)]
    pub(crate) fn document_handle_for_test(&self) -> Option<DocumentHandle> {
        self.document.as_ref().map(SharedDocument::handle)
    }

    #[cfg(test)]
    pub(crate) fn recovered_text_for_test(&self) -> Option<Vec<u8>> {
        let document = self.document.as_ref()?;
        document.serialized_bytes().ok()
    }

    #[cfg(test)]
    pub(crate) fn has_recovery_journal_for_test(&self) -> bool {
        self.coordinator.recovery_journal.is_some() || self.coordinator.recovery_worker.is_some()
    }

    #[cfg(test)]
    pub(crate) fn is_closed_suspended_for_test(&self) -> bool {
        self.closed_suspended
    }

    #[cfg(test)]
    pub(crate) fn is_saving_for_test(&self) -> bool {
        self.saving
    }

    #[cfg(test)]
    pub(crate) fn has_structure_view(&self) -> bool {
        self.structured_index.is_some()
    }

    #[cfg(test)]
    pub(crate) fn installed_projection_for_test(&self) -> Option<(u64, u64, u64)> {
        self.derived_projection_snapshot.as_ref().map(|snapshot| {
            (
                snapshot.document_epoch(),
                snapshot.revision(),
                snapshot.generation(),
            )
        })
    }

    #[cfg(test)]
    pub(crate) fn json_graph_state_for_test(
        &self,
    ) -> Option<(usize, usize, bool, bool, Option<u64>)> {
        let graph = self
            .derived_projection_snapshot
            .as_ref()?
            .as_any()
            .downcast_ref::<JsonGraphSnapshot>()?
            .projection();
        Some((
            graph.nodes.len(),
            graph.edges.len(),
            graph.truncated,
            self.derived_projection_stale,
            self.derived_projection_error_offset,
        ))
    }

    #[cfg(test)]
    pub(crate) fn graph_selected_item_for_test(&self) -> Option<String> {
        self.graph_selected_item
            .as_ref()
            .map(|item| item.as_str().to_owned())
    }

    #[cfg(test)]
    pub(crate) fn json_graph_search_state_for_test(&self) -> (usize, usize) {
        (self.graph_search_matches.len(), self.graph_search_selected)
    }

    #[cfg(test)]
    pub(crate) fn json_graph_root_identity_for_test(&self) -> Option<(String, String)> {
        let graph = self
            .derived_projection_snapshot
            .as_ref()?
            .as_any()
            .downcast_ref::<JsonGraphSnapshot>()?
            .projection();
        let root = graph.nodes.first()?;
        Some((root.json_path.to_string(), root.label.to_string()))
    }

    #[cfg(test)]
    pub(crate) fn json_graph_error_for_test(&self) -> Option<(String, Option<u64>)> {
        self.derived_projection_error
            .as_ref()
            .map(|error| (error.to_string(), self.derived_projection_error_offset))
    }

    #[cfg(test)]
    pub(crate) fn json_search_input_for_test(&self) -> Entity<Block> {
        self.structured_filter_input.clone()
    }

    #[cfg(test)]
    pub(crate) fn json_graph_edit_input_for_test(&self) -> Entity<Block> {
        self.graph_edit_input.clone()
    }

    #[cfg(test)]
    pub(crate) fn json_graph_edit_open_for_test(&self) -> bool {
        self.graph_edit_target.is_some()
    }

    #[cfg(test)]
    pub(crate) fn begin_json_graph_node_edit_for_test(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(snapshot) = self
            .derived_projection_snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.as_any().downcast_ref::<JsonGraphSnapshot>())
        else {
            return;
        };
        let Some(node) = snapshot
            .projection()
            .nodes
            .iter()
            .find(|node| node.id.as_str() == id)
        else {
            return;
        };
        let target = JsonGraphEditTarget {
            item_id: node.id.clone(),
            range: node.source.range.clone(),
            document_epoch: snapshot.document_epoch(),
            base_revision: snapshot.revision(),
            label: node.label.clone(),
            kind: node.kind,
        };
        self.begin_json_graph_edit(target, window, cx);
    }

    #[cfg(test)]
    pub(crate) fn begin_json_graph_item_edit_for_test(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = JsonGraphItemId::new(id);
        if let Some(target) = self.resolve_json_graph_edit_target(&id) {
            self.begin_json_graph_edit(target, window, cx);
        }
    }

    #[cfg(test)]
    pub(crate) fn source_text_for_test(&self) -> String {
        let Some(document) = self.document.as_ref() else {
            return String::new();
        };
        document
            .serialized_bytes()
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            .unwrap_or_default()
    }

    #[cfg(test)]
    pub(crate) fn source_view_for_test(&self) -> bool {
        self.view_mode == DocumentHostViewMode::Source
    }

    #[cfg(test)]
    pub(crate) fn delimited_live_for_test(&self) -> bool {
        self.is_delimited_document() && self.view_mode == DocumentHostViewMode::Live
    }

    #[cfg(test)]
    pub(crate) fn structured_cell_input_for_test(&self) -> Entity<Block> {
        self.structured_cell_input.clone()
    }

    #[cfg(test)]
    pub(crate) fn structured_selected_cell_for_test(&self) -> Option<(Option<u64>, usize)> {
        self.structured_selected_cell
            .map(|cell| (cell.record, cell.column))
    }

    #[cfg(test)]
    pub(crate) fn structured_loaded_row_count_for_test(&self) -> usize {
        self.structured_rows.len()
    }

    #[cfg(test)]
    pub(crate) fn insert_delimited_column_for_test(
        &mut self,
        before: usize,
        header: &str,
        cx: &mut Context<Self>,
    ) {
        self.transform_delimited_column(
            DelimitedEdit::InsertColumn {
                before,
                header: header.to_owned(),
            },
            cx,
        );
    }

    #[cfg(test)]
    pub(crate) fn source_cache_len_for_test(&self) -> usize {
        self.source_rows.len()
    }

    #[cfg(test)]
    pub(crate) fn source_list_window_for_test(&self) -> (usize, usize, usize) {
        (
            self.source_list_origin,
            self.source_list_len(),
            self.line_count(),
        )
    }

    #[cfg(test)]
    pub(crate) fn source_row_is_current_for_test(&self, line: usize) -> bool {
        self.source_rows.contains_key(&line)
            && self.source_row_epochs.get(&line) == Some(&self.source_cache_epoch)
    }

    #[cfg(test)]
    pub(crate) fn source_row_height_for_test(&self) -> f32 {
        self.source_row_height
    }

    #[cfg(test)]
    pub(crate) fn error_for_test(&self) -> Option<String> {
        self.error.as_ref().map(ToString::to_string)
    }

    #[cfg(test)]
    pub(crate) fn scroll_top_line_for_test(&self) -> usize {
        let handle = self.scroll_handle.0.borrow().base_handle.clone();
        self.source_list_origin.saturating_add(
            (-f32::from(handle.offset().y) / self.source_row_height.max(1.0))
                .max(0.0)
                .floor() as usize,
        )
    }

    #[cfg(test)]
    pub(crate) fn structured_scroll_top_row_for_test(&self) -> usize {
        let handle = self.structured_scroll_handle.0.borrow().base_handle.clone();
        (-f32::from(handle.offset().y) / 26.0).max(0.0).floor() as usize
    }

    #[cfg(test)]
    pub(crate) fn document_view_ids_for_test(&self) -> Option<(String, Option<String>)> {
        self.document.as_ref()?;
        Some((
            self.tab_view_state
                .active_view
                .as_ref()?
                .as_str()
                .to_owned(),
            self.tab_view_state
                .active_view
                .as_ref()
                .map(|view| view.as_str().to_owned()),
        ))
    }

    #[cfg(test)]
    pub(crate) fn scroll_to_line_for_test(&self, line: usize) {
        let local = line.saturating_sub(self.source_list_origin);
        self.scroll_handle
            .scroll_to_item(local, ScrollStrategy::Top);
    }

    #[cfg(test)]
    pub(crate) fn scroll_page_for_test(&mut self, toward_end: bool, cx: &mut Context<Self>) {
        self.scroll_page(toward_end, cx);
    }

    #[cfg(test)]
    pub(crate) fn jump_bottom_for_test(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.on_jump_to_bottom(&JumpToBottom, window, cx);
    }

    #[cfg(test)]
    pub(crate) fn start_drag_autoscroll_for_test(&mut self, direction: i8, cx: &mut Context<Self>) {
        self.source_drag_anchor = self
            .document
            .as_ref()
            .map(SharedDocument::source_selection)
            .map(|selection| selection.anchor);
        self.start_source_drag_autoscroll(direction, cx);
    }

    #[cfg(test)]
    pub(crate) fn drag_autoscroll_tick_for_test(&mut self, cx: &mut Context<Self>) -> bool {
        self.source_drag_autoscroll_tick(cx)
    }

    /// 测试激活先把目标行带入视口，再解除 Host 桥建立原生焦点，保持真实命中与输入归属。
    #[cfg(test)]
    pub(crate) fn begin_line_edit_for_test(
        &mut self,
        line: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.reloading {
            return;
        }
        let Some(document) = &self.document else {
            return;
        };
        let Ok(Some(windowed)) =
            read_bounded_line_window(document, line as u64, self.source_window_start)
        else {
            return;
        };
        // Tests reuse the same bounded row entity as pointer activation. The first activation can
        // run before the initial row snapshot is painted; later cycles reuse the cached entity.
        let block = if let Some(block) = self
            .source_row_blocks
            .get(&line)
            .filter(|block| block.read(cx).display_text() == windowed.text.as_ref())
            .cloned()
        {
            block
        } else {
            let text = windowed.text.to_string();
            let host = cx.entity().downgrade();
            let block = cx.new(move |cx| {
                let mut block = Block::with_record(
                    cx,
                    BlockRecord::with_plain_text(BlockKind::Paragraph, text),
                );
                block.set_compact_source_host();
                block.set_host_action_handler(move |action, window, cx| {
                    let host = host.clone();
                    window.defer(cx, move |window, cx| {
                        let _ = host.update(cx, |view, cx| {
                            view.on_line_edit_host_action(action, window, cx)
                        });
                    });
                });
                block
            });
            cx.subscribe(&block, Self::on_line_edit_event).detach();
            self.source_row_blocks.insert(line, block.clone());
            block
        };
        let BoundedLineWindow {
            replace_range,
            ending,
            leading_truncated,
            trailing_truncated,
            ..
        } = windowed;
        block.update(cx, |block, cx| {
            block.set_source_host_input_focus_handle(None, cx);
            block.selected_range = block.display_text().len()..block.display_text().len();
            block.focus_handle.focus(window);
        });
        self.active_edit = Some(SourceLineEdit {
            line,
            range: replace_range,
            base_revision: self.document.as_ref().map_or(0, SharedDocument::revision),
            ending,
            leading_truncated,
            trailing_truncated,
            block,
        });
        self.scroll_source_line(line, ScrollStrategy::Center);
        cx.emit(DocumentHostEvent::StateChanged);
        cx.notify();
    }

    #[cfg(test)]
    pub(crate) fn active_edit_for_test(&self) -> Option<(usize, Entity<Block>)> {
        self.active_edit
            .as_ref()
            .map(|edit| (edit.line, edit.block.clone()))
    }

    #[cfg(test)]
    pub(crate) fn select_lines_for_test(&mut self, lines: Range<usize>) {
        self.select_source_lines(lines, false);
        self.active_edit = None;
    }

    #[cfg(test)]
    pub(crate) fn select_source_range_for_test(&mut self, range: Range<u64>, reversed: bool) {
        let Some(document) = self.document.as_mut() else {
            return;
        };
        let _ = document.set_selection(range.clone(), reversed);
        let start_line = document
            .line_for_offset(range.start)
            .and_then(|line| usize::try_from(line).ok())
            .unwrap_or_default();
        let end_line = document
            .line_for_offset(range.end.saturating_sub(1))
            .and_then(|line| usize::try_from(line).ok())
            .unwrap_or(start_line);
        self.selection_anchor = Some(if reversed { end_line } else { start_line });
        self.selected_lines = Some(start_line..end_line.saturating_add(1));
        self.active_edit = None;
    }

    #[cfg(test)]
    pub(crate) fn source_selection_for_test(&self) -> Option<SourceSelection> {
        self.document.as_ref().map(SharedDocument::source_selection)
    }

    #[cfg(test)]
    pub(crate) fn workspace_source_state_for_test(&self) -> (SourceSelection, f32) {
        let state = &self.tab_view_state;
        (state.source.selection, state.source.line_offset_y)
    }
}
