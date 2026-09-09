// @author kongweiguang

use super::*;

pub(super) fn collect_list_item_region(
    lines: &[String],
    start: usize,
    marker_indent_columns: usize,
) -> usize {
    let mut index = start + 1;
    let mut pending_blank_lines = 0usize;
    while index < lines.len() {
        let line = &lines[index];
        if line.trim().is_empty() {
            pending_blank_lines += 1;
            index += 1;
            continue;
        }

        if parse_list_marker(line)
            .is_some_and(|marker| marker.indent_columns <= marker_indent_columns)
        {
            return index.saturating_sub(pending_blank_lines);
        }

        if parse_list_marker(line).is_some() {
            pending_blank_lines = 0;
            index += 1;
            continue;
        }

        let (indent_columns, _) = leading_indent_columns_and_bytes(line);
        if indent_columns > marker_indent_columns || pending_blank_lines == 0 {
            pending_blank_lines = 0;
            index += 1;
            continue;
        }

        return index.saturating_sub(pending_blank_lines);
    }
    index
}

pub(super) fn looks_like_root_block_start(lines: &[String], index: usize) -> bool {
    let line = &lines[index];
    if line.trim().is_empty() {
        return true;
    }

    parse_opening_fence(line).is_some()
        || is_block_html_start(line)
        || is_footnote_definition_start(line)
        || is_reference_definition_start(line)
        || strip_indented_code_prefix(line).is_some()
        || parse_list_marker(line).is_some()
        || is_quote_start(line)
        || BlockKind::parse_atx_heading_line(line).is_some()
        || BlockKind::parse_separator_line(line)
        || lines
            .get(index + 1)
            .and_then(|next| BlockKind::parse_setext_underline(next))
            .is_some()
        || is_root_table_candidate_line(line)
        || is_display_math_start(line)
}

pub(super) fn projection_region(
    kind: ProjectionRegionKind,
    lines: &[String],
    line_offsets: &[usize],
    start: usize,
    end: usize,
) -> ProjectionRegion {
    let byte_start = line_offsets[start];
    let byte_end = line_offsets[end - 1] + lines[end - 1].len();
    ProjectionRegion {
        kind,
        lines: start..end,
        bytes: byte_start..byte_end,
    }
}

pub(super) fn collect_frontmatter_region(lines: &[String]) -> Option<usize> {
    let opening = lines.first()?.strip_prefix('\u{feff}').unwrap_or(&lines[0]);
    if opening.trim_end() != "---" {
        return None;
    }

    // YAML 文档结束标记必须从行首开始；缩进在标量内容里的 `---` 不能提前闭合。
    lines.iter().enumerate().skip(1).find_map(|(index, line)| {
        let fence = line.trim_end();
        matches!(fence, "---" | "...").then_some(index + 1)
    })
}

/// 将源码切分为顶层区域。该扫描不创建 Entity，可在后台线程执行。
pub(in crate::editor) fn scan_projection_regions(lines: &[String]) -> Vec<ProjectionRegion> {
    scan_projection_regions_from_offset(lines, true)
}

/// 扫描增量后缀时必须显式说明它是否仍位于文档起点，避免正文中的 `---`
/// 因切片后恰好落在首行而被误识别为 frontmatter。
pub(in crate::editor) fn scan_projection_regions_from_offset(
    lines: &[String],
    allow_frontmatter: bool,
) -> Vec<ProjectionRegion> {
    let mut line_offsets = Vec::with_capacity(lines.len());
    let mut byte_offset = 0usize;
    for line in lines {
        line_offsets.push(byte_offset);
        byte_offset += line.len() + 1;
    }

    let mut regions = Vec::new();
    let mut index = 0usize;
    while index < lines.len() {
        let start = index;
        let line = &lines[index];
        let kind;

        if allow_frontmatter
            && index == 0
            && let Some(end) = collect_frontmatter_region(lines)
        {
            index = end;
            kind = ProjectionRegionKind::Frontmatter;
        } else if line.trim().is_empty() {
            while index < lines.len() && lines[index].trim().is_empty() {
                index += 1;
            }
            kind = ProjectionRegionKind::Blank;
        } else if let Some(fence) = parse_opening_fence(line) {
            if let Some(closing) = find_matching_closing_fence(lines, index, &fence) {
                index = closing + 1;
                kind = ProjectionRegionKind::FencedCode;
            } else {
                index = collect_paragraph_region_end(lines, index);
                kind = ProjectionRegionKind::Paragraph;
            }
        } else if let Some(end) = collect_closed_html_comment_region(lines, index) {
            index = end;
            kind = ProjectionRegionKind::Comment;
        } else if is_block_html_start(line) {
            index = collect_block_html_region(lines, index);
            kind = ProjectionRegionKind::Html;
        } else if is_footnote_definition_start(line) {
            index = collect_footnote_definition_region(lines, index);
            kind = ProjectionRegionKind::FootnoteDefinition;
        } else if is_reference_definition_start(line) {
            index = collect_reference_definition_region(lines, index);
            kind = ProjectionRegionKind::ReferenceDefinition;
        } else if lines
            .get(index + 1)
            .and_then(|next| BlockKind::parse_setext_underline(next))
            .is_some()
        {
            index += 2;
            kind = ProjectionRegionKind::SetextHeading;
        } else if parse_standalone_image(line).is_some() {
            index += 1;
            kind = ProjectionRegionKind::StandaloneImage;
        } else if strip_indented_code_prefix(line).is_some() {
            index += 1;
            while index < lines.len()
                && (strip_indented_code_prefix(&lines[index]).is_some()
                    || lines[index].trim().is_empty())
            {
                index += 1;
            }
            kind = ProjectionRegionKind::IndentedCode;
        } else if parse_list_marker(line).is_some() {
            while index < lines.len() {
                let Some(marker) = parse_list_marker(&lines[index]) else {
                    break;
                };
                index = collect_list_item_region(lines, index, marker.indent_columns);
            }
            kind = ProjectionRegionKind::List;
        } else if is_quote_start(line) {
            index = collect_quote_raw_region(lines, index);
            kind = ProjectionRegionKind::Quote;
        } else if ResourceRecord::parse(line, None).is_some() {
            index += 1;
            kind = ProjectionRegionKind::StandaloneResource;
        } else if BlockKind::parse_atx_heading_line(line).is_some() {
            index += 1;
            kind = ProjectionRegionKind::AtxHeading;
        } else if BlockKind::parse_separator_line(line) {
            index += 1;
            kind = ProjectionRegionKind::Separator;
        } else if is_root_table_candidate_line(line) {
            index = collect_root_table_candidate_region(lines, index);
            kind = ProjectionRegionKind::RootTableCandidate;
        } else if let Some(end) = collect_pipeless_table_region(lines, index)
            && parse_root_table_region(&lines[index..end]).is_some()
        {
            index = end;
            kind = ProjectionRegionKind::PipelessTable;
        } else if is_display_math_start(line) {
            index = collect_display_math_region(lines, index);
            kind = ProjectionRegionKind::DisplayMath;
        } else {
            index = collect_paragraph_region_end(lines, index);
            kind = ProjectionRegionKind::Paragraph;
        }

        regions.push(projection_region(kind, lines, &line_offsets, start, index));
    }
    regions
}

pub(super) fn collect_paragraph_region_end(lines: &[String], start: usize) -> usize {
    let mut paragraph_lines = vec![lines[start].clone()];
    let mut index = start + 1;
    while index < lines.len() {
        if (lines[index].trim().is_empty() || looks_like_root_block_start(lines, index))
            && !paragraph_can_continue_through_boundary(&paragraph_lines, lines, index)
        {
            break;
        }
        paragraph_lines.push(lines[index].clone());
        index += 1;
    }
    index
}

/// 为非递归顶层区域生成纯 `BlockRecord`。返回值与 `regions` 保持一一对应，
/// 递归容器和空行返回 `None`，由 UI 安装器根据相邻状态完成结构组装。
pub(in crate::editor) fn prepare_projection_nodes(
    lines: &[String],
    regions: &[ProjectionRegion],
) -> Vec<Option<Arc<[PreparedBlockNode]>>> {
    prepare_projection_nodes_internal(lines, regions, true)
}

pub(super) fn prepare_projection_nodes_internal(
    lines: &[String],
    regions: &[ProjectionRegion],
    allow_root_footnote_definitions: bool,
) -> Vec<Option<Arc<[PreparedBlockNode]>>> {
    if regions.len() >= 1024 {
        regions
            .par_iter()
            .map(|region| prepare_projection_region(lines, region, allow_root_footnote_definitions))
            .collect()
    } else {
        regions
            .iter()
            .map(|region| prepare_projection_region(lines, region, allow_root_footnote_definitions))
            .collect()
    }
}

pub(super) fn prepare_projection_region(
    lines: &[String],
    region: &ProjectionRegion,
    allow_root_footnote_definitions: bool,
) -> Option<Arc<[PreparedBlockNode]>> {
    let region_lines = lines.get(region.lines.clone())?;
    let markdown = || region_lines.join("\n");
    let records = match region.kind {
        ProjectionRegionKind::Blank => return None,
        ProjectionRegionKind::Frontmatter => vec![BlockRecord::raw_markdown(markdown())],
        ProjectionRegionKind::FootnoteDefinition => {
            if allow_root_footnote_definitions {
                return prepare_footnote_node(region_lines)
                    .map(|node| Arc::from(vec![node].into_boxed_slice()));
            }
            vec![BlockRecord::raw_markdown(markdown())]
        }
        ProjectionRegionKind::List => {
            return prepare_simple_list_nodes(region_lines)
                .map(|nodes| Arc::from(nodes.into_boxed_slice()));
        }
        ProjectionRegionKind::Quote => {
            return prepare_simple_quote_node(region_lines)
                .map(|node| Arc::from(vec![node].into_boxed_slice()));
        }
        ProjectionRegionKind::FencedCode => {
            vec![collect_fenced_code_record(region_lines, 0)?.0]
        }
        ProjectionRegionKind::Comment => vec![BlockRecord::comment(markdown())],
        ProjectionRegionKind::Html => vec![html_or_raw_record(markdown())],
        ProjectionRegionKind::ReferenceDefinition => {
            vec![BlockRecord::raw_markdown(markdown())]
        }
        ProjectionRegionKind::SetextHeading => {
            let level = region_lines
                .get(1)
                .and_then(|line| BlockKind::parse_setext_underline(line))?;
            let title = region_lines.first()?.trim_end().to_string();
            vec![native_record(BlockKind::Heading { level }, title)]
        }
        ProjectionRegionKind::StandaloneImage => {
            vec![BlockRecord::paragraph(markdown().trim().to_string())]
        }
        ProjectionRegionKind::StandaloneResource => {
            vec![record_from_markdown_value(markdown().trim())?]
        }
        ProjectionRegionKind::IndentedCode => {
            vec![collect_indented_code_record(region_lines, 0)?.0]
        }
        ProjectionRegionKind::AtxHeading => {
            let (level, content) = region_lines
                .first()
                .and_then(|line| BlockKind::parse_atx_heading_line(line))?;
            vec![native_record(BlockKind::Heading { level }, content)]
        }
        ProjectionRegionKind::Separator => vec![BlockRecord::new(
            BlockKind::Separator,
            InlineTextTree::plain(String::new()),
        )],
        ProjectionRegionKind::RootTableCandidate => {
            if let Some(record) = record_from_markdown_value(&markdown())
                .filter(|record| record.kind == BlockKind::Table)
            {
                vec![record]
            } else {
                region_lines
                    .iter()
                    .cloned()
                    .map(BlockRecord::paragraph)
                    .collect()
            }
        }
        ProjectionRegionKind::PipelessTable => {
            vec![record_from_markdown_value(&markdown())?]
        }
        ProjectionRegionKind::DisplayMath => vec![math_or_raw_record(markdown())],
        ProjectionRegionKind::Paragraph => {
            vec![native_record(BlockKind::Paragraph, markdown())]
        }
    };
    Some(Arc::from(
        records
            .into_iter()
            .map(PreparedBlockNode::leaf)
            .collect::<Vec<_>>()
            .into_boxed_slice(),
    ))
}

fn record_from_markdown_value(markdown: &str) -> Option<BlockRecord> {
    let document = parse_markdown_value(markdown);
    let [block] = document.blocks.as_slice() else {
        return None;
    };
    BlockRecord::from_markdown_value(block)
}

/// 后台快速投影也携带源码编号，确保简单列表和复杂 importer 的语义一致。
pub(super) fn prepare_simple_list_nodes(lines: &[String]) -> Option<Vec<PreparedBlockNode>> {
    let mut nodes = Vec::new();
    let mut index = 0usize;
    while index < lines.len() {
        let marker = parse_list_marker(&lines[index])?;
        let item_end = collect_list_item_region(lines, index, marker.indent_columns);
        let mut node = PreparedBlockNode::leaf(native_record(marker.kind, marker.text));
        node.record.ordered_list_start = marker.ordered_start;
        if item_end > index + 1 {
            let body = &lines[index + 1..item_end];
            if body.iter().any(|line| line.trim().is_empty()) {
                return None;
            }
            let (indent, _) = leading_indent_columns_and_bytes(body.first()?);
            if indent <= marker.indent_columns {
                return None;
            }
            let nested = dedent_lines(body, indent);
            if parse_list_marker(nested.first()?).is_none() {
                return None;
            }
            node.children = prepare_simple_list_nodes(&nested)?;
        }
        nodes.push(node);
        index = item_end;
    }
    (!nodes.is_empty()).then_some(nodes)
}

pub(super) fn prepare_simple_quote_node(lines: &[String]) -> Option<PreparedBlockNode> {
    let mut dequoted = Vec::with_capacity(lines.len());
    for line in lines {
        let content = strip_one_quote_level(line)?;
        if content.trim().is_empty() {
            return None;
        }
        dequoted.push(content);
    }

    let is_nested = |index: usize| {
        let line = &dequoted[index];
        parse_list_marker(line).is_some()
            || is_quote_start(line)
            || parse_opening_fence(line).is_some()
            || strip_indented_code_prefix(line).is_some()
            || quote_content_starts_unsupported(&dequoted, index)
            || parse_standalone_image(line).is_some()
    };
    if (0..dequoted.len()).any(is_nested) {
        return None;
    }

    if let Some((variant, title)) = dequoted
        .first()
        .and_then(|line| CalloutVariant::parse_header_line(line))
    {
        let children = if dequoted.len() > 1 {
            vec![PreparedBlockNode::leaf(native_record(
                BlockKind::Paragraph,
                dequoted[1..].join("\n"),
            ))]
        } else {
            Vec::new()
        };
        return Some(PreparedBlockNode {
            record: native_record(BlockKind::Callout(variant), title),
            children,
        });
    }

    Some(PreparedBlockNode::leaf(native_record(
        BlockKind::Quote,
        dequoted.join("\n"),
    )))
}

pub(super) fn prepare_footnote_node(lines: &[String]) -> Option<PreparedBlockNode> {
    let (id, first_line) = parse_footnote_definition_head(lines.first()?)?;
    let mut body_lines = Vec::new();
    if !first_line.is_empty() {
        body_lines.push(first_line);
    }
    for line in lines.iter().skip(1) {
        if line.trim().is_empty() {
            body_lines.push(String::new());
        } else {
            body_lines.push(
                strip_leading_columns(line, 4)
                    .unwrap_or(line.as_str())
                    .to_string(),
            );
        }
    }

    let children = prepare_complete_nodes(&body_lines, false)?;
    Some(PreparedBlockNode {
        record: BlockRecord::new(BlockKind::FootnoteDefinition, InlineTextTree::plain(id)),
        children,
    })
}

pub(super) fn prepare_complete_nodes(
    lines: &[String],
    allow_root_footnote_definitions: bool,
) -> Option<Vec<PreparedBlockNode>> {
    let regions = scan_projection_regions(lines);
    let prepared =
        prepare_projection_nodes_internal(lines, &regions, allow_root_footnote_definitions);
    let mut roots: Vec<PreparedBlockNode> = Vec::new();
    for (index, (region, nodes)) in regions.iter().zip(prepared).enumerate() {
        if region.kind == ProjectionRegionKind::Blank {
            let previous_is_list = roots
                .last()
                .is_some_and(|node| node.record.kind.is_list_item());
            let next_is_list = regions
                .get(index + 1)
                .is_some_and(|next| next.kind == ProjectionRegionKind::List);
            let blank_count = region.lines.len();
            let preserved = if roots.is_empty() || (previous_is_list && next_is_list) {
                blank_count
            } else {
                blank_count.saturating_sub(1)
            };
            roots.extend(
                (0..preserved).map(|_| PreparedBlockNode::leaf(BlockRecord::source_blank())),
            );
        } else {
            roots.extend(nodes?.iter().cloned());
        }
    }
    Some(roots)
}

pub(super) fn attach_child_blocks(
    parent: &Entity<super::Block>,
    children: Vec<Entity<super::Block>>,
    cx: &mut Context<Editor>,
) {
    if children.is_empty() {
        return;
    }

    parent.update(cx, move |parent, _cx| {
        parent.children.extend(children);
    });
}

pub(super) fn code_block_record(language: Option<SharedString>, content: String) -> BlockRecord {
    BlockRecord::new(
        BlockKind::CodeBlock { language },
        InlineTextTree::plain(content),
    )
}

pub(super) fn collect_fenced_code_record(
    lines: &[String],
    start: usize,
) -> Option<(BlockRecord, usize)> {
    let fence = parse_opening_fence(&lines[start])?;
    let closing_index = find_matching_closing_fence(lines, start, &fence)?;
    if is_mermaid_info_string(fence.language.as_ref().map(|language| language.as_ref())) {
        return Some((
            BlockRecord::mermaid(lines[start..=closing_index].join("\n")),
            closing_index + 1,
        ));
    }

    let content = lines[start + 1..closing_index].join("\n");
    Some((
        code_block_record(fence.language.clone(), content),
        closing_index + 1,
    ))
}

pub(super) fn collect_fenced_code_block(
    cx: &mut Context<Editor>,
    lines: &[String],
    start: usize,
) -> Option<(Entity<super::Block>, usize)> {
    let (record, end) = collect_fenced_code_record(lines, start)?;
    Some((Editor::new_block(cx, record), end))
}

pub(super) fn collect_indented_code_record(
    lines: &[String],
    start: usize,
) -> Option<(BlockRecord, usize)> {
    let stripped = strip_indented_code_prefix(&lines[start])?;
    let mut code_lines = vec![stripped.to_string()];
    let mut code_index = start + 1;
    while code_index < lines.len() {
        if let Some(stripped) = strip_indented_code_prefix(&lines[code_index]) {
            code_lines.push(stripped.to_string());
            code_index += 1;
        } else if lines[code_index].trim().is_empty() {
            code_lines.push(String::new());
            code_index += 1;
        } else {
            break;
        }
    }
    Some((code_block_record(None, code_lines.join("\n")), code_index))
}

pub(super) fn collect_indented_code_block(
    cx: &mut Context<Editor>,
    lines: &[String],
    start: usize,
) -> Option<(Entity<super::Block>, usize)> {
    let (record, end) = collect_indented_code_record(lines, start)?;
    Some((Editor::new_block(cx, record), end))
}

pub(super) fn raw_block(cx: &mut Context<Editor>, markdown: String) -> Entity<super::Block> {
    Editor::new_block(cx, BlockRecord::raw_markdown(markdown))
}

pub(super) fn comment_block(cx: &mut Context<Editor>, markdown: String) -> Entity<super::Block> {
    Editor::new_block(cx, BlockRecord::comment(markdown))
}

pub(super) fn html_or_raw_block(
    cx: &mut Context<Editor>,
    markdown: String,
) -> Entity<super::Block> {
    Editor::new_block(cx, html_or_raw_record(markdown))
}

pub(super) fn html_or_raw_record(markdown: String) -> BlockRecord {
    let document = parse_html_document(&markdown);
    if document.safety == HtmlSafetyClass::Semantic
        && !should_keep_unclosed_html_projection_raw(&markdown)
    {
        // 复用已经分类完成的 HtmlDocument，避免 BlockRecord::html 再解析一次。
        let mut record = BlockRecord::with_plain_text(BlockKind::HtmlBlock, markdown.clone());
        record.html = Some(document);
        record.raw_fallback = Some(markdown);
        record
    } else {
        BlockRecord::raw_markdown(markdown)
    }
}

/// 保留编辑器对未闭合块 HTML 的历史源码回退语义。
///
/// 领域层仍使用 html5ever 的 HTML5 自动修复能力；这里只限制 Markdown
/// projection，避免一个未闭合的行首标签吸收后续文本后再以可视 HTML 形式
/// 脱离原始编辑边界。完整闭合的片段和领域测试不受影响。
fn should_keep_unclosed_html_projection_raw(markdown: &str) -> bool {
    let first_line = markdown.lines().next().unwrap_or_default();
    let Some(HtmlBlockStart::Tag {
        name,
        self_closing,
        closes_same_line,
    }) = parse_html_block_start(first_line)
    else {
        return false;
    };
    if self_closing || closes_same_line {
        return false;
    }

    let lower = markdown.to_ascii_lowercase();
    !lower.contains(&format!("</{}", name.to_ascii_lowercase()))
}

pub(super) fn math_or_raw_block(
    cx: &mut Context<Editor>,
    markdown: String,
) -> Entity<super::Block> {
    Editor::new_block(cx, math_or_raw_record(markdown))
}

pub(super) fn math_or_raw_record(markdown: String) -> BlockRecord {
    if parse_display_math_source(&markdown).is_some() {
        BlockRecord::math(markdown)
    } else {
        BlockRecord::raw_markdown(markdown)
    }
}

pub(super) fn collect_comment_block(
    cx: &mut Context<Editor>,
    lines: &[String],
    start: usize,
) -> Option<(Entity<super::Block>, usize)> {
    let end = collect_closed_html_comment_region(lines, start)?;
    Some((comment_block(cx, lines[start..end].join("\n")), end))
}

pub(super) fn native_block(
    cx: &mut Context<Editor>,
    kind: BlockKind,
    markdown: String,
) -> Entity<super::Block> {
    Editor::new_block(cx, native_record(kind, markdown))
}

pub(super) fn native_record(kind: BlockKind, markdown: String) -> BlockRecord {
    if kind == BlockKind::Paragraph {
        if let Some(resource) = ResourceRecord::parse(&markdown, None) {
            return BlockRecord::resource(resource);
        }
    }
    BlockRecord::new(kind, InlineTextTree::from_markdown(&markdown))
}

pub(super) fn source_blank_block(cx: &mut Context<Editor>) -> Entity<super::Block> {
    Editor::new_block(cx, BlockRecord::source_blank())
}

pub(super) fn standalone_image_block(
    cx: &mut Context<Editor>,
    markdown: String,
) -> Entity<super::Block> {
    Editor::new_block(cx, BlockRecord::paragraph(markdown.trim().to_string()))
}

pub(super) fn is_standalone_image_paragraph(lines: &[String]) -> bool {
    lines.len() == 1 && parse_standalone_image(&lines[0]).is_some()
}

pub(super) fn starts_with_standalone_image_child_paragraph(lines: &[String]) -> bool {
    if lines.is_empty() || !is_standalone_image_paragraph(&lines[..1]) {
        return false;
    }

    lines.get(1).is_none_or(|next| {
        next.trim().is_empty()
            || parse_list_marker(next).is_some()
            || is_quote_start(next)
            || parse_opening_fence(next).is_some()
            || strip_indented_code_prefix(next).is_some()
            || is_block_html_start(next)
            || is_footnote_definition_start(next)
            || is_reference_definition_start(next)
            || is_root_table_candidate_line(next)
            || is_display_math_start(next)
    })
}

pub(super) fn append_markdown_to_block(
    block: &Entity<super::Block>,
    separator: &str,
    markdown: &str,
    cx: &mut Context<Editor>,
) {
    block.update(cx, |block, _cx| {
        let mut title = block.record.title.clone();
        if !separator.is_empty() {
            title.append_tree(InlineTextTree::plain(separator.to_string()));
        }
        title.append_tree(InlineTextTree::from_markdown(markdown));
        block.record.set_title(title);
        block.sync_edit_mode_from_kind();
        block.sync_render_cache();
    });
}

pub(super) fn plain_text_paragraph_block(
    cx: &mut Context<Editor>,
    text: String,
) -> Entity<super::Block> {
    Editor::new_block(cx, BlockRecord::paragraph(text))
}

pub(super) fn append_quote_separator_children(
    children: &mut Vec<Entity<super::Block>>,
    count: usize,
    cx: &mut Context<Editor>,
) {
    for _ in 0..count {
        children.push(source_blank_block(cx));
    }
}

pub(super) fn build_native_footnote_definition_block(
    cx: &mut Context<Editor>,
    lines: &[String],
) -> Option<Entity<super::Block>> {
    let (id, first_line) = parse_footnote_definition_head(lines.first()?)?;
    let mut body_lines = Vec::new();
    if !first_line.is_empty() {
        body_lines.push(first_line);
    }

    for line in lines.iter().skip(1) {
        if line.trim().is_empty() {
            body_lines.push(String::new());
        } else {
            body_lines.push(
                strip_leading_columns(line, 4)
                    .unwrap_or(line.as_str())
                    .to_string(),
            );
        }
    }

    let children = Editor::build_blocks_from_lines_internal(cx, &body_lines, false);
    let block = Editor::new_block(
        cx,
        BlockRecord::new(BlockKind::FootnoteDefinition, InlineTextTree::plain(id)),
    );
    attach_child_blocks(&block, children, cx);
    Some(block)
}

impl Editor {}
