// @author kongweiguang

use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use gpui::{
    AnyWindowHandle, AppContext, BorrowAppContext, ClickEvent, EntityInputHandler, KeyDownEvent,
    Keystroke, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, OwnedMenu, OwnedMenuItem,
    TestAppContext, VisualTestContext, point, px, size,
};
use sysinfo::{ProcessesToUpdate, System, get_current_pid};

use super::projection::ProjectionRegionKind;
use super::virtual_surface::{VirtualRegionIndex, VirtualSurfaceState};
use super::{
    CrossBlockSelection, CrossBlockSelectionEndpoint, Editor, InfoDialogKind,
    PreparedSplitProjection, ViewMode,
};
use crate::components::{
    BlockEvent, BlockKind, CloseWindow, EditingCommandId, FindInDocument, FocusNext,
    ImageReferenceDefinitions, ImageResolvedSource, InlineTextTree, Newline, NoRecentFiles,
    QuitApplication, ReplaceInDocument, SaveDocument, TableCellInlineImageSegment,
    TableColumnAlignment, Undo, parse_table_cell_inline_images, superscript_ordinal,
};

#[gpui::test]
async fn rendered_toc_uses_heading_projection_and_navigation_does_not_mutate_source(
    cx: &mut TestAppContext,
) {
    init_editor_test_app(cx);
    let editor = cx
        .new(|cx| Editor::from_markdown(cx, "[TOC]\n\n# Overview\n\n## Details".to_string(), None));

    editor.update(cx, |editor, cx| {
        let source_before = editor.source_document.revision();
        editor.rebuild_runtime_context_from_markdown("[TOC]\n\n# Overview\n\n## Details", cx);
        let (toc, heading_entities) = {
            let visible = editor.document.visible_blocks();
            (
                visible[0].entity.clone(),
                visible[1..]
                    .iter()
                    .map(|visible| visible.entity.clone())
                    .collect::<Vec<_>>(),
            )
        };
        let headings = heading_entities
            .iter()
            .map(|heading| heading.entity_id())
            .collect::<Vec<_>>();
        let entries = toc.read(cx).toc_entries.clone();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].level, 1);
        assert_eq!(entries[0].title.as_ref(), "Overview");
        assert_eq!(entries[1].level, 2);
        assert_eq!(entries[1].target, headings[1]);
        assert!(
            heading_entities
                .iter()
                .all(|heading| heading.read(cx).toc_entries.is_empty()),
            "TOC runtime state must stay scoped to the rendered marker"
        );

        editor.on_block_event(
            toc.clone(),
            &BlockEvent::RequestJumpToTocHeading {
                target: entries[1].target,
            },
            cx,
        );
        assert_eq!(editor.active_entity_id, Some(headings[1]));
        assert_eq!(editor.source_document.revision(), source_before);
        assert!(!editor.document_dirty);

        let first_heading = heading_entities[0].clone();
        first_heading.update(cx, |block, _cx| {
            block
                .record
                .set_title(InlineTextTree::plain("Renamed Overview"));
        });
        editor.sync_runtime_context_after_block_edit(&first_heading, cx);
        assert_eq!(
            toc.read(cx).toc_entries[0].title.as_ref(),
            "Renamed Overview"
        );
    });
}

#[gpui::test]
async fn paragraph_menu_block_kind_change_preserves_source_and_undo(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let editor = cx.new(|cx| Editor::from_markdown(cx, "menu text".to_string(), None));

    editor.update(cx, |editor, cx| {
        let block = editor.document.first_root().expect("paragraph").clone();
        editor.focus_block(block.entity_id());
        editor.set_active_block_kind(BlockKind::Heading { level: 2 }, cx);
        assert_eq!(block.read(cx).kind(), BlockKind::Heading { level: 2 });
        assert_eq!(block.read(cx).display_text(), "menu text");
        assert_eq!(editor.document.markdown_text(cx), "## menu text");
        assert!(editor.document_dirty);
        assert_eq!(editor.undo_history.len(), 1);
    });
}

#[gpui::test]
async fn clicking_block_gutter_opens_the_block_action_menu(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let (editor, visual) =
        cx.add_window_view(|_window, cx| Editor::from_markdown(cx, "menu text".to_owned(), None));
    visual.simulate_resize(size(px(720.0), px(520.0)));
    let block = editor.update_in(visual, |editor, window, _cx| {
        let block = editor.document.first_root().expect("paragraph").clone();
        editor.focus_block(block.entity_id());
        block.read(_cx).focus_handle.focus(window);
        block
    });
    redraw(visual);

    let handle = visual
        .debug_bounds("focused-block-context-actions")
        .expect("focused block actions");
    visual.simulate_mouse_down(handle.center(), MouseButton::Left, Modifiers::default());
    visual.simulate_mouse_up(handle.center(), MouseButton::Left, Modifiers::default());
    visual.run_until_parked();
    redraw(visual);

    assert!(block.read_with(visual, |block, _cx| block.slash_menu.is_some()));
    assert!(visual.debug_bounds("slash-command-menu").is_some());
}

#[gpui::test]
async fn clicking_unfocused_code_block_gutter_opens_its_action_menu(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let (editor, visual) = cx.add_window_view(|_window, cx| {
        Editor::from_markdown(cx, "```rust\nfn main() {}\n```".to_owned(), None)
    });
    visual.simulate_resize(size(px(900.0), px(640.0)));
    let (code, _external_focus) = editor.update_in(visual, |editor, window, cx| {
        let code = editor.document.first_root().expect("code block").clone();
        let external_focus = cx.focus_handle();
        external_focus.focus(window);
        (code, external_focus)
    });
    redraw(visual);

    let surface = visual
        .debug_bounds("code-block-surface")
        .expect("code block surface");
    visual.simulate_mouse_move(surface.center(), None, Modifiers::default());
    redraw(visual);
    let handle = visual
        .debug_bounds("block-context-actions")
        .expect("unfocused code block actions");
    visual.simulate_mouse_move(handle.center(), None, Modifiers::default());
    redraw(visual);
    let handle = visual
        .debug_bounds("block-context-actions")
        .expect("hovered code block actions");
    visual.simulate_mouse_down(handle.center(), MouseButton::Left, Modifiers::default());
    visual.simulate_mouse_up(handle.center(), MouseButton::Left, Modifiers::default());
    visual.run_until_parked();
    redraw(visual);

    assert!(code.read_with(visual, |block, _cx| block.slash_menu.is_some()));
    assert!(visual.debug_bounds("slash-command-menu").is_some());
}

#[gpui::test]
async fn block_handle_drag_tracks_pointer_half_and_drops_after_document_tail(
    cx: &mut TestAppContext,
) {
    init_editor_test_app(cx);
    let (editor, visual) = cx.add_window_view(|_window, cx| {
        Editor::from_markdown(cx, "one\n\ntwo\n\nthree".to_owned(), None)
    });
    visual.simulate_resize(size(px(720.0), px(520.0)));
    let third_entity = editor.update_in(visual, |editor, window, cx| {
        let visible = editor.document.visible_blocks().to_vec();
        let first = visible[0].entity.clone();
        let third = visible[2].entity.clone();
        editor.focus_block(first.entity_id());
        first.read(cx).focus_handle.focus(window);
        third
    });
    redraw(visual);

    let handle = visual
        .debug_bounds("focused-block-context-actions")
        .expect("first block handle");
    let gutter = visual
        .debug_bounds("block-context-gutter")
        .expect("focused block gutter");
    assert!(visual.debug_bounds("block-context-add-icon").is_none());
    let icon = visual
        .debug_bounds("block-context-actions-icon")
        .expect("visible gutter icon");
    assert_eq!(icon.size, size(px(14.0), px(14.0)));
    assert!(icon.left() >= gutter.left());
    assert!(icon.right() <= gutter.right());
    for selector in [
        "block-context-actions-dot-0-0",
        "block-context-actions-dot-0-1",
        "block-context-actions-dot-0-2",
        "block-context-actions-dot-1-0",
        "block-context-actions-dot-1-1",
        "block-context-actions-dot-1-2",
        "block-context-actions-dot-2-0",
        "block-context-actions-dot-2-1",
        "block-context-actions-dot-2-2",
    ] {
        let dot = visual
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("missing block grip dot: {selector}"));
        assert_eq!(dot.size, size(px(2.0), px(2.0)));
        assert!(dot.left() >= icon.left() && dot.right() <= icon.right());
        assert!(dot.top() >= icon.top() && dot.bottom() <= icon.bottom());
    }
    let target = third_entity
        .read_with(visual, |block, _cx| block.last_bounds)
        .expect("third block bounds");
    let lower_half = point(target.center().x, target.bottom() - px(2.0));
    visual.simulate_mouse_down(handle.center(), MouseButton::Left, Modifiers::default());
    visual.simulate_mouse_move(lower_half, MouseButton::Left, Modifiers::default());
    redraw(visual);
    assert_eq!(
        third_entity.read_with(visual, |block, _cx| block.block_drop_placement),
        crate::components::BlockDropPlacement::After
    );

    visual.simulate_mouse_up(lower_half, MouseButton::Left, Modifiers::default());
    visual.run_until_parked();
    editor.read_with(visual, |editor, _cx| {
        assert_eq!(editor.source_document.text(), "two\n\nthree\n\none");
        assert_eq!(editor.undo_history.len(), 1);
    });
}

#[gpui::test]
async fn clicking_canvas_below_trailing_code_block_adds_focused_paragraph(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let (editor, visual) = cx.add_window_view(|_window, cx| {
        Editor::from_markdown(cx, "```rust\nfn main() {}\n```".to_owned(), None)
    });
    visual.simulate_resize(size(px(720.0), px(520.0)));
    redraw(visual);

    let code_surface = visual
        .debug_bounds("code-block-surface")
        .expect("code block surface");
    let editor_pane = visual
        .debug_bounds("editor-source-pane")
        .expect("editor pane");
    let click_y = (code_surface.bottom() + px(24.0)).min(editor_pane.bottom() - px(8.0));
    assert!(click_y > code_surface.bottom());
    let blank = point(code_surface.center().x, click_y);
    visual.simulate_mouse_down(blank, MouseButton::Left, Modifiers::default());
    visual.simulate_mouse_up(blank, MouseButton::Left, Modifiers::default());
    visual.run_until_parked();
    redraw(visual);

    editor.read_with(visual, |editor, cx| {
        let roots = editor.document.root_blocks();
        assert_eq!(roots.len(), 2);
        assert_eq!(roots[1].read(cx).kind(), BlockKind::Paragraph);
        assert_eq!(editor.active_entity_id, Some(roots[1].entity_id()));
        assert_eq!(editor.undo_history.len(), 1);
    });
}

#[gpui::test]
async fn clicking_canvas_below_trailing_separator_adds_focused_paragraph(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let (editor, visual) =
        cx.add_window_view(|_window, cx| Editor::from_markdown(cx, "---".to_owned(), None));
    visual.simulate_resize(size(px(720.0), px(520.0)));
    redraw(visual);

    let separator = visual
        .debug_bounds("separator-surface")
        .expect("separator surface");
    let tail = visual
        .debug_bounds("editor-document-tail-blank")
        .expect("document tail blank area");
    assert!(tail.top() >= separator.bottom());
    let blank = point(tail.center().x, tail.top() + px(24.0));
    visual.simulate_mouse_down(blank, MouseButton::Left, Modifiers::default());
    visual.simulate_mouse_up(blank, MouseButton::Left, Modifiers::default());
    visual.run_until_parked();
    redraw(visual);

    editor.read_with(visual, |editor, cx| {
        let roots = editor.document.root_blocks();
        assert_eq!(roots.len(), 2);
        assert_eq!(roots[0].read(cx).kind(), BlockKind::Separator);
        assert_eq!(roots[1].read(cx).kind(), BlockKind::Paragraph);
        assert_eq!(editor.active_entity_id, Some(roots[1].entity_id()));
        assert_eq!(editor.undo_history.len(), 1);
    });
}

#[gpui::test]
async fn quote_block_gutter_stays_left_of_quote_guide(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let (editor, visual) = cx
        .add_window_view(|_window, cx| Editor::from_markdown(cx, "> quoted line".to_owned(), None));
    visual.simulate_resize(size(px(720.0), px(520.0)));
    editor.update_in(visual, |editor, window, cx| {
        let quote = editor
            .document
            .visible_blocks()
            .iter()
            .find(|block| block.entity.read(cx).visible_quote_depth > 0)
            .expect("visible quote block")
            .entity
            .clone();
        editor.focus_block(quote.entity_id());
        quote.read(cx).focus_handle.focus(window);
    });
    redraw(visual);

    let gutter = visual
        .debug_bounds("block-context-gutter")
        .expect("quote block gutter");
    let guide = visual.debug_bounds("quote-guide-0").expect("quote guide");
    assert!(
        gutter.right() < guide.left(),
        "gutter {gutter:?} overlaps quote guide {guide:?}"
    );
}

#[gpui::test]
async fn rendered_block_surfaces_share_the_paragraph_content_edges(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let source = concat!(
        "alignment baseline\n\n",
        "# heading\n\n",
        "- bullet\n\n",
        "1. numbered\n\n",
        "- [ ] task\n\n",
        "> quoted\n\n",
        "| Name | Use |\n| --- | --- |\n| one | two |\n\n",
        "```rust\nfn main() {}\n```\n\n",
        "---\n\n",
        "> [!NOTE]\n> callout body\n\n",
        "reference[^1]\n\n",
        "[^1]: footnote body",
    );
    let (editor, visual) =
        cx.add_window_view(|_window, cx| Editor::from_markdown(cx, source.to_owned(), None));
    visual.simulate_resize(size(px(1600.0), px(1200.0)));
    redraw(visual);
    // LaTeX 与 Mermaid 都在防抖后异步生成 SVG；布局断言必须观察最终 surface，
    // 不能把 pending placeholder 当成生产渲染结果。
    visual.executor().advance_clock(Duration::from_millis(300));
    visual.run_until_parked();
    redraw(visual);

    let baseline = editor.read_with(visual, |editor, cx| {
        editor
            .document
            .visible_blocks()
            .iter()
            .find(|visible| visible.entity.read(cx).display_text() == "alignment baseline")
            .and_then(|visible| visible.entity.read(cx).last_bounds)
            .expect("baseline paragraph bounds")
    });
    for selector in [
        "table-surface",
        "code-block-surface",
        "separator-surface",
        "callout-surface",
        "footnote-surface",
    ] {
        let surface = visual
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("missing {selector}"));
        let left_gap = f32::from(surface.left() - baseline.left()).abs();
        let right_gap = f32::from(surface.right() - baseline.right()).abs();
        assert!(
            left_gap <= 0.5 && right_gap <= 0.5,
            "{selector} must share paragraph edges; baseline={baseline:?}, surface={surface:?}"
        );
    }

    for selector in [
        "quote-guide-0",
        "bulleted-list-marker-slot-filled",
        "numbered-list-marker-slot",
        "task-checkbox",
    ] {
        let marker = visual
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("missing {selector}"));
        let left_gap = f32::from(marker.left() - baseline.left()).abs();
        assert!(
            left_gap <= 0.5,
            "{selector} must start on the paragraph edge; baseline={baseline:?}, marker={marker:?}"
        );
    }
}

#[gpui::test]
async fn specialized_rendered_surfaces_share_the_paragraph_content_edges(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let source = concat!(
        "---\nname: alignment\n---\n\n",
        "alignment baseline\n\n",
        "[TOC]\n\n",
        "# Overview\n\n",
        "<div>html block</div>\n\n",
        "$$\nx^2\n$$\n\n",
        "```mermaid\nflowchart LR\nA --> B\n```\n\n",
        "![alignment image](missing-alignment-image.png)",
    );
    let (editor, visual) =
        cx.add_window_view(|_window, cx| Editor::from_markdown(cx, source.to_owned(), None));
    editor.update(visual, |editor, cx| {
        let baseline = editor
            .document
            .visible_blocks()
            .iter()
            .find(|visible| visible.entity.read(cx).display_text() == "alignment baseline")
            .expect("baseline paragraph")
            .entity
            .clone();
        editor.focus_block(baseline.entity_id());
    });
    visual.simulate_resize(size(px(1600.0), px(1200.0)));
    redraw(visual);
    // LaTeX 与 Mermaid 都在防抖后异步生成 SVG；布局断言必须观察最终 surface，
    // 不能把 pending placeholder 当成生产渲染结果。
    visual.executor().advance_clock(Duration::from_millis(300));
    visual.run_until_parked();
    redraw(visual);

    let baseline = editor.read_with(visual, |editor, cx| {
        editor
            .document
            .visible_blocks()
            .iter()
            .find(|visible| visible.entity.read(cx).display_text() == "alignment baseline")
            .and_then(|visible| visible.entity.read(cx).last_bounds)
            .expect("baseline paragraph bounds")
    });
    #[cfg(feature = "native-html-render")]
    let html_selector = "rendered-html-surface";
    #[cfg(not(feature = "native-html-render"))]
    let html_selector = "html-raw-text";
    for selector in [
        "yaml-frontmatter",
        "document-toc",
        html_selector,
        "math-rendered-content",
        // Mermaid 的 SVG 内容位于 1px 工作台边框内；对齐契约属于外框而非内层画布。
        "mermaid-workbench-frame",
        "rendered-image-content",
    ] {
        let surface = visual
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("missing {selector}"));
        let left_gap = f32::from(surface.left() - baseline.left()).abs();
        let right_gap = f32::from(surface.right() - baseline.right()).abs();
        assert!(
            left_gap <= 0.5 && right_gap <= 0.5,
            "{selector} must share paragraph edges; baseline={baseline:?}, surface={surface:?}"
        );
    }
    #[cfg(feature = "native-html-render")]
    assert!(visual.debug_bounds("rendered-html-surface").is_some());
    #[cfg(not(feature = "native-html-render"))]
    assert!(visual.debug_bounds("rendered-html-surface").is_none());
}

fn assert_dialog_title_icon(
    visual: &mut VisualTestContext,
    dialog_selector: &'static str,
    icon_selector: &'static str,
    label_selector: &'static str,
) {
    let dialog = visual.debug_bounds(dialog_selector).unwrap();
    let icon = visual.debug_bounds(icon_selector).unwrap();
    let label = visual.debug_bounds(label_selector).unwrap();
    assert_eq!(icon.size, size(px(22.0), px(22.0)));
    assert!(icon.left() >= dialog.left());
    assert!(icon.right() <= dialog.right());
    assert!(label.left() > icon.right());
    assert!(label.right() <= dialog.right());
    assert!(icon.top() >= dialog.top());
    assert!(label.top() >= dialog.top());
}

/// 通过等待后台计划完成再断言，覆盖 Replace All 的原子提交和 20,000 条结果预算。
#[gpui::test]
async fn document_find_replace_is_unicode_safe_undoable_and_bounded(cx: &mut TestAppContext) {
    use super::find_replace::{
        FindOptions, compile_find_regex, find_matches, replacement_for_range,
    };

    let unicode = find_matches(
        "Rust rustacean RUST 中文中文 中文",
        "rust",
        FindOptions {
            whole_word: true,
            ..FindOptions::default()
        },
        gmark_document::Revision::INITIAL,
    );
    assert_eq!(unicode.matches, vec![0..4, 15..19]);
    let regex = compile_find_regex(
        r"(name): (\w+)",
        FindOptions {
            regex: true,
            ..FindOptions::default()
        },
    )
    .unwrap();
    assert_eq!(
        replacement_for_range(&regex, "name: Ada", 0..9, "$2 ($1)", true).as_deref(),
        Some("Ada (name)")
    );

    init_editor_test_app(cx);
    let (editor, visual_cx) = cx.add_window_view(|_window, cx| {
        Editor::from_markdown(cx, "alpha beta alpha".to_owned(), None)
    });
    editor.update_in(visual_cx, |editor, window, cx| {
        let block = editor.document.first_root().expect("paragraph").clone();
        editor.active_entity_id = Some(block.entity_id());
        block.update(cx, |block, _cx| block.selected_range = 0..5);
        editor.on_find_in_document_action(&FindInDocument, window, cx);
    });
    visual_cx
        .executor()
        .advance_clock(Duration::from_millis(40));
    visual_cx.run_until_parked();
    redraw(visual_cx);
    editor.update(visual_cx, |editor, cx| {
        let state = editor.find_panel.as_ref().expect("find panel");
        assert_eq!(state.query.read(cx).display_text(), "alpha");
        assert_eq!(state.matches, vec![0..5, 11..16]);
        assert!(state.error.is_none());
        editor
            .find_panel
            .as_mut()
            .expect("find panel")
            .tooltip_visible = Some("document-find-case");
        cx.notify();
    });
    let tab_event = KeyDownEvent {
        keystroke: Keystroke::parse("tab").expect("valid tab keystroke"),
        is_held: false,
    };
    let space_event = KeyDownEvent {
        keystroke: Keystroke::parse("space").expect("valid space keystroke"),
        is_held: false,
    };
    editor.update_in(visual_cx, |editor, window, cx| {
        assert!(editor.handle_find_panel_key(&tab_event, window, cx));
        assert!(editor.handle_find_panel_key(&space_event, window, cx));
        assert!(
            editor
                .find_panel
                .as_ref()
                .expect("find panel")
                .options
                .case_sensitive
        );
    });

    for viewport in [size(px(720.0), px(520.0)), size(px(1180.0), px(780.0))] {
        visual_cx.simulate_resize(viewport);
        redraw(visual_cx);
        visual_cx.update(|window, _cx| assert_eq!(window.scale_factor(), 2.0));
        let panel = visual_cx.debug_bounds("document-find-panel").unwrap();
        let content = visual_cx.debug_bounds("editor-content").unwrap();
        assert!(panel.left() >= content.left());
        assert!(panel.right() <= content.right());
        assert!(panel.top() >= content.top());
        assert!(panel.bottom() <= content.bottom());
        for selector in [
            "document-find-case-icon",
            "document-find-word-icon",
            "document-find-regex-icon",
            "document-find-previous-icon",
            "document-find-next-icon",
            "document-find-close-icon",
        ] {
            let icon = visual_cx.debug_bounds(selector).expect("visible find icon");
            assert_eq!(icon.size, size(px(15.0), px(15.0)), "{selector}");
            assert!(icon.left() >= panel.left(), "{selector} escaped left");
            assert!(icon.right() <= panel.right(), "{selector} escaped right");
        }
        let tooltip = visual_cx.debug_bounds("document-find-tooltip").unwrap();
        assert!(tooltip.left() >= content.left());
        assert!(tooltip.right() <= content.right());
        assert!(tooltip.top() >= content.top());
        assert!(tooltip.bottom() <= content.bottom());
    }

    editor.update_in(visual_cx, |editor, window, cx| {
        editor.on_replace_in_document_action(&ReplaceInDocument, window, cx);
        let replacement = editor
            .find_panel
            .as_ref()
            .expect("replace panel")
            .replacement
            .clone();
        replacement.update(cx, |input, input_cx| {
            input.replace_text_in_visible_range(0..0, "omega", None, false, input_cx);
        });
        editor.replace_all_find_matches(window, cx);
    });
    visual_cx.run_until_parked();
    editor.update_in(visual_cx, |editor, _window, cx| {
        assert_eq!(editor.source_document.text(), "omega beta omega");
        assert!(editor.document_dirty);
        editor.undo_document(cx);
        assert_eq!(editor.source_document.text(), "alpha beta alpha");
    });
    let escape_event = KeyDownEvent {
        keystroke: Keystroke::parse("escape").expect("valid escape keystroke"),
        is_held: false,
    };
    editor.update_in(visual_cx, |editor, window, cx| {
        assert!(editor.handle_find_panel_key(&escape_event, window, cx));
        assert!(editor.find_panel.is_none());
    });

    let large_source = (0..10_000)
        .map(|index| format!("needle paragraph {index}"))
        .collect::<Vec<_>>()
        .join("\n\n");
    let original_large_source = large_source.clone();
    let (large_editor, large_cx) = cx.add_window_view(move |_window, cx| {
        Editor::from_markdown_virtualized(cx, large_source, None)
    });
    large_editor.update_in(large_cx, |editor, window, cx| {
        editor.on_replace_in_document_action(&ReplaceInDocument, window, cx);
        let state = editor.find_panel.as_ref().expect("large find panel");
        state.query.update(cx, |input, input_cx| {
            input.replace_text_in_visible_range(0..0, "needle", None, false, input_cx);
        });
        state.replacement.update(cx, |input, input_cx| {
            input.replace_text_in_visible_range(0..0, "pin", None, false, input_cx);
        });
    });
    large_cx.executor().advance_clock(Duration::from_millis(40));
    large_cx.run_until_parked();
    large_editor.update_in(large_cx, |editor, window, cx| {
        assert_eq!(editor.find_panel.as_ref().unwrap().matches.len(), 10_000);
        editor.find_panel.as_mut().unwrap().selected = 9_999;
        editor.navigate_find_match(1, window, cx);
        assert!(editor.scroll_handle.offset().y < px(0.0));
        assert!(editor.active_entity_id.is_some());
        editor.replace_all_find_matches(window, cx);
    });
    large_cx.run_until_parked();
    large_editor.update_in(large_cx, |editor, _window, cx| {
        assert!(editor.virtual_surface.is_some());
        assert!(editor.source_document.text().starts_with("pin paragraph 0"));
        assert!(
            editor
                .source_document
                .text()
                .ends_with("pin paragraph 9999")
        );
        editor.undo_document(cx);
        assert_eq!(editor.source_document.text(), original_large_source);
    });
}
use crate::export::ExportFormat;
use crate::i18n::{I18nManager, I18nStrings};
use crate::theme::{Theme, ThemeAppearance, ThemeManager, ThemePalette};
fn init_editor_test_app(cx: &mut TestAppContext) {
    cx.update(|cx| {
        I18nManager::init(cx);
        ThemeManager::init(cx);
        crate::components::init(cx);
        crate::app::document_service::DocumentService::init(cx);
    });
}

fn temp_markdown_path(test_name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "gmark-{test_name}-{}-{nanos}.md",
        std::process::id()
    ))
}

fn temp_export_path(test_name: &str, extension: &str) -> PathBuf {
    let mut path = temp_markdown_path(test_name);
    path.set_extension(extension);
    path
}
