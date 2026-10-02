// @author kongweiguang

//! Search, navigation, and structured filter input handling.

use super::json_graph::json_graph_node_matches_query;
use super::*;

impl DocumentHost {
    /// Runs search only after text is committed, keeping IME candidate refreshes off the document viewport.
    pub(super) fn on_search_input_event(
        &mut self,
        block: Entity<Block>,
        event: &BlockEvent,
        cx: &mut Context<Self>,
    ) {
        if block == self.search_input
            && matches!(event, BlockEvent::Changed)
            && !block.read(cx).has_ime_composition()
        {
            self.schedule_search(cx);
        }
    }

    /// Advances search only after IME completion so candidate text cannot move the document viewport.
    pub(super) fn on_search_host_action(
        &mut self,
        action: BlockHostAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(&action, BlockHostAction::Submit(_)) {
            if self.defer_source_action_for_ime(
                super::source_ime::DeferredSourceAction::HostInputSubmit {
                    input: self.search_input.clone(),
                },
                window,
                cx,
            ) {
                return;
            }
            self.navigate_search(1, cx);
        } else {
            self.on_line_edit_host_action(action, window, cx);
        }
    }

    /// Ignores transient composition text because it may otherwise switch modes before the user confirms it.
    pub(super) fn on_navigation_input_event(
        &mut self,
        block: Entity<Block>,
        event: &BlockEvent,
        cx: &mut Context<Self>,
    ) {
        if block != self.navigation_input
            || !matches!(event, BlockEvent::Changed)
            || block.read(cx).has_ime_composition()
        {
            return;
        }
        let input = block.read(cx).display_text().trim().replace(['_', ','], "");
        let Ok(value) = input.parse::<u64>() else {
            return;
        };
        let line = if let Some(document) = &self.document {
            if self.navigation_is_byte {
                document.line_for_offset(value.min(document.len()))
            } else {
                Some(
                    value
                        .saturating_sub(1)
                        .min(document.line_count().saturating_sub(1)),
                )
            }
        } else if self.navigation_is_byte {
            Some(
                ((value.min(self.probe.len) as u128 * self.probe.estimated_lines.max(1) as u128)
                    / self.probe.len.max(1) as u128) as u64,
            )
        } else {
            Some(
                value
                    .saturating_sub(1)
                    .min(self.probe.estimated_lines.saturating_sub(1)),
            )
        };
        let Some(line) = line.and_then(|line| usize::try_from(line).ok()) else {
            return;
        };
        if self.navigation_is_byte {
            if let Some(document) = &self.document {
                self.anchor_source_window_for_byte(line as u64, value.min(document.len()));
            } else {
                self.source_window_start = 0;
                self.invalidate_source_rows();
            }
        } else {
            if self.source_window_start != 0 {
                self.source_window_start = 0;
                self.invalidate_source_rows();
            }
        }
        self.view_mode = DocumentHostViewMode::Source;
        self.sync_tab_active_view();
        self.select_source_lines(line..line.saturating_add(1), false);
        self.scroll_source_line(line, ScrollStrategy::Center);
        cx.emit(DocumentHostEvent::StateChanged);
        cx.notify();
    }

    /// Submits navigation after the field's IME terminal result has reached the Host.
    pub(super) fn on_navigation_host_action(
        &mut self,
        action: BlockHostAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(&action, BlockHostAction::Submit(_)) {
            if self.defer_source_action_for_ime(
                super::source_ime::DeferredSourceAction::HostInputSubmit {
                    input: self.navigation_input.clone(),
                },
                window,
                cx,
            ) {
                return;
            }
            self.navigation_visible = false;
            self.active_edit = None;
            self.focus_handle.focus(window);
            cx.notify();
        } else {
            self.on_line_edit_host_action(action, window, cx);
        }
    }

    /// Rebuilds structured filters only from committed text, so preedit updates do not restart projection work.
    pub(super) fn on_structured_filter_input_event(
        &mut self,
        block: Entity<Block>,
        event: &BlockEvent,
        cx: &mut Context<Self>,
    ) {
        if block == self.structured_filter_input
            && matches!(event, BlockEvent::Changed)
            && !block.read(cx).has_ime_composition()
        {
            if self.probe.format == DocumentFormat::Json {
                let query = block.read(cx).display_text().trim().to_lowercase();
                if let Some(id) = self.selected_projection_view.clone() {
                    document_view_state_mut(&mut self.document, &mut self.tab_view_state)
                        .derived
                        .entry(id)
                        .or_default()
                        .filter = Arc::from(query.clone());
                }
                let matches = self
                    .derived_projection_snapshot
                    .as_ref()
                    .and_then(|snapshot| snapshot.as_any().downcast_ref::<JsonGraphSnapshot>())
                    .map(|snapshot| {
                        snapshot
                            .projection()
                            .nodes
                            .iter()
                            .filter(|node| {
                                !query.is_empty() && json_graph_node_matches_query(node, &query)
                            })
                            .map(|node| node.id.clone())
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                let state = document_view_state_mut(&mut self.document, &mut self.tab_view_state)
                    .derived
                    .entry(DocumentViewId::json_graph())
                    .or_default();
                if query.is_empty() {
                    if let Some(collapsed) = self.graph_search_collapsed_before.take() {
                        state.collapsed_items = collapsed;
                    }
                } else if self.graph_search_collapsed_before.is_none() {
                    self.graph_search_collapsed_before = Some(state.collapsed_items.clone());
                }
                self.graph_search_matches = matches;
                self.graph_search_selected = 0;
                let match_id = self.graph_search_matches.first().cloned();
                self.graph_selected_item = match_id.clone();
                self.graph_pending_center = match_id;
                if let Some(selected) = self.graph_selected_item.clone() {
                    self.reveal_graph_item(&selected);
                }
                cx.notify();
                return;
            }
            self.schedule_structured_filter(cx);
        }
    }

    pub(super) fn schedule_structured_filter(&mut self, cx: &mut Context<Self>) {
        if let Some(cancellation) = self.structured_filter_cancellation.take() {
            cancellation.cancel();
        }
        self.structured_filter_generation = self.structured_filter_generation.wrapping_add(1);
        let generation = self.structured_filter_generation;
        let task_stamp = DocumentTaskStamp::capture(self, generation);
        let query = self
            .structured_filter_input
            .read(cx)
            .display_text()
            .trim()
            .to_owned();
        if let Some(id) = self.selected_projection_view.clone() {
            document_view_state_mut(&mut self.document, &mut self.tab_view_state)
                .derived
                .entry(id)
                .or_default()
                .filter = Arc::from(query.clone());
        }
        if query.is_empty() {
            self.structured_filtered_rows.clear();
            self.structured_filter_running = false;
            self.structured_rows.clear();
            self.structured_pending = None;
            cx.notify();
            return;
        }
        let Some(StructuredIndex::Delimited(index)) = self.structured_index.clone() else {
            self.structure_error = Some(
                cx.global::<I18nManager>()
                    .strings()
                    .large_document_text("column_filter_csv_only")
                    .into(),
            );
            self.structure_error_byte = None;
            return;
        };
        let cancellation = SearchCancellation::default();
        self.structured_filter_cancellation = Some(cancellation.clone());
        self.structured_filter_running = true;
        let options = DelimitedFilterOptions {
            column: self.structured_filter_column,
            case_sensitive: self.search_options.case_sensitive,
            result_limit: 10_000,
        };
        self.structured_filter_task = cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    index.filter_record_indices(&query, options, &cancellation)
                })
                .await;
            let _ = this.update(cx, |view, cx| {
                if !task_stamp.accepts_strict(view, view.structured_filter_generation) {
                    return;
                }
                view.structured_filter_running = false;
                view.structured_filter_cancellation = None;
                view.structured_rows.clear();
                view.structured_pending = None;
                match result {
                    Ok(rows) => {
                        view.structured_filtered_rows = rows;
                        view.clear_structure_error();
                    }
                    Err(gmark_paged_document::PagedDocumentError::Cancelled) => {}
                    Err(error) => view.set_structure_error(error, cx),
                }
                cx.notify();
            });
        });
        cx.notify();
    }

    pub(super) fn schedule_search(&mut self, cx: &mut Context<Self>) {
        if let Some(cancellation) = self.coordinator.search_cancellation.take() {
            cancellation.cancel();
        }
        self.coordinator.search_generation = self.coordinator.search_generation.wrapping_add(1);
        let generation = self.coordinator.search_generation;
        let query = self.search_input.read(cx).display_text().to_owned();
        if query.is_empty() {
            self.search_results.clear();
            self.search_selected = 0;
            self.search_running = false;
            self.search_error = None;
            cx.notify();
            return;
        }
        let document = self.document.clone();
        let provisional_source = self.provisional_source.clone();
        if document.is_none() && provisional_source.is_none() {
            return;
        }
        let task_stamp = DocumentTaskStamp::capture(self, generation);
        let options = self.search_options;
        let cancellation = SearchCancellation::default();
        self.coordinator.search_cancellation = Some(cancellation.clone());
        self.search_running = true;
        self.search_error = None;
        self.coordinator.search_task = cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(60))
                .await;
            let first_document = document.clone();
            let first_source = provisional_source.clone();
            let first_query = query.clone();
            let first_cancellation = cancellation.clone();
            let first_options = SearchOptions {
                result_limit: 1,
                ..options
            };
            let first = cx
                .background_spawn(async move {
                    search_document_reader(
                        first_document.as_ref(),
                        first_source.as_ref(),
                        &first_query,
                        first_options,
                        &first_cancellation,
                    )
                })
                .await;
            let continue_full = this.update(cx, |view, cx| {
                if !task_stamp.accepts_strict(view, view.coordinator.search_generation) {
                    return false;
                }
                match first {
                    Ok(matches) => {
                        view.search_results = matches;
                        view.search_selected = 0;
                        view.search_error = None;
                        if !view.search_results.is_empty() {
                            view.jump_to_search_result(cx);
                        } else {
                            view.search_running = false;
                            view.coordinator.search_cancellation = None;
                        }
                    }
                    Err(gmark_paged_document::PagedDocumentError::Cancelled) => {
                        return false;
                    }
                    Err(error) => {
                        view.search_running = false;
                        view.coordinator.search_cancellation = None;
                        view.search_results.clear();
                        view.search_error = Some(
                            cx.global::<I18nManager>()
                                .strings()
                                .large_document_error(&error)
                                .into(),
                        );
                    }
                }
                cx.notify();
                view.search_running && options.result_limit > 1
            });
            let Ok(true) = continue_full else {
                return;
            };
            let result = cx
                .background_spawn(async move {
                    search_document_reader(
                        document.as_ref(),
                        provisional_source.as_ref(),
                        &query,
                        options,
                        &cancellation,
                    )
                })
                .await;
            let _ = this.update(cx, |view, cx| {
                if !task_stamp.accepts_strict(view, view.coordinator.search_generation) {
                    return;
                }
                view.search_running = false;
                view.coordinator.search_cancellation = None;
                match result {
                    Ok(matches) => {
                        view.search_results = matches;
                        view.search_selected = 0;
                        view.search_error = None;
                    }
                    Err(gmark_paged_document::PagedDocumentError::Cancelled) => {}
                    Err(error) => {
                        view.search_results.clear();
                        view.search_error = Some(
                            cx.global::<I18nManager>()
                                .strings()
                                .large_document_error(&error)
                                .into(),
                        );
                    }
                }
                cx.notify();
            });
        });
        cx.notify();
    }
}
