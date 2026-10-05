// @author kongweiguang

use super::*;
use gmark_paged_document::FileSource;

/// 从真实渲染行分发完整点击，再输入和撤销；不直接安装选区，覆盖首次焦点接线。
#[gpui::test]
async fn paged_source_first_native_click_positions_caret_and_accepts_text(
    cx: &mut gpui::TestAppContext,
) {
    cx.update(|cx| {
        crate::i18n::I18nManager::init(cx);
        crate::theme::ThemeManager::init(cx);
        crate::components::init(cx);
    });
    let temp = tempfile::tempdir().expect("native source click tempdir");
    let path = temp.path().join("native-click.txt");
    let original = format!(
        "BEGIN {}中文👨‍👩‍👧‍👦尾 END\r\nsecond alpha_beta row\r\n",
        "x ".repeat(40_000)
    );
    fs::write(&path, &original).expect("native source click fixture");
    let probe = gmark_paged_document::probe_file(
        &path,
        gmark_paged_document::ProbeOptions {
            max_resident_bytes: 1,
            ..gmark_paged_document::ProbeOptions::default()
        },
    )
    .expect("native source click probe");
    let source = FileSource::open(&path).expect("native source click file");
    let (editor, visual) = cx.add_window_view(move |_window, cx| {
        crate::editor::Editor::from_source_backed_file(cx, path, probe, source)
    });
    visual.run_until_parked();
    let host = editor
        .read_with(visual, |editor, _cx| editor.document_host.clone())
        .expect("native Source Host in root Editor");
    visual.update(|window, cx| window.draw(cx).clear());
    visual.run_until_parked();

    for line in [0, 1] {
        let selector = if line == 0 {
            "document-host-line-body-0"
        } else {
            "document-host-line-body-1"
        };
        let row = visual
            .debug_bounds(selector)
            .expect("visible native Source row");
        let block = host
            .read_with(visual, |host, _cx| host.source_row_block_for_test(line))
            .expect("visible native Source block");
        let position = (0..400)
            .map(|x| point(row.left() + px(x as f32), row.center().y))
            .find(|position| {
                (8..10).contains(&block.read_with(visual, |block, _cx| {
                    block.index_for_mouse_position(*position)
                }))
            })
            .expect("visible target byte");
        let local = block.read_with(visual, |block, _cx| {
            block.index_for_mouse_position(position)
        });
        let line_start = if line == 0 {
            0
        } else {
            original.find("second").expect("second row") as u64
        };
        visual.simulate_click(position, gpui::Modifiers::default());
        visual.run_until_parked();
        let selection = host
            .read_with(visual, |host, _cx| host.source_selection_for_test())
            .expect("native click selection");
        assert_eq!(
            selection.range(),
            line_start + local as u64..line_start + local as u64,
            "first click must install its byte hit rather than leave the caret at line start"
        );
        visual.simulate_input("7");
        visual.run_until_parked();
        let mut expected = original.clone();
        expected.insert(selection.head.byte_offset as usize, '7');
        let actual = host.read_with(visual, |host, _cx| host.source_text_for_test());
        assert!(
            actual == expected,
            "native click input mismatch: actual bytes {}, expected {}, first difference {:?}",
            actual.len(),
            expected.len(),
            actual
                .bytes()
                .zip(expected.bytes())
                .position(|(a, b)| a != b)
        );
        visual.simulate_keystrokes("ctrl-z");
        visual.run_until_parked();
        assert_eq!(
            host.read_with(visual, |host, _cx| host.source_text_for_test()),
            original
        );
        assert_eq!(
            host.read_with(visual, |host, _cx| host.source_selection_for_test()),
            Some(selection)
        );
        if line == 0 {
            let content_end = original.find("\r\n").expect("first line ending");
            for (key, target) in [
                ("end", content_end),
                ("ctrl-home", 0),
                ("end", content_end),
                ("home", 0),
            ] {
                visual.simulate_keystrokes(key);
                visual.run_until_parked();
                let caret = host
                    .read_with(visual, |host, _cx| host.source_selection_for_test())
                    .expect("real click then navigation selection");
                assert_eq!(caret.range(), target as u64..target as u64);
                visual.update(|window, cx| window.draw(cx).clear());
                let active = host
                    .read_with(visual, |host, _cx| host.source_row_block_for_test(0))
                    .expect("navigation input row");
                active.read_with(visual, |block, _cx| {
                    let identity = block
                        .source_layout_identity
                        .as_ref()
                        .expect("current row identity");
                    assert_eq!(
                        identity.source_range.start + block.cursor_offset() as u64,
                        target as u64,
                        "painted caret and shared navigation must describe the same byte"
                    );
                    if key == "end" {
                        assert!(
                            block.display_text().ends_with("尾 END"),
                            "End must paint the actual logical line tail"
                        );
                    }
                });
                let caret_x = active.read_with(visual, |block, _cx| {
                    let lines = block.last_layout.as_ref().expect("painted input layout");
                    let bounds = block.last_bounds.expect("painted input bounds");
                    bounds.left()
                        + lines[0]
                            .position_for_index(block.cursor_offset(), block.last_line_height)
                            .expect("measured Source caret")
                            .x
                });
                let visible_right =
                    visual.update(|window, _cx| window.viewport_size().width.min(row.right()));
                assert!(
                    caret_x >= row.left() && caret_x <= visible_right,
                    "{key} caret must remain visible inside its actual row viewport"
                );
                visual.simulate_input("8");
                visual.run_until_parked();
                let mut expected = original.clone();
                expected.insert(target, '8');
                let actual = host.read_with(visual, |host, _cx| host.source_text_for_test());
                assert!(
                    actual == expected,
                    "real click then {key} must retain full Source text"
                );
                visual.simulate_keystrokes("ctrl-z");
                visual.run_until_parked();
                assert!(
                    host.read_with(visual, |host, _cx| host.source_text_for_test()) == original
                );
            }
        }
    }
}
