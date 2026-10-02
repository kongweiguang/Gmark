// @author kongweiguang

//! Action-backed command palette with background filtering.

use std::time::Duration;

use gpui::*;

#[path = "command_palette_metadata.rs"]
mod metadata;

use self::metadata::localized_action_description;
use super::{Block, BlockRecord, Editor, render::menu_icon_slot};
use crate::app_menu::menu_action_icon;
use crate::components::{
    AddLanguageConfig, BlockEvent, BoldSelection, CheckForUpdates, CodeSelection,
    EditingCommandCategory, EditingCommandId, EditingSelectionContext, EditingViewMode, ExportHtml,
    ExportImage, ExportPdf, HighlightSelection, InlineMathSelection, InsertResource,
    InstallCliTool, ItalicSelection, LinkSelection, NoRecentFiles, NormalizeLineEndingsCr,
    NormalizeLineEndingsCrLf, NormalizeLineEndingsLf, OpenCrashReports, OpenPrivacyPolicy,
    OpenSafeSource, SetBulletedList, SetCodeBlock, SetHeading1, SetHeading2, SetHeading3,
    SetHeading4, SetHeading5, SetHeading6, SetNumberedList, SetParagraph, SetQuote, SetTaskList,
    ShortcutCommand, ShowAbout, StrikethroughSelection, SubscriptSelection, SuperscriptSelection,
    UnderlineSelection, UninstallCliTool, format_shortcut_for_display, shortcut_definitions,
};
use crate::i18n::I18nStrings;
use crate::preferences::localized_shortcut_command_label;
use crate::theme::{Theme, workbench::SurfaceKind};
use crate::ui::visual_preferences::VisualPreferencesManager;

const FILTER_DEBOUNCE: Duration = Duration::from_millis(20);
const MAX_RESULTS: usize = 100;
const SEARCH_ICON: &str = "icon/ui/search.svg";
const CLOSE_ICON: &str = "icon/ui/close.svg";

fn editing_command_for_action(action: &dyn Action) -> Option<EditingCommandId> {
    if action.as_any().is::<BoldSelection>() {
        Some(EditingCommandId::Bold)
    } else if action.as_any().is::<ItalicSelection>() {
        Some(EditingCommandId::Italic)
    } else if action.as_any().is::<UnderlineSelection>() {
        Some(EditingCommandId::Underline)
    } else if action.as_any().is::<StrikethroughSelection>() {
        Some(EditingCommandId::Strikethrough)
    } else if action.as_any().is::<HighlightSelection>() {
        Some(EditingCommandId::Highlight)
    } else if action.as_any().is::<SuperscriptSelection>() {
        Some(EditingCommandId::Superscript)
    } else if action.as_any().is::<SubscriptSelection>() {
        Some(EditingCommandId::Subscript)
    } else if action.as_any().is::<InlineMathSelection>() {
        Some(EditingCommandId::InlineMath)
    } else if action.as_any().is::<CodeSelection>() {
        Some(EditingCommandId::InlineCode)
    } else if action.as_any().is::<LinkSelection>() {
        Some(EditingCommandId::Link)
    } else if action.as_any().is::<SetParagraph>() {
        Some(EditingCommandId::Paragraph)
    } else if action.as_any().is::<SetHeading1>() {
        Some(EditingCommandId::Heading1)
    } else if action.as_any().is::<SetHeading2>() {
        Some(EditingCommandId::Heading2)
    } else if action.as_any().is::<SetHeading3>() {
        Some(EditingCommandId::Heading3)
    } else if action.as_any().is::<SetHeading4>() {
        Some(EditingCommandId::Heading4)
    } else if action.as_any().is::<SetHeading5>() {
        Some(EditingCommandId::Heading5)
    } else if action.as_any().is::<SetHeading6>() {
        Some(EditingCommandId::Heading6)
    } else if action.as_any().is::<SetBulletedList>() {
        Some(EditingCommandId::BulletedList)
    } else if action.as_any().is::<SetNumberedList>() {
        Some(EditingCommandId::NumberedList)
    } else if action.as_any().is::<SetTaskList>() {
        Some(EditingCommandId::TaskList)
    } else if action.as_any().is::<SetQuote>() {
        Some(EditingCommandId::Quote)
    } else if action.as_any().is::<SetCodeBlock>() {
        Some(EditingCommandId::CodeBlock)
    } else if action.as_any().is::<InsertResource>() {
        Some(EditingCommandId::Resource)
    } else {
        None
    }
}

struct PaletteCommand {
    label: String,
    description: String,
    search_text: String,
    shortcut: String,
    icon: &'static str,
    action: Box<dyn Action>,
}

pub(super) struct CommandPaletteState {
    pub(super) input: Entity<Block>,
    restore_focus: Option<EntityId>,
    commands: Vec<PaletteCommand>,
    filtered: Vec<usize>,
    selected: usize,
    generation: u64,
    task: Option<Task<()>>,
}

impl Editor {
    /// Queues palette focus until the active editor input has a confirmed IME terminal state.
    pub(crate) fn on_command_palette_action(
        &mut self,
        _: &crate::components::CommandPalette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.defer_action_for_ime(&crate::components::CommandPalette, window, cx) {
            cx.stop_propagation();
            return;
        }
        self.close_menu_bar(cx);
        self.dismiss_contextual_overlays(cx);
        let i18n = cx.global::<crate::i18n::I18nManager>();
        let strings = i18n.strings().clone();
        let language_id = i18n.current_language_id().to_owned();
        let mut commands = window
            .available_actions(cx)
            .into_iter()
            .filter(|action| self.command_palette_action_available(action.as_ref(), cx))
            .map(|action| {
                let label = localized_action_label(action.as_ref(), &strings, &language_id);
                let description =
                    localized_action_description(action.as_ref(), &label, &language_id);
                let search_text = command_search_text(action.as_ref(), &label, &description);
                let shortcut =
                    display_shortcut(&window.keystroke_text_for(action.as_ref()), action.name());
                PaletteCommand {
                    label,
                    description,
                    search_text,
                    shortcut,
                    icon: command_icon(action.as_ref()),
                    action,
                }
            })
            .collect::<Vec<_>>();
        commands.sort_by_key(|command| command.label.to_lowercase());
        let input = cx.new(|cx| {
            let mut block = Block::with_record(cx, BlockRecord::paragraph(String::new()));
            block.set_source_raw_mode();
            block
        });
        cx.subscribe(&input, Self::on_command_palette_input_event)
            .detach();
        input.read(cx).focus_handle.focus(window);
        self.command_palette = Some(CommandPaletteState {
            input,
            restore_focus: self.active_entity_id,
            commands,
            filtered: Vec::new(),
            selected: 0,
            generation: 0,
            task: None,
        });
        self.schedule_command_palette_filter(cx);
        cx.notify();
    }

    fn command_palette_action_available(&self, action: &dyn Action, cx: &App) -> bool {
        let Some(command) = editing_command_for_action(action) else {
            return true;
        };
        let Some(block) = self
            .active_entity_id
            .and_then(|entity_id| self.focusable_entity_by_id(entity_id))
        else {
            return false;
        };
        let block = block.read(cx);
        let mut context = block.editing_command_context();
        context.view_mode = match self.view_mode {
            super::ViewMode::Rendered => EditingViewMode::Rendered,
            super::ViewMode::Source => EditingViewMode::Source,
            super::ViewMode::Split => EditingViewMode::Split,
            super::ViewMode::Preview => EditingViewMode::Preview,
        };
        if context.selection == EditingSelectionContext::AcrossBlocks
            && command.descriptor().category == EditingCommandCategory::Inline
            && !block.editor_selection_supports_inline_commands
        {
            return false;
        }
        command.is_available(context)
    }

    fn on_command_palette_input_event(
        &mut self,
        _: Entity<Block>,
        event: &BlockEvent,
        cx: &mut Context<Self>,
    ) {
        if matches!(event, BlockEvent::Changed) {
            self.schedule_command_palette_filter(cx);
        }
    }

    fn schedule_command_palette_filter(&mut self, cx: &mut Context<Self>) {
        let Some(state) = self.command_palette.as_mut() else {
            return;
        };
        state.generation = state.generation.wrapping_add(1);
        state.task = None;
        let generation = state.generation;
        let query = state.input.read(cx).display_text().trim().to_owned();
        let labels = state
            .commands
            .iter()
            .map(|command| command.search_text.clone())
            .collect::<Vec<_>>();
        state.task = Some(cx.spawn(async move |this: WeakEntity<Self>, cx| {
            cx.background_executor().timer(FILTER_DEBOUNCE).await;
            let filtered = cx
                .background_spawn(async move { filter_command_labels(&labels, &query) })
                .await;
            let _ = this.update(cx, |editor, cx| {
                let Some(state) = editor.command_palette.as_mut() else {
                    return;
                };
                if state.generation != generation {
                    return;
                }
                state.task = None;
                state.filtered = filtered;
                state.selected = 0;
                cx.notify();
            });
        }));
    }

    pub(super) fn dismiss_command_palette(&mut self) -> bool {
        let Some(state) = self.command_palette.take() else {
            return false;
        };
        self.pending_focus = state.restore_focus;
        true
    }

    /// Leaves navigation, Enter, and Escape to the palette input's IME while candidates are active.
    pub(super) fn handle_command_palette_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.has_active_ime_composition(cx) {
            return false;
        }
        let Some(state) = self.command_palette.as_mut() else {
            return false;
        };
        match event.keystroke.key.as_str() {
            "up" => state.selected = state.selected.saturating_sub(1),
            "down" => {
                state.selected = (state.selected + 1).min(state.filtered.len().saturating_sub(1));
            }
            "enter" => {
                let action = state
                    .filtered
                    .get(state.selected)
                    .and_then(|index| state.commands.get(*index))
                    .map(|command| command.action.boxed_clone());
                let restore_focus = state.restore_focus;
                self.command_palette = None;
                self.pending_focus = restore_focus;
                if let Some(action) = action {
                    window.dispatch_action(action, cx);
                }
            }
            "escape" => {
                let restore_focus = state.restore_focus;
                self.command_palette = None;
                self.pending_focus = restore_focus;
            }
            _ => return false,
        }
        cx.notify();
        true
    }

    /// Keeps the search field mounted when outside clicks request dismissal during composition.
    pub(super) fn render_command_palette_overlay(
        &self,
        theme: &Theme,
        strings: &I18nStrings,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let state = self.command_palette.as_ref()?;
        let c = &theme.colors;
        let visual_preferences = cx
            .try_global::<VisualPreferencesManager>()
            .map(VisualPreferencesManager::current)
            .unwrap_or_default();
        let palette = &c.workbench;
        let overlay_material = palette.material(SurfaceKind::GlassStrong, visual_preferences);
        let input_material = palette.material(SurfaceKind::Solid, visual_preferences);
        let d = &theme.dimensions;
        let t = &theme.typography;
        let editor = cx.entity().downgrade();
        let dismiss_editor = editor.clone();
        let close_editor = editor.clone();
        let close_tooltip: SharedString = strings.ui_close.clone().into();
        let empty_message = if state.input.read(cx).display_text().trim().is_empty() {
            strings.command_palette_prompt.clone()
        } else {
            strings.command_palette_no_results.clone()
        };
        Some(
            div()
                .id("command-palette-overlay")
                .absolute()
                .top_0()
                .left_0()
                .right_0()
                .bottom_0()
                .occlude()
                .flex()
                .justify_center()
                .items_start()
                .pt(px(82.0))
                .bg(palette.overlay_scrim)
                .on_mouse_down(MouseButton::Left, move |_event, window, cx| {
                    let _ = dismiss_editor.update(cx, |editor, cx| {
                        editor.request_command_palette_close(window, cx);
                    });
                })
                .child(
                    div()
                        .id("command-palette-dialog")
                        .debug_selector(|| "command-palette-dialog".to_owned())
                        .w(px(560.0))
                        .max_w(relative(0.92))
                        .max_h(relative(0.74))
                        .flex()
                        .flex_col()
                        .overflow_hidden()
                        .bg(overlay_material.background)
                        .border(px(d.dialog_border_width))
                        .border_color(overlay_material.border)
                        .rounded(px(d.dialog_radius.clamp(22.0, 28.0)))
                        .shadow_lg()
                        .on_mouse_down(MouseButton::Left, |_event, _window, cx| {
                            cx.stop_propagation();
                        })
                        .child(
                            div()
                                .h(px(38.0))
                                .px(px(14.0))
                                .flex()
                                .items_center()
                                .justify_between()
                                .gap(px(12.0))
                                .child(
                                    div()
                                        .min_w(px(0.0))
                                        .overflow_hidden()
                                        .truncate()
                                        .text_size(px(t.dialog_title_size))
                                        .font_weight(t.dialog_title_weight.to_font_weight())
                                        .text_color(palette.text_primary)
                                        .child(strings.command_palette_title.clone()),
                                )
                                .child(
                                    div()
                                        .id("command-palette-close")
                                        .debug_selector(|| "command-palette-close".to_owned())
                                        .size(px(28.0))
                                        .flex_shrink_0()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .rounded(px(5.0))
                                        .cursor_pointer()
                                        .hover(|this| this.bg(palette.control_hover))
                                        .tooltip(move |_window, cx| {
                                            crate::ui::ui_tooltip(close_tooltip.clone(), cx)
                                        })
                                        .child(svg().path(CLOSE_ICON).size(px(15.0)))
                                        .on_click(move |_event, window, cx| {
                                            let _ = close_editor.update(cx, |editor, cx| {
                                                editor.request_command_palette_close(window, cx);
                                            });
                                        }),
                                ),
                        )
                        .child(
                            div()
                                .id("command-palette-input")
                                .debug_selector(|| "command-palette-input".to_owned())
                                .mx(px(12.0))
                                .mb(px(10.0))
                                .min_h(px(40.0))
                                .px(px(10.0))
                                .flex()
                                .items_center()
                                .gap(px(8.0))
                                .rounded(px(6.0))
                                .border(px(d.dialog_border_width))
                                .border_color(input_material.border)
                                .bg(input_material.background)
                                .child(
                                    div()
                                        .id("command-palette-search-icon")
                                        .debug_selector(|| "command-palette-search-icon".to_owned())
                                        .size(px(16.0))
                                        .flex_shrink_0()
                                        .text_color(palette.text_secondary)
                                        .child(svg().path(SEARCH_ICON).size(px(16.0))),
                                )
                                .child(div().flex_1().min_w(px(0.0)).child(state.input.clone())),
                        )
                        .child(
                            div()
                                .id("command-palette-results")
                                .debug_selector(|| "command-palette-results".to_owned())
                                .flex_1()
                                .min_h(px(52.0))
                                .overflow_y_scroll()
                                .px(px(8.0))
                                .pb(px(8.0))
                                .children(state.filtered.is_empty().then(|| {
                                    div()
                                        .px(px(10.0))
                                        .py(px(14.0))
                                        .text_size(px(t.dialog_body_size))
                                        .text_color(palette.text_secondary)
                                        .child(empty_message)
                                }))
                                .children(state.filtered.iter().enumerate().filter_map(
                                    |(row, index)| {
                                        let command = state.commands.get(*index)?;
                                        let action = command.action.boxed_clone();
                                        let editor = editor.clone();
                                        Some(
                                            div()
                                                .id(("command-palette-result", row))
                                                .debug_selector(move || {
                                                    format!("command-palette-result-{row}")
                                                })
                                                .min_h(px(50.0))
                                                .w_full()
                                                .px(px(10.0))
                                                .flex()
                                                .items_center()
                                                .gap(px(8.0))
                                                .rounded(px(5.0))
                                                .bg(if row == state.selected {
                                                    palette.selection
                                                } else {
                                                    hsla(0.0, 0.0, 0.0, 0.0)
                                                })
                                                .hover(|this| {
                                                    this.bg(palette.control_hover)
                                                })
                                                .cursor_pointer()
                                                .child(
                                                    menu_icon_slot(
                                                        Some(command.icon),
                                                        palette.icon,
                                                    )
                                                        .debug_selector(move || {
                                                            format!(
                                                                "command-palette-result-icon-{row}"
                                                            )
                                                        }),
                                                )
                                                .child(
                                                    div()
                                                        .min_w(px(0.0))
                                                        .flex_grow()
                                                        .overflow_hidden()
                                                        .flex()
                                                        .flex_col()
                                                        .gap(px(2.0))
                                                        .child(
                                                            div()
                                                                .truncate()
                                                                .debug_selector(move || {
                                                                    format!(
                                                                        "command-palette-result-label-{row}"
                                                                    )
                                                                })
                                                                .text_size(px(t.dialog_body_size))
                                                                .text_color(palette.text_primary)
                                                                .child(command.label.clone()),
                                                        )
                                                        .child(
                                                            div()
                                                                .truncate()
                                                                .debug_selector(move || {
                                                                    format!(
                                                                        "command-palette-result-description-{row}"
                                                                    )
                                                                })
                                                                .text_size(px(
                                                                    t.dialog_body_size * 0.82,
                                                                ))
                                                                .text_color(palette.text_secondary)
                                                                .child(command.description.clone()),
                                                        ),
                                                )
                                                .child(
                                                    div()
                                                        .flex_shrink_0()
                                                        .max_w(px(160.0))
                                                        .overflow_hidden()
                                                        .truncate()
                                                        .text_right()
                                                        .debug_selector(move || {
                                                            format!(
                                                                "command-palette-result-shortcut-{row}"
                                                            )
                                                        })
                                                        .text_size(px(t.dialog_body_size * 0.86))
                                                        .text_color(palette.text_secondary)
                                                        .child(command.shortcut.clone()),
                                                )
                                                .on_click(move |_event, window, cx| {
                                                    let action = action.boxed_clone();
                                                    let _ = editor.update(cx, |editor, _cx| {
                                                        editor.dismiss_command_palette();
                                                    });
                                                    window.dispatch_action(action, cx);
                                                }),
                                        )
                                    },
                                )),
                        ),
                )
                .into_any_element(),
        )
    }
}

fn humanize_action_name(name: &str) -> String {
    let name = name.rsplit("::").next().unwrap_or(name);
    let mut output = String::with_capacity(name.len() + 8);
    let mut previous_lowercase = false;
    for ch in name.chars() {
        if ch == '_' || ch == '-' {
            if !output.ends_with(' ') {
                output.push(' ');
            }
            previous_lowercase = false;
            continue;
        }
        if ch.is_uppercase() && previous_lowercase {
            output.push(' ');
        }
        output.push(ch);
        previous_lowercase = ch.is_lowercase() || ch.is_ascii_digit();
    }
    output
}

fn canonical_action_id(value: &str) -> String {
    value
        .rsplit("::")
        .next()
        .unwrap_or(value)
        .chars()
        .filter(|ch| ch.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn shortcut_command_for_action_name(name: &str) -> Option<ShortcutCommand> {
    let action_id = canonical_action_id(name);
    shortcut_definitions()
        .iter()
        .find(|definition| canonical_action_id(definition.id) == action_id)
        .map(|definition| definition.command)
}

fn localized_action_label(action: &dyn Action, strings: &I18nStrings, language_id: &str) -> String {
    if let Some(command) = editing_command_for_action(action) {
        let descriptor = command.descriptor();
        return strings
            .slash_commands
            .get(descriptor.localization_key)
            .cloned()
            .unwrap_or_else(|| humanize_action_name(action.name()));
    }

    if language_id.starts_with("zh") {
        let label = match canonical_action_id(action.name()).as_str() {
            "normalizelineendingslf" => "统一为 LF 换行符",
            "normalizelineendingscrlf" => "统一为 CRLF 换行符",
            "normalizelineendingscr" => "统一为 CR 换行符",
            "cancelformatting" => "取消格式化",
            "collapsefold" => "折叠当前区域",
            "collapseallfolds" => "全部折叠",
            "expandfold" => "展开当前区域",
            "expandallfolds" => "全部展开",
            "focusstructuredcolumns" => "聚焦结构化列",
            "focusstructuredfilter" => "聚焦结构化筛选",
            "splitright" => "向右拆分",
            "splitdown" => "向下拆分",
            "closepane" => "关闭窗格",
            "focuspaneleft" => "聚焦左侧窗格",
            "focuspaneright" => "聚焦右侧窗格",
            "focuspaneup" => "聚焦上方窗格",
            "focuspanedown" => "聚焦下方窗格",
            "movetabtopaneleft" => "将标签页移至左侧窗格",
            "movetabtopaneright" => "将标签页移至右侧窗格",
            "movetabtopaneup" => "将标签页移至上方窗格",
            "movetabtopanedown" => "将标签页移至下方窗格",
            "balancepanes" => "平衡窗格",
            "formatdocument" => "格式化文档",
            "formatselection" => "格式化选区",
            "showdocumentoutline" => "大纲",
            "showstructureview" => "结构视图",
            "showstructuredinspector" => "检查器",
            "showdocumentinfo" => "文档信息",
            "exporthtml" => "导出为 HTML",
            "exportimage" => "导出为 PNG 图片",
            "exportpdf" => "导出为 PDF",
            "exportselection" => "导出所选内容",
            "selectlanguage" => "切换界面语言",
            "openrecentfile" => "打开最近文件",
            _ => "",
        };
        if !label.is_empty() {
            return label.to_owned();
        }
    }

    if let Some(command) = shortcut_command_for_action_name(action.name()) {
        return localized_shortcut_command_label(command, strings);
    }

    if action.as_any().is::<AddLanguageConfig>() {
        strings.menu_add_language_config.clone()
    } else if action.as_any().is::<CheckForUpdates>() {
        strings.menu_check_updates.clone()
    } else if action.as_any().is::<OpenSafeSource>() {
        strings.menu_open_safe_source.clone()
    } else if action.as_any().is::<NoRecentFiles>() {
        strings.menu_no_recent_files.clone()
    } else if action.as_any().is::<ExportHtml>() {
        strings.menu_export_html.clone()
    } else if action.as_any().is::<ExportImage>() {
        strings.menu_export_image.clone()
    } else if action.as_any().is::<ExportPdf>() {
        strings.menu_export_pdf.clone()
    } else if action.as_any().is::<OpenCrashReports>() {
        strings.menu_open_crash_reports.clone()
    } else if action.as_any().is::<OpenPrivacyPolicy>() {
        strings.menu_privacy_policy.clone()
    } else if action.as_any().is::<ShowAbout>() {
        strings.menu_about.clone()
    } else if action.as_any().is::<InstallCliTool>() {
        strings.menu_install_cli_tool.clone()
    } else if action.as_any().is::<UninstallCliTool>() {
        strings.menu_uninstall_cli_tool.clone()
    } else if action.as_any().is::<NormalizeLineEndingsLf>() {
        strings.menu_line_ending_lf.clone()
    } else if action.as_any().is::<NormalizeLineEndingsCrLf>() {
        strings.menu_line_ending_crlf.clone()
    } else if action.as_any().is::<NormalizeLineEndingsCr>() {
        strings.menu_line_ending_cr.clone()
    } else {
        humanize_action_name(action.name())
    }
}

fn command_search_text(action: &dyn Action, label: &str, description: &str) -> String {
    let mut search_text = label.to_owned();
    if let Some(command) = editing_command_for_action(action) {
        for alias in command.descriptor().aliases {
            search_text.push(' ');
            search_text.push_str(alias);
        }
    }
    search_text.push(' ');
    search_text.push_str(description);
    search_text.push(' ');
    search_text.push_str(&humanize_action_name(action.name()));
    search_text
}

fn display_shortcut(raw: &str, action_name: &str) -> String {
    let shortcut = raw.trim();
    if shortcut.is_empty()
        || shortcut.contains("::")
        || canonical_action_id(shortcut) == canonical_action_id(action_name)
    {
        String::new()
    } else {
        format_shortcut_for_display(shortcut)
    }
}

fn command_icon(action: &dyn Action) -> &'static str {
    if let Some(command) = editing_command_for_action(action) {
        return command.descriptor().icon_path;
    }
    if let Some(icon) = menu_action_icon(action) {
        return icon;
    }
    match canonical_action_id(action.name()).as_str() {
        "pageup" | "jumptotop" | "blockup" | "moveup" => "icon/ui/arrow-up.svg",
        "pagedown" | "jumptobottom" | "blockdown" | "movedown" => "icon/ui/arrow-down.svg",
        "moveleft" | "wordmoveleft" | "selectleft" | "wordselectleft" => "icon/ui/arrow-left.svg",
        "moveright" | "wordmoveright" | "selectright" | "wordselectright" => {
            "icon/ui/arrow-right.svg"
        }
        "splitright" | "focuspaneright" | "movetabtopaneright" => "icon/ui/panel-right.svg",
        "splitdown" | "focuspanedown" | "movetabtopanedown" => "icon/ui/panel-bottom.svg",
        "focuspaneleft" | "movetabtopaneleft" => "icon/ui/panel-left.svg",
        "focuspaneup" | "movetabtopaneup" => "icon/ui/arrow-up.svg",
        "closepane" => "icon/ui/close.svg",
        "balancepanes" => "icon/ui/align-center.svg",
        "exportselection" => "icon/ui/file-output.svg",
        "gotoline" | "home" | "end" | "selecthome" | "selectend" => "icon/ui/type.svg",
        "selectlanguage" => "icon/ui/keyboard.svg",
        "openrecentfile" => "icon/ui/files.svg",
        "delete" | "deleteback" | "worddeleteback" | "worddeleteforward" => "icon/ui/trash.svg",
        "indentblock" | "outdentblock" => "icon/ui/list.svg",
        "exitcodeblock" => "icon/ui/code.svg",
        _ => "icon/ui/lightbulb.svg",
    }
}

fn filter_command_labels(labels: &[String], query: &str) -> Vec<usize> {
    let mut matches = labels
        .iter()
        .enumerate()
        .filter_map(|(index, label)| {
            let score = command_match_score(label, query)?;
            Some((index, score))
        })
        .collect::<Vec<_>>();
    matches.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    matches.truncate(MAX_RESULTS);
    matches.into_iter().map(|(index, _)| index).collect()
}

fn command_match_score(label: &str, query: &str) -> Option<i64> {
    if query.is_empty() {
        return Some(0);
    }
    let query = query.to_lowercase();
    let label = label.to_lowercase();
    if label.starts_with(&query) {
        return Some(10_000 - i64::try_from(label.len()).unwrap_or(i64::MAX));
    }
    if let Some(index) = label.find(&query) {
        return Some(
            7_500
                - i64::try_from(index).unwrap_or(i64::MAX)
                - i64::try_from(label.len()).unwrap_or(i64::MAX),
        );
    }
    let mut query_chars = query.chars();
    let mut wanted = query_chars.next()?;
    let mut score = 0i64;
    let mut previous = None;
    for (index, ch) in label.chars().enumerate() {
        if ch != wanted {
            continue;
        }
        score += 100;
        if previous == Some(index.saturating_sub(1)) {
            score += 60;
        }
        previous = Some(index);
        if let Some(next) = query_chars.next() {
            wanted = next;
        } else {
            return Some(score - i64::try_from(label.len()).unwrap_or(i64::MAX));
        }
    }
    None
}

#[cfg(test)]
#[path = "../../tests/unit/editor/command_palette.rs"]
mod tests;
