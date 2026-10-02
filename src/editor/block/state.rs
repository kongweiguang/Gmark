// @author kongweiguang

//! Block semantic state and block-level Markdown parsing helpers.
//!
//! This module defines the persistent block record that is serialized to and
//! from Markdown. Block-level parsing stays intentionally narrow: only syntax
//! that the runtime tree can reconstruct is parsed into structured blocks.

use std::ops::Range;
use std::path::PathBuf;

use gpui::{AnyWindowHandle, EntityId, Image, Pixels, Point, SharedString};
use uuid::Uuid;

use super::{EditingCommandId, SlashCommand};
use crate::components::markdown::html::{HtmlDocument, parse_html_document};
use crate::components::markdown::image::parse_standalone_image;
use crate::components::markdown::inline::InlineTextTree;
use crate::components::markdown::resource::ResourceRecord;
use crate::components::{TableAxisKind, TableData};

/// Supported callout variants parsed from `[!TYPE]` quote headers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CalloutVariant {
    /// Informational note callout.
    Note,
    /// Helpful tip callout.
    Tip,
    /// High-emphasis important callout.
    Important,
    /// Warning callout for risky or surprising content.
    Warning,
    /// Caution callout for potentially harmful actions.
    Caution,
}

impl CalloutVariant {
    pub fn marker(self) -> &'static str {
        match self {
            Self::Note => "NOTE",
            Self::Tip => "TIP",
            Self::Important => "IMPORTANT",
            Self::Warning => "WARNING",
            Self::Caution => "CAUTION",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Note => "Note",
            Self::Tip => "Tip",
            Self::Important => "Important",
            Self::Warning => "Warning",
            Self::Caution => "Caution",
        }
    }

    pub fn parse_header_line(line: &str) -> Option<(Self, String)> {
        let trimmed = line.trim_start();
        let rest = trimmed.strip_prefix("[!")?;
        let marker_end = rest.find(']')?;
        let marker = &rest[..marker_end];
        let variant = match marker.to_ascii_uppercase().as_str() {
            "NOTE" => Self::Note,
            "TIP" => Self::Tip,
            "IMPORTANT" => Self::Important,
            "WARNING" => Self::Warning,
            "CAUTION" => Self::Caution,
            _ => return None,
        };
        let title = rest[marker_end + 1..].trim_start().to_string();
        Some((variant, title))
    }

    pub fn header_markdown(self, title_markdown: &str) -> String {
        if title_markdown.trim().is_empty() {
            format!("[!{}]", self.marker())
        } else {
            format!("[!{}] {}", self.marker(), title_markdown)
        }
    }

    pub fn escape_plain_quote_header(title_markdown: &str) -> String {
        let mut lines = title_markdown.splitn(2, '\n');
        let first = lines.next().unwrap_or_default();
        let rest = lines.next();
        let escaped_first = if Self::parse_header_line(first).is_some() {
            format!("\\{first}")
        } else {
            first.to_string()
        };
        match rest {
            Some(rest) => format!("{escaped_first}\n{rest}"),
            None => escaped_first,
        }
    }
}

/// The semantic type of a block, determining both its Markdown syntax and
/// visual rendering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockKind {
    /// Plain paragraph with inline formatting.
    Paragraph,
    /// Horizontal rule.
    Separator,
    /// ATX or Setext heading with a CommonMark heading level.
    Heading { level: u8 },
    /// Unordered list item.
    BulletedListItem,
    /// Task-list item with checked state.
    TaskListItem { checked: bool },
    /// Ordered list item; serialization uses canonical dot markers.
    NumberedListItem,
    /// Blockquote container.
    Quote,
    /// GitHub-style alert/callout container.
    Callout(CalloutVariant),
    /// Footnote definition container.
    FootnoteDefinition,
    /// Native pipe-table block.
    Table,
    /// Fenced code block with optional language info string.
    CodeBlock { language: Option<SharedString> },
    /// Visible HTML comment block preserved as raw comment text.
    Comment,
    /// Safe raw HTML rendered through native GPUI semantic elements.
    HtmlBlock,
    /// Display math block rendered with the LaTeX pipeline.
    MathBlock,
    /// Mermaid fenced block rendered as SVG.
    MermaidBlock,
    /// Raw Markdown fallback for syntax outside the native runtime subset.
    RawMarkdown,
}

/// Opening fence parsed from a fenced code block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeFenceOpening {
    /// Fence character, either backtick or tilde.
    pub ch: char,
    /// Length of the opening fence run.
    pub len: usize,
    /// Optional language/info string after the opening fence.
    pub language: Option<SharedString>,
}

impl BlockKind {
    /// Returns true when blocks of this kind may own child blocks in the
    /// current runtime tree.
    pub fn supports_children(&self) -> bool {
        self.is_list_item() || self.is_quote_container() || self.is_footnote_definition()
    }

    pub fn is_list_item(&self) -> bool {
        matches!(
            self,
            Self::BulletedListItem | Self::TaskListItem { .. } | Self::NumberedListItem
        )
    }

    pub fn is_numbered_list_item(&self) -> bool {
        matches!(self, Self::NumberedListItem)
    }

    pub fn is_task_list_item(&self) -> bool {
        matches!(self, Self::TaskListItem { .. })
    }

    pub fn is_code_block(&self) -> bool {
        matches!(self, Self::CodeBlock { .. })
    }

    pub fn is_quote_container(&self) -> bool {
        matches!(self, Self::Quote | Self::Callout(_))
    }

    /// Blocks that render as self-contained widgets with no caret position
    /// after them. At the end of a rendered document they need a trailing
    /// paragraph so a rendered-first user can keep typing past the structure
    /// instead of having to drop to source mode.
    pub fn is_atomic_structural(&self) -> bool {
        matches!(
            self,
            Self::Separator
                | Self::Table
                | Self::CodeBlock { .. }
                | Self::MathBlock
                | Self::MermaidBlock
                | Self::HtmlBlock
                | Self::Comment
                | Self::RawMarkdown
        )
    }

    pub fn is_callout(&self) -> bool {
        matches!(self, Self::Callout(_))
    }

    /// Blocks edited as multi-line raw text that render as self-contained
    /// widgets (code, math, HTML, mermaid, comment, raw markdown). Exiting one
    /// downward with `Down` or `Ctrl/Cmd+Enter` needs a line below to land on.
    pub fn is_multiline_text_block(&self) -> bool {
        self.is_code_block()
            || matches!(
                self,
                Self::MathBlock
                    | Self::HtmlBlock
                    | Self::MermaidBlock
                    | Self::Comment
                    | Self::RawMarkdown
            )
    }

    pub fn is_footnote_definition(&self) -> bool {
        matches!(self, Self::FootnoteDefinition)
    }

    pub fn callout_variant(&self) -> Option<CalloutVariant> {
        match self {
            Self::Callout(variant) => Some(*variant),
            _ => None,
        }
    }

    pub fn is_separator(&self) -> bool {
        matches!(self, Self::Separator)
    }

    pub fn can_nest_under(&self, parent: &Self) -> bool {
        if !parent.is_list_item() {
            return false;
        }

        self.is_list_item()
            || matches!(
                self,
                Self::Paragraph
                    | Self::Quote
                    | Self::Callout(_)
                    | Self::FootnoteDefinition
                    | Self::Table
                    | Self::CodeBlock { .. }
                    | Self::Comment
                    | Self::HtmlBlock
                    | Self::MathBlock
                    | Self::MermaidBlock
                    | Self::RawMarkdown
            )
    }

    pub fn newline_sibling_kind(&self) -> Self {
        if matches!(self, Self::TaskListItem { .. }) {
            Self::TaskListItem { checked: false }
        } else if self.is_list_item() {
            self.clone()
        } else if self.is_quote_container() {
            self.clone()
        } else if self.is_footnote_definition() {
            Self::Paragraph
        } else if self.is_code_block() || self.is_separator() {
            Self::Paragraph
        } else {
            Self::Paragraph
        }
    }

    /// Live-detects a Markdown prefix from user input and returns the
    /// corresponding [`BlockKind`] together with the character count of
    /// the prefix that should be stripped.
    pub fn detect_markdown_shortcut(value: &str) -> Option<(Self, usize)> {
        if value.starts_with("###### ") {
            Some((Self::Heading { level: 6 }, 7))
        } else if value.starts_with("##### ") {
            Some((Self::Heading { level: 5 }, 6))
        } else if value.starts_with("#### ") {
            Some((Self::Heading { level: 4 }, 5))
        } else if value.starts_with("### ") {
            Some((Self::Heading { level: 3 }, 4))
        } else if value.starts_with("## ") {
            Some((Self::Heading { level: 2 }, 3))
        } else if value.starts_with("# ") {
            Some((Self::Heading { level: 1 }, 2))
        } else if let Some((checked, prefix_len)) = Self::parse_task_list_shortcut(value) {
            Some((Self::TaskListItem { checked }, prefix_len))
        } else if value.starts_with("* ") || value.starts_with("+ ") {
            Some((Self::BulletedListItem, 2))
        } else if value.starts_with("- ") {
            Some((Self::BulletedListItem, 2))
        } else if let Some(prefix_len) = Self::numbered_list_shortcut_prefix_len(value) {
            Some((Self::NumberedListItem, prefix_len))
        } else if value.starts_with("> ") {
            Some((Self::Quote, 2))
        } else {
            None
        }
    }

    pub fn parse_atx_heading_line(line: &str) -> Option<(u8, String)> {
        let trimmed_end = line.trim_end();
        let leading_spaces = trimmed_end.bytes().take_while(|b| *b == b' ').count();
        if leading_spaces > 3 {
            return None;
        }

        let rest = &trimmed_end[leading_spaces..];
        let level = rest.bytes().take_while(|b| *b == b'#').count();
        if !(1..=6).contains(&level) {
            return None;
        }

        let content = rest[level..].strip_prefix(' ')?;
        let mut content = content.trim_end().to_string();
        if let Some(closing_hash_start) = content.rfind(' ')
            && content[closing_hash_start + 1..]
                .chars()
                .all(|ch| ch == '#')
        {
            content.truncate(closing_hash_start);
            content = content.trim_end().to_string();
        }

        Some((level as u8, content))
    }

    pub fn parse_setext_underline(line: &str) -> Option<u8> {
        let trimmed_end = line.trim_end();
        let leading_spaces = trimmed_end.bytes().take_while(|b| *b == b' ').count();
        if leading_spaces > 3 {
            return None;
        }

        let rest = &trimmed_end[leading_spaces..];
        if rest.len() < 3 {
            return None;
        }

        if rest.bytes().all(|b| b == b'=') {
            Some(1)
        } else if rest.bytes().all(|b| b == b'-') {
            Some(2)
        } else {
            None
        }
    }

    pub fn parse_code_fence_opening(value: &str) -> Option<CodeFenceOpening> {
        let trimmed = value.trim_end();
        let ch = trimmed.chars().next()?;
        if ch != '`' && ch != '~' {
            return None;
        }

        let len = trimmed.chars().take_while(|&c| c == ch).count();
        if len < 3 {
            return None;
        }

        let rest = &trimmed[ch.len_utf8() * len..];
        if ch == '`' && rest.contains('`') {
            return None;
        }

        let language = rest.trim();
        Some(CodeFenceOpening {
            ch,
            len,
            language: if language.is_empty() {
                None
            } else {
                Some(language.to_string().into())
            },
        })
    }

    pub fn parse_separator_line(value: &str) -> bool {
        let trimmed_end = value.trim_end();
        let leading_spaces = trimmed_end.bytes().take_while(|b| *b == b' ').count();
        if leading_spaces > 3 {
            return false;
        }

        let rest = &trimmed_end[leading_spaces..];
        let mut marker = None;
        let mut marker_count = 0usize;
        for ch in rest.chars() {
            if ch == ' ' {
                continue;
            }
            if !matches!(ch, '-' | '*' | '_') {
                return false;
            }
            if let Some(existing) = marker {
                if existing != ch {
                    return false;
                }
            } else {
                marker = Some(ch);
            }
            marker_count += 1;
        }

        marker_count >= 3
    }

    /// Parses a task-list marker at the start of list-item content.
    ///
    /// Accepted forms are `[ ]`, `[x]`, and `[X]`, optionally followed by a
    /// space or tab before the item text. An empty title is also valid.
    pub fn parse_task_list_item_prefix(value: &str) -> Option<(bool, usize)> {
        let bytes = value.as_bytes();
        if bytes.len() < 3 || bytes[0] != b'[' || bytes[2] != b']' {
            return None;
        }

        let checked = match bytes[1] {
            b' ' => false,
            b'x' | b'X' => true,
            _ => return None,
        };

        if bytes.len() == 3 {
            return Some((checked, 3));
        }

        if matches!(bytes[3], b' ' | b'\t') {
            Some((checked, 4))
        } else {
            None
        }
    }

    fn parse_task_list_shortcut(value: &str) -> Option<(bool, usize)> {
        let rest = value.strip_prefix("- ")?;
        let (checked, prefix_len) = Self::parse_task_list_item_prefix(rest)?;
        Some((checked, 2 + prefix_len))
    }

    fn numbered_list_shortcut_prefix_len(value: &str) -> Option<usize> {
        let digit_len = value.bytes().take_while(|b| b.is_ascii_digit()).count();
        if !(1..=9).contains(&digit_len) {
            return None;
        }

        let marker = *value.as_bytes().get(digit_len)?;
        if !matches!(marker, b'.' | b')') {
            return None;
        }

        let separator = *value.as_bytes().get(digit_len + 1)?;
        matches!(separator, b' ' | b'\t').then_some(digit_len + 2)
    }
}
#[path = "state_parts/model.rs"]
mod model;
pub(crate) use model::{
    BlockDragPayload, BlockDropPlacement, BlockHostAction, MermaidSvgExportRequest,
};
pub use model::{BlockEvent, BlockRecord, PastedImageSource, UndoCaptureKind};
pub(crate) use model::{BlockImeInteraction, BlockInputCommand};
#[cfg(test)]
#[path = "../../../tests/unit/components/block/state.rs"]
mod tests;
