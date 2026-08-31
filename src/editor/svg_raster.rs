// @author kongweiguang

//! SVG 解析与光栅化的共享安全边界。

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, LazyLock};

use anyhow::{Context as _, Result, bail};

pub(super) const MAX_SVG_SOURCE_BYTES: usize = 16 * 1024 * 1024;
pub(super) const MAX_EMBEDDED_RASTER_BYTES: usize = 8 * 1024 * 1024;
pub(super) const MAX_EMBEDDED_RASTER_SIDE: u32 = 4_096;
pub(super) const MAX_EMBEDDED_RASTER_PIXELS: u64 = 4 * 1024 * 1024;

const RESOURCE_VIOLATION_NONE: u8 = 0;
const RESOURCE_VIOLATION_EXTERNAL: u8 = 1;
const RESOURCE_VIOLATION_TOO_LARGE: u8 = 2;
const RESOURCE_VIOLATION_UNSUPPORTED: u8 = 3;
const RESOURCE_VIOLATION_INVALID_RASTER: u8 = 4;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct SvgRasterSize {
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) scale: f32,
}

/// 解析不产生文件或网络副作用的 SVG，并为内嵌栅格资源设置独立上限。
///
/// 默认 `usvg` resolver 会把任意字符串当作本地路径整体读取；这里明确关闭该能力，
/// 同时拒绝嵌套 SVG，避免递归解析绕过外层源文件预算。
pub(super) fn parse_restricted_svg(source: &[u8]) -> Result<resvg::usvg::Tree> {
    if source.len() > MAX_SVG_SOURCE_BYTES {
        bail!(
            "SVG source is {} bytes; the limit is {} bytes",
            source.len(),
            MAX_SVG_SOURCE_BYTES
        );
    }

    static FONT_DB: LazyLock<Arc<resvg::usvg::fontdb::Database>> = LazyLock::new(|| {
        let mut database = resvg::usvg::fontdb::Database::new();
        database.load_system_fonts();
        Arc::new(database)
    });

    let violation = Arc::new(AtomicU8::new(RESOURCE_VIOLATION_NONE));
    let data_violation = Arc::clone(&violation);
    let string_violation = Arc::clone(&violation);
    let default_data_resolver = resvg::usvg::ImageHrefResolver::default_data_resolver();
    let image_href_resolver = resvg::usvg::ImageHrefResolver {
        resolve_data: Box::new(move |mime, data, options| {
            if data.len() > MAX_EMBEDDED_RASTER_BYTES {
                data_violation.fetch_max(RESOURCE_VIOLATION_TOO_LARGE, Ordering::Relaxed);
                return None;
            }
            if !matches!(
                mime,
                "image/jpg" | "image/jpeg" | "image/png" | "image/gif" | "image/webp"
            ) {
                data_violation.fetch_max(RESOURCE_VIOLATION_UNSUPPORTED, Ordering::Relaxed);
                return None;
            }
            if !embedded_raster_is_bounded(mime, &data) {
                data_violation.fetch_max(RESOURCE_VIOLATION_INVALID_RASTER, Ordering::Relaxed);
                return None;
            }
            default_data_resolver(mime, data, options)
        }),
        resolve_string: Box::new(move |_href, _options| {
            string_violation.fetch_max(RESOURCE_VIOLATION_EXTERNAL, Ordering::Relaxed);
            None
        }),
    };
    let options = resvg::usvg::Options {
        resources_dir: None,
        image_href_resolver,
        fontdb: Arc::clone(&FONT_DB),
        ..resvg::usvg::Options::default()
    };
    let parsed = resvg::usvg::Tree::from_data(source, &options);

    match violation.load(Ordering::Relaxed) {
        RESOURCE_VIOLATION_EXTERNAL => {
            bail!("SVG external image references are not allowed")
        }
        RESOURCE_VIOLATION_TOO_LARGE => bail!(
            "SVG embedded raster exceeds the {} byte limit",
            MAX_EMBEDDED_RASTER_BYTES
        ),
        RESOURCE_VIOLATION_UNSUPPORTED => {
            bail!("SVG embedded image type is not allowed")
        }
        RESOURCE_VIOLATION_INVALID_RASTER => bail!(
            "SVG embedded raster is invalid or exceeds the {} px / {} pixel budget",
            MAX_EMBEDDED_RASTER_SIDE,
            MAX_EMBEDDED_RASTER_PIXELS
        ),
        _ => parsed.context("failed to parse restricted SVG"),
    }
}

/// 只读取内嵌栅格头部并核对 MIME 与解码尺寸，防止小型压缩载荷触发巨量分配。
fn embedded_raster_is_bounded(mime: &str, data: &[u8]) -> bool {
    let expected_format = match mime {
        "image/jpg" | "image/jpeg" => image::ImageFormat::Jpeg,
        "image/png" => image::ImageFormat::Png,
        "image/gif" => image::ImageFormat::Gif,
        "image/webp" => image::ImageFormat::WebP,
        _ => return false,
    };
    let Ok(reader) = image::ImageReader::new(std::io::Cursor::new(data)).with_guessed_format()
    else {
        return false;
    };
    if reader.format() != Some(expected_format) {
        return false;
    }
    let Ok((width, height)) = reader.into_dimensions() else {
        return false;
    };
    width <= MAX_EMBEDDED_RASTER_SIDE
        && height <= MAX_EMBEDDED_RASTER_SIDE
        && u64::from(width)
            .checked_mul(u64::from(height))
            .is_some_and(|pixels| pixels <= MAX_EMBEDDED_RASTER_PIXELS)
}

/// 将 SVG 按指定倍率光栅化，同时硬性限制最长边和总像素，避免取整后突破预算。
pub(super) fn bounded_svg_raster_size(
    width: f32,
    height: f32,
    requested_scale: f64,
    max_edge: u32,
    max_pixels: u64,
) -> Result<SvgRasterSize> {
    if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
        bail!("SVG dimensions must be finite and non-zero");
    }
    if !requested_scale.is_finite() || requested_scale <= 0.0 || max_edge == 0 || max_pixels == 0 {
        bail!("SVG raster budget must be finite and non-zero");
    }

    let width = f64::from(width);
    let height = f64::from(height);
    let edge_scale = (f64::from(max_edge) / width).min(f64::from(max_edge) / height);
    let pixel_scale = (max_pixels as f64 / (width * height)).sqrt();
    let scale = requested_scale.min(edge_scale).min(pixel_scale);
    if !scale.is_finite() || scale <= 0.0 {
        bail!("SVG dimensions exceed the bounded raster budget");
    }

    let output_width = (width * scale).floor().max(1.0) as u32;
    let output_height = (height * scale).floor().max(1.0) as u32;
    let output_pixels = u64::from(output_width)
        .checked_mul(u64::from(output_height))
        .context("SVG raster dimensions overflow")?;
    if output_width > max_edge || output_height > max_edge || output_pixels > max_pixels {
        bail!("SVG raster dimensions exceed the bounded raster budget");
    }

    Ok(SvgRasterSize {
        width: output_width,
        height: output_height,
        scale: scale as f32,
    })
}

/// 在目标矩形内等比放置 SVG；宽高必须同时受约束，不能只匹配最长边。
pub(super) fn fit_svg_raster_size(
    width: f32,
    height: f32,
    target_pixels: (u32, u32),
    max_edge: u32,
    max_pixels: u64,
) -> Result<SvgRasterSize> {
    if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
        bail!("SVG dimensions must be finite and non-zero");
    }
    let target_width = target_pixels.0.clamp(1, max_edge);
    let target_height = target_pixels.1.clamp(1, max_edge);
    let requested_scale = (f64::from(target_width) / f64::from(width))
        .min(f64::from(target_height) / f64::from(height));
    bounded_svg_raster_size(width, height, requested_scale, max_edge, max_pixels)
}

#[cfg(test)]
#[path = "../../tests/unit/editor/svg_raster.rs"]
mod tests;
