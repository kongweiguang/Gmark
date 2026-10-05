// @author kongweiguang

//! Source window virtualization and scroll handling.

use super::*;

impl DocumentHost {
    pub(super) fn on_horizontal_container_scroll_wheel(
        &mut self,
        event: &ScrollWheelEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let delta = event.delta.pixel_delta(px(26.0));
        let Some(axis) = modifier_horizontal_wheel_delta(
            event.modifiers.shift,
            event.modifiers.control,
            f32::from(delta.x),
            f32::from(delta.y),
        ) else {
            // 原生横向 delta 由 overflow 自身处理；普通纵向滚轮继续传给内层列表。
            return;
        };
        let max_x = self
            .structured_horizontal_scroll_handle
            .max_offset()
            .width
            .max(px(0.0));
        let mut offset = self.structured_horizontal_scroll_handle.offset();
        offset.x = (offset.x + px(axis)).min(px(0.0)).max(-max_x);
        self.structured_horizontal_scroll_handle.set_offset(offset);
        cx.stop_propagation();
        cx.notify();
    }

    pub(super) fn max_source_window_start(&self) -> u64 {
        let indexed = self.index.as_ref().map_or(0, LineIndex::max_line_bytes);
        let selected = self
            .selected_lines
            .as_ref()
            .and_then(|selection| selection.start.try_into().ok())
            .and_then(|line| self.document.as_ref()?.line_range(line))
            .map_or(0, |range| range.end.saturating_sub(range.start));
        indexed
            .max(selected)
            .saturating_sub(MAX_RENDERED_LINE_BYTES)
    }

    pub(super) fn set_source_window_start(&mut self, start: u64, cx: &mut Context<Self>) {
        let next = start.min(self.max_source_window_start());
        if self.source_window_start == next {
            return;
        }
        // 编辑按键已即时提交到 PieceDocument；横向离开当前块时只释放有界输入窗口。
        self.active_edit = None;
        self.source_window_start = next;
        self.invalidate_source_rows();
        cx.emit(DocumentHostEvent::StateChanged);
        cx.notify();
    }

    /// 只在后台读取 viewport 与 overscan。发布时同时校验 generation、横向窗口和文档快照，
    /// 因而快速滚动或编辑后的旧磁盘结果永远不能覆盖当前画面。
    pub(super) fn request_source_rows(&mut self, visible: Range<usize>, cx: &mut Context<Self>) {
        if self.document.is_none() {
            self.provisional_anchor = Some(SourceAnchor::new(
                ((self.probe.len as u128 * visible.start as u128)
                    / self.probe.estimated_lines.max(1) as u128) as u64,
                SourceAffinity::Before,
            ));
        }
        let reader = if let Some(document) = self.document.clone() {
            SourceViewportReader::Indexed(Box::new(document))
        } else if let Some(source) = self.provisional_source.clone() {
            SourceViewportReader::Provisional {
                source,
                estimated_lines: self.probe.estimated_lines.max(1),
                encoding: self.probe.encoding.clone(),
            }
        } else {
            return;
        };
        let total = self.line_count();
        let cache_epoch = self.source_cache_epoch;
        let scrolling_forward = self
            .source_last_visible
            .as_ref()
            .is_none_or(|previous| visible.start >= previous.start);
        self.source_last_visible = Some(visible.clone());
        let (overscan_before, overscan_after) = if scrolling_forward {
            (SOURCE_OVERSCAN_ROWS, SOURCE_OVERSCAN_ROWS.saturating_mul(2))
        } else {
            (SOURCE_OVERSCAN_ROWS.saturating_mul(2), SOURCE_OVERSCAN_ROWS)
        };
        let requested = visible.start.saturating_sub(overscan_before)
            ..visible.end.saturating_add(overscan_after).min(total);
        let requested_is_cached = requested.is_empty()
            || requested
                .clone()
                .all(|line| self.source_row_epochs.get(&line) == Some(&cache_epoch));
        if let Some(pending) = self.source_pending.as_ref() {
            let disjoint = pending.end <= requested.start || requested.end <= pending.start;
            if requested_is_cached && disjoint && !self.source_cancel_in_flight {
                if let Some(cancellation) = self.coordinator.source_cancellation.take() {
                    cancellation.cancel();
                }
                // 最新 viewport 已由当前 cache 完整满足：推进 generation 令后台结果过期，
                // 并在 UI 侧立即结束 task 所有权，避免无意义 completion/defer 自调度。
                self.coordinator.source_generation =
                    self.coordinator.source_generation.wrapping_add(1);
                self.source_pending = None;
                self.source_queued_visible = None;
                self.source_cancel_in_flight = false;
                self.coordinator.source_task = Task::ready(());
                self.metrics.viewport_cancellations =
                    self.metrics.viewport_cancellations.saturating_add(1);
                self.emit_viewport_cancellation_trace();
                return;
            }
            // 保留最新可见范围。当前任务发布后立即补读，不依赖下一次 render 回调；
            // 远跳与当前读取完全不相交时立即取消磁盘循环，连续滚动的重叠请求则让
            // 当前任务完成，避免滚轮小步移动造成取消风暴。
            self.source_queued_visible = Some(visible);
            if !self.source_cancel_in_flight
                && disjoint
                && let Some(cancellation) = self.coordinator.source_cancellation.as_ref()
            {
                cancellation.cancel();
                self.source_cancel_in_flight = true;
                self.metrics.viewport_cancellations =
                    self.metrics.viewport_cancellations.saturating_add(1);
                self.emit_viewport_cancellation_trace();
            }
            return;
        }
        if requested_is_cached {
            self.source_cancel_in_flight = false;
            return;
        }
        self.coordinator.source_generation = self.coordinator.source_generation.wrapping_add(1);
        self.metrics.viewport_requests = self.metrics.viewport_requests.saturating_add(1);
        let generation = self.coordinator.source_generation;
        let task_stamp = DocumentTaskStamp::capture(self, generation);
        let window_start = self.source_window_start;
        let syntax_language = crate::components::code_language_for_path(&self.path);
        let requested_center = requested.start.saturating_add(requested.len() / 2);
        let viewport_request = ViewportRequest::bounded(
            requested.start as u64,
            requested.len(),
            0,
            window_start,
            generation,
        );
        self.source_pending = Some(requested.clone());
        let cancellation = SearchCancellation::default();
        self.coordinator.source_cancellation = Some(cancellation.clone());
        let installed_range = requested.clone();
        self.coordinator.source_task = cx.spawn(async move |this, cx| {
            let rows = cx
                .background_spawn(async move {
                    let rows = match reader {
                        SourceViewportReader::Indexed(document) => document
                            .read_viewport_cancellable(&viewport_request, &cancellation)
                            .map(|snapshot| {
                                snapshot
                                    .lines
                                    .into_iter()
                                    .filter_map(|line| {
                                        Some((
                                            usize::try_from(line.line).ok()?,
                                            BoundedLineWindow::from_viewport(line),
                                        ))
                                    })
                                    .collect::<Vec<_>>()
                            }),
                        SourceViewportReader::Provisional {
                            source,
                            estimated_lines,
                            encoding,
                        } => read_provisional_source_rows(
                            &source,
                            estimated_lines,
                            requested,
                            window_start,
                            &encoding,
                            &cancellation,
                        ),
                    }?;
                    let syntax_contexts = syntax_language.map_or_else(BTreeMap::new, |language| {
                        crate::components::build_source_syntax_contexts(
                            Some(language),
                            rows.iter().map(|(line, row)| (*line, row.text.as_ref())),
                        )
                    });
                    Ok::<_, PagedDocumentError>((rows, syntax_contexts))
                })
                .await;
            let _ = this.update(cx, |view, cx| {
                if !task_stamp.accepts_strict(view, view.coordinator.source_generation)
                    || view.source_cache_epoch != cache_epoch
                    || view.source_window_start != window_start
                {
                    view.metrics.stale_viewport_results =
                        view.metrics.stale_viewport_results.saturating_add(1);
                    return;
                }
                view.source_pending = None;
                view.coordinator.source_cancellation = None;
                match rows {
                    Ok((rows, syntax_contexts)) => {
                        view.source_cancel_in_flight = false;
                        view.source_syntax_contexts.extend(syntax_contexts);
                        for (line, row) in rows {
                            let row_changed = view
                                .source_rows
                                .get(&line)
                                .is_none_or(|previous| !previous.has_same_surface_text(&row));
                            let row_is_pinned =
                                view.active_edit
                                    .as_ref()
                                    .is_some_and(|active| active.line == line)
                                    || view.source_row_blocks.get(&line).is_some_and(|block| {
                                        view.is_pending_source_ime_owner(block)
                                    });
                            if row_changed && !row_is_pinned {
                                view.source_row_blocks.remove(&line);
                            }
                            view.source_rows.insert(line, Arc::new(row));
                            view.source_row_epochs.insert(line, cache_epoch);
                        }
                        if !view.soak_ready_published && !view.source_rows.is_empty() {
                            view.publish_soak_ready_marker();
                            view.soak_ready_published = true;
                        }
                        // 保留相邻帧的重叠 viewport，避免程序化跳转时新旧范围互相驱逐；
                        // 超预算后只淘汰离当前请求最远的端点，缓存仍与文件大小解耦。
                        while view.source_rows.len() > MAX_SOURCE_CACHED_ROWS {
                            let first = view.source_rows.first_key_value().map(|(line, _)| *line);
                            let last = view.source_rows.last_key_value().map(|(line, _)| *line);
                            let evicted = match (first, last) {
                                (Some(first), Some(last))
                                    if requested_center.saturating_sub(first)
                                        >= last.saturating_sub(requested_center) =>
                                {
                                    first
                                }
                                (_, Some(last)) => last,
                                _ => break,
                            };
                            view.source_rows.remove(&evicted);
                            view.source_row_epochs.remove(&evicted);
                            view.source_syntax_contexts.remove(&evicted);
                            let row_is_pinned =
                                view.active_edit
                                    .as_ref()
                                    .is_some_and(|active| active.line == evicted)
                                    || view.source_row_blocks.get(&evicted).is_some_and(|block| {
                                        view.is_pending_source_ime_owner(block)
                                    });
                            if !row_is_pinned {
                                view.source_row_blocks.remove(&evicted);
                            }
                        }
                        let next_rows = view
                            .source_rows
                            .iter()
                            .filter(|(line, _)| {
                                view.source_row_epochs.get(line) == Some(&cache_epoch)
                            })
                            .map(|(line, row)| (*line, row.clone()))
                            .collect::<BTreeMap<_, _>>();
                        if !next_rows.is_empty() {
                            // 只有完整后台结果和 cache 元数据都就绪后才交换快照。
                            // pending 期间继续显示上一份 ScreenLines，正文不会退回空白帧。
                            let visible = view
                                .source_last_visible
                                .clone()
                                .unwrap_or_else(|| installed_range.clone());
                            let document_revision =
                                view.document.as_ref().map_or(0, SharedDocument::revision);
                            view.displayed_screen_lines = Arc::new(ScreenLines {
                                document_revision,
                                generation,
                                cache_epoch,
                                column_window_start: window_start,
                                visible,
                                rows: Arc::new(next_rows),
                            });
                            view.metrics.viewport_installs =
                                view.metrics.viewport_installs.saturating_add(1);
                            let cached_rows = view.displayed_screen_lines.rows.len();
                            if cached_rows > view.metrics.max_cached_rows {
                                view.metrics.max_cached_rows = cached_rows;
                                let profile = view.probe.profile();
                                let plan =
                                    session_plan(&profile, &view.probe, view.probe.strategy, false);
                                crate::perf::emit_document_value(
                                    "document_source_cache_peak_rows",
                                    cached_rows as u64,
                                    &profile.format,
                                    &plan,
                                );
                            }
                        }
                        if let Some(top_anchor) = view.displayed_screen_lines.top_source_anchor() {
                            document_view_state_mut(&mut view.document, &mut view.tab_view_state)
                                .source
                                .top_byte_anchor = top_anchor;
                            document_view_state_mut(&mut view.document, &mut view.tab_view_state)
                                .source
                                .line_offset_y = 0.0;
                            // provisional 逻辑行只是估算坐标。每次真实行窗口安装后保存其
                            // source byte anchor，全文索引收敛时才能回到相同正文而非 byte 0。
                            if view.document.is_none() {
                                view.provisional_anchor = Some(top_anchor);
                            }
                        }
                    }
                    Err(PagedDocumentError::Cancelled) => {}
                    Err(error) => {
                        view.source_cancel_in_flight = false;
                        view.error = Some(
                            cx.global::<I18nManager>()
                                .strings()
                                .large_document_error(&error)
                                .into(),
                        );
                    }
                }
                let queued = view.source_queued_visible.take();
                view.schedule_source_ime_replay(cx);
                cx.notify();
                if let Some(visible) = queued {
                    // 不在即将完成的 source_task 内覆盖并 drop 自己。TestApp 会让已取消
                    // 的后台读取立即就绪，直接递归启动下一任务会形成忙循环；defer 同时
                    // 保证生产 executor 的任务所有权边界清晰。
                    let this = cx.entity().downgrade();
                    cx.defer(move |cx| {
                        let _ = this.update(cx, |view, cx| view.request_source_rows(visible, cx));
                    });
                }
            });
        });
    }

    pub(super) fn invalidate_source_rows(&mut self) {
        self.coordinator.source_generation = self.coordinator.source_generation.wrapping_add(1);
        if let Some(cancellation) = self.coordinator.source_cancellation.take() {
            cancellation.cancel();
        }
        self.source_cancel_in_flight = false;
        self.source_cache_epoch = self.source_cache_epoch.wrapping_add(1);
        self.source_syntax_contexts.clear();
        self.source_pending = None;
        self.source_queued_visible = None;
        self.coordinator.source_task = Task::ready(());
    }

    /// 生产 soak 不能把“进程还活着”误判成“大文件已打开”。只有首个 Source
    /// viewport 真正安装后才发布 marker；普通运行没有该环境变量，不产生任何 I/O。
    pub(super) fn publish_soak_ready_marker(&self) {
        let Some(marker) = std::env::var_os("GMARK_SOAK_READY_PATH").map(PathBuf::from) else {
            return;
        };
        let payload = serde_json::json!({
            "schema_version": 1,
            "process_id": std::process::id(),
            "path": self.path,
            "file_len": self.probe.len,
            "line_count": self.line_count(),
            "visible_rows": self.source_rows.len(),
            "mode": "source",
        });
        let result = (|| -> std::io::Result<()> {
            if let Some(parent) = marker.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let temporary = marker.with_extension(format!("tmp-{}", std::process::id()));
            let bytes = serde_json::to_vec_pretty(&payload).map_err(std::io::Error::other)?;
            std::fs::write(&temporary, bytes)?;
            std::fs::rename(&temporary, &marker)
        })();
        if let Err(error) = result {
            eprintln!(
                "failed to publish Paged Source soak readiness '{}': {error}",
                marker.display()
            );
        }
    }

    pub(super) fn anchor_source_window_for_byte(&mut self, line: u64, byte_offset: u64) {
        let Some(range) = self
            .document
            .as_ref()
            .and_then(|document| document.line_range(line))
        else {
            if self.source_window_start != 0 {
                self.source_window_start = 0;
                self.invalidate_source_rows();
            }
            return;
        };
        let relative = byte_offset
            .clamp(range.start, range.end)
            .saturating_sub(range.start);
        let next = source_window_start_for_anchor(range.end.saturating_sub(range.start), relative);
        if self.source_window_start != next {
            self.source_window_start = next;
            self.invalidate_source_rows();
        }
    }

    pub(super) fn on_source_scroll_wheel(
        &mut self,
        event: &ScrollWheelEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let delta = event.delta.pixel_delta(px(26.0));
        let horizontal =
            event.modifiers.shift || f32::from(delta.x).abs() > f32::from(delta.y).abs();
        if !horizontal {
            // 局部 uniform_list 内部保持原生滚动；到达窗口边界后，把下一次滚轮
            // 映射回全局行并重建局部 origin，文件大小不再进入 f32 像素坐标。
            let handle = self.scroll_handle.0.borrow().base_handle.clone();
            let row_height = self.source_row_height.max(1.0);
            let local_top = (-f32::from(handle.offset().y) / row_height)
                .max(0.0)
                .floor() as usize;
            let visible_rows = (f32::from(handle.bounds().size.height) / row_height)
                .ceil()
                .max(1.0) as usize;
            let axis = f32::from(delta.y);
            let step = (axis.abs() / row_height).ceil().max(1.0) as usize;
            let at_start = local_top == 0 && self.source_list_origin > 0;
            let at_end = local_top.saturating_add(visible_rows) >= self.source_list_len()
                && self
                    .source_list_origin
                    .saturating_add(self.source_list_len())
                    < self.line_count();
            if axis > 0.0 && at_start {
                let target = self.source_list_origin.saturating_sub(step);
                self.source_list_origin = source_list_origin_for_target(self.line_count(), target);
                self.scroll_source_line_strict(target, ScrollStrategy::Top);
                cx.notify();
            } else if axis < 0.0 && at_end {
                let target = self
                    .source_list_origin
                    .saturating_add(local_top)
                    .saturating_add(step)
                    .min(self.line_count().saturating_sub(1));
                self.source_list_origin = source_list_origin_for_target(self.line_count(), target);
                self.scroll_source_line_strict(target, ScrollStrategy::Top);
                cx.notify();
            }
            cx.stop_propagation();
            return;
        }
        let axis = if event.modifiers.shift {
            f32::from(delta.y)
        } else {
            f32::from(delta.x)
        };
        let byte_delta = (-axis * SOURCE_SCROLL_BYTES_PER_PIXEL).round() as i64;
        let next = shift_source_window_start(
            self.source_window_start,
            byte_delta,
            self.max_source_window_start(),
        );
        self.set_source_window_start(next, cx);
        cx.stop_propagation();
    }
}
