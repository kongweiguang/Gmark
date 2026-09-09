// @author kongweiguang

use super::*;

impl DocumentTree {
    pub(in crate::editor) fn raw_source_text(&self, cx: &App) -> String {
        self.snapshot
            .visible
            .iter()
            .map(|visible| visible.entity.read(cx).display_text().to_string())
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub(in crate::editor) fn insert_blocks_at(
        &mut self,
        parent: Option<Entity<Block>>,
        index: usize,
        blocks: Vec<Entity<Block>>,
        cx: &mut Context<Editor>,
    ) {
        self.with_structure_mutation(cx, move |tree, cx| {
            tree.insert_blocks_at_raw(parent, index, blocks, cx);
        });
    }

    /// Runs a tree mutation and then eagerly rebuilds metadata and the visible
    /// snapshot exactly once for that mutation batch.
    pub(in crate::editor) fn with_structure_mutation<R>(
        &mut self,
        cx: &mut Context<Editor>,
        mutate: impl FnOnce(&mut Self, &mut Context<Editor>) -> R,
    ) -> R {
        let result = mutate(self, cx);
        self.rebuild_metadata_and_snapshot(cx);
        result
    }

    /// Rebuilds tree metadata and cached visible-order data from the current
    /// roots.
    ///
    /// The pass first normalizes impossible runtime-only shapes by hoisting
    /// children out of leaf blocks. It then performs one DFS to update parent
    /// UUIDs, child UUID lists, render depth, numbered-list ordinals, and the
    /// visible snapshot.
    pub(in crate::editor) fn rebuild_metadata_and_snapshot(&mut self, cx: &mut Context<Editor>) {
        Self::normalize_block_list(&mut self.roots, cx);
        self.snapshot.clear();
        Self::sync_block_list(
            &self.roots.clone(),
            None,
            None,
            0,
            0,
            None,
            None,
            0,
            None,
            None,
            None,
            cx,
            &mut self.snapshot,
        );
        self.rebuild_root_markdown_cache(cx);
    }

    pub(in crate::editor) fn root_index_for_entity(&self, entity_id: EntityId) -> Option<usize> {
        let mut current = entity_id;
        loop {
            let location = self.snapshot.location_by_entity.get(&current)?;
            let Some(parent) = location.parent.as_ref() else {
                return Some(location.index);
            };
            current = parent.entity_id();
        }
    }

    /// 返回缓存规范 Markdown 中根块首字节；缓存过期时交由调用方走完整映射。
    pub(in crate::editor) fn cached_root_source_start(&self, target_index: usize) -> Option<usize> {
        if target_index >= self.root_markdown_cache.len()
            || self.root_markdown_cache.len() != self.roots.len()
        {
            return None;
        }

        let mut bytes = 0usize;
        let mut line_count = 0usize;
        let mut pending_empty_roots = 0usize;
        let mut wrote_non_empty_root = false;
        let mut previous_was_list_item = false;
        let append_line = |bytes: &mut usize, line_count: &mut usize, line: &str| {
            if *line_count > 0 {
                *bytes += 1;
            }
            *bytes += line.len();
            *line_count += 1;
        };

        for (index, root) in self.root_markdown_cache.iter().enumerate() {
            if root.is_empty_paragraph {
                if index == target_index {
                    return Some(bytes);
                }
                pending_empty_roots += 1;
                continue;
            }
            let separator_count = if wrote_non_empty_root {
                if previous_was_list_item && root.is_list_item {
                    pending_empty_roots
                } else {
                    pending_empty_roots + 1
                }
            } else {
                pending_empty_roots
            };
            for _ in 0..separator_count {
                append_line(&mut bytes, &mut line_count, "");
            }
            if index == target_index {
                return Some(bytes + usize::from(line_count > 0));
            }
            for line in &root.lines {
                append_line(&mut bytes, &mut line_count, line);
            }
            wrote_non_empty_root = true;
            pending_empty_roots = 0;
            previous_was_list_item = root.is_list_item;
        }
        None
    }

    pub(in crate::editor) fn rebuild_root_markdown_cache(&mut self, cx: &App) {
        self.root_markdown_cache = self
            .roots
            .iter()
            .map(|root| Self::build_root_markdown_cache(root.read(cx), cx))
            .collect();
    }

    pub(super) fn build_root_markdown_cache(block: &Block, cx: &App) -> RootMarkdownCache {
        let is_empty_paragraph = Self::is_empty_root_paragraph(block);
        let mut lines = Vec::new();
        if !is_empty_paragraph {
            Self::collect_single_block_markdown_lines(block, 0, cx, &mut lines);
        }
        RootMarkdownCache {
            lines,
            is_empty_paragraph,
            is_list_item: block.kind().is_list_item(),
        }
    }

    pub(super) fn markdown_text_from_root_cache(&self) -> String {
        let estimated_bytes = self
            .root_markdown_cache
            .iter()
            .flat_map(|root| root.lines.iter())
            .map(|line| line.len() + 1)
            .sum();
        let mut markdown = String::with_capacity(estimated_bytes);
        let mut line_count = 0usize;
        let mut pending_empty_roots = 0usize;
        let mut wrote_non_empty_root = false;
        let mut previous_was_list_item = false;

        let push_line = |markdown: &mut String, line_count: &mut usize, line: &str| {
            if *line_count > 0 {
                markdown.push('\n');
            }
            markdown.push_str(line);
            *line_count += 1;
        };

        for root in &self.root_markdown_cache {
            if root.is_empty_paragraph {
                pending_empty_roots += 1;
                continue;
            }
            let separator_count = if wrote_non_empty_root {
                if previous_was_list_item && root.is_list_item {
                    pending_empty_roots
                } else {
                    pending_empty_roots + 1
                }
            } else {
                pending_empty_roots
            };
            for _ in 0..separator_count {
                push_line(&mut markdown, &mut line_count, "");
            }
            for line in &root.lines {
                push_line(&mut markdown, &mut line_count, line);
            }
            wrote_non_empty_root = true;
            pending_empty_roots = 0;
            previous_was_list_item = root.is_list_item;
        }

        let trailing_empty_lines = if wrote_non_empty_root && pending_empty_roots > 0 {
            pending_empty_roots + 1
        } else if !wrote_non_empty_root && pending_empty_roots > 1 {
            pending_empty_roots
        } else {
            0
        };
        for _ in 0..trailing_empty_lines {
            push_line(&mut markdown, &mut line_count, "");
        }
        markdown
    }

    pub(in crate::editor) fn take_children(
        block: &Entity<Block>,
        cx: &mut Context<Editor>,
    ) -> Vec<Entity<Block>> {
        let mut children = Vec::new();
        block.update(cx, |block, _cx| {
            children = std::mem::take(&mut block.children);
        });
        children
    }

    pub(in crate::editor) fn insert_blocks_at_raw(
        &mut self,
        parent: Option<Entity<Block>>,
        index: usize,
        blocks: Vec<Entity<Block>>,
        cx: &mut Context<Editor>,
    ) {
        if blocks.is_empty() {
            return;
        }

        if let Some(parent) = parent {
            parent.update(cx, move |parent, _cx| {
                for (offset, block) in blocks.iter().cloned().enumerate() {
                    parent.children.insert(index + offset, block);
                }
            });
        } else {
            for (offset, block) in blocks.into_iter().enumerate() {
                self.roots.insert(index + offset, block);
            }
        }
    }

    pub(in crate::editor) fn remove_block_by_id_raw(
        &mut self,
        entity_id: EntityId,
        cx: &mut Context<Editor>,
    ) -> Option<(Entity<Block>, BlockLocation)> {
        let location = self.find_block_location(entity_id)?;
        let removed = if let Some(parent) = location.parent.clone() {
            let mut removed = None;
            parent.update(cx, |parent, _cx| {
                removed = Some(parent.children.remove(location.index));
            });
            removed?
        } else {
            self.roots.remove(location.index)
        };

        Some((removed, location))
    }

    /// Normalizes a sibling list so only container-capable block kinds retain
    /// children.
    ///
    /// Children attached to leaf blocks are hoisted into the same parent list
    /// immediately after the leaf that previously owned them.
    fn normalize_block_list(blocks: &mut Vec<Entity<Block>>, cx: &mut Context<Editor>) {
        let mut index = 0;
        while index < blocks.len() {
            let block = blocks[index].clone();
            let mut children = Self::take_children(&block, cx);
            Self::normalize_block_list(&mut children, cx);

            if block.read(cx).kind().supports_children() {
                block.update(cx, {
                    let children = children.clone();
                    move |block, _cx| {
                        block.children = children.clone();
                    }
                });
            } else if !children.is_empty() {
                blocks.splice(index + 1..index + 1, children);
            }

            index += 1;
        }
    }

    /// 每个同级列表段独立计数；源码起点只用于段首，后续标记不覆盖自动递增。
    fn sync_block_list(
        blocks: &[Entity<Block>],
        parent_entity: Option<Entity<Block>>,
        parent_id: Option<uuid::Uuid>,
        list_depth: usize,
        inherited_quote_depth: usize,
        inherited_quote_group_anchor: Option<uuid::Uuid>,
        inherited_visible_quote_group_anchor: Option<uuid::Uuid>,
        inherited_callout_depth: usize,
        inherited_callout_anchor: Option<uuid::Uuid>,
        inherited_callout_variant: Option<CalloutVariant>,
        inherited_footnote_anchor: Option<uuid::Uuid>,
        cx: &mut Context<Editor>,
        snapshot: &mut VisibleTreeSnapshot,
    ) {
        let mut numbered_list_ordinal: Option<usize> = None;
        let mut previous_was_list_item = false;
        for (index, block) in blocks.iter().enumerate() {
            let entity_id = block.entity_id();
            let visible_index = snapshot.visible.len();
            snapshot.visible.push(VisibleBlock {
                entity: block.clone(),
            });
            snapshot
                .visible_index_by_entity
                .insert(entity_id, visible_index);
            snapshot.location_by_entity.insert(
                entity_id,
                BlockLocation {
                    parent: parent_entity.clone(),
                    index,
                },
            );

            let (block_id, kind, children, is_empty_paragraph) = {
                let block_ref = block.read(cx);
                (
                    block_ref.record.id,
                    block_ref.kind(),
                    block_ref.children.clone(),
                    block_ref.kind() == BlockKind::Paragraph
                        && block_ref.record.title.visible_text().is_empty()
                        && block_ref.children.is_empty(),
                )
            };
            let parent_is_list_item = parent_entity
                .as_ref()
                .is_some_and(|parent| parent.read(cx).kind().is_list_item());

            let content = children
                .iter()
                .map(|child| child.read(cx).record.id)
                .collect::<Vec<_>>();
            let list_ordinal = if kind.is_numbered_list_item() {
                let ordinal = numbered_list_ordinal.map_or_else(
                    || block.read(cx).record.ordered_list_start.unwrap_or(1),
                    |previous| previous.saturating_add(1),
                );
                numbered_list_ordinal = Some(ordinal);
                Some(ordinal)
            } else {
                numbered_list_ordinal = None;
                None
            };
            let is_quote_container = kind.is_quote_container();
            let own_callout_variant = kind.callout_variant();
            let quote_depth = inherited_quote_depth + usize::from(is_quote_container);
            let quote_group_anchor = if is_quote_container {
                inherited_quote_group_anchor.or(Some(block_id))
            } else {
                inherited_quote_group_anchor
            };
            let callout_depth =
                inherited_callout_depth + usize::from(own_callout_variant.is_some());
            let callout_anchor = if own_callout_variant.is_some() {
                Some(block_id)
            } else {
                inherited_callout_anchor
            };
            let callout_variant = own_callout_variant.or(inherited_callout_variant);
            let visible_quote_depth = quote_depth.saturating_sub(callout_depth);
            let visible_quote_group_anchor = match kind {
                BlockKind::Quote => inherited_visible_quote_group_anchor.or(Some(block_id)),
                BlockKind::Callout(_) => None,
                _ if visible_quote_depth == 0 => None,
                _ => inherited_visible_quote_group_anchor,
            };
            let child_visible_quote_group_anchor = if own_callout_variant.is_some() {
                None
            } else {
                visible_quote_group_anchor
            };
            let footnote_anchor = if kind.is_footnote_definition() {
                Some(block_id)
            } else {
                inherited_footnote_anchor
            };
            let child_list_depth = list_depth + usize::from(kind.is_list_item());
            let list_group_separator_candidate = is_empty_paragraph && previous_was_list_item;

            block.update(cx, move |block, _cx| {
                let structure_changed = block.record.parent != parent_id
                    || block.record.content != content
                    || block.structural_sibling_index != index
                    || block.structural_sibling_count != blocks.len();
                if structure_changed {
                    block.structural_context_revision =
                        block.structural_context_revision.wrapping_add(1);
                }
                block.record.parent = parent_id;
                block.record.content = content.clone();
                block.render_depth = list_depth;
                block.structural_sibling_index = index;
                block.structural_sibling_count = blocks.len();
                block.quote_depth = quote_depth;
                block.quote_group_anchor = quote_group_anchor;
                block.visible_quote_depth = visible_quote_depth;
                block.visible_quote_group_anchor = visible_quote_group_anchor;
                block.callout_depth = callout_depth;
                block.callout_anchor = callout_anchor;
                block.callout_variant = callout_variant;
                block.footnote_anchor = footnote_anchor;
                block.parent_is_list_item = parent_is_list_item;
                block.list_ordinal = list_ordinal;
                block.list_group_separator_candidate = list_group_separator_candidate;
            });

            let last_descendant_id = if children.is_empty() {
                entity_id
            } else {
                Self::sync_block_list(
                    &children,
                    Some(block.clone()),
                    Some(block_id),
                    child_list_depth,
                    quote_depth,
                    quote_group_anchor,
                    child_visible_quote_group_anchor,
                    callout_depth,
                    callout_anchor,
                    callout_variant,
                    footnote_anchor,
                    cx,
                    snapshot,
                );
                snapshot
                    .last_visible_descendant_by_entity
                    .get(&children.last().expect("children checked").entity_id())
                    .copied()
                    .unwrap_or_else(|| children.last().expect("children checked").entity_id())
            };

            snapshot
                .last_visible_descendant_by_entity
                .insert(entity_id, last_descendant_id);
            previous_was_list_item = kind.is_list_item();
        }
    }

    fn is_empty_root_paragraph(block: &Block) -> bool {
        block.kind() == BlockKind::Paragraph
            && block.record.title.visible_text().is_empty()
            && block.children.is_empty()
    }

    pub(super) fn collect_root_markdown_lines(
        blocks: &[Entity<Block>],
        cx: &App,
        lines: &mut Vec<String>,
    ) {
        let mut pending_empty_roots = 0usize;
        let mut wrote_non_empty_root = false;
        let mut previous_was_list_item = false;

        for block in blocks {
            let block_ref = block.read(cx);
            if Self::is_empty_root_paragraph(block_ref) {
                pending_empty_roots += 1;
                continue;
            }

            let current_is_list_item = block_ref.kind().is_list_item();
            if wrote_non_empty_root {
                let separator_count = if previous_was_list_item && current_is_list_item {
                    pending_empty_roots
                } else {
                    pending_empty_roots + 1
                };
                lines.extend(std::iter::repeat_n(String::new(), separator_count));
            } else if pending_empty_roots > 0 {
                lines.extend(std::iter::repeat_n(String::new(), pending_empty_roots));
            }

            Self::collect_single_block_markdown_lines(block_ref, 0, cx, lines);
            wrote_non_empty_root = true;
            pending_empty_roots = 0;
            previous_was_list_item = current_is_list_item;
        }

        if wrote_non_empty_root {
            if pending_empty_roots > 0 {
                lines.extend(std::iter::repeat_n(String::new(), pending_empty_roots + 1));
            }
        } else if pending_empty_roots > 1 {
            lines.extend(std::iter::repeat_n(String::new(), pending_empty_roots));
        }
    }

    fn collect_single_block_markdown_lines(
        block_ref: &Block,
        list_depth: usize,
        cx: &App,
        lines: &mut Vec<String>,
    ) {
        match block_ref.kind() {
            BlockKind::Table => {
                if let Some(table) = block_ref.record.table.as_ref() {
                    lines.extend(serialize_table_markdown_lines(table));
                }
            }
            BlockKind::CodeBlock { language } => {
                let indentation = "  ".repeat(list_depth);
                let lang_str = language.as_ref().map(|s| s.as_ref()).unwrap_or("");
                let fence = super::persistence::safe_code_fence_with_info(
                    &block_ref.record.title.visible_text(),
                    language.as_ref().map(|language| language.as_ref()),
                );
                lines.push(format!("{indentation}{fence}{lang_str}"));
                let content = block_ref.record.title.visible_text();
                for code_line in content.split('\n') {
                    lines.push(format!("{indentation}{code_line}"));
                }
                lines.push(format!("{indentation}{fence}"));
            }
            BlockKind::Quote => {
                let title_markdown =
                    CalloutVariant::escape_plain_quote_header(&block_ref.record.title_markdown());
                let indentation = "  ".repeat(list_depth);
                if !title_markdown.is_empty() || block_ref.children.is_empty() {
                    for line in title_markdown.split('\n') {
                        lines.push(format!("{indentation}> {line}"));
                    }
                }

                if !block_ref.children.is_empty() {
                    let mut child_lines = Vec::new();
                    Self::collect_markdown_lines(
                        &block_ref.children,
                        list_depth,
                        cx,
                        &mut child_lines,
                        false,
                    );
                    lines.extend(
                        child_lines
                            .into_iter()
                            .map(|line| format!("{indentation}> {line}")),
                    );
                }
            }
            BlockKind::Callout(variant) => {
                let indentation = "  ".repeat(list_depth);
                lines.push(format!(
                    "{indentation}> {}",
                    variant.header_markdown(&block_ref.record.title_markdown())
                ));
                if !block_ref.children.is_empty() {
                    let mut child_lines = Vec::new();
                    Self::collect_markdown_lines(
                        &block_ref.children,
                        list_depth,
                        cx,
                        &mut child_lines,
                        false,
                    );
                    lines.extend(
                        child_lines
                            .into_iter()
                            .map(|line| format!("{indentation}> {line}")),
                    );
                }
            }
            BlockKind::FootnoteDefinition => {
                let indentation = "  ".repeat(list_depth);
                let id = block_ref.record.title.visible_text();
                if block_ref.children.is_empty() {
                    lines.push(format!("{indentation}[^{}]:", id));
                    return;
                }

                let first_child = block_ref.children.first().cloned().expect("checked");
                let first_is_paragraph = first_child.read(cx).kind() == BlockKind::Paragraph;
                if first_is_paragraph {
                    let first_title = first_child.read(cx).record.title_markdown();
                    let mut first_lines = first_title.split('\n');
                    let first_line = first_lines.next().unwrap_or_default();
                    lines.push(format!("{indentation}[^{}]: {}", id, first_line));
                    for line in first_lines {
                        if line.is_empty() {
                            lines.push(String::new());
                        } else {
                            lines.push(format!("{indentation}    {line}"));
                        }
                    }

                    if block_ref.children.len() > 1 {
                        lines.push(String::new());
                        Self::collect_markdown_lines(&block_ref.children[1..], 2, cx, lines, true);
                    }
                } else {
                    lines.push(format!("{indentation}[^{}]:", id));
                    Self::collect_markdown_lines(&block_ref.children, 2, cx, lines, true);
                }
            }
            BlockKind::RawMarkdown
            | BlockKind::Comment
            | BlockKind::HtmlBlock
            | BlockKind::MathBlock
            | BlockKind::MermaidBlock => {
                let indentation = "  ".repeat(list_depth);
                let raw_markdown = block_ref
                    .record
                    .raw_fallback
                    .clone()
                    .unwrap_or_else(|| block_ref.record.title_markdown());
                for line in raw_markdown.split('\n') {
                    if indentation.is_empty() {
                        lines.push(line.to_string());
                    } else {
                        lines.push(format!("{indentation}{line}"));
                    }
                }
            }
            BlockKind::BulletedListItem
            | BlockKind::TaskListItem { .. }
            | BlockKind::NumberedListItem => {
                lines.push(
                    block_ref
                        .record
                        .markdown_line(list_depth, block_ref.list_ordinal),
                );
                let child_list_depth = list_depth + 1;
                for child in &block_ref.children {
                    let child_ref = child.read(cx);
                    if Self::list_child_requires_leading_blank_line(child_ref) {
                        lines.push(String::new());
                    }
                    Self::collect_single_block_markdown_lines(
                        child_ref,
                        child_list_depth,
                        cx,
                        lines,
                    );
                }
            }
            _ => {
                lines.push(
                    block_ref
                        .record
                        .markdown_line(list_depth, block_ref.list_ordinal),
                );
                let child_list_depth = list_depth + usize::from(block_ref.kind().is_list_item());
                Self::collect_markdown_lines(
                    &block_ref.children,
                    child_list_depth,
                    cx,
                    lines,
                    false,
                );
            }
        }
    }

    fn list_child_requires_leading_blank_line(block_ref: &Block) -> bool {
        if block_ref.kind() != BlockKind::Paragraph || !block_ref.children.is_empty() {
            return false;
        }

        let markdown = block_ref.record.title_markdown();
        !markdown.is_empty() && parse_standalone_image(&markdown).is_none()
    }

    fn collect_markdown_lines(
        blocks: &[Entity<Block>],
        depth: usize,
        cx: &App,
        lines: &mut Vec<String>,
        blank_line_between_siblings: bool,
    ) {
        let mut first = true;
        let mut previous_was_list_item = false;
        for block in blocks {
            let current_is_list_item = block.read(cx).kind().is_list_item();
            if !first
                && blank_line_between_siblings
                && !(previous_was_list_item && current_is_list_item)
            {
                lines.push(String::new());
            }
            first = false;

            let block_ref = block.read(cx);
            Self::collect_single_block_markdown_lines(block_ref, depth, cx, lines);
            previous_was_list_item = current_is_list_item;
        }
    }
}
