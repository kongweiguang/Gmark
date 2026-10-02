// @author kongweiguang

use gpui::{AppContext, Bounds, KeyDownEvent, Keystroke, TestAppContext, point, px, size};

use super::{
    Block, TOOLBAR_HEIGHT, ToolbarCommand, ToolbarPosition, attached_surface_placement,
    expanded_block_type_width, selection_toolbar_width, toolbar_window_position,
};
use crate::components::{
    BlockHostAction, BlockKind, BlockRecord, InlineTextTree, TableCellPosition,
    TableColumnAlignment,
};

#[test]
fn position_prefers_above_and_clamps_to_viewport_edges() {
    let left = toolbar_window_position(
        Bounds::new(point(px(2.0), px(100.0)), size(px(20.0), px(20.0))),
        Bounds::new(point(px(0.0), px(0.0)), size(px(320.0), px(200.0))),
        size(px(320.0), px(200.0)),
        0.0,
        Some(106.0),
    );
    assert_eq!(left.left, 8.0);
    assert_eq!(left.top, 100.0 - TOOLBAR_HEIGHT - 6.0);
    assert!(left.above);

    let below = toolbar_window_position(
        Bounds::new(point(px(290.0), px(2.0)), size(px(20.0), px(20.0))),
        Bounds::new(point(px(0.0), px(0.0)), size(px(320.0), px(200.0))),
        size(px(320.0), px(200.0)),
        0.0,
        Some(106.0),
    );
    assert_eq!(
        below,
        ToolbarPosition {
            left: 22.0,
            top: 28.0,
            above: false
        }
    );

    let attached_menu = toolbar_window_position(
        Bounds::new(point(px(120.0), px(350.0)), size(px(20.0), px(20.0))),
        Bounds::new(point(px(0.0), px(0.0)), size(px(480.0), px(400.0))),
        size(px(480.0), px(400.0)),
        312.0,
        Some(106.0),
    );
    assert!(attached_menu.above);
    assert!(attached_menu.top >= 8.0);
}

#[test]
fn toolbar_width_tracks_visible_buttons_without_trailing_space() {
    let roomy_bounds = Bounds::new(point(px(0.0), px(0.0)), size(px(400.0), px(200.0)));
    let compact_bounds = Bounds::new(point(px(0.0), px(0.0)), size(px(280.0), px(200.0)));

    assert_eq!(
        selection_toolbar_width(roomy_bounds, size(px(400.0), px(200.0)), Some(106.0)),
        290.0
    );
    assert_eq!(
        selection_toolbar_width(compact_bounds, size(px(280.0), px(200.0)), Some(106.0)),
        226.0
    );
    assert_eq!(
        selection_toolbar_width(roomy_bounds, size(px(400.0), px(200.0)), Some(82.0)),
        266.0
    );
    assert_eq!(
        selection_toolbar_width(roomy_bounds, size(px(400.0), px(200.0)), None),
        182.0
    );
    assert_eq!(expanded_block_type_width("zh-CN"), 82.0);
    assert_eq!(expanded_block_type_width("en-US"), 106.0);
}

#[test]
fn overflow_menu_uses_safe_content_space_instead_of_toolbar_side() {
    let toolbar_above_selection = ToolbarPosition {
        left: 80.0,
        top: 48.0,
        above: true,
    };
    let (opens_above, available_height) =
        attached_surface_placement(toolbar_above_selection, 174.0, 600.0, 24.0, 24.0);

    assert!(!opens_above);
    assert!(available_height >= 174.0);
}

/// 拖动中的浮层会遮挡命中区域，因此只在手势结束后允许工具栏重新出现。
#[gpui::test]
async fn toolbar_supports_safe_cross_block_ranges_and_suppresses_unsafe_content(
    cx: &mut TestAppContext,
) {
    let block = cx.new(|cx| Block::with_record(cx, BlockRecord::paragraph("alpha")));
    block.update(cx, |block, _cx| {
        block.selected_range = 0..5;
        block.refresh_selection_toolbar();
        assert!(block.selection_toolbar_visible());

        block.is_selecting = true;
        assert!(!block.selection_toolbar_visible());
        block.is_selecting = false;
        assert!(block.selection_toolbar_visible());

        block.editor_selection_range = Some(0..5);
        block.editor_selection_supports_inline_commands = true;
        assert!(block.selection_toolbar_visible());
        assert!(block.selection_toolbar_command_available(ToolbarCommand::Bold));
        assert!(!block.selection_toolbar_command_available(ToolbarCommand::Link));
        assert!(!block.selection_toolbar_command_available(ToolbarCommand::BlockType));
        block.editor_selection_range = None;
        block.editor_selection_supports_inline_commands = false;
        block.marked_range = Some(0..5);
        assert!(!block.selection_toolbar_visible());
        block.marked_range = None;
        block.set_source_raw_mode();
        assert!(!block.selection_toolbar_visible());
    });

    let math = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(BlockKind::Paragraph, InlineTextTree::from_markdown("$x$")),
        )
    });
    math.update(cx, |block, _cx| {
        block.selected_range = 0..block.visible_len();
        assert!(!block.selection_toolbar_visible());
    });
}

#[gpui::test]
async fn toolbar_link_and_overflow_commands_preserve_selection(cx: &mut TestAppContext) {
    let block = cx.new(|cx| Block::with_record(cx, BlockRecord::paragraph("alpha beta")));
    block.update(cx, |block, cx| {
        block.selected_range = 0..5;
        block.apply_selection_toolbar_command(ToolbarCommand::Link, cx);
        assert_eq!(block.record.title.serialize_markdown(), "[alpha]() beta");
        assert_eq!(block.selection_clean_range(), 0..5);

        block.apply_selection_toolbar_command(ToolbarCommand::Overflow, cx);
        assert!(block.selection_toolbar_overflow_open);
        block.apply_selection_toolbar_command(ToolbarCommand::Underline, cx);
        assert_eq!(
            block.record.title.serialize_markdown(),
            "[<u>alpha</u>]() beta"
        );
        assert_eq!(block.selection_clean_range(), 0..5);
        assert!(!block.selection_toolbar_overflow_open);
    });
}

#[gpui::test]
async fn outside_dismiss_closes_attached_popovers_without_losing_selection(
    cx: &mut TestAppContext,
) {
    let block = cx.new(|cx| Block::with_record(cx, BlockRecord::paragraph("alpha")));
    block.update(cx, |block, cx| {
        block.selected_range = 0..5;
        block.selection_toolbar_keyboard_active = true;
        block.selection_toolbar_overflow_open = true;
        block.selection_toolbar_type_menu_open = true;
        block.open_insert_command_menu(cx);
        assert!(block.dismiss_contextual_editing_popovers());
        assert_eq!(block.selected_range, 0..5);
        assert!(block.slash_menu.is_none());
        assert!(!block.selection_toolbar_keyboard_active);
        assert!(!block.selection_toolbar_overflow_open);
        assert!(!block.selection_toolbar_type_menu_open);
    });
}

#[gpui::test]
async fn primary_format_commands_route_to_expected_markdown(cx: &mut TestAppContext) {
    let cases = [
        (ToolbarCommand::Bold, "**alpha** beta"),
        (ToolbarCommand::Italic, "*alpha* beta"),
        (ToolbarCommand::Strikethrough, "~~alpha~~ beta"),
        (ToolbarCommand::Code, "`alpha` beta"),
    ];
    for (command, expected) in cases {
        let block = cx.new(|cx| Block::with_record(cx, BlockRecord::paragraph("alpha beta")));
        block.update(cx, |block, cx| {
            block.selected_range = 0..5;
            block.apply_selection_toolbar_command(command, cx);
            assert_eq!(block.record.title.serialize_markdown(), expected);
            assert_eq!(block.selection_clean_range(), 0..5);
        });
    }
}

#[gpui::test]
async fn overflow_extension_commands_route_to_expected_markdown(cx: &mut TestAppContext) {
    let cases = [
        (ToolbarCommand::Highlight, "==alpha== beta"),
        (ToolbarCommand::Superscript, "<sup>alpha</sup> beta"),
        (ToolbarCommand::Subscript, "<sub>alpha</sub> beta"),
        (ToolbarCommand::InlineMath, "$alpha$ beta"),
    ];
    for (command, expected) in cases {
        let block = cx.new(|cx| Block::with_record(cx, BlockRecord::paragraph("alpha beta")));
        block.update(cx, |block, cx| {
            block.selected_range = 0..5;
            block.apply_selection_toolbar_command(command, cx);
            assert_eq!(block.record.title.serialize_markdown(), expected);
        });
    }
}

#[gpui::test]
async fn table_cell_selection_exposes_safe_inline_commands_without_block_transform(
    cx: &mut TestAppContext,
) {
    let cell = cx.new(|cx| Block::with_record(cx, BlockRecord::paragraph("alpha")));
    cell.update(cx, |cell, cx| {
        cell.set_table_cell_mode(
            TableCellPosition { row: 1, column: 0 },
            TableColumnAlignment::Default,
        );
        cell.selected_range = 0..5;
        assert!(cell.selection_toolbar_visible());
        assert!(cell.selection_toolbar_command_available(ToolbarCommand::Bold));
        assert!(cell.selection_toolbar_command_available(ToolbarCommand::Link));
        assert!(!cell.selection_toolbar_command_available(ToolbarCommand::BlockType));

        cell.apply_selection_toolbar_command(ToolbarCommand::Bold, cx);
        assert_eq!(cell.record.title.serialize_markdown(), "**alpha**");
    });
}

#[gpui::test]
async fn link_editor_submit_escape_and_remove_preserve_focus_and_markdown(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let block = cx.new(|cx| Block::with_record(cx, BlockRecord::paragraph("alpha")));
    block.update(cx, |block, _cx| block.selected_range = 0..5);

    cx.update(|window, cx| {
        block.update(cx, |block, cx| block.open_selection_link_editor(window, cx));
    });
    let input = block
        .read_with(cx, |block, _cx| block.selection_toolbar_link_input.clone())
        .expect("link input");
    input.update(cx, |input, _cx| {
        input
            .record
            .set_title(InlineTextTree::plain("https://example.com"));
        input.sync_render_cache();
    });
    cx.update(|window, cx| {
        // Exercise the real GPUI path: Enter submits while the compact
        // input entity still owns its update lease.
        input.update(cx, |input, input_cx| {
            input.on_newline(&crate::components::Newline, window, input_cx);
        });
    });
    block.read_with(cx, |block, _cx| {
        assert_eq!(
            block.record.title.serialize_markdown(),
            "[alpha](https://example.com)"
        );
        assert!(block.selection_toolbar_link_input.is_none());
    });

    cx.update(|window, cx| {
        block.update(cx, |block, cx| block.open_selection_link_editor(window, cx));
    });
    let cancel_input = block
        .read_with(cx, |block, _cx| block.selection_toolbar_link_input.clone())
        .expect("cancel input");
    cancel_input.update(cx, |input, _cx| {
        input.record.set_title(InlineTextTree::plain("changed"));
        input.sync_render_cache();
    });
    cx.update(|window, cx| {
        let dismiss = cancel_input
            .read(cx)
            .host_action_handler()
            .expect("dismiss handler");
        dismiss(BlockHostAction::DismissTransientUi, window, cx);
    });
    block.read_with(cx, |block, _cx| {
        assert_eq!(
            block.record.title.serialize_markdown(),
            "[alpha](https://example.com)"
        );
        assert!(block.selection_toolbar_link_input.is_none());
    });

    cx.update(|window, cx| {
        block.update(cx, |block, cx| {
            block.open_selection_link_editor(window, cx);
            block.commit_selection_link_editor(true, window, cx);
        });
    });
    cx.update(|window, cx| {
        block.read_with(cx, |block, _cx| {
            assert_eq!(block.record.title.serialize_markdown(), "alpha");
            assert!(block.focus_handle.is_focused(window));
        });
    });
}

#[test]
fn clearing_styles_preserves_links_and_link_destination_can_be_replaced_or_removed() {
    let mut tree = InlineTextTree::from_markdown("[**alpha**](old)");
    assert!(tree.clear_text_formatting(0..5));
    assert_eq!(tree.serialize_markdown(), "[alpha](old)");
    assert_eq!(
        tree.selection_link_destination(0..5).as_deref(),
        Some("old")
    );

    assert!(tree.set_inline_link_destination(0..5, Some("new".to_owned())));
    assert_eq!(tree.serialize_markdown(), "[alpha](new)");
    assert!(tree.set_inline_link_destination(0..5, None));
    assert_eq!(tree.serialize_markdown(), "alpha");
}

#[gpui::test]
async fn alt_f10_enters_toolbar_roving_focus_and_escape_returns_to_editor(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let block = cx.new(|cx| Block::with_record(cx, BlockRecord::paragraph("alpha")));
    block.update(cx, |block, _cx| block.selected_range = 0..5);
    let alt_f10 = KeyDownEvent {
        keystroke: Keystroke::parse("alt-f10").expect("valid Alt+F10"),
        is_held: false,
    };
    let right = KeyDownEvent {
        keystroke: Keystroke::parse("right").expect("valid Right"),
        is_held: false,
    };
    let escape = KeyDownEvent {
        keystroke: Keystroke::parse("escape").expect("valid Escape"),
        is_held: false,
    };

    cx.update(|window, cx| {
        block.update(cx, |block, cx| {
            assert!(block.handle_selection_toolbar_key(&alt_f10, window, cx));
            assert!(block.selection_toolbar_keyboard_active);
            assert!(block.handle_selection_toolbar_key(&right, window, cx));
            assert_eq!(block.selection_toolbar_keyboard_index, 1);
            assert!(block.handle_selection_toolbar_key(&escape, window, cx));
            assert!(!block.selection_toolbar_keyboard_active);
            assert!(block.selection_toolbar_visible());
        });
    });
}
