// @author kongweiguang

use super::{SVG_PREVIEW_MAX_EDGE, SVG_PREVIEW_MAX_PIXELS, svg_preview_raster_size};

/// 验证常规 SVG 保持 2x 清晰度，超大 SVG 则服从共享硬预算。
#[test]
fn svg_preview_keeps_regular_documents_crisp_without_unbounded_rasters() {
    assert_eq!(
        svg_preview_raster_size(800.0, 400.0)
            .expect("regular preview size")
            .scale,
        2.0
    );

    let size = svg_preview_raster_size(100_000.0, 50_000.0).expect("bounded preview size");
    assert!(size.width <= SVG_PREVIEW_MAX_EDGE);
    assert!(size.height <= SVG_PREVIEW_MAX_EDGE);
    assert!(u64::from(size.width) * u64::from(size.height) <= SVG_PREVIEW_MAX_PIXELS);
}

/// 验证非法维度在进入 tiny-skia 分配前被拒绝。
#[test]
fn svg_preview_rejects_zero_or_non_finite_dimensions() {
    assert!(svg_preview_raster_size(0.0, 100.0).is_err());
    assert!(svg_preview_raster_size(f32::INFINITY, 100.0).is_err());
}

/// 验证 SVG 文档的 Preview/Split 入口保持可见，Source 仍为唯一可编辑表面。
#[gpui::test]
async fn svg_document_renders_in_preview_and_split_while_source_remains_editable(
    cx: &mut gpui::TestAppContext,
) {
    cx.update(|cx| {
        crate::i18n::I18nManager::init_with_language_id(cx, "en-US");
        crate::theme::ThemeManager::init(cx);
        crate::components::init(cx);
    });
    let source = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 120 60"><rect width="120" height="60" fill="#4f7cff"/></svg>"##;
    let (editor, visual) = cx.add_window_view(|_window, cx| {
        super::Editor::from_opened_markdown(
            cx,
            crate::document_io::OpenedMarkdown {
                text: source.to_owned(),
                encoding: crate::document_io::DocumentEncoding::Utf8,
                text_encoding: gmark_document_core::TextEncoding::Utf8 { bom: false },
                file_identity: None,
                loading_limits: gmark_document_core::LoadingPolicy::default().effective_limits(),
            },
            Some(std::path::PathBuf::from("preview.svg")),
        )
    });
    visual.simulate_resize(gpui::size(gpui::px(900.0), gpui::px(640.0)));

    assert_eq!(
        editor.read_with(visual, |editor, _cx| editor.view_mode),
        super::ViewMode::Preview
    );
    visual.update(|window, cx| window.draw(cx).clear());
    assert!(visual.debug_bounds("svg-preview-content").is_some());

    editor.update(visual, |editor, cx| {
        editor.set_view_mode(super::ViewMode::Source, cx)
    });
    visual.update(|window, cx| window.draw(cx).clear());
    assert!(visual.debug_bounds("editor-source-pane").is_some());

    editor.update(visual, |editor, cx| {
        editor.set_view_mode(super::ViewMode::Split, cx)
    });
    visual.update(|window, cx| window.draw(cx).clear());
    assert!(visual.debug_bounds("split-source-pane-shell").is_some());
    assert!(visual.debug_bounds("split-svg-preview-content").is_some());

    editor.update(visual, |editor, cx| {
        editor.set_view_mode(super::ViewMode::Rendered, cx);
        assert_eq!(editor.view_mode, super::ViewMode::Split);
    });
}
