// @author kongweiguang

use super::*;

#[test]
fn probe_routes_large_json_without_loading_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("records.jsonl");
    let line = b"{\"id\":1,\"name\":\"person\"}\n";
    let mut contents = Vec::new();
    for _ in 0..2_000 {
        contents.extend_from_slice(line);
    }
    fs::write(&path, &contents).unwrap();

    let probe = probe_file(
        &path,
        ProbeOptions {
            max_resident_bytes: 1_024,
            ..ProbeOptions::default()
        },
    )
    .unwrap();
    assert_eq!(probe.len, contents.len() as u64);
    assert_eq!(probe.format, DocumentFormat::JsonLines);
    assert_eq!(probe.encoding, TextEncoding::Utf8 { bom: false });
    assert_eq!(probe.strategy, OpenStrategy::Paged);
}

#[test]
fn probe_rejects_sparse_nul_and_control_heavy_binary_samples() {
    let dir = tempfile::tempdir().unwrap();
    let sparse_nul = dir.path().join("sparse-nul.txt");
    let mut sparse_bytes = vec![b'A'; 4_096];
    sparse_bytes[2_048] = 0;
    fs::write(&sparse_nul, sparse_bytes).unwrap();
    assert!(matches!(
        probe_file(&sparse_nul, ProbeOptions::default()),
        Err(PagedDocumentError::Binary)
    ));

    let controls = dir.path().join("controls.txt");
    fs::write(&controls, vec![0x01; 4_096]).unwrap();
    assert!(matches!(
        probe_file(&controls, ProbeOptions::default()),
        Err(PagedDocumentError::Binary)
    ));
}

#[test]
fn probe_keeps_control_free_legacy_text_source_backed() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("legacy.txt");
    let bytes = b"caf\xe9 r\xe9sum\xe9\r\n".repeat(128);
    fs::write(&path, bytes).unwrap();

    let probe = probe_file(&path, ProbeOptions::default()).unwrap();
    assert!(matches!(probe.encoding, TextEncoding::Legacy(_)));
    assert_eq!(probe.format, DocumentFormat::PlainText);
}

/// 区分探测样本截断与文件本身的 EOF 截断，仅前者可按 UTF-8 前缀处理。
#[test]
fn probe_recognizes_utf8_when_head_sample_cuts_a_multibyte_scalar() {
    let dir = tempfile::tempdir().unwrap();
    let sample_bytes = ProbeOptions::default().sample_bytes;

    for scalar in ["é", "世", "😀"] {
        let path = dir.path().join(format!("boundary-{}.txt", scalar.len()));
        let scalar_bytes = scalar.as_bytes();
        let mut bytes = vec![b'a'; sample_bytes - (scalar_bytes.len() - 1)];
        bytes.extend_from_slice(scalar_bytes);
        bytes.extend(std::iter::repeat_n(b'z', sample_bytes * 2));
        fs::write(&path, bytes).unwrap();

        let probe = probe_file(
            &path,
            ProbeOptions {
                max_resident_bytes: 1,
                ..ProbeOptions::default()
            },
        )
        .unwrap();

        assert_eq!(
            probe.encoding,
            TextEncoding::Utf8 { bom: false },
            "{scalar}"
        );
        assert_eq!(probe.strategy, OpenStrategy::Paged);
    }
}

/// 文件真实 EOF 的不完整 UTF-8 仍须走编码回退，不能误判为完整 UTF-8。
#[test]
fn probe_does_not_accept_incomplete_utf8_at_eof_as_utf8() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("incomplete-eof.txt");
    let mut bytes = vec![b'a'; 128];
    bytes.extend_from_slice(&"😀".as_bytes()[..3]);
    fs::write(&path, bytes).unwrap();

    let probe = probe_file(&path, ProbeOptions::default()).unwrap();

    assert!(matches!(probe.encoding, TextEncoding::Legacy(_)));
}

/// 样本内部的非法 UTF-8 字节仍须走现有编码回退，容忍范围仅限样本末尾。
#[test]
fn probe_does_not_accept_an_invalid_utf8_byte_inside_the_head_sample() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("invalid-inside-sample.txt");
    let mut bytes = b"valid text before ".to_vec();
    bytes.push(0xff);
    bytes.extend_from_slice(b" and after\n");
    fs::write(&path, bytes).unwrap();

    let probe = probe_file(&path, ProbeOptions::default()).unwrap();

    assert!(matches!(probe.encoding, TextEncoding::Legacy(_)));
}

#[test]
fn full_index_scan_rejects_binary_bytes_hidden_between_probe_samples() {
    let dir = tempfile::tempdir().unwrap();
    for (name, hidden) in [("middle-nul.txt", 0), ("middle-invalid-utf8.txt", 0xff)] {
        let path = dir.path().join(name);
        let mut bytes = vec![b'a'; 256 * 1024];
        bytes[128 * 1024] = hidden;
        fs::write(&path, bytes).unwrap();
        let probe = probe_file(&path, ProbeOptions::default()).unwrap();
        assert_eq!(probe.encoding, TextEncoding::Utf8 { bom: false });
        let source = FileSource::open(&path).unwrap();
        assert!(matches!(
            LineIndex::build(&source),
            Err(PagedDocumentError::Binary)
        ));
    }
}

#[test]
fn full_index_utf8_validation_streams_across_scan_boundaries_and_appends() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("boundary.txt");
    let mut bytes = vec![b'a'; 8 * 1024 * 1024 - 1];
    bytes.extend_from_slice("🙂\n".as_bytes());
    fs::write(&path, bytes).unwrap();
    let source = FileSource::open(&path).unwrap();
    let index = LineIndex::build(&source).unwrap();
    assert_eq!(index.line_count(), 2);

    let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
    file.write_all(&[0]).unwrap();
    file.sync_all().unwrap();
    let appended = FileSource::open(&path).unwrap();
    assert!(matches!(
        index.extend_for_append(&appended),
        Err(PagedDocumentError::Binary)
    ));
}

#[test]
fn line_index_resolves_mixed_line_endings_and_final_line() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mixed.txt");
    fs::write(&path, b"alpha\r\nbeta\ngamma").unwrap();
    let source = FileSource::open(&path).unwrap();
    let index = LineIndex::build(&source).unwrap();

    assert_eq!(index.line_count(), 3);
    assert_eq!(index.line_range(0), Some(0..7));
    assert_eq!(index.line_range(1), Some(7..12));
    assert_eq!(index.line_range(2), Some(12..17));
    assert_eq!(index.line_range(3), None);
}

#[test]
fn source_selection_exports_atomically_without_changing_the_document() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("source.txt");
    let exported = dir.path().join("selection.txt");
    fs::write(&path, "alpha\n世界\nomega\n").unwrap();
    let source = FileSource::open(&path).unwrap();
    let index = LineIndex::build(&source).unwrap();
    let document = PagedDocument::new(PieceDocument::open(source, index).unwrap());

    document
        .save_range_atomic_cancellable(
            6.."alpha\n世界\n".len() as u64,
            &exported,
            &SearchCancellation::default(),
        )
        .unwrap();

    assert_eq!(fs::read_to_string(exported).unwrap(), "世界\n");
    assert!(document.is_pristine());
    assert_eq!(document.len(), "alpha\n世界\nomega\n".len() as u64);

    let missing_parent = dir.path().join("missing").join("selection.txt");
    assert!(
        document
            .save_range_atomic_cancellable(
                6.."alpha\n世界\n".len() as u64,
                &missing_parent,
                &SearchCancellation::default(),
            )
            .is_err()
    );
    assert!(document.is_pristine());
}

#[test]
fn immutable_selection_read_honors_pre_cancelled_tasks() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cancelled-copy.txt");
    fs::write(&path, "alpha\n世界\n").unwrap();
    let source = FileSource::open(&path).unwrap();
    let index = LineIndex::build(&source).unwrap();
    let document = PagedDocument::new(PieceDocument::open(source, index).unwrap());
    let cancellation = SearchCancellation::default();
    cancellation.cancel();

    assert!(matches!(
        document.read_range_cancellable(0..document.len(), &cancellation),
        Err(gmark_paged_document::PagedDocumentError::Cancelled)
    ));
}

#[test]
fn immutable_selection_snapshot_is_stable_while_the_live_document_keeps_editing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("copy-snapshot.txt");
    fs::write(&path, "alpha\n世界\nomega\n").unwrap();
    let source = FileSource::open(&path).unwrap();
    let index = LineIndex::build(&source).unwrap();
    let mut live = PagedDocument::new(PieceDocument::open(source, index).unwrap());
    let command_range = 6..12;

    // Copy 在命令触发时克隆持久 PieceTree 根；后续编辑只能生成新根，不能改变本次结果。
    let command_snapshot = live.clone();
    live.replace_text(command_range.clone(), "updated").unwrap();

    assert_eq!(
        command_snapshot
            .read_range_cancellable(command_range, &SearchCancellation::default())
            .unwrap(),
        "世界".as_bytes()
    );
    assert_eq!(
        live.read_range(0..live.len()).unwrap(),
        "alpha\nupdated\nomega\n".as_bytes()
    );
}

#[test]
fn opened_file_source_keeps_the_command_generation_after_external_replacement() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("external-copy-source.txt");
    let replacement = dir.path().join("external-copy-replacement.txt");
    fs::write(&path, "original generation").unwrap();
    let command_source = FileSource::open(&path).unwrap();
    let opened_identity = command_source.identity().unwrap();

    // 模拟编辑器/同步工具的原子替换。稳定打开句柄必须继续指向命令触发时的文件对象，
    // 而 identity 查询仍应看到路径现在代表另一代文件。
    fs::write(&replacement, "replacement content").unwrap();
    fs::remove_file(&path).unwrap();
    fs::rename(&replacement, &path).unwrap();

    assert_eq!(
        command_source
            .read_range(0, "original generation".len() as u64)
            .unwrap(),
        b"original generation"
    );
    assert_ne!(opened_identity.os_file_id, file_id::get_file_id(&path).ok());
}

#[test]
fn line_index_pages_split_on_source_span_and_keep_variable_page_roots_seekable() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("wide-lines.txt");
    let mut file = fs::File::create(&path).unwrap();
    let payload = vec![b'x'; 4 * 1024 * 1024];
    for _ in 0..3 {
        file.write_all(&payload).unwrap();
        file.write_all(b"\n").unwrap();
    }
    file.sync_all().unwrap();
    drop(file);

    let source = FileSource::open(&path).unwrap();
    let index = LineIndex::build_cached(&source, dir.path().join("cache")).unwrap();
    let stats = index.storage_stats();
    assert!(stats.disk_backed);
    assert!(stats.page_count >= 3, "page count was {}", stats.page_count);
    let width = payload.len() as u64 + 1;
    assert_eq!(index.line_range(0), Some(0..width));
    assert_eq!(index.line_range(1), Some(width..width * 2));
    assert_eq!(index.line_range(2), Some(width * 2..width * 3));
    assert_eq!(index.line_for_offset(width * 2 + 123), Some(2));
}

#[test]
fn disk_page_cache_has_a_file_size_independent_hard_limit() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bounded-cache.bin");
    let file = fs::File::create(&path).unwrap();
    file.set_len(320 * 256 * 1024).unwrap();
    let source = FileSource::open(&path).unwrap();
    for page in 0..320u64 {
        let offset = page * 256 * 1024;
        source.read_range(offset, offset + 1).unwrap();
    }
    let stats = source.cache_stats();
    assert_eq!(stats.page_bytes, 256 * 1024);
    assert_eq!(stats.max_pages, 256);
    assert!(stats.resident_pages <= stats.max_pages);
}

#[test]
fn line_index_honors_cancellation_before_scanning_or_loading_sidecars() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cancelled-index.txt");
    fs::write(&path, b"one\ntwo\nthree\n").unwrap();
    let source = FileSource::open(&path).unwrap();
    let cancellation = SearchCancellation::default();
    cancellation.cancel();

    assert!(matches!(
        LineIndex::build_cancellable(&source, &cancellation),
        Err(gmark_paged_document::PagedDocumentError::Cancelled)
    ));
    assert!(matches!(
        LineIndex::build_cached_cancellable(&source, dir.path().join("cache"), &cancellation),
        Err(gmark_paged_document::PagedDocumentError::Cancelled)
    ));
}

#[test]
fn structured_indexers_honor_shared_lifetime_cancellation() {
    let dir = tempfile::tempdir().unwrap();
    let cancellation = SearchCancellation::default();
    cancellation.cancel();

    let csv_path = dir.path().join("cancel.csv");
    fs::write(&csv_path, b"id,name\n1,Ada\n").unwrap();
    let csv = FileSource::open(&csv_path).unwrap();
    assert!(matches!(
        DelimitedIndex::build_cancellable(&csv, DelimitedIndexOptions::default(), &cancellation,),
        Err(gmark_paged_document::PagedDocumentError::Cancelled)
    ));

    let json_path = dir.path().join("cancel.json");
    fs::write(&json_path, br#"[{"id":1}]"#).unwrap();
    let json = FileSource::open(&json_path).unwrap();
    assert!(matches!(
        JsonIndex::build_cancellable(&json, JsonIndexOptions::default(), &cancellation),
        Err(gmark_paged_document::PagedDocumentError::Cancelled)
    ));

    let markdown_path = dir.path().join("cancel.md");
    fs::write(&markdown_path, b"| a | b |\n| --- | --- |\n| 1 | 2 |\n").unwrap();
    let markdown = FileSource::open(&markdown_path).unwrap();
    let lines = LineIndex::build(&markdown).unwrap();
    assert!(matches!(
        MarkdownTableIndex::detect_all_cancellable(&markdown, lines, &cancellation),
        Err(gmark_paged_document::PagedDocumentError::Cancelled)
    ));
}

#[test]
fn editor_adapter_returns_generation_bound_utf8_safe_bounded_viewports() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("adapter.txt");
    let long_line = format!("{}终点", "😀".repeat(40_000));
    fs::write(&path, format!("zero\n{long_line}\ntwo\n")).unwrap();
    let source = FileSource::open(&path).unwrap();
    let index = LineIndex::build(&source).unwrap();
    let document = PieceDocument::open(source, index).unwrap();
    let mut adapter = PagedDocument::new(document);

    let request = ViewportRequest::bounded(1, 1, 1, 65_537, 42);
    let snapshot = adapter.read_viewport(&request).unwrap();
    assert_eq!(snapshot.generation, 42);
    assert_eq!(snapshot.requested_lines, 0..3);
    assert_eq!(snapshot.lines.len(), 3);
    let long = &snapshot.lines[1];
    assert!(long.leading_truncated);
    assert!(long.trailing_truncated);
    assert!(long.text.len() <= 64 * 1024);
    assert!(std::str::from_utf8(long.text.as_bytes()).is_ok());
    let cancellation = SearchCancellation::default();
    cancellation.cancel();
    assert!(matches!(
        adapter.read_viewport_cancellable(&request, &cancellation),
        Err(gmark_paged_document::PagedDocumentError::Cancelled)
    ));

    adapter.replace_text(0..4, "ZERO").unwrap();
    assert_eq!(adapter.backend().generation(), 1);
    assert!(adapter.undo());
    assert_eq!(adapter.backend().generation(), 2);
    assert!(adapter.redo());
    assert_eq!(adapter.backend().generation(), 3);
}

#[test]
fn line_index_sidecar_is_content_free_and_rebuilds_when_stale_or_corrupt() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cached.txt");
    let cache = dir.path().join("cache");
    fs::write(&path, b"one\ntwo\n").unwrap();
    let source = FileSource::open(&path).unwrap();
    let first = LineIndex::build_cached(&source, &cache).unwrap();
    let sidecar = LineIndex::sidecar_path(&source, &cache).unwrap();
    assert_eq!(first.line_count(), 3);
    let cached = fs::read(&sidecar).unwrap();
    assert!(!cached.windows(3).any(|window| window == b"one"));

    fs::write(&sidecar, b"broken cache").unwrap();
    let rebuilt = LineIndex::build_cached(&source, &cache).unwrap();
    assert_eq!(rebuilt.line_count(), 3);

    fs::write(&path, b"one\ntwo\nthree\n").unwrap();
    let changed_source = FileSource::open(&path).unwrap();
    let changed = LineIndex::build_cached(&changed_source, &cache).unwrap();
    assert_eq!(changed.line_count(), 4);
}

#[test]
fn cached_line_index_pages_are_disk_backed_and_resident_cache_is_bounded() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("many-lines.txt");
    let cache = dir.path().join("cache");
    let text = (0..100_000)
        .map(|line| format!("line-{line:06}\n"))
        .collect::<String>();
    fs::write(&path, text).unwrap();
    let source = FileSource::open(&path).unwrap();

    let uncached = LineIndex::build(&source).unwrap();
    let uncached_stats = uncached.storage_stats();
    assert!(uncached_stats.disk_backed);
    assert!(uncached_stats.resident_pages <= uncached_stats.max_resident_pages);

    let index = LineIndex::build_cached(&source, &cache).unwrap();
    let initial = index.storage_stats();
    assert!(initial.disk_backed);
    assert!(initial.page_count > initial.resident_pages);
    assert!(initial.resident_pages <= initial.max_resident_pages);

    for line in (0..100_000u64).step_by(4_097) {
        let range = index.line_range(line).expect("random disk-backed line");
        let expected = format!("line-{line:06}\n");
        assert_eq!(
            source.read_range(range.start, range.end).unwrap(),
            expected.as_bytes()
        );
    }
    let settled = index.storage_stats();
    assert!(settled.disk_backed);
    assert!(settled.resident_pages <= settled.max_resident_pages);
}

#[test]
fn repeated_large_appends_keep_line_index_storage_layers_bounded() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("append.log");
    let initial = (0..70_000)
        .map(|line| format!("base-{line:06}\n"))
        .collect::<String>();
    fs::write(&path, initial).unwrap();
    let mut source = FileSource::open(&path).unwrap();
    let mut index = LineIndex::build(&source).unwrap();

    for round in 0..20u64 {
        let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
        for line in 0..5_000u64 {
            writeln!(file, "append-{round:02}-{line:04}").unwrap();
        }
        file.flush().unwrap();
        source = FileSource::open(&path).unwrap();
        index = index.extend_for_append(&source).unwrap();
        let stats = index.storage_stats();
        assert!(stats.disk_backed);
        assert!(stats.resident_pages <= stats.max_resident_pages);
        assert!(stats.max_resident_pages <= 128);
    }

    assert_eq!(index.line_count(), 170_001);
    let last_content = index
        .line_range(169_999)
        .expect("last appended content line");
    assert_eq!(
        source
            .read_range(last_content.start, last_content.end)
            .unwrap(),
        b"append-19-4999\n"
    );
}
