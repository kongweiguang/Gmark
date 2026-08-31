// @author kongweiguang

use super::*;
use base64::Engine as _;
use std::io::Cursor;

/// 验证窄目标同时限制两个轴，防止最长边算法放大缓存和 GPU 上传负担。
#[test]
fn target_fit_respects_width_and_height() {
    let size =
        fit_svg_raster_size(100.0, 100.0, (320, 40), 4096, 16_777_216).expect("bounded SVG size");
    assert_eq!((size.width, size.height), (40, 40));
}

/// 验证最长边与像素预算在超大 SVG 上仍然是硬上限。
#[test]
fn raster_size_respects_edge_and_pixel_budgets() {
    let size = bounded_svg_raster_size(100_000.0, 50_000.0, 2.0, 8192, 32 * 1024 * 1024)
        .expect("bounded SVG size");
    assert!(size.width <= 8192);
    assert!(size.height <= 8192);
    assert!(u64::from(size.width) * u64::from(size.height) <= 32 * 1024 * 1024);
}

/// 验证默认 usvg 本地文件能力不会重新进入共享 SVG 解析边界。
#[test]
fn external_image_references_are_rejected() {
    let source = br#"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8">
        <image href="C:/private/oversized.png" width="8" height="8"/>
    </svg>"#;
    let error = parse_restricted_svg(source).expect_err("external reference must fail");
    assert!(error.to_string().contains("external image references"));
}

/// 验证常见的自包含栅格资源仍可使用，安全策略不破坏独立 SVG 文件。
#[test]
fn bounded_embedded_png_is_allowed() {
    let source = br#"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1">
        <image href="data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=" width="1" height="1"/>
    </svg>"#;
    parse_restricted_svg(source).expect("embedded PNG remains supported");
}

/// 验证嵌套 SVG 不会借助 data URL 形成递归解析和资源预算绕过。
#[test]
fn embedded_svg_is_rejected() {
    let source = br#"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1">
        <image href="data:image/svg+xml;base64,PHN2Zy8+" width="1" height="1"/>
    </svg>"#;
    let error = parse_restricted_svg(source).expect_err("nested SVG must fail");
    assert!(error.to_string().contains("type is not allowed"));
}

/// 验证压缩体积很小但解码边长超限的 PNG 在进入 resvg 解码器前被拒绝。
#[test]
fn embedded_raster_dimensions_are_bounded_before_rendering() {
    let image = image::RgbaImage::from_pixel(
        MAX_EMBEDDED_RASTER_SIDE + 1,
        1,
        image::Rgba([0, 128, 255, 255]),
    );
    let mut png = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image)
        .write_to(&mut png, image::ImageFormat::Png)
        .expect("encode oversized-dimension PNG");
    let encoded = base64::engine::general_purpose::STANDARD.encode(png.into_inner());
    let source = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"1\" height=\"1\">\
         <image href=\"data:image/png;base64,{encoded}\" width=\"1\" height=\"1\"/>\
         </svg>"
    );

    let error = parse_restricted_svg(source.as_bytes()).expect_err("oversized raster must fail");
    assert!(error.to_string().contains("embedded raster is invalid"));
}
