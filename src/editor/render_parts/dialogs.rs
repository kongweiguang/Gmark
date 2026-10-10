// @author kongweiguang

use super::*;

impl Editor {
    /// 模态层注册稳定焦点，并在 GPUI 裸按键之前吸收已匹配动作，覆盖自定义快捷键。
    pub(super) fn render_unsaved_changes_overlay(
        &self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let d = &theme.dimensions;
        let strings = cx.global::<I18nManager>().strings();
        let focus_ring = vec![BoxShadow {
            color: theme.colors.workbench.focus_ring,
            offset: point(px(0.0), px(0.0)),
            blur_radius: px(0.0),
            spread_radius: px(2.0),
        }];
        let actions = cx
            .key_bindings()
            .borrow()
            .bindings()
            .map(|binding| {
                (
                    binding.action().as_any().type_id(),
                    binding.action().boxed_clone(),
                )
            })
            .collect::<std::collections::HashMap<_, _>>();
        let mut overlay = modal_overlay("unsaved-changes-overlay", theme)
            .on_key_down(cx.listener(Self::on_close_dialog_key_down));
        for action in actions.into_values() {
            let editor = cx.entity().downgrade();
            overlay = overlay.on_boxed_action(action.as_ref(), move |action, window, cx| {
                let modal = editor
                    .update(cx, |editor, cx| {
                        if !editor.show_unsaved_changes_dialog {
                            return false;
                        }
                        if action
                            .as_any()
                            .is::<crate::components::DismissTransientUi>()
                        {
                            editor.cancel_close_dialog(window, cx);
                        }
                        true
                    })
                    .unwrap_or(false);
                if modal {
                    cx.stop_propagation();
                } else {
                    cx.propagate();
                }
            });
        }
        overlay.child(
            div()
                .w_full()
                .px(px(d.editor_padding))
                .flex()
                .justify_center()
                .child(
                    dialog_panel("unsaved-changes-dialog", d.dialog_width, theme)
                        .child(
                            dialog_content("unsaved-changes-content", theme)
                                .child(dialog_title_with_icon(
                                    "unsaved-changes-title",
                                    strings.unsaved_changes_title.clone(),
                                    DialogTitleIcon::Warning,
                                    theme,
                                ))
                                .child(
                                    dialog_body(strings.unsaved_changes_message.clone(), theme)
                                        .debug_selector(|| "unsaved-changes-message".to_owned()),
                                ),
                        )
                        .child(
                            dialog_actions(theme)
                                .id("unsaved-changes-actions")
                                .debug_selector(|| "unsaved-changes-actions".to_owned())
                                .child(
                                    dialog_button(
                                        "cancel-close-dialog",
                                        strings.unsaved_changes_cancel.clone(),
                                        DialogButtonKind::Secondary,
                                        theme,
                                    )
                                    .tab_index(0)
                                    .track_focus(&self.close_dialog_focus_handles[0])
                                    .focus(|style| style.shadow(focus_ring.clone()))
                                    .on_click(cx.listener(Self::on_cancel_close_dialog)),
                                )
                                .child(
                                    dialog_button(
                                        "discard-and-close-dialog",
                                        strings.unsaved_changes_discard_and_close.clone(),
                                        DialogButtonKind::Danger,
                                        theme,
                                    )
                                    .tab_index(1)
                                    .track_focus(&self.close_dialog_focus_handles[1])
                                    .focus(|style| style.shadow(focus_ring.clone()))
                                    .on_click(cx.listener(Self::on_discard_and_close)),
                                )
                                .child(
                                    dialog_button(
                                        "save-and-close-dialog",
                                        strings.unsaved_changes_save_and_close.clone(),
                                        DialogButtonKind::Primary,
                                        theme,
                                    )
                                    .tab_index(2)
                                    .track_focus(&self.close_dialog_focus_handles[2])
                                    .focus(|style| style.shadow(focus_ring.clone()))
                                    .on_click(cx.listener(Self::on_save_and_close)),
                                ),
                        ),
                ),
        )
    }

    pub(super) fn render_external_conflict_overlay(
        &self,
        theme: &Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let c = &theme.colors;
        let d = &theme.dimensions;
        let t = &theme.typography;
        let strings = cx.global::<I18nManager>().strings();
        let max_dialog_height = px(f32::from(window.viewport_size().height) * 0.9);
        let preview = self
            .external_conflict_preview
            .as_ref()
            .expect("visible external conflict requires comparison data");
        let summary = if let Some(error) = preview.disk_error.as_ref() {
            error.clone()
        } else if let Some(line) = preview.first_difference_line {
            strings
                .external_change_first_difference_template
                .replace("{line}", &line.to_string())
                .replace("{local_lines}", &preview.local_line_count.to_string())
                .replace("{local_bytes}", &preview.local_bytes.to_string())
                .replace("{disk_lines}", &preview.disk_line_count.to_string())
                .replace("{disk_bytes}", &preview.disk_bytes.to_string())
        } else {
            strings.external_change_metadata_only.clone()
        };
        let comparison = |id: &'static str, label: String, text: String| {
            div()
                .id(id)
                .debug_selector(|| id.to_owned())
                .flex_1()
                .min_w(px(240.0))
                .flex()
                .flex_col()
                .gap(px(6.0))
                .child(
                    div()
                        .text_size(px(t.dialog_button_size))
                        .font_weight(t.dialog_title_weight.to_font_weight())
                        .text_color(c.workbench.text_primary)
                        .child(label),
                )
                .child(
                    div()
                        .w_full()
                        .min_h(px(56.0))
                        .p(px(8.0))
                        .bg(c.code_bg)
                        .border(px(d.dialog_border_width))
                        .border_color(c.workbench.border_subtle)
                        .rounded(px(4.0))
                        .font_family(crate::document_host::source_monospace_font_family())
                        .text_size(px(12.0))
                        .text_color(c.workbench.text_secondary)
                        .child(text),
                )
        };

        modal_overlay("external-conflict-overlay", theme).child(
            div()
                .w_full()
                .px(px(d.editor_padding))
                .flex()
                .justify_center()
                .child(
                    dialog_panel(
                        "external-conflict-dialog",
                        (d.dialog_width * 1.65).max(640.0),
                        theme,
                    )
                    .max_h(max_dialog_height)
                    .child(
                        dialog_content("external-conflict-content", theme)
                            .child(dialog_title_with_icon(
                                "external-conflict-title",
                                strings.external_change_title.clone(),
                                DialogTitleIcon::Warning,
                                theme,
                            ))
                            .child(dialog_body(
                                strings.external_change_save_as_message.clone(),
                                theme,
                            ))
                            .child(
                                div()
                                    .id("external-conflict-path")
                                    .debug_selector(|| "external-conflict-path".to_owned())
                                    .text_size(px(t.dialog_button_size))
                                    .text_color(c.workbench.text_tertiary)
                                    .child(preview.path.clone()),
                            )
                            .child(
                                div()
                                    .id("external-conflict-summary")
                                    .debug_selector(|| "external-conflict-summary".to_owned())
                                    .text_size(px(t.dialog_body_size))
                                    .text_color(c.workbench.warning)
                                    .child(summary),
                            )
                            .child(
                                div()
                                    .w_full()
                                    .flex()
                                    .flex_wrap()
                                    .gap(px(d.dialog_gap))
                                    .child(comparison(
                                        "external-conflict-local",
                                        strings.external_change_local_label.clone(),
                                        preview.local_line.clone(),
                                    ))
                                    .child(comparison(
                                        "external-conflict-disk",
                                        strings.external_change_disk_label.clone(),
                                        preview.disk_line.clone(),
                                    )),
                            ),
                    )
                    .child(
                        dialog_actions(theme)
                            .child(
                                dialog_button(
                                    "cancel-external-conflict",
                                    strings.external_change_cancel.clone(),
                                    DialogButtonKind::Secondary,
                                    theme,
                                )
                                .on_click(cx.listener(Self::on_cancel_external_conflict)),
                            )
                            .child(
                                dialog_button(
                                    "reload-external-conflict",
                                    strings.external_change_reload.clone(),
                                    DialogButtonKind::Secondary,
                                    theme,
                                )
                                .on_click(cx.listener(Self::on_reload_external_conflict)),
                            )
                            .child(
                                dialog_button(
                                    "overwrite-external-conflict",
                                    strings.external_change_overwrite.clone(),
                                    DialogButtonKind::Danger,
                                    theme,
                                )
                                .on_click(cx.listener(Self::on_overwrite_external_conflict)),
                            )
                            .child(
                                dialog_button(
                                    "save-as-external-conflict",
                                    strings.external_change_save_as.clone(),
                                    DialogButtonKind::Primary,
                                    theme,
                                )
                                .on_click(cx.listener(Self::on_save_as_external_conflict)),
                            ),
                    ),
                ),
        )
    }

    pub(super) fn render_encoding_conversion_overlay(
        &self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let d = &theme.dimensions;
        let strings = cx.global::<I18nManager>().strings();
        let message = strings
            .encoding_conversion_message_template
            .replace("{encoding}", self.source_encoding.label());

        modal_overlay("encoding-conversion-overlay", theme).child(
            dialog_panel("encoding-conversion-dialog", d.dialog_width, theme)
                .child(
                    dialog_content("encoding-conversion-content", theme)
                        .child(dialog_title_with_icon(
                            "encoding-conversion-title",
                            strings.encoding_conversion_title.clone(),
                            DialogTitleIcon::Source,
                            theme,
                        ))
                        .child(dialog_body(message, theme)),
                )
                .child(
                    dialog_actions(theme)
                        .child(
                            dialog_button(
                                "keep-legacy-read-only",
                                strings.encoding_keep_read_only.clone(),
                                DialogButtonKind::Secondary,
                                theme,
                            )
                            .on_click(cx.listener(Self::on_keep_legacy_encoding_read_only)),
                        )
                        .child(
                            dialog_button(
                                "convert-encoding-utf8",
                                strings.encoding_convert_utf8.clone(),
                                DialogButtonKind::Primary,
                                theme,
                            )
                            .on_click(cx.listener(Self::on_convert_encoding_to_utf8)),
                        ),
                ),
        )
    }

    /// 只有资源复制展示真实计数；浏览器生成保持不确定进度，提交阶段移除取消入口。
    pub(in crate::editor) fn render_export_progress(
        &self,
        theme: &Theme,
        bottom_offset: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let c = &theme.colors;
        let d = &theme.dimensions;
        let t = &theme.typography;
        let strings = cx.global::<I18nManager>().strings();
        let cancel_requested = self.export_cancel_requested;
        let committing = self
            .export_progress
            .as_ref()
            .is_none_or(|progress| progress.is_committing());
        let progress_text = self
            .export_progress
            .as_ref()
            .map(|progress| {
                let completed = progress
                    .completed
                    .load(std::sync::atomic::Ordering::Acquire);
                let total = progress
                    .total
                    .load(std::sync::atomic::Ordering::Acquire)
                    .max(1);
                let label = if committing {
                    &strings.export_saving
                } else if cancel_requested {
                    &strings.export_cancelling
                } else {
                    &strings.export_in_progress
                };
                if total > 1 && !committing && !cancel_requested {
                    SharedString::from(format!("{label} ({}/{total})", completed.min(total)))
                } else {
                    SharedString::from(label.clone())
                }
            })
            .unwrap_or_else(|| {
                if committing {
                    strings.export_saving.clone().into()
                } else if cancel_requested {
                    strings.export_cancelling.clone().into()
                } else {
                    strings.export_in_progress.clone().into()
                }
            });
        let cancel_button = div()
            .id("cancel-export")
            .debug_selector(|| "cancel-export".to_owned())
            .h(px(24.0))
            .px(px(7.0))
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap(px(4.0))
            .rounded(px(4.0))
            .bg(c.workbench.control_surface)
            .text_color(c.workbench.text_primary)
            .child(
                svg()
                    .path(CLOSE_ICON)
                    .size(px(13.0))
                    .debug_selector(|| "cancel-export-icon".to_owned()),
            )
            .child(strings.export_cancel.clone());
        let cancel_button = if cancel_requested {
            cancel_button.opacity(0.55)
        } else {
            cancel_button
                .hover(|this| this.bg(c.workbench.control_hover))
                .cursor_pointer()
                .on_click(cx.listener(Self::on_cancel_export))
        };
        div()
            .id("export-progress")
            .debug_selector(|| "export-progress".to_owned())
            .absolute()
            // 进度条仅遮挡自身命中区域；外围编辑器仍保持非模态可交互。
            .occlude()
            .on_mouse_down(MouseButton::Left, |_event, _window, cx| {
                cx.stop_propagation()
            })
            .right(px(12.0))
            .bottom(px(bottom_offset + 10.0))
            .h(px(36.0))
            .max_w(relative(0.92))
            .px(px(10.0))
            .flex()
            .items_center()
            .gap(px(10.0))
            .rounded(px(6.0))
            .border(px(d.dialog_border_width))
            .border_color(c.workbench.border_subtle)
            .bg(c.workbench.glass_strong_surface)
            .shadow_md()
            .text_size(px(t.dialog_button_size))
            .text_color(c.workbench.text_secondary)
            .child(
                div()
                    .size(px(18.0))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(c.workbench.accent)
                    .debug_selector(|| "export-progress-icon".to_owned())
                    .child(svg().path(EXPORT_PROGRESS_ICON).size(px(16.0))),
            )
            .child(
                div()
                    .min_w(px(0.0))
                    .flex_grow()
                    .overflow_hidden()
                    .truncate()
                    .debug_selector(|| "export-progress-label".to_owned())
                    .child(progress_text),
            )
            .children((!committing).then_some(cancel_button))
            .into_any_element()
    }

    /// Builds the dropped-file replacement dialog shown when the current
    /// document has unsaved changes.
    pub(super) fn render_drop_replace_overlay(
        &self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let d = &theme.dimensions;
        let strings = cx.global::<I18nManager>().strings();

        modal_overlay("drop-replace-overlay", theme).child(
            div()
                .w_full()
                .px(px(d.editor_padding))
                .flex()
                .justify_center()
                .child(
                    dialog_panel("drop-replace-dialog", d.dialog_width, theme)
                        .child(
                            dialog_content("drop-replace-content", theme)
                                .child(dialog_title_with_icon(
                                    "drop-replace-title",
                                    strings.drop_replace_title.clone(),
                                    DialogTitleIcon::Refresh,
                                    theme,
                                ))
                                .child(dialog_body(strings.drop_replace_message.clone(), theme)),
                        )
                        .child(
                            dialog_actions(theme)
                                .child(
                                    dialog_button(
                                        "cancel-drop-replace-dialog",
                                        strings.drop_replace_cancel.clone(),
                                        DialogButtonKind::Secondary,
                                        theme,
                                    )
                                    .on_click(cx.listener(Self::on_cancel_drop_replace_dialog)),
                                )
                                .child(
                                    dialog_button(
                                        "discard-and-replace-drop-dialog",
                                        strings.drop_replace_discard_and_replace.clone(),
                                        DialogButtonKind::Danger,
                                        theme,
                                    )
                                    .on_click(cx.listener(Self::on_discard_and_replace_drop)),
                                )
                                .child(
                                    dialog_button(
                                        "save-and-replace-drop-dialog",
                                        strings.drop_replace_save_and_replace.clone(),
                                        DialogButtonKind::Primary,
                                        theme,
                                    )
                                    .on_click(cx.listener(Self::on_save_and_replace_drop)),
                                ),
                        ),
                ),
        )
    }
}
