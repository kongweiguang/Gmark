// @author kongweiguang

// 把保存对话框及大文档分支隔离，维持现有用户触发与收尾顺序。

use super::*;

impl Editor {
    /// 保存对话框和后台写入组成一个目标绑定任务，取消也释放标签导航门禁。
    pub(super) fn save_document_via_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.save_prompt_task.is_some() {
            return;
        }
        let (save_snapshot, source_format) = match self.prepare_background_save(cx) {
            Ok(Some(snapshot)) => snapshot,
            Ok(None) => {
                self.save_queued = true;
                return;
            }
            Err(error) => {
                eprintln!("保存快照准备失败: {error}");
                self.abort_pending_resource_insertion();
                self.abort_pending_tab_close_after_save(cx);
                self.abort_window_close_tab_sequence(cx);
                if self.pending_close_after_save {
                    self.abort_pending_close_after_save(cx);
                }
                return;
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
        let saved_document_epoch = self.document_epoch;
        let (default_dir, suggested_name) = self.save_dialog_defaults();
        let document_kind = self.document_kind;
        let prompt = cx.prompt_for_new_path(&default_dir, suggested_name.as_deref());
        let weak_editor = cx.entity().downgrade();
        let weak_editor_for_cancel = weak_editor.clone();
        let weak_editor_for_error = weak_editor.clone();
        let window_handle = window.window_handle();
        let should_close_after_save = self.pending_close_after_save;

        let cleanup = cx.entity().downgrade();
        self.save_prompt_task = Some(cx.spawn(
            async move |_this: WeakEntity<Self>, cx: &mut AsyncApp| {
                async {
                    let mut path = match prompt.await {
                        Ok(Ok(Some(path))) => path,
                        Ok(Ok(None)) | Err(_) => {
                            let _ = weak_editor_for_cancel.update(cx, |this, cx| {
                                this.abort_pending_resource_insertion();
                                this.abort_pending_tab_close_after_save(cx);
                                this.abort_window_close_tab_sequence(cx);
                                if should_close_after_save {
                                    this.abort_pending_close_after_save(cx);
                                }
                            });
                            return;
                        }
                        Ok(Err(err)) => {
                            let _ = weak_editor_for_error.update(cx, |this, cx| {
                                this.abort_pending_resource_insertion();
                                this.abort_pending_tab_close_after_save(cx);
                                this.abort_window_close_tab_sequence(cx);
                                if should_close_after_save {
                                    this.abort_pending_close_after_save(cx);
                                }
                            });
                            let detail = err.to_string();
                            let _ = cx.update_window(
                                window_handle,
                                move |_view: AnyView, window: &mut Window, cx: &mut App| {
                                    let strings = cx.global::<I18nManager>().strings().clone();
                                    let buttons = [strings.info_dialog_ok.as_str()];
                                    let _ = window.prompt(
                                        PromptLevel::Critical,
                                        &strings.save_failed_title,
                                        Some(&detail),
                                        &buttons,
                                        cx,
                                    );
                                },
                            );
                            return;
                        }
                    };

                    document_kind.apply_default_extension(&mut path);

                    let reservation = match weak_editor
                        .update(cx, |this, cx| this.reserve_save_as_target(&path, cx))
                    {
                        Ok(Ok(
                            crate::app::document_service::SaveAsTargetReservation::Reserved(
                                reservation,
                            ),
                        )) => reservation,
                        Ok(Ok(
                            crate::app::document_service::SaveAsTargetReservation::Occupied(target),
                        )) => {
                            let prompt = cx.update_window(
                                window_handle,
                                move |_view: AnyView, window: &mut Window, cx: &mut App| {
                                    let strings = cx.global::<I18nManager>().strings().clone();
                                    let buttons = [
                                        strings.menu_open_file.as_str(),
                                        strings.external_change_cancel.as_str(),
                                    ];
                                    window.prompt(
                                        PromptLevel::Warning,
                                        &strings.save_failed_title,
                                        Some("目标文档已在其他视图打开；是否切换到现有文档？"),
                                        &buttons,
                                        cx,
                                    )
                                },
                            );
                            let choice = match prompt {
                                Ok(receiver) => receiver.await.unwrap_or(1),
                                Err(_) => 1,
                            };
                            if choice == 0 {
                                let _ = weak_editor.update(cx, |this, cx| {
                                    let _ = this.switch_to_shared_save_as_target(target, cx);
                                    this.abort_pending_resource_insertion();
                                    this.abort_pending_tab_close_after_save(cx);
                                    this.abort_window_close_tab_sequence(cx);
                                    if should_close_after_save {
                                        this.abort_pending_close_after_save(cx);
                                    }
                                });
                            }
                            return;
                        }
                        Ok(Err(detail)) => {
                            let _ = weak_editor.update(cx, |this, cx| {
                                this.abort_pending_resource_insertion();
                                this.abort_pending_tab_close_after_save(cx);
                                this.abort_window_close_tab_sequence(cx);
                                if should_close_after_save {
                                    this.abort_pending_close_after_save(cx);
                                }
                            });
                            let _ = cx.update_window(
                                window_handle,
                                move |_view: AnyView, window: &mut Window, cx: &mut App| {
                                    let strings = cx.global::<I18nManager>().strings().clone();
                                    let buttons = [strings.info_dialog_ok.as_str()];
                                    let _ = window.prompt(
                                        PromptLevel::Critical,
                                        &strings.save_failed_title,
                                        Some(&detail),
                                        &buttons,
                                        cx,
                                    );
                                },
                            );
                            return;
                        }
                        Err(_) => {
                            let detail =
                                "Save As target reservation failed because the editor closed";
                            let _ = cx.update_window(
                                window_handle,
                                move |_view: AnyView, window: &mut Window, cx: &mut App| {
                                    let strings = cx.global::<I18nManager>().strings().clone();
                                    let buttons = [strings.info_dialog_ok.as_str()];
                                    let _ = window.prompt(
                                        PromptLevel::Critical,
                                        &strings.save_failed_title,
                                        Some(detail),
                                        &buttons,
                                        cx,
                                    );
                                },
                            );
                            return;
                        }
                    };

                    let result = cx
                .background_spawn(async move {
                    let byte_len = save_snapshot.len();
                    let write_started = super::perf::start();
                    let cancellation = gmark_paged_document::SearchCancellation::default();
                    let write_result =
                        save_snapshot.save_as_atomic_cancellable(&path, &cancellation);
                    if let Some(started) = write_started {
                        let detail = write_result.as_ref().err().map(ToString::to_string);
                        super::perf::emit(
                            "save_atomic_write",
                            started,
                            Some(usize::try_from(byte_len).unwrap_or(usize::MAX)),
                            Some(write_result.is_ok()),
                            detail.as_deref(),
                        );
                    }
                    match write_result {
                        Ok(identity) => Ok((path, markdown, saved_format, identity)),
                        Err(error) => {
                            let target_may_have_changed = matches!(
                                &error,
                                gmark_paged_document::PagedDocumentError::SourceChanged
                                    | gmark_paged_document::PagedDocumentError::Persist { .. }
                            );
                            Err((path, error.to_string(), target_may_have_changed))
                        }
                    }
                })
                .await;

                    let (path, source, source_format, identity) = match result {
                        Ok(saved) => saved,
                        Err((failed_path, detail, target_may_have_changed)) => {
                            let _ = weak_editor.update(cx, |this, cx| {
                                let failure_code = if target_may_have_changed {
                                    gmark_document_runtime::SaveFailureCode::Uncertain
                                } else {
                                    gmark_document_runtime::SaveFailureCode::Other
                                };
                                let _ = this
                                    .source_document
                                    .try_save_failed(saved_revision, failure_code);
                                this.abort_pending_resource_insertion();
                                this.abort_pending_tab_close_after_save(cx);
                                this.abort_window_close_tab_sequence(cx);
                                if should_close_after_save {
                                    this.abort_pending_close_after_save(cx);
                                }
                                if target_may_have_changed
                                    && this.document_epoch == saved_document_epoch
                                {
                                    this.apply_uncertain_save_baseline(failed_path, cx);
                                }
                            });
                            let _ = cx.update_window(
                                window_handle,
                                move |_view: AnyView, window: &mut Window, cx: &mut App| {
                                    let strings = cx.global::<I18nManager>().strings().clone();
                                    let buttons = [strings.info_dialog_ok.as_str()];
                                    let _ = window.prompt(
                                        PromptLevel::Critical,
                                        &strings.save_failed_title,
                                        Some(&detail),
                                        &buttons,
                                        cx,
                                    );
                                },
                            );
                            return;
                        }
                    };
                    let mut reservation = Some(reservation);
                    let saved_current_revision = weak_editor
                        .update(cx, move |this, cx| {
                            let Some(reservation) = reservation.take() else {
                                eprintln!("Save As reservation was already consumed");
                                return false;
                            };
                            let reservation = match reservation.commit() {
                                Ok(_) => true,
                                Err(error) => {
                                    eprintln!("Save As target commit failed: {error}");
                                    false
                                }
                            };
                            let saved_current = if reservation {
                                match this
                                    .source_document
                                    .try_save_succeeded(saved_revision, identity)
                                {
                                    Ok(_) => this.apply_background_save_success(
                                        path,
                                        source,
                                        source_format,
                                        saved_revision,
                                        saved_document_epoch,
                                        cx,
                                    ),
                                    Err(error) => {
                                        eprintln!("保存完成提交失败: {error}");
                                        false
                                    }
                                }
                            } else {
                                false
                            };
                            if !saved_current {
                                this.abort_pending_resource_insertion();
                                this.abort_pending_tab_close_after_save(cx);
                                this.abort_window_close_tab_sequence(cx);
                            }
                            if should_close_after_save && !saved_current {
                                this.abort_pending_close_after_save(cx);
                            }
                            saved_current
                        })
                        .unwrap_or(false);
                    let _ = cx.update_window(
                        window_handle,
                        move |_view: AnyView, window: &mut Window, cx: &mut App| {
                            if saved_current_revision {
                                window.set_window_edited(false);
                            }
                            if should_close_after_save && saved_current_revision {
                                window.remove_window();
                                cx.defer(crate::app_menu::continue_pending_quit);
                            }
                        },
                    );
                }
                .await;
                let _ = cleanup.update(cx, |editor, cx| {
                    editor.save_prompt_task = None;
                    cx.notify();
                });
            },
        ));
    }

    /// 所有保存调用共用候选终态门禁，内部直接调用也不能提前抓取快照。
    pub(crate) fn save_document(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.defer_action_for_ime(&crate::components::SaveDocument, window, cx) {
            return;
        }
        if let Some(document_host) = self.document_host.clone() {
            if self.file_path.is_none() {
                self.save_large_document_via_prompt(window, cx);
            } else {
                document_host.update(cx, |host, cx| {
                    host.on_save_document(&crate::components::SaveDocument, window, cx)
                });
            }
            return;
        }
        if !self.source_encoding.is_utf8() {
            self.request_encoding_conversion(cx);
            return;
        }
        if let Some(path) = self.file_path.clone() {
            self.save_existing_path_in_background(path, window, cx);
            return;
        }

        self.save_document_via_prompt(window, cx);
    }

    /// 文件身份切换必须晚于组合输入收尾，避免迟到文本落在新文件。
    pub(crate) fn save_document_as(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.defer_action_for_ime(&crate::components::SaveDocumentAs, window, cx) {
            return;
        }
        if self.document_host.is_some() {
            self.save_large_document_via_prompt(window, cx);
            return;
        }
        if !self.source_encoding.is_utf8() {
            self.request_encoding_conversion(cx);
            return;
        }
        self.save_document_via_prompt(window, cx);
    }

    /// Host 路径窗口也固定原标签，完成或取消后才允许延迟导航继续。
    pub(super) fn save_large_document_via_prompt(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.save_prompt_task.is_some() {
            return;
        }
        let Some(document_host) = self.document_host.clone() else {
            return;
        };
        let (default_dir, suggested_name) = self.save_dialog_defaults();
        let document_kind = self.document_kind;
        let prompt = cx.prompt_for_new_path(&default_dir, suggested_name.as_deref());
        let window_handle = window.window_handle();
        let weak_editor = cx.entity().downgrade();
        let cleanup = cx.entity().downgrade();
        self.save_prompt_task = Some(cx.spawn(
            async move |_this: WeakEntity<Self>, cx: &mut AsyncApp| {
                async {
                    let mut path = match prompt.await {
                        Ok(Ok(Some(path))) => path,
                        Ok(Ok(None)) | Err(_) => {
                            let _ = weak_editor.update(cx, |editor, cx| {
                                editor.abort_pending_resource_insertion();
                                editor.abort_pending_tab_close_after_save(cx);
                                editor.abort_window_close_tab_sequence(cx);
                                editor.abort_pending_close_after_save(cx);
                            });
                            return;
                        }
                        Ok(Err(error)) => {
                            let _ = weak_editor.update(cx, |editor, _cx| {
                                editor.abort_pending_resource_insertion();
                            });
                            let detail = error.to_string();
                            let _ = cx.update_window(
                                window_handle,
                                move |_view: AnyView, window: &mut Window, cx: &mut App| {
                                    let strings = cx.global::<I18nManager>().strings().clone();
                                    let buttons = [strings.info_dialog_ok.as_str()];
                                    let _ = window.prompt(
                                        PromptLevel::Critical,
                                        &strings.save_failed_title,
                                        Some(&detail),
                                        &buttons,
                                        cx,
                                    );
                                },
                            );
                            return;
                        }
                    };
                    document_kind.apply_default_extension(&mut path);
                    let _ = document_host.update(cx, move |view, cx| {
                        view.save_as_path(path, window_handle, cx);
                    });
                }
                .await;
                let _ = cleanup.update(cx, |editor, cx| {
                    editor.save_prompt_task = None;
                    cx.notify();
                });
            },
        ));
    }
}
