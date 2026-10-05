// @author kongweiguang

use super::*;

/// 从实际 Block 排版中找到目标字节，避免把字符宽度写死到焦点回归测试。
fn source_pointer_over_local_range(
    visual: &gpui::VisualTestContext,
    block: &gpui::Entity<crate::components::Block>,
    row_bounds: gpui::Bounds<gpui::Pixels>,
    target: std::ops::Range<usize>,
) -> gpui::Point<gpui::Pixels> {
    (0..400)
        .map(|x| point(row_bounds.left() + px(x as f32), row_bounds.center().y))
        .find(|position| {
            target.contains(&block.read_with(visual, |block, _cx| {
                block.index_for_mouse_position(*position)
            }))
        })
        .expect("target byte range must be hittable in the visible Source row")
}

/// 注册 GPUI 测试依赖，避免导航回归因应用全局状态缺失而偏离真实事件路径。
fn init_source_navigation_test_app(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        crate::i18n::I18nManager::init(cx);
        crate::theme::ThemeManager::init(cx);
        crate::components::init(cx);
    });
}

/// 64 KiB 行窗口恰好容纳家庭 emoji 的首个码点时，Right 仍必须跨过整个扩展字素。
#[gpui::test]
async fn paged_source_right_crosses_grapheme_at_bounded_window_edge(cx: &mut gpui::TestAppContext) {
    init_source_navigation_test_app(cx);
    let temp = tempfile::tempdir().expect("bounded grapheme tempdir");
    let path = temp.path().join("grapheme-window-edge.txt");
    let family = "👨‍👩‍👧‍👦";
    let first_codepoint_len = family.chars().next().expect("family emoji").len_utf8();
    let prefix_len = MAX_RENDERED_LINE_BYTES as usize - first_codepoint_len;
    let text = format!("{}{}tail\n", "a".repeat(prefix_len), family);
    fs::write(&path, &text).expect("grapheme window fixture");
    let probe = gmark_paged_document::probe_file(
        &path,
        gmark_paged_document::ProbeOptions {
            max_resident_bytes: 1,
            ..gmark_paged_document::ProbeOptions::default()
        },
    )
    .expect("Paged Source probe");
    assert_eq!(probe.strategy, OpenStrategy::Paged);
    let source = FileSource::open(&path).expect("Paged Source file");
    let (host, visual) =
        cx.add_window_view(move |_window, cx| DocumentHost::new(path, probe, source, cx));
    visual.run_until_parked();

    visual.update(|window, cx| {
        host.update(cx, |host, _cx| {
            host.select_source_range_and_focus_for_test(
                prefix_len as u64..prefix_len as u64,
                false,
                window,
            );
        });
    });
    visual.simulate_keystrokes("right");

    let expected_end = (prefix_len + family.len()) as u64;
    assert_eq!(
        host.read_with(visual, |host, _cx| host.source_selection_for_test())
            .expect("grapheme source selection"),
        SourceSelection::collapsed(expected_end, SourceAffinity::After)
    );
}

/// Ctrl+Right 从窗口末端附近的词内移动时，应读取有限的后续窗口并落到真实下一个词首。
#[gpui::test]
async fn paged_source_control_right_finds_word_start_after_bounded_window(
    cx: &mut gpui::TestAppContext,
) {
    init_source_navigation_test_app(cx);
    let temp = tempfile::tempdir().expect("bounded word tempdir");
    let path = temp.path().join("word-window-edge.txt");
    let prefix_len = MAX_RENDERED_LINE_BYTES as usize - "alpha".len();
    let text = format!("{}alpha beta tail\n", "x".repeat(prefix_len));
    fs::write(&path, &text).expect("word window fixture");
    let probe = gmark_paged_document::probe_file(
        &path,
        gmark_paged_document::ProbeOptions {
            max_resident_bytes: 1,
            ..gmark_paged_document::ProbeOptions::default()
        },
    )
    .expect("Paged Source probe");
    assert_eq!(probe.strategy, OpenStrategy::Paged);
    let source = FileSource::open(&path).expect("Paged Source file");
    let (host, visual) =
        cx.add_window_view(move |_window, cx| DocumentHost::new(path, probe, source, cx));
    visual.run_until_parked();

    let cursor = (prefix_len + 1) as u64;
    visual.update(|window, cx| {
        host.update(cx, |host, _cx| {
            host.select_source_range_and_focus_for_test(cursor..cursor, false, window);
        });
    });
    visual.simulate_keystrokes("ctrl-right");

    let next_word_start = (prefix_len + "alpha ".len()) as u64;
    assert_eq!(
        host.read_with(visual, |host, _cx| host.source_selection_for_test())
            .expect("word source selection"),
        SourceSelection::collapsed(next_word_start, SourceAffinity::After)
    );
}

/// RI 奇偶性需要跨窗口前文时继续解析到真实旗帜字素边界，不能静默卡住或误跳行尾。
#[gpui::test]
async fn paged_source_right_resolves_flag_boundary_with_precontext(cx: &mut gpui::TestAppContext) {
    init_source_navigation_test_app(cx);
    let temp = tempfile::tempdir().expect("regional indicator tempdir");
    let path = temp.path().join("regional-indicator-window.txt");
    let text = "🇦".repeat(40_001);
    let window_start = text.len() as u64 - MAX_RENDERED_LINE_BYTES;
    let cursor = window_start + MAX_RENDERED_LINE_BYTES / 4;
    fs::write(&path, &text).expect("regional indicator fixture");
    let probe = gmark_paged_document::probe_file(
        &path,
        gmark_paged_document::ProbeOptions {
            max_resident_bytes: 1,
            ..gmark_paged_document::ProbeOptions::default()
        },
    )
    .expect("Paged Source probe");
    assert_eq!(probe.strategy, OpenStrategy::Paged);
    let source = FileSource::open(&path).expect("Paged Source file");
    let (host, visual) =
        cx.add_window_view(move |_window, cx| DocumentHost::new(path, probe, source, cx));
    visual.run_until_parked();

    visual.update(|window, cx| {
        host.update(cx, |host, _cx| {
            host.select_source_range_and_focus_for_test(cursor..cursor, false, window);
        });
    });
    visual.simulate_keystrokes("right");

    assert_eq!(
        host.read_with(visual, |host, _cx| host.source_selection_for_test())
            .expect("regional indicator source selection"),
        SourceSelection::collapsed((cursor / 8 + 1) * 8, SourceAffinity::After)
    );
}

/// 超过窗口上限的单词仍须能从两端按 Ctrl+方向键到达真实 Unicode 词界。
#[gpui::test]
async fn paged_source_control_navigation_crosses_oversized_ascii_word(
    cx: &mut gpui::TestAppContext,
) {
    init_source_navigation_test_app(cx);
    let temp = tempfile::tempdir().expect("oversized word tempdir");
    let path = temp.path().join("oversized-word-window.txt");
    let word_len = MAX_RENDERED_LINE_BYTES as usize + 4_096;
    let text = format!("{} next\n", "x".repeat(word_len));
    fs::write(&path, &text).expect("oversized word fixture");
    let probe = gmark_paged_document::probe_file(
        &path,
        gmark_paged_document::ProbeOptions {
            max_resident_bytes: 1,
            ..gmark_paged_document::ProbeOptions::default()
        },
    )
    .expect("Paged Source probe");
    assert_eq!(probe.strategy, OpenStrategy::Paged);
    let source = FileSource::open(&path).expect("Paged Source file");
    let (host, visual) =
        cx.add_window_view(move |_window, cx| DocumentHost::new(path, probe, source, cx));
    visual.run_until_parked();

    visual.update(|window, cx| {
        host.update(cx, |host, _cx| {
            host.select_source_range_and_focus_for_test(0..0, false, window);
        });
    });
    visual.simulate_keystrokes("ctrl-right");
    let right = host
        .read_with(visual, |host, _cx| host.source_selection_for_test())
        .expect("Ctrl+Right source selection");

    visual.update(|window, cx| {
        host.update(cx, |host, _cx| {
            host.select_source_range_and_focus_for_test(
                word_len as u64..word_len as u64,
                false,
                window,
            );
        });
    });
    visual.simulate_keystrokes("ctrl-left");
    let left = host
        .read_with(visual, |host, _cx| host.source_selection_for_test())
        .expect("Ctrl+Left source selection");

    assert_eq!(
        (right, left),
        (
            SourceSelection::collapsed((word_len + 1) as u64, SourceAffinity::After),
            SourceSelection::collapsed(0, SourceAffinity::After),
        )
    );
}

/// 前后端点都未挂载时，左右键仍按选区方向折叠到完整源码范围的对应端点。
#[gpui::test]
async fn paged_source_selection_collapses_to_unmounted_endpoints(cx: &mut gpui::TestAppContext) {
    init_source_navigation_test_app(cx);
    let temp = tempfile::tempdir().expect("unmounted selection tempdir");
    let path = temp.path().join("unmounted-selection.txt");
    let mut text = String::new();
    let mut line_starts = Vec::with_capacity(1_000);
    for line in 0..1_000 {
        line_starts.push(text.len() as u64);
        text.push_str(&format!("line-{line:04}\n"));
    }
    fs::write(&path, &text).expect("unmounted selection fixture");
    let probe = gmark_paged_document::probe_file(
        &path,
        gmark_paged_document::ProbeOptions {
            max_resident_bytes: 1,
            ..gmark_paged_document::ProbeOptions::default()
        },
    )
    .expect("Paged Source probe");
    assert_eq!(probe.strategy, OpenStrategy::Paged);
    let source = FileSource::open(&path).expect("Paged Source file");
    let (host, visual) =
        cx.add_window_view(move |_window, cx| DocumentHost::new(path, probe, source, cx));
    visual.run_until_parked();

    let start_line = 500;
    let end_line = 750;
    let selection_range = line_starts[start_line]..line_starts[end_line];
    visual.update(|window, cx| {
        host.update(cx, |host, _cx| {
            host.select_source_range_and_focus_for_test(selection_range.clone(), false, window);
        });
    });
    let endpoints_are_unmounted = host.read_with(visual, |host, _cx| {
        host.source_row_block_for_test(start_line).is_none()
            && host.source_row_block_for_test(end_line).is_none()
    });
    assert!(
        endpoints_are_unmounted,
        "fixture endpoints must not be mounted"
    );
    visual.simulate_keystrokes("left");
    assert_eq!(
        host.read_with(visual, |host, _cx| host.source_selection_for_test())
            .expect("forward selection collapse"),
        SourceSelection::collapsed(selection_range.start, SourceAffinity::After)
    );

    visual.update(|window, cx| {
        host.update(cx, |host, _cx| {
            host.select_source_range_and_focus_for_test(selection_range.clone(), true, window);
        });
    });
    visual.simulate_keystrokes("right");
    assert_eq!(
        host.read_with(visual, |host, _cx| host.source_selection_for_test())
            .expect("reverse selection collapse"),
        SourceSelection::collapsed(selection_range.end, SourceAffinity::After)
    );
}

/// 水平键折叠双击词选区后输入必须落在新插入点，不能继续替换 Block 中的旧选词。
#[gpui::test]
async fn paged_source_horizontal_collapse_then_typing_uses_new_caret(
    cx: &mut gpui::TestAppContext,
) {
    init_source_navigation_test_app(cx);
    let temp = tempfile::tempdir().expect("horizontal typing tempdir");
    let path = temp.path().join("horizontal-typing.txt");
    let original = "000000 alpha_beta\n";
    fs::write(&path, original).expect("horizontal typing fixture");
    let probe = gmark_paged_document::probe_file(
        &path,
        gmark_paged_document::ProbeOptions {
            max_resident_bytes: 1,
            ..gmark_paged_document::ProbeOptions::default()
        },
    )
    .expect("Paged Source probe");
    assert_eq!(probe.strategy, OpenStrategy::Paged);
    let source = FileSource::open(&path).expect("Paged Source file");
    let (host, visual) =
        cx.add_window_view(move |_window, cx| DocumentHost::new(path, probe, source, cx));
    visual.run_until_parked();
    visual.update(|window, cx| window.draw(cx).clear());
    visual.run_until_parked();

    let row = visual
        .debug_bounds("document-host-line-body-0")
        .expect("first Source row");
    let block = host
        .read_with(visual, |host, _cx| host.source_row_block_for_test(0))
        .expect("first Source row input");
    let position = source_pointer_over_local_range(visual, &block, row, 8..10);
    visual.update(|window, cx| {
        host.update(cx, |host, cx| {
            host.activate_source_pointer_for_test(0, position, 2, window, cx);
        });
    });
    assert_eq!(
        host.read_with(visual, |host, _cx| host.source_selection_for_test())
            .expect("double-click word selection")
            .range(),
        7..17
    );

    visual.simulate_keystrokes("right");
    visual.run_until_parked();
    assert_eq!(
        host.read_with(visual, |host, _cx| host.source_selection_for_test())
            .expect("collapsed word selection"),
        SourceSelection::collapsed(17, SourceAffinity::After)
    );

    visual.simulate_input("7");
    visual.run_until_parked();
    assert_eq!(
        host.read_with(visual, |host, _cx| host.source_text_for_test()),
        "000000 alpha_beta7\n",
        "typing after collapsing a selection must use the new shared caret"
    );
}

/// Home 与 End 应把插入点放在当前源码逻辑行两端，而不是整篇文档的边界。
#[gpui::test]
async fn paged_source_home_and_end_stay_on_current_line(cx: &mut gpui::TestAppContext) {
    init_source_navigation_test_app(cx);
    let temp = tempfile::tempdir().expect("line boundary tempdir");
    let path = temp.path().join("line-boundary.txt");
    let original = "alpha beta\nsecond row\n";
    fs::write(&path, original).expect("line boundary fixture");
    let probe = gmark_paged_document::probe_file(
        &path,
        gmark_paged_document::ProbeOptions {
            max_resident_bytes: 1,
            ..gmark_paged_document::ProbeOptions::default()
        },
    )
    .expect("Paged Source probe");
    assert_eq!(probe.strategy, OpenStrategy::Paged);
    let source = FileSource::open(&path).expect("Paged Source file");
    let (host, visual) =
        cx.add_window_view(move |_window, cx| DocumentHost::new(path, probe, source, cx));
    visual.run_until_parked();

    visual.update(|window, cx| {
        host.update(cx, |host, _cx| {
            host.select_source_range_and_focus_for_test(6..6, false, window);
        });
    });
    visual.simulate_keystrokes("home");
    assert_eq!(
        host.read_with(visual, |host, _cx| host.source_selection_for_test())
            .expect("Home source selection")
            .range(),
        0..0
    );
    visual.simulate_input("7");
    visual.run_until_parked();
    assert_eq!(
        host.read_with(visual, |host, _cx| host.source_text_for_test()),
        "7alpha beta\nsecond row\n"
    );
    visual.simulate_keystrokes("ctrl-z");
    visual.run_until_parked();
    assert_eq!(
        host.read_with(visual, |host, _cx| host.source_text_for_test()),
        original
    );
    assert_eq!(
        host.read_with(visual, |host, _cx| host.source_selection_for_test())
            .expect("Home undo selection")
            .range(),
        0..0
    );

    visual.simulate_keystrokes("end");
    assert_eq!(
        host.read_with(visual, |host, _cx| host.source_selection_for_test())
            .expect("End source selection")
            .range(),
        10..10
    );
    visual.simulate_input("8");
    visual.run_until_parked();
    assert_eq!(
        host.read_with(visual, |host, _cx| host.source_text_for_test()),
        "alpha beta8\nsecond row\n"
    );
    visual.simulate_keystrokes("ctrl-z");
    visual.run_until_parked();
    assert_eq!(
        host.read_with(visual, |host, _cx| host.source_text_for_test()),
        original
    );
    assert_eq!(
        host.read_with(visual, |host, _cx| host.source_selection_for_test())
            .expect("End undo selection")
            .range(),
        10..10
    );
}

/// Ctrl+End 必须把 Paged Source 的插入点移动到尚未挂载的文档末尾。
#[gpui::test]
async fn paged_source_control_end_reaches_unmounted_document_tail(cx: &mut gpui::TestAppContext) {
    init_source_navigation_test_app(cx);
    let temp = tempfile::tempdir().expect("document tail tempdir");
    let path = temp.path().join("document-tail.txt");
    let mut original = String::new();
    for line in 0..1_000 {
        original.push_str(&format!("line-{line:04}\n"));
    }
    fs::write(&path, &original).expect("document tail fixture");
    let probe = gmark_paged_document::probe_file(
        &path,
        gmark_paged_document::ProbeOptions {
            max_resident_bytes: 1,
            ..gmark_paged_document::ProbeOptions::default()
        },
    )
    .expect("Paged Source probe");
    assert_eq!(probe.strategy, OpenStrategy::Paged);
    let source = FileSource::open(&path).expect("Paged Source file");
    let (host, visual) =
        cx.add_window_view(move |_window, cx| DocumentHost::new(path, probe, source, cx));
    visual.run_until_parked();

    visual.update(|window, cx| {
        host.update(cx, |host, _cx| {
            host.select_source_range_and_focus_for_test(2..2, false, window);
        });
    });
    assert!(host.read_with(visual, |host, _cx| {
        host.source_row_block_for_test(999).is_none()
    }));
    visual.simulate_keystrokes("ctrl-end");
    visual.run_until_parked();

    let expected_end = original.len() as u64;
    assert_eq!(
        host.read_with(visual, |host, _cx| host.source_selection_for_test())
            .expect("document tail selection")
            .range(),
        expected_end..expected_end
    );
    let before_typing = host
        .read_with(visual, |host, _cx| host.source_selection_for_test())
        .expect("selection before tail typing");
    visual.simulate_input("7");
    visual.run_until_parked();
    assert_eq!(
        host.read_with(visual, |host, _cx| host.source_text_for_test()),
        format!("{original}7")
    );
    visual.simulate_keystrokes("ctrl-z");
    visual.run_until_parked();
    assert_eq!(
        host.read_with(visual, |host, _cx| host.source_text_for_test()),
        original
    );
    assert_eq!(
        host.read_with(visual, |host, _cx| host.source_selection_for_test()),
        Some(before_typing)
    );
}

/// Right 折叠跨行选区后输入并撤销，应恢复正文及输入前的插入点状态。
#[gpui::test]
async fn paged_source_cross_line_collapse_type_and_undo_restore_state(
    cx: &mut gpui::TestAppContext,
) {
    init_source_navigation_test_app(cx);
    let temp = tempfile::tempdir().expect("cross-line undo tempdir");
    let path = temp.path().join("cross-line-undo.txt");
    let original = "first line\nsecond line\n";
    let selected = 2..16;
    fs::write(&path, original).expect("cross-line undo fixture");
    let probe = gmark_paged_document::probe_file(
        &path,
        gmark_paged_document::ProbeOptions {
            max_resident_bytes: 1,
            ..gmark_paged_document::ProbeOptions::default()
        },
    )
    .expect("Paged Source probe");
    assert_eq!(probe.strategy, OpenStrategy::Paged);
    let source = FileSource::open(&path).expect("Paged Source file");
    let (host, visual) =
        cx.add_window_view(move |_window, cx| DocumentHost::new(path, probe, source, cx));
    visual.run_until_parked();

    visual.update(|window, cx| {
        host.update(cx, |host, _cx| {
            host.select_source_range_and_focus_for_test(selected.clone(), false, window);
        });
    });
    visual.simulate_keystrokes("right");
    visual.run_until_parked();
    let before_typing = host
        .read_with(visual, |host, _cx| host.source_selection_for_test())
        .expect("selection after collapse");
    assert_eq!(before_typing.range(), selected.end..selected.end);

    visual.simulate_input("7");
    visual.run_until_parked();
    let mut expected_after_typing = original.to_owned();
    expected_after_typing.insert(
        usize::try_from(selected.end).expect("collapsed selection offset fits this fixture"),
        '7',
    );
    assert_eq!(
        host.read_with(visual, |host, _cx| host.source_text_for_test()),
        expected_after_typing
    );

    visual.simulate_keystrokes("ctrl-z");
    visual.run_until_parked();
    assert_eq!(
        host.read_with(visual, |host, _cx| host.source_text_for_test()),
        original
    );
    assert_eq!(
        host.read_with(visual, |host, _cx| host.source_selection_for_test()),
        Some(before_typing),
        "one undo must restore the caret state that preceded typing"
    );
}

/// 长词 Ctrl+Right 触发异步边界时，紧接着分发的文字应排在导航之后且一次可撤销。
#[gpui::test]
async fn paged_source_oversized_word_queues_immediate_typing_after_control_right(
    cx: &mut gpui::TestAppContext,
) {
    init_source_navigation_test_app(cx);
    let temp = tempfile::tempdir().expect("immediate typing tempdir");
    let path = temp.path().join("immediate-typing.txt");
    let word_len = 70 * 1024;
    let word = "x".repeat(word_len);
    let original = format!("{word} next\n");
    fs::write(&path, &original).expect("immediate typing fixture");
    let probe = gmark_paged_document::probe_file(
        &path,
        gmark_paged_document::ProbeOptions {
            max_resident_bytes: 1,
            ..gmark_paged_document::ProbeOptions::default()
        },
    )
    .expect("Paged Source probe");
    assert_eq!(probe.strategy, OpenStrategy::Paged);
    let source = FileSource::open(&path).expect("Paged Source file");
    let (host, visual) =
        cx.add_window_view(move |_window, cx| DocumentHost::new(path, probe, source, cx));
    visual.run_until_parked();
    visual.update(|window, cx| {
        host.update(cx, |host, _cx| {
            host.select_source_range_and_focus_for_test(0..0, false, window);
        });
    });

    let handle = visual.update(|window, _cx| window.window_handle());
    visual.cx.dispatch_keystroke(
        handle,
        gpui::Keystroke::parse("ctrl-right").expect("Ctrl+Right"),
    );
    visual
        .cx
        .dispatch_keystroke(handle, gpui::Keystroke::parse("7").expect("digit input"));
    visual.run_until_parked();

    let expected = format!("{word} 7next\n");
    assert_eq!(
        host.read_with(visual, |host, _cx| host.source_text_for_test()),
        expected,
        "queued typing must use the resolved word boundary"
    );
    visual.simulate_keystrokes("ctrl-z");
    visual.run_until_parked();
    assert_eq!(
        host.read_with(visual, |host, _cx| host.source_text_for_test()),
        original,
        "one undo must remove the text entered after navigation"
    );
}

/// 被截断的长行边界必须异步解析到真实 CRLF 行尾/行首，并按序接收紧随的输入。
#[gpui::test]
async fn paged_source_home_end_cross_truncated_line_window_and_undo(cx: &mut gpui::TestAppContext) {
    init_source_navigation_test_app(cx);
    let temp = tempfile::tempdir().expect("long line boundary tempdir");
    let path = temp.path().join("long-line-boundary.txt");
    let original = format!("BEGIN {}中文👨‍👩‍👧‍👦尾 END\r\n", "x ".repeat(40_000));
    let content_end = original.len() - "\r\n".len();
    let content_end_offset = u64::try_from(content_end).expect("long line offset fits u64");
    fs::write(&path, &original).expect("long line boundary fixture");
    let probe = gmark_paged_document::probe_file(
        &path,
        gmark_paged_document::ProbeOptions {
            max_resident_bytes: 1,
            ..gmark_paged_document::ProbeOptions::default()
        },
    )
    .expect("Paged Source probe");
    assert_eq!(probe.strategy, OpenStrategy::Paged);
    let source = FileSource::open(&path).expect("Paged Source file");
    let (host, visual) =
        cx.add_window_view(move |_window, cx| DocumentHost::new(path, probe, source, cx));
    visual.run_until_parked();
    visual.update(|window, cx| {
        host.update(cx, |host, _cx| {
            host.select_source_range_and_focus_for_test(0..0, false, window);
        });
    });

    let handle = visual.update(|window, _cx| window.window_handle());
    visual
        .cx
        .dispatch_keystroke(handle, gpui::Keystroke::parse("end").expect("End"));
    visual
        .cx
        .dispatch_keystroke(handle, gpui::Keystroke::parse("7").expect("digit input"));
    visual.run_until_parked();

    let expected_after_end = format!("{}7\r\n", &original[..content_end]);
    assert_eq!(
        host.read_with(visual, |host, _cx| host.source_text_for_test()),
        expected_after_end,
        "typing after End must be inserted before the original CRLF"
    );
    visual.simulate_keystrokes("ctrl-z");
    visual.run_until_parked();
    assert_eq!(
        host.read_with(visual, |host, _cx| host.source_text_for_test()),
        original
    );
    assert_eq!(
        host.read_with(visual, |host, _cx| host.source_selection_for_test()),
        Some(SourceSelection::collapsed(
            content_end_offset,
            SourceAffinity::After,
        )),
        "undo must restore the resolved End caret"
    );

    visual
        .cx
        .dispatch_keystroke(handle, gpui::Keystroke::parse("home").expect("Home"));
    visual
        .cx
        .dispatch_keystroke(handle, gpui::Keystroke::parse("8").expect("digit input"));
    visual.run_until_parked();
    assert_eq!(
        host.read_with(visual, |host, _cx| host.source_text_for_test()),
        format!("8{original}"),
        "typing after Home must use the true logical line start"
    );
    visual.simulate_keystrokes("ctrl-z");
    visual.run_until_parked();
    assert_eq!(
        host.read_with(visual, |host, _cx| host.source_text_for_test()),
        original
    );
    assert_eq!(
        host.read_with(visual, |host, _cx| host.source_selection_for_test()),
        Some(SourceSelection::collapsed(0, SourceAffinity::After)),
        "undo must restore the resolved Home caret"
    );
}
