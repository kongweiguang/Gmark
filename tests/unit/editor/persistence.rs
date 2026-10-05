// @author kongweiguang

use std::path::Path;

use super::{
    build_external_conflict_preview, safe_code_fence, safe_code_fence_with_info, text_line_count,
    truncate_conflict_line,
};

#[test]
fn safe_code_fence_is_longer_than_any_inner_backtick_run() {
    assert_eq!(safe_code_fence("plain code"), "```");
    assert_eq!(safe_code_fence("```\ncode"), "~~~");
    assert_eq!(safe_code_fence("value = `````"), "~~~");
    assert_eq!(safe_code_fence("```\n~~~"), "~~~~");
}

#[test]
fn safe_code_fence_with_info_uses_tildes_when_info_contains_backticks() {
    assert_eq!(
        safe_code_fence_with_info("plain code", Some("we`rd")),
        "~~~"
    );
    assert_eq!(
        safe_code_fence_with_info("plain\n~~~\ncode", Some("we`rd")),
        "~~~~"
    );
    assert_eq!(safe_code_fence_with_info("plain code", Some("rust")), "```");
}

#[test]
fn external_conflict_preview_reports_metadata_only_change() {
    let preview = build_external_conflict_preview(
        Path::new("notes.md"),
        "alpha\nbeta",
        "alpha\nbeta",
        10,
        None,
    );

    assert_eq!(preview.path, "notes.md");
    assert_eq!(preview.first_difference_line, None);
    assert_eq!(preview.local_line_count, 2);
    assert_eq!(preview.disk_line_count, 2);
    assert_eq!(preview.local_bytes, 10);
    assert_eq!(preview.disk_bytes, 10);
    assert!(preview.disk_error.is_none());
}

/// 验证 CRLF 磁盘换行不会把未变化的首行误报为冲突差异。
#[test]
fn external_conflict_preview_ignores_crlf_when_finding_first_difference() {
    let local = "<!-- @author kongweiguang -->\nlocal";
    let disk = "<!-- @author kongweiguang -->\r\n输入法验收（外部重载）";
    let preview =
        build_external_conflict_preview(Path::new("notes.md"), local, disk, disk.len(), None);

    assert_eq!(preview.first_difference_line, Some(2));
    assert_eq!(preview.local_line, "local");
    assert_eq!(preview.disk_line, "输入法验收（外部重载）");
    assert_eq!(preview.local_line_count, 2);
    assert_eq!(preview.disk_line_count, 2);
}

#[test]
fn external_conflict_preview_handles_missing_or_unreadable_disk_file() {
    let preview = build_external_conflict_preview(
        Path::new("missing.md"),
        "local",
        "",
        0,
        Some("file not found".to_owned()),
    );

    assert_eq!(preview.first_difference_line, None);
    assert_eq!(preview.local_line_count, 1);
    assert_eq!(preview.disk_line_count, 0);
    assert_eq!(preview.disk_bytes, 0);
    assert_eq!(preview.disk_error.as_deref(), Some("file not found"));
}

#[test]
fn external_conflict_preview_truncates_unicode_by_character_boundary() {
    let local = "中".repeat(241);
    let preview = build_external_conflict_preview(Path::new("long.md"), &local, "disk", 4, None);

    assert_eq!(preview.first_difference_line, Some(1));
    assert_eq!(preview.local_line.chars().count(), 243);
    assert!(preview.local_line.ends_with("..."));
    assert_eq!(preview.disk_line, "disk");
    assert_eq!(truncate_conflict_line("short"), "short");
}

#[test]
fn conflict_line_count_matches_editor_display_semantics() {
    assert_eq!(text_line_count(""), 0);
    assert_eq!(text_line_count("one"), 1);
    assert_eq!(text_line_count("one\n"), 2);
    assert_eq!(text_line_count("one\ntwo"), 2);
}
