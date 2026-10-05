// @author kongweiguang

use super::*;

impl Editor {
    /// 正文事件与关闭终态都归属于原 Host；标签已替换时忽略迟到事件，
    /// 保存队列及输入仍未收尾时只刷新显示，不能卸载正文或提前消费关闭意图。
    pub(crate) fn subscribe_document_host(
        view: &Entity<crate::document_host::DocumentHost>,
        cx: &mut Context<Self>,
    ) {
        Self::observe_document_host_accessibility(view, cx);
        cx.subscribe(view, |editor, view, event, cx| {
            if editor
                .document_host
                .as_ref()
                .is_none_or(|active| active.entity_id() != view.entity_id())
            {
                return;
            }
            match event {
                crate::document_host::DocumentHostEvent::SavedAs(path) => {
                    let tab_id = editor.tabs.active_id();
                    let _ = editor.view_state.close_tab(tab_id);
                    editor.file_path = Some(path.clone());
                    let _ = editor.view_state.open_tab(
                        crate::editor::markdown_view_state::MarkdownTabIdentity::saved(
                            path, tab_id,
                        ),
                    );
                    editor.saved_file_fingerprint = crate::recovery::fingerprint_file(path).ok();
                    editor.document_dirty = false;
                    editor.pending_window_edited = false;
                    editor.schedule_workspace_session_save(cx);
                    #[cfg(target_os = "macos")]
                    editor.schedule_platform_document_menu_refresh(cx);
                    cx.notify();
                }
                crate::document_host::DocumentHostEvent::StateChanged => {
                    let host = view.read(cx);
                    let host_dirty = host.is_dirty();
                    let save_pending = host.has_pending_save()
                        || host.has_pending_source_input()
                        || host.has_active_ime_composition(cx);
                    editor.document_dirty = host_dirty;
                    editor.pending_window_edited = editor.document_dirty;
                    editor.pending_window_title_refresh = true;
                    editor.schedule_workspace_session_save(cx);
                    #[cfg(target_os = "macos")]
                    editor.schedule_platform_document_menu_refresh(cx);
                    if !save_pending {
                        if host_dirty {
                            editor.abort_pending_tab_close_after_save(cx);
                            editor.abort_window_close_tab_sequence(cx);
                            if editor.pending_close_after_save {
                                editor.abort_pending_close_after_save(cx);
                            }
                        } else {
                            editor.continue_window_close_after_save(cx);
                            editor.finish_pending_tab_close_after_save(cx);
                        }
                    }
                    cx.notify();
                }
                crate::document_host::DocumentHostEvent::ViewModeChanged(mode) => {
                    editor.view_mode = match mode {
                        crate::document_host::DocumentHostMode::Live => ViewMode::Rendered,
                        crate::document_host::DocumentHostMode::Source => ViewMode::Source,
                        crate::document_host::DocumentHostMode::Preview => ViewMode::Preview,
                        crate::document_host::DocumentHostMode::Split => ViewMode::Split,
                    };
                    editor.schedule_workspace_session_save(cx);
                    #[cfg(target_os = "macos")]
                    editor.schedule_platform_document_menu_refresh(cx);
                    cx.notify();
                }
                crate::document_host::DocumentHostEvent::SplitRatioChanged(ratio) => {
                    editor.split_pane_ratio = ratio.clamp(0.3, 0.7);
                    editor.schedule_workspace_session_save(cx);
                    cx.notify();
                }
            }
        })
        .detach();
    }

    /// 单窗与子窗格都按当前无障碍身份刷新；不把显示更新当作正文事件或重复保存请求。
    pub(in crate::editor) fn observe_document_host_accessibility(
        view: &Entity<crate::document_host::DocumentHost>,
        cx: &mut Context<Self>,
    ) {
        cx.observe(view, |editor, _view, cx| {
            if editor.accessibility_bridge.is_some()
                && editor.accessibility_revision != Some(editor.current_accessibility_revision(cx))
            {
                cx.notify();
            }
        })
        .detach();
    }

    pub(crate) fn from_recovered(
        cx: &mut Context<Self>,
        recovered: crate::recovery::RecoveredDocument,
    ) -> Self {
        let journal = Arc::new(Mutex::new(crate::recovery::RecoveryJournal::resume(
            &recovered,
        )));
        let target_mode = match recovered.view_mode.as_str() {
            "source" => ViewMode::Source,
            "split" => ViewMode::Split,
            "preview" => ViewMode::Preview,
            _ => ViewMode::Rendered,
        };
        let selection =
            UndoSelectionSnapshot::from_source_selection(recovered.selection.source_selection());
        let base_file_changed = recovered.base_file_changed;
        if recovered.read_status == crate::recovery::RecoveryReadStatus::TruncatedTail {
            eprintln!(
                "recovery journal '{}' had a corrupt tail; restored the last CRC-valid record",
                recovered.journal_path.display()
            );
        }
        if recovered.base_file_changed {
            eprintln!(
                "recovered document base changed externally: {}",
                recovered.file_path.as_deref().map_or_else(
                    || "<untitled>".to_owned(),
                    |path| path.display().to_string()
                )
            );
        }

        let recovered_path = recovered.file_path.clone();
        let recovered_source = recovered.source.clone();
        let recovered_source_document =
            match super::document_session::EditorDocumentSession::try_new_with_initial_dirty(
                SourceDocument::new(&recovered_source),
                true,
            ) {
                Ok(source_document) => source_document,
                Err(error) => {
                    eprintln!("recovery document initialization failed: {error}");
                    let mut editor = Self::from_markdown_internal(
                        cx,
                        String::new(),
                        recovered_path,
                        false,
                        false,
                        None,
                    );
                    editor.file_open_failure = Some(FileOpenFailure {
                        path: recovered
                            .file_path
                            .unwrap_or_else(|| recovered.journal_path.clone()),
                        reason: error.to_string(),
                        action_error: None,
                    });
                    editor.document_dirty = false;
                    editor.pending_window_edited = false;
                    return editor;
                }
            };
        let mut editor = Self::from_markdown_internal(
            cx,
            recovered_source,
            recovered.file_path,
            false,
            false,
            Some(recovered_source_document),
        );
        match editor
            .source_document
            .try_restore_source_format(recovered.source_format)
        {
            Ok(true) => {}
            Ok(false) => eprintln!("恢复日志中的源码格式与恢复文本不匹配，已使用默认格式"),
            Err(error) => eprintln!("恢复日志中的源码格式提交失败: {error}"),
        }
        editor.recovery_journal = Some(journal);
        editor.external_file_conflict = base_file_changed;
        editor.recovered_session = true;
        if target_mode != ViewMode::Rendered {
            editor.set_view_mode(target_mode, cx);
        }
        editor.apply_selection_snapshot_in_current_mode(&selection, cx);
        editor.last_selection_snapshot = selection;
        editor.document_dirty = true;
        editor.pending_window_edited = true;
        editor.pending_window_title_refresh = true;
        editor
    }

    #[cfg(test)]
    pub(in crate::editor) fn from_markdown_virtualized(
        cx: &mut Context<Self>,
        markdown: String,
        file_path: Option<PathBuf>,
    ) -> Self {
        Self::from_markdown_internal(cx, markdown, file_path, true, false, None)
    }

    /// Build an Editor view over an already-open process-wide Resident document.
    /// The lease moves into the view; only immutable snapshots feed projection/UI state.
    pub(crate) fn from_shared_resident_open(
        cx: &mut Context<Self>,
        shared: crate::app::document_service::SharedResidentOpen,
        file_path: Option<PathBuf>,
    ) -> Result<Self, super::document_session::EditorDocumentSessionError> {
        Self::from_shared_resident_open_with_view_id(
            cx,
            shared,
            file_path,
            gmark_document_core::DocumentViewInstanceId::new(),
        )
    }

    /// Build a shared Resident view using a persisted view identity.  Restore
    /// callers pass the durable tab UUID so Controller view-local selection
    /// state reconnects to the same tab instead of silently creating a new
    /// random view.
    pub(crate) fn from_shared_resident_open_with_view_id(
        cx: &mut Context<Self>,
        shared: crate::app::document_service::SharedResidentOpen,
        file_path: Option<PathBuf>,
        view_id: gmark_document_core::DocumentViewInstanceId,
    ) -> Result<Self, super::document_session::EditorDocumentSessionError> {
        let source_document =
            super::document_session::EditorDocumentSession::from_lease_with_view_id(
                shared.lease,
                view_id,
            )?;
        let markdown = source_document.try_text()?;
        let source_encoding = shared.encoding;
        let mut editor = Self::from_markdown_internal(
            cx,
            markdown,
            file_path,
            false,
            false,
            Some(source_document),
        );
        editor.source_encoding = source_encoding;
        Ok(editor)
    }

    /// Construct a document-only child view over an already-open Markdown
    /// session. The session is explicitly forked by the caller when a pane
    /// needs an independent view cursor; this method never materializes a
    /// second source body or starts a duplicate file watcher.
    pub(in crate::editor) fn from_pane_session(
        cx: &mut Context<Self>,
        source_document: EditorDocumentSession,
        file_path: Option<PathBuf>,
        pane_tab_id: uuid::Uuid,
        view_state: crate::editor::panes::PaneViewStateSnapshot,
    ) -> Self {
        let identity = file_path
            .as_deref()
            .map(|path| {
                crate::editor::markdown_view_state::MarkdownTabIdentity::saved(path, pane_tab_id)
            })
            .unwrap_or_else(|| {
                crate::editor::markdown_view_state::MarkdownTabIdentity::untitled(pane_tab_id)
            });
        let mut editor = Self::from_markdown_internal(
            cx,
            String::new(),
            file_path,
            false,
            false,
            Some(source_document),
        );
        editor.pane_canvas = true;
        editor.pane_canvas_focus_enabled = false;
        editor.pane_tab_id = Some(pane_tab_id);
        let _ = editor.view_state.open_tab(identity);
        editor.restore_pane_view_state(view_state, cx);
        editor
    }

    /// Build a recovery view over the service-owned resident document.  The
    /// recovery payload supplies only view/journal metadata here; its source
    /// body must already have been moved into `SharedResidentOpen` by
    /// `DocumentService::open_recovery`.
    pub(crate) fn from_shared_recovery(
        cx: &mut Context<Self>,
        shared: crate::app::document_service::SharedResidentOpen,
        recovered: crate::recovery::RecoveredDocument,
    ) -> Result<Self, super::document_session::EditorDocumentSessionError> {
        Self::from_shared_recovery_with_view_id(
            cx,
            shared,
            recovered,
            gmark_document_core::DocumentViewInstanceId::new(),
        )
    }

    /// Recovery counterpart of [`Self::from_shared_resident_open_with_view_id`].
    /// The recovered payload supplies view metadata while the service-owned
    /// lease supplies the authoritative body and source format.
    pub(crate) fn from_shared_recovery_with_view_id(
        cx: &mut Context<Self>,
        shared: crate::app::document_service::SharedResidentOpen,
        recovered: crate::recovery::RecoveredDocument,
        view_id: gmark_document_core::DocumentViewInstanceId,
    ) -> Result<Self, super::document_session::EditorDocumentSessionError> {
        let journal = Arc::new(Mutex::new(crate::recovery::RecoveryJournal::resume(
            &recovered,
        )));
        let target_mode = match recovered.view_mode.as_str() {
            "source" => ViewMode::Source,
            "split" => ViewMode::Split,
            "preview" => ViewMode::Preview,
            _ => ViewMode::Rendered,
        };
        let selection =
            UndoSelectionSnapshot::from_source_selection(recovered.selection.source_selection());
        let base_file_changed = recovered.base_file_changed;
        let mut editor = Self::from_shared_resident_open_with_view_id(
            cx,
            shared,
            recovered.file_path.clone(),
            view_id,
        )?;
        let resident_format = editor.source_document.try_source_format()?;
        if resident_format != recovered.source_format {
            return Err(
                super::document_session::EditorDocumentSessionError::RecoveryFormatMismatch,
            );
        }
        editor.recovery_journal = Some(journal);
        editor.external_file_conflict = base_file_changed;
        editor.recovered_session = true;
        if target_mode != ViewMode::Rendered {
            editor.set_view_mode(target_mode, cx);
        }
        editor.apply_selection_snapshot_in_current_mode(&selection, cx);
        editor.last_selection_snapshot = selection;
        editor.document_dirty = editor.source_document.is_dirty();
        editor.pending_window_edited = editor.document_dirty;
        editor.pending_window_title_refresh = true;
        Ok(editor)
    }

    /// 共享正文复用既有会话；输入法队列、选择与拖选任务属于视图，不能随正文共享或恢复。
    pub(in crate::editor) fn from_markdown_internal(
        cx: &mut Context<Self>,
        markdown: String,
        file_path: Option<PathBuf>,
        force_virtual_surface: bool,
        initial_dirty: bool,
        source_document_override: Option<EditorDocumentSession>,
    ) -> Self {
        let construction_started = perf::start();
        let shared_document = source_document_override.is_some();
        let document_kind = file_path
            .as_deref()
            .map(DocumentKind::from_path)
            .unwrap_or(DocumentKind::Markdown);
        let source_document = source_document_override.unwrap_or_else(|| {
            let (file_identity, text_encoding) = file_path
                .as_deref()
                .and_then(|path| {
                    gmark_paged_document::probe_file(
                        path,
                        gmark_paged_document::ProbeOptions::default(),
                    )
                    .ok()
                    .map(|probe| (Some(probe.identity), probe.encoding))
                })
                .unwrap_or((None, gmark_document_core::TextEncoding::Utf8 { bom: false }));
            match EditorDocumentSession::try_new_with_open_context_and_dirty(
                SourceDocument::new(&markdown),
                gmark_document_core::LoadingPolicy::default().effective_limits(),
                text_encoding,
                file_identity,
                initial_dirty,
            ) {
                Ok(source_document) => source_document,
                Err(error) => {
                    eprintln!("editor document initialization failed: {error}");
                    EditorDocumentSession::shell()
                }
            }
        });
        let normalized = source_document.text();
        let saved_file_fingerprint = file_path
            .as_deref()
            .and_then(|path| crate::recovery::fingerprint_file(path).ok());
        #[cfg(not(test))]
        let recovery_journal = crate::config::AppDirs::from_system()
            .and_then(|dirs| {
                let recovery_dir = dirs.recovery_dir();
                dirs.ensure_state_parent(&recovery_dir.join(".gmark-recovery-root"))?;
                crate::recovery::RecoveryJournal::create(
                    &recovery_dir,
                    file_path.clone(),
                    markdown.clone(),
                )
            })
            .map(|journal| Arc::new(Mutex::new(journal)))
            .map_err(|error| eprintln!("failed to initialize recovery journal: {error}"))
            .ok();
        #[cfg(test)]
        let recovery_journal = None;
        let projection = Arc::new(PreparedSplitProjection::from_snapshot_adaptive(
            source_document.snapshot(),
            Self::VIRTUAL_SURFACE_REGION_THRESHOLD,
        ));
        let initial_document_dirty = source_document.try_is_dirty().unwrap_or(initial_dirty);
        let virtual_surface =
            (force_virtual_surface || Self::should_virtualize_projection(&projection)).then(|| {
                let mut surface = VirtualSurfaceState::new(Arc::clone(&projection));
                let initial_window = surface.desired_window(0.0, 720.0, 800.0, Some(0));
                surface.reconcile_mounts(initial_window, cx);
                surface
            });
        // 初次 Live 构建也必须携带原源码归属；否则首次输入只能得到规范投影，不能安全写回。
        let (mut roots, mut source_regions) = if let Some(surface) = virtual_surface.as_ref() {
            (surface.viewport_roots(), Vec::new())
        } else {
            Self::build_blocks_from_projection_reusing_with_regions(
                cx,
                &projection,
                &mut HashMap::new(),
            )
        };
        if roots.is_empty() {
            roots.push(Self::new_block(cx, BlockRecord::paragraph(String::new())));
            if virtual_surface.is_none() {
                source_regions.push((0..normalized.len(), 0..1));
            }
        }

        let mut document = DocumentTree::new(roots);
        document.rebuild_metadata_and_snapshot(cx);
        if virtual_surface.is_none() {
            document.bind_source_regions(projection.revision, source_regions, cx);
        }
        let mut status_bar = StatusBarState::default();
        status_bar.set_word_count(
            source_document.revision(),
            status_bar::count_characters(&normalized),
        );
        let pending_focus = document
            .root_blocks()
            .iter()
            .find(|block| {
                let block = block.read(cx);
                block.kind() != BlockKind::Comment && !block.record.is_yaml_frontmatter()
            })
            .or_else(|| document.first_root())
            .map(|block| block.entity_id());
        let last_stable_source = HistorySource::capture(source_document.snapshot(), normalized);
        let render_assets = cx
            .try_global::<crate::editor::render_asset_manager::SharedRenderAssetManager>()
            .cloned()
            .unwrap_or_else(|| {
                let shared =
                    crate::editor::render_asset_manager::SharedRenderAssetManager::default();
                cx.set_global(shared.clone());
                shared
            });
        let view_state = cx
            .try_global::<crate::editor::markdown_view_state::SharedMarkdownViewState>()
            .cloned()
            .unwrap_or_else(|| {
                let shared = crate::editor::markdown_view_state::SharedMarkdownViewState::default();
                cx.set_global(shared.clone());
                shared
            });

        let mut editor = Self {
            accessibility_bridge: None,
            accessibility_wake_task: None,
            accessibility_revision: None,
            document_host: None,
            source_document,
            shared_document,
            shared_event_task: None,
            pane_canvas: false,
            pane_canvas_focus_enabled: true,
            pane_canvas_viewport: None,
            pane_tab_id: None,
            pane_history_back: Vec::new(),
            pane_history_forward: Vec::new(),
            pane_host_path: None,
            pane_host_probe: None,
            source_encoding: file_path
                .as_deref()
                .and_then(|path| {
                    gmark_paged_document::probe_file(
                        path,
                        gmark_paged_document::ProbeOptions::default(),
                    )
                    .ok()
                    .map(|probe| match probe.encoding {
                        gmark_document_core::TextEncoding::Utf8 { .. } => {
                            crate::document_io::DocumentEncoding::Utf8
                        }
                        gmark_document_core::TextEncoding::Utf16Le => {
                            crate::document_io::DocumentEncoding::Legacy("UTF-16LE".to_owned())
                        }
                        gmark_document_core::TextEncoding::Utf16Be => {
                            crate::document_io::DocumentEncoding::Legacy("UTF-16BE".to_owned())
                        }
                        gmark_document_core::TextEncoding::Legacy(label) => {
                            crate::document_io::DocumentEncoding::Legacy(label)
                        }
                    })
                })
                .unwrap_or(crate::document_io::DocumentEncoding::Utf8),
            document_kind,
            document_epoch: 0,
            render_asset_scope: uuid::Uuid::new_v4(),
            projection_cache: Some(projection),
            source_spelling_cache: std::cell::RefCell::new(None),
            document,
            split_preview: None,
            split_pane_ratio: 0.5,
            split_resize_session: None,
            split_divider_focus_handle: cx.focus_handle(),
            document_toolbar_focus_handles: std::array::from_fn(|_| cx.focus_handle()),
            image_preview_focus_handles: std::array::from_fn(|_| cx.focus_handle()),
            image_preview_tile_ids: Vec::new(),
            file_open_failure_focus_handles: std::array::from_fn(|_| cx.focus_handle()),
            update_primary_focus_handle: cx.focus_handle(),
            update_secondary_focus_handle: cx.focus_handle(),
            table_cells: HashMap::new(),
            render_assets,
            render_asset_tasks: HashMap::new(),
            view_state,
            view_mode: ViewMode::Rendered,
            pending_focus,
            active_entity_id: pending_focus,
            pending_scroll_active_block_into_view: true,
            pending_scroll_recheck_after_layout: true,
            pending_save: false,
            pending_save_as: false,
            pending_ime_operations: std::collections::VecDeque::new(),
            ime_completion_requested: false,
            ime_completion_failed: false,
            ime_detached_targets: Vec::new(),
            pending_resource_insertion: None,
            save_task: None,
            save_prompt_task: None,
            save_queued: false,
            auto_save_task: None,
            spellcheck_task: None,
            export_task: None,
            export_cancel: None,
            export_progress: None,
            export_in_progress: false,
            export_cancel_requested: false,
            pending_open_link: None,
            pending_window_edited: false,
            pending_window_title_refresh: false,
            document_dirty: initial_document_dirty,
            file_path,
            image_preview_path: None,
            image_preview_zoom: 1.0,
            svg_preview_cache: None,
            file_open_failure: None,
            saved_file_fingerprint,
            file_watch_guard: None,
            file_watch_task: None,
            external_file_conflict: false,
            recovered_session: false,
            show_external_conflict_dialog: false,
            show_encoding_conversion_dialog: false,
            external_conflict_preview: None,
            external_conflict_restore_focus: None,
            allow_external_overwrite_once: false,
            scroll_handle: ScrollHandle::new(),
            last_scroll_viewport_size: None,
            prev_visible_block_ids: Vec::new(),
            row_stride_cache: HashMap::new(),
            row_stride_layout_identity: None,
            split_row_stride_layout_identity: None,
            render_row_cache: None,
            prev_render_window: None,
            close_guard_installed: false,
            show_unsaved_changes_dialog: false,
            pending_close_after_save: false,
            close_dialog_restore_focus: None,
            close_dialog_focus_handles: std::array::from_fn(|_| cx.focus_handle()),
            close_dialog_keyboard_index: 2,
            close_dialog_restore_input_focus: None,
            pending_drop_replace_path: None,
            show_drop_replace_dialog: false,
            pending_drop_replace_after_save: false,
            drop_replace_restore_focus: None,
            info_dialog: None,
            workspace: WorkspaceState::default(),
            tabs: tabs::TabState::new(),
            pane_workspace: None,
            pane_events: Rc::new(RefCell::new(Vec::new())),
            pane_notice: None,
            pane_notice_task: None,
            pane_close_generation: 0,
            pane_close_target: None,
            pane_close_save_signal: None,
            pane_close_save_markdown_editor: None,
            pane_close_save_subscription: None,
            pane_close_save_task: None,
            pane_canvas_entities: Rc::new(RefCell::new(BTreeMap::new())),
            focus_mode: false,
            typewriter_mode: false,
            status_bar,
            context_menu: None,
            resource_title_dialog: None,
            context_menu_keyboard_item: None,
            context_menu_keyboard_submenu_item: None,
            context_menu_scroll_handle: ScrollHandle::new(),
            command_palette: None,
            find_panel: None,
            table_insert_dialog: None,
            context_menu_submenu_close_task: None,
            table_axis_preview: None,
            table_axis_selection: None,
            table_cell_rectangle: None,
            table_cell_drag_anchor: None,
            table_fragment_merge: None,
            diagram_overlay: None,
            diagram_overlay_restore_focus: None,
            workspace_link_completion: None,
            cross_block_selection: None,
            cross_block_drag: None,
            active_selection_surface: selection_surface::SelectionSurface::Main,
            split_preview_cross_block_selection: None,
            split_preview_cross_block_drag: None,
            split_preview_table_cell_rectangle: None,
            split_preview_table_cell_drag_anchor: None,
            selection_autoscroll_task: None,
            selection_autoscroll_generation: 0,
            selection_autoscroll_pointer: None,
            selection_autoscroll_surface: selection_surface::SelectionSurface::Main,
            rendered_select_all_cycle: None,
            // 桌面 Markdown 编辑器的高频导航需始终可见；G 启动器仍可由用户手动收纳。
            menu_bar_expanded: true,
            menu_window_activation_subscription: None,
            menu_bar_open: None,
            menu_submenu_open: None,
            menu_keyboard_item: None,
            menu_keyboard_submenu_item: None,
            menu_bar_hovered: false,
            menu_panel_hovered: false,
            menu_submenu_panel_hovered: false,
            menu_submenu_bridge_hovered: false,
            menu_close_task: None,
            scrollbar_hovered: false,
            scrollbar_thumb_hovered: false,
            scrollbar_visible_until: Instant::now(),
            scrollbar_fade_task: None,
            smooth_scroll_animation: None,
            smooth_scroll_task: None,
            split_preview_scrollbar_hovered: false,
            split_preview_scrollbar_visible_until: Instant::now(),
            split_preview_scrollbar_fade_task: None,
            scroll_recheck_task: None,
            projection_cache_task: None,
            projection_cache_scheduled_revision: None,
            split_projection_task: None,
            split_projection_scheduled_revision: None,
            recovery_journal,
            recovery_task: None,
            recovery_generation: 0,
            scrollbar_drag: None,
            split_preview_scrollbar_drag: None,
            undo_history: Vec::new(),
            redo_history: Vec::new(),
            pending_undo_capture: None,
            virtual_undo_selections: Vec::new(),
            virtual_redo_selections: Vec::new(),
            pending_virtual_undo_selection: None,
            last_selection_snapshot: Self::empty_selection_snapshot(),
            last_stable_source,
            pending_dirty_source: None,
            history_restore_in_progress: false,
            image_reference_definitions: Arc::default(),
            link_reference_definitions: Arc::default(),
            footnote_registry: Arc::default(),
            pending_virtual_global_runtime_refresh: false,
            pending_virtual_footnote_focus: None,
            pending_virtual_footnote_backref_focus: None,
            virtual_surface,
            first_render_started: construction_started,
        };
        if editor.virtual_surface.is_some() {
            editor.rebuild_virtual_table_runtimes(cx);
            let source = editor.source_document.text();
            editor.rebuild_runtime_context_from_markdown(&source, cx);
        } else {
            editor.rebuild_table_runtimes(cx);
        }
        editor.pending_focus = editor.first_focusable_entity_id(cx);
        editor.active_entity_id = editor.pending_focus;
        editor.refresh_stable_document_snapshot(cx);
        if !editor.shared_document {
            editor.restart_file_watcher(cx);
        } else {
            editor.start_shared_event_pump(cx);
        }
        editor.schedule_active_block_spellcheck(cx);
        if let Some(started) = construction_started {
            perf::emit(
                "editor_construct",
                started,
                Some(editor.source_document.len()),
                Some(true),
                None,
            );
        }
        editor
    }
}
