// @author kongweiguang

//! Top-level editor controller and window state.
//!
//! [`Editor`] owns window-level concerns such as view mode, save/close flow,
//! scroll state, and focus deferral. The runtime block tree itself lives in
//! [`DocumentTree`], which centralizes structural mutations and cached visible
//! order metadata.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures::StreamExt;
use gmark_document::{Revision, SourceDocument};
use gmark_document_core::SourceSelection;
use gmark_paged_document::SourceAffinity;
use gpui::*;

use self::context_menu::{ContextMenuState, TableInsertDialogState};
use self::tree::DocumentTree;
#[path = "mod_parts/editor_state.rs"]
mod editor_state;
use crate::components::{
    Block, BlockKind, BlockRecord, FootnoteDefinitionBinding, FootnoteReferenceLocation,
    FootnoteRegistry, FootnoteResolvedOccurrence, ImageReferenceDefinitions, InlineTextTree,
    LinkReferenceDefinitions, parse_image_reference_definitions, parse_link_reference_definitions,
};
use crate::components::{
    TableAxisHighlight, TableAxisKind, TableAxisMarker, TableCellPosition, TableColumnAlignment,
    TableData, TableRuntime, UndoCaptureKind, serialize_table_cell_markdown,
};
use editor_state::*;
mod close;
mod command_palette;
mod context_menu;
mod diagram_overlay;
pub(crate) mod document;
mod document_session;
mod editing_navigation;
mod editing_navigation_virtual;
mod encoding;
mod events;
mod export;
mod file_drop;
mod find_replace;
mod focus_modes;
mod format_menu;
mod history;
mod image_preview;
mod ime_lifecycle;
mod ime_rebase;
mod link_completion;
mod tool_ime;
pub(crate) use crate::perf;
mod markdown_render_state;
pub(crate) mod markdown_view_state;
pub(crate) mod math_edit;
pub(crate) mod panes;
mod persistence;
mod projection;
mod recovery;
pub(crate) mod render;
pub(crate) mod render_asset_manager;
mod runtime_context;
mod selection;
mod selection_surface;
mod selection_virtualization;
pub(crate) mod services;
mod source_format;
mod source_mapping;
mod source_tools;
mod status_bar;
mod svg_preview;
mod svg_raster;
mod system_file;
mod table_edit;
mod table_fragment;
mod table_selection;
mod tabs;
pub(crate) use tabs::DetachedTab;
#[cfg(all(test, target_os = "windows"))]
#[path = "../../tests/unit/editor/ime_tool_focus.rs"]
mod ime_tool_focus_tests;
#[cfg(test)]
#[path = "../../tests/unit/editor/scenarios.rs"]
mod tests;
mod tree;
mod update;
mod virtual_surface;
mod window_state;
pub(crate) mod workspace;
mod workspace_file_ops;

use self::document_session::EditorDocumentSession;
pub(crate) use self::format_menu::DocumentMenuFormat;
use self::projection::PreparedSplitProjection;
use self::status_bar::StatusBarState;
use self::virtual_surface::VirtualSurfaceState;
use self::workspace::WorkspaceState;

/// Top-level controller that owns editor-wide state and delegates tree
/// mutations to [`DocumentTree`].
///
/// The editor subscribes to every [`BlockEvent`](crate::components::BlockEvent)
/// emitted by child blocks. Structural changes are handled centrally so focus,
/// scrolling, dirty tracking, and serialization stay synchronized.
pub struct Editor {
    accessibility_bridge: Option<crate::accessibility::AccessibilityBridge>,
    accessibility_wake_task: Option<Task<()>>,
    accessibility_revision: Option<u64>,
    /// 非 Markdown 格式与 Paged Source 的唯一应用视图宿主；Resident Markdown 由本 Editor 控制。
    document_host: Option<Entity<crate::document_host::DocumentHost>>,
    /// Markdown 源文本真值；块树只负责当前视图的可重建投影。
    source_document: EditorDocumentSession,
    /// Service-backed views share a process-wide Controller; their filesystem
    /// watcher is owned by DocumentService rather than duplicated per Editor.
    shared_document: bool,
    /// Lightweight wake-up loop for shared Controller events. It is cancelled
    /// whenever this Editor is detached from an active tab.
    shared_event_task: Option<Task<()>>,
    /// Pane child editors render only the document canvas. The window shell
    /// remains owned by the root Editor entity.
    pane_canvas: bool,
    /// Only the workspace's active pane may publish a pending block focus.
    /// Inactive sibling projections still render, but cannot steal the shared
    /// window input handler while they resynchronize.
    pane_canvas_focus_enabled: bool,
    /// 子窗格的实际内容尺寸供 Split 列宽和拖动使用，不能用整个窗口代替。
    pane_canvas_viewport: Option<Size<Pixels>>,
    /// Durable pane tab identity used to key Markdown presentation snapshots.
    /// The shell's legacy tab id is intentionally not reused for child views.
    pane_tab_id: Option<uuid::Uuid>,
    /// Body-free persisted navigation metadata retained while a pane canvas
    /// is inactive; live undo entries are appended when the child is mounted.
    pane_history_back: Vec<serde_json::Value>,
    pane_history_forward: Vec<serde_json::Value>,
    /// Source-backed host metadata retained by the shell while its active
    /// lease is moved into a pane-owned detached view.
    pane_host_path: Option<PathBuf>,
    pane_host_probe: Option<gmark_paged_document::OpenProbe>,
    /// 非 UTF-8 文件只读打开；用户明确转换后才允许编辑或保存。
    source_encoding: crate::document_io::DocumentEncoding,
    /// 无路径文档也必须保留显式类型；不能再用 `file_path == None` 偷换成 Markdown。
    document_kind: DocumentKind,
    /// 每次整文档替换递增；后台保存完成时用它拒绝回写到另一份文档。
    document_epoch: u64,
    /// Stable per-editor scope used to isolate shared render-cache entries
    /// belonging to different windows with the same document epoch.
    render_asset_scope: uuid::Uuid,
    /// Live、Preview 与 Split 共用的最近一次纯语义投影，可落后于正在编辑的源码 revision。
    projection_cache: Option<Arc<PreparedSplitProjection>>,
    source_spelling_cache: std::cell::RefCell<Option<source_mapping::SourceSpellingMap>>,
    document: DocumentTree,
    split_preview: Option<SplitPreviewState>,
    /// Split 分隔比例属于窗口布局状态；拖动期间不得进入文档事务或投影 revision。
    split_pane_ratio: f32,
    split_resize_session: Option<SplitResizeSession>,
    split_divider_focus_handle: FocusHandle,
    /// 主画布工具栏按钮跨 render 保持焦点身份，避免文档投影刷新打断键盘导航。
    document_toolbar_focus_handles: [FocusHandle; 4],
    /// 图片缩放工具条跨异步分片加载保持稳定的键盘焦点顺序。
    image_preview_focus_handles: [FocusHandle; 4],
    /// Stable tile identities used to release standalone image-preview assets
    /// when the active tab changes. Pixel buffers remain owned by GPUI.
    image_preview_tile_ids: Vec<u64>,
    file_open_failure_focus_handles: [FocusHandle; 2],
    /// 更新面板是应用级浮层，但焦点身份属于当前窗口，跨进度刷新必须保持稳定。
    update_primary_focus_handle: FocusHandle,
    update_secondary_focus_handle: FocusHandle,
    table_cells: HashMap<EntityId, TableCellBinding>,
    /// Shared bounded lifecycle for image/diagram render assets.  The model
    /// is independent from GPUI handles so async completions can be rejected
    /// before touching the live block tree.
    render_assets: render_asset_manager::SharedRenderAssetManager,
    /// Background local-image preflight/decode tasks keyed by the exact asset
    /// identity. Dropping a task cancels its completion path on tab/document
    /// replacement.
    render_asset_tasks:
        HashMap<render_asset_manager::AssetKey, (Task<()>, render_asset_manager::AssetLoadToken)>,
    /// Process-local collapse/column-width snapshots for rendered Markdown.
    view_state: markdown_view_state::SharedMarkdownViewState,
    /// Which view the editor is currently presenting.
    pub(crate) view_mode: ViewMode,
    /// Deferred focus target applied during render when a [`Window`] is
    /// available.
    pending_focus: Option<EntityId>,
    active_entity_id: Option<EntityId>,
    pending_scroll_active_block_into_view: bool,
    pending_scroll_recheck_after_layout: bool,
    pending_save: bool,
    pending_save_as: bool,
    /// 系统组合输入终态到来之前保留用户动作，不推定候选已提交。
    pending_ime_operations: std::collections::VecDeque<ime_lifecycle::DeferredImeOperation>,
    ime_completion_requested: bool,
    ime_completion_failed: bool,
    ime_detached_targets: Vec<Entity<Block>>,
    pending_resource_insertion: Option<PendingResourceInsertion>,
    /// 已有路径保存的后台任务；同时只允许一个 writer，后续请求合并到完成后的下一帧。
    save_task: Option<Task<()>>,
    /// 路径对话框到写入结束保持原标签，防止回调绑定到后来的文档。
    save_prompt_task: Option<Task<()>>,
    save_queued: bool,
    /// 每次编辑替换 Task 即重置 idle 计时；关闭设置或保存成功会取消。
    auto_save_task: Option<Task<()>>,
    /// 活动块检查按输入 debounce；后台结果以 display text 快照拒绝过期回写。
    spellcheck_task: Option<Task<()>>,
    export_task: Option<Task<()>>,
    export_cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
    export_progress: Option<Arc<export::ExportProgress>>,
    export_in_progress: bool,
    export_cancel_requested: bool,
    pending_open_link: Option<PendingOpenLink>,
    pending_window_edited: bool,
    pending_window_title_refresh: bool,
    document_dirty: bool,
    file_path: Option<PathBuf>,
    /// 图片查看是只读派生界面，路径独立于文本模型并随 Tab 快照迁移。
    image_preview_path: Option<PathBuf>,
    /// 图片缩放相对“适合可用宽度”的倍率；属于标签视图状态，不进入文档事务。
    image_preview_zoom: f32,
    /// SVG 派生预览按文档路径与 revision 缓存原始字节，避免每帧复制整份 XML。
    svg_preview_cache: Option<svg_preview::SvgPreviewCache>,
    /// 打不开的文件仍占用真实 Tab；错误与后续动作必须留在内容区，不能污染工作区扫描状态。
    file_open_failure: Option<FileOpenFailure>,
    saved_file_fingerprint: Option<crate::recovery::FileFingerprint>,
    /// 父目录 watcher 跨原子替换存活；事件仍按当前文档 path 过滤。
    file_watch_guard: Option<services::file_watch::FileWatchGuard>,
    file_watch_task: Option<Task<()>>,
    external_file_conflict: bool,
    recovered_session: bool,
    show_external_conflict_dialog: bool,
    show_encoding_conversion_dialog: bool,
    external_conflict_preview: Option<ExternalConflictPreview>,
    external_conflict_restore_focus: Option<EntityId>,
    allow_external_overwrite_once: bool,
    scroll_handle: ScrollHandle,
    last_scroll_viewport_size: Option<Size<Pixels>>,
    /// 上一帧可见块 ID，用于在文档结构变化时重新学习行高度。
    prev_visible_block_ids: Vec<EntityId>,
    /// 已测行包装器高度（包含自身顶部间距），按首块 ID 索引。
    row_stride_cache: HashMap<EntityId, f32>,
    /// 主编辑表面的主题、字体族与内容列宽身份。
    row_stride_layout_identity: Option<RowStrideLayoutIdentity>,
    /// Split 预览使用独立列宽，因此单独保留其布局身份。
    split_row_stride_layout_identity: Option<RowStrideLayoutIdentity>,
    /// 结构或分组语义未变化时跨滚动帧复用，避免逐 Entity 重扫行分组。
    render_row_cache: Option<render::RenderedRowCache>,
    /// None 表示行高缓存已失效，深滚时需允许从首个未测量行恢复。
    prev_render_window: Option<(usize, usize)>,
    close_guard_installed: bool,
    show_unsaved_changes_dialog: bool,
    /// When true, the window will close after the next successful save.
    pending_close_after_save: bool,
    /// Focus target to restore when the close dialog is dismissed.
    close_dialog_restore_focus: Option<EntityId>,
    /// 模态键盘目标独立于正文；Source 与 pane 焦点通过原句柄恢复，不依赖 Markdown EntityId。
    close_dialog_focus_handles: [FocusHandle; 3],
    close_dialog_keyboard_index: usize,
    close_dialog_restore_input_focus: Option<FocusHandle>,
    pending_drop_replace_path: Option<PathBuf>,
    show_drop_replace_dialog: bool,
    pending_drop_replace_after_save: bool,
    drop_replace_restore_focus: Option<EntityId>,
    /// Optional informational dialog shown from the Help menu.
    info_dialog: Option<InfoDialogKind>,
    /// True while an online update check is running in the background.
    workspace: WorkspaceState,
    tabs: tabs::TabState,
    /// Authoritative recursive pane workspace for the document canvas. The
    /// root Editor remains the window shell; active child canvases live in the
    /// pane entity and are keyed by pane/tab identity here for lifecycle sync.
    pane_workspace: Option<Entity<panes::PaneWorkspaceView>>,
    pane_events: Rc<RefCell<Vec<panes::PaneEvent>>>,
    /// Short-lived non-blocking pane action notice (duplicate/limit/reject).
    pane_notice: Option<SharedString>,
    pane_notice_task: Option<Task<()>>,
    /// Pending pane-local close target.  It is kept separate from the legacy
    /// tab index so a dirty pane tab can be prompted without activating or
    /// mutating another pane first.
    pane_close_target: Option<(panes::PaneId, panes::TabId)>,
    /// Monotonic request generation invalidates a cancelled or superseded
    /// pane close before any late save callback can reach the workspace.
    pane_close_generation: u64,
    /// One-shot completion bridge shared with the child Editor or DocumentHost
    /// that owns the save operation.
    pane_close_save_signal: Option<Arc<Mutex<Option<futures::channel::oneshot::Sender<u8>>>>>,
    /// The Markdown child keeps a clone of the one-shot bridge; retain its
    /// entity so cancellation can clear that clone before a late save result.
    pane_close_save_markdown_editor: Option<Entity<Editor>>,
    /// The host event subscription must outlive the asynchronous save task and
    /// be dropped immediately when the request is cancelled or settled.
    pane_close_save_subscription: Option<Subscription>,
    pane_close_save_task: Option<Task<()>>,
    pane_canvas_entities: Rc<
        RefCell<
            BTreeMap<
                panes::PaneId,
                (
                    panes::TabId,
                    gmark_document_runtime::DocumentViewInstanceId,
                    panes::PaneCanvasEntity,
                ),
            >,
        >,
    >,
    focus_mode: bool,
    typewriter_mode: bool,
    status_bar: StatusBarState,
    context_menu: Option<ContextMenuState>,
    resource_title_dialog: Option<ResourceTitleDialogState>,
    /// Logical selection for editor, workspace, and tab context menus. It is
    /// independent from GPUI focus so dismissing a menu preserves the caret.
    context_menu_keyboard_item: Option<usize>,
    context_menu_keyboard_submenu_item: Option<usize>,
    context_menu_scroll_handle: ScrollHandle,
    command_palette: Option<command_palette::CommandPaletteState>,
    find_panel: Option<find_replace::FindPanelState>,
    table_insert_dialog: Option<TableInsertDialogState>,
    context_menu_submenu_close_task: Option<Task<()>>,
    table_axis_preview: Option<TableAxisSelection>,
    table_axis_selection: Option<TableAxisSelection>,
    table_cell_rectangle: Option<table_selection::TableCellRectangle>,
    table_cell_drag_anchor: Option<(EntityId, TableCellPosition)>,
    table_fragment_merge: Option<TableFragmentMergeState>,
    diagram_overlay: Option<DiagramOverlayState>,
    diagram_overlay_restore_focus: Option<FocusHandle>,
    workspace_link_completion: Option<WorkspaceLinkCompletionState>,
    cross_block_selection: Option<CrossBlockSelection>,
    cross_block_drag: Option<CrossBlockDrag>,
    /// Split 两侧持有独立选区；命令始终归属于最近操作的文档表面。
    active_selection_surface: selection_surface::SelectionSurface,
    split_preview_cross_block_selection: Option<CrossBlockSelection>,
    split_preview_cross_block_drag: Option<CrossBlockDrag>,
    split_preview_table_cell_rectangle: Option<table_selection::TableCellRectangle>,
    split_preview_table_cell_drag_anchor: Option<(EntityId, TableCellPosition)>,
    selection_autoscroll_task: Option<Task<()>>,
    /// Prevents a stopped task's late tick from clearing a replacement task on the same surface.
    selection_autoscroll_generation: u64,
    selection_autoscroll_pointer: Option<Point<Pixels>>,
    selection_autoscroll_surface: selection_surface::SelectionSurface,
    rendered_select_all_cycle: Option<RenderedSelectAllCycle>,
    /// 应用图标独立控制一级导航是否展开，不与任一下拉面板的生命周期耦合。
    menu_bar_expanded: bool,
    menu_window_activation_subscription: Option<Subscription>,
    /// Open top-level menu in the in-window fallback menu bar.
    menu_bar_open: Option<usize>,
    /// Open child submenu inside the in-window fallback menu panel.
    menu_submenu_open: Option<usize>,
    /// Keyboard-only focus within the open main menu. Pointer hover keeps its
    /// own visual state and must not fabricate a persistent keyboard focus.
    menu_keyboard_item: Option<usize>,
    /// Keyboard focus within the currently open child submenu.
    menu_keyboard_submenu_item: Option<usize>,
    menu_bar_hovered: bool,
    menu_panel_hovered: bool,
    menu_submenu_panel_hovered: bool,
    /// Hover state for the invisible bridge spanning the gap between the menu
    /// panel and an open submenu. Tracked separately from
    /// `menu_submenu_panel_hovered` so the handoff between the two regions
    /// cannot clobber a single shared flag and tear the menu down.
    menu_submenu_bridge_hovered: bool,
    menu_close_task: Option<Task<()>>,
    scrollbar_hovered: bool,
    /// 仅表示指针位于 scrollbar hitbox；编辑画布 hover 只控制淡入淡出，不得放大 thumb。
    scrollbar_thumb_hovered: bool,
    scrollbar_visible_until: Instant,
    scrollbar_fade_task: Option<Task<()>>,
    /// 鼠标滚轮的行级输入在短时间内插值；触控板像素输入始终绕过该状态。
    smooth_scroll_animation: Option<SmoothScrollAnimation>,
    smooth_scroll_task: Option<Task<()>>,
    split_preview_scrollbar_hovered: bool,
    split_preview_scrollbar_visible_until: Instant,
    split_preview_scrollbar_fade_task: Option<Task<()>>,
    /// Forces a repaint shortly after a pending scroll-into-view that could
    /// not be satisfied yet (the target block has no measured bounds), so the
    /// scroll lands on the next frame instead of waiting for the cursor blink.
    scroll_recheck_task: Option<Task<()>>,
    /// Live/Source 只发布纯投影缓存，不触碰当前 GPUI Entity 树。
    projection_cache_task: Option<Task<()>>,
    projection_cache_scheduled_revision: Option<Revision>,
    /// 合并连续输入的 Split 投影刷新任务；替换 Task 会取消旧定时器。
    split_projection_task: Option<Task<()>>,
    split_projection_scheduled_revision: Option<Revision>,
    recovery_journal: Option<Arc<Mutex<crate::recovery::RecoveryJournal>>>,
    recovery_task: Option<Task<()>>,
    recovery_generation: u64,
    scrollbar_drag: Option<ScrollbarDragSession>,
    split_preview_scrollbar_drag: Option<ScrollbarDragSession>,
    undo_history: Vec<HistoryEntry>,
    redo_history: Vec<HistoryEntry>,
    pending_undo_capture: Option<PendingUndoCapture>,
    /// 虚拟模式由 SourceDocument 保存逆补丁；Editor 只保存选择，不复制全文。
    virtual_undo_selections: Vec<UndoSelectionSnapshot>,
    virtual_redo_selections: Vec<UndoSelectionSnapshot>,
    pending_virtual_undo_selection: Option<UndoSelectionSnapshot>,
    last_selection_snapshot: UndoSelectionSnapshot,
    /// 最近一次稳定投影的正文；正常路径共享 SourceDocument Rope，不另存完整 String。
    last_stable_source: HistorySource,
    /// `mark_dirty` 已物化的当前源码，供同一事务的历史收尾复用，避免长文档重复序列化。
    pending_dirty_source: Option<String>,
    history_restore_in_progress: bool,
    image_reference_definitions: Arc<ImageReferenceDefinitions>,
    link_reference_definitions: Arc<LinkReferenceDefinitions>,
    footnote_registry: Arc<FootnoteRegistry>,
    pending_virtual_global_runtime_refresh: bool,
    pending_virtual_footnote_focus: Option<String>,
    pending_virtual_footnote_backref_focus: Option<String>,
    /// 大文档以源码区域为全局所有权，只把 viewport 与 pinned region 物化为 Entity。
    virtual_surface: Option<VirtualSurfaceState>,
    /// 仅在性能追踪开启时存在，默认渲染路径不分配采样记录。
    first_render_started: Option<Instant>,
}

impl Drop for Editor {
    fn drop(&mut self) {
        if let Some(cancelled) = self.export_cancel.as_ref() {
            cancelled.store(true, std::sync::atomic::Ordering::Release);
        }
        // Render assets own decoded CPU/GPU payloads and background tasks. Drop
        // them explicitly with the editor entity so closing the last window
        // cannot retain a document's cache until process teardown.
        self.release_all_render_assets();
    }
}

/// Runtime binding between a table block and one cell editor.
#[derive(Clone)]
struct TableCellBinding {
    table_block: Entity<Block>,
    cell: Entity<Block>,
    position: TableCellPosition,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ExternalConflictPreview {
    path: String,
    first_difference_line: Option<usize>,
    local_line: String,
    disk_line: String,
    local_line_count: usize,
    disk_line_count: usize,
    local_bytes: usize,
    disk_bytes: usize,
    disk_error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FileOpenFailure {
    path: PathBuf,
    reason: String,
    action_error: Option<String>,
}

/// Split 模式右侧只读投影的独立运行时状态。
struct SplitPreviewState {
    document: DocumentTree,
    virtual_surface: Option<VirtualSurfaceState>,
    table_cells: HashMap<EntityId, TableCellBinding>,
    source_ranges: HashMap<EntityId, std::ops::Range<usize>>,
    scroll_handle: ScrollHandle,
    scroll_driver: Option<SplitScrollDriver>,
    row_stride_cache: HashMap<EntityId, f32>,
    previous_visible_ids: Vec<EntityId>,
    previous_render_window: Option<(usize, usize)>,
    revision: Revision,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SplitScrollDriver {
    Source,
    Preview,
}

/// 单个权威动画同时服务源码与 Split 预览，连续滚轮只更新目标而不排队。
#[derive(Clone, Copy, Debug)]
struct SmoothScrollAnimation {
    driver: SplitScrollDriver,
    start_y: Pixels,
    target_y: Pixels,
    last_applied_y: Pixels,
    started_at: Instant,
}

/// Selected row or column in a rendered native table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct TableAxisSelection {
    table_block_id: EntityId,
    kind: TableAxisKind,
    index: usize,
}

/// Pixel geometry for the custom editor scrollbar.
#[derive(Clone, Copy, Debug, PartialEq)]
struct ScrollbarGeometry {
    track_height: f32,
    thumb_height: f32,
    thumb_top: f32,
    max_scroll_y: f32,
}

/// Windowing result: the run of rows to mount, plus the top/bottom spacer
/// heights standing in for the culled rows.
#[derive(Clone, Copy, Debug, PartialEq)]
struct RenderWindow {
    run_start: usize,
    run_end: usize,
    top_h: f32,
    bottom_h: f32,
}

/// Active drag session for the custom scrollbar thumb.
#[derive(Clone, Copy, Debug, PartialEq)]
struct ScrollbarDragSession {
    pointer_offset_y: f32,
    track_height: f32,
    thumb_height: f32,
    max_scroll_y: f32,
}

/// Source-mode selection snapshot stored with undo history.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct UndoSelectionSnapshot {
    selection: SourceSelection,
}

impl UndoSelectionSnapshot {
    /// Resident Rope 与磁盘 PieceTree 共用同一套 anchor/affinity 真值；调用方仍可在
    /// Rope 边界使用 usize，但不能把恢复、history 和视图切换状态退化回 Range。
    fn from_range(range: std::ops::Range<usize>, reversed: bool) -> Self {
        let start = range.start.min(range.end);
        let end = range.start.max(range.end);
        Self {
            selection: SourceSelection::from_range(start as u64..end as u64, reversed),
        }
    }

    fn collapsed(byte_offset: usize, affinity: SourceAffinity) -> Self {
        Self {
            selection: SourceSelection::collapsed(byte_offset as u64, affinity),
        }
    }

    fn from_source_selection(selection: SourceSelection) -> Self {
        Self { selection }
    }

    fn source_selection(self) -> SourceSelection {
        self.selection
    }

    fn range(self) -> std::ops::Range<usize> {
        let range = self.selection.range();
        saturating_source_offset(range.start)..saturating_source_offset(range.end)
    }

    fn reversed(self) -> bool {
        self.selection.reversed()
    }
}

fn saturating_source_offset(offset: u64) -> usize {
    offset.min(usize::MAX as u64) as usize
}

/// 历史正文优先共享 SourceDocument 的 Rope；投影暂时不同步时才保留独立文本。
#[derive(Clone, Debug)]
enum HistorySource {
    Snapshot(gmark_document::DocumentSnapshot),
    Materialized(Arc<str>),
}

impl HistorySource {
    fn capture(snapshot: gmark_document::DocumentSnapshot, text: String) -> Self {
        if snapshot.matches_text(&text) {
            Self::Snapshot(snapshot)
        } else {
            Self::Materialized(text.into())
        }
    }

    fn matches_text(&self, text: &str) -> bool {
        match self {
            Self::Snapshot(snapshot) => snapshot.matches_text(text),
            Self::Materialized(source) => source.as_ref() == text,
        }
    }

    fn text(&self) -> String {
        match self {
            Self::Snapshot(snapshot) => snapshot.text(),
            Self::Materialized(source) => source.to_string(),
        }
    }

    fn len(&self) -> usize {
        match self {
            Self::Snapshot(snapshot) => snapshot.len(),
            Self::Materialized(source) => source.len(),
        }
    }

    fn empty() -> Self {
        Self::Materialized(Arc::from(""))
    }

    #[cfg(test)]
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// One undo history entry containing source text and selection state.
#[derive(Clone, Debug)]
struct HistoryEntry {
    source: HistorySource,
    source_format: gmark_document::SourceFormatSnapshot,
    selection: UndoSelectionSnapshot,
    timestamp: Instant,
    kind: UndoCaptureKind,
}

/// Deferred undo capture used to coalesce adjacent typing edits.
#[derive(Clone, Debug)]
struct PendingUndoCapture {
    snapshot: HistoryEntry,
}

/// Cross-block selection endpoint in visible block order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct CrossBlockSelectionEndpoint {
    pub(super) entity_id: EntityId,
    pub(super) offset: usize,
}

/// Canonical source coordinate that is valid only for the revision where it was captured.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct CrossBlockSourceAnchor {
    pub(super) byte_offset: usize,
    pub(super) revision: Revision,
}

/// Editor-level selection spanning two visible block endpoints.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct CrossBlockSelection {
    pub(super) anchor: CrossBlockSelectionEndpoint,
    pub(super) focus: CrossBlockSelectionEndpoint,
    pub(super) source_anchor: Option<CrossBlockSourceAnchor>,
    pub(super) source_focus: Option<CrossBlockSourceAnchor>,
}

/// Drag state while creating or extending a cross-block selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct CrossBlockDrag {
    pub(super) anchor: CrossBlockSelectionEndpoint,
    pub(super) source_anchor: Option<CrossBlockSourceAnchor>,
}

/// Short-lived Ctrl/Cmd+A press counter for rendered-mode selection upgrade.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct RenderedSelectAllCycle {
    entity_id: EntityId,
    count: u8,
    last_pressed_at: Instant,
}

/// Mapping from one visible block's text range to canonical Markdown offsets.
#[derive(Clone)]
pub(super) struct SourceTargetMapping {
    entity: Entity<Block>,
    full_source_range: std::ops::Range<usize>,
    content_to_source: Vec<usize>,
    source_to_content: Vec<usize>,
}

/// The two editing views the editor can present.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewMode {
    /// Rich rendered view where each block is styled by its semantic kind.
    Rendered,
    /// Plain source view where the full Markdown document is edited as a
    /// single raw buffer.
    Source,
    /// Read-only rendered document for reading and presentation.
    Preview,
    /// Editable source and read-only projection shown side by side.
    Split,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DocumentKind {
    Unspecified,
    Markdown,
    Json,
    Csv,
}

impl DocumentKind {
    fn from_path(path: &Path) -> Self {
        match path
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("md" | "markdown") => Self::Markdown,
            Some("json" | "jsonl" | "ndjson") => Self::Json,
            Some("csv") => Self::Csv,
            _ => Self::Unspecified,
        }
    }

    fn initial_source(self) -> &'static str {
        match self {
            Self::Json => "{\n}\n",
            Self::Csv => "Column 1,Column 2\n",
            Self::Unspecified | Self::Markdown => "",
        }
    }

    fn initial_view_mode(self) -> ViewMode {
        match self {
            Self::Markdown => ViewMode::Rendered,
            Self::Unspecified | Self::Json | Self::Csv => ViewMode::Source,
        }
    }

    /// JSON/CSV 标签共享通用格式宿主；Markdown 与未指定文本继续走轻量 Block 编辑器。
    fn document_host_format(self) -> Option<gmark_document_core::DocumentFormat> {
        match self {
            Self::Json => Some(gmark_document_core::DocumentFormat::Json),
            Self::Csv => Some(gmark_document_core::DocumentFormat::Delimited { delimiter: b',' }),
            Self::Unspecified | Self::Markdown => None,
        }
    }

    /// 无路径标签也必须使用与同扩展名文件一致的 Source 语法；CSV 和普通文本
    /// 保持无语法投影，避免被 Markdown 标记规则改变输入呈现。
    fn source_syntax_language(self) -> Option<&'static str> {
        match self {
            Self::Markdown => Some("markdown"),
            Self::Json => Some("json"),
            Self::Unspecified | Self::Csv => None,
        }
    }

    fn untitled_name(self) -> &'static str {
        match self {
            Self::Unspecified => "Untitled",
            Self::Markdown => "Untitled.md",
            Self::Json => "Untitled.json",
            Self::Csv => "Untitled.csv",
        }
    }

    fn default_extension(self) -> Option<&'static str> {
        match self {
            Self::Unspecified => None,
            Self::Markdown => Some("md"),
            Self::Json => Some("json"),
            Self::Csv => Some("csv"),
        }
    }

    fn apply_default_extension(self, path: &mut PathBuf) {
        if path.extension().is_none()
            && let Some(extension) = self.default_extension()
        {
            path.set_extension(extension);
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Self::Unspecified => "icon/ui/file.svg",
            Self::Markdown => "icon/workspace/markdown.svg",
            Self::Json => "icon/ui/code.svg",
            Self::Csv => "icon/ui/table.svg",
        }
    }
}

/// The informational dialogs that can be shown from the Help menu.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum InfoDialogKind {
    /// Dialog with app name and version information.
    About,
    /// Read-only facts about the active document and its current view.
    Document,
}

#[path = "core_parts/editor_construction.rs"]
mod construction;
