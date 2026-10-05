// @author kongweiguang

//! Source-offset mapping between canonical Markdown and rendered blocks.

use std::ops::Range;

use gmark_document::{TextEdit, Transaction};

use super::*;

impl Editor {
    /// 将一次已完成的投影编辑提交到源码真值。
    ///
    /// 当前迁移阶段先使用全量替换保证所有旧块编辑路径都被覆盖；后续由
    /// block/source mapping 生成最小 `TextEdit`，接口和 revision 语义保持不变。
    pub(super) fn sync_source_document_from_projection(&mut self, source: &str) {
        let snapshot = self.source_document.snapshot();
        let previous = snapshot.text();
        let Some(edit) = Self::minimal_projection_edit(&previous, source) else {
            return;
        };

        let transaction = Transaction::new(snapshot.revision(), vec![edit]);
        self.source_document
            .apply_transaction(transaction)
            .expect("投影同步必须使用当前 revision 和有效 UTF-8 边界");
    }

    /// 将旧投影和新投影压缩为一个 UTF-8 安全的最小连续替换范围。
    pub(super) fn minimal_projection_edit(previous: &str, current: &str) -> Option<TextEdit> {
        if previous == current {
            return None;
        }

        let prefix = previous
            .chars()
            .zip(current.chars())
            .take_while(|(left, right)| left == right)
            .map(|(ch, _)| ch.len_utf8())
            .sum::<usize>();

        let previous_tail = &previous[prefix..];
        let current_tail = &current[prefix..];
        let suffix = previous_tail
            .chars()
            .rev()
            .zip(current_tail.chars().rev())
            .take_while(|(left, right)| left == right)
            .map(|(ch, _)| ch.len_utf8())
            .sum::<usize>();

        Some(TextEdit::new(
            prefix..previous.len() - suffix,
            current[prefix..current.len() - suffix].to_owned(),
        ))
    }

    pub(super) fn current_document_source(&self, cx: &App) -> String {
        if self.virtual_surface.is_some() && self.view_mode == ViewMode::Rendered {
            return self.source_document.text();
        }
        match self.view_mode {
            ViewMode::Rendered => self.document.markdown_text(cx),
            ViewMode::Source | ViewMode::Split => self.document.raw_source_text(cx),
            ViewMode::Preview => self.source_document.text(),
        }
    }

    /// 调用方已完成根缓存同步时使用，避免 Rendered 模式重新遍历全部 Entity。
    pub(super) fn current_document_source_from_cache(&self, cx: &App) -> String {
        if self.virtual_surface.is_some() && self.view_mode == ViewMode::Rendered {
            return self.source_document.text();
        }
        match self.view_mode {
            ViewMode::Rendered => self.document.cached_markdown_text(cx),
            _ => self.current_document_source(cx),
        }
    }

    pub(super) fn is_empty_paragraph_separator(block: &Block) -> bool {
        block.kind() == BlockKind::Paragraph
            && block.record.title.visible_text().is_empty()
            && block.children.is_empty()
    }

    pub(super) fn is_empty_root_paragraph(block: &Block) -> bool {
        Self::is_empty_paragraph_separator(block)
    }

    pub(super) fn build_prefixed_content_mapping(
        content: &str,
        first_prefix: &str,
        continuation_prefix: &str,
    ) -> (String, Vec<usize>, Vec<usize>) {
        let mut full = String::new();
        let mut content_to_source = vec![0; content.len() + 1];
        let mut source_to_content = vec![0];

        full.push_str(first_prefix);
        source_to_content.resize(full.len() + 1, 0);

        let mut content_offset = 0usize;
        while content_offset < content.len() {
            content_to_source[content_offset] = full.len();
            let ch = content[content_offset..]
                .chars()
                .next()
                .expect("content offset should stay on char boundaries");
            let start = full.len();
            full.push(ch);
            source_to_content.resize(full.len() + 1, content_offset);
            for index in start..=full.len() {
                source_to_content[index] = content_offset;
            }
            content_offset += ch.len_utf8();
            if ch == '\n' {
                let prefix_start = full.len();
                full.push_str(continuation_prefix);
                source_to_content.resize(full.len() + 1, content_offset);
                for index in prefix_start..=full.len() {
                    source_to_content[index] = content_offset;
                }
            }
        }
        content_to_source[content.len()] = full.len();
        source_to_content[full.len()] = content.len();

        (full, content_to_source, source_to_content)
    }

    pub(super) fn build_code_block_content_mapping(
        content: &str,
        indentation: &str,
        language: Option<&SharedString>,
    ) -> (String, Vec<usize>, Vec<usize>) {
        let fence = self::persistence::safe_code_fence_with_info(
            content,
            language.map(|language| language.as_ref()),
        );
        let mut full = String::new();
        let mut content_to_source = vec![0; content.len() + 1];
        let mut source_to_content = vec![0];

        full.push_str(&fence);
        if let Some(language) = language {
            full.push_str(language);
        }
        full.push('\n');
        source_to_content.resize(full.len() + 1, 0);

        let prefix_start = full.len();
        full.push_str(indentation);
        source_to_content.resize(full.len() + 1, 0);
        for index in prefix_start..=full.len() {
            source_to_content[index] = 0;
        }

        let mut content_offset = 0usize;
        while content_offset < content.len() {
            content_to_source[content_offset] = full.len();
            let ch = content[content_offset..]
                .chars()
                .next()
                .expect("content offset should stay on char boundaries");
            let start = full.len();
            full.push(ch);
            source_to_content.resize(full.len() + 1, content_offset);
            for index in start..=full.len() {
                source_to_content[index] = content_offset;
            }
            content_offset += ch.len_utf8();
            if ch == '\n' {
                let line_prefix_start = full.len();
                full.push_str(indentation);
                source_to_content.resize(full.len() + 1, content_offset);
                for index in line_prefix_start..=full.len() {
                    source_to_content[index] = content_offset;
                }
            }
        }
        content_to_source[content.len()] = full.len();
        source_to_content[full.len()] = content.len();

        full.push('\n');
        source_to_content.resize(full.len() + 1, content.len());
        full.push_str(&fence);
        source_to_content.resize(full.len() + 1, content.len());
        source_to_content[full.len()] = content.len();

        (full, content_to_source, source_to_content)
    }

    pub(super) fn push_inline_block_mapping(
        &self,
        block: &Entity<Block>,
        content_markdown: String,
        first_prefix: String,
        continuation_prefix: String,
        quote_depth: usize,
        absolute_start: usize,
        mappings: &mut Vec<SourceTargetMapping>,
    ) -> usize {
        let (full_text, content_to_source, source_to_content) =
            Self::build_prefixed_content_mapping(
                &content_markdown,
                &first_prefix,
                &continuation_prefix,
            );
        let (full_text, content_to_source, source_to_content) =
            Self::wrap_source_mapping_with_quotes(
                full_text,
                content_to_source,
                source_to_content,
                quote_depth,
            );
        mappings.push(SourceTargetMapping {
            entity: block.clone(),
            full_source_range: absolute_start..absolute_start + full_text.len(),
            content_to_source,
            source_to_content,
        });
        full_text.len()
    }
}

#[path = "source_mapping_parts/mapper.rs"]
mod mapper;

#[path = "source_mapping_parts/spelling.rs"]
mod spelling;
pub(super) use spelling::SourceSpellingMap;
