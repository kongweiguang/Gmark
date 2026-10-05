// @author kongweiguang

//! Runtime context synchronization for blocks, references, images, and focus.

use std::collections::HashSet;
use std::sync::Arc;

use super::*;
use crate::components::TocRuntimeEntry;

impl Editor {
    pub(super) fn current_edit_target_entity_id_from_state(&self, cx: &App) -> Option<EntityId> {
        self.active_entity_id
            .filter(|entity_id| self.focusable_entity_by_id(*entity_id).is_some())
            .or_else(|| {
                self.pending_focus
                    .filter(|entity_id| self.focusable_entity_by_id(*entity_id).is_some())
            })
            .or_else(|| self.first_focusable_entity_id(cx))
    }

    pub(super) fn current_edit_target_from_state(&self, cx: &App) -> Option<Entity<Block>> {
        self.current_edit_target_entity_id_from_state(cx)
            .and_then(|entity_id| self.focusable_entity_by_id(entity_id))
    }

    fn end_block_pointer_selection_sessions_inner(
        &mut self,
        cx: &mut Context<Self>,
        notify: bool,
    ) -> bool {
        let mut changed = false;

        if let Some(target) = self.current_edit_target_from_state(cx) {
            target.update(cx, |block, _cx| {
                changed |= block.end_pointer_selection_session();
            });
        }

        for visible in self.document.visible_blocks().to_vec() {
            visible.entity.update(cx, |block, _cx| {
                changed |= block.end_pointer_selection_session();
            });
        }

        // Collect only the cell Entity handles, not the whole TableCellBinding
        // (3-field struct of Entity<Block> + TableCellPosition). The collect()
        // exists to drop the &self borrow before the .update() loop; cloning
        // an Entity is an Arc bump, so we pay that once per cell either way —
        // skipping the surrounding struct clone makes the per-frame work
        // proportional to "cell count" not "binding count + position copy".
        let cells: Vec<Entity<Block>> = self
            .table_cells
            .values()
            .map(|binding| binding.cell.clone())
            .collect();
        for cell in cells {
            cell.update(cx, |block, _cx| {
                changed |= block.end_pointer_selection_session();
            });
        }

        if changed && notify {
            cx.notify();
        }
        changed
    }

    pub(super) fn end_block_pointer_selection_sessions(&mut self, cx: &mut Context<Self>) -> bool {
        self.end_block_pointer_selection_sessions_inner(cx, true)
    }

    /// Creates a new block entity and subscribes this editor to its
    /// [`BlockEvent`](crate::components::BlockEvent) stream.
    pub(super) fn new_block(cx: &mut Context<Self>, record: BlockRecord) -> Entity<Block> {
        let frontmatter = record.is_yaml_frontmatter();
        let block = cx.new(|cx| {
            let mut block = Block::with_record(cx, record);
            // Live/Preview 中 frontmatter 是整理后的元数据区；修改原始 fence 与 YAML
            // 必须进入 Source 或 Split 左栏，避免隐藏语法与光标偏移不一致。
            block.set_read_only(frontmatter);
            block.set_ime_interactions_managed(true);
            block
        });
        cx.subscribe(&block, Self::on_block_event).detach();
        block
    }

    pub(super) fn new_table_cell_block(
        cx: &mut Context<Self>,
        title: InlineTextTree,
        position: TableCellPosition,
        alignment: TableColumnAlignment,
    ) -> Entity<Block> {
        let block = Self::new_block(cx, BlockRecord::new(BlockKind::Paragraph, title));
        block.update(cx, |block, _cx| {
            block.set_table_cell_mode(position, alignment);
        });
        block
    }

    pub(super) fn image_base_dir(&self) -> Option<PathBuf> {
        self.file_path
            .as_ref()
            .and_then(|path| path.parent().map(Path::to_path_buf))
            .or_else(|| std::env::current_dir().ok())
    }

    pub(super) fn sync_runtime_context_for_block(
        &self,
        block: &Entity<Block>,
        base_dir: Option<&Path>,
        cx: &mut Context<Self>,
    ) {
        let next_base_dir = base_dir.map(Path::to_path_buf);
        let image_reference_definitions = self.image_reference_definitions.clone();
        let link_reference_definitions = self.link_reference_definitions.clone();
        let footnote_registry = self.footnote_registry.clone();
        let document_revision = self.source_document.revision();
        block.update(cx, move |block, cx| {
            block.set_document_revision(document_revision);
            block.set_runtime_context_with_probe(
                next_base_dir.clone(),
                image_reference_definitions.clone(),
                link_reference_definitions.clone(),
                footnote_registry.clone(),
                cx,
            );
            cx.notify();
        });
    }

    pub(super) fn rebuild_footnote_registry(&mut self, cx: &App) {
        let mut definitions = HashMap::new();
        let mut visible = self.document.visible_blocks().to_vec();
        if let Some(surface) = self.virtual_surface.as_ref() {
            let existing = visible
                .iter()
                .map(|visible| visible.entity.entity_id())
                .collect::<HashSet<_>>();
            let mut mounted = Vec::new();
            for root in surface.flattened_roots() {
                collect_runtime_surface_entities(&root, &mut mounted, cx);
            }
            visible.extend(
                mounted
                    .into_iter()
                    .filter(|entity| !existing.contains(&entity.entity_id()))
                    .map(|entity| super::tree::VisibleBlock { entity }),
            );
        }
        for visible_block in &visible {
            let block = visible_block.entity.read(cx);
            if block.kind() != BlockKind::FootnoteDefinition {
                continue;
            }

            let allow_definition = self.virtual_surface.as_ref().is_some_and(|surface| {
                surface
                    .region_for_entity(visible_block.entity.entity_id())
                    .is_some()
            }) || self
                .document
                .find_block_location(visible_block.entity.entity_id())
                .is_some_and(|location| {
                    location.parent.is_none()
                        || location
                            .parent
                            .as_ref()
                            .is_some_and(|parent| parent.read(cx).kind().is_quote_container())
                });
            if !allow_definition {
                continue;
            }

            definitions
                .entry(block.record.title.visible_text().to_string())
                .or_insert(visible_block.entity.entity_id());
        }

        let mut bindings = HashMap::<String, FootnoteDefinitionBinding>::new();
        for (id, entity_id) in definitions {
            let virtual_ordinal = self
                .virtual_surface
                .as_ref()
                .and_then(|surface| surface.footnote_ordinal(&id));
            bindings.insert(
                id,
                FootnoteDefinitionBinding {
                    ordinal: virtual_ordinal,
                    definition_entity_id: entity_id,
                    first_reference: None,
                },
            );
        }

        let mut next_ordinal = 1usize;
        let mut occurrence_index = 0usize;
        let mut block_occurrences = HashMap::<uuid::Uuid, Vec<FootnoteResolvedOccurrence>>::new();
        for visible_block in visible {
            let block = visible_block.entity.read(cx);
            let block_id = block.record.id;
            for fragment in &block.record.title.fragments {
                let Some(footnote) = fragment.footnote.as_ref() else {
                    continue;
                };
                let virtual_ordinal = self
                    .virtual_surface
                    .as_ref()
                    .and_then(|surface| surface.footnote_ordinal(&footnote.id));
                let ordinal = if let Some(ordinal) = virtual_ordinal {
                    if let Some(binding) = bindings.get_mut(&footnote.id)
                        && binding.first_reference.is_none()
                    {
                        binding.first_reference = Some(FootnoteReferenceLocation {
                            entity_id: visible_block.entity.entity_id(),
                            occurrence_index,
                        });
                    }
                    Some(ordinal)
                } else if let Some(binding) = bindings.get_mut(&footnote.id) {
                    if binding.ordinal.is_none() {
                        binding.ordinal = Some(next_ordinal);
                        next_ordinal += 1;
                    }
                    if binding.first_reference.is_none() {
                        binding.first_reference = Some(FootnoteReferenceLocation {
                            entity_id: visible_block.entity.entity_id(),
                            occurrence_index,
                        });
                    }
                    binding.ordinal
                } else {
                    None
                };
                block_occurrences
                    .entry(block_id)
                    .or_default()
                    .push(FootnoteResolvedOccurrence {
                        id: footnote.id.clone(),
                        ordinal,
                        occurrence_index,
                    });
                if ordinal.is_none() {
                    occurrence_index += 1;
                    continue;
                }
                occurrence_index += 1;
            }
        }

        self.footnote_registry = Arc::new(FootnoteRegistry {
            bindings,
            block_occurrences,
        });
    }

    pub(super) fn rebuild_image_runtimes(&mut self, cx: &mut Context<Self>) {
        let markdown = self.document.markdown_text(cx);
        self.rebuild_runtime_context_from_markdown(&markdown, cx);
    }

    pub(super) fn rebuild_runtime_context_from_markdown(
        &mut self,
        markdown: &str,
        cx: &mut Context<Self>,
    ) {
        self.image_reference_definitions = Arc::new(parse_image_reference_definitions(markdown));
        self.link_reference_definitions = Arc::new(parse_link_reference_definitions(markdown));
        self.rebuild_footnote_registry(cx);
        self.sync_mounted_runtime_contexts(cx);
    }

    pub(super) fn sync_mounted_runtime_contexts(&mut self, cx: &mut Context<Self>) {
        let base_dir = self.image_base_dir();
        let visible = self.document.visible_blocks().to_vec();
        let toc_blocks = Self::mounted_toc_blocks(&visible, cx);
        for visible_block in &visible {
            self.sync_runtime_context_for_block(&visible_block.entity, base_dir.as_deref(), cx);
            if visible_block.entity.read(cx).kind() != BlockKind::Table {
                continue;
            }
            let Some(runtime) = visible_block.entity.read(cx).table_runtime.clone() else {
                continue;
            };
            for cell in runtime.header {
                self.sync_runtime_context_for_block(&cell, base_dir.as_deref(), cx);
            }
            for row in runtime.rows {
                for cell in row {
                    self.sync_runtime_context_for_block(&cell, base_dir.as_deref(), cx);
                }
            }
        }
        Self::install_toc_entries(&visible, toc_blocks, cx);
    }

    /// Window reactivation is an explicit freshness boundary for local
    /// resource metadata. Only the runtime probe cache is invalidated; the
    /// Markdown source, block revision and undo history remain untouched.
    pub(super) fn refresh_resource_probes(&mut self, cx: &mut Context<Self>) {
        let visible = self.document.visible_blocks().to_vec();
        for visible_block in &visible {
            visible_block
                .entity
                .update(cx, |block, _cx| block.invalidate_resource_probe());
        }
        self.sync_mounted_runtime_contexts(cx);
    }

    fn mounted_toc_blocks(visible: &[super::tree::VisibleBlock], cx: &App) -> Vec<Entity<Block>> {
        visible
            .iter()
            .filter(|visible_block| {
                crate::components::is_toc_marker(visible_block.entity.read(cx).display_text())
            })
            .map(|visible_block| visible_block.entity.clone())
            .collect()
    }

    fn toc_entries_for_visible_blocks(
        visible: &[super::tree::VisibleBlock],
        cx: &App,
    ) -> Arc<[TocRuntimeEntry]> {
        Arc::from(
            visible
                .iter()
                .filter_map(|visible_block| {
                    let block = visible_block.entity.read(cx);
                    let BlockKind::Heading { level } = block.kind() else {
                        return None;
                    };
                    Some(TocRuntimeEntry {
                        level,
                        title: SharedString::from(block.record.title.visible_text().to_string()),
                        target: visible_block.entity.entity_id(),
                    })
                })
                .collect::<Vec<_>>(),
        )
    }

    fn install_toc_entries(
        visible: &[super::tree::VisibleBlock],
        toc_blocks: Vec<Entity<Block>>,
        cx: &mut Context<Self>,
    ) {
        if toc_blocks.is_empty() {
            return;
        }
        let toc_entries = Self::toc_entries_for_visible_blocks(visible, cx);
        for toc_block in toc_blocks {
            let toc_entries = toc_entries.clone();
            toc_block.update(cx, |block, cx| {
                block.toc_entries = toc_entries;
                cx.notify();
            });
        }
    }

    /// 标题输入只更新真正的 `[TOC]` 投影；没有目录时不能扫描标题或广播重绘。
    fn sync_mounted_toc_entries(&mut self, cx: &mut Context<Self>) {
        let visible = self.document.visible_blocks().to_vec();
        let toc_blocks = Self::mounted_toc_blocks(&visible, cx);
        Self::install_toc_entries(&visible, toc_blocks, cx);
    }

    /// 普通文本输入只同步当前块；定义或脚注变化才重建文档级注册表并广播。
    pub(super) fn sync_runtime_context_after_block_edit(
        &mut self,
        block: &Entity<Block>,
        cx: &mut Context<Self>,
    ) {
        let _trace = crate::perf::span("block_runtime_sync");
        let (block_id, kind, markdown) = block.read_with(cx, |block, _cx| {
            (block.record.id, block.kind(), block.record.title_markdown())
        });
        let changes_definition = kind == BlockKind::RawMarkdown
            || markdown.lines().any(|line| {
                let line = line.trim_start();
                line.starts_with('[') && line.contains("]:")
            });
        let changes_footnotes = kind == BlockKind::FootnoteDefinition
            || markdown.contains("[^")
            || self
                .footnote_registry
                .occurrences_for_block(block_id)
                .is_some();
        // 目录条目与标题实体是一次性的运行时映射；只要当前文档有 `[TOC]`
        // 或正在编辑标题，就需在同一输入事务后刷新它，不让点击导航指向过期块。
        let changes_toc = matches!(kind, BlockKind::Heading { .. })
            || crate::components::is_toc_marker(&markdown)
            || self.document.visible_blocks().iter().any(|visible| {
                crate::components::is_toc_marker(visible.entity.read(cx).display_text())
            });
        if changes_definition || changes_footnotes {
            if self.virtual_surface.is_some() {
                self.pending_virtual_global_runtime_refresh = true;
                let base_dir = self.image_base_dir();
                self.sync_runtime_context_for_block(block, base_dir.as_deref(), cx);
                return;
            }
            self.rebuild_image_runtimes(cx);
            return;
        }
        if changes_toc {
            let base_dir = self.image_base_dir();
            self.sync_runtime_context_for_block(block, base_dir.as_deref(), cx);
            self.sync_mounted_toc_entries(cx);
            return;
        }

        let base_dir = self.image_base_dir();
        self.sync_runtime_context_for_block(block, base_dir.as_deref(), cx);
    }

    pub(super) fn focusable_entity_by_id(&self, entity_id: EntityId) -> Option<Entity<Block>> {
        self.document
            .block_entity_by_id(entity_id)
            .or_else(|| {
                self.virtual_surface
                    .as_ref()
                    .and_then(|surface| surface.entity_by_id(entity_id))
            })
            .or_else(|| {
                self.table_cells
                    .get(&entity_id)
                    .map(|binding| binding.cell.clone())
            })
    }

    pub(super) fn first_focusable_entity_id(&self, cx: &App) -> Option<EntityId> {
        // Frontmatter 和注释在 Live 中是只读投影，初始焦点必须落到首个正文块。
        // 文档只有元数据时仍回退到第一块，保持窗口具有稳定的活动实体。
        let first_root = self
            .document
            .root_blocks()
            .iter()
            .find(|block| {
                let block = block.read(cx);
                block.kind() != BlockKind::Comment && !block.record.is_yaml_frontmatter()
            })
            .or_else(|| self.document.first_root())?
            .clone();
        if first_root.read(cx).kind() == BlockKind::Table {
            return first_root
                .read(cx)
                .table_runtime
                .as_ref()
                .and_then(|runtime| runtime.header.first())
                .map(|cell| cell.entity_id())
                .or_else(|| Some(first_root.entity_id()));
        }
        Some(first_root.entity_id())
    }

    pub(super) fn focused_edit_target_entity_id(
        &self,
        window: &Window,
        cx: &App,
    ) -> Option<EntityId> {
        self.document
            .focused_block_entity_id(window, cx)
            .or_else(|| {
                self.table_cells
                    .values()
                    .find(|binding| binding.cell.read(cx).focus_handle.is_focused(window))
                    .map(|binding| binding.cell.entity_id())
            })
    }

    pub(super) fn focused_edit_target(&self, window: &Window, cx: &App) -> Option<Entity<Block>> {
        self.focused_edit_target_entity_id(window, cx)
            .and_then(|entity_id| self.focusable_entity_by_id(entity_id))
    }

    pub(super) fn table_cell_binding(&self, entity_id: EntityId) -> Option<TableCellBinding> {
        self.table_cells.get(&entity_id).cloned()
    }

    pub(super) fn table_block_by_id(&self, entity_id: EntityId, cx: &App) -> Option<Entity<Block>> {
        self.document
            .block_entity_by_id(entity_id)
            .filter(|block| block.read(cx).kind() == BlockKind::Table)
    }
}

fn collect_runtime_surface_entities(
    block: &Entity<Block>,
    entities: &mut Vec<Entity<Block>>,
    cx: &App,
) {
    entities.push(block.clone());
    for child in &block.read(cx).children {
        collect_runtime_surface_entities(child, entities, cx);
    }
}
