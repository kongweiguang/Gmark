// @author kongweiguang

use super::*;

impl Editor {
    /// 两种关闭提示浮层都转发到同一键盘处理器，覆盖不经过编辑器根捕获路径的按钮焦点。
    pub(in crate::editor) fn on_close_dialog_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.handle_close_dialog_key(event, window, cx);
    }

    /// 两类关闭提示共用焦点循环，并显式调用按钮入口，避免依赖 Div 的隐式键盘点击。
    pub(in crate::editor) fn handle_close_dialog_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let window_dialog = self.show_unsaved_changes_dialog;
        if !window_dialog && !self.tabs.is_close_dialog_open() {
            return false;
        }
        if let Some(index) = self
            .close_dialog_focus_handles
            .iter()
            .position(|focus| focus.is_focused(window))
        {
            self.close_dialog_keyboard_index = index;
        }
        match event.keystroke.key.as_str() {
            "escape" => {
                if window_dialog {
                    self.cancel_close_dialog(window, cx);
                } else {
                    self.cancel_tab_close_dialog(window, cx);
                }
            }
            "tab" => {
                let count = self.close_dialog_focus_handles.len();
                let current = self.close_dialog_keyboard_index % count;
                let delta = if event.keystroke.modifiers.shift {
                    count - 1
                } else {
                    1
                };
                self.close_dialog_keyboard_index = (current + delta) % count;
                self.close_dialog_focus_handles[self.close_dialog_keyboard_index].focus(window);
                cx.notify();
            }
            "enter" | "space" if !event.keystroke.modifiers.modified() => {
                if !event.is_held {
                    let click = ClickEvent::default();
                    let button_index =
                        self.close_dialog_keyboard_index % self.close_dialog_focus_handles.len();
                    match button_index {
                        0 if window_dialog => self.on_cancel_close_dialog(&click, window, cx),
                        1 if window_dialog => self.on_discard_and_close(&click, window, cx),
                        2 if window_dialog => self.on_save_and_close(&click, window, cx),
                        0 => self.on_cancel_tab_close(&click, window, cx),
                        1 => self.on_discard_tab_close(&click, window, cx),
                        _ => self.on_save_tab_close(&click, window, cx),
                    }
                }
                cx.stop_propagation();
                return true;
            }
            _ => {}
        }
        cx.stop_propagation();
        true
    }
}
