// @author kongweiguang

/// 空行和根级代码块不能吞掉后续列表的源码起始编号；视图切换也必须保留它。
#[gpui::test]
async fn ordered_list_source_starts_survive_block_breaks(cx: &mut TestAppContext) {
    let source = "1. 上传\n登录服务器\n\n```bash\ncd /app\n```\n\n2. 修改配置\n\n3. 启动\n\n```bash\nstart\n```\n\n4. 查看状态";
    let editor = cx.new(|cx| Editor::from_markdown(cx, source.to_string(), None));
    editor.update(cx, |editor, cx| {
        let canonical = source.replace("\n登录服务器", "\n   登录服务器");
        for mode in [
            crate::editor::ViewMode::Rendered,
            crate::editor::ViewMode::Preview,
            crate::editor::ViewMode::Split,
            crate::editor::ViewMode::Rendered,
        ] {
            editor.set_view_mode(mode, cx);
            let document = if mode == crate::editor::ViewMode::Split {
                &editor.split_preview.as_ref().expect("分屏应建立预览").document
            } else {
                &editor.document
            };
            let ordinals = document
                .visible_blocks()
                .iter()
                .filter_map(|visible| visible.entity.read(cx).list_ordinal)
                .collect::<Vec<_>>();
            assert_eq!(ordinals, vec![1, 2, 3, 4]);
            assert_eq!(document.markdown_text(cx), canonical);
            editor.toggle_view_mode(cx);
            editor.toggle_view_mode(cx);
        }
    });
}

/// 连续项仍按列表起点递增，不能把常见的全写 1 或任意后续标记逐字显示。
#[gpui::test]
async fn ordered_list_source_start_only_seeds_each_run(cx: &mut TestAppContext) {
    for (source, expected) in [
        ("7. 七\n1. 八\n99. 九", vec![7, 8, 9]),
        ("0) 零\n0) 一", vec![0, 1]),
        ("3. 外层\n   8. 内层\n   1. 内层续项\n1. 外层续项", vec![3, 8, 9, 4]),
    ] {
        let editor = cx.new(|cx| Editor::from_markdown(cx, source.to_string(), None));
        editor.update(cx, |editor, cx| {
            let ordinals = editor
                .document
                .visible_blocks()
                .iter()
                .filter_map(|visible| visible.entity.read(cx).list_ordinal)
                .collect::<Vec<_>>();
            assert_eq!(ordinals, expected, "{source}");
        });
    }
}

/// 数字变化而正文与 ID 不变时，预览必须拒绝旧实体，避免保留过期序号。
#[gpui::test]
async fn ordered_list_source_start_change_invalidates_reused_projection(cx: &mut TestAppContext) {
    let editor = cx.new(|cx| Editor::from_markdown(cx, "2. same".to_string(), None));
    editor.update(cx, |editor, cx| {
        let old = editor.document.visible_blocks()[0].entity.clone();
        let mut nodes =
            super::projection_builder::prepare_simple_list_nodes(&["7. same".to_string()])
                .expect("简单有序列表应产生预解析节点");
        let id = old.read(cx).record.id;
        nodes[0].record.id = id;
        let mut reusable = std::collections::HashMap::from([(id, old.clone())]);
        let rebuilt = Editor::materialize_prepared_node(&nodes[0], &mut reusable, cx);
        editor.document.replace_roots(vec![rebuilt.clone()], cx);
        assert_ne!(rebuilt.entity_id(), old.entity_id());
        assert_eq!(rebuilt.read(cx).list_ordinal, Some(7));
        assert_eq!(editor.document.markdown_text(cx), "7. same");
    });
}
