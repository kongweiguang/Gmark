// @author kongweiguang

use crate::ui::text_editing::{grapheme_range_at, logical_line_range_at, word_range_at};

#[test]
/// Keeps Source word selection aligned with the shared Unicode boundary rules.
fn source_word_selection_keeps_unicode_and_emoji_boundaries() {
    assert_eq!(word_range_at("alpha 世界 🙂", 8), 6..9);
    assert_eq!(word_range_at("alpha 世界 🙂", 13), 13..17);
}

#[test]
/// Ensures visible Source selection never bisects an emoji cluster or CRLF line.
fn source_selection_uses_full_emoji_graphemes_and_crlf_lines() {
    let text = "lead 👨‍👩‍👧‍👦 end\r\nnext";
    let family_start = text.find('👨').expect("family emoji should be present");
    assert_eq!(
        grapheme_range_at(text, family_start + 1),
        family_start..family_start + "👨‍👩‍👧‍👦".len()
    );
    assert_eq!(
        logical_line_range_at(text, family_start),
        0.."lead 👨‍👩‍👧‍👦 end".len()
    );
}
