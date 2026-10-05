// @author kongweiguang

use super::*;

/// Keep large-source failures actionable without printing the whole document.
fn assert_source_bytes_eq(actual: &[u8], expected: &[u8], context: &str) {
    if actual == expected {
        return;
    }

    let first_difference = actual
        .iter()
        .zip(expected)
        .position(|(actual, expected)| actual != expected)
        .unwrap_or(actual.len().min(expected.len()));
    let sample_start = first_difference.saturating_sub(8);
    let sample_end = first_difference.saturating_add(16);
    let actual_end = sample_end.min(actual.len());
    let expected_end = sample_end.min(expected.len());
    panic!(
        "{context}: first difference at byte {first_difference}; expected {:?}, got {:?} ({} vs {} bytes)",
        &expected[sample_start.min(expected.len())..expected_end],
        &actual[sample_start.min(actual.len())..actual_end],
        expected.len(),
        actual.len(),
    );
}

/// Exercise fallback projection with enough roots while keeping every untouched mixed-ending region checkable.
#[gpui::test]
async fn live_input_save_preserves_unedited_mixed_ending_roots(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let path = temp_markdown_path("live-input-mixed-ending-preservation");

    let mut roots = (0..1801)
        .map(|index| format!("root-{index:04} alpha_beta"))
        .collect::<Vec<_>>();
    roots[900].push_str("\nsoft-break alpha_beta");
    let original = roots.join("\r\n\r\n");

    assert_eq!(original.matches("\r\n").count(), 3600);
    assert_eq!(
        original.matches('\n').count() - original.matches("\r\n").count(),
        1
    );
    std::fs::write(&path, original.as_bytes()).unwrap();

    let expected_replacement = "7".repeat(32);

    let editor_path = path.clone();
    let editor_source = original.clone();
    let (editor, visual) = cx.add_window_view(move |_window, cx| {
        Editor::from_markdown(cx, editor_source, Some(editor_path))
    });
    redraw(visual);
    editor.update_in(visual, |editor, window, cx| {
        let block = editor
            .document
            .first_root()
            .expect("first paragraph")
            .clone();
        let start = block
            .read(cx)
            .display_text()
            .find("alpha_beta")
            .expect("first root exposes the target text");
        editor.focus_block(block.entity_id());
        block.update(cx, |block, cx| {
            block.selected_range = start..start + "alpha_beta".len();
            block.selection_reversed = false;
            block.focus_handle.focus(window);
            cx.notify();
        });
    });
    redraw(visual);
    visual.simulate_input(&expected_replacement);
    redraw(visual);

    let saved = visual.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.save_to_existing_path(&path, window, cx)
        })
    });
    assert!(saved);
    let actual = std::fs::read(&path).unwrap();
    assert!(
        actual
            .windows(expected_replacement.len())
            .any(|window| window == expected_replacement.as_bytes())
    );
    let expected_suffix = &original.as_bytes()[roots[0].len()..];
    let suffix_marker = b"root-0001 alpha_beta";
    let expected_marker_offset = expected_suffix
        .windows(suffix_marker.len())
        .position(|window| window == suffix_marker)
        .expect("first unedited root");
    let actual_marker_offset = actual
        .windows(suffix_marker.len())
        .position(|window| window == suffix_marker)
        .expect("saved first unedited root");
    let actual_suffix_start = actual_marker_offset.saturating_sub(expected_marker_offset);
    assert_source_bytes_eq(
        &actual[actual_suffix_start..],
        expected_suffix,
        "saved source changed after the edited root",
    );
    let _ = std::fs::remove_file(path);
}

/// Rebase source-region anchors synchronously so a second root stays editable before debounce refresh.
#[gpui::test]
async fn live_character_input_rebases_multiple_roots_before_projection_refresh(
    cx: &mut TestAppContext,
) {
    init_editor_test_app(cx);
    let path = temp_markdown_path("live-character-input-region-rebase");
    let original =
        "before __untouched__\r\n\r\nalpha_beta\r\n\r\nmiddle ~~untouched~~\n\nomega_gamma";
    std::fs::write(&path, original.as_bytes()).unwrap();

    let editor_path = path.clone();
    let editor_source = original.to_owned();
    let (editor, visual) = cx.add_window_view(move |_window, cx| {
        Editor::from_markdown(cx, editor_source, Some(editor_path))
    });
    redraw(visual);

    // Redraw each input but do not advance the fake clock, so projection debounce stays pending.
    for (target, replacement) in [("alpha_beta", "first"), ("omega_gamma", "second")] {
        editor.update_in(visual, |editor, window, cx| {
            let block = editor
                .document
                .root_blocks()
                .iter()
                .find(|block| block.read(cx).display_text().contains(target))
                .expect("target root")
                .clone();
            let start = block
                .read(cx)
                .display_text()
                .find(target)
                .expect("target text");
            editor.focus_block(block.entity_id());
            block.update(cx, |block, cx| {
                block.selected_range = start..start + target.len();
                block.selection_reversed = false;
                block.focus_handle.focus(window);
                cx.notify();
            });
        });
        redraw(visual);

        for character in replacement.chars() {
            visual.simulate_input(&character.to_string());
            redraw(visual);
        }
    }

    let saved = visual.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.save_to_existing_path(&path, window, cx)
        })
    });
    assert!(saved);
    let actual = std::fs::read(&path).unwrap();
    let first_start = original.find("alpha_beta").expect("first edited region");
    assert_source_bytes_eq(
        &actual[..actual.len().min(first_start)],
        &original.as_bytes()[..first_start],
        "prefix region changed during consecutive Live edits",
    );
    let middle_start = first_start + "alpha_beta".len();
    let middle_end = original.find("omega_gamma").expect("second edited region");
    let expected_middle = &original.as_bytes()[middle_start..middle_end];
    let middle_marker = b"middle ~~untouched~~";
    let expected_marker_offset = expected_middle
        .windows(middle_marker.len())
        .position(|window| window == middle_marker)
        .expect("middle region marker");
    let actual_marker_offset = actual
        .windows(middle_marker.len())
        .position(|window| window == middle_marker)
        .expect("saved middle region marker");
    let actual_middle_start = actual_marker_offset.saturating_sub(expected_marker_offset);
    let actual_middle_end = actual_middle_start
        .saturating_add(expected_middle.len())
        .min(actual.len());
    assert_source_bytes_eq(
        &actual[actual_middle_start.min(actual.len())..actual_middle_end],
        expected_middle,
        "middle region changed during consecutive Live edits",
    );
    let saved_text = String::from_utf8(actual).expect("saved Markdown stays UTF-8");
    assert!(saved_text.contains("first"));
    assert!(saved_text.contains("second"));
    let _ = std::fs::remove_file(path);
}

/// A malformed table candidate materializes several roots in one region; editing one root must leave surrounding regions byte-identical.
#[gpui::test]
async fn live_multiroot_region_save_and_undo_preserve_neighbor_source(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let path = temp_markdown_path("live-multiroot-region-undo");
    let original = "before __untouched__\r\n\r\n| first |\r\n| second |\r\n\r\nafter ~~untouched~~";
    std::fs::write(&path, original.as_bytes()).unwrap();

    let editor_path = path.clone();
    let editor_source = original.to_owned();
    let (editor, visual) = cx.add_window_view(move |_window, cx| {
        Editor::from_markdown(cx, editor_source, Some(editor_path))
    });
    redraw(visual);
    editor.update_in(visual, |editor, window, cx| {
        let roots = editor.document.root_blocks();
        let first = roots
            .iter()
            .find(|block| block.read(cx).display_text().contains("first"))
            .expect("first fallback row root");
        let block = roots
            .iter()
            .find(|block| block.read(cx).display_text().contains("second"))
            .expect("second fallback row root")
            .clone();
        assert_ne!(first.entity_id(), block.entity_id());
        let start = block
            .read(cx)
            .display_text()
            .find("second")
            .expect("target text");
        editor.focus_block(block.entity_id());
        block.update(cx, |block, cx| {
            block.selected_range = start..start + "second".len();
            block.selection_reversed = false;
            block.focus_handle.focus(window);
            cx.notify();
        });
    });
    redraw(visual);
    for character in "changed".chars() {
        visual.simulate_input(&character.to_string());
        redraw(visual);
    }

    let saved = visual.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.save_to_existing_path(&path, window, cx)
        })
    });
    assert!(saved);
    let actual = std::fs::read(&path).unwrap();
    let prefix_end = original.find("| first |").expect("complex region start");
    let suffix_marker_start = original
        .find("after ~~untouched~~")
        .expect("following region");
    assert_source_bytes_eq(
        &actual[..actual.len().min(prefix_end)],
        &original.as_bytes()[..prefix_end],
        "prefix region changed during multi-root edit",
    );
    let expected_suffix_start = suffix_marker_start.saturating_sub("\r\n\r\n".len());
    let expected_suffix = &original.as_bytes()[expected_suffix_start..];
    let marker_offset = suffix_marker_start - expected_suffix_start;
    let actual_marker_start = actual
        .windows(b"after ~~untouched~~".len())
        .position(|window| window == b"after ~~untouched~~")
        .expect("saved following region");
    let actual_suffix_start = actual_marker_start.saturating_sub(marker_offset);
    assert_source_bytes_eq(
        &actual[actual_suffix_start..],
        expected_suffix,
        "suffix region changed during multi-root edit",
    );

    visual.simulate_keystrokes("ctrl-z");
    redraw(visual);
    editor.read_with(visual, |editor, _cx| {
        assert_eq!(
            editor.source_document.text(),
            original.replace("\r\n", "\n")
        );
    });
    visual.simulate_keystrokes("ctrl-shift-z");
    redraw(visual);
    editor.read_with(visual, |editor, _cx| {
        assert!(editor.source_document.text().contains("changed"));
    });
    let _ = std::fs::remove_file(path);
}
