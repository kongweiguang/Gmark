// @author kongweiguang

//! Bounded, generation-safe render asset state.
//!
//! GPUI image handles and filesystem/network adapters live at the call site;
//! this module owns the lifecycle invariants shared by images, diagrams and
//! other asynchronous render products.  It is intentionally deterministic so
//! cancellation, stale completions and cache eviction can be tested without a
//! window or a real clipboard/network.

use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::io::{Cursor, Read};
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};

use gpui::Global;
use gpui::RenderImage;
use image::Frame;
use smallvec::smallvec;

const MAX_IMAGE_SIDE: u32 = 4096;
const MAX_IMAGE_BYTES: usize = 64 * 1024 * 1024;
const DEFAULT_BUDGET_BYTES: usize = 256 * 1024 * 1024;

/// Resource identity used by the render cache.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct AssetKey {
    /// Canonical path, URL or renderer-specific identity.
    pub(crate) identity: String,
    /// File mtime, ETag, source revision or other content version.
    pub(crate) version: String,
    /// Target pixel dimensions after layout and scale factor.
    pub(crate) pixel_width: u32,
    pub(crate) pixel_height: u32,
}

impl AssetKey {
    pub(crate) fn new(
        identity: impl Into<String>,
        version: impl Into<String>,
        pixel_width: u32,
        pixel_height: u32,
    ) -> Self {
        Self {
            identity: identity.into(),
            version: version.into(),
            pixel_width,
            pixel_height,
        }
    }
}

/// Decoded/ready payload tracked by the bounded cache.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AssetValue {
    /// Opaque bytes or serialized renderer output owned by the adapter.
    pub(crate) bytes: Arc<[u8]>,
    /// Estimated resident bytes used for eviction accounting.
    pub(crate) resident_bytes: usize,
    /// Decoded payload ready for GPUI upload. Keeping this beside the encoded
    /// bytes means a render pass never falls back to an unbounded native path
    /// decode after the manager has accepted a local image.
    pub(crate) render_image: Option<Arc<RenderImage>>,
}

impl AssetValue {
    pub(crate) fn new(bytes: impl Into<Arc<[u8]>>) -> Result<Self, AssetError> {
        let bytes = bytes.into();
        let resident_bytes = bytes.len();
        if resident_bytes > MAX_IMAGE_BYTES {
            return Err(AssetError::TooLarge {
                bytes: resident_bytes,
                limit: MAX_IMAGE_BYTES,
            });
        }
        Ok(Self {
            bytes,
            resident_bytes,
            render_image: None,
        })
    }

    /// Builds a cache value from a bounded PNG payload and prepares the
    /// channel-swizzled image representation expected by GPUI.
    pub(crate) fn from_png(bytes: impl Into<Arc<[u8]>>) -> Result<Self, AssetError> {
        let mut value = Self::new(bytes)?;
        let mut reader = image::ImageReader::new(Cursor::new(value.bytes.as_ref()))
            .with_guessed_format()
            .map_err(|error| AssetError::Decode(error.to_string()))?;
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(MAX_IMAGE_SIDE);
        limits.max_image_height = Some(MAX_IMAGE_SIDE);
        limits.max_alloc = Some(MAX_IMAGE_BYTES as u64);
        reader.limits(limits);
        let decoded = reader
            .decode()
            .map_err(|error| AssetError::Decode(error.to_string()))?
            .into_rgba8();
        let mut decoded = decoded;
        let decoded_bytes = usize::try_from(decoded.width())
            .ok()
            .and_then(|width| {
                usize::try_from(decoded.height())
                    .ok()
                    .map(|height| width.saturating_mul(height))
            })
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or_else(|| AssetError::Decode("image dimensions overflow".to_owned()))?;
        if decoded_bytes > MAX_IMAGE_BYTES {
            return Err(AssetError::TooLarge {
                bytes: decoded_bytes,
                limit: MAX_IMAGE_BYTES,
            });
        }
        // Keep both the encoded fallback bytes and the decoded frame alive:
        // the former is needed for retry/diagnostics and the latter is what
        // GPUI renders. Account for both so the LRU budget is a hard resident
        // memory ceiling instead of only measuring the larger representation.
        value.resident_bytes = value.resident_bytes.saturating_add(decoded_bytes);
        for pixel in decoded.as_flat_samples_mut().samples.chunks_exact_mut(4) {
            // GPUI's RenderImage stores BGRA while image crate exposes RGBA.
            pixel.swap(0, 2);
        }
        value.render_image = Some(Arc::new(RenderImage::new(smallvec![Frame::new(decoded)])));
        Ok(value)
    }

    pub(crate) fn render_image(&self) -> Option<Arc<RenderImage>> {
        self.render_image.clone()
    }
}

/// State visible to renderers while an asset is being prepared.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) enum AssetState {
    /// No request has been scheduled.
    #[default]
    Idle,
    /// A background task owns this generation.
    Loading {
        generation: u64,
        last_good: Option<AssetValue>,
    },
    /// Last successful value.
    Ready(AssetValue),
    /// Request failed. A previous successful value may remain available.
    Failed {
        generation: u64,
        error: String,
        last_good: Option<AssetValue>,
    },
}

impl AssetState {
    /// Returns the payload that should remain visible while loading or after a
    /// failed refresh. This is the single presentation rule used by all image
    /// renderers; a failure never blanks a previously successful frame.
    pub(crate) fn last_good(&self) -> Option<&AssetValue> {
        match self {
            Self::Ready(value) => Some(value),
            Self::Loading { last_good, .. } | Self::Failed { last_good, .. } => last_good.as_ref(),
            Self::Idle => None,
        }
    }

    pub(crate) fn error_message(&self) -> Option<&str> {
        match self {
            Self::Failed { error, .. } => Some(error),
            _ => None,
        }
    }

    pub(crate) fn is_loading(&self) -> bool {
        matches!(self, Self::Loading { .. })
    }
}

/// One cache entry and its stable generation counter.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct AssetEntry {
    pub(crate) state: AssetState,
    generation: u64,
}

/// Completion token returned when a load starts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct AssetLoadToken {
    pub(crate) generation: u64,
}

/// Why a render asset could not be accepted.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum AssetError {
    /// Decoded payload exceeds the per-item resident limit.
    TooLarge { bytes: usize, limit: usize },
    /// Completion belongs to a previous generation.
    StaleGeneration { expected: u64, actual: u64 },
    /// The adapter returned an empty error message.
    EmptyFailure,
    /// Local image bytes could not be decoded or encoded safely.
    Decode(String),
}

impl fmt::Display for AssetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge { bytes, limit } => {
                write!(
                    formatter,
                    "asset payload {bytes} bytes exceeds limit {limit}"
                )
            }
            Self::StaleGeneration { expected, actual } => {
                write!(
                    formatter,
                    "stale asset generation {actual}; expected {expected}"
                )
            }
            Self::EmptyFailure => formatter.write_str("asset provider returned an empty failure"),
            Self::Decode(error) => write!(formatter, "image decode failed: {error}"),
        }
    }
}

/// Decode a local image under strict dimensions/allocation limits and scale it
/// to the requested pixel bucket. The returned PNG bytes and decoded
/// RenderImage are adapter-owned last-good payloads; all expensive/corrupt
/// input is rejected before the GPUI upload boundary.
pub(crate) fn decode_local_image(
    path: &Path,
    target_pixels: (u32, u32),
) -> Result<AssetValue, AssetError> {
    let metadata =
        std::fs::metadata(path).map_err(|error| AssetError::Decode(error.to_string()))?;
    if metadata.len() > MAX_IMAGE_BYTES as u64 {
        return Err(AssetError::TooLarge {
            bytes: usize::try_from(metadata.len()).unwrap_or(usize::MAX),
            limit: MAX_IMAGE_BYTES,
        });
    }
    let file = std::fs::File::open(path).map_err(|error| AssetError::Decode(error.to_string()))?;
    let mut bytes = Vec::with_capacity(
        usize::try_from(metadata.len())
            .unwrap_or(MAX_IMAGE_BYTES)
            .min(MAX_IMAGE_BYTES),
    );
    file.take((MAX_IMAGE_BYTES as u64).saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| AssetError::Decode(error.to_string()))?;
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err(AssetError::TooLarge {
            bytes: bytes.len(),
            limit: MAX_IMAGE_BYTES,
        });
    }
    let mut reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|error| AssetError::Decode(error.to_string()))?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_SIDE);
    limits.max_image_height = Some(MAX_IMAGE_SIDE);
    limits.max_alloc = Some(MAX_IMAGE_BYTES as u64);
    reader.limits(limits);
    let decoded = reader
        .decode()
        .map_err(|error| AssetError::Decode(error.to_string()))?;
    let pixel_bytes = usize::try_from(decoded.width())
        .ok()
        .and_then(|width| {
            usize::try_from(decoded.height())
                .ok()
                .map(|height| width.saturating_mul(height))
        })
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| AssetError::Decode("image dimensions overflow".to_owned()))?;
    if pixel_bytes > MAX_IMAGE_BYTES {
        return Err(AssetError::TooLarge {
            bytes: pixel_bytes,
            limit: MAX_IMAGE_BYTES,
        });
    }
    let target_width = target_pixels.0.clamp(1, MAX_IMAGE_SIDE);
    let target_height = target_pixels.1.clamp(1, MAX_IMAGE_SIDE);
    let resized = if decoded.width() > target_width || decoded.height() > target_height {
        decoded.resize(
            target_width,
            target_height,
            image::imageops::FilterType::Triangle,
        )
    } else {
        decoded
    };
    let mut encoded = Cursor::new(Vec::new());
    resized
        .write_to(&mut encoded, image::ImageFormat::Png)
        .map_err(|error| AssetError::Decode(error.to_string()))?;
    AssetValue::from_png(Arc::<[u8]>::from(encoded.into_inner()))
}

/// Decode a local SVG by rasterizing it with `resvg` into a bounded PNG
/// payload the render cache presents like any other local image. The `image`
/// crate used by [`decode_local_image`] ships no SVG decoder, so without this
/// path markdown-embedded `![](…svg)` images would always fail to render.
pub(crate) fn decode_local_svg(
    path: &Path,
    target_pixels: (u32, u32),
) -> Result<AssetValue, AssetError> {
    use std::sync::LazyLock;

    let metadata =
        std::fs::metadata(path).map_err(|error| AssetError::Decode(error.to_string()))?;
    if metadata.len() > MAX_IMAGE_BYTES as u64 {
        return Err(AssetError::TooLarge {
            bytes: usize::try_from(metadata.len()).unwrap_or(usize::MAX),
            limit: MAX_IMAGE_BYTES,
        });
    }
    let file = std::fs::File::open(path).map_err(|error| AssetError::Decode(error.to_string()))?;
    let mut bytes = Vec::with_capacity(
        usize::try_from(metadata.len())
            .unwrap_or(MAX_IMAGE_BYTES)
            .min(MAX_IMAGE_BYTES),
    );
    file.take((MAX_IMAGE_BYTES as u64).saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| AssetError::Decode(error.to_string()))?;
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err(AssetError::TooLarge {
            bytes: bytes.len(),
            limit: MAX_IMAGE_BYTES,
        });
    }

    static FONT_DB: LazyLock<Arc<resvg::usvg::fontdb::Database>> = LazyLock::new(|| {
        let mut database = resvg::usvg::fontdb::Database::new();
        database.load_system_fonts();
        Arc::new(database)
    });
    let options = resvg::usvg::Options {
        fontdb: Arc::clone(&FONT_DB),
        ..resvg::usvg::Options::default()
    };
    let tree = resvg::usvg::Tree::from_data(&bytes, &options)
        .map_err(|error| AssetError::Decode(error.to_string()))?;
    let size = tree.size();
    let svg_width = size.width();
    let svg_height = size.height();
    if !svg_width.is_finite() || !svg_height.is_finite()
        || svg_width <= 0.0 || svg_height <= 0.0
    {
        return Err(AssetError::Decode(
            "SVG dimensions must be finite and non-zero".to_owned(),
        ));
    }

    // SVGs scale losslessly: render toward the layout's requested pixel size
    // while keeping every edge under the per-side limit and the decoded byte
    // ceiling shared with raster images.
    let long_edge = svg_width.max(svg_height);
    let target_edge = target_pixels.0.max(target_pixels.1).max(1) as f32;
    let target_scale = target_edge / long_edge;
    let edge_scale = MAX_IMAGE_SIDE as f32 / long_edge;
    let pixel_scale = ((MAX_IMAGE_BYTES / 4) as f32 / (svg_width * svg_height)).sqrt();
    let scale = target_scale.min(edge_scale).min(pixel_scale);
    if !scale.is_finite() || scale <= 0.0 {
        return Err(AssetError::Decode(
            "SVG dimensions exceed the bounded preview budget".to_owned(),
        ));
    }
    let width = ((svg_width * scale).round() as u32).max(1).min(MAX_IMAGE_SIDE);
    let height = ((svg_height * scale).round() as u32).max(1).min(MAX_IMAGE_SIDE);

    let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height)
        .ok_or_else(|| {
            AssetError::Decode("SVG raster dimensions exceed the renderer limit".to_owned())
        })?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    let mut raster = image::RgbaImage::from_raw(width, height, pixmap.take())
        .ok_or_else(|| {
            AssetError::Decode("SVG renderer returned an invalid pixel buffer".to_owned())
        })?;
    // tiny-skia writes premultiplied RGBA; PNG and the `image` crate expect
    // straight alpha, so un-premultiply before encoding.
    for pixel in raster.as_flat_samples_mut().samples.chunks_exact_mut(4) {
        if pixel[3] > 0 {
            let alpha = f32::from(pixel[3]) / 255.0;
            for channel in &mut pixel[..3] {
                *channel = (f32::from(*channel) / alpha).min(255.0) as u8;
            }
        }
    }
    let mut encoded = Cursor::new(Vec::new());
    raster
        .write_to(&mut encoded, image::ImageFormat::Png)
        .map_err(|error| AssetError::Decode(error.to_string()))?;
    AssetValue::from_png(Arc::<[u8]>::from(encoded.into_inner()))
}

impl std::error::Error for AssetError {}

/// Bounded LRU manager shared by render providers.
#[derive(Debug)]
pub(crate) struct RenderAssetManager {
    entries: HashMap<AssetKey, AssetEntry>,
    lru: VecDeque<AssetKey>,
    resident_bytes: usize,
    budget_bytes: usize,
    next_generation: u64,
}

/// Process-wide render-asset cache handle.
///
/// The cache itself is deliberately UI-thread-owned by its callers, but
/// keeping the state behind a shared mutex allows separate editor windows to
/// reuse the same bounded budget and generation rules.  The encoded/decoded
/// payloads are still released explicitly when a document closes or the
/// application drops the global.
#[derive(Clone, Debug)]
pub(crate) struct SharedRenderAssetManager {
    inner: Arc<Mutex<RenderAssetManager>>,
}

impl Global for SharedRenderAssetManager {}

impl Default for SharedRenderAssetManager {
    fn default() -> Self {
        Self {
            inner: Arc::new(Mutex::new(RenderAssetManager::default())),
        }
    }
}

impl SharedRenderAssetManager {
    fn lock(&self) -> MutexGuard<'_, RenderAssetManager> {
        match self.inner.lock() {
            Ok(guard) => guard,
            // A poisoned cache must not take down a document window.  The
            // cache is disposable presentation state, so recovering the
            // existing values is the safest failure mode.
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    pub(crate) fn entry(&self, key: &AssetKey) -> Option<AssetEntry> {
        self.lock().entry(key).cloned()
    }

    pub(crate) fn state(&self, key: &AssetKey) -> AssetState {
        self.lock().state(key)
    }

    pub(crate) fn begin_load(&self, key: AssetKey) -> AssetLoadToken {
        self.lock().begin_load(key)
    }

    pub(crate) fn complete(
        &self,
        key: &AssetKey,
        token: AssetLoadToken,
        value: AssetValue,
    ) -> Result<bool, AssetError> {
        self.lock().complete(key, token, value)
    }

    pub(crate) fn fail(
        &self,
        key: &AssetKey,
        token: AssetLoadToken,
        error: impl Into<String>,
    ) -> Result<bool, AssetError> {
        self.lock().fail(key, token, error)
    }

    pub(crate) fn cancel(&self, key: &AssetKey, token: AssetLoadToken) -> bool {
        self.lock().cancel(key, token)
    }

    pub(crate) fn close_document(&self, identity_prefix: &str) {
        self.lock().close_document(identity_prefix);
    }

    #[expect(
        dead_code,
        reason = "the application shutdown hook may explicitly clear the global cache"
    )]
    pub(crate) fn clear(&self) {
        self.lock().clear();
    }
}

impl Default for RenderAssetManager {
    fn default() -> Self {
        Self::with_budget(DEFAULT_BUDGET_BYTES)
    }
}

impl RenderAssetManager {
    pub(crate) fn with_budget(budget_bytes: usize) -> Self {
        Self {
            entries: HashMap::new(),
            lru: VecDeque::new(),
            resident_bytes: 0,
            budget_bytes: budget_bytes.max(1),
            next_generation: 0,
        }
    }

    // Reason: budget telemetry is staged for render diagnostics. Remove when that surface consumes it.
    #[allow(dead_code)]
    pub(crate) fn budget_bytes(&self) -> usize {
        self.budget_bytes
    }

    // Reason: residency telemetry is staged for render diagnostics. Remove when that surface consumes it.
    #[allow(dead_code)]
    pub(crate) fn resident_bytes(&self) -> usize {
        self.resident_bytes
    }

    pub(crate) fn entry(&self, key: &AssetKey) -> Option<&AssetEntry> {
        self.entries.get(key)
    }

    /// Clones the presentation state so an editor can project it into a block
    /// without exposing mutable cache internals to the renderer.
    pub(crate) fn state(&self, key: &AssetKey) -> AssetState {
        self.entries
            .get(key)
            .map(|entry| entry.state.clone())
            .unwrap_or_default()
    }

    /// Starts (or restarts) a request and returns its generation token.
    pub(crate) fn begin_load(&mut self, key: AssetKey) -> AssetLoadToken {
        self.next_generation = self.next_generation.wrapping_add(1).max(1);
        let generation = self.next_generation;
        let entry = self.entries.entry(key.clone()).or_default();
        let last_good = match &entry.state {
            AssetState::Ready(value) => Some(value.clone()),
            AssetState::Failed { last_good, .. } | AssetState::Loading { last_good, .. } => {
                last_good.clone()
            }
            AssetState::Idle => None,
        };
        entry.generation = generation;
        entry.state = AssetState::Loading {
            generation,
            last_good,
        };
        self.touch(&key);
        AssetLoadToken { generation }
    }

    /// Publishes a successful completion if its generation is still current.
    pub(crate) fn complete(
        &mut self,
        key: &AssetKey,
        token: AssetLoadToken,
        value: AssetValue,
    ) -> Result<bool, AssetError> {
        let Some(entry) = self.entries.get_mut(key) else {
            return Ok(false);
        };
        if entry.generation != token.generation {
            return Err(AssetError::StaleGeneration {
                expected: entry.generation,
                actual: token.generation,
            });
        }
        let previous = match &entry.state {
            AssetState::Ready(value) => Some(value.resident_bytes),
            AssetState::Failed { last_good, .. } => {
                last_good.as_ref().map(|value| value.resident_bytes)
            }
            AssetState::Idle => None,
            AssetState::Loading { last_good, .. } => {
                last_good.as_ref().map(|value| value.resident_bytes)
            }
        };
        self.resident_bytes = self.resident_bytes.saturating_sub(previous.unwrap_or(0));
        self.resident_bytes = self.resident_bytes.saturating_add(value.resident_bytes);
        entry.state = AssetState::Ready(value);
        self.touch(key);
        self.evict_if_needed(Some(key));
        Ok(true)
    }

    /// Publishes a failure while retaining the last-good value for visual
    /// continuity and explicit retry.
    pub(crate) fn fail(
        &mut self,
        key: &AssetKey,
        token: AssetLoadToken,
        error: impl Into<String>,
    ) -> Result<bool, AssetError> {
        let error = error.into();
        if error.trim().is_empty() {
            return Err(AssetError::EmptyFailure);
        }
        let Some(entry) = self.entries.get_mut(key) else {
            return Ok(false);
        };
        if entry.generation != token.generation {
            return Err(AssetError::StaleGeneration {
                expected: entry.generation,
                actual: token.generation,
            });
        }
        let last_good = match &entry.state {
            AssetState::Ready(value) => Some(value.clone()),
            AssetState::Failed { last_good, .. } => last_good.clone(),
            AssetState::Idle => None,
            AssetState::Loading { last_good, .. } => last_good.clone(),
        };
        entry.state = AssetState::Failed {
            generation: token.generation,
            error,
            last_good,
        };
        self.touch(key);
        Ok(true)
    }

    /// Cancels a request before its task handle is dropped. A stale
    /// completion can no longer publish, while a last-good frame remains
    /// available to the renderer until the entry is explicitly released.
    pub(crate) fn cancel(&mut self, key: &AssetKey, token: AssetLoadToken) -> bool {
        let Some(entry) = self.entries.get_mut(key) else {
            return false;
        };
        if entry.generation != token.generation {
            return false;
        }
        entry.generation = entry.generation.wrapping_add(1).max(1);
        if let AssetState::Loading { last_good, .. } = &entry.state {
            // A cancelled refresh must keep the last-good frame visible. The
            // previous implementation dropped it from the state while leaving
            // its bytes counted in the budget, causing resident-byte drift and
            // eventual premature LRU eviction.
            entry.state = last_good
                .clone()
                .map(AssetState::Ready)
                .unwrap_or(AssetState::Idle);
        }
        self.touch(key);
        true
    }

    /// Drops entries whose identity belongs to a closed document.
    pub(crate) fn close_document(&mut self, identity_prefix: &str) {
        let keys = self
            .entries
            .keys()
            .filter(|key| key.identity.starts_with(identity_prefix))
            .cloned()
            .collect::<Vec<_>>();
        for key in keys {
            self.remove(&key);
        }
    }

    /// Releases every decoded payload. Editor teardown normally drops the
    /// manager, but an explicit hook makes application shutdown and tests
    /// deterministic even when the surrounding entity remains temporarily
    /// alive while GPUI tasks drain.
    // Reason: the shared shutdown hook and lifecycle tests use explicit cache clearing. Remove when application teardown owns the inner manager directly.
    #[allow(dead_code)]
    pub(crate) fn clear(&mut self) {
        self.entries.clear();
        self.lru.clear();
        self.resident_bytes = 0;
    }

    fn touch(&mut self, key: &AssetKey) {
        self.lru.retain(|candidate| candidate != key);
        self.lru.push_back(key.clone());
    }

    fn remove(&mut self, key: &AssetKey) {
        if let Some(entry) = self.entries.remove(key) {
            self.resident_bytes = self
                .resident_bytes
                .saturating_sub(state_bytes(&entry.state));
        }
        self.lru.retain(|candidate| candidate != key);
    }

    fn evict_if_needed(&mut self, protected: Option<&AssetKey>) {
        while self.resident_bytes > self.budget_bytes {
            let Some(candidate_index) = self
                .lru
                .iter()
                .position(|candidate| !protected.is_some_and(|key| key == candidate))
                .or_else(|| (!self.lru.is_empty()).then_some(0))
            else {
                break;
            };
            let Some(candidate) = self.lru.remove(candidate_index) else {
                break;
            };
            self.remove(&candidate);
        }
    }
}

fn state_bytes(state: &AssetState) -> usize {
    match state {
        AssetState::Ready(value) => value.resident_bytes,
        AssetState::Failed { last_good, .. } => {
            last_good.as_ref().map_or(0, |value| value.resident_bytes)
        }
        AssetState::Loading { last_good, .. } => {
            last_good.as_ref().map_or(0, |value| value.resident_bytes)
        }
        AssetState::Idle => 0,
    }
}

/// Computes the bounded local-image decode dimensions.
pub(crate) fn target_pixel_size(
    logical_width: f32,
    logical_height: f32,
    scale_factor: f32,
) -> (u32, u32) {
    let width = (logical_width.max(1.0) * scale_factor.max(0.1)).ceil() as u32;
    let height = (logical_height.max(1.0) * scale_factor.max(0.1)).ceil() as u32;
    let scale = (MAX_IMAGE_SIDE as f32 / width.max(height) as f32).min(1.0);
    (
        ((width as f32 * scale).round() as u32).clamp(1, MAX_IMAGE_SIDE),
        ((height as f32 * scale).round() as u32).clamp(1, MAX_IMAGE_SIDE),
    )
}

/// Recommended worker count for local image decode.
// Reason: worker sizing is staged for the render scheduler. Remove when the scheduler consumes it.
#[allow(dead_code)]
pub(crate) fn recommended_decode_concurrency(available_parallelism: usize) -> usize {
    (available_parallelism.max(1) / 2).clamp(1, 4)
}

#[cfg(test)]
#[path = "../../tests/unit/editor/render_asset_manager.rs"]
mod tests;
