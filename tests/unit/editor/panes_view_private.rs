// @author kongweiguang

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::rc::Rc;

use gmark_document_core::{
    DocumentFormat, DocumentProfile, DocumentRevision, LoadingPolicy, SourceAffinity, SourceEdit,
    SourceSelection, TextEncoding, Transaction,
};
use gmark_document_runtime::{
    DocumentCommand, DocumentController, DocumentHandle, DocumentId, DocumentLease,
    DocumentSession, DocumentStore, DocumentViewInstanceId, FileIdentity, ResidentDocument,
    TransactionId,
};
use gmark_paged_document::FileSource;
use uuid::Uuid;

use super::*;
use crate::editor::panes::{
    PaneId, PaneNode, PaneReadOnlyKind, PaneState, PaneViewStateSnapshot, PaneWorkspace,
    PaneWorkspaceController, SplitAxis, TabId, TabView,
};
use gpui::{AppContext, TestAppContext, WeakEntity};

fn pane(seed: u128) -> PaneId {
    PaneId::from_uuid(Uuid::from_u128(seed))
}

fn lease_fixture() -> (DocumentId, DocumentLease) {
    let directory =
        tempfile::tempdir().unwrap_or_else(|error| panic!("fixture tempdir failed: {error}"));
    let path = directory.path().join("pane-view.md");
    std::fs::write(&path, "# pane").unwrap_or_else(|error| panic!("fixture write failed: {error}"));
    let source =
        FileSource::open(&path).unwrap_or_else(|error| panic!("fixture source failed: {error}"));
    let source_identity = source
        .identity()
        .unwrap_or_else(|error| panic!("fixture identity failed: {error}"));
    let profile = DocumentProfile {
        len: 6,
        format: DocumentFormat::Markdown,
        encoding: TextEncoding::Utf8 { bom: false },
        estimated_lines: 1,
        estimated_structural_units: 1,
    };
    let session = DocumentSession::new(
        profile.clone(),
        DocumentStore::Resident(Box::new(ResidentDocument::new(
            "# pane",
            profile.encoding.clone(),
            source_identity,
        ))),
        LoadingPolicy::default().resolve(&profile),
        FileIdentity::from(
            &source
                .identity()
                .unwrap_or_else(|error| panic!("fixture identity failed: {error}")),
        ),
    )
    .unwrap_or_else(|error| panic!("fixture session failed: {error}"));
    let document_id = DocumentId::new();
    let handle = DocumentHandle::new(DocumentController::new(document_id, session));
    let lease = handle.lease();
    assert_eq!(handle.lease_count(), 1);
    // The lease retains the handle; the temporary directory can be
    // removed after construction without affecting this in-memory test.
    drop(directory);
    (document_id, lease)
}

fn workspace_with_panes(count: usize) -> PaneWorkspace<(), ()> {
    let root = pane(1);
    let mut workspace = PaneWorkspace::with_root_id(root);
    for _ in 1..count {
        workspace
            .split_right_focused()
            .expect("test split should fit the model limit");
    }
    workspace
}

#[gpui::test]
async fn workspace_persistence_notification_defers_reentrant_reads(cx: &mut TestAppContext) {
    let root = pane(9);
    let document_id = DocumentId::new();
    let document = PaneDocumentRef::from_error(
        document_id,
        PathBuf::from("missing.md"),
        "fixture document is intentionally unavailable",
        DocumentViewInstanceId::new(),
        "missing.md",
    );
    let mut workspace: PaneWorkspace<DocumentId, PaneDocumentRef> =
        PaneWorkspace::with_root_id(root);
    workspace
        .insert_tab(
            root,
            TabView::new(TabId::from_uuid(Uuid::from_u128(9)), document_id, document),
        )
        .expect("fixture tab should attach to its root pane");

    // Session capture reads the mounted workspace entity.  Reproduce that read
    // from inside the notification so a synchronous callback fails with the
    // same "already being updated" panic users hit when releasing a divider.
    let target = Rc::new(RefCell::new(None::<WeakEntity<PaneWorkspaceView>>));
    let callback_target = target.clone();
    let callback_reads = Rc::new(Cell::new(0_usize));
    let reads = callback_reads.clone();
    let controller = PaneWorkspaceController::noop().with_workspace_changed(move |cx| {
        let entity = callback_target
            .borrow()
            .clone()
            .expect("test installs the workspace handle before notifying");
        entity
            .read_with(cx, |_, _| ())
            .expect("workspace must still be alive");
        reads.set(reads.get() + 1);
    });
    let view = cx.update(|cx| cx.new(|_| PaneWorkspaceView::new(workspace, controller)));
    *target.borrow_mut() = Some(view.downgrade());

    cx.update(|cx| {
        view.update(cx, |view, cx| {
            view.notify_workspace_changed_for_test(cx);
            assert_eq!(
                callback_reads.get(),
                0,
                "persistence must not read the workspace while its own update is active"
            );
        });
    });
    assert_eq!(
        callback_reads.get(),
        1,
        "the deferred persistence notification should run exactly once"
    );
}

#[test]
fn recursive_layout_has_one_rect_per_leaf_and_divider_paths() {
    let workspace = workspace_with_panes(4);
    let layout = compute_pane_layout(
        workspace.root(),
        PaneViewport::new(4_000.0, 900.0),
        workspace.focused_pane(),
    );
    assert!(!layout.is_degraded());
    assert_eq!(layout.rects().len(), 4);
    assert_eq!(layout.pane_order().len(), 4);
    assert_eq!(layout.dividers().len(), 3);
    assert!(
        layout
            .dividers()
            .iter()
            .any(|divider| divider.path().is_empty())
    );
}

#[test]
fn eight_leaf_layout_stays_within_model_limit() {
    let workspace = workspace_with_panes(8);
    let layout = compute_pane_layout(
        workspace.root(),
        PaneViewport::new(100_000.0, 1_000.0),
        workspace.focused_pane(),
    );
    assert_eq!(workspace.pane_count(), 8);
    assert_eq!(layout.pane_order().len(), 8);
    assert_eq!(layout.hidden_count(), 0);
}

#[test]
fn compact_layout_keeps_every_leaf_visible_without_mutating_tree_or_ratio() {
    let workspace = workspace_with_panes(4);
    let root_before = workspace.root().clone();
    let focused = workspace.focused_pane();
    let compact = compute_pane_layout(workspace.root(), PaneViewport::new(480.0, 220.0), focused);
    assert!(compact.is_degraded());
    assert_eq!(compact.visible_count(), 4);
    assert_eq!(compact.hidden_count(), 0);
    assert_eq!(compact.dividers().len(), 3);
    assert_eq!(workspace.root(), &root_before);

    let restored =
        compute_pane_layout(workspace.root(), PaneViewport::new(4_000.0, 900.0), focused);
    assert!(!restored.is_degraded());
    assert_eq!(restored.visible_count(), 4);
    assert_eq!(workspace.root(), &root_before);
}

#[test]
fn active_mount_plan_omits_inactive_tabs_and_mounts_one_per_leaf() {
    let root = pane(1);
    let second = pane(2);
    let mut panes = BTreeMap::new();
    let first_tab = TabView::new(TabId::from_uuid(Uuid::from_u128(11)), (), ());
    let second_tab = TabView::new(TabId::from_uuid(Uuid::from_u128(12)), (), ());
    panes.insert(
        root,
        PaneState::with_tabs(vec![first_tab.clone(), second_tab]),
    );
    panes.insert(second, PaneState::new());
    let workspace = PaneWorkspace::from_parts(
        PaneNode::Split {
            axis: SplitAxis::Horizontal,
            ratio: 0.5,
            first: Box::new(PaneNode::Leaf(root)),
            second: Box::new(PaneNode::Leaf(second)),
        },
        panes,
        root,
    )
    .expect("valid test workspace");
    assert_eq!(
        active_pane_mount_plan(&workspace),
        vec![(root, first_tab.id())]
    );
}

#[test]
fn active_mount_plan_scales_to_one_two_four_and_eight_leaves() {
    for count in [1_usize, 2, 4, 8] {
        let mut workspace = workspace_with_panes(count);
        for (index, pane_id) in workspace.pane_ids().into_iter().enumerate() {
            let tab = TabView::new(
                TabId::from_uuid(Uuid::from_u128(20_000 + count as u128 * 16 + index as u128)),
                (),
                (),
            );
            let _ = workspace.insert_tab(pane_id, tab);
        }
        let plan = active_pane_mount_plan(&workspace);
        assert_eq!(plan.len(), count);
        assert!(plan.iter().all(|(pane_id, tab_id)| {
            workspace
                .pane(*pane_id)
                .and_then(|pane| pane.tab(*tab_id))
                .is_some()
        }));
    }
}

#[test]
fn cloning_pane_document_ref_shares_one_runtime_lease() {
    let (document_id, lease) = lease_fixture();
    let first = PaneDocumentRef::new(document_id, lease, DocumentViewInstanceId::new());
    let handle = first.lease().expect("lease-backed pane document").handle();
    assert_eq!(handle.lease_count(), 1);
    let second = first.clone();
    assert_eq!(handle.lease_count(), 1);

    let root = pane(1);
    let tab = TabView::new(TabId::from_uuid(Uuid::from_u128(101)), document_id, first);
    let inactive = TabView::new(
        TabId::from_uuid(Uuid::from_u128(102)),
        DocumentId::from_uuid(Uuid::from_u128(102)),
        second,
    );
    let mut panes = BTreeMap::new();
    panes.insert(root, PaneState::with_tabs(vec![tab, inactive]));
    let workspace = PaneWorkspace::from_parts(PaneNode::Leaf(root), panes, root)
        .expect("valid lease test workspace");
    assert_eq!(active_pane_mount_plan(&workspace).len(), 1);
    assert_eq!(handle.lease_count(), 1);
}

#[test]
fn pane_document_ref_reports_shared_dirty_state() {
    let (document_id, lease) = lease_fixture();
    let view_id = DocumentViewInstanceId::new();
    let document = PaneDocumentRef::new(document_id, lease, view_id);
    assert!(!document.is_dirty());

    let handle = document
        .lease()
        .unwrap_or_else(|| panic!("fixture must retain its resident lease"))
        .handle();
    handle
        .lock()
        .unwrap_or_else(|error| panic!("fixture controller lock failed: {error}"))
        .dispatch(DocumentCommand::ApplyTransaction {
            view_id,
            transaction_id: TransactionId(1),
            transaction: Transaction::new(DocumentRevision(0), vec![SourceEdit::new(0..1, "!")]),
            selection_before: SourceSelection::default(),
            selection_after: SourceSelection::collapsed(1, SourceAffinity::After),
        })
        .unwrap_or_else(|error| panic!("fixture edit failed: {error}"));

    assert!(document.is_dirty());
}

#[test]
fn pane_view_state_snapshot_round_trips_all_persisted_fields() {
    let (document_id, lease) = lease_fixture();
    let view_id = DocumentViewInstanceId::new();
    let document = PaneDocumentRef::new(document_id, lease, view_id);
    let snapshot = PaneViewStateSnapshot {
        selection: Some(
            crate::config::workspace_session::WorkspaceSessionSelection {
                start: 3,
                end: 8,
                reversed: true,
                anchor_affinity: None,
                head_affinity: None,
            },
        ),
        scroll_x: Some(4.0),
        scroll_y: Some(-12.0),
        view_mode: Some("split".to_owned()),
        split_ratio: Some(0.42),
        markdown_fold: Some(serde_json::json!({ "key": "h1", "collapsed": true })),
        markdown_folds: vec![serde_json::json!({
            "kind": "heading",
            "key": "h1",
            "collapsed": true
        })],
        table_layout: Some(serde_json::json!({
            "columns": { "table": [0.25, 0.75] }
        })),
        forward: vec![serde_json::json!({ "kind": "redo", "revision": 2 })],
        back: vec![serde_json::json!({ "kind": "undo", "revision": 1 })],
    };
    document.set_view_state_snapshot(snapshot.clone());
    assert_eq!(document.view_state_snapshot(), snapshot);
}

#[test]
fn read_only_pane_documents_keep_path_state_without_runtime_lease() {
    let document_id = DocumentId::from_uuid(Uuid::from_u128(701));
    let image_view = DocumentViewInstanceId::from_uuid(Uuid::from_u128(702));
    let image = PaneDocumentRef::from_image(
        document_id,
        PathBuf::from("preview.png"),
        image_view,
        "preview.png",
    );
    assert_eq!(image.kind(), PaneDocumentKind::Image);
    assert!(image.lease().is_none());
    assert!(matches!(
        image.readonly_kind(),
        Some(PaneReadOnlyKind::Image { path }) if path == &PathBuf::from("preview.png")
    ));

    let error_view = DocumentViewInstanceId::from_uuid(Uuid::from_u128(703));
    let error = PaneDocumentRef::from_error(
        document_id,
        PathBuf::from("missing.md"),
        "permission denied",
        error_view,
        "missing.md",
    );
    assert_eq!(error.kind(), PaneDocumentKind::Error);
    assert!(error.lease().is_none());
    assert!(matches!(
        error.readonly_kind(),
        Some(PaneReadOnlyKind::Error { path, message })
            if path == &PathBuf::from("missing.md") && message == "permission denied"
    ));
}
