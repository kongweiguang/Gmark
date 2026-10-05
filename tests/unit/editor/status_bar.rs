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

/// 切到新文档后，底栏必须显示当前页的格式，不能沿用后台文档的 CRLF 与 BOM。
#[gpui::test]
async fn source_format_labels_follow_active_pane_document(cx: &mut TestAppContext) {
    init_status_bar_test_app(cx);
    let (root, visual) = cx.add_window_view(|_window, cx| {
        crate::editor::Editor::from_markdown(cx, "\u{feff}background\r\ntext\r\n".into(), None)
    });
    root.update(visual, |root, cx| {
        root.split_pane_toward(crate::editor::panes::PaneSplitDirection::Right, cx);
    });
    redraw_status_bar_test(visual);
    root.read_with(visual, |root, cx| {
        assert_eq!(
            root.current_source_format_labels(&I18nStrings::en_us(), cx),
            (
                I18nStrings::en_us().status_bar_encoding_utf8_bom,
                "CRLF".into()
            )
        );
    });
    root.update(visual, |root, cx| {
        let focused = root
            .pane_workspace
            .as_ref()
            .expect("workspace")
            .read(cx)
            .workspace()
            .focused_pane();
        assert!(root.new_document_tab_in_pane(focused, crate::editor::DocumentKind::Markdown, cx));
    });
    redraw_status_bar_test(visual);
    root.read_with(visual, |root, cx| {
        assert_eq!(
            root.current_source_format_labels(&I18nStrings::en_us(), cx),
            ("UTF-8".into(), "LF".into())
        );
    });
}

/// 底栏菜单调用必须仅改变活动文档，并沿用其格式历史；后台文档和只读视图不得受影响。
#[gpui::test]
async fn pane_line_ending_menu_targets_active_document_and_undo(cx: &mut TestAppContext) {
    init_status_bar_test_app(cx);
    let original = "background\r\ntext\r\n";
    let (root, visual) = cx.add_window_view(|_window, cx| {
        crate::editor::Editor::from_markdown(cx, original.into(), None)
    });
    root.update(visual, |root, cx| {
        root.split_pane_toward(crate::editor::panes::PaneSplitDirection::Right, cx);
    });
    redraw_status_bar_test(visual);
    let background = root.read_with(visual, |root, cx| {
        root.focused_pane_entities(cx)
            .0
            .expect("background Markdown pane")
    });
    root.update(visual, |root, cx| {
        let focused = root
            .pane_workspace
            .as_ref()
            .expect("workspace")
            .read(cx)
            .workspace()
            .focused_pane();
        assert!(root.new_document_tab_in_pane(focused, crate::editor::DocumentKind::Markdown, cx));
    });
    redraw_status_bar_test(visual);
    let pane = root.read_with(visual, |root, cx| {
        root.focused_pane_entities(cx)
            .0
            .expect("active Markdown pane")
    });
    root.update(visual, |root, cx| {
        root.normalize_line_endings(gmark_document::LineEnding::Cr, cx)
    });
    redraw_status_bar_test(visual);
    background.read_with(visual, |background, _| {
        assert_eq!(
            background.source_document.serialized_bytes(),
            original.as_bytes()
        );
    });
    pane.read_with(visual, |pane, _| {
        assert_eq!(
            pane.source_document.source_format().dominant,
            gmark_document::LineEnding::Cr
        );
        assert!(pane.source_document.is_dirty());
        assert_eq!(pane.undo_history.len(), 1);
    });
    pane.update(visual, |pane, cx| pane.undo_document(cx));
    redraw_status_bar_test(visual);
    pane.read_with(visual, |pane, _| {
        assert_eq!(
            pane.source_document.source_format().dominant,
            gmark_document::LineEnding::Lf
        );
        assert!(pane.source_document.text().is_empty());
    });
    root.update(visual, |root, cx| {
        root.set_view_mode(super::super::ViewMode::Preview, cx)
    });
    redraw_status_bar_test(visual);
    let revision = pane.read_with(visual, |pane, _| pane.source_document.revision());
    root.update(visual, |root, cx| {
        root.normalize_line_endings(gmark_document::LineEnding::CrLf, cx)
    });
    pane.read_with(visual, |pane, _| {
        assert_eq!(pane.source_document.revision(), revision)
    });
}

/// 鼠标和键盘选择格式后立即续写，验证菜单移除不会遗留无效焦点，且源码选区不被折叠。
#[gpui::test]
async fn pane_line_ending_choice_restores_source_focus_for_immediate_typing(
    cx: &mut TestAppContext,
) {
    init_status_bar_test_app(cx);
    let (root, visual) = cx.add_window_view(|_window, cx| {
        crate::editor::Editor::from_markdown(cx, "alpha\nbeta\n".into(), None)
    });
    visual.simulate_resize(gpui::size(gpui::px(960.0), gpui::px(640.0)));
    root.update(visual, |root, cx| {
        root.split_pane_toward(crate::editor::panes::PaneSplitDirection::Right, cx);
        root.set_view_mode(super::super::ViewMode::Source, cx);
    });
    redraw_status_bar_test(visual);
    let pane = root.read_with(visual, |root, cx| root.focused_pane_entities(cx).0.unwrap());
    let source = pane.read_with(visual, |pane, _cx| {
        pane.document.first_root().unwrap().clone()
    });
    source.update_in(visual, |source, window, cx| {
        source.selected_range = 0..5;
        source.selection_reversed = true;
        source.focus_handle.focus(window);
        cx.notify();
    });
    redraw_status_bar_test(visual);
    let picker = visual
        .debug_bounds("status-bar-line-ending-button")
        .unwrap();
    visual.simulate_click(picker.center(), gpui::Modifiers::default());
    redraw_status_bar_test(visual);
    let crlf = visual.debug_bounds("status-bar-line-ending-crlf").unwrap();
    visual.simulate_click(crlf.center(), gpui::Modifiers::default());
    redraw_status_bar_test(visual);
    visual.update(|window, cx| assert!(source.read(cx).focus_handle.is_focused(window)));
    source.read_with(visual, |source, _cx| {
        assert_eq!(source.selected_range, 0..5);
        assert!(source.selection_reversed);
    });
    visual.simulate_input("X");
    redraw_status_bar_test(visual);
    pane.read_with(visual, |pane, _cx| {
        assert_eq!(pane.source_document.serialized_bytes(), b"X\r\nbeta\r\n");
    });
    pane.update(visual, |pane, cx| pane.undo_document(cx));
    redraw_status_bar_test(visual);
    let restored_source = pane.read_with(visual, |pane, _cx| {
        pane.document.first_root().unwrap().clone()
    });
    root.update_in(visual, |root, window, _cx| {
        root.status_bar
            .line_ending_button_focus_handle
            .as_ref()
            .unwrap()
            .focus(window);
    });
    visual.simulate_keystrokes("enter");
    redraw_status_bar_test(visual);
    visual.simulate_keystrokes("enter");
    redraw_status_bar_test(visual);
    visual.update(|window, cx| assert!(restored_source.read(cx).focus_handle.is_focused(window)));
    visual.simulate_input("N");
    redraw_status_bar_test(visual);
    pane.read_with(visual, |pane, _cx| {
        assert_eq!(pane.source_document.serialized_bytes(), b"N\nbeta\n");
    });
}
