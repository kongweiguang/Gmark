// @author kongweiguang

use super::*;
use crate::i18n::I18nManager;
#[path = "editor_construction_parts/runtime.rs"]
mod construction_runtime;
#[path = "math_accessibility.rs"]
mod math_accessibility;
use math_accessibility::*;

impl Editor {
    pub(in crate::editor) const HISTORY_LIMIT: usize = 200;
    pub(in crate::editor) const HISTORY_COALESCE_WINDOW: Duration = Duration::from_millis(1_000);
    pub(in crate::editor) const SPLIT_PROJECTION_DEBOUNCE: Duration = Duration::from_millis(24);
    /// 大文档后台投影必须等待连续输入停顿，避免上一 revision 的全量行切分抢占下一按键。
    pub(in crate::editor) const VIRTUAL_PROJECTION_DEBOUNCE: Duration = Duration::from_millis(750);
    pub(in crate::editor) const RENDERED_SELECT_ALL_CYCLE_WINDOW: Duration =
        Duration::from_millis(750);
    /// 超过该区域数时，全量 GPUI Entity 已明显越过启动与内存 SLO。
    pub(in crate::editor) const VIRTUAL_SURFACE_REGION_THRESHOLD: usize = 8_192;

    pub(in crate::editor) fn should_virtualize_projection(
        projection: &PreparedSplitProjection,
    ) -> bool {
        projection.regions.len() >= Self::VIRTUAL_SURFACE_REGION_THRESHOLD
    }
    // reason: platform menu and tests construct untitled editors; remove only with that compatibility entrypoint.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn from_markdown(
        cx: &mut Context<Self>,
        markdown: String,
        file_path: Option<PathBuf>,
    ) -> Self {
        Self::from_markdown_internal(cx, markdown, file_path, false, false, None)
    }

    pub(crate) fn from_opened_markdown(
        cx: &mut Context<Self>,
        opened: crate::document_io::OpenedMarkdown,
        file_path: Option<PathBuf>,
    ) -> Self {
        let requires_conversion = !opened.encoding.is_utf8();
        let mut editor =
            Self::from_markdown_internal(cx, opened.text, file_path, false, false, None);
        editor.source_encoding = opened.encoding;
        // SVG 的源码是真值，但首次打开应直接展示派生预览；在构造阶段确定模式，
        // 避免窗口创建后的补偿更新与首帧/平台启动时序竞争。
        if requires_conversion || editor.is_svg_document() {
            editor.set_view_mode(ViewMode::Preview, cx);
        }
        if requires_conversion {
            editor.show_encoding_conversion_dialog = true;
        }
        editor
    }

    pub(crate) fn from_source_backed_file(
        cx: &mut Context<Self>,
        path: PathBuf,
        probe: gmark_paged_document::OpenProbe,
        source: gmark_paged_document::FileSource,
    ) -> Self {
        let structured_preview = probe.strategy == gmark_paged_document::OpenStrategy::Resident
            && matches!(
                probe.format,
                gmark_document_core::DocumentFormat::Json
                    | gmark_document_core::DocumentFormat::Delimited { .. }
            );
        let mut editor = Self::from_markdown_internal(
            cx,
            String::new(),
            Some(path.clone()),
            false,
            false,
            Some(EditorDocumentSession::shell()),
        );
        let pane_host_path = path.clone();
        let pane_host_probe = probe.clone();
        let source_backed_view =
            cx.new(move |cx| crate::document_host::DocumentHost::new(path, probe, source, cx));
        Self::subscribe_document_host(&source_backed_view, cx);
        editor.document_host = Some(source_backed_view);
        if structured_preview {
            if let Some(view) = editor.document_host.clone() {
                view.update(cx, |view, cx| view.show_structure_view(cx));
            }
            editor.view_mode = ViewMode::Preview;
        } else {
            editor.view_mode = ViewMode::Source;
        }
        editor.pane_host_path = Some(pane_host_path);
        editor.pane_host_probe = Some(pane_host_probe);
        editor.pending_focus = None;
        editor.active_entity_id = None;
        editor.restart_file_watcher(cx);
        editor
    }

    /// Build the window-shell compatibility editor around a service-owned
    /// source-backed handle.  The host receives the only lease; the shell's
    /// Markdown adapter is an empty compatibility projection and never starts
    /// a per-view file watcher or a second source-backed body.
    pub(crate) fn from_shared_document_host(
        cx: &mut Context<Self>,
        path: PathBuf,
        probe: gmark_paged_document::OpenProbe,
        handle: gmark_document_runtime::DocumentHandle,
        lease: gmark_document_runtime::DocumentLease,
    ) -> Self {
        let mut editor = Self::from_markdown_internal(
            cx,
            String::new(),
            Some(path.clone()),
            false,
            false,
            Some(EditorDocumentSession::shell()),
        );
        let pane_host_path = path.clone();
        let pane_host_probe = probe.clone();
        let document_host = cx.new(move |cx| {
            crate::document_host::DocumentHost::from_shared(path, probe, handle, lease, cx)
        });
        Self::subscribe_document_host(&document_host, cx);
        editor.document_host = Some(document_host);
        editor.shared_document = true;
        // `from_markdown_internal` performs the regular file-watcher setup
        // before returning.  This shell must not retain that watcher; dropping
        // the task/guard here cancels it before the first frame.
        editor.file_watch_task = None;
        editor.file_watch_guard = None;
        editor.shared_event_task = None;
        editor.view_mode = ViewMode::Source;
        editor.pane_host_path = Some(pane_host_path);
        editor.pane_host_probe = Some(pane_host_probe);
        editor.pending_focus = None;
        editor.active_entity_id = None;
        editor
    }

    /// Build a service-backed Host view with a persisted Controller view id.
    /// Construction errors are returned after the temporary fallback entity
    /// is dropped; no random replacement view or second source body is kept.
    pub(crate) fn from_shared_document_host_with_view_id(
        cx: &mut Context<Self>,
        path: PathBuf,
        probe: gmark_paged_document::OpenProbe,
        handle: gmark_document_runtime::DocumentHandle,
        lease: gmark_document_runtime::DocumentLease,
        view_id: gmark_document_core::DocumentViewInstanceId,
        presentation: crate::document_host::DocumentHostViewPresentation,
    ) -> Self {
        let mut editor = Self::from_markdown_internal(
            cx,
            String::new(),
            Some(path.clone()),
            false,
            false,
            Some(EditorDocumentSession::shell()),
        );
        let pane_host_path = path.clone();
        let pane_host_probe = probe.clone();
        let document_host = cx.new(move |cx| {
            crate::document_host::DocumentHost::from_shared_with_view_id_or_error(
                path,
                probe,
                handle,
                lease,
                view_id,
                presentation,
                cx,
            )
        });
        Self::subscribe_document_host(&document_host, cx);
        editor.document_host = Some(document_host);
        editor.shared_document = true;
        editor.file_watch_task = None;
        editor.file_watch_guard = None;
        editor.shared_event_task = None;
        editor.view_mode = ViewMode::Source;
        editor.pane_host_path = Some(pane_host_path);
        editor.pane_host_probe = Some(pane_host_probe);
        editor.pending_focus = None;
        editor.active_entity_id = None;
        editor
    }

    pub(crate) fn from_paged_recovery(
        cx: &mut Context<Self>,
        path: PathBuf,
        probe: gmark_paged_document::OpenProbe,
        source: gmark_paged_document::FileSource,
        journal_path: PathBuf,
    ) -> Self {
        let mut editor = Self::from_markdown_internal(
            cx,
            String::new(),
            Some(path.clone()),
            false,
            false,
            Some(EditorDocumentSession::shell()),
        );
        let pane_host_path = path.clone();
        let pane_host_probe = probe.clone();
        let document_host = cx.new(move |cx| {
            crate::document_host::DocumentHost::from_recovery(path, probe, source, journal_path, cx)
        });
        Self::subscribe_document_host(&document_host, cx);
        editor.document_host = Some(document_host);
        editor.pane_host_path = Some(pane_host_path);
        editor.pane_host_probe = Some(pane_host_probe);
        editor.view_mode = ViewMode::Source;
        editor.document_dirty = true;
        editor.pending_window_edited = true;
        editor.pending_focus = None;
        editor.active_entity_id = None;
        editor.restart_file_watcher(cx);
        editor
    }

    pub(crate) fn install_accessibility_bridge(&mut self, window: &Window, cx: &mut Context<Self>) {
        if self.accessibility_bridge.is_some() {
            return;
        }
        let snapshot = self.accessibility_snapshot(cx);
        self.accessibility_revision = Some(self.current_accessibility_revision(cx));
        let Some((bridge, mut wake)) =
            crate::accessibility::AccessibilityBridge::new(window, snapshot)
        else {
            return;
        };
        self.accessibility_bridge = Some(bridge);
        // 平台 action handler 可能运行在非 GPUI 线程；无界 channel 只负责唤醒一帧，
        // 真正动作仍回到现有 Editor action 路径和窗口线程，不引入空闲轮询。
        self.accessibility_wake_task = Some(cx.spawn(async move |this, cx| {
            while wake.next().await.is_some() {
                let Ok(()) = this.update(cx, |_editor, cx| cx.notify()) else {
                    break;
                };
            }
        }));
    }

    pub(in crate::editor) fn accessibility_snapshot(
        &self,
        cx: &App,
    ) -> crate::accessibility::EditorAccessibilitySnapshot {
        if let Some(mut snapshot) = self.focused_pane_accessibility_snapshot(cx) {
            self.attach_global_update_accessibility(&mut snapshot, cx);
            return snapshot;
        }
        let strings = cx.global::<I18nManager>().strings();
        if let Some(document_host) = self.document_host.as_ref() {
            let mut snapshot = document_host.read(cx).accessibility_snapshot(cx);
            self.attach_global_update_accessibility(&mut snapshot, cx);
            return snapshot;
        }
        let title = self
            .file_path
            .as_ref()
            .and_then(|path| path.file_name())
            .and_then(|name| name.to_str())
            .map(str::to_owned)
            .unwrap_or_else(|| strings.large_document_text("untitled"));
        let lines: Vec<(u64, String)> = self
            .source_document
            .text()
            .lines()
            .take(512)
            .enumerate()
            .map(|(line, text)| (line as u64, text.to_owned()))
            .collect();
        let folds = self.accessibility_folds(&lines, cx);
        let update_status = crate::updater::UpdateCoordinator::accessibility_status(cx);
        let mut snapshot = crate::accessibility::EditorAccessibilitySnapshot {
            title,
            mode: match self.view_mode {
                ViewMode::Rendered => crate::accessibility::AccessibilityMode::Live,
                ViewMode::Source => crate::accessibility::AccessibilityMode::Source,
                ViewMode::Preview => crate::accessibility::AccessibilityMode::Preview,
                ViewMode::Split => crate::accessibility::AccessibilityMode::Split,
            },
            dirty: self.is_document_dirty(),
            status: update_status.clone().unwrap_or_else(|| {
                if self.is_document_dirty() {
                    strings.large_document_text("modified").to_owned()
                } else {
                    strings.large_document_text("saved").to_owned()
                }
            }),
            error: self
                .external_file_conflict
                .then(|| strings.large_document_text("file_changed_disk").to_owned()),
            busy: self.save_task.is_some() || self.export_in_progress || update_status.is_some(),
            update_actions: Vec::new(),
            close_actions: Vec::new(),
            search_visible: self.find_panel.is_some(),
            navigation_visible: false,
            caret: None,
            lines,
            folds,
            math: self.active_math_accessibility(cx, strings),
        };
        self.attach_global_update_accessibility(&mut snapshot, cx);
        snapshot
    }

    /// 更新协调器属于应用全局而非具体文档，把状态合并到任意活动画布才能让辅助技术稳定找到恢复动作。
    fn attach_global_update_accessibility(
        &self,
        snapshot: &mut crate::accessibility::EditorAccessibilitySnapshot,
        cx: &App,
    ) {
        let update_status = crate::updater::UpdateCoordinator::accessibility_status(cx);
        snapshot.update_actions = self
            .update_accessibility_actions(cx)
            .into_iter()
            .map(|action| action.label)
            .collect();
        snapshot.close_actions = if self.show_unsaved_changes_dialog {
            let strings = cx.global::<I18nManager>().strings();
            vec![
                strings.unsaved_changes_cancel.to_string(),
                strings.unsaved_changes_discard_and_close.to_string(),
                strings.unsaved_changes_save_and_close.to_string(),
            ]
        } else {
            Vec::new()
        };
        if let Some(status) = update_status {
            snapshot.status = status;
            snapshot.busy = true;
        }
    }

    fn focused_pane_accessibility_snapshot(
        &self,
        cx: &App,
    ) -> Option<crate::accessibility::EditorAccessibilitySnapshot> {
        let pane = self
            .pane_workspace
            .as_ref()?
            .read(cx)
            .workspace()
            .focused_pane();
        let canvases = self.pane_canvas_entities.borrow();
        let (_, _, canvas) = canvases.get(&pane)?;
        match canvas {
            crate::editor::panes::PaneCanvasEntity::Markdown(canvas) => {
                Some(canvas.read(cx).accessibility_snapshot(cx))
            }
            crate::editor::panes::PaneCanvasEntity::DocumentHost(canvas) => {
                Some(canvas.read(cx).accessibility_snapshot(cx))
            }
            crate::editor::panes::PaneCanvasEntity::ReadOnly(_) => None,
        }
    }

    fn active_math_accessibility(
        &self,
        cx: &App,
        strings: &crate::i18n::I18nStrings,
    ) -> Option<crate::accessibility::AccessibilityMath> {
        let entity_id = self.active_entity_id.or(self.pending_focus)?;
        let block = self.focusable_entity_by_id(entity_id)?;
        let block = block.read(cx);
        let session = block.math_edit_session.as_ref()?;
        let page = match block.math_palette_page {
            crate::components::MathPalettePage::Symbols => {
                crate::accessibility::AccessibilityMathPage::Symbols
            }
            crate::components::MathPalettePage::Structures => {
                crate::accessibility::AccessibilityMathPage::Structures
            }
        };
        let editor = session.editor();
        let document = session.document();
        let cursor = editor.cursor();
        let slot_value = math_slot_source(document, cursor.slot())?;
        let slot_label = format!(
            "{} ({:?})",
            strings.math_palette_text("formula_editor"),
            cursor.slot().role()
        );
        let controls = math_accessibility_controls(strings);
        let grid = math_accessibility_grid(document, cursor.slot());
        Some(crate::accessibility::AccessibilityMath {
            source: document.to_latex(),
            slot_value,
            slot_cursor: cursor.offset(),
            slot_label,
            symbols_label: strings.math_palette_text("symbols_tab"),
            structures_label: strings.math_palette_text("structures_tab"),
            page,
            controls,
            grid,
        })
    }

    fn accessibility_folds(
        &self,
        lines: &[(u64, String)],
        cx: &App,
    ) -> Vec<crate::accessibility::AccessibilityFold> {
        if self.view_mode == ViewMode::Source {
            return Vec::new();
        }
        let mut cursor = 0usize;
        let mut folds = Vec::new();
        for visible in self.document.flatten_visible_blocks() {
            let (kind, key, collapsed, heading) = visible.entity.read_with(cx, |block, _cx| {
                (
                    block.kind(),
                    block
                        .presentation_fold_key
                        .as_ref()
                        .map(ToString::to_string),
                    block.presentation_collapsed,
                    block.presentation_fold_heading,
                )
            });
            let Some(key) = key else { continue };
            let Some(start_line) = next_accessibility_fold_line(lines, &mut cursor, &kind) else {
                continue;
            };
            folds.push(crate::accessibility::AccessibilityFold {
                start_line,
                end_line: start_line,
                collapsed,
                target: Some(crate::accessibility::AccessibilityFoldTarget::Rendered {
                    key,
                    heading,
                }),
            });
        }
        for index in 0..folds.len() {
            folds[index].end_line = folds
                .get(index + 1)
                .map(|fold| fold.start_line.saturating_sub(1))
                .or_else(|| lines.last().map(|(line, _)| *line))
                .unwrap_or(folds[index].start_line);
        }
        folds
    }

    /// 按实际活动窗格或输入表面的发布身份更新读屏投影，候选不需要写入正文 revision。
    pub(in crate::editor) fn current_accessibility_revision(&self, cx: &App) -> u64 {
        if let Some(revision) = self.focused_pane_accessibility_revision(cx) {
            return revision
                ^ (update_accessibility_revision(cx) << 5)
                ^ (u64::from(self.show_unsaved_changes_dialog) << 15);
        }
        if let Some(document_host) = self.document_host.as_ref() {
            return document_host.read(cx).accessibility_revision(cx)
                ^ (update_accessibility_revision(cx) << 5)
                ^ (u64::from(self.show_unsaved_changes_dialog) << 15);
        }
        let math_signature = self.accessibility_math_signature(cx);
        let flags = u64::from(self.is_document_dirty())
            | (u64::from(self.find_panel.is_some()) << 1)
            | (u64::from(self.external_file_conflict) << 2)
            | (u64::from(self.save_task.is_some()) << 3)
            | (u64::from(self.export_in_progress) << 4)
            | (update_accessibility_revision(cx) << 5)
            | (u64::from(self.show_unsaved_changes_dialog) << 15)
            | (match self.view_mode {
                ViewMode::Source => 0,
                ViewMode::Rendered => 1,
                ViewMode::Preview => 2,
                ViewMode::Split => 3,
            } << 10);
        let fold_signature =
            self.document
                .flatten_visible_blocks()
                .iter()
                .fold(0_u64, |signature, visible| {
                    let (has_key, collapsed) = visible.entity.read_with(cx, |block, _cx| {
                        (
                            block.presentation_fold_key.is_some(),
                            block.presentation_collapsed,
                        )
                    });
                    u64::from(has_key) | (u64::from(collapsed) << 1) | signature.rotate_left(5)
                });
        let flags = flags | ((fold_signature & 0x000f_ffff) << 20);
        self.source_document
            .revision()
            .get()
            .wrapping_mul(4_096)
            .wrapping_add(flags)
            .wrapping_add(math_signature.rotate_left(17))
    }

    fn focused_pane_accessibility_revision(&self, cx: &App) -> Option<u64> {
        let pane = self
            .pane_workspace
            .as_ref()?
            .read(cx)
            .workspace()
            .focused_pane();
        let pane_uuid = pane.as_uuid().as_u128();
        let pane_signature = pane_uuid as u64 ^ (pane_uuid >> 64) as u64;
        let canvases = self.pane_canvas_entities.borrow();
        let (_, _, canvas) = canvases.get(&pane)?;
        let content_revision = match canvas {
            crate::editor::panes::PaneCanvasEntity::Markdown(canvas) => {
                canvas.read(cx).accessibility_revision(cx)
            }
            crate::editor::panes::PaneCanvasEntity::DocumentHost(canvas) => {
                canvas.read(cx).accessibility_revision(cx)
            }
            crate::editor::panes::PaneCanvasEntity::ReadOnly(_) => 0,
        };
        Some(content_revision.rotate_left(13) ^ pane_signature)
    }

    fn accessibility_math_signature(&self, cx: &App) -> u64 {
        use std::hash::{Hash, Hasher};

        let Some(math) = self.active_math_accessibility(cx, cx.global::<I18nManager>().strings())
        else {
            return 0;
        };
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        math.source.hash(&mut hasher);
        math.slot_value.hash(&mut hasher);
        math.slot_cursor.hash(&mut hasher);
        math.page.hash(&mut hasher);
        if let Some(grid) = math.grid {
            grid.active_row.hash(&mut hasher);
            grid.active_column.hash(&mut hasher);
        }
        hasher.finish()
    }
}

fn next_accessibility_fold_line(
    lines: &[(u64, String)],
    cursor: &mut usize,
    kind: &BlockKind,
) -> Option<u64> {
    let matches = |text: &str| match kind {
        BlockKind::Heading { level } => {
            let trimmed = text.trim_start();
            let hashes = trimmed.bytes().take_while(|byte| *byte == b'#').count();
            hashes == usize::from(*level) && trimmed.as_bytes().get(hashes) == Some(&b' ')
        }
        BlockKind::Callout(_) => text.trim_start().starts_with("> [!"),
        _ => false,
    };
    for (index, (line, text)) in lines.iter().enumerate().skip(*cursor) {
        if matches(text) {
            *cursor = index + 1;
            return Some(*line);
        }
    }
    None
}

/// 更新面板的每一种交互状态都必须拥有独立 revision，避免读屏树停留在上一组按钮。
fn update_accessibility_revision(cx: &App) -> u64 {
    match crate::updater::UpdateCoordinator::try_state(cx) {
        None | Some(crate::updater::UpdateState::Idle) => 0,
        Some(crate::updater::UpdateState::Restoring) => 1,
        Some(crate::updater::UpdateState::Checking { .. }) => 2,
        Some(crate::updater::UpdateState::UpToDate { .. }) => 3,
        Some(crate::updater::UpdateState::Available(_)) => 4,
        Some(crate::updater::UpdateState::Downloading {
            downloaded, total, ..
        }) if total > 0 => downloaded.saturating_mul(100) / total + 20,
        Some(crate::updater::UpdateState::Downloading { .. }) => 20,
        Some(crate::updater::UpdateState::Paused { .. }) => 121,
        Some(crate::updater::UpdateState::Verifying { .. }) => 122,
        Some(crate::updater::UpdateState::Ready { .. }) => 123,
        Some(crate::updater::UpdateState::StagingInstall { .. }) => 124,
        Some(crate::updater::UpdateState::Staged { .. }) => 125,
        Some(crate::updater::UpdateState::Succeeded { .. }) => 126,
        Some(crate::updater::UpdateState::Failed {
            release, retryable, ..
        }) => 127 + u64::from(retryable) + (u64::from(release.is_some()) << 1),
    }
}
