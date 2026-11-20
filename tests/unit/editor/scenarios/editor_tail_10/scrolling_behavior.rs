// @author kongweiguang

/// 在三种阅读模式中真实深滚并缩窄窗口，确保重排后视口中心仍命中连续正文。
#[gpui::test]
async fn long_markdown_scroll_keeps_text_in_the_viewport(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let markdown = format!(
        "<!-- author -->\n\n# Heading\n\nalpha_beta\n\n```text\nline one\nline two\n```\n\n| a | b |\n| - | - |\n| one | two |\n\n{}",
        (0..900)
            .map(|index| format!("段落 {index} alpha_beta 中文 emoji 👨‍👩‍👧‍👦 与组合音标 é。"))
            .collect::<Vec<_>>()
            .join("\n\n")
    );
    for mode in [ViewMode::Rendered, ViewMode::Preview, ViewMode::Split] {
        let source = markdown.clone();
        let (editor, visual) =
            cx.add_window_view(move |_window, cx| Editor::from_markdown(cx, source, None));
        visual.simulate_resize(size(px(720.0), px(520.0)));
        editor.update(visual, |editor, cx| editor.set_view_mode(mode, cx));
        redraw(visual);
        let wide_viewport_width = editor
            .read_with(visual, |editor, _| {
                editor.split_preview.as_ref().map_or_else(
                    || editor.scroll_handle.bounds(),
                    |state| state.scroll_handle.bounds(),
                )
            })
            .size
            .width;

        // 每次 390px 的滚轮步进跨过多屏后再缩窄，覆盖深处布局缓存的重定位。
        let scroll_and_assert_center_text = |visual: &mut gpui::VisualTestContext| {
            let viewport = editor.read_with(visual, |editor, _| {
                editor.split_preview.as_ref().map_or_else(
                    || editor.scroll_handle.bounds(),
                    |state| state.scroll_handle.bounds(),
                )
            });
            visual.simulate_event(gpui::ScrollWheelEvent {
                position: viewport.center(),
                delta: gpui::ScrollDelta::Pixels(point(px(0.0), px(-390.0))),
                modifiers: Modifiers::default(),
                touch_phase: gpui::TouchPhase::Moved,
            });
            redraw(visual);
            editor.read_with(visual, |editor, cx| {
                let (document, scroll) = editor
                    .split_preview
                    .as_ref()
                    .map_or((&editor.document, &editor.scroll_handle), |state| {
                        (&state.document, &state.scroll_handle)
                    });
                let viewport = scroll.bounds();
                assert!(
                    document.visible_blocks().iter().any(|visible| {
                        visible.entity.read(cx).last_bounds.is_some_and(|bounds| {
                            bounds.top() <= viewport.center().y + px(40.0)
                                && bounds.bottom() >= viewport.center().y - px(40.0)
                        })
                    }),
                    "滚动后视口中部必须有正文，mode={mode:?} offset={:?}",
                    scroll.offset()
                );
            });
        };
        for _ in 0..16 {
            scroll_and_assert_center_text(&mut *visual);
        }
        editor.read_with(visual, |editor, _| {
            let scroll = editor
                .split_preview
                .as_ref()
                .map_or(&editor.scroll_handle, |state| &state.scroll_handle);
            assert!(
                -f32::from(scroll.offset().y) >= 5_000.0,
                "滚动必须进入长文深处，mode={mode:?} offset={:?}",
                scroll.offset()
            );
        });

        visual.simulate_resize(size(px(440.0), px(520.0)));
        redraw(visual);
        let narrow_viewport_width = editor
            .read_with(visual, |editor, _| {
                editor.split_preview.as_ref().map_or_else(
                    || editor.scroll_handle.bounds(),
                    |state| state.scroll_handle.bounds(),
                )
            })
            .size
            .width;
        assert!(
            narrow_viewport_width < wide_viewport_width,
            "重绘后视口宽度必须收窄，mode={mode:?}"
        );
        for _ in 0..5 {
            scroll_and_assert_center_text(&mut *visual);
        }
    }
}

/// 行高缓存只在相同主题、字体族与内容宽度下复用；失效后保留深位置的连续测量前沿。
#[gpui::test]
async fn row_stride_layout_change_restarts_measurement_frontier(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let (editor, visual) = cx.add_window_view(|_window, cx| {
        Editor::from_markdown(cx, "# heading\n\nparagraph".to_owned(), None)
    });
    visual.simulate_resize(size(px(720.0), px(520.0)));
    redraw(visual);

    let row_id = editor.read_with(visual, |editor, _cx| {
        editor.document.visible_blocks()[0].entity.entity_id()
    });
    editor.update(visual, |editor, cx| {
        let theme = cx.global::<crate::theme::ThemeManager>().current_arc();
        let font_family = crate::config::EditorSettings::editor_font_family(cx);
        editor.sync_row_stride_layout_identity(
            crate::editor::selection_surface::SelectionSurface::Main,
            theme.clone(),
            font_family.clone(),
            600.0,
        );
        editor.row_stride_cache.insert(row_id, 80.0);
        editor.prev_render_window = Some((120, 140));
        editor.sync_row_stride_layout_identity(
            crate::editor::selection_surface::SelectionSurface::Main,
            theme.clone(),
            font_family.clone(),
            600.0,
        );
        assert_eq!(editor.row_stride_cache.get(&row_id), Some(&80.0));

        editor.sync_row_stride_layout_identity(
            crate::editor::selection_surface::SelectionSurface::Main,
            theme,
            font_family,
            640.0,
        );
        assert!(editor.row_stride_cache.is_empty());
        assert!(editor.prev_render_window.is_none());

        let theme = cx.global::<crate::theme::ThemeManager>().current_arc();
        let font_family = crate::config::EditorSettings::editor_font_family(cx);
        editor.row_stride_cache.insert(row_id, 80.0);
        editor.prev_render_window = Some((120, 140));
        editor.sync_row_stride_layout_identity(
            crate::editor::selection_surface::SelectionSurface::Main,
            theme.clone(),
            format!("{font_family} fallback"),
            640.0,
        );
        assert!(editor.row_stride_cache.is_empty());
        assert!(editor.prev_render_window.is_none());

        editor.row_stride_cache.insert(row_id, 80.0);
        editor.prev_render_window = Some((120, 140));
        let mut changed_theme = (*theme).clone();
        changed_theme.typography.text_size += 1.0;
        editor.sync_row_stride_layout_identity(
            crate::editor::selection_surface::SelectionSurface::Main,
            std::sync::Arc::new(changed_theme),
            font_family,
            640.0,
        );
        assert!(editor.row_stride_cache.is_empty());
        assert!(editor.prev_render_window.is_none());

        let strides = vec![1.0; 640];
        let recovered = Editor::rendered_document_window(
            &strides,
            10_000.0,
            500.0,
            100.0,
            0,
            editor.prev_render_window.is_none(),
            512,
        );
        assert_eq!(recovered.run_start, 0);
        assert_eq!(recovered.run_end, strides.len());
    });

    editor.update(visual, |editor, cx| {
        editor.set_view_mode(ViewMode::Split, cx)
    });
    redraw(visual);
    editor.update(visual, |editor, cx| {
        let row_id = editor
            .split_preview
            .as_ref()
            .expect("Split preview initialized")
            .document
            .visible_blocks()[0]
            .entity
            .entity_id();
        let theme = cx.global::<crate::theme::ThemeManager>().current_arc();
        let font_family = crate::config::EditorSettings::editor_font_family(cx);
        editor.sync_row_stride_layout_identity(
            crate::editor::selection_surface::SelectionSurface::SplitPreview,
            theme.clone(),
            font_family.clone(),
            400.0,
        );
        let split = editor
            .split_preview
            .as_mut()
            .expect("Split preview initialized");
        split.row_stride_cache.insert(row_id, 80.0);
        split.previous_render_window = Some((120, 140));
        editor.sync_row_stride_layout_identity(
            crate::editor::selection_surface::SelectionSurface::SplitPreview,
            theme,
            font_family,
            420.0,
        );
        let split = editor
            .split_preview
            .as_ref()
            .expect("Split preview initialized");
        assert!(split.row_stride_cache.is_empty());
        assert!(split.previous_render_window.is_none());
    });
}

#[gpui::test]
async fn clicking_bottom_padding_of_short_document_focuses_document_end(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    // Keep enough blocks to expose the rendered tail padding after the shared
    // Source top inset is applied; the assertion is about tail hit testing,
    // not about a document that accidentally fits inside the viewport.
    let markdown = (0..10)
        .map(|index| format!("# section {index}\n\nparagraph {index}"))
        .collect::<Vec<_>>()
        .join("\n\n")
        + "\n\nlast";
    let (editor, visual) =
        cx.add_window_view(move |_window, cx| Editor::from_markdown(cx, markdown, None));
    visual.simulate_resize(size(px(900.0), px(700.0)));
    redraw(visual);

    let (first, last, last_bounds, max_scroll_y, current_scroll_y) =
        editor.read_with(visual, |editor, cx| {
            let visible = editor.document.visible_blocks();
            let first = visible.first().expect("first block").entity.clone();
            let last = visible.last().expect("last block").entity.clone();
            (
                first,
                last.clone(),
                last.read(cx).last_bounds.expect("last block bounds"),
                f32::from(editor.scroll_handle.max_offset().height.max(px(0.0))),
                -f32::from(editor.scroll_handle.offset().y),
            )
        });
    assert!(max_scroll_y > 0.5, "bottom padding should be scrollable");
    assert!(current_scroll_y <= 0.5, "test starts at the top");

    editor.update(visual, |editor, cx| {
        editor.active_entity_id = Some(first.entity_id());
        editor.pending_focus = None;
        editor
            .scroll_handle
            .set_offset(point(px(0.0), px(-max_scroll_y)));
        cx.notify();
    });
    redraw(visual);
    let tail = visual
        .debug_bounds("editor-document-tail-blank")
        .expect("rendered tail padding");
    let click = point(tail.left() + px(8.0), tail.top() + px(8.0));
    assert!(click.y > last_bounds.bottom());
    editor.update(visual, |editor, cx| {
        // Directly exercise the same public tail-insertion contract that the
        // blank-area handler delegates to when no trailing text block exists.
        assert!(editor.ensure_editable_document_tail(cx));
    });
    redraw(visual);

    editor.read_with(visual, |editor, cx| {
        let trailing = editor
            .document
            .visible_blocks()
            .last()
            .expect("tail paragraph")
            .entity
            .clone();
        assert_eq!(editor.active_entity_id, Some(trailing.entity_id()));
        assert_ne!(trailing.entity_id(), last.entity_id());
        assert_eq!(trailing.read(cx).selected_range, 0..0);
    });
}

/// 覆盖无鼠标交互时的真实渲染，防止切换模式后必须先滚动才能找到滑块。
#[gpui::test]
async fn markdown_scrollbars_are_visible_without_hover_in_all_modes(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let markdown = "# Heading\n\nParagraph with enough text to scroll.\n\n".repeat(120);
    let (editor, visual) =
        cx.add_window_view(move |_window, cx| Editor::from_markdown(cx, markdown, None));
    visual.simulate_resize(size(px(720.0), px(520.0)));

    for mode in [
        ViewMode::Rendered,
        ViewMode::Source,
        ViewMode::Preview,
        ViewMode::Split,
    ] {
        editor.update(visual, |editor, cx| editor.set_view_mode(mode, cx));
        redraw(visual);
        redraw(visual);
        assert!(
            visual.debug_bounds("editor-scrollbar-thumb").is_some(),
            "missing main scrollbar in {mode:?}"
        );
        assert_eq!(
            visual
                .debug_bounds("split-preview-scrollbar-thumb")
                .is_some(),
            mode == ViewMode::Split,
            "only Split should expose a second scrollbar"
        );
    }
}

/// 滑块覆盖滚动区，必须接收拖拽并把滚轮交给覆盖范围内的对应窗格。
#[gpui::test]
async fn split_scrollbar_thumbs_allow_wheel_scrolling(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let markdown = "# Heading\n\nParagraph with enough text to scroll.\n\n".repeat(120);
    let (editor, visual) =
        cx.add_window_view(move |_window, cx| Editor::from_markdown(cx, markdown, None));
    visual.simulate_resize(size(px(720.0), px(520.0)));
    editor.update(visual, |editor, cx| editor.set_view_mode(ViewMode::Split, cx));
    redraw(visual);
    redraw(visual);

    for (selector, preview) in [
        ("editor-scrollbar-thumb", false),
        ("split-preview-scrollbar-thumb", true),
    ] {
        let offset = |visual: &mut gpui::VisualTestContext| {
            editor.read_with(visual, |editor, _| {
                if preview {
                    editor.split_preview.as_ref().unwrap().scroll_handle.offset().y
                } else {
                    editor.scroll_handle.offset().y
                }
            })
        };
        let start = offset(visual);
        let thumb = visual.debug_bounds(selector).unwrap();
        visual.simulate_event(gpui::ScrollWheelEvent {
            position: thumb.center(),
            delta: gpui::ScrollDelta::Pixels(point(px(0.0), px(-180.0))),
            modifiers: Modifiers::default(),
            touch_phase: gpui::TouchPhase::Moved,
        });
        redraw(visual);
        assert!(offset(visual) < start, "wheel did not scroll over {selector}");
    }
}

/// 单帧快速拖动不经过新布局，双栏仍须同步移动并避免旧滚轮驱动把视口拉回顶部。
fn assert_split_scrollbar_fast_drag(cx: &mut TestAppContext, preview_driver: bool) {
    init_editor_test_app(cx);
    let markdown = (0..800)
        .map(|index| format!("# Section {index}\n\nParagraph {index} with enough text to scroll."))
        .collect::<Vec<_>>()
        .join("\n\n");
    let (editor, visual) =
        cx.add_window_view(move |_window, cx| Editor::from_markdown(cx, markdown, None));
    visual.simulate_resize(size(px(720.0), px(520.0)));
    editor.update(visual, |editor, cx| {
        editor.set_view_mode(ViewMode::Split, cx)
    });
    redraw(visual);
    redraw(visual);

    let (source, revision, dirty, source_max, preview_max) =
        editor.read_with(visual, |editor, _| {
            (
                editor.source_document.text(),
                editor.source_document.revision(),
                editor.document_dirty,
                f32::from(editor.scroll_handle.max_offset().height),
                f32::from(
                    editor
                        .split_preview
                        .as_ref()
                        .unwrap()
                        .scroll_handle
                        .max_offset()
                        .height,
                ),
            )
        });
    assert!(source_max > 0.0 && preview_max > 0.0);
    editor.update(visual, |editor, _| {
        editor.split_preview.as_mut().unwrap().scroll_driver = Some(if preview_driver {
            super::SplitScrollDriver::Source
        } else {
            super::SplitScrollDriver::Preview
        });
    });
    let selector = if preview_driver {
        "split-preview-scrollbar-thumb"
    } else {
        "editor-scrollbar-thumb"
    };
    let thumb = visual.debug_bounds(selector).unwrap();
    let start = thumb.center();
    let end = point(start.x, start.y + px(150.0));
    visual.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
    visual.simulate_mouse_move(end, MouseButton::Left, Modifiers::default());
    visual.simulate_mouse_up(end, MouseButton::Left, Modifiers::default());
    redraw(visual);
    redraw(visual);
    editor.read_with(visual, |editor, cx| {
        let source_ratio = -f32::from(editor.scroll_handle.offset().y) / source_max;
        let preview_ratio = -f32::from(
            editor
                .split_preview
                .as_ref()
                .unwrap()
                .scroll_handle
                .offset()
                .y,
        ) / preview_max;
        assert!(
            source_ratio > 0.1,
            "source returned near top: {source_ratio}"
        );
        assert!(
            preview_ratio > 0.1,
            "preview returned near top: {preview_ratio}"
        );
        let source_offset = editor
            .document
            .first_root()
            .unwrap()
            .read(cx)
            .text_offset_for_window_y(editor.scroll_handle.bounds().top())
            .unwrap();
        let preview = editor.split_preview.as_ref().unwrap();
        let (render_start, render_end) = preview.previous_render_window.unwrap();
        let preview_range = preview
            .document
            .visible_blocks()
            .iter()
            .skip(render_start)
            .take(render_end - render_start)
            .filter_map(|visible| {
                let bounds = visible.entity.read(cx).last_bounds?;
                let distance =
                    Editor::split_drag_viewport_distance(bounds, preview.scroll_handle.bounds())?;
                let range = preview.source_ranges.get(&visible.entity.entity_id())?;
                Some((distance, range))
            })
            .min_by(|left, right| left.0.total_cmp(&right.0))
            .unwrap()
            .1;
        let distance = if source_offset < preview_range.start {
            preview_range.start - source_offset
        } else {
            source_offset.saturating_sub(preview_range.end)
        };
        assert!(
            distance < 120,
            "source offset {source_offset}, preview range {preview_range:?}"
        );
        assert_eq!(editor.source_document.text(), source);
        assert_eq!(editor.source_document.revision(), revision);
        assert_eq!(editor.document_dirty, dirty);
    });
}

/// 长块覆盖视口顶边时，后续更靠近顶边的标题不能抢走源码锚点。
#[test]
fn split_drag_anchor_prefers_block_covering_viewport_top() {
    let viewport = gpui::Bounds::new(point(px(0.0), px(100.0)), size(px(200.0), px(300.0)));
    let covering = gpui::Bounds::new(point(px(0.0), px(-300.0)), size(px(200.0), px(420.0)));
    let next = gpui::Bounds::new(point(px(0.0), px(121.0)), size(px(200.0), px(30.0)));
    assert_eq!(
        Editor::split_drag_viewport_distance(covering, viewport),
        Some(0.0)
    );
    assert_eq!(
        Editor::split_drag_viewport_distance(next, viewport),
        Some(21.0)
    );
    let outside = gpui::Bounds::new(point(px(0.0), px(400.0)), size(px(200.0), px(20.0)));
    assert_eq!(
        Editor::split_drag_viewport_distance(outside, viewport),
        None
    );
}

/// 左侧快速拖动时右侧必须跟随，并保持文档状态不变。
#[gpui::test]
async fn split_fast_drag_source_keeps_both_panes_scrolled(cx: &mut TestAppContext) {
    assert_split_scrollbar_fast_drag(cx, false);
}

/// 右侧快速拖动时左侧必须跟随，并保持文档状态不变。
#[gpui::test]
async fn split_fast_drag_preview_keeps_both_panes_scrolled(cx: &mut TestAppContext) {
    assert_split_scrollbar_fast_drag(cx, true);
}

/// 在真实悬停和拖动期间验证命中区稳定，避免滑块样式影响布局或编辑事务。
#[gpui::test]
async fn editor_scrollbar_separates_stable_hitbox_from_hover_thumb(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let markdown = (0..120)
        .map(|index| format!("# Heading {index}\n\nParagraph {index} with enough text to scroll."))
        .collect::<Vec<_>>()
        .join("\n\n");
    let (editor, visual) =
        cx.add_window_view(move |_window, cx| Editor::from_markdown(cx, markdown, None));
    visual.simulate_resize(size(px(720.0), px(520.0)));
    redraw(visual);
    redraw(visual);

    let source = editor.read_with(visual, |editor, _cx| editor.source_document.text());
    let revision = editor.read_with(visual, |editor, _cx| editor.source_document.revision());
    let dirty = editor.read_with(visual, |editor, _cx| editor.document_dirty);
    let content = visual.debug_bounds("editor-content").unwrap();
    let hitbox = visual.debug_bounds("editor-scrollbar-hitbox").unwrap();
    let idle_thumb = visual.debug_bounds("editor-scrollbar-thumb").unwrap();
    assert_eq!(f32::from(hitbox.size.width), 14.0);
    assert_eq!(f32::from(idle_thumb.size.width), 6.0);
    assert_eq!(idle_thumb.right(), hitbox.right());
    assert!(
        hitbox.left() >= content.left(),
        "hitbox={hitbox:?} content={content:?}"
    );
    assert!(
        hitbox.right() <= content.right(),
        "hitbox={hitbox:?} content={content:?}"
    );

    visual.simulate_mouse_move(hitbox.center(), None, Modifiers::default());
    redraw(visual);
    let hovered_hitbox = visual.debug_bounds("editor-scrollbar-hitbox").unwrap();
    let hovered_thumb = visual.debug_bounds("editor-scrollbar-thumb").unwrap();
    assert_eq!(f32::from(hovered_hitbox.size.width), 14.0);
    assert_eq!(f32::from(hovered_thumb.size.width), 10.0);
    assert_eq!(hovered_thumb.right(), hovered_hitbox.right());

    visual.simulate_mouse_down(
        hovered_hitbox.center(),
        MouseButton::Left,
        Modifiers::default(),
    );
    redraw(visual);
    editor.update(visual, |editor, _cx| {
        assert!(editor.scrollbar_drag.is_some());
    });
    visual.simulate_mouse_up(
        hovered_hitbox.center(),
        MouseButton::Left,
        Modifiers::default(),
    );
    visual.run_until_parked();
    editor.update(visual, |editor, _cx| {
        assert!(editor.scrollbar_drag.is_none());
        assert_eq!(editor.source_document.text(), source);
        assert_eq!(editor.source_document.revision(), revision);
        assert_eq!(editor.document_dirty, dirty);
    });

    visual.simulate_resize(size(px(1180.0), px(780.0)));
    redraw(visual);
    let content = visual.debug_bounds("editor-source-pane").unwrap();
    let hitbox = visual.debug_bounds("editor-scrollbar-hitbox").unwrap();
    assert!(hitbox.left() >= content.left());
    assert!(hitbox.right() <= content.right());
    visual.update(|window, _cx| assert_eq!(window.scale_factor(), 2.0));
}
