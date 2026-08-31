// @author kongweiguang

//! 可编辑 SVG 文档的有界派生预览。

use anyhow::{Context as _, Result};
use image::Frame;
use smallvec::smallvec;
use std::hash::{Hash, Hasher};

use super::*;
use crate::i18n::I18nStrings;
use crate::theme::Theme;

pub(super) const SVG_PREVIEW_MAX_EDGE: u32 = 8_192;
pub(super) const SVG_PREVIEW_MAX_PIXELS: u64 = 32 * 1024 * 1024;
const SVG_PREVIEW_QUALITY_SCALE: f32 = 2.0;

#[derive(Clone, PartialEq, Eq)]
struct SvgPreviewSource(Arc<[u8]>);

impl Hash for SvgPreviewSource {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

pub(super) struct SvgPreviewCache {
    revision: Revision,
    path: Option<PathBuf>,
    source: SvgPreviewSource,
}

enum SvgPreviewAssetLoader {}

impl Asset for SvgPreviewAssetLoader {
    type Source = SvgPreviewSource;
    type Output = std::result::Result<Arc<RenderImage>, Arc<anyhow::Error>>;

    // reason: GPUI 的 Asset trait 固定要求返回 impl Future；remove when: trait 接受 async fn 或该 lint 不再触发。
    #[allow(clippy::manual_async_fn)]
    fn load(
        source: Self::Source,
        _cx: &mut App,
    ) -> impl Future<Output = Self::Output> + Send + 'static {
        async move {
            let result = (|| -> Result<Arc<RenderImage>> {
                let tree = super::svg_raster::parse_restricted_svg(&source.0)
                    .context("failed to parse SVG preview")?;
                let size = tree.size();
                let raster_size = svg_preview_raster_size(size.width(), size.height())?;
                let mut pixmap =
                    resvg::tiny_skia::Pixmap::new(raster_size.width, raster_size.height)
                        .context("SVG preview dimensions exceed the renderer limit")?;
                resvg::render(
                    &tree,
                    resvg::tiny_skia::Transform::from_scale(raster_size.scale, raster_size.scale),
                    &mut pixmap.as_mut(),
                );
                let mut buffer =
                    image::RgbaImage::from_raw(pixmap.width(), pixmap.height(), pixmap.take())
                        .context("SVG renderer returned an invalid pixel buffer")?;
                for pixel in buffer.as_flat_samples_mut().samples.chunks_exact_mut(4) {
                    // tiny-skia 输出预乘 RGBA；GPUI RenderImage 接收非预乘 BGRA。
                    pixel.swap(0, 2);
                    if pixel[3] > 0 {
                        let alpha = f32::from(pixel[3]) / 255.0;
                        for channel in &mut pixel[..3] {
                            *channel = (f32::from(*channel) / alpha).min(255.0) as u8;
                        }
                    }
                }
                Ok(Arc::new(RenderImage::new(smallvec![Frame::new(buffer)])))
            })();
            result.map_err(Arc::new)
        }
    }
}

/// 为独立 SVG 文档计算清晰但有界的预览尺寸，避免高 DPI 质量目标突破硬预算。
pub(super) fn svg_preview_raster_size(
    width: f32,
    height: f32,
) -> Result<super::svg_raster::SvgRasterSize> {
    super::svg_raster::bounded_svg_raster_size(
        width,
        height,
        f64::from(SVG_PREVIEW_QUALITY_SCALE),
        SVG_PREVIEW_MAX_EDGE,
        SVG_PREVIEW_MAX_PIXELS,
    )
}

impl Editor {
    pub(super) fn is_svg_document(&self) -> bool {
        self.file_path
            .as_deref()
            .is_some_and(crate::document_io::is_svg_path)
    }

    fn current_svg_preview_source(&mut self) -> SvgPreviewSource {
        let revision = self.source_document.revision();
        let path = self.file_path.clone();
        if let Some(cache) = self.svg_preview_cache.as_ref()
            && cache.revision == revision
            && cache.path == path
        {
            return cache.source.clone();
        }

        let source = SvgPreviewSource(self.source_document.text().into_bytes().into());
        self.svg_preview_cache = Some(SvgPreviewCache {
            revision,
            path,
            source: source.clone(),
        });
        source
    }

    pub(super) fn render_svg_document_preview(
        &mut self,
        theme: &Theme,
        strings: &I18nStrings,
        split: bool,
        _cx: &mut Context<Self>,
    ) -> AnyElement {
        let source = self.current_svg_preview_source();
        let asset_source = source.clone();
        let fallback_theme = theme.clone();
        let fallback_strings = strings.clone();
        let loading_theme = theme.clone();
        let loading_strings = strings.clone();
        let image = img(move |window: &mut Window, cx: &mut App| {
            window
                .use_asset::<SvgPreviewAssetLoader>(&asset_source, cx)
                .map(|result| result.map_err(ImageCacheError::Other))
        })
        .id(if split {
            "split-svg-preview-content"
        } else {
            "svg-preview-content"
        })
        .debug_selector(move || {
            if split {
                "split-svg-preview-content".to_owned()
            } else {
                "svg-preview-content".to_owned()
            }
        })
        .w_full()
        .h_auto()
        .object_fit(ObjectFit::Contain)
        .with_loading(move || {
            svg_preview_message(
                loading_strings.image_loading_without_alt.clone(),
                &loading_theme,
            )
        })
        .with_fallback(move || {
            svg_preview_message(
                fallback_strings.file_open_failed_title.clone(),
                &fallback_theme,
            )
        });

        div()
            .id(if split {
                "split-svg-preview-pane"
            } else {
                "svg-preview-pane"
            })
            .debug_selector(move || {
                if split {
                    "split-svg-preview-pane".to_owned()
                } else {
                    "svg-preview-pane".to_owned()
                }
            })
            .size_full()
            .min_w(px(0.0))
            .overflow_scroll()
            .bg(theme.colors.editor_background)
            .p(px(if split { 18.0 } else { 24.0 }))
            .flex()
            .items_start()
            .justify_center()
            .child(
                div()
                    .w_full()
                    .max_w(px(if split { 1_200.0 } else { 1_600.0 }))
                    .min_w(px(1.0))
                    .child(image),
            )
            .into_any_element()
    }
}

fn svg_preview_message(message: impl Into<SharedString>, theme: &Theme) -> AnyElement {
    div()
        .w_full()
        .min_h(px(160.0))
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(8.0))
        .text_color(theme.colors.workbench.text_secondary)
        .child(
            svg()
                .path("icon/ui/image.svg")
                .size(px(28.0))
                .text_color(theme.colors.workbench.text_secondary),
        )
        .child(message.into())
        .into_any_element()
}

#[cfg(test)]
#[path = "../../tests/unit/editor/svg_preview.rs"]
mod tests;
