// @author kongweiguang

//! Multi-document tab sessions with ownership-preserving state migration.

use super::document_session::EditorDocumentSession;
use std::mem;
use std::path::{Path, PathBuf};
#[cfg(not(test))]
use std::sync::OnceLock;
#[cfg(test)]
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
#[cfg(not(test))]
use std::time::Duration;
use std::{collections::HashMap, collections::HashSet};

use gmark_document::SourceDocument;
use gpui::prelude::FluentBuilder;
use gpui::*;

use super::{
    DocumentKind, Editor, ExternalConflictPreview, FileOpenFailure, HistoryEntry, HistorySource,
    PendingUndoCapture, UndoSelectionSnapshot, ViewMode,
    render::{
        DialogButtonKind, DialogTitleIcon, DocumentToolbarAction, clamped_floating_panel_origin,
        compact_menu_panel_height, dialog_actions, dialog_body, dialog_button, dialog_content,
        dialog_panel, dialog_title_with_icon, menu_icon_slot, modal_overlay,
    },
};
use crate::config::EditorSettings;
use crate::window_chrome::middle_ellipsis;

const CLOSED_TAB_LIMIT: usize = 20;
const CLOSED_TAB_RETAINED_BYTES_LIMIT: usize = 64 * 1024 * 1024;
const TAB_CLOSE_ICON: &str = "icon/ui/close.svg";
const NEW_TAB_ICON: &str = "icon/ui/plus.svg";
const TAB_DOCUMENT_ICON: &str = "icon/workspace/markdown.svg";
const TAB_IMAGE_ICON: &str = "icon/ui/image.svg";
const TAB_PIN_ICON: &str = "icon/editor/tab-pin.svg";
const QUICK_OPEN_ICON: &str = "icon/ui/files.svg";
const FIND_ICON: &str = "icon/ui/search.svg";
const COMMAND_PALETTE_ICON: &str = "icon/ui/keyboard.svg";
const TAB_STRIP_HEIGHT: f32 = 36.0;
/// 全局与窗格局部 Tab 共享同一半径，避免两条独立渲染路径在主题或布局演进后出现轮廓偏差。
pub(super) const TERMINAL_TAB_SHOULDER_RADIUS: f32 = 8.0;
pub(super) const TERMINAL_INACTIVE_TAB_SEPARATOR_HEIGHT: f32 = 16.0;
const TAB_TOOL_BUTTON_SIZE: f32 = 28.0;
const TAB_TOOL_GROUP_PADDING: f32 = 4.0;
const TAB_MIN_WIDTH: f32 = 96.0;
const TAB_MAX_WIDTH: f32 = 220.0;

#[cfg(test)]
static REOPEN_TEST_DELAY_MS: AtomicU64 = AtomicU64::new(0);

#[cfg(test)]
pub(super) fn set_reopen_test_delay_ms(delay_ms: u64) {
    // 原因：测试需要稳定制造慢 I/O 窗口，才能确定验证重开任务不会占用 GPUI Context。
    REOPEN_TEST_DELAY_MS.store(delay_ms, Ordering::Release);
}

/// Clears a semantic color without introducing a palette-independent brand
/// value. Transparent geometry is used only for hit areas and integrated
/// chrome where the surface below must remain visible.
fn transparent_color(mut color: Hsla) -> Hsla {
    color.a = 0.0;
    color
}

/// 使用纯绘制外扩层生成凹圆肩，是为了复刻 Terminal 轮廓而不改变 Tab 的布局盒、命中区和拖放边界。
pub(super) fn terminal_tab_shoulder_cutout(
    active_surface: Hsla,
    background: Hsla,
    left: bool,
    selector: String,
) -> Div {
    let circle = div()
        .absolute()
        .bottom_0()
        .size(px(TERMINAL_TAB_SHOULDER_RADIUS * 2.0))
        .rounded_full()
        .bg(background);
    let mut cutout = div()
        .absolute()
        .bottom_0()
        .size(px(TERMINAL_TAB_SHOULDER_RADIUS))
        .overflow_hidden()
        .bg(active_surface)
        .debug_selector(move || selector.clone());
    if left {
        // 圆形锚定在右下方，才能只用栏底色切走左肩所需象限并保持 8px 基线对齐。
        cutout = cutout
            .left(px(-TERMINAL_TAB_SHOULDER_RADIUS))
            .child(circle.right_0());
    } else {
        // 镜像锚点可避免左右肩取样不同，确保两侧在 36px 栏底形成连续且对称的切口。
        cutout = cutout
            .right(px(-TERMINAL_TAB_SHOULDER_RADIUS))
            .child(circle.left_0());
    }
    cutout
}

/// 相邻未选中 Tab 使用独立绘制线，是为了保留 Terminal 的分组节奏且不让描边参与布局或命中计算。
pub(super) fn terminal_inactive_tab_separator(
    color: Hsla,
    left_offset: f32,
    tab_height: f32,
    selector: String,
) -> Div {
    div()
        .absolute()
        .left(px(left_offset))
        .top(px(
            (tab_height - TERMINAL_INACTIVE_TAB_SEPARATOR_HEIGHT) * 0.5
        ))
        .w(px(1.0))
        .h(px(TERMINAL_INACTIVE_TAB_SEPARATOR_HEIGHT))
        .bg(color)
        .debug_selector(move || selector.clone())
}
/// Tracks the newest snapshot generation before a writer acquires the disk lock.
///
/// The separate registry lets a writer re-check freshness after waiting for the
/// serialized write lock, so a stale task cannot overwrite a newer snapshot.
#[derive(Default)]
pub(super) struct SessionWriteGenerationRegistry {
    generations: Mutex<HashMap<uuid::Uuid, u64>>,
}

impl SessionWriteGenerationRegistry {
    /// Records a generation before starting I/O so already queued writers can
    /// observe that their captured snapshot is no longer authoritative.
    pub(super) fn set(&self, session_id: uuid::Uuid, generation: u64) -> anyhow::Result<()> {
        self.generations
            .lock()
            .map_err(|_| anyhow::anyhow!("workspace session generation lock poisoned"))?
            .insert(session_id, generation);
        Ok(())
    }

    /// Checks freshness only after the caller owns the write lock, preserving
    /// the ordering that prevents an older waiter from writing after a newer one.
    pub(super) fn is_current(
        &self,
        session_id: uuid::Uuid,
        generation: u64,
    ) -> anyhow::Result<bool> {
        Ok(self
            .generations
            .lock()
            .map_err(|_| anyhow::anyhow!("workspace session generation lock poisoned"))?
            .get(&session_id)
            .copied()
            == Some(generation))
    }
}

#[cfg(not(test))]
static SESSION_WRITE_GENERATIONS: OnceLock<SessionWriteGenerationRegistry> = OnceLock::new();
#[cfg(not(test))]
static SESSION_WRITE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Clone)]
struct TabDragPayload {
    id: uuid::Uuid,
    title: String,
    background: Hsla,
    text: Hsla,
}

struct TabDragPreview {
    payload: TabDragPayload,
    position: Point<Pixels>,
}

impl Render for TabDragPreview {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .absolute()
            .left(self.position.x + px(10.0))
            .top(self.position.y + px(10.0))
            .w(px(180.0))
            .h(px(32.0))
            .px(px(10.0))
            .flex()
            .items_center()
            .overflow_hidden()
            .truncate()
            .rounded(px(6.0))
            .shadow_md()
            .bg(self.payload.background)
            .text_color(self.payload.text)
            .child(self.payload.title.clone())
    }
}

struct TabContextMenu {
    index: usize,
    position: Point<Pixels>,
}

struct NewTabMenu {
    position: Point<Pixels>,
    pane: Option<crate::editor::panes::PaneId>,
}

pub(super) struct SplitPaneMenu {
    pub(super) position: Point<Pixels>,
    pub(super) focus_handle: FocusHandle,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SessionViewSignature {
    tab_id: uuid::Uuid,
    mode: u8,
    selection_start: usize,
    selection_end: usize,
    selection_reversed: bool,
    scroll_x_bits: u32,
    scroll_y_bits: u32,
}

#[derive(Clone)]
pub(crate) struct RestoredTab {
    pub(crate) opened: crate::document_io::OpenedDocument,
    pub(crate) path: PathBuf,
    pub(crate) pinned: bool,
    pub(crate) view_mode: Option<String>,
    pub(crate) selection: Option<crate::config::workspace_session::WorkspaceSessionSelection>,
    pub(crate) scroll_x: Option<f32>,
    pub(crate) scroll_y: Option<f32>,
}

#[derive(Clone)]
pub(crate) struct DetachedTab {
    snapshot: DocumentTabSnapshot,
}

impl DetachedTab {
    pub(crate) fn file_path(&self) -> Option<&Path> {
        self.snapshot.file_path.as_deref()
    }
}

#[derive(Clone)]
pub(crate) struct DocumentTabSnapshot {
    pub(super) document_host: Option<Entity<crate::document_host::DocumentHost>>,
    pub(super) source_document: EditorDocumentSession,
    shared_document: bool,
    source_encoding: crate::document_io::DocumentEncoding,
    pub(super) document_kind: DocumentKind,
    pub(super) file_path: Option<PathBuf>,
    pub(super) image_preview_path: Option<PathBuf>,
    image_preview_zoom: f32,
    pub(super) file_open_failure: Option<FileOpenFailure>,
    saved_file_fingerprint: Option<crate::recovery::FileFingerprint>,
    pub(super) document_dirty: bool,
    pub(super) view_mode: ViewMode,
    pub(super) selection: UndoSelectionSnapshot,
    pub(super) scroll_offset: Point<Pixels>,
    undo_history: Vec<HistoryEntry>,
    redo_history: Vec<HistoryEntry>,
    pending_undo_capture: Option<PendingUndoCapture>,
    virtual_undo_selections: Vec<UndoSelectionSnapshot>,
    virtual_redo_selections: Vec<UndoSelectionSnapshot>,
    pending_virtual_undo_selection: Option<UndoSelectionSnapshot>,
    recovery_journal: Option<Arc<Mutex<crate::recovery::RecoveryJournal>>>,
    external_file_conflict: bool,
    recovered_session: bool,
    show_encoding_conversion_dialog: bool,
    external_conflict_preview: Option<ExternalConflictPreview>,
    allow_external_overwrite_once: bool,
}

/// Lease-free reopen state. Closed-tab history must not keep a Controller
/// handle, render entity, or background task alive after the active view is
/// dropped. Durable files retain only their path/identity; host documents
/// retain the immutable probe needed to rebuild the host shell. Recovery and
/// untitled documents retain only their UUID and are loaded from the journal
/// on reopen. No closed tab owns a second source body or recovery journal
/// handle.
#[derive(Clone)]
pub(super) enum ClosedDocumentSource {
    File {
        path: PathBuf,
        document_id: gmark_document_runtime::DocumentId,
    },
    Host {
        path: PathBuf,
        document_id: gmark_document_runtime::DocumentId,
        probe: gmark_paged_document::OpenProbe,
    },
    Recovery {
        document_id: gmark_document_runtime::DocumentId,
        #[cfg(test)]
        journal_path: Option<PathBuf>,
    },
    Image {
        path: PathBuf,
    },
    Error {
        path: PathBuf,
        reason: String,
    },
}

#[derive(Clone)]
struct ClosedTabSnapshot {
    source: ClosedDocumentSource,
    source_encoding: crate::document_io::DocumentEncoding,
    document_kind: DocumentKind,
    file_path: Option<PathBuf>,
    image_preview_path: Option<PathBuf>,
    image_preview_zoom: f32,
    file_open_failure: Option<FileOpenFailure>,
    saved_file_fingerprint: Option<crate::recovery::FileFingerprint>,
    document_dirty: bool,
    view_mode: ViewMode,
    selection: UndoSelectionSnapshot,
    scroll_offset: Point<Pixels>,
    undo_history: Vec<HistoryEntry>,
    redo_history: Vec<HistoryEntry>,
    pending_undo_capture: Option<PendingUndoCapture>,
    virtual_undo_selections: Vec<UndoSelectionSnapshot>,
    virtual_redo_selections: Vec<UndoSelectionSnapshot>,
    pending_virtual_undo_selection: Option<UndoSelectionSnapshot>,
    external_file_conflict: bool,
    recovered_session: bool,
    show_encoding_conversion_dialog: bool,
    external_conflict_preview: Option<ExternalConflictPreview>,
    allow_external_overwrite_once: bool,
}

impl ClosedTabSnapshot {
    fn from_document(
        snapshot: DocumentTabSnapshot,
    ) -> Result<Self, super::document_session::EditorDocumentSessionError> {
        Self::from_document_with_host(snapshot, None)
    }

    pub(super) fn from_document_with_host(
        snapshot: DocumentTabSnapshot,
        host: Option<(
            gmark_document_runtime::DocumentId,
            gmark_paged_document::OpenProbe,
        )>,
    ) -> Result<Self, super::document_session::EditorDocumentSessionError> {
        let source = if let Some((document_id, probe)) = host {
            let path = snapshot.file_path.clone().ok_or_else(|| {
                super::document_session::EditorDocumentSessionError::Controller(
                    gmark_document_runtime::ControllerError::OpenFailed(
                        "closed host tab has no path".to_owned(),
                    ),
                )
            })?;
            ClosedDocumentSource::Host {
                path,
                document_id,
                probe,
            }
        } else if snapshot.image_preview_path.is_some() {
            ClosedDocumentSource::Image {
                path: match snapshot.image_preview_path.clone() {
                    Some(path) => path,
                    None => {
                        return Err(
                            super::document_session::EditorDocumentSessionError::Controller(
                                gmark_document_runtime::ControllerError::OpenFailed(
                                    "closed image tab has no path".to_owned(),
                                ),
                            ),
                        );
                    }
                },
            }
        } else if let Some(failure) = snapshot.file_open_failure.as_ref() {
            ClosedDocumentSource::Error {
                path: failure.path.clone(),
                reason: failure.reason.clone(),
            }
        } else if snapshot.recovered_session || snapshot.recovery_journal.is_some() {
            ClosedDocumentSource::Recovery {
                document_id: snapshot.source_document.document_id()?,
                #[cfg(test)]
                journal_path: None,
            }
        } else if snapshot.file_path.is_some() {
            ClosedDocumentSource::File {
                path: match snapshot.file_path.clone() {
                    Some(path) => path,
                    None => {
                        return Err(
                            super::document_session::EditorDocumentSessionError::Controller(
                                gmark_document_runtime::ControllerError::OpenFailed(
                                    "closed file tab has no path".to_owned(),
                                ),
                            ),
                        );
                    }
                },
                document_id: snapshot.source_document.document_id()?,
            }
        } else {
            // Untitled tabs are always represented by a recovery identity.
            // If no journal exists, reopen fails closed instead of retaining
            // a second body in this history entry.
            ClosedDocumentSource::Recovery {
                document_id: snapshot.source_document.document_id()?,
                #[cfg(test)]
                journal_path: None,
            }
        };
        Ok(Self {
            source,
            source_encoding: snapshot.source_encoding,
            document_kind: snapshot.document_kind,
            file_path: snapshot.file_path,
            image_preview_path: snapshot.image_preview_path,
            image_preview_zoom: snapshot.image_preview_zoom,
            file_open_failure: snapshot.file_open_failure,
            saved_file_fingerprint: snapshot.saved_file_fingerprint,
            document_dirty: snapshot.document_dirty,
            view_mode: snapshot.view_mode,
            selection: snapshot.selection,
            scroll_offset: snapshot.scroll_offset,
            // Closed-tab history is metadata-only. The active controller's
            // undo/redo bodies are released with its final lease; recovery
            // journals remain the durable reopen source.
            undo_history: Vec::new(),
            redo_history: Vec::new(),
            pending_undo_capture: None,
            virtual_undo_selections: Vec::new(),
            virtual_redo_selections: Vec::new(),
            pending_virtual_undo_selection: None,
            external_file_conflict: snapshot.external_file_conflict,
            recovered_session: snapshot.recovered_session,
            show_encoding_conversion_dialog: snapshot.show_encoding_conversion_dialog,
            external_conflict_preview: snapshot.external_conflict_preview,
            allow_external_overwrite_once: snapshot.allow_external_overwrite_once,
        })
    }

    pub(super) fn into_document_with_source(
        self,
        source_document: EditorDocumentSession,
        document_host: Option<Entity<crate::document_host::DocumentHost>>,
    ) -> DocumentTabSnapshot {
        DocumentTabSnapshot {
            document_host,
            source_document,
            // Service-backed reopen paths keep the same Controller lease in
            // the new view.
            shared_document: true,
            source_encoding: self.source_encoding,
            document_kind: self.document_kind,
            file_path: self.file_path,
            image_preview_path: self.image_preview_path,
            image_preview_zoom: self.image_preview_zoom,
            file_open_failure: self.file_open_failure,
            saved_file_fingerprint: self.saved_file_fingerprint,
            document_dirty: self.document_dirty,
            view_mode: self.view_mode,
            selection: self.selection,
            scroll_offset: self.scroll_offset,
            undo_history: self.undo_history,
            redo_history: self.redo_history,
            pending_undo_capture: self.pending_undo_capture,
            virtual_undo_selections: self.virtual_undo_selections,
            virtual_redo_selections: self.virtual_redo_selections,
            pending_virtual_undo_selection: self.pending_virtual_undo_selection,
            recovery_journal: None,
            external_file_conflict: self.external_file_conflict,
            recovered_session: self.recovered_session,
            show_encoding_conversion_dialog: self.show_encoding_conversion_dialog,
            external_conflict_preview: self.external_conflict_preview,
            allow_external_overwrite_once: self.allow_external_overwrite_once,
        }
    }

    fn retained_source_bytes(&self) -> usize {
        self.file_path
            .as_ref()
            .map(|path| path.as_os_str().len())
            .unwrap_or_default()
            .saturating_add(std::mem::size_of_val(&self.source))
    }
}

impl DocumentTabSnapshot {
    /// 窗口级放弃必须同步消费非活动 Tab 的 journal；否则正文虽已回滚，旧日志仍会
    /// 在下次启动时复活。这里使用回滚后的快照 checkpoint，并把失败交给关闭流程阻断。
    pub(super) fn checkpoint_recovery_journal(&self) -> anyhow::Result<()> {
        let Some(journal) = self.recovery_journal.as_ref() else {
            return Ok(());
        };
        let source = self.source_document.text();
        let source_format = self.source_document.source_format();
        journal
            .lock()
            .map_err(|_| anyhow::anyhow!("inactive tab recovery journal lock poisoned"))?
            .checkpoint_formatted(self.file_path.clone(), source, source_format)
    }

    /// Build a lightweight tab snapshot around a service-owned shared view.
    /// The session remains the sole body owner; this value only carries the
    /// presentation metadata needed by `install_tab_snapshot`.
    pub(super) fn from_shared_document(
        source_document: EditorDocumentSession,
        path: PathBuf,
        source_encoding: crate::document_io::DocumentEncoding,
    ) -> Self {
        let document_dirty = source_document.try_is_dirty().unwrap_or(true);
        Self {
            document_host: None,
            source_document,
            shared_document: true,
            source_encoding,
            document_kind: DocumentKind::from_path(&path),
            file_path: Some(path.clone()),
            image_preview_path: None,
            image_preview_zoom: 1.0,
            file_open_failure: None,
            saved_file_fingerprint: crate::recovery::fingerprint_file(&path).ok(),
            document_dirty,
            view_mode: ViewMode::Rendered,
            selection: UndoSelectionSnapshot::collapsed(
                0,
                gmark_document_core::SourceAffinity::Before,
            ),
            scroll_offset: point(px(0.0), px(0.0)),
            undo_history: Vec::new(),
            redo_history: Vec::new(),
            pending_undo_capture: None,
            virtual_undo_selections: Vec::new(),
            virtual_redo_selections: Vec::new(),
            pending_virtual_undo_selection: None,
            recovery_journal: None,
            external_file_conflict: false,
            recovered_session: false,
            show_encoding_conversion_dialog: false,
            external_conflict_preview: None,
            allow_external_overwrite_once: false,
        }
    }

    pub(super) fn from_shared_host(
        document_host: Entity<crate::document_host::DocumentHost>,
        path: PathBuf,
        source_encoding: crate::document_io::DocumentEncoding,
        document_dirty: bool,
    ) -> Self {
        let mut snapshot =
            Self::from_shared_document(EditorDocumentSession::shell(), path, source_encoding);
        snapshot.document_host = Some(document_host);
        snapshot.document_dirty = document_dirty;
        snapshot
    }

    /// 以逻辑源码量约束关闭历史；Rope 共享会让实际驻留更低，但绝不能让大量
    /// 大文档及其历史因固定条数上限长期留在进程中。
    fn retained_source_bytes(&self) -> usize {
        let history_bytes = self
            .undo_history
            .iter()
            .chain(&self.redo_history)
            .map(|entry| entry.source.len())
            .chain(
                self.pending_undo_capture
                    .iter()
                    .map(|pending| pending.snapshot.source.len()),
            )
            .fold(0usize, usize::saturating_add);
        self.source_document.len().saturating_add(history_bytes)
    }
}

fn enforce_closed_tab_budget(
    closed: &mut Vec<ClosedTabSnapshot>,
    count_limit: usize,
    retained_bytes_limit: usize,
) {
    // 最新一项始终可恢复；超限时按关闭顺序淘汰更旧的完整状态。
    while closed.len() > 1
        && (closed.len() > count_limit
            || closed
                .iter()
                .map(ClosedTabSnapshot::retained_source_bytes)
                .fold(0usize, usize::saturating_add)
                > retained_bytes_limit)
    {
        closed.remove(0);
    }
}

pub(crate) struct TabRecord {
    pub(super) id: uuid::Uuid,
    pub(super) pinned: bool,
    pub(super) snapshot: Option<DocumentTabSnapshot>,
}

pub(super) struct TabState {
    pub(super) records: Vec<TabRecord>,
    pub(super) active: usize,
    open_generation: u64,
    open_task: Option<Task<()>>,
    /// 独立于普通路径打开的重开代次，避免两类后台结果互相取消或覆盖。
    reopen_generation: u64,
    /// 后台重开任务只回传准备结果，GPUI Entity 仍由完成回调创建。
    reopen_task: Option<Task<()>>,
    /// 保留被弹出的历史项，失败、取消或身份失配时按原顺序恢复。
    reopen_pending: Option<(usize, ClosedTabSnapshot)>,
    closed: Vec<ClosedTabSnapshot>,
    show_close_dialog: bool,
    close_after_save: bool,
    continue_window_close_after_save: bool,
    close_others_keep: Option<uuid::Uuid>,
    context_menu: Option<TabContextMenu>,
    new_tab_menu: Option<NewTabMenu>,
    split_pane_menu: Option<SplitPaneMenu>,
    // reason: 测试构建禁用真实会话落盘任务；remove when session writer is injected as a test adapter.
    #[cfg_attr(test, allow(dead_code))]
    session_generation: u64,
    // reason: 测试构建禁用真实会话落盘任务；remove when session writer is injected as a test adapter.
    #[cfg_attr(test, allow(dead_code))]
    session_task: Option<Task<()>>,
    dragging_tab: Option<uuid::Uuid>,
    session_id: uuid::Uuid,
    remove_session_after_window_close: bool,
    last_session_view_signature: Option<SessionViewSignature>,
    window: Option<crate::config::workspace_session::WorkspaceSessionWindow>,
    window_bounds_subscription: Option<Subscription>,
    /// UUID 绑定焦点身份，标签重排或关闭时不会把键盘焦点错误复用给另一份文档。
    focus_handles: HashMap<uuid::Uuid, FocusHandle>,
    new_tab_focus_handle: Option<FocusHandle>,
}

impl TabState {
    /// Exposes close-prompt visibility to sibling editor modules without opening the state field for mutation.
    pub(super) fn is_close_dialog_open(&self) -> bool {
        self.show_close_dialog
    }

    pub(super) fn has_new_or_split_menu(&self) -> bool {
        self.new_tab_menu.is_some() || self.split_pane_menu.is_some()
    }

    pub(super) fn dismiss_new_or_split_menu(&mut self) -> bool {
        let dismissed = self.has_new_or_split_menu();
        self.new_tab_menu = None;
        self.split_pane_menu = None;
        dismissed
    }

    pub(super) fn active_id(&self) -> uuid::Uuid {
        self.records
            .get(self.active)
            .map(|record| record.id)
            .unwrap_or_else(uuid::Uuid::nil)
    }

    pub(super) fn new() -> Self {
        Self {
            records: vec![TabRecord {
                id: uuid::Uuid::new_v4(),
                pinned: false,
                snapshot: None,
            }],
            active: 0,
            open_generation: 0,
            open_task: None,
            reopen_generation: 0,
            reopen_task: None,
            reopen_pending: None,
            closed: Vec::new(),
            show_close_dialog: false,
            close_after_save: false,
            continue_window_close_after_save: false,
            close_others_keep: None,
            context_menu: None,
            new_tab_menu: None,
            split_pane_menu: None,
            session_generation: 0,
            session_task: None,
            dragging_tab: None,
            session_id: uuid::Uuid::new_v4(),
            remove_session_after_window_close: false,
            last_session_view_signature: None,
            window: None,
            window_bounds_subscription: None,
            focus_handles: HashMap::new(),
            new_tab_focus_handle: None,
        }
    }
}

impl Editor {}

impl Editor {
    /// Returns compatibility Markdown adapter views for close/quit policy.
    /// `EditorDocumentSession::Clone` shares the existing lease Arc and does
    /// not acquire another registry lease or copy the source body.
    pub(super) fn markdown_tab_sources(&self) -> Vec<(usize, EditorDocumentSession)> {
        let mut sources = Vec::new();
        if self.pane_workspace.is_none() && self.document_host.is_none() {
            sources.push((self.tabs.active, self.source_document.clone()));
        }
        if self.pane_workspace.is_some() {
            return sources;
        }
        for (index, record) in self.tabs.records.iter().enumerate() {
            if let Some(snapshot) = record.snapshot.as_ref()
                && snapshot.document_host.is_none()
            {
                sources.push((index, snapshot.source_document.clone()));
            }
        }
        sources
    }

    pub(super) fn active_tab_index(&self) -> usize {
        self.tabs.active
    }
}

#[path = "tabs_parts/lifecycle.rs"]
mod lifecycle;
#[path = "workspace/tabs/new_tab_menu.rs"]
mod new_tab_menu;
#[path = "workspace/tabs/path_open.rs"]
mod path_open;
#[path = "tabs_parts/session.rs"]
mod session;
#[path = "workspace/tabs/split_pane_menu.rs"]
mod split_pane_menu;
#[path = "tabs_parts/view.rs"]
mod view;
#[path = "workspace/tabs/window_close.rs"]
mod window_close;

#[cfg(test)]
#[path = "../../tests/unit/editor/tabs.rs"]
mod tests;
