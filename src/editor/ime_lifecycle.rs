// @author kongweiguang

//! 用户动作等待原输入目标的系统终态；请求完成与实际提交保持两个边界。

use super::*;

/// Stores pointer hits in the pre-commit text coordinate space so terminal input never re-hits an old layout.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct DeferredSurfacePointerEndpoint {
    pub(super) entity_id: EntityId,
    pub(super) clean_offset: usize,
    pub(super) revision: Revision,
}

impl DeferredSurfacePointerEndpoint {
    /// Keeps queued pointer hits stable across layout changes by storing clean-text coordinates.
    pub(in crate::editor) fn new(
        entity_id: EntityId,
        clean_offset: usize,
        revision: Revision,
    ) -> Self {
        Self {
            entity_id,
            clean_offset,
            revision,
        }
    }
}

/// Preserves one cross-block gesture's order while allowing only adjacent movement updates to coalesce.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DeferredSurfacePointerInteraction {
    Begin {
        anchor: DeferredSurfacePointerEndpoint,
        target_entity_id: EntityId,
        extend: bool,
    },
    Move {
        focus: DeferredSurfacePointerEndpoint,
    },
    End,
}

/// 每个请求携带产生它的文档身份；标签重排不改变延迟操作的目标。
pub(in crate::editor) enum DeferredImeOperation {
    ToolFocus {
        tab: Option<uuid::Uuid>,
        intent: super::tool_ime::ToolImeIntent,
    },
    Action {
        tab: Option<uuid::Uuid>,
        action: Box<dyn Action>,
    },
    InputInteraction {
        tab: Option<uuid::Uuid>,
        surface: selection_surface::SelectionSurface,
        block: Entity<Block>,
        interaction: crate::components::block::BlockImeInteraction,
    },
    SurfacePointerSelection {
        tab: Option<uuid::Uuid>,
        surface: selection_surface::SelectionSurface,
        owner: Entity<Block>,
        interaction: DeferredSurfacePointerInteraction,
    },
    Mode {
        tab: Option<uuid::Uuid>,
        mode: ViewMode,
    },
    SwitchTab(uuid::Uuid),
    Pane(panes::PaneEvent),
    NewTab(Box<tabs::DocumentTabSnapshot>),
    CloseTab(uuid::Uuid),
}

impl Editor {
    /// 组合输入属于视图，包括表格字段与活动窗格；共享 Controller 只知道已提交正文。
    pub(in crate::editor) fn has_active_ime_composition(&self, cx: &App) -> bool {
        if self
            .ime_detached_targets
            .iter()
            .any(|block| block.read(cx).has_ime_composition())
            || self
                .document
                .flatten_visible_blocks()
                .iter()
                .any(|block| block.entity.read(cx).has_ime_composition())
            || self
                .table_cells
                .values()
                .any(|binding| binding.cell.read(cx).has_ime_composition())
            || self.find_panel.as_ref().is_some_and(|panel| {
                panel.query.read(cx).has_ime_composition()
                    || panel.replacement.read(cx).has_ime_composition()
            })
            || self
                .command_palette
                .as_ref()
                .is_some_and(|palette| palette.input.read(cx).has_ime_composition())
            || self
                .resource_title_dialog
                .as_ref()
                .is_some_and(|dialog| dialog.input.read(cx).has_ime_composition())
        {
            return true;
        }
        if let Some(host) = self.document_host.as_ref()
            && host.read(cx).has_active_ime_composition(cx)
        {
            return true;
        }
        if !self.pane_canvas {
            let (editor, host) = self.focused_pane_entities(cx);
            return editor.is_some_and(|editor| editor.read(cx).has_active_ime_composition(cx))
                || host.is_some_and(|host| host.read(cx).has_active_ime_composition(cx));
        }
        false
    }

    /// IMM 的完成请求可能异步返回候选；失败保留会话，避免保存拼音或丢掉原选区。
    pub(in crate::editor) fn wait_for_ime_completion(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.has_active_ime_composition(cx) {
            self.ime_completion_requested = false;
            self.ime_completion_failed = false;
            return false;
        }
        if self.ime_completion_failed {
            return true;
        }
        if !self.ime_completion_requested {
            self.ime_completion_requested = true;
            match window.finish_ime_composition() {
                Ok(true) => {}
                Ok(false) | Err(_) => {
                    self.ime_completion_requested = false;
                    self.ime_completion_failed = true;
                    self.show_pane_notice("输入法尚未完成，请确认或取消候选后重试操作", cx);
                }
            }
        }
        true
    }

    /// 显式意图允许重试失败请求；渲染等待终态，不按帧反复请求系统完成。
    pub(in crate::editor) fn queue_ime_operation(
        &mut self,
        mut operation: DeferredImeOperation,
        cx: &mut Context<Self>,
    ) {
        if !self.coalesce_pending_pointer_move(&mut operation) {
            self.pending_ime_operations.push_back(operation);
        }
        if self.ime_completion_failed {
            self.ime_completion_requested = false;
            self.ime_completion_failed = false;
        }
        cx.notify();
    }

    /// Keeps the queue bounded during a stalled finish request without moving pointer updates across semantic barriers.
    fn coalesce_pending_pointer_move(&mut self, incoming: &mut DeferredImeOperation) -> bool {
        match incoming {
            DeferredImeOperation::SurfacePointerSelection {
                tab,
                surface,
                owner,
                interaction: DeferredSurfacePointerInteraction::Move { focus },
            } => {
                for pending in self.pending_ime_operations.iter_mut().rev() {
                    match pending {
                        DeferredImeOperation::SurfacePointerSelection {
                            tab: pending_tab,
                            surface: pending_surface,
                            owner: pending_owner,
                            interaction:
                                DeferredSurfacePointerInteraction::Move {
                                    focus: pending_focus,
                                },
                        } if pending_tab == tab
                            && pending_surface == surface
                            && pending_owner.entity_id() == owner.entity_id() =>
                        {
                            *pending_focus = *focus;
                            return true;
                        }
                        DeferredImeOperation::InputInteraction {
                            tab: pending_tab,
                            surface: pending_surface,
                            interaction:
                                crate::components::block::BlockImeInteraction::PointerSelectionMove {
                                    ..
                                },
                            ..
                        } if pending_tab == tab && pending_surface == surface => {}
                        DeferredImeOperation::SurfacePointerSelection {
                            tab: pending_tab,
                            surface: pending_surface,
                            interaction: DeferredSurfacePointerInteraction::Move { .. },
                            ..
                        } if pending_tab == tab && pending_surface == surface => {}
                        _ => break,
                    }
                }
                false
            }
            DeferredImeOperation::InputInteraction {
                tab,
                surface,
                block,
                interaction:
                    crate::components::block::BlockImeInteraction::PointerSelectionMove { clean_offset },
            } => {
                for pending in self.pending_ime_operations.iter_mut().rev() {
                    match pending {
                        DeferredImeOperation::InputInteraction {
                            tab: pending_tab,
                            surface: pending_surface,
                            block: pending_block,
                            interaction:
                                crate::components::block::BlockImeInteraction::PointerSelectionMove {
                                    clean_offset: pending_offset,
                                },
                        } if pending_tab == tab
                            && pending_surface == surface
                            && pending_block.entity_id() == block.entity_id() =>
                        {
                            *pending_offset = *clean_offset;
                            return true;
                        }
                        DeferredImeOperation::InputInteraction {
                            tab: pending_tab,
                            surface: pending_surface,
                            interaction:
                                crate::components::block::BlockImeInteraction::PointerSelectionMove {
                                    ..
                                },
                            ..
                        } if pending_tab == tab && pending_surface == surface => {}
                        DeferredImeOperation::SurfacePointerSelection {
                            tab: pending_tab,
                            surface: pending_surface,
                            interaction: DeferredSurfacePointerInteraction::Move { .. },
                            ..
                        } if pending_tab == tab && pending_surface == surface => {}
                        _ => break,
                    }
                }
                false
            }
            _ => false,
        }
    }

    /// Finds a still queued gesture so later mouse events join its original composition owner and FIFO position.
    pub(super) fn pending_surface_pointer_owner(
        &self,
        surface: selection_surface::SelectionSurface,
    ) -> Option<(Option<uuid::Uuid>, Entity<Block>)> {
        let active_tab = self
            .tabs
            .records
            .get(self.tabs.active)
            .map(|record| record.id);
        for pending in self.pending_ime_operations.iter().rev() {
            let DeferredImeOperation::SurfacePointerSelection {
                tab,
                surface: pending_surface,
                owner,
                interaction,
            } = pending
            else {
                continue;
            };
            if *pending_surface != surface || *tab != active_tab {
                continue;
            }
            return match interaction {
                DeferredSurfacePointerInteraction::Begin { .. }
                | DeferredSurfacePointerInteraction::Move { .. } => Some((*tab, owner.clone())),
                DeferredSurfacePointerInteraction::End => None,
            };
        }
        None
    }

    /// Distinguishes an already queued release from no gesture so repeated cancel callbacks do not clear a queued down.
    pub(super) fn surface_pointer_selection_end_is_queued(
        &self,
        surface: selection_surface::SelectionSurface,
    ) -> bool {
        let active_tab = self
            .tabs
            .records
            .get(self.tabs.active)
            .map(|record| record.id);
        self.pending_ime_operations
            .iter()
            .rev()
            .find_map(|pending| {
                let DeferredImeOperation::SurfacePointerSelection {
                    tab,
                    surface: pending_surface,
                    interaction,
                    ..
                } = pending
                else {
                    return None;
                };
                (*pending_surface == surface && *tab == active_tab).then_some(matches!(
                    interaction,
                    DeferredSurfacePointerInteraction::End
                ))
            })
            .unwrap_or(false)
    }

    /// 连续历史操作保持顺序；失败后的同一动作重试不会制造第二次副作用。
    pub(in crate::editor) fn queue_document_action_for_ime<A: Action>(
        &mut self,
        action: &A,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.has_active_ime_composition(cx) {
            return false;
        }
        let tab = self.tabs.records.get(self.tabs.active).map(|tab| tab.id);
        let is_retry = self.ime_completion_failed
            && self.pending_ime_operations.back().is_some_and(|pending| {
                matches!(pending, DeferredImeOperation::Action { tab: previous, action: queued }
                    if *previous == tab && queued.as_any().type_id() == std::any::TypeId::of::<A>())
            });
        if is_retry {
            self.ime_completion_failed = false;
            self.ime_completion_requested = false;
            cx.notify();
        } else {
            self.queue_ime_operation(
                DeferredImeOperation::Action {
                    tab,
                    action: action.boxed_clone(),
                },
                cx,
            );
        }
        true
    }

    /// 完成请求交给原输入目标，不能用模拟 Enter 把拼音当成确认结果。
    pub(in crate::editor) fn defer_action_for_ime<A: Action>(
        &mut self,
        action: &A,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.queue_document_action_for_ime(action, cx) {
            return false;
        }
        self.wait_for_ime_completion(window, cx)
    }

    /// 操作按用户顺序回放；保存快照与异步保存完成都早于随后标签拆卸。
    pub(super) fn sync_pending_ime_operations(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.ime_detached_targets
            .retain(|block| block.read(cx).has_ime_composition());
        if self.pending_ime_operations.is_empty() || self.wait_for_ime_completion(window, cx) {
            return;
        }
        while let Some(next) = self.pending_ime_operations.front() {
            // 保存使用不可变快照，原标签仍可编辑；只将拆卸目标的导航留到写入结束。
            let requires_navigation = match next {
                DeferredImeOperation::Action { tab, .. }
                | DeferredImeOperation::ToolFocus { tab, .. }
                | DeferredImeOperation::InputInteraction { tab, .. }
                | DeferredImeOperation::SurfacePointerSelection { tab, .. } => {
                    *tab != self
                        .tabs
                        .records
                        .get(self.tabs.active)
                        .map(|record| record.id)
                }
                _ => true,
            };
            if requires_navigation && !self.can_switch_tabs() {
                break;
            }
            let Some(operation) = self.pending_ime_operations.pop_front() else {
                break;
            };
            let mut yield_after_input = false;
            match operation {
                DeferredImeOperation::ToolFocus { tab, intent } => {
                    if self.activate_ime_operation_tab(tab, cx) {
                        self.replay_tool_ime_intent(intent, window, cx);
                    }
                }
                DeferredImeOperation::Action { tab, action } => {
                    if self.activate_ime_operation_tab(tab, cx) {
                        self.replay_ime_action(action, window, cx);
                    }
                }
                DeferredImeOperation::InputInteraction {
                    tab,
                    surface,
                    block,
                    interaction,
                } => {
                    if self.activate_ime_operation_tab(tab, cx)
                        && self.selection_surface_for_block_id(block.entity_id()) == Some(surface)
                    {
                        self.active_selection_surface = surface;
                        if surface == selection_surface::SelectionSurface::Main
                            && matches!(
                                interaction,
                                crate::components::block::BlockImeInteraction::PointerSelection { .. }
                            )
                        {
                            self.handle_block_focus_request(&block, cx);
                        }
                        if let crate::components::block::BlockImeInteraction::InputCommand {
                            command,
                            selection,
                            reversed,
                            base_revision,
                        } = interaction
                        {
                            let restored = block.update(cx, |block, _cx| {
                                block.restore_ime_command_selection(
                                    selection,
                                    reversed,
                                    base_revision,
                                )
                            });
                            if restored {
                                if surface == selection_surface::SelectionSurface::Main {
                                    self.handle_block_focus_request(&block, cx);
                                }
                                self.replay_document_input_command(
                                    surface, &block, command, window, cx,
                                );
                                self.continue_ime_commands_after_document_events(
                                    tab,
                                    block.clone(),
                                    window,
                                    cx,
                                );
                                yield_after_input = true;
                            }
                        } else {
                            let is_inline_command = matches!(
                                interaction,
                                crate::components::block::BlockImeInteraction::InlineCommand { .. }
                            );
                            block.update(cx, |block, cx| {
                                block.apply_ime_interaction(interaction, window, cx);
                            });
                            if is_inline_command {
                                self.continue_ime_commands_after_document_events(
                                    tab,
                                    block.clone(),
                                    window,
                                    cx,
                                );
                                yield_after_input = true;
                            }
                        }
                    }
                }
                DeferredImeOperation::SurfacePointerSelection {
                    tab,
                    surface,
                    owner,
                    interaction,
                } => {
                    let tab_active = self.activate_ime_operation_tab(tab, cx);
                    if let DeferredSurfacePointerInteraction::End = interaction {
                        self.finish_deferred_surface_pointer_selection(
                            surface, &owner, tab_active, cx,
                        );
                    } else if tab_active {
                        self.replay_deferred_surface_pointer_selection(
                            surface,
                            &owner,
                            interaction,
                            cx,
                        );
                    }
                }
                DeferredImeOperation::Mode { tab, mode } => {
                    if self.activate_ime_operation_tab(tab, cx) {
                        self.set_view_mode(mode, cx);
                    }
                }
                DeferredImeOperation::SwitchTab(tab) => {
                    self.activate_ime_operation_tab(Some(tab), cx);
                }
                DeferredImeOperation::Pane(event) => {
                    self.handle_pane_event(event, Some(window), cx)
                }
                DeferredImeOperation::NewTab(snapshot) => {
                    self.new_tab_from_snapshot(*snapshot, cx);
                }
                DeferredImeOperation::CloseTab(tab) => {
                    if let Some(index) =
                        self.tabs.records.iter().position(|record| record.id == tab)
                    {
                        self.request_close_tab_index(index, cx);
                    }
                }
            }
            if yield_after_input || self.has_active_ime_composition(cx) {
                break;
            }
        }
    }

    /// 让 Changed 与 UndoCapture 先发布，再执行下一命令，避免一轮 FIFO 合并多个正文事务。
    fn continue_ime_commands_after_document_events(
        &mut self,
        tab: Option<uuid::Uuid>,
        block: Entity<Block>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let editor = cx.entity().downgrade();
        window.defer(cx, move |window, cx| {
            let _ = editor.update(cx, |editor, cx| {
                if editor
                    .tabs
                    .records
                    .get(editor.tabs.active)
                    .map(|record| record.id)
                    == tab
                {
                    editor.refresh_following_ime_command_snapshots(tab, &block, cx);
                    editor.sync_pending_ime_operations(window, cx);
                }
            });
        });
    }

    /// 只刷新紧接同一 owner 的命令，不能跨过鼠标、标签或工具焦点意图改写其顺序。
    fn refresh_following_ime_command_snapshots(
        &mut self,
        tab: Option<uuid::Uuid>,
        block: &Entity<Block>,
        cx: &App,
    ) {
        for pending in &mut self.pending_ime_operations {
            let DeferredImeOperation::InputInteraction {
                tab: pending_tab,
                block: pending_block,
                interaction,
                ..
            } = pending
            else {
                break;
            };
            if *pending_tab != tab || pending_block != block {
                break;
            }
            if !matches!(
                interaction,
                crate::components::block::BlockImeInteraction::InputCommand { .. }
                    | crate::components::block::BlockImeInteraction::InlineCommand { .. }
            ) {
                break;
            }
            block.read(cx).refresh_ime_command_snapshot(interaction);
        }
    }

    /// Replays only endpoints whose source revision is unchanged or mapped by the owner's single local IME replacement.
    fn replay_deferred_surface_pointer_selection(
        &mut self,
        surface: selection_surface::SelectionSurface,
        owner: &Entity<Block>,
        interaction: DeferredSurfacePointerInteraction,
        cx: &mut Context<Self>,
    ) {
        match interaction {
            DeferredSurfacePointerInteraction::Begin {
                anchor,
                target_entity_id,
                extend,
            } => {
                let Some(anchor) =
                    self.rebase_deferred_surface_pointer_endpoint(surface, owner, anchor, cx)
                else {
                    return;
                };
                let Some(target) = self
                    .selection_surface_entities(surface)
                    .into_iter()
                    .find(|entity| entity.entity_id() == target_entity_id)
                else {
                    return;
                };
                self.active_selection_surface = surface;
                self.handle_block_focus_request(&target, cx);
                if !extend {
                    self.clear_cross_block_selection_for_surface(surface, cx);
                }
                let drag = self.cross_block_drag_from_endpoint(surface, anchor, cx);
                self.set_cross_block_drag_for_surface(surface, Some(drag));
                cx.notify();
            }
            DeferredSurfacePointerInteraction::Move { focus } => {
                let Some(focus) =
                    self.rebase_deferred_surface_pointer_endpoint(surface, owner, focus, cx)
                else {
                    return;
                };
                self.active_selection_surface = surface;
                self.apply_surface_pointer_selection_to_endpoint(surface, focus, cx);
            }
            DeferredSurfacePointerInteraction::End => {
                self.finish_deferred_surface_pointer_selection(surface, owner, true, cx);
            }
        }
    }

    /// Rebases a captured clean offset without allowing unrelated peer edits to redirect the gesture.
    fn rebase_deferred_surface_pointer_endpoint(
        &self,
        surface: selection_surface::SelectionSurface,
        owner: &Entity<Block>,
        captured: DeferredSurfacePointerEndpoint,
        cx: &App,
    ) -> Option<CrossBlockSelectionEndpoint> {
        let current_revision = self.source_document.revision();
        if captured.revision != current_revision {
            let exactly_one_local_commit = captured
                .revision
                .get()
                .checked_add(1)
                .is_some_and(|revision| revision == current_revision.get())
                && owner.read(cx).has_local_pointer_interaction_rebase();
            if !exactly_one_local_commit {
                return None;
            }
        }

        let block = self
            .selection_surface_entities(surface)
            .into_iter()
            .find(|block| block.entity_id() == captured.entity_id)?;
        let clean_offset =
            if captured.revision != current_revision && captured.entity_id == owner.entity_id() {
                owner
                    .read(cx)
                    .rebase_pointer_interaction_offset(captured.clean_offset)?
            } else {
                captured.clean_offset
            };
        let offset = block
            .read(cx)
            .clean_to_current_range(clean_offset..clean_offset)
            .start;
        Some(CrossBlockSelectionEndpoint {
            entity_id: captured.entity_id,
            offset,
        })
    }

    /// Releases the owner-local rebase map only after the queued gesture's final endpoint has been considered.
    fn finish_deferred_surface_pointer_selection(
        &mut self,
        surface: selection_surface::SelectionSurface,
        owner: &Entity<Block>,
        clear_active_surface: bool,
        cx: &mut Context<Self>,
    ) {
        if clear_active_surface {
            self.set_cross_block_drag_for_surface(surface, None);
            self.cancel_selection_autoscroll();
            self.end_block_pointer_selection_sessions(cx);
        }
        owner.update(cx, |block, _cx| {
            block.set_ime_surface_selection_pending(false);
            block.clear_ime_interaction_rebase_if_idle();
        });
        cx.notify();
    }

    /// 已关闭的原目标不能替换成同位置的新标签；存活目标按身份激活后再走普通入口。
    fn activate_ime_operation_tab(
        &mut self,
        tab: Option<uuid::Uuid>,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(tab) = tab else {
            return true;
        };
        let Some(index) = self.tabs.records.iter().position(|record| record.id == tab) else {
            self.show_pane_notice("原文档已关闭，请在目标文档重试操作", cx);
            return false;
        };
        index == self.tabs.active || self.switch_to_tab_index(index, cx)
    }

    /// 已知延迟动作同步调用本实体入口，防止异步 window dispatch 跑到新的标签或输入框。
    fn replay_ime_action(
        &mut self,
        action: Box<dyn Action>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use crate::components::*;
        if action.as_any().is::<Undo>() {
            self.on_undo(&Undo, window, cx);
        } else if action.as_any().is::<Redo>() {
            self.on_redo(&Redo, window, cx);
        } else if action.as_any().is::<SaveDocument>() {
            self.save_document(window, cx);
        } else if action.as_any().is::<SaveDocumentAs>() {
            self.save_document_as(window, cx);
        } else if action.as_any().is::<DuplicateLine>() {
            self.on_duplicate_line(&DuplicateLine, window, cx);
        } else if action.as_any().is::<DeleteLine>() {
            self.on_delete_line(&DeleteLine, window, cx);
        } else if action.as_any().is::<MoveLineUp>() {
            self.on_move_line_up(&MoveLineUp, window, cx);
        } else if action.as_any().is::<MoveLineDown>() {
            self.on_move_line_down(&MoveLineDown, window, cx);
        } else if action.as_any().is::<IndentBlock>() {
            self.on_indent_block(&IndentBlock, window, cx);
        } else if action.as_any().is::<OutdentBlock>() {
            self.on_outdent_block(&OutdentBlock, window, cx);
        } else if action.as_any().is::<MoveToDocumentStart>() {
            self.on_move_to_document_start(&MoveToDocumentStart, window, cx);
        } else if action.as_any().is::<MoveToDocumentEnd>() {
            self.on_move_to_document_end(&MoveToDocumentEnd, window, cx);
        } else if action.as_any().is::<SelectToDocumentStart>() {
            self.on_select_to_document_start(&SelectToDocumentStart, window, cx);
        } else if action.as_any().is::<SelectToDocumentEnd>() {
            self.on_select_to_document_end(&SelectToDocumentEnd, window, cx);
        } else if action.as_any().is::<SelectUp>() {
            self.on_select_up(&SelectUp, window, cx);
        } else if action.as_any().is::<SelectDown>() {
            self.on_select_down(&SelectDown, window, cx);
        } else if action.as_any().is::<SelectPageUp>() {
            self.on_select_page_up(&SelectPageUp, window, cx);
        } else if action.as_any().is::<SelectPageDown>() {
            self.on_select_page_down(&SelectPageDown, window, cx);
        } else if action.as_any().is::<PageUp>() {
            self.on_page_up(&PageUp, window, cx);
        } else if action.as_any().is::<PageDown>() {
            self.on_page_down(&PageDown, window, cx);
        } else if action.as_any().is::<FindInDocument>() {
            self.on_find_in_document_action(&FindInDocument, window, cx);
        } else if action.as_any().is::<ReplaceInDocument>() {
            self.on_replace_in_document_action(&ReplaceInDocument, window, cx);
        } else if action.as_any().is::<FindNext>() {
            self.on_find_next_action(&FindNext, window, cx);
        } else if action.as_any().is::<FindPrevious>() {
            self.on_find_previous_action(&FindPrevious, window, cx);
        } else if action.as_any().is::<CommandPalette>() {
            self.on_command_palette_action(&CommandPalette, window, cx);
        } else if action.as_any().is::<CloseWindow>() {
            self.on_close_window(&CloseWindow, window, cx);
        } else if action.as_any().is::<QuitApplication>() {
            self.on_quit_application(&QuitApplication, window, cx);
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/editor/ime_lifecycle.rs"]
mod tests;
