// @author kongweiguang

//! Undo history and selection snapshot restoration.

use super::*;

/// 历史正文可保留原拼写；仅补齐换行格式数量，使异常旧快照仍可安全保存。
fn source_format_for_history_text(
    text: &str,
    mut format: gmark_document::SourceFormatSnapshot,
) -> gmark_document::SourceFormatSnapshot {
    let ending_count = text.bytes().filter(|byte| *byte == b'\n').count();
    format.endings.resize(ending_count, format.dominant);
    format
}

impl Editor {
    pub(super) fn empty_selection_snapshot() -> UndoSelectionSnapshot {
        UndoSelectionSnapshot::collapsed(0, SourceAffinity::Before)
    }

    /// 对外快照统一使用权威源码坐标；Live 的规范化拼写只留在投影内部。
    pub(super) fn capture_source_selection_snapshot(&self, cx: &App) -> UndoSelectionSnapshot {
        let _trace = crate::perf::span("selection_snapshot_sync");
        if self.view_mode == ViewMode::Preview {
            return self.last_selection_snapshot;
        }
        if let Some(snapshot) = self.cross_block_source_selection_snapshot(cx) {
            return self.map_selection_source_spelling(snapshot, true, cx);
        }

        if matches!(self.view_mode, ViewMode::Source | ViewMode::Split) {
            let snapshot = self
                .document
                .first_root()
                .map(|block| {
                    let block_ref = block.read(cx);
                    UndoSelectionSnapshot::from_range(
                        block_ref.selected_range.clone(),
                        block_ref.selection_reversed,
                    )
                })
                .unwrap_or_else(Self::empty_selection_snapshot);
            return snapshot;
        }

        let Some(target) = self.current_edit_target_from_state(cx) else {
            return self.last_selection_snapshot;
        };
        let mapping = self
            .build_source_target_mapping_for_entity(target.entity_id(), cx)
            .or_else(|| {
                self.build_source_target_mappings(cx)
                    .into_iter()
                    .find(|mapping| mapping.entity.entity_id() == target.entity_id())
            });
        let Some(mapping) = mapping else {
            return self.last_selection_snapshot;
        };

        let selected_range = target.read(cx).selected_range.clone();
        let content_range = target
            .read(cx)
            .current_range_to_markdown_range(selected_range);
        let max_offset = mapping.content_to_source.len().saturating_sub(1);
        let start = mapping.full_source_range.start
            + mapping.content_to_source[content_range.start.min(max_offset)];
        let end = mapping.full_source_range.start
            + mapping.content_to_source[content_range.end.min(max_offset)];

        self.map_selection_source_spelling(
            UndoSelectionSnapshot::from_range(start..end, target.read(cx).selection_reversed),
            true,
            cx,
        )
    }

    /// 历史正文来自已同步的源码；投影规范化仅供显示，不能因捕获历史改写原 Markdown 拼写。
    pub(super) fn capture_history_entry(&self, kind: UndoCaptureKind, cx: &App) -> HistoryEntry {
        let source_text = self
            .pending_dirty_source
            .clone()
            .unwrap_or_else(|| self.source_document.text());
        let source_format =
            source_format_for_history_text(&source_text, self.source_document.source_format());
        HistoryEntry {
            source: HistorySource::capture(self.source_document.snapshot(), source_text),
            source_format,
            selection: self.capture_source_selection_snapshot(cx),
            timestamp: Instant::now(),
            kind,
        }
    }

    pub(super) fn capture_stable_history_entry(&self, kind: UndoCaptureKind) -> HistoryEntry {
        let source_text = self.last_stable_source.text();
        HistoryEntry {
            source_format: source_format_for_history_text(
                &source_text,
                self.source_document.source_format(),
            ),
            source: self.last_stable_source.clone(),
            selection: self.last_selection_snapshot,
            timestamp: Instant::now(),
            kind,
        }
    }

    pub(super) fn prepare_undo_capture(&mut self, kind: UndoCaptureKind, cx: &mut Context<Self>) {
        if self.virtual_surface.is_some() {
            if self.pending_virtual_undo_selection.is_none() {
                self.pending_virtual_undo_selection =
                    Some(self.capture_source_selection_snapshot(cx));
            }
            return;
        }
        if self.history_restore_in_progress || self.pending_undo_capture.is_some() {
            return;
        }
        self.pending_undo_capture = Some(PendingUndoCapture {
            snapshot: self.capture_history_entry(kind, cx),
        });
    }

    pub(super) fn prepare_undo_capture_from_stable_snapshot(&mut self, kind: UndoCaptureKind) {
        if self.virtual_surface.is_some() {
            if self.pending_virtual_undo_selection.is_none() {
                self.pending_virtual_undo_selection = Some(self.last_selection_snapshot);
            }
            return;
        }
        if self.history_restore_in_progress || self.pending_undo_capture.is_some() {
            return;
        }
        self.pending_undo_capture = Some(PendingUndoCapture {
            snapshot: self.capture_stable_history_entry(kind),
        });
    }

    /// 表格行操作从权威源码捕获正文；事件携带修改前的局部选区，避免异步派发时读取修改后的 cell。
    /// 表格 record 此时尚未同步新内容，因此既有映射仍属于被捕获的正文和输入目标。
    pub(super) fn prepare_undo_capture_from_source_document_snapshot(
        &mut self,
        kind: UndoCaptureKind,
        entity_id: EntityId,
        markdown_selection: std::ops::Range<usize>,
        reversed: bool,
        cx: &App,
    ) {
        let selection = self
            .build_source_target_mapping_for_entity(entity_id, cx)
            .map(|mapping| {
                let max_offset = mapping.content_to_source.len().saturating_sub(1);
                let start = mapping.full_source_range.start
                    + mapping.content_to_source[markdown_selection.start.min(max_offset)];
                let end = mapping.full_source_range.start
                    + mapping.content_to_source[markdown_selection.end.min(max_offset)];
                self.map_selection_source_spelling(
                    UndoSelectionSnapshot::from_range(start..end, reversed),
                    true,
                    cx,
                )
            })
            .unwrap_or(self.last_selection_snapshot);
        if self.virtual_surface.is_some() {
            if self.pending_virtual_undo_selection.is_none() {
                self.pending_virtual_undo_selection = Some(selection);
            }
            return;
        }
        if self.history_restore_in_progress || self.pending_undo_capture.is_some() {
            return;
        }

        let source = self
            .pending_dirty_source
            .as_ref()
            .map(|source| HistorySource::Materialized(Arc::from(source.as_str())))
            .unwrap_or_else(|| HistorySource::Snapshot(self.source_document.snapshot()));
        let source_format = self
            .pending_dirty_source
            .as_deref()
            .map(|source| {
                source_format_for_history_text(source, self.source_document.source_format())
            })
            .unwrap_or_else(|| self.source_document.source_format());
        self.pending_undo_capture = Some(PendingUndoCapture {
            snapshot: HistoryEntry {
                source,
                source_format,
                selection,
                timestamp: Instant::now(),
                kind,
            },
        });
    }

    /// 选区与焦点刷新沿用权威正文；仅消费真实修改产生的暂存源码，避免下一次输入撤销退回规范化投影。
    pub(super) fn refresh_stable_document_snapshot(&mut self, cx: &App) {
        self.last_selection_snapshot = self.capture_source_selection_snapshot(cx);
        if self.virtual_surface.is_some() {
            self.pending_dirty_source = None;
            self.last_stable_source = HistorySource::empty();
            return;
        }
        let source_text = self
            .pending_dirty_source
            .take()
            .unwrap_or_else(|| self.source_document.text());
        self.last_stable_source =
            HistorySource::capture(self.source_document.snapshot(), source_text);
    }

    /// 只把成功提交的权威正文加入历史；失败投影由恢复入口保留，不能生成伪撤销或规范化快照。
    pub(super) fn finalize_pending_undo_capture(&mut self, cx: &mut Context<Self>) {
        if self.document.source_commit_error().is_some() {
            self.pending_undo_capture = None;
            self.pending_dirty_source = None;
            return;
        }
        if self.virtual_surface.is_some() {
            if let Some(selection) = self.pending_virtual_undo_selection.take() {
                self.virtual_undo_selections.push(selection);
                if self.virtual_undo_selections.len() > Self::HISTORY_LIMIT {
                    let overflow = self.virtual_undo_selections.len() - Self::HISTORY_LIMIT;
                    self.virtual_undo_selections.drain(0..overflow);
                }
                self.virtual_redo_selections.clear();
            }
            self.last_selection_snapshot = self.capture_source_selection_snapshot(cx);
            self.pending_dirty_source = None;
            return;
        }
        if self.history_restore_in_progress {
            self.pending_undo_capture = None;
            return;
        }

        let current_source = self
            .pending_dirty_source
            .take()
            .unwrap_or_else(|| self.source_document.text());
        let Some(pending) = self.pending_undo_capture.take() else {
            self.last_selection_snapshot = self.capture_source_selection_snapshot(cx);
            self.last_stable_source =
                HistorySource::capture(self.source_document.snapshot(), current_source);
            return;
        };

        if pending.snapshot.source.matches_text(&current_source)
            && self.source_document.source_format() == pending.snapshot.source_format
        {
            self.last_selection_snapshot = self.capture_source_selection_snapshot(cx);
            self.last_stable_source =
                HistorySource::capture(self.source_document.snapshot(), current_source);
            return;
        }

        // A fresh edit invalidates any forward history available for redo.
        self.redo_history.clear();

        let should_merge = self.undo_history.last().is_some_and(|entry| {
            let ordinary_text = matches!(pending.snapshot.kind, UndoCaptureKind::CoalescibleText)
                && matches!(entry.kind, UndoCaptureKind::CoalescibleText)
                && pending
                    .snapshot
                    .timestamp
                    .saturating_duration_since(entry.timestamp)
                    <= Self::HISTORY_COALESCE_WINDOW;
            let same_ime_composition = matches!(
                pending.snapshot.kind,
                UndoCaptureKind::ImeComposition | UndoCaptureKind::ImeCompositionCommit
            ) && matches!(entry.kind, UndoCaptureKind::ImeComposition);
            ordinary_text || same_ime_composition
        });
        if !should_merge {
            self.undo_history.push(pending.snapshot);
            if self.undo_history.len() > Self::HISTORY_LIMIT {
                let overflow = self.undo_history.len() - Self::HISTORY_LIMIT;
                self.undo_history.drain(0..overflow);
            }
        } else if matches!(pending.snapshot.kind, UndoCaptureKind::ImeCompositionCommit)
            && let Some(entry) = self.undo_history.last_mut()
        {
            // 提交或取消后封口，下一次 IME composition 必须建立新的撤销边界。
            entry.kind = UndoCaptureKind::ImeCompositionCommit;
        }
        self.last_selection_snapshot = self.capture_source_selection_snapshot(cx);
        self.last_stable_source =
            HistorySource::capture(self.source_document.snapshot(), current_source);
    }

    /// 历史和跨模式快照以原源码保存；只在 Live 恢复时转回规范投影，保持同步源选区不变。
    pub(super) fn apply_selection_snapshot_in_current_mode(
        &mut self,
        snapshot: &UndoSelectionSnapshot,
        cx: &mut Context<Self>,
    ) {
        self.source_document
            .sync_source_selection(snapshot.source_selection());
        let range = snapshot.range();
        let reversed = snapshot.reversed();
        match self.view_mode {
            ViewMode::Source | ViewMode::Split => {
                let Some(block) = self.document.first_root().cloned() else {
                    return;
                };
                let len = block.read(cx).visible_len();
                let selected_range = range.start.min(len)..range.end.min(len);
                block.update(cx, move |block, cx| {
                    block.selected_range = selected_range.clone();
                    block.selection_reversed = reversed;
                    block.marked_range = None;
                    block.vertical_motion_x = None;
                    block.cursor_blink_epoch = Instant::now();
                    cx.notify();
                });
                self.pending_focus = Some(block.entity_id());
                self.active_entity_id = Some(block.entity_id());
            }
            ViewMode::Rendered | ViewMode::Preview => {
                let projection_snapshot = self.map_selection_source_spelling(*snapshot, false, cx);
                let snapshot = &projection_snapshot;
                let range = snapshot.range();
                let reversed = snapshot.reversed();
                if self.apply_cross_block_selection_snapshot_if_possible(snapshot, cx) {
                    return;
                }

                let mappings = self.build_source_target_mappings(cx);
                let exact_mapping = mappings.iter().find(|mapping| {
                    let contains_start =
                        Self::source_range_contains(&mapping.full_source_range, range.start);
                    let contains_end =
                        Self::source_range_contains(&mapping.full_source_range, range.end);
                    if !contains_start || !contains_end {
                        return false;
                    }
                    let local_start = range.start.saturating_sub(mapping.full_source_range.start);
                    let local_end = range.end.saturating_sub(mapping.full_source_range.start);
                    let content_start = mapping.source_to_content
                        [local_start.min(mapping.source_to_content.len().saturating_sub(1))];
                    let content_end = mapping.source_to_content
                        [local_end.min(mapping.source_to_content.len().saturating_sub(1))];
                    let max_content = mapping.content_to_source.len().saturating_sub(1);
                    mapping.content_to_source[content_start.min(max_content)] == local_start
                        && mapping.content_to_source[content_end.min(max_content)] == local_end
                });

                if let Some(mapping) = exact_mapping {
                    let local_start = range.start - mapping.full_source_range.start;
                    let local_end = range.end - mapping.full_source_range.start;
                    let content_start = mapping.source_to_content[local_start];
                    let content_end = mapping.source_to_content[local_end];
                    let selected_range = mapping
                        .entity
                        .read(cx)
                        .markdown_range_to_current_range(content_start..content_end);
                    mapping.entity.update(cx, move |block, cx| {
                        block.selected_range = selected_range.clone();
                        block.selection_reversed = reversed;
                        block.marked_range = None;
                        block.vertical_motion_x = None;
                        block.cursor_blink_epoch = Instant::now();
                        cx.notify();
                    });
                    self.pending_focus = Some(mapping.entity.entity_id());
                    self.active_entity_id = Some(mapping.entity.entity_id());
                    return;
                }

                // 派生/Rendered 回退必须跟随真实 head，而不是规范化 Range 的 end；
                // 否则反向选择在无法精确映射时会把 caret 放到错误一侧。
                let caret_offset =
                    saturating_source_offset(snapshot.source_selection().head.byte_offset);
                let best = mappings.iter().min_by_key(|mapping| {
                    Self::source_offset_distance(&mapping.full_source_range, caret_offset)
                });
                let Some(mapping) = best else {
                    self.pending_focus = self.first_focusable_entity_id(cx);
                    self.active_entity_id = self.pending_focus;
                    return;
                };
                let local_source = if caret_offset <= mapping.full_source_range.start {
                    0
                } else if caret_offset >= mapping.full_source_range.end {
                    mapping.full_source_range.len()
                } else {
                    caret_offset - mapping.full_source_range.start
                };
                let content_offset = mapping.source_to_content
                    [local_source.min(mapping.source_to_content.len().saturating_sub(1))];
                let current_offset = mapping
                    .entity
                    .read(cx)
                    .markdown_offset_to_current_offset(content_offset);
                mapping.entity.update(cx, move |block, cx| {
                    block.assign_collapsed_selection_offset(
                        current_offset,
                        crate::components::CollapsedCaretAffinity::Default,
                        None,
                    );
                    block.marked_range = None;
                    block.cursor_blink_epoch = Instant::now();
                    cx.notify();
                });
                self.pending_focus = Some(mapping.entity.entity_id());
                self.active_entity_id = Some(mapping.entity.entity_id());
            }
        }
    }

    pub(super) fn source_range_contains(range: &std::ops::Range<usize>, offset: usize) -> bool {
        if range.start == range.end {
            offset == range.start
        } else {
            offset >= range.start && offset <= range.end
        }
    }

    pub(super) fn source_offset_distance(range: &std::ops::Range<usize>, offset: usize) -> usize {
        if Self::source_range_contains(range, offset) {
            0
        } else if offset < range.start {
            range.start - offset
        } else {
            offset.saturating_sub(range.end)
        }
    }

    /// 恢复历史中的权威源码，并让后续稳定快照与保存继续使用该源码，而非投影的规范序列化。
    pub(super) fn restore_history_entry(&mut self, entry: &HistoryEntry, cx: &mut Context<Self>) {
        // 只有真正执行 undo/redo 时才物化完整正文，历史长期驻留保持 Rope COW。
        let source_text = entry.source.text();
        self.sync_source_document_from_projection(&source_text);
        self.pending_dirty_source = Some(source_text.clone());
        // 历史快照来自 UI 输入与源码同步的交界处。即使旧版本或异常事件顺序留下了
        // 换行数量不匹配的快照，也必须修复为可序列化格式，不能在撤销时终止主线程。
        let format = source_format_for_history_text(&source_text, entry.source_format.clone());
        if let Err(error) = self.source_document.try_restore_source_format(format) {
            eprintln!("恢复历史源码格式失败: {error}");
        }
        match self.view_mode {
            ViewMode::Rendered | ViewMode::Preview => {
                self.rebuild_primary_projection_from_source_reusing(cx);
            }
            ViewMode::Source | ViewMode::Split => {
                let block = Self::new_block(cx, BlockRecord::paragraph(source_text));
                block.update(cx, |block, _cx| block.set_source_document_mode());
                self.document.replace_roots(vec![block], cx);
                self.table_cells.clear();
            }
        }

        self.set_projection_read_only(self.view_mode == ViewMode::Preview, cx);
        if self.view_mode == ViewMode::Split {
            self.schedule_split_preview_projection(cx);
        }

        self.apply_selection_snapshot_in_current_mode(&entry.selection, cx);
        self.pending_scroll_active_block_into_view = true;
        self.pending_scroll_recheck_after_layout = true;
        self.last_scroll_viewport_size = None;
        self.refresh_stable_document_snapshot(cx);
    }

    /// 引用重解析先提交所属区域，再从权威源码重建；结构整理不能规范化其它段落的原始拼写。
    pub(super) fn normalize_rendered_quote_structure(&mut self, cx: &mut Context<Self>) {
        if self.view_mode != ViewMode::Rendered {
            return;
        }

        let selection_snapshot = self.capture_source_selection_snapshot(cx);
        self.mark_dirty(cx);
        if self.document.source_commit_error().is_some() {
            return;
        }
        self.rebuild_primary_projection_from_source_reusing(cx);
        self.apply_selection_snapshot_in_current_mode(&selection_snapshot, cx);
        self.pending_scroll_active_block_into_view = true;
        self.pending_scroll_recheck_after_layout = true;
        self.last_scroll_viewport_size = None;
    }

    /// 历史写入入口校验实际表面与失败恢复门禁，备份前不能覆盖尚未提交的可见文字。
    pub(super) fn undo_document(&mut self, cx: &mut Context<Self>) {
        if !self.document_surface_is_editable()
            || self.document.source_commit_error().is_some()
            || self.has_active_ime_composition(cx)
        {
            return;
        }
        if self.virtual_surface.is_some() {
            self.undo_virtual_document(cx);
            return;
        }
        let Some(entry) = self.undo_history.pop() else {
            return;
        };

        // Snapshot the current document so redo can step forward to it.
        let current = self.capture_history_entry(UndoCaptureKind::NonCoalescible, cx);
        self.pending_undo_capture = None;
        self.history_restore_in_progress = true;
        self.clear_cross_block_selection(cx);
        self.restore_history_entry(&entry, cx);
        self.history_restore_in_progress = false;
        self.redo_history.push(current);
        self.mark_restored_document_dirty(cx);
        self.sync_table_axis_visuals(cx);
        self.dismiss_contextual_overlays(cx);
        cx.notify();
    }

    /// Split 右侧、候选与失败恢复期间保持正文及历史不变量。
    pub(super) fn redo_document(&mut self, cx: &mut Context<Self>) {
        if !self.document_surface_is_editable()
            || self.document.source_commit_error().is_some()
            || self.has_active_ime_composition(cx)
        {
            return;
        }
        if self.virtual_surface.is_some() {
            self.redo_virtual_document(cx);
            return;
        }
        let Some(entry) = self.redo_history.pop() else {
            return;
        };

        // Snapshot the current document so undo can step back to it again.
        let current = self.capture_history_entry(UndoCaptureKind::NonCoalescible, cx);
        self.pending_undo_capture = None;
        self.history_restore_in_progress = true;
        self.clear_cross_block_selection(cx);
        self.restore_history_entry(&entry, cx);
        self.history_restore_in_progress = false;
        self.undo_history.push(current);
        self.mark_restored_document_dirty(cx);
        self.sync_table_axis_visuals(cx);
        self.dismiss_contextual_overlays(cx);
        cx.notify();
    }

    fn undo_virtual_document(&mut self, cx: &mut Context<Self>) {
        let Some(selection) = self.virtual_undo_selections.pop() else {
            return;
        };
        let current_selection = self.capture_source_selection_snapshot(cx);
        let snapshot = match self.source_document.undo() {
            Ok(Some(snapshot)) => snapshot,
            Ok(None) => return,
            Err(error) => {
                eprintln!("virtual surface 撤销源码事务失败: {error}");
                return;
            }
        };
        self.virtual_redo_selections.push(current_selection);
        self.restore_virtual_document_snapshot(snapshot, &selection, cx);
    }

    fn redo_virtual_document(&mut self, cx: &mut Context<Self>) {
        let Some(selection) = self.virtual_redo_selections.pop() else {
            return;
        };
        let current_selection = self.capture_source_selection_snapshot(cx);
        let snapshot = match self.source_document.redo() {
            Ok(Some(snapshot)) => snapshot,
            Ok(None) => return,
            Err(error) => {
                eprintln!("virtual surface 重做源码事务失败: {error}");
                return;
            }
        };
        self.virtual_undo_selections.push(current_selection);
        self.restore_virtual_document_snapshot(snapshot, &selection, cx);
    }

    fn restore_virtual_document_snapshot(
        &mut self,
        snapshot: gmark_document::DocumentSnapshot,
        selection: &UndoSelectionSnapshot,
        cx: &mut Context<Self>,
    ) {
        let prepared = Arc::new(if let Some(previous) = self.projection_cache.as_deref() {
            PreparedSplitProjection::from_snapshot_incremental_regions_only(snapshot, previous)
        } else {
            PreparedSplitProjection::from_snapshot_adaptive(
                snapshot,
                Self::VIRTUAL_SURFACE_REGION_THRESHOLD,
            )
        });
        self.active_entity_id = None;
        self.pending_focus = None;
        self.install_virtual_surface_projection(Arc::clone(&prepared), cx);
        self.projection_cache = Some(prepared);
        self.apply_selection_snapshot_in_current_mode(selection, cx);
        self.last_selection_snapshot = *selection;
        self.pending_virtual_undo_selection = None;
        self.pending_scroll_active_block_into_view = true;
        self.pending_scroll_recheck_after_layout = true;
        self.last_scroll_viewport_size = None;
        self.document_dirty = true;
        self.pending_window_edited = true;
        self.pending_window_title_refresh = true;
        self.schedule_recovery_journal(cx);
        self.schedule_auto_save(cx);
        self.schedule_active_block_spellcheck(cx);
        self.sync_table_axis_visuals(cx);
        self.dismiss_contextual_overlays(cx);
        cx.notify();
    }
}
