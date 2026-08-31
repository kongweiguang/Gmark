// @author kongweiguang

use super::*;

pub(super) fn parse_autolink(
    tokens: &[CharToken],
    index: usize,
    extra_style: InlineStyle,
    extra_html_style: Option<HtmlInlineStyle>,
    builder: &mut NormalizeBuilder,
    _reference_definitions: &LinkReferenceDefinitions,
) -> Option<usize> {
    let end_index = locate_autolink(tokens, index)?;
    let target_tokens = &tokens[index + 1..end_index];
    let target = tokens_to_string(target_tokens);
    let fragments = vec![InlineFragment {
        text: target.clone(),
        style: extra_style,
        html_style: extra_html_style,
        link: Some(InlineLink::Autolink {
            target: target.clone(),
        }),
        footnote: None,
        math: None,
    }];

    let normalized_start = builder.normalized_len;
    let target_len = target.len();

    for boundary in tokens[index].source_range.start..=tokens[index].source_range.end {
        builder.visible_to_normalized[boundary] = normalized_start;
    }

    let mut local_boundary = 0usize;
    for token in target_tokens {
        let token_len = token.source_range.len();
        for delta in 0..=token_len {
            builder.visible_to_normalized[token.source_range.start + delta] =
                normalized_start + local_boundary + delta;
        }
        local_boundary += token_len;
    }

    let normalized_end = normalized_start + target_len;
    for boundary in tokens[end_index].source_range.start..=tokens[end_index].source_range.end {
        builder.visible_to_normalized[boundary] = normalized_end;
    }

    for fragment in fragments {
        builder.normalized_len += fragment.text.len();
        if let Some(last) = builder.fragments.last_mut()
            && last.style == fragment.style
            && last.html_style == fragment.html_style
            && last.link == fragment.link
            && last.footnote == fragment.footnote
            && last.math.is_none()
            && fragment.math.is_none()
        {
            last.text.push_str(&fragment.text);
        } else {
            builder.fragments.push(fragment);
        }
    }

    Some(end_index + 1)
}

#[derive(Clone, Debug)]
pub(super) struct InlineHtmlTag {
    name: String,
    attrs: Vec<HtmlAttr>,
    end_index: usize,
    self_closing: bool,
}

pub(super) fn parse_inline_html_container(
    tokens: &[CharToken],
    index: usize,
    extra_style: InlineStyle,
    extra_html_style: Option<HtmlInlineStyle>,
    builder: &mut NormalizeBuilder,
    reference_definitions: &LinkReferenceDefinitions,
) -> Option<usize> {
    let tag = locate_inline_html_open_tag(tokens, index)?;
    if tag.self_closing || !is_inline_tag(&tag.name) || has_dangerous_attrs(&tag.attrs) {
        return None;
    }

    let (close_start, close_end) =
        locate_matching_inline_html_close(tokens, tag.end_index + 1, &tag.name)?;
    let tag_style = inline_html_semantic_style(&tag.name, extra_style);
    let html_style = merge_html_styles(extra_html_style, inline_html_style(&tag));
    if tag_style == extra_style && html_style == extra_html_style {
        return None;
    }

    for token in &tokens[index..=tag.end_index] {
        builder.drop_token(token);
    }
    let _ = parse_until(
        &tokens[tag.end_index + 1..close_start],
        0,
        None,
        tag_style,
        html_style,
        builder,
        false,
        reference_definitions,
    );
    for token in &tokens[close_start..=close_end] {
        builder.drop_token(token);
    }

    Some(close_end + 1)
}

pub(super) fn inline_html_semantic_style(name: &str, style: InlineStyle) -> InlineStyle {
    match name {
        "strong" | "b" => style.with_bold(),
        "em" | "i" => style.with_italic(),
        "u" | "ins" => style.with_underline(),
        "del" => style.with_strikethrough(),
        "code" | "kbd" => style.with_code(),
        "sup" => style.with_superscript(),
        "sub" => style.with_subscript(),
        _ => style,
    }
}

pub(super) fn inline_html_style(tag: &InlineHtmlTag) -> Option<HtmlInlineStyle> {
    let node = HtmlNode {
        id: None,
        kind: HtmlNodeKind::InlineSemantic,
        tag_name: tag.name.clone(),
        attrs: tag.attrs.clone(),
        children: Vec::new(),
        raw_source: String::new(),
        source_range: 0..0,
    };
    let style = style_for_node(&node);
    (!style.is_empty()).then_some(style)
}

pub(super) fn merge_html_styles(
    parent: Option<HtmlInlineStyle>,
    child: Option<HtmlInlineStyle>,
) -> Option<HtmlInlineStyle> {
    let mut merged = parent.unwrap_or_default();
    if let Some(child) = child {
        if child.color.is_some() {
            merged.color = child.color;
        }
        if child.background_color.is_some() {
            merged.background_color = child.background_color;
        }
        if child.font_size.is_some() {
            merged.font_size = child.font_size;
        }
    }

    (!merged.is_empty()).then_some(merged)
}

/// Located inline link syntax inside the token stream.
#[derive(Clone)]
pub(super) struct LocatedInlineLink {
    pub(super) label_end: usize,
    pub(super) end_index: usize,
    pub(super) link: InlineLink,
}

pub(super) fn locate_inline_link(
    tokens: &[CharToken],
    index: usize,
    reference_definitions: &LinkReferenceDefinitions,
) -> Option<LocatedInlineLink> {
    if tokens.get(index)?.ch != '[' {
        return None;
    }
    if index > 0 && matches!(tokens[index - 1].ch, '!' | ']') {
        return None;
    }

    let mut label_depth = 0usize;
    let mut cursor = index + 1;
    let label_end = loop {
        let token = tokens.get(cursor)?;
        if token.ch == '\\' {
            cursor += 2;
            continue;
        }

        match token.ch {
            '[' => label_depth += 1,
            ']' if label_depth == 0 => break cursor,
            ']' => label_depth = label_depth.saturating_sub(1),
            _ => {}
        }
        cursor += 1;
    };

    match tokens.get(label_end + 1).map(|token| token.ch) {
        Some('(') => {
            let url_start = label_end + 2;
            let mut paren_depth = 0usize;
            cursor = url_start;
            let url_end = loop {
                let token = tokens.get(cursor)?;
                if token.ch == '\\' {
                    cursor += 2;
                    continue;
                }

                match token.ch {
                    '(' => paren_depth += 1,
                    ')' if paren_depth == 0 => break cursor,
                    ')' => paren_depth = paren_depth.saturating_sub(1),
                    _ => {}
                }
                cursor += 1;
            };

            // An empty destination such as in `[label]()` is a valid link, but the
            // target parser rejects an empty string. Recognizing it keeps the caret
            // inside the projected link while the destination is filled in.
            let (destination, title) = if url_start == url_end {
                (String::new(), None)
            } else {
                parse_link_target(&tokens_to_string(&tokens[url_start..url_end]))?
            };
            Some(LocatedInlineLink {
                label_end,
                end_index: url_end,
                link: InlineLink::Inline { destination, title },
            })
        }
        Some('[') => {
            let reference_start = label_end + 2;
            cursor = reference_start;
            let reference_end = loop {
                let token = tokens.get(cursor)?;
                if token.ch == '\\' {
                    cursor += 2;
                    continue;
                }
                if token.ch == ']' {
                    break cursor;
                }
                cursor += 1;
            };

            let raw_label = tokens_to_string(&tokens[reference_start..reference_end]);
            let link_label = if raw_label.is_empty() {
                tokens_to_string(&tokens[index + 1..label_end])
            } else {
                raw_label
            };
            let normalized_label = super::super::image::normalize_reference_label(&link_label)?;
            let LinkReferenceDefinition { destination, .. } =
                reference_definitions.get(&normalized_label)?.clone();
            Some(LocatedInlineLink {
                label_end,
                end_index: reference_end,
                link: InlineLink::Reference {
                    label: link_label,
                    destination,
                },
            })
        }
        _ => {
            let raw_label = tokens_to_string(&tokens[index + 1..label_end]);
            let normalized_label = super::super::image::normalize_reference_label(&raw_label)?;
            let LinkReferenceDefinition { destination, .. } =
                reference_definitions.get(&normalized_label)?.clone();
            Some(LocatedInlineLink {
                label_end,
                end_index: label_end,
                link: InlineLink::Reference {
                    label: raw_label,
                    destination,
                },
            })
        }
    }
}

pub(super) fn locate_autolink(tokens: &[CharToken], index: usize) -> Option<usize> {
    if tokens.get(index)?.ch != '<' {
        return None;
    }

    let mut cursor = index + 1;
    let end_index = loop {
        let token = tokens.get(cursor)?;
        if token.ch == '\\' {
            cursor += 2;
            continue;
        }
        if token.ch == '>' {
            break cursor;
        }
        cursor += 1;
    };

    let target = tokens_to_string(&tokens[index + 1..end_index]);
    (!target.is_empty() && !looks_like_non_autolink_html_tag(tokens, end_index, &target))
        .then_some(end_index)
}

pub(super) fn tokens_to_string(tokens: &[CharToken]) -> String {
    tokens.iter().map(|token| token.ch).collect()
}

pub(super) fn locate_inline_html_open_tag(
    tokens: &[CharToken],
    index: usize,
) -> Option<InlineHtmlTag> {
    if tokens.get(index)?.ch != '<' {
        return None;
    }

    let mut cursor = index + 1;
    if !tokens.get(cursor)?.ch.is_ascii_alphabetic() {
        return None;
    }
    let name_start = cursor;
    while cursor < tokens.len() && is_html_tag_name_char(tokens[cursor].ch) {
        cursor += 1;
    }
    let name = tokens_to_string(&tokens[name_start..cursor]).to_ascii_lowercase();

    match tokens.get(cursor).map(|token| token.ch) {
        Some(ch) if ch.is_whitespace() || ch == '>' || ch == '/' => {}
        _ => return None,
    }

    let attrs_start = cursor;
    let mut quote = None;
    while cursor < tokens.len() {
        let ch = tokens[cursor].ch;
        if let Some(active_quote) = quote {
            if ch == active_quote {
                quote = None;
            }
            cursor += 1;
            continue;
        }

        if ch == '"' || ch == '\'' {
            quote = Some(ch);
            cursor += 1;
            continue;
        }

        if ch == '>' {
            let attrs_source = tokens_to_string(&tokens[attrs_start..cursor]);
            let self_closing = attrs_source.trim_end().ends_with('/');
            return Some(InlineHtmlTag {
                name,
                attrs: parse_html_attrs(&attrs_source),
                end_index: cursor,
                self_closing,
            });
        }

        cursor += 1;
    }

    None
}

pub(super) fn locate_inline_html_close_tag(
    tokens: &[CharToken],
    index: usize,
    expected_name: &str,
) -> Option<usize> {
    if tokens.get(index)?.ch != '<' || tokens.get(index + 1)?.ch != '/' {
        return None;
    }

    let mut cursor = index + 2;
    while tokens
        .get(cursor)
        .is_some_and(|token| token.ch.is_whitespace())
    {
        cursor += 1;
    }
    let name_start = cursor;
    while cursor < tokens.len() && is_html_tag_name_char(tokens[cursor].ch) {
        cursor += 1;
    }
    if name_start == cursor {
        return None;
    }
    let name = tokens_to_string(&tokens[name_start..cursor]).to_ascii_lowercase();
    if name != expected_name {
        return None;
    }
    while tokens
        .get(cursor)
        .is_some_and(|token| token.ch.is_whitespace())
    {
        cursor += 1;
    }
    (tokens.get(cursor)?.ch == '>').then_some(cursor)
}

pub(super) fn locate_matching_inline_html_close(
    tokens: &[CharToken],
    mut cursor: usize,
    name: &str,
) -> Option<(usize, usize)> {
    let mut depth = 1usize;
    while cursor < tokens.len() {
        if tokens[cursor].ch != '<' {
            cursor += 1;
            continue;
        }

        if let Some(close_end) = locate_inline_html_close_tag(tokens, cursor, name) {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                return Some((cursor, close_end));
            }
            cursor = close_end + 1;
            continue;
        }

        if let Some(open) = locate_inline_html_open_tag(tokens, cursor) {
            if open.name == name && !open.self_closing {
                depth += 1;
            }
            cursor = open.end_index + 1;
            continue;
        }

        cursor += 1;
    }

    None
}

pub(super) fn looks_like_non_autolink_html_tag(
    tokens: &[CharToken],
    end_index: usize,
    target: &str,
) -> bool {
    let target = target.trim();
    if target.starts_with('/') {
        let rest = target.trim_start_matches('/').trim();
        return html_tag_name_with_attrs(rest).is_some();
    }

    if let Some((_tag_name, has_attrs_or_slash)) = html_tag_name_with_attrs(target)
        && has_attrs_or_slash
    {
        return true;
    }

    let Some((tag_name, _)) = html_tag_name_with_attrs(target) else {
        return false;
    };
    let rest = tokens_to_string(&tokens[end_index + 1..]).to_ascii_lowercase();
    let tag_name = tag_name.to_ascii_lowercase();
    rest.contains(&format!("</{tag_name}>"))
}

pub(super) fn html_tag_name_with_attrs(target: &str) -> Option<(&str, bool)> {
    if target.is_empty() {
        return None;
    }

    let first = target.as_bytes()[0];
    if !first.is_ascii_alphabetic() {
        return None;
    }

    let mut end = 0usize;
    for (index, ch) in target.char_indices() {
        if is_html_tag_name_char(ch) {
            end = index + ch.len_utf8();
        } else {
            break;
        }
    }
    if end == 0 {
        return None;
    }

    let raw_rest = &target[end..];
    let rest = raw_rest.trim();
    if rest.is_empty() {
        return Some((&target[..end], false));
    }
    (raw_rest.chars().next().is_some_and(|ch| ch.is_whitespace())
        || rest == "/"
        || rest.starts_with('/'))
    .then_some((&target[..end], true))
}

pub(super) fn is_html_tag_name_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_')
}

pub(super) fn apply_extra_style_to_fragments(
    fragments: &mut [InlineFragment],
    extra_style: InlineStyle,
    extra_html_style: Option<HtmlInlineStyle>,
) {
    for fragment in fragments {
        if extra_style.bold {
            fragment.style.bold = true;
        }
        if extra_style.italic {
            fragment.style.italic = true;
        }
        if extra_style.underline {
            fragment.style.underline = true;
        }
        if extra_style.strikethrough {
            fragment.style.strikethrough = true;
        }
        if extra_style.code {
            fragment.style.code = true;
        }
        if extra_style.highlight {
            fragment.style.highlight = true;
        }
        if extra_style.has_script() {
            fragment.style.script = extra_style.script;
        }
        fragment.html_style = merge_html_styles(extra_html_style, fragment.html_style);
    }
}

/// 按稳定优先级识别当前位置的开放分隔符，并把标记种类传给边界策略。
/// 下划线需要区别于星号应用 CommonMark 词内约束，不能只按字符序列判定。
pub(super) fn match_open_delimiter(tokens: &[CharToken], index: usize) -> Option<Delimiter> {
    if matches_sequence(tokens, index, "<strong>") {
        Some(Delimiter::BoldHtml)
    } else if matches_sequence(tokens, index, "<em>") {
        Some(Delimiter::ItalicHtml)
    } else if matches_sequence(tokens, index, "<u>") {
        Some(Delimiter::Underline)
    } else if matches_sequence(tokens, index, "~~") {
        Some(Delimiter::StrikethroughMarkdown)
    } else if is_double_equals_delimiter(tokens, index) && can_open_emphasis(tokens, index, 2, '=')
    {
        Some(Delimiter::HighlightMarkdown)
    } else if matches_sequence(tokens, index, "^") && can_open_script(tokens, index, '^') {
        Some(Delimiter::SuperscriptMarkdown)
    } else if is_single_tilde_delimiter(tokens, index) && can_open_script(tokens, index, '~') {
        Some(Delimiter::SubscriptMarkdown)
    } else if matches_sequence(tokens, index, "**") && can_open_emphasis(tokens, index, 2, '*') {
        Some(Delimiter::BoldMarkdown { marker: '*' })
    } else if matches_sequence(tokens, index, "__") && can_open_emphasis(tokens, index, 2, '_') {
        Some(Delimiter::BoldMarkdown { marker: '_' })
    } else if matches_sequence(tokens, index, "*") && can_open_emphasis(tokens, index, 1, '*') {
        Some(Delimiter::ItalicMarkdown { marker: '*' })
    } else if matches_sequence(tokens, index, "_") && can_open_emphasis(tokens, index, 1, '_') {
        Some(Delimiter::ItalicMarkdown { marker: '_' })
    } else if tokens[index].ch == '`' {
        // Count the run of consecutive backticks.
        let run_len = backtick_run_len(tokens, index);
        // A backtick run is only a valid opener if it is NOT immediately
        // followed by another backtick (no double-counting).
        if run_len > 0 {
            Some(Delimiter::CodeMarkdown { run_len })
        } else {
            None
        }
    } else {
        None
    }
}

/// Returns the length of the consecutive backtick run starting at `index`.
pub(super) fn backtick_run_len(tokens: &[CharToken], index: usize) -> usize {
    let mut len = 0;
    while index + len < tokens.len() && tokens[index + len].ch == '`' {
        len += 1;
    }
    // A backtick run is only valid if it's not immediately preceded by an
    // additional backtick (the run must start at `index`).
    if index > 0 && tokens[index - 1].ch == '`' {
        return 0;
    }
    len
}

/// 预判开放分隔符是否存在合法闭合点，必须复用实际解析阶段的边界规则。
/// 这样可避免预判接受、递归解析拒绝后把后续文本错误吞入当前样式范围。
pub(super) fn has_closing_delimiter(
    tokens: &[CharToken],
    index: usize,
    delimiter: Delimiter,
) -> bool {
    let skip = delimiter.token_len();
    let close_str = delimiter.close();

    // For code spans we look for a matching-length backtick run;
    // for emphasis we just scan for the close string.
    if let Delimiter::CodeMarkdown { .. } = delimiter {
        let mut cursor = index + skip;
        while cursor < tokens.len() {
            if tokens[cursor].ch == '\\'
                && let Some(escaped_len) = escaped_sequence_token_len(tokens, cursor)
            {
                cursor += 1 + escaped_len;
                continue;
            }

            if tokens[cursor].ch == '`' && backtick_run_len(tokens, cursor) == skip {
                return true;
            }

            cursor += 1;
        }
        return false;
    }

    if matches!(
        delimiter,
        Delimiter::SuperscriptMarkdown | Delimiter::SubscriptMarkdown
    ) {
        let marker = match delimiter {
            Delimiter::SuperscriptMarkdown => '^',
            Delimiter::SubscriptMarkdown => '~',
            _ => unreachable!(),
        };
        return locate_script_close(tokens, index + skip, marker).is_some();
    }

    let body_start = index + skip;
    let requires_body = emphasis_requires_body(delimiter);
    let mut cursor = body_start;
    while cursor < tokens.len() {
        if tokens[cursor].ch == '\\'
            && let Some(escaped_len) = escaped_sequence_token_len(tokens, cursor)
        {
            cursor += 1 + escaped_len;
            continue;
        }

        let closes = if delimiter == Delimiter::HighlightMarkdown {
            is_double_equals_delimiter(tokens, cursor)
        } else {
            matches_sequence(tokens, cursor, &close_str)
        };
        if closes && can_close_delimiter(tokens, cursor, delimiter) {
            // Emphasis spans must enclose at least one character; a close
            // sitting immediately after the open (e.g. `**` or `*` `*`) is an
            // empty span and is treated as literal text instead.
            if requires_body && cursor == body_start {
                cursor += 1;
                continue;
            }
            return true;
        }

        cursor += 1;
    }

    false
}
