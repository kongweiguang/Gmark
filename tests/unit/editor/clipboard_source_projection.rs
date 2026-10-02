// @author kongweiguang

use super::*;
use gmark_markdown::{BlockKind, InlineKind};

/// 部分行内代码的纯文本与富文本必须同范围，避免复制时带上未选邻文。
#[test]
fn partial_inline_code_selection_clips_visible_and_markdown_values() {
    let source = "before `alpha 👨‍👩‍👧‍👦 beta` after";
    let selected_value = "👨‍👩‍👧‍👦";
    let start = source
        .find(selected_value)
        .expect("selected emoji in source");
    let selection = virtualized_clipboard_selection(source, start..start + selected_value.len())
        .expect("partial inline code selection");
    assert_eq!(selection.visible_text, selected_value);
    let rich = gmark_markdown::parse_markdown(&selection.markdown);
    assert!(
        matches!(&rich.blocks[0].inlines[0].kind, InlineKind::Code(value) if value == selected_value)
    );
    assert!(!selection.markdown.contains("alpha"));
    assert!(!selection.markdown.contains("beta"));
}

/// 富文本序列化前需映射原 CRLF 字节范围，不能按规范化后的 LF 偏移截取代码。
#[test]
fn partial_fenced_code_selection_maps_crlf_endpoints() {
    let source = "```rust\r\nalpha\r\nbeta\r\n```";
    let selected_value = "beta";
    let start = source
        .find(selected_value)
        .expect("selected code in source");
    let selection = virtualized_clipboard_selection(source, start..start + selected_value.len())
        .expect("partial fenced code selection");
    assert_eq!(selection.visible_text, selected_value);
    let rich = gmark_markdown::parse_markdown(&selection.markdown);
    let code_block = rich
        .blocks
        .iter()
        .find(|block| matches!(&block.kind, BlockKind::CodeBlock(_)))
        .expect("serialized code block");
    assert_eq!(
        code_block.plain_text().trim_end_matches(['\r', '\n']),
        selected_value
    );
    assert!(!selection.markdown.contains("alpha"));
}

/// 部分链接标签保留目标，但纯文本与富文本都只包含已选文字。
#[test]
fn partial_link_label_selection_keeps_matching_plain_text() {
    let source = "before [visible label](https://example.test) after";
    let selected_value = "sible la";
    let start = source
        .find(selected_value)
        .expect("selected link label in source");
    let selection = virtualized_clipboard_selection(source, start..start + selected_value.len())
        .expect("partial link label selection");
    assert_eq!(selection.visible_text, selected_value);
    assert!(
        selection
            .markdown
            .contains("[sible la](https://example.test)")
    );
    let rich = gmark_markdown::parse_markdown(&selection.markdown);
    assert_eq!(rich.blocks[0].plain_text(), selected_value);
}
