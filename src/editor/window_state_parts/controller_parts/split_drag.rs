// @author kongweiguang

use super::*;

impl Editor {
    /// 覆盖视口顶边的长块应优先于后续标题；不相交的旧几何不能成为锚点。
    pub(in crate::editor) fn split_drag_viewport_distance(
        bounds: Bounds<Pixels>,
        viewport: Bounds<Pixels>,
    ) -> Option<f32> {
        if bounds.bottom() <= viewport.top() || bounds.top() >= viewport.bottom() {
            return None;
        }
        Some(if bounds.top() <= viewport.top() {
            0.0
        } else {
            f32::from(bounds.top() - viewport.top())
        })
    }

    /// 拖动只消费本帧实际挂载且进入视口的块，避免旧 bounds 将另一侧拉回旧位置。
    pub(in crate::editor) fn sync_split_drag_after_paint(
        &mut self,
        pending: PendingSplitDragScroll,
        painted_entities: &[Entity<Block>],
        cx: &mut Context<Self>,
    ) {
        if self.view_mode != ViewMode::Split
            || self
                .split_preview
                .as_ref()
                .and_then(|state| state.pending_drag_scroll)
                != Some(pending)
        {
            return;
        }
        if let PendingSplitDragScroll::AlignSource(source_offset) = pending {
            let Some(state) = self.split_preview.as_mut() else {
                return;
            };
            let target = painted_entities
                .iter()
                .filter_map(|entity| {
                    let range = state.source_ranges.get(&entity.entity_id())?;
                    let bounds = entity.read(cx).last_bounds?;
                    let distance = if source_offset < range.start {
                        range.start - source_offset
                    } else {
                        source_offset.saturating_sub(range.end)
                    };
                    Some((distance, range.len(), bounds.top()))
                })
                .min_by_key(|(distance, len, _)| (*distance, *len));
            state.pending_drag_scroll = None;
            if let Some((_, _, target_top)) = target {
                let mut offset = state.scroll_handle.offset();
                offset.y = (offset.y + state.scroll_handle.bounds().top() - target_top)
                    .max(-state.scroll_handle.max_offset().height.max(px(0.0)))
                    .min(px(0.0));
                state.scroll_handle.set_offset(offset);
                cx.notify();
            }
            return;
        }
        let PendingSplitDragScroll::Paint(driver) = pending else {
            return;
        };
        let source_offset = match driver {
            SplitScrollDriver::Source => self.document.first_root().and_then(|block| {
                block
                    .read(cx)
                    .text_offset_for_window_y(self.scroll_handle.bounds().top())
            }),
            SplitScrollDriver::Preview => self.split_preview.as_ref().and_then(|state| {
                let viewport = state.scroll_handle.bounds();
                painted_entities
                    .iter()
                    .filter_map(|entity| {
                        let bounds = entity.read(cx).last_bounds?;
                        let distance = Self::split_drag_viewport_distance(bounds, viewport)?;
                        let range = state.source_ranges.get(&entity.entity_id())?;
                        Some((distance, range.len(), range.start))
                    })
                    .min_by(|left, right| {
                        left.0
                            .total_cmp(&right.0)
                            .then_with(|| left.1.cmp(&right.1))
                    })
                    .map(|(_, _, offset)| offset)
            }),
        };
        let Some(state) = self.split_preview.as_mut() else {
            return;
        };
        state.pending_drag_scroll = None;
        let Some(source_offset) = source_offset else {
            if driver == SplitScrollDriver::Preview {
                let preview_max = f32::from(state.scroll_handle.max_offset().height.max(px(0.0)));
                if preview_max > 0.0 {
                    let ratio =
                        (-f32::from(state.scroll_handle.offset().y) / preview_max).clamp(0.0, 1.0);
                    let mut offset = self.scroll_handle.offset();
                    offset.y = -self.scroll_handle.max_offset().height.max(px(0.0)) * ratio;
                    self.scroll_handle.set_offset(offset);
                    cx.notify();
                }
            }
            return;
        };
        match driver {
            SplitScrollDriver::Source => {
                state.pending_drag_scroll =
                    Some(PendingSplitDragScroll::SourceAnchor(source_offset));
                cx.notify();
            }
            SplitScrollDriver::Preview => {
                let source_y = self
                    .document
                    .first_root()
                    .and_then(|block| block.read(cx).window_y_for_text_offset(source_offset));
                if let Some(source_y) = source_y {
                    let next = self.scroll_handle.offset().y + self.scroll_handle.bounds().top()
                        - source_y;
                    let mut offset = self.scroll_handle.offset();
                    offset.y = next
                        .max(-self.scroll_handle.max_offset().height.max(px(0.0)))
                        .min(px(0.0));
                    self.scroll_handle.set_offset(offset);
                    cx.notify();
                }
            }
        }
    }
}
