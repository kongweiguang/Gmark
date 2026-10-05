// @author kongweiguang

use super::*;

/// 从行首流式计数到窗口起点后的首个字素边界；窗口若切开 RI 配对或 ZWJ 序列，
/// 返回边界之后的绝对偏移，使可见 Block 从完整字素开始计数而不改变后续配对奇偶。
pub(in super::super) fn resolve_grapheme_prefix_for_window(
    snapshot: &dyn DocumentSnapshot,
    line_range: &Range<u64>,
    window_start: u64,
    cancel: &SearchCancellation,
) -> Result<(usize, u64), PagedDocumentError> {
    validate_scan(snapshot, line_range, cancel)?;
    let total_len = usize::try_from(line_range.end.saturating_sub(line_range.start))
        .map_err(|_| PagedDocumentError::RangeTooLarge)?;
    if window_start < line_range.start || window_start > line_range.end {
        return Err(PagedDocumentError::InvalidRange {
            start: window_start,
            end: window_start,
            len: snapshot.len(),
        });
    }
    if window_start == line_range.start {
        return Ok((0, window_start));
    }

    let mut chunk_start = line_range.start;
    let (_, mut text) = read_forward_chunk(snapshot, chunk_start, line_range.end, cancel)?;
    let mut cursor = GraphemeCursor::new(0, total_len, true);
    let mut grapheme_count = 0usize;
    let target = usize::try_from(window_start - line_range.start)
        .map_err(|_| PagedDocumentError::RangeTooLarge)?;

    loop {
        if cancel.is_cancelled() {
            return Err(PagedDocumentError::Cancelled);
        }
        let chunk_offset = usize::try_from(chunk_start - line_range.start)
            .map_err(|_| PagedDocumentError::RangeTooLarge)?;
        match query_grapheme_cursor(&mut cursor, CursorQuery::Next, &text, chunk_offset) {
            Ok(CursorResult::Boundary(Some(boundary))) => {
                grapheme_count = grapheme_count.saturating_add(1);
                if boundary >= target {
                    let boundary =
                        u64::try_from(boundary).map_err(|_| PagedDocumentError::RangeTooLarge)?;
                    return Ok((grapheme_count, line_range.start.saturating_add(boundary)));
                }
            }
            Ok(CursorResult::Boundary(None)) if target == total_len => {
                return Ok((grapheme_count, line_range.end));
            }
            Ok(CursorResult::Boundary(None)) => {
                return Err(PagedDocumentError::InvalidUtf8Boundary);
            }
            Ok(CursorResult::IsBoundary(_)) => {
                return Err(PagedDocumentError::InvalidUtf8Boundary);
            }
            Err(GraphemeIncomplete::PreContext(requested)) => {
                let context_end = line_range
                    .start
                    .checked_add(requested as u64)
                    .ok_or(PagedDocumentError::RangeTooLarge)?;
                let (context_start, context) =
                    read_backward_chunk(snapshot, context_end, line_range.start, cancel)?;
                let context_offset = usize::try_from(context_start - line_range.start)
                    .map_err(|_| PagedDocumentError::RangeTooLarge)?;
                cursor.provide_context(&context, context_offset);
            }
            Err(GraphemeIncomplete::NextChunk) => {
                let next_start = chunk_start.saturating_add(text.len() as u64);
                if next_start >= line_range.end {
                    if target == total_len {
                        return Ok((grapheme_count, line_range.end));
                    }
                    return Err(PagedDocumentError::InvalidUtf8Boundary);
                }
                let (read_start, next_text) =
                    read_forward_chunk(snapshot, next_start, line_range.end, cancel)?;
                if next_text.is_empty() || read_start == chunk_start {
                    return Err(PagedDocumentError::InvalidUtf8Boundary);
                }
                chunk_start = read_start;
                text = next_text;
            }
            Err(GraphemeIncomplete::PrevChunk) => {
                let (previous_start, previous) =
                    read_backward_chunk(snapshot, chunk_start, line_range.start, cancel)?;
                if previous.is_empty() || previous_start == chunk_start {
                    return Err(PagedDocumentError::InvalidUtf8Boundary);
                }
                chunk_start = previous_start;
                text = previous;
            }
            Err(GraphemeIncomplete::InvalidOffset) => {
                return Err(PagedDocumentError::InvalidUtf8Boundary);
            }
        }
    }
}
