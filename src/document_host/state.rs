// @author kongweiguang

//! Private source, projection, and interaction state owned by DocumentHost.

use super::*;

#[derive(Clone, Debug)]
pub(super) struct StructuredRow {
    pub(super) index: u64,
    pub(super) byte_range: Range<u64>,
    pub(super) column_start: usize,
    pub(super) cells: Vec<String>,
    pub(super) depth: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct JsonNode {
    pub(super) container_path: Vec<u64>,
    pub(super) item: u64,
    pub(super) depth: usize,
}

impl JsonNode {
    pub(super) fn path(&self) -> Vec<u64> {
        let mut path = self.container_path.clone();
        path.push(self.item);
        path
    }
}

pub(super) struct SourceLineEdit {
    pub(super) line: usize,
    pub(super) range: std::ops::Range<u64>,
    pub(super) base_revision: u64,
    pub(super) ending: String,
    pub(super) leading_truncated: bool,
    pub(super) trailing_truncated: bool,
    pub(super) block: Entity<Block>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct BoundedLineWindow {
    pub(super) content_range: Range<u64>,
    pub(super) replace_range: Range<u64>,
    pub(super) text: SharedString,
    pub(super) ending: String,
    pub(super) leading_truncated: bool,
    pub(super) trailing_truncated: bool,
    pub(super) display: SharedString,
    pub(super) display_with_endings: OnceLock<SharedString>,
}

impl BoundedLineWindow {
    pub(super) fn new(
        content_range: Range<u64>,
        replace_range: Range<u64>,
        text: String,
        ending: String,
        leading_truncated: bool,
        trailing_truncated: bool,
    ) -> Self {
        let text: SharedString = text.into();
        let display = if leading_truncated || trailing_truncated {
            let mut rendered = String::with_capacity(text.len().saturating_add(4));
            if leading_truncated {
                rendered.push_str("… ");
            }
            rendered.push_str(&text);
            if trailing_truncated {
                rendered.push_str(" …");
            }
            rendered.into()
        } else {
            // 常见路径直接复用 GPUI SharedString 的同一份 Arc backing storage。
            text.clone()
        };
        Self {
            content_range,
            replace_range,
            text,
            ending,
            leading_truncated,
            trailing_truncated,
            display,
            display_with_endings: OnceLock::new(),
        }
    }

    pub(super) fn rendered(&self, show_line_endings: bool) -> SharedString {
        if show_line_endings {
            if self.trailing_truncated || self.ending.is_empty() {
                return self.display.clone();
            }
            self.display_with_endings
                .get_or_init(|| rendered_line_window_text(self, true).into())
                .clone()
        } else {
            self.display.clone()
        }
    }

    /// 前序编辑会平移本行的 source byte range，但可见文本不一定变化。此时 Block 仍是
    /// 有效的输入与布局宿主；独立 SourceLayoutIdentity 会更新坐标并按需失效 shaped layout。
    pub(super) fn has_same_surface_text(&self, other: &Self) -> bool {
        self.text == other.text
            && self.ending == other.ending
            && self.leading_truncated == other.leading_truncated
            && self.trailing_truncated == other.trailing_truncated
    }
}

/// 一帧 Source 的原子行快照。正文、行号、选择映射、命中测试和无障碍树只能
/// 读取此对象；后台 row cache 仅用于组装下一帧，不能被渲染层半途观察。
#[derive(Clone, Debug)]
pub(super) struct ScreenLines {
    pub(super) document_revision: u64,
    pub(super) generation: u64,
    pub(super) cache_epoch: u64,
    pub(super) column_window_start: u64,
    pub(super) visible: Range<usize>,
    pub(super) rows: Arc<BTreeMap<usize, Arc<BoundedLineWindow>>>,
}

impl Default for ScreenLines {
    fn default() -> Self {
        Self {
            document_revision: 0,
            generation: 0,
            cache_epoch: 0,
            column_window_start: 0,
            visible: 0..0,
            rows: Arc::new(BTreeMap::new()),
        }
    }
}

impl ScreenLines {
    pub(super) fn row(&self, line: usize) -> Option<&BoundedLineWindow> {
        self.rows.get(&line).map(Arc::as_ref)
    }

    pub(super) fn top_source_anchor(&self) -> Option<SourceAnchor> {
        self.row(self.visible.start)
            .map(|row| SourceAnchor::new(row.content_range.start, SourceAffinity::Before))
    }

    /// 随机远跳的新范围尚未读取时，按旧可见区的相对行序保留上一帧正文。
    /// 一旦新范围已有任意真实行，就不再混合两个坐标系，避免 selection/hit-test
    /// 错把旧文本映射到新的 source offset。
    pub(super) fn should_retain_previous_frame(&self, requested_visible: &Range<usize>) -> bool {
        !self.rows.is_empty()
            && !requested_visible
                .clone()
                .any(|line| self.rows.contains_key(&line))
    }

    pub(super) fn retained_rows(&self, show_line_endings: bool) -> Vec<(usize, SharedString)> {
        self.rows
            .range(self.visible.clone())
            .map(|(line, row)| (*line, row.rendered(show_line_endings)))
            .collect()
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct PagedDocumentMetrics {
    pub(super) viewport_requests: u64,
    pub(super) viewport_installs: u64,
    pub(super) stale_viewport_results: u64,
    pub(super) viewport_cancellations: u64,
    pub(super) max_cached_rows: usize,
    pub(super) layout_cache_hits: u64,
    pub(super) layout_cache_misses: u64,
    pub(super) max_layout_cache_entries: usize,
    pub(super) blank_frames_after_content: u64,
    pub(super) copy_requests: u64,
    pub(super) copied_bytes: u64,
    pub(super) export_requests: u64,
    pub(super) exported_bytes: u64,
    pub(super) projection_installs: u64,
    pub(super) stale_projection_results: u64,
}

/// 所有后台结果携带同一组文档身份。只读快照任务可选择仅校验 epoch（例如 Copy
/// 允许正文继续编辑），会回写坐标或 UI 状态的任务必须同时校验 revision 与 generation。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct DocumentTaskStamp {
    pub(super) document_epoch: u64,
    pub(super) document_revision: Option<u64>,
    pub(super) generation: u64,
}

impl DocumentTaskStamp {
    pub(super) fn capture(view: &DocumentHost, generation: u64) -> Self {
        Self {
            document_epoch: view.document_epoch,
            document_revision: view.document.as_ref().map(SharedDocument::revision),
            generation,
        }
    }

    pub(super) fn accepts_identity(self, view: &DocumentHost, generation: u64) -> bool {
        self.document_epoch == view.document_epoch && self.generation == generation
    }

    pub(super) fn accepts_strict(self, view: &DocumentHost, generation: u64) -> bool {
        self.accepts_identity(view, generation)
            && self.document_revision == view.document.as_ref().map(SharedDocument::revision)
    }
}

#[derive(Clone)]
pub(super) enum SourceViewportReader {
    Indexed(Box<SharedDocument>),
    Provisional {
        source: FileSource,
        estimated_lines: u64,
        encoding: TextEncoding,
    },
}

#[derive(Clone)]
pub(super) struct JsonGraphContextMenu {
    pub(super) node: JsonGraphItemId,
    /// 相对 JSON 画布的坐标，避免工作区与 Tab 外壳偏移菜单位置。
    pub(super) position: gpui::Point<gpui::Pixels>,
}

#[derive(Clone)]
pub(super) struct JsonGraphEditTarget {
    pub(super) item_id: JsonGraphItemId,
    pub(super) range: Range<u64>,
    pub(super) document_epoch: u64,
    pub(super) base_revision: u64,
    pub(super) label: Arc<str>,
    pub(super) kind: JsonValueKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum JsonGraphEditIssue {
    Invalid,
    Stale,
    TooLarge,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct StructuredCellEdit {
    pub(super) record: Option<u64>,
    pub(super) column: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum StructuredMenuTarget {
    Row(u64),
    Column(usize),
}
