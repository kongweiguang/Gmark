// @author kongweiguang

use super::{
    StatusBarState, count_characters, normalized_action_id, should_render_file_status,
    source_format_labels,
};
use crate::i18n::I18nStrings;
use gmark_document::Revision;
use gpui::{AppContext, TestAppContext, VisualTestContext};

#[test]
fn action_names_normalize_to_stable_status_button_ids() {
    assert_eq!(
        normalized_action_id("gmark::ToggleViewMode"),
        "toggle_view_mode"
    );
    assert_eq!(normalized_action_id("save-document"), "save_document");
    assert_eq!(normalized_action_id("plugin.action"), "plugin_action");
}
use gmark_document::{LineEnding, LineEndingStatus, SourceFormatSummary};

#[test]
fn empty_text_has_zero_characters() {
    assert_eq!(count_characters(""), 0);
}

#[test]
fn latin_text_counts_letters_and_spaces() {
    assert_eq!(count_characters("hello world"), 11);
    assert_eq!(count_characters("one two"), 7);
}

#[test]
fn cjk_characters_are_counted_individually() {
    assert_eq!(count_characters("你好世界"), 4);
    assert_eq!(count_characters("中文"), 2);
}

#[test]
fn virtual_region_edit_updates_cached_graphemes_for_new_revision() {
    let old_revision = Revision::from_u64(7);
    let new_revision = Revision::from_u64(8);
    let old_region = "e\u{301} 👨‍👩‍👧‍👦";
    let new_region = "你好 👩‍💻";
    let unchanged_count = count_characters("prefix\n\nsuffix");
    let mut state = StatusBarState::default();
    state.set_word_count(old_revision, unchanged_count + count_characters(old_region));

    state.apply_virtual_text_edit(old_revision, new_revision, old_region, new_region);

    assert_eq!(
        state.cached_word_count(new_revision),
        Some(unchanged_count + count_characters(new_region))
    );
    assert_eq!(state.cached_word_count(old_revision), None);
}

#[test]
fn line_endings_count_as_one_character() {
    assert_eq!(count_characters("a\nb"), 3);
    assert_eq!(count_characters("a\r\nb"), 3);
}

#[test]
fn extended_graphemes_count_as_one_visible_character() {
    assert_eq!(count_characters("e\u{301}"), 1);
    assert_eq!(count_characters("👨‍👩‍👧‍👦"), 1);
    assert_eq!(count_characters("  "), 2);
}

#[test]
fn external_conflict_is_visible_but_recovered_session_is_silent() {
    assert!(should_render_file_status(false, true));
    assert!(!should_render_file_status(true, false));
    assert!(!should_render_file_status(false, false));
}

#[test]
fn source_format_labels_cover_bom_uniform_empty_and_mixed_documents() {
    let strings = I18nStrings::en_us();
    let format = |utf8_bom, line_endings, dominant| SourceFormatSummary {
        utf8_bom,
        line_endings,
        dominant,
    };

    assert_eq!(
        source_format_labels(
            &format(false, LineEndingStatus::None, LineEnding::CrLf),
            &crate::document_io::DocumentEncoding::Utf8,
            &strings,
        ),
        ("UTF-8".to_owned(), "CRLF".to_owned())
    );
    assert_eq!(
        source_format_labels(
            &format(
                true,
                LineEndingStatus::Uniform(LineEnding::CrLf),
                LineEnding::CrLf,
            ),
            &crate::document_io::DocumentEncoding::Utf8,
            &strings
        ),
        ("UTF-8 BOM".to_owned(), "CRLF".to_owned())
    );
    assert_eq!(
        source_format_labels(
            &format(false, LineEndingStatus::Mixed, LineEnding::Lf),
            &crate::document_io::DocumentEncoding::Utf8,
            &strings
        )
        .1,
        "Mixed"
    );
    assert_eq!(
        source_format_labels(
            &format(
                false,
                LineEndingStatus::Uniform(LineEnding::Lf),
                LineEnding::Lf,
            ),
            &crate::document_io::DocumentEncoding::Legacy("GB18030".to_owned()),
            &strings,
        )
        .0,
        "GB18030"
    );
}

/// 窗口壳的坐标必须来自活动窗格；Live 撤销后的 Source 选区仍按逻辑行和完整字素显示。
#[gpui::test]
async fn source_cursor_tracks_restored_reverse_selection_after_live_undo(cx: &mut TestAppContext) {
    init_status_bar_test_app(cx);
    let wrapped_prefix = "x".repeat(240);
    let original =
        format!("first\nsecond\nthird\nfourth\n{wrapped_prefix}👨‍👩‍👧‍👦e\u{301}alpha_beta tail");
    let document_source = original.clone();
    let (root, visual) = cx.add_window_view(move |_window, cx| {
        crate::editor::Editor::from_markdown(cx, document_source, None)
    });
    root.update(visual, |root, cx| {
        root.split_pane_toward(crate::editor::panes::PaneSplitDirection::Right, cx);
    });
    redraw_status_bar_test(visual);
    let editor = root.read_with(visual, |root, cx| {
        root.focused_pane_entities(cx)
            .0
            .expect("active Markdown pane")
    });

    editor.update_in(visual, |editor, window, cx| {
        let block = editor
            .document
            .root_blocks()
            .iter()
            .find(|block| block.read(cx).display_text().contains("alpha_beta"))
            .cloned()
            .expect("Live paragraph containing selected word");
        let start = block
            .read(cx)
            .display_text()
            .find("alpha_beta")
            .expect("selected word offset");
        editor.focus_block(block.entity_id());
        block.update(cx, |block, cx| {
            block.selected_range = start..start + "alpha_beta".len();
            block.selection_reversed = true;
            block.focus_handle.focus(window);
            cx.notify();
        });
    });
    redraw_status_bar_test(visual);

    visual.simulate_input("S29");
    redraw_status_bar_test(visual);
    visual.simulate_keystrokes("ctrl-z");
    redraw_status_bar_test(visual);
    editor.read_with(visual, |editor, cx| {
        assert_eq!(editor.source_document.text(), original);
        let selected = editor
            .document
            .root_blocks()
            .iter()
            .find(|block| block.read(cx).display_text().contains("alpha_beta"))
            .expect("restored Live paragraph")
            .read(cx);
        assert_eq!(
            &selected.display_text()[selected.selected_range.clone()],
            "alpha_beta"
        );
        assert!(selected.selection_reversed);
    });

    root.update(visual, |root, cx| {
        root.set_view_mode(super::super::ViewMode::Source, cx);
    });
    redraw_status_bar_test(visual);
    let source_start = original.find("alpha_beta").expect("source word offset");
    editor.read_with(visual, |editor, cx| {
        let source = editor.document.first_root().expect("Source block").read(cx);
        assert_eq!(
            source.selected_range,
            source_start..source_start + "alpha_beta".len()
        );
        assert!(source.selection_reversed);
        assert_eq!(editor.compute_source_cursor_position(cx), (5, 243));
    });
    root.read_with(visual, |root, cx| {
        assert_eq!(root.compute_source_cursor_position(cx), (5, 243));
    });

    visual.simulate_input("S29");
    redraw_status_bar_test(visual);
    editor.read_with(visual, |editor, cx| {
        assert_eq!(
            editor.source_document.text(),
            original.replacen("alpha_beta", "S29", 1)
        );
        assert_eq!(editor.compute_source_cursor_position(cx), (5, 246));
    });
    root.read_with(visual, |root, cx| {
        assert_eq!(root.compute_source_cursor_position(cx), (5, 246));
    });
}

/// 初始化编辑器渲染与输入依赖，确保回归走真实 GPUI 的焦点、按键和撤销路径。
fn init_status_bar_test_app(cx: &mut TestAppContext) {
    cx.update(|cx| {
        crate::i18n::I18nManager::init(cx);
        crate::theme::ThemeManager::init(cx);
        crate::components::init(cx);
    });
}

/// 等待异步投影和键盘事件完成，使光标断言观察到稳定编辑状态。
fn redraw_status_bar_test(cx: &mut VisualTestContext) {
    cx.update(|window, cx| window.draw(cx).clear());
    cx.run_until_parked();
}
