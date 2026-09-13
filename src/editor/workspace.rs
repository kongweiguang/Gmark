// @author kongweiguang

//! Lightweight workspace panel state, file-tree scanning, and outline parsing.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, atomic::AtomicBool};
use std::time::Duration;

use anyhow::Result;
use gpui::*;

use super::{
    Block, BlockKind, BlockRecord, ContextMenuState, DocumentKind, Editor, UndoSelectionSnapshot,
    ViewMode,
    render::{
        DialogButtonKind, DialogTitleIcon, compact_dialog_actions, dialog_actions, dialog_button,
        dialog_content, dialog_panel, dialog_title_with_icon, modal_overlay,
    },
    workspace_file_ops,
};
use crate::components::BlockEvent;
use crate::i18n::I18nStrings;
use crate::theme::Theme;
use crate::window_chrome::middle_ellipsis;

const FOLDER_ICON: &str = "icon/workspace/folder.svg";
const MARKDOWN_ICON: &str = "icon/workspace/markdown.svg";
const FILE_ICON: &str = "icon/ui/file.svg";
const FILES_TAB_ICON: &str = "icon/ui/files.svg";
const OUTLINE_TAB_ICON: &str = "icon/ui/outline.svg";
const SEARCH_TAB_ICON: &str = "icon/ui/search.svg";
const CHEVRON_RIGHT_ICON: &str = "icon/ui/chevron-right.svg";
const CHEVRON_DOWN_ICON: &str = "icon/ui/chevron-down.svg";
const CLOSE_ICON: &str = "icon/ui/close.svg";
const REFRESH_ICON: &str = "icon/ui/refresh.svg";
const CHECK_ICON: &str = "icon/ui/check.svg";
const WARNING_ICON: &str = "icon/ui/triangle-alert.svg";
const WORKSPACE_PANEL_TARGET_RATIO: f32 = 0.15;
const WORKSPACE_PANEL_AUTO_MIN_WIDTH: f32 = 248.0;
const WORKSPACE_PANEL_MIN_WIDTH: f32 = 200.0;
const WORKSPACE_PANEL_MAX_WIDTH: f32 = 360.0;
pub(super) const WORKSPACE_COMPACT_BREAKPOINT: f32 = 900.0;
const WORKSPACE_COMPACT_OVERLAY_WIDTH: f32 = 280.0;
pub(super) const DOCUMENT_SIDEBAR_PANEL_AUTO_MIN_WIDTH: f32 = 240.0;
pub(super) const DOCUMENT_SIDEBAR_PANEL_MIN_WIDTH: f32 = 200.0;
pub(super) const DOCUMENT_SIDEBAR_PANEL_MAX_WIDTH: f32 = 360.0;
pub(super) const DOCUMENT_SIDEBAR_COMPACT_OVERLAY_WIDTH: f32 = 280.0;
const WORKSPACE_RESIZE_HIT_WIDTH: f32 = 7.0;
const WORKSPACE_RESIZE_KEYBOARD_STEP: f32 = 4.0;
const WORKSPACE_RESIZE_KEYBOARD_LARGE_STEP: f32 = 16.0;
/// Dense navigation rows remain compact while preserving a comfortable 30px
/// pointer target and a clear hierarchy at nested depths.
const WORKSPACE_NODE_HEIGHT: f32 = 30.0;
const WORKSPACE_NODE_INDENT: f32 = 16.0;
const SEARCH_DEBOUNCE: Duration = Duration::from_millis(250);
const QUICK_OPEN_DEBOUNCE: Duration = Duration::from_millis(30);
const TOOLTIP_DELAY: Duration = Duration::from_millis(500);
const QUICK_OPEN_MAX_RESULTS: usize = 100;
const SEARCH_MAX_RESULTS: usize = 500;
const SEARCH_MAX_FILE_BYTES: u64 = 5 * 1024 * 1024;
/// Bound the amount of work a user-selected folder can cause before it reaches the UI.
const WORKSPACE_SCAN_MAX_ENTRIES: usize = 20_000;
/// Keep tree construction and keyboard traversal bounded even for pathological paths.
const WORKSPACE_SCAN_MAX_DEPTH: usize = 64;

fn workspace_status_row(
    id: &'static str,
    icon_selector: &'static str,
    icon: &'static str,
    message: String,
    color: Hsla,
    font_size: f32,
) -> Stateful<Div> {
    div()
        .id(id)
        .debug_selector(move || id.to_owned())
        .min_h(px(22.0))
        .w_full()
        .px(px(8.0))
        .py(px(2.0))
        .flex()
        .items_center()
        .gap(px(7.0))
        .text_size(px(font_size))
        .text_color(color)
        .child(
            div()
                .id(icon_selector)
                .debug_selector(move || icon_selector.to_owned())
                .size(px(18.0))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    svg()
                        .path(icon)
                        .size(px(14.0))
                        .text_color(color)
                        .debug_selector(move || format!("{icon_selector}-svg")),
                ),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .line_height(px(font_size * 1.35))
                .truncate()
                .child(message),
        )
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum WorkspaceTab {
    #[default]
    Files,
    /// 旧工作区大纲已迁移到右侧文档导航；保留内部状态仅用于兼容既有会话测试。
    // reason: 旧会话仍可能包含 Outline；remove when: 会话迁移不再接受该标签且兼容测试删除。
    #[allow(dead_code)]
    Outline,
    Search,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct WorkspaceSearchOptions {
    case_sensitive: bool,
    whole_word: bool,
    regex: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct WorkspaceSearchMatch {
    path: PathBuf,
    relative_path: String,
    line: usize,
    column: usize,
    preview: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct QuickOpenMatch {
    path: PathBuf,
    relative_path: String,
    score: i64,
}

struct QuickOpenState {
    input: Entity<Block>,
    results: Vec<QuickOpenMatch>,
    selected: usize,
    running: bool,
    generation: u64,
    task: Option<Task<()>>,
}

/// 描述文件树扫描的可观察生命周期，避免渲染层用多个易失字段猜测后台任务状态。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) enum WorkspaceScanState {
    #[default]
    Idle,
    Scanning {
        generation: u64,
    },
    Ready {
        generation: u64,
    },
    Failed {
        generation: u64,
        error: String,
    },
}

struct WorkspaceScanResult {
    root: PathBuf,
    tree: WorkspaceTreeNode,
    quick_open_paths: Vec<PathBuf>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum WorkspaceOperationKind {
    Rename,
    Move,
    NewFile,
    NewFolder,
    Delete,
}

#[derive(Clone)]
enum WorkspacePendingPlan {
    Move(super::workspace_file_ops::WorkspaceMovePlan),
    Create(super::workspace_file_ops::WorkspaceCreatePlan),
    Delete(super::workspace_file_ops::WorkspaceDeletePlan),
}

#[derive(Clone)]
enum WorkspaceUndoOperation {
    Move(super::workspace_file_ops::WorkspaceMovePlan),
    Create(super::workspace_file_ops::WorkspaceCreatePlan),
}

struct WorkspaceOperationDialog {
    kind: WorkspaceOperationKind,
    source: PathBuf,
    input: Entity<Block>,
    plan: Option<WorkspacePendingPlan>,
    error: Option<String>,
    running: bool,
}

#[derive(Clone)]
struct WorkspaceDragPayload {
    path: PathBuf,
    label: String,
    background: Hsla,
    text: Hsla,
}

struct WorkspaceDragPreview {
    payload: WorkspaceDragPayload,
    position: Point<Pixels>,
}

impl Render for WorkspaceDragPreview {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .absolute()
            .left(self.position.x + px(10.0))
            .top(self.position.y + px(10.0))
            .max_w(px(260.0))
            .px(px(10.0))
            .h(px(WORKSPACE_NODE_HEIGHT))
            .flex()
            .items_center()
            .overflow_hidden()
            .truncate()
            .rounded(px(6.0))
            .shadow_md()
            .bg(self.payload.background)
            .text_color(self.payload.text)
            .child(self.payload.label.clone())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum WorkspaceTreeKind {
    Directory(PathBuf),
    File(PathBuf),
    Heading { line: usize, level: u8 },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct WorkspaceTreeNode {
    id: String,
    label: String,
    kind: WorkspaceTreeKind,
    children: Vec<WorkspaceTreeNode>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum WorkspaceSelection {
    File(PathBuf),
    Outline(String),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum WorkspaceKeyboardZone {
    #[default]
    Tabs,
    Body,
    SearchOptions,
    SearchResults,
}

#[derive(Clone)]
struct WorkspaceKeyboardNode {
    id: String,
    kind: WorkspaceTreeKind,
    has_children: bool,
    parent_id: Option<String>,
}

#[derive(Default)]
pub(super) struct WorkspaceState {
    pub(super) is_open: bool,
    /// `None` preserves the product default: docked workspace visible on wide windows.
    docked_open_preference: Option<bool>,
    /// Tracks breakpoint transitions without treating compact overlay visibility as a preference.
    compact_layout: Option<bool>,
    active_tab: WorkspaceTab,
    tooltip_hovered: Option<&'static str>,
    tooltip_visible: Option<&'static str>,
    tooltip_task: Option<Task<()>>,
    root: Option<PathBuf>,
    explicit_root: Option<PathBuf>,
    file_tree: Option<WorkspaceTreeNode>,
    file_error: Option<String>,
    file_scan_task: Option<Task<()>>,
    file_scan_generation: u64,
    file_scanning: bool,
    file_scan_state: WorkspaceScanState,
    file_scan_requested_root: Option<PathBuf>,
    file_scan_show_hidden_files: bool,
    file_scan_cancel: Option<Arc<AtomicBool>>,
    quick_open_paths: Vec<PathBuf>,
    outline_tree: Vec<WorkspaceTreeNode>,
    outline_source: Option<String>,
    outline_revision: Option<(u64, gmark_document::Revision)>,
    outline_requested_source: Option<String>,
    outline_requested_revision: Option<(u64, gmark_document::Revision)>,
    outline_generation: u64,
    outline_task: Option<Task<()>>,
    outline_running: bool,
    expanded: HashSet<String>,
    pinned_empty_directories: HashSet<PathBuf>,
    selected: Option<WorkspaceSelection>,
    search_input: Option<Entity<Block>>,
    search_options: WorkspaceSearchOptions,
    search_results: Vec<WorkspaceSearchMatch>,
    search_error: Option<String>,
    search_running: bool,
    search_generation: u64,
    search_task: Option<Task<()>>,
    pending_navigation: Option<(PathBuf, usize)>,
    operation_dialog: Option<WorkspaceOperationDialog>,
    file_operation_task: Option<Task<()>>,
    file_operation_generation: u64,
    undo_file_operation: Option<WorkspaceUndoOperation>,
    operation_error: Option<String>,
    quick_open: Option<QuickOpenState>,
    focus_handle: Option<FocusHandle>,
    keyboard_zone: WorkspaceKeyboardZone,
    search_selected: usize,
    panel_scroll: ScrollHandle,
    panel_width: Option<f32>,
    resize_session: Option<WorkspaceResizeSession>,
    resize_focus_handle: Option<FocusHandle>,
    /// 文档导航是独立于工作区文件树的右侧上下文面板。
    pub(super) document_sidebar_open: bool,
    document_sidebar_docked_open_preference: Option<bool>,
    document_sidebar_compact_layout: Option<bool>,
    document_sidebar_panel_width: Option<f32>,
    pub(super) document_sidebar_scroll: ScrollHandle,
    pub(super) document_sidebar_focus_handle: Option<FocusHandle>,
    document_sidebar_resize_session: Option<WorkspaceResizeSession>,
    document_sidebar_resize_focus_handle: Option<FocusHandle>,
}

#[derive(Clone, Copy, Debug)]
struct WorkspaceResizeSession {
    start_x: Pixels,
    start_width: f32,
    direction: f32,
}

impl Editor {}

#[path = "workspace_parts/controller.rs"]
mod controller;
#[path = "workspace/navigation/file_operations.rs"]
mod file_operations;
#[path = "workspace_parts/input.rs"]
mod input;
#[path = "workspace_parts/navigation.rs"]
mod navigation;
#[path = "workspace_parts/operations.rs"]
mod operations;
#[path = "workspace/scanning.rs"]
mod scanning;
#[path = "workspace_parts/search_view.rs"]
mod search_view;
#[path = "workspace_parts/tooltip.rs"]
mod tooltip;

#[cfg(test)]
use scanning::{collect_markdown_paths, scan_workspace_dir};
use scanning::{
    insert_workspace_directory, insert_workspace_file, remove_workspace_path, scan_workspace,
    sort_workspace_tree, stable_node_hash, workspace_path_is_hidden,
};

use tooltip::render_workspace_tooltip;

pub(super) fn is_markdown_file(path: &Path) -> bool {
    crate::document_io::is_markdown_path(path)
}

fn rank_quick_open_paths(root: &Path, paths: Vec<PathBuf>, query: &str) -> Vec<QuickOpenMatch> {
    let mut matches = paths
        .into_iter()
        .filter_map(|path| {
            let relative_path = path
                .strip_prefix(root)
                .unwrap_or(path.as_path())
                .to_string_lossy()
                .replace('\\', "/");
            let file_name = path
                .file_name()
                .map(|name| name.to_string_lossy())
                .unwrap_or_default();
            let score = if query.is_empty() {
                0
            } else {
                let path_score = subsequence_score(&relative_path, query)?;
                let file_score = subsequence_score(&file_name, query)
                    .map(|score| score + 2_000)
                    .unwrap_or(i64::MIN);
                path_score.max(file_score)
            };
            Some(QuickOpenMatch {
                path,
                relative_path,
                score,
            })
        })
        .collect::<Vec<_>>();
    matches.sort_by(|left, right| {
        right.score.cmp(&left.score).then_with(|| {
            left.relative_path
                .to_lowercase()
                .cmp(&right.relative_path.to_lowercase())
        })
    });
    matches.truncate(QUICK_OPEN_MAX_RESULTS);
    matches
}

pub(super) fn subsequence_score(candidate: &str, query: &str) -> Option<i64> {
    let candidate_chars = candidate.chars().collect::<Vec<_>>();
    let query_chars = query
        .chars()
        .flat_map(char::to_lowercase)
        .collect::<Vec<_>>();
    if query_chars.is_empty() {
        return Some(0);
    }
    let mut query_index = 0usize;
    let mut score = 0i64;
    let mut previous_match = None;
    for (index, ch) in candidate_chars.iter().enumerate() {
        if ch
            .to_lowercase()
            .eq(std::iter::once(query_chars[query_index]))
        {
            score += 100;
            if index == 0
                || candidate_chars
                    .get(index.wrapping_sub(1))
                    .is_some_and(|previous| matches!(*previous, '/' | '\\' | '-' | '_' | ' '))
            {
                score += 60;
            }
            if previous_match == Some(index.saturating_sub(1)) {
                score += 80;
            } else if let Some(previous) = previous_match {
                score -= i64::try_from(index.saturating_sub(previous + 1)).unwrap_or(i64::MAX);
            }
            previous_match = Some(index);
            query_index += 1;
            if query_index == query_chars.len() {
                return Some(score - i64::try_from(candidate_chars.len()).unwrap_or(i64::MAX));
            }
        }
    }
    None
}

impl WorkspaceState {
    /// 返回扫描阶段生成的 Markdown 快照；补全不得在每次按键时重新遍历文件树。
    pub(super) fn markdown_snapshot(&self) -> Option<(PathBuf, Vec<PathBuf>)> {
        let root = self.root.clone()?;
        self.file_tree.as_ref()?;
        Some((root, self.quick_open_paths.clone()))
    }

    /// 从后台扫描交付的树快照判断路径类型，避免菜单渲染在 UNC/慢盘上同步查询元数据。
    pub(super) fn snapshot_path_is_directory(&self, target: &Path) -> Option<bool> {
        fn find_kind(node: &WorkspaceTreeNode, target: &Path) -> Option<bool> {
            match &node.kind {
                WorkspaceTreeKind::Directory(path) if path == target => Some(true),
                WorkspaceTreeKind::File(path) if path == target => Some(false),
                _ => node
                    .children
                    .iter()
                    .find_map(|child| find_kind(child, target)),
            }
        }

        self.file_tree
            .as_ref()
            .and_then(|tree| find_kind(tree, target))
    }

    /// 只接受已完成快照中的文件，未知路径保持不可打开，避免渲染回调触发磁盘 IO。
    pub(super) fn snapshot_path_is_file(&self, target: &Path) -> bool {
        matches!(self.snapshot_path_is_directory(target), Some(false))
    }

    /// 增量同步新建路径，避免小型文件操作为了 Quick Open 再次触发整棵目录扫描。
    pub(super) fn insert_created_path(
        &mut self,
        root: &Path,
        path: &Path,
        kind: super::workspace_file_ops::WorkspaceCreateKind,
    ) -> bool {
        if self.root.as_deref() != Some(root) {
            return false;
        }
        if !self.file_scan_show_hidden_files && workspace_path_is_hidden(root, path) {
            return true;
        }
        let Some(tree) = self.file_tree.as_mut() else {
            return false;
        };
        match kind {
            super::workspace_file_ops::WorkspaceCreateKind::File => {
                insert_workspace_file(tree, root, path);
                if is_markdown_file(path)
                    && !self
                        .quick_open_paths
                        .iter()
                        .any(|candidate| candidate == path)
                {
                    self.quick_open_paths.push(path.to_path_buf());
                }
            }
            super::workspace_file_ops::WorkspaceCreateKind::Directory => {
                insert_workspace_directory(tree, root, path);
            }
        }
        sort_workspace_tree(tree);
        true
    }

    /// 从树和 Markdown 平面索引同时移除路径，保持后续导航不读到已删除文件。
    pub(super) fn remove_path(&mut self, path: &Path) -> bool {
        let Some(tree) = self.file_tree.as_mut() else {
            return false;
        };
        let removed = remove_workspace_path(tree, path, &self.pinned_empty_directories);
        if removed {
            self.quick_open_paths
                .retain(|candidate| !candidate.starts_with(path));
        }
        removed
    }

    #[cfg(test)]
    // Tests install a deterministic snapshot so link completion does not touch the filesystem.
    pub(super) fn install_markdown_snapshot_for_test(
        &mut self,
        root: PathBuf,
        paths: Vec<PathBuf>,
    ) {
        self.root = Some(root.clone());
        self.quick_open_paths = paths.clone();
        self.file_tree = Some(WorkspaceTreeNode {
            id: root.to_string_lossy().to_string(),
            label: root
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| root.to_string_lossy().to_string()),
            kind: WorkspaceTreeKind::Directory(root),
            children: paths
                .into_iter()
                .map(|path| WorkspaceTreeNode {
                    id: path.to_string_lossy().to_string(),
                    label: path
                        .file_name()
                        .map(|name| name.to_string_lossy().to_string())
                        .unwrap_or_default(),
                    kind: WorkspaceTreeKind::File(path),
                    children: Vec::new(),
                })
                .collect(),
        });
    }
}

fn search_workspace(
    root: &Path,
    query: &str,
    options: WorkspaceSearchOptions,
    show_hidden_files: bool,
) -> Result<Vec<WorkspaceSearchMatch>, String> {
    let source_pattern = if options.regex {
        query.to_owned()
    } else {
        regex::escape(query)
    };
    let pattern = if options.whole_word {
        format!(r"(?u:\b(?:{source_pattern})\b)")
    } else {
        source_pattern
    };
    let matcher = regex::RegexBuilder::new(&pattern)
        .case_insensitive(!options.case_sensitive)
        .unicode(true)
        .build()
        .map_err(|error| error.to_string())?;
    let mut results = Vec::new();
    let walker = ignore::WalkBuilder::new(root)
        .hidden(!show_hidden_files)
        .follow_links(false)
        .git_ignore(true)
        .git_exclude(true)
        .require_git(false)
        .parents(true)
        .build();

    for entry in walker {
        if results.len() >= SEARCH_MAX_RESULTS {
            break;
        }
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => continue,
        };
        let path = entry.path();
        if !entry.file_type().is_some_and(|kind| kind.is_file()) || !is_markdown_file(path) {
            continue;
        }
        if entry
            .metadata()
            .map(|metadata| metadata.len() > SEARCH_MAX_FILE_BYTES)
            .unwrap_or(true)
        {
            continue;
        }
        let Ok(bytes) = fs::read(path) else {
            continue;
        };
        let Ok(opened) = crate::document_io::decode_markdown_bytes(&bytes) else {
            continue;
        };
        let content = opened.text;
        let relative_path = path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        for (line_index, line) in content.lines().enumerate() {
            for found in matcher.find_iter(line) {
                let column = line[..found.start()].chars().count() + 1;
                results.push(WorkspaceSearchMatch {
                    path: path.to_path_buf(),
                    relative_path: relative_path.clone(),
                    line: line_index + 1,
                    column,
                    preview: truncate_search_preview(line),
                });
                if results.len() >= SEARCH_MAX_RESULTS {
                    break;
                }
            }
            if results.len() >= SEARCH_MAX_RESULTS {
                break;
            }
        }
    }
    Ok(results)
}

fn truncate_search_preview(line: &str) -> String {
    const MAX_CHARS: usize = 240;
    let mut chars = line.trim().chars();
    let preview = chars.by_ref().take(MAX_CHARS).collect::<String>();
    if chars.next().is_some() {
        format!("{preview}...")
    } else {
        preview
    }
}

/// 在后台一次性建立规范化树和 Quick Open 平面索引，避免 UI 线程接触目录 IO 或递归排序。
#[path = "workspace/layout_outline.rs"]
mod layout_outline;
use layout_outline::{build_outline_tree, collect_visible_keyboard_nodes, prune_outline_state};
pub(crate) use layout_outline::{
    document_sidebar_panel_width_for_viewport, workspace_panel_width_for_viewport,
    workspace_uses_overlay,
};

#[cfg(test)]
#[path = "../../tests/unit/editor/workspace.rs"]
mod tests;
