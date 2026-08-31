// @author kongweiguang

use super::*;
use unicode_categories::UnicodeCategories;

/// Whether `delimiter` requires a non-empty body. Emphasis and strikethrough
/// markers must enclose at least one character; code spans may be empty and
/// script markers already constrain their bodies elsewhere.
pub(super) fn emphasis_requires_body(delimiter: Delimiter) -> bool {
    matches!(
        delimiter,
        Delimiter::BoldMarkdown { .. }
            | Delimiter::ItalicMarkdown { .. }
            | Delimiter::StrikethroughMarkdown
            | Delimiter::HighlightMarkdown
            | Delimiter::BoldHtml
            | Delimiter::ItalicHtml
            | Delimiter::Underline
    )
}

pub(super) fn locate_script_close(
    tokens: &[CharToken],
    mut cursor: usize,
    marker: char,
) -> Option<usize> {
    let body_start = cursor;
    while cursor < tokens.len() {
        if tokens[cursor].ch == '\\'
            && let Some(escaped_len) = escaped_sequence_token_len(tokens, cursor)
        {
            cursor += 1 + escaped_len;
            continue;
        }

        let is_close = if marker == '~' {
            is_single_tilde_delimiter(tokens, cursor)
        } else {
            tokens[cursor].ch == marker
        };
        if is_close {
            return valid_script_body(tokens, body_start, cursor).then_some(cursor);
        }

        cursor += 1;
    }

    None
}

pub(super) fn valid_script_body(tokens: &[CharToken], start: usize, end: usize) -> bool {
    start < end
        && tokens[start..end]
            .iter()
            .all(|token| token.ch.is_ascii_alphanumeric())
}

pub(super) fn is_single_tilde_delimiter(tokens: &[CharToken], index: usize) -> bool {
    tokens.get(index).is_some_and(|token| token.ch == '~')
        && index
            .checked_sub(1)
            .and_then(|prev| tokens.get(prev))
            .is_none_or(|token| token.ch != '~')
        && tokens.get(index + 1).is_none_or(|token| token.ch != '~')
}

pub(super) fn is_double_equals_delimiter(tokens: &[CharToken], index: usize) -> bool {
    matches_sequence(tokens, index, "==")
        && index
            .checked_sub(1)
            .and_then(|previous| tokens.get(previous))
            .is_none_or(|token| token.ch != '=')
        && tokens.get(index + 2).is_none_or(|token| token.ch != '=')
}

pub(super) fn matches_sequence(tokens: &[CharToken], index: usize, sequence: &str) -> bool {
    sequence
        .chars()
        .enumerate()
        .all(|(offset, ch)| tokens.get(index + offset).is_some_and(|t| t.ch == ch))
}

pub(super) fn escaped_sequence_token_len(tokens: &[CharToken], index: usize) -> Option<usize> {
    let next_index = index + 1;
    if next_index >= tokens.len() {
        return None;
    }

    if matches_sequence(tokens, next_index, "</strong>") {
        Some(9)
    } else if matches_sequence(tokens, next_index, "<strong>") {
        Some(8)
    } else if matches_sequence(tokens, next_index, "</em>") {
        Some(5)
    } else if matches_sequence(tokens, next_index, "<em>") {
        Some(4)
    } else if matches_sequence(tokens, next_index, "</u>") {
        Some(4)
    } else if matches_sequence(tokens, next_index, "<u>") {
        Some(3)
    } else if matches_sequence(tokens, next_index, "\\")
        || matches_sequence(tokens, next_index, "*")
        || matches_sequence(tokens, next_index, "_")
        || matches_sequence(tokens, next_index, "~")
        || matches_sequence(tokens, next_index, "[")
        || matches_sequence(tokens, next_index, "]")
        || matches_sequence(tokens, next_index, "`")
        || matches_sequence(tokens, next_index, "^")
        || matches_sequence(tokens, next_index, "=")
    {
        Some(1)
    } else {
        None
    }
}

pub(super) fn escape_literal_text_with_offset_map(text: &str) -> InlineMarkdownOffsetMap {
    let mut escaped = String::new();
    let mut visible_to_markdown = vec![0; text.len() + 1];
    let mut markdown_to_visible = vec![0];
    let mut index = 0;

    while index < text.len() {
        visible_to_markdown[index] = escaped.len();
        if text[index..].starts_with("</strong>") {
            let start = escaped.len();
            escaped.push('\\');
            escaped.push_str("</strong>");
            markdown_to_visible.resize(escaped.len() + 1, index);
            for local in 0..=escaped.len() - start {
                markdown_to_visible[start + local] = index;
            }
            index += 9;
            continue;
        }

        if text[index..].starts_with("<strong>") {
            let start = escaped.len();
            escaped.push('\\');
            escaped.push_str("<strong>");
            markdown_to_visible.resize(escaped.len() + 1, index);
            for local in 0..=escaped.len() - start {
                markdown_to_visible[start + local] = index;
            }
            index += 8;
            continue;
        }

        if text[index..].starts_with("</em>") {
            let start = escaped.len();
            escaped.push('\\');
            escaped.push_str("</em>");
            markdown_to_visible.resize(escaped.len() + 1, index);
            for local in 0..=escaped.len() - start {
                markdown_to_visible[start + local] = index;
            }
            index += 5;
            continue;
        }

        if text[index..].starts_with("<em>") {
            let start = escaped.len();
            escaped.push('\\');
            escaped.push_str("<em>");
            markdown_to_visible.resize(escaped.len() + 1, index);
            for local in 0..=escaped.len() - start {
                markdown_to_visible[start + local] = index;
            }
            index += 4;
            continue;
        }

        if text[index..].starts_with("</u>") {
            let start = escaped.len();
            escaped.push('\\');
            escaped.push_str("</u>");
            markdown_to_visible.resize(escaped.len() + 1, index);
            for local in 0..=escaped.len() - start {
                markdown_to_visible[start + local] = index;
            }
            index += 4;
            continue;
        }

        if text[index..].starts_with("<u>") {
            let start = escaped.len();
            escaped.push('\\');
            escaped.push_str("<u>");
            markdown_to_visible.resize(escaped.len() + 1, index);
            for local in 0..=escaped.len() - start {
                markdown_to_visible[start + local] = index;
            }
            index += 3;
            continue;
        }

        if text[index..].starts_with('\\') {
            let start = escaped.len();
            escaped.push_str("\\\\");
            markdown_to_visible.resize(escaped.len() + 1, index);
            for local in 0..=escaped.len() - start {
                markdown_to_visible[start + local] = index;
            }
            index += 1;
            continue;
        }

        if text[index..].starts_with('*') {
            let start = escaped.len();
            escaped.push_str("\\*");
            markdown_to_visible.resize(escaped.len() + 1, index);
            for local in 0..=escaped.len() - start {
                markdown_to_visible[start + local] = index;
            }
            index += 1;
            continue;
        }

        if text[index..].starts_with('_') {
            let start = escaped.len();
            escaped.push_str("\\_");
            markdown_to_visible.resize(escaped.len() + 1, index);
            for local in 0..=escaped.len() - start {
                markdown_to_visible[start + local] = index;
            }
            index += 1;
            continue;
        }

        if text[index..].starts_with('~') {
            let start = escaped.len();
            escaped.push_str("\\~");
            markdown_to_visible.resize(escaped.len() + 1, index);
            for local in 0..=escaped.len() - start {
                markdown_to_visible[start + local] = index;
            }
            index += 1;
            continue;
        }

        if text[index..].starts_with('^') {
            let start = escaped.len();
            escaped.push_str("\\^");
            markdown_to_visible.resize(escaped.len() + 1, index);
            for local in 0..=escaped.len() - start {
                markdown_to_visible[start + local] = index;
            }
            index += 1;
            continue;
        }

        if text[index..].starts_with('`') {
            let start = escaped.len();
            escaped.push_str("\\`");
            markdown_to_visible.resize(escaped.len() + 1, index);
            for local in 0..=escaped.len() - start {
                markdown_to_visible[start + local] = index;
            }
            index += 1;
            continue;
        }

        let ch = text[index..].chars().next().unwrap();
        let start = escaped.len();
        escaped.push(ch);
        markdown_to_visible.resize(escaped.len() + 1, index);
        for local in 0..=escaped.len() - start {
            markdown_to_visible[start + local] = index;
        }
        index += ch.len_utf8();
    }
    visible_to_markdown[text.len()] = escaped.len();
    markdown_to_visible[escaped.len()] = text.len();

    InlineMarkdownOffsetMap {
        markdown: escaped,
        visible_to_markdown,
        markdown_to_visible,
    }
}

pub(super) fn escape_code_span_text_with_offset_map(text: &str) -> InlineMarkdownOffsetMap {
    let needs_padding = !text.is_empty()
        && !text.chars().all(|ch| ch == ' ')
        && (text.starts_with([' ', '`']) || text.ends_with([' ', '`']));
    let leading_padding = usize::from(needs_padding);

    let mut markdown = String::new();
    if needs_padding {
        markdown.push(' ');
    }
    markdown.push_str(text);
    if needs_padding {
        markdown.push(' ');
    }

    let mut visible_to_markdown = vec![0; text.len() + 1];
    for (visible, markdown_offset) in visible_to_markdown.iter_mut().enumerate() {
        *markdown_offset = leading_padding + visible;
    }

    let content_start = leading_padding;
    let content_end = leading_padding + text.len();
    let mut markdown_to_visible = vec![0; markdown.len() + 1];
    for (markdown_offset, visible) in markdown_to_visible.iter_mut().enumerate() {
        *visible = if markdown_offset <= content_start {
            0
        } else if markdown_offset >= content_end {
            text.len()
        } else {
            markdown_offset - content_start
        };
    }

    InlineMarkdownOffsetMap {
        markdown,
        visible_to_markdown,
        markdown_to_visible,
    }
}

/// Viterbi-like DP that picks the optimal delimiter stack for each fragment.
///
/// Each fragment's style can be expressed with either Markdown or HTML
/// delimiters.  We minimize the total number of delimiter characters written
/// plus a penalty for HTML variants.  A large penalty is added when a
/// transition would produce 4+ consecutive `*` characters (Markdown ambiguity).
pub(super) fn choose_fragment_stacks(fragments: &[InlineFragment]) -> Vec<Vec<Delimiter>> {
    // Enumerate the 1-2 possible delimiter stacks for each fragment's style.
    let variants = fragments
        .iter()
        .enumerate()
        .map(|(index, fragment)| {
            stack_variants(
                fragment,
                index.checked_sub(1).and_then(|i| fragments.get(i)),
            )
        })
        .collect::<Vec<_>>();

    // DP table: costs[fragment_index][choice_index]
    let mut costs: Vec<Vec<usize>> = variants
        .iter()
        .map(|choices| vec![usize::MAX; choices.len()])
        .collect();
    let mut previous_choice: Vec<Vec<Option<usize>>> = variants
        .iter()
        .map(|choices| vec![None; choices.len()])
        .collect();

    // Initial fragment: cost from empty stack to each variant.
    for (choice_index, stack) in variants[0].iter().enumerate() {
        costs[0][choice_index] = stack_transition_cost(&[], stack) + stack_variant_penalty(stack);
    }

    // Forward pass: compute minimum cost for each fragment's choices.
    for fragment_index in 1..variants.len() {
        for (choice_index, stack) in variants[fragment_index].iter().enumerate() {
            for (prev_index, prev_stack) in variants[fragment_index - 1].iter().enumerate() {
                let prev_cost = costs[fragment_index - 1][prev_index];
                if prev_cost == usize::MAX {
                    continue;
                }

                let cost = prev_cost
                    + stack_transition_cost(prev_stack, stack)
                    + stack_variant_penalty(stack);
                if cost < costs[fragment_index][choice_index] {
                    costs[fragment_index][choice_index] = cost;
                    previous_choice[fragment_index][choice_index] = Some(prev_index);
                }
            }
        }
    }

    // Backtrack: choose the best final stack and trace back through the DP.
    let last_fragment_index = variants.len() - 1;
    let (mut best_choice, _) = variants[last_fragment_index]
        .iter()
        .enumerate()
        .map(|(choice_index, stack)| {
            (
                choice_index,
                costs[last_fragment_index][choice_index] + stack_transition_cost(stack, &[]),
            )
        })
        .min_by(|(left_index, left_cost), (right_index, right_cost)| {
            left_cost.cmp(right_cost).then_with(|| {
                stack_preference_key(&variants[last_fragment_index][*left_index]).cmp(
                    &stack_preference_key(&variants[last_fragment_index][*right_index]),
                )
            })
        })
        .unwrap_or((0, 0));

    let mut chosen = vec![Vec::new(); variants.len()];
    for fragment_index in (0..variants.len()).rev() {
        chosen[fragment_index] = variants[fragment_index][best_choice].clone();
        if let Some(prev_index) = previous_choice[fragment_index][best_choice] {
            best_choice = prev_index;
        }
    }

    chosen
}

pub(super) fn stack_variants(
    fragment: &InlineFragment,
    previous_fragment: Option<&InlineFragment>,
) -> Vec<Vec<Delimiter>> {
    let style = fragment.style;
    let code_run_len = style.code.then(|| code_delimiter_run_len(&fragment.text));
    let mut markdown_stack = Vec::new();
    if style.bold {
        markdown_stack.push(Delimiter::BoldMarkdown { marker: '*' });
    }
    if style.underline {
        markdown_stack.push(Delimiter::Underline);
    }
    if style.highlight {
        markdown_stack.push(Delimiter::HighlightMarkdown);
    }
    if style.strikethrough {
        markdown_stack.push(Delimiter::StrikethroughMarkdown);
    }
    match style.script {
        InlineScript::Normal => {}
        InlineScript::Superscript
            if can_use_markdown_script_delimiters(previous_fragment, fragment) =>
        {
            markdown_stack.push(Delimiter::SuperscriptMarkdown)
        }
        InlineScript::Superscript => markdown_stack.push(Delimiter::SuperscriptHtml),
        InlineScript::Subscript
            if style.strikethrough
                || !can_use_markdown_script_delimiters(previous_fragment, fragment) =>
        {
            markdown_stack.push(Delimiter::SubscriptHtml)
        }
        InlineScript::Subscript => markdown_stack.push(Delimiter::SubscriptMarkdown),
    }
    if style.italic {
        markdown_stack.push(Delimiter::ItalicMarkdown { marker: '*' });
    }
    // Code is always the innermost delimiter so it nests inside emphasis.
    if let Some(run_len) = code_run_len {
        markdown_stack.push(Delimiter::CodeMarkdown { run_len });
    }

    let has_emphasis = style.bold || style.italic;
    if !has_emphasis {
        return vec![markdown_stack];
    }

    let mut html_stack = Vec::new();
    if style.bold {
        html_stack.push(Delimiter::BoldHtml);
    }
    if style.underline {
        html_stack.push(Delimiter::Underline);
    }
    if style.highlight {
        html_stack.push(Delimiter::HighlightMarkdown);
    }
    if style.strikethrough {
        html_stack.push(Delimiter::StrikethroughMarkdown);
    }
    match style.script {
        InlineScript::Normal => {}
        InlineScript::Superscript => html_stack.push(Delimiter::SuperscriptHtml),
        InlineScript::Subscript => html_stack.push(Delimiter::SubscriptHtml),
    }
    if style.italic {
        html_stack.push(Delimiter::ItalicHtml);
    }
    if let Some(run_len) = code_run_len {
        html_stack.push(Delimiter::CodeMarkdown { run_len });
    }

    vec![markdown_stack, html_stack]
}

pub(crate) fn can_use_markdown_script_delimiters(
    previous_fragment: Option<&InlineFragment>,
    fragment: &InlineFragment,
) -> bool {
    // This guard is shared by serialization and inline projection. Markdown
    // script markers need a plain ASCII owner immediately before the script
    // fragment; otherwise we fall back to <sup>/<sub> so the next parse sees
    // the same style boundary.
    let Some(previous) = previous_fragment else {
        return false;
    };
    if previous.style.has_script() {
        return false;
    }
    previous
        .text
        .chars()
        .next_back()
        .is_some_and(|ch| ch.is_ascii_alphanumeric())
        && previous.html_style == fragment.html_style
        && previous.link == fragment.link
        && previous.footnote.is_none()
        && fragment.footnote.is_none()
        && previous.math.is_none()
        && fragment.math.is_none()
        && styles_match_ignoring_script(previous.style, fragment.style)
}

pub(super) fn styles_match_ignoring_script(left: InlineStyle, right: InlineStyle) -> bool {
    left.bold == right.bold
        && left.italic == right.italic
        && left.underline == right.underline
        && left.highlight == right.highlight
        && left.strikethrough == right.strikethrough
        && left.code == right.code
}

pub(super) fn code_delimiter_run_len(text: &str) -> usize {
    let mut longest = 0usize;
    let mut current = 0usize;
    for ch in text.chars() {
        if ch == '`' {
            current += 1;
            longest = longest.max(current);
        } else {
            current = 0;
        }
    }
    longest + 1
}

pub(super) fn stack_transition_len(from: &[Delimiter], to: &[Delimiter]) -> usize {
    let common = common_prefix_len(from, to);
    let close_len = from[common..]
        .iter()
        .rev()
        .map(|delimiter| delimiter.close().len())
        .sum::<usize>();
    let open_len = to[common..]
        .iter()
        .map(|delimiter| delimiter.open().len())
        .sum::<usize>();
    close_len + open_len
}

/// Cost of closing `from` delimiters and opening `to` delimiters in sequence.
/// Adds a heavy penalty if the resulting string would contain 4+ consecutive
/// `*` characters, which Markdown parsers may interpret ambiguously.
pub(super) fn stack_transition_cost(from: &[Delimiter], to: &[Delimiter]) -> usize {
    let marker_len = stack_transition_len(from, to);
    let marker_string = stack_transition_string(from, to);
    let ambiguity_penalty =
        if !from.is_empty() && !to.is_empty() && longest_star_run(&marker_string) >= 4 {
            1_000
        } else {
            0
        };
    marker_len + ambiguity_penalty
}

pub(super) fn stack_variant_penalty(stack: &[Delimiter]) -> usize {
    if stack.iter().any(|delimiter| delimiter.is_html()) {
        64
    } else {
        0
    }
}

pub(super) fn write_stack_transition(output: &mut String, from: &[Delimiter], to: &[Delimiter]) {
    let common = common_prefix_len(from, to);
    for delimiter in from[common..].iter().rev() {
        output.push_str(&delimiter.close());
    }
    for delimiter in &to[common..] {
        output.push_str(&delimiter.open());
    }
}

pub(super) fn stack_transition_string(from: &[Delimiter], to: &[Delimiter]) -> String {
    let mut output = String::new();
    write_stack_transition(&mut output, from, to);
    output
}

pub(super) fn common_prefix_len(left: &[Delimiter], right: &[Delimiter]) -> usize {
    let mut index = 0;
    while index < left.len() && index < right.len() && left[index] == right[index] {
        index += 1;
    }
    index
}

pub(super) fn stack_preference_key(stack: &[Delimiter]) -> Vec<u8> {
    stack
        .iter()
        .map(|delimiter| delimiter.preference_rank())
        .collect()
}

pub(super) fn longest_star_run(text: &str) -> usize {
    let mut max_run = 0;
    let mut current_run = 0;
    for ch in text.chars() {
        if ch == '*' {
            current_run += 1;
            max_run = max_run.max(current_run);
        } else {
            current_run = 0;
        }
    }
    max_run
}

pub(super) fn style_flag_enabled(style: InlineStyle, flag: StyleFlag) -> bool {
    match flag {
        StyleFlag::Bold => style.bold,
        StyleFlag::Italic => style.italic,
        StyleFlag::Underline => style.underline,
        StyleFlag::Strikethrough => style.strikethrough,
        StyleFlag::Highlight => style.highlight,
        StyleFlag::Code => style.code,
        StyleFlag::Superscript => style.script == InlineScript::Superscript,
        StyleFlag::Subscript => style.script == InlineScript::Subscript,
    }
}

pub(super) fn set_style_flag(
    mut style: InlineStyle,
    flag: StyleFlag,
    enabled: bool,
) -> InlineStyle {
    match flag {
        StyleFlag::Bold => style.bold = enabled,
        StyleFlag::Italic => style.italic = enabled,
        StyleFlag::Underline => style.underline = enabled,
        StyleFlag::Strikethrough => style.strikethrough = enabled,
        StyleFlag::Highlight => style.highlight = enabled,
        StyleFlag::Code => style.code = enabled,
        StyleFlag::Superscript => {
            style.script = if enabled {
                InlineScript::Superscript
            } else if style.script == InlineScript::Superscript {
                InlineScript::Normal
            } else {
                style.script
            }
        }
        StyleFlag::Subscript => {
            style.script = if enabled {
                InlineScript::Subscript
            } else if style.script == InlineScript::Subscript {
                InlineScript::Normal
            } else {
                style.script
            }
        }
    }
    style
}

pub(super) fn clamp_to_char_boundary(text: &str, offset: usize) -> usize {
    let clamped = offset.min(text.len());
    if text.is_char_boundary(clamped) {
        return clamped;
    }

    let mut boundary = clamped;
    while boundary > 0 && !text.is_char_boundary(boundary) {
        boundary -= 1;
    }
    boundary
}

/// 为 `_` 应用 CommonMark 左右侧规则，避免标识符内部下划线被当作格式标记。
/// 其它扩展标记保留旧判定，防止修复标准下划线时改变 GMark 已有组合语法。
pub(super) fn can_open_emphasis(
    tokens: &[CharToken],
    index: usize,
    len: usize,
    marker: char,
) -> bool {
    if marker != '_' {
        return tokens
            .get(index + len)
            .is_some_and(|token| !token.ch.is_whitespace());
    }
    if index > 0 && tokens[index - 1].ch == marker {
        return false;
    }

    let flanking = delimiter_flanking(tokens, index, len);
    flanking.left && (!flanking.right || flanking.before_is_punctuation)
}

pub(super) fn can_open_script(tokens: &[CharToken], index: usize, marker: char) -> bool {
    if token_is_backslash_escaped(tokens, index) {
        return false;
    }

    if marker == '~' && !is_single_tilde_delimiter(tokens, index) {
        return false;
    }

    index > 0
        && tokens[index - 1].ch.is_ascii_alphanumeric()
        && tokens
            .get(index + 1)
            .is_some_and(|token| token.ch.is_ascii_alphanumeric())
}

/// 为 `_` 应用 CommonMark 右侧规则；其它标记继续使用旧闭合语义。
pub(super) fn can_close_emphasis(
    tokens: &[CharToken],
    index: usize,
    len: usize,
    marker: char,
) -> bool {
    if marker != '_' {
        return index > 0 && !tokens[index - 1].ch.is_whitespace();
    }
    if index > 0 && tokens[index - 1].ch == marker {
        return false;
    }

    let flanking = delimiter_flanking(tokens, index, len);
    flanking.right && (!flanking.left || flanking.after_is_punctuation)
}

/// 统一关闭判断，但仅让 Markdown 强调采用新规则，保留高亮、上下标和 HTML 扩展的既有语义。
pub(super) fn can_close_delimiter(
    tokens: &[CharToken],
    index: usize,
    delimiter: Delimiter,
) -> bool {
    match delimiter {
        Delimiter::BoldMarkdown { marker } => can_close_emphasis(tokens, index, 2, marker),
        Delimiter::ItalicMarkdown { marker } => can_close_emphasis(tokens, index, 1, marker),
        _ => index > 0 && !tokens[index - 1].ch.is_whitespace(),
    }
}

#[derive(Clone, Copy)]
struct DelimiterFlanking {
    left: bool,
    right: bool,
    before_is_punctuation: bool,
    after_is_punctuation: bool,
}

/// 只读取分隔符两侧字符并计算左右侧属性，确保 Unicode 标点和符号与 CommonMark 一致处理。
fn delimiter_flanking(tokens: &[CharToken], index: usize, len: usize) -> DelimiterFlanking {
    let before = index.checked_sub(1).and_then(|offset| tokens.get(offset));
    let after = tokens.get(index + len);
    let before_is_whitespace = before.is_none_or(|token| token.ch.is_whitespace());
    let after_is_whitespace = after.is_none_or(|token| token.ch.is_whitespace());
    let before_is_punctuation = before.is_some_and(|token| markdown_punctuation(token.ch));
    let after_is_punctuation = after.is_some_and(|token| markdown_punctuation(token.ch));

    DelimiterFlanking {
        left: !after_is_whitespace
            && (!after_is_punctuation || before_is_whitespace || before_is_punctuation),
        right: !before_is_whitespace
            && (!before_is_punctuation || after_is_whitespace || after_is_punctuation),
        before_is_punctuation,
        after_is_punctuation,
    }
}

/// CommonMark 把 Unicode Punctuation 与 Symbol 类别都视作标点，不能只检查 ASCII。
fn markdown_punctuation(ch: char) -> bool {
    ch.is_punctuation() || ch.is_symbol()
}
