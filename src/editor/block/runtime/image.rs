// @author kongweiguang

//! Rendered standalone image runtime state.

use super::*;

/// Cached standalone image presentation state for a block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ImageRuntime {
    pub(crate) alt: String,
    pub(crate) src: String,
    pub(crate) title: Option<String>,
    pub(crate) width_percent: u8,
    pub(crate) resolved_source: ImageResolvedSource,
    /// Process-local bounded image preparation identity. Remote sources leave
    /// this unset and continue through GPUI's URL/resource loader unchanged.
    pub(crate) asset_key: Option<crate::editor::render_asset_manager::AssetKey>,
    /// Explicit local preparation state projected by the editor scheduler.
    pub(crate) asset_state: crate::editor::render_asset_manager::AssetState,
}

impl ImageRuntime {
    pub(crate) fn local_asset_state(
        &self,
    ) -> Option<(
        &crate::editor::render_asset_manager::AssetKey,
        &crate::editor::render_asset_manager::AssetState,
    )> {
        self.asset_key.as_ref().map(|key| (key, &self.asset_state))
    }
}

/// A local `<img>` discovered inside the shared safe HTML tree.  The editor
/// scheduler owns its decoded payload; the HTML renderer only consumes the
/// projected key/state by node id.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct HtmlImageAssetRequest {
    pub(crate) node_id: gmark_markdown::HtmlNodeId,
    pub(crate) path: PathBuf,
}

impl Block {
    pub(crate) fn local_html_image_requests(&self) -> Vec<HtmlImageAssetRequest> {
        let Some(document) = self
            .record
            .html
            .as_ref()
            .filter(|document| document.is_semantic())
        else {
            return Vec::new();
        };
        let mut requests = Vec::new();
        collect_html_image_requests(
            &document.nodes,
            self.image_base_dir.as_deref(),
            &mut requests,
        );
        requests
    }

    pub(crate) fn html_image_asset_state(
        &self,
        node_id: gmark_markdown::HtmlNodeId,
    ) -> Option<(
        &crate::editor::render_asset_manager::AssetKey,
        &crate::editor::render_asset_manager::AssetState,
    )> {
        self.html_image_asset_states
            .get(&node_id)
            .map(|(key, state)| (key, state))
    }

    pub(crate) fn bind_html_image_asset_state(
        &mut self,
        node_id: gmark_markdown::HtmlNodeId,
        key: crate::editor::render_asset_manager::AssetKey,
        state: crate::editor::render_asset_manager::AssetState,
    ) -> bool {
        let entry = self
            .html_image_asset_states
            .entry(node_id)
            .or_insert_with(|| {
                (
                    key.clone(),
                    crate::editor::render_asset_manager::AssetState::Idle,
                )
            });
        let changed = entry.0 != key || entry.1 != state;
        *entry = (key, state);
        changed
    }

    pub(crate) fn set_html_image_asset_state(
        &mut self,
        node_id: gmark_markdown::HtmlNodeId,
        key: &crate::editor::render_asset_manager::AssetKey,
        state: crate::editor::render_asset_manager::AssetState,
    ) -> bool {
        let Some((current_key, current_state)) = self.html_image_asset_states.get_mut(&node_id)
        else {
            return false;
        };
        if current_key != key || *current_state == state {
            return false;
        }
        *current_state = state;
        true
    }

    pub(crate) fn bind_image_asset_state(
        &mut self,
        key: crate::editor::render_asset_manager::AssetKey,
        state: crate::editor::render_asset_manager::AssetState,
    ) -> bool {
        let Some(runtime) = self.image_runtime.as_mut() else {
            return false;
        };
        let changed = runtime.asset_key.as_ref() != Some(&key) || runtime.asset_state != state;
        runtime.asset_key = Some(key);
        runtime.asset_state = state;
        changed
    }

    /// Projects editor-owned image preparation state into the block snapshot.
    /// The key guard prevents a completion for an old path/size bucket from
    /// mutating a runtime that has already changed in the document.
    pub(crate) fn set_image_asset_state(
        &mut self,
        key: crate::editor::render_asset_manager::AssetKey,
        state: crate::editor::render_asset_manager::AssetState,
    ) -> bool {
        let Some(runtime) = self.image_runtime.as_mut() else {
            return false;
        };
        if runtime.asset_key.as_ref() != Some(&key) {
            return false;
        }
        if runtime.asset_state == state {
            return false;
        }
        runtime.asset_state = state;
        true
    }
}

fn collect_html_image_requests(
    nodes: &[crate::components::HtmlNode],
    base_dir: Option<&Path>,
    output: &mut Vec<HtmlImageAssetRequest>,
) {
    for node in nodes {
        if node.tag_name == "img"
            && let Some(node_id) = node.id
            && let Some(src) = node
                .attrs
                .iter()
                .find(|attribute| attribute.name == "src")
                .and_then(|attribute| attribute.value.as_deref())
                .filter(|src| !src.trim().is_empty())
            && let ImageResolvedSource::Local(path) = resolve_image_source(src, base_dir)
        {
            output.push(HtmlImageAssetRequest { node_id, path });
        }
        collect_html_image_requests(&node.children, base_dir, output);
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ImageResizeSession {
    pub(crate) start_x: Pixels,
    pub(crate) start_percent: u8,
    pub(crate) available_width: f32,
}

impl Block {
    pub(crate) fn image_runtime(&self) -> Option<&ImageRuntime> {
        self.image_runtime.as_ref()
    }

    /// Source 整体正文不是单个图片块，避免每次输入序列化整篇文档。
    pub(super) fn can_present_as_image(&self) -> bool {
        if self.is_source_raw_mode() && self.show_source_line_numbers() {
            return false;
        }
        self.is_table_cell()
            || matches!(
                self.kind(),
                BlockKind::Paragraph
                    | BlockKind::BulletedListItem
                    | BlockKind::NumberedListItem
                    | BlockKind::TaskListItem { .. }
            )
    }

    /// Whether this block's text is a lone image that renders as a
    /// self-contained image widget. Unlike `showing_rendered_image`, this is
    /// derived from the title text rather than the computed runtime, so it is
    /// valid before image runtimes are (re)built.
    pub(crate) fn renders_as_standalone_image(&self) -> bool {
        self.can_present_as_image() && self.standalone_image_markdown_for_runtime().is_some()
    }

    pub(super) fn compute_image_runtime(
        &self,
        base_dir: Option<&Path>,
        syntax: ImageSyntax,
    ) -> Option<ImageRuntime> {
        let resolved_target = syntax.resolve_target(&self.image_reference_definitions)?;
        self.can_present_as_image().then(|| ImageRuntime {
            alt: syntax.alt.clone(),
            src: resolved_target.src.clone(),
            title: resolved_target.title.clone(),
            width_percent: syntax.width_percent,
            resolved_source: resolve_image_source(&resolved_target.src, base_dir),
            asset_key: None,
            asset_state: crate::editor::render_asset_manager::AssetState::Idle,
        })
    }

    pub(crate) fn image_runtime_for_syntax(&self, syntax: ImageSyntax) -> Option<ImageRuntime> {
        self.compute_image_runtime(self.image_base_dir.as_deref(), syntax)
    }

    pub(crate) fn image_base_dir(&self) -> Option<&Path> {
        self.image_base_dir.as_deref()
    }

    pub(super) fn sync_image_runtime(&mut self) {
        let next_runtime = if self.can_present_as_image() {
            self.standalone_image_markdown_for_runtime()
                .and_then(|markdown| parse_standalone_image(&markdown))
                .and_then(|syntax| {
                    self.compute_image_runtime(self.image_base_dir.as_deref(), syntax)
                })
        } else {
            None
        };

        if next_runtime.is_none() {
            self.image_edit_expanded = false;
            self.image_expand_requested = false;
            self.image_retry_requested = false;
        }
        self.image_runtime = next_runtime;
    }

    /// Requests a fresh bounded decode after a failed local-image load. The
    /// editor owns the generation/token transition; the block only records
    /// the user intent so the render pass remains side-effect free.
    pub(crate) fn request_image_retry(&mut self, cx: &mut Context<Self>) {
        if self.image_runtime.is_some() {
            self.image_retry_requested = true;
            cx.notify();
        }
    }

    pub(crate) fn take_image_retry_request(&mut self) -> bool {
        std::mem::take(&mut self.image_retry_requested)
    }

    fn standalone_image_markdown_for_runtime(&self) -> Option<String> {
        let visible = self.record.title.visible_text();
        if parse_standalone_image(&visible).is_some() {
            return Some(visible);
        }

        let serialized = self.record.title.serialize_markdown();
        parse_standalone_image(&serialized)
            .is_some()
            .then_some(serialized)
    }

    pub(crate) fn request_image_edit_expansion(&mut self) {
        if self.image_runtime.is_some() {
            self.image_expand_requested = true;
        }
    }

    pub(super) fn consume_requested_image_edit_expansion(&mut self) -> bool {
        if self.image_runtime.is_some() && self.image_expand_requested && !self.image_edit_expanded
        {
            self.image_expand_requested = false;
            self.image_edit_expanded = true;
            self.clear_inline_projection();
            self.assign_collapsed_selection_offset(
                self.visible_len(),
                CollapsedCaretAffinity::Default,
                None,
            );
            self.cursor_blink_epoch = Instant::now();
            self.clear_vertical_motion();
            return true;
        }

        false
    }

    pub(crate) fn sync_image_focus_state(&mut self, focused: bool) -> bool {
        if self.image_runtime.is_none() {
            let had_image_state = self.image_edit_expanded
                || self.image_expand_requested
                || self.image_retry_requested
                || self.image_selected
                || self.image_resize_session.is_some()
                || self.image_preview_width_percent.is_some();
            if had_image_state {
                self.image_edit_expanded = false;
                self.image_expand_requested = false;
                self.image_retry_requested = false;
                self.image_selected = false;
                self.image_resize_session = None;
                self.image_preview_width_percent = None;
                self.clear_inline_projection();
                return true;
            }
            return false;
        }

        if focused {
            return self.consume_requested_image_edit_expansion();
        }

        self.image_selected = false;
        self.image_resize_session = None;
        self.image_preview_width_percent = None;

        if self.image_edit_expanded {
            self.image_edit_expanded = false;
            self.clear_inline_projection();
            return true;
        }

        false
    }

    pub(crate) fn showing_rendered_image(&self) -> bool {
        self.image_runtime.is_some() && !self.is_source_raw_mode() && !self.image_edit_expanded
    }

    pub(crate) fn select_rendered_image(&mut self, cx: &mut Context<Self>) {
        if !self.is_read_only() && self.showing_rendered_image() && !self.image_selected {
            self.image_selected = true;
            self.selected_range = 0..0;
            self.marked_range = None;
            cx.notify();
        }
    }

    pub(crate) fn current_image_width_percent(&self) -> u8 {
        self.image_preview_width_percent
            .or_else(|| {
                self.image_runtime
                    .as_ref()
                    .map(|runtime| runtime.width_percent)
            })
            .unwrap_or(100)
            .clamp(10, 100)
    }

    pub(crate) fn start_image_resize(
        &mut self,
        start_x: Pixels,
        available_width: f32,
        cx: &mut Context<Self>,
    ) {
        if self.is_read_only() || !self.image_selected || !self.showing_rendered_image() {
            return;
        }
        let start_percent = self.current_image_width_percent();
        self.image_resize_session = Some(ImageResizeSession {
            start_x,
            start_percent,
            available_width: available_width.max(1.0),
        });
        self.image_preview_width_percent = Some(start_percent);
        cx.notify();
    }

    pub(crate) fn update_image_resize(&mut self, pointer_x: Pixels, cx: &mut Context<Self>) {
        let Some(session) = self.image_resize_session else {
            return;
        };
        let delta_percent = ((f32::from(pointer_x - session.start_x) / session.available_width)
            * 100.0)
            .round() as i32;
        let next = (i32::from(session.start_percent) + delta_percent).clamp(10, 100) as u8;
        if self.image_preview_width_percent != Some(next) {
            self.image_preview_width_percent = Some(next);
            cx.notify();
        }
    }

    pub(crate) fn finish_image_resize(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(session) = self.image_resize_session.take() else {
            return false;
        };
        let next = self
            .image_preview_width_percent
            .take()
            .unwrap_or(session.start_percent);
        if next == session.start_percent {
            cx.notify();
            return true;
        }
        let source = self.record.title.visible_text().to_owned();
        let Some(rewritten) = rewrite_standalone_image_width(&source, next) else {
            cx.notify();
            return true;
        };
        self.prepare_undo_capture(UndoCaptureKind::NonCoalescible, cx);
        self.replace_text_in_visible_range(0..self.visible_len(), &rewritten, None, false, cx);
        true
    }

    pub(crate) fn cancel_image_selection(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.image_selected {
            return false;
        }
        self.image_selected = false;
        self.image_resize_session = None;
        self.image_preview_width_percent = None;
        cx.notify();
        true
    }
}
