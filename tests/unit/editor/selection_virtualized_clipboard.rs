// @author kongweiguang

use gpui::{AppContext, TestAppContext, VisualTestContext, point, px};

use super::{CrossBlockDrag, CrossBlockSelectionEndpoint, Editor, SelectionSurface};
use crate::i18n::I18nManager;
use crate::theme::ThemeManager;

/// 多窗格使用真实 Ctrl+A/C 快捷键，确保跨块选区由所属子编辑器复制，而非窗口壳或本地块。
#[gpui::test]
async fn pane_full_selection_shortcuts_copy_visible_text_and_protect_preview(
    cx: &mut TestAppContext,
) {
    init_editor_test_app(cx);
    for mode in [
        super::ViewMode::Rendered,
        super::ViewMode::Preview,
        super::ViewMode::Split,
    ] {
        let original = "alpha **bold**\n\nbeta";
        let (root, visual) =
            cx.add_window_view(|_window, cx| Editor::from_markdown(cx, original.to_owned(), None));
        root.update(visual, |editor, cx| {
            editor.split_pane_toward(crate::editor::panes::PaneSplitDirection::Right, cx);
        });
        redraw(visual);
        let pane = root.read_with(visual, |editor, cx| {
            let id = editor
                .pane_workspace
                .as_ref()
                .expect("workspace")
                .read(cx)
                .workspace()
                .focused_pane();
            let canvases = editor.pane_canvas_entities.borrow();
            match &canvases.get(&id).expect("active canvas").2 {
                crate::editor::panes::PaneCanvasEntity::Markdown(canvas) => {
                    canvas.read(cx).editor()
                }
                _ => panic!("Markdown pane expected"),
            }
        });
        root.update(visual, |editor, cx| editor.set_view_mode(mode, cx));
        redraw(visual);
        pane.update_in(visual, |editor, window, cx| {
            assert_eq!(editor.view_mode, mode, "use the window's mode switch path");
            let surface = if mode == super::ViewMode::Split {
                SelectionSurface::SplitPreview
            } else {
                SelectionSurface::Main
            };
            let block = editor
                .selection_surface_entities(surface)
                .first()
                .expect("first text block")
                .clone();
            editor.active_selection_surface = surface;
            editor.active_entity_id = Some(block.entity_id());
            block.read(cx).focus_handle.focus(window);
        });
        redraw(visual);
        visual.simulate_keystrokes("ctrl-a");
        redraw(visual);
        let revision = pane.read_with(visual, |editor, _| editor.source_document.revision());
        let _ = Editor::take_rich_clipboard_write_for_test();
        visual.simulate_keystrokes("ctrl-c");
        redraw(visual);
        assert_eq!(
            visual
                .read_from_clipboard()
                .and_then(|item| item.text())
                .as_deref(),
            Some("alpha bold\n\nbeta"),
            "mode {mode:?}"
        );
        let (html, text) = Editor::take_rich_clipboard_write_for_test().expect("rich copy");
        assert_eq!(text, "alpha bold\n\nbeta");
        assert!(html.contains("<strong>bold</strong>"));
        if mode != super::ViewMode::Rendered {
            for keys in [
                "ctrl-x",
                "backspace",
                "delete",
                "ctrl-v",
                "ctrl-z",
                "ctrl-shift-z",
                "ctrl-y",
                "tab",
            ] {
                visual.simulate_keystrokes(keys);
                redraw(visual);
                pane.read_with(visual, |editor, _| {
                    assert_eq!(editor.source_document.text(), original, "{mode:?} {keys}");
                    assert_eq!(editor.source_document.revision(), revision);
                    assert!(!editor.source_document.is_dirty());
                    assert!(editor.undo_history.is_empty());
                });
            }
        }
    }
}

/// Initializes the globals used by rendered editor tests without constructing the app shell.
fn init_editor_test_app(cx: &mut TestAppContext) {
    cx.update(|cx| {
        I18nManager::init(cx);
        ThemeManager::init(cx);
        crate::components::init(cx);
    });
}

/// Redraws after scrolling so virtualized mounts reflect the source position under test.
fn redraw(cx: &mut VisualTestContext) {
    cx.update(|window, cx| window.draw(cx).clear());
    cx.run_until_parked();
}

/// 窗口壳是模式的同步源，快捷键不能只切子 Editor 后又被父窗口下一帧覆盖。
#[gpui::test]
async fn pane_toggle_shortcut_survives_parent_redraw(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let (root, visual) = cx
        .add_window_view(|_window, cx| Editor::from_markdown(cx, "alpha\n\nbeta".to_owned(), None));
    root.update(visual, |editor, cx| {
        editor.split_pane_toward(crate::editor::panes::PaneSplitDirection::Right, cx);
    });
    redraw(visual);
    let pane = root.read_with(visual, |editor, cx| {
        editor.focused_pane_entities(cx).0.expect("Markdown pane")
    });
    for mode in [super::ViewMode::Source, super::ViewMode::Rendered] {
        visual.simulate_keystrokes("ctrl-/");
        redraw(visual);
        root.update(visual, |_, cx| cx.notify());
        redraw(visual);
        assert_eq!(root.read_with(visual, |editor, _| editor.view_mode), mode);
        assert_eq!(pane.read_with(visual, |editor, _| editor.view_mode), mode);
        assert_eq!(
            pane.read_with(visual, |editor, _| editor.source_document.text()),
            "alpha\n\nbeta"
        );
    }
}

/// 父窗口排队模式意图，保留原候选目标直到终态；不能为了同步模式先拆掉输入实体。
#[cfg(target_os = "windows")]
#[gpui::test]
async fn pane_toggle_shortcut_waits_for_original_ime_target(cx: &mut TestAppContext) {
    use gpui::{CompositionEnd, EntityInputHandler};
    init_editor_test_app(cx);
    let (root, visual) =
        cx.add_window_view(|_window, cx| Editor::from_markdown(cx, "alpha".to_owned(), None));
    root.update(visual, |editor, cx| {
        editor.split_pane_toward(crate::editor::panes::PaneSplitDirection::Right, cx);
    });
    redraw(visual);
    let pane = root.read_with(visual, |editor, cx| {
        editor.focused_pane_entities(cx).0.expect("Markdown pane")
    });
    let owner = pane.update_in(visual, |editor, window, cx| {
        let owner = editor.document.first_root().expect("text target").clone();
        owner.update(cx, |block, cx| {
            block.focus_handle.focus(window);
            block.selected_range = 0..5;
            block.replace_and_mark_text_in_range(None, "ni", Some(2..2), window, cx);
        });
        owner
    });
    redraw(visual);
    visual.simulate_keystrokes("ctrl-/");
    redraw(visual);
    assert_eq!(
        pane.read_with(visual, |editor, _| editor.view_mode),
        super::ViewMode::Rendered
    );
    assert_eq!(
        pane.read_with(visual, |editor, _| editor.source_document.text()),
        "alpha"
    );
    owner.update_in(visual, |block, window, cx| {
        block.replace_text_in_range(None, "你", window, cx);
        block.composition_ended(CompositionEnd::Committed, window, cx);
    });
    redraw(visual);
    assert_eq!(
        root.read_with(visual, |editor, _| editor.view_mode),
        super::ViewMode::Source
    );
    assert_eq!(
        pane.read_with(visual, |editor, _| editor.view_mode),
        super::ViewMode::Source
    );
    assert_eq!(
        pane.read_with(visual, |editor, _| editor.source_document.text()),
        "你"
    );
}

/// Exercises ordinary Copy after virtualization unmounts the selection anchor and checks both clipboard formats.
#[gpui::test]
async fn virtualized_cross_block_ctrl_c_copies_visible_text_after_anchor_unmount(
    cx: &mut TestAppContext,
) {
    init_editor_test_app(cx);
    let source = (0..9_000)
        .map(|index| match index {
            0 => "intro **bold-start** tail".to_owned(),
            8_999 => "far *italic-end* suffix".to_owned(),
            _ => format!("*paragraph {index:05}*"),
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    let (editor, visual) =
        cx.add_window_view(move |_window, cx| Editor::from_markdown_virtualized(cx, source, None));
    redraw(visual);

    let first_id = editor.update(visual, |editor, cx| {
        let first = editor
            .document
            .first_root()
            .expect("first viewport block")
            .clone();
        let endpoint = CrossBlockSelectionEndpoint {
            entity_id: first.entity_id(),
            offset: 8,
        };
        let source_anchor =
            editor.cross_block_source_anchor_for_endpoint(SelectionSurface::Main, endpoint, cx);
        editor.active_entity_id = Some(first.entity_id());
        editor.pending_focus = Some(first.entity_id());
        editor.cross_block_drag = Some(CrossBlockDrag {
            anchor: endpoint,
            source_anchor,
        });
        let max_y = f32::from(editor.scroll_handle.max_offset().height.max(px(0.0)));
        editor
            .scroll_handle
            .set_offset(point(px(0.0), px(-max_y * 0.75)));
        first.entity_id()
    });
    redraw(visual);

    let (last_id, _, last_text) = editor.update(visual, |editor, cx| {
        let surface = editor.virtual_surface.as_ref().expect("virtual surface");
        assert!(
            surface.entity_by_id(first_id).is_some(),
            "the anchor stays pinned"
        );
        assert!(editor.document.block_entity_by_id(first_id).is_none());

        let focus = editor
            .document
            .visible_blocks()
            .last()
            .expect("far viewport block")
            .entity
            .clone();
        let text = focus.read(cx).display_text().to_owned();
        let end = focus.read(cx).visible_len();
        editor.apply_surface_pointer_selection_to_endpoint(
            SelectionSurface::Main,
            CrossBlockSelectionEndpoint {
                entity_id: focus.entity_id(),
                offset: end,
            },
            cx,
        );
        let selection = editor
            .normalized_cross_block_selection(cx)
            .expect("source-anchored selection");
        assert!(
            selection.start_index.is_none(),
            "the source anchor is unmounted"
        );
        (focus.entity_id(), end, text)
    });
    redraw(visual);

    let _ = Editor::take_rich_clipboard_write_for_test();
    visual.simulate_keystrokes("ctrl-c");
    visual.run_until_parked();

    let copied = visual
        .read_from_clipboard()
        .and_then(|item| item.text())
        .expect("Ctrl+C writes the GPUI clipboard");
    assert!(copied.starts_with("ld-start tail\n\n"));
    assert!(copied.contains("paragraph 03000"));
    assert!(copied.ends_with(&last_text));
    assert!(
        !copied.contains('*'),
        "ordinary copy omits Markdown delimiters"
    );

    let (html, rich_plain_text) = Editor::take_rich_clipboard_write_for_test()
        .expect("Ctrl+C also writes the rich clipboard payload");
    assert_eq!(rich_plain_text, copied);
    assert!(html.contains("<strong>ld-start</strong> tail"));
    assert!(html.contains("<em>paragraph 03000</em>"));
    assert!(last_id != first_id);
}
