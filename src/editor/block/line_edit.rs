// @author kongweiguang

//! Source-range plans for line-oriented editing shared by Block and Editor.

use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LineOperation {
    Indent,
    Outdent,
    Duplicate,
    Delete,
    MoveUp,
    MoveDown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LineEdit {
    pub(crate) range: Range<usize>,
    pub(crate) replacement: String,
    pub(crate) selection: Range<usize>,
    pub(crate) reversed: bool,
}

#[derive(Clone, Copy)]
struct LogicalLine {
    start: usize,
    content_end: usize,
    end: usize,
}

/// Keeps source offsets byte-based while recognizing all supported physical line endings.
fn logical_lines(text: &str) -> Vec<LogicalLine> {
    let bytes = text.as_bytes();
    let mut lines = Vec::new();
    let mut start = 0;
    let mut index = 0;

    while index < bytes.len() {
        let ending_len = match bytes[index] {
            b'\r' if bytes.get(index + 1) == Some(&b'\n') => 2,
            b'\r' | b'\n' => 1,
            _ => {
                index += 1;
                continue;
            }
        };
        lines.push(LogicalLine {
            start,
            content_end: index,
            end: index + ending_len,
        });
        index += ending_len;
        start = index;
    }

    lines.push(LogicalLine {
        start,
        content_end: bytes.len(),
        end: bytes.len(),
    });
    lines
}

/// Assigns a boundary offset after a line ending to the following line, matching caret behavior.
fn line_index_at(lines: &[LogicalLine], offset: usize) -> usize {
    lines
        .iter()
        .position(|line| offset < line.end || (line.start == line.end && offset == line.start))
        .unwrap_or_else(|| lines.len().saturating_sub(1))
}

/// Expands a byte selection to the logical lines it intersects without including a next row at its start.
fn selected_line_indices(lines: &[LogicalLine], range: &Range<usize>) -> (usize, usize) {
    let first = line_index_at(lines, range.start);
    if range.is_empty() {
        return (first, first);
    }
    let last = line_index_at(lines, range.end - 1);
    (first, last.max(first))
}

/// Splits a replacement region into line contents and positional endings so moved rows keep the file's newline spelling.
fn line_parts(text: &str) -> (Vec<&str>, Vec<&str>) {
    let bytes = text.as_bytes();
    let mut contents = Vec::new();
    let mut endings = Vec::new();
    let mut start = 0;
    let mut index = 0;

    while index < bytes.len() {
        let ending_len = match bytes[index] {
            b'\r' if bytes.get(index + 1) == Some(&b'\n') => 2,
            b'\r' | b'\n' => 1,
            _ => {
                index += 1;
                continue;
            }
        };
        contents.push(&text[start..index]);
        endings.push(&text[index..index + ending_len]);
        index += ending_len;
        start = index;
    }
    contents.push(&text[start..]);
    (contents, endings)
}

/// Maps an original selection endpoint through per-line prefix insertions or removals.
fn map_offset_through_prefixes(offset: usize, changes: &[(usize, usize, usize)]) -> usize {
    let mut added = 0usize;
    let mut removed = 0usize;

    for &(start, old_len, new_len) in changes {
        if offset < start {
            break;
        }
        if old_len > 0 && offset < start.saturating_add(old_len) {
            return start
                .saturating_sub(removed)
                .saturating_add(added)
                .saturating_add(new_len);
        }
        added = added.saturating_add(new_len);
        removed = removed.saturating_add(old_len);
    }

    offset.saturating_sub(removed).saturating_add(added)
}

/// Chooses an existing line ending near the edited rows, falling back to LF for a new empty document.
fn preferred_line_ending(text: &str, lines: &[LogicalLine], last: usize) -> &'static str {
    lines[..=last]
        .iter()
        .rev()
        .chain(lines.iter().skip(last + 1))
        .find_map(|line| match &text[line.content_end..line.end] {
            "\r\n" => Some("\r\n"),
            "\r" => Some("\r"),
            "\n" => Some("\n"),
            _ => None,
        })
        .unwrap_or("\n")
}

/// Plans one atomic line edit so Editor callers can commit one source transaction and restore the mapped selection.
pub(crate) fn plan_line_operation(
    text: &str,
    range: Range<usize>,
    reversed: bool,
    operation: LineOperation,
) -> Option<LineEdit> {
    if range.start > range.end
        || range.end > text.len()
        || !text.is_char_boundary(range.start)
        || !text.is_char_boundary(range.end)
    {
        return None;
    }

    let lines = logical_lines(text);
    let (first, last) = selected_line_indices(&lines, &range);
    let first_line = *lines.get(first)?;
    let last_line = *lines.get(last)?;

    match operation {
        LineOperation::Indent | LineOperation::Outdent => {
            let edit_range = first_line.start..last_line.end;
            let mut replacement = String::with_capacity(edit_range.len());
            let mut changes = Vec::with_capacity(last - first + 1);

            for line in &lines[first..=last] {
                let content = &text[line.start..line.content_end];
                let ending = &text[line.content_end..line.end];
                match operation {
                    LineOperation::Indent => {
                        changes.push((line.start, 0, 4));
                        replacement.push_str("    ");
                        replacement.push_str(content);
                    }
                    LineOperation::Outdent => {
                        let removed = if content.starts_with('\t') {
                            1
                        } else {
                            content
                                .bytes()
                                .take_while(|byte| *byte == b' ')
                                .count()
                                .min(4)
                        };
                        changes.push((line.start, removed, 0));
                        replacement.push_str(&content[removed..]);
                    }
                    _ => unreachable!(),
                }
                replacement.push_str(ending);
            }

            if replacement == text[edit_range.clone()] {
                return None;
            }
            Some(LineEdit {
                range: edit_range,
                replacement,
                selection: map_offset_through_prefixes(range.start, &changes)
                    ..map_offset_through_prefixes(range.end, &changes),
                reversed,
            })
        }
        LineOperation::Duplicate => {
            let edit_range = first_line.start..last_line.end;
            let original = &text[edit_range.clone()];
            let has_ending = matches!(original.as_bytes().last(), Some(b'\n' | b'\r'));
            let inserted_ending = if has_ending {
                ""
            } else {
                preferred_line_ending(text, &lines, last)
            };
            let mut replacement =
                String::with_capacity(original.len().saturating_mul(2) + inserted_ending.len());
            replacement.push_str(original);
            replacement.push_str(inserted_ending);
            let duplicate_start = edit_range
                .start
                .saturating_add(original.len())
                .saturating_add(inserted_ending.len());
            replacement.push_str(original);
            let selected_len = last_line.content_end.saturating_sub(first_line.start);
            Some(LineEdit {
                range: edit_range,
                replacement,
                selection: duplicate_start..duplicate_start.saturating_add(selected_len),
                reversed,
            })
        }
        LineOperation::Delete => {
            let mut edit_range = first_line.start..last_line.end;
            if last + 1 == lines.len() && first > 0 && last_line.content_end == last_line.end {
                edit_range.start = lines[first - 1].content_end;
            }
            Some(LineEdit {
                range: edit_range.clone(),
                replacement: String::new(),
                selection: edit_range.start..edit_range.start,
                reversed: false,
            })
        }
        LineOperation::MoveUp | LineOperation::MoveDown => {
            let (edit_start, edit_end, move_down) = match operation {
                LineOperation::MoveUp if first > 0 => {
                    (lines[first - 1].start, last_line.end, false)
                }
                LineOperation::MoveDown if last + 1 < lines.len() => {
                    (first_line.start, lines[last + 1].end, true)
                }
                _ => return None,
            };
            let edit_range = edit_start..edit_end;
            let (mut contents, endings) = line_parts(&text[edit_range.clone()]);
            let selected_count = last - first + 1;
            let rotated_end = selected_count + 1;
            if contents.len() < rotated_end {
                return None;
            }
            let mut selected = vec![false; contents.len()];
            if move_down {
                selected[..selected_count].fill(true);
                contents[..rotated_end].rotate_right(1);
                selected[..rotated_end].rotate_right(1);
            } else {
                selected[1..rotated_end].fill(true);
                contents[..rotated_end].rotate_left(1);
                selected[..rotated_end].rotate_left(1);
            }

            let mut replacement = String::with_capacity(edit_range.len());
            let mut selection_start = None;
            let mut selection_end = None;
            for (index, content) in contents.iter().enumerate() {
                if selected[index] && selection_start.is_none() {
                    selection_start = Some(replacement.len());
                }
                replacement.push_str(content);
                if selected[index] {
                    selection_end = Some(replacement.len());
                }
                if let Some(ending) = endings.get(index) {
                    replacement.push_str(ending);
                    if selected[index] && selected.get(index + 1) == Some(&true) {
                        selection_end = Some(replacement.len());
                    }
                }
            }

            let selection_start = selection_start?;
            let selection_end = selection_end?;
            Some(LineEdit {
                range: edit_range.clone(),
                replacement,
                selection: edit_range.start + selection_start..edit_range.start + selection_end,
                reversed,
            })
        }
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/components/block/line_edit.rs"]
mod tests;
