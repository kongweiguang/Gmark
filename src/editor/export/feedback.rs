// @author kongweiguang

//! Save-path confirmation and user-facing export feedback.

use std::path::{Path, PathBuf};

use gpui::*;

use super::Editor;
use crate::export::ExportFormat;
use crate::i18n::I18nManager;
use crate::preferences::EditorSettings;

/// 可恢复失败保留原生对话框，并重新打开保存流程以让用户改位置或确认覆盖。
pub(super) fn show_document_export_error(
    editor: WeakEntity<Editor>,
    format: ExportFormat,
    window_handle: AnyWindowHandle,
    detail: String,
    cx: &mut AsyncApp,
) {
    eprintln!("文档导出失败: {detail}");
    let prompt = cx.update_window(window_handle, move |_view, window, cx| {
        let strings = cx.global::<I18nManager>().strings_arc();
        let message = export_failure_message(&detail, &strings);
        window.prompt(
            PromptLevel::Warning,
            &strings.export_failed_title,
            Some(&message),
            &[
                strings.export_retry.as_str(),
                strings.export_cancel.as_str(),
            ],
            cx,
        )
    });
    cx.spawn(async move |cx| {
        if let Ok(prompt) = prompt
            && prompt.await.ok() == Some(0)
        {
            let _ = cx.update_window(window_handle, move |_view, window, cx| {
                let _ = editor.update(cx, |editor, cx| {
                    editor.export_document_via_prompt(format, window, cx);
                });
            });
        }
    })
    .detach();
}

/// 已知浏览器与写入故障给出可执行的提示；未知错误保留简短详情，完整诊断进入日志。
pub(super) fn export_failure_message(detail: &str, strings: &crate::i18n::I18nStrings) -> String {
    if detail.contains("No Chromium-compatible browser") {
        strings.export_failed_missing_browser.clone()
    } else if detail.starts_with("PDF export timed out while waiting for Chromium")
        || detail.starts_with("image export timed out while waiting for Chromium")
    {
        strings.export_failed_timeout.clone()
    } else if [
        "failed to launch Chromium for ",
        "failed to build Chromium browser config",
        "failed to open export HTML in Chromium",
        "Chromium did not finish loading export HTML",
        "Chromium failed to ",
    ]
    .iter()
    .any(|prefix| detail.starts_with(prefix))
    {
        strings.export_failed_browser.clone()
    } else if detail.starts_with("原子写入 ") {
        strings.export_failed_write.clone()
    } else {
        let brief: String = detail.chars().take(360).collect();
        strings
            .export_failed_generic_template
            .replace("{error}", &brief)
    }
}

/// 格式后缀与实际字节保持一致；大小写正确的用户文件名原样保留。
pub(super) fn normalized_export_path(mut path: PathBuf, extension: &str) -> PathBuf {
    if !path
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case(extension))
    {
        path.set_extension(extension);
    }
    path
}

/// 后缀规范化可能得到对话框未确认过的既有文件，必须对最终目标补一次覆盖确认。
pub(super) async fn confirm_export_path(
    path: PathBuf,
    extension: &str,
    window_handle: AnyWindowHandle,
    cx: &mut AsyncApp,
) -> Option<PathBuf> {
    let normalized = normalized_export_path(path.clone(), extension);
    if normalized != path && normalized.exists() {
        let target = normalized.clone();
        let prompt = cx
            .update_window(window_handle, move |_view, window, cx| {
                let strings = cx.global::<I18nManager>().strings_arc();
                let message = strings
                    .export_replace_message_template
                    .replace("{path}", &target.to_string_lossy());
                window.prompt(
                    PromptLevel::Warning,
                    &strings.export_replace_title,
                    Some(&message),
                    &[
                        strings.export_replace.as_str(),
                        strings.export_cancel.as_str(),
                    ],
                    cx,
                )
            })
            .ok()?;
        if prompt.await.ok() != Some(0) {
            return None;
        }
    }
    Some(normalized)
}

/// 完成提示复用底栏；专注模式或隐藏底栏时用原生通知，保证用户能获知实际落盘结果。
pub(super) fn show_export_completed(
    editor: &WeakEntity<Editor>,
    path: &Path,
    window_handle: AnyWindowHandle,
    cx: &mut AsyncApp,
) {
    let hidden_status = editor
        .update(cx, |editor, cx| {
            let strings = cx.global::<I18nManager>().strings_arc();
            let name = path
                .file_name()
                .unwrap_or(path.as_os_str())
                .to_string_lossy();
            let target = format!("{name} · {}", path.display());
            editor.show_pane_notice(
                strings.export_completed_template.replace("{path}", &target),
                cx,
            );
            editor.focus_mode || !EditorSettings::status_bar_preferences(cx).enabled
        })
        .unwrap_or(false);
    let hidden_status = cx
        .update_window(window_handle, |view, _window, cx| {
            view.downcast::<Editor>()
                .map(|root| {
                    root.read(cx).focus_mode || !EditorSettings::status_bar_preferences(cx).enabled
                })
                .unwrap_or(hidden_status)
        })
        .unwrap_or(hidden_status);
    if hidden_status {
        let target = path.to_path_buf();
        let _ = cx.update_window(window_handle, move |_view, window, cx| {
            let strings = cx.global::<I18nManager>().strings_arc();
            let message = strings
                .export_completed_template
                .replace("{path}", &target.to_string_lossy());
            let _ = window.prompt(
                PromptLevel::Info,
                &strings.export_completed_title,
                Some(&message),
                &[strings.info_dialog_ok.as_str()],
                cx,
            );
        });
    }
}

/// 其它轻量导出沿用确认按钮，友好说明与完整日志使用与文档导出相同的诊断边界。
pub(super) fn show_export_error(window: &mut Window, cx: &mut App, detail: &str) {
    eprintln!("导出失败: {detail}");
    let strings = cx.global::<I18nManager>().strings().clone();
    let message = export_failure_message(detail, &strings);
    let buttons = [strings.info_dialog_ok.as_str()];
    let _ = window.prompt(
        PromptLevel::Critical,
        &strings.export_failed_title,
        Some(&message),
        &buttons,
        cx,
    );
}
