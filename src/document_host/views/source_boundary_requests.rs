// @author kongweiguang

//! 将超长源码的边界查找移出绘制线程，并将结果绑定到发起时的真实选区。

use super::source_boundaries::resolve_word_range;
use super::source_ime::DeferredSourceAction;
use super::source_pointer::source_word_drag_selection;
use super::*;

type BoundaryCommit = Box<dyn FnOnce(&mut DocumentHost, &mut Window, &mut Context<DocumentHost>)>;

/// 边界已算出但原输入法仍有候选时，只保留所属窗口的待安装结果。
pub(super) struct SourceBoundaryCompletion {
    window: gpui::AnyWindowHandle,
    commit: BoundaryCommit,
}

impl DocumentHost {
    /// 导航、回放与失败恢复都仍拥有用户输入；生命周期不能只等输入法候选而提前拆卸视图。
    pub(crate) fn has_pending_source_input(&self) -> bool {
        self.coordinator.source_boundary_cancellation.is_some()
            || self.coordinator.source_boundary_completion.is_some()
            || !self.coordinator.source_boundary_actions.is_empty()
            || self.coordinator.source_boundary_recovery_text.is_some()
    }

    /// 恢复复制必须能越过生命周期等待，否则用户无法释放关闭门禁；正文命令继续等待。
    pub(crate) fn has_source_boundary_recovery(&self) -> bool {
        self.coordinator.source_boundary_recovery_text.is_some()
            && self.coordinator.source_boundary_cancellation.is_none()
            && self.coordinator.source_boundary_actions.is_empty()
    }

    /// 编辑器生命周期回归以可控异步边界制造待提交状态；原生入队行为另由 Host 按键回归覆盖。
    #[cfg(test)]
    pub(crate) fn hold_source_boundary_with_confirmed_text_for_test(
        &mut self,
        release: futures::channel::oneshot::Receiver<()>,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = self
            .document
            .as_ref()
            .map_or(0, |document| document.source_selection().head.byte_offset);
        self.request_source_boundary(
            move |_snapshot, _cancel| async move {
                release.await.map_err(|_| PagedDocumentError::Cancelled)?;
                Ok(target)
            },
            |host, target, window, cx| {
                host.set_source_selection(
                    SourceSelection::collapsed(target, SourceAffinity::After),
                    cx,
                );
                host.restore_source_navigation_input(window, cx);
            },
            window,
            cx,
        );
        self.queue_source_boundary_text(text, None, UndoCaptureKind::CoalescibleText);
    }

    /// 保留文字的复制重试先于行内 Copy；独立查找字段仍保持自己的剪贴板行为。
    pub(super) fn on_source_boundary_copy_capture(
        &mut self,
        _: &Copy,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.source_text_surface_has_focus(window, cx) && self.copy_source_boundary_recovery(cx)
        {
            cx.stop_propagation();
        } else {
            cx.propagate();
        }
    }

    /// 状态失效立即释放门禁；已确认文字只交给可验证的剪贴板，失败保留并允许 Ctrl+C 重试。
    pub(super) fn cancel_source_boundary(&mut self, cx: &mut Context<Self>) {
        if let Some(cancellation) = self.coordinator.source_boundary_cancellation.take() {
            cancellation.cancel();
        }
        self.coordinator.source_boundary_generation =
            self.coordinator.source_boundary_generation.wrapping_add(1);
        self.coordinator.source_boundary_completion = None;
        self.coordinator.source_boundary_input_owner = None;
        for action in self.coordinator.source_boundary_actions.drain(..) {
            if let DeferredSourceAction::BoundaryText { text, .. } = action {
                self.coordinator
                    .source_boundary_recovery_text
                    .get_or_insert_with(String::new)
                    .push_str(&text);
            }
        }
        self.copy_source_boundary_recovery(cx);
        cx.emit(DocumentHostEvent::StateChanged);
        cx.notify();
    }

    /// 只有原生剪贴板成功才释放保留文字；不把边界失败降级为在旧位置写入。
    fn copy_source_boundary_recovery(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(text) = self.coordinator.source_boundary_recovery_text.clone() else {
            return false;
        };
        if self.accept_native_cut_clipboard(&text) {
            if cfg!(test) {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
            }
            self.coordinator.source_boundary_recovery_text = None;
            self.error = Some("文字定位已取消，未写入文字已复制；请粘贴后重试。".into());
        } else {
            self.error = Some("文字定位已取消，未写入文字已保留；请按 Ctrl+C 复制后重试。".into());
        }
        cx.emit(DocumentHostEvent::StateChanged);
        cx.notify();
        true
    }

    /// 所属 Source 命令先于后续输入发布；相邻鼠标移动只保留最新命中，避免无界积压。
    pub(super) fn defer_source_action_for_boundary(
        &mut self,
        action: DeferredSourceAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if matches!(action, DeferredSourceAction::Copy) && self.copy_source_boundary_recovery(cx) {
            return true;
        }
        if self.coordinator.source_boundary_cancellation.is_none() {
            return false;
        }
        if let DeferredSourceAction::Pointer(
            current @ source_ime::DeferredSourcePointerAction::Move { .. },
        ) = action
        {
            if let Some(DeferredSourceAction::Pointer(
                previous @ source_ime::DeferredSourcePointerAction::Move { .. },
            )) = self.coordinator.source_boundary_actions.back_mut()
            {
                *previous = current;
            } else {
                self.coordinator
                    .source_boundary_actions
                    .push_back(DeferredSourceAction::Pointer(current));
            }
        } else {
            self.coordinator.source_boundary_actions.push_back(action);
        }
        if self.coordinator.source_boundary_completion.is_some()
            && self.has_active_ime_composition(cx)
            && !matches!(window.finish_ime_composition(), Ok(true))
        {
            self.error = Some("输入法尚未结束，请确认或取消候选后重试。".into());
            cx.notify();
        }
        true
    }

    /// 原生确认文字在解析期间通过 Host 桥暂存，不先改旧位置；相邻输入合为一次事务。
    pub(super) fn queue_source_boundary_text(
        &mut self,
        text: &str,
        selected_range_relative: Option<Range<usize>>,
        undo_kind: UndoCaptureKind,
    ) {
        if undo_kind == UndoCaptureKind::CoalescibleText
            && selected_range_relative.is_none()
            && let Some(DeferredSourceAction::BoundaryText {
                text: previous,
                selected_range_relative: None,
                undo_kind: UndoCaptureKind::CoalescibleText,
            }) = self.coordinator.source_boundary_actions.back_mut()
        {
            previous.push_str(text);
        } else {
            self.coordinator.source_boundary_actions.push_back(
                DeferredSourceAction::BoundaryText {
                    text: text.to_owned(),
                    selected_range_relative,
                    undo_kind,
                },
            );
        }
    }

    /// 已确认文字沿用普通 Source 事务和输入组；解析只影响时序，不能把连续键入拆成多次撤销。
    pub(super) fn commit_source_boundary_text(
        &mut self,
        text: &str,
        selected_range_relative: Option<Range<usize>>,
        undo_kind: UndoCaptureKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(document) = self.document.clone() else {
            return;
        };
        if self.has_active_ime_composition(cx)
            || self.reloading
            || !matches!(
                self.view_mode,
                DocumentHostViewMode::Source | DocumentHostViewMode::Split
            )
        {
            return;
        }
        let before = document.source_selection();
        let range = before.range();
        let after = selected_range_relative
            .filter(|relative| text.get(relative.clone()).is_some())
            .map(|relative| {
                SourceSelection::from_range(
                    range.start + relative.start as u64..range.start + relative.end as u64,
                    false,
                )
            })
            .unwrap_or_else(|| {
                SourceSelection::collapsed(
                    range.start.saturating_add(text.len() as u64),
                    SourceAffinity::After,
                )
            });
        let group = self.source_typing_group_for_edit(
            &document,
            document.revision(),
            before,
            after,
            &range,
            text,
            Some(undo_kind),
        );
        let result = document.next_transaction_id().and_then(|id| {
            let transaction = Transaction::new(
                DocumentRevision(document.revision()),
                vec![SourceEdit::new(range.clone(), text)],
            );
            if let Some(group) = group {
                document.apply_typing_transaction(id, transaction, before, after, group)
            } else {
                document.apply_transaction(id, transaction, before, after)
            }
        });
        if let Err(_error) = result {
            self.break_source_typing_group();
            self.coordinator
                .source_boundary_recovery_text
                .get_or_insert_with(String::new)
                .push_str(text);
            self.copy_source_boundary_recovery(cx);
            return;
        }
        if let Some(group) = group {
            self.install_source_replacement_with_typing_group(
                range,
                text,
                Some(after),
                true,
                false,
                false,
                group,
                cx,
            );
        } else {
            self.install_source_replacement_with_selection(
                range,
                text,
                Some(after),
                true,
                false,
                false,
                cx,
            );
        }
        self.restore_source_navigation_input(window, cx);
        self.finish_source_typing_group(group, &document, after, text);
    }

    /// 等真实 IME 终态再安装边界结果，避免导航途中切换原生输入实体而改变确认位置。
    pub(super) fn finish_source_boundary_after_ime(&mut self, cx: &mut Context<Self>) {
        if self.has_active_ime_composition(cx) {
            return;
        }
        let Some(completion) = self.coordinator.source_boundary_completion.take() else {
            return;
        };
        let this = cx.entity().downgrade();
        cx.defer(move |cx| {
            let _ = cx.update_window(completion.window, move |_root, window, cx| {
                if let Some(this) = this.upgrade() {
                    this.update(cx, |view, cx| (completion.commit)(view, window, cx));
                }
            });
        });
    }

    /// 每次只回放一个动作，正文事件和重绘先完成，再消费下一项；新的慢导航可以继续接管队列。
    fn replay_source_boundary_actions(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.coordinator.source_boundary_cancellation.is_some() {
            return;
        }
        let Some(action) = self.coordinator.source_boundary_actions.pop_front() else {
            cx.emit(DocumentHostEvent::StateChanged);
            cx.notify();
            return;
        };
        self.execute_deferred_source_action(action, window, cx);
        if !self.coordinator.source_boundary_actions.is_empty() {
            let this = cx.entity().downgrade();
            let handle = window.window_handle();
            cx.defer(move |cx| {
                let _ = cx.update_window(handle, move |_root, window, cx| {
                    if let Some(this) = this.upgrade() {
                        this.update(cx, |view, cx| {
                            view.replay_source_boundary_actions(window, cx)
                        });
                    }
                });
            });
        } else {
            cx.emit(DocumentHostEvent::StateChanged);
            cx.notify();
        }
    }
    /// 双击解析期间只持有命中锚点，释放鼠标后仍完成选择，但不重启拖选或滚动。
    pub(super) fn request_source_word_click(
        &mut self,
        line: usize,
        click_count: usize,
        shift: bool,
        hit: u64,
        line_range: Range<u64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.request_source_boundary(
            move |snapshot, cancel| async move {
                resolve_word_range(snapshot.as_ref(), line_range, hit, &cancel)
            },
            move |view, range, window, cx| {
                let dragging = view.source_drag_anchor.is_some();
                view.install_source_pointer_selection(
                    line,
                    click_count,
                    shift,
                    SourceSelection::from_range(range, false),
                    window,
                    cx,
                );
                if !dragging {
                    view.end_source_pointer_selection();
                }
            },
            window,
            cx,
        );
    }

    /// 按下词尚未解析时同时查找两端，避免拖动比后台双击先完成而丢失完整锚点单位。
    pub(super) fn request_source_word_drag(
        &mut self,
        line: usize,
        position: Point<Pixels>,
        anchor: SourceAnchor,
        hit: u64,
        anchor_line_range: Range<u64>,
        line_range: Range<u64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let anchor_range = self.source_drag_anchor_range.clone();
        self.request_source_boundary(
            move |snapshot, cancel| async move {
                let unit = match anchor_range {
                    Some(range) if !range.is_empty() => range,
                    _ => resolve_word_range(
                        snapshot.as_ref(),
                        anchor_line_range,
                        anchor.byte_offset,
                        &cancel,
                    )?,
                };
                let target = resolve_word_range(snapshot.as_ref(), line_range, hit, &cancel)?;
                Ok(source_word_drag_selection(unit, target))
            },
            move |view, selection, window, cx| {
                view.install_source_pointer_move(selection, line, position, window, cx)
            },
            window,
            cx,
        );
    }

    /// 普通行仍同步命中；罕见的长词/字素按不可变快照后台解析。
    /// 选择、revision、模式或输入焦点已改变时丢弃迟到结果，不把旧鼠标命中套到新正文。
    pub(super) fn request_source_boundary<T, Resolve, Resolved, Apply>(
        &mut self,
        resolve: Resolve,
        apply: Apply,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) where
        T: Send + 'static,
        Resolve: FnOnce(Arc<dyn DocumentSnapshot>, SearchCancellation) -> Resolved + Send + 'static,
        Resolved: std::future::Future<Output = Result<T, PagedDocumentError>> + Send + 'static,
        Apply: FnOnce(&mut Self, T, &mut Window, &mut Context<Self>) + 'static,
    {
        let Some(document) = self.document.as_ref() else {
            return;
        };
        let snapshot = match document.snapshot() {
            Ok(snapshot) => snapshot,
            Err(error) => {
                self.error = Some(format!("无法读取文字边界，请重试：{error}").into());
                cx.notify();
                return;
            }
        };
        let revision = snapshot.revision().0;
        let selection = document.source_selection();
        let view_id = document.view_id();
        let epoch = self.document_epoch;
        let mode = self.view_mode;
        let window_handle = window.window_handle();
        // 当前行借用 Host 输入桥；后台完成前，原生文字事件不能发布旧位置的编辑。
        self.active_edit = None;
        self.focus_handle.focus(window);
        self.sync_source_selection_visuals(cx);
        if self.coordinator.source_boundary_cancellation.is_some() {
            self.cancel_source_boundary(cx);
        }
        self.coordinator.source_boundary_input_owner = self
            .source_text_input_owner_line(selection, cx)
            .and_then(|line| self.source_row_blocks.get(&line).cloned());
        self.coordinator.source_boundary_generation =
            self.coordinator.source_boundary_generation.wrapping_add(1);
        let generation = self.coordinator.source_boundary_generation;
        let cancellation = SearchCancellation::default();
        self.coordinator.source_boundary_cancellation = Some(cancellation.clone());
        cx.emit(DocumentHostEvent::StateChanged);
        let work = cx
            .background_executor()
            .spawn(resolve(snapshot, cancellation));
        self.coordinator.source_boundary_task = cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = cx.update_window(window_handle, move |_root, window, cx| {
                let Some(this) = this.upgrade() else {
                    return;
                };
                this.update(cx, |view, cx| {
                    if view.coordinator.source_boundary_generation != generation {
                        return;
                    }
                    if view.document_epoch != epoch
                        || view.view_mode != mode
                        || view.coordinator.lifetime_cancellation.is_cancelled()
                        || !view.source_text_surface_has_focus(window, cx)
                        || view.document.as_ref().is_none_or(|document| {
                            document.view_id() != view_id
                                || document.revision() != revision
                                || document.source_selection() != selection
                        })
                    {
                        view.cancel_source_boundary(cx);
                        return;
                    }
                    match result {
                        Ok(value) => {
                            let commit: BoundaryCommit = Box::new(move |view, window, cx| {
                                if view.coordinator.source_boundary_generation != generation {
                                    return;
                                }
                                if view.document_epoch != epoch
                                    || view.view_mode != mode
                                    || view.coordinator.lifetime_cancellation.is_cancelled()
                                    || !view.source_text_surface_has_focus(window, cx)
                                    || view.document.as_ref().is_none_or(|document| {
                                        document.view_id() != view_id
                                            || document.revision() != revision
                                            || document.source_selection() != selection
                                    })
                                {
                                    view.cancel_source_boundary(cx);
                                    return;
                                }
                                view.coordinator.source_boundary_cancellation = None;
                                view.coordinator.source_boundary_input_owner = None;
                                apply(view, value, window, cx);
                                view.replay_source_boundary_actions(window, cx);
                            });
                            if view.has_active_ime_composition(cx) {
                                view.coordinator.source_boundary_completion =
                                    Some(SourceBoundaryCompletion {
                                        window: window_handle,
                                        commit,
                                    });
                                if !matches!(window.finish_ime_composition(), Ok(true)) {
                                    view.error =
                                        Some("输入法尚未结束，请确认或取消候选后重试。".into());
                                    cx.notify();
                                }
                            } else {
                                commit(view, window, cx);
                            }
                        }
                        Err(PagedDocumentError::Cancelled) => view.cancel_source_boundary(cx),
                        Err(error) => {
                            view.cancel_source_boundary(cx);
                            view.restore_source_navigation_input(window, cx);
                            if view.error.is_none() {
                                view.error =
                                    Some(format!("无法定位文字边界，请重试：{error}").into());
                            }
                            cx.notify();
                        }
                    }
                });
            });
        });
    }
}
