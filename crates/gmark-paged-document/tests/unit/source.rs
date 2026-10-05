// @author kongweiguang

use super::PageCache;
use std::sync::Arc;

fn page(value: u8) -> Arc<[u8]> {
    Arc::from([value])
}

#[test]
fn mature_lru_keeps_recent_hits_and_evicts_the_oldest_page() {
    let mut cache = PageCache::with_capacity(2);
    cache.insert(1, page(1));
    cache.insert(2, page(2));

    assert_eq!(cache.get(1).as_deref(), Some([1].as_slice()));
    cache.insert(3, page(3));

    assert!(cache.get(2).is_none());
    assert_eq!(cache.get(1).as_deref(), Some([1].as_slice()));
    assert_eq!(cache.get(3).as_deref(), Some([3].as_slice()));
    assert_eq!(cache.len(), 2);
}

#[cfg(windows)]
#[test]
/// 已打开且允许 delete sharing 的 reader 仍必须走原子替换路径，不能因重试改变保存语义。
fn shared_source_allows_atomic_replacement_while_open() {
    use std::io::Write;

    let directory = tempfile::tempdir().expect("tempdir");
    let path = directory.path().join("source.txt");
    std::fs::write(&path, b"old").expect("write source");
    let source = super::super::FileSource::open(&path).expect("open source");
    let mut temporary = tempfile::NamedTempFile::new_in(directory.path()).expect("temp file");
    temporary.write_all(b"new").expect("write replacement");
    temporary.as_file().sync_all().expect("sync replacement");
    let temporary_path = temporary.into_temp_path();

    super::replace_existing_windows(
        &temporary_path,
        &path,
        &super::super::SearchCancellation::default(),
    )
    .expect("replace open source");
    assert_eq!(std::fs::read(&path).expect("read replacement"), b"new");
    drop(source);
}

#[cfg(windows)]
#[test]
/// 退避期间释放禁止 delete sharing 的句柄后，应对同一原文件进行下一次原子尝试。
fn windows_replace_retries_after_sharing_lock_is_released() {
    use std::fs::OpenOptions;
    use std::io::Write;
    use std::os::windows::fs::OpenOptionsExt;

    const FILE_SHARE_READ: u32 = 0x0000_0001;
    const FILE_SHARE_WRITE: u32 = 0x0000_0002;

    let directory = tempfile::tempdir().expect("tempdir");
    let path = directory.path().join("source.txt");
    std::fs::write(&path, b"old").expect("write source");
    let mut temporary = tempfile::NamedTempFile::new_in(directory.path()).expect("temp file");
    temporary.write_all(b"new").expect("write replacement");
    temporary.as_file().sync_all().expect("sync replacement");
    let temporary_path = temporary.into_temp_path();
    let mut blocker = Some(
        OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .open(&path)
            .expect("open deny-delete handle"),
    );
    let original_identity = super::super::FileSource::open(&path)
        .expect("open source identity")
        .identity()
        .expect("read source identity");
    let mut delays = Vec::new();

    super::replace_existing_windows_with_retry(
        &temporary_path,
        &path,
        &original_identity,
        &super::super::SearchCancellation::default(),
        super::replace_existing_windows_once,
        |delay| {
            delays.push(delay);
            drop(blocker.take());
        },
    )
    .expect("replace after releasing transient lock");

    assert_eq!(delays, [std::time::Duration::from_millis(20)]);
    assert_eq!(std::fs::read(&path).expect("read replacement"), b"new");
}

#[cfg(windows)]
#[test]
/// 持续禁止 delete sharing 时必须保留原文，并由 TempPath 清理未提交的暂存文件。
fn windows_persist_gives_up_on_permanent_sharing_lock() {
    use std::fs::OpenOptions;
    use std::io::Write;
    use std::os::windows::fs::OpenOptionsExt;

    const FILE_SHARE_READ: u32 = 0x0000_0001;
    const FILE_SHARE_WRITE: u32 = 0x0000_0002;

    let directory = tempfile::tempdir().expect("tempdir");
    let path = directory.path().join("source.txt");
    std::fs::write(&path, b"old").expect("write source");
    let mut temporary = tempfile::NamedTempFile::new_in(directory.path()).expect("temp file");
    temporary.write_all(b"new").expect("write replacement");
    temporary.as_file().sync_all().expect("sync replacement");
    let temporary_path = temporary.path().to_path_buf();
    let blocker = OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .open(&path)
        .expect("open deny-delete handle");

    let result = super::persist_temporary(
        temporary,
        &path,
        &super::super::SearchCancellation::default(),
    );

    let error = result.expect_err("permanent sharing lock should fail without replacing");
    assert!(matches!(
        &error,
        super::super::PagedDocumentError::Persist { .. }
    ));
    assert!(
        !error.target_may_have_changed(),
        "confirmed no-commit failure should keep the retry-save UI"
    );
    assert_eq!(std::fs::read(&path).expect("read original"), b"old");
    assert!(!temporary_path.exists(), "failed temp must be removed");
    drop(blocker);
}

#[cfg(windows)]
#[test]
/// 首次瞬态错误后若目标身份变化，必须在下一次 ReplaceFile 前停止，保留外部内容。
fn windows_replace_retry_refuses_a_changed_destination() {
    use std::io::Write;

    let directory = tempfile::tempdir().expect("tempdir");
    let path = directory.path().join("source.txt");
    std::fs::write(&path, b"old").expect("write source");
    let mut temporary = tempfile::NamedTempFile::new_in(directory.path()).expect("temp file");
    temporary.write_all(b"new").expect("write replacement");
    temporary.as_file().sync_all().expect("sync replacement");
    let original_identity = super::super::FileSource::open(&path)
        .expect("open source identity")
        .identity()
        .expect("read source identity");
    let mut attempts = 0;
    let mut delays = Vec::new();

    let result = super::replace_existing_windows_with_retry(
        temporary.path(),
        &path,
        &original_identity,
        &super::super::SearchCancellation::default(),
        |_, destination| {
            attempts += 1;
            std::fs::write(destination, b"external")?;
            Err(std::io::Error::from_raw_os_error(1175))
        },
        |delay| delays.push(delay),
    );

    let error = super::windows_persist_error(
        &path,
        result.expect_err("changed destination must refuse retry"),
    );
    assert!(matches!(
        error,
        super::super::PagedDocumentError::SourceChanged
    ));
    assert_eq!(attempts, 1);
    assert_eq!(delays, [std::time::Duration::from_millis(20)]);
    assert_eq!(
        std::fs::read(&path).expect("read external change"),
        b"external"
    );
}

#[cfg(windows)]
#[test]
/// 替换系统调用返回不确定错误时，即使取消同时到达也必须保留冲突刷新信号。
fn windows_replace_preserves_uncertain_error_after_cancellation() {
    use std::io::Write;

    let directory = tempfile::tempdir().expect("tempdir");
    let path = directory.path().join("source.txt");
    std::fs::write(&path, b"old").expect("write source");
    let mut temporary = tempfile::NamedTempFile::new_in(directory.path()).expect("temp file");
    temporary.write_all(b"new").expect("write replacement");
    temporary.as_file().sync_all().expect("sync replacement");
    let original_identity = super::super::FileSource::open(&path)
        .expect("open source identity")
        .identity()
        .expect("read source identity");
    let cancellation = super::super::SearchCancellation::default();
    let mut attempts = 0;

    let result = super::replace_existing_windows_with_retry(
        temporary.path(),
        &path,
        &original_identity,
        &cancellation,
        |_, _| {
            attempts += 1;
            cancellation.cancel();
            Err(std::io::Error::from_raw_os_error(1176))
        },
        |_| panic!("uncertain errors must not enter the retry wait"),
    );

    let error = super::windows_persist_error(
        &path,
        result.expect_err("uncertain replacement error must be preserved"),
    );
    assert_eq!(attempts, 1);
    match &error {
        super::super::PagedDocumentError::Persist { source, .. } => {
            assert_eq!(source.raw_os_error(), Some(1176));
        }
        error => panic!("expected persist error, received {error:?}"),
    }
    assert!(error.target_may_have_changed());
}

#[cfg(windows)]
#[test]
/// 即使 1175 持续出现，也只能执行初次尝试加三次有界重试。
fn windows_replace_retry_has_a_fixed_attempt_budget() {
    use std::io::Write;

    let directory = tempfile::tempdir().expect("tempdir");
    let path = directory.path().join("source.txt");
    std::fs::write(&path, b"old").expect("write source");
    let mut temporary = tempfile::NamedTempFile::new_in(directory.path()).expect("temp file");
    temporary.write_all(b"new").expect("write replacement");
    temporary.as_file().sync_all().expect("sync replacement");
    let original_identity = super::super::FileSource::open(&path)
        .expect("open source identity")
        .identity()
        .expect("read source identity");
    let mut attempts = 0;
    let mut delays = Vec::new();

    let result = super::replace_existing_windows_with_retry(
        temporary.path(),
        &path,
        &original_identity,
        &super::super::SearchCancellation::default(),
        |_, _| {
            attempts += 1;
            Err(std::io::Error::from_raw_os_error(1175))
        },
        |delay| delays.push(delay),
    );

    let error = super::windows_persist_error(&path, result.expect_err("retry budget exhausted"));
    assert!(!error.target_may_have_changed());
    let source = match &error {
        super::super::PagedDocumentError::Persist { source, .. } => source,
        error => panic!("expected confirmed non-commit error, received {error:?}"),
    };
    let cause = source.get_ref().expect("non-commit marker");
    let original = std::error::Error::source(cause)
        .expect("original ReplaceFile error")
        .downcast_ref::<std::io::Error>()
        .expect("original I/O error");
    assert_eq!(original.raw_os_error(), Some(1175));
    assert_eq!(attempts, 4);
    assert_eq!(
        delays,
        [
            std::time::Duration::from_millis(20),
            std::time::Duration::from_millis(40),
            std::time::Duration::from_millis(80),
        ]
    );
    assert_eq!(std::fs::read(&path).expect("read original"), b"old");
}

#[cfg(windows)]
#[test]
/// 退避中取消必须阻止第二次替换，保留原目标并让暂存路径正常清理。
fn windows_replace_cancellation_keeps_original_and_cleans_temporary() {
    use std::io::Write;

    let directory = tempfile::tempdir().expect("tempdir");
    let path = directory.path().join("source.txt");
    std::fs::write(&path, b"old").expect("write source");
    let mut temporary = tempfile::NamedTempFile::new_in(directory.path()).expect("temp file");
    temporary.write_all(b"new").expect("write replacement");
    temporary.as_file().sync_all().expect("sync replacement");
    let temporary_path = temporary.into_temp_path();
    let staged_path = temporary_path.to_path_buf();
    let original_identity = super::super::FileSource::open(&path)
        .expect("open source identity")
        .identity()
        .expect("read source identity");
    let cancellation = super::super::SearchCancellation::default();
    let mut attempts = 0;

    let result = super::replace_existing_windows_with_retry(
        &temporary_path,
        &path,
        &original_identity,
        &cancellation,
        |_, _| {
            attempts += 1;
            Err(std::io::Error::from_raw_os_error(1175))
        },
        |_| cancellation.cancel(),
    );

    let source = result.expect_err("cancelled retry");
    assert_eq!(source.kind(), std::io::ErrorKind::Interrupted);
    assert!(matches!(
        super::windows_persist_error(&path, source),
        super::super::PagedDocumentError::Cancelled
    ));
    assert_eq!(attempts, 1);
    assert_eq!(std::fs::read(&path).expect("read original"), b"old");
    drop(temporary_path);
    assert!(!staged_path.exists(), "cancelled temp must be removed");
}

#[cfg(windows)]
#[test]
/// 替换前已取消时应返回领域取消结果，且不得让暂存文件遗留在磁盘。
fn windows_persist_cancelled_before_replace_keeps_original_and_cleans_temporary() {
    use std::io::Write;

    let directory = tempfile::tempdir().expect("tempdir");
    let path = directory.path().join("source.txt");
    std::fs::write(&path, b"old").expect("write source");
    let mut temporary = tempfile::NamedTempFile::new_in(directory.path()).expect("temp file");
    temporary.write_all(b"new").expect("write replacement");
    temporary.as_file().sync_all().expect("sync replacement");
    let temporary_path = temporary.path().to_path_buf();
    let cancellation = super::super::SearchCancellation::default();
    cancellation.cancel();

    let result = super::persist_temporary(temporary, &path, &cancellation);

    assert!(matches!(
        result,
        Err(super::super::PagedDocumentError::Cancelled)
    ));
    assert_eq!(std::fs::read(&path).expect("read original"), b"old");
    assert!(!temporary_path.exists(), "cancelled temp must be removed");
}
