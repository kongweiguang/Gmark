// @author kongweiguang

//! Unicode word and grapheme boundaries for bounded, disk-backed Source rows.

use std::ops::Range;

use gmark_document_core::{DocumentSnapshot, SnapshotError};
use gmark_paged_document::{MAX_SYSTEM_CLIPBOARD_BYTES, PagedDocumentError, SearchCancellation};
use unicode_categories::UnicodeCategories;
use unicode_segmentation::{GraphemeCursor, GraphemeIncomplete, UnicodeSegmentation};

use self::words::{is_safe_word_separator, resolve_ascii_word_move, resolve_ascii_word_range};
use super::BoundedLineWindow;

#[path = "source_boundaries_words.rs"]
mod words;

#[path = "source_boundaries_prefix.rs"]
mod prefix;
pub(super) use prefix::resolve_grapheme_prefix_for_window;

const BOUNDARY_CHUNK_BYTES: u64 = 64 * 1024;

#[derive(Clone, Copy)]
enum CursorQuery {
    Is,
    Previous,
    Next,
}

#[derive(Clone, Copy)]
enum CursorResult {
    IsBoundary(bool),
    Boundary(Option<usize>),
}

/// 从快照读取失败映射到 Source 操作的公开错误类型，保留越界和取消语义。
fn map_snapshot_error(error: SnapshotError) -> PagedDocumentError {
    match error {
        SnapshotError::InvalidRange { start, end, len } => {
            PagedDocumentError::InvalidRange { start, end, len }
        }
        SnapshotError::RangeTooLarge => PagedDocumentError::RangeTooLarge,
        SnapshotError::Read(message) => PagedDocumentError::Search(message),
    }
}

/// 检查扫描取消并校验行范围，避免把损坏索引转换为无效快照读取。
fn validate_scan(
    snapshot: &dyn DocumentSnapshot,
    line_range: &Range<u64>,
    cancel: &SearchCancellation,
) -> Result<(), PagedDocumentError> {
    if cancel.is_cancelled() {
        return Err(PagedDocumentError::Cancelled);
    }
    if line_range.start > line_range.end || line_range.end > snapshot.len() {
        return Err(PagedDocumentError::InvalidRange {
            start: line_range.start,
            end: line_range.end,
            len: snapshot.len(),
        });
    }
    Ok(())
}

/// 行索引范围包含换行符，但 Source 的可见文本不包含；所有边界操作统一使用内容范围。
fn content_line_range(
    snapshot: &dyn DocumentSnapshot,
    line_range: Range<u64>,
    cancel: &SearchCancellation,
) -> Result<Range<u64>, PagedDocumentError> {
    validate_scan(snapshot, &line_range, cancel)?;
    if line_range.is_empty() {
        return Ok(line_range);
    }
    if cancel.is_cancelled() {
        return Err(PagedDocumentError::Cancelled);
    }
    let last = snapshot
        .read_range(line_range.end - 1..line_range.end)
        .map_err(map_snapshot_error)?;
    let mut end = line_range.end;
    match last.first().copied() {
        Some(b'\n') => {
            end -= 1;
            if end > line_range.start {
                if cancel.is_cancelled() {
                    return Err(PagedDocumentError::Cancelled);
                }
                let previous = snapshot
                    .read_range(end - 1..end)
                    .map_err(map_snapshot_error)?;
                if previous.first() == Some(&b'\r') {
                    end -= 1;
                }
            }
        }
        Some(b'\r') => end -= 1,
        _ => {}
    }
    Ok(line_range.start..end)
}

/// 给无法恢复增量状态的 Unicode 分词窗口设总暂存上限，并先检查整数溢出。
fn check_unicode_window_growth(
    current_len: usize,
    additional_len: usize,
) -> Result<(), PagedDocumentError> {
    let total = current_len
        .checked_add(additional_len)
        .and_then(|len| u64::try_from(len).ok())
        .ok_or(PagedDocumentError::RangeTooLarge)?;
    if total > MAX_SYSTEM_CLIPBOARD_BYTES {
        return Err(PagedDocumentError::RangeTooLarge);
    }
    Ok(())
}

/// 把 Word_Break 会忽略的 Mark/Format 识别为可能依赖窗口外字母的词上下文。
fn is_word_ignored_character(ch: char) -> bool {
    ch.is_mark_nonspacing()
        || ch.is_mark_spacing_combining()
        || ch.is_mark_enclosing()
        || ch.is_other_format()
}

/// 有界扩展词扫描窗口；预算和可恢复分配失败都在修改缓冲前处理。
fn extend_unicode_word_window(
    snapshot: &dyn DocumentSnapshot,
    line_range: &Range<u64>,
    window_start: &mut u64,
    text: &mut String,
    expand_left: bool,
    expand_right: bool,
    cancel: &SearchCancellation,
) -> Result<(), PagedDocumentError> {
    if expand_left {
        let (new_start, prefix) =
            read_backward_chunk(snapshot, *window_start, line_range.start, cancel)?;
        if prefix.is_empty() || new_start == *window_start {
            return Err(PagedDocumentError::Search(
                "Unicode word scan made no progress while reading backward".into(),
            ));
        }
        check_unicode_window_growth(text.len(), prefix.len())?;
        text.try_reserve_exact(prefix.len())
            .map_err(|_| PagedDocumentError::RangeTooLarge)?;
        text.insert_str(0, &prefix);
        *window_start = new_start;
    }
    if expand_right {
        let window_end = *window_start + text.len() as u64;
        let (_, suffix) = read_forward_chunk(snapshot, window_end, line_range.end, cancel)?;
        if suffix.is_empty() {
            return Err(PagedDocumentError::Search(
                "Unicode word scan made no progress while reading forward".into(),
            ));
        }
        check_unicode_window_growth(text.len(), suffix.len())?;
        text.try_reserve_exact(suffix.len())
            .map_err(|_| PagedDocumentError::RangeTooLarge)?;
        text.push_str(&suffix);
    }
    Ok(())
}

/// 将窗口起点退回 UTF-8 字符起点；最多查看前三个前序字节即可确定字符宽度。
fn align_utf8_start(
    snapshot: &dyn DocumentSnapshot,
    start: u64,
    lower_bound: u64,
    cancel: &SearchCancellation,
) -> Result<u64, PagedDocumentError> {
    if start <= lower_bound {
        return Ok(lower_bound);
    }
    if cancel.is_cancelled() {
        return Err(PagedDocumentError::Cancelled);
    }
    let probe_start = start.saturating_sub(3).max(lower_bound);
    let bytes = snapshot
        .read_range(probe_start..start)
        .map_err(map_snapshot_error)?;
    let Some(relative) = bytes
        .iter()
        .rposition(|byte| byte & 0b1100_0000 != 0b1000_0000)
    else {
        return Ok(start);
    };
    let lead = bytes[relative];
    let width = if lead < 0x80 {
        1
    } else if lead & 0b1110_0000 == 0b1100_0000 {
        2
    } else if lead & 0b1111_0000 == 0b1110_0000 {
        3
    } else if lead & 0b1111_1000 == 0b1111_0000 {
        4
    } else {
        return Ok(start);
    };
    let lead_offset = probe_start + relative as u64;
    Ok(if lead_offset + width as u64 > start {
        lead_offset
    } else {
        start
    })
}

/// 读取一段不超过 64 KiB 的 UTF-8 文本；仅允许末尾不完整字符被留给下一块。
fn read_forward_chunk(
    snapshot: &dyn DocumentSnapshot,
    start: u64,
    upper_bound: u64,
    cancel: &SearchCancellation,
) -> Result<(u64, String), PagedDocumentError> {
    if cancel.is_cancelled() {
        return Err(PagedDocumentError::Cancelled);
    }
    let end = start.saturating_add(BOUNDARY_CHUNK_BYTES).min(upper_bound);
    if start >= end {
        return Ok((start, String::new()));
    }
    let bytes = snapshot
        .read_range(start..end)
        .map_err(map_snapshot_error)?;
    let valid_len = match std::str::from_utf8(&bytes) {
        Ok(text) => text.len(),
        Err(error) if error.error_len().is_none() => error.valid_up_to(),
        Err(_) => return Err(PagedDocumentError::Binary),
    };
    let text =
        String::from_utf8(bytes[..valid_len].to_vec()).map_err(|_| PagedDocumentError::Binary)?;
    Ok((start, text))
}

/// 从行尾向前读取 UTF-8 边界对齐的有限块，避免跨块反向扫描拆开多字节字符。
fn read_backward_chunk(
    snapshot: &dyn DocumentSnapshot,
    end: u64,
    lower_bound: u64,
    cancel: &SearchCancellation,
) -> Result<(u64, String), PagedDocumentError> {
    let desired = end
        .saturating_sub(BOUNDARY_CHUNK_BYTES.saturating_sub(3))
        .max(lower_bound);
    let start = align_utf8_start(snapshot, desired, lower_bound, cancel)?;
    if cancel.is_cancelled() {
        return Err(PagedDocumentError::Cancelled);
    }
    let bytes = snapshot
        .read_range(start..end)
        .map_err(map_snapshot_error)?;
    let text = String::from_utf8(bytes).map_err(|_| PagedDocumentError::Binary)?;
    Ok((start, text))
}

/// 在局部文本中返回命中位置对应的 Unicode 单词，非单词字符回退到完整字素。
fn local_word_range(text: &str, offset: usize) -> Range<usize> {
    let grapheme = local_grapheme_range(text, offset);
    text.unicode_word_indices()
        .find_map(|(start, word)| {
            let end = start + word.len();
            (start <= grapheme.start && grapheme.start < end).then_some(start..end)
        })
        .unwrap_or(grapheme)
}

/// 只返回 Unicode 分词器确认的单词；非单词命中由完整字素游标处理。
fn local_unicode_word_range(text: &str, offset: usize) -> Option<Range<usize>> {
    let grapheme = local_grapheme_range(text, offset);
    text.unicode_word_indices().find_map(|(start, word)| {
        let end = start + word.len();
        (start <= grapheme.start && grapheme.start < end).then_some(start..end)
    })
}

/// 执行一类 GraphemeCursor 查询，统一保留其跨块状态与上下文请求。
fn query_grapheme_cursor(
    cursor: &mut GraphemeCursor,
    query: CursorQuery,
    text: &str,
    chunk_start: usize,
) -> Result<CursorResult, GraphemeIncomplete> {
    match query {
        CursorQuery::Is => cursor
            .is_boundary(text, chunk_start)
            .map(CursorResult::IsBoundary),
        CursorQuery::Previous => cursor
            .prev_boundary(text, chunk_start)
            .map(CursorResult::Boundary),
        CursorQuery::Next => cursor
            .next_boundary(text, chunk_start)
            .map(CursorResult::Boundary),
    }
}

/// 以 <=64 KiB 主块回答字素边界查询，按 GraphemeCursor 请求补前文或相邻块。
fn resolve_grapheme_query(
    snapshot: &dyn DocumentSnapshot,
    line_range: &Range<u64>,
    offset: u64,
    query: CursorQuery,
    cancel: &SearchCancellation,
) -> Result<CursorResult, PagedDocumentError> {
    let line_len = line_range.end - line_range.start;
    let total_len = usize::try_from(line_len).map_err(|_| PagedDocumentError::RangeTooLarge)?;
    let offset = offset.clamp(line_range.start, line_range.end);
    let cursor_offset = usize::try_from(offset - line_range.start)
        .map_err(|_| PagedDocumentError::RangeTooLarge)?;
    if matches!(query, CursorQuery::Is) && (cursor_offset == 0 || cursor_offset == total_len) {
        return Ok(CursorResult::IsBoundary(true));
    }
    if matches!(query, CursorQuery::Previous) && cursor_offset == 0 {
        return Ok(CursorResult::Boundary(None));
    }
    if matches!(query, CursorQuery::Next) && cursor_offset == total_len {
        return Ok(CursorResult::Boundary(None));
    }

    let desired_start = offset
        .saturating_sub(BOUNDARY_CHUNK_BYTES / 2)
        .max(line_range.start);
    let mut chunk_start = align_utf8_start(snapshot, desired_start, line_range.start, cancel)?;
    let (_, mut text) = read_forward_chunk(snapshot, chunk_start, line_range.end, cancel)?;
    let mut cursor = GraphemeCursor::new(cursor_offset, total_len, true);
    loop {
        if cancel.is_cancelled() {
            return Err(PagedDocumentError::Cancelled);
        }
        let local_chunk_start = usize::try_from(chunk_start - line_range.start)
            .map_err(|_| PagedDocumentError::RangeTooLarge)?;
        match query_grapheme_cursor(&mut cursor, query, &text, local_chunk_start) {
            Ok(CursorResult::IsBoundary(value)) => {
                return Ok(CursorResult::IsBoundary(value));
            }
            Ok(CursorResult::Boundary(value)) => {
                return Ok(CursorResult::Boundary(value));
            }
            Err(GraphemeIncomplete::PreContext(requested)) => {
                let context_end = line_range.start + requested as u64;
                let (context_start, context) =
                    read_backward_chunk(snapshot, context_end, line_range.start, cancel)?;
                let context_start = usize::try_from(context_start - line_range.start)
                    .map_err(|_| PagedDocumentError::RangeTooLarge)?;
                cursor.provide_context(&context, context_start);
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
            Err(GraphemeIncomplete::NextChunk) => {
                let chunk_end = chunk_start + text.len() as u64;
                let (next_start, next) =
                    read_forward_chunk(snapshot, chunk_end, line_range.end, cancel)?;
                if next.is_empty() || next_start == chunk_start {
                    return Err(PagedDocumentError::InvalidUtf8Boundary);
                }
                chunk_start = next_start;
                text = next;
            }
            Err(GraphemeIncomplete::InvalidOffset) => {
                return Err(PagedDocumentError::InvalidUtf8Boundary);
            }
        }
    }
}

/// 用流式前后边界查询选中完整字素，不保留整条长行或组合序列副本。
fn resolve_grapheme_range(
    snapshot: &dyn DocumentSnapshot,
    line_range: &Range<u64>,
    hit: u64,
    cancel: &SearchCancellation,
) -> Result<Range<u64>, PagedDocumentError> {
    let hit = hit.clamp(line_range.start, line_range.end);
    let boundary = resolve_grapheme_query(snapshot, line_range, hit, CursorQuery::Is, cancel)?;
    let CursorResult::IsBoundary(is_boundary) = boundary else {
        return Err(PagedDocumentError::InvalidUtf8Boundary);
    };
    let start = if is_boundary && hit < line_range.end {
        hit
    } else {
        let previous =
            resolve_grapheme_query(snapshot, line_range, hit, CursorQuery::Previous, cancel)?;
        match previous {
            CursorResult::Boundary(Some(offset)) => line_range.start + offset as u64,
            CursorResult::Boundary(None) => line_range.start,
            CursorResult::IsBoundary(_) => return Err(PagedDocumentError::InvalidUtf8Boundary),
        }
    };
    if start == line_range.end {
        return Ok(start..start);
    }
    let next = resolve_grapheme_query(snapshot, line_range, start, CursorQuery::Next, cancel)?;
    let end = match next {
        CursorResult::Boundary(Some(offset)) => line_range.start + offset as u64,
        CursorResult::Boundary(None) => line_range.end,
        CursorResult::IsBoundary(_) => return Err(PagedDocumentError::InvalidUtf8Boundary),
    };
    Ok(start..end)
}

/// 将字节命中收敛到一个完整扩展字素，行尾命中按前一个字素处理。
fn local_grapheme_range(text: &str, offset: usize) -> Range<usize> {
    let offset = offset.min(text.len());
    for (start, grapheme) in text.grapheme_indices(true) {
        let end = start + grapheme.len();
        if start <= offset && offset < end {
            return start..end;
        }
    }
    text.grapheme_indices(true)
        .next_back()
        .map_or(offset..offset, |(start, grapheme)| {
            start..start + grapheme.len()
        })
}

/// 按字素或单词规则计算局部水平移动目标，调用方负责处理窗口外延。
fn local_horizontal_target(text: &str, offset: usize, forward: bool, by_word: bool) -> usize {
    let offset = offset.min(text.len());
    if by_word {
        if forward {
            text.unicode_word_indices()
                .map(|(start, _)| start)
                .find(|start| *start > offset)
                .unwrap_or(text.len())
        } else {
            text.unicode_word_indices()
                .map(|(start, _)| start)
                .take_while(|start| *start < offset)
                .last()
                .unwrap_or(0)
        }
    } else if forward {
        text.grapheme_indices(true)
            .map(|(start, grapheme)| start + grapheme.len())
            .find(|end| *end > offset)
            .unwrap_or(text.len())
    } else {
        text.grapheme_indices(true)
            .map(|(start, _)| start)
            .take_while(|start| *start < offset)
            .last()
            .unwrap_or(0)
    }
}

/// 用带截断哨兵的 GraphemeCursor 验证可见候选边界；缺前文时宁可异步解析也不猜奇偶。
fn bounded_grapheme_boundary(row: &BoundedLineWindow, offset: usize) -> Option<bool> {
    let text = &row.text;
    if offset > text.len() || !text.is_char_boundary(offset) {
        return None;
    }
    if offset == 0 {
        return (!row.leading_truncated).then_some(true);
    }
    if offset == text.len() {
        return (!row.trailing_truncated).then_some(true);
    }
    let chunk_start = if row.leading_truncated {
        1_usize
    } else {
        0_usize
    };
    let total_len = chunk_start
        .checked_add(text.len())?
        .checked_add(if row.trailing_truncated { 1 } else { 0 })?;
    let cursor_offset = chunk_start.checked_add(offset)?;
    let mut cursor = GraphemeCursor::new(cursor_offset, total_len, true);
    cursor.is_boundary(text, chunk_start).ok()
}

/// 分页截断处的 Mark/Format 可能依赖窗口外词上下文，因此未证明时交给完整 UAX 解析。
pub(super) fn bounded_word_range(row: &BoundedLineWindow, hit: u64) -> Option<Range<u64>> {
    let local_hit = usize::try_from(hit.saturating_sub(row.content_range.start))
        .unwrap_or(usize::MAX)
        .min(row.text.len());
    let local = local_word_range(&row.text, local_hit);
    let is_word = local_unicode_word_range(&row.text, local_hit).is_some();
    let has_ignored_context = !is_word
        && (row.leading_truncated || row.trailing_truncated)
        && row.text[local.clone()]
            .chars()
            .all(is_word_ignored_character);
    if has_ignored_context {
        return None;
    }
    let start_proven = !row.leading_truncated
        || row.text[..local.start]
            .chars()
            .next_back()
            .is_some_and(is_safe_word_separator);
    let end_proven = !row.trailing_truncated
        || row.text[local.end..]
            .chars()
            .next()
            .is_some_and(is_safe_word_separator);
    if is_word && (!start_proven || !end_proven) {
        return None;
    }
    if bounded_grapheme_boundary(row, local.start) != Some(true)
        || bounded_grapheme_boundary(row, local.end) != Some(true)
    {
        return None;
    }
    Some(row.content_range.start + local.start as u64..row.content_range.start + local.end as u64)
}

/// 去掉行终止符后解析词/字素；ASCII 常量内存扫描，Unicode 上下文暂存最多 64 MiB。
pub(super) fn resolve_word_range(
    snapshot: &dyn DocumentSnapshot,
    line_range: Range<u64>,
    hit: u64,
    cancel: &SearchCancellation,
) -> Result<Range<u64>, PagedDocumentError> {
    let line_range = content_line_range(snapshot, line_range, cancel)?;
    if line_range.is_empty() {
        return Ok(line_range.start..line_range.start);
    }
    let hit = hit.clamp(line_range.start, line_range.end);
    if let Some(range) = resolve_ascii_word_range(snapshot, &line_range, hit, cancel)? {
        return Ok(range);
    }
    // unicode_word_indices 没有可恢复的分块游标；非 ASCII 单词为保持 UAX 语义仍会扩展并暂存分段。
    let desired_start = hit
        .saturating_sub(BOUNDARY_CHUNK_BYTES / 2)
        .max(line_range.start);
    let start = align_utf8_start(snapshot, desired_start, line_range.start, cancel)?;
    let (_, mut text) = read_forward_chunk(snapshot, start, line_range.end, cancel)?;
    let mut window_start = start;
    loop {
        if cancel.is_cancelled() {
            return Err(PagedDocumentError::Cancelled);
        }
        let local_hit = usize::try_from(hit.saturating_sub(window_start))
            .unwrap_or(usize::MAX)
            .min(text.len());
        let Some(local) = local_unicode_word_range(&text, local_hit) else {
            let grapheme = local_grapheme_range(&text, local_hit);
            let grapheme_text = &text[grapheme.clone()];
            let is_ignored_context =
                !grapheme_text.is_empty() && grapheme_text.chars().all(is_word_ignored_character);
            if !is_ignored_context {
                return resolve_grapheme_range(snapshot, &line_range, hit, cancel);
            }
            let window_end = window_start + text.len() as u64;
            let left_context_proven = window_start == line_range.start
                || text[..grapheme.start]
                    .chars()
                    .next_back()
                    .is_some_and(is_safe_word_separator);
            let right_context_proven = window_end == line_range.end
                || text[grapheme.end..]
                    .chars()
                    .next()
                    .is_some_and(is_safe_word_separator);
            let expand_left = !left_context_proven;
            let expand_right = !right_context_proven;
            if !expand_left && !expand_right {
                return resolve_grapheme_range(snapshot, &line_range, hit, cancel);
            }
            extend_unicode_word_window(
                snapshot,
                &line_range,
                &mut window_start,
                &mut text,
                expand_left,
                expand_right,
                cancel,
            )?;
            continue;
        };
        let window_end = window_start + text.len() as u64;
        let left_boundary_proven = window_start == line_range.start
            || text[..local.start]
                .chars()
                .next_back()
                .is_some_and(is_safe_word_separator);
        let right_boundary_proven = window_end == line_range.end
            || text[local.end..]
                .chars()
                .next()
                .is_some_and(is_safe_word_separator);
        let expand_left = !left_boundary_proven;
        let expand_right = !right_boundary_proven;
        if !expand_left && !expand_right {
            return Ok(window_start + local.start as u64..window_start + local.end as u64);
        }
        extend_unicode_word_window(
            snapshot,
            &line_range,
            &mut window_start,
            &mut text,
            expand_left,
            expand_right,
            cancel,
        )?;
    }
}

/// 返回可见窗口内可确定的字素/单词移动目标；按词移动还需证明目标左边界。
pub(super) fn bounded_horizontal_target(
    row: &BoundedLineWindow,
    line_range: Range<u64>,
    offset: u64,
    forward: bool,
    by_word: bool,
) -> Option<u64> {
    let local_offset = usize::try_from(offset.saturating_sub(row.content_range.start))
        .unwrap_or(usize::MAX)
        .min(row.text.len());
    let local_target = local_horizontal_target(&row.text, local_offset, forward, by_word);
    if by_word {
        let target_is_word_start = local_target < row.text.len();
        let start_proven = !row.leading_truncated
            || (local_target > 0
                && row.text[..local_target]
                    .chars()
                    .next_back()
                    .is_some_and(is_safe_word_separator));
        if (target_is_word_start && !start_proven)
            || (local_target == row.text.len() && row.trailing_truncated)
        {
            return None;
        }
    }
    if bounded_grapheme_boundary(row, local_target) != Some(true) {
        return None;
    }
    Some((row.content_range.start + local_target as u64).clamp(line_range.start, line_range.end))
}

/// 逐块解析水平移动；字素游标保持有界，Unicode 按词移动最多暂存 64 MiB 上下文。
pub(super) fn resolve_horizontal_target(
    snapshot: &dyn DocumentSnapshot,
    line_range: Range<u64>,
    offset: u64,
    forward: bool,
    by_word: bool,
    cancel: &SearchCancellation,
) -> Result<u64, PagedDocumentError> {
    let line_range = content_line_range(snapshot, line_range, cancel)?;
    if line_range.is_empty() {
        return Ok(line_range.start);
    }
    let offset = offset.clamp(line_range.start, line_range.end);
    if !by_word {
        let query = if forward {
            CursorQuery::Next
        } else {
            CursorQuery::Previous
        };
        return match resolve_grapheme_query(snapshot, &line_range, offset, query, cancel)? {
            CursorResult::Boundary(Some(boundary)) => Ok(line_range.start + boundary as u64),
            CursorResult::Boundary(None) => Ok(if forward {
                line_range.end
            } else {
                line_range.start
            }),
            CursorResult::IsBoundary(_) => Err(PagedDocumentError::InvalidUtf8Boundary),
        };
    }
    if let Some(target) = resolve_ascii_word_move(snapshot, &line_range, offset, forward, cancel)? {
        return Ok(target);
    }
    // 非 ASCII 词移动保留 unicode_word_indices 的完整上下文，回退窗口可能随词或分隔区增长。
    let desired_start = offset
        .saturating_sub(BOUNDARY_CHUNK_BYTES / 2)
        .max(line_range.start);
    let mut window_start = align_utf8_start(snapshot, desired_start, line_range.start, cancel)?;
    let (_, mut text) = read_forward_chunk(snapshot, window_start, line_range.end, cancel)?;
    loop {
        if cancel.is_cancelled() {
            return Err(PagedDocumentError::Cancelled);
        }
        let local_offset = usize::try_from(offset.saturating_sub(window_start))
            .unwrap_or(usize::MAX)
            .min(text.len());
        let target = local_horizontal_target(&text, local_offset, forward, by_word);
        let window_end = window_start + text.len() as u64;
        if by_word && forward && target == text.len() && window_end < line_range.end {
            let (_, suffix) = read_forward_chunk(snapshot, window_end, line_range.end, cancel)?;
            if suffix.is_empty() {
                return Err(PagedDocumentError::Search(
                    "Unicode word movement made no progress while reading forward".into(),
                ));
            }
            check_unicode_window_growth(text.len(), suffix.len())?;
            text.try_reserve_exact(suffix.len())
                .map_err(|_| PagedDocumentError::RangeTooLarge)?;
            text.push_str(&suffix);
        } else if by_word
            && target < text.len()
            && window_start > line_range.start
            && !text[..target]
                .chars()
                .next_back()
                .is_some_and(is_safe_word_separator)
        {
            let (new_start, prefix) =
                read_backward_chunk(snapshot, window_start, line_range.start, cancel)?;
            if prefix.is_empty() || new_start == window_start {
                return Err(PagedDocumentError::Search(
                    "Unicode word movement made no progress while reading backward".into(),
                ));
            }
            check_unicode_window_growth(text.len(), prefix.len())?;
            text.try_reserve_exact(prefix.len())
                .map_err(|_| PagedDocumentError::RangeTooLarge)?;
            text.insert_str(0, &prefix);
            window_start = new_start;
        } else {
            return Ok(window_start + target as u64);
        }
    }
}
