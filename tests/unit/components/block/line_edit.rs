// @author kongweiguang

use super::{LineOperation, plan_line_operation};

/// Confirms multiline indentation keeps CRLF bytes and shifts the selection with its text.
#[test]
fn indent_preserves_crlf_and_selection_offsets() {
    let edit = plan_line_operation("one\r\ntwo\r\nthree", 6..7, false, LineOperation::Indent)
        .expect("selected line should be indented");

    assert_eq!(edit.range, 5..10);
    assert_eq!(edit.replacement, "    two\r\n");
    assert_eq!(edit.selection, 10..11);
}

/// Confirms duplication of a final unterminated line uses the document's existing CRLF convention.
#[test]
fn duplicate_final_line_preserves_crlf_convention() {
    let edit = plan_line_operation("one\r\ntwo", 6..6, false, LineOperation::Duplicate)
        .expect("caret line should be duplicated");

    assert_eq!(edit.replacement, "two\r\ntwo");
    assert_eq!(edit.selection, 10..13);
}

/// Confirms moving into a trailing blank line changes content order without normalizing line endings.
#[test]
fn move_down_swaps_with_trailing_blank_line() {
    let edit = plan_line_operation("a\nb\n", 3..3, false, LineOperation::MoveDown)
        .expect("line before trailing blank should move down");

    assert_eq!(edit.range, 2..4);
    assert_eq!(edit.replacement, "\nb");
    assert_eq!(edit.selection, 3..4);
}

/// Confirms a moved selection keeps its source direction after offsets are remapped.
#[test]
fn moving_reversed_selection_preserves_direction() {
    let edit = plan_line_operation("first\nsecond\nthird", 6..12, true, LineOperation::MoveDown)
        .expect("selected line should move down");

    assert_eq!(edit.replacement, "third\nsecond");
    assert_eq!(edit.selection, 12..18);
    assert!(edit.reversed);
}

/// Confirms deleting a final unterminated row also removes its preceding separator.
#[test]
fn delete_final_line_removes_preceding_separator() {
    let edit = plan_line_operation("a\nb", 2..2, false, LineOperation::Delete)
        .expect("current final line should be deleted");

    assert_eq!(edit.range, 1..3);
    assert!(edit.replacement.is_empty());
    assert_eq!(edit.selection, 1..1);
}

/// Rejects byte ranges that split a UTF-8 code point before any string slicing occurs.
#[test]
fn invalid_utf8_range_does_not_produce_an_edit() {
    assert!(plan_line_operation("汉字", 1..2, false, LineOperation::Duplicate).is_none());
}
