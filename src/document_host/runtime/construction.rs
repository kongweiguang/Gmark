// @author kongweiguang

//! DocumentHost construction and untitled-session setup.

use super::*;

impl DocumentHost {
    pub(crate) fn new(
        path: PathBuf,
        probe: OpenProbe,
        source: FileSource,
        cx: &mut Context<Self>,
    ) -> Self {
        // Compatibility callers still begin with a bounded provisional source
        // window.  The single async indexing path below installs the one
        // Controller-backed session once its identity/index are ready; keeping
        // this phase provisional preserves read-only rows and avoids a second
        // temporary body while the worker is running.
        Self::new_with_source(path, probe, Some(source), cx)
    }

    /// Builds every persistent Host input once so IME owner subscriptions survive view remounts.
    pub(super) fn new_with_source(
        path: PathBuf,
        probe: OpenProbe,
        source: Option<FileSource>,
        cx: &mut Context<Self>,
    ) -> Self {
        let file_backed = source.is_some();
        let strings = cx
            .try_global::<I18nManager>()
            .map(I18nManager::strings_arc)
            .unwrap_or_else(|| Arc::new(I18nStrings::en_us()));
        let preview_lines = source.as_ref().map_or_else(Vec::new, |source| {
            source
                .read_range(0, probe.len.min(PREFIX_PREVIEW_BYTES))
                .map(|bytes| decode_provisional_bytes(&bytes, &probe.encoding, 0))
                .map(|text| {
                    text.lines()
                        .map(|line| SharedString::from(line.trim_end_matches('\r').to_owned()))
                        .collect()
                })
                .unwrap_or_else(|error| {
                    vec![
                        strings
                            .large_document_text("decode_first_window")
                            .replace("{error}", &error.to_string())
                            .into(),
                    ]
                })
        });
        let search_placeholder = strings.large_document_text("find_placeholder").to_owned();
        let search_input = cx.new(move |cx| {
            let mut block = Block::with_record(
                cx,
                BlockRecord::with_plain_text(BlockKind::Paragraph, String::new()),
            );
            block.set_source_raw_mode();
            block.set_input_placeholder(search_placeholder);
            block.set_host_submit_enabled(true);
            block
        });
        cx.subscribe(&search_input, Self::on_search_input_event)
            .detach();
        cx.subscribe(&search_input, Self::on_host_input_ime_event)
            .detach();
        let navigation_placeholder = strings
            .large_document_text("go_to_line_placeholder")
            .to_owned();
        let navigation_input = cx.new(move |cx| {
            let mut block = Block::with_record(
                cx,
                BlockRecord::with_plain_text(BlockKind::Paragraph, String::new()),
            );
            block.set_source_raw_mode();
            block.set_input_placeholder(navigation_placeholder);
            block.set_host_submit_enabled(true);
            block
        });
        cx.subscribe(&navigation_input, Self::on_navigation_input_event)
            .detach();
        cx.subscribe(&navigation_input, Self::on_host_input_ime_event)
            .detach();
        let structured_filter_placeholder = if probe.format == DocumentFormat::Json {
            cx.try_global::<I18nManager>()
                .map(|manager| manager.strings().json_graph_search_placeholder.clone())
                .unwrap_or_else(|| strings.json_graph_search_placeholder.clone())
        } else {
            strings.large_document_text("filter_rows").to_owned()
        };
        let structured_filter_input = cx.new(move |cx| {
            let mut block = Block::with_record(
                cx,
                BlockRecord::with_plain_text(BlockKind::Paragraph, String::new()),
            );
            block.set_source_raw_mode();
            block.set_input_placeholder(structured_filter_placeholder);
            block
        });
        let graph_edit_input = cx.new(|cx| {
            let mut block = Block::with_record(
                cx,
                BlockRecord::with_plain_text(BlockKind::Paragraph, String::new()),
            );
            block.set_source_raw_mode();
            block
        });
        cx.subscribe(&graph_edit_input, Self::on_host_input_ime_event)
            .detach();
        let structured_cell_input = cx.new(|cx| {
            let mut block = Block::with_record(
                cx,
                BlockRecord::with_plain_text(BlockKind::Paragraph, String::new()),
            );
            block.set_compact_source_host();
            block.set_host_text_size(12.0);
            block.set_host_submit_enabled(true);
            block
        });
        cx.subscribe(&structured_cell_input, Self::on_host_input_ime_event)
            .detach();
        cx.subscribe(
            &structured_filter_input,
            Self::on_structured_filter_input_event,
        )
        .detach();
        cx.subscribe(&structured_filter_input, Self::on_host_input_ime_event)
            .detach();
        let tail_enabled = path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("log"));
        // SourceBacked 只决定存储与物化策略；外层 Editor 再按格式能力决定首屏模式。
        let initial_view_mode = DocumentHostViewMode::Source;
        let lifetime_cancellation = SearchCancellation::default();
        let mut view_registry = DocumentViewRegistry::default();
        let mut selected_projection_view = None;
        let json_focused_roots = Arc::new(Mutex::new(HashMap::new()));
        if derived_views_enabled(probe.strategy) && probe.format == DocumentFormat::Json {
            let provider = JsonGraphProjectionProvider::new(json_focused_roots.clone());
            selected_projection_view = Some(provider.descriptor().id.clone());
            let registered = view_registry.register(Arc::new(provider));
            debug_assert!(registered, "built-in JSON graph view id must be unique");
        } else if derived_views_enabled(probe.strategy)
            && let Some(provider) = RegisteredStructuredProvider::for_format(&probe.format)
        {
            selected_projection_view = Some(provider.view_id());
            let registered = view_registry.register(Arc::new(provider));
            debug_assert!(registered, "built-in derived view id must be unique");
        }
        let source_language = SourceLanguageId::for_path(&path);
        let mut view = Self {
            path,
            probe,
            index: None,
            document: None,
            provisional_source: source,
            structured_index: None,
            structured_rows: BTreeMap::new(),
            structured_pending: None,
            structured_generation: 0,
            structured_cancellation: None,
            structure_error: None,
            structure_error_byte: None,
            structured_filter_input,
            structured_cell_input,
            structured_focus_handle: cx.focus_handle(),
            structured_cell_edit: None,
            structured_selected_cell: None,
            structured_cell_overrides: BTreeMap::new(),
            structured_cell_source_edits: Vec::new(),
            structured_context_target: None,
            structured_column_progress: None,
            structured_filter_column: None,
            structured_filtered_rows: Vec::new(),
            structured_filter_generation: 0,
            structured_filter_cancellation: None,
            structured_filter_running: false,
            hidden_structured_columns: BTreeSet::new(),
            structured_column_window_start: 0,
            json_child_indexes: BTreeMap::new(),
            json_expanded_nodes: BTreeSet::new(),
            json_rows: BTreeMap::new(),
            json_expand_generation: 0,
            json_expand_cancellation: None,
            view_registry,
            tab_view_state: DocumentViewState::default(),
            view_back_history: VecDeque::new(),
            view_forward_history: VecDeque::new(),
            selected_projection_view,
            document_epoch: 1,
            derived_projection_generation: 0,
            derived_projection_cancellation: None,
            derived_projection_snapshot: None,
            derived_projection_error: None,
            derived_projection_error_offset: None,
            derived_projection_stale: false,
            derived_projection_root: None,
            json_focused_roots,
            graph_selected_item: None,
            graph_search_matches: Vec::new(),
            graph_search_selected: 0,
            graph_search_collapsed_before: None,
            graph_context_menu: None,
            graph_edit_target: None,
            graph_edit_input,
            graph_edit_error: None,
            graph_edit_issue: None,
            graph_edit_original: None,
            graph_state_initialized: false,
            graph_projection_identity: None,
            graph_row_limits: HashMap::new(),
            graph_layout_cache: None,
            graph_needs_fit: true,
            graph_fit_all_requested: false,
            graph_last_viewport: None,
            graph_pan_session: None,
            graph_pending_center: None,
            graph_recenter_anchor: None,
            graph_focus_handle: cx.focus_handle(),
            graph_focus_subscription: None,
            json_split_ratio: 0.5,
            json_split_drag: None,
            json_split_focus_handle: cx.focus_handle(),
            derived_projection_task: Task::ready(()),
            view_mode: initial_view_mode,
            preview_lines,
            source_rows: BTreeMap::new(),
            displayed_screen_lines: Arc::new(ScreenLines::default()),
            metrics: PagedDocumentMetrics::default(),
            first_render_started: crate::perf::start(),
            source_row_blocks: BTreeMap::new(),
            pending_source_ime_action: None,
            source_ime_snapshot: None,
            source_typing_group: None,
            source_pending_undo_capture: None,
            source_syntax_contexts: BTreeMap::new(),
            source_row_epochs: BTreeMap::new(),
            source_cache_epoch: 0,
            soak_ready_published: false,
            source_pending: None,
            source_queued_visible: None,
            source_last_visible: None,
            source_list_origin: 0,
            source_language,
            fold_projection: FoldProjectionIndex::default(),
            fold_parser: Arc::new(Mutex::new(ResidentFoldParser::default())),
            fold_snapshot_revision: None,
            fold_window: None,
            fold_generation: 0,
            fold_cancellation: None,
            fold_task: Task::ready(()),
            folding_enabled: crate::preferences::EditorSettings::code_folding(cx),
            format_generation: 0,
            format_cancellation: None,
            format_task: Task::ready(()),
            format_running: false,
            save_after_format: None,
            source_cancel_in_flight: false,
            source_row_height: FALLBACK_SOURCE_ROW_HEIGHT,
            active_edit: None,
            source_cursor_column: SourceCursorColumnState::default(),
            suppressed_line_edit_text: None,
            selection_anchor: None,
            selected_lines: None,
            source_drag_anchor: None,
            source_drag_anchor_range: None,
            source_drag_granularity: super::source_pointer::SourceDragGranularity::Character,
            source_drag_autoscroll_direction: 0,
            source_drag_autoscroll_task: Task::ready(()),
            document_host_bounds: Arc::new(Mutex::new(None)),
            source_context_menu: None,
            source_context_menu_focus_handle: cx.focus_handle(),
            search_input,
            search_visible: false,
            navigation_input,
            navigation_visible: false,
            navigation_is_byte: false,
            show_line_endings: false,
            search_options: SearchOptions::default(),
            search_results: Vec::new(),
            search_selected: 0,
            search_running: false,
            search_error: None,
            mode_notice: None,
            tail_enabled,
            external_monitor_owned: file_backed,
            controller_events: None,
            controller_event_task: Task::ready(()),
            saving: false,
            reloading: false,
            error: None,
            coordinator: DocumentCoordinator::new(lifetime_cancellation),
            focus_handle: cx.focus_handle(),
            scroll_handle: UniformListScrollHandle::new(),
            structured_scroll_handle: UniformListScrollHandle::new(),
            structured_horizontal_scroll_handle: ScrollHandle::new(),
            source_window_start: 0,
            pending_source_collapsed_folds: BTreeSet::new(),
            provisional_anchor: Some(SourceAnchor::new(0, SourceAffinity::Before)),
            closed_suspended: false,
            structured_task: Task::ready(()),
            structured_progress_task: Task::ready(()),
            structured_filter_task: Task::ready(()),
            json_expand_task: Task::ready(()),
            clipboard_generation: 0,
            clipboard_cancellation: None,
            clipboard_task: Task::ready(()),
            #[cfg(test)]
            fail_next_native_clipboard_write_for_test: false,
            selection_export_generation: 0,
            selection_export_cancellation: None,
            selection_export_task: Task::ready(()),
        };
        view.fold_projection.set_real_line_count(view.line_count());
        if file_backed {
            view.start_initial_index(cx);
            view.start_external_monitor(cx);
        }
        view
    }

    /// 新建格式标签直接安装内存 Resident session；逻辑路径只负责语言与格式能力，
    /// 不创建用户看不见的临时文件，也不会在首次保存时绕过“另存为”。
    pub(crate) fn new_untitled(
        logical_path: PathBuf,
        format: DocumentFormat,
        initial_source: &'static str,
        cx: &mut Context<Self>,
    ) -> Self {
        let len = initial_source.len() as u64;
        let estimated_lines = initial_source.lines().count().max(1) as u64;
        let estimated_structural_units = initial_source
            .bytes()
            .filter(|byte| matches!(byte, b'|' | b',' | b'\t' | b'{' | b'}' | b'[' | b']'))
            .count() as u64;
        let source_identity = gmark_paged_document::FileIdentity {
            path: logical_path.clone(),
            len,
            modified_nanos: None,
            os_file_id: None,
        };
        let probe = OpenProbe {
            len,
            identity: source_identity.clone(),
            options: gmark_paged_document::ProbeOptions::default(),
            format: format.clone(),
            encoding: TextEncoding::Utf8 { bom: false },
            strategy: OpenStrategy::Resident,
            force_safe_source: false,
            estimated_lines,
            estimated_structural_units,
        };
        let profile = probe.profile();
        let plan = session_plan(&profile, &probe, OpenStrategy::Resident, true);
        let store = gmark_document_runtime::DocumentStore::Resident(Box::new(
            gmark_document_runtime::ResidentDocument::new(
                initial_source,
                probe.encoding.clone(),
                source_identity.clone(),
            ),
        ));
        let document = DocumentSession::new(
            profile,
            store,
            plan,
            gmark_document_runtime::FileIdentity::from(&source_identity),
        )
        .expect("untitled format document must resolve to a resident session");
        let preview_lines = initial_source
            .lines()
            .map(|line| SharedString::from(line.to_owned()))
            .collect();
        let mut view = Self::new_with_source(logical_path, probe, None, cx);
        view.preview_lines = preview_lines;
        view.install_document_session(document);

        if let DocumentFormat::Delimited { delimiter } = format {
            let snapshot: Arc<[u8]> = initial_source.as_bytes().to_vec().into();
            match DelimitedIndex::build_snapshot_cancellable(
                snapshot,
                DelimitedIndexOptions {
                    delimiter,
                    ..DelimitedIndexOptions::default()
                },
                &SearchCancellation::default(),
            ) {
                Ok(index) => view.structured_index = Some(StructuredIndex::Delimited(index)),
                Err(error) => view.error = Some(localized_document_error(&error, cx)),
            }
        }
        view
    }
}
