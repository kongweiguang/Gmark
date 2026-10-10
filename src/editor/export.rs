// @author kongweiguang

//! Editor-facing export flow and file writing.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};
use std::thread;

#[cfg(test)]
use anyhow::Context as _;
use futures::FutureExt as _;
use futures::channel::oneshot;
use gpui::*;

use super::Editor;
use crate::export::{self as document_export, ExportFormat};
use crate::i18n::I18nManager;
use crate::theme::{Theme, ThemeManager};
use feedback::{
    confirm_export_path, show_document_export_error, show_export_completed, show_export_error,
};
#[cfg(test)]
use feedback::{export_failure_message, normalized_export_path};

mod feedback;

enum ExportTaskResult {
    Complete,
    Cancelled,
    Failed(String),
}

#[repr(u8)]
enum ExportPhase {
    Generating,
    Cancelled,
    Saving,
}

#[derive(Default)]
pub(super) struct ExportProgress {
    pub(super) completed: AtomicUsize,
    pub(super) total: AtomicUsize,
    phase: AtomicU8,
}

impl ExportProgress {
    /// 取消与最终提交竞争同一个状态，保证“正在取消”不会与替换目标文件同时成立。
    fn request_cancel(&self, cancelled: &AtomicBool) -> bool {
        if self
            .phase
            .compare_exchange(
                ExportPhase::Generating as u8,
                ExportPhase::Cancelled as u8,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
        {
            cancelled.store(true, Ordering::Release);
            true
        } else {
            false
        }
    }

    /// 原子文件替换开始后不能回滚；在此边界以前响应取消，以后明确展示正在保存。
    fn begin_commit(&self, cancelled: &AtomicBool) -> bool {
        !cancelled.load(Ordering::Acquire)
            && self
                .phase
                .compare_exchange(
                    ExportPhase::Generating as u8,
                    ExportPhase::Saving as u8,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_ok()
    }

    /// 界面读取与后台写入共享提交状态，避免保存阶段仍提供无法兑现的取消按钮。
    pub(super) fn is_committing(&self) -> bool {
        self.phase.load(Ordering::Acquire) == ExportPhase::Saving as u8
    }
}

impl Editor {
    /// 窗口壳读取任务所有者的进度，取消监听仍绑定所有者；切换活动窗格不隐藏后台任务。
    pub(super) fn render_current_export_progress(
        &self,
        theme: &Theme,
        bottom_offset: f32,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if self.export_in_progress {
            return Some(self.render_export_progress(theme, bottom_offset, cx));
        }
        let owner = self
            .pane_canvas_entities
            .borrow()
            .values()
            .filter_map(|(_, _, canvas)| canvas.markdown_editor(cx))
            .find(|editor| editor.read(cx).export_in_progress)?;
        Some(owner.update(cx, |editor, cx| {
            editor.render_export_progress(theme, bottom_offset, cx)
        }))
    }

    /// 底栏优先显示窗口提示，再读取活动叶子或后台叶子的结果，不复制任务状态和计时器。
    pub(super) fn current_pane_notice(&self, cx: &App) -> Option<SharedString> {
        self.pane_notice.clone().or_else(|| {
            self.focused_pane_entities(cx)
                .0
                .and_then(|editor| editor.read(cx).pane_notice.clone())
                .or_else(|| {
                    self.pane_canvas_entities
                        .borrow()
                        .values()
                        .filter_map(|(_, _, canvas)| canvas.markdown_editor(cx))
                        .find_map(|editor| editor.read(cx).pane_notice.clone())
                })
        })
    }

    /// 仅导出反馈可见及其收尾时刷新窗口壳，避免把叶子每次正文输入都升级成整窗重绘。
    pub(super) fn observe_pane_export_feedback(editor: &Entity<Self>, cx: &mut Context<Self>) {
        let feedback = |editor: &Self| {
            (
                editor.export_in_progress,
                editor.export_cancel_requested,
                editor.export_progress.as_ref().map(|progress| {
                    (
                        progress.completed.load(Ordering::Acquire),
                        progress.total.load(Ordering::Acquire),
                        progress.is_committing(),
                    )
                }),
                editor.pane_notice.clone(),
            )
        };
        let mut previous = feedback(editor.read(cx));
        cx.observe(editor, move |_root, editor, cx| {
            let next = feedback(editor.read(cx));
            if previous != next {
                previous = next;
                cx.notify();
            }
        })
        .detach();
    }

    fn export_dialog_defaults(&self, format: ExportFormat) -> (PathBuf, String) {
        let extension = format.extension();
        if let Some(path) = self.file_path.as_ref() {
            let directory = path
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
            let stem = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .filter(|stem| !stem.is_empty())
                .unwrap_or("untitled");
            return (directory, format!("{stem}.{extension}"));
        }

        (
            std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            format!("untitled.{extension}"),
        )
    }

    fn mermaid_svg_export_dialog_defaults(&self) -> (PathBuf, String) {
        mermaid_svg_export_defaults(self.file_path.as_deref())
    }

    fn export_title(&self) -> String {
        self.file_path
            .as_ref()
            .and_then(|path| path.file_stem())
            .map(|stem| stem.to_string_lossy().to_string())
            .filter(|stem| !stem.is_empty())
            .unwrap_or_else(|| "Untitled".to_string())
    }

    #[cfg(test)]
    fn render_export_bytes(
        format: ExportFormat,
        markdown: &str,
        theme: &Theme,
        title: &str,
        source_base_dir: Option<&Path>,
    ) -> anyhow::Result<Vec<u8>> {
        match format {
            ExportFormat::Html => Ok(document_export::render_html_with_base_dir(
                markdown,
                theme,
                title,
                source_base_dir,
            )
            .into_bytes()),
            ExportFormat::Png => {
                document_export::render_png(markdown, theme, title, source_base_dir)
            }
            ExportFormat::Pdf => {
                document_export::render_pdf(markdown, theme, title, source_base_dir)
            }
        }
    }

    #[cfg(test)]
    fn write_export_bytes(
        format: ExportFormat,
        markdown: &str,
        theme: &Theme,
        title: &str,
        path: &Path,
        source_base_dir: Option<&Path>,
    ) -> anyhow::Result<()> {
        let bytes = Self::render_export_bytes(format, markdown, theme, title, source_base_dir)?;
        std::fs::write(path, bytes).with_context(|| format!("failed to write '{}'", path.display()))
    }

    #[cfg(test)]
    fn write_export_bytes_cancellable(
        format: ExportFormat,
        markdown: &str,
        theme: &Theme,
        title: &str,
        path: &Path,
        source_base_dir: Option<&Path>,
        cancelled: &AtomicBool,
    ) -> ExportTaskResult {
        Self::write_export_bytes_cancellable_with_progress(
            format,
            markdown,
            theme,
            title,
            path,
            source_base_dir,
            cancelled,
            None,
        )
    }

    /// 生成与资源复制允许取消；提交前竞争取消状态，失败时只清理本次创建的资源。
    fn write_export_bytes_cancellable_with_progress(
        format: ExportFormat,
        markdown: &str,
        theme: &Theme,
        title: &str,
        path: &Path,
        source_base_dir: Option<&Path>,
        cancelled: &AtomicBool,
        progress: Option<&ExportProgress>,
    ) -> ExportTaskResult {
        if cancelled.load(Ordering::Acquire) {
            return ExportTaskResult::Cancelled;
        }
        let mut prepared_html_resources: Option<document_export::PreparedHtmlResources> = None;
        let rendered = match format {
            ExportFormat::Html => {
                let prepared = match document_export::prepare_html_resources_with_progress(
                    markdown,
                    source_base_dir,
                    path,
                    cancelled,
                    progress.map(|progress| &progress.completed),
                ) {
                    Ok(prepared) => prepared,
                    Err(_error) if cancelled.load(Ordering::Acquire) => {
                        return ExportTaskResult::Cancelled;
                    }
                    Err(error) => return ExportTaskResult::Failed(error.to_string()),
                };
                let bytes = document_export::render_html_with_base_dir(
                    &prepared.markdown,
                    theme,
                    title,
                    source_base_dir,
                )
                .into_bytes();
                prepared_html_resources = Some(prepared);
                Ok(bytes)
            }
            ExportFormat::Png => document_export::render_png_cancellable(
                markdown,
                theme,
                title,
                source_base_dir,
                cancelled,
            ),
            ExportFormat::Pdf => document_export::render_pdf_cancellable(
                markdown,
                theme,
                title,
                source_base_dir,
                cancelled,
            ),
        };
        let bytes = match rendered {
            Ok(bytes) => bytes,
            Err(_) if cancelled.load(Ordering::Acquire) => {
                if let Some(prepared) = prepared_html_resources.as_ref() {
                    prepared.cleanup_created();
                }
                return ExportTaskResult::Cancelled;
            }
            Err(error) => {
                if let Some(prepared) = prepared_html_resources.as_ref() {
                    prepared.cleanup_created();
                }
                return ExportTaskResult::Failed(error.to_string());
            }
        };
        if cancelled.load(Ordering::Acquire)
            || progress.is_some_and(|progress| !progress.begin_commit(cancelled))
        {
            if let Some(prepared) = prepared_html_resources.as_ref() {
                prepared.cleanup_created();
            }
            return ExportTaskResult::Cancelled;
        }
        if let Some(progress) = progress {
            progress
                .completed
                .store(progress.total.load(Ordering::Acquire), Ordering::Release);
        }
        match gmark_document::atomic_write(path, &bytes) {
            Ok(()) => ExportTaskResult::Complete,
            Err(error) => {
                if let Some(prepared) = prepared_html_resources.as_ref() {
                    prepared.cleanup_created();
                }
                ExportTaskResult::Failed(error.to_string())
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn export_document_to_path(
        &self,
        format: ExportFormat,
        path: &Path,
        cx: &App,
    ) -> anyhow::Result<()> {
        let markdown = self.serialized_document_text(cx);
        let theme = cx.global::<ThemeManager>().current().clone();
        let title = self.export_title();
        let source_base_dir = self.file_path.as_ref().and_then(|path| path.parent());
        Self::write_export_bytes(format, &markdown, &theme, &title, path, source_base_dir)
    }

    /// 保存位置始终经系统对话框确认；后台任务收尾后再报告结果，失败重试重新保留覆盖确认。
    pub(crate) fn export_document_via_prompt(
        &mut self,
        format: ExportFormat,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.export_task.is_some() {
            return;
        }
        let markdown = self.serialized_document_text(cx);
        let theme = cx.global::<ThemeManager>().current().clone();
        let title = self.export_title();
        let source_base_dir = self
            .file_path
            .as_ref()
            .and_then(|path| path.parent())
            .map(Path::to_path_buf);
        let (default_dir, suggested_name) = self.export_dialog_defaults(format);
        let prompt = cx.prompt_for_new_path(&default_dir, Some(&suggested_name));
        let window_handle = window.window_handle();
        let cancelled = Arc::new(AtomicBool::new(false));
        let progress = Arc::new(ExportProgress::default());
        self.export_cancel = Some(Arc::clone(&cancelled));
        self.export_progress = Some(Arc::clone(&progress));
        self.export_cancel_requested = false;

        self.export_task = Some(cx.spawn(
            async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
                let path = match prompt.await {
                    Ok(Ok(Some(path))) => path,
                    Ok(Ok(None)) | Err(_) => {
                        let _ = this.update(cx, |editor, _cx| {
                            editor.export_task = None;
                            editor.export_cancel = None;
                            editor.export_progress = None;
                            editor.export_in_progress = false;
                            editor.export_cancel_requested = false;
                        });
                        return;
                    }
                    Ok(Err(err)) => {
                        let _ = this.update(cx, |editor, _cx| {
                            editor.export_task = None;
                            editor.export_cancel = None;
                            editor.export_progress = None;
                        });
                        let detail = err.to_string();
                        let _ = cx.update_window(
                            window_handle,
                            move |_view: AnyView, window: &mut Window, cx: &mut App| {
                                show_export_error(window, cx, &detail);
                            },
                        );
                        return;
                    }
                };

                let Some(path) = confirm_export_path(path, format.extension(), window_handle, cx).await else {
                    let _ = this.update(cx, |editor, cx| {
                        editor.export_task = None;
                        editor.export_cancel = None;
                        editor.export_progress = None;
                        cx.notify();
                    });
                    return;
                };

                let total = if format == ExportFormat::Html {
                    document_export::count_local_resource_cards(
                        &markdown,
                        source_base_dir.as_deref(),
                    )
                } else {
                    1
                };
                progress.total.store(total.max(1), Ordering::Release);

                let _ = this.update(cx, |editor, cx| {
                    editor.export_in_progress = true;
                    cx.notify();
                });

                let (sender, receiver) = oneshot::channel();
                let exported_path = path.clone();
                let worker_cancelled = Arc::clone(&cancelled);
                let worker_progress = Arc::clone(&progress);
                let spawn_result = thread::Builder::new()
                    .name("gmark-export".to_string())
                    .spawn(move || {
                        let result = Self::write_export_bytes_cancellable_with_progress(
                            format,
                            &markdown,
                            &theme,
                            &title,
                            &path,
                            source_base_dir.as_deref(),
                            &worker_cancelled,
                            Some(&worker_progress),
                        );
                        let _ = sender.send(result);
                    });

                if let Err(err) = spawn_result {
                    let _ = this.update(cx, |editor, cx| {
                        editor.export_task = None;
                        editor.export_cancel = None;
                        editor.export_progress = None;
                        editor.export_in_progress = false;
                        editor.export_cancel_requested = false;
                        cx.notify();
                    });
                    let detail = format!("failed to start export task: {err}");
                    let _ = cx.update_window(
                        window_handle,
                        move |_view: AnyView, window: &mut Window, cx: &mut App| {
                            show_export_error(window, cx, &detail);
                        },
                    );
                    return;
                }

                let mut receiver = receiver.fuse();
                let mut last_progress = (0, false);
                let result = loop {
                    let refresh = cx.background_executor().timer(std::time::Duration::from_millis(100)).fuse();
                    futures::pin_mut!(refresh);
                    futures::select! {
                        result = receiver => break result.unwrap_or_else(|_| ExportTaskResult::Failed("export task stopped before reporting a result".to_owned())),
                        () = refresh => {
                            let current = (progress.completed.load(Ordering::Acquire), progress.is_committing());
                            if current != last_progress {
                                let _ = this.update(cx, |_editor, cx| cx.notify());
                                last_progress = current;
                            }
                        }
                    }
                };
                let _ = this.update(cx, |editor, cx| {
                    editor.export_task = None;
                    editor.export_cancel = None;
                    editor.export_progress = None;
                    editor.export_in_progress = false;
                    editor.export_cancel_requested = false;
                    let strings = cx.global::<I18nManager>().strings().clone();
                    match &result {
                        ExportTaskResult::Complete => {},
                        ExportTaskResult::Cancelled => {
                            editor.show_pane_notice(strings.export_cancelled.clone(), cx)
                        }
                        ExportTaskResult::Failed(_) => {}
                    }
                    cx.notify();
                });
                match result {
                    ExportTaskResult::Complete => show_export_completed(&this, &exported_path, window_handle, cx),
                    ExportTaskResult::Failed(detail) => show_document_export_error(this, format, window_handle, detail, cx),
                    ExportTaskResult::Cancelled => {},
                }
            },
        ));
    }

    /// Saves one already-rendered Mermaid SVG without re-running document export.
    /// The diagram is immutable at this boundary, so the editor owns only the
    /// save dialog, atomic write, and recoverable failure presentation.
    pub(crate) fn export_mermaid_svg_via_prompt(
        &mut self,
        svg: String,
        window_handle: AnyWindowHandle,
        cx: &mut Context<Self>,
    ) {
        if self.export_task.is_some() {
            return;
        }
        let (default_dir, suggested_name) = self.mermaid_svg_export_dialog_defaults();
        let prompt = cx.prompt_for_new_path(&default_dir, Some(&suggested_name));
        self.export_task = Some(cx.spawn(async move |this: WeakEntity<Self>, cx| {
            let path = match prompt.await {
                Ok(Ok(Some(path))) => path,
                Ok(Ok(None)) | Err(_) => {
                    let _ = this.update(cx, |editor, _cx| {
                        editor.export_task = None;
                    });
                    return;
                }
                Ok(Err(error)) => {
                    let detail = error.to_string();
                    let _ = this.update(cx, |editor, _cx| {
                        editor.export_task = None;
                    });
                    let _ = cx.update_window(
                        window_handle,
                        move |_view: AnyView, window: &mut Window, cx: &mut App| {
                            show_export_error(window, cx, &detail);
                        },
                    );
                    return;
                }
            };
            let Some(path) = confirm_export_path(path, "svg", window_handle, cx).await else {
                let _ = this.update(cx, |editor, cx| {
                    editor.export_task = None;
                    cx.notify();
                });
                return;
            };
            let exported_path = path.clone();

            let result = cx
                .background_spawn(async move { write_mermaid_svg(&path, &svg) })
                .await;
            let _ = this.update(cx, |editor, cx| {
                editor.export_task = None;
                cx.notify();
            });
            if let Err(detail) = result {
                let _ = cx.update_window(
                    window_handle,
                    move |_view: AnyView, window: &mut Window, cx: &mut App| {
                        show_export_error(window, cx, &detail);
                    },
                );
            } else {
                show_export_completed(&this, &exported_path, window_handle, cx);
            }
        }));
    }

    /// Export the current resident selection as Markdown while preserving the
    /// source selection's byte boundaries. The serialized document is the
    /// Live 锚点已映射到源码文本，切片必须使用相同拼写，避免显示投影的表格和注释长度错位。
    pub(crate) fn export_selection_via_prompt(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.export_task.is_some() {
            return;
        }
        let range = self.capture_source_selection_snapshot(cx).range();
        if range.is_empty() {
            return;
        }
        let source = self.serialized_document_text(cx);
        let Some(selection) = source.get(range).map(str::to_owned) else {
            return;
        };
        let default_dir = self
            .file_path
            .as_ref()
            .and_then(|path| path.parent())
            .map(Path::to_path_buf)
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
        let stem = self
            .file_path
            .as_ref()
            .and_then(|path| path.file_stem())
            .map(|stem| stem.to_string_lossy().into_owned())
            .filter(|stem| !stem.is_empty())
            .unwrap_or_else(|| "untitled".to_owned());
        let prompt = cx.prompt_for_new_path(&default_dir, Some(&format!("{stem}.selection.md")));
        let window_handle = window.window_handle();

        self.export_task = Some(cx.spawn(async move |this: WeakEntity<Self>, cx| {
            let path = match prompt.await {
                Ok(Ok(Some(path))) => path,
                Ok(Ok(None)) | Err(_) => {
                    let _ = this.update(cx, |editor, _cx| {
                        editor.export_task = None;
                        editor.export_in_progress = false;
                    });
                    return;
                }
                Ok(Err(error)) => {
                    let detail = error.to_string();
                    let _ = this.update(cx, |editor, _cx| {
                        editor.export_task = None;
                        editor.export_in_progress = false;
                    });
                    let _ = cx.update_window(
                        window_handle,
                        move |_view: AnyView, window: &mut Window, cx: &mut App| {
                            show_export_error(window, cx, &detail);
                        },
                    );
                    return;
                }
            };

            let Some(path) = confirm_export_path(path, "md", window_handle, cx).await else {
                let _ = this.update(cx, |editor, cx| {
                    editor.export_task = None;
                    cx.notify();
                });
                return;
            };
            let exported_path = path.clone();
            let _ = this.update(cx, |editor, cx| {
                editor.export_in_progress = true;
                cx.notify();
            });
            let result = cx
                .background_spawn(async move {
                    gmark_document::atomic_write(&path, selection.as_bytes())
                        .map_err(|error| error.to_string())
                })
                .await;
            let _ = this.update(cx, |editor, cx| {
                editor.export_task = None;
                editor.export_in_progress = false;
                cx.notify();
            });
            if let Err(detail) = result {
                let _ = cx.update_window(
                    window_handle,
                    move |_view: AnyView, window: &mut Window, cx: &mut App| {
                        show_export_error(window, cx, &detail);
                    },
                );
            } else {
                show_export_completed(&this, &exported_path, window_handle, cx);
            }
        }));
    }

    /// 保存阶段已越过可取消边界；只有取消赢得状态竞争时才改变界面和浏览器取消信号。
    pub(crate) fn on_cancel_export(
        &mut self,
        _: &ClickEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(cancelled) = self.export_cancel.as_ref()
            && self
                .export_progress
                .as_ref()
                .is_some_and(|progress| progress.request_cancel(cancelled))
        {
            self.export_cancel_requested = true;
            cx.notify();
        }
    }
}

fn mermaid_svg_export_defaults(file_path: Option<&Path>) -> (PathBuf, String) {
    let directory = file_path
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let stem = file_path
        .and_then(Path::file_stem)
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.is_empty())
        .unwrap_or("untitled");
    (directory, format!("{stem}-mermaid.svg"))
}

fn write_mermaid_svg(path: &Path, svg: &str) -> Result<(), String> {
    gmark_document::atomic_write(path, svg.as_bytes()).map_err(|error| error.to_string())
}

#[cfg(test)]
#[path = "../../tests/unit/editor/export.rs"]
mod tests;
