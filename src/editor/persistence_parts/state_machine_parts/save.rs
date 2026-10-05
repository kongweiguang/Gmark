// @author kongweiguang

// 把快照写入与既有路径保存流程放在同一边界，保持保存状态机的原子写语义。

use super::*;

const EXISTING_SAVE_FAILURE_NOTICE: &str = "保存失败，修改已保留；按 Ctrl+S 重试";
const EXISTING_SAVE_UNCERTAIN_NOTICE: &str = "保存结果待确认，修改仍保留；检查文件后再重试";

/// 构造保存竞态下的磁盘对比快照；二次身份校验失败仍是可恢复冲突，不能降级成
/// 只有“确定”按钮的通用错误，否则用户既无法比较版本，也可能在重试时误覆盖新文件。
fn existing_save_conflict_preview(path: &Path, local: &str) -> ExternalConflictPreview {
    let (disk, disk_bytes, disk_error) = match std::fs::read(path) {
        Ok(bytes) => (
            String::from_utf8_lossy(&bytes).into_owned(),
            bytes.len(),
            None,
        ),
        Err(error) => (String::new(), 0, Some(error.to_string())),
    };
    build_external_conflict_preview(path, local, &disk, disk_bytes, disk_error)
}

/// 在后台写入不可变快照，并把预检与原子替换之间发生的 SourceChanged 统一映射
/// 到冲突恢复流程；只有可能已经跨过替换边界的 Persist 才属于不确定失败。
fn write_existing_snapshot(
    snapshot: gmark_document_runtime::DocumentSaveSnapshot,
    source_format: gmark_document::SourceFormatSnapshot,
    path: &std::path::Path,
    overwrite: bool,
    expected_fingerprint: Option<crate::recovery::FileFingerprint>,
) -> ExistingSaveOutcome {
    let revision = gmark_document::Revision::from_u64(snapshot.revision.0);
    let source = snapshot
        .resident_baseline
        .as_ref()
        .map(gmark_document::DocumentSnapshot::text)
        .or_else(|| {
            snapshot
                .read_all()
                .ok()
                .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        })
        .unwrap_or_default();
    let saved_format = snapshot.source_format.clone().unwrap_or(source_format);

    if !overwrite
        && let Some(expected) = expected_fingerprint
        && crate::recovery::fingerprint_file(path)
            .map(|current| current != expected)
            .unwrap_or(true)
    {
        return ExistingSaveOutcome::Conflict {
            revision,
            preview: existing_save_conflict_preview(path, &source),
        };
    }

    let write_started = super::perf::start();
    let byte_len = snapshot.len();
    let cancellation = gmark_paged_document::SearchCancellation::default();
    // An explicit overwrite confirmation intentionally discards the old
    // fingerprint precondition. The same immutable snapshot writer is used,
    // but Save As mode does not reject the target's changed identity.
    let result = if overwrite {
        snapshot.save_as_atomic_cancellable(path, &cancellation)
    } else {
        snapshot.save_atomic_cancellable(path, &cancellation)
    };
    if let Some(started) = write_started {
        let detail = result.as_ref().err().map(ToString::to_string);
        super::perf::emit(
            "save_atomic_write",
            started,
            Some(usize::try_from(byte_len).unwrap_or(usize::MAX)),
            Some(result.is_ok()),
            detail.as_deref(),
        );
    }
    match result {
        Ok(identity) => ExistingSaveOutcome::Saved {
            source,
            source_format: saved_format,
            revision,
            identity,
        },
        Err(gmark_paged_document::PagedDocumentError::SourceChanged) => {
            ExistingSaveOutcome::Conflict {
                revision,
                preview: existing_save_conflict_preview(path, &source),
            }
        }
        Err(error) => ExistingSaveOutcome::Failed {
            revision,
            detail: error.to_string(),
            target_may_have_changed: error.target_may_have_changed(),
        },
    }
}

impl Editor {
    /// 详细原因只进入诊断；只有仍指向原文档时才展示短提示，避免后台回调污染新文档。
    fn present_existing_save_failure(
        &mut self,
        expected_document_epoch: u64,
        detail: &str,
        target_may_have_changed: bool,
        cx: &mut Context<Self>,
    ) {
        eprintln!("已有文档保存失败: {detail}");
        if self.document_epoch != expected_document_epoch {
            return;
        }
        let notice = if target_may_have_changed {
            EXISTING_SAVE_UNCERTAIN_NOTICE
        } else {
            EXISTING_SAVE_FAILURE_NOTICE
        };
        self.show_pane_notice(notice, cx);
    }

    /// 只清理保存状态自己的短提示；其他操作新发出的底栏提示必须继续显示。
    fn clear_existing_save_failure_notice(
        &mut self,
        expected_document_epoch: u64,
        cx: &mut Context<Self>,
    ) {
        if self.document_epoch != expected_document_epoch {
            return;
        }
        let is_save_notice = self.pane_notice.as_ref().is_some_and(|notice| {
            notice.as_ref() == EXISTING_SAVE_FAILURE_NOTICE
                || notice.as_ref() == EXISTING_SAVE_UNCERTAIN_NOTICE
        });
        if is_save_notice {
            self.pane_notice_task = None;
            self.pane_notice = None;
            cx.notify();
        }
    }

    /// 原子替换后持久化同步失败时，磁盘可能已经是新内容，但不能宣称保存成功。
    /// 刷新 fingerprint 允许用户重试，同时保留 dirty 与恢复 journal。
    pub(super) fn apply_uncertain_save_baseline(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let path_changed = self.file_path.as_ref() != Some(&path);
        self.saved_file_fingerprint = crate::recovery::fingerprint_file(&path).ok();
        self.external_file_conflict = false;
        self.allow_external_overwrite_once = false;
        self.document_kind = DocumentKind::from_path(&path);
        self.file_path = Some(path);
        if path_changed {
            self.restart_file_watcher(cx);
        }
        self.document_dirty = true;
        self.pending_window_edited = true;
        self.pending_window_title_refresh = true;
        self.schedule_recovery_journal(cx);
        self.schedule_auto_save(cx);
        self.sync_workspace_after_document_path_change(cx);
        cx.notify();
    }

    /// 后台保存以文档代际绑定反馈，失败不夺焦点，冲突仍进入专用比较恢复流程。
    pub(super) fn save_existing_path_in_background(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let dispatch_started = super::perf::start();
        let request_document_epoch = self.document_epoch;
        if self.save_task.is_some() {
            self.save_queued = true;
            return;
        }

        let overwrite = std::mem::take(&mut self.allow_external_overwrite_once);
        if self.external_file_conflict && !overwrite {
            self.present_external_file_conflict(&path, window, cx);
            return;
        }
        let expected_fingerprint = self.saved_file_fingerprint.clone();
        let should_close_after_save = self.pending_close_after_save;
        let conflict_restore_focus = if self.external_conflict_restore_focus.is_none()
            && self.close_dialog_restore_focus.is_none()
        {
            self.document.focused_block_entity_id(window, cx)
        } else {
            None
        };
        let (save_snapshot, source_format) = match self.prepare_background_save(cx) {
            Ok(Some(snapshot)) => snapshot,
            Ok(None) => {
                self.save_queued = true;
                return;
            }
            Err(detail) => {
                self.abort_pending_tab_close_after_save(cx);
                self.abort_window_close_tab_sequence(cx);
                if should_close_after_save {
                    self.abort_pending_close_after_save(cx);
                }
                self.present_existing_save_failure(
                    request_document_epoch,
                    &format!("保存快照准备失败: {detail}"),
                    false,
                    cx,
                );
                return;
            }
        };
        let Some(snapshot) = save_snapshot.resident_baseline.clone() else {
            self.abort_pending_tab_close_after_save(cx);
            self.abort_window_close_tab_sequence(cx);
            if should_close_after_save {
                self.abort_pending_close_after_save(cx);
            }
            self.present_existing_save_failure(
                request_document_epoch,
                "保存快照缺少 Resident 基线",
                false,
                cx,
            );
            return;
        };
        let document_epoch = request_document_epoch;
        let byte_len = snapshot.len();
        let window_handle = window.window_handle();
        let worker_path = path.clone();

        self.save_task = Some(cx.spawn(async move |this: WeakEntity<Self>, cx| {
            let mut next = Some((
                save_snapshot,
                source_format,
                overwrite,
                expected_fingerprint,
            ));
            let mut saved_current_revision = false;
            let mut conflict = false;
            while let Some((snapshot, source_format, overwrite, expected_fingerprint)) = next.take()
            {
                let path_for_write = worker_path.clone();
                let outcome = cx
                    .background_spawn(async move {
                        write_existing_snapshot(
                            snapshot,
                            source_format,
                            &path_for_write,
                            overwrite,
                            expected_fingerprint,
                        )
                    })
                    .await;

                let continuation = this.update(cx, |editor, cx| match outcome {
                    ExistingSaveOutcome::Saved {
                        source,
                        source_format,
                        revision,
                        identity,
                    } => {
                        let promoted = match editor
                            .source_document
                            .try_save_succeeded(revision, identity)
                        {
                            Ok(promoted) => {
                                saved_current_revision = editor.apply_background_save_success(
                                    path.clone(),
                                    source,
                                    source_format,
                                    revision,
                                    document_epoch,
                                    cx,
                                );
                                editor.clear_existing_save_failure_notice(document_epoch, cx);
                                promoted.and_then(|snapshot| {
                                    let format = snapshot.source_format.clone()?;
                                    Some((snapshot, format))
                                })
                            }
                            Err(completion_error) => {
                                editor.present_existing_save_failure(
                                    document_epoch,
                                    &format!("保存完成提交失败: {completion_error}"),
                                    true,
                                    cx,
                                );
                                editor.abort_pending_tab_close_after_save(cx);
                                editor.abort_window_close_tab_sequence(cx);
                                if should_close_after_save {
                                    editor.abort_pending_close_after_save(cx);
                                }
                                None
                            }
                        };
                        if should_close_after_save && !saved_current_revision {
                            editor.abort_pending_close_after_save(cx);
                        }
                        promoted
                    }
                    ExistingSaveOutcome::Conflict { revision, preview } => {
                        if let Err(completion_error) = editor.source_document.try_save_failed(
                            revision,
                            gmark_document_runtime::SaveFailureCode::Conflict,
                        ) {
                            eprintln!("冲突保存状态提交失败: {completion_error}");
                        }
                        editor.external_file_conflict = true;
                        editor.external_conflict_preview = Some(preview);
                        if editor.external_conflict_restore_focus.is_none() {
                            editor.external_conflict_restore_focus = conflict_restore_focus;
                        }
                        editor.show_external_conflict_dialog = true;
                        editor.close_menu_bar(cx);
                        editor.hide_info_dialog(cx);
                        conflict = true;
                        if should_close_after_save {
                            editor.pending_close_after_save = true;
                        }
                        cx.notify();
                        None
                    }
                    ExistingSaveOutcome::Failed {
                        revision,
                        detail,
                        target_may_have_changed,
                    } => {
                        if let Err(completion_error) = editor.source_document.try_save_failed(
                            revision,
                            gmark_document_runtime::SaveFailureCode::Other,
                        ) {
                            eprintln!("保存失败状态提交失败: {completion_error}");
                        }
                        editor.abort_pending_tab_close_after_save(cx);
                        editor.abort_window_close_tab_sequence(cx);
                        if should_close_after_save {
                            editor.abort_pending_close_after_save(cx);
                        }
                        if target_may_have_changed && editor.document_epoch == document_epoch {
                            editor.apply_uncertain_save_baseline(path.clone(), cx);
                        }
                        editor.present_existing_save_failure(
                            document_epoch,
                            &detail,
                            target_may_have_changed,
                            cx,
                        );
                        None
                    }
                });
                next = match continuation {
                    Ok(Some((snapshot, format))) => Some((snapshot, format, true, None)),
                    Ok(None) | Err(_) => None,
                };
                if next.is_none() {
                    let _ = this.update(cx, |editor, cx| {
                        editor.save_task = None;
                        if std::mem::take(&mut editor.save_queued) && editor.is_document_dirty() {
                            editor.pending_save = true;
                            cx.notify();
                        }
                    });
                }
            }

            let _ = cx.update_window(
                window_handle,
                move |_view: AnyView, window: &mut Window, cx: &mut App| {
                    if saved_current_revision {
                        window.set_window_edited(false);
                        if should_close_after_save {
                            window.remove_window();
                            cx.defer(crate::app_menu::continue_pending_quit);
                        }
                    } else if conflict {
                        window.blur();
                    }
                },
            );
        }));

        if let Some(started) = dispatch_started {
            super::perf::emit(
                "save_background_dispatch",
                started,
                Some(byte_len),
                Some(true),
                None,
            );
        }
    }

    /// 同步保存用于测试与显式阻塞入口；失败反馈与后台共用，SourceChanged 仍进入专用冲突恢复流程。
    pub(in crate::editor) fn save_to_existing_path(
        &mut self,
        path: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let document_epoch = self.document_epoch;
        let overwrite = self.allow_external_overwrite_once;
        if self.existing_path_has_external_change(path) {
            self.present_external_file_conflict(path, window, cx);
            return false;
        }
        let (save_snapshot, source_format) = match self.prepare_background_save(cx) {
            Ok(Some(snapshot)) => snapshot,
            Ok(None) => {
                self.save_queued = true;
                return false;
            }
            Err(error) => {
                self.present_existing_save_failure(
                    document_epoch,
                    &format!("保存快照准备失败: {error}"),
                    false,
                    cx,
                );
                return false;
            }
        };
        let markdown = save_snapshot
            .resident_baseline
            .as_ref()
            .map(gmark_document::DocumentSnapshot::text)
            .or_else(|| {
                save_snapshot
                    .read_all()
                    .ok()
                    .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            })
            .unwrap_or_default();
        let saved_format = save_snapshot
            .source_format
            .clone()
            .unwrap_or_else(|| source_format.clone());
        let saved_revision = gmark_document::Revision::from_u64(save_snapshot.revision.0);
        let byte_len = save_snapshot.len();
        let write_started = super::perf::start();
        let cancellation = gmark_paged_document::SearchCancellation::default();
        let result = if overwrite {
            save_snapshot.save_as_atomic_cancellable(path, &cancellation)
        } else {
            save_snapshot.save_atomic_cancellable(path, &cancellation)
        };
        if let Some(started) = write_started {
            let detail = result.as_ref().err().map(ToString::to_string);
            super::perf::emit(
                "save_atomic_write",
                started,
                Some(usize::try_from(byte_len).unwrap_or(usize::MAX)),
                Some(result.is_ok()),
                detail.as_deref(),
            );
        }
        match result {
            Ok(identity) => {
                if let Err(error) = self
                    .source_document
                    .try_save_succeeded(saved_revision, identity)
                {
                    self.present_existing_save_failure(
                        document_epoch,
                        &format!("保存完成提交失败: {error}"),
                        true,
                        cx,
                    );
                    return false;
                }
                self.apply_successful_save(path.to_path_buf(), markdown, saved_format, cx);
                self.clear_existing_save_failure_notice(document_epoch, cx);
                window.set_window_edited(false);
                true
            }
            Err(err) => {
                if matches!(
                    &err,
                    gmark_paged_document::PagedDocumentError::SourceChanged
                ) {
                    let _ = self.source_document.try_save_failed(
                        saved_revision,
                        gmark_document_runtime::SaveFailureCode::Conflict,
                    );
                    self.external_file_conflict = true;
                    self.present_external_file_conflict(path, window, cx);
                    return false;
                }
                let target_may_have_changed = err.target_may_have_changed();
                let code = if target_may_have_changed {
                    gmark_document_runtime::SaveFailureCode::Uncertain
                } else {
                    gmark_document_runtime::SaveFailureCode::Other
                };
                let _ = self.source_document.try_save_failed(saved_revision, code);
                if target_may_have_changed {
                    self.apply_uncertain_save_baseline(path.to_path_buf(), cx);
                }
                let detail = err.to_string();
                self.present_existing_save_failure(
                    document_epoch,
                    &detail,
                    target_may_have_changed,
                    cx,
                );
                false
            }
        }
    }
}
