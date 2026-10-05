// @author kongweiguang

use super::source_boundaries::{resolve_horizontal_target, resolve_word_range};
use gmark_document_core::{DocumentRevision, DocumentSnapshot, SnapshotError};
use gmark_paged_document::SearchCancellation;
use std::ops::Range;
use std::sync::atomic::{AtomicU64, Ordering};

struct BoundarySnapshot {
    text: String,
    max_read: AtomicU64,
}

impl DocumentSnapshot for BoundarySnapshot {
    /// 固定快照身份，测试只比较公开的边界结果和读取预算。
    fn revision(&self) -> DocumentRevision {
        DocumentRevision(0)
    }
    /// 用 UTF-8 字节长度模拟磁盘快照，不让字符数量掩盖偏移问题。
    fn len(&self) -> u64 {
        self.text.len() as u64
    }
    /// 不要求读取端点对齐字符；真实快照返回字节，由解析器负责跨块 UTF-8。
    fn read_range(&self, range: Range<u64>) -> Result<Vec<u8>, SnapshotError> {
        self.max_read
            .fetch_max(range.end.saturating_sub(range.start), Ordering::Relaxed);
        let bytes = self
            .text
            .as_bytes()
            .get(range.start as usize..range.end as usize)
            .ok_or(SnapshotError::InvalidRange {
                start: range.start,
                end: range.end,
                len: self.len(),
            })?;
        Ok(bytes.to_vec())
    }
}

/// 以完整文本规则校验分页词界、超长 Extend 上下文、字素和 CRLF 行尾。
#[test]
fn paged_source_streamed_boundaries_match_complete_unicode_rules() {
    let prefix = "prior\r\n";
    let family = "👨‍👩‍👧‍👦";
    let cases = [
        "alpha_beta 123 ___ tail".into(),
        "can't 123,456 אב\"גד カタカナ 中文 e\u{301} 🙂🇨🇳".into(),
        format!("a {family} z"),
        format!("{} next", "x".repeat(180_000)),
        format!("a{}b tail", "\u{301}".repeat(40_000)),
        format!("a{}b tail", "\u{200e}".repeat(40_000)),
        format!("{}'{} tail", "é".repeat(40_000), "à".repeat(10)),
        "🇦".repeat(40_001),
    ];
    for content in cases {
        let snapshot = BoundarySnapshot {
            text: format!("{prefix}{content}\r\n"),
            max_read: AtomicU64::new(0),
        };
        let start = prefix.len() as u64;
        let end = start + content.len() as u64;
        let line_range = start..end + 2;
        let mut offsets = if content.len() < 300 {
            content
                .char_indices()
                .map(|(index, _)| index)
                .collect::<Vec<_>>()
        } else {
            let center = (0..=content.len() / 2)
                .rev()
                .find(|offset| content.is_char_boundary(*offset))
                .unwrap_or_default();
            [
                0,
                center,
                32_768,
                65_536,
                110_852,
                content.len().saturating_sub(4),
                content.len(),
            ]
            .into_iter()
            .filter(|offset| *offset <= content.len() && content.is_char_boundary(*offset))
            .collect()
        };
        offsets.push(content.len());
        for offset in offsets {
            let word = crate::ui::text_editing::word_range_at(&content, offset);
            let selected = resolve_word_range(
                &snapshot,
                line_range.clone(),
                start + offset as u64,
                &SearchCancellation::default(),
            )
            .expect("stream word boundary");
            assert_eq!(
                selected,
                start + word.start as u64..start + word.end as u64,
                "word at {offset} in {} bytes",
                content.len()
            );
            for forward in [false, true] {
                for by_word in [false, true] {
                    let expected = match (forward, by_word) {
                        (true, true) => {
                            crate::ui::text_editing::next_word_boundary(&content, offset)
                        }
                        (false, true) => {
                            crate::ui::text_editing::previous_word_boundary(&content, offset)
                        }
                        (true, false) => {
                            crate::ui::text_editing::next_grapheme_boundary(&content, offset)
                        }
                        (false, false) => {
                            crate::ui::text_editing::previous_grapheme_boundary(&content, offset)
                        }
                    };
                    let target = resolve_horizontal_target(
                        &snapshot,
                        line_range.clone(),
                        start + offset as u64,
                        forward,
                        by_word,
                        &SearchCancellation::default(),
                    )
                    .expect("stream horizontal boundary");
                    assert_eq!(
                        target,
                        start + expected as u64,
                        "move at {offset}, forward={forward}, word={by_word}, len={}",
                        content.len()
                    );
                }
            }
        }
        assert!(
            snapshot.max_read.load(Ordering::Relaxed) <= 64 * 1024,
            "no whole-line snapshot allocation"
        );
    }
}
