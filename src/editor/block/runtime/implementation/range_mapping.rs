// @author kongweiguang

use super::*;

impl Block {
    pub(crate) fn markdown_range_to_current_range(&self, range: Range<usize>) -> Range<usize> {
        if self.uses_raw_text_editing() || self.kind().is_code_block() {
            let len = self.visible_len();
            return range.start.min(len)..range.end.min(len);
        }

        let clean_range = self
            .record
            .title
            .markdown_offset_map()
            .markdown_to_visible_range(range);
        self.clean_to_current_range(clean_range)
    }

    pub(crate) fn markdown_offset_to_current_offset(&self, offset: usize) -> usize {
        self.markdown_range_to_current_range(offset..offset).start
    }

    pub(crate) fn prepare_undo_capture(&self, kind: UndoCaptureKind, cx: &mut Context<Self>) {
        cx.emit(BlockEvent::PrepareUndo { kind });
    }

    pub(in super::super) fn utf16_to_utf8_in(text: &str, offset: usize) -> usize {
        let mut utf8_offset = 0;
        let mut utf16_count = 0;

        for ch in text.chars() {
            if utf16_count >= offset {
                break;
            }
            utf16_count += ch.len_utf16();
            utf8_offset += ch.len_utf8();
        }

        utf8_offset
    }

    pub(in super::super) fn utf8_to_utf16_in(text: &str, offset: usize) -> usize {
        let mut utf16_offset = 0;
        let mut utf8_count = 0;

        for ch in text.chars() {
            if utf8_count >= offset {
                break;
            }
            utf8_count += ch.len_utf8();
            utf16_offset += ch.len_utf16();
        }

        utf16_offset
    }

    pub(in super::super) fn utf16_range_to_utf8_in(
        text: &str,
        range_utf16: &Range<usize>,
    ) -> Range<usize> {
        Self::utf16_to_utf8_in(text, range_utf16.start)
            ..Self::utf16_to_utf8_in(text, range_utf16.end)
    }

    pub(in super::super) fn utf8_range_to_utf16_in(
        text: &str,
        range: &Range<usize>,
    ) -> Range<usize> {
        Self::utf8_to_utf16_in(text, range.start)..Self::utf8_to_utf16_in(text, range.end)
    }

    /// Detect Markdown shortcut prefixes in the edited title and convert the
    /// block's kind accordingly (e.g. `"- " -> BulletedListItem`).
    ///
    /// Only triggers when the current kind is [`BlockKind::Paragraph`].
    /// Returns the potentially updated kind, the title with prefix stripped,
    /// the new cursor offset, and the number of prefix characters removed.
    fn normalize_after_title_edit(
        &self,
        mut next_title: InlineTextTree,
        cursor: usize,
    ) -> (BlockKind, InlineTextTree, usize, usize) {
        if self.is_table_cell() {
            return (self.kind(), next_title, cursor, 0);
        }

        if !self.uses_raw_text_editing() && self.kind() == BlockKind::Paragraph {
            let visible_text = next_title.visible_text();
            if let Some((kind, prefix_len)) = BlockKind::detect_markdown_shortcut(&visible_text) {
                next_title.remove_visible_prefix(prefix_len);
                return (
                    kind,
                    next_title,
                    cursor.saturating_sub(prefix_len),
                    prefix_len,
                );
            }
        }

        if !self.uses_raw_text_editing() && self.kind() == BlockKind::BulletedListItem {
            let visible_text = next_title.visible_text();
            if let Some((checked, prefix_len)) =
                BlockKind::parse_task_list_item_prefix(&visible_text)
            {
                next_title.remove_visible_prefix(prefix_len);
                return (
                    BlockKind::TaskListItem { checked },
                    next_title,
                    cursor.saturating_sub(prefix_len),
                    prefix_len,
                );
            }
        }

        (self.kind(), next_title, cursor, 0)
    }

    fn quote_line_starts_block_syntax(line: &str) -> bool {
        let trimmed_end = line.trim_end();
        if trimmed_end.is_empty() {
            return false;
        }

        let leading_spaces = trimmed_end.bytes().take_while(|b| *b == b' ').count();
        if leading_spaces >= 4 {
            return true;
        }

        BlockKind::detect_markdown_shortcut(&format!("{trimmed_end} "))
            .is_some_and(|(kind, _)| kind != BlockKind::Paragraph)
            || BlockKind::parse_code_fence_opening(trimmed_end).is_some()
            || BlockKind::parse_separator_line(trimmed_end)
            || BlockKind::parse_atx_heading_line(trimmed_end).is_some()
    }

    pub(super) fn multiline_quote_edit_requires_reparse(text: &str) -> bool {
        text.split('\n')
            .skip(1)
            .any(Self::quote_line_starts_block_syntax)
    }

    fn adjust_range_for_shortcut(range: &Range<usize>, removed_prefix_len: usize) -> Range<usize> {
        range.start.saturating_sub(removed_prefix_len)..range.end.saturating_sub(removed_prefix_len)
    }

    fn projected_styles_touching_display_range(
        &self,
        display_range: &Range<usize>,
    ) -> Vec<(usize, StyleFlag)> {
        let mut targets = Vec::new();
        for segment in self.projection_segments() {
            let touches = display_range.start < segment.display_range.end
                && segment.display_range.start < display_range.end;
            if touches
                && matches!(
                    segment.kind,
                    ExpandedInlineSegmentKind::OpeningDelimiter(_)
                        | ExpandedInlineSegmentKind::ClosingDelimiter(_)
                )
            {
                let kind = match segment.kind {
                    ExpandedInlineSegmentKind::OpeningDelimiter(kind)
                    | ExpandedInlineSegmentKind::ClosingDelimiter(kind) => kind,
                    _ => continue,
                };
                if let Some(flag) = kind.style_flag() {
                    let target = (segment.fragment_index, flag);
                    if !targets.contains(&target) {
                        targets.push(target);
                    }
                }
            }
        }
        targets
    }

    fn clean_offset_before_fragment_index(fragments: &[InlineFragment], index: usize) -> usize {
        fragments
            .iter()
            .take(index)
            .map(|fragment| fragment.text.len())
            .sum()
    }

    fn replacement_is_pure_link_run(fragments: &[InlineFragment]) -> bool {
        let Some(first_link) = fragments
            .first()
            .and_then(|fragment| fragment.link.as_ref())
        else {
            return false;
        };

        fragments
            .iter()
            .all(|fragment| fragment.link.as_ref() == Some(first_link))
    }

    /// 链接标签维持既有投影选区；纯链接快速路径也须先拒绝只读，避免绕过 title 写边界。
    fn apply_link_projection_edit(
        &mut self,
        link_run: &ExpandedLinkRun,
        visible_range: Range<usize>,
        new_text: &str,
        selected_range_relative: Option<Range<usize>>,
        mark_inserted_text: bool,
        cx: &mut Context<Self>,
    ) {
        if self.is_read_only() {
            return;
        }
        let local_visible_range = visible_range.start - link_run.display_range.start
            ..visible_range.end - link_run.display_range.start;
        let local_display_text = self.display_text()[link_run.display_range.clone()].to_string();
        let local_tree = InlineTextTree::plain(local_display_text);
        let local_result = local_tree.replace_visible_range_with_link_references(
            local_visible_range.clone(),
            new_text,
            InlineInsertionAttributes::default(),
            &self.link_reference_definitions,
        );
        let replacement_fragments = local_result.tree.fragments.clone();

        let replacement_start = link_run.start_fragment_index;
        let replacement_clean_start = Self::clean_offset_before_fragment_index(
            &self.record.title.fragments,
            replacement_start,
        );
        let mut next_title = self.record.title.clone();
        next_title.replace_fragment_range(
            link_run.start_fragment_index..link_run.end_fragment_index,
            replacement_fragments.clone(),
        );

        if Self::replacement_is_pure_link_run(&replacement_fragments) {
            let old_kind = self.record.kind.clone();
            let old_title = self.record.title.clone();
            self.record.set_title(next_title.clone());
            self.sync_edit_mode_from_kind();
            self.sync_render_cache();

            let replacement_visible_len = replacement_fragments
                .iter()
                .map(|fragment| fragment.text.len())
                .sum::<usize>();
            let selected_clean =
                replacement_clean_start..replacement_clean_start + replacement_visible_len;
            self.rebuild_inline_projection(selected_clean.clone(), None);

            let local_selected = selected_range_relative.clone().unwrap_or_else(|| {
                let cursor = local_visible_range.start + new_text.len();
                cursor..cursor
            });
            if let Some(projected_link_run) = self.projection.as_ref().and_then(|projection| {
                projection
                    .link_runs
                    .iter()
                    .find(|run| run.clean_range == selected_clean)
            }) {
                let start = projected_link_run.display_range.start
                    + local_selected
                        .start
                        .min(projected_link_run.display_range.len());
                let end = projected_link_run.display_range.start
                    + local_selected
                        .end
                        .min(projected_link_run.display_range.len());
                self.selected_range = start..end;
                self.selection_reversed = false;
                self.marked_range = if mark_inserted_text && !new_text.is_empty() {
                    Some(start..end)
                } else {
                    None
                };
                self.collapsed_caret_affinity = CollapsedCaretAffinity::Default;
                self.cursor_blink_epoch = Instant::now();
                self.clear_vertical_motion();
                if self.record.kind != old_kind || self.record.title != old_title {
                    cx.emit(BlockEvent::Changed);
                }
                cx.notify();
                return;
            }
        }

        let local_selected = selected_range_relative.as_ref().map(|relative| {
            let absolute = local_visible_range.start + relative.start
                ..local_visible_range.start + relative.end;
            local_result.map_range(&absolute)
        });
        let cursor = local_selected
            .as_ref()
            .map(|range| range.end)
            .unwrap_or_else(|| local_result.map_offset(local_visible_range.start + new_text.len()));
        let prefix = replacement_clean_start;
        let selected_clean = local_selected.map(|range| prefix + range.start..prefix + range.end);
        let marked_clean = if mark_inserted_text && !new_text.is_empty() {
            let inserted_range =
                local_visible_range.start..local_visible_range.start + new_text.len();
            let mapped = local_result.map_range(&inserted_range);
            Some(prefix + mapped.start..prefix + mapped.end)
        } else {
            None
        };
        self.apply_title_edit(
            next_title,
            prefix + cursor,
            marked_clean,
            selected_clean.clone(),
            selected_clean
                .as_ref()
                .and_then(|range| (!range.is_empty()).then_some(false)),
            false,
            false,
            cx,
        );
    }

    fn insertion_attributes_for_current_offset(
        &self,
        current_offset: usize,
    ) -> InlineInsertionAttributes {
        if self.uses_raw_text_editing() {
            return InlineInsertionAttributes::default();
        }

        if self.projection.is_none() {
            return self
                .record
                .title
                .attributes_for_insertion_at(current_offset);
        }

        for segment in self.projection_segments() {
            match segment.kind {
                ExpandedInlineSegmentKind::StyledText
                    if current_offset >= segment.display_range.start
                        && current_offset <= segment.display_range.end =>
                {
                    let fragment = &self.record.title.fragments[segment.fragment_index];
                    return InlineInsertionAttributes {
                        style: fragment.style,
                        html_style: fragment.html_style,
                        link: fragment.link.clone(),
                        footnote: fragment.footnote.clone(),
                        math: None,
                    };
                }
                ExpandedInlineSegmentKind::OpeningDelimiter(_)
                    if current_offset == segment.display_range.end =>
                {
                    let fragment = &self.record.title.fragments[segment.fragment_index];
                    return InlineInsertionAttributes {
                        style: fragment.style,
                        html_style: fragment.html_style,
                        link: fragment.link.clone(),
                        footnote: fragment.footnote.clone(),
                        math: None,
                    };
                }
                ExpandedInlineSegmentKind::ClosingDelimiter(_)
                    if current_offset == segment.display_range.start =>
                {
                    let fragment = &self.record.title.fragments[segment.fragment_index];
                    return InlineInsertionAttributes {
                        style: fragment.style,
                        html_style: fragment.html_style,
                        link: fragment.link.clone(),
                        footnote: fragment.footnote.clone(),
                        math: None,
                    };
                }
                // Caret just outside a span: after a closing delimiter or before
                // an opening one. Insert plain text so it isn't absorbed back into
                // the span, matching how code and strikethrough already behave.
                ExpandedInlineSegmentKind::ClosingDelimiter(_)
                    if current_offset == segment.display_range.end =>
                {
                    return InlineInsertionAttributes::default();
                }
                ExpandedInlineSegmentKind::OpeningDelimiter(_)
                    if current_offset == segment.display_range.start =>
                {
                    return InlineInsertionAttributes::default();
                }
                ExpandedInlineSegmentKind::LinkTargetText => {
                    if let Some(link_group) = segment.link_group
                        && let Some(link_run) = self
                            .projection
                            .as_ref()
                            .and_then(|projection| projection.link_runs.get(link_group))
                        && current_offset >= link_run.target_display_range.start
                        && current_offset <= link_run.target_display_range.end
                    {
                        return InlineInsertionAttributes::default();
                    }
                }
                _ => {}
            }
        }

        self.record
            .title
            .attributes_for_insertion_at(self.current_to_clean_offset(current_offset))
    }

    fn attributes_for_fragment(fragment: &InlineFragment) -> InlineInsertionAttributes {
        InlineInsertionAttributes {
            style: fragment.style,
            html_style: fragment.html_style,
            link: fragment.link.clone(),
            footnote: fragment.footnote.clone(),
            math: None,
        }
    }

    fn replacement_attributes_for_visible_range(
        &self,
        visible_range: &Range<usize>,
    ) -> InlineInsertionAttributes {
        if self.uses_raw_text_editing() {
            return InlineInsertionAttributes::default();
        }

        if visible_range.is_empty() {
            return self.insertion_attributes_for_current_offset(visible_range.start);
        }

        if self.projection.is_some() {
            return self
                .projected_replacement_attributes_for_visible_range(visible_range)
                .unwrap_or_default();
        }

        self.fragment_attributes_for_clean_range(self.current_to_clean_range(visible_range.clone()))
            .unwrap_or_default()
    }

    fn projected_replacement_attributes_for_visible_range(
        &self,
        visible_range: &Range<usize>,
    ) -> Option<InlineInsertionAttributes> {
        self.projection_segments().iter().find_map(|segment| {
            (segment.kind == ExpandedInlineSegmentKind::StyledText
                && segment.display_range.start <= visible_range.start
                && visible_range.end <= segment.display_range.end)
                .then(|| {
                    self.record
                        .title
                        .fragments
                        .get(segment.fragment_index)
                        .map(Self::attributes_for_fragment)
                })
                .flatten()
        })
    }

    fn fragment_attributes_for_clean_range(
        &self,
        clean_range: Range<usize>,
    ) -> Option<InlineInsertionAttributes> {
        if clean_range.is_empty() {
            return None;
        }

        let mut cursor = 0usize;
        for fragment in &self.record.title.fragments {
            let fragment_start = cursor;
            let fragment_end = fragment_start + fragment.text.len();
            if fragment_start <= clean_range.start && clean_range.end <= fragment_end {
                return Some(Self::attributes_for_fragment(fragment));
            }
            cursor = fragment_end;
        }

        None
    }

    pub(in super::super) fn collapsed_caret_inherits_inline_code_style(&self) -> bool {
        self.selected_range.is_empty()
            && !self.uses_raw_text_editing()
            && self
                .insertion_attributes_for_current_offset(self.cursor_offset())
                .style
                .code
    }

    /// Apply a new title to the block, running shortcut detection and
    /// updating the render cache, cursor, and selection state.  Emits
    /// [`BlockEvent::Changed`] if the kind or title actually changed.
    /// 标记被解析吸收时需要重建投影，但原有闭合符仍在光标右侧时必须留在样式内。
    /// 只读在真正改动 title 前再次校验，阻止迟到浮层提交先污染投影再被上层拒绝。
    pub(in super::super) fn apply_title_edit(
        &mut self,
        next_title: InlineTextTree,
        cursor_clean: usize,
        marked_range_clean: Option<Range<usize>>,
        selected_range_clean: Option<Range<usize>>,
        selected_range_reversed: Option<bool>,
        caret_may_have_closed_span: bool,
        caret_before_existing_closer: bool,
        cx: &mut Context<Self>,
    ) {
        if self.is_read_only() {
            return;
        }
        let old_kind = self.record.kind.clone();
        let old_title = self.record.title.clone();
        let old_title_was_empty = old_title.visible_text().is_empty();
        let mut collapsed_affinity = self.current_collapsed_caret_affinity();
        let keep_projection =
            self.projection.is_some() && self.edit_mode.supports_inline_projection();

        let (next_kind, normalized_title, adjusted_cursor, shortcut_removed_len) =
            self.normalize_after_title_edit(next_title, cursor_clean);
        let should_restart_numbered_list = old_kind == BlockKind::Paragraph
            && old_title_was_empty
            && self.list_group_separator_candidate
            && next_kind == BlockKind::NumberedListItem;

        let next_marked_clean = marked_range_clean
            .as_ref()
            .map(|range| Self::adjust_range_for_shortcut(range, shortcut_removed_len));
        let next_selected_clean = selected_range_clean
            .as_ref()
            .map(|range| Self::adjust_range_for_shortcut(range, shortcut_removed_len))
            .unwrap_or_else(|| adjusted_cursor..adjusted_cursor);

        self.record.kind = next_kind;
        self.record.set_title(normalized_title);
        self.numbered_list_restart_requested = should_restart_numbered_list;
        self.sync_edit_mode_from_kind();
        self.sync_render_cache();
        // Rebuild when a projection already existed, or when this edit may have
        // closed a delimiter, creating a span whose markers now need projecting.
        if self.edit_mode.supports_inline_projection()
            && (keep_projection || caret_may_have_closed_span)
        {
            self.rebuild_inline_projection(next_selected_clean.clone(), next_marked_clean.clone());
        }

        // If the edit closed a span (its delimiters were absorbed), place the
        // caret after the new closing marker so typing continues as plain text.
        if caret_may_have_closed_span
            && !caret_before_existing_closer
            && next_selected_clean.is_empty()
            && self.projection.as_ref().is_some_and(|projection| {
                projection.caret_closes_span_at_clean(next_selected_clean.start)
            })
        {
            collapsed_affinity = CollapsedCaretAffinity::OuterEnd;
        }

        self.marked_range = next_marked_clean
            .clone()
            .map(|range| self.clean_to_current_range(range));
        if next_selected_clean.is_empty() {
            let offset = self.clean_to_current_cursor_offset_with_affinity(
                next_selected_clean.start,
                collapsed_affinity,
            );
            self.assign_collapsed_selection_offset(offset, collapsed_affinity, None);
        } else {
            self.selected_range = self.clean_to_current_range(next_selected_clean);
            self.selection_reversed = selected_range_reversed.unwrap_or(self.selection_reversed);
            self.collapsed_caret_affinity = CollapsedCaretAffinity::Default;
        }
        self.cursor_blink_epoch = Instant::now();
        self.clear_vertical_motion();

        if self.record.kind != old_kind || self.record.title != old_title {
            cx.emit(BlockEvent::Changed);
        }
        cx.notify();
    }

    /// Replace text in visible coordinates: splice `new_text` into the title
    /// at `visible_range`, re-parse inline markers, and update cursor state.
    /// When `mark_inserted_text` is true the inserted text becomes the IME
    /// marked range.
    ///
    /// When the block is in editing-expansion mode (code spans show `` ` ``
    /// delimiters), the `visible_range` is first mapped back to the original
    /// tree's offset space.
    /// 通过归一化映射识别光标右侧被吸收的已有标记，区分输入正文与输入闭合符。
    pub(crate) fn replace_text_in_visible_range(
        &mut self,
        visible_range: Range<usize>,
        new_text: &str,
        selected_range_relative: Option<Range<usize>>,
        mark_inserted_text: bool,
        cx: &mut Context<Self>,
    ) {
        self.replace_text_in_visible_range_with_direction(
            visible_range,
            new_text,
            selected_range_relative,
            mark_inserted_text,
            None,
            cx,
        );
    }

    /// Applies a replacement with an optional anchor direction for operations that move whole raw rows.
    pub(crate) fn replace_text_in_visible_range_with_direction(
        &mut self,
        visible_range: Range<usize>,
        new_text: &str,
        selected_range_relative: Option<Range<usize>>,
        mark_inserted_text: bool,
        selected_range_reversed: Option<bool>,
        cx: &mut Context<Self>,
    ) {
        if self.kind().is_separator() && !self.uses_raw_text_editing() {
            return;
        }

        let inserted_attributes = self.replacement_attributes_for_visible_range(&visible_range);

        // Inline `[label](url)` links round-trip through their projected source,
        // so edit them via the link projection even when the block is otherwise
        // source-preserving (for example it also contains inline math). This keeps
        // a link's anchor text editable the same way in every block; reference and
        // autolink links stay on the markdown-space path below, which preserves
        // their original source spelling.
        if !self.uses_raw_text_editing()
            && let Some(link_run) = self
                .projected_link_run_fully_covering_range(&visible_range)
                .filter(|run| !run.link.is_source_preserving())
                .cloned()
        {
            self.apply_link_projection_edit(
                &link_run,
                visible_range,
                new_text,
                selected_range_relative,
                mark_inserted_text,
                cx,
            );
            return;
        }

        if self.should_use_markdown_space_link_edit() {
            self.apply_markdown_space_title_edit(
                visible_range,
                new_text,
                selected_range_relative,
                mark_inserted_text,
                cx,
            );
            return;
        }

        // Editing outside an inline link's run would otherwise re-derive the
        // inline tree from collapsed visible text, which no longer contains the
        // `[label](url)` markers and silently drops the link. Edit in markdown
        // space (as source-preserving links already do) so the link round-trips.
        if !self.uses_raw_text_editing() && self.record.title.has_inline_links() {
            self.apply_markdown_space_title_edit(
                visible_range,
                new_text,
                selected_range_relative,
                mark_inserted_text,
                cx,
            );
            return;
        }

        if self.kind() == BlockKind::Paragraph && !self.uses_raw_text_editing() {
            let mut literal = self.display_text().to_owned();
            literal.replace_range(visible_range.clone(), new_text);
            if BlockKind::parse_code_fence_opening(&literal).is_some() {
                // 段首围栏在输入语言首字母时不能进入行内代码投影；否则 ```j
                // 会被折叠为 `j`，Enter 再也无法识别代码块。这里保持字面文本，
                // 直到 on_newline 原子地把整个前缀转换为 CodeBlock。
                let inserted = visible_range.start..visible_range.start + new_text.len();
                let selected_range = selected_range_relative.as_ref().map(|relative| {
                    visible_range.start + relative.start..visible_range.start + relative.end
                });
                let cursor = selected_range
                    .as_ref()
                    .map(|range| range.end)
                    .unwrap_or(inserted.end);
                let marked_range =
                    (mark_inserted_text && !new_text.is_empty()).then_some(inserted.clone());
                self.apply_title_edit(
                    InlineTextTree::plain(literal),
                    cursor,
                    marked_range,
                    selected_range,
                    Some(false),
                    false,
                    false,
                    cx,
                );
                return;
            }
        }

        let clean_range = self.current_to_clean_range(visible_range.clone());
        let mut base_title = self.record.title.clone();
        let overlaps_delimiters = self.projection.is_some() && !self.uses_raw_text_editing();
        if overlaps_delimiters {
            let touched_styles = self.projected_styles_touching_display_range(&visible_range);
            if !touched_styles.is_empty() {
                base_title.unwrap_styles_on_fragments(&touched_styles);
            }
        }

        let base_visible_len = base_title.visible_text().len();
        let replaced_text = self.display_text()[visible_range.clone()].to_string();
        let result = if self.uses_raw_text_editing() {
            base_title.replace_visible_range_raw(
                clean_range.clone(),
                new_text,
                InlineInsertionAttributes::default(),
            )
        } else {
            base_title.replace_visible_range_with_link_references(
                clean_range.clone(),
                new_text,
                inserted_attributes,
                &self.link_reference_definitions,
            )
        };

        // A span was closed when re-parsing absorbed delimiters into a style,
        // leaving the clean text shorter than expected. Skip IME and deletions.
        let expected_visible_len =
            base_visible_len.saturating_sub(clean_range.len()) + new_text.len();
        let caret_may_have_closed_span = !self.uses_raw_text_editing()
            && !new_text.is_empty()
            && !mark_inserted_text
            && result.tree.visible_text().len() < expected_visible_len;
        let quote_structure_edit = !self.uses_raw_text_editing()
            && self.quote_depth > 0
            && (new_text.contains('\n')
                || replaced_text.contains('\n')
                || (self.kind() == BlockKind::Quote
                    && Self::multiline_quote_edit_requires_reparse(&result.tree.visible_text())));
        if quote_structure_edit {
            self.quote_reparse_requested = true;
        }
        let inserted_range = clean_range.start..clean_range.start + new_text.len();
        let marked_range = if mark_inserted_text && !new_text.is_empty() {
            Some(result.map_range(&inserted_range))
        } else {
            None
        };
        let selected_range = selected_range_relative.as_ref().map(|relative| {
            let absolute = clean_range.start + relative.start..clean_range.start + relative.end;
            result.map_range(&absolute)
        });
        let cursor = selected_range
            .as_ref()
            .map(|range| range.end)
            .unwrap_or_else(|| result.map_offset(clean_range.start + new_text.len()));

        let inserted_end = clean_range.start + new_text.len();
        let caret_before_existing_closer = caret_may_have_closed_span
            && base_title.visible_text()[clean_range.end..]
                .chars()
                .next()
                .is_some_and(|next| {
                    result.map_offset(inserted_end)
                        == result.map_offset(inserted_end + next.len_utf8())
                });

        self.apply_title_edit(
            result.tree,
            cursor,
            marked_range,
            selected_range.clone(),
            selected_range.as_ref().and_then(|range| {
                (!range.is_empty()).then_some(selected_range_reversed.unwrap_or(false))
            }),
            caret_may_have_closed_span,
            caret_before_existing_closer,
            cx,
        );
    }
}
