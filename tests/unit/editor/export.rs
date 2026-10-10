// @author kongweiguang

use super::{
    Editor, ExportProgress, ExportTaskResult, export_failure_message, mermaid_svg_export_defaults,
    write_mermaid_svg,
};
use crate::export::ExportFormat;
use crate::theme::Theme;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

/// 工作区壳必须呈现叶子的后台状态，并把取消点击交回任务所有者，不能静默导出。
#[gpui::test]
async fn workspace_export_feedback_and_cancel_reach_leaf(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        crate::i18n::I18nManager::init_with_language_id(cx, "en-US");
        crate::theme::ThemeManager::init(cx);
        crate::components::init(cx);
    });
    let (root, visual) =
        cx.add_window_view(|_window, cx| Editor::from_markdown(cx, "# test".to_owned(), None));
    root.update(visual, |root, cx| {
        root.split_pane_toward(crate::editor::panes::PaneSplitDirection::Right, cx);
    });
    visual.update(|window, cx| window.draw(cx).clear());
    visual.run_until_parked();
    let leaf = root.read_with(visual, |root, cx| {
        root.focused_pane_entities(cx).0.expect("Markdown pane")
    });
    let cancelled = std::sync::Arc::new(AtomicBool::new(false));
    leaf.update(visual, |leaf, cx| {
        leaf.export_in_progress = true;
        leaf.export_cancel = Some(cancelled.clone());
        leaf.export_progress = Some(std::sync::Arc::new(ExportProgress::default()));
        cx.notify();
    });
    visual.run_until_parked();
    visual.update(|window, cx| window.draw(cx).clear());
    assert!(visual.debug_bounds("export-progress").is_some());
    let cancel = visual
        .debug_bounds("cancel-export")
        .expect("visible cancel");
    visual.simulate_click(cancel.center(), gpui::Modifiers::default());
    assert!(cancelled.load(std::sync::atomic::Ordering::Acquire));
    leaf.update(visual, |leaf, cx| {
        leaf.export_in_progress = false;
        leaf.show_pane_notice("Exported · test.html", cx);
    });
    visual.run_until_parked();
    visual.update(|window, cx| window.draw(cx).clear());
    assert!(visual.debug_bounds("export-progress").is_none());
    assert!(visual.debug_bounds("status-bar-pane-notice").is_some());
    visual
        .executor()
        .advance_clock(std::time::Duration::from_secs(3));
    visual.run_until_parked();
    visual.update(|window, cx| window.draw(cx).clear());
    assert!(visual.debug_bounds("status-bar-pane-notice").is_none());
    root.update(visual, |root, cx| {
        root.focus_mode = true;
        cx.notify();
    });
    visual.update(|window, cx| {
        let leaf = leaf.downgrade();
        let handle = window.window_handle();
        cx.spawn(async move |cx| {
            super::show_export_completed(&leaf, Path::new("test.html"), handle, cx);
        })
        .detach();
    });
    visual.run_until_parked();
    assert!(
        visual.has_pending_prompt(),
        "hidden shell status needs completion dialog"
    );
    visual.simulate_prompt_answer("Ok");
}

/// 无后缀与误填后缀都必须映射到将被确认的真实导出目标，不能误覆盖 Markdown 源文件。
#[test]
fn export_file_name_matches_selected_format() {
    for (chosen, expected) in [
        ("note", "note.pdf"),
        ("note.md", "note.pdf"),
        ("note.pdf", "note.pdf"),
        ("note.PDF", "note.PDF"),
    ] {
        assert_eq!(
            super::normalized_export_path(PathBuf::from(chosen), "pdf"),
            PathBuf::from(expected)
        );
    }
}

/// 取消与写盘的公开结果必须互斥：接受取消后不能再替换文件，进入提交后不能声称正在取消。
#[test]
fn export_cancel_and_commit_are_mutually_exclusive() {
    let cancelled = AtomicBool::new(false);
    let progress = ExportProgress::default();
    assert!(progress.request_cancel(&cancelled));
    assert!(!progress.begin_commit(&cancelled));
    assert!(!progress.is_committing());

    let cancelled = AtomicBool::new(false);
    let progress = ExportProgress::default();
    assert!(progress.begin_commit(&cancelled));
    assert!(progress.is_committing());
    assert!(!progress.request_cancel(&cancelled));
    assert!(!cancelled.load(std::sync::atomic::Ordering::Acquire));
}

/// 浏览器诊断给普通用户可执行的提示，不将程序路径和 websocket 启动日志塞进弹窗。
#[test]
fn export_browser_failures_have_localized_recovery_guidance() {
    let strings = crate::i18n::I18nStrings::zh_cn();
    let message = export_failure_message(
        "failed to launch Chromium for PDF export: websocket",
        &strings,
    );
    assert!(message.contains("重试"));
    assert!(message.contains("HTML"));
    assert!(!message.contains("websocket"));
    assert!(
        export_failure_message("PDF export timed out while waiting for Chromium", &strings)
            .contains("超时")
    );
    assert!(
        export_failure_message("No Chromium-compatible browser was found", &strings)
            .contains("安装")
    );
    let message = export_failure_message(
        "cannot write C:/Chromium/note.pdf: permission denied",
        &strings,
    );
    assert!(message.contains("保存位置"));
    assert!(!message.contains("浏览器"));
    let message = export_failure_message(
        "原子写入 C:/验收.html 在 persist-temporary 阶段失败: 拒绝访问",
        &strings,
    );
    assert!(message.contains("占用"));
    assert!(!message.contains("persist-temporary"));
}

/// 最终写入失败后允许使用新目标重试，既有目标与文档内容均保持可恢复。
#[test]
fn failed_html_export_can_retry_to_another_path() -> anyhow::Result<()> {
    let root = std::env::temp_dir().join(format!("gmark-export-retry-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&root)?;
    let cancelled = AtomicBool::new(false);
    let result = Editor::write_export_bytes_cancellable(
        ExportFormat::Html,
        "# retry",
        &Theme::default_theme(),
        "Doc",
        &root,
        None,
        &cancelled,
    );
    assert!(matches!(result, ExportTaskResult::Failed(_)));
    let target = root.join("retry.html");
    let result = Editor::write_export_bytes_cancellable(
        ExportFormat::Html,
        "# retry",
        &Theme::default_theme(),
        "Doc",
        &target,
        None,
        &cancelled,
    );
    assert!(matches!(result, ExportTaskResult::Complete));
    assert!(std::fs::read_to_string(&target)?.contains("retry"));
    std::fs::remove_file(target)?;
    std::fs::remove_dir(root)?;
    Ok(())
}

#[test]
fn png_export_uses_png_extension() {
    assert_eq!(ExportFormat::Png.extension(), "png");
}

#[test]
fn cancelled_export_preserves_existing_target() {
    let path =
        std::env::temp_dir().join(format!("gmark-cancel-export-{}.html", uuid::Uuid::new_v4()));
    std::fs::write(&path, b"existing").unwrap();
    let cancelled = AtomicBool::new(true);

    let result = Editor::write_export_bytes_cancellable(
        ExportFormat::Html,
        "# replacement",
        &Theme::default_theme(),
        "Doc",
        &path,
        None,
        &cancelled,
    );
    assert!(matches!(result, ExportTaskResult::Cancelled));
    assert_eq!(std::fs::read(&path).unwrap(), b"existing");
    let _ = std::fs::remove_file(path);
}

#[test]
fn mermaid_svg_export_uses_document_directory_and_stable_suggested_name() {
    let document = Path::new("C:/work/notes/architecture.md");
    let (directory, name) = mermaid_svg_export_defaults(Some(document));
    assert_eq!(directory, PathBuf::from("C:/work/notes"));
    assert_eq!(name, "architecture-mermaid.svg");

    let (_, untitled_name) = mermaid_svg_export_defaults(None);
    assert_eq!(untitled_name, "untitled-mermaid.svg");
}

#[test]
fn mermaid_svg_export_writes_the_current_vector_bytes_atomically() {
    let path =
        std::env::temp_dir().join(format!("gmark-mermaid-export-{}.svg", uuid::Uuid::new_v4()));
    write_mermaid_svg(&path, "<svg viewBox=\"0 0 1 1\"></svg>").unwrap();
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "<svg viewBox=\"0 0 1 1\"></svg>"
    );
    let _ = std::fs::remove_file(path);
}
