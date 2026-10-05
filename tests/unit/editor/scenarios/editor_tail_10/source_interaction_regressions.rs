// @author kongweiguang

use super::*;
use std::path::PathBuf;

#[path = "source_cursor_column.rs"]
mod source_cursor_column;
#[path = "source_pointer_input.rs"]
mod source_pointer_input;

/// 分页与 Markdown 标签共处恢复窗格时，系统标题和修改标记必须跟随实际输入目标。
#[gpui::test]
async fn paged_pane_window_title_tracks_active_tab_and_dirty(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let temp = tempfile::tempdir().expect("pane title tempdir");
    let path = temp.path().join("paged-title.txt");
    fs::write(&path, "alpha beta\n").expect("pane title fixture");
    let (editor, visual) = open_paged_source(cx, path);
    editor.update(visual, |editor, cx| {
        editor.split_pane_toward(crate::editor::panes::PaneSplitDirection::Right, cx);
        let pane = editor.pane_workspace.as_ref().expect("workspace").read(cx).workspace().focused_pane();
        assert!(editor.new_document_tab_in_pane(pane, DocumentKind::Markdown, cx));
    });
    visual.run_until_parked();
    redraw(visual);
    assert_eq!(visual.window_title().as_deref(), Some("Gmark"));
    editor.update(visual, |editor, cx| {
        let workspace = editor.pane_workspace.as_ref().expect("pane workspace").read(cx).workspace();
        let pane = workspace.focused_pane();
        let tab = workspace.pane(pane).expect("focused pane").tabs()[0].id();
        editor.handle_pane_event(crate::editor::panes::PaneEvent::ActivateTab { pane, tab }, None, cx);
    });
    visual.run_until_parked();
    redraw(visual);
    assert_eq!(visual.window_title().as_deref(), Some("Gmark - paged-title.txt"));
    let row = visual.debug_bounds("document-host-line-body-0").expect("source input row");
    visual.simulate_click(row.center(), Modifiers::default());
    visual.simulate_input("7");
    visual.run_until_parked();
    redraw(visual);
    assert!(visual.window_title().expect("dirty title").ends_with("Gmark - paged-title.txt"));
    assert_ne!(visual.window_title().as_deref(), Some("Gmark - paged-title.txt"));
    for (index, expected) in [(1, "Gmark"), (0, "Gmark - paged-title.txt")] {
        editor.update(visual, |editor, cx| {
            let workspace = editor.pane_workspace.as_ref().expect("workspace").read(cx).workspace();
            let pane = workspace.focused_pane();
            let tab = workspace.pane(pane).expect("focused pane").tabs()[index].id();
            editor.handle_pane_event(crate::editor::panes::PaneEvent::ActivateTab { pane, tab }, None, cx);
        });
        visual.run_until_parked();
        redraw(visual);
        redraw(visual);
        let title = visual.window_title().expect("active pane title");
        if index == 1 {
            assert_eq!(title, expected, "dirty background Host must not replace active title");
        } else {
            assert!(title.ends_with(expected));
            assert_ne!(title, expected, "reactivating dirty Host must retain marker");
        }
    }
    let row = visual.debug_bounds("document-host-line-body-0").expect("reactivated source row");
    visual.simulate_click(row.center(), Modifiers::default());
    visual.simulate_keystrokes("ctrl-z");
    visual.run_until_parked();
    redraw(visual);
    assert_eq!(visual.window_title().as_deref(), Some("Gmark - paged-title.txt"));
}

/// Creates the bounded Paged Source test surface once so each regression targets an interaction boundary.
fn open_paged_source(
    cx: &mut TestAppContext,
    path: PathBuf,
) -> (gpui::Entity<Editor>, &mut gpui::VisualTestContext) {
    let probe = gmark_paged_document::probe_file(
        &path,
        gmark_paged_document::ProbeOptions {
            max_resident_bytes: 1,
            ..gmark_paged_document::ProbeOptions::default()
        },
    )
    .expect("Paged Source probe");
    assert_eq!(probe.strategy, gmark_paged_document::OpenStrategy::Paged);
    let source = gmark_paged_document::FileSource::open(&path).expect("Paged Source file");
    let (editor, visual) = cx.add_window_view(move |_window, cx| {
        Editor::from_source_backed_file(cx, path, probe, source)
    });
    visual.run_until_parked();
    redraw(visual);
    (editor, visual)
}

/// 真实多击事件会先由 Block 选词再冒泡到 Host；末端不能重新命中并吞掉词后分隔符。
#[gpui::test]
async fn large_source_double_click_copies_only_word_and_replacement_keeps_separator(
    cx: &mut TestAppContext,
) {
    init_editor_test_app(cx);
    let temp = tempfile::tempdir().expect("double-click word tempdir");
    let path = temp.path().join("double-click-word.txt");
    let original = "000000 alpha_beta 中文输入\nsecond line\n";
    fs::write(&path, original).expect("double-click word fixture");
    let (editor, visual) = open_paged_source(cx, path);
    let host = editor
        .read_with(visual, |editor, _cx| editor.document_host.clone())
        .expect("double-click word Host");
    let bounds = visual
        .debug_bounds("document-host-line-body-0")
        .expect("first row bounds");
    let position = point(bounds.left() + px(90.0), bounds.center().y);
    visual.simulate_click(position, Modifiers::default());
    visual.simulate_event(gpui::MouseDownEvent {
        position,
        modifiers: Modifiers::default(),
        button: MouseButton::Left,
        click_count: 2,
        first_mouse: false,
    });
    visual.simulate_mouse_move(position, MouseButton::Left, Modifiers::default());
    visual.simulate_mouse_up(position, MouseButton::Left, Modifiers::default());
    visual.run_until_parked();
    visual.simulate_keystrokes("ctrl-c");
    assert_eq!(
        visual.read(|cx| cx.read_from_clipboard().and_then(|item| item.text())),
        Some("alpha_beta".into()),
        "double click must not copy the separator after the selected word"
    );
    visual.simulate_input("你");
    visual.run_until_parked();
    assert_eq!(
        host.read_with(visual, |view, _cx| view.source_text_for_test()),
        "000000 你 中文输入\nsecond line\n"
    );
    visual.update(|window, cx| {
        host.update(cx, |view, cx| view.undo_for_test(window, cx));
    });
    assert_eq!(
        host.read_with(visual, |view, _cx| view.source_text_for_test()),
        original
    );
    assert_eq!(
        host.read_with(visual, |view, _cx| view.source_selection_for_test())
            .expect("restored word selection")
            .range(),
        7..17
    );
}

/// 跨行选区替换后的连续输入必须合并为一次撤销，并恢复原方向选区。
#[gpui::test]
async fn large_source_cross_line_typing_replaces_selection_and_undo_restores_it(
    cx: &mut TestAppContext,
) {
    init_editor_test_app(cx);
    let temp = tempfile::tempdir().expect("cross-line typing tempdir");
    let path = temp.path().join("cross-line-typing.txt");
    let original = "alpha\nbeta\ngamma\n";
    fs::write(&path, original).expect("cross-line typing fixture");
    let (editor, visual) = open_paged_source(cx, path);
    let host = editor
        .read_with(visual, |editor, _cx| editor.document_host.clone())
        .expect("cross-line typing Host");
    let original_selection = gmark_document_core::SourceSelection::from_range(3..14, true);
    visual.update(|window, cx| {
        host.update(cx, |view, _cx| {
            view.select_source_range_and_focus_for_test(3..14, true, window);
        });
    });
    redraw(visual);

    visual.simulate_input("替换");
    visual.run_until_parked();
    assert_eq!(
        host.read_with(visual, |view, _cx| view.source_text_for_test()),
        "alp替换ma\n",
        "the first input must replace the selected cross-line bytes"
    );
    visual.simulate_input("🙂");
    visual.run_until_parked();
    assert_eq!(
        host.read_with(visual, |view, _cx| view.source_text_for_test()),
        "alp替换🙂ma\n",
        "consecutive text input must continue at the reanchored caret"
    );
    visual.update(|window, cx| {
        host.update(cx, |view, cx| view.undo_for_test(window, cx));
    });
    assert_eq!(
        host.read_with(visual, |view, _cx| view.source_text_for_test()),
        original
    );
    assert_eq!(
        host.read_with(visual, |view, _cx| view.source_selection_for_test()),
        Some(original_selection),
        "undo must restore the byte range and its reverse direction"
    );
}

/// 非首行超长内容重锚后，行内偏移必须保住活动输入块以接收后续文字。
#[gpui::test]
async fn large_source_nonfirst_long_line_replacement_keeps_continuous_typing(
    cx: &mut TestAppContext,
) {
    init_editor_test_app(cx);
    let temp = tempfile::tempdir().expect("nonfirst long-line typing tempdir");
    let path = temp.path().join("nonfirst-long-line.txt");
    let first_line = "p".repeat(64 * 1024 + 1);
    let target_line = "a".repeat(3 * 64 * 1024);
    let target_column = 50 * 1024;
    let line_start = u64::try_from(first_line.len() + 1).expect("second line start");
    let selection_start = line_start + target_column as u64;
    let original_selection = gmark_document_core::SourceSelection::from_range(
        selection_start..selection_start + 4,
        false,
    );
    let original = format!("{first_line}\n{target_line}\n");
    fs::write(&path, &original).expect("nonfirst long-line fixture");
    let (editor, visual) = open_paged_source(cx, path);
    let host = editor
        .read_with(visual, |editor, _cx| editor.document_host.clone())
        .expect("nonfirst long-line typing Host");
    visual.update(|window, cx| {
        host.update(cx, |view, _cx| {
            view.select_source_range_and_focus_for_test(
                selection_start..selection_start + 4,
                false,
                window,
            );
        });
    });
    redraw(visual);

    visual.simulate_input("XY");
    visual.run_until_parked();
    assert_eq!(
        host.read_with(visual, |view, _cx| view
            .active_edit_for_test()
            .map(|(line, _)| line)),
        Some(1),
        "the reanchored input owner must remain active on the second logical line"
    );
    let after_replacement = host.read_with(visual, |view, _cx| view.source_text_for_test());
    let selection_start = usize::try_from(selection_start).expect("selection start");
    assert_eq!(
        after_replacement.get(selection_start - 2..selection_start + 7),
        Some("aaXYaaaaa"),
        "the first replacement must stay at the selected column"
    );

    visual.simulate_input("🙂");
    visual.run_until_parked();
    let after_continuation = host.read_with(visual, |view, _cx| view.source_text_for_test());
    assert_eq!(
        after_continuation.get(selection_start - 2..selection_start + 9),
        Some("aaXY🙂aaa"),
        "continued input must land after the replacement in the same long row"
    );
    visual.update(|window, cx| {
        host.update(cx, |view, cx| view.undo_for_test(window, cx));
    });
    assert_eq!(
        host.read_with(visual, |view, _cx| view.source_text_for_test()),
        original,
        "one undo must restore the original large source document"
    );
    assert_eq!(
        host.read_with(visual, |view, _cx| view.source_selection_for_test()),
        Some(original_selection),
        "undo must restore the selected range on the nonfirst line"
    );
}

/// IME 候选绑定到活动端点所在的虚拟 Block，取消不落盘，确认后一次提交且撤销恢复全文选区。
#[cfg(target_os = "windows")]
#[gpui::test]
async fn large_source_cross_line_ime_cancel_commit_and_undo_are_atomic(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let temp = tempfile::tempdir().expect("cross-line IME tempdir");
    let path = temp.path().join("cross-line-ime.txt");
    let original = "alpha\nbeta\ngamma\n";
    fs::write(&path, original).expect("cross-line IME fixture");
    let (editor, visual) = open_paged_source(cx, path);
    let host = editor
        .read_with(visual, |editor, _cx| editor.document_host.clone())
        .expect("cross-line IME Host");
    let original_selection = gmark_document_core::SourceSelection::from_range(3..11, true);
    visual.update(|window, cx| {
        host.update(cx, |view, _cx| {
            view.select_source_range_and_focus_for_test(3..11, true, window);
        });
    });
    redraw(visual);
    let owner = host
        .read_with(visual, |view, _cx| view.source_row_block_for_test(0))
        .expect("the Source Block containing the active head must be mounted");
    let composing = "拼音🙂";
    let composing_end = composing.encode_utf16().count();
    visual.update(|window, cx| {
        owner.update(cx, |block, block_cx| {
            <crate::components::Block as gpui::EntityInputHandler>::replace_and_mark_text_in_range(
                block,
                None,
                composing,
                Some(composing_end..composing_end),
                window,
                block_cx,
            );
        });
    });
    visual.run_until_parked();
    owner.read_with(visual, |block, _cx| {
        assert!(block.has_ime_composition());
        assert_eq!(
            block
                .ime_visible_text(&crate::components::BlockImeCompositionOwner::BlockText)
                .as_deref(),
            Some("alp拼音🙂")
        );
    });
    assert_eq!(
        host.read_with(visual, |view, _cx| view.source_text_for_test()),
        original,
        "preedit must not enter the Paged document"
    );

    visual.update(|window, cx| {
        owner.update(cx, |block, block_cx| {
            <crate::components::Block as gpui::EntityInputHandler>::composition_ended(
                block,
                gpui::CompositionEnd::Cancelled,
                window,
                block_cx,
            );
        });
    });
    visual.run_until_parked();
    assert_eq!(
        host.read_with(visual, |view, _cx| view.source_text_for_test()),
        original
    );
    assert_eq!(
        host.read_with(visual, |view, _cx| view.source_selection_for_test()),
        Some(original_selection),
        "cancelling composition must leave the original shared selection intact"
    );

    visual.update(|window, cx| {
        owner.update(cx, |block, block_cx| {
            <crate::components::Block as gpui::EntityInputHandler>::replace_and_mark_text_in_range(
                block,
                None,
                composing,
                Some(composing_end..composing_end),
                window,
                block_cx,
            );
            <crate::components::Block as gpui::EntityInputHandler>::replace_text_in_range(
                block, None, "汉🙂", window, block_cx,
            );
            <crate::components::Block as gpui::EntityInputHandler>::composition_ended(
                block,
                gpui::CompositionEnd::Committed,
                window,
                block_cx,
            );
        });
    });
    visual.run_until_parked();
    assert_eq!(
        host.read_with(visual, |view, _cx| view.source_text_for_test()),
        "alp汉🙂gamma\n",
        "the finalized text must replace the original cross-line range once"
    );
    visual.update(|window, cx| {
        host.update(cx, |view, cx| view.undo_for_test(window, cx));
    });
    assert_eq!(
        host.read_with(visual, |view, _cx| view.source_text_for_test()),
        original
    );
    assert_eq!(
        host.read_with(visual, |view, _cx| view.source_selection_for_test()),
        Some(original_selection),
        "one undo must restore both full source bytes and the original selection"
    );
}

/// IME 终态后紧随的普通按键可能仍带着旧跨块投影，但必须写入已提交插入点。
#[cfg(target_os = "windows")]
#[gpui::test]
async fn large_source_ime_commit_keeps_immediate_following_character(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let temp = tempfile::tempdir().expect("IME continuation tempdir");
    let path = temp.path().join("ime-continuation.txt");
    let original = "alpha_beta\r\nsecond line\r\n";
    fs::write(&path, original).expect("IME continuation fixture");
    let (editor, visual) = open_paged_source(cx, path);
    let host = editor
        .read_with(visual, |editor, _cx| editor.document_host.clone())
        .expect("IME continuation Host");
    for reversed in [false, true] {
        let original_selection = gmark_document_core::SourceSelection::from_range(0..10, reversed);
        visual.update(|window, cx| {
            host.update(cx, |view, _cx| {
                view.select_source_range_and_focus_for_test(0..10, reversed, window);
            });
        });
        redraw(visual);
        let owner = host
            .read_with(visual, |view, _cx| view.source_row_block_for_test(0))
            .expect("mounted Source IME owner");

        visual.update(|window, cx| {
            owner.update(cx, |block, block_cx| {
                <crate::components::Block as gpui::EntityInputHandler>::replace_and_mark_text_in_range(
                    block,
                    None,
                    "ni",
                    Some(2..2),
                    window,
                    block_cx,
                );
                <crate::components::Block as gpui::EntityInputHandler>::replace_text_in_range(
                    block, None, "你", window, block_cx,
                );
                <crate::components::Block as gpui::EntityInputHandler>::composition_ended(
                    block,
                    gpui::CompositionEnd::Committed,
                    window,
                    block_cx,
                );
                <crate::components::Block as gpui::EntityInputHandler>::replace_text_in_range(
                    block, None, "7", window, block_cx,
                );
            });
        });
        visual.run_until_parked();
        assert_eq!(
            host.read_with(visual, |view, _cx| view.source_text_for_test()),
            "你7\r\nsecond line\r\n",
            "the character after candidate confirmation must land at the committed caret"
        );

        visual.update(|window, cx| {
            host.update(cx, |view, cx| view.undo_for_test(window, cx));
        });
        assert_eq!(
            host.read_with(visual, |view, _cx| view.source_text_for_test()),
            "你\r\nsecond line\r\n",
            "undo must first remove the ordinary character without reverting the IME commit"
        );
        assert_eq!(
            host.read_with(visual, |view, _cx| view.source_selection_for_test())
                .expect("caret after undoing the ordinary character")
                .range(),
            3..3
        );

        visual.update(|window, cx| {
            host.update(cx, |view, cx| view.undo_for_test(window, cx));
        });
        let restored = host.read_with(visual, |view, _cx| view.source_text_for_test());
        assert_eq!(
            restored.as_bytes(),
            original.as_bytes(),
            "undoing the IME commit must restore the original bytes, including CRLF"
        );
        assert_eq!(
            host.read_with(visual, |view, _cx| view.source_selection_for_test()),
            Some(original_selection),
            "undoing the IME commit must restore the original selection direction"
        );
    }
}

/// Host 焦点转移后，仍挂载的旧行收到普通原生文字事件也不得再改正文。
#[gpui::test]
async fn large_source_late_native_text_after_host_blur_is_ignored(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let temp = tempfile::tempdir().expect("late Source input tempdir");
    let path = temp.path().join("late-source-input.txt");
    let original = "alpha beta\ngamma\n";
    fs::write(&path, original).expect("late Source input fixture");
    let (editor, visual) = open_paged_source(cx, path);
    let host = editor
        .read_with(visual, |editor, _cx| editor.document_host.clone())
        .expect("late Source input Host");
    visual.update(|window, cx| {
        host.update(cx, |view, _cx| {
            view.select_source_range_and_focus_for_test(1..5, false, window);
        });
    });
    redraw(visual);
    let owner = host
        .read_with(visual, |view, _cx| view.source_row_block_for_test(0))
        .expect("mounted Source input owner");

    visual.update(|window, cx| {
        let other_focus = cx.focus_handle();
        other_focus.focus(window);
        owner.update(cx, |block, block_cx| {
            <crate::components::Block as gpui::EntityInputHandler>::replace_text_in_range(
                block, None, "late", window, block_cx,
            );
        });
    });
    visual.run_until_parked();
    assert_eq!(
        host.read_with(visual, |view, _cx| view.source_text_for_test()),
        original,
        "a still-mounted bridge must reject ordinary text after Host focus moved"
    );
}

/// 重叠的 peer revision 必须取消暂存候选，并拒绝随后到达的旧输入目标确认。
#[cfg(target_os = "windows")]
#[gpui::test]
async fn large_source_ime_rejects_late_commit_after_shared_overlap(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let temp = tempfile::tempdir().expect("shared IME conflict tempdir");
    let path = temp.path().join("shared-ime-conflict.txt");
    let original = "alpha\nbeta\ngamma\n";
    fs::write(&path, original).expect("shared IME conflict fixture");
    let (editor, visual) = open_paged_source(cx, path);
    let host = editor
        .read_with(visual, |editor, _cx| editor.document_host.clone())
        .expect("shared IME conflict Host");
    visual.update(|window, cx| {
        host.update(cx, |view, _cx| {
            view.select_source_range_and_focus_for_test(3..11, true, window);
        });
    });
    redraw(visual);
    let owner = host
        .read_with(visual, |view, _cx| view.source_row_block_for_test(0))
        .expect("mounted IME owner row");
    let composing = "拼音🙂";
    let composing_end = composing.encode_utf16().count();
    visual.update(|window, cx| {
        owner.update(cx, |block, block_cx| {
            <crate::components::Block as gpui::EntityInputHandler>::replace_and_mark_text_in_range(
                block,
                None,
                composing,
                Some(composing_end..composing_end),
                window,
                block_cx,
            );
        });
    });
    visual.run_until_parked();

    host.update(visual, |view, cx| {
        view.apply_peer_source_edit_and_rebase_ime_for_test(6..8, "peer", cx)
            .expect("overlapping peer revision");
    });
    visual.run_until_parked();
    let peer_text = "alpha\npeerta\ngamma\n";
    assert_eq!(
        host.read_with(visual, |view, _cx| view.source_text_for_test()),
        peer_text,
        "the shared revision must be visible before the delayed native result"
    );

    visual.update(|window, cx| {
        owner.update(cx, |block, block_cx| {
            <crate::components::Block as gpui::EntityInputHandler>::replace_text_in_range(
                block, None, "汉🙂", window, block_cx,
            );
            <crate::components::Block as gpui::EntityInputHandler>::composition_ended(
                block,
                gpui::CompositionEnd::Committed,
                window,
                block_cx,
            );
        });
    });
    visual.run_until_parked();
    assert_eq!(
        host.read_with(visual, |view, _cx| view.source_text_for_test()),
        peer_text,
        "a native result pinned to the overlapping old selection must not overwrite peer text"
    );
}

/// 双击词后真实同点和跨词 MouseMove 必须继续按完整 Unicode 词边界扩选。
#[gpui::test]
async fn large_source_double_click_word_survives_real_mouse_moves(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let temp = tempfile::tempdir().expect("double-click Source tempdir");
    let path = temp.path().join("double-click-source.txt");
    let original = "zero alpha beta gamma\nnext row\n";
    fs::write(&path, original).expect("double-click Source fixture");
    let (editor, visual) = open_paged_source(cx, path);
    let host = editor
        .read_with(visual, |editor, _cx| editor.document_host.clone())
        .expect("double-click Source Host");
    let row = visual
        .debug_bounds("document-host-line-body-0")
        .expect("first Source row");
    let block = host
        .read_with(visual, |view, _cx| view.source_row_block_for_test(0))
        .expect("first Source row input");
    let word_position = (0..180)
        .map(|x| point(row.left() + px(x as f32), row.center().y))
        .find(|position| {
            (12..15).contains(&block.read_with(visual, |block, _cx| {
                block.index_for_mouse_position(*position)
            }))
        })
        .expect("a hit inside beta");
    visual.update(|window, cx| {
        host.update(cx, |view, cx| {
            view.activate_source_pointer_for_test(0, word_position, 2, window, cx);
        });
    });
    let selected_word = gmark_document_core::SourceSelection::from_range(11..15, false);
    assert_eq!(
        host.read_with(visual, |view, _cx| view.source_selection_for_test()),
        Some(selected_word)
    );

    visual.simulate_mouse_move(word_position, MouseButton::Left, Modifiers::default());
    visual.run_until_parked();
    assert_eq!(
        host.read_with(visual, |view, _cx| view.source_selection_for_test()),
        Some(selected_word),
        "a stationary move emitted during double-click must not collapse the word"
    );
    let next_word_position = (0..180)
        .map(|x| point(row.left() + px(x as f32), row.center().y))
        .find(|position| {
            (17..21).contains(&block.read_with(visual, |block, _cx| {
                block.index_for_mouse_position(*position)
            }))
        })
        .expect("a hit inside gamma");
    visual.simulate_mouse_move(next_word_position, MouseButton::Left, Modifiers::default());
    visual.run_until_parked();
    assert_eq!(
        host.read_with(visual, |view, _cx| view.source_selection_for_test())
            .map(|selection| selection.range()),
        Some(11..21),
        "word dragging must extend to the target word's outer boundary"
    );
    visual.simulate_mouse_up(next_word_position, MouseButton::Left, Modifiers::default());
}

/// 三击后真实 MouseMove 保持逻辑行粒度，跨行时将终点扩展到目标行边界。
#[gpui::test]
async fn large_source_triple_click_line_drag_preserves_logical_rows(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let temp = tempfile::tempdir().expect("triple-click Source tempdir");
    let path = temp.path().join("triple-click-source.txt");
    let original = "first row\nsecond row\nthird row";
    fs::write(&path, original).expect("triple-click Source fixture");
    let (editor, visual) = open_paged_source(cx, path);
    let host = editor
        .read_with(visual, |editor, _cx| editor.document_host.clone())
        .expect("triple-click Source Host");
    let first_row = visual
        .debug_bounds("document-host-line-body-0")
        .expect("first logical row");
    let first_block = host
        .read_with(visual, |view, _cx| view.source_row_block_for_test(0))
        .expect("first logical row input");
    let first_position = (0..80)
        .map(|x| point(first_row.left() + px(x as f32), first_row.center().y))
        .find(|position| {
            (1..8).contains(&first_block.read_with(visual, |block, _cx| {
                block.index_for_mouse_position(*position)
            }))
        })
        .expect("a hit inside the first logical row");
    visual.update(|window, cx| {
        host.update(cx, |view, cx| {
            view.activate_source_pointer_for_test(0, first_position, 3, window, cx);
        });
    });
    let first_line_selection = host
        .read_with(visual, |view, _cx| view.source_selection_for_test())
        .expect("triple-click line selection");
    visual.simulate_mouse_move(first_position, MouseButton::Left, Modifiers::default());
    visual.run_until_parked();
    assert_eq!(
        host.read_with(visual, |view, _cx| view.source_selection_for_test()),
        Some(first_line_selection),
        "a stationary move after triple-click must preserve the selected row"
    );

    let second_row = visual
        .debug_bounds("document-host-line-body-1")
        .expect("second logical row");
    let second_block = host
        .read_with(visual, |view, _cx| view.source_row_block_for_test(1))
        .expect("second logical row input");
    let second_position = (0..80)
        .map(|x| point(second_row.left() + px(x as f32), second_row.center().y))
        .find(|position| {
            (1..9).contains(&second_block.read_with(visual, |block, _cx| {
                block.index_for_mouse_position(*position)
            }))
        })
        .expect("a hit inside the second logical row");
    visual.simulate_mouse_move(second_position, MouseButton::Left, Modifiers::default());
    visual.run_until_parked();
    let dragged = host
        .read_with(visual, |view, _cx| view.source_selection_for_test())
        .expect("line drag selection");
    assert_eq!(dragged.range().start, 0);
    assert!(
        dragged.range().end >= "first row\nsecond row".len() as u64,
        "line granularity should include the target row: {dragged:?}"
    );
    visual.simulate_mouse_up(second_position, MouseButton::Left, Modifiers::default());
}

/// 原生剪贴板预检失败时，跨行 Cut 必须保留正文、共享选区和原剪贴板内容。
#[gpui::test]
async fn large_source_cross_line_cut_failure_keeps_document_and_selection(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let temp = tempfile::tempdir().expect("cut failure tempdir");
    let path = temp.path().join("cut-failure-source.txt");
    let original = "alpha\nbeta\ngamma\n";
    fs::write(&path, original).expect("cut failure fixture");
    let (editor, visual) = open_paged_source(cx, path);
    let host = editor
        .read_with(visual, |editor, _cx| editor.document_host.clone())
        .expect("cut failure Source Host");
    let original_selection = gmark_document_core::SourceSelection::from_range(3..14, true);
    let dirty_before = host.read_with(visual, |view, _cx| view.is_dirty());
    visual.write_to_clipboard(gpui::ClipboardItem::new_string("sentinel".to_owned()));
    visual.update(|window, cx| {
        host.update(cx, |view, cx| {
            view.select_source_range_and_focus_for_test(3..14, true, window);
            view.fail_next_native_clipboard_write_for_test();
            view.cut_for_test(window, cx);
        });
    });
    visual.run_until_parked();

    assert_eq!(
        host.read_with(visual, |view, _cx| view.source_text_for_test()),
        original,
        "failed native clipboard acceptance must not delete Source bytes"
    );
    assert_eq!(
        host.read_with(visual, |view, _cx| view.source_selection_for_test()),
        Some(original_selection)
    );
    assert_eq!(
        host.read_with(visual, |view, _cx| view.is_dirty()),
        dirty_before,
        "a rejected native write must not change dirty state"
    );
    assert_eq!(
        visual
            .read_from_clipboard()
            .and_then(|item| item.text().map(|text| text.to_string()))
            .as_deref(),
        Some("sentinel"),
        "a failed cut must leave the previous clipboard item untouched"
    );
    visual.update(|window, cx| {
        host.update(cx, |view, cx| view.undo_for_test(window, cx));
    });
    visual.run_until_parked();
    assert_eq!(
        host.read_with(visual, |view, _cx| view.source_text_for_test()),
        original,
        "a rejected native write must not add an undo entry"
    );
}
