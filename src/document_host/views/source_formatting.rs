// @author kongweiguang

use super::*;

impl DocumentHost {
    pub(super) fn start_format_before_save(
        &mut self,
        window: gpui::AnyWindowHandle,
        cx: &mut Context<Self>,
    ) {
        self.save_after_format = Some(window);
        self.start_source_format(false, cx);
    }

    /// Waits for native Source composition before taking a formatting snapshot.
    pub(crate) fn on_format_document(
        &mut self,
        _: &FormatDocument,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.defer_source_action_for_ime(
            super::source_ime::DeferredSourceAction::FormatDocument,
            window,
            cx,
        ) {
            return;
        }
        self.start_source_format(false, cx);
    }

    /// Waits for native Source composition before capturing a selection for formatting.
    pub(crate) fn on_format_selection(
        &mut self,
        _: &FormatSelection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.defer_source_action_for_ime(
            super::source_ime::DeferredSourceAction::FormatSelection,
            window,
            cx,
        ) {
            return;
        }
        self.start_source_format(true, cx);
    }

    pub(crate) fn on_cancel_formatting(
        &mut self,
        _: &CancelFormatting,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cancel_source_formatting();
        cx.notify();
    }

    pub(super) fn cancel_source_formatting(&mut self) {
        self.format_generation = self.format_generation.wrapping_add(1);
        if let Some(cancellation) = self.format_cancellation.take() {
            cancellation.cancel();
        }
        self.format_running = false;
        self.save_after_format = None;
        self.format_task = Task::ready(());
        if self
            .coordinator
            .external_status
            .as_deref()
            .is_some_and(|status| status.as_ref() == "正在格式化…")
        {
            self.coordinator.external_status = None;
        }
    }

    pub(super) fn start_source_format(&mut self, selection_only: bool, cx: &mut Context<Self>) {
        if self.probe.strategy == OpenStrategy::Paged {
            self.error = Some("大文件模式不支持全文或选区格式化".into());
            self.save_after_format = None;
            cx.notify();
            return;
        }
        let Some(document) = self.document.clone() else {
            return;
        };
        let selection = selection_only
            .then(|| document.source_selection().range())
            .filter(|range| !range.is_empty());
        if selection_only && selection.is_none() {
            self.error = Some("请先选择需要格式化的源码".into());
            self.save_after_format = None;
            cx.notify();
            return;
        }
        let resolution = crate::source_tools::resolve_formatter(
            self.source_language,
            &self.path,
            selection.clone(),
        );
        if let crate::source_tools::FormatterResolution::Unavailable(message) = &resolution {
            self.error = Some(message.clone().into());
            self.save_after_format = None;
            cx.notify();
            return;
        }
        if let crate::source_tools::FormatterResolution::External(spec) = &resolution {
            if selection.is_some() && !spec.supports_range {
                self.error = Some("当前格式化器不支持选区格式化".into());
                self.save_after_format = None;
                cx.notify();
                return;
            }
            if spec.from_workspace {
                self.mode_notice = Some("正在使用工作区 .gmark.toml 中的 Shell 格式化器".into());
            }
        }
        let json_selection_indent = if selection.is_some()
            && matches!(
                resolution,
                crate::source_tools::FormatterResolution::BuiltinJson
            ) {
            selection
                .as_ref()
                .and_then(|range| {
                    document
                        .line_for_offset(range.start)
                        .map(|line| (range, line))
                })
                .and_then(|(range, line)| {
                    document.line_range(line).and_then(|line_range| {
                        document.read_range(line_range.start..range.start).ok()
                    })
                })
                .and_then(|prefix| String::from_utf8(prefix).ok())
                .map(|prefix| {
                    prefix.chars().fold(0usize, |column, character| {
                        if character == '\t' {
                            column.saturating_add(4 - column % 4)
                        } else {
                            column.saturating_add(1)
                        }
                    })
                })
                .unwrap_or_default()
        } else {
            0
        };

        let save_after_format = self.save_after_format.take();
        self.cancel_source_formatting();
        self.save_after_format = save_after_format;
        self.format_generation = self.format_generation.wrapping_add(1);
        let generation = self.format_generation;
        let task_stamp = DocumentTaskStamp::capture(self, generation);
        let cancellation = SearchCancellation::default();
        let worker_cancellation = cancellation.clone();
        let revision = document.revision();
        let range = selection.unwrap_or(0..document.len());
        let read_range = range.clone();
        self.format_cancellation = Some(cancellation);
        self.format_running = true;
        self.coordinator.external_status = Some("正在格式化…".into());
        self.error = None;
        self.format_task = cx.spawn(async move |this, cx| {
            let formatted = cx
                .background_spawn(async move {
                    let bytes = document
                        .read_range_cancellable(read_range, &worker_cancellation)
                        .map_err(|error| {
                            crate::source_tools::FormatError::External(error.to_string())
                        })?;
                    if worker_cancellation.is_cancelled() {
                        return Err(crate::source_tools::FormatError::Cancelled);
                    }
                    let input = std::str::from_utf8(&bytes)
                        .map_err(|_| crate::source_tools::FormatError::InvalidUtf8)?;
                    match resolution {
                        crate::source_tools::FormatterResolution::BuiltinJson => {
                            crate::source_tools::format_json(input).map(|candidate| {
                                crate::source_tools::indent_multiline_candidate(
                                    candidate,
                                    json_selection_indent,
                                )
                            })
                        }
                        crate::source_tools::FormatterResolution::BuiltinJsonLines => {
                            crate::source_tools::format_json_lines(input)
                        }
                        crate::source_tools::FormatterResolution::External(spec) => {
                            crate::source_tools::run_shell_formatter(
                                &spec,
                                &bytes,
                                &worker_cancellation,
                            )
                        }
                        crate::source_tools::FormatterResolution::Unavailable(message) => {
                            Err(crate::source_tools::FormatError::MissingFormatter(message))
                        }
                    }
                })
                .await;
            let _ = this.update(cx, |view, cx| {
                if !task_stamp.accepts_strict(view, view.format_generation) {
                    return;
                }
                view.format_cancellation = None;
                view.format_running = false;
                view.coordinator.external_status = None;
                let candidate = match formatted {
                    Ok(candidate) => candidate,
                    Err(crate::source_tools::FormatError::Cancelled) => {
                        cx.notify();
                        return;
                    }
                    Err(error) => {
                        view.save_after_format = None;
                        view.error = Some(error.to_string().into());
                        cx.notify();
                        return;
                    }
                };
                let Some(current) = view.document.clone() else {
                    return;
                };
                if current.revision() != revision {
                    view.save_after_format = None;
                    view.error = Some("格式化期间文档已变化，请重试".into());
                    cx.notify();
                    return;
                }
                let current_bytes = match current.read_range(range.clone()) {
                    Ok(bytes) => bytes,
                    Err(error) => {
                        view.save_after_format = None;
                        view.error = Some(localized_document_error(&error, cx));
                        cx.notify();
                        return;
                    }
                };
                if current_bytes == candidate.as_bytes() {
                    view.error = None;
                    if let Some(window) = view.save_after_format.take() {
                        view.start_save(view.path.clone(), false, window, cx);
                    }
                    cx.notify();
                    return;
                }
                if let Err(error) = current.replace_range(range.clone(), candidate.clone()) {
                    view.save_after_format = None;
                    view.error = Some(localized_document_error(&error, cx));
                    cx.notify();
                    return;
                }
                // 格式化后的后台重解析会按结构路径恢复仍匹配的折叠项；这里保留旧状态
                // 作为匹配基线，不按“普通编辑命中即展开”的规则提前清空。
                view.install_source_replacement(range, &candidate, false, false, true, cx);
                if let Some(window) = view.save_after_format.take() {
                    view.start_save(view.path.clone(), false, window, cx);
                }
            });
        });
        cx.notify();
    }
}
