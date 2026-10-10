// @author kongweiguang

use std::fs;
use std::path::Path;
use std::sync::atomic::AtomicBool;

use gmark_export::{
    ClipboardSelection, ExportCancellation, ExportCancellationHandle, ExportTheme,
    export_clipboard_fragment, prepare_html_resources, render_html, render_pdf_cancellable,
    render_png_cancellable,
};
use gmark_markdown::parse_markdown;
use uuid::Uuid;

/// 导出语言元数据必须跟随正文文字，避免中文文件被读屏当作英文朗读。
#[test]
fn html_export_labels_chinese_content_language() {
    let html = render_html(
        "# 中文导出\n\n可分享的文件。",
        &ExportTheme::default(),
        "Doc",
    );
    assert!(html.contains("<html lang=\"zh\">"));
    for (text, language) in [("# 日本語の文書", "ja"), ("# 한글 文書", "ko")] {
        let html = render_html(text, &ExportTheme::default(), "Doc");
        assert!(html.contains(&format!("<html lang=\"{language}\">")));
    }
}

#[test]
fn html_export_projects_toc_math_mermaid_and_safe_html() {
    let html = render_html(
        "[TOC]\n\n# Title\n\n$$\nx^2\n$$\n\n```mermaid\nflowchart LR\nA --> B\n```\n\n<span style=\"color:blue; background-image:url(javascript:bad)\">safe</span>",
        &ExportTheme::default(),
        "Doc",
    );

    assert!(html.contains("<nav class=\"gmark-toc\""));
    assert!(html.contains("<h1 id=\"title\">Title</h1>"));
    assert!(html.contains("class=\"vlt-math\""));
    assert!(html.contains("class=\"vlt-mermaid\""));
    assert!(html.contains("color: rgba(0,0,255,1.000);"));
    assert!(!html.contains("background-image"));
}

#[test]
fn clipboard_and_export_share_the_markdown_visible_text_contract() {
    let markdown =
        "# Title\n\nA **bold** [link](https://example.test)\n\n| A | B |\n|---|---|\n| C | D |";
    let visible = parse_markdown(markdown).visible_text_projection();
    let fragment = export_clipboard_fragment(
        ClipboardSelection::Markdown {
            markdown: markdown.to_owned(),
        },
        &ExportTheme::default(),
        None,
    );
    assert!(visible.text.contains("Title"));
    assert!(fragment.plain_text.contains("bold"));
    assert!(fragment.html.contains("<strong>bold</strong>"));
}

#[test]
fn html_export_projects_unicode_toc_entries_without_frontmatter_or_fence_headings() {
    let html = render_html(
        "[TOC]\n\n---\ntitle: ignored\n---\n\n# 你好 **gmark**\n## 你好 gmark\nTitle\n-----\n~~~md\n# ignored\n~~~",
        &ExportTheme::default(),
        "Doc",
    );

    assert!(html.contains("href=\"#你好-gmark\""));
    assert!(html.contains("href=\"#你好-gmark-1\""));
    assert!(html.contains("href=\"#title\""));
    assert!(!html.contains("href=\"#ignored\""));
}

#[test]
fn html_export_replaces_invalid_mermaid_with_a_safe_error_projection() {
    let html = render_html(
        "~~~mermaid\nnot a real mermaid diagram ::::\n~~~",
        &ExportTheme::default(),
        "Doc",
    );

    assert!(html.contains("vlt-mermaid-error"));
    assert!(!html.contains("<script"));
}

#[test]
fn html_export_sanitizes_raw_html_events_without_stripping_safe_markup_or_the_shell() {
    let html = render_html(
        "before <script>alert('inline')</script> after\n\n<div onclick=\"alert('block')\">block</div>\n\n<a href=\"java&#x73;cript:alert('url')\">unsafe link</a>\n\nprefix <span class=\"safe-inline\" title=\"still safe\">benign</span> suffix",
        &ExportTheme::default(),
        "Doc",
    );
    let lower = html.to_ascii_lowercase();

    assert!(!lower.contains("<script"));
    assert!(!lower.contains("onclick"));
    assert!(!lower.contains("javascript:"));
    assert!(!lower.contains("href=\"java"));
    assert!(html.contains("<span class=\"safe-inline\" title=\"still safe\">benign</span>"));
    assert!(html.contains("<!doctype html>"));
    assert!(html.contains("<style>"));
    assert!(html.contains("<main class=\"vlt-document\">"));
}

#[test]
fn html_export_inlines_local_images() {
    let root = std::env::temp_dir().join(format!("gmark-export-image-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    fs::write(
        root.join("diagram.svg"),
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 1 1\"></svg>",
    )
    .unwrap();

    let html = gmark_export::render_html_with_base_dir(
        "![diagram](diagram.svg)",
        &ExportTheme::default(),
        "Doc",
        Some(&root),
    );
    let _ = fs::remove_dir_all(&root);

    assert!(html.contains("data:image/svg+xml;base64,"));
    assert!(!html.contains("src=\"diagram.svg\""));
}

#[test]
fn resource_cleanup_removes_only_owned_files() {
    let root = std::env::temp_dir().join(format!("gmark-export-resource-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("demo.pdf"), b"fixture").unwrap();
    let output = root.join("note.html");
    let cancelled = AtomicBool::new(false);

    let prepared = prepare_html_resources(
        "[Demo](./demo.pdf \"gmark:resource\")",
        Some(&root),
        &output,
        &cancelled,
    )
    .unwrap();
    assert!(root.join("note.assets/demo.pdf").is_file());
    assert!(prepared.markdown.contains("note.assets/demo.pdf"));

    prepared.cleanup_created();
    assert!(!root.join("note.assets/demo.pdf").exists());
    let _ = fs::remove_dir_all(&root);
}

#[cfg(any(unix, windows))]
#[test]
fn resource_export_rejects_a_precreated_redirected_assets_directory() {
    let root = std::env::temp_dir().join(format!("gmark-export-resource-{}", Uuid::new_v4()));
    let outside = std::env::temp_dir().join(format!("gmark-export-outside-{}", Uuid::new_v4()));
    let assets = root.join("note.assets");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&outside).unwrap();
    fs::write(root.join("demo.pdf"), b"fixture").unwrap();
    create_directory_link(&assets, &outside).unwrap();

    let cancelled = AtomicBool::new(false);
    let result = prepare_html_resources(
        "[Demo](./demo.pdf \"gmark:resource\")",
        Some(&root),
        &root.join("note.html"),
        &cancelled,
    );
    let wrote_outside = outside.join("demo.pdf").exists();

    let _ = remove_directory_link(&assets);
    let _ = fs::remove_dir_all(&root);
    let _ = fs::remove_dir_all(&outside);

    assert!(result.is_err());
    assert!(!wrote_outside);
}

#[cfg(unix)]
fn create_directory_link(link: &Path, target: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(windows)]
fn create_directory_link(link: &Path, target: &Path) -> std::io::Result<()> {
    let output = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(link)
        .arg(target)
        .output()?;
    if output.status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other(format!(
            "failed to create test junction: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )))
    }
}

#[cfg(unix)]
fn remove_directory_link(link: &Path) -> std::io::Result<()> {
    fs::remove_file(link)
}

#[cfg(windows)]
fn remove_directory_link(link: &Path) -> std::io::Result<()> {
    fs::remove_dir(link)
}

#[test]
fn cancellation_short_circuits_chromium_work_without_launching_a_browser() {
    let cancelled = AtomicBool::new(true);
    let theme = ExportTheme::default();

    assert_eq!(
        render_png_cancellable("# Title", &theme, "Doc", None, &cancelled)
            .unwrap_err()
            .to_string(),
        "export cancelled"
    );
    assert_eq!(
        render_pdf_cancellable("# Title", &theme, "Doc", None, &cancelled)
            .unwrap_err()
            .to_string(),
        "export cancelled"
    );

    let handle = ExportCancellationHandle::default();
    assert!(!handle.is_cancelled());
    handle.cancel();
    assert!(handle.is_cancelled());
}
