// @author kongweiguang

//! Bottom status bar: sidebar toggle, mode switch, cursor position,
//! character count, and custom buttons.

use std::collections::{HashMap, HashSet};

use gmark_document::Revision;
use gpui::*;
use unicode_segmentation::UnicodeSegmentation;

use super::{Editor, ViewMode};
use crate::i18n::I18nStrings;
use crate::theme::Theme;

const SIDEBAR_ICON: &str = "icon/ui/panel-left.svg";
const SEARCH_ICON: &str = "icon/ui/search.svg";
const DOCUMENT_SIDEBAR_ICON: &str = "icon/ui/panel-right.svg";
const LIVE_MODE_ICON: &str = "icon/ui/live.svg";
const SOURCE_MODE_ICON: &str = "icon/ui/source.svg";
const SPLIT_MODE_ICON: &str = "icon/ui/split.svg";
const PREVIEW_MODE_ICON: &str = "icon/ui/preview.svg";
const MORE_ICON: &str = "icon/ui/more-horizontal.svg";
const RECOVERY_ICON: &str = "icon/ui/refresh.svg";
const CONFLICT_ICON: &str = "icon/ui/triangle-alert.svg";
const SHARED_DOCUMENT_ICON: &str = "icon/ui/link.svg";
const TOOLTIP_DELAY: std::time::Duration = std::time::Duration::from_millis(500);
const FORMAT_OVERFLOW_BREAKPOINT: f32 = 900.0;
const METADATA_OVERFLOW_BREAKPOINT: f32 = 760.0;
const ASYNC_CHARACTER_COUNT_THRESHOLD: usize = 1024 * 1024;
const CHARACTER_COUNT_IDLE_DELAY: std::time::Duration = std::time::Duration::from_millis(750);
const ALL_STATUS_VIEW_MODES: [super::ViewMode; 4] = [
    super::ViewMode::Rendered,
    super::ViewMode::Source,
    super::ViewMode::Split,
    super::ViewMode::Preview,
];
const PAGED_STATUS_VIEW_MODES: [super::ViewMode; 1] = [super::ViewMode::Source];
const SVG_STATUS_VIEW_MODES: [super::ViewMode; 3] = [
    super::ViewMode::Source,
    super::ViewMode::Split,
    super::ViewMode::Preview,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StatusTooltip {
    Sidebar,
    Search,
    DocumentSidebar,
    Mode(super::ViewMode),
}

#[derive(Default)]
pub(super) struct StatusBarState {
    pub sidebar_hovered: bool,
    pub search_hovered: bool,
    pub document_sidebar_hovered: bool,
    pub mode_hovered: Option<super::ViewMode>,
    custom_button_hovered: Option<String>,
    word_count: Option<(Revision, usize)>,
    word_count_scheduled_revision: Option<Revision>,
    word_count_task: Option<Task<()>>,
    pub(super) format_overflow_open: bool,
    tooltip_hovered: Option<StatusTooltip>,
    tooltip_visible: Option<StatusTooltip>,
    tooltip_task: Option<Task<()>>,
    /// 下拉入口与菜单项跨 render 保持稳定焦点身份，菜单关闭后可准确回到触发按钮。
    pub(super) mode_menu_open: bool,
    pub(super) line_ending_menu_open: bool,
    pub(super) mode_button_focus_handle: Option<FocusHandle>,
    pub(super) mode_focus_handles: Option<[FocusHandle; 4]>,
    pub(super) line_ending_button_focus_handle: Option<FocusHandle>,
    pub(super) line_ending_focus_handles: Option<[FocusHandle; 3]>,
    pub(super) sidebar_focus_handle: Option<FocusHandle>,
    pub(super) search_focus_handle: Option<FocusHandle>,
    pub(super) document_sidebar_focus_handle: Option<FocusHandle>,
    pub(super) overflow_focus_handle: Option<FocusHandle>,
    pub(super) conflict_focus_handle: Option<FocusHandle>,
    pub(super) custom_button_focus_handles: HashMap<String, FocusHandle>,
}

impl StatusBarState {
    pub(super) fn cached_word_count(&self, revision: Revision) -> Option<usize> {
        self.word_count
            .filter(|(cached_revision, _)| *cached_revision == revision)
            .map(|(_, count)| count)
    }

    pub(super) fn set_word_count(&mut self, revision: Revision, count: usize) {
        self.word_count = Some((revision, count));
        self.word_count_scheduled_revision = None;
        self.word_count_task = None;
    }

    pub(super) fn invalidate_word_count(&mut self) {
        self.word_count_scheduled_revision = None;
        self.word_count_task = None;
    }

    pub(super) fn apply_virtual_text_edit(
        &mut self,
        old_revision: Revision,
        new_revision: Revision,
        old_text: &str,
        new_text: &str,
    ) {
        let Some(total) = self.cached_word_count(old_revision) else {
            self.invalidate_word_count();
            return;
        };
        // virtual edit 替换完整顶层区域，区域两侧是未修改换行；字素簇不跨换行，
        // 因此整区域的新旧差值是精确的，组合音标和 ZWJ emoji 仍按完整区域分段。
        let old_count = count_characters(old_text);
        let new_count = count_characters(new_text);
        self.set_word_count(
            new_revision,
            total.saturating_sub(old_count).saturating_add(new_count),
        );
    }
}

impl Editor {
    /// 右下角模式按钮保持单一视觉契约；文件类型只决定其背后的实时编辑能力。
    pub(in crate::editor) fn activate_status_view_mode(
        &mut self,
        mode: super::ViewMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let json_graph = (mode == super::ViewMode::Rendered)
            .then(|| self.document_host.clone())
            .flatten()
            .filter(|view| view.read(cx).is_json_document());
        let Some(graph_view) = json_graph else {
            self.set_view_mode(mode, cx);
            return;
        };

        self.set_view_mode(super::ViewMode::Preview, cx);
        graph_view.update(cx, |view, cx| {
            view.begin_selected_json_graph_edit(window, cx)
        });
    }

    fn set_status_tooltip_hover(
        &mut self,
        tooltip: StatusTooltip,
        hovered: bool,
        cx: &mut Context<Self>,
    ) {
        self.status_bar.tooltip_task = None;
        self.status_bar.tooltip_hovered = hovered.then_some(tooltip);
        self.status_bar.tooltip_visible = None;
        if hovered {
            self.status_bar.tooltip_task = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(TOOLTIP_DELAY).await;
                let _ = this.update(cx, |editor, cx| {
                    if editor.status_bar.tooltip_hovered == Some(tooltip) {
                        editor.status_bar.tooltip_visible = Some(tooltip);
                        editor.status_bar.tooltip_task = None;
                        cx.notify();
                    }
                });
            }));
        }
        cx.notify();
    }

    pub(super) fn set_status_sidebar_tooltip_hover(
        &mut self,
        hovered: bool,
        cx: &mut Context<Self>,
    ) {
        self.set_status_tooltip_hover(StatusTooltip::Sidebar, hovered, cx);
    }

    pub(super) fn set_status_search_tooltip_hover(
        &mut self,
        hovered: bool,
        cx: &mut Context<Self>,
    ) {
        self.status_bar.search_hovered = hovered;
        self.set_status_tooltip_hover(StatusTooltip::Search, hovered, cx);
    }

    pub(super) fn set_status_document_sidebar_tooltip_hover(
        &mut self,
        hovered: bool,
        cx: &mut Context<Self>,
    ) {
        self.status_bar.document_sidebar_hovered = hovered;
        self.set_status_tooltip_hover(StatusTooltip::DocumentSidebar, hovered, cx);
    }

    pub(super) fn set_status_mode_tooltip_hover(
        &mut self,
        mode: super::ViewMode,
        hovered: bool,
        cx: &mut Context<Self>,
    ) {
        self.status_bar.mode_hovered = hovered.then_some(mode);
        self.set_status_tooltip_hover(StatusTooltip::Mode(mode), hovered, cx);
    }

    /// Long Source rows need an asynchronous grapheme prefix; the status bar omits stale columns rather than inventing one.
    pub(super) fn render_status_bar(
        &mut self,
        theme: &Theme,
        strings: &I18nStrings,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if self.focus_mode {
            return None;
        }
        let prefs = self.status_bar_preferences(cx);
        if !prefs.enabled {
            return None;
        }

        let c = &theme.colors;
        let d = &theme.dimensions;
        let custom_button_ids: HashSet<_> = prefs
            .custom_buttons
            .iter()
            .map(|button| button.id.clone())
            .collect();
        self.status_bar
            .custom_button_focus_handles
            .retain(|id, _| custom_button_ids.contains(id));

        let mut left_items: Vec<AnyElement> = Vec::new();

        if prefs.show_sidebar_toggle {
            left_items.push(render_sidebar_toggle(
                &mut self.status_bar,
                self.workspace.is_open,
                theme,
                strings,
                cx,
            ));
            let search_active = self.workspace.is_open && self.workspace_search_active();
            left_items.push(render_workspace_search_toggle(
                &mut self.status_bar,
                search_active,
                theme,
                strings,
                cx,
            ));
        }

        if let Some(view_count) = self.current_shared_view_count(cx) {
            let label = format!(
                "{} · {}",
                strings.status_bar_shared_document,
                strings
                    .status_bar_shared_views_template
                    .replace("{count}", &view_count.to_string()),
            );
            let tooltip = label.clone();
            left_items.push(
                div()
                    .id("status-bar-shared-document")
                    .debug_selector(|| "status-bar-shared-document".to_owned())
                    .h(px(d.status_bar_height))
                    .max_w(px(220.0))
                    .px(px(6.0))
                    .flex()
                    .items_center()
                    .gap(px(5.0))
                    .rounded(px(4.0))
                    .text_size(px(d.status_bar_text_size))
                    .text_color(c.workbench.text_secondary)
                    .tooltip(move |_window, cx| crate::ui::ui_tooltip(tooltip.clone(), cx))
                    .child(
                        svg()
                            .path(SHARED_DOCUMENT_ICON)
                            .size(px(13.0))
                            .text_color(c.workbench.accent),
                    )
                    .child(label)
                    .into_any_element(),
            );
        }

        if let Some(notice) = self.pane_notice.clone() {
            left_items.push(
                div()
                    .id("status-bar-pane-notice")
                    .debug_selector(|| "status-bar-pane-notice".to_owned())
                    .h(px(d.status_bar_height))
                    .max_w(px(420.0))
                    .px(px(6.0))
                    .flex()
                    .items_center()
                    .gap(px(5.0))
                    .overflow_hidden()
                    .truncate()
                    .text_size(px(d.status_bar_text_size))
                    .text_color(c.workbench.text_secondary)
                    .child(
                        svg()
                            .path(CONFLICT_ICON)
                            .size(px(13.0))
                            .text_color(c.workbench.accent),
                    )
                    .child(notice)
                    .into_any_element(),
            );
        }

        let mut right_items: Vec<AnyElement> = Vec::new();
        let mut overflow_items: Vec<AnyElement> = Vec::new();
        let resident_growth_status = self.source_document.resident_growth_reason().map(|_| {
            SharedString::from(strings.large_document_text("resident_growth_reopen_source"))
        });
        let document_host_status = self
            .document_host
            .as_ref()
            .map(|view| view.read(cx).status_text(strings))
            .or(resident_growth_status);

        if let Some(status) = &document_host_status {
            right_items.push(
                div()
                    .id("status-bar-document-host-status")
                    .debug_selector(|| "status-bar-document-host-status".to_owned())
                    .max_w(px(260.0))
                    .overflow_hidden()
                    .truncate()
                    .text_size(px(11.0))
                    .text_color(c.text_placeholder)
                    .child(status.clone())
                    .into_any_element(),
            );
        }

        if should_render_file_status(self.recovered_session, self.external_file_conflict) {
            right_items.push(render_recovery_status(
                &mut self.status_bar,
                self.external_file_conflict,
                theme,
                strings,
                cx,
            ));
        }

        let viewport_width = viewport_width_for_status(window);
        let (resident_encoding, line_ending) = self.current_source_format_labels(strings, cx);
        let encoding = self
            .document_host
            .as_ref()
            .map_or(resident_encoding, |view| view.read(cx).encoding_label());
        // 换行符是高频可编辑格式，固定紧邻模式入口；编码等低频信息才进入窄屏 overflow。
        if viewport_width >= FORMAT_OVERFLOW_BREAKPOINT {
            right_items.push(render_source_format_label(encoding.clone(), theme));
        } else {
            overflow_items.push(render_overflow_text(
                "status-bar-overflow-encoding",
                encoding.clone(),
                theme,
            ));
        }

        if let Some(document_host) = self.document_host.clone() {
            let follow_enabled = document_host.read(cx).follow_enabled();
            let follow_view = document_host.clone();
            overflow_items.push(
                render_large_overflow_action(
                    "status-bar-large-follow",
                    if follow_enabled {
                        strings.large_document_text("pause_log_following")
                    } else {
                        strings.large_document_text("follow_appended_content")
                    }
                    .to_owned(),
                    follow_enabled,
                    theme,
                )
                .on_click(cx.listener(move |editor, _, _, cx| {
                    follow_view.update(cx, |view, cx| view.toggle_follow(cx));
                    editor.status_bar.format_overflow_open = false;
                    cx.notify();
                }))
                .into_any_element(),
            );
            overflow_items.push(render_overflow_text(
                "status-bar-large-reopen-encoding-label",
                strings
                    .large_document_text("reopen_with_encoding")
                    .to_owned(),
                theme,
            ));
            let current_encoding = document_host.read(cx).encoding_label();
            for (id, active_label, encoding) in [
                (
                    "status-bar-large-reopen-utf8",
                    "UTF-8",
                    gmark_document_core::TextEncoding::Utf8 { bom: false },
                ),
                (
                    "status-bar-large-reopen-utf8-bom",
                    "UTF-8 BOM",
                    gmark_document_core::TextEncoding::Utf8 { bom: true },
                ),
                (
                    "status-bar-large-reopen-utf16-le",
                    "UTF-16 LE",
                    gmark_document_core::TextEncoding::Utf16Le,
                ),
                (
                    "status-bar-large-reopen-utf16-be",
                    "UTF-16 BE",
                    gmark_document_core::TextEncoding::Utf16Be,
                ),
                (
                    "status-bar-large-reopen-windows-1252",
                    "WINDOWS-1252",
                    gmark_document_core::TextEncoding::Legacy("windows-1252".to_owned()),
                ),
                (
                    "status-bar-large-reopen-gbk",
                    "GBK",
                    gmark_document_core::TextEncoding::Legacy("gbk".to_owned()),
                ),
                (
                    "status-bar-large-reopen-shift-jis",
                    "SHIFT_JIS",
                    gmark_document_core::TextEncoding::Legacy("shift_jis".to_owned()),
                ),
            ] {
                let encoding_view = document_host.clone();
                overflow_items.push(
                    render_large_overflow_action(
                        id,
                        strings
                            .large_document_text("reopen_as_template")
                            .replace("{encoding}", active_label),
                        current_encoding == active_label,
                        theme,
                    )
                    .on_click(cx.listener(move |editor, _, window, cx| {
                        let encoding = encoding.clone();
                        encoding_view.update(cx, |view, cx| {
                            view.reopen_with_encoding(encoding, window, cx)
                        });
                        editor.view_mode = super::ViewMode::Source;
                        editor.status_bar.format_overflow_open = false;
                        cx.notify();
                    }))
                    .into_any_element(),
                );
            }
            if document_host.read(cx).has_registered_structure_view()
                && !document_host.read(cx).is_json_document()
            {
                let structure_active = document_host.read(cx).structure_view_active();
                let structured_view = document_host.clone();
                overflow_items.push(
                    render_large_overflow_action(
                        "status-bar-large-structure",
                        if structure_active {
                            strings.large_document_text("return_to_source")
                        } else {
                            strings.large_document_text("open_structured_view")
                        }
                        .to_owned(),
                        structure_active,
                        theme,
                    )
                    .on_click(cx.listener(move |editor, _, _, cx| {
                        structured_view.update(cx, |view, cx| {
                            if structure_active {
                                view.show_source_view(cx);
                            } else {
                                view.show_structure_view(cx);
                            }
                        });
                        // Structure 是同一 Tab 内的文件工具，不冒充 Live/Preview 模式。
                        editor.view_mode = super::ViewMode::Source;
                        editor.status_bar.format_overflow_open = false;
                        cx.notify();
                    }))
                    .into_any_element(),
                );
                let split_active = document_host.read(cx).structured_split_active();
                let split_view = document_host.clone();
                overflow_items.push(
                    render_large_overflow_action(
                        "status-bar-large-structured-split",
                        if split_active {
                            strings.large_document_text("close_source_structure_split")
                        } else {
                            strings.large_document_text("open_source_structure_split")
                        }
                        .to_owned(),
                        split_active,
                        theme,
                    )
                    .on_click(cx.listener(move |editor, _, _, cx| {
                        split_view.update(cx, |view, cx| {
                            if split_active {
                                view.show_source_view(cx);
                            } else {
                                view.show_split_view(cx);
                            }
                        });
                        editor.view_mode = super::ViewMode::Source;
                        editor.status_bar.format_overflow_open = false;
                        cx.notify();
                    }))
                    .into_any_element(),
                );
            }
            let endings_visible = document_host.read(cx).line_endings_visible();
            overflow_items.push(
                render_large_overflow_action(
                    "status-bar-large-line-endings",
                    if endings_visible {
                        strings.large_document_text("hide_line_endings")
                    } else {
                        strings.large_document_text("show_line_endings")
                    }
                    .to_owned(),
                    endings_visible,
                    theme,
                )
                .on_click(cx.listener(move |editor, _, _, cx| {
                    document_host.update(cx, |view, cx| view.toggle_line_endings(cx));
                    editor.status_bar.format_overflow_open = false;
                    cx.notify();
                }))
                .into_any_element(),
            );
        }

        if prefs.show_cursor_position
            && matches!(
                self.view_mode,
                super::ViewMode::Source | super::ViewMode::Split
            )
        {
            let position = if let Some(view) = self.document_host.as_ref() {
                view.update(cx, |view, cx| {
                    view.prepare_cursor_position(cx);
                    view.cursor_position(cx)
                })
            } else {
                Some(self.compute_source_cursor_position(cx))
            };
            if let Some(position) = position {
                let cursor = render_cursor(position, theme);
                if viewport_width < METADATA_OVERFLOW_BREAKPOINT {
                    overflow_items.push(cursor);
                } else {
                    right_items.push(cursor);
                }
            }
        }

        if document_host_status.is_none() && prefs.show_word_count {
            let revision = self.source_document.revision();
            let total_count = if let Some(count) = self.status_bar.cached_word_count(revision) {
                Some(count)
            } else if self.source_document.len() < ASYNC_CHARACTER_COUNT_THRESHOLD {
                let text = self.serialized_document_text(cx);
                let count = count_characters(&text);
                self.status_bar.set_word_count(revision, count);
                Some(count)
            } else {
                self.schedule_character_count(revision, cx);
                self.status_bar.word_count.map(|(_, count)| count)
            };
            let selection_count = self
                .selected_markdown_text(cx)
                .as_deref()
                .map(count_characters);
            if let Some(total_count) = total_count {
                let character_count =
                    render_character_count(selection_count, total_count, theme, strings);
                if viewport_width < METADATA_OVERFLOW_BREAKPOINT {
                    overflow_items.push(character_count);
                } else {
                    right_items.push(character_count);
                }
            }
        }

        for button in &prefs.custom_buttons {
            let button = render_custom_button(&mut self.status_bar, button, theme, cx);
            if viewport_width < METADATA_OVERFLOW_BREAKPOINT {
                overflow_items.push(button);
            } else {
                right_items.push(button);
            }
        }

        if !overflow_items.is_empty() {
            right_items.push(render_source_format_overflow_button(
                &mut self.status_bar,
                theme,
                cx,
            ));
        }

        let line_ending_picker =
            render_line_ending_picker(&mut self.status_bar, line_ending, theme, cx);
        if prefs.show_mode_switch {
            let (json_document, paged_document) =
                self.document_host.as_ref().map_or((false, false), |view| {
                    let view = view.read(cx);
                    (view.is_json_document(), view.is_paged_document())
                });
            // Paged 后端没有 resident Markdown 投影；状态栏必须只呈现真实可用的
            // Source，不能用点击后再回退的方式暴露误导入口。
            let available_modes = if paged_document {
                &PAGED_STATUS_VIEW_MODES[..]
            } else if self.is_svg_document() {
                &SVG_STATUS_VIEW_MODES[..]
            } else {
                &ALL_STATUS_VIEW_MODES[..]
            };
            let mode_switch = render_mode_switch(
                &mut self.status_bar,
                self.view_mode,
                available_modes,
                json_document,
                theme,
                strings,
                cx,
            );
            right_items.push(
                div()
                    .id("status-bar-format-mode-group")
                    .debug_selector(|| "status-bar-format-mode-group".to_owned())
                    .flex()
                    .items_center()
                    .gap(px(2.0))
                    .child(line_ending_picker)
                    .child(mode_switch)
                    .into_any_element(),
            );
        } else {
            right_items.push(line_ending_picker);
        }

        if prefs.show_sidebar_toggle {
            right_items.push(render_document_sidebar_toggle(
                &mut self.status_bar,
                self.workspace.document_sidebar_open,
                theme,
                strings,
                cx,
            ));
        }

        let bar = div()
            .id("status-bar")
            .debug_selector(|| "status-bar".to_owned())
            .relative()
            .h(px(d.status_bar_height))
            .w_full()
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_between()
            .bg(c.workbench.navigation_surface)
            .border_t(px(1.0))
            .border_color(c.workbench.border_subtle)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(d.status_bar_item_gap))
                    .children(left_items),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(d.status_bar_item_gap))
                    .children(right_items),
            )
            .children(
                (!overflow_items.is_empty() && self.status_bar.format_overflow_open).then(|| {
                    div()
                        .id("status-bar-format-overflow")
                        .debug_selector(|| "status-bar-format-overflow".to_owned())
                        .absolute()
                        .right(px(0.0))
                        .bottom(px(d.status_bar_height + 4.0))
                        .min_w(px(180.0))
                        .occlude()
                        .p(px(10.0))
                        .flex()
                        .flex_col()
                        .gap(px(6.0))
                        .bg(c.workbench.elevated_surface)
                        .border(px(d.dialog_border_width))
                        .border_color(c.workbench.border_subtle)
                        .rounded(px(10.0))
                        .shadow_lg()
                        .children(overflow_items)
                }),
            )
            .into_any_element();

        Some(bar)
    }

    /// 长文档字符统计只在后台物化 Rope；发布前校验 revision，旧结果不得覆盖新输入。
    fn schedule_character_count(&mut self, revision: Revision, cx: &mut Context<Self>) {
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

    /// 窗口壳不持有活动文字选区，先查询当前窗格；局部坐标按 Source 逻辑行和完整字素计算。
    pub(super) fn compute_source_cursor_position(&self, cx: &App) -> (usize, usize) {
        if let Some(editor) = self.focused_pane_entities(cx).0 {
            return editor.read(cx).compute_source_cursor_position(cx);
        }
        let snapshot = self.capture_source_selection_snapshot(cx);
        let cursor_offset =
            super::saturating_source_offset(snapshot.source_selection().head.byte_offset);
        let text = self.document.raw_source_text(cx);
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

#[path = "status_bar_parts/format.rs"]
mod format;
use format::{render_source_format_label, source_format_labels, viewport_width_for_status};
#[path = "status_bar_parts/context.rs"]
mod context;

#[cfg(test)]
#[path = "../../tests/unit/editor/status_bar.rs"]
mod tests;
#[path = "status_bar_parts/view.rs"]
mod view;
pub(crate) use view::count_characters;
#[cfg(test)]
use view::normalized_action_id;
use view::{
    render_character_count, render_cursor, render_custom_button, render_document_sidebar_toggle,
    render_large_overflow_action, render_line_ending_picker, render_mode_switch,
    render_overflow_text, render_recovery_status, render_sidebar_toggle,
    render_source_format_overflow_button, render_workspace_search_toggle,
    should_render_file_status,
};
