// @author kongweiguang

use std::{fs, path::PathBuf};

use anyhow::Result;
use gmark_config::{
    AppDirs, SessionSelection, WORKSPACE_SESSION_VERSION, WorkspaceSession,
    WorkspaceSessionAffinity, WorkspaceSessionDocumentRef, WorkspaceSessionPane,
    WorkspaceSessionPaneId, WorkspaceSessionPaneNode, WorkspaceSessionPaneViewState,
    WorkspaceSessionSelection, WorkspaceSessionSplitAxis, WorkspaceSessionStore,
    WorkspaceSessionTab, WorkspaceSessionWindow, WorkspaceSessionWindowState,
};
use tempfile::TempDir;
use uuid::Uuid;

fn temporary_store() -> Result<(TempDir, WorkspaceSessionStore)> {
    let temporary = TempDir::new()?;
    let store = WorkspaceSessionStore::new(AppDirs::from_root(temporary.path()));
    Ok((temporary, store))
}

fn tab(path: &str, pinned: bool) -> WorkspaceSessionTab {
    WorkspaceSessionTab::new(PathBuf::from(path), pinned)
}

fn session(id: Uuid, path: &str, pinned: bool) -> WorkspaceSession {
    session_with_tabs(id, vec![tab(path, pinned)], 0, None)
}

fn session_with_tabs(
    id: Uuid,
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

fn write_registry(store: &WorkspaceSessionStore, contents: impl AsRef<[u8]>) -> Result<()> {
    fs::create_dir_all(store.dirs().state_root())?;
    fs::write(store.dirs().workspace_session_file(), contents)?;
    Ok(())
}

#[test]
fn reads_v1_through_v8_compatibility_samples() -> Result<()> {
    let (_temporary, store) = temporary_store()?;
    write_registry(
        &store,
        r#"{"version":1,"tabs":[{"path":"v1.md","pinned":true}],"active_index":0}"#,
    )?;
    let v1 = store.read()?;
    assert_eq!(v1.len(), 1);
    let v1_pane = v1[0]
        .focused()
        .ok_or_else(|| anyhow::anyhow!("missing v1 pane"))?;
    assert_eq!(
        v1_pane.tabs[0].document.file_path(),
        Some(PathBuf::from("v1.md").as_path())
    );
    assert!(v1_pane.tabs[0].pinned);

    for version in 2..=8 {
        let id = Uuid::new_v4();
        let (tab_suffix, session_suffix) = match version {
            2 => ("", ""),
            3 => (",\"view_mode\":\"split\"", ""),
            4 => (
                "",
                ",\"window\":{\"x\":12.0,\"y\":13.0,\"width\":900.0,\"height\":700.0}",
            ),
            5 => ("", ",\"workspace_panel_width\":318.0"),
            6 => ("", ",\"split_pane_ratio\":0.62"),
            7 => ("", ",\"workspace_docked_open\":false"),
            8 => (
                "",
                ",\"document_sidebar_width\":300.0,\"document_sidebar_docked_open\":true",
            ),
            _ => unreachable!(),
        };
        let selection = if version == 7 {
            ",\"selection\":{\"start\":2,\"end\":9,\"reversed\":true}"
        } else if version == 8 {
            ",\"selection\":{\"start\":2,\"end\":9,\"reversed\":false,\"anchor_affinity\":\"after\",\"head_affinity\":\"before\"}"
        } else {
            ""
        };
        let contents = format!(
            r#"{{"version":{version},"windows":[{{"id":"{id}","tabs":[{{"path":"v{version}.md","pinned":false{tab_suffix}{selection}}}],"active_index":0{session_suffix}}}]}}"#
        );
        write_registry(&store, contents)?;
        let restored = store.read()?;
        let restored_session = &restored[0];
        assert_eq!(restored_session.id, id);
        match version {
            2 => assert!(
                restored_session.focused().unwrap().tabs[0]
                    .state
                    .view_mode
                    .is_none()
            ),
            3 => assert_eq!(
                restored_session.focused().unwrap().tabs[0]
                    .state
                    .view_mode
                    .as_deref(),
                Some("split")
            ),
            4 => assert!(restored_session.window.is_some()),
            5 => assert_eq!(restored_session.workspace_panel_width, Some(318.0)),
            6 => assert_eq!(restored_session.split_pane_ratio, Some(0.62)),
            7 => {
                assert_eq!(restored_session.workspace_docked_open, Some(false));
                let selection = restored_session.focused().unwrap().tabs[0]
                    .state
                    .selection
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("v7 selection missing"))?;
                let neutral = selection.selection_for_range(selection.start..selection.end);
                assert_eq!(neutral.anchor.byte_offset, 9);
                assert_eq!(neutral.anchor.affinity, WorkspaceSessionAffinity::After);
                assert_eq!(neutral.head.byte_offset, 2);
                assert_eq!(neutral.head.affinity, WorkspaceSessionAffinity::Before);
            }
            8 => {
                assert_eq!(restored_session.document_sidebar_width, Some(300.0));
                assert_eq!(restored_session.document_sidebar_docked_open, Some(true));
                let selection = restored_session.focused().unwrap().tabs[0]
                    .state
                    .selection
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("v8 selection missing"))?;
                let neutral = selection.selection_for_range(selection.start..selection.end);
                assert_eq!(neutral.anchor.affinity, WorkspaceSessionAffinity::After);
                assert_eq!(neutral.head.affinity, WorkspaceSessionAffinity::Before);
            }
            _ => unreachable!(),
        }
    }
    Ok(())
}

#[test]
fn writes_current_version_after_loading_a_legacy_registry() -> Result<()> {
    let (_temporary, store) = temporary_store()?;
    let id = Uuid::new_v4();
    write_registry(
        &store,
        format!(
            r#"{{"version":2,"windows":[{{"id":"{id}","tabs":[{{"path":"old.md","pinned":false}}],"active_index":0}}]}}"#
        ),
    )?;
    let restored = store.read()?;
    store.upsert(&restored[0])?;
    let written: serde_json::Value =
        serde_json::from_slice(&fs::read(store.dirs().workspace_session_file())?)?;
    assert_eq!(
        written.get("version").and_then(serde_json::Value::as_u64),
        Some(u64::from(WORKSPACE_SESSION_VERSION))
    );
    Ok(())
}

#[test]
fn migrates_transitional_v9_without_board_data_and_rejects_board_markers() -> Result<()> {
    let (_temporary, store) = temporary_store()?;
    let id = Uuid::new_v4();
    write_registry(
        &store,
        format!(
            r#"{{"version":9,"windows":[{{"id":"{id}","tabs":[{{"path":"ordinary.md","pinned":false,"view_mode":"live"}}],"active_index":0}}]}}"#
        ),
    )?;
    let restored = store.read()?;
    assert_eq!(
        restored[0].focused().unwrap().tabs[0].document.file_path(),
        Some(PathBuf::from("ordinary.md").as_path())
    );
    store.upsert(&restored[0])?;
    let written: serde_json::Value =
        serde_json::from_slice(&fs::read(store.dirs().workspace_session_file())?)?;
    assert_eq!(
        written.get("version").and_then(serde_json::Value::as_u64),
        Some(u64::from(WORKSPACE_SESSION_VERSION))
    );

    write_registry(
        &store,
        format!(
            r#"{{"version":9,"windows":[{{"id":"{id}","tabs":[{{"path":"board.gboard","pinned":false,"view_mode":"board","board":{{}}}}],"active_index":0}}]}}"#
        ),
    )?;
    assert!(store.read().is_err());
    Ok(())
}

#[test]
fn normalizes_session_state_and_uses_neutral_selection_types() -> Result<()> {
    let (_temporary, store) = temporary_store()?;
    let id = Uuid::new_v4();
    let mut regular = tab("regular.md", false);
    regular.state.view_mode = Some("unsupported".into());
    regular.state.selection = Some(WorkspaceSessionSelection {
        start: 9,
        end: 2,
        reversed: true,
        anchor_affinity: None,
        head_affinity: None,
    });
    regular.state.scroll_x = Some(50_000_000.0);
    regular.state.scroll_y = Some(-50_000_000.0);
    let mut value = session_with_tabs(
        id,
        vec![
            tab("   ", false),
            regular,
            tab("pinned.md", true),
            tab("regular.md", true),
        ],
        1,
        Some(PathBuf::from(" ")),
    );
    value.window = Some(WorkspaceSessionWindow {
        x: 9_000_000.0,
        y: -9_000_000.0,
        width: 100.0,
        height: 100_000.0,
        state: WorkspaceSessionWindowState::Maximized,
        display_uuid: None,
    });
    value.workspace_panel_width = Some(900.0);
    value.document_sidebar_width = Some(f32::NAN);
    value.split_pane_ratio = Some(0.9);
    store.upsert(&value)?;

    let restored = store.read()?;
    let restored = &restored[0];
    let restored_pane = restored.focused().unwrap();
    assert_eq!(restored_pane.tabs.len(), 2);
    assert_eq!(
        restored_pane.tabs[0].document.file_path(),
        Some(PathBuf::from("pinned.md").as_path())
    );
    assert_eq!(
        restored_pane.tabs[1].document.file_path(),
        Some(PathBuf::from("regular.md").as_path())
    );
    assert_eq!(restored_pane.active_tab, Some(restored_pane.tabs[1].id));
    assert!(restored_pane.tabs[1].state.view_mode.is_none());
    let selection = restored_pane.tabs[1]
        .state
        .selection
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("selection missing after normalization"))?;
    assert_eq!((selection.start, selection.end), (2, 9));
    assert_eq!(
        selection.selection_for_range(2..9),
        SessionSelection::from_range(2..9, true)
    );
    assert_eq!(restored_pane.tabs[1].state.scroll_x, Some(10_000_000.0));
    assert_eq!(restored_pane.tabs[1].state.scroll_y, Some(-10_000_000.0));
    assert!(restored.workspace_root.is_none());
    assert_eq!(restored.workspace_panel_width, Some(360.0));
    assert!(restored.document_sidebar_width.is_none());
    assert_eq!(restored.split_pane_ratio, Some(0.7));
    let window = restored
        .window
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("normalized window missing"))?;
    assert_eq!((window.x, window.y), (1_000_000.0, -1_000_000.0));
    assert_eq!((window.width, window.height), (720.0, 32_768.0));
    assert_eq!(window.state, WorkspaceSessionWindowState::Maximized);
    Ok(())
}

#[test]
fn updates_windows_atomically_and_removes_paths_with_active_index_repair() -> Result<()> {
    let (_temporary, store) = temporary_store()?;
    let first_id = Uuid::new_v4();
    let second_id = Uuid::new_v4();
    let third_id = Uuid::new_v4();
    store.upsert(&session(first_id, "a.md", false))?;
    store.upsert(&session(second_id, "b.md", false))?;
    store.upsert(&session(first_id, "updated.md", false))?;
    let after_update = store.read()?;
    assert_eq!(
        after_update
            .iter()
            .map(|value| value.id)
            .collect::<Vec<_>>(),
        vec![second_id, first_id]
    );
    assert_eq!(
        after_update[1].focused().unwrap().tabs[0]
            .document
            .file_path(),
        Some(PathBuf::from("updated.md").as_path())
    );

    store.upsert(&session(third_id, "updated.md", true))?;
    let after_duplicate = store.read()?;
    assert_eq!(
        after_duplicate
            .iter()
            .map(|value| value.id)
            .collect::<Vec<_>>(),
        vec![second_id, first_id, third_id]
    );
    assert!(after_duplicate[2].focused().unwrap().tabs[0].pinned);

    let before_invalid_update = fs::read(store.dirs().workspace_session_file())?;
    let invalid = session_with_tabs(
        Uuid::new_v4(),
        (0..101)
            .map(|index| tab(&format!("{index}.md"), false))
            .collect(),
        0,
        None,
    );
    assert!(store.upsert(&invalid).is_err());
    assert_eq!(
        fs::read(store.dirs().workspace_session_file())?,
        before_invalid_update
    );

    store.remove_paths(&[PathBuf::from("updated.md")])?;
    assert_eq!(
        store
            .read()?
            .iter()
            .map(|value| value.id)
            .collect::<Vec<_>>(),
        vec![second_id]
    );
    store.remove(second_id)?;
    assert!(!store.dirs().workspace_session_file().exists());
    Ok(())
}

/// 会话过滤必须把 Windows 盘符、设备前缀和 UNC 的同一路径视为同一排除目标。
#[cfg(windows)]
#[test]
fn without_paths_matches_drive_and_unc_aliases_in_both_directions() {
    let aliases = [
        (r"C:\docs\selection.md", r"\\?\c:\docs\selection.md"),
        (r"\\?\C:\docs\selection.md", "c:/docs/selection.md"),
        (
            r"\\server\share\docs\ime.md",
            r"\\?\UNC\SERVER\share\docs\ime.md",
        ),
        (
            r"\\?\UNC\server\share\docs\ime.md",
            "//SERVER/share/docs/ime.md",
        ),
    ];

    for (stored_path, excluded_path) in aliases {
        let workspace = session(Uuid::new_v4(), stored_path, false);
        assert!(
            workspace
                .without_paths(&[PathBuf::from(excluded_path)])
                .is_none(),
            "session path {stored_path:?} should match alias {excluded_path:?}"
        );
    }
}

/// Registry 清理使用与会话过滤相同的 Windows 路径身份规则，但保留其它窗口记录。
#[cfg(windows)]
#[test]
fn remove_paths_matches_drive_and_unc_aliases_in_both_directions() -> Result<()> {
    let (_temporary, store) = temporary_store()?;
    let aliases = [
        (r"C:\docs\selection.md", r"\\?\c:\docs\selection.md"),
        (r"\\?\C:\docs\selection.md", "c:/docs/selection.md"),
        (
            r"\\server\share\docs\ime.md",
            r"\\?\UNC\SERVER\share\docs\ime.md",
        ),
        (
            r"\\?\UNC\server\share\docs\ime.md",
            "//SERVER/share/docs/ime.md",
        ),
    ];

    for (stored_path, excluded_path) in aliases {
        let id = Uuid::new_v4();
        store.upsert(&session(id, stored_path, false))?;
        store.remove_paths(&[PathBuf::from(excluded_path)])?;
        assert!(
            store.read()?.iter().all(|workspace| workspace.id != id),
            "registry path {stored_path:?} should be removed using alias {excluded_path:?}"
        );
    }
    Ok(())
}

#[test]
fn rejects_unknown_corrupt_and_over_limit_registries() -> Result<()> {
    let (_temporary, store) = temporary_store()?;
    write_registry(&store, r#"{"version":99,"windows":[]}"#)?;
    assert!(store.read().is_err());

    write_registry(&store, "not json")?;
    assert!(store.read().is_err());

    write_registry(&store, vec![b' '; 1_048_577])?;
    assert!(store.read().is_err());

    let windows = (0..21)
        .map(|index| {
            format!(
                r#"{{"id":"{}","tabs":[{{"path":"{index}.md","pinned":false}}],"active_index":0}}"#,
                Uuid::new_v4()
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    write_registry(&store, format!(r#"{{"version":8,"windows":[{windows}]}}"#))?;
    assert!(store.read().is_err());
    Ok(())
}

#[test]
fn v10_round_trips_recursive_panes_references_and_view_state() -> Result<()> {
    let (_temporary, store) = temporary_store()?;
    let first_id = Uuid::new_v4();
    let recovery_id = Uuid::new_v4();
    let mut first = WorkspaceSessionTab::new(PathBuf::from("same.md"), true);
    first.state = WorkspaceSessionPaneViewState {
        selection: Some(WorkspaceSessionSelection {
            start: 9,
            end: 2,
            reversed: true,
            anchor_affinity: None,
            head_affinity: None,
        }),
        scroll_x: Some(12.0),
        scroll_y: Some(-4.0),
        view_mode: Some("split".to_owned()),
        split_ratio: Some(0.25),
        markdown_fold: Some(serde_json::json!({"heading": 2})),
        markdown_folds: Vec::new(),
        table_layout: Some(serde_json::json!({"columns": [0.4, 0.6]})),
        forward: (0..40).map(|value| serde_json::json!(value)).collect(),
        back: (0..40).map(|value| serde_json::json!(value)).collect(),
    };
    let recovery = WorkspaceSessionTab::recovery(recovery_id, false);
    let mut session = session_with_tabs(first_id, vec![first], 0, None);
    let left = session.focused_pane;
    let right = WorkspaceSessionPaneId::new();
    session
        .panes
        .insert(right, WorkspaceSessionPane::new(vec![recovery], None));
    session.root = WorkspaceSessionPaneNode::Split {
        axis: WorkspaceSessionSplitAxis::Vertical,
        ratio: 0.1,
        first: Box::new(WorkspaceSessionPaneNode::Leaf(left)),
        second: Box::new(WorkspaceSessionPaneNode::Leaf(right)),
    };
    session.focused_pane = right;
    store.upsert(&session)?;

    let restored = store.read()?;
    let restored = &restored[0];
    assert!(matches!(
        &restored.root,
        WorkspaceSessionPaneNode::Split {
            axis: WorkspaceSessionSplitAxis::Vertical,
            ratio,
            ..
        } if (*ratio - 0.1).abs() < f32::EPSILON
    ));
    assert_eq!(restored.panes.len(), 2);
    assert_eq!(restored.focused_pane, right);
    let focused = restored
        .panes
        .get(&right)
        .ok_or_else(|| anyhow::anyhow!("missing right pane"))?;
    assert!(
        matches!(focused.tabs[0].document, WorkspaceSessionDocumentRef::Recovery(id) if id == recovery_id)
    );
    assert_eq!(focused.active_tab, Some(focused.tabs[0].id));
    let left = restored
        .panes
        .get(&left)
        .ok_or_else(|| anyhow::anyhow!("missing left pane"))?;
    let state = &left.tabs[0].state;
    assert_eq!(state.forward.len(), 32);
    assert_eq!(state.back.len(), 32);
    assert_eq!(state.split_ratio, Some(0.25));
    assert_eq!(
        state
            .selection
            .as_ref()
            .map(|value| (value.start, value.end)),
        Some((2, 9))
    );
    let json: serde_json::Value =
        serde_json::from_slice(&fs::read(store.dirs().workspace_session_file())?)?;
    assert_eq!(json["version"], serde_json::json!(10));
    assert!(json["windows"][0].get("tabs").is_none());
    Ok(())
}

#[test]
fn same_pane_paths_are_deduplicated_but_other_panes_and_windows_keep_them() -> Result<()> {
    let (_temporary, store) = temporary_store()?;
    let first_id = Uuid::new_v4();
    let second_id = Uuid::new_v4();
    let mut session = session_with_tabs(
        first_id,
        vec![
            tab("same.md", false),
            tab("same.md", true),
            tab("other.md", false),
        ],
        0,
        None,
    );
    let left = session.focused_pane;
    let right = WorkspaceSessionPaneId::new();
    session.panes.insert(
        right,
        WorkspaceSessionPane::new(vec![tab("same.md", false)], None),
    );
    session.root = WorkspaceSessionPaneNode::Split {
        axis: WorkspaceSessionSplitAxis::Horizontal,
        ratio: 0.5,
        first: Box::new(WorkspaceSessionPaneNode::Leaf(left)),
        second: Box::new(WorkspaceSessionPaneNode::Leaf(right)),
    };
    store.upsert(&session)?;
    store.upsert(&session_with_tabs(
        second_id,
        vec![tab("same.md", false)],
        0,
        None,
    ))?;
    let restored = store.read()?;
    assert_eq!(restored.len(), 2);
    let first = restored
        .iter()
        .find(|value| value.id == first_id)
        .ok_or_else(|| anyhow::anyhow!("first missing"))?;
    assert_eq!(first.panes.get(&left).map(|pane| pane.tabs.len()), Some(2));
    assert_eq!(first.panes.get(&right).map(|pane| pane.tabs.len()), Some(1));
    Ok(())
}

#[test]
fn malformed_v10_pane_references_fail_closed() -> Result<()> {
    let (_temporary, store) = temporary_store()?;
    let id = Uuid::new_v4();
    let missing = WorkspaceSessionPaneId::new();
    write_registry(
        &store,
        serde_json::json!({
            "version": 10,
            "windows": [{
                "id": id,
                "root": {"leaf": missing},
                "panes": {},
                "focused_pane": missing
            }]
        })
        .to_string(),
    )?;
    assert!(store.read().is_err());
    Ok(())
}

#[test]
fn malformed_v10_view_ids_fail_closed() -> Result<()> {
    let (_temporary, store) = temporary_store()?;
    let id = Uuid::new_v4();
    let pane_id = Uuid::new_v4();
    let nil = Uuid::nil();
    let base = |tabs: serde_json::Value| {
        serde_json::json!({
            "version": 10,
            "windows": [{
                "id": id,
                "root": {"leaf": pane_id},
                "panes": {
                    pane_id.to_string(): {"tabs": tabs, "active_tab": nil}
                },
                "focused_pane": pane_id
            }]
        })
    };

    write_registry(
        &store,
        base(serde_json::json!([{
            "id": nil,
            "document": {"file": "nil.md"},
            "state": {}
        }]))
        .to_string(),
    )?;
    assert!(store.read().is_err());

    let duplicate = Uuid::new_v4();
    write_registry(
        &store,
        base(serde_json::json!([
            {"id": duplicate, "document": {"file": "a.md"}, "state": {}},
            {"id": duplicate, "document": {"file": "b.md"}, "state": {}}
        ]))
        .to_string(),
    )?;
    assert!(store.read().is_err());
    Ok(())
}

#[test]
fn first_pre_v10_write_creates_non_overwriting_backup() -> Result<()> {
    let (_temporary, store) = temporary_store()?;
    let id = Uuid::new_v4();
    let legacy = format!(
        r#"{{"version":8,"windows":[{{"id":"{id}","tabs":[{{"path":"old.md","pinned":false}}],"active_index":0}}]}}"#
    );
    write_registry(&store, &legacy)?;
    let restored = store.read()?;
    store.upsert(&restored[0])?;
    let backup = store.pre_v10_backup_path();
    assert_eq!(fs::read(&backup)?, legacy.as_bytes());
    let original_backup = fs::read(&backup)?;
    store.upsert(&session_with_tabs(id, vec![tab("new.md", false)], 0, None))?;
    assert_eq!(fs::read(backup)?, original_backup);
    Ok(())
}

#[test]
fn failed_pre_v10_backup_keeps_original_session_untouched() -> Result<()> {
    let (_temporary, store) = temporary_store()?;
    let id = Uuid::new_v4();
    let legacy = format!(
        r#"{{"version":8,"windows":[{{"id":"{id}","tabs":[{{"path":"old.md","pinned":false}}],"active_index":0}}]}}"#
    );
    write_registry(&store, &legacy)?;
    fs::create_dir(store.pre_v10_backup_path())?;
    let restored = store.read()?;
    assert!(store.upsert(&restored[0]).is_err());
    assert_eq!(
        fs::read(store.dirs().workspace_session_file())?,
        legacy.as_bytes()
    );
    Ok(())
}
