// @author kongweiguang

use super::projection_builder::*;
use super::*;

impl Editor {
    pub(super) fn collect_paragraph_block(
        cx: &mut Context<Self>,
        lines: &[String],
        start: usize,
    ) -> (Entity<super::Block>, usize) {
        let mut paragraph_lines = vec![lines[start].to_string()];
        let mut index = start + 1;
        while index < lines.len() {
            if (lines[index].trim().is_empty() || looks_like_root_block_start(lines, index))
                && !paragraph_can_continue_through_boundary(&paragraph_lines, lines, index)
            {
                break;
            }
            paragraph_lines.push(lines[index].to_string());
            index += 1;
        }

        (
            native_block(cx, BlockKind::Paragraph, paragraph_lines.join("\n")),
            index,
        )
    }

    pub(super) fn collect_quote_block(
        cx: &mut Context<Self>,
        lines: &[String],
        start: usize,
    ) -> (Entity<super::Block>, usize) {
        let end = collect_quote_raw_region(lines, start);
        let region = &lines[start..end];
        let mut dequoted = Vec::with_capacity(region.len());
        for line in region {
            if line.trim().is_empty() {
                dequoted.push(String::new());
                continue;
            }

            let Some(content) = strip_one_quote_level(line) else {
                return (raw_block(cx, region.join("\n")), end);
            };
            dequoted.push(content);
        }

        let Some(block) = Self::build_native_quote_block(cx, &dequoted) else {
            return (raw_block(cx, region.join("\n")), end);
        };

        (block, end)
    }

    pub(super) fn build_native_quote_block(
        cx: &mut Context<Self>,
        lines: &[String],
    ) -> Option<Entity<super::Block>> {
        if let Some(header_index) = lines.iter().position(|line| !line.trim().is_empty())
            && let Some((variant, title)) = CalloutVariant::parse_header_line(&lines[header_index])
        {
            return Self::build_native_callout_block(
                cx,
                &lines[header_index + 1..],
                variant,
                title,
            );
        }

        let mut title_markdown = String::new();
        let mut children = Vec::new();
        let mut index = 0usize;
        let mut pending_blank_lines = 0usize;
        let mut saw_child = false;

        while index < lines.len() {
            let line = &lines[index];
            if line.trim().is_empty() {
                pending_blank_lines += 1;
                index += 1;
                continue;
            }

            if is_table_candidate_line(line) {
                if pending_blank_lines > 0 && (!title_markdown.is_empty() || !children.is_empty()) {
                    append_quote_separator_children(&mut children, pending_blank_lines, cx);
                }
                let table_end = collect_table_candidate_region(lines, index);
                let table_region = &lines[index..table_end];
                if let Some(table) = parse_table_region(table_region) {
                    children.push(Self::new_block(cx, BlockRecord::table(table)));
                } else {
                    children.push(raw_block(cx, table_region.join("\n")));
                }
                saw_child = true;
                pending_blank_lines = 0;
                index = table_end;
                continue;
            }

            if is_footnote_definition_start(line) {
                if pending_blank_lines > 0 && (!title_markdown.is_empty() || !children.is_empty()) {
                    append_quote_separator_children(&mut children, pending_blank_lines, cx);
                }
                let footnote_end = collect_footnote_definition_region(lines, index);
                if let Some(footnote) =
                    build_native_footnote_definition_block(cx, &lines[index..footnote_end])
                {
                    children.push(footnote);
                    saw_child = true;
                    pending_blank_lines = 0;
                    index = footnote_end;
                    continue;
                }
            }

            if let Some((comment, consumed)) = collect_comment_block(cx, lines, index) {
                if pending_blank_lines > 0 && (!title_markdown.is_empty() || !children.is_empty()) {
                    append_quote_separator_children(&mut children, pending_blank_lines, cx);
                }
                children.push(comment);
                saw_child = true;
                pending_blank_lines = 0;
                index = consumed;
                continue;
            }

            if is_block_html_start(line) {
                if pending_blank_lines > 0 && (!title_markdown.is_empty() || !children.is_empty()) {
                    append_quote_separator_children(&mut children, pending_blank_lines, cx);
                }
                let html_end = collect_block_html_region(lines, index);
                children.push(html_or_raw_block(cx, lines[index..html_end].join("\n")));
                saw_child = true;
                pending_blank_lines = 0;
                index = html_end;
                continue;
            }

            if is_display_math_start(line) {
                if pending_blank_lines > 0 && (!title_markdown.is_empty() || !children.is_empty()) {
                    append_quote_separator_children(&mut children, pending_blank_lines, cx);
                }
                let math_end = collect_display_math_region(lines, index);
                children.push(math_or_raw_block(cx, lines[index..math_end].join("\n")));
                saw_child = true;
                pending_blank_lines = 0;
                index = math_end;
                continue;
            }

            if let Some(unsupported_end) = collect_unsupported_quote_region(lines, index) {
                if pending_blank_lines > 0 && (!title_markdown.is_empty() || !children.is_empty()) {
                    append_quote_separator_children(&mut children, pending_blank_lines, cx);
                }
                children.push(raw_block(cx, lines[index..unsupported_end].join("\n")));
                saw_child = true;
                pending_blank_lines = 0;
                index = unsupported_end;
                continue;
            }

            if is_quote_start(line) {
                if pending_blank_lines > 0 && (!title_markdown.is_empty() || !children.is_empty()) {
                    append_quote_separator_children(&mut children, pending_blank_lines, cx);
                }
                let (quote, consumed) = Self::collect_quote_block(cx, lines, index);
                if quote.read(cx).kind() == BlockKind::RawMarkdown {
                    return None;
                }
                children.push(quote);
                saw_child = true;
                pending_blank_lines = 0;
                index = consumed;
                continue;
            }

            if parse_list_marker(line).is_some() {
                if pending_blank_lines > 0 && (!title_markdown.is_empty() || !children.is_empty()) {
                    append_quote_separator_children(&mut children, pending_blank_lines, cx);
                }
                let (list_blocks, consumed) = Self::collect_list_blocks(cx, lines, index);
                if list_blocks
                    .iter()
                    .any(|block| block.read(cx).kind() == BlockKind::RawMarkdown)
                {
                    return None;
                }
                children.extend(list_blocks);
                saw_child = true;
                pending_blank_lines = 0;
                index = consumed;
                continue;
            }

            if parse_opening_fence(line).is_some()
                && let Some((code_block, consumed)) = collect_fenced_code_block(cx, lines, index)
            {
                if pending_blank_lines > 0 && (!title_markdown.is_empty() || !children.is_empty()) {
                    append_quote_separator_children(&mut children, pending_blank_lines, cx);
                }
                children.push(code_block);
                saw_child = true;
                pending_blank_lines = 0;
                index = consumed;
                continue;
            }

            if starts_with_standalone_image_child_paragraph(&lines[index..]) {
                if pending_blank_lines > 0 && (!title_markdown.is_empty() || !children.is_empty()) {
                    append_quote_separator_children(&mut children, pending_blank_lines, cx);
                }
                children.push(standalone_image_block(cx, line.to_string()));
                saw_child = true;
                pending_blank_lines = 0;
                index += 1;
                continue;
            }

            if strip_indented_code_prefix(line).is_some()
                && let Some((code_block, consumed)) = collect_indented_code_block(cx, lines, index)
            {
                if pending_blank_lines > 0 && (!title_markdown.is_empty() || !children.is_empty()) {
                    append_quote_separator_children(&mut children, pending_blank_lines, cx);
                }
                children.push(code_block);
                saw_child = true;
                pending_blank_lines = 0;
                index = consumed;
                continue;
            }

            let mut paragraph_lines = vec![line.clone()];
            index += 1;
            while index < lines.len() {
                let next = &lines[index];
                if next.trim().is_empty()
                    || is_quote_start(next)
                    || parse_list_marker(next).is_some()
                    || parse_opening_fence(next).is_some()
                    || strip_indented_code_prefix(next).is_some()
                    || quote_content_starts_unsupported(lines, index)
                {
                    break;
                }

                paragraph_lines.push(next.clone());
                index += 1;
            }

            if is_standalone_image_paragraph(&paragraph_lines) {
                if pending_blank_lines > 0 && (!title_markdown.is_empty() || !children.is_empty()) {
                    append_quote_separator_children(&mut children, pending_blank_lines, cx);
                }
                children.push(standalone_image_block(cx, paragraph_lines.join("\n")));
                saw_child = true;
                pending_blank_lines = 0;
                continue;
            }

            if saw_child {
                if pending_blank_lines > 0 && (!title_markdown.is_empty() || !children.is_empty()) {
                    append_quote_separator_children(&mut children, pending_blank_lines, cx);
                }
                children.push(native_block(
                    cx,
                    BlockKind::Paragraph,
                    paragraph_lines.join("\n"),
                ));
                pending_blank_lines = 0;
                continue;
            }

            if !title_markdown.is_empty() {
                title_markdown.push_str(if pending_blank_lines > 0 {
                    "\n\n"
                } else {
                    "\n"
                });
            }
            title_markdown.push_str(&paragraph_lines.join("\n"));
            pending_blank_lines = 0;
        }

        if pending_blank_lines > 0 && (!title_markdown.is_empty() || !children.is_empty()) {
            append_quote_separator_children(&mut children, pending_blank_lines, cx);
        }

        let block = native_block(cx, BlockKind::Quote, title_markdown);
        attach_child_blocks(&block, children, cx);
        Some(block)
    }

    pub(super) fn build_native_callout_block(
        cx: &mut Context<Self>,
        lines: &[String],
        variant: CalloutVariant,
        title: String,
    ) -> Option<Entity<super::Block>> {
        let mut children = Vec::new();
        let mut index = 0usize;
        let mut pending_blank_lines = 0usize;

        while index < lines.len() {
            let line = &lines[index];
            if line.trim().is_empty() {
                pending_blank_lines += 1;
                index += 1;
                continue;
            }

            if pending_blank_lines > 0 {
                append_quote_separator_children(&mut children, pending_blank_lines, cx);
                pending_blank_lines = 0;
            }

            if is_table_candidate_line(line) {
                let table_end = collect_table_candidate_region(lines, index);
                let table_region = &lines[index..table_end];
                if let Some(table) = parse_table_region(table_region) {
                    children.push(Self::new_block(cx, BlockRecord::table(table)));
                } else {
                    children.push(raw_block(cx, table_region.join("\n")));
                }
                index = table_end;
                continue;
            }

            if is_footnote_definition_start(line) {
                let footnote_end = collect_footnote_definition_region(lines, index);
                if let Some(footnote) =
                    build_native_footnote_definition_block(cx, &lines[index..footnote_end])
                {
                    children.push(footnote);
                    index = footnote_end;
                    continue;
                }
            }

            if let Some((comment, consumed)) = collect_comment_block(cx, lines, index) {
                children.push(comment);
                index = consumed;
                continue;
            }

            if is_block_html_start(line) {
                let html_end = collect_block_html_region(lines, index);
                children.push(html_or_raw_block(cx, lines[index..html_end].join("\n")));
                index = html_end;
                continue;
            }

            if is_display_math_start(line) {
                let math_end = collect_display_math_region(lines, index);
                children.push(math_or_raw_block(cx, lines[index..math_end].join("\n")));
                index = math_end;
                continue;
            }

            if let Some(unsupported_end) = collect_unsupported_quote_region(lines, index) {
                children.push(raw_block(cx, lines[index..unsupported_end].join("\n")));
                index = unsupported_end;
                continue;
            }

            if is_quote_start(line) {
                let (quote, consumed) = Self::collect_quote_block(cx, lines, index);
                if quote.read(cx).kind() == BlockKind::RawMarkdown {
                    return None;
                }
                children.push(quote);
                index = consumed;
                continue;
            }

            if parse_list_marker(line).is_some() {
                let (list_blocks, consumed) = Self::collect_list_blocks(cx, lines, index);
                if list_blocks
                    .iter()
                    .any(|block| block.read(cx).kind() == BlockKind::RawMarkdown)
                {
                    return None;
                }
                children.extend(list_blocks);
                index = consumed;
                continue;
            }

            if parse_opening_fence(line).is_some()
                && let Some((code_block, consumed)) = collect_fenced_code_block(cx, lines, index)
            {
                children.push(code_block);
                index = consumed;
                continue;
            }

            if starts_with_standalone_image_child_paragraph(&lines[index..]) {
                children.push(standalone_image_block(cx, line.to_string()));
                index += 1;
                continue;
            }

            if strip_indented_code_prefix(line).is_some()
                && let Some((code_block, consumed)) = collect_indented_code_block(cx, lines, index)
            {
                children.push(code_block);
                index = consumed;
                continue;
            }

            let mut paragraph_lines = vec![line.clone()];
            index += 1;
            while index < lines.len() {
                let next = &lines[index];
                if next.trim().is_empty()
                    || is_quote_start(next)
                    || parse_list_marker(next).is_some()
                    || parse_opening_fence(next).is_some()
                    || strip_indented_code_prefix(next).is_some()
                    || quote_content_starts_unsupported(lines, index)
                {
                    break;
                }

                paragraph_lines.push(next.clone());
                index += 1;
            }

            children.push(native_block(
                cx,
                BlockKind::Paragraph,
                paragraph_lines.join("\n"),
            ));
        }

        if pending_blank_lines > 0 {
            append_quote_separator_children(&mut children, pending_blank_lines, cx);
        }

        let block = Editor::new_block(
            cx,
            BlockRecord::new(
                BlockKind::Callout(variant),
                InlineTextTree::from_markdown(&title),
            ),
        );
        attach_child_blocks(&block, children, cx);
        Some(block)
    }

    /// 复杂列表保留源码起点，子块与续行的收集不能把列表标记降成无编号类型。
    pub(super) fn collect_list_blocks(
        cx: &mut Context<Self>,
        lines: &[String],
        start: usize,
    ) -> (Vec<Entity<super::Block>>, usize) {
        let mut roots = Vec::new();
        let mut index = start;

        while index < lines.len() {
            let Some(marker) = parse_list_marker(&lines[index]) else {
                break;
            };

            let item_end = collect_list_item_region(lines, index, marker.indent_columns);
            let mut record = native_record(marker.kind.clone(), marker.text);
            record.ordered_list_start = marker.ordered_start;
            let block = Editor::new_block(cx, record);
            let mut body_index = index + 1;
            let mut pending_blank_lines = 0usize;
            let mut fallback_raw = false;
            let mut saw_child = false;

            while body_index < item_end {
                let line = &lines[body_index];
                if line.trim().is_empty() {
                    pending_blank_lines += 1;
                    body_index += 1;
                    continue;
                }

                let (line_indent_columns, _) = leading_indent_columns_and_bytes(line);
                if line_indent_columns > marker.indent_columns {
                    let anchor_dedented =
                        dedent_lines(&lines[body_index..item_end], line_indent_columns);

                    if parse_list_marker(&anchor_dedented[0]).is_some() {
                        let (children, consumed) =
                            Self::collect_list_blocks(cx, &anchor_dedented, 0);
                        attach_child_blocks(&block, children, cx);
                        body_index += consumed;
                        pending_blank_lines = 0;
                        saw_child = true;
                        continue;
                    }

                    if is_quote_start(&anchor_dedented[0]) {
                        let (quote, consumed) = Self::collect_quote_block(cx, &anchor_dedented, 0);
                        if quote.read(cx).kind() == BlockKind::RawMarkdown {
                            fallback_raw = true;
                            break;
                        }

                        attach_child_blocks(&block, vec![quote], cx);
                        body_index += consumed;
                        pending_blank_lines = 0;
                        saw_child = true;
                        continue;
                    }

                    if parse_opening_fence(&anchor_dedented[0]).is_some()
                        && let Some((code_block, consumed)) =
                            collect_fenced_code_block(cx, &anchor_dedented, 0)
                    {
                        attach_child_blocks(&block, vec![code_block], cx);
                        body_index += consumed;
                        pending_blank_lines = 0;
                        saw_child = true;
                        continue;
                    }

                    if is_root_table_candidate_line(&anchor_dedented[0]) {
                        let table_end = collect_root_table_candidate_region(&anchor_dedented, 0);
                        let table_region = &anchor_dedented[..table_end];
                        let child = if let Some(table) = parse_root_table_region(table_region) {
                            Self::new_block(cx, BlockRecord::table(table))
                        } else {
                            raw_block(cx, table_region.join("\n"))
                        };
                        attach_child_blocks(&block, vec![child], cx);
                        body_index += table_end;
                        pending_blank_lines = 0;
                        saw_child = true;
                        continue;
                    }

                    if starts_with_standalone_image_child_paragraph(&anchor_dedented) {
                        attach_child_blocks(
                            &block,
                            vec![standalone_image_block(cx, anchor_dedented[0].clone())],
                            cx,
                        );
                        body_index += 1;
                        pending_blank_lines = 0;
                        saw_child = true;
                        continue;
                    }

                    if line_indent_columns >= marker.content_indent_columns {
                        let content_dedented = dedent_lines(
                            &lines[body_index..item_end],
                            marker.content_indent_columns,
                        );
                        if strip_indented_code_prefix(&content_dedented[0]).is_some() {
                            let Some((code_block, consumed)) =
                                collect_indented_code_block(cx, &content_dedented, 0)
                            else {
                                unreachable!(
                                    "indented code prefix disappeared after child detection"
                                );
                            };

                            attach_child_blocks(&block, vec![code_block], cx);
                            body_index += consumed;
                            pending_blank_lines = 0;
                            saw_child = true;
                            continue;
                        }
                    }

                    if is_reference_definition_start(&anchor_dedented[0]) {
                        let consumed = collect_reference_definition_region(&anchor_dedented, 0);
                        attach_child_blocks(
                            &block,
                            vec![raw_block(cx, anchor_dedented[..consumed].join("\n"))],
                            cx,
                        );
                        body_index += consumed;
                        pending_blank_lines = 0;
                        saw_child = true;
                        continue;
                    }

                    if let Some((comment, consumed)) =
                        collect_comment_block(cx, &anchor_dedented, 0)
                    {
                        attach_child_blocks(&block, vec![comment], cx);
                        body_index += consumed;
                        pending_blank_lines = 0;
                        saw_child = true;
                        continue;
                    }

                    if is_block_html_start(&anchor_dedented[0]) {
                        let consumed = collect_block_html_region(&anchor_dedented, 0);
                        attach_child_blocks(
                            &block,
                            vec![html_or_raw_block(
                                cx,
                                anchor_dedented[..consumed].join("\n"),
                            )],
                            cx,
                        );
                        body_index += consumed;
                        pending_blank_lines = 0;
                        saw_child = true;
                        continue;
                    }

                    if is_footnote_definition_start(&anchor_dedented[0]) {
                        let consumed = collect_footnote_definition_region(&anchor_dedented, 0);
                        attach_child_blocks(
                            &block,
                            vec![raw_block(cx, anchor_dedented[..consumed].join("\n"))],
                            cx,
                        );
                        body_index += consumed;
                        pending_blank_lines = 0;
                        saw_child = true;
                        continue;
                    }

                    if is_display_math_start(&anchor_dedented[0]) {
                        let consumed = collect_display_math_region(&anchor_dedented, 0);
                        attach_child_blocks(
                            &block,
                            vec![math_or_raw_block(
                                cx,
                                anchor_dedented[..consumed].join("\n"),
                            )],
                            cx,
                        );
                        body_index += consumed;
                        pending_blank_lines = 0;
                        saw_child = true;
                        continue;
                    }

                    let should_promote_plain_child = pending_blank_lines > 0
                        || saw_child
                        || block.read(cx).display_text().is_empty()
                        || parse_standalone_image(&block.read(cx).record.title_markdown())
                            .is_some();
                    if should_promote_plain_child {
                        let (paragraph, consumed) =
                            Self::collect_paragraph_block(cx, &anchor_dedented, 0);
                        attach_child_blocks(&block, vec![paragraph], cx);
                        body_index += consumed;
                        pending_blank_lines = 0;
                        saw_child = true;
                        continue;
                    }
                }

                if line_indent_columns >= marker.content_indent_columns {
                    let content_dedented =
                        dedent_lines(&lines[body_index..item_end], marker.content_indent_columns);
                    if strip_indented_code_prefix(&content_dedented[0]).is_some() {
                        let Some((code_block, consumed)) =
                            collect_indented_code_block(cx, &content_dedented, 0)
                        else {
                            unreachable!("indented code prefix disappeared after detection");
                        };

                        attach_child_blocks(&block, vec![code_block], cx);
                        body_index += consumed;
                        pending_blank_lines = 0;
                        saw_child = true;
                        continue;
                    }
                }

                let trimmed = line.trim_start_matches([' ', '\t']);
                append_markdown_to_block(
                    &block,
                    if pending_blank_lines > 0 {
                        "\n\n"
                    } else {
                        "\n"
                    },
                    trimmed,
                    cx,
                );
                pending_blank_lines = 0;
                body_index += 1;
            }

            if fallback_raw {
                roots.push(raw_block(cx, lines[index..item_end].join("\n")));
            } else {
                roots.push(block);
            }
            index = item_end;
        }

        (roots, index)
    }
}
