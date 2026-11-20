// @author kongweiguang

/// 整份源码仅作为编辑投影，资源标记不得触发段落资源实体或改变原文。
#[test]
fn source_document_record_preserves_resource_markdown_as_plain_text() {
    let source = "[spec](./spec.pdf \"gmark:resource\")";
    let mut record = BlockRecord::source_document(source);
    assert!(record.resource.is_none());
    assert_eq!(record.title.visible_text(), source);
    assert!(BlockRecord::paragraph(source).resource.is_some());
    record.set_title(InlineTextTree::plain(source));
    assert!(record.resource.is_none());
    assert_eq!(record.title.visible_text(), source);
}

use super::*;

#[test]
fn projects_editable_blocks_from_shared_markdown_values() {
    let document = gmark_markdown::parse_markdown(
        "# Title\n\n[clip](clip.mp4 \"gmark:resource\")\n\n| A | B |\n| --- | ---: |\n| 1 | 2 |",
    );

    let heading = BlockRecord::from_markdown_value(&document.blocks[0]).expect("heading");
    assert_eq!(heading.kind, BlockKind::Heading { level: 1 });
    assert_eq!(heading.title.visible_text(), "Title");

    let resource = BlockRecord::from_markdown_value(&document.blocks[1]).expect("resource");
    assert!(resource.resource.is_some());

    let table = BlockRecord::from_markdown_value(&document.blocks[2]).expect("table");
    assert_eq!(table.kind, BlockKind::Table);
    assert_eq!(
        table.table.expect("table value").alignments[1],
        crate::components::TableColumnAlignment::Right
    );
}

#[test]
fn identifies_only_complete_yaml_frontmatter_raw_blocks() {
    assert!(BlockRecord::raw_markdown("---\nname: example\n---").is_yaml_frontmatter());
    assert!(BlockRecord::raw_markdown("---\nname: example\n...").is_yaml_frontmatter());
    assert!(!BlockRecord::raw_markdown("---\nname: incomplete").is_yaml_frontmatter());
    assert!(!BlockRecord::paragraph("---\nname: example\n---").is_yaml_frontmatter());
}

#[test]
fn detects_markdown_shortcuts() {
    assert_eq!(
        BlockKind::detect_markdown_shortcut("- item"),
        Some((BlockKind::BulletedListItem, 2))
    );
    assert_eq!(
        BlockKind::detect_markdown_shortcut("1. item"),
        Some((BlockKind::NumberedListItem, 3))
    );
    assert_eq!(
        BlockKind::detect_markdown_shortcut("12. item"),
        Some((BlockKind::NumberedListItem, 4))
    );
    assert_eq!(
        BlockKind::detect_markdown_shortcut("1) item"),
        Some((BlockKind::NumberedListItem, 3))
    );
    assert_eq!(
        BlockKind::detect_markdown_shortcut("12)\titem"),
        Some((BlockKind::NumberedListItem, 4))
    );
    assert_eq!(
        BlockKind::detect_markdown_shortcut("1234567890) item"),
        None
    );
    assert_eq!(
        BlockKind::detect_markdown_shortcut("# heading"),
        Some((BlockKind::Heading { level: 1 }, 2))
    );
    assert_eq!(
        BlockKind::detect_markdown_shortcut("## heading"),
        Some((BlockKind::Heading { level: 2 }, 3))
    );
    assert_eq!(
        BlockKind::detect_markdown_shortcut("### heading"),
        Some((BlockKind::Heading { level: 3 }, 4))
    );
    assert_eq!(
        BlockKind::detect_markdown_shortcut("#### heading"),
        Some((BlockKind::Heading { level: 4 }, 5))
    );
    assert_eq!(
        BlockKind::detect_markdown_shortcut("##### heading"),
        Some((BlockKind::Heading { level: 5 }, 6))
    );
    assert_eq!(
        BlockKind::detect_markdown_shortcut("###### heading"),
        Some((BlockKind::Heading { level: 6 }, 7))
    );
    assert_eq!(
        BlockKind::detect_markdown_shortcut("- [ ] task"),
        Some((BlockKind::TaskListItem { checked: false }, 6))
    );
    assert_eq!(
        BlockKind::detect_markdown_shortcut("- [x] task"),
        Some((BlockKind::TaskListItem { checked: true }, 6))
    );
    assert_eq!(
        BlockKind::detect_markdown_shortcut("* item"),
        Some((BlockKind::BulletedListItem, 2))
    );
    assert_eq!(
        BlockKind::detect_markdown_shortcut("+ item"),
        Some((BlockKind::BulletedListItem, 2))
    );
    assert_eq!(BlockKind::detect_markdown_shortcut("#no-space"), None);
    assert_eq!(BlockKind::detect_markdown_shortcut("```"), None);
    assert_eq!(
        BlockKind::detect_markdown_shortcut("> quote"),
        Some((BlockKind::Quote, 2))
    );
    assert_eq!(BlockKind::detect_markdown_shortcut(">no-space"), None);
}

#[test]
fn parses_separator_lines() {
    assert!(BlockKind::parse_separator_line("---"));
    assert!(BlockKind::parse_separator_line("----"));
    assert!(BlockKind::parse_separator_line("***"));
    assert!(BlockKind::parse_separator_line("_ _ _"));
    assert!(BlockKind::parse_separator_line(" - - - "));
    assert!(!BlockKind::parse_separator_line("--"));
    assert!(BlockKind::parse_separator_line(" ---"));
    assert!(!BlockKind::parse_separator_line("---x"));
}

#[test]
fn parses_code_fence_openings() {
    assert_eq!(
        BlockKind::parse_code_fence_opening("```rust"),
        Some(CodeFenceOpening {
            ch: '`',
            len: 3,
            language: Some("rust".into()),
        })
    );
    assert_eq!(
        BlockKind::parse_code_fence_opening("~~~ts"),
        Some(CodeFenceOpening {
            ch: '~',
            len: 3,
            language: Some("ts".into()),
        })
    );
    assert_eq!(
        BlockKind::parse_code_fence_opening("```"),
        Some(CodeFenceOpening {
            ch: '`',
            len: 3,
            language: None,
        })
    );
    assert_eq!(BlockKind::parse_code_fence_opening("``"), None);
    assert_eq!(BlockKind::parse_code_fence_opening("```ru`st"), None);
}

#[test]
fn parses_task_list_item_prefixes() {
    assert_eq!(
        BlockKind::parse_task_list_item_prefix("[ ] a"),
        Some((false, 4))
    );
    assert_eq!(
        BlockKind::parse_task_list_item_prefix("[x] a"),
        Some((true, 4))
    );
    assert_eq!(
        BlockKind::parse_task_list_item_prefix("[X] a"),
        Some((true, 4))
    );
    assert_eq!(BlockKind::parse_task_list_item_prefix("[a] a"), None);
}

#[test]
fn serializes_supported_block_kinds() {
    let list = BlockRecord::new(
        BlockKind::BulletedListItem,
        InlineTextTree::from_markdown("*item*"),
    );
    let numbered = BlockRecord::new(
        BlockKind::NumberedListItem,
        InlineTextTree::from_markdown("step"),
    );
    let task = BlockRecord::new(
        BlockKind::TaskListItem { checked: true },
        InlineTextTree::from_markdown("done"),
    );
    let heading = BlockRecord::new(
        BlockKind::Heading { level: 2 },
        InlineTextTree::from_markdown("**title**"),
    );
    let quote = BlockRecord::new(BlockKind::Quote, InlineTextTree::plain("quoted text"));
    let paragraph = BlockRecord::paragraph("plain");
    let comment = BlockRecord::comment("<!--\ncomment\n-->");

    assert_eq!(list.markdown_line(0, None), "- *item*");
    assert_eq!(list.markdown_line(2, None), "    - *item*");
    assert_eq!(task.markdown_line(0, None), "- [x] done");
    assert_eq!(task.markdown_line(2, None), "    - [x] done");
    assert_eq!(numbered.markdown_line(0, Some(3)), "3. step");
    assert_eq!(numbered.markdown_line(2, Some(12)), "    12. step");
    assert_eq!(heading.markdown_line(0, None), "## **title**");
    assert_eq!(quote.markdown_line(0, None), "> quoted text");
    assert_eq!(quote.markdown_line(2, None), "    > quoted text");
    assert_eq!(paragraph.markdown_line(1, None), "  plain");
    assert_eq!(comment.markdown_line(0, None), "<!--\ncomment\n-->");
    assert_eq!(comment.markdown_line(1, None), "  <!--\n  comment\n  -->");
}

#[test]
fn standalone_image_markdown_line_preserves_underscores() {
    let markdown = "![1.1_进制转换例子](./NetworkEngineerSummer.assets/1.1_进制转换例子.jpg)";
    let paragraph = BlockRecord::paragraph(markdown);

    assert_eq!(paragraph.markdown_line(0, None), markdown);
}

#[test]
fn quote_serializes_back_to_markdown() {
    let record = BlockRecord::new(BlockKind::Quote, InlineTextTree::plain("text"));
    let line = record.markdown_line(0, None);
    assert_eq!(line, "> text");
}

#[test]
fn resource_cards_preserve_container_markers_for_lists_quotes_and_callouts() {
    let syntax = r#"[spec](./spec.pdf "gmark:resource")"#;
    let list = BlockRecord::new(
        BlockKind::BulletedListItem,
        InlineTextTree::from_markdown(syntax),
    );
    let quote = BlockRecord::new(BlockKind::Quote, InlineTextTree::from_markdown(syntax));
    let callout = BlockRecord::new(
        BlockKind::Callout(CalloutVariant::Note),
        InlineTextTree::from_markdown(syntax),
    );

    assert!(list.resource.is_some());
    assert!(quote.resource.is_some());
    assert!(callout.resource.is_some());
    assert_eq!(list.markdown_line(0, None), format!("- {syntax}"));
    assert_eq!(quote.markdown_line(0, None), format!("> {syntax}"));
    assert_eq!(
        callout.markdown_line(0, None),
        format!("> [!NOTE] {syntax}")
    );
}

#[test]
fn parses_h2_and_h3_lines_with_correct_levels() {
    let h2 = BlockKind::parse_atx_heading_line("## hello");
    assert_eq!(h2, Some((2, "hello".to_string())));

    let h3 = BlockKind::parse_atx_heading_line("### hello");
    assert_eq!(h3, Some((3, "hello".to_string())));
}

#[test]
fn parses_atx_headings_with_closing_hashes_and_setext_underlines() {
    let atx = BlockKind::parse_atx_heading_line("  ### title ######");
    assert_eq!(atx, Some((3, "title".to_string())));

    assert_eq!(BlockKind::parse_setext_underline("==="), Some(1));
    assert_eq!(BlockKind::parse_setext_underline("---"), Some(2));
    assert_eq!(BlockKind::parse_setext_underline("- - -"), None);
}

#[test]
fn code_block_kind_stores_language() {
    let kind = BlockKind::CodeBlock {
        language: Some(SharedString::from("rust")),
    };
    assert!(kind.is_code_block());
    assert!(!kind.is_list_item());

    let no_lang = BlockKind::CodeBlock { language: None };
    assert!(no_lang.is_code_block());
}

#[test]
fn code_block_markdown_line_returns_plain_content() {
    let record = BlockRecord::new(
        BlockKind::CodeBlock {
            language: Some("rust".into()),
        },
        InlineTextTree::plain("let x = 1;\nprintln!(\"hi\");"),
    );
    // markdown_line returns bare content; fences are added by persistence layer.
    let line = record.markdown_line(0, None);
    assert_eq!(line, "let x = 1;\nprintln!(\"hi\");");
}

#[test]
fn separator_markdown_line_round_trips() {
    let record = BlockRecord::new(BlockKind::Separator, InlineTextTree::plain(String::new()));
    assert_eq!(record.markdown_line(0, None), "---");
    assert!(BlockKind::parse_separator_line("---"));
}

#[test]
fn task_list_serializes_canonical_markdown() {
    let record = BlockRecord::new(
        BlockKind::TaskListItem { checked: false },
        InlineTextTree::plain("todo"),
    );
    assert_eq!(record.markdown_line(0, None), "- [ ] todo");
}
