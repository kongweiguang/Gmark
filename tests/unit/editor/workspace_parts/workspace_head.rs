// @author kongweiguang

    use super::{
        DOCUMENT_SIDEBAR_COMPACT_OVERLAY_WIDTH, DOCUMENT_SIDEBAR_PANEL_MAX_WIDTH,
        DOCUMENT_SIDEBAR_PANEL_AUTO_MIN_WIDTH, DOCUMENT_SIDEBAR_PANEL_MIN_WIDTH,
        WORKSPACE_COMPACT_OVERLAY_WIDTH, WORKSPACE_PANEL_AUTO_MIN_WIDTH, WORKSPACE_PANEL_MAX_WIDTH,
        WORKSPACE_PANEL_MIN_WIDTH, WORKSPACE_RESIZE_HIT_WIDTH, WORKSPACE_SCAN_MAX_ENTRIES,
        WorkspaceKeyboardZone, WorkspaceSearchMatch,
        WorkspaceSearchOptions, WorkspaceSelection, WorkspaceState, WorkspaceTab,
        WorkspaceScanState, WorkspaceTreeKind, build_outline_tree, insert_workspace_directory,
        prune_outline_state, rank_quick_open_paths, scan_workspace, scan_workspace_dir,
        search_workspace,
        document_sidebar_panel_width_for_viewport, workspace_panel_width_for_viewport,
        workspace_uses_overlay,
    };
    use gpui::{AppContext as _, KeyDownEvent, Keystroke, Modifiers, MouseButton, point, px, size};
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::AtomicBool;

    fn init_workspace_test_app(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            crate::i18n::I18nManager::init(cx);
            crate::theme::ThemeManager::init(cx);
            crate::components::init(cx);
        });
    }

    fn key_event(key: &str) -> KeyDownEvent {
        KeyDownEvent {
            keystroke: Keystroke::parse(key).expect("valid workspace test keystroke"),
            is_held: false,
        }
    }

    fn assert_workspace_header_layout(visual: &mut gpui::VisualTestContext) {
        let panel = visual.debug_bounds("workspace-panel").unwrap();
        assert!(visual.debug_bounds("workspace-panel-header").is_none());
        assert!(visual.debug_bounds("workspace-tab-files").is_none());
        assert!(visual.debug_bounds("workspace-tab-search").is_none());
        assert!(visual.debug_bounds("workspace-collapse").is_none());
        assert!(panel.size.height > px(0.0));
    }

    #[gpui::test]
    async fn workspace_panel_has_no_invisible_top_controls(cx: &mut gpui::TestAppContext) {
        init_workspace_test_app(cx);
        let (editor, visual) = cx.add_window_view(|_window, cx| {
            super::Editor::from_markdown(cx, "# Heading".to_owned(), None)
        });
        editor.update(visual, |editor, cx| {
            editor.workspace.is_open = true;
            cx.notify();
        });
        visual.update(|window, cx| window.draw(cx).clear());
        assert_workspace_header_layout(visual);
    }

    #[test]
    fn workspace_scan_keeps_dirs_and_all_file_types() {
        let root =
            std::env::temp_dir().join(format!("gmark-workspace-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(root.join("nested")).expect("create dirs");
        fs::write(root.join("a.md"), "a").expect("write md");
        fs::write(root.join("a.txt"), "text").expect("write txt");
        fs::write(root.join(".secret.md"), "secret").expect("write hidden md");
        fs::create_dir_all(root.join(".private")).expect("create hidden dir");
        fs::write(root.join(".private").join("note.md"), "private")
            .expect("write file in hidden dir");
        fs::write(root.join("ignored.md"), "ignored").expect("write ignored md");
        fs::write(root.join(".gitignore"), "ignored.md\n").expect("write gitignore");
        fs::write(root.join("nested").join("b.md"), "b").expect("write nested md");

        let tree = scan_workspace_dir(&root).expect("scan tree");
        let labels = tree
            .children
            .iter()
            .map(|node| node.label.as_str())
            .collect::<Vec<_>>();
        assert_eq!(labels, vec!["nested", "a.md", "a.txt"]);
        assert!(matches!(
            tree.children[0].kind,
            WorkspaceTreeKind::Directory(_)
        ));
        assert!(matches!(
            tree.children[1].kind,
            WorkspaceTreeKind::File(_)
        ));
        let canonical_root = dunce::canonicalize(&root).expect("canonical root");
        let workspace = WorkspaceState {
            root: Some(canonical_root.clone()),
            file_tree: Some(tree.clone()),
            ..WorkspaceState::default()
        };
        assert!(workspace.snapshot_path_is_file(&canonical_root.join("a.txt")));
        assert_eq!(
            workspace.snapshot_path_is_directory(&canonical_root.join("nested")),
            Some(true)
        );
        assert_eq!(
            workspace.snapshot_path_is_directory(&canonical_root.join("missing")),
            None
        );

        let cancelled = AtomicBool::new(false);
        let visible = scan_workspace(&root, &[], true, &cancelled).expect("scan hidden files");
        let visible_labels = visible
            .tree
            .children
            .iter()
            .map(|node| node.label.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            visible_labels,
            vec![
                ".private",
                "nested",
                ".gitignore",
                ".secret.md",
                "a.md",
                "a.txt"
            ]
        );
        assert!(
            visible
                .quick_open_paths
                .contains(&canonical_root.join(".secret.md"))
        );
        assert!(
            visible
                .quick_open_paths
                .contains(&canonical_root.join(".private").join("note.md"))
        );

        let _ = fs::remove_dir_all(root);
    }

    /// 锁定后台扫描同时产出平面索引并响应取消的边界，避免 Quick Open 回退到树递归。
    #[test]
    fn workspace_scan_builds_a_flat_quick_open_index_and_honors_cancellation() {
        let root =
            std::env::temp_dir().join(format!("gmark-workspace-scan-index-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(root.join("nested")).expect("create dirs");
        fs::write(root.join("note.md"), "note").expect("write markdown");
        fs::write(root.join("nested").join("deep.md"), "deep").expect("write nested markdown");
        fs::write(root.join("nested").join("data.txt"), "text").expect("write text");
        let canonical_root = dunce::canonicalize(&root).expect("canonical root");

        let cancelled = AtomicBool::new(false);
        let result = scan_workspace(&root, &[], false, &cancelled).expect("scan workspace");
        assert_eq!(
            result.quick_open_paths,
            vec![
                canonical_root.join("nested").join("deep.md"),
                canonical_root.join("note.md")
            ]
        );

        cancelled.store(true, std::sync::atomic::Ordering::Release);
        assert!(scan_workspace(&root, &[], false, &cancelled).is_err());
        let _ = fs::remove_dir_all(root);
    }

    /// 锁定恢复的 pinned 路径也受 20,000 次规范化上限约束，避免旧会话拖垮后台扫描。
    #[test]
    fn workspace_scan_bounds_pinned_canonicalize_work() {
        let root =
            std::env::temp_dir().join(format!("gmark-workspace-pinned-limit-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).expect("create root");
        let pinned = vec![root.clone(); WORKSPACE_SCAN_MAX_ENTRIES + 1];
        let cancelled = AtomicBool::new(false);

        let error = match scan_workspace(&root, &pinned, false, &cancelled) {
            Ok(_) => panic!("pinned limit should reject excess work"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("pinned paths"));
        let _ = fs::remove_dir_all(root);
    }

    /// 锁定渲染仅观察状态，避免首帧因隐式同步扫描而吞掉目录选择后的刷新。
    #[gpui::test]
    async fn rendering_an_open_workspace_does_not_start_a_scan(cx: &mut gpui::TestAppContext) {
        init_workspace_test_app(cx);
        let root =
            std::env::temp_dir().join(format!("gmark-workspace-render-scan-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).expect("create root");
        let (editor, visual) = cx.add_window_view(|_window, cx| {
            super::Editor::from_markdown(cx, "document".to_owned(), None)
        });
        editor.update(visual, |editor, cx| {
            editor.workspace.is_open = true;
            editor.workspace.explicit_root = Some(root.clone());
            cx.notify();
        });
        visual.update(|window, cx| window.draw(cx).clear());
        editor.update(visual, |editor, _cx| {
            assert!(editor.workspace.file_scan_task.is_none());
            assert_eq!(editor.workspace.file_scan_state, WorkspaceScanState::Idle);
        });
        let _ = fs::remove_dir_all(root);
    }

    #[gpui::test]
    async fn changing_hidden_file_preference_rescans_an_open_workspace(
        cx: &mut gpui::TestAppContext,
    ) {
        init_workspace_test_app(cx);
        cx.update(|cx| {
            crate::config::EditorSettings::init(
                cx,
                true,
                crate::config::AutoSavePreference::Off,
                true,
            );
            crate::config::EditorSettings::set_show_hidden_files_for_test(cx, false);
        });
        let root = std::env::temp_dir().join(format!(
            "gmark-workspace-hidden-refresh-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&root).expect("create root");
        fs::write(root.join("visible.md"), "visible").expect("write visible file");
        fs::write(root.join(".hidden.md"), "hidden").expect("write hidden file");
        let (editor, visual) = cx.add_window_view(|_window, cx| {
            super::Editor::from_markdown(cx, "document".to_owned(), None)
        });
        editor.update(visual, |editor, cx| {
            editor.set_explicit_workspace_root(root.clone(), cx);
        });
        visual.run_until_parked();
        editor.update(visual, |editor, _cx| {
            let tree = editor.workspace.file_tree.as_ref().expect("workspace tree");
            assert!(tree.children.iter().all(|node| node.label != ".hidden.md"));
        });

        visual.update(|window, cx| {
            crate::config::EditorSettings::set_show_hidden_files_for_test(cx, true);
            cx.refresh_windows();
            window.draw(cx).clear();
        });
        visual.run_until_parked();
        editor.update(visual, |editor, _cx| {
            let tree = editor.workspace.file_tree.as_ref().expect("rescanned workspace tree");
            assert!(tree.children.iter().any(|node| node.label == ".hidden.md"));
        });

        let _ = fs::remove_dir_all(root);
    }

    /// 锁定无效目录也能由后台任务直接进入 Failed，而不依赖额外输入触发重绘。
    #[gpui::test]
    async fn explicit_workspace_scan_reaches_failed_without_a_follow_up_input(
        cx: &mut gpui::TestAppContext,
    ) {
        init_workspace_test_app(cx);
        let root = std::env::temp_dir().join(format!(
            "gmark-workspace-missing-{}",
            uuid::Uuid::new_v4()
        ));
        let (editor, visual) = cx.add_window_view(|_window, cx| {
            super::Editor::from_markdown(cx, "document".to_owned(), None)
        });
        editor.update(visual, |editor, cx| {
            editor.set_explicit_workspace_root(root.clone(), cx);
            assert!(editor.workspace.file_scan_task.is_some());
        });
        visual.update(|window, cx| window.draw(cx).clear());
        visual.run_until_parked();
        visual.update(|window, cx| window.draw(cx).clear());
        assert!(visual.debug_bounds("workspace-files-error").is_some());
        editor.update(visual, |editor, _cx| {
            assert!(matches!(
                editor.workspace.file_scan_state,
                WorkspaceScanState::Failed { .. }
            ));
            assert!(editor.workspace.file_error.is_some());
        });
    }

    /// 锁定失败目录可通过同一路径重新提交，避免一次瞬态文件系统错误永久卡在 Failed。
    #[gpui::test]
    async fn failed_workspace_scan_can_retry_same_root_after_it_is_fixed(
        cx: &mut gpui::TestAppContext,
    ) {
        init_workspace_test_app(cx);
        let root = std::env::temp_dir().join(format!(
            "gmark-workspace-retry-{}",
            uuid::Uuid::new_v4()
        ));
        let (editor, visual) = cx.add_window_view(|_window, cx| {
            super::Editor::from_markdown(cx, "document".to_owned(), None)
        });

        editor.update(visual, |editor, cx| {
            editor.set_explicit_workspace_root(root.clone(), cx);
            assert!(matches!(
                editor.workspace.file_scan_state,
                WorkspaceScanState::Scanning { .. }
            ));
        });
        visual.update(|window, cx| window.draw(cx).clear());
        visual.run_until_parked();
        let failed_generation = editor.update(visual, |editor, _cx| {
            match &editor.workspace.file_scan_state {
                WorkspaceScanState::Failed { generation, .. } => *generation,
                state => panic!("expected Failed after missing root, got {state:?}"),
            }
        });

        fs::create_dir_all(&root).expect("create recovered root");
        let recovered = root.join("recovered.md");
        fs::write(&recovered, "recovered").expect("write recovered file");
        let canonical_recovered = dunce::canonicalize(&recovered).expect("canonical recovered");
        editor.update(visual, |editor, cx| {
            editor.set_explicit_workspace_root(root.clone(), cx);
            match &editor.workspace.file_scan_state {
                WorkspaceScanState::Scanning { generation } => {
                    assert!(*generation > failed_generation);
                    assert!(editor.workspace.file_scan_task.is_some());
                }
                state => panic!("expected retry to start Scanning, got {state:?}"),
            }
        });
        visual.update(|window, cx| window.draw(cx).clear());
        visual.run_until_parked();
        editor.update(visual, |editor, _cx| {
            assert!(matches!(
                editor.workspace.file_scan_state,
                WorkspaceScanState::Ready { .. }
            ));
            assert!(editor.workspace.file_scan_task.is_none());
            assert!(editor.workspace.file_tree.is_some());
            assert_eq!(
                editor.workspace.quick_open_paths,
                vec![canonical_recovered.clone()]
            );
        });

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn workspace_tree_create_and_delete_are_incremental() {
        let root = std::env::temp_dir().join(format!(
            "gmark-workspace-incremental-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&root).unwrap();
        let existing = root.join("existing.md");
        fs::write(&existing, "existing").unwrap();
        let mut workspace = WorkspaceState {
            root: Some(root.clone()),
            file_tree: Some(scan_workspace_dir(&root).unwrap()),
            quick_open_paths: vec![existing.clone()],
            file_scan_generation: 17,
            ..WorkspaceState::default()
        };
        let created = root.join("created.md");
        fs::write(&created, "created").unwrap();

        assert!(workspace.insert_created_path(
            &root,
            &created,
            crate::editor::workspace_file_ops::WorkspaceCreateKind::File,
        ));
        assert_eq!(workspace.file_scan_generation, 17);
        assert_eq!(workspace.quick_open_paths, vec![existing.clone(), created.clone()]);
        let labels = workspace
            .file_tree
            .as_ref()
            .unwrap()
            .children
            .iter()
            .map(|node| node.label.as_str())
            .collect::<Vec<_>>();
        assert_eq!(labels, vec!["created.md", "existing.md"]);

        assert!(workspace.remove_path(&created));
        assert_eq!(workspace.file_scan_generation, 17);
        assert_eq!(workspace.quick_open_paths, vec![existing.clone()]);
        let labels = workspace
            .file_tree
            .as_ref()
            .unwrap()
            .children
            .iter()
            .map(|node| node.label.as_str())
            .collect::<Vec<_>>();
        assert_eq!(labels, vec!["existing.md"]);

        let _ = fs::remove_dir_all(root);
    }

    #[gpui::test]
    async fn binary_file_open_failure_is_rendered_in_its_tab(
        cx: &mut gpui::TestAppContext,
    ) {
        init_workspace_test_app(cx);
        let root =
            std::env::temp_dir().join(format!("gmark-binary-tab-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let binary = root.join("sample.exe");
        fs::write(&binary, b"MZ\0\0\0\0\xff\xfe\0\0\0\0").unwrap();
        let (editor, visual) = cx.add_window_view(|_window, cx| {
            super::Editor::from_markdown(cx, "document".to_owned(), None)
        });
        visual.simulate_resize(gpui::size(gpui::px(720.0), gpui::px(520.0)));

        editor.update_in(visual, |editor, window, cx| {
            editor.open_workspace_file(binary.clone(), window, cx);
        });
        visual.run_until_parked();
        visual.update(|window, cx| window.draw(cx).clear());

        editor.update(visual, |editor, _cx| {
            assert_eq!(editor.file_path.as_ref(), Some(&binary));
            assert!(editor.file_open_failure.is_some());
            assert!(editor.saved_file_fingerprint.is_none());
            assert!(editor.workspace.operation_error.is_none());
        });
        assert!(visual.debug_bounds("file-open-failure").is_some());
        assert!(
            visual
                .debug_bounds("file-open-failure-open-system")
                .is_some()
        );
        assert!(visual.debug_bounds("file-open-failure-reveal").is_some());
        let content = visual.debug_bounds("editor-content").unwrap();
        let placeholder = visual.debug_bounds("file-open-failure").unwrap();
        let file_name = visual.debug_bounds("file-open-failure-name").unwrap();
        assert!(placeholder.left() >= content.left());
        assert!(placeholder.right() <= content.right());
        assert!(f32::from(file_name.size.width) > 100.0);
        assert!(visual.debug_bounds("file-open-failure-reason").is_none());
        editor.update_in(visual, |editor, window, cx| {
            editor.file_open_failure_focus_handles[0].focus(window);
            assert!(editor.file_open_failure_focus_handles[0].is_focused(window));
            cx.notify();
        });
        visual.update(|window, cx| window.draw(cx).clear());

        let _ = fs::remove_dir_all(root);
    }

    #[gpui::test]
    async fn switching_between_workspace_files_keeps_the_window_alive(
        cx: &mut gpui::TestAppContext,
    ) {
        init_workspace_test_app(cx);
        let root = std::env::temp_dir().join(format!(
            "gmark-workspace-switch-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&root).unwrap();
        let paths = [
            root.join("first.md"),
            root.join("second.md"),
            root.join("third.md"),
        ];
        fs::write(&paths[0], "# First\n\n- one\n- two\n").unwrap();
        fs::write(&paths[1], "# Second\n\n| A | B |\n| - | - |\n| 1 | 2 |\n").unwrap();
        fs::write(&paths[2], "# Third\n\n> [!NOTE]\n> keep switching\n").unwrap();
        let (editor, visual) = cx.add_window_view(|_window, cx| {
            super::Editor::from_markdown(cx, "initial".to_owned(), None)
        });

        for path in paths.iter().cycle().take(12) {
            editor.update_in(visual, |editor, window, cx| {
                editor.open_workspace_file(path.clone(), window, cx);
            });
            visual.run_until_parked();
            visual.update(|window, cx| window.draw(cx).clear());
            editor.update(visual, |editor, _cx| {
                assert_eq!(editor.file_path.as_ref(), Some(path));
            });
        }

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn explicitly_created_empty_directory_can_be_merged_into_scanned_tree() {
        let root =
            std::env::temp_dir().join(format!("gmark-empty-dir-tree-{}", uuid::Uuid::new_v4()));
        let empty = root.join("empty");
        fs::create_dir_all(&empty).unwrap();
        fs::write(root.join("note.md"), "note").unwrap();
        let mut tree = scan_workspace_dir(&root).unwrap();
        assert!(tree.children.iter().all(|node| node.label != "empty"));

        insert_workspace_directory(&mut tree, &root, &empty);
        assert!(tree.children.iter().any(|node| {
            matches!(&node.kind, WorkspaceTreeKind::Directory(path) if path == &empty)
        }));

        let _ = fs::remove_dir_all(root);
    }
