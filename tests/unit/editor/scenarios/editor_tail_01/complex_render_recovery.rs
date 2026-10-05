// @author kongweiguang

/// 布局审查解除正文光标跟随并设置明确视口，不改变正文或历史，只观察当前帧的图形。
fn prepare_complex_layout_viewport(
    editor: &mut Editor,
    window: &mut gpui::Window,
    cx: &mut gpui::Context<Editor>,
) {
    window.blur();
    editor.active_entity_id = None;
    editor.pending_focus = None;
    editor.pending_scroll_active_block_into_view = false;
    editor.pending_scroll_recheck_after_layout = false;
    editor.scroll_handle.set_offset(point(px(0.0), px(0.0)));
    cx.notify();
}

/// 缩放后等待当前视口的渲染终态，并实际滚动到告警，避免用旧帧 selector 验证新布局。
#[gpui::test]
async fn complex_render_failure_keeps_last_successful_math_and_mermaid_svg(
    cx: &mut TestAppContext,
) {
    init_editor_test_app(cx);
    let source = "$$\nx^2\n$$\n\n```mermaid\ngraph TD\nA --> B\n```\n\nafter";
    let (editor, visual_cx) =
        cx.add_window_view(move |_window, cx| Editor::from_markdown(cx, source.to_owned(), None));
    editor.update(visual_cx, |editor, _cx| {
        let paragraph = editor.document.visible_blocks()[2].entity.clone();
        editor.focus_block(paragraph.entity_id());
    });
    redraw(visual_cx);
    visual_cx
        .executor()
        .advance_clock(Duration::from_millis(300));
    visual_cx.run_until_parked();
    redraw(visual_cx);
    assert!(visual_cx.debug_bounds("math-rendered-content").is_some());
    assert!(visual_cx.debug_bounds("mermaid-rendered-content").is_some());

    let (math_path, mermaid_path) = editor.read_with(visual_cx, |editor, cx| {
        let visible = editor.document.visible_blocks();
        (
            visible[0]
                .entity
                .read(cx)
                .last_successful_math_render
                .as_ref()
                .expect("math cache")
                .path
                .clone(),
            visible[1]
                .entity
                .read(cx)
                .last_successful_mermaid_render
                .as_ref()
                .expect("mermaid cache")
                .path
                .clone(),
        )
    });

    editor.update(visual_cx, |editor, cx| {
        let math = editor.document.visible_blocks()[0].entity.clone();
        let raw = math.read(cx).display_text().to_owned();
        let start = raw.find("x^2").unwrap();
        math.update(cx, |block, block_cx| {
            block
                .prepare_undo_capture(crate::components::UndoCaptureKind::NonCoalescible, block_cx);
            block.replace_text_in_visible_range(
                start..start + "x^2".len(),
                "\\frac{",
                None,
                false,
                block_cx,
            );
        });
        let paragraph = editor.document.visible_blocks()[2].entity.clone();
        editor.focus_block(paragraph.entity_id());
    });
    redraw(visual_cx);
    visual_cx
        .executor()
        .advance_clock(Duration::from_millis(300));
    visual_cx.run_until_parked();
    redraw(visual_cx);
    assert!(visual_cx.debug_bounds("math-render-fallback").is_some());
    assert!(visual_cx.debug_bounds("math-render-warning").is_some());
    editor.read_with(visual_cx, |editor, cx| {
        assert!(editor.source_document.text().contains("\\frac{"));
        assert_eq!(
            editor.document.visible_blocks()[0]
                .entity
                .read(cx)
                .last_successful_math_render
                .as_ref()
                .unwrap()
                .path,
            math_path
        );
    });

    editor.update(visual_cx, |editor, cx| {
        let mermaid = editor.document.visible_blocks()[1].entity.clone();
        let raw = mermaid.read(cx).display_text().to_owned();
        let start = raw.find("graph TD\nA --> B").unwrap();
        mermaid.update(cx, |block, block_cx| {
            block
                .prepare_undo_capture(crate::components::UndoCaptureKind::NonCoalescible, block_cx);
            block.replace_text_in_visible_range(
                start..start + "graph TD\nA --> B".len(),
                "not a real mermaid diagram ::::",
                None,
                false,
                block_cx,
            );
        });
        let paragraph = editor.document.visible_blocks()[2].entity.clone();
        editor.focus_block(paragraph.entity_id());
    });
    redraw(visual_cx);
    visual_cx
        .executor()
        .advance_clock(Duration::from_millis(300));
    visual_cx.run_until_parked();
    redraw(visual_cx);
    assert!(visual_cx.debug_bounds("mermaid-render-fallback").is_some());
    assert!(visual_cx.debug_bounds("mermaid-render-warning").is_some());
    editor.read_with(visual_cx, |editor, cx| {
        assert!(
            editor
                .source_document
                .text()
                .contains("not a real mermaid diagram ::::")
        );
        assert_eq!(
            editor.document.visible_blocks()[1]
                .entity
                .read(cx)
                .last_successful_mermaid_render
                .as_ref()
                .unwrap()
                .path,
            mermaid_path
        );
    });

    for viewport in [size(px(720.0), px(520.0)), size(px(1180.0), px(780.0))] {
        visual_cx.simulate_resize(viewport);
        editor.update_in(visual_cx, |editor, window, cx| {
            prepare_complex_layout_viewport(editor, window, cx);
        });
        redraw(visual_cx);
        visual_cx.executor().advance_clock(Duration::from_millis(300));
        visual_cx.run_until_parked();
        redraw(visual_cx);
        for (selector, icon_selector) in [
            ("math-render-warning", "math-render-warning-icon"),
            ("mermaid-render-warning", "mermaid-render-warning-icon"),
        ] {
            if selector == "mermaid-render-warning" {
                editor.update(visual_cx, |editor, cx| {
                    let bottom = -editor.scroll_handle.max_offset().height;
                    editor.pending_scroll_active_block_into_view = false;
                    editor.pending_scroll_recheck_after_layout = false;
                    editor.scroll_handle.set_offset(point(px(0.0), bottom));
                    cx.notify();
                });
                redraw(visual_cx);
            }
            let content = visual_cx.debug_bounds("editor-content").unwrap();
            let mermaid_frame = visual_cx.debug_bounds("mermaid-workbench-frame").unwrap();
            let warning = visual_cx.debug_bounds(selector).unwrap();
            let icon = visual_cx.debug_bounds(icon_selector).unwrap();
            assert_eq!(warning.size.height, px(22.0));
            assert_eq!(icon.size, size(px(14.0), px(14.0)));
            let visible_bounds = if selector == "mermaid-render-warning" {
                // Mermaid 告警位于内部滚动内容中，未裁剪布局仍可能保留旧 SVG
                // 的固有宽度；对外可见边界由工作台外框负责。
                mermaid_frame
            } else {
                warning
            };
            assert!(visible_bounds.left() >= content.left());
            assert!(
                visible_bounds.right() <= content.right(),
                "{selector}: visible={visible_bounds:?}, warning={warning:?}, content={content:?}"
            );
            assert!(icon.left() >= warning.left());
            assert!(icon.right() <= warning.right());
            assert!(icon.top() >= warning.top());
            assert!(icon.bottom() <= warning.bottom());
        }
    }

    editor.update(visual_cx, |editor, cx| {
        editor.undo_document(cx);
        let paragraph = editor.document.visible_blocks()[2].entity.clone();
        editor.focus_block(paragraph.entity_id());
    });
    redraw(visual_cx);
    visual_cx
        .executor()
        .advance_clock(Duration::from_millis(300));
    visual_cx.run_until_parked();
    redraw(visual_cx);
    editor.read_with(visual_cx, |editor, cx| {
        assert!(
            !editor
                .source_document
                .text()
                .contains("not a real mermaid diagram ::::")
        );
        let visible = editor.document.visible_blocks();
        assert!(visible[0].entity.read(cx).math_render_error.is_some());
        assert!(visible[1].entity.read(cx).mermaid_render_error.is_none());
    });
    editor.update_in(visual_cx, |editor, window, cx| {
        prepare_complex_layout_viewport(editor, window, cx);
    });
    redraw(visual_cx);
    // 撤销从权威源码重建实体，不继承旧实体的成功图片；当前失败必须仍显示告警。
    // 前面的原实体断言负责验证编辑失败保留图片，不能用其旧帧 selector 冒充撤销状态。
    assert!(visual_cx.debug_bounds("math-render-warning").is_some());

    editor.update(visual_cx, |editor, cx| {
        editor.undo_document(cx);
        let paragraph = editor.document.visible_blocks()[2].entity.clone();
        editor.focus_block(paragraph.entity_id());
    });
    redraw(visual_cx);
    editor.read_with(visual_cx, |editor, cx| {
        assert_eq!(editor.source_document.text(), source);
        let visible = editor.document.visible_blocks();
        assert!(visible[0].entity.read(cx).math_render_error.is_none());
        assert!(visible[1].entity.read(cx).mermaid_render_error.is_none());
    });
}
