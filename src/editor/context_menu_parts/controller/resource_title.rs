// @author kongweiguang

use super::*;
use crate::components::{Block, BlockEvent};
use crate::theme::workbench::SurfaceKind;
use crate::ui::visual_preferences::VisualPreferencesManager;

impl Editor {
    pub(in crate::editor) fn open_resource_from_context_menu(
        &mut self,
        _event: &ClickEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(block) = self.resource_context_block(cx) else {
            return;
        };
        let Some(record) = self.resource_context_record(cx) else {
            return;
        };
        self.close_context_menu(cx);
        block.update(cx, |block, cx| {
            block.request_resource_open(&record, cx);
        });
    }

    pub(in crate::editor) fn reveal_resource_from_context_menu(
        &mut self,
        _event: &ClickEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let block = self.resource_context_block(cx);
        let path = self
            .resource_context_record(cx)
            .as_ref()
            .and_then(ResourceRecord::local_path)
            .map(std::path::Path::to_path_buf);
        self.close_context_menu(cx);
        if let Some(path) = path
            && let Err(error) = crate::resource_io::reveal_local_resource(&path)
        {
            if let Some(block) = block {
                block.update(cx, |block, cx| block.mark_resource_open_failed(cx));
            }
            eprintln!("failed to reveal resource '{}': {error}", path.display());
        }
    }

    /// Opens title editing only when the resource belongs to a writable document surface.
    pub(in crate::editor) fn edit_resource_title_from_context_menu(
        &mut self,
        _event: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(block) = self.resource_context_block(cx) else {
            return;
        };
        if !self.resource_target_is_editable(block.entity_id()) {
            return;
        }
        let Some(previous) = self.resource_context_record(cx) else {
            return;
        };
        self.request_resource_title_dialog(block.entity_id(), previous, window, cx);
    }

    /// Holds title-dialog creation until the current IME owner finishes and revalidates the resource.
    pub(in crate::editor) fn request_resource_title_dialog(
        &mut self,
        entity_id: EntityId,
        previous: ResourceRecord,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.resource_target_is_editable(entity_id)
            || !self.resource_content_matches(entity_id, &previous, cx)
            || self.resource_title_dialog.is_some()
        {
            return;
        }
        if self.defer_tool_ime_intent(
            super::super::super::tool_ime::ToolImeIntent::ResourceTitleOpen {
                entity_id,
                previous: previous.clone(),
            },
            window,
            cx,
        ) {
            return;
        }
        self.open_resource_title_dialog(entity_id, previous, window, cx);
    }

    /// Creates the input only after the original resource meaning is still current.
    fn open_resource_title_dialog(
        &mut self,
        entity_id: EntityId,
        previous: ResourceRecord,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.resource_target_is_editable(entity_id)
            || !self.resource_content_matches(entity_id, &previous, cx)
            || self.resource_title_dialog.is_some()
        {
            return;
        }
        if self.context_menu.as_ref().is_some_and(|menu| {
            matches!(menu, ContextMenuState::Resource { entity_id: menu_id, .. } if *menu_id == entity_id)
        }) {
            self.close_context_menu(cx);
        }
        let label = previous.label.clone();
        let input = cx.new(|cx| {
            let mut input =
                crate::components::Block::with_record(cx, BlockRecord::paragraph(label));
            input.set_source_raw_mode();
            input
        });
        cx.subscribe(&input, Self::on_resource_title_input_event)
            .detach();
        input.read(cx).focus_handle.focus(window);
        self.resource_title_dialog = Some(crate::editor::ResourceTitleDialogState {
            entity_id,
            previous,
            input,
        });
        cx.notify();
    }

    /// Retries a terminal action without reading or unmounting the title field while candidates are active.
    pub(in crate::editor) fn request_resource_title_confirmation(
        &mut self,
        entity_id: EntityId,
        input_id: EntityId,
        previous: &ResourceRecord,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.resource_title_dialog_matches(entity_id, input_id, previous)
            || !self.resource_target_is_editable(entity_id)
        {
            return;
        }
        if !self.resource_content_matches(entity_id, previous, cx) {
            self.show_pane_notice("资源内容已变化，未应用旧标题，请重新编辑", cx);
            return;
        }
        if self.defer_tool_ime_intent(
            super::super::super::tool_ime::ToolImeIntent::ResourceTitleConfirm {
                entity_id,
                input_id,
                previous: previous.clone(),
            },
            window,
            cx,
        ) {
            return;
        }
        self.confirm_resource_title_dialog(cx);
    }

    /// Keeps a delayed Cancel tied to the exact dialog even if a replacement dialog opens later.
    pub(in crate::editor) fn request_resource_title_cancellation(
        &mut self,
        entity_id: EntityId,
        input_id: EntityId,
        previous: &ResourceRecord,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.resource_title_dialog_matches(entity_id, input_id, previous) {
            return;
        }
        if self.defer_tool_ime_intent(
            super::super::super::tool_ime::ToolImeIntent::ResourceTitleCancel {
                entity_id,
                input_id,
                previous: previous.clone(),
            },
            window,
            cx,
        ) {
            return;
        }
        self.cancel_resource_title_dialog(cx);
    }

    /// Matches the exact dialog and semantic resource snapshot, ignoring Markdown spelling.
    fn resource_title_dialog_matches(
        &self,
        entity_id: EntityId,
        input_id: EntityId,
        previous: &ResourceRecord,
    ) -> bool {
        self.resource_title_dialog.as_ref().is_some_and(|dialog| {
            dialog.entity_id == entity_id
                && dialog.input.entity_id() == input_id
                && resource_records_match(&dialog.previous, previous)
        })
    }

    /// Ensures a shared edit did not change the resource target while its title dialog was open.
    fn resource_content_matches(
        &self,
        entity_id: EntityId,
        previous: &ResourceRecord,
        cx: &App,
    ) -> bool {
        self.focusable_entity_by_id(entity_id).is_some_and(|block| {
            block
                .read(cx)
                .record
                .resource
                .as_ref()
                .is_some_and(|record| resource_records_match(record, previous))
        })
    }

    /// Captures the concrete input and domain snapshot used to validate a delayed dialog command.
    fn current_resource_title_dialog_identity(
        &self,
    ) -> Option<(EntityId, EntityId, ResourceRecord)> {
        let dialog = self.resource_title_dialog.as_ref()?;
        Some((
            dialog.entity_id,
            dialog.input.entity_id(),
            dialog.previous.clone(),
        ))
    }

    /// Keeps the dialog mounted while any input surface still owns pre-edit text.
    pub(in crate::editor) fn cancel_resource_title_dialog(&mut self, cx: &mut Context<Self>) {
        if self.has_active_ime_composition(cx) {
            return;
        }
        if let Some(dialog) = self.resource_title_dialog.take() {
            if self.focusable_entity_by_id(dialog.entity_id).is_some() {
                self.focus_block(dialog.entity_id);
            }
            cx.notify();
        }
    }

    /// Revalidates the writable target and resource identity immediately before the write.
    pub(in crate::editor) fn confirm_resource_title_dialog(&mut self, cx: &mut Context<Self>) {
        if self.has_active_ime_composition(cx) {
            return;
        }
        let Some(entity_id) = self
            .resource_title_dialog
            .as_ref()
            .map(|dialog| dialog.entity_id)
        else {
            return;
        };
        if !self.resource_target_is_editable(entity_id) {
            return;
        }
        let Some(previous) = self
            .resource_title_dialog
            .as_ref()
            .map(|dialog| dialog.previous.clone())
        else {
            return;
        };
        if !self.resource_content_matches(entity_id, &previous, cx) {
            self.show_pane_notice("资源内容已变化，未应用旧标题，请重新编辑", cx);
            return;
        }
        let Some(dialog) = self.resource_title_dialog.take() else {
            return;
        };
        let label = dialog.input.read(cx).display_text().to_owned();
        let Some(block) = self.focusable_entity_by_id(dialog.entity_id) else {
            cx.notify();
            return;
        };
        let destination = if dialog.previous.is_local() {
            dialog.previous.destination.replace('\\', "/")
        } else {
            dialog.previous.destination.clone()
        };
        let base_dir = self.image_base_dir();
        let record = ResourceRecord::from_parts(
            label,
            destination,
            dialog.previous.explicit_kind,
            base_dir.as_deref(),
        );
        let markdown = record.to_markdown();

        self.prepare_undo_capture(crate::components::UndoCaptureKind::NonCoalescible, cx);
        let title = InlineTextTree::from_markdown(&markdown);
        let cursor = title.visible_len();
        Self::set_block_title_and_kind(&block, block.read(cx).kind(), title, cursor, cx);
        self.rebuild_image_runtimes(cx);
        self.mark_dirty(cx);
        self.finalize_pending_undo_capture(cx);
        self.focus_block(dialog.entity_id);
        cx.notify();
    }

    /// Lets candidate keys reach the title field before Enter or Escape acts on the dialog.
    pub(in crate::editor) fn on_resource_title_dialog_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .resource_title_dialog
            .as_ref()
            .is_some_and(|dialog| dialog.input.read(cx).has_ime_composition())
        {
            return;
        }
        match event.keystroke.key.as_str() {
            "enter" => {
                if let Some((entity_id, input_id, previous)) =
                    self.current_resource_title_dialog_identity()
                {
                    self.request_resource_title_confirmation(
                        entity_id, input_id, &previous, window, cx,
                    );
                }
                cx.stop_propagation();
            }
            "escape" => {
                if let Some((entity_id, input_id, previous)) =
                    self.current_resource_title_dialog_identity()
                {
                    self.request_resource_title_cancellation(
                        entity_id, input_id, &previous, window, cx,
                    );
                }
                cx.stop_propagation();
            }
            _ => {}
        }
    }

    /// Routes button confirmation through the same identity and IME completion gates as Enter.
    pub(in crate::editor) fn on_confirm_resource_title_dialog(
        &mut self,
        _event: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some((entity_id, input_id, previous)) = self.current_resource_title_dialog_identity()
        {
            self.request_resource_title_confirmation(entity_id, input_id, &previous, window, cx);
        }
    }

    /// Routes button cancellation through the same identity and IME completion gates as Escape.
    pub(in crate::editor) fn on_cancel_resource_title_dialog(
        &mut self,
        _event: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some((entity_id, input_id, previous)) = self.current_resource_title_dialog_identity()
        {
            self.request_resource_title_cancellation(entity_id, input_id, &previous, window, cx);
        }
    }

    /// Wakes queued intents on terminal input while rejecting events from an older dialog.
    fn on_resource_title_input_event(
        &mut self,
        input: Entity<Block>,
        event: &BlockEvent,
        cx: &mut Context<Self>,
    ) {
        if self
            .resource_title_dialog
            .as_ref()
            .is_none_or(|dialog| dialog.input.entity_id() != input.entity_id())
        {
            return;
        }
        match event {
            BlockEvent::ImeCompositionEnded { .. } => {
                self.ime_completion_requested = false;
                self.ime_completion_failed = false;
                cx.notify();
            }
            BlockEvent::ImeCompositionFinishFailed => {
                self.ime_completion_requested = false;
                self.ime_completion_failed = true;
                self.show_pane_notice("输入法暂未完成，请确认或取消候选后重试操作", cx);
            }
            _ => {}
        }
    }

    pub(in crate::editor) fn render_resource_title_dialog_overlay(
        &self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let dialog = self.resource_title_dialog.as_ref()?;
        let strings = cx.global::<I18nManager>().strings().clone();
        let title = strings
            .slash_commands
            .get("resource_edit_title")
            .cloned()
            .unwrap_or_else(|| "Edit Resource Title".to_owned());
        let c = &theme.colors;
        let visual_preferences = cx
            .try_global::<VisualPreferencesManager>()
            .map(VisualPreferencesManager::current)
            .unwrap_or_default();
        let palette = &c.workbench;
        let solid_material = palette.material(SurfaceKind::Solid, visual_preferences);
        let d = &theme.dimensions;
        let t = &theme.typography;
        Some(
            modal_overlay("resource-title-dialog-overlay", theme)
                .capture_key_down(cx.listener(Self::on_resource_title_dialog_key_down))
                .child(
                    dialog_panel("resource-title-dialog", d.dialog_width.min(480.0), theme)
                        .on_mouse_down(MouseButton::Left, |_event, _window, cx| {
                            cx.stop_propagation()
                        })
                        .child(
                            crate::editor::render::dialog_content(
                                "resource-title-dialog-content",
                                theme,
                            )
                            .child(dialog_title_with_icon(
                                "resource-title-dialog-title",
                                title,
                                DialogTitleIcon::Files,
                                theme,
                            ))
                            .child(
                                div()
                                    .text_size(px(t.dialog_body_size))
                                    .text_color(palette.text_primary)
                                    .child(
                                        strings
                                            .slash_commands
                                            .get("resource_title_field")
                                            .cloned()
                                            .unwrap_or_else(|| "Title".to_owned()),
                                    ),
                            )
                            .child(
                                div()
                                    .id("resource-title-dialog-input")
                                    .debug_selector(|| "resource-title-dialog-input".to_owned())
                                    .min_h(px(38.0))
                                    .w_full()
                                    .px(px(8.0))
                                    .flex()
                                    .items_center()
                                    .rounded(px(6.0))
                                    .border(px(d.dialog_border_width))
                                    .border_color(solid_material.border)
                                    .bg(solid_material.background)
                                    .child(dialog.input.clone()),
                            ),
                        )
                        .child(
                            dialog_actions(theme)
                                .child(
                                    dialog_button(
                                        "cancel-resource-title-dialog",
                                        strings.open_link_cancel.clone(),
                                        DialogButtonKind::Secondary,
                                        theme,
                                    )
                                    .on_click(cx.listener(Self::on_cancel_resource_title_dialog)),
                                )
                                .child(
                                    dialog_button(
                                        "confirm-resource-title-dialog",
                                        strings.info_dialog_ok.clone(),
                                        DialogButtonKind::Primary,
                                        theme,
                                    )
                                    .on_click(cx.listener(Self::on_confirm_resource_title_dialog)),
                                ),
                        ),
                )
                .into_any_element(),
        )
    }
}

/// Compares persisted resource meaning while ignoring source-Markdown serialization details.
fn resource_records_match(current: &ResourceRecord, expected: &ResourceRecord) -> bool {
    current.label == expected.label
        && current.destination == expected.destination
        && current.explicit_kind == expected.explicit_kind
}

#[cfg(all(test, target_os = "windows"))]
#[path = "../../../../tests/unit/editor/ime_resource_title.rs"]
mod ime_resource_title_tests;
