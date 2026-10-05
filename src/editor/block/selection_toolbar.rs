// @author kongweiguang

//! Contextual formatting toolbar for a simple single-block text selection.

use std::ops::Range;

use gpui::prelude::FluentBuilder;
use gpui::*;

use super::{
    Block, BlockEvent, BlockHostAction, BlockRecord, EditingCommandId, EditingContext,
    EditingSelectionContext, INLINE_COMMANDS, InlineFormat, TRANSFORM_COMMANDS, UndoCaptureKind,
};
use crate::components::markdown::inline::StyleFlag;
use crate::i18n::{I18nManager, I18nStrings};
use crate::theme::{Theme, workbench::SurfaceKind};
use crate::ui::visual_preferences::VisualPreferencesManager;

// 宽度与按钮、间距和 2px 面板内边距严格对应，避免尾部留下额外空白。
const TOOLBAR_WITHOUT_BLOCK_TYPE_WIDTH: f32 = 182.0;
const TOOLBAR_COMPACT_WIDTH: f32 = 226.0;
const TOOLBAR_BLOCK_TYPE_CHINESE_WIDTH: f32 = 82.0;
const TOOLBAR_BLOCK_TYPE_DEFAULT_WIDTH: f32 = 106.0;
const TOOLBAR_HEIGHT: f32 = 32.0;
const TOOLBAR_GAP: f32 = 6.0;
const OVERFLOW_MENU_HEIGHT: f32 = 174.0;
const VIEWPORT_INSET: f32 = 8.0;
const CODE_ICON: &str = "icon/ui/code.svg";
const LINK_ICON: &str = "icon/ui/link.svg";
const MORE_ICON: &str = "icon/ui/more-horizontal.svg";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ToolbarCommand {
    BlockType,
    Bold,
    Italic,
    Strikethrough,
    Code,
    Link,
    Overflow,
    Underline,
    Highlight,
    Superscript,
    Subscript,
    InlineMath,
    ClearFormatting,
}

impl ToolbarCommand {
    const PRIMARY: [Self; 7] = [
        Self::BlockType,
        Self::Bold,
        Self::Italic,
        Self::Strikethrough,
        Self::Code,
        Self::Link,
        Self::Overflow,
    ];

    fn id(self) -> &'static str {
        match self {
            Self::BlockType => "block-type",
            Self::Bold => "bold",
            Self::Italic => "italic",
            Self::Strikethrough => "strikethrough",
            Self::Code => "code",
            Self::Link => "link",
            Self::Overflow => "overflow",
            Self::Underline => "underline",
            Self::Highlight => "highlight",
            Self::Superscript => "superscript",
            Self::Subscript => "subscript",
            Self::InlineMath => "inline-math",
            Self::ClearFormatting => "clear-formatting",
        }
    }

    fn symbol(self) -> &'static str {
        match self {
            Self::BlockType => "T",
            Self::Bold => "B",
            Self::Italic => "I",
            Self::Strikethrough => "S",
            Self::Code => "<>",
            Self::Link => "[]",
            Self::Overflow => "...",
            Self::Underline => "U",
            Self::Highlight => "==",
            Self::Superscript => "x²",
            Self::Subscript => "x₂",
            Self::InlineMath => "$",
            Self::ClearFormatting => "Tx",
        }
    }

    fn label(self, strings: &I18nStrings) -> String {
        match self {
            Self::BlockType => strings
                .slash_commands
                .get("paragraph")
                .cloned()
                .unwrap_or_else(|| "Paragraph".to_owned()),
            Self::Bold => strings.selection_toolbar_bold.clone(),
            Self::Italic => strings.selection_toolbar_italic.clone(),
            Self::Strikethrough => strings.selection_toolbar_strikethrough.clone(),
            Self::Code => strings.selection_toolbar_inline_code.clone(),
            Self::Link => strings.selection_toolbar_link.clone(),
            Self::Overflow => strings.selection_toolbar_more.clone(),
            Self::Underline => strings.selection_toolbar_underline.clone(),
            Self::Highlight => strings
                .slash_commands
                .get("highlight")
                .cloned()
                .unwrap_or_else(|| "Highlight".to_owned()),
            Self::Superscript => strings
                .slash_commands
                .get("superscript")
                .cloned()
                .unwrap_or_else(|| "Superscript".to_owned()),
            Self::Subscript => strings
                .slash_commands
                .get("subscript")
                .cloned()
                .unwrap_or_else(|| "Subscript".to_owned()),
            Self::InlineMath => strings
                .slash_commands
                .get("inline_math")
                .cloned()
                .unwrap_or_else(|| "Inline Math".to_owned()),
            Self::ClearFormatting => strings
                .slash_commands
                .get("clear_formatting")
                .cloned()
                .unwrap_or_else(|| "Clear Formatting".to_owned()),
        }
    }

    fn editing_command(self) -> Option<EditingCommandId> {
        match self {
            Self::BlockType => None,
            Self::Bold => Some(EditingCommandId::Bold),
            Self::Italic => Some(EditingCommandId::Italic),
            Self::Underline => Some(EditingCommandId::Underline),
            Self::Highlight => Some(EditingCommandId::Highlight),
            Self::Superscript => Some(EditingCommandId::Superscript),
            Self::Subscript => Some(EditingCommandId::Subscript),
            Self::InlineMath => Some(EditingCommandId::InlineMath),
            Self::Strikethrough => Some(EditingCommandId::Strikethrough),
            Self::Code => Some(EditingCommandId::InlineCode),
            Self::Link => Some(EditingCommandId::Link),
            Self::ClearFormatting => Some(EditingCommandId::ClearFormatting),
            Self::Overflow => None,
        }
    }
}

#[path = "selection_toolbar_parts/controller.rs"]
mod controller;

#[derive(Clone, Copy, Debug, PartialEq)]
struct ToolbarPosition {
    left: f32,
    top: f32,
    above: bool,
}

/// 区分布局尚未测量与已测量但无法显示工具栏，供绘制和键盘入口共用。
#[derive(Clone, Copy, Debug, PartialEq)]
enum SelectionToolbarGeometry {
    Unmeasured,
    Hidden,
    Positioned(ToolbarPosition),
}

/// 依据文字行与所属视口的交集选取紧凑宽度，避免浮层覆盖到相邻窗格。
fn selection_toolbar_width(
    horizontal_bounds: Bounds<Pixels>,
    viewport: Bounds<Pixels>,
    expanded_block_type_width: Option<f32>,
) -> f32 {
    let Some(expanded_block_type_width) = expanded_block_type_width else {
        return TOOLBAR_WITHOUT_BLOCK_TYPE_WIDTH;
    };
    let left_edge = (f32::from(horizontal_bounds.left()) + VIEWPORT_INSET)
        .max(f32::from(viewport.left()) + VIEWPORT_INSET);
    let right_edge =
        f32::from(horizontal_bounds.right()).min(f32::from(viewport.right())) - VIEWPORT_INSET;
    let expanded_width = TOOLBAR_WITHOUT_BLOCK_TYPE_WIDTH + 2.0 + expanded_block_type_width;
    if right_edge - left_edge >= expanded_width {
        expanded_width
    } else {
        TOOLBAR_COMPACT_WIDTH
    }
}

fn expanded_block_type_width(language_id: &str) -> f32 {
    if language_id.starts_with("zh") {
        TOOLBAR_BLOCK_TYPE_CHINESE_WIDTH
    } else {
        TOOLBAR_BLOCK_TYPE_DEFAULT_WIDTH
    }
}

/// 零尺寸只会出现在所属滚动面首次布局前；此时用窗口范围保留首帧工具栏。
fn resolved_toolbar_viewport(
    owner_viewport: Option<Bounds<Pixels>>,
    window_viewport: Size<Pixels>,
) -> Bounds<Pixels> {
    owner_viewport
        .filter(|bounds| bounds.size.width > px(0.0) && bounds.size.height > px(0.0))
        .unwrap_or_else(|| Bounds::new(point(px(0.0), px(0.0)), window_viewport))
}

/// 用裁剪后的可见选区定位工具栏；无交集时不生成浮层，保留选区供滚回后恢复。
fn toolbar_window_position(
    selection: Bounds<Pixels>,
    horizontal_bounds: Bounds<Pixels>,
    viewport: Bounds<Pixels>,
    attached_surface_height: f32,
    expanded_block_type_width: Option<f32>,
) -> Option<ToolbarPosition> {
    let visible_selection = selection.intersect(&viewport);
    if visible_selection.size.width <= px(0.0) || visible_selection.size.height <= px(0.0) {
        return None;
    }
    let viewport_left = f32::from(viewport.left());
    let viewport_right = f32::from(viewport.right());
    let viewport_top = f32::from(viewport.top());
    let viewport_bottom = f32::from(viewport.bottom());
    let safe_left = viewport_left + VIEWPORT_INSET;
    let safe_right = viewport_right - VIEWPORT_INSET;
    let safe_top = viewport_top + VIEWPORT_INSET;
    let safe_bottom = viewport_bottom - VIEWPORT_INSET;
    let toolbar_width =
        selection_toolbar_width(horizontal_bounds, viewport, expanded_block_type_width);
    if safe_right - safe_left < toolbar_width || safe_bottom - safe_top < TOOLBAR_HEIGHT {
        return None;
    }

    let line_left = (f32::from(horizontal_bounds.left()) + VIEWPORT_INSET).max(safe_left);
    let line_right = f32::from(horizontal_bounds.right()).min(viewport_right) - VIEWPORT_INSET;
    let line_max_left = line_right - toolbar_width;
    let max_left = if line_max_left >= line_left {
        line_max_left.min(safe_right - toolbar_width)
    } else {
        safe_right - toolbar_width
    };
    let min_left = if line_max_left >= line_left {
        line_left
    } else {
        safe_left
    };
    let ideal_left = f32::from(visible_selection.center().x) - toolbar_width / 2.0;
    let left = ideal_left.clamp(min_left, max_left);
    let required_height = TOOLBAR_HEIGHT
        + if attached_surface_height > 0.0 {
            TOOLBAR_GAP + attached_surface_height
        } else {
            0.0
        };
    let available_above = (f32::from(visible_selection.top()) - safe_top).max(0.0);
    let available_below = (safe_bottom - f32::from(visible_selection.bottom())).max(0.0);
    let above = available_above >= required_height
        || (available_below < required_height && available_above > available_below);
    let top = if above {
        f32::from(visible_selection.top()) - TOOLBAR_HEIGHT - TOOLBAR_GAP
    } else {
        f32::from(visible_selection.bottom()) + TOOLBAR_GAP
    }
    .clamp(safe_top, safe_bottom - TOOLBAR_HEIGHT);
    Some(ToolbarPosition { left, top, above })
}

/// 让工具栏菜单与工具栏共享同一正文边界，避免菜单展开到其它窗格或窗口装饰上。
fn attached_surface_placement(
    position: ToolbarPosition,
    surface_height: f32,
    viewport: Bounds<Pixels>,
) -> (bool, f32) {
    let safe_top = f32::from(viewport.top()) + VIEWPORT_INSET;
    let safe_bottom = f32::from(viewport.bottom()) - VIEWPORT_INSET;
    let available_above = (position.top - TOOLBAR_GAP - safe_top).max(0.0);
    let available_below = (safe_bottom - position.top - TOOLBAR_HEIGHT - TOOLBAR_GAP).max(0.0);
    // 菜单与工具栏共享所属滚动面的裁剪范围，因此按该矩形两侧空间决定方向。
    let opens_above = available_above >= surface_height
        || (available_below < surface_height && available_above > available_below);
    let available_height = if opens_above {
        available_above
    } else {
        available_below
    };
    (opens_above, available_height)
}

impl Block {
    /// 只保存有效的所属滚动范围；首次布局的零尺寸留给窗口兜底，避免隐藏正常工具栏。
    pub(crate) fn set_selection_toolbar_viewport(&mut self, viewport: Option<Bounds<Pixels>>) {
        let viewport =
            viewport.filter(|bounds| bounds.size.width > px(0.0) && bounds.size.height > px(0.0));
        if self.selection_toolbar_viewport != viewport {
            self.selection_toolbar_viewport = viewport;
        }
    }

    /// 绘制与快捷键共用同一位置判断，并保留“尚未测量”以兼容首次布局状态。
    fn selection_toolbar_geometry(
        &self,
        window_viewport: Size<Pixels>,
        language_id: &str,
    ) -> SelectionToolbarGeometry {
        let Some(selection) = self.active_range_or_cursor_bounds() else {
            return SelectionToolbarGeometry::Unmeasured;
        };
        let Some(text_bounds) = self.last_bounds else {
            return SelectionToolbarGeometry::Unmeasured;
        };
        let attached_surface_height = if self.selection_toolbar_type_menu_open {
            312.0
        } else if self.selection_toolbar_overflow_open {
            OVERFLOW_MENU_HEIGHT
        } else if self.selection_toolbar_link_input.is_some() {
            42.0
        } else {
            0.0
        };
        let show_block_type = !self.is_table_cell() && self.editor_selection_range.is_none();
        let expanded_block_type_width =
            show_block_type.then(|| expanded_block_type_width(language_id));
        let viewport = resolved_toolbar_viewport(self.selection_toolbar_viewport, window_viewport);

        match toolbar_window_position(
            selection,
            text_bounds,
            viewport,
            attached_surface_height,
            expanded_block_type_width,
        ) {
            Some(position) => SelectionToolbarGeometry::Positioned(position),
            None => SelectionToolbarGeometry::Hidden,
        }
    }

    /// Closes only transient children of contextual editing UI. The text
    /// selection and base selection toolbar remain intact so an outside click
    /// can establish the next caret without silently discarding user state.
    /// 浮层实体与焦点缓存同时释放，迟到回调不能把焦点归还给已关闭的输入。
    pub(crate) fn dismiss_contextual_editing_popovers(&mut self) -> bool {
        let had_transient = self.dismiss_slash_menu()
            || self.selection_toolbar_keyboard_active
            || self.selection_toolbar_overflow_open
            || self.selection_toolbar_type_menu_open
            || self.selection_toolbar_link_input.is_some();
        self.selection_toolbar_keyboard_active = false;
        self.selection_toolbar_overflow_open = false;
        self.selection_toolbar_type_menu_open = false;
        self.selection_toolbar_link_input = None;
        self.selection_toolbar_link_focus = None;
        self.selection_toolbar_link_range = None;
        self.selection_toolbar_link_had_target = false;
        had_transient
    }

    fn selection_toolbar_range(&self) -> Option<Range<usize>> {
        if self.is_read_only()
            || self.uses_raw_text_editing()
            || self.marked_range.is_some()
            || self.showing_rendered_image()
        {
            return None;
        }
        let range = if let Some(range) = self.editor_selection_range.clone() {
            if !self.editor_selection_supports_inline_commands {
                return None;
            }
            self.current_to_clean_range(range)
        } else {
            if self.selected_range.is_empty() {
                return None;
            }
            self.selection_clean_range()
        };
        self.record
            .title
            .selection_supports_toolbar(range.clone())
            .then_some(range)
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/components/block/selection_toolbar.rs"]
mod tests;
