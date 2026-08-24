// @author kongweiguang

use super::*;

/// Resident Markdown 的注册表键可以使用 UI 友好的规范路径，但保存快照必须保留
/// 文件适配器探测到的真实身份；Windows 下两种 canonical path 表示不同，混用会
/// 把未发生外部修改的第一次保存误判为 SourceChanged。
#[test]
fn resident_disk_open_keeps_runtime_identity_for_first_save() -> TestResult {
    let root = tempfile::tempdir()?;
    let path = write_fixture(root.path())?;
    let service = DocumentService::new();
    let policy = LoadingPolicy::default();
    let probe = service.probe_file(&path, policy, |normalized, loading| {
        crate::document_io::probe_document_with_policy(normalized, loading)
    })?;
    let opened = service.open_resident_file(&path, policy, |normalized, _| {
        crate::document_io::read_resident_text_from_probe(
            normalized,
            &probe,
            policy.effective_limits(),
        )
        .map(|opened| ResidentMarkdownSource::from_opened(normalized, opened))
    })?;
    let handle = opened.handle();
    handle
        .lock()
        .map_err(|error| test_error(error.to_string()))?
        .dispatch(DocumentCommand::ApplyTransaction {
            view_id: DocumentViewInstanceId::new(),
            transaction_id: gmark_document_runtime::TransactionId(1),
            transaction: Transaction::new(
                DocumentRevision(0),
                vec![SourceEdit::new(0..0, "saved\n")],
            ),
            selection_before: SourceSelection::default(),
            selection_after: SourceSelection::default(),
        })?;
    let snapshot = handle
        .request_save_snapshot()?
        .ok_or_else(|| test_error("first save request was not started"))?;
    let current_identity = FileSource::open(&path)?.identity()?;

    assert_eq!(
        snapshot.identity.canonical_path, current_identity.path,
        "resident open must not replace the filesystem identity with the registry path"
    );
    let saved_identity = snapshot
        .save_atomic_cancellable(&path, &gmark_paged_document::SearchCancellation::default())?;
    handle.complete_save(snapshot.revision, saved_identity)?;
    assert_eq!(std::fs::read_to_string(&path)?, "saved\n# shared\n");
    Ok(())
}

fn concurrent_file_open_runs_loader_once_and_shares_handle_and_leases() -> TestResult {
    let root = tempfile::tempdir()?;
    let path = write_fixture(root.path())?;
    let service = Arc::new(DocumentService::new());
    let loader_count = Arc::new(AtomicUsize::new(0));
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();

    let first_service = service.clone();
    let first_path = path.clone();
    let first_count = loader_count.clone();
    let first = thread::spawn(move || {
        first_service.open_resident_file(
            &first_path,
            LoadingPolicy::default(),
            move |normalized, _policy| {
                first_count.fetch_add(1, Ordering::SeqCst);
                started_tx
                    .send(())
                    .map_err(|error| test_error(error.to_string()))?;
                release_rx
                    .recv()
                    .map_err(|error| test_error(error.to_string()))?;
                Ok::<_, Box<dyn Error + Send + Sync>>(source(normalized, "# shared\n"))
            },
        )
    });

    started_rx
        .recv()
        .map_err(|error| test_error(error.to_string()))?;
    let second_service = service.clone();
    let second_path = path.clone();
    let second_count = loader_count.clone();
    let second = thread::spawn(move || {
        second_service.open_resident_file(
            &second_path,
            LoadingPolicy::default(),
            move |normalized, _policy| {
                second_count.fetch_add(1, Ordering::SeqCst);
                Ok::<_, Box<dyn Error + Send + Sync>>(source(
                    normalized,
                    "unexpected second body\n",
                ))
            },
        )
    });

    release_tx
        .send(())
        .map_err(|error| test_error(error.to_string()))?;
    let first = first
        .join()
        .map_err(|_| test_error("first open thread panicked"))??;
    let second = second
        .join()
        .map_err(|_| test_error("second open thread panicked"))??;

    assert_eq!(loader_count.load(Ordering::SeqCst), 1);
    assert_eq!(first.open, RegistryOpen::Inserted);
    assert_eq!(second.open, RegistryOpen::Existing);
    assert_eq!(first.document_id, second.document_id);
    assert_eq!(first.handle().lease_count(), 2);
    let first_handle = first.handle();
    let controller = first_handle
        .lock()
        .map_err(|error| test_error(error.to_string()))?;
    assert_eq!(controller.session().len(), 9);
    drop(controller);

    let shared_handle = first.handle();
    drop(first);
    assert_eq!(shared_handle.lease_count(), 1);
    drop(second);
    assert_eq!(shared_handle.lease_count(), 0);

    let reopened =
        service.open_resident_file(&path, LoadingPolicy::default(), |normalized, _policy| {
            loader_count.fetch_add(1, Ordering::SeqCst);
            Ok::<_, Box<dyn Error + Send + Sync>>(source(normalized, "# reopened\n"))
        })?;
    assert_eq!(reopened.open, RegistryOpen::Inserted);
    assert_eq!(loader_count.load(Ordering::SeqCst), 2);
    Ok(())
}

fn failed_open_wakes_waiters_and_allows_a_retry() -> TestResult {
    let root = tempfile::tempdir()?;
    let path = write_fixture(root.path())?;
    let service = Arc::new(DocumentService::new());
    let loader_count = Arc::new(AtomicUsize::new(0));
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();

    let first_service = service.clone();
    let first_path = path.clone();
    let first_count = loader_count.clone();
    let first = thread::spawn(move || {
        first_service.open_resident_file(
            &first_path,
            LoadingPolicy::default(),
            move |_normalized, _policy| {
                first_count.fetch_add(1, Ordering::SeqCst);
                started_tx
                    .send(())
                    .map_err(|error| test_error(error.to_string()))?;
                release_rx
                    .recv()
                    .map_err(|error| test_error(error.to_string()))?;
                Err::<ResidentMarkdownSource, Box<dyn Error + Send + Sync>>(Box::new(
                    io::Error::other("synthetic load failure"),
                ))
            },
        )
    });

    started_rx
        .recv()
        .map_err(|error| test_error(error.to_string()))?;
    let second_service = service.clone();
    let second_path = path.clone();
    let second_count = loader_count.clone();
    let (second_ready_tx, second_ready_rx) = mpsc::channel();
    let second = thread::spawn(move || {
        let _ = second_ready_tx.send(());
        second_service.open_resident_file(
            &second_path,
            LoadingPolicy::default(),
            move |_normalized, _policy| {
                second_count.fetch_add(1, Ordering::SeqCst);
                Err::<ResidentMarkdownSource, Box<dyn Error + Send + Sync>>(Box::new(
                    io::Error::other("unexpected second loader"),
                ))
            },
        )
    });

    // Give the waiter a deterministic chance to join the in-flight registry
    // slot before the owner publishes the failure and removes it for retry.
    second_ready_rx
        .recv()
        .map_err(|error| test_error(error.to_string()))?;
    thread::sleep(std::time::Duration::from_millis(100));
    release_tx
        .send(())
        .map_err(|error| test_error(error.to_string()))?;
    let first = first
        .join()
        .map_err(|_| test_error("first failure thread panicked"))?;
    let second = second
        .join()
        .map_err(|_| test_error("second failure thread panicked"))?;
    assert_eq!(loader_count.load(Ordering::SeqCst), 1);
    assert_eq!(
        first.as_ref().err().map(ToString::to_string),
        second.as_ref().err().map(ToString::to_string)
    );
    let first_error = first
        .err()
        .ok_or_else(|| test_error("first open unexpectedly succeeded"))?;
    assert!(first_error.to_string().contains("synthetic load failure"));

    let retry =
        service.open_resident_file(&path, LoadingPolicy::default(), |normalized, _policy| {
            loader_count.fetch_add(1, Ordering::SeqCst);
            Ok::<_, Box<dyn Error + Send + Sync>>(source(normalized, "# retry\n"))
        })?;
    assert_eq!(retry.open, RegistryOpen::Inserted);
    assert_eq!(loader_count.load(Ordering::SeqCst), 2);
    Ok(())
}

fn concurrent_probe_and_body_open_each_run_one_opening_owner() -> TestResult {
    let root = tempfile::tempdir()?;
    let path = write_fixture(root.path())?;
    let service = Arc::new(DocumentService::new());
    let probe_count = Arc::new(AtomicUsize::new(0));
    let body_count = Arc::new(AtomicUsize::new(0));
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();

    let first_service = service.clone();
    let first_path = path.clone();
    let first_probe_count = probe_count.clone();
    let first_body_count = body_count.clone();
    let first = thread::spawn(move || {
        let probe = first_service.probe_file(
            &first_path,
            LoadingPolicy::default(),
            |normalized, policy| {
                first_probe_count.fetch_add(1, Ordering::SeqCst);
                started_tx
                    .send(())
                    .map_err(|error| test_error(error.to_string()))?;
                release_rx
                    .recv()
                    .map_err(|error| test_error(error.to_string()))?;
                crate::document_io::probe_document_with_policy(normalized, policy)
                    .map_err(|error| test_error(error.to_string()))
            },
        )?;
        let opened = first_service.open_resident_file(
            &first_path,
            LoadingPolicy::default(),
            move |normalized, _| {
                first_body_count.fetch_add(1, Ordering::SeqCst);
                Ok::<_, io::Error>(source(normalized, "# single body\n"))
            },
        )?;
        Ok::<_, Box<dyn Error + Send + Sync>>((probe, opened))
    });

    started_rx
        .recv()
        .map_err(|error| test_error(error.to_string()))?;
    let second_service = service.clone();
    let second_path = path.clone();
    let second_probe_count = probe_count.clone();
    let second_body_count = body_count.clone();
    let second = thread::spawn(move || {
        let probe = second_service.probe_file(
            &second_path,
            LoadingPolicy::default(),
            |normalized, policy| {
                second_probe_count.fetch_add(1, Ordering::SeqCst);
                crate::document_io::probe_document_with_policy(normalized, policy)
                    .map_err(|error| test_error(error.to_string()))
            },
        )?;
        let opened = second_service.open_resident_file(
            &second_path,
            LoadingPolicy::default(),
            move |normalized, _| {
                second_body_count.fetch_add(1, Ordering::SeqCst);
                Ok::<_, io::Error>(source(normalized, "unexpected body\n"))
            },
        )?;
        Ok::<_, Box<dyn Error + Send + Sync>>((probe, opened))
    });

    release_tx
        .send(())
        .map_err(|error| test_error(error.to_string()))?;
    let (first_probe, first_open) = first
        .join()
        .map_err(|_| test_error("first combined open thread panicked"))??;
    let (second_probe, second_open) = second
        .join()
        .map_err(|_| test_error("second combined open thread panicked"))??;
    assert_eq!(probe_count.load(Ordering::SeqCst), 1);
    assert_eq!(body_count.load(Ordering::SeqCst), 1);
    assert_eq!(first_probe, second_probe);
    assert_ne!(first_open.open, second_open.open);
    assert!(matches!(
        (first_open.open, second_open.open),
        (RegistryOpen::Inserted, RegistryOpen::Existing)
            | (RegistryOpen::Existing, RegistryOpen::Inserted)
    ));
    Ok(())
}
fn concurrent_paged_host_open_runs_preparation_once_and_shares_controller() -> TestResult {
    let root = tempfile::tempdir()?;
    let path = root.path().join("records.txt");
    std::fs::write(&path, b"row 1\nrow 2\n")?;
    let policy = LoadingPolicy {
        max_resident_bytes: Some(1),
        force_safe_source: false,
    };
    let probe = crate::document_io::probe_document_with_policy(&path, policy)?;
    assert_eq!(probe.strategy, OpenStrategy::Paged);
    let service = Arc::new(DocumentService::new());
    let prepare_count = Arc::new(AtomicUsize::new(0));
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();

    let first_service = service.clone();
    let first_path = path.clone();
    let first_probe = probe.clone();
    let first_count = prepare_count.clone();
    let first = thread::spawn(move || {
        first_service.open_paged(
            &first_path,
            first_probe,
            policy,
            move |normalized, probe, _policy| {
                first_count.fetch_add(1, Ordering::SeqCst);
                started_tx
                    .send(())
                    .map_err(|error| test_error(error.to_string()))?;
                release_rx
                    .recv()
                    .map_err(|error| test_error(error.to_string()))?;
                let source =
                    FileSource::open(normalized).map_err(|error| test_error(error.to_string()))?;
                prepare_utf8_source(source, probe.encoding.clone())
                    .map_err(|error| test_error(error.to_string()))
            },
        )
    });

    started_rx
        .recv()
        .map_err(|error| test_error(error.to_string()))?;
    let second_service = service.clone();
    let second_path = path.clone();
    let second_probe = probe.clone();
    let second_count = prepare_count.clone();
    let second = thread::spawn(move || {
        second_service.open_paged(
            &second_path,
            second_probe,
            policy,
            move |normalized, probe, _policy| {
                second_count.fetch_add(1, Ordering::SeqCst);
                let source =
                    FileSource::open(normalized).map_err(|error| test_error(error.to_string()))?;
                prepare_utf8_source(source, probe.encoding.clone())
                    .map_err(|error| test_error(error.to_string()))
            },
        )
    });

    release_tx
        .send(())
        .map_err(|error| test_error(error.to_string()))?;
    let first = first
        .join()
        .map_err(|_| test_error("first paged open thread panicked"))??;
    let second = second
        .join()
        .map_err(|_| test_error("second paged open thread panicked"))??;
    assert_eq!(prepare_count.load(Ordering::SeqCst), 1);
    assert_eq!(first.open, RegistryOpen::Inserted);
    assert_eq!(second.open, RegistryOpen::Existing);
    assert_eq!(first.document_id, second.document_id);
    assert_eq!(first.handle().lease_count(), 2);
    let handle = first.handle();
    let controller = handle
        .lock()
        .map_err(|error| test_error(error.to_string()))?;
    assert_eq!(
        controller.session().store.kind(),
        DocumentBackendKind::Paged
    );
    assert_eq!(controller.session().len(), 12);
    drop(controller);
    drop(first);
    drop(second);
    assert_eq!(handle.lease_count(), 0);
    Ok(())
}

fn resident_format_host_open_uses_shared_resident_controller() -> TestResult {
    let root = tempfile::tempdir()?;
    let path = root.path().join("records.json");
    std::fs::write(&path, br#"{"items":[1,2]}"#)?;
    let policy = LoadingPolicy::default();
    let probe = crate::document_io::probe_document_with_policy(&path, policy)?;
    assert_eq!(probe.strategy, OpenStrategy::Resident);
    let service = DocumentService::new();
    let opened =
        service.open_document_host(&path, probe.clone(), policy, |normalized, probe, _| {
            let source = FileSource::open(normalized)?;
            prepare_utf8_source(source, probe.encoding.clone())
        })?;
    assert_eq!(opened.probe.format, probe.format);
    let handle = opened.handle();
    let controller = handle
        .lock()
        .map_err(|error| test_error(error.to_string()))?;
    assert_eq!(
        controller.session().store.kind(),
        DocumentBackendKind::Resident
    );
    assert_eq!(controller.session().profile.format, probe.format);
    Ok(())
}

#[test]
fn paged_host_open_failure_is_shared_then_retryable() -> TestResult {
    let root = tempfile::tempdir()?;
    let path = root.path().join("retry.txt");
    std::fs::write(&path, b"row 1\nrow 2\n")?;
    let policy = LoadingPolicy {
        max_resident_bytes: Some(1),
        force_safe_source: false,
    };
    let probe = crate::document_io::probe_document_with_policy(&path, policy)?;
    assert_eq!(probe.strategy, OpenStrategy::Paged);
    let service = DocumentService::new();
    let attempts = AtomicUsize::new(0);

    let failed = service.open_paged(&path, probe.clone(), policy, |_, _, _| {
        attempts.fetch_add(1, Ordering::SeqCst);
        Err(io::Error::other("synthetic preparation failure"))
    });
    assert!(matches!(
        failed,
        Err(DocumentServiceError::OpenFailed(message)) if message.contains("synthetic preparation failure")
    ));

    let opened = service.open_paged(&path, probe, policy, |normalized, probe, _| {
        attempts.fetch_add(1, Ordering::SeqCst);
        let source = FileSource::open(normalized)?;
        prepare_utf8_source(source, probe.encoding.clone())
    })?;
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
    assert_eq!(opened.open, RegistryOpen::Inserted);
    drop(opened);
    Ok(())
}

// 原因：超时后必须切换到新代次，旧 owner 的迟到 probe 结果不能污染重试。
#[test]
fn probe_waiter_timeout_does_not_run_a_second_loader() -> TestResult {
    let root = tempfile::tempdir()?;
    let path = write_fixture(root.path())?;
    let service = Arc::new(DocumentService::new());
    let loader_count = Arc::new(AtomicUsize::new(0));
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let owner_service = service.clone();
    let owner_path = path.clone();
    let owner_count = loader_count.clone();
    let owner = thread::spawn(move || {
        owner_service.probe_file_with_deadline(
            &owner_path,
            LoadingPolicy::default(),
            std::time::Duration::from_secs(5),
            |normalized, policy| {
                owner_count.fetch_add(1, Ordering::SeqCst);
                started_tx
                    .send(())
                    .map_err(|error| test_error(error.to_string()))?;
                release_rx
                    .recv()
                    .map_err(|error| test_error(error.to_string()))?;
                crate::document_io::probe_document_with_policy(normalized, policy)
                    .map_err(|error| test_error(error.to_string()))
            },
        )
    });
    started_rx
        .recv()
        .map_err(|error| test_error(error.to_string()))?;

    let waiter_started = std::time::Instant::now();
    let waiter = service.probe_file_with_deadline(
        &path,
        LoadingPolicy::default(),
        std::time::Duration::from_millis(20),
        |_normalized,
         _policy|
         -> Result<gmark_paged_document::OpenProbe, Box<dyn Error + Send + Sync>> {
            Err(test_error("waiter must subscribe to the owner"))
        },
    );
    assert!(waiter_started.elapsed() < std::time::Duration::from_secs(1));
    assert!(matches!(
        waiter,
        Err(DocumentServiceError::OpenFailed(message)) if message.contains("timed out")
    ));

    let retry = service.probe_file_with_deadline(
        &path,
        LoadingPolicy::default(),
        std::time::Duration::from_secs(1),
        |normalized, policy| {
            loader_count.fetch_add(1, Ordering::SeqCst);
            crate::document_io::probe_document_with_policy(normalized, policy)
                .map_err(|error| test_error(error.to_string()))
        },
    )?;
    assert_eq!(retry.strategy, OpenStrategy::Resident);

    release_tx
        .send(())
        .map_err(|error| test_error(error.to_string()))?;
    owner
        .join()
        .map_err(|_| test_error("probe owner panicked"))??;
    assert_eq!(loader_count.load(Ordering::SeqCst), 2);
    Ok(())
}

// 原因：owner 失败需要广播同一错误并清理槽位，使后续同路径打开可以重试。
#[test]
fn probe_owner_failure_is_broadcast_and_next_generation_can_retry() -> TestResult {
    let root = tempfile::tempdir()?;
    let path = write_fixture(root.path())?;
    let service = Arc::new(DocumentService::new());
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let owner_service = service.clone();
    let owner_path = path.clone();
    let owner = thread::spawn(move || {
        owner_service.probe_file_with_deadline(
            &owner_path,
            LoadingPolicy::default(),
            std::time::Duration::from_secs(1),
            |_normalized, _policy| {
                started_tx
                    .send(())
                    .map_err(|error| test_error(error.to_string()))?;
                release_rx
                    .recv()
                    .map_err(|error| test_error(error.to_string()))?;
                Err::<gmark_paged_document::OpenProbe, Box<dyn Error + Send + Sync>>(test_error(
                    "probe owner failed",
                ))
            },
        )
    });
    started_rx
        .recv()
        .map_err(|error| test_error(error.to_string()))?;
    let waiter_service = service.clone();
    let waiter_path = path.clone();
    let waiter = thread::spawn(move || {
        waiter_service.probe_file_with_deadline(
            &waiter_path,
            LoadingPolicy::default(),
            std::time::Duration::from_secs(1),
            |_normalized, _policy| {
                Err::<gmark_paged_document::OpenProbe, Box<dyn Error + Send + Sync>>(test_error(
                    "waiter must not load",
                ))
            },
        )
    });
    thread::sleep(std::time::Duration::from_millis(100));
    release_tx
        .send(())
        .map_err(|error| test_error(error.to_string()))?;
    let owner = owner
        .join()
        .map_err(|_| test_error("probe owner panicked"))?;
    let owner_error = owner
        .err()
        .ok_or_else(|| test_error("probe owner unexpectedly succeeded"))?;
    let waiter_error = waiter
        .join()
        .map_err(|_| test_error("probe waiter panicked"))?
        .err()
        .ok_or_else(|| test_error("probe waiter unexpectedly succeeded"))?;
    assert_eq!(owner_error.to_string(), waiter_error.to_string());
    assert!(owner_error.to_string().contains("probe owner failed"));

    let retry = service.probe_file_with_deadline(
        &path,
        LoadingPolicy::default(),
        std::time::Duration::from_secs(1),
        |normalized, policy| {
            crate::document_io::probe_document_with_policy(normalized, policy)
                .map_err(|error| test_error(error.to_string()))
        },
    )?;
    assert_eq!(retry.strategy, OpenStrategy::Resident);
    Ok(())
}

#[test]
fn untitled_and_recovery_uuid_keys_preserve_identity() -> TestResult {
    let service = DocumentService::new();
    let untitled_id = DocumentId::new();
    let first = service.open_untitled(
        Some(untitled_id),
        source(Path::new("untitled.md"), "untitled"),
    )?;
    let second =
        service.open_untitled(Some(untitled_id), source(Path::new("other.md"), "ignored"))?;
    assert_eq!(first.open, RegistryOpen::Inserted);
    assert_eq!(second.open, RegistryOpen::Existing);
    assert_eq!(first.document_id, untitled_id);
    assert_eq!(second.document_id, untitled_id);
    drop(first);
    drop(second);

    let recovery_id = DocumentId::new();
    let recovery = service.open_recovery(
        recovery_id,
        source(Path::new("recovery.md"), "recovered body"),
    )?;
    assert_eq!(recovery.document_id, recovery_id);
    let recovery_handle = recovery.handle();
    let controller = recovery_handle
        .lock()
        .map_err(|error| test_error(error.to_string()))?;
    assert!(controller.session().dirty);
    Ok(())
}
