// @author kongweiguang

use super::{centered_column_ratio, centered_column_width};
use crate::ui::text_editing::{
    grapheme_range_at, logical_line_range_at, next_grapheme_boundary, previous_grapheme_boundary,
    word_range_at,
};
use crate::ui::theme::Theme;

/// Keep platform byte offsets inside complete user-perceived Unicode characters.
#[test]
fn text_selection_helpers_keep_extended_graphemes_intact() {
    let text = "A👨‍👩‍👧‍👦e\u{301}🇨🇳Z";
    let family_start = 1;
    let family_end = family_start + "👨‍👩‍👧‍👦".len();
    let accent_start = family_end;
    let accent_end = accent_start + "e\u{301}".len();
    let flag_start = accent_end;
    let flag_end = flag_start + "🇨🇳".len();

    assert_eq!(
        grapheme_range_at(text, family_start + 2),
        family_start..family_end
    );
    assert_eq!(
        grapheme_range_at(text, accent_start + 1),
        accent_start..accent_end
    );
    assert_eq!(
        grapheme_range_at(text, flag_start + 1),
        flag_start..flag_end
    );
    assert_eq!(previous_grapheme_boundary(text, family_end), family_start);
    assert_eq!(next_grapheme_boundary(text, family_start + 2), family_end);
}

/// Use default Unicode boundaries for CJK and whole graphemes for emoji and punctuation.
#[test]
fn word_selection_uses_unicode_words_and_grapheme_fallbacks() {
    let text = "word_2 中文, 👨‍👩‍👧‍👦!";
    let word_start = 0;
    let word_end = "word_2".len();
    let chinese_start = text.find("中文").expect("Chinese sample should exist");
    let emoji_start = text.find("👨‍👩‍👧‍👦").expect("emoji sample should exist");
    let emoji_end = emoji_start + "👨‍👩‍👧‍👦".len();

    assert_eq!(word_range_at(text, 2), word_start..word_end);
    assert_eq!(
        word_range_at(text, chinese_start + 1),
        chinese_start..chinese_start + "中".len()
    );
    assert_eq!(word_range_at(text, emoji_start + 2), emoji_start..emoji_end);
}

/// Keep line selection bounded to content even when the source uses CRLF separators.
#[test]
fn logical_line_selection_excludes_crlf_and_handles_the_final_line() {
    let text = "first\r\n第二\r\nthird";
    let second_start = "first\r\n".len();
    let second_end = second_start + "第二".len();

    assert_eq!(logical_line_range_at(text, 5), 0..5);
    assert_eq!(
        logical_line_range_at(text, second_start + 1),
        second_start..second_end
    );
    assert_eq!(
        logical_line_range_at(text, text.len()),
        second_end + 2..text.len()
    );
}

/// Centered content width remains constrained by the active theme's viewport bounds.
#[test]
fn centered_columns_keep_the_existing_viewport_bounds() {
    let dimensions = Theme::xcode_dark().dimensions;

    assert_eq!(
        centered_column_ratio(dimensions.centered_shrink_start, &dimensions),
        1.0
    );
    assert_eq!(
        centered_column_ratio(dimensions.centered_shrink_end, &dimensions),
        dimensions.centered_min_ratio
    );
    assert_eq!(centered_column_width(100.0, &dimensions), 52.0);
}
