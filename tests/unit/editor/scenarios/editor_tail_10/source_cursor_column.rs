// @author kongweiguang

use super::*;

/// 长行窗口前的复合字素不在活动 Block 内；End 后输入仍须报告全文字素列并保留 CRLF。
#[gpui::test]
async fn large_source_status_column_counts_unicode_before_horizontal_window(
    cx: &mut TestAppContext,
) {
    init_editor_test_app(cx);
    let temp = tempfile::tempdir().expect("long-line status tempdir");
    let path = temp.path().join("long-line-status-column.txt");
    let unicode_prefix = "e\u{301}👨‍👩‍👧‍👦";
    let window_start_after_input = 80 * 1024 - 64 * 1024 + "🙂".len();
    let prefix_end = window_start_after_input - 1;
    let line_prefix = format!(
        "{unicode_prefix}{}",
        "a".repeat(prefix_end - unicode_prefix.len())
    );
    let original_line = format!(
        "{line_prefix}e\u{301}{}",
        "a".repeat(80 * 1024 - line_prefix.len() - "e\u{301}".len())
    );
    let original = format!("{original_line}\r\nsecond line\r\n");
    fs::write(&path, &original).expect("long-line status fixture");
    let (editor, visual) = open_paged_source(cx, path);
    let host = editor
        .read_with(visual, |editor, _cx| editor.document_host.clone())
        .expect("long-line status Host");

    visual.update(|window, cx| {
        host.update(cx, |view, _cx| {
            view.select_source_range_and_focus_for_test(0..0, false, window);
        });
    });
    redraw(visual);
    visual.simulate_keystrokes("end");
    assert_eq!(
        host.read_with(visual, |view, _cx| view.source_selection_for_test())
            .expect("caret at logical line end")
            .head
            .byte_offset,
        original_line.len() as u64,
        "End must reach the logical line end before the immediate input"
    );
    visual.simulate_input("🙂");
    visual.run_until_parked();

    let expected_column =
        unicode_segmentation::UnicodeSegmentation::graphemes(original_line.as_str(), true).count()
            + 2;
    assert_eq!(
        host.read_with(visual, |view, cx| view.cursor_position(cx)),
        Some((1, expected_column)),
        "the status column must include the Unicode graphemes before the bounded input window"
    );
    assert_eq!(
        host.read_with(visual, |view, _cx| view.source_text_for_test()),
        format!("{original_line}🙂\r\nsecond line\r\n"),
        "typing at End must preserve the original CRLF bytes and following line"
    );
}

/// 长 RI 行的活动窗口可能切开旗帜配对；真实导航须先激活目标行，状态列再按整行字素计数。
#[gpui::test]
async fn large_source_status_column_counts_regional_indicators_at_middle_and_end(
    cx: &mut TestAppContext,
) {
    init_editor_test_app(cx);
    let temp = tempfile::tempdir().expect("regional-indicator status tempdir");
    let path = temp.path().join("regional-indicator-status-column.txt");
    let text = "🇦".repeat(40_001);
    fs::write(&path, &text).expect("regional-indicator status fixture");
    let (editor, visual) = open_paged_source(cx, path);
    let host = editor
        .read_with(visual, |editor, _cx| editor.document_host.clone())
        .expect("regional-indicator status Host");
    let middle = (text.len() * 3 / 4 / 8) * 8;
    let end = text.len();

    for (offset, first_key, second_key, adjacent_offset) in [
        (middle, "right", "left", middle + 8),
        (end, "left", "right", end - 4),
    ] {
        visual.update(|window, cx| {
            host.update(cx, |view, _cx| {
                let offset = offset as u64;
                view.select_source_range_and_focus_for_test(offset..offset, false, window);
            });
        });
        visual.simulate_keystrokes(first_key);
        visual.run_until_parked();
        assert_eq!(
            host.read_with(visual, |view, _cx| {
                view.source_selection_for_test()
                    .map(|selection| selection.head.byte_offset)
            }),
            Some(adjacent_offset as u64),
            "navigation must activate the target row and move by one complete RI grapheme"
        );
        visual.simulate_keystrokes(second_key);
        visual.run_until_parked();

        assert_eq!(
            host.read_with(visual, |view, _cx| {
                view.active_edit_for_test().map(|(line, _)| line)
            }),
            Some(0),
            "real horizontal navigation must materialize the target Source row"
        );
        assert_eq!(
            host.read_with(visual, |view, _cx| {
                view.source_selection_for_test()
                    .map(|selection| selection.head.byte_offset)
            }),
            Some(offset as u64),
            "the caret must return to the requested byte before checking its column"
        );

        redraw(visual);
        visual.run_until_parked();
        redraw(visual);
        let expected_column =
            unicode_segmentation::UnicodeSegmentation::graphemes(&text[..offset], true).count() + 1;
        assert_eq!(
            host.read_with(visual, |view, cx| view.cursor_position(cx)),
            Some((1, expected_column)),
            "the status column must count RI graphemes across the bounded window at byte {offset}"
        );
    }
}
