// @author kongweiguang

#[cfg(feature = "code-highlight-extra")]
use gmark_source_tools::build_source_syntax_contexts;
use gmark_source_tools::{
    HighlightEngine, SourceLanguage, TokenClass, highlight_fenced_code, highlight_source,
    highlight_source_cancellable,
};

/// 取消不能伪装成成功的空高亮；正常快照与原有同步接口语义一致。
#[test]
fn cancellable_highlight_preserves_result_and_rejects_cancelled_work() {
    let source = "# 中文 heading\n\n`code`\n";
    let flag = std::sync::atomic::AtomicUsize::new(0);
    let normal = highlight_source(SourceLanguage::Markdown, source);
    assert_eq!(
        highlight_source_cancellable(SourceLanguage::Markdown, source, &flag),
        Some(normal)
    );
    flag.store(1, std::sync::atomic::Ordering::Relaxed);
    assert_eq!(
        highlight_source_cancellable(SourceLanguage::Markdown, source, &flag),
        None
    );
    assert_eq!(
        highlight_source_cancellable(SourceLanguage::Json, "{}", &flag),
        None
    );
}

#[test]
fn unknown_fences_fall_back_to_plain_text_and_json_keeps_lexical_spans() {
    let plain = highlight_fenced_code(Some("not-a-language"), "anything");
    assert_eq!(plain.language, SourceLanguage::PlainText);
    assert_eq!(plain.engine, HighlightEngine::PlainTextFallback);
    assert!(plain.spans.is_empty());

    let source = "{\"name\":\"界\",\"ok\":true}";
    let json = highlight_source(SourceLanguage::Json, source);
    assert_eq!(json.engine, HighlightEngine::JsonFallback);
    assert!(json.spans.iter().any(|span| {
        span.class == TokenClass::Property && span.range.slice(source).ok() == Some("\"name\"")
    }));
    assert!(
        json.spans
            .iter()
            .any(|span| span.class == TokenClass::Constant)
    );
}

/// 纯文本和 Mermaid 围栏由各自的显示路径处理，按需加载语法时也不能误走代码着色。
#[test]
fn registered_non_code_fences_keep_plain_text_spans() {
    for (info, language) in [
        ("text", SourceLanguage::PlainText),
        ("mmd", SourceLanguage::Mermaid),
    ] {
        let result = highlight_fenced_code(Some(info), "fn example() { 42 }");
        assert_eq!(result.language, language);
        assert_eq!(result.engine, HighlightEngine::PlainTextFallback);
        assert!(result.spans.is_empty());
    }
}

#[cfg(feature = "code-highlight-official")]
#[test]
fn official_grammar_bundle_produces_semantic_rust_spans() {
    let highlighted = highlight_fenced_code(Some("rs line-numbers"), "fn answer() -> u8 { 42 }");
    assert_eq!(highlighted.language, SourceLanguage::Rust);
    assert_eq!(highlighted.engine, HighlightEngine::TreeSitter);
    assert!(!highlighted.spans.is_empty());
}

#[cfg(feature = "code-highlight-official")]
#[test]
fn markdown_source_highlights_structure_inline_syntax_and_fenced_code() {
    let source = "# 标题\n\n- [x] **重要** 与 *强调* [链接](https://example.com) `let x = 1`\n\n> 引用\n\n---\n\n```rust\nfn answer() -> u8 { 42 }\n```\n";
    let highlighted = highlight_source(SourceLanguage::Markdown, source);

    assert_eq!(highlighted.engine, HighlightEngine::TreeSitter);
    assert!(!highlighted.spans.is_empty());

    for window in highlighted.spans.windows(2) {
        assert!(
            window[0].range.start() <= window[1].range.start(),
            "highlight spans must be ordered: {:?} then {:?}",
            window[0].range,
            window[1].range
        );
    }
    for span in &highlighted.spans {
        assert!(!span.range.is_empty());
        assert!(span.range.validate_for(source).is_ok());
        assert!(span.range.slice(source).is_ok());
    }

    let has = |class, text: &str| {
        highlighted
            .spans
            .iter()
            .any(|span| span.class == class && span.range.slice(source).ok() == Some(text))
    };
    let has_fragment = |class, text: &str| {
        highlighted.spans.iter().any(|span| {
            span.class == class
                && span
                    .range
                    .slice(source)
                    .is_ok_and(|highlighted| highlighted.contains(text))
        })
    };
    assert!(has(TokenClass::Keyword, "标题"));
    assert!(has_fragment(TokenClass::Keyword, "#"));
    assert!(has_fragment(TokenClass::Keyword, "-"));
    assert!(has_fragment(TokenClass::Keyword, "[x]"));
    assert!(has_fragment(TokenClass::Keyword, ">"));
    assert!(has_fragment(TokenClass::Keyword, "---"));
    assert!(has(TokenClass::Keyword, "```"));
    assert!(has(TokenClass::Property, "rust"));
    assert!(has(TokenClass::Keyword, "重要"));
    assert!(has(TokenClass::Keyword, "强调"));
    assert!(has(TokenClass::Property, "链接"));
    assert!(has(TokenClass::String, "https://example.com"));
    assert!(has(TokenClass::String, "let x = 1"));
    assert!(
        has(TokenClass::Function, "answer"),
        "Markdown fenced code spans: {:?}",
        highlighted
            .spans
            .iter()
            .map(|span| (span.class, span.range.slice(source).ok()))
            .collect::<Vec<_>>()
    );
}

#[cfg(feature = "code-highlight-config")]
#[test]
fn config_grammar_bundle_produces_yaml_spans() {
    let highlighted = highlight_source(SourceLanguage::Yaml, "name: gmark\nenabled: true\n");
    assert_eq!(highlighted.engine, HighlightEngine::TreeSitter);
    assert!(!highlighted.spans.is_empty());
}

#[cfg(feature = "code-highlight-extra")]
#[test]
fn extra_grammar_bundle_produces_semantic_spans() {
    let samples = [
        (
            SourceLanguage::Sql,
            "SELECT value FROM items WHERE value > 0;",
        ),
        (SourceLanguage::Lua, "local value = 42\nprint(value)"),
        (SourceLanguage::Swift, "let greeting: String = \"hello\""),
        (SourceLanguage::PowerShell, "$items = Get-ChildItem"),
        (
            SourceLanguage::Containerfile,
            "FROM rust:latest\nRUN cargo build",
        ),
    ];
    for (language, source) in samples {
        let highlighted = highlight_source(language, source);
        assert_eq!(highlighted.engine, HighlightEngine::TreeSitter);
        assert!(
            !highlighted.spans.is_empty(),
            "expected spans for {language:?}"
        );
    }
}

#[cfg(feature = "code-highlight-extra")]
#[test]
fn source_rows_keep_multiline_sql_context() {
    let lines = [
        "SELECT vehicle_id, sequence, action, road_id, target_road_id,",
        "       start_time, end_time, duration, speed",
        "FROM snowplow_table",
        "WHERE vehicle_id = :vehicle_id",
        "  AND action = 'clean'",
        "ORDER BY start_time",
        "LIMIT 1;",
    ];
    let contexts = build_source_syntax_contexts(
        SourceLanguage::Sql,
        lines.iter().enumerate().map(|(line, text)| (line, *text)),
    );
    assert_eq!(contexts.len(), lines.len());
    assert!(contexts.iter().all(|(line, context)| {
        let highlighted = context.highlight(lines[*line]);
        highlighted.engine == HighlightEngine::TreeSitter && !highlighted.spans.is_empty()
    }));
}

#[cfg(all(
    feature = "code-highlight-core",
    not(feature = "code-highlight-official")
))]
#[test]
fn core_without_official_grammars_remains_a_safe_fallback() {
    let highlighted = highlight_source(SourceLanguage::Rust, "fn answer() {}");
    assert_eq!(highlighted.engine, HighlightEngine::PlainTextFallback);
    assert!(highlighted.spans.is_empty());
}
