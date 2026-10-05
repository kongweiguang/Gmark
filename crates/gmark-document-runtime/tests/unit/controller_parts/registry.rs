// @author kongweiguang

use super::*;

#[test]
fn registry_returns_the_same_handle_for_the_same_path() {
    let registry = DocumentRegistry::default();
    let identity = session().file_identity.clone();
    let key = DocumentRegistryKey::for_file(&identity);
    let (first, first_lease, state) = registry
        .open_or_insert_leased(key.clone(), || {
            Ok(DocumentController::new(DocumentId::new(), session()))
        })
        .unwrap_or_else(|error| panic!("first open: {error}"));
    assert_eq!(state, RegistryOpen::Inserted);
    let (second, second_lease, state) = registry
        .open_or_insert_leased(key, || {
            Ok(DocumentController::new(DocumentId::new(), session()))
        })
        .unwrap_or_else(|error| panic!("second open: {error}"));
    assert_eq!(state, RegistryOpen::Existing);
    assert!(Arc::ptr_eq(&first.0, &second.0));
    drop(first_lease);
    drop(second_lease);
}

#[test]
fn opening_same_key_merges_slow_creation_without_running_create_twice() {
    let registry = Arc::new(DocumentRegistry::default());
    let identity = session().file_identity.clone();
    let key = DocumentRegistryKey::for_file(&identity);
    let creates = Arc::new(AtomicUsize::new(0));
    let first_registry = registry.clone();
    let first_key = key.clone();
    let first_creates = creates.clone();
    let started = Arc::new(Barrier::new(2));
    let first_started = started.clone();
    let first = thread::spawn(move || {
        first_registry.open_or_insert_leased(first_key, || {
            first_creates.fetch_add(1, Ordering::SeqCst);
            first_started.wait();
            // Keep the Opening slot alive long enough for the waiter thread to
            // deterministically observe and join it on slower CI workers.
            thread::sleep(Duration::from_millis(100));
            Ok(DocumentController::new(DocumentId::new(), session()))
        })
    });
    let second_registry = registry.clone();
    let second_key = key.clone();
    let second_creates = creates.clone();
    let second_started = started.clone();
    let second = thread::spawn(move || {
        second_started.wait();
        second_registry.open_or_insert_leased(second_key, || {
            second_creates.fetch_add(1, Ordering::SeqCst);
            Ok(DocumentController::new(DocumentId::new(), session()))
        })
    });
    let first = first
        .join()
        .unwrap_or_else(|_| panic!("first open panicked"));
    let second = second
        .join()
        .unwrap_or_else(|_| panic!("second open panicked"));
    let (first, first_lease, first_state) =
        first.unwrap_or_else(|error| panic!("first open: {error}"));
    let (second, second_lease, second_state) =
        second.unwrap_or_else(|error| panic!("second open: {error}"));
    assert_eq!(creates.load(Ordering::SeqCst), 1);
    assert_eq!(first_state, RegistryOpen::Inserted);
    assert_eq!(second_state, RegistryOpen::Existing);
    assert!(Arc::ptr_eq(&first.0, &second.0));
    drop(first_lease);
    drop(second_lease);
}

#[test]
fn leased_open_publishes_owner_lease_before_waiters_can_observe_ready() {
    let registry = Arc::new(DocumentRegistry::default());
    let identity = session().file_identity.clone();
    let key = DocumentRegistryKey::for_file(&identity);
    let first_registry = registry.clone();
    let first_key = key.clone();
    let started = Arc::new(Barrier::new(2));
    let first_started = started.clone();
    let first = thread::spawn(move || {
        first_registry.open_or_insert_leased(first_key, || {
            first_started.wait();
            thread::sleep(Duration::from_millis(30));
            Ok(DocumentController::new(DocumentId::new(), session()))
        })
    });
    let second_registry = registry.clone();
    let second_key = key.clone();
    let second_started = started.clone();
    let second = thread::spawn(move || {
        second_started.wait();
        second_registry.open_or_insert_leased(second_key, || {
            Err(ControllerError::open_failed("unexpected second loader"))
        })
    });
    let (first_handle, first_lease, first_state) = first
        .join()
        .unwrap_or_else(|_| panic!("first open panicked"))
        .unwrap_or_else(|error| panic!("first open: {error}"));
    let (second_handle, second_lease, second_state) = second
        .join()
        .unwrap_or_else(|_| panic!("second open panicked"))
        .unwrap_or_else(|error| panic!("second open: {error}"));
    assert_eq!(first_state, RegistryOpen::Inserted);
    assert_eq!(second_state, RegistryOpen::Existing);
    assert!(Arc::ptr_eq(&first_handle.0, &second_handle.0));
    assert_eq!(first_handle.lease_count(), 2);
    drop(first_lease);
    assert_eq!(second_handle.lease_count(), 1);
    drop(second_lease);
    let (_, lease, state) = registry
        .open_or_insert_leased(key, || {
            Ok(DocumentController::new(DocumentId::new(), session()))
        })
        .unwrap_or_else(|error| panic!("reopen: {error}"));
    assert_eq!(state, RegistryOpen::Inserted);
    drop(lease);
}

#[test]
fn opening_failure_is_shared_with_waiters_and_key_can_retry() {
    let registry = Arc::new(DocumentRegistry::default());
    let identity = session().file_identity.clone();
    let key = DocumentRegistryKey::for_file(&identity);
    let creates = Arc::new(AtomicUsize::new(0));
    let (started_sender, started_receiver) = mpsc::sync_channel(0);
    let first_registry = registry.clone();
    let first_key = key.clone();
    let first_creates = creates.clone();
    let first = thread::spawn(move || {
        first_registry.open_or_insert_leased(first_key, || {
            first_creates.fetch_add(1, Ordering::SeqCst);
            started_sender
                .send(())
                .unwrap_or_else(|error| panic!("signal opening loader: {error}"));
            thread::sleep(Duration::from_millis(30));
            Err(ControllerError::open_failed("decoder failed"))
        })
    });
    started_receiver
        .recv()
        .unwrap_or_else(|error| panic!("wait opening loader: {error}"));
    let second_registry = registry.clone();
    let second_key = key.clone();
    let second_creates = creates.clone();
    let second = thread::spawn(move || {
        second_registry.open_or_insert_leased(second_key, || {
            second_creates.fetch_add(1, Ordering::SeqCst);
            Err(ControllerError::open_failed("unexpected second loader"))
        })
    });
    let first = first
        .join()
        .unwrap_or_else(|_| panic!("first open panicked"));
    let second = second
        .join()
        .unwrap_or_else(|_| panic!("second open panicked"));
    assert!(
        matches!(first, Err(ControllerError::OpenFailed(message)) if message == "decoder failed")
    );
    assert!(
        matches!(second, Err(ControllerError::OpenFailed(message)) if message == "decoder failed")
    );
    assert_eq!(creates.load(Ordering::SeqCst), 1);
    let (_, lease, state) = registry
        .open_or_insert_leased(key, || {
            Ok(DocumentController::new(DocumentId::new(), session()))
        })
        .unwrap_or_else(|error| panic!("retry open: {error}"));
    assert_eq!(state, RegistryOpen::Inserted);
    drop(lease);
}

/// Prove a bounded waiter broadcasts one timeout and prevents a late owner from resurrecting it.
#[test]
fn opening_timeout_broadcasts_to_waiters_and_stale_owner_cannot_publish() {
    let registry = Arc::new(DocumentRegistry::default());
    let identity = session().file_identity.clone();
    let key = DocumentRegistryKey::for_file(&identity);
    let (started_sender, started_receiver) = mpsc::sync_channel(0);
    let (release_sender, release_receiver) = mpsc::sync_channel(0);
    let owner_registry = registry.clone();
    let owner_key = key.clone();
    let owner = thread::spawn(move || {
        owner_registry.open_or_insert_leased_with_timeout(owner_key, Duration::from_secs(2), || {
            started_sender
                .send(())
                .unwrap_or_else(|error| panic!("signal timeout owner: {error}"));
            if release_receiver.recv().is_err() {
                return Err(ControllerError::open_failed("timeout owner was cancelled"));
            }
            Ok(DocumentController::new(DocumentId::new(), session()))
        })
    });
    started_receiver
        .recv()
        .unwrap_or_else(|error| panic!("wait timeout owner: {error}"));

    let waiters_ready = Arc::new(Barrier::new(4));
    let mut waiters = Vec::new();
    for _ in 0..3 {
        let waiter_registry = registry.clone();
        let waiter_key = key.clone();
        let waiter_ready = waiters_ready.clone();
        waiters.push(thread::spawn(move || {
            waiter_ready.wait();
            waiter_registry.open_or_insert_leased_with_timeout(
                waiter_key,
                Duration::from_millis(250),
                || {
                    Err(ControllerError::open_failed(
                        "unexpected timeout waiter owner",
                    ))
                },
            )
        }));
    }
    waiters_ready.wait();

    for waiter in waiters {
        let result = waiter
            .join()
            .unwrap_or_else(|_| panic!("timeout waiter panicked"));
        assert!(matches!(result, Err(ControllerError::OpenTimedOut { .. })));
    }

    release_sender
        .send(())
        .unwrap_or_else(|error| panic!("release timeout owner: {error}"));
    let owner = owner
        .join()
        .unwrap_or_else(|_| panic!("timeout owner panicked"));
    assert!(matches!(owner, Err(ControllerError::OpenTimedOut { .. })));

    let (_, lease, state) = registry
        .open_or_insert_leased(key, || {
            Ok(DocumentController::new(DocumentId::new(), session()))
        })
        .unwrap_or_else(|error| panic!("retry after timeout: {error}"));
    assert_eq!(state, RegistryOpen::Inserted);
    drop(lease);
}

#[test]
fn snapshot_subscription_starts_after_snapshot_sequence_without_a_gap() {
    let controller = DocumentController::new(DocumentId::new(), session());
    let view = DocumentViewInstanceId::new();
    let handle = DocumentHandle::new(controller);
    let (snapshot, mut subscription) = handle
        .subscribe_with_snapshot()
        .unwrap_or_else(|error| panic!("subscribe: {error}"));
    assert_eq!(snapshot.sequence, 0);
    {
        let mut controller = handle
            .lock()
            .unwrap_or_else(|error| panic!("lock: {error}"));
        controller
            .dispatch(DocumentCommand::ApplyTransaction {
                view_id: view,
                transaction_id: TransactionId(7),
                transaction: Transaction::new(
                    DocumentRevision(0),
                    vec![SourceEdit::new(0..0, "x")],
                ),
                selection_before: SourceSelection::default(),
                selection_after: SourceSelection::collapsed(
                    1,
                    gmark_document_core::SourceAffinity::After,
                ),
            })
            .unwrap_or_else(|error| panic!("dispatch: {error}"));
    }
    let events = subscription
        .poll()
        .unwrap_or_else(|error| panic!("poll: {error}"));
    assert!(!events.is_empty());
    assert_eq!(events[0].sequence(), snapshot.sequence + 1);
    assert!(
        events
            .windows(2)
            .all(|pair| pair[0].sequence() < pair[1].sequence())
    );
}

#[test]
fn mutation_map_relocates_other_views_and_undo_restores_origin_only() {
    let mut controller = DocumentController::new(DocumentId::new(), session());
    let first = DocumentViewInstanceId::new();
    let second = DocumentViewInstanceId::new();
    controller.register_view(first);
    controller.register_view(second);
    controller.set_view_selection(
        second,
        SourceSelection::collapsed(3, gmark_document_core::SourceAffinity::Before),
    );
    controller
        .dispatch(DocumentCommand::ApplyTransaction {
            view_id: first,
            transaction_id: TransactionId(8),
            transaction: Transaction::new(DocumentRevision(0), vec![SourceEdit::new(0..0, "long")]),
            selection_before: SourceSelection::default(),
            selection_after: SourceSelection::collapsed(
                4,
                gmark_document_core::SourceAffinity::After,
            ),
        })
        .unwrap_or_else(|error| panic!("dispatch: {error}"));
    assert_eq!(
        controller.view_selection(second),
        Some(SourceSelection::collapsed(
            7,
            gmark_document_core::SourceAffinity::Before
        ))
    );
    controller.close_view(first);
    controller
        .dispatch(DocumentCommand::Undo {
            view_id: second,
            transaction_id: TransactionId(9),
        })
        .unwrap_or_else(|error| panic!("undo: {error}"));
    assert_eq!(controller.view_selection(first), None);
    assert_eq!(
        controller.view_selection(second),
        Some(SourceSelection::collapsed(
            3,
            gmark_document_core::SourceAffinity::Before
        ))
    );
}

#[test]
fn lease_count_controls_registry_lifetime_independently_of_handle_clones() {
    let registry = DocumentRegistry::default();
    let identity = session().file_identity.clone();
    let key = DocumentRegistryKey::for_file(&identity);
    let (handle, lease, state) = registry
        .open_or_insert_leased(key.clone(), || {
            Ok(DocumentController::new(DocumentId::new(), session()))
        })
        .unwrap_or_else(|error| panic!("open: {error}"));
    assert_eq!(state, RegistryOpen::Inserted);
    let clone = handle.clone();
    drop(handle);
    assert_eq!(clone.lease_count(), 1);
    drop(clone);
    drop(lease);
    let (_, lease, state) = registry
        .open_or_insert_leased(key, || {
            Ok(DocumentController::new(DocumentId::new(), session()))
        })
        .unwrap_or_else(|error| panic!("reopen: {error}"));
    assert_eq!(state, RegistryOpen::Inserted);
    drop(lease);
}

#[test]
fn last_lease_callback_runs_once_after_intermediate_clones_and_reentrant_state_lock() {
    let registry = DocumentRegistry::default();
    let identity = session().file_identity.clone();
    let key = DocumentRegistryKey::for_file(&identity);
    let (handle, lease, state) = registry
        .open_or_insert_leased(key.clone(), || {
            Ok(DocumentController::new(DocumentId::new(), session()))
        })
        .unwrap_or_else(|error| panic!("open callback handle: {error}"));
    assert_eq!(state, RegistryOpen::Inserted);
    let intermediate = lease.clone();
    let calls = Arc::new(AtomicUsize::new(0));
    let service_state = Arc::new(Mutex::new(()));
    let callback_calls = calls.clone();
    let callback_state = service_state.clone();
    handle
        .register_last_lease_callback(Arc::new(move || {
            let _guard = callback_state
                .lock()
                .unwrap_or_else(|_| panic!("service state lock poisoned"));
            callback_calls.fetch_add(1, Ordering::SeqCst);
        }))
        .unwrap_or_else(|error| panic!("register callback: {error}"));
    drop(intermediate);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    drop(lease);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let next_lease = handle.lease();
    drop(next_lease);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let (_, reopened_lease, reopened_state) = registry
        .open_or_insert_leased(key, || {
            Ok(DocumentController::new(DocumentId::new(), session()))
        })
        .unwrap_or_else(|error| panic!("reopen callback handle: {error}"));
    assert_eq!(reopened_state, RegistryOpen::Inserted);
    drop(reopened_lease);
}

#[test]
fn reload_prepared_document_validates_clean_baseline_and_advances_revision() {
    let handle = DocumentHandle::new(DocumentController::new(DocumentId::new(), session()));
    let (before, mut subscription) = handle
        .subscribe_with_snapshot()
        .unwrap_or_else(|error| panic!("subscribe: {error}"));
    let prepared = session();
    let prepared_identity = prepared.file_identity.clone();
    let expected_identity = before.identity.clone();
    handle
        .lock()
        .unwrap_or_else(|error| panic!("lock reload: {error}"))
        .dispatch(DocumentCommand::ReloadPreparedDocument {
            expected_revision: before.revision,
            expected_identity,
            prepared,
        })
        .unwrap_or_else(|error| panic!("reload: {error}"));
    let controller = handle
        .lock()
        .unwrap_or_else(|error| panic!("lock reloaded state: {error}"));
    assert_eq!(controller.session().revision(), 1);
    assert!(!controller.session().dirty);
    assert_eq!(controller.session().file_identity, prepared_identity);
    drop(controller);
    let events = subscription
        .poll()
        .unwrap_or_else(|error| panic!("poll reload: {error}"));
    assert!(
        events
            .iter()
            .any(|event| matches!(event, DocumentEvent::IdentityChanged { .. }))
    );
}

/// 验证准备 reload 后若 peer 已提交新 revision，确认重载也不能覆盖该编辑。
#[test]
fn confirmed_reload_rejects_revision_changed_after_preparation() {
    let handle = DocumentHandle::new(DocumentController::new(DocumentId::new(), session()));
    let (before, _subscription) = handle
        .subscribe_with_snapshot()
        .unwrap_or_else(|error| panic!("subscribe: {error}"));
    let expected_identity = before.identity.clone();
    let prepared = session();
    let peer_view = DocumentViewInstanceId::new();
    handle
        .lock()
        .unwrap_or_else(|error| panic!("lock peer registration: {error}"))
        .register_view(peer_view);

    handle
        .lock()
        .unwrap_or_else(|error| panic!("lock peer edit: {error}"))
        .dispatch(DocumentCommand::ApplyTransaction {
            view_id: peer_view,
            transaction_id: TransactionId(1),
            transaction: Transaction::new(before.revision, vec![SourceEdit::new(0..1, "t")]),
            selection_before: SourceSelection::default(),
            selection_after: SourceSelection::default(),
        })
        .unwrap_or_else(|error| panic!("peer edit: {error}"));

    let result = handle
        .lock()
        .unwrap_or_else(|error| panic!("lock stale reload: {error}"))
        .dispatch(DocumentCommand::ReloadPreparedDocumentAfterConfirmation {
            expected_revision: before.revision,
            expected_identity,
            prepared,
        });
    assert!(matches!(
        result,
        Err(ControllerError::ExternalRevisionMismatch { .. })
    ));

    let controller = handle
        .lock()
        .unwrap_or_else(|error| panic!("lock preserved peer edit: {error}"));
    assert_eq!(controller.session().revision(), before.revision.0 + 1);
    assert!(controller.session().dirty);
    assert_eq!(
        controller.session().snapshot().read_range(0..3).unwrap(),
        b"tne"
    );
}

#[test]
fn external_append_applies_prepared_tail_and_updates_revision_identity() {
    let directory = tempfile::tempdir().unwrap_or_else(|error| panic!("tempdir: {error}"));
    let path = directory.path().join("append.txt");
    fs::write(&path, "one").unwrap_or_else(|error| panic!("initial write: {error}"));
    let initial_source = FileSource::open(&path)
        .and_then(|source| source.identity())
        .unwrap_or_else(|error| panic!("initial identity: {error}"));
    let profile = DocumentProfile {
        len: 3,
        format: DocumentFormat::PlainText,
        encoding: TextEncoding::Utf8 { bom: false },
        estimated_lines: 1,
        estimated_structural_units: 0,
    };
    let expected_identity = FileIdentity::from(&initial_source);
    let session = DocumentSession::new(
        profile.clone(),
        DocumentStore::Resident(Box::new(ResidentDocument::new(
            "one",
            profile.encoding.clone(),
            initial_source,
        ))),
        LoadingPolicy::default().resolve(&profile),
        expected_identity.clone(),
    )
    .unwrap_or_else(|error| panic!("session: {error}"));
    let handle = DocumentHandle::new(DocumentController::new(DocumentId::new(), session));
    fs::write(&path, "one\ntwo").unwrap_or_else(|error| panic!("append write: {error}"));
    let source = FileSource::open(&path).unwrap_or_else(|error| panic!("append source: {error}"));
    let identity = FileIdentity::from(
        &source
            .identity()
            .unwrap_or_else(|error| panic!("append identity: {error}")),
    );
    let index = LineIndex::build(&source).unwrap_or_else(|error| panic!("append index: {error}"));
    handle
        .lock()
        .unwrap_or_else(|error| panic!("lock append: {error}"))
        .dispatch(DocumentCommand::AcceptExternalAppend {
            expected_revision: DocumentRevision(0),
            expected_identity,
            source,
            index,
            identity: identity.clone(),
        })
        .unwrap_or_else(|error| panic!("append transition: {error}"));
    let controller = handle
        .lock()
        .unwrap_or_else(|error| panic!("lock appended state: {error}"));
    assert_eq!(controller.session().revision(), 1);
    assert!(!controller.session().dirty);
    assert_eq!(controller.session().file_identity, identity);
    assert_eq!(
        controller.session().snapshot().read_range(0..7).unwrap(),
        b"one\ntwo"
    );
}

#[test]
fn save_as_reservation_rejects_collision_and_releases_failed_target() {
    let registry = DocumentRegistry::default();
    let source_identity = session().file_identity.clone();
    let source_key = DocumentRegistryKey::for_file(&source_identity);
    let (source, source_lease, _) = registry
        .open_or_insert_leased(source_key.clone(), || {
            Ok(DocumentController::new(DocumentId::new(), session()))
        })
        .unwrap_or_else(|error| panic!("source: {error}"));
    let target = DocumentRegistryKey::Untitled(DocumentId::new());
    let reservation = registry
        .reserve_save_as(&source, target.clone())
        .unwrap_or_else(|error| panic!("reserve: {error}"));
    assert!(matches!(
        registry.reserve_save_as(&source, target.clone()),
        Err(ControllerError::KeyReserved(_))
    ));
    reservation.release();
    let reservation = registry
        .reserve_save_as(&source, target.clone())
        .unwrap_or_else(|error| panic!("reserve after release: {error}"));
    let committed = reservation
        .commit()
        .unwrap_or_else(|error| panic!("commit: {error}"));
    assert!(Arc::ptr_eq(&source.0, &committed.0));
    let target_identity = FileIdentity {
        canonical_path: PathBuf::from("save-as-target.txt"),
        len: source_identity.len,
        modified_nanos: source_identity.modified_nanos,
        platform_id: source_identity.platform_id.clone(),
    };
    {
        let mut controller = source
            .lock()
            .unwrap_or_else(|error| panic!("lock save-as identity: {error}"));
        controller
            .dispatch(DocumentCommand::RequestSave)
            .unwrap_or_else(|error| panic!("save-as request: {error}"));
        controller
            .dispatch(DocumentCommand::SaveSucceeded {
                revision: DocumentRevision(0),
                identity: target_identity.clone(),
            })
            .unwrap_or_else(|error| panic!("save-as completion: {error}"));
        assert_eq!(controller.session().file_identity, target_identity);
    }

    let (target_handle, target_lease, target_state) = registry
        .open_or_insert_leased(target.clone(), || {
            Err(ControllerError::open_failed("target should be ready"))
        })
        .unwrap_or_else(|error| panic!("target open: {error}"));
    assert_eq!(target_state, RegistryOpen::Existing);
    assert!(Arc::ptr_eq(&target_handle.0, &source.0));
    let occupied = registry
        .reserve_save_as_outcome(&source, target.clone())
        .unwrap_or_else(|error| panic!("occupied outcome: {error}"));
    let occupied_lease = match occupied {
        SaveAsReserveOutcome::Occupied { handle, lease } => {
            assert!(Arc::ptr_eq(&handle.0, &source.0));
            lease
        }
        SaveAsReserveOutcome::Reserved(_) => panic!("occupied target was reserved"),
    };
    drop(occupied_lease);

    let (old_handle, old_lease, old_state) = registry
        .open_or_insert_leased(source_key, || {
            Ok(DocumentController::new(DocumentId::new(), session()))
        })
        .unwrap_or_else(|error| panic!("old path open: {error}"));
    assert_eq!(old_state, RegistryOpen::Inserted);
    assert!(!Arc::ptr_eq(&old_handle.0, &source.0));
    drop(target_lease);
    drop(old_lease);
    drop(source_lease);
    let (_, target_lease, target_state) = registry
        .open_or_insert_leased(target, || {
            Ok(DocumentController::new(DocumentId::new(), session()))
        })
        .unwrap_or_else(|error| panic!("target reopen: {error}"));
    assert_eq!(target_state, RegistryOpen::Inserted);
    drop(target_lease);
}

#[test]
fn save_as_occupied_outcome_returns_existing_handle_without_merging_documents() {
    let registry = DocumentRegistry::default();
    let source_identity = session().file_identity.clone();
    let source_key = DocumentRegistryKey::for_file(&source_identity);
    let (source, source_lease, _) = registry
        .open_or_insert_leased(source_key, || {
            Ok(DocumentController::new(DocumentId::new(), session()))
        })
        .unwrap_or_else(|error| panic!("source: {error}"));
    let target_key = DocumentRegistryKey::Untitled(DocumentId::new());
    let (existing, existing_lease, _) = registry
        .open_or_insert_leased(target_key.clone(), || {
            Ok(DocumentController::new(DocumentId::new(), session()))
        })
        .unwrap_or_else(|error| panic!("target: {error}"));
    let existing_id = existing
        .lock()
        .unwrap_or_else(|error| panic!("existing lock: {error}"))
        .document_id();
    let outcome = registry
        .reserve_save_as_outcome(&source, target_key)
        .unwrap_or_else(|error| panic!("occupied outcome: {error}"));
    let lease = match outcome {
        SaveAsReserveOutcome::Occupied { handle, lease } => {
            assert!(Arc::ptr_eq(&handle.0, &existing.0));
            assert_eq!(
                handle
                    .lock()
                    .unwrap_or_else(|error| panic!("occupied handle lock: {error}"))
                    .document_id(),
                existing_id
            );
            lease
        }
        SaveAsReserveOutcome::Reserved(_) => panic!("occupied target was reserved"),
    };
    drop(lease);
    drop(existing_lease);
    drop(source_lease);
}

#[test]
fn external_conflict_is_broadcast_with_a_monotonic_sequence() {
    let handle = DocumentHandle::new(DocumentController::new(DocumentId::new(), session()));
    let (snapshot, mut subscription) = handle
        .subscribe_with_snapshot()
        .unwrap_or_else(|error| panic!("subscribe: {error}"));
    let identity = session().file_identity.clone();
    handle
        .lock()
        .unwrap_or_else(|error| panic!("lock: {error}"))
        .dispatch(DocumentCommand::ExternalConflict {
            identity: identity.clone(),
        })
        .unwrap_or_else(|error| panic!("conflict: {error}"));
    let event = subscription
        .poll()
        .unwrap_or_else(|error| panic!("poll: {error}"))
        .into_iter()
        .find(|event| matches!(event, DocumentEvent::ExternalConflict { .. }));
    assert!(
        matches!(event, Some(DocumentEvent::ExternalConflict { sequence, .. }) if sequence > snapshot.sequence)
    );
}

#[test]
fn document_id_round_trips_as_a_uuid() {
    let id = DocumentId::new();
    let encoded = serde_json::to_string(&id).unwrap_or_else(|error| panic!("encode: {error}"));
    let decoded: DocumentId =
        serde_json::from_str(&encoded).unwrap_or_else(|error| panic!("decode: {error}"));
    assert_eq!(id, decoded);
    assert_ne!(id, DocumentId::new());
}

#[test]
fn lagging_subscription_reports_a_typed_gap_after_bounded_reclamation() {
    let handle = DocumentHandle::new(DocumentController::new(DocumentId::new(), session()));
    let (_snapshot, mut subscription) = handle
        .subscribe_with_snapshot()
        .unwrap_or_else(|error| panic!("subscribe: {error}"));
    let identity = session().file_identity.clone();
    for _ in 0..4_200 {
        handle
            .lock()
            .unwrap_or_else(|error| panic!("lock: {error}"))
            .dispatch(DocumentCommand::ExternalConflict {
                identity: identity.clone(),
            })
            .unwrap_or_else(|error| panic!("conflict: {error}"));
    }
    assert!(matches!(
        subscription.poll(),
        Err(ControllerError::SubscriptionLagged { .. })
    ));
}
