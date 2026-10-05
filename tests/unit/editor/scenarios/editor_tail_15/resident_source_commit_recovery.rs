// @author kongweiguang

use super::*;

/// 虚拟输入失去区域归属时不能用挂载树覆盖全文；恢复备份同时保留视口外原文和未提交文字。
#[gpui::test]
async fn unmapped_virtual_input_is_backed_up_before_recovery(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let original = (0..10_000)
        .map(|index| format!("paragraph {index} 中文"))
        .collect::<Vec<_>>()
        .join("\n\n");
    let source = original.clone();
    let (editor, visual) =
        cx.add_window_view(move |_window, cx| Editor::from_markdown(cx, source, None));
    redraw(visual);
    let local = editor.update_in(visual, |editor, window, cx| {
        assert!(editor.virtual_surface.is_some());
        let local = Editor::new_block(cx, crate::editor::BlockRecord::paragraph("unsubmitted".to_owned()));
        editor.document.replace_roots(vec![local.clone()], cx);
        editor.focus_block(local.entity_id());
        local.update(cx, |block, cx| {
            block.selected_range = 0..0;
            block.focus_handle.focus(window);
            cx.notify();
        });
        local
    });
    redraw(visual);
    visual.simulate_input("X");
    redraw(visual);
    editor.read_with(visual, |editor, cx| {
        assert_eq!(local.read(cx).display_text(), "Xunsubmitted");
        assert_eq!(editor.source_document.text(), original);
        assert!(editor.document.source_commit_error().is_some());
        assert!(editor.virtual_undo_selections.is_empty());
    });
    editor.update(visual, |editor, cx| {
        assert!(!editor.sync_virtual_surface_mounts(80_000.0, 720.0, 800.0, cx));
        assert_eq!(editor.document.first_root().unwrap().entity_id(), local.entity_id());
    });
    visual.simulate_keystrokes("ctrl-c");
    redraw(visual);
    let backup = visual.read_from_clipboard().and_then(|item| item.text()).unwrap();
    assert!(backup.starts_with(&original));
    assert!(backup.contains("Xunsubmitted"));
    editor.read_with(visual, |editor, _cx| {
        assert!(editor.document.source_commit_error().is_none());
        assert_eq!(editor.source_document.text(), original);
    });
}

/// 让源码区域 revision 失效，再切换到第二根输入，以验证恢复不会只复制当前焦点区域。
#[gpui::test]
async fn stale_resident_source_commit_is_backed_up_before_recovery(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("resident-source-commit.md");
    let original = "alpha\r\n\r\nbeta";
    std::fs::write(&path, original.as_bytes()).unwrap();

    let editor_path = path.clone();
    let source = original.to_owned();
    let (editor, visual) =
        cx.add_window_view(move |_window, cx| Editor::from_markdown(cx, source, Some(editor_path)));
    redraw(visual);

    let authoritative_revision = editor.update(visual, |editor, cx| {
        let revision = editor.source_document.revision();
        let roots = editor.document.root_blocks();
        assert_eq!(roots.len(), 2);
        let mut regions = Vec::with_capacity(roots.len());
        for (index, block) in roots.iter().enumerate() {
            let (bound_revision, range, region_roots) = editor
                .document
                .source_region_for_entity(block.entity_id())
                .expect("each paragraph has an independent source region");
            assert_eq!(bound_revision, revision);
            assert_eq!(region_roots.len(), 1);
            regions.push((range, index..index + 1));
        }
        assert_ne!(regions[0].0, regions[1].0);
        let stale_revision = gmark_document::Revision::from_u64(revision.get().saturating_add(1));
        editor
            .document
            .bind_source_regions(stale_revision, regions, cx);
        revision
    });
    assert_eq!(
        editor.read_with(visual, |editor, _cx| editor.source_document.text()),
        "alpha\n\nbeta"
    );

    editor.update_in(visual, |editor, window, cx| {
        let block = editor.document.root_blocks()[0].clone();
        let end = block.read(cx).visible_len();
        editor.focus_block(block.entity_id());
        block.update(cx, |block, cx| {
            block.selected_range = 0..end;
            block.selection_reversed = false;
            block.focus_handle.focus(window);
            cx.notify();
        });
    });
    redraw(visual);
    visual.simulate_input("local first");
    redraw(visual);

    editor.update_in(visual, |editor, window, cx| {
        let block = editor.document.root_blocks()[1].clone();
        let end = block.read(cx).visible_len();
        editor.focus_block(block.entity_id());
        block.update(cx, |block, cx| {
            block.selected_range = 0..end;
            block.selection_reversed = false;
            block.focus_handle.focus(window);
            cx.notify();
        });
    });
    redraw(visual);
    visual.simulate_input("local second");
    redraw(visual);

    editor.read_with(visual, |editor, _cx| {
        assert_eq!(
            editor.document.source_commit_error(),
            Some("当前输入区域的源码版本已变化")
        );
        assert!(editor.document_dirty);
        assert_eq!(editor.source_document.revision(), authoritative_revision);
        assert_eq!(editor.source_document.text(), "alpha\n\nbeta");
        assert!(editor.undo_history.is_empty());
        assert_eq!(
            editor.active_entity_id,
            Some(editor.document.root_blocks()[1].entity_id())
        );
    });

    let saved = visual.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.save_to_existing_path(&path, window, cx)
        })
    });
    assert!(!saved);
    assert_eq!(std::fs::read(&path).unwrap(), original.as_bytes());

    visual.simulate_keystrokes("ctrl-c");
    redraw(visual);
    assert_eq!(
        visual.read(|cx| cx.read_from_clipboard().and_then(|item| item.text())),
        Some("local first\n\nlocal second".to_owned())
    );
    editor.read_with(visual, |editor, cx| {
        assert!(editor.document.source_commit_error().is_none());
        assert!(!editor.document_dirty);
        assert_eq!(editor.source_document.text(), "alpha\n\nbeta");
        assert_eq!(editor.source_document.revision(), authoritative_revision);
        let visible_roots = editor
            .document
            .root_blocks()
            .iter()
            .map(|block| block.read(cx).display_text())
            .collect::<Vec<_>>();
        assert_eq!(visible_roots, vec!["alpha".to_owned(), "beta".to_owned()]);
    });

    editor.update_in(visual, |editor, window, cx| {
        let block = editor.document.root_blocks()[0].clone();
        let end = block.read(cx).visible_len();
        editor.focus_block(block.entity_id());
        block.update(cx, |block, cx| {
            block.selected_range = 0..end;
            block.selection_reversed = false;
            block.focus_handle.focus(window);
            cx.notify();
        });
    });
    redraw(visual);
    visual.simulate_input("recovered");
    redraw(visual);

    editor.read_with(visual, |editor, _cx| {
        assert!(editor.document.source_commit_error().is_none());
        assert_eq!(editor.source_document.text(), "recovered\n\nbeta");
    });
    let saved = visual.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.save_to_existing_path(&path, window, cx)
        })
    });
    assert!(saved);
    assert_eq!(std::fs::read(&path).unwrap(), b"recovered\r\n\r\nbeta");
}

/// 跨块输入只发送 payload；拒绝事务后保留连续确认结果，并阻止切模式绕过恢复边界。
#[gpui::test]
async fn rejected_resident_cross_block_input_preserves_confirmed_payload(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("resident-cross-block-recovery.md");
    let original = "alpha\r\n\r\nbeta";
    std::fs::write(&path, original).unwrap();
    let editor_path = path.clone();
    let (editor, visual) = cx.add_window_view(move |_window, cx| {
        Editor::from_markdown(cx, original.to_owned(), Some(editor_path))
    });
    redraw(visual);
    editor.update_in(visual, |editor, window, cx| {
        let first = editor.document.first_root().unwrap().clone();
        editor.focus_block(first.entity_id());
        first.update(cx, |block, _cx| block.focus_handle.focus(window));
    });
    redraw(visual);
    visual.simulate_keystrokes("ctrl-a");
    redraw(visual);
    editor.update(visual, |editor, cx| {
        assert!(editor.cross_block_selection.is_some());
        let revision = editor.source_document.revision();
        let regions = editor
            .document
            .root_blocks()
            .iter()
            .enumerate()
            .map(|(index, block)| {
                let (_, range, _) = editor
                    .document
                    .source_region_for_entity(block.entity_id())
                    .unwrap();
                (range, index..index + 1)
            })
            .collect();
        editor.document.bind_source_regions(
            gmark_document::Revision::from_u64(revision.get() + 1),
            regions,
            cx,
        );
    });
    visual.simulate_input("你");
    redraw(visual);
    visual.simulate_input("好");
    redraw(visual);
    editor.read_with(visual, |editor, _cx| {
        assert_eq!(editor.document.source_commit_replacement(), Some("你好"));
        assert_eq!(editor.source_document.text(), "alpha\n\nbeta");
        assert!(editor.document_dirty);
        assert!(editor.undo_history.is_empty());
    });
    visual.simulate_keystrokes("ctrl-/");
    redraw(visual);
    editor.read_with(visual, |editor, _cx| {
        assert_eq!(editor.view_mode, ViewMode::Rendered)
    });
    assert!(!visual.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.save_to_existing_path(&path, window, cx)
        })
    }));
    assert_eq!(std::fs::read(&path).unwrap(), original.as_bytes());
    visual.simulate_keystrokes("ctrl-c");
    redraw(visual);
    assert_eq!(
        visual.read(|cx| cx.read_from_clipboard().and_then(|item| item.text())),
        Some("alpha\n\nbeta\n\n你好".to_owned())
    );
    editor.read_with(visual, |editor, _cx| {
        assert!(editor.document.source_commit_error().is_none());
        assert!(editor.document.source_commit_replacement().is_none());
        assert_eq!(editor.source_document.text(), "alpha\n\nbeta");
    });
}
