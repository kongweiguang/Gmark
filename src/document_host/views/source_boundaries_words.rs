// @author kongweiguang

//! 可证明符合 UAX 词界的 ASCII 常量内存扫描。

use std::ops::Range;

use gmark_document_core::DocumentSnapshot;
use gmark_paged_document::{PagedDocumentError, SearchCancellation};

use super::{BOUNDARY_CHUNK_BYTES, read_backward_chunk, read_forward_chunk};

/// 稳定 ASCII 单词字节类，用于长标识符的常量内存扫描路径。
fn is_ascii_word_character(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

/// unicode_word_indices 不把只有 ExtendNumLet 下划线的分段视为单词。
fn is_ascii_word_letter_or_digit(ch: char) -> bool {
    ch.is_ascii_alphanumeric()
}

/// 仅接受 Unicode 分词规则中可证明不连接相邻字词的分隔符。
pub(super) fn is_safe_word_separator(ch: char) -> bool {
    ch.is_whitespace()
        || matches!(
            ch,
            '!' | '?'
                | ';'
                | '/'
                | '\\'
                | '('
                | ')'
                | '['
                | ']'
                | '{'
                | '}'
                | '<'
                | '>'
                | '='
                | '@'
                | '#'
                | '$'
                | '%'
                | '^'
                | '&'
                | '*'
                | '+'
                | '-'
                | '~'
                | '|'
        )
}

/// 返回特定源码位置的相邻字符，读取按文档行边界保持在 64 KiB 以内。
fn read_character_at(
    snapshot: &dyn DocumentSnapshot,
    line_range: &Range<u64>,
    offset: u64,
    before: bool,
    cancel: &SearchCancellation,
) -> Result<Option<char>, PagedDocumentError> {
    if before {
        if offset <= line_range.start {
            return Ok(None);
        }
        let (_, text) = read_backward_chunk(snapshot, offset, line_range.start, cancel)?;
        Ok(text.chars().next_back())
    } else {
        if offset >= line_range.end {
            return Ok(None);
        }
        let (_, text) = read_forward_chunk(snapshot, offset, line_range.end, cancel)?;
        Ok(text.chars().next())
    }
}

/// 对两侧均遇到安全分隔符或行端的 ASCII 单词做常量内存双向扫描。
pub(super) fn resolve_ascii_word_range(
    snapshot: &dyn DocumentSnapshot,
    line_range: &Range<u64>,
    hit: u64,
    cancel: &SearchCancellation,
) -> Result<Option<Range<u64>>, PagedDocumentError> {
    let hit = hit.clamp(line_range.start, line_range.end);
    let selected = if hit == line_range.end {
        read_character_at(snapshot, line_range, hit, true, cancel)?
    } else {
        read_character_at(snapshot, line_range, hit, false, cancel)?
    };
    if !selected.is_some_and(is_ascii_word_character) {
        return Ok(None);
    }
    let mut contains_letter_or_digit = selected.is_some_and(is_ascii_word_letter_or_digit);

    let mut start = if hit == line_range.end {
        hit - selected.map_or(0, |ch| ch.len_utf8() as u64)
    } else {
        hit
    };
    loop {
        if cancel.is_cancelled() {
            return Err(PagedDocumentError::Cancelled);
        }
        if start == line_range.start {
            break;
        }
        let (_, text) = read_backward_chunk(snapshot, start, line_range.start, cancel)?;
        if text.is_empty() {
            return Err(PagedDocumentError::Search(
                "ASCII word scan made no progress while reading backward".into(),
            ));
        }
        let mut cursor = start;
        let mut separator_found = false;
        for ch in text.chars().rev() {
            let ch_start = cursor - ch.len_utf8() as u64;
            if is_ascii_word_character(ch) {
                contains_letter_or_digit |= is_ascii_word_letter_or_digit(ch);
                start = ch_start;
                cursor = ch_start;
            } else if is_safe_word_separator(ch) {
                separator_found = true;
                break;
            } else {
                return Ok(None);
            }
        }
        if separator_found || start == line_range.start {
            break;
        }
    }

    let mut end = if hit == line_range.end {
        hit
    } else {
        hit + selected.map_or(0, |ch| ch.len_utf8() as u64)
    };
    loop {
        if cancel.is_cancelled() {
            return Err(PagedDocumentError::Cancelled);
        }
        if end >= line_range.end {
            break;
        }
        let (_, text) = read_forward_chunk(snapshot, end, line_range.end, cancel)?;
        if text.is_empty() {
            return Err(PagedDocumentError::Search(
                "ASCII word scan made no progress while reading forward".into(),
            ));
        }
        let mut separator_found = false;
        for ch in text.chars() {
            if is_ascii_word_character(ch) {
                contains_letter_or_digit |= is_ascii_word_letter_or_digit(ch);
                end += ch.len_utf8() as u64;
            } else if is_safe_word_separator(ch) {
                separator_found = true;
                break;
            } else {
                return Ok(None);
            }
        }
        if separator_found || text.is_empty() {
            break;
        }
    }
    if !contains_letter_or_digit {
        return Ok(None);
    }
    Ok(Some(start..end))
}

/// 为 Ctrl/Option 词移动流式定位纯 ASCII 单词；遇到 UAX 连接字符即退回通用分词。
pub(super) fn resolve_ascii_word_move(
    snapshot: &dyn DocumentSnapshot,
    line_range: &Range<u64>,
    offset: u64,
    forward: bool,
    cancel: &SearchCancellation,
) -> Result<Option<u64>, PagedDocumentError> {
    let offset = offset.clamp(line_range.start, line_range.end);
    if forward {
        let mut cursor = offset;
        let starts_inside_word = read_character_at(snapshot, line_range, cursor, false, cancel)?
            .is_some_and(is_ascii_word_character);
        let mut seeking_next = !starts_inside_word;
        let mut candidate_start = None;
        let mut candidate_has_letter_or_digit = false;
        while cursor < line_range.end {
            if cancel.is_cancelled() {
                return Err(PagedDocumentError::Cancelled);
            }
            let (_, text) = read_forward_chunk(snapshot, cursor, line_range.end, cancel)?;
            if text.is_empty() {
                break;
            }
            for ch in text.chars() {
                if is_ascii_word_character(ch) {
                    if candidate_start.is_none() {
                        candidate_start = Some(cursor);
                    }
                    candidate_has_letter_or_digit |= is_ascii_word_letter_or_digit(ch);
                } else if is_safe_word_separator(ch) {
                    if seeking_next && candidate_has_letter_or_digit {
                        return Ok(candidate_start);
                    }
                    seeking_next = true;
                    candidate_start = None;
                    candidate_has_letter_or_digit = false;
                } else {
                    return Ok(None);
                }
                cursor += ch.len_utf8() as u64;
            }
        }
        Ok(if seeking_next && candidate_has_letter_or_digit {
            candidate_start
        } else {
            Some(line_range.end)
        })
    } else {
        let mut cursor = offset;
        let mut candidate_start = None;
        let mut candidate_has_letter_or_digit = false;
        while cursor > line_range.start {
            if cancel.is_cancelled() {
                return Err(PagedDocumentError::Cancelled);
            }
            let (chunk_start, text) =
                read_backward_chunk(snapshot, cursor, line_range.start, cancel)?;
            if text.is_empty() {
                break;
            }
            for ch in text.chars().rev() {
                let ch_start = cursor - ch.len_utf8() as u64;
                if is_ascii_word_character(ch) {
                    candidate_start = Some(ch_start);
                    candidate_has_letter_or_digit |= is_ascii_word_letter_or_digit(ch);
                } else if is_safe_word_separator(ch) {
                    if candidate_has_letter_or_digit {
                        return Ok(candidate_start);
                    }
                    candidate_start = None;
                    candidate_has_letter_or_digit = false;
                } else {
                    return Ok(None);
                }
                cursor = ch_start;
            }
            if cursor == chunk_start {
                continue;
            }
        }
        Ok(if candidate_has_letter_or_digit {
            candidate_start
        } else {
            Some(line_range.start)
        })
    }
}
