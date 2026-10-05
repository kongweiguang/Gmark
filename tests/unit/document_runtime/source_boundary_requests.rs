// @author kongweiguang

use super::*;

/// 组合输入在慢导航期间仍属于原 Block；确认文字等导航后发布，取消只结束候选且不产生历史。
#[gpui::test]
async fn paged_source_pending_boundary_waits_for_ime_terminal(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        crate::i18n::I18nManager::init(cx);
        crate::theme::ThemeManager::init(cx);
        crate::components::init(cx);
    });
    let temp = tempfile::tempdir().expect("boundary IME tempdir");
    for terminal in [
        gpui::CompositionEnd::Committed,
        gpui::CompositionEnd::Cancelled,
    ] {
        let path = temp.path().join(format!("{terminal:?}.txt"));
        fs::write(&path, "alpha beta\r\n").expect("boundary IME fixture");
        let probe = gmark_paged_document::probe_file(
            &path,
            gmark_paged_document::ProbeOptions {
                max_resident_bytes: 1,
                ..Default::default()
            },
        )
        .expect("Paged probe");
        let source = FileSource::open(&path).expect("Paged file");
        let (host, visual) =
            cx.add_window_view(move |_window, cx| DocumentHost::new(path, probe, source, cx));
        visual.run_until_parked();
        let (release, wait) = futures::channel::oneshot::channel::<()>();
        visual.update(|window, cx| {
            host.update(cx, |host, cx| {
                host.select_source_range_and_focus_for_test(1..1, false, window);
                host.request_source_boundary(
                    move |_snapshot, _cancel| async move {
                        wait.await.expect("release boundary while composing");
                        Ok(6)
                    },
                    |host, target, window, cx| {
                        host.set_source_selection(
                            SourceSelection::collapsed(target, SourceAffinity::After),
                            cx,
                        );
                        host.restore_source_navigation_input(window, cx);
                    },
                    window,
                    cx,
                );
            })
        });
        visual.update(|window, cx| window.draw(cx).clear());
        let owner = host
            .read_with(visual, |host, _cx| host.source_row_block_for_test(0))
            .expect("original input owner");
        visual.update(|window, cx| owner.update(cx, |block, cx| {
            <crate::components::Block as gpui::EntityInputHandler>::replace_and_mark_text_in_range(block, None, "ni", Some(2..2), window, cx);
        }));
        release.send(()).expect("release resolver");
        visual.run_until_parked();
        assert_eq!(
            host.read_with(visual, |host, _cx| host.source_text_for_test()),
            "alpha beta\r\n"
        );
        assert!(host.read_with(visual, |host, _cx| {
            host.coordinator.source_boundary_completion.is_some()
        }));
        visual.update(|window, cx| {
            owner.update(cx, |block, cx| {
                if terminal == gpui::CompositionEnd::Committed {
                    <crate::components::Block as gpui::EntityInputHandler>::replace_text_in_range(
                        block, None, "你🙂", window, cx,
                    );
                }
                <crate::components::Block as gpui::EntityInputHandler>::composition_ended(
                    block, terminal, window, cx,
                );
            })
        });
        visual.run_until_parked();
        let expected = if terminal == gpui::CompositionEnd::Committed {
            "alpha 你🙂beta\r\n"
        } else {
            "alpha beta\r\n"
        };
        assert_eq!(
            host.read_with(visual, |host, _cx| host.source_text_for_test()),
            expected
        );
        assert!(host.read_with(visual, |host, _cx| {
            host.coordinator.source_boundary_cancellation.is_none()
        }));
        if terminal == gpui::CompositionEnd::Committed {
            visual.simulate_keystrokes("ctrl-z");
            assert_eq!(
                host.read_with(visual, |host, _cx| host.source_text_for_test()),
                "alpha beta\r\n"
            );
            assert_eq!(
                host.read_with(visual, |host, _cx| host.source_selection_for_test()),
                Some(SourceSelection::collapsed(6, SourceAffinity::After))
            );
        }
    }
}

/// 慢导航后的原生输入、保存与拆卸共用生命周期；快照不能早于最后按键，租约不能携带未提交文字。
#[gpui::test]
async fn paged_source_pending_boundary_orders_native_typing(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        crate::i18n::I18nManager::init(cx);
        crate::theme::ThemeManager::init(cx);
        crate::components::init(cx);
    });
    let temp = tempfile::tempdir().expect("ordered boundary tempdir");
    let path = temp.path().join("ordered.txt");
    let (release, wait) = futures::channel::oneshot::channel::<()>();
    fs::write(&path, "alpha beta\n").expect("ordered fixture");
    let probe = gmark_paged_document::probe_file(
        &path,
        gmark_paged_document::ProbeOptions {
            max_resident_bytes: 1,
            ..Default::default()
        },
    )
    .expect("Paged probe");
    let source = FileSource::open(&path).expect("Paged file");
    let task_path = path.clone();
    let (host, visual) =
        cx.add_window_view(move |_window, cx| DocumentHost::new(task_path, probe, source, cx));
    visual.run_until_parked();
    visual.update(|window, cx| window.draw(cx).clear());
    visual.update(|window, cx| {
        host.update(cx, |host, cx| {
            host.select_source_range_and_focus_for_test(1..1, false, window);
            host.request_source_boundary(
                move |_snapshot, _cancel| async move {
                    wait.await.expect("release boundary after native input");
                    Ok(6)
                },
                |host, target, window, cx| {
                    host.set_source_selection(
                        SourceSelection::collapsed(target, SourceAffinity::After),
                        cx,
                    );
                    host.restore_source_navigation_input(window, cx);
                },
                window,
                cx,
            );
        })
    });
    // 安装原生输入桥但不等待解析；dispatch 不会像 simulate_input 一样等待后台执行器。
    visual.update(|window, cx| window.draw(cx).clear());
    let handle = visual.update(|window, _cx| window.window_handle());
    visual
        .cx
        .dispatch_keystroke(handle, gpui::Keystroke::parse("7").expect("native digit"));
    assert!(host.read_with(visual, |host, _cx| {
        !host.coordinator.source_boundary_actions.is_empty()
    }));
    visual.update(|window, cx| {
        host.update(cx, |host, cx| {
            assert!(host.detach_view(cx).is_none());
            host.on_save_document(&SaveDocument, window, cx);
            assert!(host.has_pending_source_input());
        });
    });
    assert_eq!(
        fs::read_to_string(&path).expect("unsaved pending input"),
        "alpha beta\n"
    );
    release.send(()).expect("boundary still waiting");
    visual.run_until_parked();
    assert_eq!(
        host.read_with(visual, |host, _cx| host.source_text_for_test()),
        "alpha 7beta\n"
    );
    for _ in 0..5000 {
        visual.run_until_parked();
        if fs::read_to_string(&path).expect("ordered saved input") == "alpha 7beta\n" {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert_eq!(
        fs::read_to_string(&path).expect("latest saved input"),
        "alpha 7beta\n"
    );
    visual.simulate_input("8");
    assert_eq!(
        host.read_with(visual, |host, _cx| host.source_text_for_test()),
        "alpha 78beta\n"
    );
    visual.simulate_keystrokes("ctrl-z");
    assert_eq!(
        host.read_with(visual, |host, _cx| host.source_text_for_test()),
        "alpha 7beta\n",
        "保存结束原输入组，后续文字单独撤销"
    );
    visual.simulate_keystrokes("ctrl-z");
    assert_eq!(
        host.read_with(visual, |host, _cx| host.source_text_for_test()),
        "alpha beta\n"
    );
}

/// 读取与剪贴板失败仍保留确认文字；Ctrl+C 成功前不得把待恢复文字随租约拆走。
#[gpui::test]
async fn paged_source_failed_boundary_preserves_confirmed_typing(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        crate::i18n::I18nManager::init(cx);
        crate::theme::ThemeManager::init(cx);
        crate::components::init(cx);
    });
    let temp = tempfile::tempdir().expect("failed boundary tempdir");
    let path = temp.path().join("failed.txt");
    let (release, wait) = futures::channel::oneshot::channel::<()>();
    fs::write(&path, "alpha beta\n").expect("failed fixture");
    let probe = gmark_paged_document::probe_file(
        &path,
        gmark_paged_document::ProbeOptions {
            max_resident_bytes: 1,
            ..Default::default()
        },
    )
    .expect("Paged probe");
    let source = FileSource::open(&path).expect("Paged file");
    let (host, visual) =
        cx.add_window_view(move |_window, cx| DocumentHost::new(path, probe, source, cx));
    visual.run_until_parked();
    visual.update(|window, cx| window.draw(cx).clear());
    visual.update(|window, cx| {
        host.update(cx, |host, cx| {
            host.select_source_range_and_focus_for_test(1..1, false, window);
            host.fail_next_native_clipboard_write_for_test = true;
            host.request_source_boundary::<u64, _, _, _>(
                move |_snapshot, _cancel| async move {
                    wait.await
                        .expect("release failed boundary after native input");
                    Err(PagedDocumentError::Search("injected read failure".into()))
                },
                |_host, _target, _window, _cx| panic!("failed resolution must not install"),
                window,
                cx,
            );
        })
    });
    visual.update(|window, cx| window.draw(cx).clear());
    let handle = visual.update(|window, _cx| window.window_handle());
    visual
        .cx
        .dispatch_keystroke(handle, gpui::Keystroke::parse("7").expect("native digit"));
    assert!(host.read_with(visual, |host, _cx| {
        !host.coordinator.source_boundary_actions.is_empty()
    }));
    release.send(()).expect("failed boundary still waiting");
    visual.run_until_parked();
    assert_eq!(
        host.read_with(visual, |host, _cx| host.source_text_for_test()),
        "alpha beta\n"
    );
    assert_eq!(
        host.read_with(visual, |host, _cx| host
            .coordinator
            .source_boundary_recovery_text
            .clone()),
        Some("7".into())
    );
    visual.update(|_window, cx| {
        host.update(cx, |host, cx| {
            assert!(host.has_pending_source_input());
            assert!(host.detach_view(cx).is_none());
        })
    });
    visual.simulate_keystrokes("ctrl-c");
    assert_eq!(
        visual.read(|cx| cx.read_from_clipboard().and_then(|item| item.text())),
        Some("7".into())
    );
    assert!(host.read_with(visual, |host, _cx| {
        host.coordinator.source_boundary_recovery_text.is_none()
    }));
    visual.simulate_input("8");
    assert_eq!(
        host.read_with(visual, |host, _cx| host.source_text_for_test()),
        "a8lpha beta\n"
    );
}

/// 让后台完成回调在用户的新选择、编辑或焦点变化之后返回，不依赖文件扫描耗时制造竞态。
#[gpui::test]
async fn paged_source_late_boundary_cannot_override_user_state(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        crate::i18n::I18nManager::init(cx);
        crate::theme::ThemeManager::init(cx);
        crate::components::init(cx);
    });
    let temp = tempfile::tempdir().expect("boundary request tempdir");
    for scenario in ["selection", "revision", "focus", "mode", "closed"] {
        let path = temp.path().join(format!("{scenario}.txt"));
        fs::write(&path, "alpha beta\n").expect("boundary request fixture");
        let probe = gmark_paged_document::probe_file(
            &path,
            gmark_paged_document::ProbeOptions {
                max_resident_bytes: 1,
                ..Default::default()
            },
        )
        .expect("Paged probe");
        let source = FileSource::open(&path).expect("Paged file");
        let (host, visual) =
            cx.add_window_view(move |_window, cx| DocumentHost::new(path, probe, source, cx));
        visual.run_until_parked();
        let expected = visual.update(|window, cx| {
            host.update(cx, |host, cx| {
                host.select_source_range_and_focus_for_test(1..1, false, window);
                host.request_source_boundary(
                    |_snapshot, _cancel| async { Ok(0..5) },
                    |host, range, _window, cx| {
                        host.set_source_selection(SourceSelection::from_range(range, false), cx)
                    },
                    window,
                    cx,
                );
                match scenario {
                    "selection" => host.set_source_selection(
                        SourceSelection::collapsed(8, SourceAffinity::After),
                        cx,
                    ),
                    "revision" => {
                        let document = host.document.as_ref().expect("shared document");
                        document.replace_range(0..0, "7").expect("new user edit");
                        document
                            .set_source_selection(SourceSelection::collapsed(
                                1,
                                SourceAffinity::Before,
                            ))
                            .expect("same byte caret in new revision");
                    }
                    "focus" => host.search_input.read(cx).focus_handle.focus(window),
                    "mode" => host.view_mode = DocumentHostViewMode::Structure,
                    "closed" => host.coordinator.cancel_all(),
                    _ => unreachable!("known scenario"),
                }
                host.source_selection_for_test().expect("user selection")
            })
        });
        visual.run_until_parked();
        assert_eq!(
            host.read_with(visual, |host, _cx| host.source_selection_for_test()),
            Some(expected),
            "late boundary must respect {scenario}"
        );
        assert!(
            host.read_with(visual, |host, _cx| {
                host.coordinator.source_boundary_cancellation.is_none()
                    && host.coordinator.source_boundary_completion.is_none()
                    && host.coordinator.source_boundary_actions.is_empty()
            }),
            "stale {scenario} must release the input queue"
        );
    }
}
