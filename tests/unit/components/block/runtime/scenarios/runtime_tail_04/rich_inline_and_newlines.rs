// @author kongweiguang

#[gpui::test]
async fn inline_math_focus_stays_rendered_rich_and_keeps_links(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::Paragraph,
                InlineTextTree::from_markdown("**bold** $x^2$ [repo](https://example.com)"),
            ),
        )
    });

    block.update(cx, |block, cx| {
        // The math source is shown inline (`$x^2$`) while bold and the link stay
        // collapsed; the block never falls back to raw Markdown editing.
        assert_eq!(block.display_text(), "bold $x^2$ repo");

        // Focusing with the caret inside the math keeps the rendered-rich
        // projection rather than dumping the whole block to raw source, so the
        // link in the same block keeps its link attribute.
        let caret = "bold $".len();
        block.move_to(caret, cx);
        block.sync_inline_projection_for_focus(true);
        assert!(!block.uses_raw_text_editing());
        assert!(block.record.title.has_mixed_inline_visuals());
        assert!(block.record.title.has_inline_links());
        assert!(
            block.inline_spans().iter().any(|span| span.link.is_some()),
            "link must stay styled while editing the math in the same block"
        );
        assert_eq!(
            block.record.title.serialize_markdown(),
            "**bold** $x^2$ [repo](https://example.com)"
        );
    });
}

#[gpui::test]
async fn script_spans_focus_stay_rendered_rich(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::Paragraph,
                InlineTextTree::from_markdown("x^2^ and H~2~O"),
            ),
        )
    });

    block.update(cx, |block, _cx| {
        assert_eq!(block.display_text(), "x2 and H2O");
        assert_eq!(block.inline_spans()[0].style.script, InlineScript::Normal);
        assert_eq!(
            block.inline_spans()[1].style.script,
            InlineScript::Superscript
        );
        assert!(!block.uses_raw_text_editing());
        assert_eq!(block.display_text(), "x2 and H2O");
        assert_eq!(block.record.title.serialize_markdown(), "x^2^ and H~2~O");
    });
}

#[gpui::test]
async fn link_anchor_emphasis_delimiters_are_revealed_when_caret_inside(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::Paragraph,
                InlineTextTree::from_markdown("[**bold**](https://example.com)"),
            ),
        )
    });

    block.update(cx, |block, cx| {
        // Collapsed, only the styled anchor text is shown.
        assert_eq!(block.display_text(), "bold");

        // With the caret inside the bold anchor text, the projection reveals both
        // the link syntax and the anchor's own `**` emphasis markers, so they can
        // be edited instead of staying invisible.
        block.move_to(2, cx);
        block.sync_inline_projection_for_focus(true);
        assert_eq!(block.display_text(), "[**bold**](https://example.com)");
    });
}

#[gpui::test]
async fn mermaid_block_uses_raw_text_editing(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let markdown = "```mermaid\nflowchart LR\nA --> B\n```";
    let block = cx.new(|cx| Block::with_record(cx, BlockRecord::mermaid(markdown)));

    block.update(cx, |block, _cx| {
        assert_eq!(block.kind(), BlockKind::MermaidBlock);
        assert!(block.uses_raw_text_editing());
        assert_eq!(block.display_text(), markdown);
        assert_eq!(block.record.markdown_line(0, None), markdown);
    });
}

#[gpui::test]
async fn enter_inside_projected_inline_code_inserts_hard_line_without_splitting(
    cx: &mut TestAppContext,
) {
    let cx = cx.add_empty_window();
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::Paragraph,
                InlineTextTree::from_markdown("`line 1\nline 2`"),
            ),
        )
    });

    block.update(cx, |block, cx| {
        let offset = "line 1\n".len();
        block.selected_range = offset..offset;
        block.sync_inline_projection_for_focus(true);
        cx.notify();
    });

    cx.update(|window, cx| {
        block.update(cx, |block, block_cx| {
            block.on_newline(&Newline, window, block_cx);
        });
    });

    block.read_with(cx, |block, _cx| {
        let text = "line 1\n\nline 2";
        assert_eq!(block.kind(), BlockKind::Paragraph);
        assert_eq!(block.record.title.visible_text(), text);
        assert!(
            block
                .record
                .title
                .render_cache()
                .spans()
                .iter()
                .any(|span| span.style.code && span.range == (0..text.len()))
        );
    });
}

#[gpui::test]
async fn enter_outside_inline_code_still_splits_paragraph(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::Paragraph,
                InlineTextTree::from_markdown("alpha beta"),
            ),
        )
    });

    block.update(cx, |block, cx| {
        block.selected_range = "alpha".len().."alpha".len();
        cx.notify();
    });

    cx.update(|window, cx| {
        block.update(cx, |block, block_cx| {
            block.on_newline(&Newline, window, block_cx);
        });
    });

    block.read_with(cx, |block, _cx| {
        assert_eq!(block.kind(), BlockKind::Paragraph);
        assert_eq!(block.display_text(), "alpha");
        assert_eq!(block.selected_range, "alpha".len().."alpha".len());
    });
}

#[gpui::test]
async fn enter_inside_comment_block_inserts_hard_line_without_splitting(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::comment("<!--\n**not bold** [not link](https://example.com)\n-->"),
        )
    });

    block.update(cx, |block, cx| {
        let offset = "<!--\n".len();
        block.selected_range = offset..offset;
        cx.notify();
    });

    cx.update(|window, cx| {
        block.update(cx, |block, block_cx| {
            block.on_newline(&Newline, window, block_cx);
        });
    });

    block.read_with(cx, |block, _cx| {
        assert_eq!(block.kind(), BlockKind::Comment);
        assert_eq!(
            block.display_text(),
            "<!--\n\n**not bold** [not link](https://example.com)\n-->"
        );
        assert_eq!(block.inline_spans().len(), 1);
        assert_eq!(block.inline_spans()[0].range, 0..block.display_text().len());
        assert_eq!(block.inline_spans()[0].style, InlineStyle::default());
    });
}

#[gpui::test]
/// 块快捷输入不涉及行内闭合符，仍应直接生成任务项。
async fn paragraph_shortcut_creates_task_item_directly(cx: &mut TestAppContext) {
    let block = cx.new(|cx| Block::with_record(cx, BlockRecord::paragraph(String::new())));

    block.update(cx, |block, cx| {
        block.apply_title_edit(
            InlineTextTree::plain("- [x] task"),
            10,
            None,
            None,
            None,
            false,
            false,
            cx,
        );
    });

    let kind = block.read_with(cx, |block, _cx| block.kind());
    let text = block.read_with(cx, |block, _cx| block.display_text().to_string());
    assert_eq!(kind, BlockKind::TaskListItem { checked: true });
    assert_eq!(text, "task");
}

#[gpui::test]
/// 光标亲和性扩展不能改变有序列表的快捷识别。
async fn paragraph_shortcut_creates_parenthesized_numbered_list_directly(cx: &mut TestAppContext) {
    let block = cx.new(|cx| Block::with_record(cx, BlockRecord::paragraph(String::new())));

    block.update(cx, |block, cx| {
        block.apply_title_edit(
            InlineTextTree::plain("1) item"),
            7,
            None,
            None,
            None,
            false,
            false,
            cx,
        );
    });

    let kind = block.read_with(cx, |block, _cx| block.kind());
    let text = block.read_with(cx, |block, _cx| block.display_text().to_string());
    assert_eq!(kind, BlockKind::NumberedListItem);
    assert_eq!(text, "item");
}

#[gpui::test]
/// 两阶段列表转换使用普通光标语义，不模拟行内闭合事件。
async fn bullet_shortcut_upgrades_to_task_item_after_box_prefix(cx: &mut TestAppContext) {
    let block = cx.new(|cx| Block::with_record(cx, BlockRecord::paragraph(String::new())));

    block.update(cx, |block, cx| {
        block.apply_title_edit(
            InlineTextTree::plain("- "),
            2,
            None,
            None,
            None,
            false,
            false,
            cx,
        );
    });
    let kind = block.read_with(cx, |block, _cx| block.kind());
    assert_eq!(kind, BlockKind::BulletedListItem);

    block.update(cx, |block, cx| {
        block.apply_title_edit(
            InlineTextTree::plain("[ ] "),
            4,
            None,
            None,
            None,
            false,
            false,
            cx,
        );
    });

    let kind = block.read_with(cx, |block, _cx| block.kind());
    let text = block.read_with(cx, |block, _cx| block.display_text().to_string());
    assert_eq!(kind, BlockKind::TaskListItem { checked: false });
    assert_eq!(text, "");
}
