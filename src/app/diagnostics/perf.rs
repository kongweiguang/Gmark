// @author kongweiguang

//! 可选的编辑器性能事件采样。
//!
//! 默认关闭；设置 `GMARK_PERF_TRACE=1` 后向 stderr 输出一行一个 JSON 记录。
//! 输入响应会分别记录到 Editor render 与目标文本元素 paint；二者都不代表平台 present。

use std::cell::{Cell, RefCell};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use gmark_document::Revision;
use gpui::EntityId;
use serde::Serialize;

use gmark_document_core::{DocumentBackendKind, DocumentFormat, OpenPlan, OpenReason};

static ENABLED: OnceLock<bool> = OnceLock::new();
static SEQUENCE: AtomicU64 = AtomicU64::new(1);
static INPUT_PAINT_GENERATION: AtomicU64 = AtomicU64::new(1);
const INPUT_PAINT_KIND_COUNT: usize = 4;

thread_local! {
    /// GPUI 输入与 Entity 事件在同一 UI 线程传递；只保留一轮批量编辑最早的起点。
    static INPUT_MUTATION_STARTED: Cell<Option<Instant>> = const { Cell::new(None) };
    /// GPUI 会合并高频鼠标和候选更新；每帧每类只保留最近一次，避免无界排队。
    static INPUT_TO_PAINT: RefCell<[Option<PendingPaintTrace>; INPUT_PAINT_KIND_COUNT]> =
        const { RefCell::new([None; INPUT_PAINT_KIND_COUNT]) };
}

#[derive(Serialize)]
struct TraceRecord<'a> {
    schema_version: u8,
    sequence: u64,
    unix_time_ms: u128,
    event: &'a str,
    elapsed_us: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_bytes: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    success: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    document_format: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    document_backend: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    open_reason: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    value: Option<u64>,
}

#[derive(Clone, Copy)]
pub(crate) struct PendingInputTrace {
    started: Instant,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum InputPaintKind {
    Typing,
    SelectionDrag,
    ImePreedit,
    ImeCommit,
}

impl InputPaintKind {
    /// 固定槽位避免热路径构造 map；枚举新增值时须同步调整缓冲区长度。
    const fn slot(self) -> usize {
        match self {
            Self::Typing => 0,
            Self::SelectionDrag => 1,
            Self::ImePreedit => 2,
            Self::ImeCommit => 3,
        }
    }

    /// 事件名保持低基数，便于离线聚合 p50/p95 且不携带文档内容。
    const fn event_name(self) -> &'static str {
        match self {
            Self::Typing => "typing_to_gpui_text_paint",
            Self::SelectionDrag => "selection_drag_to_gpui_text_paint",
            Self::ImePreedit => "ime_preedit_to_gpui_text_paint",
            Self::ImeCommit => "ime_commit_to_gpui_text_paint",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum InputPaintSurface {
    BlockText,
    CodeLanguage,
    MathSource,
}

/// Identifies a rendered input state without retaining document text or view metadata.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct InputPaintSnapshot {
    pub(crate) surface: InputPaintSurface,
    pub(crate) revision: Revision,
    pub(crate) selection_start: usize,
    pub(crate) selection_end: usize,
    pub(crate) selection_reversed: bool,
    pub(crate) editor_selection: Option<(usize, usize)>,
    pub(crate) composition_generation: Option<u64>,
}

#[derive(Clone, Copy)]
pub(crate) struct InputPaintStart {
    started: Instant,
}

#[derive(Clone, Copy)]
pub(crate) struct PendingPaintTrace {
    kind: InputPaintKind,
    target: EntityId,
    snapshot: InputPaintSnapshot,
    started: Instant,
}

pub(crate) fn env_value_enables_trace(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

fn enabled() -> bool {
    *ENABLED.get_or_init(|| {
        std::env::var("GMARK_PERF_TRACE")
            .ok()
            .is_some_and(|value| env_value_enables_trace(&value))
    })
}

pub(crate) fn start() -> Option<Instant> {
    enabled().then(Instant::now)
}

pub(crate) fn begin_input_mutation() {
    if !enabled() {
        return;
    }
    INPUT_MUTATION_STARTED.with(|started| {
        if started.get().is_none() {
            started.set(Some(Instant::now()));
        }
    });
}

pub(crate) fn take_input_mutation() -> Option<PendingInputTrace> {
    if !enabled() {
        return None;
    }
    INPUT_MUTATION_STARTED.with(|started| {
        started
            .replace(None)
            .map(|started| PendingInputTrace { started })
    })
}

/// Captures the start time before an input handler changes its local model.
pub(crate) fn start_input_to_gpui_paint() -> Option<InputPaintStart> {
    enabled().then(|| InputPaintStart {
        started: Instant::now(),
    })
}

/// 从真实输入事件开始计时；元素 `paint` 回调负责取出并记录 GPUI 绘制边界。
pub(crate) fn begin_input_to_gpui_paint(
    kind: InputPaintKind,
    target: EntityId,
    snapshot: InputPaintSnapshot,
) {
    if !enabled() {
        return;
    }
    INPUT_TO_PAINT.with(|traces| {
        if let Ok(mut traces) = traces.try_borrow_mut() {
            store_input_to_gpui_paint(&mut traces, kind, target, snapshot, Instant::now());
        }
    });
}

/// Finishes a timed mutation with its post-edit identity so an old caret cannot validate it.
pub(crate) fn finish_input_to_gpui_paint(
    started: Option<InputPaintStart>,
    kind: InputPaintKind,
    target: EntityId,
    snapshot: InputPaintSnapshot,
) {
    let Some(started) = started else {
        return;
    };
    if !enabled() {
        return;
    }
    INPUT_TO_PAINT.with(|traces| {
        if let Ok(mut traces) = traces.try_borrow_mut() {
            store_input_to_gpui_paint(&mut traces, kind, target, snapshot, started.started);
        }
    });
}

/// Allocates a per-composition generation only while tracing, keeping candidate samples distinct.
pub(crate) fn next_input_paint_generation() -> Option<u64> {
    enabled().then(|| INPUT_PAINT_GENERATION.fetch_add(1, Ordering::Relaxed))
}

/// Drops deferred IME samples when their pinned owner reaches a terminal state.
pub(crate) fn invalidate_ime_input_to_gpui_paint(target: EntityId, surface: InputPaintSurface) {
    if !enabled() {
        return;
    }
    INPUT_TO_PAINT.with(|traces| {
        let Ok(mut pending) = traces.try_borrow_mut() else {
            return;
        };
        invalidate_ime_pending_for_surface(&mut pending, target, surface);
    });
}

/// Takes matching input states at their actual text paint and discards stale states on that surface.
pub(crate) fn take_input_to_gpui_paint(
    target: EntityId,
    snapshot: InputPaintSnapshot,
) -> [Option<PendingPaintTrace>; INPUT_PAINT_KIND_COUNT] {
    if !enabled() {
        return [None; INPUT_PAINT_KIND_COUNT];
    }
    INPUT_TO_PAINT.with(|traces| {
        let Ok(mut pending) = traces.try_borrow_mut() else {
            return [None; INPUT_PAINT_KIND_COUNT];
        };
        take_input_to_gpui_paint_for_target(&mut pending, target, snapshot)
    })
}

/// Keeps one pending sample per interaction class when GPUI coalesces input updates.
fn store_input_to_gpui_paint(
    pending: &mut [Option<PendingPaintTrace>; INPUT_PAINT_KIND_COUNT],
    kind: InputPaintKind,
    target: EntityId,
    snapshot: InputPaintSnapshot,
    started: Instant,
) {
    pending[kind.slot()] = Some(PendingPaintTrace {
        kind,
        target,
        snapshot,
        started,
    });
}

/// Removes only composition-owned samples for one block surface, leaving typing and drag intact.
fn invalidate_ime_pending_for_surface(
    pending: &mut [Option<PendingPaintTrace>; INPUT_PAINT_KIND_COUNT],
    target: EntityId,
    surface: InputPaintSurface,
) {
    for kind in [InputPaintKind::ImePreedit, InputPaintKind::ImeCommit] {
        let slot = kind.slot();
        if pending[slot]
            .is_some_and(|trace| trace.target == target && trace.snapshot.surface == surface)
        {
            pending[slot] = None;
        }
    }
}

/// Leaves other targets/surfaces queued, consumes valid states, and removes stale states in place.
fn take_input_to_gpui_paint_for_target(
    pending: &mut [Option<PendingPaintTrace>; INPUT_PAINT_KIND_COUNT],
    target: EntityId,
    snapshot: InputPaintSnapshot,
) -> [Option<PendingPaintTrace>; INPUT_PAINT_KIND_COUNT] {
    let mut ready = [None; INPUT_PAINT_KIND_COUNT];
    for (slot, trace) in pending.iter_mut().enumerate() {
        let Some(sample) = trace.as_ref() else {
            continue;
        };
        if sample.target != target || sample.snapshot.surface != snapshot.surface {
            continue;
        }
        let Some(sample) = trace.take() else {
            continue;
        };
        if sample.snapshot.matches(sample.kind, snapshot) {
            ready[slot] = Some(sample);
        }
    }
    ready
}

impl InputPaintSnapshot {
    /// Allows one source commit after local text input; candidates still require an exact composition generation.
    fn matches(self, kind: InputPaintKind, current: Self) -> bool {
        let revision_matches = self.revision == current.revision
            || (matches!(kind, InputPaintKind::Typing | InputPaintKind::ImeCommit)
                && self
                    .revision
                    .get()
                    .checked_add(1)
                    .is_some_and(|revision| current.revision == Revision::from_u64(revision)));
        self.surface == current.surface
            && revision_matches
            && self.selection_start == current.selection_start
            && self.selection_end == current.selection_end
            && self.selection_reversed == current.selection_reversed
            && self.editor_selection == current.editor_selection
            && (!matches!(kind, InputPaintKind::ImePreedit | InputPaintKind::ImeCommit)
                || self.composition_generation == current.composition_generation)
    }
}

impl PendingInputTrace {
    pub(crate) fn record_dirty_sync(self, source_bytes: usize) {
        emit(
            "input_to_dirty_sync",
            self.started,
            Some(source_bytes),
            None,
            None,
        );
    }

    /// 保留旧的 Editor::render 指标，与文本元素 paint 延迟分开统计。
    pub(crate) fn record_next_render(self, source_bytes: usize) {
        emit(
            "input_to_next_render",
            self.started,
            Some(source_bytes),
            None,
            Some("Editor::render boundary; not Element::paint or platform present"),
        );
    }
}

impl PendingPaintTrace {
    /// 记录文本元素已进入 GPUI paint；这仍不证明操作系统已呈现该帧。
    pub(crate) fn record_gpui_text_paint(self) {
        emit(
            self.kind.event_name(),
            self.started,
            None,
            None,
            Some("GPUI text element paint boundary; not platform present"),
        );
    }
}

pub(crate) fn emit(
    event: &'static str,
    started: Instant,
    source_bytes: Option<usize>,
    success: Option<bool>,
    detail: Option<&str>,
) {
    if !enabled() {
        return;
    }
    let elapsed_us = started.elapsed().as_micros().min(u64::MAX as u128) as u64;
    let unix_time_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis());
    let record = TraceRecord {
        schema_version: 1,
        sequence: SEQUENCE.fetch_add(1, Ordering::Relaxed),
        unix_time_ms,
        event,
        elapsed_us,
        source_bytes,
        success,
        detail,
        document_format: None,
        document_backend: None,
        open_reason: None,
        value: None,
    };
    emit_record(&record);
}

/// 记录文档生命周期边界。字段只接受封闭领域枚举和数值，禁止正文、路径或错误文本
/// 进入本地诊断；这样即使用户显式开启 trace，也不会泄露文档内容与文件位置。
pub(crate) fn emit_document(
    event: &'static str,
    started: Instant,
    source_bytes: Option<usize>,
    success: Option<bool>,
    format: &DocumentFormat,
    plan: &OpenPlan,
    detail: Option<&'static str>,
) {
    if !enabled() {
        return;
    }
    let record = TraceRecord {
        schema_version: 1,
        sequence: SEQUENCE.fetch_add(1, Ordering::Relaxed),
        unix_time_ms: unix_time_ms(),
        event,
        elapsed_us: started.elapsed().as_micros().min(u64::MAX as u128) as u64,
        source_bytes,
        success,
        detail,
        document_format: Some(document_format_name(format)),
        document_backend: Some(document_backend_name(plan.backend)),
        open_reason: Some(open_reason_name(plan.reason)),
        value: None,
    };
    emit_record(&record);
}

/// 记录有界缓存和取消计数。调用方只在新峰值或离散状态变化时调用，避免每帧刷日志。
pub(crate) fn emit_document_value(
    event: &'static str,
    value: u64,
    format: &DocumentFormat,
    plan: &OpenPlan,
) {
    if !enabled() {
        return;
    }
    let record = TraceRecord {
        schema_version: 1,
        sequence: SEQUENCE.fetch_add(1, Ordering::Relaxed),
        unix_time_ms: unix_time_ms(),
        event,
        elapsed_us: 0,
        source_bytes: None,
        success: None,
        detail: None,
        document_format: Some(document_format_name(format)),
        document_backend: Some(document_backend_name(plan.backend)),
        open_reason: Some(open_reason_name(plan.reason)),
        value: Some(value),
    };
    emit_record(&record);
}

fn unix_time_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis())
}

fn document_format_name(format: &DocumentFormat) -> &'static str {
    match format {
        DocumentFormat::PlainText => "plain_text",
        DocumentFormat::Markdown => "markdown",
        DocumentFormat::Json => "json",
        DocumentFormat::JsonLines => "json_lines",
        DocumentFormat::Delimited { delimiter: b'\t' } => "tsv",
        DocumentFormat::Delimited { .. } => "csv",
    }
}

fn document_backend_name(backend: DocumentBackendKind) -> &'static str {
    match backend {
        DocumentBackendKind::Resident => "resident",
        DocumentBackendKind::Paged => "paged",
    }
}

fn open_reason_name(reason: OpenReason) -> &'static str {
    match reason {
        OpenReason::WithinResidentLimits => "within_resident_limits",
        OpenReason::ForcedSafeSource => "forced_safe_source",
        OpenReason::ByteLimitExceeded => "byte_limit_exceeded",
    }
}

fn emit_record(record: &TraceRecord<'_>) {
    if let Ok(json) = serde_json::to_string(&record) {
        eprintln!("gmark_perf {json}");
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/app/perf_input.rs"]
mod tests;
