// @author kongweiguang

use gmark_paged_document::{
    DelimitedEdit, DelimitedFilterOptions, DelimitedIndex, DelimitedIndexOptions, FileSource,
    JsonIndex, JsonIndexOptions, JsonRootKind, LineIndex, MarkdownTableIndex, PagedRecoveryJournal,
    PagedRecoveryReadStatus, PagedRecoverySelection, PieceDocument, SearchCancellation,
    SourceAffinity, SourceAnchor, TextEncoding, apply_delimited_column_edit,
    list_paged_recovery_journals, replay_paged_recovery, serialize_delimited_record,
    validate_json_lines_cancellable, validate_json_lines_from_cancellable,
};
use std::fs;

/// 实时批量输入必须保留组首根；内部逐段替换不能把公开事务拆成多个撤销项。
#[test]
fn paged_live_typing_batch_replaces_selection_and_undoes_once() {
    let directory = tempfile::tempdir().unwrap_or_else(|error| panic!("batch tempdir: {error}"));
    let path = directory.path().join("batch.txt");
    fs::write(&path, b"alpha\nbeta\ngamma\n")
        .unwrap_or_else(|error| panic!("batch fixture: {error}"));
    let source = FileSource::open(&path).unwrap_or_else(|error| panic!("batch source: {error}"));
    let index = LineIndex::build(&source).unwrap_or_else(|error| panic!("batch index: {error}"));
    let mut document = PieceDocument::open(source, index)
        .unwrap_or_else(|error| panic!("batch document: {error}"));
    let group_id = gmark_document_core::TypingGroupId::new();
    for (range, text) in [(3..14, "替"), (6..6, "换"), (9..9, "🙂")] {
        document
            .replace_text_batch_in_typing_group(&[(range, text.into())], group_id)
            .unwrap_or_else(|error| panic!("live typing batch: {error}"));
    }
    assert_eq!(
        document
            .read_range(0..document.len())
            .unwrap_or_else(|error| panic!("read typed text: {error}")),
        "alp替换🙂ma\n".as_bytes()
    );
    assert!(document.undo());
    assert_eq!(
        document
            .read_range(0..document.len())
            .unwrap_or_else(|error| panic!("read undone text: {error}")),
        b"alpha\nbeta\ngamma\n"
    );
    assert!(document.redo());
    assert_eq!(
        document
            .read_range(0..document.len())
            .unwrap_or_else(|error| panic!("read redone text: {error}")),
        "alp替换🙂ma\n".as_bytes()
    );
}

#[test]
fn csv_index_seeks_records_with_quoted_newlines_without_loading_all_rows() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("records.csv");
    fs::write(
        &path,
        b"id,name,note\r\n1,Alice,plain\r\n2,Bob,\"two\r\nlines\"\r\n3,Chen,last\r\n",
    )
    .unwrap();
    let source = FileSource::open(&path).unwrap();
    let index = DelimitedIndex::build(
        &source,
        DelimitedIndexOptions {
            checkpoint_records: 2,
            checkpoint_bytes: u64::MAX,
            ..DelimitedIndexOptions::default()
        },
    )
    .unwrap();

    assert_eq!(index.headers(), ["id", "name", "note"]);
    assert_eq!(index.record_count(), 3);
    assert!(index.checkpoint_count() >= 2);
    let rows = index.read_records(1, 2).unwrap();
    assert_eq!(rows[0].fields, ["2", "Bob", "two\r\nlines"]);
    assert_eq!(rows[1].fields, ["3", "Chen", "last"]);
    let projected = index.read_records_columns(1, 1, 1..3).unwrap();
    assert_eq!(projected[0].fields, ["Bob", "two\r\nlines"]);
    let cancellation = SearchCancellation::default();
    assert_eq!(
        index
            .filter_record_indices(
                "TWO",
                DelimitedFilterOptions {
                    column: Some(2),
                    ..DelimitedFilterOptions::default()
                },
                &cancellation,
            )
            .unwrap(),
        vec![1]
    );
    let cancelled = SearchCancellation::default();
    cancelled.cancel();
    assert!(
        index
            .filter_record_indices("Alice", DelimitedFilterOptions::default(), &cancelled,)
            .is_err()
    );
}

#[test]
fn delimited_index_exposes_header_range_and_synthetic_ragged_columns() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ragged.tsv");
    fs::write(&path, b"name\tscore\nAda\t10\textra\nBob\n").unwrap();
    let source = FileSource::open(&path).unwrap();
    let index = DelimitedIndex::build(
        &source,
        DelimitedIndexOptions {
            delimiter: b'\t',
            ..DelimitedIndexOptions::default()
        },
    )
    .unwrap();

    assert_eq!(index.headers(), ["name", "score", "Column 3"]);
    assert_eq!(index.column_count(), 3);
    assert_eq!(index.delimiter(), b'\t');
    let header = index.read_header().unwrap().unwrap();
    assert_eq!(header.byte_range, 0..11);
    assert_eq!(header.fields, ["name", "score"]);
    assert_eq!(index.read_records(1, 1).unwrap()[0].fields, ["Bob", "", ""]);
}

#[test]
fn delimited_record_serialization_quotes_only_when_required() {
    let record = serialize_delimited_record(
        &["Ada".into(), "comma, quote \" and\nline".into()],
        b',',
        "\r\n",
    );
    assert_eq!(record, "Ada,\"comma, quote \"\" and\nline\"\r\n");
}

#[test]
fn piece_document_stream_replace_is_one_undo_transaction() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("stream.txt");
    fs::write(&path, "before\n").unwrap();
    let source = FileSource::open(&path).unwrap();
    let index = LineIndex::build(&source).unwrap();
    let mut document = PieceDocument::open(source, index).unwrap();
    let replacement = "表格,".repeat(300_000);
    document
        .replace_text_reader(0..document.len(), replacement.as_bytes())
        .unwrap();
    assert_eq!(
        document.read_range(0..document.len()).unwrap(),
        replacement.as_bytes()
    );
    assert!(document.undo());
    assert_eq!(document.read_range(0..document.len()).unwrap(), b"before\n");
}

#[test]
fn delimited_column_transform_streams_and_preserves_record_terminators() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("columns.csv");
    fs::write(&path, b"name,score\r\nAda,10\r\nBob,20").unwrap();
    let source = FileSource::open(&path).unwrap();
    let index = LineIndex::build(&source).unwrap();
    let document = PieceDocument::open(source, index).unwrap();
    let inserted = apply_delimited_column_edit(
        &document,
        DelimitedIndexOptions::default(),
        &DelimitedEdit::InsertColumn {
            before: 1,
            header: "team".into(),
        },
        &SearchCancellation::default(),
    )
    .unwrap();
    assert_eq!(
        inserted.read_range(0..inserted.len()).unwrap(),
        b"name,team,score\r\nAda,,10\r\nBob,,20"
    );
    let removed = apply_delimited_column_edit(
        &inserted,
        DelimitedIndexOptions::default(),
        &DelimitedEdit::DeleteColumn { column: 2 },
        &SearchCancellation::default(),
    )
    .unwrap();
    assert_eq!(
        removed.read_range(0..removed.len()).unwrap(),
        b"name,team\r\nAda,\r\nBob,"
    );
    let cancelled = SearchCancellation::default();
    cancelled.cancel();
    assert!(matches!(
        apply_delimited_column_edit(
            &document,
            DelimitedIndexOptions::default(),
            &DelimitedEdit::DeleteColumn { column: 0 },
            &cancelled,
        ),
        Err(gmark_paged_document::PagedDocumentError::Cancelled)
    ));
}

#[test]
fn csv_sidecar_is_content_free_and_recovers_from_stale_or_corrupt_cache() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cached.csv");
    let cache = dir.path().join("cache");
    fs::write(&path, b"name,score\nAlice,10\nBob,20\n").unwrap();
    let options = DelimitedIndexOptions {
        checkpoint_records: 1,
        ..DelimitedIndexOptions::default()
    };
    let source = FileSource::open(&path).unwrap();
    let index = DelimitedIndex::build_cached(&source, options, &cache).unwrap();
    let sidecar = DelimitedIndex::sidecar_path(&source, options, &cache).unwrap();
    let sidecar_bytes = fs::read(&sidecar).unwrap();
    assert!(memchr::memmem::find(&sidecar_bytes, b"Alice").is_none());
    assert!(memchr::memmem::find(&sidecar_bytes, b"name").is_none());
    assert_eq!(index.read_records(1, 1).unwrap()[0].fields, ["Bob", "20"]);

    fs::write(&sidecar, b"corrupt cache").unwrap();
    let rebuilt = DelimitedIndex::build_cached(&source, options, &cache).unwrap();
    assert_eq!(rebuilt.record_count(), 2);
    assert!(fs::read(&sidecar).unwrap().len() > b"corrupt cache".len());

    fs::write(&path, b"name,score\nElise,10\nBob,20\n").unwrap();
    let changed = FileSource::open(&path).unwrap();
    let refreshed = DelimitedIndex::build_cached(&changed, options, &cache).unwrap();
    assert_eq!(
        refreshed.read_records(0, 1).unwrap()[0].fields,
        ["Elise", "10"]
    );
}

#[test]
fn json_index_resolves_root_items_without_building_a_dom() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("records.json");
    fs::write(
        &path,
        br#"[
          {"id": 1, "text": "comma, brace } and quote \""},
          [2, 3, {"nested": true}],
          "tail"
        ]"#,
    )
    .unwrap();
    let source = FileSource::open(&path).unwrap();
    let index = JsonIndex::build(
        &source,
        JsonIndexOptions {
            checkpoint_items: 1,
            checkpoint_bytes: u64::MAX,
        },
    )
    .unwrap();

    assert_eq!(index.root_kind(), JsonRootKind::Array);
    assert_eq!(index.item_count(), 3);
    assert_eq!(index.checkpoint_count(), 3);
    let second = index.item_range(1).unwrap().unwrap();
    assert_eq!(
        String::from_utf8(source.read_range(second.start, second.end).unwrap()).unwrap(),
        "[2, 3, {\"nested\": true}]"
    );
    let child = index
        .child_index(
            1,
            JsonIndexOptions {
                checkpoint_items: 1,
                checkpoint_bytes: u64::MAX,
            },
        )
        .unwrap()
        .expect("array child index");
    assert_eq!(child.root_kind(), JsonRootKind::Array);
    assert_eq!(child.item_count(), 3);
    let nested = child
        .child_index(2, JsonIndexOptions::default())
        .unwrap()
        .expect("object grandchild index");
    assert_eq!(nested.root_kind(), JsonRootKind::Object);
    assert_eq!(nested.item_count(), 1);
    let (key, value) = nested
        .item_key_value_ranges(0)
        .unwrap()
        .expect("nested object item");
    let key = key.expect("nested object key");
    assert_eq!(
        String::from_utf8(source.read_range(key.start, key.end).unwrap()).unwrap(),
        "\"nested\""
    );
    assert_eq!(source.read_range(value.start, value.end).unwrap(), b"true");
}

#[test]
fn json_index_rejects_invalid_syntax_with_a_source_byte_offset() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("invalid.json");
    let text = b"{\n  \"ok\": 1,\n  \"broken\": ]\n}\n";
    fs::write(&path, text).unwrap();
    let source = FileSource::open(&path).unwrap();

    let error = JsonIndex::build(&source, JsonIndexOptions::default()).unwrap_err();

    let gmark_paged_document::PagedDocumentError::InvalidJson { offset, message } = error else {
        panic!("invalid syntax must report InvalidJson");
    };
    assert_eq!(
        offset,
        text.iter().rposition(|byte| *byte == b']').unwrap() as u64
    );
    assert!(!message.is_empty());
}

#[test]
fn json_lines_validation_is_streaming_cancellable_and_reports_global_byte_offsets() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("records.jsonl");
    let text = b"{\"ok\":1}\n[1,2,3]\n{\"broken\":]}\n";
    fs::write(&path, text).unwrap();
    let source = FileSource::open(&path).unwrap();
    let lines = LineIndex::build(&source).unwrap();
    let cancellation = SearchCancellation::default();

    let error = validate_json_lines_cancellable(&source, &lines, &cancellation).unwrap_err();
    let gmark_paged_document::PagedDocumentError::InvalidJson { offset, message } = error else {
        panic!("invalid JSONL record must report InvalidJson");
    };
    assert_eq!(
        offset,
        text.iter().rposition(|byte| *byte == b']').unwrap() as u64
    );
    assert!(!message.is_empty());

    let cancelled = SearchCancellation::default();
    cancelled.cancel();
    assert!(matches!(
        validate_json_lines_cancellable(&source, &lines, &cancelled),
        Err(gmark_paged_document::PagedDocumentError::Cancelled)
    ));

    let suffix_path = dir.path().join("suffix.jsonl");
    fs::write(&suffix_path, b"]\n{\"ok\":true}\n").unwrap();
    let suffix_source = FileSource::open(&suffix_path).unwrap();
    let suffix_lines = LineIndex::build(&suffix_source).unwrap();
    validate_json_lines_from_cancellable(
        &suffix_source,
        &suffix_lines,
        1,
        &SearchCancellation::default(),
    )
    .unwrap();
}

#[test]
fn json_sidecar_is_content_free_and_recovers_from_stale_or_corrupt_cache() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cached.json");
    let cache = dir.path().join("cache");
    fs::write(&path, br#"{"secret":[1,2],"tail":true}"#).unwrap();
    let options = JsonIndexOptions {
        checkpoint_items: 1,
        checkpoint_bytes: u64::MAX,
    };
    let source = FileSource::open(&path).unwrap();
    let index = JsonIndex::build_cached(&source, options, &cache).unwrap();
    let sidecar = JsonIndex::sidecar_path(&source, options, &cache).unwrap();
    let sidecar_bytes = fs::read(&sidecar).unwrap();
    assert!(memchr::memmem::find(&sidecar_bytes, b"secret").is_none());
    assert!(memchr::memmem::find(&sidecar_bytes, b"tail").is_none());
    assert_eq!(index.item_count(), 2);

    fs::write(&sidecar, b"corrupt cache").unwrap();
    let rebuilt = JsonIndex::build_cached(&source, options, &cache).unwrap();
    assert_eq!(rebuilt.item_count(), 2);
    assert!(fs::read(&sidecar).unwrap().len() > b"corrupt cache".len());

    fs::write(&path, br#"{"public":[1,2],"tail":true}"#).unwrap();
    let changed = FileSource::open(&path).unwrap();
    let refreshed = JsonIndex::build_cached(&changed, options, &cache).unwrap();
    let first = refreshed.item_range(0).unwrap().unwrap();
    assert!(
        String::from_utf8(changed.read_range(first.start, first.end).unwrap())
            .unwrap()
            .starts_with("\"public\"")
    );
}

#[test]
fn json_index_streaming_validator_covers_tokens_escapes_and_deep_nesting() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("syntax.json");
    let cases = [
        (br#"null"#.as_slice(), true),
        (br#"-12.5e+3"#.as_slice(), true),
        (
            r#"{"text":"raw UTF-8 世界","escaped":"\uD834\uDD1E"}"#.as_bytes(),
            true,
        ),
        (br#"[true,false,null,{"nested":[]}]"#.as_slice(), true),
        (br#"{"broken":] }"#.as_slice(), false),
        (br#"[1,]"#.as_slice(), false),
        (br#"{"x":"\q"}"#.as_slice(), false),
        (br#"{"x":"\uD834x"}"#.as_slice(), false),
        (br#"01"#.as_slice(), false),
        (br#"1."#.as_slice(), false),
        (br#"1e+"#.as_slice(), false),
        (br#"true false"#.as_slice(), false),
    ];
    for (text, valid) in cases {
        fs::write(&path, text).unwrap();
        let source = FileSource::open(&path).unwrap();
        assert_eq!(
            JsonIndex::build(&source, JsonIndexOptions::default()).is_ok(),
            valid,
            "JSON validation mismatch for {}",
            String::from_utf8_lossy(text)
        );
    }

    let depth = 4_096usize;
    let deeply_nested = format!("{}0{}", "[".repeat(depth), "]".repeat(depth));
    fs::write(&path, deeply_nested).unwrap();
    let source = FileSource::open(&path).unwrap();
    assert!(JsonIndex::build(&source, JsonIndexOptions::default()).is_ok());
}

#[test]
fn markdown_table_index_reads_only_requested_rows_and_honors_escaped_pipes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("table.md");
    fs::write(
        &path,
        b"# People\n\n| name | note |\n| :--- | ---: |\n| Alice | a\\|b |\n| Bob | `x|y` |\n\nAfter table\n",
    )
    .unwrap();
    let source = FileSource::open(&path).unwrap();
    let lines = LineIndex::build(&source).unwrap();
    let index = MarkdownTableIndex::detect(&source, lines)
        .unwrap()
        .expect("table should be detected");

    assert_eq!(index.headers(), ["name", "note"]);
    assert_eq!(index.header_line(), 2);
    assert_eq!(index.row_count(), 2);
    let rows = index.read_rows(1, 1).unwrap();
    assert_eq!(rows[0].cells, ["Bob", "`x|y`"]);
}

#[test]
fn markdown_table_index_detects_every_table_in_one_document() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tables.md");
    fs::write(
        &path,
        b"# Report\n\n| name | score |\n| --- | ---: |\n| Ada | 10 |\n\nBetween tables.\n\n| city | country | note |\n| :--- | :--- | ---: |\n| Paris | France | a\\|b |\n| Tokyo | Japan | `x|y` |\n\nDone.\n",
    )
    .unwrap();
    let source = FileSource::open(&path).unwrap();
    let lines = LineIndex::build(&source).unwrap();

    let tables = MarkdownTableIndex::detect_all(&source, lines).unwrap();

    assert_eq!(tables.len(), 2);
    assert_eq!(tables[0].headers(), ["name", "score"]);
    assert_eq!(tables[0].header_line(), 2);
    assert_eq!(tables[0].row_count(), 1);
    assert_eq!(tables[1].headers(), ["city", "country", "note"]);
    assert_eq!(tables[1].header_line(), 8);
    assert_eq!(tables[1].row_count(), 2);
    let second_row = tables[1].read_rows(1, 1).unwrap();
    assert_eq!(second_row[0].cells, ["Tokyo", "Japan", "`x|y`"]);
    assert!(second_row[0].byte_range.start > tables[0].read_rows(0, 1).unwrap()[0].byte_range.end);
}

#[test]
fn markdown_table_index_ignores_pipe_tables_inside_fenced_code() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fenced-table.md");
    fs::write(
        &path,
        b"```md\n| fake | table |\n| --- | --- |\n| no | index |\n```\n\n| real | table |\n| --- | --- |\n| yes | index |\n",
    )
    .unwrap();
    let source = FileSource::open(&path).unwrap();
    let lines = LineIndex::build(&source).unwrap();

    let tables = MarkdownTableIndex::detect_all(&source, lines).unwrap();

    assert_eq!(tables.len(), 1);
    assert_eq!(tables[0].headers(), ["real", "table"]);
    assert_eq!(tables[0].header_line(), 6);
}

#[test]
fn chunked_piece_replacement_is_one_undo_transaction() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("chunked.txt");
    fs::write(&path, b"alpha omega").unwrap();
    let source = FileSource::open(&path).unwrap();
    let index = LineIndex::build(&source).unwrap();
    let mut document = PieceDocument::open(source, index).unwrap();

    document
        .replace_text_chunks(0..5, ["large ", "chunked ", "edit"])
        .unwrap();
    assert_eq!(
        document.read_range(0..document.len()).unwrap(),
        b"large chunked edit omega"
    );
    assert!(document.undo());
    assert_eq!(
        document.read_range(0..document.len()).unwrap(),
        b"alpha omega"
    );
    assert!(document.redo());
    assert_eq!(
        document.read_range(0..document.len()).unwrap(),
        b"large chunked edit omega"
    );
}

#[test]
fn paged_recovery_keeps_base_on_disk_and_replays_edits_undo_redo_and_truncated_tail() {
    use std::io::Write as _;

    let dir = tempfile::tempdir().unwrap();
    let source_path = dir.path().join("recovery-source.txt");
    fs::write(&source_path, b"alpha\nbeta\n").unwrap();
    let source = FileSource::open(&source_path).unwrap();
    let mut journal = PagedRecoveryJournal::create(
        dir.path().join("recovery"),
        &source,
        TextEncoding::Utf8 { bom: false },
    )
    .unwrap();
    let journal_path = journal.path().to_path_buf();
    assert!(
        !fs::read(&journal_path)
            .unwrap()
            .windows(b"alpha\nbeta".len())
            .any(|window| window == b"alpha\nbeta"),
        "large recovery base must never embed source text"
    );

    let selection = Some(PagedRecoverySelection {
        anchor: SourceAnchor::new(5, SourceAffinity::Before),
        head: SourceAnchor::new(2, SourceAffinity::After),
    });
    journal
        .record_replace(0..5, "ALPHA", selection, "source")
        .unwrap();
    journal.record_undo(selection, "source").unwrap();
    journal.record_redo(selection, "source").unwrap();
    let recovered = replay_paged_recovery(&journal_path).unwrap();
    assert_eq!(recovered.read_status, PagedRecoveryReadStatus::Complete);
    assert_eq!(recovered.selection, selection);
    assert_eq!(
        recovered
            .document
            .read_range(0..recovered.document.len())
            .unwrap(),
        b"ALPHA\nbeta\n"
    );

    fs::OpenOptions::new()
        .append(true)
        .open(&journal_path)
        .unwrap()
        .write_all(b"GMRJ")
        .unwrap();
    let recovered_tail = replay_paged_recovery(&journal_path).unwrap();
    assert_eq!(
        recovered_tail.read_status,
        PagedRecoveryReadStatus::TruncatedTail
    );
    assert_eq!(
        recovered_tail
            .document
            .read_range(0..recovered_tail.document.len())
            .unwrap(),
        b"ALPHA\nbeta\n"
    );

    journal.checkpoint().unwrap();
    assert!(!journal_path.exists());
}

/// 输入组恢复后仍应一次撤销；没有分组字段的旧日志保持独立事务边界。
#[test]
fn paged_recovery_replays_typing_groups_as_one_undo_and_accepts_legacy_records() {
    use gmark_document_core::{
        DocumentRevision, RecoveryAction, RecoveryBackend, RecoveryRecord, SourceEdit, Transaction,
        TypingGroupId,
    };

    let dir = tempfile::tempdir().unwrap();
    let source_path = dir.path().join("typing-recovery.txt");
    fs::write(&source_path, b"one").unwrap();
    let source = FileSource::open(&source_path).unwrap();
    let mut journal = PagedRecoveryJournal::create(
        dir.path().join("recovery"),
        &source,
        TextEncoding::Utf8 { bom: false },
    )
    .unwrap();
    let view_id = gmark_document_core::DocumentViewId::source();
    let group_id = TypingGroupId::new();
    let mut record = |action| {
        RecoveryBackend::record(
            &mut journal,
            &RecoveryRecord {
                action,
                selection: None,
                view_id: view_id.clone(),
            },
        )
        .unwrap();
    };

    // This ungrouped action writes the same payload shape as journals created before group IDs.
    record(RecoveryAction::Transaction(Transaction::new(
        DocumentRevision(0),
        vec![SourceEdit::new(0..1, "O")],
    )));
    record(RecoveryAction::TypingTransaction {
        transaction: Transaction::new(DocumentRevision(1), vec![SourceEdit::new(3..3, "x")]),
        group_id,
    });
    record(RecoveryAction::TypingTransaction {
        transaction: Transaction::new(DocumentRevision(2), vec![SourceEdit::new(4..4, "y")]),
        group_id,
    });
    let mut recovered = replay_paged_recovery(journal.path()).unwrap();
    assert_eq!(
        recovered
            .document
            .read_range(0..recovered.document.len())
            .unwrap(),
        b"Onexy"
    );
    assert!(recovered.document.undo());
    assert_eq!(
        recovered
            .document
            .read_range(0..recovered.document.len())
            .unwrap(),
        b"One"
    );
    assert!(recovered.document.redo());
    assert_eq!(
        recovered
            .document
            .read_range(0..recovered.document.len())
            .unwrap(),
        b"Onexy"
    );
    assert!(recovered.document.undo());
    assert_eq!(
        recovered
            .document
            .read_range(0..recovered.document.len())
            .unwrap(),
        b"One"
    );
    assert!(recovered.document.undo());
    assert_eq!(
        recovered
            .document
            .read_range(0..recovered.document.len())
            .unwrap(),
        b"one"
    );
}

#[test]
fn paged_recovery_refuses_a_changed_base_file() {
    let dir = tempfile::tempdir().unwrap();
    let source_path = dir.path().join("changed-base.txt");
    fs::write(&source_path, b"original").unwrap();
    let source = FileSource::open(&source_path).unwrap();
    let journal = PagedRecoveryJournal::create(
        dir.path().join("recovery"),
        &source,
        TextEncoding::Utf8 { bom: false },
    )
    .unwrap();
    fs::write(&source_path, b"changed!").unwrap();

    assert!(replay_paged_recovery(journal.path()).is_err());
}

#[test]
fn paged_recovery_listing_skips_a_suppressed_stale_journal_and_keeps_the_active_session()
-> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let source_path = dir.path().join("suppressed-recovery-source.txt");
    fs::write(&source_path, b"alpha")?;
    let source = FileSource::open(&source_path)?;
    let recovery_dir = dir.path().join("recovery");
    let mut stale =
        PagedRecoveryJournal::create(&recovery_dir, &source, TextEncoding::Utf8 { bom: false })?;
    stale.record_replace(0..5, "stale", None, "source")?;
    let stale_marker = stale.path().with_extension("large-journal.suppressed");
    fs::write(&stale_marker, b"gmark-recovery-suppressed-v1\n")?;
    let blocked = recovery_dir.join("blocked.large-journal");
    fs::create_dir(&blocked)?;
    let blocked_marker = blocked.with_extension("large-journal.suppressed");
    fs::write(&blocked_marker, b"gmark-recovery-suppressed-v1\n")?;

    let mut active =
        PagedRecoveryJournal::create(&recovery_dir, &source, TextEncoding::Utf8 { bom: false })?;
    active.record_replace(0..5, "active", None, "source")?;
    let active_path = active.path().to_path_buf();

    assert_eq!(
        list_paged_recovery_journals(&recovery_dir)?,
        vec![active_path]
    );
    assert!(!stale.path().exists());
    assert!(!stale_marker.exists());
    assert!(blocked.exists());
    assert!(blocked_marker.exists());
    Ok(())
}

#[test]
fn paged_recovery_rejects_unbounded_chunk_count_without_allocating() {
    use std::io::Write as _;

    use gmark_recovery_codec::{RecordKind, encode_record_payload};

    let dir = tempfile::tempdir().unwrap();
    let source_path = dir.path().join("bounded-recovery-source.txt");
    fs::write(&source_path, b"alpha").unwrap();
    let source = FileSource::open(&source_path).unwrap();
    let journal = PagedRecoveryJournal::create(
        dir.path().join("recovery"),
        &source,
        TextEncoding::Utf8 { bom: false },
    )
    .unwrap();
    let payload = serde_json::to_vec(&serde_json::json!({
        "command": "replace_chunk",
        "transaction": 1,
        "start": 0,
        "end": 0,
        "chunk_index": 0,
        "chunk_count": u32::MAX,
        "text": "x",
        "selection": null,
        "view_mode": "source"
    }))
    .unwrap();
    let frame = encode_record_payload(RecordKind::Edit, &payload).unwrap();
    fs::OpenOptions::new()
        .append(true)
        .open(journal.path())
        .unwrap()
        .write_all(&frame)
        .unwrap();

    assert!(replay_paged_recovery(journal.path()).is_err());
}
