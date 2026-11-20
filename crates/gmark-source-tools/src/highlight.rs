// @author kongweiguang

use crate::{ByteRange, SourceLanguage};
use std::sync::atomic::{AtomicUsize, Ordering};

#[cfg(feature = "code-highlight-core")]
use std::collections::HashMap;
#[cfg(feature = "code-highlight-core")]
use std::sync::LazyLock;
#[cfg(feature = "code-highlight-core")]
use tree_sitter_highlight::{Highlight, HighlightConfiguration, HighlightEvent, Highlighter};

/// 适配器可映射到主题色的稳定语义 token 类别。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum TokenClass {
    Comment,
    Keyword,
    String,
    Number,
    Type,
    Function,
    Constant,
    Variable,
    Property,
    Operator,
    Punctuation,
}

/// 一个按源码 UTF-8 byte range 定位的语义高亮 token。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HighlightSpan {
    pub range: ByteRange,
    pub class: TokenClass,
}

/// 本次高亮所采用的无副作用引擎路径。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HighlightEngine {
    /// JSON 使用稳定词法扫描，以支持不完整的可见行片段。
    JsonFallback,
    /// 已成功运行对应 grammar 的 Tree-sitter highlights query。
    TreeSitter,
    /// 未启用 grammar、解析失败或语言未知时的安全空 token 回退。
    PlainTextFallback,
}

/// 语义高亮结果；UI 主题、TextRun 等转换由适配器负责。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HighlightResult {
    pub language: SourceLanguage,
    pub spans: Vec<HighlightSpan>,
    pub engine: HighlightEngine,
}

/// 高亮代码围栏；未知或缺失语言明确退回为 plain text。
pub fn highlight_fenced_code(language_info: Option<&str>, source: &str) -> HighlightResult {
    let language =
        SourceLanguage::from_fence_info(language_info).unwrap_or(SourceLanguage::PlainText);
    highlight_source(language, source)
}

/// 高亮已识别的源码。所有输入都返回结果，不能使用 grammar 时不会产生错误或副作用。
pub fn highlight_source(language: SourceLanguage, source: &str) -> HighlightResult {
    highlight_source_inner(language, source, None).unwrap_or(HighlightResult {
        language,
        spans: Vec::new(),
        engine: HighlightEngine::PlainTextFallback,
    })
}

/// 后台源码分析接受调用方取消信号，取消时不发布部分 token 或误报纯文本结果。
pub fn highlight_source_cancellable(
    language: SourceLanguage,
    source: &str,
    cancellation: &AtomicUsize,
) -> Option<HighlightResult> {
    highlight_source_inner(language, source, Some(cancellation))
}

/// 常规与可取消入口共用解析规则，避免后台处理改变着色语义。
fn highlight_source_inner(
    language: SourceLanguage,
    source: &str,
    cancellation: Option<&AtomicUsize>,
) -> Option<HighlightResult> {
    if cancellation.is_some_and(|flag| flag.load(Ordering::Relaxed) != 0) {
        return None;
    }
    if matches!(language, SourceLanguage::Json | SourceLanguage::JsonLines) {
        let spans = json_highlight_fallback(source);
        if cancellation.is_some_and(|flag| flag.load(Ordering::Relaxed) != 0) {
            return None;
        }
        return Some(HighlightResult {
            language,
            spans,
            engine: HighlightEngine::JsonFallback,
        });
    }

    #[cfg(feature = "code-highlight-core")]
    if let Some(config) = CODE_HIGHLIGHT_REGISTRY.config_for(language)
        && let Some(spans) = tree_sitter_highlight(config, source, cancellation)
    {
        return Some(HighlightResult {
            language,
            spans,
            engine: HighlightEngine::TreeSitter,
        });
    }

    if cancellation.is_some_and(|flag| flag.load(Ordering::Relaxed) != 0) {
        return None;
    }
    Some(HighlightResult {
        language,
        spans: Vec::new(),
        engine: HighlightEngine::PlainTextFallback,
    })
}

#[cfg(feature = "code-highlight-core")]
const HIGHLIGHT_NAMES: &[&str] = &[
    "attribute",
    "boolean",
    "character",
    "comment",
    "conditional",
    "constant",
    "constant.builtin",
    "constructor",
    "embedded",
    "field",
    "float",
    "function",
    "function.builtin",
    "keyword",
    "label",
    "method",
    "module",
    "name",
    "number",
    "operator",
    "parameter",
    "preproc",
    "property",
    "property.builtin",
    "punctuation",
    "punctuation.bracket",
    "punctuation.delimiter",
    "punctuation.special",
    "repeat",
    "storageclass",
    "string",
    "string.special",
    "tag",
    "type",
    "type.builtin",
    "variable",
    "variable.builtin",
    "variable.member",
    "variable.parameter",
    "text.title",
    "text.literal",
    "text.uri",
    "text.reference",
    "text.emphasis",
    "text.strong",
    "string.escape",
    "markup.marker",
];

#[cfg(feature = "code-highlight-core")]
type LazyHighlightConfig = LazyLock<Option<HighlightConfiguration>>;

#[cfg(feature = "code-highlight-core")]
struct HighlightRegistry {
    configs: HashMap<SourceLanguage, LazyHighlightConfig>,
    inline_config: LazyHighlightConfig,
}

#[cfg(feature = "code-highlight-core")]
static CODE_HIGHLIGHT_REGISTRY: LazyLock<HighlightRegistry> = LazyLock::new(HighlightRegistry::new);

#[cfg(feature = "code-highlight-core")]
impl HighlightRegistry {
    /// 首次遇到围栏只登记构造器，避免界面线程同时为所有语言编译查询。
    fn new() -> Self {
        #[cfg(feature = "code-highlight-official")]
        let inline_config: LazyHighlightConfig =
            LazyLock::new(crate::highlight_configs::build_markdown_inline_config);
        #[cfg(not(feature = "code-highlight-official"))]
        let inline_config: LazyHighlightConfig = LazyLock::new(|| None);
        let configs = {
            #[cfg(any(
                feature = "code-highlight-official",
                feature = "code-highlight-config",
                feature = "code-highlight-extra"
            ))]
            {
                let mut configs = HashMap::new();
                #[cfg(feature = "code-highlight-official")]
                register_config(
                    &mut configs,
                    SourceLanguage::Rust,
                    crate::highlight_configs::build_rust_config,
                );
                #[cfg(feature = "code-highlight-official")]
                register_config(
                    &mut configs,
                    SourceLanguage::JavaScript,
                    crate::highlight_configs::build_javascript_config,
                );
                #[cfg(feature = "code-highlight-official")]
                register_config(
                    &mut configs,
                    SourceLanguage::JavaScriptJsx,
                    crate::highlight_configs::build_jsx_config,
                );
                #[cfg(feature = "code-highlight-official")]
                register_config(
                    &mut configs,
                    SourceLanguage::TypeScript,
                    crate::highlight_configs::build_typescript_config,
                );
                #[cfg(feature = "code-highlight-official")]
                register_config(
                    &mut configs,
                    SourceLanguage::TypeScriptTsx,
                    crate::highlight_configs::build_tsx_config,
                );
                #[cfg(feature = "code-highlight-official")]
                register_config(
                    &mut configs,
                    SourceLanguage::Json,
                    crate::highlight_configs::build_json_config,
                );
                #[cfg(feature = "code-highlight-official")]
                register_config(
                    &mut configs,
                    SourceLanguage::Markdown,
                    crate::highlight_configs::build_markdown_config,
                );
                #[cfg(feature = "code-highlight-official")]
                register_config(
                    &mut configs,
                    SourceLanguage::Bash,
                    crate::highlight_configs::build_bash_config,
                );
                #[cfg(feature = "code-highlight-official")]
                register_config(
                    &mut configs,
                    SourceLanguage::C,
                    crate::highlight_configs::build_c_config,
                );
                #[cfg(feature = "code-highlight-official")]
                register_config(
                    &mut configs,
                    SourceLanguage::Cpp,
                    crate::highlight_configs::build_cpp_config,
                );
                #[cfg(feature = "code-highlight-official")]
                register_config(
                    &mut configs,
                    SourceLanguage::CSharp,
                    crate::highlight_configs::build_csharp_config,
                );
                #[cfg(feature = "code-highlight-official")]
                register_config(
                    &mut configs,
                    SourceLanguage::Css,
                    crate::highlight_configs::build_css_config,
                );
                #[cfg(feature = "code-highlight-official")]
                register_config(
                    &mut configs,
                    SourceLanguage::Go,
                    crate::highlight_configs::build_go_config,
                );
                #[cfg(feature = "code-highlight-official")]
                register_config(
                    &mut configs,
                    SourceLanguage::Html,
                    crate::highlight_configs::build_html_config,
                );
                #[cfg(feature = "code-highlight-official")]
                register_config(
                    &mut configs,
                    SourceLanguage::Java,
                    crate::highlight_configs::build_java_config,
                );
                #[cfg(feature = "code-highlight-official")]
                register_config(
                    &mut configs,
                    SourceLanguage::Php,
                    crate::highlight_configs::build_php_config,
                );
                #[cfg(feature = "code-highlight-official")]
                register_config(
                    &mut configs,
                    SourceLanguage::Python,
                    crate::highlight_configs::build_python_config,
                );
                #[cfg(feature = "code-highlight-official")]
                register_config(
                    &mut configs,
                    SourceLanguage::Ruby,
                    crate::highlight_configs::build_ruby_config,
                );
                #[cfg(feature = "code-highlight-config")]
                register_config(
                    &mut configs,
                    SourceLanguage::Yaml,
                    crate::highlight_configs::build_yaml_config,
                );
                #[cfg(feature = "code-highlight-config")]
                register_config(
                    &mut configs,
                    SourceLanguage::Toml,
                    crate::highlight_configs::build_toml_config,
                );
                #[cfg(feature = "code-highlight-extra")]
                register_config(
                    &mut configs,
                    SourceLanguage::Sql,
                    crate::highlight_configs::build_sql_config,
                );
                #[cfg(feature = "code-highlight-extra")]
                register_config(
                    &mut configs,
                    SourceLanguage::Lua,
                    crate::highlight_configs::build_lua_config,
                );
                #[cfg(feature = "code-highlight-extra")]
                register_config(
                    &mut configs,
                    SourceLanguage::Swift,
                    crate::highlight_configs::build_swift_config,
                );
                #[cfg(feature = "code-highlight-extra")]
                register_config(
                    &mut configs,
                    SourceLanguage::PowerShell,
                    crate::highlight_configs::build_powershell_config,
                );
                #[cfg(feature = "code-highlight-extra")]
                register_config(
                    &mut configs,
                    SourceLanguage::Containerfile,
                    crate::highlight_configs::build_containerfile_config,
                );
                configs
            }

            #[cfg(not(any(
                feature = "code-highlight-official",
                feature = "code-highlight-config",
                feature = "code-highlight-extra"
            )))]
            {
                HashMap::new()
            }
        };
        Self {
            configs,
            inline_config,
        }
    }

    /// 同一语言在并发调用下只编译一次，失败也缓存，纯文本不触发任何语法构建。
    fn config_for(&self, language: SourceLanguage) -> Option<&HighlightConfiguration> {
        LazyLock::force(self.configs.get(&language)?).as_ref()
    }

    fn config_for_injection(&self, language_name: &str) -> Option<&HighlightConfiguration> {
        if language_name.eq_ignore_ascii_case("markdown_inline") {
            return self.inline_config.as_ref();
        }
        SourceLanguage::from_alias(language_name).and_then(|language| self.config_for(language))
    }
}

#[cfg(any(
    feature = "code-highlight-official",
    feature = "code-highlight-config",
    feature = "code-highlight-extra"
))]
/// 登记轻量函数指针，保留嵌入语言与独立调用共用同一份查询缓存的契约。
fn register_config(
    configs: &mut HashMap<SourceLanguage, LazyHighlightConfig>,
    language: SourceLanguage,
    build: fn() -> Option<HighlightConfiguration>,
) {
    configs.insert(language, LazyLock::new(build));
}

#[cfg(any(
    feature = "code-highlight-official",
    feature = "code-highlight-config",
    feature = "code-highlight-extra"
))]
pub(crate) fn configure_highlights(
    language: tree_sitter::Language,
    name: &'static str,
    highlights_query: &str,
    injections_query: &str,
    locals_query: &str,
) -> Option<HighlightConfiguration> {
    let mut config = HighlightConfiguration::new(
        language,
        name,
        highlights_query,
        injections_query,
        locals_query,
    )
    .ok()?;
    config.configure(HIGHLIGHT_NAMES);
    Some(config)
}

#[cfg(feature = "code-highlight-core")]
fn tree_sitter_highlight(
    config: &HighlightConfiguration,
    source: &str,
    cancellation: Option<&AtomicUsize>,
) -> Option<Vec<HighlightSpan>> {
    let mut highlighter = Highlighter::new();
    let events = highlighter
        .highlight(config, source.as_bytes(), cancellation, |language_name| {
            CODE_HIGHLIGHT_REGISTRY.config_for_injection(language_name)
        })
        .ok()?;
    let mut spans = Vec::new();
    let mut active: Vec<Option<TokenClass>> = Vec::new();
    for event in events {
        match event.ok()? {
            HighlightEvent::Source { start, end } => {
                if let Some(Some(class)) = active.last().copied() {
                    push_highlight_span(&mut spans, source, start, end, class);
                }
            }
            HighlightEvent::HighlightStart(highlight) => {
                active.push(class_for_highlight(highlight));
            }
            HighlightEvent::HighlightEnd => {
                active.pop();
            }
        }
    }
    Some(spans)
}

/// JSON 片段的轻量词法扫描：只识别稳定 token，不要求输入是完整文档。
fn json_highlight_fallback(source: &str) -> Vec<HighlightSpan> {
    let bytes = source.as_bytes();
    let mut spans = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'/' if bytes.get(index + 1) == Some(&b'/') => {
                let start = index;
                index += 2;
                while bytes.get(index).is_some_and(|byte| *byte != b'\n') {
                    index += 1;
                }
                push_highlight_span(&mut spans, source, start, index, TokenClass::Comment);
            }
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                let start = index;
                index += 2;
                while index + 1 < bytes.len() && !(bytes[index] == b'*' && bytes[index + 1] == b'/')
                {
                    index += 1;
                }
                index = index.saturating_add(2).min(bytes.len());
                push_highlight_span(&mut spans, source, start, index, TokenClass::Comment);
            }
            b'"' => {
                let start = index;
                index += 1;
                let mut escaped = false;
                while let Some(&byte) = bytes.get(index) {
                    index += 1;
                    if escaped {
                        escaped = false;
                    } else if byte == b'\\' {
                        escaped = true;
                    } else if byte == b'"' {
                        break;
                    }
                }
                let mut next = index;
                while bytes
                    .get(next)
                    .is_some_and(|byte| byte.is_ascii_whitespace())
                {
                    next += 1;
                }
                let class = if bytes.get(next) == Some(&b':') {
                    TokenClass::Property
                } else {
                    TokenClass::String
                };
                push_highlight_span(&mut spans, source, start, index, class);
            }
            b'-' | b'0'..=b'9' => {
                let start = index;
                index += 1;
                while bytes.get(index).is_some_and(|byte| {
                    matches!(*byte, b'0'..=b'9' | b'.' | b'e' | b'E' | b'+' | b'-')
                }) {
                    index += 1;
                }
                push_highlight_span(&mut spans, source, start, index, TokenClass::Number);
            }
            b't' if source
                .get(index..)
                .is_some_and(|tail| tail.starts_with("true")) =>
            {
                push_highlight_span(&mut spans, source, index, index + 4, TokenClass::Constant);
                index += 4;
            }
            b'f' if source
                .get(index..)
                .is_some_and(|tail| tail.starts_with("false")) =>
            {
                push_highlight_span(&mut spans, source, index, index + 5, TokenClass::Constant);
                index += 5;
            }
            b'n' if source
                .get(index..)
                .is_some_and(|tail| tail.starts_with("null")) =>
            {
                push_highlight_span(&mut spans, source, index, index + 4, TokenClass::Constant);
                index += 4;
            }
            b'{' | b'}' | b'[' | b']' | b':' | b',' => {
                push_highlight_span(
                    &mut spans,
                    source,
                    index,
                    index + 1,
                    TokenClass::Punctuation,
                );
                index += 1;
            }
            _ => index += 1,
        }
    }
    spans
}

fn push_highlight_span(
    spans: &mut Vec<HighlightSpan>,
    source: &str,
    start: usize,
    end: usize,
    class: TokenClass,
) {
    if start >= end {
        return;
    }
    let Ok(range) = ByteRange::from_source_offsets(source, start, end) else {
        return;
    };
    if let Some(last) = spans.last_mut()
        && last.class == class
        && last.range.end() == range.start()
    {
        last.range = ByteRange::new(last.range.start(), range.end()).unwrap_or(last.range);
        return;
    }
    spans.push(HighlightSpan { range, class });
}

#[cfg(feature = "code-highlight-core")]
fn class_for_highlight(highlight: Highlight) -> Option<TokenClass> {
    let name = HIGHLIGHT_NAMES.get(highlight.0)?;
    Some(match *name {
        "comment" => TokenClass::Comment,
        "keyword" | "tag" => TokenClass::Keyword,
        "string" | "string.special" | "string.escape" | "embedded" => TokenClass::String,
        "number" => TokenClass::Number,
        "type" | "type.builtin" | "module" => TokenClass::Type,
        "function" | "function.builtin" | "constructor" => TokenClass::Function,
        "constant" | "constant.builtin" => TokenClass::Constant,
        "variable" | "variable.builtin" | "variable.parameter" => TokenClass::Variable,
        "property" | "property.builtin" | "attribute" => TokenClass::Property,
        "text.title" | "text.emphasis" | "text.strong" | "markup.marker" => TokenClass::Keyword,
        "text.literal" | "text.uri" => TokenClass::String,
        "text.reference" => TokenClass::Property,
        "operator" => TokenClass::Operator,
        "punctuation" | "punctuation.bracket" | "punctuation.delimiter" | "punctuation.special" => {
            TokenClass::Punctuation
        }
        _ => return None,
    })
}
