// @author kongweiguang

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context as _, Result, anyhow, bail};

use super::{
    WORKSPACE_SCAN_MAX_DEPTH, WORKSPACE_SCAN_MAX_ENTRIES, WorkspaceScanResult, WorkspaceTreeKind,
    WorkspaceTreeNode, is_markdown_file,
};

#[cfg(test)]
pub(super) fn collect_markdown_paths(node: &WorkspaceTreeNode, paths: &mut Vec<PathBuf>) {
    if let WorkspaceTreeKind::File(path) = &node.kind
        && is_markdown_file(path)
    {
        paths.push(path.clone());
    }
    for child in &node.children {
        collect_markdown_paths(child, paths);
    }
}

/// 在后台一次性建立规范化树和 Quick Open 平面索引，避免 UI 线程接触目录 IO 或递归排序。
pub(super) fn scan_workspace(
    path: &Path,
    pinned_empty_directories: &[PathBuf],
    show_hidden_files: bool,
    cancelled: &AtomicBool,
) -> Result<WorkspaceScanResult> {
    if cancelled.load(Ordering::Acquire) {
        bail!("workspace scan cancelled");
    }
    let root_path = dunce::canonicalize(path)
        .with_context(|| format!("failed to resolve workspace root '{}'", path.display()))?;
    if !root_path.is_dir() {
        bail!("workspace root is not a directory: '{}'", path.display());
    }
    fs::read_dir(&root_path)
        .with_context(|| format!("failed to read '{}'", root_path.display()))?;
    let mut root = WorkspaceTreeNode {
        id: file_node_id(&root_path),
        label: file_label(&root_path),
        kind: WorkspaceTreeKind::Directory(root_path.clone()),
        children: Vec::new(),
    };
    let walker = ignore::WalkBuilder::new(&root_path)
        .hidden(!show_hidden_files)
        .follow_links(false)
        .git_ignore(true)
        .git_exclude(true)
        .require_git(false)
        .build();
    let mut entry_count = 0usize;
    // 将恢复路径的 canonicalize 计入同一工作量预算，避免旧会话绕过 20,000 上限。
    let mut work_count = 0usize;
    let mut scanned_paths = HashSet::new();
    let mut quick_open_paths = Vec::new();
    for entry in walker {
        if cancelled.load(Ordering::Acquire) {
            bail!("workspace scan cancelled");
        }
        let entry = entry.context("failed to walk workspace directory")?;
        if entry.depth() > WORKSPACE_SCAN_MAX_DEPTH {
            bail!(
                "workspace scan exceeds the {}-level depth limit",
                WORKSPACE_SCAN_MAX_DEPTH
            );
        }
        if entry.depth() == 0 {
            continue;
        }
        if scanned_paths.insert(entry.path().to_path_buf()) {
            work_count = work_count
                .checked_add(1)
                .ok_or_else(|| anyhow!("workspace scan work count overflow"))?;
            if work_count > WORKSPACE_SCAN_MAX_ENTRIES {
                bail!(
                    "workspace scan exceeds the {}-entry limit",
                    WORKSPACE_SCAN_MAX_ENTRIES
                );
            }
            entry_count = entry_count
                .checked_add(1)
                .ok_or_else(|| anyhow!("workspace scan entry count overflow"))?;
            if entry_count > WORKSPACE_SCAN_MAX_ENTRIES {
                bail!(
                    "workspace scan exceeds the {}-entry limit",
                    WORKSPACE_SCAN_MAX_ENTRIES
                );
            }
        }
        if !entry.file_type().is_some_and(|kind| kind.is_file()) {
            continue;
        }
        insert_workspace_file(&mut root, &root_path, entry.path());
        if is_markdown_file(entry.path()) {
            quick_open_paths.push(entry.path().to_path_buf());
        }
    }
    for directory in pinned_empty_directories {
        if cancelled.load(Ordering::Acquire) {
            bail!("workspace scan cancelled");
        }
        work_count = work_count
            .checked_add(1)
            .ok_or_else(|| anyhow!("workspace scan work count overflow"))?;
        if work_count > WORKSPACE_SCAN_MAX_ENTRIES {
            bail!(
                "workspace scan exceeds the {}-entry limit while processing pinned paths",
                WORKSPACE_SCAN_MAX_ENTRIES
            );
        }
        let Ok(directory) = dunce::canonicalize(directory) else {
            continue;
        };
        if !show_hidden_files && workspace_path_is_hidden(&root_path, &directory) {
            continue;
        }
        if directory.starts_with(&root_path) && directory.is_dir() {
            let relative_depth = directory
                .strip_prefix(&root_path)
                .map(|relative| relative.components().count())
                .unwrap_or(WORKSPACE_SCAN_MAX_DEPTH + 1);
            if relative_depth > WORKSPACE_SCAN_MAX_DEPTH {
                bail!(
                    "workspace scan exceeds the {}-level depth limit",
                    WORKSPACE_SCAN_MAX_DEPTH
                );
            }
            let inserted_directories =
                insert_workspace_directory(&mut root, &root_path, &directory);
            entry_count = entry_count
                .checked_add(inserted_directories)
                .ok_or_else(|| anyhow!("workspace scan entry count overflow"))?;
            if entry_count > WORKSPACE_SCAN_MAX_ENTRIES {
                bail!(
                    "workspace scan exceeds the {}-entry limit",
                    WORKSPACE_SCAN_MAX_ENTRIES
                );
            }
        }
    }
    if cancelled.load(Ordering::Acquire) {
        bail!("workspace scan cancelled");
    }
    sort_workspace_tree(&mut root);
    quick_open_paths.sort_by(|left, right| {
        let left_relative = left
            .strip_prefix(&root_path)
            .unwrap_or(left.as_path())
            .to_string_lossy()
            .replace('\\', "/");
        let right_relative = right
            .strip_prefix(&root_path)
            .unwrap_or(right.as_path())
            .to_string_lossy()
            .replace('\\', "/");
        left_relative
            .to_lowercase()
            .cmp(&right_relative.to_lowercase())
            .then_with(|| left_relative.cmp(&right_relative))
    });
    Ok(WorkspaceScanResult {
        root: root_path,
        tree: root,
        quick_open_paths,
    })
}

// Keep the existing tree-only test seam while production callers consume the richer background result.
#[cfg(test)]
pub(super) fn scan_workspace_dir(path: &Path) -> Result<WorkspaceTreeNode> {
    let cancelled = AtomicBool::new(false);
    Ok(scan_workspace(path, &[], false, &cancelled)?.tree)
}

pub(super) fn workspace_path_is_hidden(root: &Path, path: &Path) -> bool {
    path.strip_prefix(root).is_ok_and(|relative| {
        relative.components().any(|component| {
            component
                .as_os_str()
                .to_str()
                .is_some_and(|name| name.starts_with('.') && name != "." && name != "..")
        })
    })
}

pub(super) fn insert_workspace_file(root: &mut WorkspaceTreeNode, base: &Path, file: &Path) {
    let Some(parent) = file.parent() else {
        return;
    };
    let Ok(relative_parent) = parent.strip_prefix(base) else {
        return;
    };
    let mut current = root;
    let mut directory_path = base.to_path_buf();
    for component in relative_parent.components() {
        directory_path.push(component.as_os_str());
        let index = current
            .children
            .iter()
            .position(|node| {
                matches!(&node.kind, WorkspaceTreeKind::Directory(path) if path == &directory_path)
            })
            .unwrap_or_else(|| {
                current.children.push(WorkspaceTreeNode {
                    id: file_node_id(&directory_path),
                    label: file_label(&directory_path),
                    kind: WorkspaceTreeKind::Directory(directory_path.clone()),
                    children: Vec::new(),
                });
                current.children.len() - 1
            });
        current = &mut current.children[index];
    }
    if !current
        .children
        .iter()
        .any(|node| matches!(&node.kind, WorkspaceTreeKind::File(path) if path == file))
    {
        current.children.push(WorkspaceTreeNode {
            id: file_node_id(file),
            label: file_label(file),
            kind: WorkspaceTreeKind::File(file.to_path_buf()),
            children: Vec::new(),
        });
    }
}

/// 补齐被忽略但仍需展示的空目录，并返回新增节点数以便后台扫描执行总量限额。
pub(super) fn insert_workspace_directory(
    root: &mut WorkspaceTreeNode,
    base: &Path,
    directory: &Path,
) -> usize {
    let Ok(relative) = directory.strip_prefix(base) else {
        return 0;
    };
    let mut current = root;
    let mut directory_path = base.to_path_buf();
    let mut inserted = 0;
    for component in relative.components() {
        directory_path.push(component.as_os_str());
        let index = current
            .children
            .iter()
            .position(|node| {
                matches!(&node.kind, WorkspaceTreeKind::Directory(path) if path == &directory_path)
            })
            .unwrap_or_else(|| {
                inserted += 1;
                current.children.push(WorkspaceTreeNode {
                    id: file_node_id(&directory_path),
                    label: file_label(&directory_path),
                    kind: WorkspaceTreeKind::Directory(directory_path.clone()),
                    children: Vec::new(),
                });
                current.children.len() - 1
            });
        current = &mut current.children[index];
    }
    inserted
}

pub(super) fn sort_workspace_tree(node: &mut WorkspaceTreeNode) {
    node.children.sort_by(|left, right| {
        let left_dir = matches!(left.kind, WorkspaceTreeKind::Directory(_));
        let right_dir = matches!(right.kind, WorkspaceTreeKind::Directory(_));
        right_dir
            .cmp(&left_dir)
            .then_with(|| left.label.to_lowercase().cmp(&right.label.to_lowercase()))
    });
    for child in &mut node.children {
        sort_workspace_tree(child);
    }
}

pub(super) fn remove_workspace_path(
    node: &mut WorkspaceTreeNode,
    target: &Path,
    pinned_empty_directories: &HashSet<PathBuf>,
) -> bool {
    let mut removed = false;
    node.children.retain_mut(|child| {
        let child_path = match &child.kind {
            WorkspaceTreeKind::Directory(path) | WorkspaceTreeKind::File(path) => {
                Some(path.clone())
            }
            WorkspaceTreeKind::Heading { .. } => None,
        };
        if child_path.as_deref() == Some(target) {
            removed = true;
            return false;
        }
        if matches!(child.kind, WorkspaceTreeKind::Directory(_)) {
            removed |= remove_workspace_path(child, target, pinned_empty_directories);
            if child.children.is_empty()
                && child_path
                    .as_ref()
                    .is_some_and(|path| !pinned_empty_directories.contains(path))
            {
                return false;
            }
        }
        true
    });
    removed
}

fn file_label(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

fn file_node_id(path: &Path) -> String {
    format!("file:{}", path.to_string_lossy())
}

pub(super) fn stable_node_hash(id: &str) -> u64 {
    use std::hash::{Hash, Hasher};

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    id.hash(&mut hasher);
    hasher.finish()
}
