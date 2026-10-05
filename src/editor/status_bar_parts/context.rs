// @author kongweiguang

//! Status-bar preference and shared-document context queries.

use gpui::App;

use crate::i18n::I18nStrings;
use crate::preferences::StatusBarPreferences;

use super::Editor;

impl Editor {
    /// 窗口壳不再持有正文；格式与编码随活动窗格读取，按钮和窄窗菜单共用同一来源。
    pub(super) fn current_source_format_labels(
        &self,
        strings: &I18nStrings,
        cx: &App,
    ) -> (String, String) {
        if let Some(editor) = self.focused_pane_entities(cx).0 {
            return editor.read(cx).current_source_format_labels(strings, cx);
        }
        super::source_format_labels(
            &self.source_document.source_format_summary(),
            &self.source_encoding,
            strings,
        )
    }

    // Reason: Keep preference access beside the context query so the renderer only coordinates
    // the resulting status-bar regions and does not own lookup details.
    pub(super) fn status_bar_preferences(&self, cx: &App) -> StatusBarPreferences {
        crate::preferences::EditorSettings::status_bar_preferences(cx)
    }

    // Reason: Resolve the focused pane first so shared-view counts describe the document the
    // user is looking at, while preserving the existing single-document fallback.
    pub(super) fn current_shared_view_count(&self, cx: &App) -> Option<usize> {
        let pane_document = self.pane_workspace.as_ref().and_then(|workspace| {
            let workspace = workspace.read(cx);
            let pane = workspace.workspace().focused_pane();
            let document = workspace.workspace().pane(pane)?.active_tab()?.view();
            let count = document
                .lease()
                .map(|lease| lease.handle().lease_count())
                .or_else(|| document.host_lease_count());
            Some((document.document_id(), count))
        });
        let pane_count = pane_document.and_then(|(document_id, count)| {
            count.or_else(|| {
                self.pane_document_close_states(cx)
                    .into_iter()
                    .find(|state| state.document_id == document_id)
                    .map(|state| state.global_lease_count)
            })
        });
        let count = pane_count.unwrap_or_else(|| {
            self.document_host.as_ref().map_or_else(
                || self.source_document.lease_count(),
                |host| host.read(cx).lease_count(),
            )
        });
        (count > 1).then_some(count)
    }
}
