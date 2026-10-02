// @author kongweiguang

//! 虚拟视口只提供当前布局；文档边界必须由完整源码决定。

use super::*;

impl Editor {
    /// 先保存源码锚点再挂载目标，避免导航把视口边界当作全文边界或丢失反向选区。
    pub(super) fn move_to_virtual_document_boundary(
        &mut self,
        surface: SelectionSurface,
        focused: &Entity<Block>,
        at_end: bool,
        extend: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if surface != SelectionSurface::Main
            || !matches!(self.view_mode, ViewMode::Rendered | ViewMode::Preview)
            || self.virtual_surface.is_none()
        {
            return false;
        }
        let revision = self.source_document.snapshot().revision();
        let destination = if at_end {
            self.source_document.len()
        } else {
            0
        };
        if self
            .virtual_surface
            .as_ref()
            .is_some_and(|surface| surface.projection_revision() != revision)
        {
            // 仅显式跳转在此同步追上投影；正常键入继续沿用后台增量解析。
            let prepared = self.prepare_current_projection();
            self.install_virtual_surface_projection(prepared, cx);
        }
        let Some(virtual_surface) = self.virtual_surface.as_ref() else {
            return false;
        };
        if virtual_surface.projection_revision() != revision {
            return true;
        }
        let Some(y) = virtual_surface.y_for_source_offset(destination) else {
            return true;
        };
        let previous = self.cross_block_selection_for_surface(surface);
        let anchor = previous
            .map(|selection| selection.anchor)
            .unwrap_or_else(|| {
                let block = focused.read(cx);
                CrossBlockSelectionEndpoint {
                    entity_id: focused.entity_id(),
                    offset: if block.selection_reversed {
                        block.selected_range.end
                    } else {
                        block.selected_range.start
                    },
                }
            });
        let source_anchor = previous
            .and_then(|selection| selection.source_anchor)
            .filter(|anchor| anchor.revision == revision)
            .or_else(|| self.cross_block_source_anchor_for_endpoint(surface, anchor, cx));
        if extend && source_anchor.is_none_or(|anchor| anchor.revision != revision) {
            return true;
        }

        let viewport = f32::from(self.scroll_handle.bounds().size.height).max(1.0);
        let offset = self.scroll_handle.offset();
        self.scroll_handle.set_offset(point(offset.x, px(-y)));
        self.sync_virtual_surface_mounts(y, viewport, 800.0, cx);
        let blocks = self.selection_surface_entities(surface);
        let Some(boundary) = (if at_end {
            blocks.last()
        } else {
            blocks.first()
        })
        .cloned() else {
            return true;
        };
        let boundary_offset = if at_end {
            boundary.read(cx).visible_len()
        } else {
            0
        };
        let focus = CrossBlockSelectionEndpoint {
            entity_id: boundary.entity_id(),
            offset: boundary_offset,
        };
        self.clear_cross_block_selection_for_surface(surface, cx);
        if extend && source_anchor.is_some_and(|anchor| anchor.byte_offset != destination) {
            if anchor.entity_id == focus.entity_id {
                Self::set_local_selection(&boundary, anchor.offset, focus.offset, cx);
            } else {
                Self::set_local_cursor(&boundary, boundary_offset, cx);
            }
            self.set_cross_block_selection_for_surface(
                surface,
                Some(CrossBlockSelection {
                    anchor,
                    focus,
                    source_anchor,
                    source_focus: Some(CrossBlockSourceAnchor {
                        byte_offset: destination,
                        revision,
                    }),
                }),
            );
            self.sync_cross_block_selection_visuals_for_surface(surface, cx);
        } else {
            Self::set_local_cursor(&boundary, boundary_offset, cx);
        }
        self.focus_navigation_target(surface, &boundary, window, cx);
        cx.notify();
        true
    }
}
