// @author kongweiguang

//! Non-modal, source-authoritative document find and replace.

use std::ops::Range;
use std::sync::Arc;
use std::time::Duration;

use gmark_document::{Revision, TextEdit, Transaction};
use gmark_markdown::{Replaceability, parse_markdown};
use gpui::*;
use regex::{Regex, RegexBuilder};

use super::markdown_view_state;
use super::{Block, BlockRecord, Editor, PreparedSplitProjection, UndoSelectionSnapshot, ViewMode};
use crate::components::{BlockEvent, UndoCaptureKind};
use crate::i18n::{I18nManager, I18nStrings};
use crate::theme::{Theme, workbench::SurfaceKind};

const FIND_DEBOUNCE: Duration = Duration::from_millis(40);
const TOOLTIP_DELAY: Duration = Duration::from_millis(500);
const MAX_FIND_QUERY_BYTES: usize = 4 * 1024;
const MAX_REPLACE_OUTPUT_BYTES: usize = 64 * 1024 * 1024;
const MAX_FIND_MATCHES: usize = 20_000;
const FIND_CASE_ICON: &str = "icon/ui/case-sensitive.svg";
const FIND_WORD_ICON: &str = "icon/ui/whole-word.svg";
const FIND_REGEX_ICON: &str = "icon/ui/regex.svg";
const CHEVRON_UP_ICON: &str = "icon/ui/chevron-up.svg";
const CHEVRON_DOWN_ICON: &str = "icon/ui/chevron-down.svg";
const CLOSE_ICON: &str = "icon/ui/close.svg";

/// 后台查找/替换结果只有同时匹配 revision 与 generation 才能进入 UI 事务。
pub(super) fn find_replace_result_is_current(
    expected_revision: Revision,
    expected_generation: u64,
    current_revision: Revision,
    current_generation: u64,
) -> bool {
    expected_revision == current_revision && expected_generation == current_generation
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct FindOptions {
    pub(super) case_sensitive: bool,
    pub(super) whole_word: bool,
    pub(super) regex: bool,
}

pub(super) struct FindPanelState {
    pub(super) query: Entity<Block>,
    pub(super) replacement: Entity<Block>,
    pub(super) show_replace: bool,
    pub(super) options: FindOptions,
    pub(super) matches: Vec<Range<usize>>,
    pub(super) match_metadata: Vec<FindMatchMetadata>,
    pub(super) selected: usize,
    pub(super) error: Option<String>,
    pub(super) truncated: bool,
    pub(super) revision: Revision,
    generation: u64,
    task: Option<Task<()>>,
    replace_task: Option<Task<()>>,
    tooltip_hovered: Option<&'static str>,
    pub(super) tooltip_visible: Option<&'static str>,
    tooltip_task: Option<Task<()>>,
    keyboard_target: FindKeyboardTarget,
    focus_handle: FocusHandle,
    restore_focus: Option<FindRestoreFocus>,
    /// Original rendered fold choices restored when find closes. Finding may
    /// temporarily reveal a match inside a collapsed region.
    pub(super) view_state_before_expand: Option<markdown_view_state::MarkdownViewState>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct FindRestoreFocus {
    pub(super) surface: super::selection_surface::SelectionSurface,
    pub(super) entity_id: EntityId,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum FindKeyboardTarget {
    #[default]
    Query,
    Replacement,
    CaseSensitive,
    WholeWord,
    Regex,
    Previous,
    Next,
    Close,
}

impl FindKeyboardTarget {
    fn is_control(self) -> bool {
        !matches!(self, Self::Query | Self::Replacement)
    }
}

#[path = "find_replace_parts/controller.rs"]
mod controller;
#[path = "find_replace_parts/transactions.rs"]
mod transactions;

pub(super) struct FindResult {
    pub(super) revision: Revision,
    pub(super) matches: Vec<Range<usize>>,
    pub(super) match_metadata: Vec<FindMatchMetadata>,
    pub(super) error: Option<String>,
    pub(super) truncated: bool,
}

struct ReplaceAllPlan {
    edits: Vec<TextEdit>,
    selection: Range<usize>,
}

/// 在后台构造完整的最终源码，先验证最终字节上限和所有范围，再交给主线程以单个
/// revision gate 提交；这样主线程只处理一个全源码 edit，超限或异常也不会留下半组替换。
pub(super) fn build_replace_all_plan(
    source: &str,
    query: &str,
    replacement_template: &str,
    options: FindOptions,
    matches: &[Range<usize>],
    metadata: &[FindMatchMetadata],
) -> Result<ReplaceAllPlan, String> {
    if query.len() > MAX_FIND_QUERY_BYTES {
        return Err(format!(
            "查找内容超过 {} KiB 安全限制",
            MAX_FIND_QUERY_BYTES / 1024
        ));
    }
    if replacement_template.len() > MAX_REPLACE_OUTPUT_BYTES {
        return Err("替换结果超过 64 MiB 安全限制".to_owned());
    }
    if source.len() > MAX_REPLACE_OUTPUT_BYTES {
        return Err("替换结果超过 64 MiB 安全限制".to_owned());
    }
    let regex = compile_find_regex(query, options).map_err(|error| error.to_string())?;
    let mut replacements = Vec::with_capacity(matches.len());
    let mut output_len = source.len();
    let mut previous_end = 0;

    for (index, range) in matches.iter().enumerate() {
        let Some(metadata) = metadata.get(index) else {
            return Err("查找结果元数据不完整，替换已取消".to_owned());
        };
        if metadata.replaceability != gmark_markdown::Replaceability::Direct {
            continue;
        }
        if metadata.source.as_ref() != Some(range) {
            return Err("查找结果与源码范围不一致，替换已取消".to_owned());
        }
        if range.start > range.end
            || range.end > source.len()
            || range.start < previous_end
            || !source.is_char_boundary(range.start)
            || !source.is_char_boundary(range.end)
        {
            return Err("查找结果范围无效，替换已取消".to_owned());
        }
        let Some(replacement) = replacement_for_range(
            &regex,
            source,
            range.clone(),
            replacement_template,
            options.regex,
        ) else {
            return Err("替换模板与查找结果不匹配，替换已取消".to_owned());
        };
        if replacement.len() > MAX_REPLACE_OUTPUT_BYTES {
            return Err("替换结果超过 64 MiB 安全限制".to_owned());
        }
        output_len = checked_replace_output_len(output_len, range, replacement.len())?;
        previous_end = range.end;
        replacements.push((range.clone(), replacement));
    }

    if replacements.is_empty() {
        return Ok(ReplaceAllPlan {
            edits: Vec::new(),
            selection: 0..0,
        });
    }

    let mut final_source = String::with_capacity(output_len);
    let mut source_cursor = 0;
    let mut first_selection = None;
    for (range, replacement) in replacements {
        final_source.push_str(&source[source_cursor..range.start]);
        let selection_start = final_source.len();
        let selection_end = selection_start
            .checked_add(replacement.len())
            .ok_or_else(|| "替换后的光标范围溢出，替换已取消".to_owned())?;
        final_source.push_str(&replacement);
        if first_selection.is_none() {
            first_selection = Some(selection_start..selection_end);
        }
        source_cursor = range.end;
    }
    final_source.push_str(&source[source_cursor..]);
    if final_source.len() != output_len {
        return Err("替换结果长度校验失败，替换已取消".to_owned());
    }

    Ok(ReplaceAllPlan {
        edits: vec![TextEdit::new(0..source.len(), final_source)],
        selection: first_selection.unwrap_or(0..0),
    })
}

/// 用 checked 算术计算一组替换后的文档长度，避免极端范围把安全上限检查绕成整数溢出。
fn checked_replace_output_len(
    source_len: usize,
    range: &Range<usize>,
    replacement_len: usize,
) -> Result<usize, String> {
    let removed = range
        .end
        .checked_sub(range.start)
        .ok_or_else(|| "替换范围长度溢出，替换已取消".to_owned())?;
    let output_len = source_len
        .checked_sub(removed)
        .and_then(|length| length.checked_add(replacement_len))
        .ok_or_else(|| "替换结果长度溢出，替换已取消".to_owned())?;
    if output_len > MAX_REPLACE_OUTPUT_BYTES {
        return Err("替换结果超过 64 MiB 安全限制".to_owned());
    }
    Ok(output_len)
}

/// Rendered find metadata kept parallel to the source ranges used by the
/// existing selection/navigation machinery.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct FindMatchMetadata {
    pub(super) visible: Range<usize>,
    pub(super) source: Option<Range<usize>>,
    pub(super) replaceability: Replaceability,
}

impl Editor {
    /// Captures the actual focused text surface before Find takes focus from its source block.
    fn capture_find_restore_focus(&self, window: &Window, cx: &App) -> Option<FindRestoreFocus> {
        if let Some((surface, block)) = self.focused_document_target(window, cx) {
            return Some(FindRestoreFocus {
                surface,
                entity_id: block.entity_id(),
            });
        }
        let surface = self.active_selection_surface;
        let entity_id = match surface {
            super::selection_surface::SelectionSurface::Main => self.active_entity_id,
            super::selection_surface::SelectionSurface::SplitPreview => self
                .cross_block_selection_for_surface(surface)
                .map(|selection| selection.focus.entity_id),
        }?;
        self.selection_surface_contains_entity(surface, entity_id)
            .then_some(FindRestoreFocus { surface, entity_id })
    }

    /// 打开查找时从当前聚焦表面生成查询文本，避免旧源码快照误取其他窗格的选区。
    ///
    /// Live、Preview 与 Split 右侧使用可见文本映射；Source 与 Split Main 保留原始 Markdown。
    /// 查找接管焦点时清除模式切换遗留的待办焦点，关闭时再按本次捕获的目标恢复。
    /// 焦点已转移到工具界面时只使用本次捕获的恢复目标，避免陈旧快照造成跨窗格串选。
    fn open_find_panel(&mut self, show_replace: bool, window: &mut Window, cx: &mut Context<Self>) {
        let restore_focus = self.capture_find_restore_focus(window, cx);
        self.close_menu_bar(cx);
        self.dismiss_contextual_overlays(cx);
        self.pending_focus = None;
        if let Some(state) = self.find_panel.as_mut() {
            state.show_replace |= show_replace;
            if show_replace {
                state.keyboard_target = FindKeyboardTarget::Replacement;
                state.replacement.read(cx).focus_handle.focus(window);
            } else {
                state.keyboard_target = FindKeyboardTarget::Query;
                state.query.read(cx).focus_handle.focus(window);
            }
            cx.notify();
            return;
        }

        let source_revision = self.source_document.revision();
        let selection_target = self.focused_document_target(window, cx).or_else(|| {
            let restore_focus = restore_focus?;
            self.selection_surface_entities(restore_focus.surface)
                .into_iter()
                .find(|target| target.entity_id() == restore_focus.entity_id)
                .map(|target| (restore_focus.surface, target))
        });
        let initial_query = selection_target
            .map(|(surface, target)| {
                if self.view_mode == ViewMode::Source
                    || (self.view_mode == ViewMode::Split
                        && surface == super::selection_surface::SelectionSurface::Main)
                {
                    let block = target.read(cx);
                    block
                        .display_text()
                        .get(block.selected_range.clone())
                        .map(str::to_owned)
                        .unwrap_or_default()
                } else {
                    self.selected_visible_text_for_target(surface, &target, cx)
                        .unwrap_or_default()
                }
            })
            .filter(|text| !text.contains(['\r', '\n']) && text.len() <= 256)
            .unwrap_or_default();
        let strings = cx.global::<I18nManager>().strings_arc();
        let query_placeholder = strings.find_query_placeholder.clone();
        let query = cx.new(|cx| {
            let mut block = Block::with_record(cx, BlockRecord::paragraph(initial_query));
            block.set_source_raw_mode();
            block.set_input_placeholder(query_placeholder);
            block
        });
        let replace_placeholder = strings.find_replace_placeholder.clone();
        let replacement = cx.new(|cx| {
            let mut block = Block::with_record(cx, BlockRecord::paragraph(String::new()));
            block.set_source_raw_mode();
            block.set_input_placeholder(replace_placeholder);
            block
        });
        cx.subscribe(&query, Self::on_find_panel_input_event)
            .detach();
        cx.subscribe(&replacement, Self::on_find_panel_input_event)
            .detach();
        query.read(cx).focus_handle.focus(window);
        let find_focus_handle = cx.focus_handle();
        self.find_panel = Some(FindPanelState {
            query,
            replacement,
            show_replace,
            options: FindOptions::default(),
            matches: Vec::new(),
            match_metadata: Vec::new(),
            selected: 0,
            error: None,
            truncated: false,
            revision: source_revision,
            generation: 0,
            task: None,
            replace_task: None,
            tooltip_hovered: None,
            tooltip_visible: None,
            tooltip_task: None,
            keyboard_target: FindKeyboardTarget::Query,
            focus_handle: find_focus_handle,
            restore_focus,
            view_state_before_expand: None,
        });
        self.schedule_find(cx);
        cx.notify();
    }

    fn set_find_tooltip_hover(&mut self, id: &'static str, hovered: bool, cx: &mut Context<Self>) {
        let Some(state) = self.find_panel.as_mut() else {
            return;
        };
        state.tooltip_task = None;
        state.tooltip_hovered = hovered.then_some(id);
        state.tooltip_visible = None;
        if hovered {
            state.tooltip_task = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(TOOLTIP_DELAY).await;
                let _ = this.update(cx, |editor, cx| {
                    let Some(state) = editor.find_panel.as_mut() else {
                        return;
                    };
                    if state.tooltip_hovered == Some(id) {
                        state.tooltip_visible = Some(id);
                        state.tooltip_task = None;
                        cx.notify();
                    }
                });
            }));
        }
        cx.notify();
    }

    fn on_find_panel_input_event(
        &mut self,
        input: Entity<Block>,
        event: &BlockEvent,
        cx: &mut Context<Self>,
    ) {
        if !matches!(event, BlockEvent::Changed) {
            return;
        }
        let is_query = self
            .find_panel
            .as_ref()
            .is_some_and(|state| state.query.entity_id() == input.entity_id());
        if is_query {
            self.schedule_find(cx);
        } else {
            cx.notify();
        }
    }

    /// 先在 UI 线程拒绝超长查询并用 revision/generation 标记后台搜索，避免极端输入
    /// 把正则扫描和过期结果带回编辑器主线程。
    pub(super) fn schedule_find(&mut self, cx: &mut Context<Self>) {
        let anchor = self.capture_source_selection_snapshot(cx).range().end;
        let Some(state) = self.find_panel.as_mut() else {
            return;
        };
        state.generation = state.generation.wrapping_add(1);
        state.task = None;
        // A query change invalidates an in-flight replacement plan as well; dropping
        // that task prevents a background result from committing against new input.
        state.replace_task = None;
        state.error = None;
        let generation = state.generation;
        let query = state.query.read(cx).shared_display_text();
        if query.len() > MAX_FIND_QUERY_BYTES {
            state.matches.clear();
            state.match_metadata.clear();
            state.selected = 0;
            state.truncated = false;
            state.error = Some(format!(
                "查找内容超过 {} KiB 安全限制",
                MAX_FIND_QUERY_BYTES / 1024
            ));
            cx.notify();
            return;
        }
        if query.is_empty() {
            state.matches.clear();
            state.match_metadata.clear();
            state.selected = 0;
            state.truncated = false;
            cx.notify();
            return;
        }
        let options = state.options;
        let rendered = matches!(
            self.view_mode,
            super::ViewMode::Rendered | super::ViewMode::Preview | super::ViewMode::Split
        );
        let snapshot = self.source_document.snapshot();
        state.task = Some(cx.spawn(async move |this: WeakEntity<Self>, cx| {
            cx.background_executor().timer(FIND_DEBOUNCE).await;
            let result = cx
                .background_spawn(async move {
                    find_matches_for_view(
                        &snapshot.text(),
                        query.as_ref(),
                        options,
                        snapshot.revision(),
                        rendered,
                    )
                })
                .await;
            let _ = this.update(cx, |editor, cx| {
                let current_revision = editor.source_document.revision();
                let stale = {
                    let Some(state) = editor.find_panel.as_mut() else {
                        return;
                    };
                    if state.generation != generation {
                        return;
                    }
                    state.task = None;
                    if result.revision != current_revision {
                        true
                    } else {
                        state.revision = result.revision;
                        state.error = result.error;
                        state.truncated = result.truncated;
                        state.matches = result.matches;
                        state.match_metadata = result.match_metadata;
                        state.selected = state
                            .matches
                            .iter()
                            .position(|range| range.start >= anchor)
                            .unwrap_or(0);
                        false
                    }
                };
                if stale {
                    editor.schedule_find(cx);
                    return;
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
}

fn render_find_tooltip(
    label: String,
    theme: &Theme,
    visual_preferences: crate::theme::workbench::ResolvedVisualPreferences,
) -> AnyElement {
    let palette = &theme.colors.workbench;
    let material = palette.material(SurfaceKind::Glass, visual_preferences);
    div()
        .id("document-find-tooltip")
        .debug_selector(|| "document-find-tooltip".to_owned())
        .absolute()
        .top(px(30.0))
        .left(px(0.0))
        .min_w(px(92.0))
        .h(px(26.0))
        .px(px(8.0))
        .flex()
        .items_center()
        .justify_center()
        .whitespace_nowrap()
        .rounded(px(5.0))
        .bg(material.background)
        .border(px(theme.dimensions.dialog_border_width))
        .border_color(material.border)
        .shadow_md()
        .text_size(px(theme.dimensions.status_bar_text_size))
        .text_color(palette.text_primary)
        .child(label)
        .into_any_element()
}

pub(super) fn compile_find_regex(query: &str, options: FindOptions) -> Result<Regex, regex::Error> {
    let pattern = if options.regex {
        query.to_owned()
    } else {
        regex::escape(query)
    };
    RegexBuilder::new(&pattern)
        .case_insensitive(!options.case_sensitive)
        .unicode(true)
        .build()
}

// Reason: this facade preserves the editor search contract. Remove when all callers use the view-aware entry point.
#[allow(dead_code)]
pub(super) fn find_matches(
    source: &str,
    query: &str,
    options: FindOptions,
    revision: Revision,
) -> FindResult {
    find_matches_for_view(source, query, options, revision, false)
}

/// 在后台查询前复用统一的字节上限和 20,000 条匹配门禁，让渲染视图与源码视图
/// 对同一输入返回一致的可恢复错误，而不是继续构造无界结果。
pub(super) fn find_matches_for_view(
    source: &str,
    query: &str,
    options: FindOptions,
    revision: Revision,
    rendered: bool,
) -> FindResult {
    if query.len() > MAX_FIND_QUERY_BYTES {
        return FindResult {
            revision,
            matches: Vec::new(),
            match_metadata: Vec::new(),
            error: Some(format!(
                "查找内容超过 {} KiB 安全限制",
                MAX_FIND_QUERY_BYTES / 1024
            )),
            truncated: false,
        };
    }
    if rendered {
        let document = parse_markdown(source);
        let projection = document.visible_text_projection();
        let result = find_text_ranges(&projection.text, query, options);
        let (visible_matches, error, truncated) = match result {
            Ok(value) => (value.0, None, value.1),
            Err(error) => (Vec::new(), Some(error), false),
        };
        let mut matches = Vec::with_capacity(visible_matches.len());
        let mut match_metadata = Vec::with_capacity(visible_matches.len());
        for visible in visible_matches {
            let direct = projection.source_range_for_visible(visible.clone());
            let Some(source_range) =
                direct.or_else(|| projection.source_bounds_for_visible(visible.clone()))
            else {
                continue;
            };
            matches.push(source_range.start..source_range.end);
            match_metadata.push(FindMatchMetadata {
                visible,
                source: Some(source_range.start..source_range.end),
                replaceability: direct
                    .map(|_| Replaceability::Direct)
                    .unwrap_or(Replaceability::Derived),
            });
        }
        return FindResult {
            revision,
            matches,
            match_metadata,
            error,
            truncated,
        };
    }

    let result = find_text_ranges(source, query, options);
    let (matches, error, truncated) = match result {
        Ok(value) => (value.0, None, value.1),
        Err(error) => (Vec::new(), Some(error), false),
    };
    let match_metadata = matches
        .iter()
        .cloned()
        .map(|range| FindMatchMetadata {
            visible: range.clone(),
            source: Some(range),
            replaceability: Replaceability::Direct,
        })
        .collect();
    FindResult {
        revision,
        matches,
        match_metadata,
        error,
        truncated,
    }
}

fn find_text_ranges(
    source: &str,
    query: &str,
    options: FindOptions,
) -> Result<(Vec<Range<usize>>, bool), String> {
    let regex = match compile_find_regex(query, options) {
        Ok(regex) => regex,
        Err(error) => return Err(error.to_string()),
    };
    let mut matches = Vec::new();
    let mut truncated = false;
    for found in regex.find_iter(source) {
        let range = found.start()..found.end();
        if options.whole_word && !has_word_boundaries(source, &range) {
            continue;
        }
        if matches.len() == MAX_FIND_MATCHES {
            truncated = true;
            break;
        }
        matches.push(range);
    }
    Ok((matches, truncated))
}

fn has_word_boundaries(source: &str, range: &Range<usize>) -> bool {
    let left_is_word = source[..range.start]
        .chars()
        .next_back()
        .is_some_and(is_word_character);
    let start_is_word = source[range.start..]
        .chars()
        .next()
        .is_some_and(is_word_character);
    let end_is_word = source[..range.end]
        .chars()
        .next_back()
        .is_some_and(is_word_character);
    let right_is_word = source[range.end..]
        .chars()
        .next()
        .is_some_and(is_word_character);
    (!left_is_word || !start_is_word) && (!right_is_word || !end_is_word)
}

fn is_word_character(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_'
}

pub(super) fn replacement_for_range(
    regex: &Regex,
    source: &str,
    range: Range<usize>,
    template: &str,
    expand_captures: bool,
) -> Option<String> {
    if !expand_captures {
        return Some(template.to_owned());
    }
    let captures = regex.captures_at(source, range.start)?;
    let matched = captures.get(0)?;
    if matched.start() != range.start || matched.end() != range.end {
        return None;
    }
    let mut replacement = String::new();
    captures.expand(template, &mut replacement);
    Some(replacement)
}

#[cfg(test)]
#[path = "../../tests/unit/editor/find_replace_limits.rs"]
mod tests;

#[cfg(test)]
#[path = "../../tests/unit/editor/find_replace_surface.rs"]
mod surface_tests;
