// @author kongweiguang

//! DocumentHost test-only interaction helpers.

use super::*;
use gmark_document_core::{DocumentRevision, DocumentSnapshot, SnapshotError};
use std::sync::Arc;

/// Immutable selection view passed to the runtime's encoded snapshot writer.
/// It keeps selection export lock-free without reviving a host-owned save plan.
struct SelectionSnapshot {
    source: Arc<dyn DocumentSnapshot>,
    range: Range<u64>,
}

impl DocumentSnapshot for SelectionSnapshot {
    fn revision(&self) -> DocumentRevision {
        self.source.revision()
    }

    fn len(&self) -> u64 {
        self.range.end.saturating_sub(self.range.start)
    }

    fn read_range(&self, requested: Range<u64>) -> Result<Vec<u8>, SnapshotError> {
        let len = self.len();
        if requested.start > requested.end || requested.end > len {
            return Err(SnapshotError::InvalidRange {
                start: requested.start,
                end: requested.end,
                len,
            });
        }
        self.source.read_range(
            self.range.start.saturating_add(requested.start)
                ..self.range.start.saturating_add(requested.end),
        )
    }
}

/// Test-only snapshot so assertions can observe viewport cache accounting
/// without making the production metrics state part of the host API.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PagedDocumentMetricsSnapshot {
    pub(crate) viewport_requests: u64,
    pub(crate) viewport_installs: u64,
    pub(crate) max_cached_rows: usize,
    pub(crate) blank_frames_after_content: u64,
}

impl DocumentHost {
    #[cfg(test)]
    pub(crate) fn source_row_block_count_for_test(&self) -> usize {
        self.source_row_blocks.len()
    }

    #[cfg(test)]
    pub(crate) fn source_row_block_for_test(&self, line: usize) -> Option<Entity<Block>> {
        self.source_row_blocks.get(&line).cloned()
    }

    #[cfg(test)]
    pub(crate) fn inactive_source_row_block_for_test(&self) -> Option<(usize, Entity<Block>)> {
        let active_line = self.active_edit.as_ref().map(|active| active.line);
        self.source_row_blocks
            .iter()
            .find(|(line, _)| Some(**line) != active_line)
            .map(|(line, block)| (*line, block.clone()))
    }

    #[cfg(test)]
    pub(crate) fn screen_lines_contract_for_test(
        &self,
    ) -> (u64, u64, u64, u64, Range<usize>, usize, bool, bool) {
        let screen = &self.displayed_screen_lines;
        let epochs_match = screen
            .rows
            .keys()
            .all(|line| self.source_row_epochs.get(line) == Some(&screen.cache_epoch));
        let revision_matches =
            screen.document_revision == self.document.as_ref().map_or(0, SharedDocument::revision);
        (
            screen.document_revision,
            screen.generation,
            screen.cache_epoch,
            screen.column_window_start,
            screen.visible.clone(),
            screen.rows.len(),
            epochs_match,
            revision_matches,
        )
    }

    #[cfg(test)]
    pub(crate) fn metrics_for_test(&self) -> super::PagedDocumentMetricsSnapshot {
        let metrics = self.metrics;
        super::PagedDocumentMetricsSnapshot {
            viewport_requests: metrics.viewport_requests,
            viewport_installs: metrics.viewport_installs,
            max_cached_rows: metrics.max_cached_rows,
            blank_frames_after_content: metrics.blank_frames_after_content,
        }
    }

    #[cfg(test)]
    pub(crate) fn source_layout_cache_metrics_for_test(&self, cx: &App) -> (u64, u64, usize) {
        self.source_row_blocks.values().fold(
            (0u64, 0u64, 0usize),
            |(hits, misses, entries), block| {
                let block = block.read(cx);
                (
                    hits.saturating_add(block.source_layout_cache_hits),
                    misses.saturating_add(block.source_layout_cache_misses),
                    entries + usize::from(block.source_layout_cache_key.is_some()),
                )
            },
        )
    }

    #[cfg(test)]
    pub(crate) fn viewport_cancellations_for_test(&self) -> u64 {
        self.metrics.viewport_cancellations
    }

    #[cfg(test)]
    pub(crate) fn export_selection_to_path_for_test(
        &self,
        path: &Path,
        force_utf8: bool,
    ) -> Result<String, PagedDocumentError> {
        let range = self
            .selected_source_byte_range()
            .ok_or_else(|| PagedDocumentError::InvalidTransaction("missing selection".into()))?;
        let document = self
            .document
            .as_ref()
            .ok_or_else(|| PagedDocumentError::InvalidTransaction("missing document".into()))?;
        // Keep the test on the same immutable snapshot boundary as production
        // saves.  The encoded range operation remains owned by the runtime
        // backend; the host no longer exposes a plan or range-save wrapper.
        let snapshot = document
            .with_session(gmark_document_runtime::DocumentSession::save_snapshot)
            .map_err(|error| PagedDocumentError::InvalidTransaction(error.to_string()))?;
        let selection = SelectionSnapshot {
            source: snapshot.snapshot.clone(),
            range,
        };
        if !force_utf8 && let Some(plan) = snapshot.paged_save_plan.as_ref() {
            let encoding = plan.encoding_name();
            plan.save_snapshot_atomic_as_cancellable(
                &selection,
                path,
                &SearchCancellation::default(),
            )?;
            return Ok(encoding);
        }
        let bytes = selection
            .read_range(0..selection.len())
            .map_err(|error| PagedDocumentError::InvalidTransaction(error.to_string()))?;
        gmark_document::atomic_write(path, &bytes).map_err(|error| {
            PagedDocumentError::Persist {
                path: path.to_path_buf(),
                source: std::io::Error::other(error.to_string()),
            }
        })?;
        Ok("UTF-8".to_owned())
    }

    #[cfg(test)]
    pub(crate) fn source_context_menu_open_for_test(&self) -> bool {
        self.source_context_menu.is_some()
    }

    #[cfg(test)]
    pub(crate) fn source_context_menu_is_focused_for_test(&self, window: &Window) -> bool {
        self.source_context_menu_focus_handle.is_focused(window)
    }

    #[cfg(test)]
    pub(crate) fn copy_for_test(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.on_copy(&Copy, window, cx);
    }

    #[cfg(test)]
    pub(crate) fn cut_for_test(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.on_cut(&Cut, window, cx);
    }

    #[cfg(test)]
    pub(crate) fn paste_for_test(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.on_paste(&Paste, window, cx);
    }

    #[cfg(test)]
    pub(crate) fn undo_for_test(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.on_undo(&Undo, window, cx);
    }

    #[cfg(test)]
    pub(crate) fn redo_for_test(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.on_redo(&Redo, window, cx);
    }

    #[cfg(test)]
    pub(crate) fn close_navigation_for_test(&mut self, cx: &mut Context<Self>) {
        self.navigation_visible = false;
        cx.notify();
    }

    #[cfg(test)]
    pub(crate) fn navigation_visible_for_test(&self) -> bool {
        self.navigation_visible
    }

    #[cfg(test)]
    pub(crate) fn search_text_for_test(&self, cx: &App) -> String {
        self.search_input.read(cx).display_text().to_owned()
    }

    /// Exposes the real Host field so IME tests can drive its native composition lifecycle.
    #[cfg(test)]
    pub(crate) fn search_input_for_test(&self) -> Entity<Block> {
        self.search_input.clone()
    }

    /// Lets focus-transition tests verify that navigation receives focus only after IME termination.
    #[cfg(test)]
    pub(crate) fn navigation_input_for_test(&self) -> Entity<Block> {
        self.navigation_input.clone()
    }

    #[cfg(test)]
    pub(crate) fn host_is_focused_for_test(&self, window: &Window) -> bool {
        self.focus_handle.is_focused(window)
    }

    #[cfg(test)]
    pub(crate) fn pending_external_change_for_test(&self) -> Option<ExternalChange> {
        self.coordinator.pending_external_change.clone()
    }

    #[cfg(test)]
    pub(crate) fn external_monitor_paused_for_test(&self) -> bool {
        self.coordinator.external_monitor_paused
    }

    #[cfg(test)]
    pub(crate) fn keep_local_for_test(&mut self, cx: &mut Context<Self>) {
        self.keep_local_after_external_change(cx);
    }

    #[cfg(test)]
    pub(crate) fn reload_from_disk_for_test(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.reload_from_disk(window, cx);
    }

    #[cfg(test)]
    pub(crate) fn markdown_table_state_for_test(&self) -> Option<(usize, usize, Vec<String>, u64)> {
        let StructuredIndex::MarkdownTables { tables, selected } =
            self.structured_index.as_ref()?
        else {
            return None;
        };
        let table = tables.get(*selected)?;
        Some((
            *selected,
            tables.len(),
            table.headers().to_vec(),
            table.row_count(),
        ))
    }

    #[cfg(test)]
    pub(crate) fn structure_error_for_test(&self) -> Option<(String, Option<u64>)> {
        Some((
            self.structure_error.as_ref()?.to_string(),
            self.structure_error_byte,
        ))
    }
}
