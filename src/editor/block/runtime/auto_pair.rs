// @author kongweiguang

//! Typora-style bracket, quote, and common Markdown delimiter pairing.

use std::ops::Range;

use gpui::Context;

use super::{Block, CollapsedCaretAffinity};
use crate::components::UndoCaptureKind;

#[derive(Clone, Debug, PartialEq, Eq)]
enum AutoPairEdit {
    Replace {
        range: Range<usize>,
        text: String,
        selected_range_relative: Range<usize>,
    },
    MoveTo(usize),
}

impl Block {
    /// Applies one plain keyboard insertion as an atomic pair edit when a rule matches.
    pub(crate) fn try_apply_auto_pair_input(
        &mut self,
        visible_range: Range<usize>,
        input: &str,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.is_read_only()
            || self.marked_range.is_some()
            || self.editor_selection_range.is_some()
            || visible_range != self.selected_range
        {
            return false;
        }
        let normal = crate::config::EditorSettings::auto_pair_brackets(cx);
        let markdown = crate::config::EditorSettings::auto_pair_markdown(cx);
        let Some(edit) =
            auto_pair_edit(self.display_text(), visible_range, input, normal, markdown)
        else {
            return false;
        };

        match edit {
            AutoPairEdit::Replace {
                range,
                text,
                selected_range_relative,
            } => {
                // 配对字符与选择包裹必须共用一个历史边界，后续正文输入才能独立撤销。
                self.prepare_undo_capture(UndoCaptureKind::NonCoalescible, cx);
                self.replace_text_in_visible_range(
                    range,
                    &text,
                    Some(selected_range_relative),
                    false,
                    cx,
                );
            }
            AutoPairEdit::MoveTo(offset) => {
                self.assign_collapsed_selection_offset(
                    offset,
                    CollapsedCaretAffinity::Default,
                    None,
                );
                cx.notify();
            }
        }
        true
    }

    /// Deletes both sides only while the caret is inside an empty generated pair.
    pub(crate) fn try_delete_empty_auto_pair(&mut self, cx: &mut Context<Self>) -> bool {
        if self.is_read_only()
            || !self.selected_range.is_empty()
            || self.marked_range.is_some()
            || self.editor_selection_range.is_some()
        {
            return false;
        }
        let normal = crate::config::EditorSettings::auto_pair_brackets(cx);
        let markdown = crate::config::EditorSettings::auto_pair_markdown(cx);
        let Some(range) =
            empty_pair_range(self.display_text(), self.cursor_offset(), normal, markdown)
        else {
            return false;
        };
        self.prepare_undo_capture(UndoCaptureKind::NonCoalescible, cx);
        self.replace_text_in_visible_range(range, "", Some(0..0), false, cx);
        true
    }
}

/// 反引号按字面输入，避免自动闭合和跳过规则干扰手写代码及任意长度围栏。
fn auto_pair_edit(
    source: &str,
    selection: Range<usize>,
    input: &str,
    normal: bool,
    markdown: bool,
) -> Option<AutoPairEdit> {
    if selection.end > source.len()
        || !source.is_char_boundary(selection.start)
        || !source.is_char_boundary(selection.end)
    {
        return None;
    }

    // At an empty leading `**` pair, Space means a bullet shortcut, not italic text.
    if markdown && input == " " && source == "**" && selection == (1..1) {
        return Some(AutoPairEdit::Replace {
            range: 0..2,
            text: "* ".to_owned(),
            selected_range_relative: 2..2,
        });
    }

    let selected = &source[selection.clone()];
    let collapsed = selection.is_empty();
    let next = source[selection.end..].chars().next();
    let previous = source[..selection.start].chars().next_back();

    if normal {
        if collapsed
            && let Some(close) = closing_normal_marker(input)
            && next == Some(close)
        {
            return Some(AutoPairEdit::MoveTo(selection.end + close.len_utf8()));
        }
        if let Some(close) = opening_normal_marker(input) {
            if matches!(input, "'" | "\"")
                && collapsed
                && previous.is_some_and(char::is_alphanumeric)
            {
                return None;
            }
            return Some(wrap_selection(
                selection,
                input,
                &close.to_string(),
                selected,
            ));
        }
    }

    if !markdown {
        return None;
    }

    if collapsed && markdown_closing_marker(input).is_some_and(|marker| next == Some(marker)) {
        if matches!(input, "*" | "_") && previous == next && selection.start > 0 {
            let marker = input.as_bytes()[0] as char;
            let start = selection.start - marker.len_utf8();
            let end = selection.end + marker.len_utf8();
            return Some(AutoPairEdit::Replace {
                range: start..end,
                text: marker.to_string().repeat(4),
                selected_range_relative: 2..2,
            });
        }
        return Some(AutoPairEdit::MoveTo(selection.end + input.len()));
    }

    match input {
        "*" | "_" | "$" => Some(wrap_selection(selection, input, input, selected)),
        "~" if !collapsed => Some(wrap_selection(selection, "~~", "~~", selected)),
        "^" if !collapsed => Some(wrap_selection(selection, "^", "^", selected)),
        _ => None,
    }
}

fn wrap_selection(range: Range<usize>, open: &str, close: &str, selected: &str) -> AutoPairEdit {
    let mut text = String::with_capacity(open.len() + selected.len() + close.len());
    text.push_str(open);
    text.push_str(selected);
    text.push_str(close);
    AutoPairEdit::Replace {
        range,
        text,
        selected_range_relative: open.len()..open.len() + selected.len(),
    }
}

fn opening_normal_marker(input: &str) -> Option<char> {
    match input {
        "(" => Some(')'),
        "[" => Some(']'),
        "{" => Some('}'),
        "\"" => Some('"'),
        "'" => Some('\''),
        _ => None,
    }
}

fn closing_normal_marker(input: &str) -> Option<char> {
    match input {
        ")" => Some(')'),
        "]" => Some(']'),
        "}" => Some('}'),
        "\"" => Some('"'),
        "'" => Some('\''),
        _ => None,
    }
}

/// 仅自动配对的标记允许跳过闭合符；手写反引号不得吞掉按键。
fn markdown_closing_marker(input: &str) -> Option<char> {
    match input {
        "*" => Some('*'),
        "_" => Some('_'),
        "$" => Some('$'),
        _ => None,
    }
}

/// 手写反引号逐个删除，不再被当作自动生成的空字符对。
fn empty_pair_range(
    source: &str,
    cursor: usize,
    normal: bool,
    markdown: bool,
) -> Option<Range<usize>> {
    if cursor == 0 || cursor >= source.len() || !source.is_char_boundary(cursor) {
        return None;
    }
    let previous = source[..cursor].chars().next_back()?;
    let next = source[cursor..].chars().next()?;
    let matches_normal = normal
        && matches!(
            (previous, next),
            ('(', ')') | ('[', ']') | ('{', '}') | ('"', '"') | ('\'', '\'')
        );
    let matches_markdown =
        markdown && previous == next && matches!(previous, '*' | '_' | '$' | '~' | '^');
    (matches_normal || matches_markdown)
        .then_some(cursor - previous.len_utf8()..cursor + next.len_utf8())
}

#[cfg(test)]
#[path = "../../../../tests/unit/components/block/runtime/auto_pair.rs"]
mod tests;
