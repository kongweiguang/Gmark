// @author kongweiguang

//! Contextual top-level menu model for the active document format.
//!
//! The format label is derived from the opened document host when one exists;
//! this keeps JSONL, TSV and other probed formats distinct from the editor's
//! legacy extension-only `DocumentKind` fallback.

use gpui::{App, Context, Menu, MenuItem, OwnedMenu, SharedString, Window};

use super::{DocumentKind, Editor};
use crate::components::{
    ExportHtml, ExportImage, ExportPdf, ExportSelection, FocusStructuredColumns,
    FocusStructuredFilter, InsertResource, NoRecentFiles, ShowDocumentInfo, ShowDocumentOutline,
    ShowStructureView, ShowStructuredInspector,
};
pub(crate) use crate::document_host::DocumentMenuFormat;
use crate::i18n::I18nManager;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FormatMenuItemKind {
    InsertResource,
    Outline,
    Structure,
    Records,
    Inspector,
    Table,
    Filter,
    Columns,
    DocumentInfo,
    ExportSelection,
    ExportHtml,
    ExportImage,
    ExportPdf,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct DocumentMenuCapabilities {
    pub(crate) has_selection: bool,
    pub(crate) has_structure: bool,
    pub(crate) has_json_selection: bool,
    pub(crate) has_filter: bool,
    pub(crate) has_columns: bool,
    pub(crate) paged: bool,
    pub(crate) export_in_progress: bool,
    pub(crate) outline_checked: bool,
    pub(crate) structure_checked: bool,
    pub(crate) inspector_checked: bool,
    pub(crate) filter_checked: bool,
    pub(crate) columns_checked: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FormatMenuDisabledReason {
    NoSelection,
    ExportInProgress,
    PagedSource,
    ProjectionUnavailable,
    JsonNodeNotSelected,
    FilterUnavailable,
    ColumnsUnavailable,
}

impl FormatMenuDisabledReason {
    pub(crate) fn label(self, chinese: bool) -> &'static str {
        match (self, chinese) {
            (Self::NoSelection, true) => "请先选择非空源码范围",
            (Self::NoSelection, false) => "Select a non-empty source range first",
            (Self::ExportInProgress, true) => "导出进行中",
            (Self::ExportInProgress, false) => "An export is already in progress",
            (Self::PagedSource, true) => "Paged 文档仅支持源码视图",
            (Self::PagedSource, false) => "Paged documents expose Source view only",
            (Self::ProjectionUnavailable, true) => "结构投影尚未可用",
            (Self::ProjectionUnavailable, false) => "The structure projection is unavailable",
            (Self::JsonNodeNotSelected, true) => "请先选择一个 JSON 节点",
            (Self::JsonNodeNotSelected, false) => "Select a JSON node first",
            (Self::FilterUnavailable, true) => "当前文档没有可用的筛选器",
            (Self::FilterUnavailable, false) => "Filtering is unavailable for this document",
            (Self::ColumnsUnavailable, true) => "当前文档没有可用的列工具",
            (Self::ColumnsUnavailable, false) => "Column tools are unavailable for this document",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FormatMenuItemState {
    pub(crate) kind: FormatMenuItemKind,
    pub(crate) enabled: bool,
    pub(crate) checked: bool,
    pub(crate) disabled_reason: Option<FormatMenuDisabledReason>,
}

impl FormatMenuItemState {
    fn enabled(kind: FormatMenuItemKind, checked: bool) -> Self {
        Self {
            kind,
            enabled: true,
            checked,
            disabled_reason: None,
        }
    }

    fn disabled(kind: FormatMenuItemKind, reason: FormatMenuDisabledReason) -> Self {
        Self {
            kind,
            enabled: false,
            checked: false,
            disabled_reason: Some(reason),
        }
    }
}

impl FormatMenuItemKind {
    pub(crate) fn label(self, format: DocumentMenuFormat, chinese: bool) -> &'static str {
        match (self, format, chinese) {
            (Self::InsertResource, _, true) => "插入资源",
            (Self::InsertResource, _, false) => "Insert Resource",
            (Self::Outline, _, true) => "大纲",
            (Self::Outline, _, false) => "Outline",
            (Self::Structure, _, true) => "结构视图",
            (Self::Structure, _, false) => "Structure",
            (Self::Records, _, true) => "记录视图",
            (Self::Records, _, false) => "Records",
            (Self::Inspector, _, true) => "检查器",
            (Self::Inspector, _, false) => "Inspector",
            (Self::Table, _, true) => "表格视图",
            (Self::Table, _, false) => "Table View",
            (Self::Filter, DocumentMenuFormat::JsonLines, true) => "筛选记录",
            (Self::Filter, DocumentMenuFormat::JsonLines, false) => "Filter Records",
            (Self::Filter, DocumentMenuFormat::Csv | DocumentMenuFormat::Tsv, true) => "筛选行",
            (Self::Filter, DocumentMenuFormat::Csv | DocumentMenuFormat::Tsv, false) => {
                "Filter Rows"
            }
            (Self::Filter, _, true) => "筛选",
            (Self::Filter, _, false) => "Filter",
            (Self::Columns, _, true) => "列工具",
            (Self::Columns, _, false) => "Column Tools",
            (Self::DocumentInfo, _, true) => "文档信息",
            (Self::DocumentInfo, _, false) => "Document Info",
            (Self::ExportSelection, _, true) => "导出选区",
            (Self::ExportSelection, _, false) => "Export Selection",
            (Self::ExportHtml, _, true) => "HTML",
            (Self::ExportHtml, _, false) => "HTML",
            (Self::ExportImage, _, true) => "PNG 图片",
            (Self::ExportImage, _, false) => "PNG Image",
            (Self::ExportPdf, _, true) => "PDF",
            (Self::ExportPdf, _, false) => "PDF",
        }
    }

    fn menu_item(self, format: DocumentMenuFormat, chinese: bool) -> MenuItem {
        match self {
            Self::InsertResource => MenuItem::action(self.label(format, chinese), InsertResource),
            Self::Outline => MenuItem::action(self.label(format, chinese), ShowDocumentOutline),
            Self::Structure | Self::Records | Self::Table => {
                MenuItem::action(self.label(format, chinese), ShowStructureView)
            }
            Self::Inspector => {
                MenuItem::action(self.label(format, chinese), ShowStructuredInspector)
            }
            Self::Filter => MenuItem::action(self.label(format, chinese), FocusStructuredFilter),
            Self::Columns => MenuItem::action(self.label(format, chinese), FocusStructuredColumns),
            Self::DocumentInfo => MenuItem::action(self.label(format, chinese), ShowDocumentInfo),
            Self::ExportSelection => MenuItem::action(self.label(format, chinese), ExportSelection),
            Self::ExportHtml => MenuItem::action(self.label(format, chinese), ExportHtml),
            Self::ExportImage => MenuItem::action(self.label(format, chinese), ExportImage),
            Self::ExportPdf => MenuItem::action(self.label(format, chinese), ExportPdf),
        }
    }
}

pub(crate) fn format_menu_item_states(
    format: DocumentMenuFormat,
    capabilities: DocumentMenuCapabilities,
) -> Vec<FormatMenuItemState> {
    let projection_reason = if capabilities.paged {
        FormatMenuDisabledReason::PagedSource
    } else {
        FormatMenuDisabledReason::ProjectionUnavailable
    };
    let export_state = |kind| {
        capabilities
            .export_in_progress
            .then_some(FormatMenuItemState::disabled(
                kind,
                FormatMenuDisabledReason::ExportInProgress,
            ))
            .unwrap_or_else(|| FormatMenuItemState::enabled(kind, false))
    };
    let selection_state = if capabilities.export_in_progress {
        FormatMenuItemState::disabled(
            FormatMenuItemKind::ExportSelection,
            FormatMenuDisabledReason::ExportInProgress,
        )
    } else if capabilities.has_selection {
        FormatMenuItemState::enabled(FormatMenuItemKind::ExportSelection, false)
    } else {
        FormatMenuItemState::disabled(
            FormatMenuItemKind::ExportSelection,
            FormatMenuDisabledReason::NoSelection,
        )
    };
    let projection_state = |kind, checked| {
        capabilities
            .has_structure
            .then_some(FormatMenuItemState::enabled(kind, checked))
            .unwrap_or_else(|| FormatMenuItemState::disabled(kind, projection_reason))
    };

    match format {
        DocumentMenuFormat::Markdown => vec![
            FormatMenuItemState::enabled(FormatMenuItemKind::InsertResource, false),
            FormatMenuItemState::enabled(FormatMenuItemKind::Outline, capabilities.outline_checked),
            FormatMenuItemState::enabled(FormatMenuItemKind::DocumentInfo, false),
            export_state(FormatMenuItemKind::ExportHtml),
            export_state(FormatMenuItemKind::ExportImage),
            export_state(FormatMenuItemKind::ExportPdf),
            selection_state,
        ],
        DocumentMenuFormat::Json => vec![
            projection_state(
                FormatMenuItemKind::Structure,
                capabilities.structure_checked,
            ),
            if capabilities.has_json_selection {
                FormatMenuItemState::enabled(
                    FormatMenuItemKind::Inspector,
                    capabilities.inspector_checked,
                )
            } else {
                FormatMenuItemState::disabled(
                    FormatMenuItemKind::Inspector,
                    if capabilities.has_structure {
                        FormatMenuDisabledReason::JsonNodeNotSelected
                    } else {
                        projection_reason
                    },
                )
            },
            FormatMenuItemState::enabled(FormatMenuItemKind::DocumentInfo, false),
            selection_state,
        ],
        DocumentMenuFormat::JsonLines => vec![
            projection_state(FormatMenuItemKind::Records, capabilities.structure_checked),
            if capabilities.paged {
                FormatMenuItemState::disabled(
                    FormatMenuItemKind::Filter,
                    FormatMenuDisabledReason::PagedSource,
                )
            } else if capabilities.has_filter {
                FormatMenuItemState::enabled(
                    FormatMenuItemKind::Filter,
                    capabilities.filter_checked,
                )
            } else {
                FormatMenuItemState::disabled(
                    FormatMenuItemKind::Filter,
                    FormatMenuDisabledReason::FilterUnavailable,
                )
            },
            FormatMenuItemState::enabled(FormatMenuItemKind::DocumentInfo, false),
            selection_state,
        ],
        DocumentMenuFormat::Csv | DocumentMenuFormat::Tsv => vec![
            projection_state(FormatMenuItemKind::Table, capabilities.structure_checked),
            if capabilities.paged {
                FormatMenuItemState::disabled(
                    FormatMenuItemKind::Filter,
                    FormatMenuDisabledReason::PagedSource,
                )
            } else if capabilities.has_filter {
                FormatMenuItemState::enabled(
                    FormatMenuItemKind::Filter,
                    capabilities.filter_checked,
                )
            } else {
                FormatMenuItemState::disabled(
                    FormatMenuItemKind::Filter,
                    FormatMenuDisabledReason::FilterUnavailable,
                )
            },
            if capabilities.paged {
                FormatMenuItemState::disabled(
                    FormatMenuItemKind::Columns,
                    FormatMenuDisabledReason::PagedSource,
                )
            } else if capabilities.has_columns {
                FormatMenuItemState::enabled(
                    FormatMenuItemKind::Columns,
                    capabilities.columns_checked,
                )
            } else {
                FormatMenuItemState::disabled(
                    FormatMenuItemKind::Columns,
                    FormatMenuDisabledReason::ColumnsUnavailable,
                )
            },
            FormatMenuItemState::enabled(FormatMenuItemKind::DocumentInfo, false),
            selection_state,
        ],
        DocumentMenuFormat::Text => vec![
            FormatMenuItemState::enabled(FormatMenuItemKind::DocumentInfo, false),
            selection_state,
        ],
    }
}

impl Editor {
    #[cfg(target_os = "macos")]
    pub(crate) fn schedule_platform_document_menu_refresh(&self, cx: &Context<Self>) {
        // App::set_menus is the macOS system-menu boundary. Defer the update
        // until the current entity notification has released its borrow.
        cx.spawn(async move |_this, async_cx| {
            let _ = async_cx.update(|cx| crate::app_menu::install_menus(cx));
        })
        .detach();
    }

    pub(crate) fn contextual_menus(&self, mut menus: Vec<OwnedMenu>, cx: &App) -> Vec<OwnedMenu> {
        let dynamic = self.build_document_menu(cx).owned();
        let insert_at = menus
            .iter()
            .position(|menu| matches!(menu.name.as_ref(), "Help" | "帮助"))
            .unwrap_or(menus.len());
        menus.insert(insert_at, dynamic);
        menus
    }

    pub(crate) fn document_menu_format(&self, cx: &App) -> DocumentMenuFormat {
        if let Some(host) = self.document_host.as_ref() {
            return host.read(cx).document_menu_format();
        }
        match self.document_kind {
            DocumentKind::Markdown => DocumentMenuFormat::Markdown,
            DocumentKind::Json => DocumentMenuFormat::Json,
            DocumentKind::Csv => DocumentMenuFormat::Csv,
            DocumentKind::Unspecified => DocumentMenuFormat::Text,
        }
    }

    pub(crate) fn build_document_menu(&self, cx: &App) -> Menu {
        let format = self.document_menu_format(cx);
        let chinese = cx
            .global::<I18nManager>()
            .current_language_id()
            .starts_with("zh");
        let capabilities = self.document_menu_capabilities(cx);
        let states = format_menu_item_states(format, capabilities);
        let state_for = |kind| {
            states
                .iter()
                .find(|state| state.kind == kind)
                .copied()
                .expect("format menu model must contain every rendered item")
        };
        let action_item = |kind: FormatMenuItemKind| {
            let state = state_for(kind);
            if state.enabled {
                kind.menu_item(format, chinese)
            } else {
                // GPUI 0.2.2 does not retain disabled/checked metadata in
                // OwnedMenu. NoRecentFiles is the existing disabled sentinel
                // understood by both the renderer and keyboard navigator;
                // the pure state model supplies the reason for the tooltip.
                MenuItem::action(kind.label(format, chinese), NoRecentFiles)
            }
        };

        let mut items = Vec::new();
        match format {
            DocumentMenuFormat::Markdown => {
                items.push(action_item(FormatMenuItemKind::InsertResource));
                items.push(MenuItem::separator());
                items.push(action_item(FormatMenuItemKind::Outline));
                items.push(action_item(FormatMenuItemKind::DocumentInfo));
                items.push(MenuItem::separator());
                items.push(MenuItem::submenu(Menu {
                    name: if chinese { "导出" } else { "Export" }.into(),
                    items: vec![
                        action_item(FormatMenuItemKind::ExportHtml),
                        action_item(FormatMenuItemKind::ExportImage),
                        action_item(FormatMenuItemKind::ExportPdf),
                    ],
                }));
                items.push(action_item(FormatMenuItemKind::ExportSelection));
            }
            DocumentMenuFormat::Json => {
                items.push(action_item(FormatMenuItemKind::Structure));
                items.push(action_item(FormatMenuItemKind::Inspector));
                items.push(action_item(FormatMenuItemKind::DocumentInfo));
                items.push(MenuItem::separator());
                items.push(action_item(FormatMenuItemKind::ExportSelection));
            }
            DocumentMenuFormat::JsonLines => {
                items.push(action_item(FormatMenuItemKind::Records));
                items.push(action_item(FormatMenuItemKind::Filter));
                items.push(action_item(FormatMenuItemKind::DocumentInfo));
                items.push(MenuItem::separator());
                items.push(action_item(FormatMenuItemKind::ExportSelection));
            }
            DocumentMenuFormat::Csv | DocumentMenuFormat::Tsv => {
                items.push(action_item(FormatMenuItemKind::Table));
                items.push(action_item(FormatMenuItemKind::Filter));
                items.push(action_item(FormatMenuItemKind::Columns));
                items.push(action_item(FormatMenuItemKind::DocumentInfo));
                items.push(MenuItem::separator());
                items.push(action_item(FormatMenuItemKind::ExportSelection));
            }
            DocumentMenuFormat::Text => {
                items.push(action_item(FormatMenuItemKind::DocumentInfo));
                items.push(MenuItem::separator());
                items.push(action_item(FormatMenuItemKind::ExportSelection));
            }
        }

        Menu {
            name: format.label(chinese).into(),
            items,
        }
    }

    fn document_has_selection(&self, cx: &App) -> bool {
        if let Some(host) = self.document_host.as_ref() {
            return host.read(cx).has_source_selection();
        }
        !self
            .capture_source_selection_snapshot(cx)
            .range()
            .is_empty()
    }

    pub(crate) fn document_menu_capabilities(&self, cx: &App) -> DocumentMenuCapabilities {
        let host = self.document_host.as_ref().map(|host| host.read(cx));
        let has_structure = host
            .as_ref()
            .is_some_and(|host| host.has_registered_structure_view());
        let has_json_selection = host
            .as_ref()
            .is_some_and(|host| host.has_json_graph_selection());
        let has_filter = host
            .as_ref()
            .is_some_and(|host| host.supports_structured_filter());
        let has_columns = host
            .as_ref()
            .is_some_and(|host| host.is_delimited_document());
        DocumentMenuCapabilities {
            has_selection: self.document_has_selection(cx),
            has_structure,
            has_json_selection,
            has_filter,
            has_columns,
            paged: host.as_ref().is_some_and(|host| host.is_paged_document()),
            export_in_progress: self.export_in_progress
                || host
                    .as_ref()
                    .is_some_and(|host| host.selection_export_in_progress()),
            outline_checked: self.workspace.document_sidebar_open,
            structure_checked: self.view_mode == super::ViewMode::Preview,
            inspector_checked: false,
            filter_checked: false,
            columns_checked: has_columns && self.view_mode == super::ViewMode::Rendered,
        }
    }

    pub(crate) fn document_menu_action_checked(&self, action: &dyn gpui::Action) -> bool {
        (action.as_any().is::<ShowStructureView>() && self.view_mode == super::ViewMode::Preview)
            || (action.as_any().is::<ShowDocumentOutline>() && self.workspace.document_sidebar_open)
    }

    pub(crate) fn document_menu_disabled_reason(
        &self,
        name: &str,
        action: &dyn gpui::Action,
        cx: &App,
    ) -> Option<SharedString> {
        if !action.as_any().is::<NoRecentFiles>() {
            return None;
        }
        let format = self.document_menu_format(cx);
        let chinese = cx
            .global::<I18nManager>()
            .current_language_id()
            .starts_with("zh");
        let state = format_menu_item_states(format, self.document_menu_capabilities(cx))
            .into_iter()
            .find(|state| !state.enabled && state.kind.label(format, chinese) == name)?;
        state
            .disabled_reason
            .map(|reason| reason.label(chinese).into())
    }

    pub(crate) fn document_info_lines(&self, cx: &App) -> Vec<String> {
        let chinese = cx
            .global::<I18nManager>()
            .current_language_id()
            .starts_with("zh");
        let format = self.document_menu_format(cx).label(chinese);
        let (path, encoding, bytes, lines, endings) =
            if let Some(host) = self.document_host.as_ref() {
                let host = host.read(cx);
                (
                    self.file_path
                        .as_deref()
                        .map_or_else(|| "Untitled".to_owned(), |path| path.display().to_string()),
                    host.encoding_label(),
                    host.document_length(),
                    host.document_line_count(),
                    host.document_line_ending_label(),
                )
            } else {
                let source = self.current_document_source(cx);
                let summary = self.source_document.source_format_summary();
                let endings = match summary.line_endings {
                    gmark_document::LineEndingStatus::None => "—".to_owned(),
                    gmark_document::LineEndingStatus::Mixed => "Mixed".to_owned(),
                    gmark_document::LineEndingStatus::Uniform(ending) => match ending {
                        gmark_document::LineEnding::Lf => "LF".to_owned(),
                        gmark_document::LineEnding::CrLf => "CRLF".to_owned(),
                        gmark_document::LineEnding::Cr => "CR".to_owned(),
                    },
                };
                (
                    self.file_path
                        .as_deref()
                        .map_or_else(|| "Untitled".to_owned(), |path| path.display().to_string()),
                    self.source_encoding.label().to_owned(),
                    source.len() as u64,
                    source.lines().count().max(1) as u64,
                    endings,
                )
            };
        let view = match (self.view_mode, chinese) {
            (super::ViewMode::Rendered, true) => "Live",
            (super::ViewMode::Rendered, false) => "Live",
            (super::ViewMode::Source, true) => "源码",
            (super::ViewMode::Source, false) => "Source",
            (super::ViewMode::Preview, true) => "预览",
            (super::ViewMode::Preview, false) => "Preview",
            (super::ViewMode::Split, true) => "分栏",
            (super::ViewMode::Split, false) => "Split",
        };
        if chinese {
            vec![
                format!("路径：{path}"),
                format!("格式：{format}"),
                format!("编码：{encoding}"),
                format!("换行符：{endings}"),
                format!("大小：{bytes} 字节"),
                format!("行数：{lines}"),
                format!("当前视图：{view}"),
            ]
        } else {
            vec![
                format!("Path: {path}"),
                format!("Format: {format}"),
                format!("Encoding: {encoding}"),
                format!("Line endings: {endings}"),
                format!("Size: {bytes} bytes"),
                format!("Lines: {lines}"),
                format!("View: {view}"),
            ]
        }
    }

    pub(crate) fn on_show_document_outline(
        &mut self,
        _: &ShowDocumentOutline,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_menu_bar(cx);
        self.open_document_sidebar(window, cx);
    }

    pub(crate) fn on_show_structure_view(
        &mut self,
        _: &ShowStructureView,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_menu_bar(cx);
        if !self.pane_canvas
            && let Some(host) = self.focused_pane_entities(cx).1
        {
            host.update(cx, |host, cx| host.show_structure_view(cx));
        } else if let Some(host) = self.document_host.clone() {
            host.update(cx, |host, cx| host.show_structure_view(cx));
        }
    }

    pub(crate) fn on_show_structured_inspector(
        &mut self,
        _: &ShowStructuredInspector,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_menu_bar(cx);
        if !self.pane_canvas
            && let Some(host) = self.focused_pane_entities(cx).1
        {
            host.update(cx, |host, cx| host.focus_json_inspector(window, cx));
        } else if let Some(host) = self.document_host.clone() {
            host.update(cx, |host, cx| host.focus_json_inspector(window, cx));
        }
    }

    pub(crate) fn on_focus_structured_filter(
        &mut self,
        _: &FocusStructuredFilter,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_menu_bar(cx);
        if !self.pane_canvas
            && let Some(host) = self.focused_pane_entities(cx).1
        {
            host.update(cx, |host, cx| host.focus_structured_filter(window, cx));
        } else if let Some(host) = self.document_host.clone() {
            host.update(cx, |host, cx| host.focus_structured_filter(window, cx));
        }
    }

    pub(crate) fn on_focus_structured_columns(
        &mut self,
        _: &FocusStructuredColumns,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_menu_bar(cx);
        if !self.pane_canvas
            && let Some(host) = self.focused_pane_entities(cx).1
        {
            host.update(cx, |host, cx| host.focus_structured_columns(window, cx));
        } else if let Some(host) = self.document_host.clone() {
            host.update(cx, |host, cx| host.focus_structured_columns(window, cx));
        }
    }

    pub(crate) fn on_show_document_info(
        &mut self,
        _: &ShowDocumentInfo,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_info_dialog(super::InfoDialogKind::Document, cx);
    }

    /// 菜单和命令面板共享活动窗格的选区快照，避免把窗口壳的空选区当成实际输入。
    pub(crate) fn on_export_selection_action(
        &mut self,
        _: &ExportSelection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_menu_bar(cx);
        if !self.pane_canvas
            && let Some(host) = self.focused_pane_entities(cx).1
        {
            host.update(cx, |host, cx| host.export_selection_from_menu(window, cx));
        } else if !self.pane_canvas
            && let Some(editor) = self.focused_pane_entities(cx).0
        {
            editor.update(cx, |editor, cx| {
                editor.export_selection_via_prompt(window, cx)
            });
        } else if let Some(host) = self.document_host.clone() {
            host.update(cx, |host, cx| host.export_selection_from_menu(window, cx));
        } else {
            self.export_selection_via_prompt(window, cx);
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/editor/format_menu.rs"]
mod tests;
