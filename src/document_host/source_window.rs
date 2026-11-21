// @author kongweiguang

//! Bounded source-window decoding and scroll-coordinate helpers.

use super::*;

/// 全文索引尚未完成时按估算行号映射到字节锚点。每一行最多两次 64 KiB 读取，
/// 因而首屏、滚动条拖动和关闭窗口都不依赖 O(file_size) 扫描。
pub(super) fn read_provisional_source_rows(
    source: &FileSource,
    estimated_lines: u64,
    requested: Range<usize>,
    column_start: u64,
    encoding: &TextEncoding,
    cancellation: &SearchCancellation,
) -> Result<Vec<(usize, BoundedLineWindow)>, gmark_paged_document::PagedDocumentError> {
    let len = source.identity()?.len;
    if len == 0 {
        return Ok(vec![(
            requested.start,
            BoundedLineWindow::new(0..0, 0..0, String::new(), String::new(), false, false),
        )]);
    }
    requested
        .map(|logical_line| {
            if cancellation.is_cancelled() {
                return Err(PagedDocumentError::Cancelled);
            }
            let target = ((len as u128 * logical_line as u128) / estimated_lines.max(1) as u128)
                .min(len.saturating_sub(1) as u128) as u64;
            read_provisional_line_window(source, target, column_start, encoding)
                .map(|window| (logical_line, window))
        })
        .collect()
}

fn read_provisional_line_window(
    source: &FileSource,
    mut target: u64,
    column_start: u64,
    encoding: &TextEncoding,
) -> Result<BoundedLineWindow, gmark_paged_document::PagedDocumentError> {
    let len = source.identity()?.len;
    let utf16 = matches!(encoding, TextEncoding::Utf16Le | TextEncoding::Utf16Be);
    if utf16 {
        target -= target % 2;
    }
    let mut backward_start = target.saturating_sub(MAX_RENDERED_LINE_BYTES);
    if utf16 {
        backward_start -= backward_start % 2;
    }
    let backward = source.read_range(backward_start, target)?;
    let known_line_start = last_line_break_end(&backward, backward_start, encoding);
    let physical_start = known_line_start.unwrap_or(target);
    let aligned_column = if utf16 {
        column_start - column_start % 2
    } else {
        column_start
    };
    let mut start = physical_start.saturating_add(aligned_column).min(len);
    if start < len && matches!(encoding, TextEncoding::Utf8 { .. }) {
        let probe = source.read_range(start, (start + 4).min(len))?;
        start = start.saturating_add(
            probe
                .iter()
                .take_while(|byte| **byte & 0b1100_0000 == 0b1000_0000)
                .count() as u64,
        );
    }
    let read_end = start.saturating_add(MAX_RENDERED_LINE_BYTES).min(len);
    let mut bytes = source.read_range(start, read_end)?;
    let newline_end = first_line_break_end(&bytes, start, encoding);
    if let Some(newline_end) = newline_end {
        bytes.truncate(newline_end);
    }
    let source_end = start.saturating_add(bytes.len() as u64);
    let ending_len = encoded_line_ending_len(&bytes, encoding);
    let content_end = source_end.saturating_sub(ending_len as u64);
    let content_bytes = &bytes[..bytes.len().saturating_sub(ending_len)];
    let text = decode_provisional_bytes(content_bytes, encoding, start);
    let content_range = start..content_end;
    Ok(BoundedLineWindow::new(
        content_range,
        physical_start..source_end,
        text,
        decoded_line_ending(ending_len, utf16),
        known_line_start.is_none() && target > 0 || start > physical_start,
        newline_end.is_none() && source_end < len,
    ))
}

fn last_line_break_end(bytes: &[u8], absolute_start: u64, encoding: &TextEncoding) -> Option<u64> {
    match encoding {
        TextEncoding::Utf16Le => bytes
            .chunks_exact(2)
            .enumerate()
            .filter(|(_, pair)| u16::from_le_bytes([pair[0], pair[1]]) == b'\n' as u16)
            .map(|(index, _)| absolute_start + (index as u64 + 1) * 2)
            .next_back(),
        TextEncoding::Utf16Be => bytes
            .chunks_exact(2)
            .enumerate()
            .filter(|(_, pair)| u16::from_be_bytes([pair[0], pair[1]]) == b'\n' as u16)
            .map(|(index, _)| absolute_start + (index as u64 + 1) * 2)
            .next_back(),
        _ => bytes
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map(|position| absolute_start + position as u64 + 1),
    }
}

fn first_line_break_end(
    bytes: &[u8],
    _absolute_start: u64,
    encoding: &TextEncoding,
) -> Option<usize> {
    match encoding {
        TextEncoding::Utf16Le => bytes
            .chunks_exact(2)
            .position(|pair| u16::from_le_bytes([pair[0], pair[1]]) == b'\n' as u16)
            .map(|index| (index + 1) * 2),
        TextEncoding::Utf16Be => bytes
            .chunks_exact(2)
            .position(|pair| u16::from_be_bytes([pair[0], pair[1]]) == b'\n' as u16)
            .map(|index| (index + 1) * 2),
        _ => bytes
            .iter()
            .position(|byte| *byte == b'\n')
            .map(|index| index + 1),
    }
}

fn encoded_line_ending_len(bytes: &[u8], encoding: &TextEncoding) -> usize {
    match encoding {
        TextEncoding::Utf16Le if bytes.ends_with(&[b'\r', 0, b'\n', 0]) => 4,
        TextEncoding::Utf16Be if bytes.ends_with(&[0, b'\r', 0, b'\n']) => 4,
        TextEncoding::Utf16Le if bytes.ends_with(&[b'\n', 0]) => 2,
        TextEncoding::Utf16Be if bytes.ends_with(&[0, b'\n']) => 2,
        _ if bytes.ends_with(b"\r\n") => 2,
        _ if bytes.ends_with(b"\n") || bytes.ends_with(b"\r") => 1,
        _ => 0,
    }
}

fn decoded_line_ending(ending_len: usize, utf16: bool) -> String {
    match (ending_len, utf16) {
        (4, true) | (2, false) => "\r\n".to_owned(),
        (2, true) | (1, false) => "\n".to_owned(),
        _ => String::new(),
    }
}

pub(super) fn decode_provisional_bytes(
    bytes: &[u8],
    encoding: &TextEncoding,
    absolute_start: u64,
) -> String {
    match encoding {
        TextEncoding::Utf8 { bom } => {
            let bytes = if *bom && absolute_start == 0 {
                bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes)
            } else {
                bytes
            };
            String::from_utf8_lossy(bytes).into_owned()
        }
        TextEncoding::Utf16Le | TextEncoding::Utf16Be => {
            let bytes = if absolute_start == 0 {
                bytes
                    .strip_prefix(&[0xff, 0xfe])
                    .or_else(|| bytes.strip_prefix(&[0xfe, 0xff]))
                    .unwrap_or(bytes)
            } else {
                bytes
            };
            let units = bytes.chunks_exact(2).map(|pair| match encoding {
                TextEncoding::Utf16Le => u16::from_le_bytes([pair[0], pair[1]]),
                TextEncoding::Utf16Be => u16::from_be_bytes([pair[0], pair[1]]),
                _ => unreachable!(),
            });
            String::from_utf16_lossy(&units.collect::<Vec<_>>())
        }
        TextEncoding::Legacy(label) => encoding_rs::Encoding::for_label(label.as_bytes())
            .map(|encoding| encoding.decode(bytes).0.into_owned())
            .unwrap_or_else(|| String::from_utf8_lossy(bytes).into_owned()),
    }
}

pub(super) trait BoundedSourceDocument {
    fn line_range(&self, line: u64) -> Option<Range<u64>>;
    fn read_range(
        &self,
        range: Range<u64>,
    ) -> Result<Vec<u8>, gmark_paged_document::PagedDocumentError>;
}

impl BoundedSourceDocument for SharedDocument {
    fn line_range(&self, line: u64) -> Option<Range<u64>> {
        SharedDocument::line_range(self, line)
    }

    fn read_range(
        &self,
        range: Range<u64>,
    ) -> Result<Vec<u8>, gmark_paged_document::PagedDocumentError> {
        SharedDocument::read_range(self, range)
    }
}

impl BoundedSourceDocument for DocumentSession {
    fn line_range(&self, line: u64) -> Option<Range<u64>> {
        DocumentSession::line_range(self, line)
    }

    fn read_range(
        &self,
        range: Range<u64>,
    ) -> Result<Vec<u8>, gmark_paged_document::PagedDocumentError> {
        DocumentSession::read_range(self, range)
    }
}

pub(super) fn read_bounded_line_window(
    document: &impl BoundedSourceDocument,
    line: u64,
    requested_start: u64,
) -> Result<Option<BoundedLineWindow>, gmark_paged_document::PagedDocumentError> {
    let Some(line_range) = document.line_range(line) else {
        return Ok(None);
    };
    let tail_start = line_range.end.saturating_sub(2).max(line_range.start);
    let tail = document.read_range(tail_start..line_range.end)?;
    let ending_len = if tail.ends_with(b"\r\n") {
        2u64
    } else if tail.ends_with(b"\n") || tail.ends_with(b"\r") {
        1
    } else {
        0
    };
    let content_end = line_range.end.saturating_sub(ending_len);
    let content_len = content_end.saturating_sub(line_range.start);
    let relative_start = requested_start.min(content_len.saturating_sub(MAX_RENDERED_LINE_BYTES));
    let mut start = line_range.start.saturating_add(relative_start);
    if start > line_range.start && start < content_end {
        // 横向窗口可能落在多字节码点内部；最多向前跳过三个 continuation byte。
        let probe_end = (start + 4).min(content_end);
        let probe = document.read_range(start..probe_end)?;
        let skipped = probe
            .iter()
            .take_while(|byte| **byte & 0b1100_0000 == 0b1000_0000)
            .count() as u64;
        start = start.saturating_add(skipped);
    }
    let requested_end = (start + MAX_RENDERED_LINE_BYTES).min(content_end);
    let mut bytes = document.read_range(start..requested_end)?;
    let mut end = requested_end;
    if let Err(error) = std::str::from_utf8(&bytes)
        && error.error_len().is_none()
    {
        bytes.truncate(error.valid_up_to());
        end = start.saturating_add(bytes.len() as u64);
    }
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let ending = if end == content_end && ending_len > 0 {
        String::from_utf8_lossy(&tail[tail.len() - ending_len as usize..]).into_owned()
    } else {
        String::new()
    };
    let replace_end = if end == content_end {
        line_range.end
    } else {
        end
    };
    Ok(Some(BoundedLineWindow::new(
        start..end,
        start..replace_end,
        text,
        ending,
        start > line_range.start,
        end < content_end,
    )))
}

pub(super) fn rendered_line_ending(ending: &str) -> &'static str {
    match ending {
        "\r\n" => "␍␊",
        "\n" => "␊",
        "\r" => "␍",
        _ => "",
    }
}

pub(super) fn text_encoding_label(encoding: &TextEncoding) -> String {
    match encoding {
        TextEncoding::Utf8 { bom: false } => "UTF-8".to_owned(),
        TextEncoding::Utf8 { bom: true } => "UTF-8 BOM".to_owned(),
        TextEncoding::Utf16Le => "UTF-16 LE".to_owned(),
        TextEncoding::Utf16Be => "UTF-16 BE".to_owned(),
        TextEncoding::Legacy(label) => label.to_uppercase(),
    }
}

pub(super) fn rendered_line_window_text(
    window: &BoundedLineWindow,
    show_line_endings: bool,
) -> String {
    let mut text = String::with_capacity(window.text.len().saturating_add(6));
    if window.leading_truncated {
        text.push_str("… ");
    }
    text.push_str(&window.text);
    if window.trailing_truncated {
        text.push_str(" …");
    } else if show_line_endings {
        text.push_str(rendered_line_ending(&window.ending));
    }
    text
}

pub(super) fn shift_source_window_start(current: u64, delta: i64, maximum: u64) -> u64 {
    if delta >= 0 {
        current.saturating_add(delta as u64).min(maximum)
    } else {
        current.saturating_sub(delta.unsigned_abs())
    }
}

pub(super) fn source_window_start_for_anchor(line_len: u64, relative_byte: u64) -> u64 {
    relative_byte
        .min(line_len)
        .saturating_sub(MAX_RENDERED_LINE_BYTES / 4)
        .min(line_len.saturating_sub(MAX_RENDERED_LINE_BYTES))
}

/// 窄窗格也要保留可拖动行程：常规宽度按可见窗口占比缩放，空间不足时至少留半个轨道。
///
/// 源码长行只用 64 KiB 窗口虚拟滚动，滑块过短会让极窄分屏的拖动范围退化为零；
/// 因此只在正常宽度下维持 28px 最小视觉尺寸，窄轨道按比例缩小，避免点击滚动条把
/// 当前长行窗口重置到首段。
pub(super) fn source_horizontal_thumb_width(
    track_width: f32,
    visible_bytes: u64,
    total_bytes: u64,
) -> f32 {
    if !track_width.is_finite() || track_width <= 0.0 {
        return 0.0;
    }
    if total_bytes <= visible_bytes {
        return track_width;
    }
    let minimum = 28.0_f32.min(track_width * 0.5);
    let proportional = track_width * visible_bytes as f32 / total_bytes.max(1) as f32;
    proportional.clamp(minimum, track_width)
}

pub(super) fn source_window_start_from_pointer(
    pointer_x: gpui::Pixels,
    track_left: gpui::Pixels,
    track_width: f32,
    thumb_width: f32,
    maximum: u64,
    current: u64,
) -> u64 {
    if maximum == 0 {
        return 0;
    }
    let travel = (track_width - thumb_width).max(0.0);
    if !travel.is_finite() || travel <= 0.0 {
        return current.min(maximum);
    }
    let thumb_left = (f32::from(pointer_x - track_left) - thumb_width * 0.5).clamp(0.0, travel);
    ((thumb_left / travel) as f64 * maximum as f64).round() as u64
}

pub(super) fn source_list_origin_for_target(total: usize, target: usize) -> usize {
    if total <= SOURCE_LIST_WINDOW_ROWS {
        return 0;
    }
    target
        .saturating_sub(SOURCE_LIST_WINDOW_ROWS / 2)
        .min(total - SOURCE_LIST_WINDOW_ROWS)
}

pub(super) fn source_line_from_scrollbar_pointer(
    pointer_y: gpui::Pixels,
    track_top: gpui::Pixels,
    track_height: f32,
    thumb_height: f32,
    max_top_line: usize,
) -> usize {
    let travel = (track_height - thumb_height).max(0.0);
    let thumb_top = (f32::from(pointer_y - track_top) - thumb_height * 0.5).clamp(0.0, travel);
    let progress = if travel > 0.0 {
        thumb_top / travel
    } else {
        0.0
    };
    (progress as f64 * max_top_line as f64).round() as usize
}
