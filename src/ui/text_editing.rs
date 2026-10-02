// @author kongweiguang

use std::ops::Range;

use unicode_segmentation::{GraphemeCursor, UnicodeSegmentation};

/// Clamp a platform supplied byte offset before it is used to slice Rust text.
fn char_boundary_before(text: &str, offset: usize) -> usize {
    let mut offset = offset.min(text.len());
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

/// 保留合法边界，将命中或平台提供的内部偏移向选区外侧扩展；局部游标避免扫描整段长文。
pub(crate) fn clamp_grapheme_boundary(text: &str, offset: usize, toward_end: bool) -> usize {
    let requested = offset.min(text.len());
    let offset = char_boundary_before(text, requested);
    let mut cursor = GraphemeCursor::new(offset, text.len(), true);
    let is_boundary = cursor.is_boundary(text, 0).unwrap_or(false);
    if is_boundary && (requested == offset || !toward_end) {
        return offset;
    }
    if toward_end {
        cursor
            .next_boundary(text, 0)
            .ok()
            .flatten()
            .unwrap_or(text.len())
    } else {
        cursor.prev_boundary(text, 0).ok().flatten().unwrap_or(0)
    }
}

/// Return the complete grapheme under a byte offset, treating end-of-text as
/// the preceding grapheme so a double click at the end remains useful.
pub(crate) fn grapheme_range_at(text: &str, offset: usize) -> Range<usize> {
    let offset = char_boundary_before(text, offset);
    for (start, grapheme) in text.grapheme_indices(true) {
        let end = start + grapheme.len();
        if start <= offset && offset < end {
            return start..end;
        }
    }
    let start = previous_grapheme_boundary(text, offset);
    start..text.len()
}

/// Find the previous extended-grapheme boundary; carets must never split a
/// combined accent, flag, or joined emoji sequence.
pub(crate) fn previous_grapheme_boundary(text: &str, offset: usize) -> usize {
    let offset = char_boundary_before(text, offset);
    let mut cursor = GraphemeCursor::new(offset, text.len(), true);
    cursor.prev_boundary(text, 0).ok().flatten().unwrap_or(0)
}

/// Find the next extended-grapheme boundary while keeping invalid offsets
/// inside their containing cluster instead of indexing into UTF-8 bytes.
pub(crate) fn next_grapheme_boundary(text: &str, offset: usize) -> usize {
    let offset = char_boundary_before(text, offset);
    let mut cursor = GraphemeCursor::new(offset, text.len(), true);
    cursor
        .next_boundary(text, 0)
        .ok()
        .flatten()
        .unwrap_or(text.len())
}

/// Select a Unicode word at `offset`; punctuation, whitespace, and emoji fall
/// back to one complete grapheme rather than merging into a neighboring word.
pub(crate) fn word_range_at(text: &str, offset: usize) -> Range<usize> {
    let grapheme = grapheme_range_at(text, offset);
    if grapheme.is_empty() {
        return grapheme;
    }

    text.unicode_word_indices()
        .find_map(|(start, word)| {
            let end = start + word.len();
            (start <= grapheme.start && grapheme.start < end).then_some(start..end)
        })
        .unwrap_or(grapheme)
}

/// Find the previous Unicode word start using the editor's existing movement
/// rule: skip separators and stop at the preceding word segment.
pub(crate) fn previous_word_boundary(text: &str, offset: usize) -> usize {
    let offset = char_boundary_before(text, offset);
    text.unicode_word_indices()
        .map(|(start, _)| start)
        .take_while(|start| *start < offset)
        .last()
        .unwrap_or(0)
}

/// Find the next Unicode word start using the editor's existing movement rule.
pub(crate) fn next_word_boundary(text: &str, offset: usize) -> usize {
    let offset = char_boundary_before(text, offset);
    text.unicode_word_indices()
        .map(|(start, _)| start)
        .find(|start| *start > offset)
        .unwrap_or(text.len())
}

/// Return a logical line's content range, excluding CR/LF terminators so
/// selecting or replacing a line does not accidentally consume its separator.
pub(crate) fn logical_line_range_at(text: &str, offset: usize) -> Range<usize> {
    let offset = char_boundary_before(text, offset);
    let start = logical_line_start(text, offset);
    let mut end = text[offset..]
        .find('\n')
        .map_or(text.len(), |relative| offset + relative);
    if end > start && text.as_bytes().get(end - 1) == Some(&b'\r') {
        end -= 1;
    }
    start..end
}

/// Return the start of the logical line containing a UTF-8 byte offset.
pub(crate) fn logical_line_start(text: &str, offset: usize) -> usize {
    let offset = char_boundary_before(text, offset);
    text[..offset].rfind('\n').map_or(0, |newline| newline + 1)
}

/// Return the content end of the logical line containing a UTF-8 byte offset.
pub(crate) fn logical_line_end(text: &str, offset: usize) -> usize {
    logical_line_range_at(text, offset).end
}
