// @author kongweiguang

use super::*;
use gpui::prelude::FluentBuilder;

impl Editor {
    /// 单窗与子窗格复用双面布局；弹性宽比约束首帧与缩放，测量尺寸只参与内容列和拖动计算。
    pub(super) fn render_resident_surface(
        &mut self,
        content: AnyElement,
        theme: &Theme,
        available_width: f32,
        viewport_height: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if self.view_mode != super::ViewMode::Split {
            return content;
        }
        let ratio = clamped_split_pane_ratio(self.split_pane_ratio, available_width);
        let preview_width = available_width * (1.0 - ratio);
        let preview = self
            .render_split_preview_pane(theme, preview_width, viewport_height, cx)
            .unwrap_or_else(|| div().flex_1().into_any_element());
        let divider_editor = cx.entity().downgrade();
        let divider_focused = self.split_divider_focus_handle.is_focused(window);
        let divider_active = self.split_resize_session.is_some() || divider_focused;
        let divider_focus_handle = self.split_divider_focus_handle.clone();
        let divider_key_editor = cx.entity().downgrade();
        div()
            .w_full()
            .h_full()
            .flex()
            .min_w(px(0.0))
            .child(
                div()
                    .id("split-source-pane-shell")
                    .debug_selector(|| "split-source-pane-shell".to_owned())
                    .h_full()
                    .flex_1()
                    .map(|mut shell| {
                        shell.style().flex_grow = Some(ratio);
                        shell
                    })
                    .min_w(px(0.0))
                    .child(content),
            )
            .child(
                div()
                    .id("split-divider")
                    .debug_selector(|| "split-divider".to_owned())
                    .relative()
                    .h_full()
                    .w(px(SPLIT_DIVIDER_HIT_WIDTH))
                    .flex_none()
                    .tab_index(0)
                    .track_focus(&divider_focus_handle)
                    .cursor_col_resize()
                    .hover(|this| this.bg(theme.colors.workbench.accent_soft))
                    .focus(|this| this.bg(theme.colors.workbench.accent_soft))
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .bottom_0()
                            .left(px((SPLIT_DIVIDER_HIT_WIDTH - 1.0) * 0.5))
                            .w(px(1.0))
                            .bg(if divider_active {
                                theme.colors.workbench.focus_ring
                            } else {
                                theme.colors.workbench.border_subtle
                            })
                            .debug_selector(|| "split-divider-line".to_owned()),
                    )
                    .on_mouse_down(MouseButton::Left, move |event, window, cx| {
                        divider_focus_handle.focus(window);
                        let _ = divider_editor.update(cx, |editor, cx| {
                            if event.click_count >= 2 {
                                editor.split_pane_ratio = 0.5;
                                editor.split_resize_session = None;
                                editor.schedule_workspace_session_save(cx);
                                cx.notify();
                            } else {
                                editor.start_split_resize(
                                    event.position.x,
                                    available_width,
                                    ratio,
                                    cx,
                                );
                            }
                        });
                        cx.stop_propagation();
                    })
                    .on_key_down(move |event, window, cx| {
                        let _ = divider_key_editor.update(cx, |editor, cx| {
                            editor.on_split_divider_key_down(event, available_width, window, cx);
                        });
                    }),
            )
            .child(
                div()
                    .h_full()
                    .flex()
                    .flex_1()
                    .min_w(px(0.0))
                    .map(|mut shell| {
                        shell.style().flex_grow = Some(1.0 - ratio);
                        shell
                    })
                    .child(preview),
            )
            .into_any_element()
    }
}
