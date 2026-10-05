// @author kongweiguang

use super::*;
use std::ops::Range;
use std::path::PathBuf;

/// 注册窗口级 GPUI 全局状态，让这些 Source 边界用例走真实焦点与布局路径。
fn init_source_pointer_test_app(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        crate::i18n::I18nManager::init(cx);
        crate::theme::ThemeManager::init(cx);
        crate::components::init(cx);
    });
}

/// 在滚动和输入事件后重绘 Source 视图，避免依赖其他测试模块的私有辅助函数。
fn redraw(cx: &mut gpui::VisualTestContext) {
    cx.update(|window, cx| window.draw(cx).clear());
    cx.run_until_parked();
}

/// 用最小驻留阈值打开真实 Paged 文档，避免大行回归意外走常驻文本实现。
fn open_paged_source_host(
    cx: &mut gpui::TestAppContext,
    path: PathBuf,
) -> (gpui::Entity<DocumentHost>, &mut gpui::VisualTestContext) {
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
    let (host, visual) =
        cx.add_window_view(move |_window, cx| DocumentHost::new(path, probe, source, cx));
    visual.run_until_parked();
    redraw(visual);
    (host, visual)
}

/// 通过 Block 的实际布局命中寻找目标字节，避免把字体 advance 假设写进交互测试。
fn pointer_over_local_range(
    visual: &gpui::VisualTestContext,
    block: &gpui::Entity<crate::components::Block>,
    row_bounds: gpui::Bounds<gpui::Pixels>,
    target: Range<usize>,
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

/// 只通过可见滚动条移动到行尾窗口，确保边界由用户交互产生而不是测试注入状态。
fn scroll_to_source_window_end(visual: &mut gpui::VisualTestContext) {
    let scrollbar = visual
        .debug_bounds("document-host-horizontal-scrollbar")
        .expect("long Source row exposes its horizontal scrollbar");
    let end = point(scrollbar.right() - px(1.0), scrollbar.center().y);
    visual.simulate_mouse_down(end, MouseButton::Left, gpui::Modifiers::default());
    visual.simulate_mouse_up(end, MouseButton::Left, gpui::Modifiers::default());
    visual.run_until_parked();
    redraw(visual);
}

/// 首屏窗口截断一个连续单词时，双击仍应选到文档中的实际词尾。
#[gpui::test]
async fn paged_source_double_click_selects_word_beyond_64k_window(cx: &mut gpui::TestAppContext) {
    init_source_pointer_test_app(cx);
    let temp = tempfile::tempdir().expect("long word tempdir");
    let path = temp.path().join("long-word.txt");
    let word = "w".repeat(MAX_RENDERED_LINE_BYTES as usize + 512);
    fs::write(&path, &word).expect("long word fixture");
    let (host, visual) = open_paged_source_host(cx, path);
    let row = visual
        .debug_bounds("document-host-line-body-0")
        .expect("first Source row");
    let block = host
        .read_with(visual, |host, _cx| host.source_row_block_for_test(0))
        .expect("first Source row input");
    let position = pointer_over_local_range(visual, &block, row, 2..6);

    visual.update(|window, cx| {
        host.update(cx, |host, cx| {
            host.activate_source_pointer_for_test(0, position, 2, window, cx);
        });
    });
    redraw(visual);

    let selected = host
        .read_with(visual, |host, _cx| host.source_selection_for_test())
        .expect("Source word selection");
    assert_eq!(
        selected.range(),
        0..word.len() as u64,
        "double-click must continue past the bounded row window to the word's true end"
    );
}

/// 家庭 emoji 跨入末页窗口左边界时，双击可见成员仍应选中完整字素。
#[gpui::test]
async fn paged_source_double_click_keeps_family_grapheme_across_window_start(
    cx: &mut gpui::TestAppContext,
) {
    init_source_pointer_test_app(cx);
    let temp = tempfile::tempdir().expect("family boundary tempdir");
    let path = temp.path().join("family-window-boundary.txt");
    let prefix = "anchor ";
    let family = "👨‍👩‍👧‍👦";
    let first_member_len = family.chars().next().expect("family member").len_utf8();
    let total_len = MAX_RENDERED_LINE_BYTES as usize + prefix.len() + first_member_len;
    let suffix = ".".repeat(total_len - prefix.len() - family.len());
    let text = format!("{prefix}{family}{suffix}");
    fs::write(&path, &text).expect("family boundary fixture");
    let (host, visual) = open_paged_source_host(cx, path);
    scroll_to_source_window_end(visual);

    let row = visual
        .debug_bounds("document-host-line-body-0")
        .expect("last Source window row");
    let block = host
        .read_with(visual, |host, _cx| host.source_row_block_for_test(0))
        .expect("last Source window input");
    let woman_start_in_window = "\u{200d}".len();
    let position = pointer_over_local_range(
        visual,
        &block,
        row,
        woman_start_in_window..woman_start_in_window + "👩".len(),
    );

    visual.update(|window, cx| {
        host.update(cx, |host, cx| {
            host.activate_source_pointer_for_test(0, position, 2, window, cx);
        });
    });
    redraw(visual);

    let selected = host
        .read_with(visual, |host, _cx| host.source_selection_for_test())
        .expect("family selection");
    assert_eq!(
        selected.range(),
        prefix.len() as u64..(prefix.len() + family.len()) as u64,
        "the bounded window must not split the selected family grapheme"
    );
}

/// 从后续单词拖向跨窗口家庭 emoji 时，词粒度扩选必须回到该字素的真实源码起点。
#[gpui::test]
async fn paged_source_word_drag_keeps_family_grapheme_before_window_start(
    cx: &mut gpui::TestAppContext,
) {
    init_source_pointer_test_app(cx);
    let temp = tempfile::tempdir().expect("family word drag tempdir");
    let path = temp.path().join("family-word-drag.txt");
    let prefix = "anchor ";
    let family = "👨‍👩‍👧‍👦";
    let first_member_len = family.chars().next().expect("family member").len_utf8();
    let word = "target";
    let total_len = MAX_RENDERED_LINE_BYTES as usize + prefix.len() + first_member_len;
    let suffix_len = total_len - prefix.len() - family.len() - 1 - word.len();
    let text = format!("{prefix}{family} {word}{}", ".".repeat(suffix_len));
    fs::write(&path, &text).expect("family word drag fixture");
    let (host, visual) = open_paged_source_host(cx, path);
    scroll_to_source_window_end(visual);

    let row = visual
        .debug_bounds("document-host-line-body-0")
        .expect("last Source window row");
    let block = host
        .read_with(visual, |host, _cx| host.source_row_block_for_test(0))
        .expect("last Source window input");
    let family_tail_len = family.len() - first_member_len;
    let word_start_in_window = family_tail_len + 1;
    let word_start = (prefix.len() + family.len() + 1) as u64;
    let word_end = word_start + word.len() as u64;
    let anchor = pointer_over_local_range(
        visual,
        &block,
        row,
        word_start_in_window..word_start_in_window + word.len(),
    );

    visual.update(|window, cx| {
        host.update(cx, |host, cx| {
            host.activate_source_pointer_for_test(0, anchor, 2, window, cx);
        });
    });
    redraw(visual);
    assert_eq!(
        host.read_with(visual, |host, _cx| host.source_selection_for_test())
            .expect("initial target word selection")
            .range(),
        word_start..word_end,
        "the drag must begin with a valid word selection after the grapheme"
    );

    let woman_start_in_window = 3;
    let family_hit = pointer_over_local_range(
        visual,
        &block,
        row,
        woman_start_in_window..woman_start_in_window + "👩".len(),
    );
    visual.simulate_mouse_move(family_hit, MouseButton::Left, gpui::Modifiers::default());
    visual.run_until_parked();
    redraw(visual);

    let selected = host
        .read_with(visual, |host, _cx| host.source_selection_for_test())
        .expect("word drag selection");
    assert_eq!(
        selected.range(),
        prefix.len() as u64..word_end,
        "word dragging must include the grapheme prefix omitted from the row window"
    );
    visual.simulate_mouse_up(family_hit, MouseButton::Left, gpui::Modifiers::default());
}
