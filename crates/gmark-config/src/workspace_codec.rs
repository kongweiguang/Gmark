// @author kongweiguang

//! `workspace-session.json` 的版本兼容解码、迁移和安全归一化。

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

use anyhow::{Result, bail};
use serde::Deserialize;

use crate::workspace_session::{
    WorkspaceSession, WorkspaceSessionDocumentRef, WorkspaceSessionPane, WorkspaceSessionPaneId,
    WorkspaceSessionPaneNode, WorkspaceSessionPaneViewState, WorkspaceSessionRegistry,
    WorkspaceSessionSelection, WorkspaceSessionTab, WorkspaceSessionWindow,
};

const LEGACY_SESSION_VERSION: u32 = 1;
const REGISTRY_VERSION_V2: u32 = 2;
const REGISTRY_VERSION_V3: u32 = 3;
const REGISTRY_VERSION_V4: u32 = 4;
const REGISTRY_VERSION_V5: u32 = 5;
const REGISTRY_VERSION_V6: u32 = 6;
const REGISTRY_VERSION_V7: u32 = 7;
const REGISTRY_VERSION_V8: u32 = 8;
const TRANSITIONAL_NON_BOARD_REGISTRY_VERSION: u32 = 9;
pub(crate) const CURRENT_REGISTRY_VERSION: u32 = 10;
pub(crate) const SESSION_FILE_LIMIT: u64 = 1024 * 1024;
pub(crate) const SESSION_TAB_LIMIT: usize = 100;
pub(crate) const SESSION_WINDOW_LIMIT: usize = 20;
pub(crate) const SESSION_PANE_LEAF_LIMIT: usize = 8;
pub(crate) const SESSION_HISTORY_LIMIT: usize = 32;
const SESSION_OPAQUE_MAX_DEPTH: usize = 16;
const SESSION_OPAQUE_MAX_BYTES: usize = 64 * 1024;
const SESSION_OPAQUE_MAX_ITEMS: usize = 128;
const SESSION_OPAQUE_MAX_STRING: usize = 4096;

#[derive(Clone, Debug, Deserialize)]
struct LegacyWorkspaceSession {
    version: u32,
    tabs: Vec<LegacyWorkspaceSessionTab>,
    active_index: usize,
    #[serde(default)]
    workspace_root: Option<PathBuf>,
}

#[derive(Clone, Debug, Deserialize)]
struct LegacyWorkspaceSessionRegistry {
    version: u32,
    windows: Vec<LegacyWorkspaceSessionWindow>,
}

#[derive(Clone, Debug, Deserialize)]
struct LegacyWorkspaceSessionWindow {
    id: uuid::Uuid,
    tabs: Vec<LegacyWorkspaceSessionTab>,
    active_index: usize,
    #[serde(default)]
    workspace_root: Option<PathBuf>,
    #[serde(default)]
    window: Option<WorkspaceSessionWindow>,
    #[serde(default)]
    workspace_panel_width: Option<f32>,
    #[serde(default)]
    workspace_docked_open: Option<bool>,
    #[serde(default)]
    document_sidebar_width: Option<f32>,
    #[serde(default)]
    document_sidebar_docked_open: Option<bool>,
    #[serde(default)]
    split_pane_ratio: Option<f32>,
}

#[derive(Clone, Debug, Deserialize)]
struct LegacyWorkspaceSessionTab {
    path: PathBuf,
    #[serde(default)]
    pinned: bool,
    #[serde(default)]
    view_mode: Option<String>,
    #[serde(default)]
    selection: Option<WorkspaceSessionSelection>,
    #[serde(default)]
    scroll_x: Option<f32>,
    #[serde(default)]
    scroll_y: Option<f32>,
}

/// 解码 v1-v10 registry。旧版本在此转换为 v10 root+map 结构。
pub(crate) fn decode_registry(bytes: &[u8]) -> Result<WorkspaceSessionRegistry> {
    let value: serde_json::Value = serde_json::from_slice(bytes)?;
    let version = value
        .get("version")
        .and_then(serde_json::Value::as_u64)
        .and_then(|version| u32::try_from(version).ok())
        .unwrap_or_default();
    match version {
        CURRENT_REGISTRY_VERSION => {
            let mut registry: WorkspaceSessionRegistry = serde_json::from_value(value)?;
            registry.version = CURRENT_REGISTRY_VERSION;
            Ok(registry)
        }
        TRANSITIONAL_NON_BOARD_REGISTRY_VERSION => {
            if value_contains_board_marker(&value) {
                bail!("workspace session registry version {version} contains Board-owned data");
            }
            decode_legacy_registry(value, version)
        }
        LEGACY_SESSION_VERSION => {
            let legacy: LegacyWorkspaceSession = serde_json::from_value(value)?;
            if legacy.version != LEGACY_SESSION_VERSION {
                bail!("invalid legacy workspace session version");
            }
            let tabs = legacy.tabs.into_iter().map(legacy_tab).collect::<Vec<_>>();
            Ok(WorkspaceSessionRegistry {
                version: CURRENT_REGISTRY_VERSION,
                windows: vec![legacy_session(
                    uuid::Uuid::new_v4(),
                    tabs,
                    legacy.active_index,
                    legacy.workspace_root,
                )],
            })
        }
        REGISTRY_VERSION_V2 | REGISTRY_VERSION_V3 | REGISTRY_VERSION_V4 | REGISTRY_VERSION_V5
        | REGISTRY_VERSION_V6 | REGISTRY_VERSION_V7 | REGISTRY_VERSION_V8 => {
            decode_legacy_registry(value, version)
        }
        version => bail!("unsupported workspace session registry version {version}"),
    }
}

fn decode_legacy_registry(
    value: serde_json::Value,
    version: u32,
) -> Result<WorkspaceSessionRegistry> {
    let legacy: LegacyWorkspaceSessionRegistry = serde_json::from_value(value)?;
    if legacy.version != version {
        bail!("invalid legacy workspace session version");
    }
    let windows = legacy
        .windows
        .into_iter()
        .map(|window| {
            let tabs = window.tabs.into_iter().map(legacy_tab).collect::<Vec<_>>();
            let mut session =
                legacy_session(window.id, tabs, window.active_index, window.workspace_root);
            session.window = window.window;
            session.workspace_panel_width = window.workspace_panel_width;
            session.workspace_docked_open = window.workspace_docked_open;
            session.document_sidebar_width = window.document_sidebar_width;
            session.document_sidebar_docked_open = window.document_sidebar_docked_open;
            session.split_pane_ratio = window.split_pane_ratio;
            session
        })
        .collect::<Vec<_>>();
    Ok(WorkspaceSessionRegistry {
        version: CURRENT_REGISTRY_VERSION,
        windows,
    })
}

fn legacy_tab(tab: LegacyWorkspaceSessionTab) -> WorkspaceSessionTab {
    let mut value = WorkspaceSessionTab::new(tab.path, tab.pinned);
    value.state.view_mode = tab.view_mode;
    value.state.selection = tab.selection;
    value.state.scroll_x = tab.scroll_x;
    value.state.scroll_y = tab.scroll_y;
    value
}

/// Migrate the private flat DTO used by v1-v9 into the canonical single-leaf
/// representation.  No legacy shape is exposed by the public session model.
fn legacy_session(
    id: uuid::Uuid,
    tabs: Vec<WorkspaceSessionTab>,
    active_index: usize,
    workspace_root: Option<PathBuf>,
) -> WorkspaceSession {
    let mut session = WorkspaceSession::single_pane(id, workspace_root);
    let pane_id = session.focused_pane;
    let active_tab = tabs
        .get(active_index.min(tabs.len().saturating_sub(1)))
        .map(|tab| tab.id);
    session
        .panes
        .insert(pane_id, WorkspaceSessionPane::new(tabs, active_tab));
    session
}

// A short-lived build wrote ordinary sessions as v9 before that version was reserved for Board.
// Accept only the data shape owned by main; silently dropping Board state would corrupt recovery.
fn value_contains_board_marker(value: &serde_json::Value) -> bool {
    let windows_have_marker = value
        .get("windows")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|window| window.get("tabs").and_then(serde_json::Value::as_array))
        .flatten()
        .any(|tab| {
            tab.get("board").is_some()
                || tab.get("view_mode").and_then(serde_json::Value::as_str) == Some("board")
        });
    let legacy_have_marker = value
        .get("tabs")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .any(|tab| {
            tab.get("board").is_some()
                || tab.get("view_mode").and_then(serde_json::Value::as_str) == Some("board")
        });
    windows_have_marker || legacy_have_marker
}

/// 判断原始内容是否需要创建 v10 迁移备份。
pub(crate) fn is_pre_v10_bytes(bytes: &[u8]) -> bool {
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(bytes) else {
        return false;
    };
    value
        .get("version")
        .and_then(serde_json::Value::as_u64)
        .is_some_and(|version| version < u64::from(CURRENT_REGISTRY_VERSION))
}

/// 归一化并校验 registry 的 pane 引用关系。
pub(crate) fn normalize_registry(
    mut registry: WorkspaceSessionRegistry,
) -> Result<WorkspaceSessionRegistry> {
    if registry.version != CURRENT_REGISTRY_VERSION {
        bail!(
            "unsupported workspace session registry version {}",
            registry.version
        );
    }
    if registry.windows.len() > SESSION_WINDOW_LIMIT {
        bail!("workspace session registry exceeds the 20 window safety limit");
    }

    let mut seen_windows = HashSet::new();
    let mut windows = Vec::with_capacity(registry.windows.len());
    // 保持旧行为：后写窗口拥有重复窗口 ID。
    for session in registry.windows.into_iter().rev() {
        if !seen_windows.insert(session.id) {
            continue;
        }
        let session = normalize_session(session)?;
        if session.has_tabs() {
            windows.push(session);
        }
    }
    windows.reverse();
    registry.windows = windows;
    Ok(registry)
}

/// 归一化一个 v10 session。
pub(crate) fn normalize_session(mut session: WorkspaceSession) -> Result<WorkspaceSession> {
    if session.panes.is_empty() {
        bail!("workspace session has no panes");
    }

    let mut leaves = Vec::new();
    let mut referenced = HashSet::new();
    normalize_tree(
        &mut session.root,
        &mut session.panes,
        &mut referenced,
        &mut leaves,
    )?;
    if leaves.is_empty() || leaves.len() > SESSION_PANE_LEAF_LIMIT {
        bail!("workspace session exceeds the 8 pane leaf safety limit");
    }
    if referenced.len() != session.panes.len() {
        bail!("workspace session pane map contains an unreachable pane");
    }
    if !referenced.contains(&session.focused_pane) {
        bail!("workspace session focused pane is not in the pane tree");
    }

    let mut tab_ids = HashSet::new();
    for pane_id in &leaves {
        let Some(pane) = session.panes.get_mut(pane_id) else {
            bail!("workspace session pane map is missing a leaf");
        };
        normalize_pane(pane, &mut tab_ids)?;
    }
    if session
        .workspace_root
        .as_ref()
        .is_some_and(|root| path_is_empty(root))
    {
        session.workspace_root = None;
    }
    session.window = session.window.and_then(normalize_window);
    session.workspace_panel_width = normalize_width(session.workspace_panel_width);
    session.document_sidebar_width = normalize_width(session.document_sidebar_width);
    session.split_pane_ratio = session
        .split_pane_ratio
        .filter(|ratio| ratio.is_finite())
        .map(|ratio| ratio.clamp(0.3, 0.7));
    // v1-v9 stored this split ratio at window level. Preserve it in the focused
    // view state as the v10 source of truth while retaining the legacy field for
    // readers that still expose it in the window contract.
    if let Some(ratio) = session.split_pane_ratio
        && let Some(pane) = session.panes.get_mut(&session.focused_pane)
        && let Some(tab) = pane
            .active_tab
            .and_then(|id| pane.tabs.iter_mut().find(|tab| tab.id == id))
        && tab.state.split_ratio.is_none()
    {
        tab.state.split_ratio = Some(ratio.clamp(0.1, 0.9));
    }
    Ok(session)
}

fn normalize_tree(
    node: &mut WorkspaceSessionPaneNode,
    panes: &mut std::collections::BTreeMap<WorkspaceSessionPaneId, WorkspaceSessionPane>,
    referenced: &mut HashSet<WorkspaceSessionPaneId>,
    leaves: &mut Vec<WorkspaceSessionPaneId>,
) -> Result<()> {
    match node {
        WorkspaceSessionPaneNode::Leaf(id) => {
            if id.as_uuid().is_nil() {
                bail!("workspace session pane tree contains a nil pane id");
            }
            if !panes.contains_key(id) {
                bail!("workspace session pane tree references a missing pane");
            }
            if !referenced.insert(*id) {
                bail!("workspace session pane tree references a pane more than once");
            }
            leaves.push(*id);
        }
        WorkspaceSessionPaneNode::Split {
            ratio,
            first,
            second,
            ..
        } => {
            *ratio = if ratio.is_finite() {
                ratio.clamp(0.1, 0.9)
            } else {
                0.5
            };
            normalize_tree(first, panes, referenced, leaves)?;
            normalize_tree(second, panes, referenced, leaves)?;
        }
    }
    Ok(())
}

fn normalize_pane(
    pane: &mut WorkspaceSessionPane,
    tab_ids: &mut HashSet<uuid::Uuid>,
) -> Result<()> {
    if pane.tabs.len() > SESSION_TAB_LIMIT {
        bail!("workspace session pane exceeds the 100 tab safety limit");
    }
    let active_tab = pane.active_tab;
    let mut seen_paths = HashSet::new();
    let mut tabs: Vec<WorkspaceSessionTab> = Vec::with_capacity(pane.tabs.len());
    for mut tab in pane.tabs.drain(..) {
        if tab.id.is_nil() {
            bail!("workspace session contains a nil tab view instance id");
        }
        if !tab_ids.insert(tab.id) {
            bail!("workspace session contains duplicate tab view instance IDs");
        }
        normalize_view_state(&mut tab.state)?;
        let keep = match &tab.document {
            WorkspaceSessionDocumentRef::File(path) => {
                !path_is_empty(path) && seen_paths.insert(path_identity(path))
            }
            WorkspaceSessionDocumentRef::Recovery(document_id) => {
                if document_id.is_nil() {
                    bail!("workspace session contains a nil recovery document id");
                }
                true
            }
        };
        if keep {
            tabs.push(tab);
        }
    }
    tabs.sort_by_key(|tab| !tab.pinned);
    pane.tabs = tabs;
    pane.active_tab = active_tab
        .filter(|id| pane.tabs.iter().any(|tab| tab.id == *id))
        .or_else(|| pane.tabs.first().map(|tab| tab.id));
    Ok(())
}

fn normalize_view_state(state: &mut WorkspaceSessionPaneViewState) -> Result<()> {
    if let Some(selection) = state.selection.as_mut()
        && selection.start > selection.end
    {
        std::mem::swap(&mut selection.start, &mut selection.end);
    }
    state.view_mode = state
        .view_mode
        .take()
        .filter(|mode| matches!(mode.as_str(), "live" | "source" | "preview" | "split"));
    state.split_ratio = state
        .split_ratio
        .filter(|ratio| ratio.is_finite())
        .map(|ratio| ratio.clamp(0.1, 0.9));
    state.scroll_x = state
        .scroll_x
        .filter(|value| value.is_finite())
        .map(|value| value.clamp(-10_000_000.0, 10_000_000.0));
    state.scroll_y = state
        .scroll_y
        .filter(|value| value.is_finite())
        .map(|value| value.clamp(-10_000_000.0, 10_000_000.0));
    clamp_history(&mut state.forward);
    clamp_history(&mut state.back);
    if let Some(value) = state.markdown_fold.as_ref() {
        validate_opaque(value, 0, "markdown_fold")?;
    }
    if state.markdown_folds.len() > SESSION_HISTORY_LIMIT {
        bail!("workspace session markdown fold state exceeds the 32 item limit");
    }
    for value in &state.markdown_folds {
        validate_opaque(value, 0, "markdown_folds")?;
    }
    if let Some(value) = state.table_layout.as_ref() {
        validate_opaque(value, 0, "table_layout")?;
    }
    for value in &state.forward {
        validate_opaque(value, 0, "forward")?;
    }
    for value in &state.back {
        validate_opaque(value, 0, "back")?;
    }
    Ok(())
}

fn clamp_history(history: &mut Vec<serde_json::Value>) {
    if history.len() > SESSION_HISTORY_LIMIT {
        let excess = history.len() - SESSION_HISTORY_LIMIT;
        history.drain(0..excess);
    }
}

fn validate_opaque(value: &serde_json::Value, depth: usize, field: &str) -> Result<()> {
    if depth > SESSION_OPAQUE_MAX_DEPTH {
        bail!("workspace session {field} exceeds the opaque state depth limit");
    }
    let encoded = serde_json::to_vec(value)?;
    if encoded.len() > SESSION_OPAQUE_MAX_BYTES {
        bail!("workspace session {field} exceeds the opaque state size limit");
    }
    match value {
        serde_json::Value::Array(values) => {
            if values.len() > SESSION_OPAQUE_MAX_ITEMS {
                bail!("workspace session {field} exceeds the opaque state item limit");
            }
            for value in values {
                validate_opaque(value, depth + 1, field)?;
            }
        }
        serde_json::Value::Object(values) => {
            if values.len() > SESSION_OPAQUE_MAX_ITEMS {
                bail!("workspace session {field} exceeds the opaque state item limit");
            }
            for (key, value) in values {
                if key.len() > SESSION_OPAQUE_MAX_STRING {
                    bail!("workspace session {field} contains an oversized key");
                }
                validate_opaque(value, depth + 1, field)?;
            }
        }
        serde_json::Value::String(value) if value.len() > SESSION_OPAQUE_MAX_STRING => {
            bail!("workspace session {field} contains an oversized string");
        }
        serde_json::Value::Number(value)
            if value.as_f64().is_some_and(|number| !number.is_finite()) =>
        {
            bail!("workspace session {field} contains a non-finite number");
        }
        _ => {}
    }
    Ok(())
}

fn normalize_width(width: Option<f32>) -> Option<f32> {
    width
        .filter(|width| width.is_finite())
        .map(|width| width.clamp(200.0, 360.0))
}

fn normalize_window(mut window: WorkspaceSessionWindow) -> Option<WorkspaceSessionWindow> {
    if !window.x.is_finite()
        || !window.y.is_finite()
        || !window.width.is_finite()
        || !window.height.is_finite()
        || window.width <= 0.0
        || window.height <= 0.0
    {
        return None;
    }
    // 会话文件来自崩溃恢复边界，先限制异常坐标与尺寸，显示器级裁剪由宿主完成。
    window.x = window.x.clamp(-1_000_000.0, 1_000_000.0);
    window.y = window.y.clamp(-1_000_000.0, 1_000_000.0);
    window.width = window.width.clamp(720.0, 32_768.0);
    window.height = window.height.clamp(520.0, 32_768.0);
    Some(window)
}

fn path_is_empty(path: &Path) -> bool {
    path.as_os_str().is_empty() || path.to_string_lossy().trim().is_empty()
}

/// 用统一词法身份匹配会话排除路径，避免在启动恢复边界触发文件系统查询。
pub(crate) fn path_identity(path: &Path) -> String {
    let value = path.to_string_lossy();
    if cfg!(windows) {
        normalize_windows_path_identity(&value)
    } else {
        value.into_owned()
    }
}

/// 只在词法层归一 Windows 路径，避免会话过滤为识别同一路径而访问慢盘。
fn normalize_windows_path_identity(path: &str) -> String {
    let normalized = path.replace('/', "\\").to_lowercase();
    if let Some(unc_path) = normalized.strip_prefix("\\\\?\\unc\\") {
        return format!("\\\\{unc_path}");
    }

    if let Some(drive_path) = normalized.strip_prefix("\\\\?\\") {
        let bytes = drive_path.as_bytes();
        if bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && bytes[2] == b'\\'
        {
            return drive_path.to_owned();
        }
    }

    normalized
}
