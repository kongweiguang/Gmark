// @author kongweiguang

//! Source byte-format commands that do not alter normalized Markdown text.

use gmark_document::LineEnding;
use gpui::*;

use super::*;

impl Editor {
    /// 菜单与动作共用活动窗格路由；仅所属可编辑文档提交一次格式历史，不能修改窗口壳。
    pub(crate) fn normalize_line_endings(&mut self, ending: LineEnding, cx: &mut Context<Self>) {
        if !self.pane_canvas {
            let (markdown, host) = self.focused_pane_entities(cx);
            if let Some(editor) = markdown {
                editor.update(cx, |editor, cx| editor.normalize_line_endings(ending, cx));
                return;
            }
            if host.is_some() {
                return;
            }
        }
        if !self.document_surface_is_editable() || self.has_active_ime_composition(cx) {
            return;
        }

        self.finalize_pending_undo_capture(cx);
        self.prepare_undo_capture(UndoCaptureKind::NonCoalescible, cx);
        match self.source_document.normalize_line_endings(ending) {
            Ok(Some(_)) => {
                // 格式事务不重建文本投影；否则虚拟模式会额外提交一次块文本事务，
                // 破坏 SourceDocument 历史与 Editor 选择历史的一一对应关系。
                self.finalize_pending_undo_capture(cx);
                self.mark_source_format_dirty(cx);
            }
            Ok(None) => {
                self.pending_undo_capture = None;
                self.pending_virtual_undo_selection = None;
            }
            Err(error) => {
                self.pending_undo_capture = None;
                self.pending_virtual_undo_selection = None;
                eprintln!("换行格式规范化失败: {error}");
            }
        }
    }

    /// Marks a byte-format-only mutation dirty without synchronizing unchanged projections.
    pub(super) fn mark_source_format_dirty(&mut self, cx: &mut Context<Self>) {
        self.pending_dirty_source = None;
        if !self.document_dirty {
            self.document_dirty = true;
            self.pending_window_edited = true;
            self.pending_window_title_refresh = true;
        }
        self.schedule_recovery_journal(cx);
        self.schedule_auto_save(cx);
        cx.notify();
    }
}
