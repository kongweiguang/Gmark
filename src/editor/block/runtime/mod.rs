// @author kongweiguang
//! Editable block runtime and block-local state transitions.

use std::collections::HashMap;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gpui::*;
use unicode_segmentation::*;

mod auto_pair;
mod code;
mod image;
pub(crate) use image::{ImageResizeSession, ImageRuntime};
mod mermaid;
mod projection;
mod resource;
mod source_layout;
mod table;

pub(crate) use source_layout::{SourceLayoutCacheKey, SourceLayoutIdentity};

use self::projection::{
    ExpandedInlineProjection, ExpandedInlineSegment, ExpandedInlineSegmentKind, ExpandedLinkRun,
    ProjectedLinkSelectionSnapshot,
};
use self::resource::ResourceRuntime;
use super::element;
use super::{
    BlockEvent, BlockHostAction, BlockKind, BlockRecord, CalloutVariant, CodeFenceOpening,
    EditingCommandId, FootnoteRegistry, SlashMenuState, UndoCaptureKind,
};
use super::{
    CodeHighlightResult, SourceSyntaxContext, highlight_code_block, project_highlight_result,
};
use super::{
    ImageReferenceDefinitions, ImageResolvedSource, ImageSyntax, LinkReferenceDefinitions,
    parse_standalone_image, resolve_image_source, rewrite_standalone_image_width,
};
#[cfg(test)]
use crate::components::InlineFootnoteHit;
use crate::components::markdown::inline::{
    InlineFragment, InlineInsertionAttributes, InlineRenderCache, InlineSpan, InlineTextTree,
    StyleFlag,
};
#[cfg(test)]
use crate::components::markdown::inline::{InlineLinkHit, InlineStyle};
use crate::components::{
    ResourceLocation, ResourceRecord, ResourceStatus, TableAxisHighlight, TableAxisMarker,
    TableCellPosition, TableColumnAlignment, TableColumnLayout, TableRuntime,
};

/// Inline formatting command issued by editor actions.
#[derive(Clone, Copy)]
pub(crate) enum InlineFormat {
    /// Toggle bold formatting.
    Bold,
    /// Toggle italic formatting.
    Italic,
    /// Toggle strikethrough formatting.
    Strikethrough,
    /// Toggle underline formatting.
    Underline,
    /// Toggle highlight formatting.
    Highlight,
    /// Toggle superscript formatting.
    Superscript,
    /// Toggle subscript formatting.
    Subscript,
    /// Toggle inline code formatting.
    Code,
}

/// Editing semantics for the current block.
///
/// Rich blocks edit the attribute-based text tree, while source mode and code
/// blocks edit raw text without inline Markdown normalization.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EditMode {
    /// Attribute-based rich text editing for normal rendered blocks.
    RenderedRich,
    /// Raw Markdown editing for source-mode and raw fallback blocks.
    SourceRaw,
    /// Raw text editing for fenced code block contents.
    CodeBlockRaw,
}

/// Mermaid 块的局部工作台视图；它只影响当前运行期的呈现，不参与 Markdown 序列化。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum MermaidViewMode {
    Source,
    #[default]
    Preview,
    Split,
}

pub(crate) type HostActionHandler = Rc<dyn Fn(BlockHostAction, &mut Window, &mut App)>;

impl EditMode {
    fn for_kind(kind: &BlockKind) -> Self {
        if kind.is_code_block() {
            Self::CodeBlockRaw
        } else if matches!(
            kind,
            BlockKind::RawMarkdown
                | BlockKind::Comment
                | BlockKind::HtmlBlock
                | BlockKind::MathBlock
                | BlockKind::MermaidBlock
        ) {
            Self::SourceRaw
        } else {
            Self::RenderedRich
        }
    }

    fn uses_raw_text_editing(self) -> bool {
        matches!(self, Self::SourceRaw | Self::CodeBlockRaw)
    }

    fn supports_inline_projection(self) -> bool {
        matches!(self, Self::RenderedRich)
    }
}

#[path = "implementation/editing.rs"]
mod editing;
#[path = "implementation/inline_projection.rs"]
mod inline_projection;
#[path = "implementation/math_edit.rs"]
pub(super) mod math_edit;
#[path = "implementation/math_source.rs"]
pub(super) mod math_source;
#[path = "implementation/range_mapping.rs"]
mod range_mapping;

impl EventEmitter<BlockEvent> for Block {}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum MathPalettePage {
    #[default]
    Symbols,
    Structures,
}

/// A single editable block in the document tree.
///
/// Each block holds a [`BlockRecord`] containing the persistent data (kind,
/// title, UUIDs) and a [`FocusHandle`] for keyboard routing.  Runtime state
/// such as selection, cursor blink, and layout cache live on the struct.
///
/// Blocks delegate structural operations (split, merge, indent, delete) to
/// the parent editor via `BlockEvent` emissions.
pub struct Block {
    pub record: BlockRecord,
    pub(crate) render_cache: InlineRenderCache,
    code_highlight: Option<CodeHighlightResult>,
    source_syntax_language: Option<SharedString>,
    source_syntax_context: Option<SourceSyntaxContext>,
    pub(crate) last_successful_math_render: Option<crate::components::LatexSvgRender>,
    pub(crate) last_successful_mermaid_render: Option<crate::components::MermaidSvgRender>,
    pub(crate) math_render_error: Option<String>,
    pub(crate) mermaid_render_error: Option<String>,
    pub(crate) math_preview_key: Option<u64>,
    pub(crate) mermaid_preview_key: Option<u64>,
    /// 成功 SVG 对应的输入身份。源码改变后必须失效，避免导出上一次图表。
    pub(crate) mermaid_successful_preview_key: Option<u64>,
    pub(crate) math_preview_task: Option<Task<()>>,
    pub(crate) mermaid_preview_task: Option<Task<()>>,
    pub(crate) mermaid_view_mode: MermaidViewMode,
    /// Mermaid 预览独立于文档正文滚动，预览与分屏共用位置以保持切换连续性。
    pub(crate) mermaid_preview_scroll_handle: ScrollHandle,
    /// 预览态没有文本排版边界；块菜单和拖放仍需使用工作台的真实窗口坐标。
    pub(crate) mermaid_workbench_bounds: Arc<Mutex<Option<Bounds<Pixels>>>>,
    pub(crate) mermaid_copy_feedback: bool,
    pub(crate) mermaid_copy_feedback_task: Option<Task<()>>,
    pub children: Vec<Entity<Block>>,
    pub focus_handle: FocusHandle,
    pub(crate) code_language_focus_handle: FocusHandle,
    pub(crate) code_language_selected_range: Range<usize>,
    pub(crate) code_language_selection_reversed: bool,
    pub(crate) code_language_marked_range: Option<Range<usize>>,
    pub(crate) code_language_last_layout: Option<ShapedLine>,
    pub(crate) code_language_last_bounds: Option<Bounds<Pixels>>,
    pub(crate) code_language_is_selecting: bool,
    pub(crate) code_language_menu_open: bool,
    pub(crate) code_language_menu_selected: usize,
    /// Dedicated keyboard focus target for the copy action so activating it
    /// never steals the code or language editor's input focus semantics.
    pub(crate) code_copy_focus_handle: FocusHandle,
    pub(crate) code_copy_feedback: bool,
    pub(crate) code_copy_feedback_task: Option<Task<()>>,
    pub selected_range: Range<usize>,
    pub selection_reversed: bool,
    pub(crate) editor_selection_range: Option<Range<usize>>,
    pub(crate) editor_selection_supports_inline_commands: bool,
    pub marked_range: Option<Range<usize>>,
    pub(crate) spelling_diagnostics: Arc<[crate::spellcheck::SpellingDiagnostic]>,
    pub last_layout: Option<Vec<WrappedLine>>,
    pub(crate) source_layout_identity: Option<SourceLayoutIdentity>,
    pub(crate) source_layout_cache_key: Option<SourceLayoutCacheKey>,
    pub(crate) source_layout_cache_hits: u64,
    pub(crate) source_layout_cache_misses: u64,
    pub last_bounds: Option<Bounds<Pixels>>,
    pub last_line_height: Pixels,
    pub render_depth: usize,
    /// Position inside the direct sibling list. This is projection metadata,
    /// refreshed by `DocumentTree` after every structural mutation.
    pub(crate) structural_sibling_index: usize,
    pub(crate) structural_sibling_count: usize,
    pub(crate) structural_context_revision: u64,
    pub quote_depth: usize,
    pub(crate) quote_group_anchor: Option<uuid::Uuid>,
    pub(crate) visible_quote_depth: usize,
    pub(crate) visible_quote_group_anchor: Option<uuid::Uuid>,
    pub(crate) callout_depth: usize,
    pub(crate) callout_anchor: Option<uuid::Uuid>,
    pub(crate) callout_variant: Option<CalloutVariant>,
    pub(crate) footnote_anchor: Option<uuid::Uuid>,
    pub(crate) parent_is_list_item: bool,
    pub list_ordinal: Option<usize>,
    pub is_selecting: bool,
    pub cursor_blink_epoch: Instant,
    pub vertical_motion_x: Option<Pixels>,
    pub(super) cursor_blink_task: Option<Task<()>>,
    /// Cached projection used to show editable inline delimiters for the
    /// currently touched inline span(s).
    pub(crate) projection: Option<ExpandedInlineProjection>,
    /// Inputs that produced the current `projection`. When the next
    /// `sync_inline_projection_for_focus` computes the same inputs, the
    /// rebuild is skipped — saves a full O(fragments + text) walk per
    /// render frame (cursor blink + every arrow keypress).
    projection_cache_key: Option<(bool, Range<usize>, Option<Range<usize>>)>,
    /// Display text held as a SharedString so renders can clone an Arc
    /// instead of re-allocating per frame. Refreshed in `sync_render_cache`,
    /// `rebuild_inline_projection`, and `clear_inline_projection`.
    cached_display_text: SharedString,
    input_placeholder: Option<SharedString>,
    host_action_handler: Option<HostActionHandler>,
    host_submit_enabled: bool,
    collapsed_caret_affinity: CollapsedCaretAffinity,
    /// When true, block-level shortcuts and inline formatting are
    /// suppressed; the block stores raw text for source-mode editing.
    pub(crate) edit_mode: EditMode,
    /// 只读投影禁止输入、选择和结构编辑，但仍允许链接与脚注导航。
    read_only: bool,
    show_source_line_numbers: bool,
    /// 大文件 viewport 的活动行由宿主统一提供行高、行号槽与选择背景；Block 只负责
    /// 普通 Source 输入与光标，不能在聚焦时再引入第二套 padding/min-height/chrome。
    compact_source_host: bool,
    /// 紧凑宿主可覆盖源码输入字号，使内联编辑器与宿主文本在聚焦前后保持同一尺寸。
    host_text_size: Option<f32>,
    pub(crate) table_runtime: Option<TableRuntime>,
    pub(crate) table_cell_position: Option<TableCellPosition>,
    pub(crate) table_cell_alignment: Option<TableColumnAlignment>,
    pub(crate) table_axis_preview: Option<TableAxisMarker>,
    pub(crate) table_axis_selection: Option<TableAxisMarker>,
    pub(crate) table_axis_highlight: TableAxisHighlight,
    pub(crate) table_append_column_edge_hovered: bool,
    pub(crate) table_append_column_hovered: bool,
    pub(crate) table_append_column_zone_hovered: bool,
    pub(crate) table_append_column_button_hovered: bool,
    pub(crate) table_append_column_close_task: Option<Task<()>>,
    pub(crate) table_append_row_edge_hovered: bool,
    pub(crate) table_append_row_hovered: bool,
    pub(crate) table_append_row_zone_hovered: bool,
    pub(crate) table_append_row_button_hovered: bool,
    pub(crate) table_append_row_close_task: Option<Task<()>>,
    /// Render-only fold metadata. Refreshed by Editor from process-local view
    /// state and never serialized into Markdown.
    pub(crate) presentation_hidden: bool,
    pub(crate) presentation_collapsed: bool,
    pub(crate) presentation_fold_key: Option<SharedString>,
    pub(crate) presentation_fold_heading: bool,
    pub(crate) fold_focus_handle: FocusHandle,
    /// Table width fractions are view state and are deliberately separate
    /// from the editable table model and its undo history.
    pub(crate) table_column_layout: Option<TableColumnLayout>,
    pub(crate) table_view_key: Option<SharedString>,
    pub(crate) table_column_resize_session: Option<TableColumnResizeSession>,
    pub(crate) table_column_resize_focus_handle: FocusHandle,
    pub(crate) table_column_resize_boundary: Option<usize>,
    /// Visible-text range of an inline `$...$` fragment while it is being
    /// edited structurally. Display-math blocks leave this unset.
    pub(crate) math_edit_inline_range: Option<Range<usize>>,
    /// Revision-checked structured editing session. The block may update its
    /// source through normal local transactions, but an external revision is
    /// never silently overwritten on commit.
    pub(crate) math_edit_session: Option<crate::editor::math_edit::MathEditSession>,
    /// UTF-8 range of the active IME composition inside the current math slot.
    /// This is deliberately separate from `marked_range`, whose coordinates
    /// belong to the Markdown-visible block projection.
    pub(crate) math_marked_range: Option<Range<usize>>,
    /// Structured formula focus remains separate from the raw Markdown input.
    pub(crate) math_structure_focus_handle: FocusHandle,
    /// Compact LaTeX source surface focus. This is intentionally distinct from
    /// the two-dimensional structure focus so source edits can use the normal
    /// GPUI EntityInputHandler/IME path without stealing the visual caret.
    pub(crate) math_source_focus_handle: FocusHandle,
    pub(crate) math_source_selected_range: Range<usize>,
    pub(crate) math_source_selection_reversed: bool,
    pub(crate) math_source_marked_range: Option<Range<usize>>,
    pub(crate) math_source_last_layout: Option<ShapedLine>,
    pub(crate) math_source_last_bounds: Option<Bounds<Pixels>>,
    pub(crate) math_source_is_selecting: bool,
    pub(crate) math_visual_scroll_handle: ScrollHandle,
    /// Visible command page in the compact floating formula palette.
    pub(crate) math_palette_page: MathPalettePage,
    /// User-positioned palette translation and active pointer anchor.
    pub(crate) math_palette_offset: Point<Pixels>,
    pub(crate) math_palette_drag_anchor: Option<Point<Pixels>>,
    pub(crate) math_palette_anchor_y: Option<Pixels>,
    pub(crate) document_revision: gmark_document::Revision,
    image_runtime: Option<ImageRuntime>,
    resource_runtime: Option<ResourceRuntime>,
    resource_probe_key: Option<String>,
    resource_probe_task: Option<Task<()>>,
    pub(crate) resource_selected: bool,
    image_edit_expanded: bool,
    image_expand_requested: bool,
    /// Set by a failed-image placeholder's Retry action and consumed by the
    /// editor-level asset scheduler on the next render pass.
    pub(crate) image_retry_requested: bool,
    pub(crate) image_selected: bool,
    pub(crate) image_resize_session: Option<ImageResizeSession>,
    pub(crate) image_preview_width_percent: Option<u8>,
    /// Per-`<details>` disclosure state. The source `open` attribute is only
    /// the initial value; edits never collapse unrelated disclosures.
    pub(crate) html_details_state: HashMap<gmark_markdown::HtmlNodeId, bool>,
    pub(crate) html_details_source_hash: Option<u64>,
    /// Generation-safe local image state for semantic HTML nodes.  Kept
    /// separate from standalone image runtime because HTML images are keyed by
    /// their stable domain node id rather than by block title text.
    pub(crate) html_image_asset_states: HashMap<
        gmark_markdown::HtmlNodeId,
        (
            crate::editor::render_asset_manager::AssetKey,
            crate::editor::render_asset_manager::AssetState,
        ),
    >,
    image_base_dir: Option<PathBuf>,
    image_reference_definitions: Arc<ImageReferenceDefinitions>,
    link_reference_definitions: Arc<LinkReferenceDefinitions>,
    footnote_registry: Arc<FootnoteRegistry>,
    /// 文档目录是可重建投影；不属于 Markdown 源码或 undo 状态。
    pub(crate) toc_entries: Arc<[TocRuntimeEntry]>,
    pub(crate) list_group_separator_candidate: bool,
    numbered_list_restart_requested: bool,
    quote_reparse_requested: bool,
    pub(crate) slash_menu: Option<SlashMenuState>,
    pub(crate) slash_menu_dismissed_query: Option<String>,
    pub(crate) slash_menu_scroll_handle: ScrollHandle,
    pub(crate) block_drop_placement: super::BlockDropPlacement,
    pub(crate) selection_toolbar_dismissed_range: Option<Range<usize>>,
    pub(crate) selection_toolbar_keyboard_active: bool,
    pub(crate) selection_toolbar_keyboard_index: usize,
    pub(crate) selection_toolbar_overflow_open: bool,
    pub(crate) selection_toolbar_type_menu_open: bool,
    pub(crate) selection_toolbar_link_input: Option<Entity<Block>>,
    pub(crate) selection_toolbar_link_range: Option<Range<usize>>,
    pub(crate) selection_toolbar_link_had_target: bool,
}

/// A clickable heading entry resolved by the editor for one rendered `[TOC]` block.
#[derive(Clone, Debug)]
pub(crate) struct TocRuntimeEntry {
    pub(crate) level: u8,
    pub(crate) title: SharedString,
    pub(crate) target: EntityId,
}

#[derive(Clone, Debug)]
pub(crate) struct TableColumnResizeSession {
    pub(crate) boundary: usize,
    pub(crate) start_x: Pixels,
    pub(crate) start_layout: TableColumnLayout,
    pub(crate) table_width: f32,
}

/// How a collapsed caret at an inline projection boundary inherits style.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum CollapsedCaretAffinity {
    /// Use the normal insertion-attribute lookup.
    #[default]
    Default,
    /// Treat the caret as being just outside the opening delimiter.
    OuterStart,
    /// Treat the caret as being just outside the closing delimiter.
    OuterEnd,
}

impl Block {
    pub fn with_record(cx: &mut Context<Self>, record: BlockRecord) -> Self {
        let edit_mode = EditMode::for_kind(&record.kind);
        let render_cache = record.title.render_cache();
        let mut block = Self {
            record,
            render_cache,
            code_highlight: None,
            source_syntax_language: None,
            source_syntax_context: None,
            last_successful_math_render: None,
            last_successful_mermaid_render: None,
            math_render_error: None,
            mermaid_render_error: None,
            math_preview_key: None,
            mermaid_preview_key: None,
            mermaid_successful_preview_key: None,
            math_preview_task: None,
            mermaid_preview_task: None,
            mermaid_view_mode: MermaidViewMode::default(),
            mermaid_preview_scroll_handle: ScrollHandle::new(),
            mermaid_workbench_bounds: Arc::new(Mutex::new(None)),
            mermaid_copy_feedback: false,
            mermaid_copy_feedback_task: None,
            children: Vec::new(),
            focus_handle: cx.focus_handle(),
            code_language_focus_handle: cx.focus_handle(),
            code_language_selected_range: 0..0,
            code_language_selection_reversed: false,
            code_language_marked_range: None,
            code_language_last_layout: None,
            code_language_last_bounds: None,
            code_language_is_selecting: false,
            code_language_menu_open: false,
            code_language_menu_selected: 0,
            code_copy_focus_handle: cx.focus_handle(),
            code_copy_feedback: false,
            code_copy_feedback_task: None,
            selected_range: 0..0,
            selection_reversed: false,
            editor_selection_range: None,
            editor_selection_supports_inline_commands: false,
            marked_range: None,
            spelling_diagnostics: Arc::default(),
            last_layout: None,
            source_layout_identity: None,
            source_layout_cache_key: None,
            source_layout_cache_hits: 0,
            source_layout_cache_misses: 0,
            last_bounds: None,
            last_line_height: px(20.0),
            render_depth: 0,
            structural_sibling_index: 0,
            structural_sibling_count: 1,
            structural_context_revision: 0,
            quote_depth: 0,
            quote_group_anchor: None,
            visible_quote_depth: 0,
            visible_quote_group_anchor: None,
            callout_depth: 0,
            callout_anchor: None,
            callout_variant: None,
            footnote_anchor: None,
            parent_is_list_item: false,
            list_ordinal: None,
            is_selecting: false,
            cursor_blink_epoch: Instant::now(),
            vertical_motion_x: None,
            cursor_blink_task: None,
            projection: None,
            projection_cache_key: None,
            cached_display_text: SharedString::default(),
            input_placeholder: None,
            host_action_handler: None,
            host_submit_enabled: false,
            collapsed_caret_affinity: CollapsedCaretAffinity::Default,
            edit_mode,
            read_only: false,
            show_source_line_numbers: false,
            compact_source_host: false,
            host_text_size: None,
            table_runtime: None,
            table_cell_position: None,
            table_cell_alignment: None,
            table_axis_preview: None,
            table_axis_selection: None,
            table_axis_highlight: TableAxisHighlight::None,
            table_append_column_edge_hovered: false,
            table_append_column_hovered: false,
            table_append_column_zone_hovered: false,
            table_append_column_button_hovered: false,
            table_append_column_close_task: None,
            table_append_row_edge_hovered: false,
            table_append_row_hovered: false,
            table_append_row_zone_hovered: false,
            table_append_row_button_hovered: false,
            table_append_row_close_task: None,
            presentation_hidden: false,
            presentation_collapsed: false,
            presentation_fold_key: None,
            presentation_fold_heading: false,
            fold_focus_handle: cx.focus_handle(),
            table_column_layout: None,
            table_view_key: None,
            table_column_resize_session: None,
            table_column_resize_focus_handle: cx.focus_handle(),
            table_column_resize_boundary: None,
            math_edit_inline_range: None,
            math_edit_session: None,
            math_marked_range: None,
            math_structure_focus_handle: cx.focus_handle(),
            math_source_focus_handle: cx.focus_handle(),
            math_source_selected_range: 0..0,
            math_source_selection_reversed: false,
            math_source_marked_range: None,
            math_source_last_layout: None,
            math_source_last_bounds: None,
            math_source_is_selecting: false,
            math_visual_scroll_handle: ScrollHandle::new(),
            math_palette_page: MathPalettePage::Symbols,
            math_palette_offset: point(px(0.0), px(0.0)),
            math_palette_drag_anchor: None,
            math_palette_anchor_y: None,
            document_revision: gmark_document::Revision::INITIAL,
            image_runtime: None,
            resource_runtime: None,
            resource_probe_key: None,
            resource_probe_task: None,
            resource_selected: false,
            image_edit_expanded: false,
            image_expand_requested: false,
            image_retry_requested: false,
            image_selected: false,
            image_resize_session: None,
            image_preview_width_percent: None,
            html_details_state: HashMap::new(),
            html_details_source_hash: None,
            html_image_asset_states: HashMap::new(),
            image_base_dir: None,
            image_reference_definitions: Arc::default(),
            link_reference_definitions: Arc::default(),
            footnote_registry: Arc::default(),
            toc_entries: Arc::default(),
            list_group_separator_candidate: false,
            numbered_list_restart_requested: false,
            quote_reparse_requested: false,
            slash_menu: None,
            slash_menu_dismissed_query: None,
            slash_menu_scroll_handle: ScrollHandle::new(),
            block_drop_placement: super::BlockDropPlacement::Before,
            selection_toolbar_dismissed_range: None,
            selection_toolbar_keyboard_active: false,
            selection_toolbar_keyboard_index: 0,
            selection_toolbar_overflow_open: false,
            selection_toolbar_type_menu_open: false,
            selection_toolbar_link_input: None,
            selection_toolbar_link_range: None,
            selection_toolbar_link_had_target: false,
        };
        block.sync_code_highlight();
        block.refresh_cached_display_text();
        block
    }

    pub fn kind(&self) -> BlockKind {
        self.record.kind.clone()
    }

    pub(crate) fn is_source_raw_mode(&self) -> bool {
        self.edit_mode == EditMode::SourceRaw
    }

    pub(crate) fn is_read_only(&self) -> bool {
        self.read_only
    }

    /// 切换投影可编辑性，并同步清理所有可能跨模式残留的瞬态编辑状态；尤其是
    /// 代码语言菜单不能在 Live 实体转为 Preview 后继续接受局部修改。
    pub(crate) fn set_read_only(&mut self, read_only: bool) {
        if self.read_only == read_only {
            return;
        }
        self.read_only = read_only;
        self.is_selecting = false;
        self.code_language_is_selecting = false;
        self.code_language_menu_open = false;
        self.marked_range = None;
        self.code_language_marked_range = None;
        self.math_marked_range = None;
        if read_only {
            if self.kind() == BlockKind::MermaidBlock {
                self.mermaid_view_mode = MermaidViewMode::Preview;
            }
            self.clear_inline_projection();
            self.editor_selection_range = None;
            self.editor_selection_supports_inline_commands = false;
        }
    }

    pub(crate) fn show_source_line_numbers(&self) -> bool {
        self.show_source_line_numbers
    }

    pub(crate) fn take_quote_reparse_requested(&mut self) -> bool {
        let requested = self.quote_reparse_requested;
        self.quote_reparse_requested = false;
        requested
    }

    pub(crate) fn take_numbered_list_restart_requested(&mut self) -> bool {
        let requested = self.numbered_list_restart_requested;
        self.numbered_list_restart_requested = false;
        requested
    }

    pub(crate) fn set_runtime_context(
        &mut self,
        base_dir: Option<PathBuf>,
        image_reference_definitions: Arc<ImageReferenceDefinitions>,
        link_reference_definitions: Arc<LinkReferenceDefinitions>,
        footnote_registry: Arc<FootnoteRegistry>,
    ) {
        if self.image_base_dir != base_dir {
            self.image_base_dir = base_dir.clone();
        }
        if self.image_reference_definitions != image_reference_definitions {
            self.image_reference_definitions = image_reference_definitions;
        }
        self.sync_link_reference_definitions(link_reference_definitions);
        self.sync_footnote_registry(footnote_registry);
        self.sync_image_runtime();
        self.sync_resource_runtime_without_probe(base_dir);
    }

    pub(crate) fn set_document_revision(&mut self, revision: gmark_document::Revision) {
        self.document_revision = revision;
    }

    /// Updates the same context as the legacy/test-friendly setter and starts
    /// the asynchronous filesystem probe when a mounted editor block has a
    /// GPUI context available. Keeping the probe-specific argument out of the
    /// base setter preserves the narrow runtime API used by block tests and
    /// non-mounted callers.
    pub(crate) fn set_runtime_context_with_probe(
        &mut self,
        base_dir: Option<PathBuf>,
        image_reference_definitions: Arc<ImageReferenceDefinitions>,
        link_reference_definitions: Arc<LinkReferenceDefinitions>,
        footnote_registry: Arc<FootnoteRegistry>,
        cx: &mut Context<Self>,
    ) {
        self.set_runtime_context(
            base_dir.clone(),
            image_reference_definitions,
            link_reference_definitions,
            footnote_registry,
        );
        self.sync_resource_runtime(base_dir, cx);
    }

    pub(crate) fn uses_raw_text_editing(&self) -> bool {
        self.edit_mode.uses_raw_text_editing()
    }

    pub(crate) fn set_source_raw_mode(&mut self) {
        self.clear_inline_projection();
        self.edit_mode = EditMode::SourceRaw;
        self.show_source_line_numbers = false;
        self.compact_source_host = false;
    }

    pub(crate) fn set_compact_source_host(&mut self) {
        self.set_source_raw_mode();
        self.compact_source_host = true;
    }

    pub(crate) fn set_host_text_size(&mut self, text_size: f32) {
        self.host_text_size = Some(text_size.max(1.0));
    }

    pub(crate) fn host_text_size(&self) -> Option<f32> {
        self.host_text_size
    }

    pub(crate) fn set_source_layout_identity(&mut self, identity: SourceLayoutIdentity) {
        if self.source_layout_identity.as_ref() == Some(&identity) {
            return;
        }
        self.source_layout_identity = Some(identity);
        self.source_layout_cache_key = None;
    }

    pub(crate) fn compact_source_host(&self) -> bool {
        self.compact_source_host
    }

    pub(crate) fn set_source_document_mode(&mut self) {
        self.set_source_document_mode_with_language(Some("markdown"));
    }

    pub(crate) fn set_source_document_mode_with_language(
        &mut self,
        language: Option<&'static str>,
    ) {
        self.set_source_raw_mode();
        self.show_source_line_numbers = true;
        self.set_source_syntax_language(language);
    }

    pub(crate) fn sync_edit_mode_from_kind(&mut self) {
        if self.table_cell_position.is_some() {
            self.edit_mode = EditMode::RenderedRich;
            self.show_source_line_numbers = false;
            return;
        }
        if self.edit_mode != EditMode::SourceRaw {
            if self.kind().is_code_block() {
                self.clear_inline_projection();
            }
            self.edit_mode = EditMode::for_kind(&self.record.kind);
            self.show_source_line_numbers = false;
        }
    }

    pub fn display_text(&self) -> &str {
        self.current_cache().visible_text()
    }

    /// 返回最接近窗口纵坐标的源码字节偏移，用于 Split 滚动锚点。
    pub(crate) fn text_offset_for_window_y(&self, window_y: Pixels) -> Option<usize> {
        let bounds = self.last_bounds?;
        let lines = self.last_layout.as_ref()?;
        let relative_y = (window_y - bounds.top()).max(px(0.0));
        let (line_index, _) =
            super::element::wrapped_line_for_y(lines, self.last_line_height, relative_y)?;
        super::element::hard_line_ranges(self.display_text())
            .get(line_index)
            .map(|range| range.start)
    }

    /// 返回源码字节偏移所在硬行的窗口纵坐标。
    pub(crate) fn window_y_for_text_offset(&self, offset: usize) -> Option<Pixels> {
        let bounds = self.last_bounds?;
        let lines = self.last_layout.as_ref()?;
        let ranges = super::element::hard_line_ranges(self.display_text());
        let (line_index, _) = super::element::line_index_for_offset(&ranges, offset);
        Some(
            bounds.top()
                + super::element::wrapped_line_top(lines, self.last_line_height, line_index),
        )
    }

    /// Cheap clone of the current display text as a `SharedString` (Arc bump)
    /// — avoids a fresh String allocation per render. The cached value is
    /// refreshed by [`Self::refresh_cached_display_text`] whenever the
    /// underlying text might have changed.
    pub(crate) fn shared_display_text(&self) -> SharedString {
        self.cached_display_text.clone()
    }

    pub(crate) fn set_input_placeholder(&mut self, placeholder: impl Into<SharedString>) {
        self.input_placeholder = Some(placeholder.into());
    }

    /// 大文件活动行仍是普通 Block 输入面，但窗口级命令必须回到宿主文档处理，
    /// 否则聚焦行编辑后 Ctrl+F/Ctrl+G/保存/历史与翻页都会失去路由。
    pub(crate) fn set_host_action_handler(
        &mut self,
        handler: impl Fn(BlockHostAction, &mut Window, &mut App) + 'static,
    ) {
        self.host_action_handler = Some(Rc::new(handler));
    }

    pub(crate) fn has_host_action_handler(&self) -> bool {
        self.host_action_handler.is_some()
    }

    pub(crate) fn set_host_submit_enabled(&mut self, enabled: bool) {
        self.host_submit_enabled = enabled;
    }

    pub(crate) fn host_submit_enabled(&self) -> bool {
        self.host_submit_enabled
    }

    pub(crate) fn host_action_handler(&self) -> Option<HostActionHandler> {
        self.host_action_handler.clone()
    }

    pub(crate) fn input_placeholder(&self) -> Option<SharedString> {
        self.input_placeholder.clone()
    }

    fn refresh_cached_display_text(&mut self) {
        let current = self.current_cache().visible_text();
        if self.cached_display_text.as_ref() != current {
            self.cached_display_text = SharedString::from(current.to_string());
        }
    }
}

#[cfg(test)]
#[path = "../../../../tests/unit/components/block/runtime/scenarios.rs"]
mod tests;
