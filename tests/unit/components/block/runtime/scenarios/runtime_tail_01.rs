// @author kongweiguang

/// Source 中的图片语法属于原始文本，编辑后不得触发 Live 图片卡片检测。
#[gpui::test]
async fn source_document_does_not_build_image_runtime(cx: &mut TestAppContext) {
    let block = cx.new(|cx| {
        let mut block = Block::with_record(cx, BlockRecord::source_document("![image](photo.png)"));
        block.set_source_document_mode();
        block.sync_render_cache();
        block
    });
    block.read_with(cx, |block, _| {
        assert!(!block.can_present_as_image());
        assert!(block.image_runtime().is_none());
        assert_eq!(block.display_text(), "![image](photo.png)");
    });
}

/// 后台高亮只能发布所属代次，切换语言后迟到的结果不能覆盖新状态。
#[gpui::test]
async fn large_source_highlight_discards_stale_result(cx: &mut TestAppContext) {
    let block = cx.new(|cx| {
        let mut block = Block::with_record(cx, BlockRecord::paragraph("# a\n".repeat(262_144)));
        block.set_source_document_mode();
        block
    });
    block.update(cx, |block, cx| {
        assert!(block.source_highlight_pending);
        assert!(block.code_highlight_result().is_none());
        block.schedule_source_highlight(cx);
        assert!(block.source_highlight_running);
        block.set_source_syntax_language(None);
        assert!(!block.source_highlight_pending);
    });
    cx.run_until_parked();
    block.read_with(cx, |block, _| {
        assert!(!block.source_highlight_running);
        assert!(block.code_highlight_result().is_none());
    });
}

/// 常驻源码重绘必须复用布局，而语法变化必须重新生成颜色，避免长文档每次闪烁都重排。
#[gpui::test]
async fn resident_source_reuses_layout_until_syntax_changes(cx: &mut TestAppContext) {
    cx.update(|cx| {
        I18nManager::init(cx);
        ThemeManager::init(cx);
    });
    let (block, visual_cx) = cx.add_window_view(|_window, cx| {
        let mut block =
            Block::with_record(cx, BlockRecord::paragraph("# Header\ntext\n".repeat(1000)));
        block.set_source_document_mode();
        block
    });
    visual_cx.update(|window, cx| window.draw(cx).clear());
    let initial = block.read_with(visual_cx, |block, _| {
        (
            block.source_layout_cache_hits,
            block.source_layout_cache_misses,
        )
    });
    assert!(initial.1 > 0);
    visual_cx.update(|window, cx| window.draw(cx).clear());
    let reused = block.read_with(visual_cx, |block, _| {
        (
            block.source_layout_cache_hits,
            block.source_layout_cache_misses,
        )
    });
    assert!(reused.0 > initial.0);
    assert_eq!(reused.1, initial.1);
    block.update(visual_cx, |block, cx| {
        block.set_source_syntax_language(None);
        cx.notify();
    });
    visual_cx.update(|window, cx| window.draw(cx).clear());
    let changed = block.read_with(visual_cx, |block, _| block.source_layout_cache_misses);
    assert!(changed > reused.1);
}

#[gpui::test]
async fn ime_selection_ignores_editor_external_selection(cx: &mut TestAppContext) {
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

    block.update(cx, |block, _cx| {
        block.selected_range = 1..1;
        block.editor_selection_range = Some(0..block.visible_len());
    });

    let selection = cx.update(|window, cx| {
        block.update(cx, |block, block_cx| {
            <Block as EntityInputHandler>::selected_text_range(block, false, window, block_cx)
                .expect("selection")
        })
    });

    assert_eq!(selection.range, 1..1);
    assert!(!selection.reversed);
}

#[gpui::test]
async fn focusing_rendered_image_does_not_auto_expand(cx: &mut TestAppContext) {
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::Paragraph,
                InlineTextTree::from_markdown("![diagram](./assets/diagram.png)"),
            ),
        )
    });

    block.update(cx, |block, _cx| {
        block.sync_render_cache();
        assert!(block.showing_rendered_image());
        assert!(!block.image_edit_expanded);

        assert!(!block.sync_image_focus_state(true));
        assert!(block.showing_rendered_image());
        assert!(!block.image_edit_expanded);
    });
}

#[gpui::test]
async fn rendered_image_single_click_selects_and_double_click_expands(cx: &mut TestAppContext) {
    cx.update(|cx| {
        I18nManager::init(cx);
        ThemeManager::init(cx);
    });
    let (block, visual_cx) = cx.add_window_view(|_window, cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::Paragraph,
                InlineTextTree::from_markdown("![diagram](./assets/diagram.png)"),
            ),
        )
    });
    let mut event = MouseDownEvent {
        button: MouseButton::Left,
        position: point(px(100.0), px(100.0)),
        modifiers: Modifiers::default(),
        click_count: 1,
        first_mouse: false,
    };

    visual_cx.update(|window, cx| {
        block.update(cx, |block, block_cx| {
            block.sync_render_cache();
            block.focus_handle.focus(window);
            block.on_mouse_down(&event, window, block_cx);
            assert!(block.image_selected);
            assert!(block.showing_rendered_image());

            block.on_newline(&Newline, window, block_cx);
            assert!(block.image_edit_expanded);
            assert!(!block.showing_rendered_image());

            assert!(block.sync_image_focus_state(false));
            assert!(block.showing_rendered_image());

            block.select_rendered_image(block_cx);
            event.click_count = 2;
            block.on_mouse_down(&event, window, block_cx);
            assert!(block.image_edit_expanded);
            assert!(!block.showing_rendered_image());
        });
    });
}

#[gpui::test]
async fn requested_rendered_image_expansion_enters_raw_markdown_editing(cx: &mut TestAppContext) {
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::Paragraph,
                InlineTextTree::from_markdown("![diagram](./assets/diagram.png)"),
            ),
        )
    });

    block.update(cx, |block, _cx| {
        block.sync_render_cache();
        block.request_image_edit_expansion();
        assert!(block.sync_image_focus_state(true));
        assert!(block.image_edit_expanded);
        assert!(!block.showing_rendered_image());
        assert_eq!(block.cursor_offset(), block.visible_len());
    });
}

#[gpui::test]
async fn blurred_valid_rendered_image_recovers_image_presentation(cx: &mut TestAppContext) {
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::Paragraph,
                InlineTextTree::from_markdown("![diagram](./assets/diagram.png)"),
            ),
        )
    });

    block.update(cx, |block, _cx| {
        block.sync_render_cache();
        block.request_image_edit_expansion();
        assert!(block.sync_image_focus_state(true));
        assert!(block.image_edit_expanded);

        assert!(block.sync_image_focus_state(false));
        assert!(!block.image_edit_expanded);
        assert!(block.showing_rendered_image());
    });
}

#[gpui::test]
async fn broken_rendered_image_syntax_blurs_back_to_plain_text(cx: &mut TestAppContext) {
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::Paragraph,
                InlineTextTree::from_markdown("![diagram](./assets/diagram.png)"),
            ),
        )
    });

    block.update(cx, |block, _cx| {
        block.sync_render_cache();
        block.request_image_edit_expansion();
        assert!(block.sync_image_focus_state(true));

        block
            .record
            .set_title(InlineTextTree::from_markdown("not an image anymore"));
        block.sync_render_cache();
        assert!(!block.sync_image_focus_state(false));
        assert!(block.image_runtime().is_none());
        assert!(!block.image_edit_expanded);
        assert!(!block.showing_rendered_image());
        assert_eq!(block.display_text(), "not an image anymore");
    });
}

#[gpui::test]
async fn code_block_cache_builds_rust_highlight_spans(cx: &mut TestAppContext) {
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::CodeBlock {
                    language: Some("rust".into()),
                },
                InlineTextTree::plain("fn main() {\n    let value: i32 = 42;\n}\n"),
            ),
        )
    });

    let highlight = block
        .read_with(cx, |block, _cx| block.code_highlight_result().cloned())
        .expect("code block should cache a highlight result");
    assert_eq!(highlight.language, CodeLanguageKey::Rust);
    assert!(!highlight.spans.is_empty());
}

#[gpui::test]
async fn source_document_mode_enables_markdown_highlighting(cx: &mut TestAppContext) {
    let block = cx.new(|cx| {
        let mut block = Block::with_record(cx, BlockRecord::paragraph("# Heading\n\n`code`"));
        block.set_source_document_mode();
        block
    });

    let highlight = block
        .read_with(cx, |block, _cx| block.code_highlight_result().cloned())
        .expect("source document should cache markdown highlighting");
    assert_eq!(highlight.language, CodeLanguageKey::Markdown);
    assert!(!highlight.spans.is_empty());
}

#[gpui::test]
async fn code_block_cache_updates_when_language_changes(cx: &mut TestAppContext) {
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::CodeBlock {
                    language: Some("rust".into()),
                },
                InlineTextTree::plain("fn main() {\n    let value = 42;\n}\n"),
            ),
        )
    });

    block.update(cx, |block, _cx| {
        block.record.kind = BlockKind::CodeBlock {
            language: Some("text".into()),
        };
        block.sync_render_cache();
    });

    let highlight = block
        .read_with(cx, |block, _cx| block.code_highlight_result().cloned())
        .expect("known plain fallback should still cache a result");
    assert_eq!(highlight.language, CodeLanguageKey::PlainText);
    assert!(highlight.spans.is_empty());
}

#[gpui::test]
async fn code_block_language_setter_updates_highlight_without_changing_content(
    cx: &mut TestAppContext,
) {
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::CodeBlock {
                    language: Some("rust".into()),
                },
                InlineTextTree::plain("print('hello')"),
            ),
        )
    });

    block.update(cx, |block, cx| {
        let range = 0..block.code_language_text().len();
        block.replace_code_language_text_in_range(range, "python", None, false, cx);
    });

    block.read_with(cx, |block, _cx| {
        assert_eq!(block.code_language_text(), "python");
        assert_eq!(block.display_text(), "print('hello')");
        assert_eq!(
            block
                .code_highlight_result()
                .expect("python should highlight")
                .language,
            CodeLanguageKey::Python
        );
    });
}

#[gpui::test]
async fn code_block_language_accepts_unknown_language_as_plain_rendering(cx: &mut TestAppContext) {
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::CodeBlock {
                    language: Some("rust".into()),
                },
                InlineTextTree::plain("fn main() {}"),
            ),
        )
    });

    block.update(cx, |block, cx| {
        let range = 0..block.code_language_text().len();
        block.replace_code_language_text_in_range(range, "unknown-lang", None, false, cx);
    });

    block.read_with(cx, |block, _cx| {
        assert_eq!(block.code_language_text(), "unknown-lang");
        assert!(block.code_highlight_result().is_none());
    });
}

#[gpui::test]
async fn code_language_input_uses_ime_path_without_touching_code_content(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::CodeBlock {
                    language: Some("rust".into()),
                },
                InlineTextTree::plain("fn main() {}"),
            ),
        )
    });

    cx.update(|window, cx| {
        block.update(cx, |block, block_cx| {
            block.code_language_focus_handle.focus(window);
            block.code_language_selected_range = 0..block.code_language_text().len();
            block.selected_range = 3..3;
            <Block as EntityInputHandler>::replace_text_in_range(
                block, None, "python", window, block_cx,
            );
        });
    });

    block.read_with(cx, |block, _cx| {
        assert_eq!(block.code_language_text(), "python");
        assert_eq!(block.display_text(), "fn main() {}");
        assert_eq!(block.selected_range, 3..3);
        assert_eq!(block.code_language_selected_range, 6..6);
    });
}

#[gpui::test]
async fn code_language_input_handles_utf16_ranges(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::CodeBlock {
                    language: Some("zh😀kana".into()),
                },
                InlineTextTree::plain("body"),
            ),
        )
    });

    cx.update(|window, cx| {
        block.update(cx, |block, block_cx| {
            block.code_language_focus_handle.focus(window);
            <Block as EntityInputHandler>::replace_text_in_range(
                block,
                Some(2..4),
                "py",
                window,
                block_cx,
            );
        });
    });

    block.read_with(cx, |block, _cx| {
        assert_eq!(block.code_language_text(), "zhpykana");
        assert_eq!(block.display_text(), "body");
    });
}

#[gpui::test]
async fn code_language_input_clears_language_when_empty(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::CodeBlock {
                    language: Some("rust".into()),
                },
                InlineTextTree::plain("body"),
            ),
        )
    });

    cx.update(|window, cx| {
        block.update(cx, |block, block_cx| {
            block.code_language_focus_handle.focus(window);
            block.code_language_selected_range = 0..block.code_language_text().len();
            <Block as EntityInputHandler>::replace_text_in_range(block, None, "", window, block_cx);
        });
    });

    block.read_with(cx, |block, _cx| {
        assert_eq!(block.code_language_text(), "");
        assert!(matches!(
            block.kind(),
            BlockKind::CodeBlock { language: None }
        ));
        assert!(block.code_highlight_result().is_none());
    });
}

#[gpui::test]
async fn ending_pointer_selection_session_preserves_text_state(cx: &mut TestAppContext) {
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::CodeBlock {
                    language: Some("rust".into()),
                },
                InlineTextTree::plain("fn main() {}"),
            ),
        )
    });

    block.update(cx, |block, _cx| {
        block.is_selecting = true;
        block.code_language_is_selecting = true;
        block.selected_range = 3..7;
        block.marked_range = Some(4..6);
        block.code_language_selected_range = 1..3;
        block.code_language_marked_range = Some(1..2);

        assert!(block.end_pointer_selection_session());
        assert!(!block.is_selecting);
        assert!(!block.code_language_is_selecting);
        assert_eq!(block.selected_range, 3..7);
        assert_eq!(block.marked_range, Some(4..6));
        assert_eq!(block.code_language_selected_range, 1..3);
        assert_eq!(block.code_language_marked_range, Some(1..2));

        assert!(!block.end_pointer_selection_session());
    });
}

#[gpui::test]
async fn non_dragging_mouse_move_ends_stale_text_selection(cx: &mut TestAppContext) {
    cx.update(|cx| {
        I18nManager::init(cx);
        ThemeManager::init(cx);
    });
    let (block, cx) = cx.add_window_view(|_window, cx| {
        Block::with_record(
            cx,
            BlockRecord::new(BlockKind::Paragraph, InlineTextTree::plain("hello world")),
        )
    });

    let event = MouseMoveEvent {
        position: point(px(8.0), px(8.0)),
        pressed_button: None,
        modifiers: Modifiers::default(),
    };
    cx.update(|window, cx| {
        block.update(cx, |block, cx| {
            block.is_selecting = true;
            block.selected_range = 3..7;
            block.marked_range = Some(4..6);

            block.on_mouse_move(&event, window, cx);

            assert!(!block.is_selecting);
            assert_eq!(block.selected_range, 3..7);
            assert_eq!(block.marked_range, Some(4..6));
        });
    });
}

#[gpui::test]
async fn dragging_mouse_move_keeps_text_selection_session_active(cx: &mut TestAppContext) {
    cx.update(|cx| {
        I18nManager::init(cx);
        ThemeManager::init(cx);
    });
    let (block, cx) = cx.add_window_view(|_window, cx| {
        Block::with_record(
            cx,
            BlockRecord::new(BlockKind::Paragraph, InlineTextTree::plain("hello world")),
        )
    });

    let event = MouseMoveEvent {
        position: point(px(8.0), px(8.0)),
        pressed_button: Some(MouseButton::Left),
        modifiers: Modifiers::default(),
    };
    cx.update(|window, cx| {
        block.update(cx, |block, cx| {
            block.is_selecting = true;
            block.on_mouse_move(&event, window, cx);
            assert!(block.is_selecting);
        });
    });
}

#[gpui::test]
async fn non_dragging_mouse_move_ends_stale_code_language_selection(cx: &mut TestAppContext) {
    cx.update(|cx| {
        I18nManager::init(cx);
        ThemeManager::init(cx);
    });
    let (block, cx) = cx.add_window_view(|_window, cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::CodeBlock {
                    language: Some("rust".into()),
                },
                InlineTextTree::plain("fn main() {}"),
            ),
        )
    });

    let event = MouseMoveEvent {
        position: point(px(8.0), px(8.0)),
        pressed_button: None,
        modifiers: Modifiers::default(),
    };
    cx.update(|window, cx| {
        block.update(cx, |block, cx| {
            block.code_language_is_selecting = true;
            block.code_language_selected_range = 1..3;
            block.code_language_marked_range = Some(1..2);

            block.on_code_language_mouse_move(&event, window, cx);

            assert!(!block.code_language_is_selecting);
            assert_eq!(block.code_language_selected_range, 1..3);
            assert_eq!(block.code_language_marked_range, Some(1..2));
        });
    });
}

#[gpui::test]
async fn code_language_mouse_up_out_ends_selection_without_clearing_text_state(
    cx: &mut TestAppContext,
) {
    cx.update(|cx| {
        I18nManager::init(cx);
        ThemeManager::init(cx);
    });
    let (block, cx) = cx.add_window_view(|_window, cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::CodeBlock {
                    language: Some("rust".into()),
                },
                InlineTextTree::plain("fn main() {}"),
            ),
        )
    });

    let event = MouseUpEvent {
        position: point(px(200.0), px(200.0)),
        button: MouseButton::Left,
        modifiers: Modifiers::default(),
        click_count: 1,
    };
    cx.update(|window, cx| {
        block.update(cx, |block, cx| {
            block.is_selecting = true;
            block.code_language_is_selecting = true;
            block.selected_range = 3..7;
            block.marked_range = Some(4..6);
            block.code_language_selected_range = 1..3;
            block.code_language_marked_range = Some(1..2);

            block.on_code_language_mouse_up_out(&event, window, cx);

            assert!(block.is_selecting);
            assert!(!block.code_language_is_selecting);
            assert_eq!(block.selected_range, 3..7);
            assert_eq!(block.marked_range, Some(4..6));
            assert_eq!(block.code_language_selected_range, 1..3);
            assert_eq!(block.code_language_marked_range, Some(1..2));
        });
    });
}

#[gpui::test]
async fn code_block_without_language_keeps_plain_rendering(cx: &mut TestAppContext) {
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::CodeBlock { language: None },
                InlineTextTree::plain("no highlighting here"),
            ),
        )
    });

    assert!(block.read_with(cx, |block, _cx| block.code_highlight_result().is_none()));
}

#[gpui::test]
async fn editing_link_anchor_in_math_block_matches_plain_paragraph(cx: &mut TestAppContext) {
    // A block mixing inline math with a link is "source preserving", which used
    // to route its link edits through the markdown-space path. That path assumed
    // the anchor label began right after `[`, so the anchor's own emphasis
    // markers shifted the mapping and edits landed on the wrong character. Inline
    // links now edit through the link projection in every block, so deleting a
    // revealed anchor delimiter touches the delimiter, not a label character.
    let cx = cx.add_empty_window();
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::Paragraph,
                InlineTextTree::from_markdown("$x^2$ [**bold**](https://e.com)"),
            ),
        )
    });

    cx.update(|window, cx| {
        block.update(cx, |block, cx| {
            block.move_to("$x^2$ bo".len(), cx);
            block.sync_inline_projection_for_focus(true);

            // Caret just past the revealed opening `**` of the bold anchor.
            let projected = block.display_text().to_string();
            assert_eq!(projected, "$x^2$ [**bold**](https://e.com)");
            let after_open = projected.find("[**").unwrap() + "[**".len();
            block.selected_range = after_open..after_open;

            block.on_delete_back(&DeleteBack, window, cx);

            let markdown = block.record.title.serialize_markdown();
            assert!(
                markdown.starts_with("$x^2$ "),
                "math source preserved: {markdown:?}"
            );
            assert!(
                markdown.contains("bold"),
                "anchor label must stay intact, only the delimiter is edited: {markdown:?}"
            );
        });
    });
}

#[gpui::test]
async fn completing_link_in_math_block_places_caret_after_closing_paren(cx: &mut TestAppContext) {
    // A block mixing math with a link edits in markdown space. Typing the closing
    // `)` completes the link, and the caret must land just past it (like a plain
    // paragraph) rather than inside the anchor before `]`.
    let cx = cx.add_empty_window();
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::Paragraph,
                InlineTextTree::from_markdown("$x$ [link](google.com"),
            ),
        )
    });

    cx.update(|window, cx| {
        block.update(cx, |block, cx| {
            block.move_to(block.visible_len(), cx);
            block.sync_inline_projection_for_focus(true);
            block.replace_text_in_range(None, ")", window, cx);
            block.sync_inline_projection_for_focus(true);

            assert_eq!(
                block.record.title.serialize_markdown(),
                "$x$ [link](google.com)"
            );
            assert_eq!(block.display_text(), "$x$ [link](google.com)");
            let end = block.visible_len();
            assert_eq!(block.selected_range, end..end);
        });
    });
}

#[gpui::test]
async fn rtl_selection_across_trailing_link_keeps_block_end_anchor(cx: &mut TestAppContext) {
    // A link sitting at the very end of a block that also contains inline math
    // stays expanded while the projection is rebuilt on every render. Dragging a
    // selection right-to-left from the block end across the link used to collapse
    // the anchor onto the closing `]` of the anchor text, because the trailing
    // `](url)` delimiters all share one clean offset and the remap snapped back
    // to the inner cursor position. The anchor must stay at the block end.
    let cx = cx.add_empty_window();
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::Paragraph,
                InlineTextTree::from_markdown("$x$ [link](google.com)"),
            ),
        )
    });

    block.update(cx, |block, cx| {
        block.move_to(block.visible_len(), cx);
        block.sync_inline_projection_for_focus(true);
        let end = block.visible_len();
        assert_eq!(block.display_text(), "$x$ [link](google.com)");

        // Start an RTL selection at the block end and drag the head left,
        // re-syncing the projection after each move like the render loop does.
        block.move_to(end, cx);
        block.sync_inline_projection_for_focus(true);
        for target in (0..end).rev() {
            block.select_to(target, cx);
            block.sync_inline_projection_for_focus(true);
            assert_eq!(
                block.selected_range,
                target..end,
                "RTL selection anchor must stay at the block end (head {target})"
            );
            assert!(block.selection_reversed);
        }
    });
}

#[gpui::test]
async fn typing_destination_into_empty_link_parens_keeps_caret_inside(cx: &mut TestAppContext) {
    // Fixes an edge case where batched auto-pair macro `()+Left` caused first character typed
    // into `()` of link to snap the caret past `)` with rest of URL landing outside the link.
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::Paragraph,
                InlineTextTree::from_markdown("[GitHub]"),
            ),
        )
    });

    block.update(cx, |block, cx| {
        // Auto-pair the `()` after the label, then drop the caret between them.
        block.selected_range = 8..8;
        block.sync_inline_projection_for_focus(true);
        block.replace_text_in_visible_range(8..8, "()", None, false, cx);
        block.sync_inline_projection_for_focus(true);
        let between = block.display_text().find(')').expect("closing paren");
        block.selected_range = between..between;
        block.sync_inline_projection_for_focus(true);
        for ch in "https://github.com".chars() {
            let at = block.selected_range.clone();
            block.replace_text_in_visible_range(at, &ch.to_string(), None, false, cx);
            block.sync_inline_projection_for_focus(true);
        }
    });

    block.read_with(cx, |block, _cx| {
        assert_eq!(
            block.record.title.serialize_markdown(),
            "[GitHub](https://github.com)"
        );
        assert_eq!(
            block.inline_link_at(1).map(str::to_string),
            Some("https://github.com".to_string())
        );
        // Caret stays inside `()`, just before the closing `)`.
        let close = block.display_text().find(')').expect("closing paren");
        assert_eq!(block.selected_range, close..close);
    });
}
