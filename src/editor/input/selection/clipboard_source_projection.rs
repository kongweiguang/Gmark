// @author kongweiguang

//! Source-range projection for virtualized clipboard selections.
//!
//! This parses the complete document once so source offsets continue to refer
//! to the original Markdown while visible text and rich fragments are clipped.

use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation;

pub(super) struct VirtualizedClipboardSelection {
    pub(super) visible_text: String,
    pub(super) markdown: String,
}

/// Aligns both endpoints to extended grapheme boundaries so a clipboard range cannot split a visible character.
fn grapheme_aligned_source_range(source: &str, mut range: Range<usize>) -> Range<usize> {
    for (start, grapheme) in source.grapheme_indices(true) {
        let end = start + grapheme.len();
        if start < range.start && range.start < end {
            range.start = start;
        }
        if start < range.end && range.end < end {
            range.end = end;
            break;
        }
    }
    range
}

/// Maps a code value back to source bytes only when normalization proves the character boundaries.
fn code_value_source_offsets(
    source: &str,
    source_range: Range<usize>,
    value: &str,
) -> Option<Vec<usize>> {
    let spelling = source.get(source_range.clone())?;
    if spelling == value {
        return Some((source_range.start..=source_range.end).collect());
    }

    let opening_len = spelling.bytes().take_while(|byte| *byte == 0x60).count();
    let closing_len = spelling
        .bytes()
        .rev()
        .take_while(|byte| *byte == 0x60)
        .count();
    if opening_len > 0 && opening_len == closing_len && spelling.len() >= opening_len * 2 {
        let body_start = opening_len;
        let body_end = spelling.len() - closing_len;
        let body = spelling.get(body_start..body_end)?;
        let (mut normalized, mut offsets) =
            normalized_code_source(body, source_range.start + body_start, ' ');
        if normalized.chars().any(|ch| ch != ' ')
            && normalized.starts_with(' ')
            && normalized.ends_with(' ')
        {
            normalized.remove(0);
            offsets.remove(0);
            normalized.pop();
            offsets.pop();
        }
        if normalized == value {
            return Some(offsets);
        }
    }

    let (normalized, offsets) = normalized_code_source(spelling, source_range.start, '\n');
    if normalized == value {
        return Some(offsets);
    }

    let relative_start = spelling.find(value)?;
    if spelling[relative_start + value.len()..].contains(value) {
        return None;
    }
    let start = source_range.start + relative_start;
    Some((start..=start + value.len()).collect())
}

/// Normalizes code line endings while preserving visible UTF-8 byte boundaries as source offsets.
fn normalized_code_source(
    spelling: &str,
    source_start: usize,
    newline: char,
) -> (String, Vec<usize>) {
    let mut normalized = String::new();
    let mut offsets = vec![source_start];
    let mut chars = spelling.char_indices().peekable();
    while let Some((relative_start, ch)) = chars.next() {
        let absolute_start = source_start + relative_start;
        if ch == '\r' || ch == '\n' {
            let absolute_end = if ch == '\r' && chars.peek().is_some_and(|(_, next)| *next == '\n')
            {
                chars
                    .next()
                    .map(|(next_start, _)| source_start + next_start + 1)
                    .unwrap_or(absolute_start + ch.len_utf8())
            } else {
                absolute_start + ch.len_utf8()
            };
            normalized.push(newline);
            offsets.push(absolute_end);
        } else {
            normalized.push(ch);
            offsets.extend((1..=ch.len_utf8()).map(|delta| absolute_start + delta));
        }
    }
    (normalized, offsets)
}

/// Converts a source overlap to code-value bytes, respecting newline normalization and grapheme-safe endpoints.
fn visible_range_for_code_source(
    offsets: &[usize],
    source_range: &Range<usize>,
) -> Option<Range<usize>> {
    let start = offsets
        .partition_point(|offset| *offset < source_range.start)
        .min(offsets.len().saturating_sub(1));
    let end = offsets
        .partition_point(|offset| *offset <= source_range.end)
        .saturating_sub(1)
        .min(offsets.len().saturating_sub(1));
    (start < end).then_some(start..end)
}

/// Projects one canonical source range without reparsing a partial Markdown fragment.
fn visible_text_for_source_range(
    document: &gmark_markdown::MarkdownDocument,
    source_range: Range<usize>,
) -> Option<String> {
    let source = &document.source;
    source.get(source_range.clone())?;
    let projection = document.visible_text_projection();
    let mut visible = String::new();

    for (index, segment) in projection.segments.iter().enumerate() {
        if segment.kind == gmark_markdown::VisibleTextKind::Separator {
            let Some(previous) = projection.segments[..index]
                .iter()
                .rev()
                .find_map(|candidate| candidate.source)
            else {
                continue;
            };
            let Some(next) = projection.segments[index + 1..]
                .iter()
                .find_map(|candidate| candidate.source)
            else {
                continue;
            };
            if source_range.start <= previous.end && source_range.end >= next.start {
                let separator = projection.text.get(segment.visible.clone())?;
                let gap_start = previous.end.min(next.start);
                let gap = source.get(gap_start..next.start)?;
                if separator == "\n" && gap.bytes().filter(|byte| *byte == b'\n').count() > 1 {
                    visible.push_str("\n\n");
                } else {
                    visible.push_str(separator);
                }
            }
            continue;
        }

        let Some(source_segment) = segment.source else {
            continue;
        };
        let start = source_range.start.max(source_segment.start);
        let end = source_range.end.min(source_segment.end);
        if start >= end {
            continue;
        }

        let text = projection.text.get(segment.visible.clone())?;
        if source_segment.len() == segment.visible.len() {
            let visible_start = start - source_segment.start;
            let visible_end = end - source_segment.start;
            visible.push_str(text.get(visible_start..visible_end)?);
        } else if segment.kind == gmark_markdown::VisibleTextKind::Code {
            let offsets =
                code_value_source_offsets(source, source_segment.start..source_segment.end, text)?;
            let visible_range = visible_range_for_code_source(&offsets, &(start..end))?;
            visible.push_str(text.get(visible_range)?);
        } else {
            // Decoded entities and other non-isomorphic nodes stay whole when a range touches them.
            visible.push_str(text);
        }
    }

    (!visible.is_empty()).then_some(visible)
}

/// Clips code values through their source map while retaining semantic leaves without a safe partial map.
fn selected_inline_for_source_range(
    inline: &gmark_markdown::Inline,
    source_range: &Range<usize>,
    source: &str,
    code_context: bool,
) -> Option<gmark_markdown::Inline> {
    use gmark_markdown::{Inline, InlineKind};

    let overlaps = inline.source.start < source_range.end && source_range.start < inline.source.end;
    match &inline.kind {
        InlineKind::Text(value) if code_context => {
            if !overlaps {
                return None;
            }
            let offsets =
                code_value_source_offsets(source, inline.source.start..inline.source.end, value)?;
            let overlap = source_range.start.max(inline.source.start)
                ..source_range.end.min(inline.source.end);
            let value_range = visible_range_for_code_source(&offsets, &overlap)?;
            let selected = value.get(value_range)?;
            (!selected.is_empty()).then(|| Inline::synthetic(InlineKind::Text(selected.to_owned())))
        }
        InlineKind::Text(value) => {
            if !overlaps {
                return None;
            }
            if inline.source.len() == value.len() {
                let start = source_range.start.max(inline.source.start) - inline.source.start;
                let end = source_range.end.min(inline.source.end) - inline.source.start;
                let selected = value.get(start..end)?;
                if selected.is_empty() {
                    return None;
                }
                return Some(Inline::synthetic(InlineKind::Text(selected.to_owned())));
            }
            Some(inline.clone())
        }
        InlineKind::Code(value) => {
            if !overlaps {
                return None;
            }
            let offsets =
                code_value_source_offsets(source, inline.source.start..inline.source.end, value)?;
            let overlap = source_range.start.max(inline.source.start)
                ..source_range.end.min(inline.source.end);
            let value_range = visible_range_for_code_source(&offsets, &overlap)?;
            let selected = value.get(value_range)?;
            (!selected.is_empty()).then(|| Inline::synthetic(InlineKind::Code(selected.to_owned())))
        }
        InlineKind::SoftBreak
        | InlineKind::HardBreak
        | InlineKind::InlineMath(_)
        | InlineKind::Html(_)
        | InlineKind::FootnoteReference(_)
            if overlaps =>
        {
            Some(inline.clone())
        }
        InlineKind::TaskListMarker(_) => None,
        _ => {
            let children = inline
                .children
                .iter()
                .filter_map(|child| {
                    selected_inline_for_source_range(child, source_range, source, code_context)
                })
                .collect::<Vec<_>>();
            (!children.is_empty()).then(|| Inline {
                kind: selected_inline_kind_for_clipboard(&inline.kind),
                source: gmark_markdown::SourceRange::empty(0),
                children,
            })
        }
    }
}

/// Makes selected reference links self-contained for rich paste targets.
fn selected_inline_kind_for_clipboard(
    kind: &gmark_markdown::InlineKind,
) -> gmark_markdown::InlineKind {
    use gmark_markdown::{InlineKind, LinkKind};

    let normalize_target = |target: &gmark_markdown::LinkTarget| {
        let mut target = target.clone();
        if !target.destination.is_empty()
            && matches!(
                &target.kind,
                LinkKind::Reference | LinkKind::Collapsed | LinkKind::Shortcut
            )
        {
            target.kind = LinkKind::Inline;
        }
        target
    };

    match kind {
        InlineKind::Link(target) => InlineKind::Link(normalize_target(target)),
        InlineKind::Image(target) => InlineKind::Image(normalize_target(target)),
        _ => kind.clone(),
    }
}

/// Preserves the table shape while retaining only cells crossed by the source range.
fn selected_table_cell_for_source_range(
    cell: &gmark_markdown::TableCell,
    source_range: &Range<usize>,
    source: &str,
) -> gmark_markdown::TableCell {
    if source_range.start <= cell.source.start && cell.source.end <= source_range.end {
        return cell.clone();
    }
    let mut selected = cell.clone();
    selected.inlines = cell
        .inlines
        .iter()
        .filter_map(|inline| selected_inline_for_source_range(inline, source_range, source, false))
        .collect();
    selected
}

/// Rebuilds selected value-model nodes so partial endpoints retain surrounding Markdown styles.
fn selected_block_for_source_range(
    block: &gmark_markdown::Block,
    source_range: &Range<usize>,
    source: &str,
) -> Option<gmark_markdown::Block> {
    use gmark_markdown::BlockKind;

    let overlaps = block.source.start < source_range.end && source_range.start < block.source.end;
    if !overlaps {
        return None;
    }
    if source_range.start <= block.source.start && block.source.end <= source_range.end {
        return Some(block.clone());
    }
    if let BlockKind::Table(table) = &block.kind {
        let mut selected = block.clone();
        if let BlockKind::Table(selected_table) = &mut selected.kind {
            selected_table.header = table
                .header
                .iter()
                .map(|cell| selected_table_cell_for_source_range(cell, source_range, source))
                .collect();
            selected_table.rows = table
                .rows
                .iter()
                .map(|row| {
                    row.iter()
                        .map(|cell| {
                            selected_table_cell_for_source_range(cell, source_range, source)
                        })
                        .collect()
                })
                .collect();
        }
        return Some(selected);
    }
    if matches!(
        &block.kind,
        BlockKind::Html(_)
            | BlockKind::Metadata(_)
            | BlockKind::ThematicBreak
            | BlockKind::DisplayMath
    ) {
        return Some(block.clone());
    }
    if matches!(&block.kind, BlockKind::RawMarkdown) {
        let start = source_range.start.max(block.source.start);
        let end = source_range.end.min(block.source.end);
        let mut selected = block.clone();
        selected.raw_source = block
            .raw_source
            .get(start.saturating_sub(block.source.start)..end.saturating_sub(block.source.start))?
            .to_owned();
        return (!selected.raw_source.is_empty()).then_some(selected);
    }

    let mut selected = block.clone();
    let code_context = matches!(&block.kind, BlockKind::CodeBlock(_));
    selected.inlines = block
        .inlines
        .iter()
        .filter_map(|inline| {
            selected_inline_for_source_range(inline, source_range, source, code_context)
        })
        .collect();
    selected.children = block
        .children
        .iter()
        .filter_map(|child| selected_block_for_source_range(child, source_range, source))
        .collect();
    (!selected.inlines.is_empty() || !selected.children.is_empty()).then_some(selected)
}

/// Builds visible text and rich Markdown from one full-source parse for a virtualized selection.
pub(super) fn virtualized_clipboard_selection(
    source: &str,
    source_range: Range<usize>,
) -> Option<VirtualizedClipboardSelection> {
    source.get(source_range.clone())?;
    let source_range = grapheme_aligned_source_range(source, source_range);
    let document = gmark_markdown::parse_markdown(source);
    let visible_text = visible_text_for_source_range(&document, source_range.clone())?;
    let selected_blocks = document
        .blocks
        .iter()
        .filter_map(|block| selected_block_for_source_range(block, &source_range, source))
        .collect();
    let mut selected_document = document;
    selected_document.blocks = selected_blocks;
    let markdown = gmark_markdown::serialize_canonical_markdown(&selected_document);
    (!markdown.is_empty()).then_some(VirtualizedClipboardSelection {
        visible_text,
        markdown,
    })
}

#[cfg(test)]
#[path = "../../../../tests/unit/editor/clipboard_source_projection.rs"]
mod tests;
