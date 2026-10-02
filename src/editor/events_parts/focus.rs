// @author kongweiguang

use super::*;

impl Editor {
    /// 焦点只刷新前后两个文字目标，长文不因单击而重绘所有可见块。
    pub(in crate::editor) fn handle_block_focus_request(
        &mut self,
        block: &Entity<Block>,
        cx: &mut Context<Self>,
    ) {
        let previous = self
            .active_entity_id
            .and_then(|id| self.focusable_entity_by_id(id));
        self.close_menu_bar(cx);
        self.clear_table_axis_preview(cx);
        self.clear_table_axis_selection(cx);
        self.focus_block(block.entity_id());
        if let Some(previous) = previous.filter(|previous| previous != block) {
            previous.update(cx, |_, cx| cx.notify());
        }
        block.update(cx, |_, cx| cx.notify());
        cx.notify();
    }
}
