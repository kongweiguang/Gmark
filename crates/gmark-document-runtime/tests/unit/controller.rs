// @author kongweiguang

use std::fs;
use std::path::PathBuf;
use std::sync::Barrier;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use gmark_document_core::{
    DocumentFormat, DocumentProfile, DocumentViewInstanceId, LoadingPolicy, SourceEdit,
    SourceSelection, TextEncoding,
};
use gmark_paged_document::{FileSource, LineIndex};

use super::*;
use crate::{DocumentStore, ResidentDocument};

fn session() -> DocumentSession {
    let directory = tempfile::tempdir().unwrap_or_else(|error| panic!("tempdir: {error}"));
    let path = directory.path().join("controller.txt");
    fs::write(&path, "one").unwrap_or_else(|error| panic!("fixture write: {error}"));
    let source_identity = FileSource::open(&path)
        .and_then(|source| source.identity())
        .unwrap_or_else(|error| panic!("source identity: {error}"));
    let identity = FileIdentity::from(&source_identity);
    let profile = DocumentProfile {
        len: 3,
        format: DocumentFormat::PlainText,
        encoding: TextEncoding::Utf8 { bom: false },
        estimated_lines: 1,
        estimated_structural_units: 0,
    };
    DocumentSession::new(
        profile.clone(),
        DocumentStore::Resident(Box::new(ResidentDocument::new(
            "one",
            profile.encoding.clone(),
            source_identity,
        ))),
        LoadingPolicy::default().resolve(&profile),
        identity,
    )
    .unwrap_or_else(|error| panic!("session: {error}"))
}

#[test]
fn set_encoding_is_shared_metadata_without_a_body_undo_entry() {
    let handle = DocumentHandle::new(DocumentController::new(DocumentId::new(), session()));
    let first_view = DocumentViewInstanceId::new();
    let second_view = DocumentViewInstanceId::new();
    let (_snapshot, mut subscription) = handle
        .subscribe_with_snapshot()
        .unwrap_or_else(|error| panic!("subscribe: {error}"));

    let revision = handle
        .set_encoding(first_view, TransactionId(1), TextEncoding::Utf16Le)
        .unwrap_or_else(|error| panic!("set encoding: {error}"));
    assert_eq!(revision, DocumentRevision(1));
    {
        let controller = handle
            .lock()
            .unwrap_or_else(|error| panic!("lock encoding: {error}"));
        assert_eq!(controller.session().encoding(), &TextEncoding::Utf16Le);
        assert_eq!(controller.session().profile.encoding, TextEncoding::Utf16Le);
        assert!(controller.session().dirty);
        assert_eq!(
            controller
                .session()
                .serialized_bytes()
                .unwrap_or_else(|error| panic!("serialize encoding: {error}")),
            [0xff, 0xfe, b'o', 0, b'n', 0, b'e', 0]
        );
        assert_eq!(
            controller
                .session()
                .snapshot()
                .read_range(0..3)
                .unwrap_or_else(|error| panic!("read body: {error}")),
            b"one"
        );
    }

    // Encoding metadata is not a fake empty body transaction: undo has no
    // operation to consume and therefore leaves both revision and encoding.
    handle
        .lock()
        .unwrap_or_else(|error| panic!("lock undo: {error}"))
        .dispatch(DocumentCommand::Undo {
            view_id: second_view,
            transaction_id: TransactionId(2),
        })
        .unwrap_or_else(|error| panic!("undo: {error}"));
    assert_eq!(
        handle
            .lock()
            .unwrap_or_else(|error| panic!("lock revision: {error}"))
            .session()
            .revision(),
        1
    );

    let first_save = handle
        .request_save_snapshot()
        .unwrap_or_else(|error| panic!("request first save: {error}"))
        .unwrap_or_else(|| panic!("first save missing"));
    assert_eq!(first_save.encoding, TextEncoding::Utf16Le);

    let next_revision = handle
        .set_encoding(
            second_view,
            TransactionId(3),
            TextEncoding::Utf8 { bom: false },
        )
        .unwrap_or_else(|error| panic!("restore utf8: {error}"));
    assert_eq!(next_revision, DocumentRevision(2));
    assert!(
        handle
            .request_save_snapshot()
            .unwrap_or_else(|error| panic!("queue second save: {error}"))
            .is_none()
    );
    let promoted = handle
        .complete_save(DocumentRevision(1), first_save.identity.clone())
        .unwrap_or_else(|error| panic!("complete first save: {error}"))
        .unwrap_or_else(|| panic!("second save was not promoted"));
    assert_eq!(promoted.revision, DocumentRevision(2));
    assert_eq!(promoted.encoding, TextEncoding::Utf8 { bom: false });
    assert!(
        handle
            .lock()
            .unwrap_or_else(|error| panic!("lock dirty after stale save: {error}"))
            .session()
            .dirty
    );

    let events = subscription
        .poll()
        .unwrap_or_else(|error| panic!("poll events: {error}"));
    assert!(events.iter().any(|event| {
        matches!(
            event,
            DocumentEvent::RevisionChanged {
                view_id,
                revision: DocumentRevision(1),
                mutation,
                ..
            } if *view_id == first_view && *mutation == DocumentMutationMap::empty()
        )
    }));
    assert!(events.iter().any(|event| {
        matches!(
            event,
            DocumentEvent::DirtyChanged {
                revision: DocumentRevision(1),
                dirty: true,
                ..
            }
        )
    }));
}

#[test]
fn saves_coalesce_and_old_save_does_not_clear_newer_edits() {
    let handle = DocumentHandle::new(DocumentController::new(DocumentId::new(), session()));
    let (_snapshot, _subscription) = handle
        .subscribe_with_snapshot()
        .unwrap_or_else(|error| panic!("subscribe: {error}"));
    handle
        .lock()
        .unwrap_or_else(|error| panic!("lock first edit: {error}"))
        .dispatch(DocumentCommand::ApplyTransaction {
            view_id: DocumentViewInstanceId::new(),
            transaction_id: TransactionId(1),
            transaction: Transaction::new(DocumentRevision(0), vec![SourceEdit::new(0..1, "t")]),
            selection_before: SourceSelection::default(),
            selection_after: SourceSelection::collapsed(
                1,
                gmark_document_core::SourceAffinity::After,
            ),
        })
        .unwrap_or_else(|error| panic!("first edit: {error}"));
    let first_save = handle
        .request_save_snapshot()
        .unwrap_or_else(|error| panic!("first save request: {error}"))
        .unwrap_or_else(|| panic!("first save should start"));
    assert_eq!(
        first_save
            .read_all()
            .unwrap_or_else(|error| panic!("read first save: {error}")),
        b"tne"
    );
    handle
        .lock()
        .unwrap_or_else(|error| panic!("lock second edit: {error}"))
        .dispatch(DocumentCommand::ApplyTransaction {
            view_id: DocumentViewInstanceId::new(),
            transaction_id: TransactionId(2),
            transaction: Transaction::new(DocumentRevision(1), vec![SourceEdit::new(1..2, "w")]),
            selection_before: SourceSelection::collapsed(
                1,
                gmark_document_core::SourceAffinity::After,
            ),
            selection_after: SourceSelection::collapsed(
                2,
                gmark_document_core::SourceAffinity::After,
            ),
        })
        .unwrap_or_else(|error| panic!("second edit: {error}"));
    assert!(
        handle
            .request_save_snapshot()
            .unwrap_or_else(|error| panic!("second save request: {error}"))
            .is_none()
    );
    let identity = handle
        .lock()
        .unwrap_or_else(|error| panic!("lock identity: {error}"))
        .session()
        .file_identity
        .clone();
    let target_identity = FileIdentity {
        canonical_path: PathBuf::from("old-revision-save-as.txt"),
        len: identity.len,
        modified_nanos: identity.modified_nanos,
        platform_id: identity.platform_id.clone(),
    };
    handle
        .lock()
        .unwrap_or_else(|error| panic!("lock old save: {error}"))
        .dispatch(DocumentCommand::SaveSucceeded {
            revision: DocumentRevision(1),
            identity: target_identity.clone(),
        })
        .unwrap_or_else(|error| panic!("old save completion: {error}"));

    assert!(
        handle
            .lock()
            .unwrap_or_else(|error| panic!("lock dirty: {error}"))
            .session()
            .dirty
    );
    assert_eq!(
        handle
            .lock()
            .unwrap_or_else(|error| panic!("lock identity after old save: {error}"))
            .session()
            .file_identity,
        target_identity
    );
    // Save promotion is returned directly by complete_save; save queue
    // transitions are deliberately not broadcast as a seventh event kind.
    handle
        .lock()
        .unwrap_or_else(|error| panic!("lock undo: {error}"))
        .dispatch(DocumentCommand::Undo {
            view_id: DocumentViewInstanceId::new(),
            transaction_id: TransactionId(3),
        })
        .unwrap_or_else(|error| panic!("undo newer edit: {error}"));
    let restored = handle
        .lock()
        .unwrap_or_else(|error| panic!("lock restored snapshot: {error}"))
        .session()
        .snapshot()
        .read_range(0..3)
        .unwrap_or_else(|error| panic!("read restored snapshot: {error}"));
    assert_eq!(restored, b"tne");
    assert!(
        !handle
            .lock()
            .unwrap_or_else(|error| panic!("lock final dirty: {error}"))
            .session()
            .dirty
    );
}

#[test]
fn completion_returns_promoted_snapshot_without_a_third_request() {
    let handle = DocumentHandle::new(DocumentController::new(DocumentId::new(), session()));
    let first_view = DocumentViewInstanceId::new();
    handle
        .lock()
        .unwrap_or_else(|error| panic!("lock first edit: {error}"))
        .dispatch(DocumentCommand::ApplyTransaction {
            view_id: first_view,
            transaction_id: TransactionId(1),
            transaction: Transaction::new(DocumentRevision(0), vec![SourceEdit::new(0..1, "t")]),
            selection_before: SourceSelection::default(),
            selection_after: SourceSelection::collapsed(
                1,
                gmark_document_core::SourceAffinity::After,
            ),
        })
        .unwrap_or_else(|error| panic!("first edit: {error}"));
    let first_snapshot = handle
        .request_save_snapshot()
        .unwrap_or_else(|error| panic!("first save request: {error}"))
        .unwrap_or_else(|| panic!("first save should start"));
    let identity = first_snapshot.identity.clone();
    handle
        .lock()
        .unwrap_or_else(|error| panic!("lock second edit: {error}"))
        .dispatch(DocumentCommand::ApplyTransaction {
            view_id: first_view,
            transaction_id: TransactionId(2),
            transaction: Transaction::new(DocumentRevision(1), vec![SourceEdit::new(1..2, "w")]),
            selection_before: SourceSelection::collapsed(
                1,
                gmark_document_core::SourceAffinity::After,
            ),
            selection_after: SourceSelection::collapsed(
                2,
                gmark_document_core::SourceAffinity::After,
            ),
        })
        .unwrap_or_else(|error| panic!("second edit: {error}"));
    assert!(
        handle
            .request_save_snapshot()
            .unwrap_or_else(|error| panic!("second save request: {error}"))
            .is_none()
    );
    let promoted = handle
        .complete_save(DocumentRevision(1), identity.clone())
        .unwrap_or_else(|error| panic!("first completion: {error}"))
        .unwrap_or_else(|| panic!("pending save should be promoted"));
    assert_eq!(promoted.revision, DocumentRevision(2));
    assert_eq!(promoted.source_format, first_snapshot.source_format);
    assert_eq!(
        promoted
            .read_all()
            .unwrap_or_else(|error| panic!("read promoted save: {error}")),
        b"twe"
    );
    assert!(
        handle
            .complete_save(DocumentRevision(2), identity)
            .unwrap_or_else(|error| panic!("second completion: {error}"))
            .is_none()
    );
    assert!(
        !handle
            .lock()
            .unwrap_or_else(|error| panic!("lock final dirty: {error}"))
            .session()
            .dirty
    );
}

#[test]
fn discard_changes_requires_final_lease_and_broadcasts_clean_state() {
    let handle = DocumentHandle::new(DocumentController::new(DocumentId::new(), session()));
    let mut subscription = handle
        .subscribe_with_snapshot()
        .unwrap_or_else(|error| panic!("subscribe: {error}"))
        .1;
    let first_lease = handle.lease();
    let second_lease = first_lease.clone();
    let view_id = DocumentViewInstanceId::new();
    handle
        .lock()
        .unwrap_or_else(|error| panic!("lock edit: {error}"))
        .dispatch(DocumentCommand::ApplyTransaction {
            view_id,
            transaction_id: TransactionId(1),
            transaction: Transaction::new(DocumentRevision(0), vec![SourceEdit::new(0..1, "t")]),
            selection_before: SourceSelection::default(),
            selection_after: SourceSelection::default(),
        })
        .unwrap_or_else(|error| panic!("edit: {error}"));
    assert!(matches!(
        handle.discard_current_changes(),
        Err(ControllerError::SharedDocumentStillLeased)
    ));
    drop(second_lease);
    assert!(
        handle
            .discard_current_changes()
            .unwrap_or_else(|error| panic!("discard: {error}"))
    );
    let controller = handle
        .lock()
        .unwrap_or_else(|error| panic!("lock clean: {error}"));
    assert!(!controller.session().dirty);
    assert_eq!(controller.session().revision(), 1);
    drop(controller);
    assert!(
        subscription
            .poll()
            .unwrap_or_else(|error| panic!("poll discard: {error}"))
            .into_iter()
            .any(|event| matches!(event, DocumentEvent::DirtyChanged { dirty: false, .. }))
    );
    drop(first_lease);
}

#[test]
fn discard_changes_requires_matching_owned_lease_count() {
    let handle = DocumentHandle::new(DocumentController::new(DocumentId::new(), session()));
    let first_lease = handle.lease();
    let second_lease = first_lease.clone();
    let view_id = DocumentViewInstanceId::new();
    handle
        .lock()
        .unwrap_or_else(|error| panic!("lock edit: {error}"))
        .dispatch(DocumentCommand::ApplyTransaction {
            view_id,
            transaction_id: TransactionId(1),
            transaction: Transaction::new(DocumentRevision(0), vec![SourceEdit::new(0..1, "t")]),
            selection_before: SourceSelection::default(),
            selection_after: SourceSelection::default(),
        })
        .unwrap_or_else(|error| panic!("edit: {error}"));

    for expected_owned_leases in [1, 3, 0] {
        assert!(matches!(
            handle.discard_current_changes_for_owned_leases(expected_owned_leases),
            Err(ControllerError::SharedDocumentStillLeased)
        ));
        assert!(
            handle
                .lock()
                .unwrap_or_else(|error| panic!("lock dirty state: {error}"))
                .session()
                .dirty,
            "a mismatched owned lease count must leave the document dirty"
        );
    }

    assert!(
        handle
            .discard_current_changes_for_owned_leases(2)
            .unwrap_or_else(|error| panic!("discard matching owned leases: {error}"))
    );
    assert!(
        !handle
            .lock()
            .unwrap_or_else(|error| panic!("lock clean state: {error}"))
            .session()
            .dirty
    );

    drop(second_lease);
    drop(first_lease);
}

#[test]
fn discard_owned_leases_race_with_new_lease_without_clearing_shared_dirty_state() {
    for _ in 0..32 {
        let handle = DocumentHandle::new(DocumentController::new(DocumentId::new(), session()));
        let owner_lease = handle.lease();
        let view_id = DocumentViewInstanceId::new();
        handle
            .lock()
            .unwrap_or_else(|error| panic!("lock edit: {error}"))
            .dispatch(DocumentCommand::ApplyTransaction {
                view_id,
                transaction_id: TransactionId(1),
                transaction: Transaction::new(
                    DocumentRevision(0),
                    vec![SourceEdit::new(0..1, "t")],
                ),
                selection_before: SourceSelection::default(),
                selection_after: SourceSelection::default(),
            })
            .unwrap_or_else(|error| panic!("edit: {error}"));

        let start = Arc::new(Barrier::new(3));
        let order = Arc::new(AtomicUsize::new(0));
        let discard_handle = handle.clone();
        let discard_start = start.clone();
        let discard_order = order.clone();
        let discard_thread = thread::spawn(move || {
            discard_start.wait();
            // Mark the discard attempt before entering the gated helper.  If
            // the contender wins the gate first, its marker wins instead.
            let _ = discard_order.compare_exchange(0, 2, Ordering::SeqCst, Ordering::SeqCst);
            discard_handle.discard_current_changes_for_owned_leases(1)
        });
        let contender_handle = handle.clone();
        let contender_start = start.clone();
        let contender_order = order.clone();
        let contender_thread = thread::spawn(move || {
            contender_start.wait();
            let lease = contender_handle.lease();
            let _ = contender_order.compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst);
            lease
        });
        start.wait();

        let discard_result = discard_thread
            .join()
            .unwrap_or_else(|_| panic!("discard thread panicked"));
        let contender_lease = contender_thread
            .join()
            .unwrap_or_else(|_| panic!("lease thread panicked"));
        let dirty = handle
            .lock()
            .unwrap_or_else(|error| panic!("lock dirty state: {error}"))
            .session()
            .dirty;

        match discard_result {
            Ok(changed) => {
                assert!(changed);
                assert_eq!(order.load(Ordering::SeqCst), 2);
                assert!(!dirty, "discard won before the new lease was acquired");
            }
            Err(ControllerError::SharedDocumentStillLeased) => {
                assert!(dirty, "a competing lease must not observe a clean discard");
            }
            Err(error) => panic!("unexpected discard error: {error}"),
        }

        drop(contender_lease);
        drop(owner_lease);
    }
}

#[test]
fn discard_owned_leases_holds_gate_until_revision_guard_completes() {
    let handle = DocumentHandle::new(DocumentController::new(DocumentId::new(), session()));
    let owner_lease = handle.lease();
    let view_id = DocumentViewInstanceId::new();
    handle
        .lock()
        .unwrap_or_else(|error| panic!("lock edit: {error}"))
        .dispatch(DocumentCommand::ApplyTransaction {
            view_id,
            transaction_id: TransactionId(1),
            transaction: Transaction::new(DocumentRevision(0), vec![SourceEdit::new(0..1, "t")]),
            selection_before: SourceSelection::default(),
            selection_after: SourceSelection::default(),
        })
        .unwrap_or_else(|error| panic!("edit: {error}"));

    // Hold Controller so the discard thread has to wait after validating the
    // lease count. Its independent gate must remain held while that wait is
    // in progress, otherwise the contender could slip in before the discard.
    let controller_guard = handle
        .lock()
        .unwrap_or_else(|error| panic!("hold controller lock: {error}"));
    let discard_handle = handle.clone();
    let discard_thread =
        thread::spawn(move || discard_handle.discard_current_changes_for_owned_leases(1));
    let gate_observed = (0..100_000).any(|_| {
        if handle.0.lease_gate.try_lock().is_err() {
            true
        } else {
            thread::yield_now();
            false
        }
    });
    assert!(gate_observed, "discard did not acquire its lease gate");

    let contender_start = Arc::new(Barrier::new(2));
    let contender_handle = handle.clone();
    let contender_start_thread = contender_start.clone();
    let contender_thread = thread::spawn(move || {
        contender_start_thread.wait();
        contender_handle.lease()
    });
    contender_start.wait();
    drop(controller_guard);

    assert!(
        discard_thread
            .join()
            .unwrap_or_else(|_| panic!("discard thread panicked"))
            .unwrap_or_else(|error| panic!("discard: {error}"))
    );
    let contender_lease = contender_thread
        .join()
        .unwrap_or_else(|_| panic!("lease thread panicked"));
    assert_eq!(handle.lease_count(), 2);
    assert!(
        !handle
            .lock()
            .unwrap_or_else(|error| panic!("lock clean state: {error}"))
            .session()
            .dirty
    );

    drop(contender_lease);
    drop(owner_lease);
}

#[test]
fn save_state_callback_runs_after_completion_and_can_be_removed() {
    let handle = DocumentHandle::new(DocumentController::new(DocumentId::new(), session()));
    let notifications = Arc::new(Mutex::new(Vec::<SaveStateNotification>::new()));
    let sink = Arc::clone(&notifications);
    let callback_handle = handle.downgrade();
    let registration = handle
        .register_save_state_callback(Arc::new(move |state| {
            let callback_handle = callback_handle
                .upgrade()
                .unwrap_or_else(|| panic!("callback handle was dropped"));
            assert_eq!(
                callback_handle
                    .save_in_flight_revision()
                    .unwrap_or_else(|error| panic!("callback state lock: {error}")),
                state.in_flight_revision
            );
            sink.lock()
                .unwrap_or_else(|error| panic!("callback sink lock: {error}"))
                .push(state);
        }))
        .unwrap_or_else(|error| panic!("register callback: {error}"));
    let view = DocumentViewInstanceId::new();
    handle
        .lock()
        .unwrap_or_else(|error| panic!("lock edit: {error}"))
        .dispatch(DocumentCommand::ApplyTransaction {
            view_id: view,
            transaction_id: TransactionId(1),
            transaction: Transaction::new(DocumentRevision(0), vec![SourceEdit::new(0..1, "t")]),
            selection_before: SourceSelection::default(),
            selection_after: SourceSelection::default(),
        })
        .unwrap_or_else(|error| panic!("edit: {error}"));
    let first = handle
        .request_save_snapshot()
        .unwrap_or_else(|error| panic!("request first: {error}"))
        .unwrap_or_else(|| panic!("missing first save"));
    handle
        .lock()
        .unwrap_or_else(|error| panic!("lock second edit: {error}"))
        .dispatch(DocumentCommand::ApplyTransaction {
            view_id: view,
            transaction_id: TransactionId(2),
            transaction: Transaction::new(DocumentRevision(1), vec![SourceEdit::new(1..2, "w")]),
            selection_before: SourceSelection::default(),
            selection_after: SourceSelection::default(),
        })
        .unwrap_or_else(|error| panic!("second edit: {error}"));
    let second = handle
        .request_save_snapshot()
        .unwrap_or_else(|error| panic!("request second: {error}"));
    assert!(second.is_none());
    handle
        .complete_save(DocumentRevision(1), first.identity)
        .unwrap_or_else(|error| panic!("complete first: {error}"));
    let identity = handle
        .lock()
        .unwrap_or_else(|error| panic!("lock identity: {error}"))
        .session()
        .file_identity
        .clone();
    handle
        .complete_save(DocumentRevision(2), identity)
        .unwrap_or_else(|error| panic!("complete second: {error}"));
    assert_eq!(
        *notifications
            .lock()
            .unwrap_or_else(|error| panic!("read callbacks: {error}")),
        vec![
            SaveStateNotification {
                in_flight_revision: Some(DocumentRevision(2)),
                pending_revision: None,
            },
            SaveStateNotification {
                in_flight_revision: None,
                pending_revision: None,
            },
        ]
    );
    assert!(
        registration
            .unregister()
            .unwrap_or_else(|error| panic!("unregister callback: {error}"))
    );
}

#[test]
fn failed_save_discards_pending_until_an_explicit_retry() {
    let handle = DocumentHandle::new(DocumentController::new(DocumentId::new(), session()));
    let view = DocumentViewInstanceId::new();
    let edit = |base_revision, range, replacement, transaction_id| {
        handle
            .lock()
            .unwrap_or_else(|error| panic!("lock edit: {error}"))
            .dispatch(DocumentCommand::ApplyTransaction {
                view_id: view,
                transaction_id,
                transaction: Transaction::new(
                    DocumentRevision(base_revision),
                    vec![SourceEdit::new(range, replacement)],
                ),
                selection_before: SourceSelection::default(),
                selection_after: SourceSelection::default(),
            })
            .unwrap_or_else(|error| panic!("edit: {error}"));
    };
    edit(0, 0..1, "t", TransactionId(1));
    let first = handle
        .request_save_snapshot()
        .unwrap_or_else(|error| panic!("first save request: {error}"))
        .unwrap_or_else(|| panic!("first save should start"));
    edit(1, 1..2, "w", TransactionId(2));
    assert!(
        handle
            .request_save_snapshot()
            .unwrap_or_else(|error| panic!("pending save request: {error}"))
            .is_none()
    );
    assert!(
        handle
            .fail_save(DocumentRevision(1), SaveFailureCode::Conflict)
            .unwrap_or_else(|error| panic!("save failure: {error}"))
            .is_none()
    );
    assert!(
        handle
            .lock()
            .unwrap_or_else(|error| panic!("lock dirty: {error}"))
            .session()
            .dirty
    );
    let retry = handle
        .request_save_snapshot()
        .unwrap_or_else(|error| panic!("retry save request: {error}"))
        .unwrap_or_else(|| panic!("retry should start"));
    assert_eq!(retry.revision, DocumentRevision(2));
    assert_eq!(
        retry
            .read_all()
            .unwrap_or_else(|error| panic!("read retry: {error}")),
        b"twe"
    );
    assert!(
        handle
            .complete_save(DocumentRevision(2), first.identity)
            .unwrap_or_else(|error| panic!("retry completion: {error}"))
            .is_none()
    );
}

#[path = "controller_parts/registry.rs"]
mod registry;

#[path = "controller_parts/typing.rs"]
mod typing;
