// @author kongweiguang

use super::*;
use std::io::Write;

fn key(name: &str) -> AssetKey {
    AssetKey::new(name, "v1", 100, 100)
}

fn value(size: usize) -> AssetValue {
    AssetValue::new(Arc::<[u8]>::from(vec![0; size])).expect("test value under limit")
}

fn write_png(path: &std::path::Path, width: u32, height: u32) {
    let image = image::RgbaImage::from_pixel(width, height, image::Rgba([0, 128, 255, 255]));
    image.save(path).expect("write png");
}

#[test]
fn stale_completion_is_rejected_and_last_good_survives_failure() {
    let mut manager = RenderAssetManager::with_budget(1024);
    let key = key("doc/image.png");
    let first = manager.begin_load(key.clone());
    assert!(manager.complete(&key, first, value(10)).expect("complete"));
    let second = manager.begin_load(key.clone());
    assert_eq!(
        manager.complete(&key, first, value(20)),
        Err(AssetError::StaleGeneration {
            expected: second.generation,
            actual: first.generation
        })
    );
    assert!(manager.fail(&key, second, "network").expect("failure"));
    assert!(matches!(
        manager.entry(&key).map(|entry| &entry.state),
        Some(AssetState::Failed {
            last_good: Some(_),
            ..
        })
    ));
}

#[test]
fn cancelling_refresh_restores_last_good_without_byte_drift() {
    let mut manager = RenderAssetManager::with_budget(1024);
    let key = key("doc/image.png");
    let first = manager.begin_load(key.clone());
    manager
        .complete(&key, first, value(40))
        .expect("initial value");
    let refresh = manager.begin_load(key.clone());
    assert!(manager.cancel(&key, refresh));
    assert_eq!(
        manager.complete(&key, refresh, value(44)),
        Err(AssetError::StaleGeneration {
            expected: refresh.generation + 1,
            actual: refresh.generation,
        })
    );
    assert_eq!(manager.resident_bytes(), 40);
    assert!(matches!(
        manager.entry(&key).map(|entry| &entry.state),
        Some(AssetState::Ready(AssetValue {
            resident_bytes: 40,
            ..
        }))
    ));
}

#[test]
fn lru_budget_evicts_old_ready_values() {
    let mut manager = RenderAssetManager::with_budget(100);
    let first_key = key("first");
    let second_key = key("second");
    let first = manager.begin_load(first_key.clone());
    manager
        .complete(&first_key, first, value(70))
        .expect("first");
    let second = manager.begin_load(second_key.clone());
    manager
        .complete(&second_key, second, value(70))
        .expect("second");
    assert!(manager.entry(&first_key).is_none());
    assert_eq!(manager.resident_bytes(), 70);
}

#[test]
fn budget_is_hard_even_when_one_value_is_larger_than_the_configured_budget() {
    let mut manager = RenderAssetManager::with_budget(32);
    let key = key("oversized-for-budget");
    let token = manager.begin_load(key.clone());
    manager.complete(&key, token, value(40)).expect("value");
    assert!(manager.entry(&key).is_none());
    assert_eq!(manager.resident_bytes(), 0);
}

#[test]
fn dimensions_and_concurrency_are_bounded() {
    assert_eq!(target_pixel_size(100.0, 50.0, 2.0), (200, 100));
    assert_eq!(target_pixel_size(10_000.0, 1.0, 2.0).0, MAX_IMAGE_SIDE);
    assert_eq!(recommended_decode_concurrency(0), 1);
    assert_eq!(recommended_decode_concurrency(32), 4);
}

#[test]
fn corrupt_local_images_fail_before_entering_the_cache() {
    let mut path = std::env::temp_dir();
    path.push(format!("gmark-corrupt-image-{}.bin", std::process::id()));
    let mut file = std::fs::File::create(&path).expect("temp image");
    file.write_all(b"not an image")
        .expect("write corrupt image");
    let result = decode_local_image(&path, (64, 64));
    let _ = std::fs::remove_file(path);
    assert!(matches!(result, Err(AssetError::Decode(_))));
}

#[test]
fn encoded_payload_limit_is_rejected_before_cache_admission() {
    let result = AssetValue::new(Arc::<[u8]>::from(vec![0; MAX_IMAGE_BYTES + 1]));
    assert!(matches!(
        result,
        Err(AssetError::TooLarge {
            bytes,
            limit: MAX_IMAGE_BYTES
        }) if bytes == MAX_IMAGE_BYTES + 1
    ));
}

#[test]
fn oversized_dimensions_are_rejected_before_resizing() {
    let path =
        std::env::temp_dir().join(format!("gmark-oversized-image-{}.png", std::process::id()));
    write_png(&path, MAX_IMAGE_SIDE + 1, 1);
    let result = decode_local_image(&path, (64, 64));
    let _ = std::fs::remove_file(path);
    assert!(matches!(result, Err(AssetError::Decode(_))));
}

#[test]
fn successful_decode_contains_a_renderable_payload() {
    let path = std::env::temp_dir().join(format!("gmark-valid-image-{}.png", std::process::id()));
    write_png(&path, 8, 4);
    let result = decode_local_image(&path, (4, 4)).expect("decode image");
    let _ = std::fs::remove_file(path);
    assert!(result.render_image().is_some());
}

#[test]
fn failed_refresh_can_be_retried_without_losing_last_good() {
    let mut manager = RenderAssetManager::with_budget(1024);
    let key = key("doc/retry.png");
    let first = manager.begin_load(key.clone());
    manager
        .complete(&key, first, value(12))
        .expect("initial value");
    let retry = manager.begin_load(key.clone());
    manager.fail(&key, retry, "decode").expect("failed refresh");
    let retained = manager.state(&key);
    assert!(retained.error_message().is_some());
    assert_eq!(
        retained.last_good().map(|value| value.resident_bytes),
        Some(12)
    );
    let retry_again = manager.begin_load(key.clone());
    assert!(
        manager
            .complete(&key, retry_again, value(16))
            .expect("retry value")
    );
    assert!(matches!(
        manager.state(&key),
        AssetState::Ready(AssetValue {
            resident_bytes: 16,
            ..
        })
    ));
}

#[test]
fn close_document_releases_ready_and_failed_last_good_bytes() {
    let mut manager = RenderAssetManager::with_budget(1024);
    let ready_key = key("doc/close-ready.png");
    let failed_key = key("doc/close-failed.png");
    let ready = manager.begin_load(ready_key.clone());
    manager
        .complete(&ready_key, ready, value(20))
        .expect("ready");
    let failed = manager.begin_load(failed_key.clone());
    manager
        .complete(&failed_key, failed, value(24))
        .expect("failed seed");
    let failed_retry = manager.begin_load(failed_key.clone());
    manager
        .fail(&failed_key, failed_retry, "decode")
        .expect("failure");
    assert_eq!(manager.resident_bytes(), 44);
    manager.close_document("doc/");
    assert_eq!(manager.resident_bytes(), 0);
    assert!(manager.entry(&ready_key).is_none());
    assert!(manager.entry(&failed_key).is_none());
}

#[test]
fn clear_releases_all_payloads_for_application_shutdown() {
    let mut manager = RenderAssetManager::with_budget(1024);
    let key = key("doc/shutdown.png");
    let token = manager.begin_load(key.clone());
    manager.complete(&key, token, value(24)).expect("ready");
    manager.clear();
    assert_eq!(manager.resident_bytes(), 0);
    assert!(manager.entry(&key).is_none());
}

fn write_svg(path: &std::path::Path, body: &str) {
    let mut file = std::fs::File::create(path).expect("temp svg");
    file.write_all(body.as_bytes()).expect("write svg");
}

#[test]
fn svg_images_rasterize_into_a_renderable_payload() {
    let mut path = std::env::temp_dir();
    path.push(format!("gmark-svg-image-{}.svg", std::process::id()));
    write_svg(
        &path,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 8 4\">\
         <rect width=\"8\" height=\"4\" fill=\"#0080ff\"/></svg>",
    );
    let result = decode_local_svg(&path, (8, 4));
    let _ = std::fs::remove_file(path);
    let value = result.expect("svg decodes");
    assert!(value.render_image().is_some());
}

#[test]
fn invalid_svg_fails_with_decode_error() {
    let mut path = std::env::temp_dir();
    path.push(format!("gmark-corrupt-svg-{}.svg", std::process::id()));
    write_svg(&path, "not an svg");
    let result = decode_local_svg(&path, (8, 4));
    let _ = std::fs::remove_file(path);
    assert!(matches!(result, Err(AssetError::Decode(_))));
}
