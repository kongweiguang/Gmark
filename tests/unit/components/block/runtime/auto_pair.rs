// @author kongweiguang

use super::{AutoPairEdit, auto_pair_edit, empty_pair_range};
use crate::components::{Block, BlockRecord};
use gpui::AppContext as _;

#[test]
fn normal_pairs_wrap_selection_insert_inside_and_skip_existing_close() {
    assert_eq!(
        auto_pair_edit("", 0..0, "(", true, false),
        Some(AutoPairEdit::Replace {
            range: 0..0,
            text: "()".to_owned(),
            selected_range_relative: 1..1,
        })
    );
    assert_eq!(
        auto_pair_edit("中文", 0.."中文".len(), "[", true, false),
        Some(AutoPairEdit::Replace {
            range: 0.."中文".len(),
            text: "[中文]".to_owned(),
            selected_range_relative: 1..1 + "中文".len(),
        })
    );
    assert_eq!(
        auto_pair_edit("()", 1..1, ")", true, false),
        Some(AutoPairEdit::MoveTo(2))
    );
}

#[test]
fn apostrophes_inside_words_remain_literal_input() {
    assert_eq!(auto_pair_edit("can", 3..3, "'", true, false), None);
    assert_eq!(auto_pair_edit("James book", 5..5, "'", true, false), None);
    assert_eq!(auto_pair_edit("word", 4..4, "\"", true, false), None);
    assert!(matches!(
        auto_pair_edit("", 0..0, "'", true, false),
        Some(AutoPairEdit::Replace { text, .. }) if text == "''"
    ));
}

#[test]
fn markdown_pairs_promote_bold_and_keep_bullet_shortcut_reachable() {
    assert!(matches!(
        auto_pair_edit("", 0..0, "*", false, true),
        Some(AutoPairEdit::Replace { text, selected_range_relative, .. })
            if text == "**" && selected_range_relative == (1..1)
    ));
    assert!(matches!(
        auto_pair_edit("**", 1..1, "*", false, true),
        Some(AutoPairEdit::Replace { text, selected_range_relative, .. })
            if text == "****" && selected_range_relative == (2..2)
    ));
    assert!(matches!(
        auto_pair_edit("**", 1..1, " ", false, true),
        Some(AutoPairEdit::Replace { text, selected_range_relative, .. })
            if text == "* " && selected_range_relative == (2..2)
    ));
}

/// 反引号完全由用户输入，不能配对、吞掉已有闭合符或包裹选区。
#[test]
fn backticks_remain_literal_even_when_markdown_pairing_is_enabled() {
    for (source, selection) in [
        ("", 0..0),
        ("``", 1..1),
        ("``", 2..2),
        ("```", 3..3),
        ("````", 4..4),
        ("word", 0..4),
        ("`a`", 2..2),
    ] {
        assert_eq!(auto_pair_edit(source, selection, "`", true, true), None);
    }
    assert_eq!(empty_pair_range("``", 1, true, true), None);
}

#[test]
fn markdown_selection_uses_supported_delimiter_widths() {
    assert!(matches!(
        auto_pair_edit("old", 0..3, "~", false, true),
        Some(AutoPairEdit::Replace { text, selected_range_relative, .. })
            if text == "~~old~~" && selected_range_relative == (2..5)
    ));
    assert!(matches!(
        auto_pair_edit("2", 0..1, "^", false, true),
        Some(AutoPairEdit::Replace { text, .. }) if text == "^2^"
    ));
}

#[test]
fn backspace_recognizes_only_enabled_empty_pairs() {
    assert_eq!(auto_pair_edit("", 0..0, "(", false, false), None);
    assert_eq!(auto_pair_edit("", 0..0, "*", false, false), None);
    assert_eq!(empty_pair_range("a()b", 2, true, false), Some(1..3));
    assert_eq!(empty_pair_range("a**b", 2, false, true), Some(1..3));
    assert_eq!(empty_pair_range("a()b", 2, false, false), None);
    assert_eq!(empty_pair_range("(x)", 2, true, true), None);
}

#[gpui::test]
async fn markdown_pairs_keep_the_caret_inside_live_projection(cx: &mut gpui::TestAppContext) {
    for (marker_count, expected) in [(1, "*word*"), (2, "**word**")] {
        let block = cx.new(|cx| Block::with_record(cx, BlockRecord::paragraph(String::new())));
        block.update(cx, |block, cx| {
            assert!(block.try_apply_auto_pair_input(0..0, "*", cx));
            if marker_count == 2 {
                assert!(block.try_apply_auto_pair_input(1..1, "*", cx));
            }
            let caret = block.selected_range.clone();
            block.replace_text_in_visible_range(caret, "word", None, false, cx);
            block.sync_inline_projection_for_focus(true);
            assert_eq!(block.record.title.serialize_markdown(), expected);
            assert!(block.display_text().contains("word"));
            assert!(block.selected_range.end <= block.visible_len());
        });
    }
}

/// 逐次经过真实输入接口并刷新投影，防止只测一次批量插入漏掉首字符后的光标逃逸。
#[gpui::test]
async fn manual_backticks_keep_sequential_typing_inside_code(cx: &mut gpui::TestAppContext) {
    let cx = cx.add_empty_window();
    for text in ["word", "中文🙂"] {
        let block = cx.new(|cx| Block::with_record(cx, BlockRecord::paragraph("")));
        cx.update(|window, cx| {
            block.update(cx, |block, cx| {
                block.focus_handle.focus(window);
                block.sync_inline_projection_for_focus(true);
                for expected in ["`", "``"] {
                    gpui::EntityInputHandler::replace_text_in_range(block, None, "`", window, cx);
                    assert_eq!(block.display_text(), expected);
                    assert_eq!(block.cursor_offset(), expected.len());
                }
                block.move_to(1, cx);
                let mut typed = String::new();
                for ch in text.chars() {
                    let input = ch.to_string();
                    gpui::EntityInputHandler::replace_text_in_range(
                        block, None, &input, window, cx,
                    );
                    block.sync_inline_projection_for_focus(true);
                    typed.push(ch);
                    assert_eq!(
                        block.record.title.serialize_markdown(),
                        format!("`{typed}`")
                    );
                    assert_eq!(block.cursor_offset(), 1 + typed.len());
                }
            })
        });
    }
}

/// 手动键入闭合符后仍应退出代码；禁止自动配对不能让普通正文继续继承代码样式。
#[gpui::test]
async fn manual_backticks_close_code_and_keep_following_text_plain(cx: &mut gpui::TestAppContext) {
    let cx = cx.add_empty_window();
    let block = cx.new(|cx| Block::with_record(cx, BlockRecord::paragraph("")));
    cx.update(|window, cx| {
        block.update(cx, |block, cx| {
            block.focus_handle.focus(window);
            block.sync_inline_projection_for_focus(true);
            for ch in "`word`!".chars() {
                gpui::EntityInputHandler::replace_text_in_range(
                    block,
                    None,
                    &ch.to_string(),
                    window,
                    cx,
                );
                block.sync_inline_projection_for_focus(true);
            }
            assert_eq!(block.record.title.serialize_markdown(), "`word`!");
            assert_eq!(block.cursor_offset(), block.visible_len());
        })
    });
}

/// 每个围栏字符必须按键数原样落入文本，覆盖段首、语言名及长围栏。
#[gpui::test]
async fn manual_backticks_fences_never_insert_extra_markers(cx: &mut gpui::TestAppContext) {
    let cx = cx.add_empty_window();
    for text in ["```rust", "`````", "prefix ```"] {
        let block = cx.new(|cx| Block::with_record(cx, BlockRecord::paragraph("")));
        cx.update(|window, cx| {
            block.update(cx, |block, cx| {
                block.focus_handle.focus(window);
                block.sync_inline_projection_for_focus(true);
                let mut typed = String::new();
                for ch in text.chars() {
                    gpui::EntityInputHandler::replace_text_in_range(
                        block,
                        None,
                        &ch.to_string(),
                        window,
                        cx,
                    );
                    block.sync_inline_projection_for_focus(true);
                    typed.push(ch);
                    assert_eq!(block.display_text(), typed);
                    assert_eq!(block.cursor_offset(), typed.len());
                }
            })
        });
    }
}
