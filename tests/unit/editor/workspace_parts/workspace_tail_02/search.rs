// @author kongweiguang

    #[test]
    fn quick_open_ranking_prefers_file_name_and_is_stable_when_empty() {
        let root = PathBuf::from("C:/notes");
        let paths = vec![
            root.join("z/readme.md"),
            root.join("readme/archive.md"),
            root.join("a/alpha.md"),
        ];
        let ranked = rank_quick_open_paths(&root, paths.clone(), "rdm");
        assert_eq!(ranked[0].relative_path, "z/readme.md");

        let all = rank_quick_open_paths(&root, paths, "");
        assert_eq!(
            all.iter()
                .map(|result| result.relative_path.as_str())
                .collect::<Vec<_>>(),
            vec!["a/alpha.md", "readme/archive.md", "z/readme.md"]
        );
    }

    #[test]
    fn workspace_search_supports_case_word_regex_utf8_and_gitignore() {
        let root = std::env::temp_dir().join(format!("gmark-search-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(root.join("nested")).unwrap();
        fs::write(root.join(".gitignore"), "ignored.md\n").unwrap();
        fs::write(
            root.join("notes.md"),
            "Alpha alphabet\n中文 alpha\nissue-42 and issue-7",
        )
        .unwrap();
        fs::write(root.join("nested").join("more.markdown"), "alpha").unwrap();
        fs::write(root.join("ignored.md"), "alpha").unwrap();
        fs::write(root.join("plain.txt"), "alpha").unwrap();
        fs::write(root.join(".hidden.md"), "alpha").unwrap();
        fs::write(root.join("legacy.md"), [b'c', b'a', b'f', 0xe9]).unwrap();

        let insensitive = search_workspace(&root, "alpha", WorkspaceSearchOptions::default(), false)
            .expect("plain search");
        assert_eq!(insensitive.len(), 4);
        assert!(
            insensitive
                .iter()
                .all(|result| result.path != root.join(".hidden.md"))
        );
        assert!(
            insensitive
                .iter()
                .all(|result| result.path != root.join("ignored.md"))
        );
        assert!(insensitive.iter().any(|result| {
            result.relative_path == "notes.md" && result.line == 2 && result.column == 4
        }));
        let with_hidden =
            search_workspace(&root, "alpha", WorkspaceSearchOptions::default(), true)
                .expect("search hidden files");
        assert!(
            with_hidden
                .iter()
                .any(|result| result.path == root.join(".hidden.md"))
        );

        let case_sensitive = search_workspace(
            &root,
            "Alpha",
            WorkspaceSearchOptions {
                case_sensitive: true,
                ..WorkspaceSearchOptions::default()
            },
            false,
        )
        .unwrap();
        assert_eq!(case_sensitive.len(), 1);

        let whole_word = search_workspace(
            &root,
            "alpha",
            WorkspaceSearchOptions {
                whole_word: true,
                ..WorkspaceSearchOptions::default()
            },
            false,
        )
        .unwrap();
        assert_eq!(whole_word.len(), 3);

        let regex = search_workspace(
            &root,
            r"issue-\d+",
            WorkspaceSearchOptions {
                regex: true,
                ..WorkspaceSearchOptions::default()
            },
            false,
        )
        .unwrap();
        assert_eq!(regex.len(), 2);
        assert!(
            search_workspace(
                &root,
                "(",
                WorkspaceSearchOptions {
                    regex: true,
                    ..WorkspaceSearchOptions::default()
                },
                false,
            )
            .is_err()
        );
        let legacy =
            search_workspace(&root, "café", WorkspaceSearchOptions::default(), false).unwrap();
        assert_eq!(legacy.len(), 1);
        assert_eq!(legacy[0].relative_path, "legacy.md");

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn workspace_search_enforces_result_and_file_size_budgets() {
        let root =
            std::env::temp_dir().join(format!("gmark-search-budget-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("many.md"), "hit\n".repeat(700)).unwrap();
        let oversized = fs::File::create(root.join("oversized.md")).unwrap();
        oversized.set_len(super::SEARCH_MAX_FILE_BYTES + 1).unwrap();

        let results =
            search_workspace(&root, "hit", WorkspaceSearchOptions::default(), false).unwrap();
        assert_eq!(results.len(), super::SEARCH_MAX_RESULTS);
        assert!(
            results
                .iter()
                .all(|result| result.path.ends_with("many.md"))
        );

        let _ = fs::remove_dir_all(root);
    }

    #[gpui::test]
    async fn workspace_search_tab_renders_input_controls_and_results(
        cx: &mut gpui::TestAppContext,
    ) {
        init_workspace_test_app(cx);
        let root = std::env::temp_dir().join(format!("gmark-search-ui-{}", uuid::Uuid::new_v4()));
        let nested = root.join("a-very-long-workspace-directory-name");
        fs::create_dir_all(&nested).unwrap();
        let path = nested.join("a-very-long-document-name-that-keeps-extension.markdown");
        fs::write(&path, "needle here").unwrap();
        let editor_path = path.clone();
        let workspace_root = root.clone();
        let (editor, visual) = cx.add_window_view(move |_window, cx| {
            super::Editor::from_markdown(cx, "needle here".to_owned(), Some(editor_path))
        });

        editor.update(visual, |editor, cx| {
            editor.workspace.is_open = true;
            editor.workspace.root = Some(workspace_root.clone());
            editor.workspace.explicit_root = Some(workspace_root.clone());
            editor.set_workspace_tab(super::WorkspaceTab::Search, cx);
            let input = editor.ensure_workspace_search_input(cx);
            let expected = cx
                .global::<crate::i18n::I18nManager>()
                .strings()
                .workspace_search_prompt
                .clone();
            let placeholder = input.read(cx).input_placeholder();
            assert_eq!(
                placeholder.as_ref().map(|placeholder| placeholder.as_ref()),
                Some(expected.as_str())
            );
        });
        visual.update(|window, cx| window.draw(cx).clear());
        let search_field = visual.debug_bounds("workspace-search-input").unwrap();
        let search_icon = visual
            .debug_bounds("workspace-search-input-icon")
            .unwrap();
        assert!(search_icon.left() >= search_field.left());
        assert!(search_icon.right() <= search_field.right());
        assert!(visual.debug_bounds("workspace-search-status").is_none());

        editor.update(visual, |editor, cx| {
            let input = editor.ensure_workspace_search_input(cx);
            input.update(cx, |input, cx| {
                input.replace_text_in_visible_range(0..0, "needle", None, false, cx);
            });
            editor.schedule_workspace_search(cx);
        });
        visual.executor().advance_clock(super::SEARCH_DEBOUNCE);
        visual.run_until_parked();
        visual.update(|window, cx| window.draw(cx).clear());
        visual.run_until_parked();

        assert!(visual.debug_bounds("workspace-search-input").is_some());
        assert!(visual.debug_bounds("workspace-search-case").is_some());
        assert!(visual.debug_bounds("workspace-search-word").is_some());
        assert!(visual.debug_bounds("workspace-search-regex").is_some());
        for (control, icon) in [
            ("workspace-search-case", "workspace-search-case-icon"),
            ("workspace-search-word", "workspace-search-word-icon"),
            ("workspace-search-regex", "workspace-search-regex-icon"),
        ] {
            let control = visual.debug_bounds(control).unwrap();
            let icon = visual.debug_bounds(icon).unwrap();
            assert_eq!(icon.size, size(px(15.0), px(15.0)));
            assert!(icon.left() >= control.left());
            assert!(icon.right() <= control.right());
        }
        let result = visual.debug_bounds("workspace-search-result-0").unwrap();
        let path = visual
            .debug_bounds("workspace-search-result-path-0")
            .unwrap();
        let location = visual
            .debug_bounds("workspace-search-result-location-0")
            .unwrap();
        assert!(path.left() >= result.left());
        assert!(path.right() <= location.left());
        assert!(location.right() <= result.right());
        assert!(path.bottom() <= result.bottom());
        assert!(location.bottom() <= result.bottom());

        let _ = fs::remove_dir_all(root);
    }
