// @author kongweiguang

//! Delimited-table cell and row editing.

use super::*;

impl DocumentHost {
    /// Defers cell actions until the Block update lease ends, so Host callbacks can safely read or refocus the editor.
    pub(super) fn begin_structured_cell_edit(
        &mut self,
        record: Option<u64>,
        column: usize,
        value: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.view_mode != DocumentHostViewMode::Live || !self.is_delimited_document() {
            return;
        }
        self.structured_cell_edit = Some(StructuredCellEdit { record, column });
        let host = cx.entity().downgrade();
        self.structured_cell_input.update(cx, move |input, cx| {
            input.set_host_action_handler(move |action, window, cx| {
                let host = host.clone();
                window.defer(cx, move |window, cx| {
                    let _ = host.update(cx, |view, cx| {
                        view.on_structured_cell_host_action(action, window, cx)
                    });
                });
            });
            let len = input.display_text().len();
            input.replace_text_in_visible_range(0..len, &value, None, false, cx);
            input.focus_handle.focus(window);
        });
        cx.notify();
    }

    /// Keeps an in-progress cell edit authoritative before pointer focus moves to another table cell.
    pub(super) fn select_structured_cell(
        &mut self,
        target: StructuredCellEdit,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.view_mode == DocumentHostViewMode::Source {
            return;
        }
        if self.defer_source_action_for_ime(
            super::source_ime::DeferredSourceAction::StructuredCellSelect(target),
            window,
            cx,
        ) {
            return;
        }
        if self
            .structured_cell_edit
            .is_some_and(|editing| editing != target)
        {
            // 点击另一格会先把焦点交还表格；必须在旧编辑器失焦前提交其权威文本，
            // 否则下一次渲染只会重新读取索引中的旧值，造成用户输入静默丢失。
            let value = self
                .structured_cell_input
                .read(cx)
                .display_text()
                .to_owned();
            if !self.commit_structured_cell_edit(value, cx) {
                return;
            }
        }
        self.structured_selected_cell = Some(target);
        self.structured_focus_handle.focus(window);
        cx.notify();
    }

    /// 处理快捷键 action 之后仍到达网格的原始按键，负责进入单元格编辑和取消等路径。
    pub(super) fn on_structured_table_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.structured_focus_handle.is_focused(window)
            && !self
                .structured_cell_input
                .read(cx)
                .focus_handle
                .is_focused(window)
        {
            return;
        }
        let Some(selected) = self.structured_selected_cell else {
            return;
        };
        match event.keystroke.key.as_str() {
            "enter" => {
                if self.structured_cell_edit.is_some() {
                    return;
                }
                if let Some(value) = self.structured_cell_value(selected) {
                    self.begin_structured_cell_edit(
                        selected.record,
                        selected.column,
                        value,
                        window,
                        cx,
                    );
                    cx.stop_propagation();
                }
            }
            "tab" => {
                let delta = if event.keystroke.modifiers.shift {
                    -1
                } else {
                    1
                };
                let _ = self.move_structured_cell_by_tab(delta, window, cx);
            }
            "escape" => {
                if self.structured_cell_input.read(cx).has_ime_composition() {
                    return;
                }
                self.structured_cell_edit = None;
                self.structured_focus_handle.focus(window);
                cx.stop_propagation();
                cx.notify();
            }
            _ => {}
        }
    }

    /// 网格拥有焦点时先接管 Tab，避免外围 Source 行操作消费同一个快捷键。
    pub(super) fn on_structured_grid_indent(
        &mut self,
        _: &IndentBlock,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let _ = self.move_structured_cell_by_tab(1, window, cx);
    }

    /// 让 Shift+Tab 复用 Tab 的提交和焦点恢复路径，避免两套邻格导航行为分叉。
    pub(super) fn on_structured_grid_outdent(
        &mut self,
        _: &OutdentBlock,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let _ = self.move_structured_cell_by_tab(-1, window, cx);
    }

    /// 仅在分隔表格拥有焦点时提交并移动选中格；提交失败仍消费 action，以保留当前编辑现场。
    fn move_structured_cell_by_tab(
        &mut self,
        delta: i32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let focused = self.structured_focus_handle.is_focused(window)
            || self
                .structured_cell_input
                .read(cx)
                .focus_handle
                .is_focused(window);
        if !self.is_delimited_document() || !focused {
            return false;
        }
        if self.structured_cell_input.read(cx).has_ime_composition() {
            return false;
        }
        cx.stop_propagation();
        let Some(selected) = self.structured_selected_cell else {
            return true;
        };
        if self.structured_cell_edit.is_some() {
            let value = self
                .structured_cell_input
                .read(cx)
                .display_text()
                .to_owned();
            if !self.commit_structured_cell_edit(value, cx) {
                return true;
            }
        }
        let Some(next) = self.adjacent_structured_cell(selected, delta) else {
            return true;
        };
        self.structured_selected_cell = Some(next);
        self.structured_focus_handle.focus(window);
        cx.notify();
        true
    }

    /// Uses checked row-major offsets so stale selections and oversized indexes cannot wrap into another cell.
    fn adjacent_structured_cell(
        &self,
        current: StructuredCellEdit,
        delta: i32,
    ) -> Option<StructuredCellEdit> {
        let StructuredIndex::Delimited(index) = self.structured_index.as_ref()? else {
            return None;
        };
        let columns = u64::try_from(index.column_count().max(1)).ok()?;
        let slots = columns.checked_mul(index.record_count().checked_add(1)?)?;
        let column = u64::try_from(current.column).ok()?;
        if column >= columns {
            return None;
        }
        let position = match current.record {
            Some(record) if record < index.record_count() => record
                .checked_add(1)?
                .checked_mul(columns)?
                .checked_add(column)?,
            Some(_) => return None,
            None => column,
        };
        if position >= slots {
            return None;
        }
        let next = match delta.cmp(&0) {
            std::cmp::Ordering::Less => position.checked_sub(1).unwrap_or(slots - 1),
            std::cmp::Ordering::Greater if position + 1 == slots => 0,
            std::cmp::Ordering::Greater => position + 1,
            std::cmp::Ordering::Equal => position,
        };
        if next < columns {
            Some(StructuredCellEdit {
                record: None,
                column: usize::try_from(next).ok()?,
            })
        } else {
            Some(StructuredCellEdit {
                record: Some(next / columns - 1),
                column: usize::try_from(next % columns).ok()?,
            })
        }
    }

    fn structured_cell_value(&self, target: StructuredCellEdit) -> Option<String> {
        if let Some(value) = self.structured_cell_overrides.get(&target) {
            return Some(value.clone());
        }
        let StructuredIndex::Delimited(index) = self.structured_index.as_ref()? else {
            return None;
        };
        if let Some(record) = target.record {
            index
                .read_records(record, 1)
                .ok()?
                .pop()?
                .fields
                .get(target.column)
                .cloned()
                .or_else(|| Some(String::new()))
        } else {
            index.headers().get(target.column).cloned()
        }
    }

    /// 派生预览只复制用户实际选中的单元格；CSV 使用索引读取完整字段，
    /// 其余结构视图使用当前受限视口快照，避免一次复制触发无界文件扫描。
    pub(super) fn selected_structured_cell_text(&self) -> Option<String> {
        let target = self.structured_selected_cell?;
        if matches!(self.structured_index, Some(StructuredIndex::Delimited(_))) {
            return self.structured_cell_value(target);
        }
        if target.record.is_none() {
            return self
                .structured_index
                .as_ref()?
                .headers()
                .get(target.column)
                .cloned();
        }
        let record = target.record?;
        self.structured_rows
            .values()
            .find(|row| row.index == record)
            .or_else(|| self.json_rows.values().find(|row| row.index == record))
            .and_then(|row| {
                target
                    .column
                    .checked_sub(row.column_start)
                    .map(|index| (row, index))
            })
            .and_then(|(row, index)| row.cells.get(index))
            .cloned()
    }

    /// Commits and advances on cell Tab actions while consuming row commands that have no meaning inside one field.
    pub(in crate::document_host) fn on_structured_cell_host_action(
        &mut self,
        action: BlockHostAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            BlockHostAction::Submit(value) => {
                if self.defer_source_action_for_ime(
                    super::source_ime::DeferredSourceAction::HostInputSubmit {
                        input: self.structured_cell_input.clone(),
                    },
                    window,
                    cx,
                ) {
                    return;
                }
                let confirmed = self
                    .structured_cell_input
                    .read(cx)
                    .display_text()
                    .to_owned();
                let value = if confirmed.is_empty() && !value.is_empty() {
                    value.to_string()
                } else {
                    confirmed
                };
                if self.commit_structured_cell_edit(value, cx) {
                    self.structured_focus_handle.focus(window);
                }
            }
            BlockHostAction::LineOperation(operation) => {
                if self.structured_cell_input.read(cx).has_ime_composition() {
                    return;
                }
                let delta = match operation {
                    crate::components::LineOperation::Indent => 1,
                    crate::components::LineOperation::Outdent => -1,
                    _ => return,
                };
                let Some(current) = self.structured_cell_edit else {
                    return;
                };
                let Some(next) = self.adjacent_structured_cell(current, delta) else {
                    return;
                };
                let value = self
                    .structured_cell_input
                    .read(cx)
                    .display_text()
                    .to_owned();
                if self.commit_structured_cell_edit(value, cx) {
                    self.structured_selected_cell = Some(next);
                    self.structured_focus_handle.focus(window);
                    cx.notify();
                }
            }
            BlockHostAction::DismissTransientUi => {
                if self.structured_cell_input.read(cx).has_ime_composition() {
                    return;
                }
                self.structured_cell_edit = None;
                self.structured_focus_handle.focus(window);
                cx.notify();
            }
            _ => {}
        }
    }

    /// Retains the edit target until its source transaction succeeds, allowing callers to keep focus on failure.
    fn commit_structured_cell_edit(&mut self, value: String, cx: &mut Context<Self>) -> bool {
        let Some(target) = self.structured_cell_edit else {
            return false;
        };
        let Some(StructuredIndex::Delimited(index)) = self.structured_index.as_ref() else {
            return false;
        };
        let record = if let Some(record) = target.record {
            index
                .read_records(record, 1)
                .ok()
                .and_then(|mut rows| rows.pop())
        } else {
            index.read_header().ok().flatten()
        };
        let Some(mut record) = record else {
            return false;
        };
        let baseline_range = record.byte_range.clone();
        for (edited, override_value) in &self.structured_cell_overrides {
            if edited.record == target.record {
                record
                    .fields
                    .resize(index.column_count().max(edited.column + 1), String::new());
                record.fields[edited.column] = override_value.clone();
            }
        }
        record
            .fields
            .resize(index.column_count().max(target.column + 1), String::new());
        record.fields[target.column] = value.clone();
        let current_range = self.current_structured_record_range(&baseline_range);
        let Some(document) = self.document.as_ref() else {
            return false;
        };
        let Ok(current_bytes) = document.read_range(current_range.clone()) else {
            return false;
        };
        let terminator = delimited_record_terminator(&current_bytes);
        let replacement = serialize_delimited_record(&record.fields, index.delimiter(), terminator);
        if current_bytes == replacement.as_bytes() {
            self.structured_cell_edit = None;
            return true;
        }
        if !self.replace_delimited_table_source_range(baseline_range, &replacement, cx) {
            return false;
        }
        self.structured_cell_edit = None;
        self.structured_cell_overrides.insert(target, value);
        true
    }

    /// 结构索引中的区间属于本轮连续编辑开始前的基线。后台重建完成前只需累加
    /// 之前整条记录替换造成的偏移，即可继续安全编辑相邻行或同一行的其他列。
    pub(super) fn current_structured_record_range(&self, baseline: &Range<u64>) -> Range<u64> {
        let mut shift_before = 0i128;
        let mut shift_inside = 0i128;
        for (edited, delta) in &self.structured_cell_source_edits {
            if edited.end <= baseline.start {
                shift_before += i128::from(*delta);
            } else if edited == baseline {
                shift_inside += i128::from(*delta);
            }
        }
        let shift = |value: u64, delta: i128| {
            if delta >= 0 {
                value.saturating_add(u64::try_from(delta).unwrap_or(u64::MAX))
            } else {
                value.saturating_sub(u64::try_from(-delta).unwrap_or(u64::MAX))
            }
        };
        shift(baseline.start, shift_before)
            ..shift(baseline.end, shift_before.saturating_add(shift_inside))
    }

    pub(super) fn insert_delimited_row(&mut self, before: u64, cx: &mut Context<Self>) {
        let Some(StructuredIndex::Delimited(index)) = self.structured_index.as_ref() else {
            return;
        };
        let count = index.record_count();
        let before = before.min(count);
        let Some(document) = self.document.as_ref() else {
            return;
        };
        let fields = vec![String::new(); index.column_count().max(1)];
        if document.is_empty() && index.column_count() == 0 {
            let replacement = format!(
                "{}{}",
                serialize_delimited_record(
                    &[cx.global::<I18nManager>()
                        .strings()
                        .large_document_text("default_column_template")
                        .replace("{number}", "1")],
                    index.delimiter(),
                    "\n",
                ),
                serialize_delimited_record(&fields, index.delimiter(), "")
            );
            self.replace_delimited_table_source_range(0..0, &replacement, cx);
            return;
        }
        let (offset, prefix, terminator) = if before < count {
            let Some(row) = index
                .read_records(before, 1)
                .ok()
                .and_then(|mut rows| rows.pop())
            else {
                return;
            };
            let current_range = self.current_structured_record_range(&row.byte_range);
            let terminator = document
                .read_range(current_range)
                .ok()
                .map(|bytes| delimited_record_terminator(&bytes))
                .unwrap_or("\n");
            (row.byte_range.start, "", terminator)
        } else {
            let len = document.len();
            let trailing = (len > 0)
                .then(|| document.read_range(len.saturating_sub(2)..len).ok())
                .flatten()
                .unwrap_or_default();
            if trailing.ends_with(b"\n") || trailing.ends_with(b"\r") {
                (len, "", delimited_record_terminator(&trailing))
            } else if len > 0 {
                (len, "\n", "")
            } else {
                (0, "", "")
            }
        };
        let mut replacement = prefix.to_owned();
        replacement.push_str(&serialize_delimited_record(
            &fields,
            index.delimiter(),
            terminator,
        ));
        self.replace_delimited_table_source_range(offset..offset, &replacement, cx);
    }

    pub(super) fn delete_delimited_row(&mut self, record: u64, cx: &mut Context<Self>) {
        let Some(StructuredIndex::Delimited(index)) = self.structured_index.as_ref() else {
            return;
        };
        let Some(row) = index
            .read_records(record, 1)
            .ok()
            .and_then(|mut rows| rows.pop())
        else {
            return;
        };
        self.replace_delimited_table_source_range(row.byte_range, "", cx);
    }

    pub(super) fn transform_delimited_column(
        &mut self,
        edit: DelimitedEdit,
        cx: &mut Context<Self>,
    ) {
        let Some(document) = self.document.clone() else {
            return;
        };
        let DocumentFormat::Delimited { delimiter } = self.probe.format else {
            return;
        };
        if let Some(cancellation) = self.structured_cancellation.take() {
            cancellation.cancel();
        }
        self.structured_generation = self.structured_generation.wrapping_add(1);
        let generation = self.structured_generation;
        let base_revision = document.revision();
        let total = self
            .structured_index
            .as_ref()
            .map_or(0, |index| index.row_count().saturating_add(1));
        let progress = Arc::new(AtomicU64::new(0));
        self.structured_column_progress = Some((progress.clone(), total));
        let cancellation = SearchCancellation::default();
        self.structured_cancellation = Some(cancellation.clone());
        self.structure_error = Some(
            cx.global::<I18nManager>()
                .strings()
                .large_document_text("updating_columns")
                .into(),
        );
        self.structured_progress_task = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(100))
                    .await;
                let running = this
                    .update(cx, |view, cx| {
                        let running = view.structured_column_progress.is_some();
                        if running {
                            cx.notify();
                        }
                        running
                    })
                    .unwrap_or(false);
                if !running {
                    break;
                }
            }
        });
        self.structured_task = cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    transform_delimited_adapter(document, delimiter, edit, &cancellation, &progress)
                })
                .await;
            let _ = this.update(cx, |view, cx| {
                if view.structured_generation != generation
                    || view.document.as_ref().map(SharedDocument::revision) != Some(base_revision)
                    || view.coordinator.pending_external_change.is_some()
                {
                    view.structured_column_progress = None;
                    return;
                }
                view.structured_cancellation = None;
                view.structured_column_progress = None;
                match result {
                    Ok(replacement) => view.install_delimited_transformation(replacement, cx),
                    Err(PagedDocumentError::Cancelled) => {}
                    Err(error) => view.set_structure_error(error, cx),
                }
            });
        });
    }

    pub(super) fn cancel_delimited_column_transform(&mut self, cx: &mut Context<Self>) {
        if let Some(cancellation) = self.structured_cancellation.take() {
            cancellation.cancel();
        }
        self.structured_generation = self.structured_generation.wrapping_add(1);
        self.structured_column_progress = None;
        self.clear_structure_error();
        cx.notify();
    }

    /// Apply a completed CSV/TSV transformation once and hand its immutable
    /// result to the serial worker instead of writing recovery under UI state.
    fn install_delimited_transformation(&mut self, replacement: String, cx: &mut Context<Self>) {
        self.active_edit = None;
        self.structured_cell_edit = None;
        let Some(document) = self.document.clone() else {
            return;
        };
        let base_revision = document.revision();
        let old_len = document.len();
        if let Err(error) = document.replace_range(0..old_len, replacement.as_str()) {
            self.set_structure_error(error, cx);
            return;
        }
        self.enqueue_recovery_transaction(
            &document,
            base_revision,
            0..old_len,
            &replacement,
            Some(SourceSelection::collapsed(
                replacement.len() as u64,
                SourceAffinity::After,
            )),
            recovery_view_id(self.view_mode),
            cx,
        );
        self.tail_enabled = false;
        let preserve_live_table = matches!(
            self.view_mode,
            DocumentHostViewMode::Live | DocumentHostViewMode::Split
        ) && self.structured_index.is_some();
        if preserve_live_table {
            // 列变换已在新文档中完成；旧表格只负责撑住当前帧，直到新索引和可见行
            // 原子安装，期间不能退回 Source。
            self.structured_pending = None;
            self.structured_cell_overrides.clear();
            self.structured_cell_source_edits.clear();
            self.hidden_structured_columns.clear();
            self.structured_column_window_start = 0;
        } else {
            self.structured_index = None;
            self.invalidate_structured_runtime();
        }
        self.invalidate_source_rows();
        self.schedule_search(cx);
        self.schedule_delimited_snapshot_rebuild(cx);
        if preserve_live_table {
            self.clear_structure_error();
        }
        cx.emit(DocumentHostEvent::StateChanged);
        cx.notify();
    }
}
