// @author kongweiguang

//! Source-based row operations for virtualized Live selections.

use std::{ops::Range, sync::Arc};

use gmark_document::TextEdit;
use gpui::{Context, Window};

use super::{Editor, UndoSelectionSnapshot, ViewMode};
use crate::components::{EditingCommandHistory, EditingCommandId, LineOperation, UndoCaptureKind};
use crate::editor::projection::{PreparedSplitProjection, ProjectionRegion, ProjectionRegionKind};
use crate::editor::selection_surface::SelectionSurface;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LiveUnitKind {
    Paragraph,
    ListItem,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct LiveSourceUnit {
    range: Range<usize>,
    kind: LiveUnitKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct VirtualLineEdit {
    range: Range<usize>,
    replacement: String,
    selection: Range<usize>,
    reversed: bool,
}

impl Editor {
    /// Routes mounted list groups through tree mutations while retaining source edits for genuinely virtual cross-region selections.
    pub(super) fn handle_virtual_live_selection_line_operation(
        &mut self,
        operation: LineOperation,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let virtual_edit = self.virtual_surface.is_some();
        let resident_indent = matches!(operation, LineOperation::Indent | LineOperation::Outdent);
        if self.view_mode != ViewMode::Rendered
            || self.active_selection_surface != SelectionSurface::Main
            || (!virtual_edit && !resident_indent)
        {
            return false;
        }

        let Some(selection) =
            self.normalized_cross_block_selection_for_surface(SelectionSurface::Main, cx)
        else {
            return false;
        };
        let Some(source_range) = self.cross_block_source_range_for_normalized(selection, cx) else {
            return virtual_edit
                && (selection.start_index.is_none() || selection.end_index.is_none());
        };
        let snapshot = self.source_document.snapshot();
        let projection = self
            .projection_cache
            .as_ref()
            .filter(|projection| projection.revision == snapshot.revision())
            .cloned()
            .unwrap_or_else(|| {
                Arc::new(PreparedSplitProjection::from_snapshot_adaptive(
                    snapshot,
                    Self::VIRTUAL_SURFACE_REGION_THRESHOLD,
                ))
            });
        let crosses_regions = selection_spans_multiple_live_regions(&projection, &source_range);
        let has_unmounted_endpoint =
            selection.start_index.is_none() || selection.end_index.is_none();
        let resident_cross_block = selection
            .start_index
            .zip(selection.end_index)
            .is_some_and(|(start, end)| start != end)
            || crosses_regions;
        if !virtual_edit && !resident_cross_block {
            return false;
        }
        if !self.document_surface_is_editable() {
            return true;
        }

        if resident_indent
            && resident_cross_block
            && !has_unmounted_endpoint
            && selection_within_single_list_region(&projection, &source_range)
        {
            self.apply_live_list_group_indentation(operation, selection, source_range, cx);
            return true;
        }
        if virtual_edit && !has_unmounted_endpoint && !crosses_regions {
            return false;
        }

        let Some(edit) = plan_virtual_live_line_operation(
            &projection,
            source_range,
            selection.reversed,
            operation,
        ) else {
            return true;
        };

        let selected = UndoSelectionSnapshot::from_range(edit.selection.clone(), edit.reversed);
        if virtual_edit {
            self.prepare_undo_capture(UndoCaptureKind::NonCoalescible, cx);
            if !self.apply_virtual_cross_block_source_edit(edit.range, &edit.replacement, cx) {
                self.pending_virtual_undo_selection = None;
                return true;
            }
            self.apply_selection_snapshot_in_current_mode(&selected, cx);
            self.finalize_pending_undo_capture(cx);
        } else {
            if !self.apply_find_edits(
                vec![TextEdit::new(edit.range, edit.replacement)],
                edit.selection,
                cx,
            ) {
                return true;
            }
            self.apply_selection_snapshot_in_current_mode(&selected, cx);
            self.last_selection_snapshot = selected;
        }

        if let Some(command) = editing_command_for(operation) {
            EditingCommandHistory::record(command, cx);
        }
        self.request_active_block_scroll_into_view(cx);
        cx.notify();
        true
    }
}

/// Restricts mounted structural list edits to a single projected list, preserving parser-owned nesting semantics.
fn selection_within_single_list_region(
    projection: &PreparedSplitProjection,
    selection: &Range<usize>,
) -> bool {
    projection.regions.iter().any(|region| {
        region.kind == ProjectionRegionKind::List
            && region.bytes.start <= selection.start
            && selection.end <= region.bytes.end
    })
}

/// Detects cross-region selection even when both endpoints happen to remain mounted in the viewport.
fn selection_spans_multiple_live_regions(
    projection: &PreparedSplitProjection,
    selection: &Range<usize>,
) -> bool {
    let mut count = 0usize;
    for region in &projection.regions {
        if !matches!(
            region.kind,
            ProjectionRegionKind::Paragraph | ProjectionRegionKind::List
        ) {
            continue;
        }
        let overlaps = selection.start < region.bytes.end && region.bytes.start < selection.end;
        let touches_edge = !selection.is_empty()
            && (selection.start == region.bytes.end || selection.end == region.bytes.start);
        if overlaps || touches_edge {
            count += 1;
            if count > 1 {
                return true;
            }
        }
    }
    false
}

/// Builds an atomic source edit from projected Live units and refuses ranges that cross unsupported structures.
fn plan_virtual_live_line_operation(
    projection: &PreparedSplitProjection,
    selection: Range<usize>,
    reversed: bool,
    operation: LineOperation,
) -> Option<VirtualLineEdit> {
    let source = projection.source.as_str();
    if selection.start > selection.end
        || selection.end > source.len()
        || !source.is_char_boundary(selection.start)
        || !source.is_char_boundary(selection.end)
    {
        return None;
    }

    let units = live_source_units(projection)?;
    let selected_indices = units
        .iter()
        .enumerate()
        .filter_map(|(index, unit)| {
            unit_touches_selection(&unit.range, &selection).then_some(index)
        })
        .collect::<Vec<_>>();
    let (&first_index, &last_index) = (selected_indices.first()?, selected_indices.last()?);
    if selected_indices.len() < 2
        || last_index.saturating_sub(first_index) + 1 != selected_indices.len()
    {
        return None;
    }

    let first = units.get(first_index)?.range.start;
    let last = units.get(last_index)?.range.end;
    if !only_live_regions_cover(projection, first..last) {
        return None;
    }

    match operation {
        LineOperation::Duplicate => {
            plan_duplicate(source, &units, first_index, last_index, reversed)
        }
        LineOperation::Delete => plan_delete(source, &units, first_index, last_index),
        LineOperation::MoveUp => plan_move(source, &units, first_index, last_index, true, reversed),
        LineOperation::MoveDown => {
            plan_move(source, &units, first_index, last_index, false, reversed)
        }
        LineOperation::Indent | LineOperation::Outdent => {
            plan_indentation(source, &units, first_index, last_index, operation, reversed)
        }
    }
}

/// Collects source spans from the projection cache so selection never depends on mounted entities.
fn live_source_units(projection: &PreparedSplitProjection) -> Option<Vec<LiveSourceUnit>> {
    let mut units = Vec::new();
    for region in &projection.regions {
        match region.kind {
            ProjectionRegionKind::Paragraph if !region.bytes.is_empty() => {
                units.push(LiveSourceUnit {
                    range: region.bytes.clone(),
                    kind: LiveUnitKind::Paragraph,
                });
            }
            ProjectionRegionKind::List => {
                for range in list_item_source_ranges(projection, region)? {
                    units.push(LiveSourceUnit {
                        range,
                        kind: LiveUnitKind::ListItem,
                    });
                }
            }
            _ => {}
        }
    }
    units.sort_by_key(|unit| unit.range.start);
    units
        .windows(2)
        .all(|pair| pair[0].range.end <= pair[1].range.start)
        .then_some(units)
}

/// Finds outer list-item source spans while leaving blank separators between items intact.
fn list_item_source_ranges(
    projection: &PreparedSplitProjection,
    region: &ProjectionRegion,
) -> Option<Vec<Range<usize>>> {
    let lines = projection.lines.get(region.lines.clone())?;
    let mut starts = Vec::new();
    let mut minimum_indent = None;
    for (index, line) in lines.iter().enumerate() {
        if let Some(indent) = list_marker_indent(line) {
            minimum_indent =
                Some(minimum_indent.map_or(indent, |minimum: usize| minimum.min(indent)));
            starts.push((index, indent));
        }
    }
    let minimum_indent = minimum_indent?;
    let starts = starts
        .into_iter()
        .filter_map(|(index, indent)| (indent == minimum_indent).then_some(index))
        .collect::<Vec<_>>();
    if starts.is_empty() {
        return None;
    }

    let mut line_offsets = Vec::with_capacity(lines.len());
    let mut offset = region.bytes.start;
    for line in lines {
        line_offsets.push(offset);
        offset = offset.checked_add(line.len())?.checked_add(1)?;
    }

    let mut ranges = Vec::with_capacity(starts.len());
    for (position, &start_line) in starts.iter().enumerate() {
        let mut end_line = starts.get(position + 1).copied().unwrap_or(lines.len());
        if end_line < lines.len() {
            while end_line > start_line + 1 && lines[end_line - 1].trim().is_empty() {
                end_line -= 1;
            }
        }
        let end_line = end_line.saturating_sub(1);
        let range_start = *line_offsets.get(start_line)?;
        let range_end = line_offsets
            .get(end_line)
            .copied()?
            .checked_add(lines.get(end_line)?.len())?;
        if range_start < range_end {
            ranges.push(range_start..range_end);
        }
    }
    Some(ranges)
}

/// Returns the source indentation of a Markdown list marker, using the parser's four-column tab stops.
fn list_marker_indent(line: &str) -> Option<usize> {
    let line = line.strip_suffix('\r').unwrap_or(line);
    let mut byte_offset = 0usize;
    let mut columns = 0usize;
    for character in line.chars() {
        match character {
            ' ' => columns += 1,
            '\t' => columns += 4 - (columns % 4),
            _ => break,
        }
        byte_offset += character.len_utf8();
    }
    let rest = line.get(byte_offset..)?;
    let first = rest.chars().next()?;
    if matches!(first, '-' | '*' | '+') {
        return rest
            .get(first.len_utf8()..)?
            .chars()
            .next()
            .filter(|character| matches!(character, ' ' | '\t'))
            .map(|_| columns);
    }

    let digit_len = rest.bytes().take_while(u8::is_ascii_digit).count();
    if !(1..=9).contains(&digit_len) {
        return None;
    }
    let marker = *rest.as_bytes().get(digit_len)?;
    let separator = *rest.as_bytes().get(digit_len + 1)?;
    (matches!(marker, b'.' | b')') && matches!(separator, b' ' | b'\t')).then_some(columns)
}

/// Treats a unit boundary as selected when a cross-block caret lands exactly on that block edge.
fn unit_touches_selection(unit: &Range<usize>, selection: &Range<usize>) -> bool {
    selection.start < unit.end && unit.start < selection.end
        || (!selection.is_empty() && (unit.end == selection.start || unit.start == selection.end))
}

/// Rejects selected ranges that include a non-editable projected structure such as a table or heading.
fn only_live_regions_cover(projection: &PreparedSplitProjection, range: Range<usize>) -> bool {
    projection.regions.iter().all(|region| {
        let overlaps = region.bytes.start < range.end && range.start < region.bytes.end;
        !overlaps
            || matches!(
                region.kind,
                ProjectionRegionKind::Blank
                    | ProjectionRegionKind::Paragraph
                    | ProjectionRegionKind::List
            )
    })
}

/// Inserts a duplicate after the selected unit group with the existing neighboring separator spelling.
fn plan_duplicate(
    source: &str,
    units: &[LiveSourceUnit],
    first: usize,
    last: usize,
    reversed: bool,
) -> Option<VirtualLineEdit> {
    let start = units.get(first)?.range.start;
    let end = units.get(last)?.range.end;
    let following = units.get(last + 1);
    let separator = following
        .map(|following| &source[end..following.range.start])
        .filter(|separator| !separator.is_empty() && separator.chars().all(char::is_whitespace))
        .or_else(|| {
            first.checked_sub(1).and_then(|previous| {
                let previous = units.get(previous)?;
                let separator = &source[previous.range.end..start];
                (!separator.is_empty() && separator.chars().all(char::is_whitespace))
                    .then_some(separator)
            })
        })
        .or_else(|| {
            (first < last)
                .then(|| &source[units[first].range.end..units[first + 1].range.start])
                .filter(|separator| {
                    !separator.is_empty() && separator.chars().all(char::is_whitespace)
                })
        })
        .unwrap_or("\n\n");
    let content = &source[start..end];
    let selection_start = end.checked_add(separator.len())?;
    let selection_end = selection_start.checked_add(content.len())?;
    Some(VirtualLineEdit {
        range: end..end,
        replacement: format!("{separator}{content}"),
        selection: selection_start..selection_end,
        reversed,
    })
}

/// Deletes selected units together with only one boundary separator so neighboring blocks remain separated.
fn plan_delete(
    source: &str,
    units: &[LiveSourceUnit],
    first: usize,
    last: usize,
) -> Option<VirtualLineEdit> {
    let selected_start = units.get(first)?.range.start;
    let selected_end = units.get(last)?.range.end;
    let following = units.get(last + 1).filter(|next| {
        source[selected_end..next.range.start]
            .chars()
            .all(char::is_whitespace)
    });
    let preceding = first
        .checked_sub(1)
        .and_then(|index| units.get(index))
        .filter(|previous| {
            source[previous.range.end..selected_start]
                .chars()
                .all(char::is_whitespace)
        });
    let range = following.map_or_else(
        || {
            preceding.map_or(selected_start..selected_end, |previous| {
                previous.range.end..selected_end
            })
        },
        |next| selected_start..next.range.start,
    );
    Some(VirtualLineEdit {
        selection: range.start..range.start,
        range,
        replacement: String::new(),
        reversed: false,
    })
}

/// Moves a selected unit group by rotating it with one adjacent source unit and retaining every separator byte.
fn plan_move(
    source: &str,
    units: &[LiveSourceUnit],
    first: usize,
    last: usize,
    up: bool,
    reversed: bool,
) -> Option<VirtualLineEdit> {
    let (edit_first, edit_last, selected_position, order) = if up {
        let previous = first.checked_sub(1)?;
        (
            previous,
            last,
            0,
            (first..=last)
                .chain(std::iter::once(previous))
                .collect::<Vec<_>>(),
        )
    } else {
        let next = last.checked_add(1)?;
        units.get(next)?;
        (
            first,
            next,
            1,
            std::iter::once(next)
                .chain(first..=last)
                .collect::<Vec<_>>(),
        )
    };
    let original_group = units.get(edit_first)?.range.start..units.get(edit_last)?.range.end;
    let original = source.get(original_group.clone())?;
    for pair in units[edit_first..=edit_last].windows(2) {
        if !only_whitespace(source, pair[0].range.end..pair[1].range.start) {
            return None;
        }
    }
    let original_separators = units[edit_first..=edit_last]
        .windows(2)
        .map(|pair| {
            source
                .get(pair[0].range.end..pair[1].range.start)
                .map(str::to_owned)
        })
        .collect::<Option<Vec<_>>>()?;
    let mut replacement = String::with_capacity(original.len());
    let mut selection_start = None;
    let mut selection_end = None;
    let selected_count = last - first + 1;
    for (position, unit_index) in order.iter().copied().enumerate() {
        if position > 0 {
            replacement.push_str(original_separators.get(position - 1)?);
        }
        if position == selected_position {
            selection_start = Some(original_group.start.checked_add(replacement.len())?);
        }
        let unit = units.get(unit_index)?;
        replacement.push_str(source.get(unit.range.clone())?);
        if position >= selected_position && position < selected_position + selected_count {
            selection_end = Some(original_group.start.checked_add(replacement.len())?);
        }
    }
    if replacement == original {
        return None;
    }
    Some(VirtualLineEdit {
        range: original_group,
        replacement,
        selection: selection_start?..selection_end?,
        reversed,
    })
}

/// Verifies a source gap before reordering so an unselected heading or atomic block cannot be moved or dropped.
fn only_whitespace(source: &str, range: Range<usize>) -> bool {
    source
        .get(range)
        .is_some_and(|gap| gap.chars().all(char::is_whitespace))
}

/// Indents or outdents only selected unit contents; blank lines and their existing separators remain untouched.
fn plan_indentation(
    source: &str,
    units: &[LiveSourceUnit],
    first: usize,
    last: usize,
    operation: LineOperation,
    reversed: bool,
) -> Option<VirtualLineEdit> {
    let range = units.get(first)?.range.start..units.get(last)?.range.end;
    let mut replacement = String::with_capacity(range.len());
    let mut cursor = range.start;
    let mut changed = false;
    for unit in units.get(first..=last)? {
        replacement.push_str(source.get(cursor..unit.range.start)?);
        let current = source.get(unit.range.clone())?;
        let Some(next) = transform_live_unit(current, unit.kind, operation) else {
            return None;
        };
        changed |= current != next;
        replacement.push_str(&next);
        cursor = unit.range.end;
    }
    replacement.push_str(source.get(cursor..range.end)?);
    if !changed {
        return None;
    }
    let selection_end = range.start.checked_add(replacement.len())?;
    Some(VirtualLineEdit {
        range: range.clone(),
        replacement,
        selection: range.start..selection_end,
        reversed,
    })
}

/// Applies one indentation prefix per nonblank source line and preserves LF, CRLF, and bare CR endings.
fn transform_live_unit(text: &str, kind: LiveUnitKind, operation: LineOperation) -> Option<String> {
    let prefix = match kind {
        LiveUnitKind::Paragraph => 4,
        LiveUnitKind::ListItem => 2,
    };
    let mut output = String::with_capacity(text.len().saturating_add(prefix));
    let bytes = text.as_bytes();
    let mut start = 0usize;
    let mut changed = false;
    while start < bytes.len() {
        let mut end = start;
        while end < bytes.len() && !matches!(bytes[end], b'\r' | b'\n') {
            end += 1;
        }
        let content = text.get(start..end)?;
        let ending_end = if bytes.get(end) == Some(&b'\r') && bytes.get(end + 1) == Some(&b'\n') {
            end + 2
        } else if end < bytes.len() {
            end + 1
        } else {
            end
        };
        let ending = text.get(end..ending_end)?;
        if content.is_empty() {
            output.push_str(ending);
        } else {
            match operation {
                LineOperation::Indent => {
                    for _ in 0..prefix {
                        output.push(' ');
                    }
                    output.push_str(content);
                    changed = true;
                }
                LineOperation::Outdent => {
                    let removed = if content.starts_with('\t') {
                        1
                    } else {
                        content
                            .bytes()
                            .take_while(|byte| *byte == b' ')
                            .count()
                            .min(prefix)
                    };
                    output.push_str(content.get(removed..)?);
                    changed |= removed > 0;
                }
                _ => return None,
            }
            output.push_str(ending);
        }
        start = ending_end;
    }
    (changed && output != text).then_some(output)
}

/// Maps history-visible row actions to the existing command history without inventing indent entries.
fn editing_command_for(operation: LineOperation) -> Option<EditingCommandId> {
    match operation {
        LineOperation::Duplicate => Some(EditingCommandId::DuplicateBlock),
        LineOperation::Delete => Some(EditingCommandId::DeleteBlock),
        LineOperation::MoveUp => Some(EditingCommandId::MoveBlockUp),
        LineOperation::MoveDown => Some(EditingCommandId::MoveBlockDown),
        LineOperation::Indent | LineOperation::Outdent => None,
    }
}
