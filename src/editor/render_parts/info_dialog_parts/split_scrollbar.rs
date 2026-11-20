// @author kongweiguang

use super::*;

impl Editor {
    /// 绘制完成后只处理本帧挂载的实体，避免旧块几何参与拖动锚点计算。
    pub(super) fn split_drag_sync_canvas(
        &self,
        painted_entities: Vec<Entity<Block>>,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let pending = match self.split_preview.as_ref()?.pending_drag_scroll? {
            pending @ (PendingSplitDragScroll::Paint(_)
            | PendingSplitDragScroll::AlignSource(_)) => pending,
            PendingSplitDragScroll::SourceAnchor(_) => return None,
        };
        let editor = cx.entity().downgrade();
        Some(
            canvas(
                |_, _, _| (),
                move |_, _, window, cx| {
                    let editor = editor.clone();
                    let painted_entities = painted_entities.clone();
                    window.defer(cx, move |_, cx| {
                        let _ = editor.update(cx, |editor, cx| {
                            editor.sync_split_drag_after_paint(pending, &painted_entities, cx);
                        });
                    });
                },
            )
            .absolute()
            .size_full()
            .into_any_element(),
        )
    }

    /// 滑块独立接管拖拽，滚轮继续交由下层预览处理，以保留分屏同步的驱动权。
    pub(super) fn render_split_preview_scrollbar(
        &self,
        theme: &Theme,
        geometry: super::ScrollbarGeometry,
        track_origin_y: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let editor = cx.entity().downgrade();
        let drag_editor = editor.clone();
        let visual_width = if self.split_preview_scrollbar_drag.is_some()
            || self.split_preview_scrollbar_hovered
        {
            EDITOR_SCROLLBAR_HOVER_WIDTH
        } else {
            theme.dimensions.scrollbar_width
        };

        div()
            .id("split-preview-scrollbar-hitbox")
            .debug_selector(|| "split-preview-scrollbar-hitbox".to_owned())
            .absolute()
            .block_mouse_except_scroll()
            .top(px(geometry.thumb_top))
            .right(px(theme.dimensions.scrollbar_right))
            .w(px(EDITOR_SCROLLBAR_HIT_WIDTH))
            .h(px(geometry.thumb_height))
            .cursor_pointer()
            .on_hover(cx.listener(Self::on_split_preview_scrollbar_hover))
            .on_mouse_down(MouseButton::Left, move |event, _window, cx| {
                let pointer_offset_y =
                    f32::from(event.position.y) - track_origin_y - geometry.thumb_top;
                let _ = drag_editor.update(cx, |editor, cx| {
                    cx.stop_propagation();
                    editor.start_split_preview_scrollbar_drag(
                        pointer_offset_y,
                        geometry.track_height,
                        geometry.thumb_height,
                        geometry.max_scroll_y,
                        cx,
                    );
                });
            })
            .child(
                div()
                    .id("split-preview-scrollbar-thumb")
                    .debug_selector(|| "split-preview-scrollbar-thumb".to_owned())
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .right_0()
                    .w(px(visual_width))
                    .rounded(px(999.0))
                    .bg(theme.colors.scrollbar_thumb),
            )
            .child(
                canvas(
                    |_, _, _| (),
                    move |_thumb_bounds, _, window, _| {
                        window.on_mouse_event({
                            let editor = editor.clone();
                            move |_event: &MouseUpEvent, phase, _window, cx| {
                                if !phase.bubble() {
                                    return;
                                }
                                let _ = editor.update(cx, |editor, cx| {
                                    editor.end_split_preview_scrollbar_drag(cx);
                                });
                            }
                        });

                        window.on_mouse_event({
                            let editor = editor.clone();
                            move |event: &MouseMoveEvent, phase, _window, cx| {
                                if !phase.bubble() || !event.dragging() {
                                    return;
                                }
                                let pointer_y_in_track =
                                    f32::from(event.position.y) - track_origin_y;
                                let _ = editor.update(cx, |editor, cx| {
                                    editor.update_split_preview_scrollbar_drag(
                                        pointer_y_in_track,
                                        cx,
                                    );
                                });
                            }
                        });
                    },
                )
                .size_full(),
            )
            .into_any_element()
    }
}
