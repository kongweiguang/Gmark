// @author kongweiguang

use super::*;

impl Block {
    /// Preview 不能创建可写浮层；预先缓存输入焦点，避免提交回调重入借用输入实体。
    pub(in super::super) fn open_selection_link_editor(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.is_read_only() {
            self.clear_selection_link_editor_state();
            cx.notify();
            return;
        }
        let Some(range) = self.selection_toolbar_range() else {
            return;
        };
        let target = self
            .record
            .title
            .selection_link_destination(range.clone())
            .unwrap_or_default();
        let had_target = self.record.title.selection_has_link(range.clone());
        let input = cx.new(|cx| {
            let mut input = Block::with_record(cx, BlockRecord::paragraph(target));
            input.set_compact_source_host();
            input.set_input_placeholder("https://example.com");
            input.set_host_submit_enabled(true);
            input
        });
        let parent = cx.entity().downgrade();
        input.update(cx, move |input, _cx| {
            input.set_host_action_handler(move |action, window, cx| match action {
                BlockHostAction::Submit(destination) => {
                    let destination = {
                        let destination = destination.trim();
                        (!destination.is_empty()).then(|| destination.to_owned())
                    };
                    let _ = parent.update(cx, |block, cx| {
                        block.commit_selection_link_destination(destination, window, cx)
                    });
                }
                BlockHostAction::DismissTransientUi => {
                    let _ = parent.update(cx, |block, cx| {
                        block.cancel_selection_link_editor(window, cx)
                    });
                }
                _ => {}
            });
            input.focus_handle.focus(window);
        });
        self.selection_toolbar_link_focus = Some(input.read(cx).focus_handle.clone());
        self.selection_toolbar_link_input = Some(input);
        self.selection_toolbar_link_range = Some(range);
        self.selection_toolbar_link_had_target = had_target;
        self.selection_toolbar_overflow_open = false;
        self.selection_toolbar_type_menu_open = false;
        cx.notify();
    }

    pub(in super::super) fn commit_selection_link_editor(
        &mut self,
        remove: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let destination = if remove {
            None
        } else {
            self.selection_toolbar_link_input
                .as_ref()
                .map(|input| input.read(cx).display_text().trim().to_owned())
                .filter(|target| !target.is_empty())
        };
        self.commit_selection_link_destination(destination, window, cx);
    }

    /// 提交链接目标只改变链接属性并保留选区；只读视图丢弃迟到提交且不改变焦点。
    fn commit_selection_link_destination(
        &mut self,
        destination: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.is_read_only() {
            self.clear_selection_link_editor_state();
            cx.notify();
            return;
        }
        let Some(range) = self.selection_toolbar_link_range.clone() else {
            return;
        };
        let mut next_title = self.record.title.clone();
        if next_title.set_inline_link_destination(range.clone(), destination) {
            self.prepare_undo_capture(UndoCaptureKind::NonCoalescible, cx);
            self.apply_title_edit(
                next_title,
                range.end,
                None,
                Some(range),
                Some(self.selection_reversed),
                false,
                false,
                cx,
            );
        }
        self.close_selection_link_editor(window, cx);
    }

    fn cancel_selection_link_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_selection_link_editor(window, cx);
    }

    /// 拒绝或迟到动作只清理浮层，不接管用户已转移的焦点。
    fn clear_selection_link_editor_state(&mut self) {
        self.selection_toolbar_link_input = None;
        self.selection_toolbar_link_focus = None;
        self.selection_toolbar_link_range = None;
        self.selection_toolbar_link_had_target = false;
    }

    /// 只按缓存句柄归还原输入的焦点，既避免实体重入，也防止迟到取消抢走查找框。
    fn close_selection_link_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let return_focus = self
            .selection_toolbar_link_focus
            .as_ref()
            .is_some_and(|focus| focus.is_focused(window));
        self.clear_selection_link_editor_state();
        if return_focus {
            self.focus_handle.focus(window);
        }
        cx.notify();
    }
}

#[cfg(test)]
#[path = "../../../../tests/unit/components/block/selection_link_readonly.rs"]
mod tests;
