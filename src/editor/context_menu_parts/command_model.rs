// @author kongweiguang

use super::*;

impl Editor {
    /// Rebuilds current enablement from the owning pane so stale or read-only targets cannot inherit edit commands.
    pub(super) fn context_menu_command_model(&self, cx: &App) -> ContextMenuCommandModel {
        let entry = |command, enabled| ContextMenuCommandEntry { command, enabled };
        if let Some((_index, _pinned, can_close_others)) = self.tab_context_menu_info() {
            return ContextMenuCommandModel {
                main: vec![
                    entry(ContextMenuCommand::TabTogglePin, true),
                    entry(ContextMenuCommand::TabClose, true),
                    entry(ContextMenuCommand::TabCloseOthers, can_close_others),
                ],
                submenu: Vec::new(),
            };
        }

        let Some(menu) = self.context_menu.as_ref() else {
            return ContextMenuCommandModel::default();
        };
        match menu {
            ContextMenuState::Insert { .. } => ContextMenuCommandModel {
                main: vec![entry(ContextMenuCommand::OpenInsertSubmenu, true)],
                submenu: INSERT_COMMANDS
                    .into_iter()
                    .map(|command| entry(ContextMenuCommand::Insert(command), true))
                    .collect(),
            },
            ContextMenuState::Spelling { diagnostic, .. } => ContextMenuCommandModel {
                main: diagnostic
                    .replacements
                    .iter()
                    .enumerate()
                    .map(|(index, _)| entry(ContextMenuCommand::SpellingSuggestion(index), true))
                    .collect(),
                submenu: Vec::new(),
            },
            ContextMenuState::Resource { entity_id, .. } => {
                let Some(_block) = self.focusable_entity_by_id(*entity_id) else {
                    return ContextMenuCommandModel::default();
                };
                let Some(resource) = self.resource_context_record(cx) else {
                    return ContextMenuCommandModel::default();
                };
                let local = resource.local_path().is_some();
                let missing = matches!(
                    self.resource_context_status(cx),
                    Some(ResourceStatus::Missing)
                );
                let editable = self.resource_target_is_editable(*entity_id);
                ContextMenuCommandModel {
                    main: vec![
                        entry(ContextMenuCommand::ResourceOpen, !resource.is_unsafe_url()),
                        entry(ContextMenuCommand::ResourceReveal, local),
                        entry(ContextMenuCommand::ResourceEditTitle, editable),
                        entry(ContextMenuCommand::ResourceReplace, editable),
                        entry(ContextMenuCommand::ResourceCopyAddress, true),
                        entry(ContextMenuCommand::ResourceConvertLink, editable),
                        entry(ContextMenuCommand::ResourceDelete, editable),
                        entry(ContextMenuCommand::ResourceRelocate, missing && editable),
                    ],
                    submenu: Vec::new(),
                }
            }
            ContextMenuState::TableAxis { selection, .. } => {
                let Some(table) = self
                    .table_block_by_id(selection.table_block_id, cx)
                    .and_then(|block| block.read(cx).record.table.clone())
                else {
                    return ContextMenuCommandModel::default();
                };
                let main = match selection.kind {
                    TableAxisKind::Column => vec![
                        entry(ContextMenuCommand::InsertColumnBefore, true),
                        entry(ContextMenuCommand::InsertColumnAfter, true),
                        entry(ContextMenuCommand::DuplicateColumn, true),
                        entry(ContextMenuCommand::AlignColumnLeft, true),
                        entry(ContextMenuCommand::AlignColumnCenter, true),
                        entry(ContextMenuCommand::AlignColumnRight, true),
                        entry(ContextMenuCommand::MoveColumnLeft, selection.index > 0),
                        entry(
                            ContextMenuCommand::MoveColumnRight,
                            selection.index + 1 < table.column_count(),
                        ),
                        entry(ContextMenuCommand::DeleteColumn, table.column_count() > 1),
                        entry(ContextMenuCommand::DeleteTable, true),
                    ],
                    TableAxisKind::Row if selection.index == 0 => vec![
                        entry(ContextMenuCommand::InsertRowBefore, true),
                        entry(ContextMenuCommand::InsertRowAfter, true),
                        entry(ContextMenuCommand::DuplicateRow, true),
                        entry(ContextMenuCommand::ToggleTableHeaders, true),
                        entry(ContextMenuCommand::MoveRowUp, false),
                        entry(
                            ContextMenuCommand::MoveRowDown,
                            selection.index < table.rows.len(),
                        ),
                        entry(ContextMenuCommand::DeleteRow, !table.rows.is_empty()),
                        entry(ContextMenuCommand::DeleteTable, true),
                    ],
                    TableAxisKind::Row => vec![
                        entry(ContextMenuCommand::InsertRowBefore, true),
                        entry(ContextMenuCommand::InsertRowAfter, true),
                        entry(ContextMenuCommand::DuplicateRow, true),
                        entry(ContextMenuCommand::MoveRowUp, selection.index > 0),
                        entry(
                            ContextMenuCommand::MoveRowDown,
                            selection.index < table.rows.len(),
                        ),
                        entry(ContextMenuCommand::DeleteRow, true),
                        entry(ContextMenuCommand::DeleteTable, true),
                    ],
                };
                ContextMenuCommandModel {
                    main,
                    submenu: Vec::new(),
                }
            }
            ContextMenuState::Workspace { .. } => ContextMenuCommandModel {
                main: vec![
                    entry(
                        ContextMenuCommand::WorkspaceOpen,
                        self.workspace_context_target_is_file(),
                    ),
                    entry(ContextMenuCommand::WorkspaceReveal, true),
                    entry(ContextMenuCommand::WorkspaceCopyPath, true),
                    entry(
                        ContextMenuCommand::WorkspaceCopyRelativePath,
                        !self.workspace_context_target_is_root(),
                    ),
                    entry(ContextMenuCommand::WorkspaceNewFile, true),
                    entry(ContextMenuCommand::WorkspaceNewFolder, true),
                    entry(
                        ContextMenuCommand::WorkspaceRename,
                        !self.workspace_context_target_is_root(),
                    ),
                    entry(
                        ContextMenuCommand::WorkspaceMove,
                        !self.workspace_context_target_is_root(),
                    ),
                    entry(ContextMenuCommand::WorkspaceRefresh, true),
                    entry(
                        ContextMenuCommand::WorkspaceUndo,
                        self.workspace_can_undo_file_operation(),
                    ),
                    entry(
                        ContextMenuCommand::WorkspaceDelete,
                        !self.workspace_context_target_is_root(),
                    ),
                ],
                submenu: Vec::new(),
            },
            ContextMenuState::Text {
                position,
                surface,
                entity_id,
                ..
            } => {
                if !self.text_context_target_is_owned_by_surface(*surface, *entity_id) {
                    return ContextMenuCommandModel::default();
                }
                let Some(block) = self.text_context_block_for_surface(*surface, *entity_id) else {
                    return ContextMenuCommandModel::default();
                };
                let editable = self.document_surface_is_editable_for(*surface);
                let selected = !block.read(cx).selected_range.is_empty()
                    || block
                        .read(cx)
                        .editor_selection_range
                        .as_ref()
                        .is_some_and(|range| !range.is_empty())
                    || self.cross_block_selection_for_surface(*surface).is_some()
                    || self.table_cell_rectangle_for_surface(*surface).is_some();
                let mut main = Vec::with_capacity(11);
                if block.read(cx).pointer_link_hit(*position).is_some() {
                    main.push(entry(ContextMenuCommand::TextOpenLink, true));
                }
                main.extend([
                    entry(ContextMenuCommand::TextCopy, selected),
                    entry(ContextMenuCommand::TextCopyAsMarkdown, selected),
                    entry(ContextMenuCommand::TextCut, selected && editable),
                    entry(ContextMenuCommand::TextPaste, editable),
                    entry(ContextMenuCommand::TextSelectAll, true),
                    entry(ContextMenuCommand::TextDuplicateLine, editable),
                    entry(ContextMenuCommand::TextDeleteLine, editable),
                    entry(ContextMenuCommand::TextMoveLineUp, editable),
                    entry(ContextMenuCommand::TextMoveLineDown, editable),
                ]);
                if editable && self.view_mode == ViewMode::Rendered {
                    main.push(entry(ContextMenuCommand::TextInsert, true));
                }
                ContextMenuCommandModel {
                    main,
                    submenu: Vec::new(),
                }
            }
        }
    }
}
