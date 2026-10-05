// @author kongweiguang

use super::*;

impl Editor {
    /// 窗口壳与子窗格共用文档收尾；候选仍暂存时不能把其临时选区写入正文历史。
    pub(super) fn sync_document_frame(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_split_scroll_handles(cx);
        if let Some(focus_handle) = self.diagram_overlay_restore_focus.take() {
            window.defer(cx, move |window, _cx| focus_handle.focus(window));
        }
        if !self.has_active_ime_composition(cx) {
            self.last_selection_snapshot = self.capture_source_selection_snapshot(cx);
            self.source_document
                .sync_source_selection(self.last_selection_snapshot.source_selection());
        }
        self.refresh_find_if_stale(cx);
        self.sync_pending_save(window, cx);
        self.sync_pending_save_as(window, cx);
        self.sync_pending_open_link(window, cx);
    }

    /// 子窗格持有跨块选区与只读边界，输入捕获必须在所属 Editor 上处理；窗口壳没有这些状态。
    /// 查找与错误恢复同样属于正文目标；Esc 已绑定为 Action，需在子窗格收尾而非窗口壳。
    pub(super) fn render_pane_document_canvas(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.global::<ThemeManager>().current_arc();
        let strings = cx.global::<I18nManager>().strings_arc();
        let content = self.render_document_content(window, cx);
        let viewport = self.pane_canvas_viewport.unwrap_or(window.viewport_size());
        let content = self.render_resident_surface(
            content,
            &theme,
            (f32::from(viewport.width) - SPLIT_DIVIDER_HIT_WIDTH).max(1.0),
            f32::from(viewport.height).max(1.0),
            window,
            cx,
        );
        let editor = cx.entity().downgrade();
        let mut base = div()
            .id("pane-document-surface")
            .debug_selector(|| "pane-document-surface".to_owned())
            .size_full()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .relative()
            .capture_action(cx.listener(Self::on_copy_capture))
            .capture_action(cx.listener(Self::on_copy_as_markdown_capture))
            .capture_action(cx.listener(Self::on_cut_capture))
            .capture_action(cx.listener(Self::on_paste_capture))
            .capture_action(cx.listener(Self::on_delete_capture))
            .capture_action(cx.listener(Self::on_delete_back_capture))
            .capture_key_down(cx.listener(Self::on_editor_key_down_capture))
            .on_action(cx.listener(Self::on_dismiss_transient_ui))
            .font(editor_text_font(cx))
            .child(
                canvas(
                    move |bounds, window, cx| {
                        let viewport = bounds.size;
                        let changed = editor
                            .read_with(cx, |editor, _| {
                                editor.pane_canvas_viewport != Some(viewport)
                            })
                            .unwrap_or(false);
                        if changed && viewport.width > px(0.0) && viewport.height > px(0.0) {
                            let editor = editor.clone();
                            window.defer(cx, move |_window, cx| {
                                let _ = editor.update(cx, |editor, cx| {
                                    if editor.pane_canvas_viewport != Some(viewport) {
                                        editor.pane_canvas_viewport = Some(viewport);
                                        cx.notify();
                                    }
                                });
                            });
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .child(content);
        if let Some(panel) = self.render_find_panel(&theme, &strings, 0.0, cx) {
            base = base.child(panel);
        }
        if let Some(menu) = self.render_context_menu_overlay(&theme, window, cx) {
            base = base.child(menu);
        }
        if let Some(dialog) = self.render_table_insert_dialog_overlay(&theme, cx) {
            base = base.child(dialog);
        }
        if let Some(prompt) = self.render_table_fragment_merge_prompt(&theme, &strings, cx) {
            base = base.child(prompt);
        }
        if let Some(completion) = self.render_workspace_link_completion(&theme, &strings, cx) {
            base = base.child(completion);
        }
        if let Some(overlay) = self.render_diagram_overlay(&theme, &strings, window, cx) {
            base = base.child(overlay);
        }
        base = base.children(self.render_resource_title_dialog_overlay(&theme, cx));
        self.render_document_dialogs(base, &theme, window, cx)
    }

    /// 保存冲突、编码与丢弃确认属于正文目标，必须在其可见表面提供原有恢复入口。
    pub(super) fn render_document_dialogs(
        &self,
        base: impl ParentElement + IntoElement,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if self.show_external_conflict_dialog {
            base.child(self.render_external_conflict_overlay(theme, window, cx))
        } else if self.show_encoding_conversion_dialog {
            base.child(self.render_encoding_conversion_overlay(theme, cx))
        } else if let Some(kind) = self.info_dialog {
            base.child(self.render_info_dialog_overlay(theme, kind, cx))
        } else if self.show_drop_replace_dialog {
            base.child(self.render_drop_replace_overlay(theme, cx))
        } else if self.show_unsaved_changes_dialog {
            base.child(self.render_unsaved_changes_overlay(theme, cx))
        } else {
            base
        }
        .into_any_element()
    }
}
