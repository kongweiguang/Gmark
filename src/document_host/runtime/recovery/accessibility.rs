// @author kongweiguang

//! Status and accessibility projections.

use super::*;

impl DocumentHost {
    /// 大文件的运行状态由 Editor 的统一状态栏承载，内容视图不再绘制第二条状态栏。
    pub(crate) fn status_text(&self, strings: &I18nStrings) -> SharedString {
        if let Some(error) = &self.error {
            return error.clone();
        }
        if self.reloading {
            return strings.large_document_text("reloading").into();
        }
        if self.saving {
            return strings.large_document_text("saving").into();
        }
        if let Some(notice) = &self.mode_notice {
            return notice.clone();
        }
        if self
            .document
            .as_ref()
            .and_then(SharedDocument::resident_growth_reason)
            .is_some()
        {
            return strings
                .large_document_text("resident_growth_reopen_source")
                .into();
        }
        if self.index.is_none() {
            return strings
                .large_document_text("indexing_status_template")
                .replace(
                    "{mib}",
                    &format!("{:.1}", self.probe.len as f64 / (1024.0 * 1024.0)),
                )
                .into();
        }
        strings
            .large_document_text("size_lines_template")
            .replace(
                "{mib}",
                &format!("{:.1}", self.probe.len as f64 / (1024.0 * 1024.0)),
            )
            .replace("{lines}", &self.line_count().to_string())
            .into()
    }

    /// 无障碍正文复用当前有界输入表面；候选与刚输入的文字无需等待后台行缓存追平。
    pub(crate) fn accessibility_snapshot(
        &self,
        cx: &App,
    ) -> crate::accessibility::EditorAccessibilitySnapshot {
        let title = self
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::to_owned)
            .unwrap_or_else(|| {
                cx.global::<I18nManager>()
                    .strings()
                    .large_document_text("untitled")
            });
        let input = self.source_accessibility_input(cx);
        let lines = self
            .displayed_screen_lines
            .rows
            .iter()
            .map(|(line, row)| {
                let text = match input {
                    Some((input_line, block)) if input_line == *line => {
                        block.read(cx).display_text_with_ime().to_string()
                    }
                    _ => row.text.to_string(),
                };
                (*line as u64, text)
            })
            .collect();
        let folds = self
            .fold_projection
            .regions()
            .iter()
            .filter(|region| {
                self.displayed_screen_lines
                    .rows
                    .contains_key(&region.start_line)
            })
            .map(|region| crate::accessibility::AccessibilityFold {
                start_line: region.start_line as u64,
                end_line: region.end_line as u64,
                collapsed: self.fold_projection.is_collapsed(region.id),
                target: Some(crate::accessibility::AccessibilityFoldTarget::SourceLine),
            })
            .collect();
        let error = self
            .error
            .as_ref()
            .or(self.coordinator.recovery_error.as_ref())
            .or(self.structure_error.as_ref())
            .map(ToString::to_string);
        crate::accessibility::EditorAccessibilitySnapshot {
            title,
            mode: match self.view_mode {
                DocumentHostViewMode::Source => crate::accessibility::AccessibilityMode::Source,
                DocumentHostViewMode::Live => crate::accessibility::AccessibilityMode::Live,
                DocumentHostViewMode::Structure => crate::accessibility::AccessibilityMode::Preview,
                DocumentHostViewMode::Split => crate::accessibility::AccessibilityMode::Split,
            },
            dirty: document_dirty_state(&self.document),
            status: self
                .status_text(cx.global::<I18nManager>().strings())
                .to_string(),
            error,
            busy: self.saving || self.reloading || self.index.is_none() || self.search_running,
            update_actions: Vec::new(),
            close_actions: Vec::new(),
            search_visible: self.search_visible,
            navigation_visible: self.navigation_visible,
            caret: Some(self.accessibility_caret(cx)),
            lines,
            folds,
            math: None,
        }
    }

    /// 布局缓存与输入表面共用发布身份，避免正文未变的候选更新或移动插入点被忽略。
    pub(crate) fn accessibility_revision(&self, cx: &App) -> u64 {
        use std::hash::{Hash, Hasher};

        let flags = u64::from(document_dirty_state(&self.document))
            | (u64::from(self.saving) << 1)
            | (u64::from(self.reloading) << 2)
            | (u64::from(self.search_running) << 3)
            | (u64::from(self.search_visible) << 4)
            | (u64::from(self.navigation_visible) << 5)
            | (u64::from(self.error.is_some()) << 6)
            | (u64::from(self.structure_error.is_some()) << 7)
            | (u64::from(self.coordinator.recovery_error.is_some()) << 8)
            | (match self.view_mode {
                DocumentHostViewMode::Source => 0,
                DocumentHostViewMode::Live => 1,
                DocumentHostViewMode::Structure => 2,
                DocumentHostViewMode::Split => 3,
            } << 10);
        let row_signature = self
            .displayed_screen_lines
            .rows
            .first_key_value()
            .map_or(0, |(line, _)| *line as u64)
            .wrapping_mul(31)
            .wrapping_add(
                self.displayed_screen_lines
                    .rows
                    .last_key_value()
                    .map_or(0, |(line, _)| *line as u64),
            )
            .wrapping_mul(31)
            .wrapping_add(self.displayed_screen_lines.rows.len() as u64);
        let fold_signature = self
            .fold_projection
            .regions()
            .iter()
            .fold(0_u64, |hash, region| {
                hash.wrapping_mul(31)
                    .wrapping_add(region.start_line as u64)
                    .wrapping_mul(31)
                    .wrapping_add(u64::from(self.fold_projection.is_collapsed(region.id)))
            });
        let mut message_hasher = std::collections::hash_map::DefaultHasher::new();
        self.error.hash(&mut message_hasher);
        self.structure_error.hash(&mut message_hasher);
        self.coordinator.recovery_error.hash(&mut message_hasher);
        self.mode_notice.hash(&mut message_hasher);
        self.coordinator.external_status.hash(&mut message_hasher);
        if let Some(document) = self.document.as_ref() {
            document.revision().hash(&mut message_hasher);
            let selection = document.source_selection();
            selection.anchor.byte_offset.hash(&mut message_hasher);
            selection.head.byte_offset.hash(&mut message_hasher);
        }
        if let Some((line, owner)) = self.source_accessibility_input(cx) {
            let block = owner.read(cx);
            line.hash(&mut message_hasher);
            owner.entity_id().hash(&mut message_hasher);
            block
                .display_text_with_ime()
                .as_ref()
                .hash(&mut message_hasher);
            block.selected_range.hash(&mut message_hasher);
            block.selection_reversed.hash(&mut message_hasher);
        }
        self.displayed_screen_lines
            .cache_epoch
            .wrapping_mul(31)
            .wrapping_add(self.displayed_screen_lines.document_revision)
            .wrapping_mul(31)
            .wrapping_add(self.displayed_screen_lines.generation)
            .wrapping_mul(31)
            .wrapping_add(self.displayed_screen_lines.column_window_start)
            .wrapping_mul(31)
            .wrapping_add(row_signature)
            .wrapping_mul(31)
            .wrapping_add(fold_signature)
            .wrapping_mul(31)
            .wrapping_add(self.coordinator.search_generation)
            .wrapping_mul(31)
            .wrapping_add(self.coordinator.external_generation)
            .wrapping_mul(31)
            .wrapping_add(message_hasher.finish())
            .wrapping_mul(512)
            .wrapping_add(flags)
    }

    /// 只投影与权威 revision 一致的输入 owner；旧实体不能把候选带到新的文档位置。
    fn source_accessibility_input(&self, cx: &App) -> Option<(usize, &Entity<Block>)> {
        let revision = self.document.as_ref()?.revision();
        if let Some(snapshot) = self.source_ime_snapshot.as_ref()
            && snapshot.revision == revision
            && !snapshot.row_entity.read(cx).is_read_only()
        {
            return Some((snapshot.line, &snapshot.row_entity));
        }
        let active = self.active_edit.as_ref()?;
        (active.base_revision == revision && !active.block.read(cx).is_read_only())
            .then_some((active.line, &active.block))
    }

    pub(crate) fn activate_accessibility_error(&mut self, cx: &mut Context<Self>) {
        if let Some(offset) = self.structure_error_byte {
            self.jump_byte_offset_to_source(offset, cx);
        }
    }
}
