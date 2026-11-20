// @author kongweiguang

use super::*;
use crate::editor::PendingSplitDragScroll;

#[path = "info_dialog_parts/split_scrollbar.rs"]
mod split_scrollbar;

impl Editor {
    pub(super) fn info_dialog_title<'a>(
        &self,
        strings: &'a I18nStrings,
        kind: InfoDialogKind,
    ) -> &'a str {
        match kind {
            InfoDialogKind::About => &strings.help_about_title,
            InfoDialogKind::Document => &strings.document_sidebar_info,
        }
    }

    // reason: 测试断言稳定文案而正式渲染走富元素路径；remove when: 正式渲染复用此助手或测试覆盖富元素路径。
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn about_dialog_body_lines(strings: &I18nStrings) -> Vec<String> {
        vec![
            format!("Gmark {}", env!("CARGO_PKG_VERSION")),
            strings.help_about_message.clone(),
            format!("{}: {}", strings.help_about_github_label, ABOUT_GITHUB_URL),
        ]
    }

    pub(super) fn render_info_dialog_body(
        &self,
        theme: &Theme,
        strings: &I18nStrings,
        kind: InfoDialogKind,
        cx: &App,
    ) -> AnyElement {
        let c = &theme.colors;
        let d = &theme.dimensions;
        let t = &theme.typography;
        let body_style = |this: Div| {
            this.text_size(px(t.dialog_body_size))
                .font_weight(t.dialog_body_weight.to_font_weight())
                .line_height(rems(t.text_line_height))
                .text_color(c.workbench.text_secondary)
        };

        match kind {
            InfoDialogKind::About => div()
                .flex()
                .flex_col()
                .gap(px(d.dialog_gap * 0.5))
                .child(body_style(div()).child(format!("Gmark {}", env!("CARGO_PKG_VERSION"))))
                .child(body_style(div()).child(strings.help_about_message.clone()))
                .child(
                    body_style(div())
                        .flex()
                        .flex_wrap()
                        .gap(px(4.0))
                        .child(format!("{}:", strings.help_about_github_label))
                        .child(
                            div()
                                .id("about-github-link")
                                .cursor_pointer()
                                .text_color(c.workbench.accent)
                                .underline()
                                .child(ABOUT_GITHUB_URL)
                                .on_click(move |_, _, cx| {
                                    open_about_github_url(cx);
                                }),
                        ),
                )
                .into_any_element(),
            InfoDialogKind::Document => {
                let c = &theme.colors;
                let t = &theme.typography;
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.0))
                    .children(self.document_info_lines(cx).into_iter().map(|line| {
                        div()
                            .text_size(px(t.dialog_body_size))
                            .font_weight(t.dialog_body_weight.to_font_weight())
                            .line_height(rems(t.text_line_height))
                            .text_color(c.workbench.text_secondary)
                            .child(line)
                    }))
                    .into_any_element()
            }
        }
    }

    pub(super) fn on_dismiss_info_dialog(
        &mut self,
        _: &ClickEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.hide_info_dialog(cx);
    }

    pub(super) fn render_info_dialog_overlay(
        &self,
        theme: &Theme,
        kind: InfoDialogKind,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let d = &theme.dimensions;
        let strings = cx.global::<I18nManager>().strings();

        modal_overlay("info-dialog-overlay", theme).child(
            div()
                .w_full()
                .px(px(d.editor_padding))
                .flex()
                .justify_center()
                .child(
                    // 文档信息固定包含七个字段，预留完整阅读高度，避免“当前视图”被操作区遮住。
                    dialog_panel("info-dialog", d.dialog_width, theme)
                        .min_h(px(384.0))
                        .child(
                            dialog_content("info-dialog-content", theme)
                                .child(dialog_title_with_icon(
                                    "info-dialog-title",
                                    self.info_dialog_title(strings, kind).to_string(),
                                    DialogTitleIcon::Info,
                                    theme,
                                ))
                                .child(self.render_info_dialog_body(theme, strings, kind, cx)),
                        )
                        .child(
                            dialog_actions(theme).child(
                                dialog_button(
                                    "dismiss-info-dialog",
                                    strings.info_dialog_ok.clone(),
                                    DialogButtonKind::Primary,
                                    theme,
                                )
                                .on_click(cx.listener(Self::on_dismiss_info_dialog)),
                            ),
                        ),
                ),
        )
    }

    pub(super) fn start_split_resize(
        &mut self,
        pointer_x: Pixels,
        available_width: f32,
        current_ratio: f32,
        cx: &mut Context<Self>,
    ) {
        self.split_pane_ratio = current_ratio;
        self.split_resize_session = Some(SplitResizeSession {
            start_x: pointer_x,
            start_ratio: current_ratio,
            available_width: available_width.max(1.0),
        });
        cx.notify();
    }

    pub(super) fn on_split_divider_key_down(
        &mut self,
        event: &KeyDownEvent,
        available_width: f32,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (minimum, maximum) = split_pane_ratio_bounds(available_width);
        let step = if event.keystroke.modifiers.shift {
            SPLIT_KEYBOARD_LARGE_STEP
        } else {
            SPLIT_KEYBOARD_STEP
        };
        let next = match event.keystroke.key.as_str() {
            "left" => self.split_pane_ratio - step,
            "right" => self.split_pane_ratio + step,
            "home" => minimum,
            "end" => maximum,
            "enter" => 0.5,
            _ => return,
        };
        let next = clamped_split_pane_ratio(next, available_width);
        if (self.split_pane_ratio - next).abs() > f32::EPSILON {
            // 键盘调整是窗口布局提交，不进入 Markdown undo；会话写入仍由既有合并任务限流。
            self.split_pane_ratio = next;
            self.split_resize_session = None;
            self.schedule_workspace_session_save(cx);
            cx.notify();
        }
        cx.stop_propagation();
    }

    pub(super) fn on_editor_layout_resize_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(session) = self.split_resize_session {
            let delta = f32::from(event.position.x - session.start_x) / session.available_width;
            let ratio =
                clamped_split_pane_ratio(session.start_ratio + delta, session.available_width);
            if (self.split_pane_ratio - ratio).abs() > f32::EPSILON {
                self.split_pane_ratio = ratio;
                cx.notify();
            }
            cx.stop_propagation();
            return;
        }
        if self.on_document_sidebar_resize_mouse_move(event, cx) {
            return;
        }
        self.on_workspace_resize_mouse_move(event, window, cx);
    }

    pub(super) fn on_editor_layout_resize_mouse_up(
        &mut self,
        event: &MouseUpEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.split_resize_session.take().is_some() {
            // 与侧栏 resize 一致，拖动帧只改内存，释放后才合并写入窗口会话。
            self.schedule_workspace_session_save(cx);
            cx.notify();
            cx.stop_propagation();
            return;
        }
        if self.on_document_sidebar_resize_mouse_up(cx) {
            return;
        }
        self.on_workspace_resize_mouse_up(event, window, cx);
    }

    /// Split 使用自身实际列宽识别行高缓存，并按实际溢出显示滑块，与左侧交互一致。
    pub(super) fn render_split_preview_pane(
        &mut self,
        theme: &Theme,
        pane_width: f32,
        fallback_viewport_height: f32,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if self.is_svg_document() {
            let strings = cx.global::<I18nManager>().strings_arc();
            return Some(self.render_svg_document_preview(theme, &strings, true, cx));
        }
        if let Some(state) = self.split_preview.as_mut() {
            if let (Some(PendingSplitDragScroll::SourceAnchor(source_offset)), Some(surface)) =
                (state.pending_drag_scroll, state.virtual_surface.as_ref())
            {
                if let Some(target) = surface.y_for_source_offset(source_offset) {
                    let viewport = f32::from(state.scroll_handle.bounds().size.height).max(1.0);
                    let top = editor_top_padding(
                        super::ViewMode::Split,
                        false,
                        viewport,
                        &theme.dimensions,
                    );
                    let mut offset = state.scroll_handle.offset();
                    offset.y =
                        -px(target + top).min(state.scroll_handle.max_offset().height.max(px(0.0)));
                    state.scroll_handle.set_offset(offset);
                }
                state.pending_drag_scroll =
                    Some(PendingSplitDragScroll::AlignSource(source_offset));
            }
        }
        self.sync_split_virtual_surface_mounts(cx);
        let (visible, scroll_handle, virtual_spacers) = {
            let state = self.split_preview.as_ref()?;
            (
                state.document.visible_blocks().to_vec(),
                state.scroll_handle.clone(),
                state
                    .virtual_surface
                    .as_ref()
                    .map(|surface| (surface.top_spacer_height(), surface.bottom_spacer_height())),
            )
        };
        let d = &theme.dimensions;
        let measured_viewport_height = f32::from(scroll_handle.bounds().size.height);
        let viewport_height = if measured_viewport_height > 1.0 {
            measured_viewport_height
        } else {
            fallback_viewport_height.max(1.0)
        };
        let top_padding = editor_top_padding(super::ViewMode::Split, false, viewport_height, d);
        let bottom_padding = editor_bottom_padding(viewport_height, d);
        let centered_width = crate::ui::centered_column_width(pane_width, d);
        let max_scroll_y = f32::from(scroll_handle.max_offset().height.max(px(0.0)));
        let mut current_scroll_y = (-f32::from(scroll_handle.offset().y)).clamp(0.0, max_scroll_y);
        let scrollbar_geometry =
            Self::scrollbar_geometry(viewport_height, max_scroll_y, current_scroll_y);
        let show_scrollbar = max_scroll_y > 0.5;
        let spacing_for = |index: usize| {
            visible[index]
                .entity
                .read_with(cx, |block, _cx| RenderedRowSpacingInfo::from_block(block))
        };
        let mut rows = Vec::new();
        let mut row_starts = Vec::new();
        let mut gap_state = RenderedRowGapState::default();
        let mut index = 0usize;

        while index < visible.len() {
            let first_spacing = spacing_for(index);
            let Some(top_gap) = gap_state.root_gap(first_spacing, d.block_gap) else {
                index += 1;
                continue;
            };

            if let (Some(callout_anchor), Some(callout_variant)) =
                (first_spacing.callout_anchor, first_spacing.callout_variant)
            {
                let mut children = Vec::new();
                let mut end = index;
                let mut callout_gaps = RenderedRowGapState::default();
                while end < visible.len() && spacing_for(end).callout_anchor == Some(callout_anchor)
                {
                    let spacing = spacing_for(end);
                    if let Some(gap) = callout_gaps.callout_gap(spacing, d) {
                        children.push(
                            div()
                                .w_full()
                                .flex_shrink_0()
                                .mt(px(gap))
                                .child(visible[end].entity.clone())
                                .into_any_element(),
                        );
                    }
                    end += 1;
                }
                let (accent, background) = callout_colors(callout_variant, theme);
                row_starts.push(index);
                rows.push(
                    div()
                        .w(px(centered_width))
                        .max_w(relative(1.0))
                        .flex_shrink_0()
                        .mt(px(top_gap))
                        .flex()
                        .flex_col()
                        .px(px(d.callout_padding_x))
                        .py(px(d.callout_padding_y))
                        .rounded(px(d.callout_radius))
                        .border_l(px(d.callout_border_width))
                        .border_color(accent)
                        .bg(background)
                        .children(children)
                        .into_any_element(),
                );
                gap_state.finish_group(callout_gaps.last_content());
                index = end;
                continue;
            }

            if let Some(footnote_anchor) = first_spacing.footnote_anchor {
                let mut children = Vec::new();
                let mut end = index;
                let mut footnote_gaps = RenderedRowGapState::default();
                while end < visible.len()
                    && spacing_for(end).footnote_anchor == Some(footnote_anchor)
                {
                    let spacing = spacing_for(end);
                    if let Some(gap) = footnote_gaps.footnote_gap(spacing, d.block_gap) {
                        children.push(
                            div()
                                .w_full()
                                .flex_shrink_0()
                                .mt(px(gap))
                                .child(visible[end].entity.clone())
                                .into_any_element(),
                        );
                    }
                    end += 1;
                }
                row_starts.push(index);
                rows.push(
                    div()
                        .w(px(centered_width))
                        .max_w(relative(1.0))
                        .flex_shrink_0()
                        .mt(px(top_gap))
                        .child(footnote_group_shell(children, theme, d))
                        .into_any_element(),
                );
                gap_state.finish_group(footnote_gaps.last_content());
                index = end;
                continue;
            }

            row_starts.push(index);
            rows.push(
                div()
                    .w(px(centered_width))
                    .max_w(relative(1.0))
                    .flex_shrink_0()
                    .mt(px(top_gap))
                    .child(visible[index].entity.clone())
                    .into_any_element(),
            );
            index += 1;
        }

        if let Some((top_spacer, bottom_spacer)) = virtual_spacers {
            let mut mounted = Vec::with_capacity(rows.len() + 2);
            if top_spacer > 0.5 {
                mounted.push(
                    div()
                        .w_full()
                        .flex_shrink_0()
                        .h(px(top_spacer))
                        .into_any_element(),
                );
            }
            mounted.extend(rows);
            if bottom_spacer > 0.5 {
                mounted.push(
                    div()
                        .w_full()
                        .flex_shrink_0()
                        .h(px(bottom_spacer))
                        .into_any_element(),
                );
            }
            let pane = div()
                .id("split-preview-pane")
                .debug_selector(|| "split-preview-pane".to_owned())
                .h_full()
                .flex_1()
                .min_w(px(0.0))
                .relative()
                .bg(theme.colors.editor_background)
                .child(
                    div()
                        .id("split-preview-scroll")
                        .debug_selector(|| "split-preview-scroll".to_owned())
                        .h_full()
                        .w_full()
                        .overflow_y_scroll()
                        .scrollbar_width(px(0.0))
                        .track_scroll(&scroll_handle)
                        .on_scroll_wheel(cx.listener(Self::on_split_preview_scroll_wheel))
                        .capture_any_mouse_down(
                            cx.listener(Self::on_split_preview_capture_mouse_down),
                        )
                        .on_mouse_down(
                            MouseButton::Right,
                            cx.listener(|editor, event, window, cx| {
                                editor.on_text_context_menu_mouse_down_for_surface(
                                    crate::editor::selection_surface::SelectionSurface::SplitPreview,
                                    event,
                                    window,
                                    cx,
                                );
                            }),
                        )
                        .on_mouse_move(cx.listener(Self::on_split_preview_mouse_move))
                        .on_mouse_up(
                            MouseButton::Left,
                            cx.listener(Self::on_split_preview_mouse_up),
                        )
                        .on_mouse_up_out(
                            MouseButton::Left,
                            cx.listener(Self::on_split_preview_mouse_up),
                        )
                        .px(px(d.editor_padding))
                        .pt(px(top_padding))
                        .pb(px(bottom_padding))
                        .flex()
                        .flex_col()
                        .items_center()
                        .child(
                            div()
                                .id("split-preview-content")
                                .debug_selector(|| "split-preview-content".to_owned())
                                .w_full()
                                .flex()
                                .flex_col()
                                .items_center()
                                .children(mounted),
                        ),
                );
            let pane = if show_scrollbar {
                pane.child(self.render_split_preview_scrollbar(
                    theme,
                    scrollbar_geometry,
                    f32::from(scroll_handle.bounds().origin.y),
                    cx,
                ))
            } else {
                pane
            };
            let painted_entities = if matches!(
                self.split_preview
                    .as_ref()
                    .and_then(|state| state.pending_drag_scroll),
                Some(
                    PendingSplitDragScroll::Paint(crate::editor::SplitScrollDriver::Preview)
                        | PendingSplitDragScroll::AlignSource(_)
                )
            ) {
                visible.iter().map(|block| block.entity.clone()).collect()
            } else {
                Vec::new()
            };
            let pane = if let Some(canvas) = self.split_drag_sync_canvas(painted_entities, cx) {
                pane.child(canvas)
            } else {
                pane
            };
            return Some(pane.into_any_element());
        }

        let row_ids: Vec<EntityId> = row_starts
            .iter()
            .map(|&start| visible[start].entity.entity_id())
            .collect();
        let visible_ids: Vec<EntityId> = visible
            .iter()
            .map(|block| block.entity.entity_id())
            .collect();
        self.sync_row_stride_layout_identity(
            crate::editor::selection_surface::SelectionSurface::SplitPreview,
            cx.global::<crate::theme::ThemeManager>().current_arc(),
            crate::config::EditorSettings::editor_font_family(cx),
            centered_width,
        );
        let state = self
            .split_preview
            .as_mut()
            .expect("Split 模式必须持有右侧预览投影");
        if visible_ids != state.previous_visible_ids {
            state.previous_visible_ids = visible_ids;
            state.row_stride_cache.clear();
            state.previous_render_window = None;
        }
        if state.row_stride_cache.len() > row_ids.len().saturating_mul(2) {
            let live: std::collections::HashSet<EntityId> = row_ids.iter().copied().collect();
            state.row_stride_cache.retain(|id, _| live.contains(id));
        }
        let estimate = d.block_min_height.max(1.0);
        let strides: Vec<f32> = row_ids
            .iter()
            .map(|id| state.row_stride_cache.get(id).copied().unwrap_or(estimate))
            .collect();
        if let Some(PendingSplitDragScroll::SourceAnchor(source_offset)) = state.pending_drag_scroll
        {
            let target_row = row_starts
                .iter()
                .enumerate()
                .filter_map(|(row, &start)| {
                    let end = row_starts.get(row + 1).copied().unwrap_or(visible.len());
                    visible[start..end]
                        .iter()
                        .filter_map(|block| {
                            let range = state.source_ranges.get(&block.entity.entity_id())?;
                            let distance = if source_offset < range.start {
                                range.start - source_offset
                            } else {
                                source_offset.saturating_sub(range.end)
                            };
                            Some((distance, range.len(), row))
                        })
                        .min()
                })
                .min()
                .map(|(_, _, row)| row);
            if let Some(row) = target_row {
                let target = top_padding
                    + strides[..row]
                        .iter()
                        .map(|stride| stride.max(0.0))
                        .sum::<f32>();
                let mut offset = scroll_handle.offset();
                offset.y = -px(target.min(max_scroll_y));
                scroll_handle.set_offset(offset);
                current_scroll_y = target.min(max_scroll_y);
            }
            state.pending_drag_scroll = Some(PendingSplitDragScroll::AlignSource(source_offset));
        }
        let scrollbar_geometry =
            Self::scrollbar_geometry(viewport_height, max_scroll_y, current_scroll_y);
        let measurement_frontier = row_ids
            .iter()
            .position(|id| !state.row_stride_cache.contains_key(id))
            .unwrap_or(row_ids.len());
        let restoring_deep_offset = state.previous_render_window.is_none();
        let local_scroll_y = (current_scroll_y - top_padding).max(0.0);
        let render_window = Self::rendered_window(
            &strides,
            local_scroll_y,
            viewport_height,
            RENDER_OVERDRAW_PX,
            None,
        );
        let render_window = Self::recover_render_window_to_measurement_frontier(
            render_window,
            &strides,
            local_scroll_y,
            measurement_frontier,
            restoring_deep_offset,
        );
        state.previous_render_window = Some((render_window.run_start, render_window.run_end));
        let painted_start = row_starts
            .get(render_window.run_start)
            .copied()
            .unwrap_or(visible.len());
        let painted_end = row_starts
            .get(render_window.run_end)
            .copied()
            .unwrap_or(visible.len());
        let painted_entities = if matches!(
            state.pending_drag_scroll,
            Some(
                PendingSplitDragScroll::Paint(crate::editor::SplitScrollDriver::Preview)
                    | PendingSplitDragScroll::AlignSource(_)
            )
        ) {
            visible[painted_start..painted_end]
                .iter()
                .map(|block| block.entity.clone())
                .collect()
        } else {
            Vec::new()
        };

        let top_height = render_window.top_h;
        let mut mounted = Vec::with_capacity(
            render_window
                .run_end
                .saturating_sub(render_window.run_start)
                + 2,
        );
        if top_height > 0.5 {
            mounted.push(
                div()
                    .w_full()
                    .flex_shrink_0()
                    .h(px(top_height))
                    .into_any_element(),
            );
        }
        let editor = cx.entity().downgrade();
        mounted.extend(rows.into_iter().enumerate().filter_map(|(row, element)| {
            (row >= render_window.run_start && row < render_window.run_end).then(|| {
                Self::render_measured_document_row(
                    editor.clone(),
                    row_ids[row],
                    crate::editor::selection_surface::SelectionSurface::SplitPreview,
                    element,
                )
            })
        }));
        if render_window.bottom_h > 0.5 {
            mounted.push(
                div()
                    .w_full()
                    .flex_shrink_0()
                    .h(px(render_window.bottom_h))
                    .into_any_element(),
            );
        }

        let pane = div()
            .id("split-preview-pane")
            .debug_selector(|| "split-preview-pane".to_owned())
            .h_full()
            .flex_1()
            .min_w(px(0.0))
            .relative()
            .bg(theme.colors.editor_background)
            .child(
                div()
                    .id("split-preview-scroll")
                    .debug_selector(|| "split-preview-scroll".to_owned())
                    .h_full()
                    .w_full()
                    .overflow_y_scroll()
                    .scrollbar_width(px(0.0))
                    .track_scroll(&scroll_handle)
                    .on_scroll_wheel(cx.listener(Self::on_split_preview_scroll_wheel))
                    .capture_any_mouse_down(cx.listener(Self::on_split_preview_capture_mouse_down))
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(|editor, event, window, cx| {
                            editor.on_text_context_menu_mouse_down_for_surface(
                                crate::editor::selection_surface::SelectionSurface::SplitPreview,
                                event,
                                window,
                                cx,
                            );
                        }),
                    )
                    .on_mouse_move(cx.listener(Self::on_split_preview_mouse_move))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(Self::on_split_preview_mouse_up),
                    )
                    .on_mouse_up_out(
                        MouseButton::Left,
                        cx.listener(Self::on_split_preview_mouse_up),
                    )
                    .px(px(d.editor_padding))
                    .pt(px(top_padding))
                    .pb(px(bottom_padding))
                    .flex()
                    .flex_col()
                    .items_center()
                    .child(
                        div()
                            .id("split-preview-content")
                            .debug_selector(|| "split-preview-content".to_owned())
                            .w_full()
                            .flex()
                            .flex_col()
                            .items_center()
                            .children(mounted),
                    ),
            );
        let pane = if show_scrollbar {
            pane.child(self.render_split_preview_scrollbar(
                theme,
                scrollbar_geometry,
                f32::from(scroll_handle.bounds().origin.y),
                cx,
            ))
        } else {
            pane
        };
        let pane = if let Some(canvas) = self.split_drag_sync_canvas(painted_entities, cx) {
            pane.child(canvas)
        } else {
            pane
        };
        Some(pane.into_any_element())
    }
}
