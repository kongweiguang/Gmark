// @author kongweiguang

use super::projection_builder::*;
use super::*;

fn materialize_blank_run(
    cx: &mut Context<Editor>,
    preserved: usize,
    keep_edit_host: bool,
) -> Vec<Entity<super::Block>> {
    let source_blank_count = if keep_edit_host {
        preserved.saturating_sub(1)
    } else {
        preserved
    };
    let mut blocks = (0..source_blank_count)
        .map(|_| source_blank_block(cx))
        .collect::<Vec<_>>();
    if keep_edit_host && preserved > 0 {
        blocks.push(native_block(cx, BlockKind::Paragraph, String::new()));
    }
    blocks
}

impl Editor {
    /// Builds runtime blocks from Markdown lines.
    ///
    /// Native blocks are created only for syntax the runtime editor can edit
    /// safely. More complex valid Markdown regions fall back to
    /// [`BlockKind::RawMarkdown`] so they are preserved exactly on save.
    pub(in crate::editor) fn build_blocks_from_lines(
        cx: &mut Context<Self>,
        lines: &[String],
    ) -> Vec<Entity<super::Block>> {
        Self::build_blocks_from_lines_internal(cx, lines, true)
    }

    /// 将后台准备好的顶层区域物化为 GPUI 块，不再在 UI 线程重复扫描区域边界。
    #[cfg(test)]
    pub(in crate::editor) fn build_blocks_from_projection(
        cx: &mut Context<Self>,
        prepared: &PreparedSplitProjection,
    ) -> Vec<Entity<super::Block>> {
        Self::build_blocks_from_projection_reusing(cx, prepared, &mut HashMap::new())
    }

    pub(in crate::editor) fn build_blocks_from_projection_reusing(
        cx: &mut Context<Self>,
        prepared: &PreparedSplitProjection,
        reusable: &mut HashMap<uuid::Uuid, Entity<super::Block>>,
    ) -> Vec<Entity<super::Block>> {
        let lines = &prepared.lines;
        let regions = &prepared.regions;
        debug_assert_eq!(regions.len(), prepared.nodes.len());
        let mut roots = Vec::new();
        for (region_index, region) in regions.iter().enumerate() {
            let Some(region_lines) = lines.get(region.lines.clone()) else {
                continue;
            };
            if let Some(nodes) = prepared.nodes.get(region_index).and_then(Option::as_ref) {
                roots.extend(
                    nodes
                        .iter()
                        .map(|node| Self::materialize_prepared_node(node, reusable, cx)),
                );
                continue;
            }
            let markdown = || region_lines.join("\n");

            match region.kind {
                ProjectionRegionKind::Blank => {
                    let blank_run_len = region_lines.len();
                    let previous_root_is_list_item = roots
                        .last()
                        .map(|block: &Entity<super::Block>| block.read(cx).kind().is_list_item())
                        .unwrap_or(false);
                    let next_root_is_list_item = regions
                        .get(region_index + 1)
                        .is_some_and(|next| next.kind == ProjectionRegionKind::List);
                    let preserved = if roots.is_empty()
                        || (previous_root_is_list_item && next_root_is_list_item)
                    {
                        blank_run_len
                    } else {
                        blank_run_len.saturating_sub(1)
                    };
                    let whole_document_is_blank = roots.is_empty()
                        && region_index + 1 == regions.len()
                        && regions[..region_index]
                            .iter()
                            .all(|candidate| candidate.kind == ProjectionRegionKind::Blank);
                    roots.extend(materialize_blank_run(
                        cx,
                        preserved,
                        whole_document_is_blank,
                    ));
                }
                ProjectionRegionKind::Frontmatter => roots.push(raw_block(cx, markdown())),
                ProjectionRegionKind::FencedCode => {
                    if let Some((block, _)) = collect_fenced_code_block(cx, region_lines, 0) {
                        roots.push(block);
                    } else {
                        roots.push(native_block(cx, BlockKind::Paragraph, markdown()));
                    }
                }
                ProjectionRegionKind::Comment => {
                    if let Some((block, _)) = collect_comment_block(cx, region_lines, 0) {
                        roots.push(block);
                    } else {
                        roots.push(raw_block(cx, markdown()));
                    }
                }
                ProjectionRegionKind::Html => roots.push(html_or_raw_block(cx, markdown())),
                ProjectionRegionKind::FootnoteDefinition => {
                    if let Some(block) = build_native_footnote_definition_block(cx, region_lines) {
                        roots.push(block);
                    } else {
                        roots.push(raw_block(cx, markdown()));
                    }
                }
                ProjectionRegionKind::ReferenceDefinition => {
                    roots.push(raw_block(cx, markdown()));
                }
                ProjectionRegionKind::SetextHeading => {
                    let level = region_lines
                        .get(1)
                        .and_then(|line| BlockKind::parse_setext_underline(line));
                    if let (Some(level), Some(title)) = (level, region_lines.first()) {
                        roots.push(native_block(
                            cx,
                            BlockKind::Heading { level },
                            title.trim_end().to_string(),
                        ));
                    } else {
                        roots.push(native_block(cx, BlockKind::Paragraph, markdown()));
                    }
                }
                ProjectionRegionKind::StandaloneImage => {
                    roots.push(standalone_image_block(cx, markdown()));
                }
                ProjectionRegionKind::StandaloneResource => {
                    roots.push(native_block(cx, BlockKind::Paragraph, markdown()));
                }
                ProjectionRegionKind::IndentedCode => {
                    if let Some((block, _)) = collect_indented_code_block(cx, region_lines, 0) {
                        roots.push(block);
                    } else {
                        roots.push(native_block(cx, BlockKind::Paragraph, markdown()));
                    }
                }
                ProjectionRegionKind::List => {
                    let (blocks, _) = Self::collect_list_blocks(cx, region_lines, 0);
                    roots.extend(blocks);
                }
                ProjectionRegionKind::Quote => {
                    let (block, _) = Self::collect_quote_block(cx, region_lines, 0);
                    roots.push(block);
                }
                ProjectionRegionKind::AtxHeading => {
                    if let Some((level, content)) = region_lines
                        .first()
                        .and_then(|line| BlockKind::parse_atx_heading_line(line))
                    {
                        roots.push(native_block(cx, BlockKind::Heading { level }, content));
                    } else {
                        roots.push(native_block(cx, BlockKind::Paragraph, markdown()));
                    }
                }
                ProjectionRegionKind::Separator => roots.push(Self::new_block(
                    cx,
                    BlockRecord::new(BlockKind::Separator, InlineTextTree::plain(String::new())),
                )),
                ProjectionRegionKind::RootTableCandidate => {
                    if let Some(table) = parse_root_table_region(region_lines) {
                        roots.push(Self::new_block(cx, BlockRecord::table(table)));
                    } else {
                        roots.extend(
                            region_lines
                                .iter()
                                .cloned()
                                .map(|line| plain_text_paragraph_block(cx, line)),
                        );
                    }
                }
                ProjectionRegionKind::PipelessTable => {
                    if let Some(table) = parse_root_table_region(region_lines) {
                        roots.push(Self::new_block(cx, BlockRecord::table(table)));
                    } else {
                        roots.push(native_block(cx, BlockKind::Paragraph, markdown()));
                    }
                }
                ProjectionRegionKind::DisplayMath => {
                    roots.push(math_or_raw_block(cx, markdown()));
                }
                ProjectionRegionKind::Paragraph => {
                    roots.push(native_block(cx, BlockKind::Paragraph, markdown()));
                }
            }
        }
        roots
    }

    /// 只物化一个投影区域，供虚拟 surface 的 viewport/pinned region 按需挂载。
    pub(in crate::editor) fn materialize_projection_region(
        cx: &mut Context<Self>,
        prepared: &PreparedSplitProjection,
        region_index: usize,
        reusable: &mut HashMap<uuid::Uuid, Entity<super::Block>>,
    ) -> Vec<Entity<super::Block>> {
        let Some(region) = prepared.regions.get(region_index) else {
            return Vec::new();
        };
        let Some(region_lines) = prepared.lines.get(region.lines.clone()) else {
            return Vec::new();
        };
        if let Some(nodes) = prepared.nodes.get(region_index).and_then(Option::as_ref) {
            return nodes
                .iter()
                .map(|node| Self::materialize_prepared_node(node, reusable, cx))
                .collect();
        }

        if region.kind == ProjectionRegionKind::Blank {
            let previous_is_list = prepared.regions[..region_index]
                .iter()
                .rev()
                .find(|candidate| candidate.kind != ProjectionRegionKind::Blank)
                .is_some_and(|candidate| candidate.kind == ProjectionRegionKind::List);
            let next_is_list = prepared.regions[region_index + 1..]
                .iter()
                .find(|candidate| candidate.kind != ProjectionRegionKind::Blank)
                .is_some_and(|candidate| candidate.kind == ProjectionRegionKind::List);
            let preserved = if region_index == 0 || (previous_is_list && next_is_list) {
                region_lines.len()
            } else {
                region_lines.len().saturating_sub(1)
            };
            let whole_document_is_blank = prepared
                .regions
                .iter()
                .all(|candidate| candidate.kind == ProjectionRegionKind::Blank);
            return materialize_blank_run(
                cx,
                preserved,
                whole_document_is_blank && region_index + 1 == prepared.regions.len(),
            );
        }

        // 复杂区域继续复用既有权威 importer，但输入已被 region 边界限制，不会扫描全文。
        Self::build_blocks_from_lines_internal(cx, region_lines, true)
    }

    /// 复用必须包含列表起点比较，否则只改源码数字时会继续显示旧编号。
    pub(super) fn materialize_prepared_node(
        node: &PreparedBlockNode,
        reusable: &mut HashMap<uuid::Uuid, Entity<super::Block>>,
        cx: &mut Context<Self>,
    ) -> Entity<super::Block> {
        if let Some(block) = reusable.remove(&node.record.id) {
            let semantic_matches = {
                let current = &block.read(cx).record;
                current.kind == node.record.kind
                    && current.title == node.record.title
                    && current.table == node.record.table
                    && current.html == node.record.html
                    && current.raw_fallback == node.record.raw_fallback
                    && current.resource == node.record.resource
                    && current.ordered_list_start == node.record.ordered_list_start
            };
            if semantic_matches {
                return block;
            }
        }
        let block = Self::new_block(cx, node.record.clone());
        let children = node
            .children
            .iter()
            .map(|child| Self::materialize_prepared_node(child, reusable, cx))
            .collect();
        attach_child_blocks(&block, children, cx);
        block
    }

    pub(super) fn build_blocks_from_lines_internal(
        cx: &mut Context<Self>,
        lines: &[String],
        allow_root_footnote_definitions: bool,
    ) -> Vec<Entity<super::Block>> {
        let mut roots = Vec::new();
        let mut index = 0;

        while index < lines.len() {
            let line = &lines[index];
            if line.trim().is_empty() {
                let blank_start = index;
                while index < lines.len() && lines[index].trim().is_empty() {
                    index += 1;
                }

                let blank_run_len = index - blank_start;
                let previous_root_is_list_item = roots
                    .last()
                    .map(|block: &Entity<super::Block>| block.read(cx).kind().is_list_item())
                    .unwrap_or(false);
                let next_root_is_list_item = lines
                    .get(index)
                    .is_some_and(|line| parse_list_marker(line).is_some());
                let preserved_empty_blocks = if roots.is_empty() {
                    blank_run_len
                } else if previous_root_is_list_item && next_root_is_list_item {
                    blank_run_len
                } else {
                    blank_run_len.saturating_sub(1)
                };

                let whole_document_is_blank = roots.is_empty() && index == lines.len();
                roots.extend(materialize_blank_run(
                    cx,
                    preserved_empty_blocks,
                    whole_document_is_blank,
                ));
                continue;
            }

            if parse_opening_fence(line).is_some() {
                let Some((block, next_index)) = collect_fenced_code_block(cx, lines, index) else {
                    let paragraph = Self::collect_paragraph_block(cx, lines, index);
                    roots.push(paragraph.0);
                    index = paragraph.1;
                    continue;
                };

                roots.push(block);
                index = next_index;
                continue;
            }

            if let Some((block, end)) = collect_comment_block(cx, lines, index) {
                roots.push(block);
                index = end;
                continue;
            }

            if is_block_html_start(line) {
                let end = collect_block_html_region(lines, index);
                roots.push(html_or_raw_block(cx, lines[index..end].join("\n")));
                index = end;
                continue;
            }

            if is_footnote_definition_start(line) {
                let end = collect_footnote_definition_region(lines, index);
                if allow_root_footnote_definitions {
                    if let Some(block) =
                        build_native_footnote_definition_block(cx, &lines[index..end])
                    {
                        roots.push(block);
                    } else {
                        roots.push(raw_block(cx, lines[index..end].join("\n")));
                    }
                } else {
                    roots.push(raw_block(cx, lines[index..end].join("\n")));
                }
                index = end;
                continue;
            }

            if is_reference_definition_start(line) {
                let end = collect_reference_definition_region(lines, index);
                roots.push(raw_block(cx, lines[index..end].join("\n")));
                index = end;
                continue;
            }

            if let Some(level) = lines
                .get(index + 1)
                .and_then(|next| BlockKind::parse_setext_underline(next))
            {
                roots.push(native_block(
                    cx,
                    BlockKind::Heading { level },
                    line.trim_end().to_string(),
                ));
                index += 2;
                continue;
            }

            if parse_standalone_image(line).is_some() {
                roots.push(standalone_image_block(cx, line.to_string()));
                index += 1;
                continue;
            }

            if strip_indented_code_prefix(line).is_some() {
                let Some((block, next_index)) = collect_indented_code_block(cx, lines, index)
                else {
                    unreachable!("indented code prefix disappeared after detection");
                };

                roots.push(block);
                index = next_index;
                continue;
            }

            if parse_list_marker(line).is_some() {
                let (blocks, next_index) = Self::collect_list_blocks(cx, lines, index);
                roots.extend(blocks);
                index = next_index;
                continue;
            }

            if is_quote_start(line) {
                let (block, next_index) = Self::collect_quote_block(cx, lines, index);
                roots.push(block);
                index = next_index;
                continue;
            }

            if let Some((level, content)) = BlockKind::parse_atx_heading_line(line) {
                roots.push(native_block(cx, BlockKind::Heading { level }, content));
                index += 1;
                continue;
            }

            if BlockKind::parse_separator_line(line) {
                roots.push(Self::new_block(
                    cx,
                    BlockRecord::new(BlockKind::Separator, InlineTextTree::plain(String::new())),
                ));
                index += 1;
                continue;
            }

            if is_root_table_candidate_line(line) {
                let end = collect_root_table_candidate_region(lines, index);
                let region = &lines[index..end];
                if let Some(table) = parse_root_table_region(region) {
                    roots.push(Self::new_block(cx, BlockRecord::table(table)));
                } else {
                    roots.extend(
                        region
                            .iter()
                            .cloned()
                            .map(|line| plain_text_paragraph_block(cx, line)),
                    );
                }
                index = end;
                continue;
            }

            if let Some(end) = collect_pipeless_table_region(lines, index)
                && let Some(table) = parse_root_table_region(&lines[index..end])
            {
                roots.push(Self::new_block(cx, BlockRecord::table(table)));
                index = end;
                continue;
            }

            if is_display_math_start(line) {
                let end = collect_display_math_region(lines, index);
                roots.push(math_or_raw_block(cx, lines[index..end].join("\n")));
                index = end;
                continue;
            }

            let paragraph = Self::collect_paragraph_block(cx, lines, index);
            roots.push(paragraph.0);
            index = paragraph.1;
        }

        roots
    }
}
