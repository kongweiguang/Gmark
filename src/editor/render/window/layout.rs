// @author kongweiguang

use super::*;

impl Editor {
    /// 排版身份变化时丢弃对应表面的旧行高，并重置窗口标记以启用深滚测量前沿。
    pub(in crate::editor) fn sync_row_stride_layout_identity(
        &mut self,
        surface: crate::editor::selection_surface::SelectionSurface,
        theme: std::sync::Arc<crate::theme::Theme>,
        font_family: String,
        content_width: f32,
    ) {
        let (identity, cache, previous_window) = match surface {
            crate::editor::selection_surface::SelectionSurface::Main => (
                &mut self.row_stride_layout_identity,
                &mut self.row_stride_cache,
                &mut self.prev_render_window,
            ),
            crate::editor::selection_surface::SelectionSurface::SplitPreview => {
                let Some(state) = self.split_preview.as_mut() else {
                    self.split_row_stride_layout_identity = Some(RowStrideLayoutIdentity::new(
                        theme,
                        font_family,
                        content_width,
                    ));
                    return;
                };
                (
                    &mut self.split_row_stride_layout_identity,
                    &mut state.row_stride_cache,
                    &mut state.previous_render_window,
                )
            }
        };
        if identity
            .as_ref()
            .is_some_and(|previous| previous.matches(&theme, &font_family, content_width))
        {
            return;
        }
        *identity = Some(RowStrideLayoutIdentity::new(
            theme,
            font_family,
            content_width,
        ));
        cache.clear();
        *previous_window = None;
    }

    /// 直接记录完整行的布局高度，包含分组、表格和 margin；文本 bounds 可能来自不同
    /// 滚动帧，不能通过相邻坐标相减学习行高。只有测量变化时才请求下一帧收敛裁剪区。
    pub(in crate::editor) fn render_measured_document_row(
        editor: WeakEntity<Self>,
        row_id: EntityId,
        surface: crate::editor::selection_surface::SelectionSurface,
        content: AnyElement,
    ) -> AnyElement {
        div()
            .relative()
            .w_full()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .items_center()
            .child(content)
            .child(
                canvas(
                    move |bounds, _, cx| {
                        let height = f32::from(bounds.size.height);
                        if height.is_finite() && height >= 0.0 {
                            let _ = editor.update(cx, |editor, cx| {
                                let cache = match surface {
                                    crate::editor::selection_surface::SelectionSurface::Main => &mut editor.row_stride_cache,
                                    crate::editor::selection_surface::SelectionSurface::SplitPreview => {
                                        let Some(state) = editor.split_preview.as_mut() else { return; };
                                        &mut state.row_stride_cache
                                    }
                                };
                                let previous = cache.insert(row_id, height);
                                if previous.is_none_or(|previous| (previous - height).abs() > 0.5) {
                                    cx.notify();
                                }
                            });
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .into_any_element()
    }

    pub(in crate::editor) fn scrollbar_geometry(
        viewport_height: f32,
        max_scroll_y: f32,
        current_scroll_y: f32,
    ) -> ScrollbarGeometry {
        let track_height = viewport_height.max(20.0);
        let content_height = viewport_height + max_scroll_y;
        let thumb_height = if max_scroll_y > 0.5 {
            (track_height * (viewport_height / content_height)).clamp(28.0, track_height)
        } else {
            track_height
        };
        let progress = if max_scroll_y > 0.0 {
            current_scroll_y.clamp(0.0, max_scroll_y) / max_scroll_y
        } else {
            0.0
        };
        let thumb_top = (track_height - thumb_height).max(0.0) * progress;
        ScrollbarGeometry {
            track_height,
            thumb_height,
            thumb_top,
            max_scroll_y,
        }
    }

    pub(in crate::editor) fn scroll_offset_for_thumb_top(
        thumb_top: f32,
        track_height: f32,
        thumb_height: f32,
        max_scroll_y: f32,
    ) -> f32 {
        if max_scroll_y <= 0.0 {
            return 0.0;
        }

        let travel = (track_height - thumb_height).max(0.0);
        if travel <= 0.0 {
            return 0.0;
        }

        let progress = (thumb_top / travel).clamp(0.0, 1.0);
        max_scroll_y * progress
    }

    /// Picks the contiguous run of rows to mount; the culled runs become two
    /// spacers and the focused row stays mounted. `strides[i]` is row `i`'s
    /// measured wrapper footprint, including its leading margin; their running
    /// sum places each row against a band from the current scroll offset.
    /// Unmeasured rows use a lower-bound estimate. A restored offset beyond that
    /// estimate extends to the first unknown row; ordinary scrolling stays bounded.
    /// Pure, so it is unit-tested headlessly.
    pub(in crate::editor) fn rendered_window(
        strides: &[f32],
        scroll_y: f32,
        viewport_height: f32,
        overdraw: f32,
        focus_row: Option<usize>,
    ) -> RenderWindow {
        let n = strides.len();
        if n == 0 {
            return RenderWindow {
                run_start: 0,
                run_end: 0,
                top_h: 0.0,
                bottom_h: 0.0,
            };
        }

        let band_top = scroll_y - overdraw;
        let band_bottom = scroll_y + viewport_height + overdraw;

        let mut run_start = n;
        let mut run_end = 0usize;
        let mut top_of_start = 0.0f32;
        let mut bottom_of_end = 0.0f32;
        let mut cursor = 0.0f32;
        for (index, &stride) in strides.iter().enumerate() {
            let top = cursor;
            let bottom = cursor + stride.max(0.0);
            if bottom >= band_top && top <= band_bottom {
                if index < run_start {
                    run_start = index;
                    top_of_start = top;
                }
                run_end = index + 1;
                bottom_of_end = bottom;
            }
            cursor = bottom;
        }
        let total = cursor;

        // Nothing hit the band (float edge, or estimate short of scroll): mount
        // the last row so the viewport never lands on a spacer.
        if run_start >= run_end {
            run_start = n - 1;
            run_end = n;
            top_of_start = total - strides[n - 1].max(0.0);
            bottom_of_end = total;
        }

        // Keep the focused row mounted; GPUI blurs an unmounted caret. Reaching a
        // far focus row widens the run, but autoscroll makes that rare.
        if let Some(focus_row) = focus_row {
            let focus_row = focus_row.min(n - 1);
            if focus_row < run_start {
                run_start = focus_row;
                top_of_start = strides[..focus_row].iter().map(|s| s.max(0.0)).sum();
            }
            if focus_row + 1 > run_end {
                run_end = focus_row + 1;
                bottom_of_end = strides[..=focus_row].iter().map(|s| s.max(0.0)).sum();
            }
        }

        RenderWindow {
            run_start,
            run_end,
            top_h: top_of_start.max(0.0),
            bottom_h: (total - bottom_of_end).max(0.0),
        }
    }

    /// 未测量行的最小高度只适合裁剪已知前缀；恢复到较深滚动位置时，必须从首个
    /// 未测量行连续挂载到目标窗口，避免低估总高后只渲染末行并留下整屏空白。
    pub(in crate::editor) fn include_render_measurement_frontier(
        mut window: RenderWindow,
        strides: &[f32],
        measurement_frontier: usize,
    ) -> RenderWindow {
        let frontier = measurement_frontier.min(strides.len());
        if frontier < window.run_start {
            window.run_start = frontier;
            window.top_h = strides[..frontier]
                .iter()
                .map(|stride| stride.max(0.0))
                .sum();
        }
        window
    }

    /// 失效缓存恢复到超出最小高度估算的深位置时，从首个未知行连续挂载到目标窗口。
    pub(in crate::editor) fn recover_render_window_to_measurement_frontier(
        window: RenderWindow,
        strides: &[f32],
        scroll_y: f32,
        measurement_frontier: usize,
        restoring_deep_offset: bool,
    ) -> RenderWindow {
        let estimated_total = strides.iter().map(|stride| stride.max(0.0)).sum::<f32>();
        if restoring_deep_offset && scroll_y > estimated_total {
            Self::include_render_measurement_frontier(window, strides, measurement_frontier)
        } else {
            window
        }
    }

    /// 小文档完整挂载可避免滚动与行高学习之间的空白帧；超过阈值后才启用裁剪。
    /// 只有恢复偏移已经落在估算总高之外时才扩展到测量前沿；普通深滚动必须保持
    /// 有界窗口，否则一个未测量的首行会让数百行被同时挂载。
    pub(in crate::editor) fn rendered_document_window(
        strides: &[f32],
        scroll_y: f32,
        viewport_height: f32,
        overdraw: f32,
        measurement_frontier: usize,
        restoring_deep_offset: bool,
        virtualization_threshold: usize,
    ) -> RenderWindow {
        if strides.len() < virtualization_threshold {
            return RenderWindow {
                run_start: 0,
                run_end: strides.len(),
                top_h: 0.0,
                bottom_h: 0.0,
            };
        }
        let window = Self::rendered_window(strides, scroll_y, viewport_height, overdraw, None);
        Self::recover_render_window_to_measurement_frontier(
            window,
            strides,
            scroll_y,
            measurement_frontier,
            restoring_deep_offset,
        )
    }
}
