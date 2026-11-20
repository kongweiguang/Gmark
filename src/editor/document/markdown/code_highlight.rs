// @author kongweiguang

//! GPUI-facing semantic-highlight adapter.

use std::collections::BTreeMap;
use std::ops::Range;
use std::path::Path;
use std::sync::atomic::AtomicUsize;

use gmark_source_tools::{
    HighlightResult, SourceLanguage, build_source_syntax_contexts as build_domain_contexts,
    detect_language, highlight_source, highlight_source_cancellable, resolve_fence_language,
};
use gpui::Hsla;

use crate::theme::ThemeColors;

pub(crate) use gmark_source_tools::{
    FENCE_LANGUAGE_MENU_ITEMS as CODE_LANGUAGE_MENU_ITEMS, SourceLanguage as CodeLanguageKey,
    SourceSyntaxContext, TokenClass as CodeHighlightClass,
};

/// Highlighted byte range inside a code block, projected to GPUI's native offsets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CodeHighlightSpan {
    pub(crate) range: Range<usize>,
    pub(crate) class: CodeHighlightClass,
}

/// Highlight result cached on a code block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CodeHighlightResult {
    pub(crate) language: CodeLanguageKey,
    pub(crate) spans: Vec<CodeHighlightSpan>,
}

pub(crate) fn resolve_code_language_key(language: Option<&str>) -> Option<CodeLanguageKey> {
    resolve_fence_language(language)
}

/// 根据独立源码文件的常见扩展名复用领域语言目录。
pub(crate) fn code_language_for_path(path: &Path) -> Option<&'static str> {
    let language = detect_language(path);
    (language != SourceLanguage::PlainText).then_some(language.canonical_name())
}

pub(crate) fn highlight_code_block(
    language: Option<&str>,
    source: &str,
) -> Option<CodeHighlightResult> {
    let language = resolve_code_language_key(language)?;
    Some(project_highlight_result(highlight_source(language, source)))
}

/// 在源文件视图卸载或内容更新时终止后台语法分析，避免废弃任务继续占用 CPU。
pub(crate) fn highlight_code_block_cancellable(
    language: Option<&str>,
    source: &str,
    cancellation: &AtomicUsize,
) -> Option<CodeHighlightResult> {
    let language = resolve_code_language_key(language)?;
    highlight_source_cancellable(language, source, cancellation).map(project_highlight_result)
}

pub(crate) fn build_source_syntax_contexts<'a>(
    language: Option<&str>,
    rows: impl IntoIterator<Item = (usize, &'a str)>,
) -> BTreeMap<usize, SourceSyntaxContext> {
    resolve_code_language_key(language)
        .map(|language| build_domain_contexts(language, rows))
        .unwrap_or_default()
}

pub(crate) fn project_highlight_result(highlighted: HighlightResult) -> CodeHighlightResult {
    let spans = highlighted
        .spans
        .into_iter()
        .filter_map(|span| {
            let start = usize::try_from(span.range.start()).ok()?;
            let end = usize::try_from(span.range.end()).ok()?;
            (start < end).then_some(CodeHighlightSpan {
                range: start..end,
                class: span.class,
            })
        })
        .collect();
    CodeHighlightResult {
        language: highlighted.language,
        spans,
    }
}

pub(crate) fn code_highlight_color(colors: &ThemeColors, class: CodeHighlightClass) -> Hsla {
    match class {
        CodeHighlightClass::Comment => colors.code_syntax_comment,
        CodeHighlightClass::Keyword => colors.code_syntax_keyword,
        CodeHighlightClass::String => colors.code_syntax_string,
        CodeHighlightClass::Number => colors.code_syntax_number,
        CodeHighlightClass::Type => colors.code_syntax_type,
        CodeHighlightClass::Function => colors.code_syntax_function,
        CodeHighlightClass::Constant => colors.code_syntax_constant,
        CodeHighlightClass::Variable => colors.code_syntax_variable,
        CodeHighlightClass::Property => colors.code_syntax_property,
        CodeHighlightClass::Operator => colors.code_syntax_operator,
        CodeHighlightClass::Punctuation => colors.code_syntax_punctuation,
    }
}

#[cfg(test)]
#[path = "../../../../tests/unit/components/markdown/code_highlight.rs"]
mod tests;
