// @author kongweiguang

//! Full HTML document generation from Markdown and a neutral export theme.

use std::collections::HashMap;
use std::path::Path;

use base64::{Engine as _, engine::general_purpose};
use gmark_markdown::escape_html;
use pulldown_cmark::{Event, Options, Parser, html};

use crate::ExportTheme;
use crate::markup::{
    is_closing_fence, is_root_comment_start, opening_fence, rewrite_local_image_event,
    rewrite_scaled_standalone_images, rewrite_unsafe_html_blocks,
};
use crate::math::{rewrite_display_math_blocks, rewrite_inline_math, rewrite_mermaid_blocks};
use crate::resources::rewrite_standalone_resource_cards;
use crate::theme::{chromium_pdf_theme_css, theme_css};

/// Builds a standalone browser HTML document.
pub fn render_html(markdown: &str, theme: &ExportTheme, title: &str) -> String {
    render_html_with_base_dir(markdown, theme, title, None)
}

/// Builds browser HTML while resolving local image paths relative to the source document.
pub fn render_html_with_base_dir(
    markdown: &str,
    theme: &ExportTheme,
    title: &str,
    base_dir: Option<&Path>,
) -> String {
    render_html_document(markdown, theme, title, base_dir, &theme_css(theme))
}

/// Builds only the safe rendered body used by rich clipboard adapters.
///
/// It intentionally reuses the exact same rewrite, URL, HTML, math and
/// Mermaid pipeline as full HTML/PDF export; callers must not parse Markdown
/// a second time merely to populate `text/html`.
pub fn render_html_fragment_with_base_dir(
    markdown: &str,
    theme: &ExportTheme,
    base_dir: Option<&Path>,
) -> String {
    render_browser_html_body(markdown, theme, base_dir)
}

/// Builds HTML tailored to Chromium's print-to-PDF pipeline.
pub fn render_chromium_pdf_html_with_base_dir(
    markdown: &str,
    theme: &ExportTheme,
    title: &str,
    base_dir: Option<&Path>,
) -> String {
    render_html_document(
        markdown,
        theme,
        title,
        base_dir,
        &chromium_pdf_theme_css(theme),
    )
}

/// Detects Tibetan text so the document can opt into an appropriate language and font fallback.
pub fn contains_tibetan_text(text: &str) -> bool {
    text.chars()
        .any(|character| ('\u{0f00}'..='\u{0fff}').contains(&character))
}

/// 标注正文采用的文字语言，让中文与藏文导出使用合适的读屏语言和字体回退。
fn render_html_document(
    markdown: &str,
    theme: &ExportTheme,
    title: &str,
    base_dir: Option<&Path>,
    css: &str,
) -> String {
    let language = if contains_tibetan_text(markdown) || contains_tibetan_text(title) {
        "bo"
    } else if markdown
        .chars()
        .chain(title.chars())
        .any(|character| ('\u{3040}'..='\u{30ff}').contains(&character))
    {
        "ja"
    } else if markdown
        .chars()
        .chain(title.chars())
        .any(|character| ('\u{ac00}'..='\u{d7af}').contains(&character))
    {
        "ko"
    } else if markdown
        .chars()
        .chain(title.chars())
        .any(|character| ('\u{4e00}'..='\u{9fff}').contains(&character))
    {
        "zh"
    } else {
        "en"
    };
    let body = render_browser_html_body(markdown, theme, base_dir);
    format!(
        "<!doctype html>\n<html lang=\"{language}\">\n<head>\n<meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n<title>{}</title>\n<style>\n{css}\n</style>\n</head>\n<body>\n<main class=\"vlt-document\">\n{body}</main>\n</body>\n</html>\n",
        escape_html(title)
    )
}

fn render_browser_html_body(
    markdown: &str,
    theme: &ExportTheme,
    base_dir: Option<&Path>,
) -> String {
    let rewritten = rewrite_unsafe_html_blocks(markdown, base_dir, markdown_options());
    let rewritten = rewrite_standalone_resource_cards(&rewritten, base_dir);
    let rewritten = rewrite_scaled_standalone_images(&rewritten);
    let rewritten = rewrite_visible_comment_blocks(&rewritten);
    let rewritten = rewrite_unsafe_html_blocks(&rewritten, base_dir, markdown_options());
    let rewritten = rewrite_display_math_blocks(&rewritten, theme);
    let rewritten = rewrite_inline_math(&rewritten, theme);
    let rewritten = rewrite_mermaid_blocks(&rewritten, theme);
    let rewritten = rewrite_table_of_contents_markers(&rewritten);
    let parser = Parser::new_ext(&rewritten, markdown_options())
        .map(|event| rewrite_local_image_event(event, base_dir));
    let mut body = String::new();
    html::push_html(&mut body, parser);
    let body = restore_visible_comment_payloads(&body);
    inject_heading_ids(&body, &collect_toc_entries(markdown))
}

fn markdown_options() -> Options {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_FOOTNOTES);
    options.insert(Options::ENABLE_TASKLISTS);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_GFM);
    options
}

fn rewrite_visible_comment_blocks(markdown: &str) -> String {
    let lines = markdown.split('\n').collect::<Vec<_>>();
    let mut rewritten = Vec::with_capacity(lines.len());
    let mut index = 0;
    let mut active_fence = None;
    while index < lines.len() {
        let line = lines[index];
        if let Some((marker, length)) = active_fence {
            rewritten.push(line.to_owned());
            if is_closing_fence(line, marker, length) {
                active_fence = None;
            }
            index += 1;
            continue;
        }
        if let Some(fence) = opening_fence(line) {
            active_fence = Some(fence);
            rewritten.push(line.to_owned());
            index += 1;
            continue;
        }
        if !is_root_comment_start(line) {
            rewritten.push(line.to_owned());
            index += 1;
            continue;
        }
        let start = index;
        while index < lines.len() && !lines[index].contains("-->") {
            index += 1;
        }
        if index >= lines.len() {
            rewritten.push(line.to_owned());
            index = start + 1;
            continue;
        }
        let raw = lines[start..=index].join("\n");
        rewritten.push(format!(
            "<pre class=\"vlt-comment\">{}{}</pre>",
            VISIBLE_COMMENT_PREFIX,
            general_purpose::STANDARD.encode(raw.as_bytes()),
        ));
        index += 1;
    }
    rewritten.join("\n")
}

const VISIBLE_COMMENT_PREFIX: &str = "gmark-comment:";

fn restore_visible_comment_payloads(html: &str) -> String {
    let mut output = String::with_capacity(html.len());
    let mut cursor = 0;
    while let Some(relative_start) = html[cursor..].find(VISIBLE_COMMENT_PREFIX) {
        let start = cursor + relative_start;
        output.push_str(&html[cursor..start]);
        let payload_start = start + VISIBLE_COMMENT_PREFIX.len();
        let end = html[payload_start..]
            .find('<')
            .map(|offset| payload_start + offset)
            .unwrap_or(html.len());
        let payload = &html[payload_start..end];
        let Ok(decoded) = general_purpose::STANDARD.decode(payload) else {
            output.push_str(VISIBLE_COMMENT_PREFIX);
            cursor = payload_start;
            continue;
        };
        let Ok(decoded) = String::from_utf8(decoded) else {
            output.push_str(VISIBLE_COMMENT_PREFIX);
            cursor = payload_start;
            continue;
        };
        output.push_str(&escape_html(&decoded));
        cursor = end;
    }
    output.push_str(&html[cursor..]);
    output
}

fn rewrite_table_of_contents_markers(markdown: &str) -> String {
    let entries = collect_toc_entries(markdown);
    let mut active_fence = None;
    let mut rewritten = Vec::with_capacity(markdown.lines().count());
    for line in markdown.split('\n') {
        if let Some((marker, length)) = active_fence {
            rewritten.push(line.to_owned());
            if is_closing_fence(line, marker, length) {
                active_fence = None;
            }
            continue;
        }
        if let Some(fence) = opening_fence(line) {
            active_fence = Some(fence);
            rewritten.push(line.to_owned());
            continue;
        }
        if line.trim().eq_ignore_ascii_case("[toc]") {
            rewritten.push(render_table_of_contents_html(&entries));
        } else {
            rewritten.push(line.to_owned());
        }
    }
    rewritten.join("\n")
}

fn render_table_of_contents_html(entries: &[TocEntry]) -> String {
    let mut output = String::from("<nav class=\"gmark-toc\" aria-label=\"Table of contents\"><ol>");
    for entry in entries {
        output.push_str(&format!(
            "<li class=\"gmark-toc-level-{}\"><a href=\"#{}\">{}</a></li>",
            entry.level,
            escape_html(&entry.slug),
            escape_html(&entry.title)
        ));
    }
    output.push_str("</ol></nav>");
    output
}

fn inject_heading_ids(body: &str, entries: &[TocEntry]) -> String {
    let mut output = body.to_owned();
    let mut search_from = 0;
    for entry in entries {
        let open = format!("<h{}>", entry.level);
        let Some(offset) = output[search_from..].find(&open) else {
            continue;
        };
        let offset = search_from + offset;
        let replacement = format!("<h{} id=\"{}\">", entry.level, escape_html(&entry.slug));
        output.replace_range(offset..offset + open.len(), &replacement);
        search_from = offset + replacement.len();
    }
    output
}

#[derive(Clone, Debug)]
struct TocEntry {
    level: u8,
    title: String,
    slug: String,
}

fn collect_toc_entries(markdown: &str) -> Vec<TocEntry> {
    let lines = markdown.lines().collect::<Vec<_>>();
    let mut entries = Vec::new();
    let mut slugs = HashMap::<String, usize>::new();
    let mut index = frontmatter_end(&lines).unwrap_or(0);
    let mut fence = None;
    let mut comment_open = false;
    while index < lines.len() {
        let line = lines[index];
        if let Some((marker, length)) = fence {
            if is_closing_fence(line, marker, length) {
                fence = None;
            }
            index += 1;
            continue;
        }
        if let Some(next) = opening_fence(line) {
            fence = Some(next);
            index += 1;
            continue;
        }
        if comment_open {
            comment_open = !line.contains("-->");
            index += 1;
            continue;
        }
        if line.trim_start().starts_with("<!--") {
            comment_open = !line.contains("-->");
            index += 1;
            continue;
        }
        if let Some((level, title)) = atx_heading(line) {
            push_toc_entry(&mut entries, &mut slugs, level, title);
            index += 1;
            continue;
        }
        if let Some(level) = lines
            .get(index + 1)
            .and_then(|line| setext_level(line))
            .filter(|_| !line.trim().is_empty())
        {
            push_toc_entry(&mut entries, &mut slugs, level, line.trim().to_owned());
            index += 2;
            continue;
        }
        index += 1;
    }
    entries
}

fn push_toc_entry(
    entries: &mut Vec<TocEntry>,
    slugs: &mut HashMap<String, usize>,
    level: u8,
    raw_title: String,
) {
    let title = plain_heading_text(&raw_title);
    if title.is_empty() {
        return;
    }
    let base = heading_slug(&title);
    let count = slugs.entry(base.clone()).or_insert(0);
    *count += 1;
    let slug = if *count == 1 {
        base
    } else {
        format!("{base}-{}", *count - 1)
    };
    entries.push(TocEntry { level, title, slug });
}

fn atx_heading(line: &str) -> Option<(u8, String)> {
    let trimmed = line.trim_start();
    let level = trimmed.bytes().take_while(|byte| *byte == b'#').count();
    if !(1..=6).contains(&level)
        || !trimmed
            .as_bytes()
            .get(level)
            .is_some_and(u8::is_ascii_whitespace)
    {
        return None;
    }
    Some((
        level as u8,
        trimmed[level..]
            .trim()
            .trim_end_matches('#')
            .trim_end()
            .to_owned(),
    ))
}

fn setext_level(line: &str) -> Option<u8> {
    let marker = line.trim();
    (!marker.is_empty() && marker.chars().all(|character| character == '='))
        .then_some(1)
        .or_else(|| {
            (!marker.is_empty() && marker.chars().all(|character| character == '-')).then_some(2)
        })
}

fn plain_heading_text(raw: &str) -> String {
    let mut visible = String::new();
    for event in Parser::new_ext(raw, markdown_options()) {
        match event {
            Event::Text(text) | Event::Code(text) => visible.push_str(&text),
            Event::SoftBreak | Event::HardBreak => visible.push(' '),
            _ => {}
        }
    }
    if visible.trim().is_empty() {
        raw.replace("**", "")
            .replace("__", "")
            .replace(['*', '_', '`'], "")
            .trim()
            .to_owned()
    } else {
        visible.trim().to_owned()
    }
}

fn heading_slug(title: &str) -> String {
    let mut slug = String::new();
    let mut previous_separator = false;
    for character in title.trim().chars() {
        if character.is_alphanumeric()
            || character == '_'
            || character == '-'
            || !character.is_ascii()
        {
            slug.extend(character.to_lowercase());
            previous_separator = false;
        } else if !previous_separator && !slug.is_empty() {
            slug.push('-');
            previous_separator = true;
        }
    }
    let slug = slug.trim_matches('-');
    if slug.is_empty() {
        "section".to_owned()
    } else {
        slug.to_owned()
    }
}

fn frontmatter_end(lines: &[&str]) -> Option<usize> {
    (lines.first()?.trim_start_matches('\u{feff}').trim() == "---").then(|| {
        lines
            .iter()
            .enumerate()
            .skip(1)
            .find_map(|(index, line)| matches!(line.trim(), "---" | "...").then_some(index + 1))
    })?
}
