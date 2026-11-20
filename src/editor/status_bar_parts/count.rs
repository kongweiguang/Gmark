// @author kongweiguang

//! 字符统计和源码光标位置共享状态栏的快照与修订约束。

use super::*;

impl Editor {
    /// 常驻源码直接读取字节选区；长选区后台统计，全文选择复用当前 revision 的总数。
    pub(super) fn resident_source_selection_count(
        &mut self,
        revision: Revision,
        cx: &mut Context<Self>,
    ) -> Option<usize> {
        let Some(root) = self.document.first_root() else {
            return None;
        };
        let block = root.read(cx);
        if !block.is_source_raw_mode()
            || !block.show_source_line_numbers()
            || block.has_ime_composition()
        {
            return self
                .selected_markdown_text(cx)
                .as_deref()
                .map(count_characters);
        }
        let range = block.selected_range.clone();
        let text = block.shared_display_text();
        if range.is_empty() || text.as_ref().get(range.clone()).is_none() {
            self.status_bar.selection_count = None;
            self.status_bar.selection_count_scheduled = None;
            self.status_bar.selection_count_task = None;
            return None;
        }
        if range.start == 0 && range.end == text.len() && text.len() == self.source_document.len() {
            self.status_bar.selection_count = None;
            self.status_bar.selection_count_scheduled = None;
            self.status_bar.selection_count_task = None;
            return self.status_bar.cached_word_count(revision);
        }
        let key = SelectionCountKey {
            revision,
            range,
            text,
        };
        if let Some((cached_key, count)) = self.status_bar.selection_count.as_ref()
            && *cached_key == key
        {
            return Some(*count);
        }
        if key.range.end - key.range.start < ASYNC_CHARACTER_COUNT_THRESHOLD {
            self.status_bar.selection_count_scheduled = None;
            self.status_bar.selection_count_task = None;
            let count = key
                .text
                .as_ref()
                .get(key.range.clone())
                .map(count_characters)?;
            self.status_bar.selection_count = Some((key, count));
            return Some(count);
        }
        if self.status_bar.selection_count_scheduled.as_ref() != Some(&key) {
            self.status_bar.selection_count_scheduled = Some(key.clone());
            self.status_bar.selection_count_task =
                Some(cx.spawn(async move |this: WeakEntity<Self>, cx| {
                    cx.background_executor()
                        .timer(CHARACTER_COUNT_IDLE_DELAY)
                        .await;
                    let work = key.clone();
                    let count = cx
                        .background_spawn(async move {
                            work.text.as_ref().get(work.range).map(count_characters)
                        })
                        .await;
                    let _ = this.update(cx, |editor, cx| {
                        if editor.status_bar.selection_count_scheduled.as_ref() != Some(&key) {
                            return;
                        }
                        let current = editor.document.first_root().is_some_and(|root| {
                            let block = root.read(cx);
                            let text = block.shared_display_text();
                            block.selected_range == key.range
                                && std::ptr::eq(text.as_ref(), key.text.as_ref())
                        });
                        editor.status_bar.selection_count_scheduled = None;
                        editor.status_bar.selection_count_task = None;
                        if editor.source_document.revision() == key.revision
                            && current
                            && editor.active_selection_surface == SelectionSurface::Main
                            && matches!(editor.view_mode, ViewMode::Source | ViewMode::Split)
                            && let Some(count) = count
                        {
                            editor.status_bar.selection_count = Some((key, count));
                            cx.notify();
                        }
                    });
                }));
        }
        None
    }

    /// 长文档字符统计只在后台物化 Rope；发布前校验 revision，旧结果不得覆盖新输入。
    pub(super) fn schedule_character_count(&mut self, revision: Revision, cx: &mut Context<Self>) {
        if self.status_bar.word_count_scheduled_revision == Some(revision) {
            return;
        }
        let snapshot = self.source_document.snapshot();
        self.status_bar.word_count_scheduled_revision = Some(revision);
        self.status_bar.word_count_task =
            Some(cx.spawn(async move |this: WeakEntity<Self>, cx| {
                cx.background_executor()
                    .timer(CHARACTER_COUNT_IDLE_DELAY)
                    .await;
                let count = cx
                    .background_spawn(async move { count_characters(&snapshot.text()) })
                    .await;
                let _ = this.update(cx, |editor, cx| {
                    if editor.source_document.revision() != revision
                        || editor.status_bar.word_count_scheduled_revision != Some(revision)
                    {
                        return;
                    }
                    editor.status_bar.set_word_count(revision, count);
                    cx.notify();
                });
            }));
    }

    /// 窗口壳先查询当前窗格；常驻源码直接借用正文，避免光标刷新时复制整个文件。
    pub(in crate::editor) fn compute_source_cursor_position(&self, cx: &App) -> (usize, usize) {
        if let Some(editor) = self.focused_pane_entities(cx).0 {
            return editor.read(cx).compute_source_cursor_position(cx);
        }
        let snapshot = self.capture_source_selection_snapshot(cx);
        let cursor_offset =
            super::super::saturating_source_offset(snapshot.source_selection().head.byte_offset);
        let resident = self.document.first_root().filter(|root| {
            let block = root.read(cx);
            block.is_source_raw_mode() && block.show_source_line_numbers()
        });
        let fallback;
        let text = if let Some(root) = resident.as_ref() {
            root.read(cx).display_text()
        } else {
            fallback = self.document.raw_source_text(cx);
            fallback.as_str()
        };
        let mut clamped = cursor_offset.min(text.len());
        while clamped > 0 && !text.is_char_boundary(clamped) {
            clamped -= 1;
        }

        let line = text[..clamped].matches('\n').count() + 1;
        let last_newline = text[..clamped].rfind('\n').map(|i| i + 1).unwrap_or(0);
        let col = text[last_newline..clamped].graphemes(true).count() + 1;
        (line, col)
    }
}
