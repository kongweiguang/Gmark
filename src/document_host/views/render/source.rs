// @author kongweiguang

//! Source virtual-list and scrollbar rendering for the document host.

use super::*;
use crate::theme::workbench::SurfaceKind;
use crate::ui::visual_preferences::VisualPreferencesManager;

#[derive(Clone, Copy)]
pub(super) struct SourceSurfaceMetrics {
    pub(super) horizontal_padding: f32,
    pub(super) top_padding: f32,
    pub(super) row_height: f32,
    pub(super) gutter_width: f32,
    fold_lane_width: f32,
    number_width: f32,
    content_width: f32,
    observed_line_bytes: u64,
}

impl DocumentHost {
    /// Keeps the active native IME row mounted while refreshing Source viewport and layout metrics.
    pub(super) fn prepare_source_surface(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> SourceSurfaceMetrics {
        self.maybe_schedule_fold_refresh(cx);
        window.set_window_edited(document_dirty_state(&self.document));
        let line_count = self.line_count();
        let folding_enabled = crate::preferences::EditorSettings::code_folding(cx);
        if self.folding_enabled != folding_enabled {
            self.folding_enabled = folding_enabled;
            if !folding_enabled {
                self.fold_projection.expand_all();
                self.clear_source_row_blocks_except_ime_owner();
            }
        }
        self.fold_projection.set_real_line_count(line_count);
        self.source_list_origin = self.source_list_origin.min(
            self.fold_projection
                .visible_line_count()
                .saturating_sub(SOURCE_LIST_WINDOW_ROWS),
        );
        let observed_line_bytes = self
            .index
            .as_ref()
            .map_or(0, LineIndex::max_line_bytes)
            .max(
                self.preview_lines
                    .iter()
                    .map(|line| line.len() as u64)
                    .max()
                    .unwrap_or(0),
            )
            .max(
                self.active_edit
                    .as_ref()
                    .map(|edit| edit.block.read(cx).display_text().len() as u64)
                    .unwrap_or(0),
            );
        let theme = cx.global::<ThemeManager>().current_arc();
        let dimensions = &theme.dimensions;
        let viewport_width = f32::from(window.viewport_size().width).max(1.0);
        let (horizontal_padding, top_padding) = source_surface_padding(dimensions);
        let content_width = (viewport_width - 2.0 * horizontal_padding).max(1.0);
        let row_height =
            (theme.typography.text_line_height * f32::from(window.rem_size())).max(1.0);
        self.source_row_height = row_height;
        let number_width = f32::from(source_line_number_gutter_width(
            line_count,
            px(theme.typography.text_size),
        ));
        let fold_lane_width = if self.folding_enabled && self.source_language.supports_folding() {
            20.0
        } else {
            0.0
        };

        SourceSurfaceMetrics {
            horizontal_padding,
            top_padding,
            row_height,
            gutter_width: number_width + fold_lane_width,
            fold_lane_width,
            number_width,
            content_width,
            observed_line_bytes,
        }
    }

    pub(super) fn render_source_list(
        &mut self,
        surface: SourceSurfaceMetrics,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let theme = cx.global::<ThemeManager>().current_arc();
        let colors = &theme.colors;
        let dimensions = &theme.dimensions;
        let source_text_size = theme.typography.text_size;
        let source_line_height = theme.typography.text_line_height;
        let visual_preferences = cx
            .try_global::<VisualPreferencesManager>()
            .map(VisualPreferencesManager::current)
            .unwrap_or_default();
        let source_material = colors
            .workbench
            .material(SurfaceKind::Editor, visual_preferences);
        let line_text_color = colors.text_default;
        let line_number_color = colors.text_placeholder;
        let gutter_separator_color = source_material.border.opacity(0.7);
        let active_line_color = colors.source_mode_block_bg.opacity(0.55);
        let source_background = source_material.background;
        let fold_placeholder_accent_color = colors.code_syntax_property;
        let fold_placeholder_punctuation_color = colors.code_syntax_punctuation;
        let fold_placeholder_background = fold_placeholder_accent_color.opacity(0.12);

        uniform_list(
            "document-host-lines",
            self.source_list_len(),
            cx.processor(
                move |this, local_range: std::ops::Range<usize>, _window, _cx| {
                    // keyed uniform_list 可跨 render 复用 processor；全局 origin 必须在
                    // 调用时读取，不能捕获创建该 element 时的旧窗口。
                    let source_list_origin = this.source_list_origin;
                    let visible_range = source_list_origin.saturating_add(local_range.start)
                        ..source_list_origin.saturating_add(local_range.end);
                    let real_lines = visible_range
                        .clone()
                        .map(|line| this.fold_projection.real_line_for_visible(line))
                        .collect::<Vec<_>>();
                    let requested_visible = real_lines.first().copied().unwrap_or_default()
                        ..real_lines
                            .last()
                            .copied()
                            .unwrap_or_default()
                            .saturating_add(1);
                    this.request_source_rows(requested_visible.clone(), _cx);
                    let first_requested = requested_visible.start;
                    let retain_previous_frame = this.fold_projection.visible_line_count()
                        == this.line_count()
                        && this
                            .displayed_screen_lines
                            .should_retain_previous_frame(&requested_visible);
                    let retained_rows = retain_previous_frame
                        .then(|| {
                            this.displayed_screen_lines
                                .retained_rows(this.show_line_endings)
                        })
                        .unwrap_or_default();
                    real_lines
                        .into_iter()
                        .map(|line| {
                            let exact_row = this.displayed_screen_lines.row(line).map(|row| {
                                (
                                    row.leading_truncated,
                                    row.trailing_truncated,
                                    (!row.trailing_truncated && this.show_line_endings)
                                        .then(|| rendered_line_ending(&row.ending))
                                        .filter(|marker| !marker.is_empty()),
                                    row.rendered(this.show_line_endings),
                                )
                            });
                            let retained_row = exact_row
                                .is_none()
                                .then(|| {
                                    let ordinal = line.checked_sub(requested_visible.start)?;
                                    retained_rows.get(ordinal).cloned()
                                })
                                .flatten();
                            let retained_old_frame = retained_row.is_some();
                            let display_line = retained_row
                                .as_ref()
                                .map_or(line, |(display_line, _)| *display_line);
                            let source_block = (!retained_old_frame)
                                .then(|| this.ensure_source_row_block(line, _cx))
                                .flatten();
                            let active_line = this
                                .active_edit
                                .as_ref()
                                .is_some_and(|edit| edit.line == line);
                            let fold_region =
                                this.fold_projection.region_starting(line).map(|region| {
                                    (
                                        region.end_line,
                                        this.fold_projection.is_collapsed(region.id),
                                    )
                                });
                            let fold_placeholder = this.source_fold_placeholder(line);
                            let gutter = DocumentHost::render_source_gutter(
                                line,
                                display_line,
                                fold_region.map(|region| region.0),
                                fold_region.is_some_and(|region| region.1),
                                surface.gutter_width,
                                surface.fold_lane_width,
                                surface.number_width,
                                line_number_color,
                                gutter_separator_color,
                                active_line_color,
                                _cx,
                            );
                            let fold_placeholder = fold_placeholder.map(|label| {
                                DocumentHost::render_source_fold_placeholder(
                                    label,
                                    fold_placeholder_background,
                                    fold_placeholder_accent_color,
                                    fold_placeholder_punctuation_color,
                                    line_number_color,
                                )
                            });
                            div()
                                .id(("document-host-line", line))
                                .h(px(surface.row_height))
                                .min_w_full()
                                .flex()
                                .items_center()
                                .text_size(px(source_text_size))
                                .line_height(rems(source_line_height))
                                .text_color(line_text_color)
                                .bg(if active_line {
                                    active_line_color
                                } else {
                                    source_background
                                })
                                .child(gutter)
                                .child({
                                    let mut body = div()
                                        .debug_selector(move || {
                                            format!("document-host-line-body-{line}")
                                        })
                                        .flex_1()
                                        .min_w(px(0.0))
                                        .h_full()
                                        .overflow_hidden();
                                    if let Some(block) = source_block {
                                        let (
                                            leading_truncated,
                                            trailing_truncated,
                                            ending_marker,
                                            _,
                                        ) = exact_row
                                            .as_ref()
                                            .expect("source block requires exact row");
                                        body = body.child(
                                            div()
                                                .size_full()
                                                .flex()
                                                .items_center()
                                                .whitespace_nowrap()
                                                .text_color(line_text_color)
                                                .children((*leading_truncated).then_some("… "))
                                                .child(
                                                    div()
                                                        .h_full()
                                                        .flex_1()
                                                        .min_w(px(0.0))
                                                        .overflow_hidden()
                                                        .child(block),
                                                )
                                                .children((*trailing_truncated).then_some(" …"))
                                                .children(*ending_marker)
                                                .children(fold_placeholder),
                                        );
                                    } else {
                                        let display = retained_row
                                            .as_ref()
                                            .map(|(_, display)| display.clone())
                                            .or_else(|| {
                                                exact_row
                                                    .as_ref()
                                                    .map(|(_, _, _, display)| display.clone())
                                            })
                                            .unwrap_or_else(|| this.line_text(line));
                                        body = body.whitespace_nowrap().child(display);
                                    }
                                    if retained_old_frame && line == first_requested {
                                        body = body.relative().child(
                                            div()
                                                .debug_selector(|| {
                                                    "document-host-retained-frame-progress"
                                                        .to_owned()
                                                })
                                                .absolute()
                                                .top_0()
                                                .right(px(8.0))
                                                .text_color(line_number_color)
                                                .child(
                                                    _cx.global::<I18nManager>()
                                                        .strings()
                                                        .large_document_text(
                                                            "loading_next_viewport",
                                                        )
                                                        .to_owned(),
                                                ),
                                        );
                                    }
                                    body
                                })
                                .when(!retained_old_frame, |row| {
                                    row.on_mouse_down(
                                        MouseButton::Left,
                                        _cx.listener(move |this, event, window, cx| {
                                            this.select_or_edit_line(line, event, window, cx);
                                        }),
                                    )
                                })
                                .into_any_element()
                        })
                        .collect::<Vec<_>>()
                },
            ),
        )
        .track_scroll(self.scroll_handle.clone())
        .font_family(source_monospace_font_family())
        .h_full()
        .w(px(surface.content_width))
        .max_w(relative(1.0))
        .px(px(dimensions.block_padding_x))
        .bg(source_material.background)
    }

    pub(super) fn render_source_scrollbar(
        &mut self,
        surface: SourceSurfaceMetrics,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement + use<>> {
        let colors = &cx.global::<ThemeManager>().current_arc().colors;
        let visual_preferences = cx
            .try_global::<VisualPreferencesManager>()
            .map(VisualPreferencesManager::current)
            .unwrap_or_default();
        let scrollbar_material = colors
            .workbench
            .material(SurfaceKind::Glass, visual_preferences);
        let source_scroll = self.scroll_handle.0.borrow().base_handle.clone();
        let source_scroll_bounds = source_scroll.bounds();
        let source_viewport_height = f32::from(source_scroll_bounds.size.height.max(px(1.0)));
        let source_visible_rows = (source_viewport_height / surface.row_height)
            .ceil()
            .max(1.0) as usize;
        let source_local_top = (-f32::from(source_scroll.offset().y) / surface.row_height)
            .max(0.0)
            .floor() as usize;
        let visible_line_count = self.fold_projection.visible_line_count().max(1);
        let source_global_top = self
            .source_list_origin
            .saturating_add(source_local_top)
            .min(visible_line_count.saturating_sub(1));
        let source_max_top_line = visible_line_count.saturating_sub(source_visible_rows);
        let source_thumb_height = if source_max_top_line > 0 {
            (source_viewport_height * source_visible_rows as f32 / visible_line_count as f32)
                .clamp(28.0_f32.min(source_viewport_height), source_viewport_height)
        } else {
            source_viewport_height
        };
        let source_thumb_top = if source_max_top_line > 0 {
            (source_viewport_height - source_thumb_height)
                * (source_global_top as f64 / source_max_top_line as f64) as f32
        } else {
            0.0
        };

        (source_max_top_line > 0).then(|| {
            let track_top = source_scroll_bounds.top();
            div()
                .id("document-host-scrollbar")
                .debug_selector(|| "document-host-scrollbar".to_owned())
                .absolute()
                .top_0()
                .bottom_0()
                .right(px(3.0))
                .w(px(12.0))
                .cursor_pointer()
                .bg(scrollbar_material.background.opacity(0.28))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, event: &gpui::MouseDownEvent, window, _cx| {
                        let visible_line = source_line_from_scrollbar_pointer(
                            event.position.y,
                            track_top,
                            source_viewport_height,
                            source_thumb_height,
                            source_max_top_line,
                        );
                        let line = this.fold_projection.real_line_for_visible(visible_line);
                        this.scroll_source_line_strict(line, ScrollStrategy::Top);
                        window.refresh();
                    }),
                )
                .on_mouse_move(cx.listener(
                    move |this, event: &gpui::MouseMoveEvent, window, _cx| {
                        if event.dragging() {
                            let visible_line = source_line_from_scrollbar_pointer(
                                event.position.y,
                                track_top,
                                source_viewport_height,
                                source_thumb_height,
                                source_max_top_line,
                            );
                            let line = this.fold_projection.real_line_for_visible(visible_line);
                            this.scroll_source_line_strict(line, ScrollStrategy::Top);
                            window.refresh();
                        }
                    },
                ))
                .child(
                    div()
                        .absolute()
                        .top(px(source_thumb_top))
                        .right(px(2.0))
                        .w(px(7.0))
                        .h(px(source_thumb_height))
                        .rounded(px(999.0))
                        .bg(colors.workbench.icon.opacity(0.62)),
                )
        })
    }

    pub(super) fn render_source_horizontal_scrollbar(
        &mut self,
        surface: SourceSurfaceMetrics,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement + use<>> {
        let theme = cx.global::<ThemeManager>().current_arc();
        let colors = &theme.colors;
        let dimensions = &theme.dimensions;
        let visual_preferences = cx
            .try_global::<VisualPreferencesManager>()
            .map(VisualPreferencesManager::current)
            .unwrap_or_default();
        let scrollbar_material = colors
            .workbench
            .material(SurfaceKind::Glass, visual_preferences);
        let source_scroll = self.scroll_handle.0.borrow().base_handle.clone();
        let source_scroll_bounds = source_scroll.bounds();
        let source_max_window_start = surface
            .observed_line_bytes
            .saturating_sub(MAX_RENDERED_LINE_BYTES);
        self.source_window_start = self.source_window_start.min(source_max_window_start);
        let source_horizontal_track_left = source_scroll_bounds.left()
            + px(dimensions.block_padding_x + surface.gutter_width + 2.0);
        let source_horizontal_track_width =
            f32::from((source_scroll_bounds.size.width - px(96.0)).max(px(40.0)));
        let source_horizontal_thumb_width = if source_max_window_start > 0 {
            (source_horizontal_track_width * MAX_RENDERED_LINE_BYTES as f32
                / surface.observed_line_bytes.max(1) as f32)
                .clamp(28.0, source_horizontal_track_width)
        } else {
            source_horizontal_track_width
        };
        let source_horizontal_thumb_left = if source_max_window_start > 0 {
            (source_horizontal_track_width - source_horizontal_thumb_width)
                * (self.source_window_start as f64 / source_max_window_start as f64) as f32
        } else {
            0.0
        };

        (source_max_window_start > 0).then(|| {
            div()
                .id("document-host-horizontal-scrollbar")
                .debug_selector(|| "document-host-horizontal-scrollbar".to_owned())
                .absolute()
                .left(px(dimensions.block_padding_x + surface.gutter_width + 2.0))
                .right(px(18.0))
                .bottom(px(2.0))
                .h(px(12.0))
                .cursor_pointer()
                .bg(scrollbar_material.background.opacity(0.28))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, event: &gpui::MouseDownEvent, _window, cx| {
                        let next = source_window_start_from_pointer(
                            event.position.x,
                            source_horizontal_track_left,
                            source_horizontal_track_width,
                            source_horizontal_thumb_width,
                            source_max_window_start,
                        );
                        this.set_source_window_start(next, cx);
                    }),
                )
                .on_mouse_move(cx.listener(
                    move |this, event: &gpui::MouseMoveEvent, _window, cx| {
                        if event.dragging() {
                            let next = source_window_start_from_pointer(
                                event.position.x,
                                source_horizontal_track_left,
                                source_horizontal_track_width,
                                source_horizontal_thumb_width,
                                source_max_window_start,
                            );
                            this.set_source_window_start(next, cx);
                        }
                    },
                ))
                .child(
                    div()
                        .absolute()
                        .left(px(source_horizontal_thumb_left))
                        .bottom(px(2.0))
                        .w(px(source_horizontal_thumb_width))
                        .h(px(7.0))
                        .rounded(px(999.0))
                        .bg(colors.workbench.icon.opacity(0.62)),
                )
        })
    }
}
