// @author kongweiguang

use super::*;

impl Block {
    pub(crate) fn inline_tree_from_markdown_with_context(&self, markdown: &str) -> InlineTextTree {
        InlineTextTree::from_markdown_with_link_references(
            markdown,
            &self.link_reference_definitions,
        )
    }

    pub fn inline_spans(&self) -> &[InlineSpan] {
        self.current_cache().spans()
    }

    #[cfg(test)]
    pub fn inline_style_at(&self, offset: usize) -> InlineStyle {
        self.current_cache().style_at(offset)
    }

    #[cfg(test)]
    pub(crate) fn inline_html_style_at(
        &self,
        offset: usize,
    ) -> Option<crate::components::HtmlInlineStyle> {
        self.current_cache().html_style_at(offset)
    }

    #[cfg(test)]
    pub(crate) fn inline_link_at(&self, offset: usize) -> Option<&str> {
        self.current_cache().link_at(offset)
    }

    #[cfg(test)]
    pub(crate) fn inline_link_hit_at(&self, offset: usize) -> Option<&InlineLinkHit> {
        self.current_cache().link_hit_at(offset)
    }

    #[cfg(test)]
    pub(crate) fn inline_footnote_hit_at(&self, offset: usize) -> Option<&InlineFootnoteHit> {
        self.current_cache().footnote_hit_at(offset)
    }

    pub(crate) fn has_mixed_inline_visuals(&self) -> bool {
        self.record.title.has_mixed_inline_visuals()
    }

    pub(crate) fn footnote_definition_id(&self) -> Option<String> {
        self.kind()
            .is_footnote_definition()
            .then(|| self.record.title.visible_text())
    }

    pub(crate) fn footnote_definition_ordinal(&self) -> Option<usize> {
        self.footnote_definition_id()
            .as_deref()
            .and_then(|id| self.footnote_registry.ordinal(id))
    }

    pub(crate) fn footnote_definition_has_backref(&self) -> bool {
        self.footnote_definition_id().as_deref().is_some_and(|id| {
            self.footnote_registry
                .binding(id)
                .and_then(|binding| binding.first_reference.as_ref())
                .is_some()
        })
    }

    pub(crate) fn current_range_for_footnote_occurrence(
        &self,
        occurrence_index: usize,
    ) -> Option<Range<usize>> {
        let mut clean_offset = 0usize;
        for fragment in &self.record.title.fragments {
            let len = fragment.text.len();
            if fragment
                .footnote
                .as_ref()
                .is_some_and(|footnote| footnote.occurrence_index == occurrence_index)
            {
                return Some(self.clean_to_current_range(clean_offset..clean_offset + len));
            }
            clean_offset += len;
        }
        None
    }

    pub fn is_empty(&self) -> bool {
        self.display_text().is_empty()
    }

    pub fn is_direct_list_child(&self) -> bool {
        self.parent_is_list_item && !self.kind().is_list_item()
    }

    pub fn is_nested_list_item(&self) -> bool {
        self.parent_is_list_item && self.kind().is_list_item()
    }

    pub fn can_adjust_list_nesting(&self) -> bool {
        (self.kind().is_list_item() || self.parent_is_list_item) && !self.kind().is_code_block()
    }

    pub fn can_outdent_list_nesting(&self) -> bool {
        self.kind().is_list_item() || self.parent_is_list_item
    }

    pub(crate) fn visible_len(&self) -> usize {
        self.current_cache().visible_len()
    }

    pub(crate) fn split_title(&self, offset: usize) -> (InlineTextTree, InlineTextTree) {
        self.record
            .title
            .split_at(self.current_to_clean_offset(offset))
    }

    pub(super) fn clear_vertical_motion(&mut self) {
        self.vertical_motion_x = None;
    }

    pub(crate) fn sync_render_cache(&mut self) {
        let clean_selected = self.current_to_clean_range(self.selected_range.clone());
        let clean_marked = self
            .marked_range
            .clone()
            .map(|range| self.current_to_clean_range(range));
        let (clean_anchor, clean_focus) = self.clean_selection_anchor_focus();
        let (anchor_affinity, focus_affinity) = self.selection_endpoint_affinities();
        let collapsed_affinity = self.current_collapsed_caret_affinity();
        let keep_projection =
            self.projection.is_some() && self.edit_mode.supports_inline_projection();
        self.render_cache = self.record.title.render_cache();
        self.sync_code_highlight();
        self.sync_image_runtime();
        if self.kind() != BlockKind::MathBlock {
            self.last_successful_math_render = None;
            self.math_render_error = None;
        }
        if self.kind() != BlockKind::MermaidBlock {
            self.last_successful_mermaid_render = None;
            self.mermaid_render_error = None;
            self.mermaid_successful_preview_key = None;
        } else {
            // 原始 fenced Markdown 是图表渲染与导出的唯一真值。文本变更后保留旧预览
            // 仅用于视觉回退，但不得再把它当作“当前”SVG 导出。
            self.mermaid_successful_preview_key = None;
        }
        self.projection = None;
        self.projection_cache_key = None;
        if keep_projection {
            self.rebuild_inline_projection(clean_selected.clone(), clean_marked.clone());
            if clean_selected.is_empty() {
                let offset = self.clean_to_current_cursor_offset_with_affinity(
                    clean_selected.start,
                    collapsed_affinity,
                );
                self.assign_collapsed_selection_offset(offset, collapsed_affinity, None);
            } else {
                self.set_selection_from_clean_anchor_focus(
                    clean_anchor,
                    clean_focus,
                    anchor_affinity,
                    focus_affinity,
                );
                self.collapsed_caret_affinity = CollapsedCaretAffinity::Default;
            }
            self.marked_range = clean_marked.map(|range| self.clean_to_current_range(range));
        } else {
            self.set_selection_from_anchor_focus(clean_anchor, clean_focus);
            self.marked_range = clean_marked;
            self.collapsed_caret_affinity = CollapsedCaretAffinity::Default;
        }
        self.refresh_cached_display_text();
    }

    pub(super) fn sync_link_reference_definitions(
        &mut self,
        link_reference_definitions: Arc<LinkReferenceDefinitions>,
    ) {
        if self.link_reference_definitions == link_reference_definitions {
            return;
        }

        let selected_markdown = (!self.uses_raw_text_editing())
            .then(|| self.current_range_to_markdown_range(self.selected_range.clone()));
        let marked_markdown = (!self.uses_raw_text_editing())
            .then(|| {
                self.marked_range
                    .clone()
                    .map(|range| self.current_range_to_markdown_range(range))
            })
            .flatten();
        let selection_reversed = self.selection_reversed;
        let collapsed_affinity = self.current_collapsed_caret_affinity();
        let had_projection = self.projection.is_some();

        self.link_reference_definitions = link_reference_definitions;
        if self.uses_raw_text_editing() {
            return;
        }

        let markdown = self.record.title.serialize_markdown();
        let next_title = InlineTextTree::from_markdown_with_link_references(
            &markdown,
            &self.link_reference_definitions,
        );
        if self.record.title == next_title {
            return;
        }

        self.record.set_title(next_title);
        self.sync_edit_mode_from_kind();
        self.sync_render_cache();

        if let Some(selected_markdown) = selected_markdown {
            let restored = self.markdown_range_to_current_range(selected_markdown);
            if restored.is_empty() {
                self.assign_collapsed_selection_offset(
                    restored.start,
                    collapsed_affinity,
                    self.vertical_motion_x,
                );
            } else {
                self.selected_range = restored;
                self.selection_reversed = selection_reversed;
                self.collapsed_caret_affinity = CollapsedCaretAffinity::Default;
            }
        }

        self.marked_range =
            marked_markdown.map(|range| self.markdown_range_to_current_range(range));

        if had_projection {
            self.sync_inline_projection_for_focus(true);
        }
    }

    pub(super) fn sync_footnote_registry(&mut self, footnote_registry: Arc<FootnoteRegistry>) {
        if self.footnote_registry == footnote_registry {
            return;
        }

        let selected_markdown = (!self.uses_raw_text_editing())
            .then(|| self.current_range_to_markdown_range(self.selected_range.clone()));
        let marked_markdown = (!self.uses_raw_text_editing())
            .then(|| {
                self.marked_range
                    .clone()
                    .map(|range| self.current_range_to_markdown_range(range))
            })
            .flatten();
        let selection_reversed = self.selection_reversed;
        let collapsed_affinity = self.current_collapsed_caret_affinity();
        let had_projection = self.projection.is_some();

        self.footnote_registry = footnote_registry;
        if self.uses_raw_text_editing() || !self.record.title.has_footnote_references() {
            return;
        }

        let mut next_title = self.record.title.clone();
        let mut occurrence_iter = self
            .footnote_registry
            .occurrences_for_block(self.record.id)
            .unwrap_or(&[])
            .iter();
        next_title.apply_footnote_reference_state(|id| {
            let occurrence = occurrence_iter.next()?;
            if occurrence.id != id {
                return None;
            }
            Some((occurrence.ordinal?, occurrence.occurrence_index))
        });
        if self.record.title == next_title {
            return;
        }

        self.record.set_title(next_title);
        self.sync_edit_mode_from_kind();
        self.sync_render_cache();

        if let Some(selected_markdown) = selected_markdown {
            let restored = self.markdown_range_to_current_range(selected_markdown);
            if restored.is_empty() {
                self.assign_collapsed_selection_offset(
                    restored.start,
                    collapsed_affinity,
                    self.vertical_motion_x,
                );
            } else {
                self.selected_range = restored;
                self.selection_reversed = selection_reversed;
                self.collapsed_caret_affinity = CollapsedCaretAffinity::Default;
            }
        }

        self.marked_range =
            marked_markdown.map(|range| self.markdown_range_to_current_range(range));

        if had_projection {
            self.sync_inline_projection_for_focus(true);
        }
    }

    pub(super) fn should_use_markdown_space_link_edit(&self) -> bool {
        !self.uses_raw_text_editing() && self.record.title.has_source_preserving_links()
    }

    /// 保留链接源码的坐标映射，闭合链接时仍允许光标离开新识别的样式。
    pub(super) fn apply_markdown_space_title_edit(
        &mut self,
        visible_range: Range<usize>,
        new_text: &str,
        selected_range_relative: Option<Range<usize>>,
        mark_inserted_text: bool,
        cx: &mut Context<Self>,
    ) {
        let old_visible_len = self.record.title.visible_text().len();
        let markdown_range = self.current_range_to_markdown_range(visible_range.clone());
        let mut markdown = self.record.title.serialize_markdown();
        let replaced_text = markdown[markdown_range.clone()].to_string();
        markdown.replace_range(markdown_range.clone(), new_text);

        let next_title = InlineTextTree::from_markdown_with_link_references(
            &markdown,
            &self.link_reference_definitions,
        );
        let map = next_title.markdown_offset_map();
        let selected_markdown = selected_range_relative.as_ref().map(|relative| {
            markdown_range.start + relative.start..markdown_range.start + relative.end
        });
        let cursor_markdown = selected_markdown
            .as_ref()
            .map(|range| range.end)
            .unwrap_or(markdown_range.start + new_text.len());
        let marked_markdown = if mark_inserted_text && !new_text.is_empty() {
            Some(markdown_range.start..markdown_range.start + new_text.len())
        } else {
            None
        };
        let selected_clean = selected_markdown
            .as_ref()
            .map(|range| map.markdown_to_visible_range(range.clone()));
        let marked_clean = marked_markdown
            .as_ref()
            .map(|range| map.markdown_to_visible_range(range.clone()));
        let cursor_clean = map.markdown_to_visible_offset(cursor_markdown);

        let quote_structure_edit = self.quote_depth > 0
            && (new_text.contains('\n')
                || replaced_text.contains('\n')
                || (self.kind() == BlockKind::Quote
                    && Self::multiline_quote_edit_requires_reparse(&next_title.visible_text())));
        if quote_structure_edit {
            self.quote_reparse_requested = true;
        }

        // Typing a closing marker (for example the `)` that completes a link)
        // absorbs that markup into a span, so the clean text grows by less than
        // the inserted text. Flag it so the caret is placed just past the new
        // closing delimiter instead of landing inside the span.
        let caret_may_have_closed_span = !new_text.is_empty()
            && !mark_inserted_text
            && next_title.visible_text().len() < old_visible_len + new_text.len();

        self.apply_title_edit(
            next_title,
            cursor_clean,
            marked_clean,
            selected_clean.clone(),
            selected_clean
                .as_ref()
                .and_then(|range| (!range.is_empty()).then_some(false)),
            caret_may_have_closed_span,
            false,
            cx,
        );
    }

    pub(crate) fn current_cache(&self) -> &InlineRenderCache {
        self.projection
            .as_ref()
            .map(|projection| &projection.cache)
            .unwrap_or(&self.render_cache)
    }

    pub(crate) fn sync_inline_projection_for_focus(&mut self, focused: bool) {
        let supports_projection = self.edit_mode.supports_inline_projection();
        if !focused || !supports_projection {
            self.clear_inline_projection();
            return;
        }

        let projected_link_selection = self.projection.as_ref().and_then(|projection| {
            projection
                .link_run_fully_covering_range(&self.selected_range)
                .map(|run| ProjectedLinkSelectionSnapshot {
                    clean_range: run.clean_range.clone(),
                    display_relative_range: self
                        .selected_range
                        .start
                        .saturating_sub(run.display_range.start)
                        ..self
                            .selected_range
                            .end
                            .saturating_sub(run.display_range.start),
                    selection_reversed: self.selection_reversed,
                })
        });
        let clean_selected = self.current_to_clean_range(self.selected_range.clone());
        let clean_marked = self
            .marked_range
            .clone()
            .map(|range| self.current_to_clean_range(range));
        if self.projection_cache_key.as_ref()
            == Some(&(
                supports_projection,
                clean_selected.clone(),
                clean_marked.clone(),
            ))
        {
            return;
        }
        let (clean_anchor, clean_focus) = self.clean_selection_anchor_focus();
        let (anchor_affinity, focus_affinity) = self.selection_endpoint_affinities();
        let collapsed_affinity = self.current_collapsed_caret_affinity();
        self.rebuild_inline_projection(clean_selected.clone(), clean_marked.clone());
        if let Some(snapshot) = projected_link_selection
            && let Some(run) = self
                .projection
                .as_ref()
                .and_then(|projection| projection.link_run_for_clean_range(&snapshot.clean_range))
        {
            let start = run.display_range.start
                + snapshot
                    .display_relative_range
                    .start
                    .min(run.display_range.len());
            let end = run.display_range.start
                + snapshot
                    .display_relative_range
                    .end
                    .min(run.display_range.len());
            self.selected_range = start..end;
            self.selection_reversed = snapshot.selection_reversed;
            self.collapsed_caret_affinity = CollapsedCaretAffinity::Default;
        } else if clean_selected.is_empty() {
            let offset = self.clean_to_current_cursor_offset_with_affinity(
                clean_selected.start,
                collapsed_affinity,
            );
            self.assign_collapsed_selection_offset(offset, collapsed_affinity, None);
        } else {
            self.set_selection_from_clean_anchor_focus(
                clean_anchor,
                clean_focus,
                anchor_affinity,
                focus_affinity,
            );
            self.collapsed_caret_affinity = CollapsedCaretAffinity::Default;
        }
        self.marked_range = clean_marked.map(|range| self.clean_to_current_range(range));
    }

    pub(crate) fn clear_inline_projection(&mut self) {
        if self.projection.is_none() {
            self.projection_cache_key = None;
            return;
        }

        let clean_marked = self
            .marked_range
            .clone()
            .map(|range| self.current_to_clean_range(range));
        let (clean_anchor, clean_focus) = self.clean_selection_anchor_focus();
        self.projection = None;
        self.projection_cache_key = None;
        self.set_selection_from_anchor_focus(clean_anchor, clean_focus);
        self.marked_range = clean_marked;
        self.collapsed_caret_affinity = CollapsedCaretAffinity::Default;
        self.refresh_cached_display_text();
    }

    pub(super) fn rebuild_inline_projection(
        &mut self,
        clean_selected: Range<usize>,
        clean_marked: Option<Range<usize>>,
    ) {
        self.projection_cache_key = Some((
            self.edit_mode.supports_inline_projection(),
            clean_selected.clone(),
            clean_marked.clone(),
        ));
        self.projection = ExpandedInlineProjection::build(
            &self.record.title.fragments,
            clean_selected,
            clean_marked,
        );
        self.refresh_cached_display_text();
    }

    pub(super) fn projection_segments(&self) -> &[ExpandedInlineSegment] {
        self.projection
            .as_ref()
            .map(|projection| projection.segments.as_slice())
            .unwrap_or(&[])
    }

    pub(super) fn projected_link_run_fully_covering_range(
        &self,
        range: &Range<usize>,
    ) -> Option<&ExpandedLinkRun> {
        self.projection
            .as_ref()
            .and_then(|projection| projection.link_run_fully_covering_range(range))
    }

    fn collapsed_caret_affinity_for_display_offset(&self, offset: usize) -> CollapsedCaretAffinity {
        self.projection
            .as_ref()
            .map(|projection| projection.collapsed_affinity_for_display_offset(offset))
            .unwrap_or(CollapsedCaretAffinity::Default)
    }

    /// Affinity of the current selection's anchor and focus, used to restore
    /// each endpoint accurately when the projection is rebuilt.
    fn selection_endpoint_affinities(&self) -> (CollapsedCaretAffinity, CollapsedCaretAffinity) {
        let (anchor, focus) = self.selection_anchor_focus();
        (
            self.collapsed_caret_affinity_for_display_offset(anchor),
            self.collapsed_caret_affinity_for_display_offset(focus),
        )
    }

    pub(super) fn current_collapsed_caret_affinity(&self) -> CollapsedCaretAffinity {
        if !self.selected_range.is_empty() {
            return CollapsedCaretAffinity::Default;
        }

        self.projection
            .as_ref()
            .map(|projection| {
                projection.collapsed_affinity_for_display_offset(self.cursor_offset())
            })
            .unwrap_or(self.collapsed_caret_affinity)
    }

    /// Recomputes projection-side caret affinity after restoring a cancelled IME selection.
    pub(crate) fn sync_collapsed_caret_affinity(&mut self) {
        self.collapsed_caret_affinity = if self.selected_range.is_empty() {
            self.projection
                .as_ref()
                .map(|projection| {
                    projection.collapsed_affinity_for_display_offset(self.cursor_offset())
                })
                .unwrap_or(CollapsedCaretAffinity::Default)
        } else {
            CollapsedCaretAffinity::Default
        };
    }

    pub(crate) fn assign_collapsed_selection_offset(
        &mut self,
        offset: usize,
        affinity: CollapsedCaretAffinity,
        preferred_x: Option<Pixels>,
    ) {
        let clamped_offset = offset.min(self.visible_len());
        self.selected_range = clamped_offset..clamped_offset;
        self.selection_reversed = false;
        self.vertical_motion_x = preferred_x;
        self.collapsed_caret_affinity = affinity;
        self.sync_collapsed_caret_affinity();
    }

    fn clean_to_current_cursor_offset(&self, clean: usize) -> usize {
        let Some(projection) = &self.projection else {
            return clean;
        };
        projection
            .clean_to_display_cursor
            .get(clean.min(projection.clean_to_display_cursor.len().saturating_sub(1)))
            .copied()
            .unwrap_or(clean)
    }

    pub(super) fn clean_to_current_cursor_offset_with_affinity(
        &self,
        clean: usize,
        affinity: CollapsedCaretAffinity,
    ) -> usize {
        let Some(projection) = &self.projection else {
            return clean;
        };
        projection
            .display_offset_for_clean_cursor(clean, affinity)
            .unwrap_or_else(|| self.clean_to_current_cursor_offset(clean))
    }

    fn clean_to_current_range_start(&self, clean: usize) -> usize {
        self.clean_to_current_cursor_offset(clean)
    }

    fn clean_to_current_range_end(&self, clean: usize) -> usize {
        self.clean_to_current_cursor_offset(clean)
    }

    pub(crate) fn clean_to_current_range(&self, range: Range<usize>) -> Range<usize> {
        if range.is_empty() {
            let offset = self.clean_to_current_cursor_offset(range.start);
            offset..offset
        } else {
            self.clean_to_current_range_start(range.start)
                ..self.clean_to_current_range_end(range.end)
        }
    }

    pub(crate) fn current_to_clean_range(&self, range: Range<usize>) -> Range<usize> {
        self.current_to_clean_offset(range.start)..self.current_to_clean_offset(range.end)
    }

    pub(crate) fn current_to_clean_offset(&self, offset: usize) -> usize {
        self.unexpand_offset(offset)
    }

    #[cfg(test)]
    pub(crate) fn pointer_target_offset(&self, offset: usize) -> usize {
        self.projection
            .as_ref()
            .map(|projection| projection.pointer_target_offset(offset))
            .unwrap_or(offset)
    }

    pub(crate) fn projected_move_left_target(
        &self,
        offset: usize,
    ) -> Option<(usize, CollapsedCaretAffinity)> {
        self.projection
            .as_ref()
            .and_then(|projection| projection.move_left_target(offset))
    }

    pub(crate) fn projected_move_right_target(
        &self,
        offset: usize,
    ) -> Option<(usize, CollapsedCaretAffinity)> {
        self.projection
            .as_ref()
            .and_then(|projection| projection.move_right_target(offset))
    }

    pub(crate) fn selection_clean_range(&self) -> Range<usize> {
        self.current_to_clean_range(self.selected_range.clone())
    }

    pub(crate) fn current_range_to_markdown_range(&self, range: Range<usize>) -> Range<usize> {
        if self.uses_raw_text_editing() || self.kind().is_code_block() {
            return range.start.min(self.visible_len())..range.end.min(self.visible_len());
        }

        if let Some(link_run) = self.projected_link_run_fully_covering_range(&range) {
            let map = self.record.title.markdown_offset_map();
            let label_markdown_start = map.visible_to_markdown_offset(link_run.clean_range.start);
            let run_markdown_start =
                label_markdown_start.saturating_sub(link_run.link.open_marker().len());
            let start = run_markdown_start
                + range
                    .start
                    .saturating_sub(link_run.display_range.start)
                    .min(link_run.display_range.len());
            let end = run_markdown_start
                + range
                    .end
                    .saturating_sub(link_run.display_range.start)
                    .min(link_run.display_range.len());
            return start..end;
        }

        if let Some(footnote_run) = self
            .projection
            .as_ref()
            .and_then(|projection| projection.footnote_run_fully_covering_range(&range))
        {
            let raw = footnote_run.footnote.raw_markdown();
            let raw_len = raw.len();
            let local_start = range
                .start
                .saturating_sub(footnote_run.display_range.start)
                .min(footnote_run.display_range.len());
            let local_end = range
                .end
                .saturating_sub(footnote_run.display_range.start)
                .min(footnote_run.display_range.len());
            let mapped_start = (raw_len * local_start) / footnote_run.display_range.len().max(1);
            let mapped_end = (raw_len * local_end) / footnote_run.display_range.len().max(1);
            let map = self.record.title.markdown_offset_map();
            let run_markdown_start = map.visible_to_markdown_offset(footnote_run.clean_range.start);
            return run_markdown_start + mapped_start..run_markdown_start + mapped_end;
        }

        let clean_range = self.current_to_clean_range(range);
        self.record
            .title
            .markdown_offset_map()
            .visible_to_markdown_range(clean_range)
    }
}
