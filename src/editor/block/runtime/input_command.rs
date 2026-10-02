// @author kongweiguang

use super::*;
use crate::components::block::BlockInputCommand;

impl Block {
    /// 普通编辑动作不能走 IME RESULT 通道；等待原 owner 终态后仍执行同一个意图。
    pub(crate) fn guard_input_command(
        &mut self,
        command: BlockInputCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.is_read_only() && command != BlockInputCommand::SelectAll {
            return true;
        }
        if !self.defer_input_command_if_composing(command, cx) {
            return false;
        }
        if !self.ime_interactions_managed && !matches!(window.finish_ime_composition(), Ok(true)) {
            cx.emit(BlockEvent::ImeCompositionFinishFailed);
        }
        true
    }

    /// 仅原实体回放本地动作；文档跨块与表格选区由 Editor 在此前先行处理。
    pub(crate) fn replay_input_command(
        &mut self,
        command: BlockInputCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use crate::components::*;
        match command {
            BlockInputCommand::Cut => self.on_cut(&Cut, window, cx),
            BlockInputCommand::Paste => self.on_paste(&Paste, window, cx),
            BlockInputCommand::PasteAsPlainText => {
                self.on_paste_as_plain_text(&PasteAsPlainText, window, cx)
            }
            BlockInputCommand::Delete => self.on_delete(&Delete, window, cx),
            BlockInputCommand::DeleteBack => self.on_delete_back(&DeleteBack, window, cx),
            BlockInputCommand::WordDeleteBack => {
                self.on_word_delete_back(&WordDeleteBack, window, cx)
            }
            BlockInputCommand::WordDeleteForward => {
                self.on_word_delete_forward(&WordDeleteForward, window, cx)
            }
            BlockInputCommand::SelectAll => self.on_select_all(&SelectAll, window, cx),
        }
    }
}
