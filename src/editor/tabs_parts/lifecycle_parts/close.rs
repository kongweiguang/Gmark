// @author kongweiguang

use super::*;
use std::time::Duration as StdDuration;

enum PreparedClosedTab {
    Snapshot(Box<DocumentTabSnapshot>),
    Host(Box<PreparedClosedHost>),
}

struct PreparedClosedHost {
    closed: ClosedTabSnapshot,
    open: crate::app::document_service::SharedDocumentHostOpen,
    dirty: bool,
    saved_file_fingerprint: Option<crate::recovery::FileFingerprint>,
}

impl Editor {
    pub(in crate::editor) fn tab_strip_height(&self) -> f32 {
        // Once the legacy shell has been promoted into a pane workspace, every
        // legacy tab has been migrated into the root leaf and each pane owns its
        // own Zed-style tab bar.
        if self.focus_mode || self.pane_workspace.is_some() {
            0.0
        } else {
            TAB_STRIP_HEIGHT
        }
    }

    pub(crate) fn push_closed_tab(
        &mut self,
        snapshot: DocumentTabSnapshot,
        cx: &mut Context<Self>,
    ) {
        let host_metadata = snapshot.document_host.as_ref().and_then(|host| {
            let host = host.read(cx);
            Some((host.document_id()?, host.probe().clone()))
        });
        if let Some(document_host) = snapshot.document_host.as_ref() {
            document_host.update(cx, |view, _cx| view.suspend_for_closed_tab());
        }
        let closed = match ClosedTabSnapshot::from_document_with_host(snapshot, host_metadata) {
            Ok(closed) => closed,
            Err(error) => {
                eprintln!("closed-tab snapshot capture failed: {error}");
                return;
            }
        };
        self.tabs.closed.push(closed);
        enforce_closed_tab_budget(
            &mut self.tabs.closed,
            CLOSED_TAB_LIMIT,
            CLOSED_TAB_RETAINED_BYTES_LIMIT,
        );
    }

    /// Prepares every filesystem-backed part of a closed tab outside GPUI.
    ///
    /// The result deliberately contains only immutable metadata, leases, and
    /// resident sessions.  Host entities are created later on the UI thread,
    /// because GPUI entities must never cross the background executor boundary.
    // 原因：重开在后台重新探测磁盘身份，关闭前的长度与编码可能已因保存而变化；仅保留原加载选项与安全 Source 意图。
    fn prepare_reopen_closed_tab(
        service: crate::app::document_service::DocumentService,
        closed: ClosedTabSnapshot,
    ) -> Result<PreparedClosedTab, String> {
        #[cfg(test)]
        {
            let delay_ms = super::REOPEN_TEST_DELAY_MS.load(std::sync::atomic::Ordering::Acquire);
            if delay_ms > 0 {
                std::thread::sleep(StdDuration::from_millis(delay_ms));
            }
        }
        let loading = gmark_document_core::LoadingPolicy::default();
        match closed.source.clone() {
            ClosedDocumentSource::File { path, .. } => {
                let probe = service
                    .probe_file(&path, loading, |normalized, policy| {
                        crate::document_io::probe_document_with_policy(normalized, policy)
                    })
                    .map_err(|error| error.to_string())?;
                if crate::document_io::is_markdown_path(&path)
                    && probe.strategy == gmark_paged_document::OpenStrategy::Resident
                {
                    let limits = loading.effective_limits();
                    let shared = service
                        .open_resident_file(&path, loading, |normalized, _| {
                            crate::document_io::read_resident_text_from_probe(
                                normalized, &probe, limits,
                            )
                            .map(|opened| {
                                crate::app::document_service::ResidentMarkdownSource::from_opened(
                                    normalized, opened,
                                )
                            })
                        })
                        .map_err(|error| error.to_string())?;
                    let encoding = shared.encoding.clone();
                    let session = EditorDocumentSession::from_lease(shared.lease)
                        .map_err(|error| error.to_string())?;
                    let dirty = session.try_is_dirty().unwrap_or(true);
                    let mut snapshot = closed.into_document_with_source(session, None);
                    snapshot.file_path = Some(path.clone());
                    snapshot.source_encoding = encoding;
                    snapshot.shared_document = true;
                    snapshot.document_dirty = dirty;
                    snapshot.saved_file_fingerprint = snapshot
                        .file_path
                        .as_deref()
                        .and_then(|path| crate::recovery::fingerprint_file(path).ok());
                    Ok(PreparedClosedTab::Snapshot(Box::new(snapshot)))
                } else {
                    let open = service
                        .open_document_host(
                            &path,
                            probe.clone(),
                            loading,
                            |normalized, probe, _| {
                                let source = gmark_paged_document::FileSource::open(normalized)
                                    .map_err(|error| {
                                        anyhow::anyhow!(
                                            "failed to open '{}': {error}",
                                            normalized.display()
                                        )
                                    })?;
                                gmark_paged_document::prepare_utf8_source(
                                    source,
                                    probe.encoding.clone(),
                                )
                                .map_err(|error| {
                                    anyhow::anyhow!("failed to prepare source: {error}")
                                })
                            },
                        )
                        .map_err(|error| error.to_string())?;
                    let dirty = open
                        .lease
                        .handle()
                        .lock()
                        .map(|controller| controller.session().dirty)
                        .unwrap_or(true);
                    let saved_file_fingerprint = crate::recovery::fingerprint_file(&path).ok();
                    Ok(PreparedClosedTab::Host(Box::new(PreparedClosedHost {
                        closed,
                        open,
                        dirty,
                        saved_file_fingerprint,
                    })))
                }
            }
            ClosedDocumentSource::Host { path, probe, .. } => {
                let options = probe.options;
                let force_safe_source = probe.force_safe_source;
                let mut probe = service
                    .probe_file(&path, loading, move |normalized, _policy| {
                        gmark_paged_document::probe_file(normalized, options)
                    })
                    .map_err(|error| error.to_string())?;
                probe.force_safe_source = force_safe_source;
                let open = service
                    .open_document_host(&path, probe, loading, |normalized, probe, _| {
                        let source = gmark_paged_document::FileSource::open(normalized).map_err(
                            |error| {
                                anyhow::anyhow!(
                                    "failed to open '{}': {error}",
                                    normalized.display()
                                )
                            },
                        )?;
                        gmark_paged_document::prepare_utf8_source(source, probe.encoding.clone())
                            .map_err(|error| anyhow::anyhow!("failed to prepare source: {error}"))
                    })
                    .map_err(|error| error.to_string())?;
                let dirty = open
                    .lease
                    .handle()
                    .lock()
                    .map(|controller| controller.session().dirty)
                    .unwrap_or(true);
                let saved_file_fingerprint = crate::recovery::fingerprint_file(&path).ok();
                Ok(PreparedClosedTab::Host(Box::new(PreparedClosedHost {
                    closed,
                    open,
                    dirty,
                    saved_file_fingerprint,
                })))
            }
            ClosedDocumentSource::Recovery {
                document_id,
                #[cfg(test)]
                journal_path,
            } => {
                let recovery_dir = {
                    #[cfg(test)]
                    if let Some(path) = journal_path.as_deref() {
                        path.parent().map(Path::to_path_buf)
                    } else {
                        None
                    }
                    #[cfg(not(test))]
                    {
                        None
                    }
                }
                .or_else(|| {
                    crate::config::AppDirs::from_system()
                        .ok()
                        .map(|dirs| dirs.recovery_dir())
                })
                .ok_or_else(|| "recovery directory is unavailable".to_owned())?;
                let recovered = crate::recovery::load_recovery_documents(&recovery_dir)
                    .map_err(|error| error.to_string())?
                    .into_iter()
                    .find(|document| {
                        uuid::Uuid::parse_str(&document.document_id)
                            .ok()
                            .map(gmark_document_runtime::DocumentId::from_uuid)
                            == Some(document_id)
                    })
                    .ok_or_else(|| "closed recovery journal is unavailable".to_owned())?;
                let source = crate::app::document_service::ResidentMarkdownSource::from_recovered(
                    recovered.source.clone(),
                    recovered.file_path.clone(),
                    recovered.source_format.clone(),
                )
                .map_err(|error| error.to_string())?;
                let shared = service
                    .open_recovery(document_id, source)
                    .map_err(|error| error.to_string())?;
                let session = EditorDocumentSession::from_lease(shared.lease)
                    .map_err(|error| error.to_string())?;
                let mut snapshot = closed.into_document_with_source(session, None);
                snapshot.file_path = recovered.file_path.clone();
                snapshot.source_encoding = shared.encoding;
                snapshot.shared_document = true;
                snapshot.document_dirty = true;
                snapshot.recovered_session = true;
                snapshot.recovery_journal = Some(Arc::new(Mutex::new(
                    crate::recovery::RecoveryJournal::resume(&recovered),
                )));
                Ok(PreparedClosedTab::Snapshot(Box::new(snapshot)))
            }
            ClosedDocumentSource::Image { path } => Ok(PreparedClosedTab::Snapshot(Box::new(
                Self::snapshot_for_image_preview(path),
            ))),
            ClosedDocumentSource::Error { path, reason } => Ok(PreparedClosedTab::Snapshot(
                Box::new(Self::snapshot_for_file_open_failure(path, reason)),
            )),
        }
    }

    /// Materializes a host entity only after background preparation succeeds.
    ///
    /// Keeping this small UI-side step separate prevents a late or failed
    /// worker from touching the current editor state before its identity gate.
    // 原因：GPUI Entity 只能在 Context 中创建，但创建必须晚于所有文件/恢复 I/O，避免重开按钮同步卡住。
    fn snapshot_from_prepared_reopen(
        prepared: PreparedClosedTab,
        cx: &mut Context<Self>,
    ) -> Result<DocumentTabSnapshot, String> {
        match prepared {
            PreparedClosedTab::Snapshot(snapshot) => Ok(*snapshot),
            PreparedClosedTab::Host(prepared) => {
                let PreparedClosedHost {
                    closed,
                    open,
                    dirty,
                    saved_file_fingerprint,
                } = *prepared;
                let view_id = gmark_document_core::DocumentViewInstanceId::new();
                let crate::app::document_service::SharedDocumentHostOpen {
                    lease,
                    probe,
                    file_path,
                    encoding,
                    ..
                } = open;
                let handle = lease.handle();
                let host_path = file_path.clone();
                let host = cx.new(move |cx| {
                    crate::document_host::DocumentHost::from_shared_with_view_id_or_error(
                        host_path,
                        probe,
                        handle,
                        lease,
                        view_id,
                        crate::document_host::DocumentHostViewPresentation::default(),
                        cx,
                    )
                });
                let mut snapshot =
                    closed.into_document_with_source(EditorDocumentSession::shell(), Some(host));
                snapshot.file_path = Some(file_path);
                snapshot.source_encoding = encoding;
                snapshot.shared_document = true;
                snapshot.document_dirty = dirty;
                snapshot.saved_file_fingerprint = saved_file_fingerprint;
                Ok(snapshot)
            }
        }
    }

    /// Restores a pending closed-tab entry at its original history position.
    ///
    /// Reopen preparation is allowed to outlive other close actions; inserting
    /// at the captured position keeps newer closed tabs ahead of a failed retry.
    // 原因：失败、取消和身份失配都必须保留用户的最近关闭历史，不能因后台结果迟到而吞掉条目。
    fn restore_pending_reopen(&mut self) {
        let Some((index, closed)) = self.tabs.reopen_pending.take() else {
            return;
        };
        let insert_at = index.min(self.tabs.closed.len());
        self.tabs.closed.insert(insert_at, closed);
    }

    /// 关闭等原候选和分页确认文字发布后再读取 dirty；待恢复文字不能随标签销毁。
    pub(in crate::editor) fn request_close_tab_index(
        &mut self,
        index: usize,
        cx: &mut Context<Self>,
    ) {
        if self.has_active_ime_composition(cx) || self.has_pending_source_input_for_tab(index, cx) {
            if let Some(tab) = self.tabs.records.get(index).map(|tab| tab.id) {
                self.queue_ime_operation(
                    crate::editor::ime_lifecycle::DeferredImeOperation::CloseTab(tab),
                    cx,
                );
            }
            return;
        }
        if index >= self.tabs.records.len() {
            return;
        }
        self.close_menu_bar(cx);
        let (dirty, lease_count) = if index == self.tabs.active {
            (self.is_document_dirty(), self.source_document.lease_count())
        } else {
            self.tabs.records[index]
                .snapshot
                .as_ref()
                .map(|snapshot| {
                    let dirty = if snapshot.document_host.is_some() {
                        snapshot.document_dirty
                    } else {
                        snapshot.source_document.is_dirty()
                    };
                    (dirty, snapshot.source_document.lease_count())
                })
                .unwrap_or((false, 0))
        };
        if dirty && lease_count <= 1 {
            if index != self.tabs.active && !self.switch_to_tab_index(index, cx) {
                return;
            }
            self.tabs.show_close_dialog = true;
            cx.notify();
            return;
        }
        self.close_tab_index_without_prompt(index, true, cx);
    }

    /// 活动标签关闭后旧输入句柄已失效，清除恢复目标；关闭非活动标签则保留当前编辑焦点。
    pub(super) fn close_tab_index_without_prompt(
        &mut self,
        index: usize,
        keep_for_restore: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        if index >= self.tabs.records.len() {
            return false;
        }
        if self.tabs.records.len() == 1 {
            self.checkpoint_recovery_journal();
            let closed_id = self.tabs.records[0].id;
            self.release_render_assets_for_active_document(cx);
            let closed = self.capture_active_tab(cx);
            let _ = self.view_state.close_tab(closed_id);
            if keep_for_restore {
                self.push_closed_tab(closed, cx);
            }
            self.tabs.records[0] = TabRecord {
                id: uuid::Uuid::new_v4(),
                pinned: false,
                snapshot: None,
            };
            self.close_dialog_restore_input_focus = None;
            self.replace_document_from_markdown(String::new(), None, cx);
            self.schedule_workspace_session_save(cx);
            return true;
        }

        if index != self.tabs.active {
            let record = self.tabs.records.remove(index);
            let _ = self.view_state.close_tab(record.id);
            if index < self.tabs.active {
                self.tabs.active -= 1;
            }
            if keep_for_restore && let Some(snapshot) = record.snapshot {
                self.push_closed_tab(snapshot, cx);
            }
            self.schedule_workspace_session_save(cx);
            cx.notify();
            return true;
        }

        self.checkpoint_recovery_journal();
        let closed_id = self.tabs.records[index].id;
        self.release_render_assets_for_active_document(cx);
        let closed = self.capture_active_tab(cx);
        let _ = self.view_state.close_tab(closed_id);
        self.tabs.records.remove(index);
        self.tabs.active = index.min(self.tabs.records.len() - 1);
        let target = self.tabs.records[self.tabs.active]
            .snapshot
            .take()
            .expect("inactive tab must own a snapshot");
        if keep_for_restore {
            self.push_closed_tab(closed, cx);
        }
        self.close_dialog_restore_input_focus = None;
        self.install_tab_snapshot(target, cx);
        self.schedule_workspace_session_save(cx);
        true
    }

    pub(in crate::editor) fn close_tabs_affected_by_deleted_path(
        &mut self,
        target: &Path,
        cx: &mut Context<Self>,
    ) -> bool {
        let (mut indices, has_dirty) = self.workspace_tabs_affected_by_path(target);
        if has_dirty {
            return false;
        }
        indices.sort_unstable_by(|left, right| right.cmp(left));
        for index in indices {
            self.close_tab_index_without_prompt(index, false, cx);
        }
        true
    }

    pub(crate) fn on_close_tab_action(
        &mut self,
        _: &crate::components::CloseTab,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.request_close_tab_index(self.tabs.active, cx);
    }

    pub(crate) fn on_new_tab_action(
        &mut self,
        _: &crate::components::NewTab,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.new_untitled_tab(cx);
    }

    /// Schedules a closed-tab restore without giving a late worker ownership
    /// of the active tab's current state.
    // 原因：重开结果必须先通过 tab/document 身份门禁，再捕获当前快照并安装新标签。
    pub(crate) fn on_reopen_closed_tab_action(
        &mut self,
        _: &crate::components::ReopenClosedTab,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.tabs.reopen_task.is_some() {
            return;
        }
        let Some(service) = cx
            .try_global::<crate::app::document_service::DocumentService>()
            .cloned()
        else {
            eprintln!("closed-tab restore failed: document service is not initialized");
            return;
        };
        let Some(closed) = self.tabs.closed.pop() else {
            return;
        };
        // 原因：磁盘文件可能只是短暂不可用，失败后应允许用户重试；没有日志的恢复项已经永久失去正文，继续塞回历史只会让重开命令反复失败。
        let restore_after_prepare_failure = matches!(
            &closed.source,
            ClosedDocumentSource::File { .. } | ClosedDocumentSource::Host { .. }
        );
        let pending_index = self.tabs.closed.len();
        self.tabs.reopen_pending = Some((pending_index, closed.clone()));
        self.tabs.reopen_generation = self.tabs.reopen_generation.wrapping_add(1);
        let generation = self.tabs.reopen_generation;
        let expected_tab_id = self.tabs.active_id();
        let expected_document_epoch = self.document_epoch;
        self.tabs.reopen_task = Some(cx.spawn(async move |this: WeakEntity<Self>, cx| {
            let prepared = cx
                .background_spawn(async move { Self::prepare_reopen_closed_tab(service, closed) })
                .await;
            let _ = this.update(cx, |editor, cx| {
                if editor.tabs.reopen_generation != generation {
                    return;
                }
                editor.tabs.reopen_task = None;
                let target_is_current = editor.tabs.active_id() == expected_tab_id
                    && editor.document_epoch == expected_document_epoch;
                if !target_is_current || !editor.can_switch_tabs() {
                    editor.restore_pending_reopen();
                    cx.notify();
                    return;
                }
                let prepared = match prepared {
                    Ok(prepared) => prepared,
                    Err(error) => {
                        if restore_after_prepare_failure {
                            editor.restore_pending_reopen();
                        } else {
                            let _ = editor.tabs.reopen_pending.take();
                        }
                        eprintln!("closed-tab restore failed: {error}");
                        cx.notify();
                        return;
                    }
                };
                let snapshot = match Self::snapshot_from_prepared_reopen(prepared, cx) {
                    Ok(snapshot) => snapshot,
                    Err(error) => {
                        editor.restore_pending_reopen();
                        eprintln!("closed-tab host restore failed: {error}");
                        cx.notify();
                        return;
                    }
                };
                let _ = editor.tabs.reopen_pending.take();
                let current = editor.capture_active_tab(cx);
                editor.tabs.records[editor.tabs.active].snapshot = Some(current);
                editor.tabs.records.push(TabRecord {
                    id: uuid::Uuid::new_v4(),
                    pinned: false,
                    snapshot: None,
                });
                editor.tabs.active = editor.tabs.records.len() - 1;
                editor.install_tab_snapshot(snapshot, cx);
                editor.schedule_workspace_session_save(cx);
            });
        }));
    }

    pub(crate) fn on_previous_tab_action(
        &mut self,
        _: &crate::components::PreviousTab,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.tabs.records.len() < 2 {
            return;
        }
        let target = if self.tabs.active == 0 {
            self.tabs.records.len() - 1
        } else {
            self.tabs.active - 1
        };
        self.dismiss_contextual_overlays(cx);
        self.switch_to_tab_index(target, cx);
    }

    pub(crate) fn on_next_tab_action(
        &mut self,
        _: &crate::components::NextTab,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.tabs.records.len() < 2 {
            return;
        }
        let target = (self.tabs.active + 1) % self.tabs.records.len();
        self.dismiss_contextual_overlays(cx);
        self.switch_to_tab_index(target, cx);
    }

    /// 鼠标与键盘取消立即恢复提示前的输入目标，避免取消后的同帧按键落到已关闭按钮。
    pub(in crate::editor) fn on_cancel_tab_close(
        &mut self,
        _: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cancel_tab_close_dialog(window, cx);
    }

    /// 取消同步清理关闭意图并恢复当前仍挂载的输入面；成功关闭时则由卸载路径丢弃旧焦点。
    pub(in crate::editor) fn cancel_tab_close_dialog(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.tabs.show_close_dialog = false;
        self.tabs.close_after_save = false;
        self.tabs.close_others_keep = None;
        self.invalidate_pane_close_save(cx);
        self.pane_close_target = None;
        if let Some(focus) = self.close_dialog_restore_input_focus.take() {
            focus.focus(window);
        }
        cx.stop_propagation();
        cx.notify();
    }

    /// Discards only the requested pane lease, cancelling any save observer so
    /// an earlier Save action cannot close the discarded replacement tab.
    pub(in crate::editor) fn on_discard_tab_close(
        &mut self,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.tabs.show_close_dialog = false;
        self.tabs.close_after_save = false;
        self.invalidate_pane_close_save(cx);
        if let Some((pane, tab)) = self.pane_close_target.take() {
            let Some(workspace) = self.pane_workspace.clone() else {
                self.pane_close_target = Some((pane, tab));
                self.tabs.show_close_dialog = true;
                cx.stop_propagation();
                cx.notify();
                return;
            };
            // Keep the target canvas mounted while discarding.  This preserves
            // the exact input surface on failure and lets the child Editor or
            // DocumentHost clear its recovery journal before the tab is
            // detached and dropped.
            let mounted_markdown = self.pane_canvas_entities.borrow().get(&pane).and_then(
                |(_, _, canvas)| match canvas {
                    crate::editor::panes::PaneCanvasEntity::Markdown(entity) => {
                        Some(entity.clone())
                    }
                    crate::editor::panes::PaneCanvasEntity::DocumentHost(_)
                    | crate::editor::panes::PaneCanvasEntity::ReadOnly(_) => None,
                },
            );
            let mounted_host = self.pane_canvas_entities.borrow().get(&pane).and_then(
                |(_, _, canvas)| match canvas {
                    crate::editor::panes::PaneCanvasEntity::DocumentHost(entity) => {
                        Some(entity.clone())
                    }
                    crate::editor::panes::PaneCanvasEntity::Markdown(_)
                    | crate::editor::panes::PaneCanvasEntity::ReadOnly(_) => None,
                },
            );
            let discarded = if let Some(canvas) = mounted_markdown {
                let child = canvas.read(cx).editor();
                let mut discarded = false;
                child.update(cx, |editor, _cx| {
                    let Ok(handle) = editor.source_document.handle() else {
                        return;
                    };
                    match handle.discard_current_changes_for_owned_leases(1) {
                        Ok(_) => {
                            editor.document_dirty = false;
                            discarded = true;
                        }
                        Err(error) => {
                            eprintln!("failed to discard pane markdown changes: {error}");
                        }
                    }
                });
                if discarded {
                    canvas.update(cx, |canvas, cx| canvas.checkpoint_discarded_recovery(cx));
                }
                discarded
            } else if let Some(canvas) = mounted_host {
                let host = canvas.read(cx).host();
                let mut discarded = false;
                host.update(cx, |host, cx| {
                    discarded = host.discard_unsaved_changes_for_owned_leases(1, cx);
                });
                discarded
            } else {
                workspace
                    .read(cx)
                    .workspace()
                    .tab(pane, tab)
                    .and_then(|tab| match tab.view().kind() {
                        crate::editor::panes::PaneDocumentKind::Markdown => tab
                            .view()
                            .lease()
                            .and_then(|lease| lease.handle().discard_current_changes().ok()),
                        crate::editor::panes::PaneDocumentKind::DocumentHost => tab
                            .view()
                            .host_handle()
                            .and_then(|handle| handle.discard_current_changes().ok()),
                        crate::editor::panes::PaneDocumentKind::Image
                        | crate::editor::panes::PaneDocumentKind::Error => Some(true),
                    })
                    .unwrap_or(false)
            };
            if discarded {
                self.pane_close_target = Some((pane, tab));
                if !self.close_pane_tab_now(&workspace, pane, tab, cx) {
                    self.tabs.show_close_dialog = true;
                    cx.notify();
                }
            } else {
                self.pane_close_target = Some((pane, tab));
                self.tabs.show_close_dialog = true;
                cx.stop_propagation();
                cx.notify();
            }
            cx.stop_propagation();
            return;
        }
        if self.discard_current_document_changes(cx)
            && self.close_tab_index_without_prompt(self.tabs.active, false, cx)
        {
            self.advance_close_other_tabs(cx);
        }
        cx.stop_propagation();
    }

    pub(in crate::editor) fn on_save_tab_close(
        &mut self,
        _: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.tabs.show_close_dialog = false;
        if let Some((pane, tab)) = self.pane_close_target {
            self.start_pane_tab_save(pane, tab, window, cx);
            cx.stop_propagation();
            return;
        }
        self.tabs.close_after_save = true;
        self.tabs.continue_window_close_after_save = false;
        self.save_document(window, cx);
        cx.stop_propagation();
    }

    /// Reports the Markdown child's terminal save result through the parent
    /// one-shot instead of requiring a timer to inspect mutable state.
    pub(in crate::editor) fn finish_pending_tab_close_after_save(
        &mut self,
        cx: &mut Context<Self>,
    ) {
        if self.pane_close_save_signal.is_some() {
            self.signal_pane_close_save(u8::from(!self.is_document_dirty()));
            return;
        }
        if self.tabs.close_after_save && !self.is_document_dirty() {
            self.tabs.close_after_save = false;
            if self.close_tab_index_without_prompt(self.tabs.active, true, cx) {
                self.advance_close_other_tabs(cx);
            }
        }
    }

    /// Reports save failure/cancellation once while retaining the pane tab and
    /// its existing error surface for the user to inspect or retry.
    pub(in crate::editor) fn abort_pending_tab_close_after_save(&mut self, cx: &mut Context<Self>) {
        if self.pane_close_save_signal.is_some() {
            self.signal_pane_close_save(2);
        }
        if self.tabs.close_after_save {
            self.tabs.close_after_save = false;
            self.tabs.close_others_keep = None;
            cx.notify();
        }
    }
}
