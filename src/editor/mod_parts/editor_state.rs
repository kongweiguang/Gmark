// @author kongweiguang

use super::*;

/// 行高只在排版环境相同的视图内可复用，避免旧字体度量污染虚拟窗口位置。
#[derive(Clone)]
pub(super) struct RowStrideLayoutIdentity {
    theme: Arc<crate::theme::Theme>,
    font_family: String,
    content_width: f32,
}

impl RowStrideLayoutIdentity {
    /// 保留主题 Arc 身份而非颜色值比较，确保任何主题排版参数变化都会失效缓存。
    pub(super) fn new(
        theme: Arc<crate::theme::Theme>,
        font_family: String,
        content_width: f32,
    ) -> Self {
        Self {
            theme,
            font_family,
            content_width,
        }
    }

    /// 半像素内的宽度抖动不改变排版；字体族或主题对象变化则必须重测行高。
    pub(super) fn matches(
        &self,
        theme: &Arc<crate::theme::Theme>,
        font_family: &str,
        content_width: f32,
    ) -> bool {
        Arc::ptr_eq(&self.theme, theme)
            && self.font_family == font_family
            && (self.content_width - content_width).abs() <= 0.5
    }
}

/// Link navigation request deferred until a `Window` is available.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PendingOpenLink {
    pub(crate) prompt_target: String,
    pub(crate) open_target: String,
}

/// A resource picked before an untitled document enters Save As. The source
/// path is kept outside the Markdown transaction so cancelling Save As cannot
/// leave a copied file or a partial insertion behind.
#[derive(Clone)]
pub(super) enum PendingResourceInsertion {
    /// Picker/slash-command insertion keeps its structural destination until
    /// Save As succeeds; no file is copied before this state is resumed.
    Prompted {
        block: Entity<Block>,
        parent: Option<Entity<Block>>,
        index: usize,
        original_kind: BlockKind,
        cleaned_title: InlineTextTree,
        cursor: usize,
        query_only: bool,
        source: PathBuf,
    },
    /// Drag/drop and single-path paste preserve the exact inline split. The
    /// source path is materialized only after the document has a path.
    Pasted {
        block: Entity<Block>,
        leading: InlineTextTree,
        trailing: InlineTextTree,
        source: PathBuf,
    },
    /// Resource replacement retains the author-facing label and explicit kind.
    /// The target entity is re-resolved after materialization so a disappeared
    /// block cannot leave behind a copy created by this attempt.
    Replace {
        entity_id: EntityId,
        previous: crate::components::ResourceRecord,
        source: PathBuf,
    },
}

pub(super) struct ResourceTitleDialogState {
    pub(super) entity_id: EntityId,
    pub(super) previous: crate::components::ResourceRecord,
    pub(super) input: Entity<Block>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TableFragmentMergeDirection {
    IntoPrevious,
    IntoNext,
}

#[derive(Clone)]
pub(super) struct TableFragmentMergeTarget {
    pub(super) table_id: EntityId,
    pub(super) direction: TableFragmentMergeDirection,
    pub(super) rows: Vec<Vec<InlineTextTree>>,
}

#[derive(Clone)]
pub(super) struct TableFragmentMergeState {
    pub(super) base_revision: Revision,
    pub(super) parent_id: Option<EntityId>,
    pub(super) fragment_ids: Vec<EntityId>,
    pub(super) targets: Vec<TableFragmentMergeTarget>,
}

#[derive(Clone)]
pub(super) struct DiagramOverlayState {
    pub(super) block_id: EntityId,
    pub(super) preview_key: u64,
    pub(super) rendered: crate::components::MermaidSvgRender,
    /// `None` 时随视口适配；用户滚轮缩放或请求原始尺寸后保留显式比例。
    pub(super) manual_scale: Option<f32>,
    pub(super) scale_focus_handle: FocusHandle,
    pub(super) close_focus_handle: FocusHandle,
    pub(super) focus_close_on_render: bool,
}

#[derive(Clone, Debug)]
pub(super) struct WorkspaceLinkCandidate {
    pub(super) path: PathBuf,
    pub(super) relative_workspace_path: String,
    pub(super) title: String,
    pub(super) disambiguate: bool,
}

#[derive(Clone, Debug)]
pub(super) struct WorkspaceLinkCompletionState {
    pub(super) block_id: EntityId,
    pub(super) base_revision: Revision,
    pub(super) trigger_range: std::ops::Range<usize>,
    pub(super) selected: usize,
    pub(super) candidates: Vec<WorkspaceLinkCandidate>,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct SplitResizeSession {
    pub(super) start_x: Pixels,
    pub(super) start_ratio: f32,
    pub(super) available_width: f32,
}
