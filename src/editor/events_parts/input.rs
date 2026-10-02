// @author kongweiguang

use super::*;

impl Editor {
    pub(super) fn sibling_drop_insert_index(
        source_index: usize,
        target_index: usize,
        placement: BlockDropPlacement,
    ) -> usize {
        match (placement, source_index < target_index) {
            (BlockDropPlacement::Before, true) => target_index.saturating_sub(1),
            (BlockDropPlacement::Before, false) => target_index,
            (BlockDropPlacement::After, true) => target_index,
            (BlockDropPlacement::After, false) => target_index.saturating_add(1),
        }
    }

    pub(super) fn menu_item_is_keyboard_selectable(item: &OwnedMenuItem) -> bool {
        match item {
            OwnedMenuItem::Action { action, .. } => !action.as_ref().as_any().is::<NoRecentFiles>(),
            OwnedMenuItem::Submenu(_) => true,
            OwnedMenuItem::Separator | OwnedMenuItem::SystemMenu(_) => false,
        }
    }

    pub(in crate::editor) fn adjacent_menu_item(
        items: &[OwnedMenuItem],
        current: Option<usize>,
        forward: bool,
    ) -> Option<usize> {
        if items.is_empty() {
            return None;
        }

        let start = current.unwrap_or(if forward { items.len() - 1 } else { 0 });
        (1..=items.len())
            .map(|step| {
                if forward {
                    (start + step) % items.len()
                } else {
                    (start + items.len() - (step % items.len())) % items.len()
                }
            })
            .find(|index| Self::menu_item_is_keyboard_selectable(&items[*index]))
    }

    pub(in crate::editor) fn edge_menu_item(items: &[OwnedMenuItem], first: bool) -> Option<usize> {
        let mut indices: Box<dyn Iterator<Item = usize>> = if first {
            Box::new(0..items.len())
        } else {
            Box::new((0..items.len()).rev())
        };
        indices.find(|index| Self::menu_item_is_keyboard_selectable(&items[*index]))
    }

    /// Handles the Windows/Linux in-window application menu without moving
    /// focus away from the editor. The logical cursor is separate from hover,
    /// so closing the menu restores the exact caret or workspace focus.
    pub(in crate::editor) fn handle_in_window_menu_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !supports_in_window_menu() {
            return false;
        }
        let Some(menus) = cx
            .try_global::<crate::app_menu::AppMenuState>()
            .map(|state| state.in_window_menus.clone())
            .or_else(|| cx.get_menus())
            .filter(|menus| !menus.is_empty())
            .map(|menus| self.contextual_menus(menus, cx))
        else {
            if self.menu_bar_open.is_some() {
                self.close_menu_bar(cx);
                return true;
            }
            return false;
        };
        self.handle_in_window_menu_key_with_menus(event, &menus, window, cx)
    }

    pub(in crate::editor) fn handle_in_window_menu_key_with_menus(
        &mut self,
        event: &KeyDownEvent,
        menus: &[OwnedMenu],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let key = event.keystroke.key.as_str();
        if self.menu_bar_open.is_none() {
            let modifiers = event.keystroke.modifiers;
            let unmodified_f10 = key.eq_ignore_ascii_case("f10")
                && !modifiers.control
                && !modifiers.platform
                && !modifiers.alt
                && !modifiers.shift;
            // Windows sends a standalone Alt as its own key. Do not claim AltGr or Alt-based
            // shortcuts: those retain a non-Alt key or Ctrl/Shift modifier and must reach text
            // input/the platform unchanged.
            let standalone_alt = key.eq_ignore_ascii_case("alt")
                && !modifiers.control
                && !modifiers.platform
                && !modifiers.shift;
            if !(unmodified_f10 || standalone_alt) {
                return false;
            }
            self.menu_bar_expanded = true;
            self.open_menu_bar(0, cx);
            self.menu_keyboard_item = Self::edge_menu_item(&menus[0].items, true);
            cx.notify();
            return true;
        }

        if !matches!(
            key,
            "up" | "down" | "left" | "right" | "enter" | "space" | "escape" | "home" | "end"
        ) {
            return false;
        }
        let open_menu = self.menu_bar_open.unwrap_or(0).min(menus.len() - 1);
        let main_items = &menus[open_menu].items;

        match key {
            "escape" => self.close_menu_bar(cx),
            "left" if self.menu_keyboard_submenu_item.is_some() => {
                self.menu_keyboard_submenu_item = None;
                self.close_menu_submenu(cx);
            }
            "left" => {
                let next_menu = (open_menu + menus.len() - 1) % menus.len();
                self.open_menu_bar(next_menu, cx);
                self.menu_keyboard_item = Self::edge_menu_item(&menus[next_menu].items, true);
                cx.notify();
            }
            "up" | "down" => {
                let forward = key == "down";
                if let (Some(submenu_index), Some(_)) =
                    (self.menu_submenu_open, self.menu_keyboard_submenu_item)
                    && let Some(OwnedMenuItem::Submenu(submenu)) = main_items.get(submenu_index)
                {
                    self.menu_keyboard_submenu_item = Self::adjacent_menu_item(
                        &submenu.items,
                        self.menu_keyboard_submenu_item,
                        forward,
                    );
                } else {
                    self.menu_keyboard_item =
                        Self::adjacent_menu_item(main_items, self.menu_keyboard_item, forward);
                    self.menu_keyboard_submenu_item = None;
                }
                cx.notify();
            }
            "home" | "end" => {
                let first = key == "home";
                if let Some(submenu_index) = self.menu_submenu_open
                    && self.menu_keyboard_submenu_item.is_some()
                    && let Some(OwnedMenuItem::Submenu(submenu)) = main_items.get(submenu_index)
                {
                    self.menu_keyboard_submenu_item = Self::edge_menu_item(&submenu.items, first);
                } else {
                    self.menu_keyboard_item = Self::edge_menu_item(main_items, first);
                }
                cx.notify();
            }
            "right" => {
                if let Some(item_index) = self.menu_keyboard_item
                    && let Some(OwnedMenuItem::Submenu(submenu)) = main_items.get(item_index)
                {
                    self.open_menu_submenu(item_index, cx);
                    self.menu_keyboard_submenu_item = Self::edge_menu_item(&submenu.items, true);
                    cx.notify();
                } else {
                    let next_menu = (open_menu + 1) % menus.len();
                    self.open_menu_bar(next_menu, cx);
                    self.menu_keyboard_item = Self::edge_menu_item(&menus[next_menu].items, true);
                    cx.notify();
                }
            }
            "enter" | "space" => {
                if let Some(submenu_index) = self.menu_submenu_open
                    && let Some(child_index) = self.menu_keyboard_submenu_item
                    && let Some(OwnedMenuItem::Submenu(submenu)) = main_items.get(submenu_index)
                    && let Some(OwnedMenuItem::Action { action, .. }) =
                        submenu.items.get(child_index)
                {
                    let action = action.boxed_clone();
                    let editor = cx.entity().downgrade();
                    self.close_menu_bar(cx);
                    dispatch_menu_action_for_editor(action.as_ref(), &editor, window, cx);
                } else if let Some(item_index) = self.menu_keyboard_item
                    && let Some(item) = main_items.get(item_index)
                {
                    match item {
                        OwnedMenuItem::Submenu(submenu) => {
                            self.open_menu_submenu(item_index, cx);
                            self.menu_keyboard_submenu_item =
                                Self::edge_menu_item(&submenu.items, true);
                            cx.notify();
                        }
                        OwnedMenuItem::Action { action, .. } => {
                            if action.as_ref().as_any().is::<NoRecentFiles>() {
                                return true;
                            }
                            let action = action.boxed_clone();
                            let editor = cx.entity().downgrade();
                            self.close_menu_bar(cx);
                            dispatch_menu_action_for_editor(action.as_ref(), &editor, window, cx);
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
        true
    }

    pub(super) fn focused_block_for_tab_key(
        &self,
        window: &mut Window,
        cx: &App,
    ) -> Option<Entity<super::Block>> {
        let is_focused = |block: &Entity<super::Block>| {
            let block = block.read(cx);
            block.focus_handle.is_focused(window)
                || block.code_language_focus_handle.is_focused(window)
        };

        if let Some(block) = self
            .active_entity_id
            .and_then(|entity_id| self.focusable_entity_by_id(entity_id))
            .filter(is_focused)
        {
            return Some(block);
        }

        for binding in self.table_cells.values() {
            if is_focused(&binding.cell) {
                return Some(binding.cell.clone());
            }
        }

        self.document
            .visible_blocks()
            .iter()
            .find_map(|visible| is_focused(&visible.entity).then(|| visible.entity.clone()))
    }

    /// Opens the selected resource card's menu at its last painted origin.
    /// The logical menu cursor is initialized independently from pointer hover,
    /// matching native Menu-key and Shift+F10 behavior.
    pub(in crate::editor) fn open_resource_context_menu_from_keyboard(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.context_menu.is_some() {
            return false;
        }
        let modifiers = event.keystroke.modifiers;
        let key = event.keystroke.key.as_str();
        let menu_key = matches!(key, "menu" | "contextmenu")
            && !modifiers.control
            && !modifiers.platform
            && !modifiers.alt;
        let shift_f10 = key.eq_ignore_ascii_case("f10")
            && modifiers.shift
            && !modifiers.control
            && !modifiers.platform
            && !modifiers.alt;
        if !(menu_key || shift_f10) {
            return false;
        }

        let Some(block) = self.focused_block_for_tab_key(window, cx) else {
            return false;
        };
        let (selected, has_resource, position) = block.read_with(cx, |block, _cx| {
            (
                block.resource_selected,
                block.record.resource.is_some(),
                block
                    .last_bounds
                    .map(|bounds| bounds.origin)
                    .unwrap_or_else(|| point(px(16.0), px(16.0))),
            )
        });
        if !selected || !has_resource {
            return false;
        }

        self.context_menu = Some(crate::editor::ContextMenuState::Resource {
            position,
            entity_id: block.entity_id(),
        });
        self.context_menu_keyboard_item = Some(0);
        self.context_menu_keyboard_submenu_item = None;
        cx.notify();
        true
    }

    /// Captures Tab before block-local actions so indentation waits for native IME completion.
    pub(crate) fn on_editor_key_down_capture(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.keystroke.key == "escape" && self.tabs.dismiss_new_or_split_menu() {
            cx.notify();
            cx.stop_propagation();
            return;
        }
        if self.handle_update_panel_key(event, window, cx)
            || self.handle_workspace_link_completion_key(event, cx)
            || self.handle_diagram_overlay_key(event, window, cx)
            || self.handle_table_cell_selection_key(event, cx)
            || self.handle_context_menu_key(event, window, cx)
            || self.open_resource_context_menu_from_keyboard(event, window, cx)
            || self.handle_in_window_menu_key(event, window, cx)
            || self.handle_find_panel_key(event, window, cx)
            || self.handle_command_palette_key(event, window, cx)
            || self.handle_quick_open_key(event, window, cx)
            || self.handle_document_sidebar_key(event, window, cx)
            || self.handle_workspace_key(event, window, cx)
        {
            cx.stop_propagation();
            return;
        }
        if let Some(target) = self.focused_block_for_tab_key(window, cx)
            && target.update(cx, |block, block_cx| {
                block.handle_code_language_menu_key(event, window, block_cx)
            })
        {
            cx.stop_propagation();
            return;
        }
        if let Some(target) = self.focused_block_for_tab_key(window, cx)
            && target.update(cx, |block, block_cx| {
                block.handle_selection_toolbar_key(event, window, block_cx)
            })
        {
            cx.stop_propagation();
            return;
        }
        if let Some(target) = self.focused_block_for_tab_key(window, cx)
            && target.update(cx, |block, block_cx| {
                block.handle_slash_menu_key(event, block_cx)
            })
        {
            cx.stop_propagation();
            return;
        }
        if event.keystroke.key != "tab" {
            return;
        }

        let modifiers = event.keystroke.modifiers;
        if modifiers.control || modifiers.platform || modifiers.alt || modifiers.function {
            return;
        }

        let Some(target) = self.focused_block_for_tab_key(window, cx) else {
            return;
        };

        let handles_tab = {
            let block = target.read(cx);
            if block.code_language_focus_handle.is_focused(window) {
                cx.stop_propagation();
                return;
            }
            block.is_table_cell()
                || block.kind().is_list_item()
                || block.kind() == BlockKind::Paragraph
                || block.kind().is_code_block()
        };

        if !handles_tab {
            return;
        }

        if modifiers.shift {
            self.on_outdent_block(&OutdentBlock, window, cx);
        } else {
            self.on_indent_block(&IndentBlock, window, cx);
        }
        cx.stop_propagation();
    }

    pub(super) fn build_plain_paste_blocks_from_lines(
        cx: &mut Context<Self>,
        lines: &[String],
    ) -> Vec<Entity<super::Block>> {
        let mut blocks = lines
            .iter()
            .filter(|line| !line.trim().is_empty())
            .map(|line| {
                Self::new_block(
                    cx,
                    BlockRecord::new(BlockKind::Paragraph, InlineTextTree::from_markdown(line)),
                )
            })
            .collect::<Vec<_>>();

        if blocks.is_empty() && !lines.is_empty() {
            blocks.push(Self::new_block(
                cx,
                BlockRecord::new(BlockKind::Paragraph, InlineTextTree::plain(String::new())),
            ));
        }

        blocks
    }

    pub(super) fn block_is_quote_structure_related(
        &self,
        block: &Entity<super::Block>,
        cx: &App,
    ) -> bool {
        if self.view_mode != super::ViewMode::Rendered {
            return false;
        }

        let block_ref = block.read(cx);
        block_ref.kind().is_quote_container()
            || block_ref.quote_depth > 0
            || block_ref.quote_group_anchor.is_some()
    }

    pub(super) fn refresh_rendered_quote_metadata_if_needed(
        &mut self,
        block: &Entity<super::Block>,
        cx: &mut Context<Self>,
    ) {
        if !self.block_is_quote_structure_related(block, cx) {
            return;
        }

        self.document.rebuild_metadata_and_snapshot(cx);
    }

    pub(super) fn rendered_quote_text_requires_reparse(
        block: &Entity<super::Block>,
        cx: &App,
    ) -> bool {
        let block_ref = block.read(cx);
        if block_ref.quote_depth == 0 && !block_ref.kind().is_quote_container() {
            return false;
        }

        let text = block_ref.display_text();
        if !text.contains('\n') {
            return false;
        }

        text.split('\n').skip(1).any(|line| {
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
        })
    }

    pub(super) fn block_event_clears_cross_block_selection(event: &BlockEvent) -> bool {
        matches!(
            event,
            BlockEvent::Changed
                | BlockEvent::RequestNewline { .. }
                | BlockEvent::RequestEnterCalloutBody
                | BlockEvent::RequestQuoteBreak
                | BlockEvent::RequestCalloutBreak
                | BlockEvent::RequestMergeIntoPrev { .. }
                | BlockEvent::RequestPasteMultiline { .. }
                | BlockEvent::RequestPasteImage { .. }
                | BlockEvent::RequestSlashCommand { .. }
                | BlockEvent::RequestMoveBlock { .. }
                | BlockEvent::RequestIndent
                | BlockEvent::RequestOutdent
                | BlockEvent::RequestDowngradeNestedListItemToChildParagraph
                | BlockEvent::ToggleTaskChecked
                | BlockEvent::RequestAppendTableColumn
                | BlockEvent::RequestAppendTableRow
                | BlockEvent::RequestDelete
        )
    }

    pub(crate) fn focus_block(&mut self, entity_id: EntityId) {
        self.pending_focus = Some(entity_id);
        self.active_entity_id = Some(entity_id);
        self.pending_scroll_active_block_into_view = true;
    }

    pub(super) fn reset_block_cursor(
        block: &Entity<super::Block>,
        cursor: usize,
        cx: &mut Context<Self>,
    ) {
        block.update(cx, move |block, cx| {
            block.selected_range = cursor..cursor;
            block.selection_reversed = false;
            block.marked_range = None;
            block.vertical_motion_x = None;
            block.cursor_blink_epoch = Instant::now();
            cx.notify();
        });
    }

    pub(in crate::editor) fn focus_block_range(
        &mut self,
        block: &Entity<super::Block>,
        range: std::ops::Range<usize>,
        cx: &mut Context<Self>,
    ) {
        block.update(cx, move |block, cx| {
            block.selected_range = range.clone();
            block.selection_reversed = false;
            block.marked_range = None;
            block.vertical_motion_x = None;
            block.cursor_blink_epoch = Instant::now();
            cx.notify();
        });
        self.focus_block(block.entity_id());
    }

    pub(super) fn current_image_paste_behavior() -> ImagePasteBehavior {
        read_app_preferences()
            .map(|preferences| preferences.image_paste_behavior)
            .unwrap_or(ImagePasteBehavior::None)
    }

    pub(super) fn image_paste_root_dir(&self) -> anyhow::Result<PathBuf> {
        if let Some(parent) = self.file_path.as_ref().and_then(|path| path.parent()) {
            return Ok(parent.to_path_buf());
        }
        std::env::current_dir().context("failed to resolve current working directory")
    }

    pub(super) fn clipboard_image_extension(format: ImageFormat) -> &'static str {
        match format {
            ImageFormat::Png => "png",
            ImageFormat::Jpeg => "jpg",
            ImageFormat::Webp => "webp",
            ImageFormat::Gif => "gif",
            ImageFormat::Svg => "svg",
            ImageFormat::Bmp => "bmp",
            ImageFormat::Tiff => "tiff",
        }
    }

    pub(super) fn image_target_dir(
        &self,
        behavior: ImagePasteBehavior,
        root_dir: &Path,
        source: &PastedImageSource,
    ) -> anyhow::Result<PathBuf> {
        match behavior {
            ImagePasteBehavior::None | ImagePasteBehavior::CopyToDocumentFolder => {
                Ok(root_dir.to_path_buf())
            }
            ImagePasteBehavior::CopyToAssetsFolder => Ok(root_dir.join("assets")),
            ImagePasteBehavior::CopyToNamedAssetsFolder => {
                let base = self
                    .file_path
                    .as_ref()
                    .and_then(|path| path.file_stem())
                    .and_then(|stem| stem.to_str())
                    .filter(|stem| !stem.trim().is_empty())
                    .unwrap_or("untitle");
                if self.file_path.is_some() {
                    return Ok(root_dir.join(format!("{base}.assets")));
                }

                for index in 0.. {
                    let folder = if index == 0 {
                        "untitle.assets".to_string()
                    } else {
                        format!("untitle{index}.assets")
                    };
                    let path = root_dir.join(folder);
                    if !path.exists() {
                        return Ok(path);
                    }
                    if matches!(source, PastedImageSource::LocalPath(_)) {
                        continue;
                    }
                }
                unreachable!("unbounded search should always return");
            }
        }
    }

    pub(super) fn unique_file_path(dir: &Path, preferred_name: &str) -> PathBuf {
        let preferred = Path::new(preferred_name);
        let stem = preferred
            .file_stem()
            .and_then(|stem| stem.to_str())
            .filter(|stem| !stem.is_empty())
            .unwrap_or("image");
        let extension = preferred.extension().and_then(|ext| ext.to_str());
        for index in 0.. {
            let file_name = if index == 0 {
                preferred_name.to_string()
            } else if let Some(extension) = extension {
                format!("{stem}{index}.{extension}")
            } else {
                format!("{stem}{index}")
            };
            let candidate = dir.join(file_name);
            if !candidate.exists() {
                return candidate;
            }
        }
        unreachable!("unbounded search should always return");
    }

    pub(super) fn path_parent_eq(left: &Path, right: &Path) -> bool {
        let Some(parent) = left.parent() else {
            return false;
        };
        let left = parent
            .canonicalize()
            .unwrap_or_else(|_| parent.to_path_buf());
        let right = right.canonicalize().unwrap_or_else(|_| right.to_path_buf());
        left == right
    }

    pub(super) fn materialize_pasted_image(
        &self,
        source: &PastedImageSource,
    ) -> anyhow::Result<(PathBuf, bool)> {
        let behavior = Self::current_image_paste_behavior();
        let root_dir = self.image_paste_root_dir()?;

        if matches!(behavior, ImagePasteBehavior::None)
            && let PastedImageSource::LocalPath(path) = source
        {
            return Ok((path.clone(), false));
        }

        let target_dir = self.image_target_dir(behavior, &root_dir, source)?;
        fs::create_dir_all(&target_dir)
            .with_context(|| format!("failed to create '{}'", target_dir.display()))?;

        match source {
            PastedImageSource::LocalPath(path) => {
                if Self::path_parent_eq(path, &target_dir) {
                    return Ok((path.clone(), behavior != ImagePasteBehavior::None));
                }
                let file_name = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("image");
                let target = Self::unique_file_path(&target_dir, file_name);
                fs::copy(path, &target).with_context(|| {
                    format!(
                        "failed to copy '{}' to '{}'",
                        path.display(),
                        target.display()
                    )
                })?;
                Ok((target, behavior != ImagePasteBehavior::None))
            }
            PastedImageSource::ClipboardImage(image) => {
                let file_name = format!(
                    "pasted-image.{}",
                    Self::clipboard_image_extension(image.format)
                );
                let target = Self::unique_file_path(&target_dir, &file_name);
                fs::write(&target, &image.bytes)
                    .with_context(|| format!("failed to write '{}'", target.display()))?;
                Ok((target, behavior != ImagePasteBehavior::None))
            }
            PastedImageSource::LocalResource(_) => Err(anyhow!(
                "ordinary resources must use the resource insertion path"
            )),
        }
    }

    pub(super) fn markdown_path_string(path: &Path) -> String {
        path.to_string_lossy().replace('\\', "/")
    }

    pub(super) fn markdown_image_target(path: &str) -> String {
        path.chars()
            .flat_map(|ch| match ch {
                '\\' | '(' | ')' | '"' => ['\\', ch].into_iter().collect::<Vec<_>>(),
                _ => [ch].into_iter().collect::<Vec<_>>(),
            })
            .collect()
    }

    pub(super) fn markdown_image_alt(path: &Path) -> String {
        let alt = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .filter(|stem| !stem.is_empty())
            .unwrap_or("image");
        alt.chars()
            .flat_map(|ch| match ch {
                '\\' | ']' => ['\\', ch].into_iter().collect::<Vec<_>>(),
                _ => [ch].into_iter().collect::<Vec<_>>(),
            })
            .collect()
    }

    pub(super) fn relative_markdown_path(root_dir: &Path, path: &Path) -> Option<String> {
        let relative = path.strip_prefix(root_dir).ok()?;
        Some(format!("./{}", Self::markdown_path_string(relative)))
    }

    pub(super) fn pasted_image_markdown(
        &self,
        source: &PastedImageSource,
    ) -> anyhow::Result<String> {
        let root_dir = self.image_paste_root_dir()?;
        let (path, relative) = self.materialize_pasted_image(source)?;
        let path_text = if relative {
            Self::relative_markdown_path(&root_dir, &path)
                .ok_or_else(|| anyhow!("failed to create a relative image path"))?
        } else {
            Self::markdown_path_string(&path)
        };
        Ok(format!(
            "![{}]({})",
            Self::markdown_image_alt(&path),
            Self::markdown_image_target(&path_text)
        ))
    }

    pub(super) fn pasted_resource_markdown(
        &self,
        source: &Path,
    ) -> anyhow::Result<(String, crate::resource_io::MaterializedResource)> {
        let behavior = read_app_preferences()
            .map(|preferences| preferences.resource_insert_behavior())
            .unwrap_or(ImagePasteBehavior::None);
        crate::resource_io::resource_markdown_for_path(
            "",
            source,
            self.file_path.as_deref(),
            behavior,
            None,
        )
    }

    pub(in crate::editor) fn show_image_paste_error(
        &self,
        err: anyhow::Error,
        cx: &mut Context<Self>,
    ) {
        let strings = cx.global::<crate::i18n::I18nManager>().strings().clone();
        if let Some(window) = cx.active_window() {
            let ok = strings.info_dialog_ok.clone();
            let title = strings.image_paste_failed_title.clone();
            let detail = err.to_string();
            let _ = window.update(cx, |_view, window, cx| {
                let buttons = [ok.as_str()];
                let _ = window.prompt(PromptLevel::Critical, &title, Some(&detail), &buttons, cx);
            });
        } else {
            eprintln!("{}: {err}", strings.image_paste_failed_title);
        }
    }
}
