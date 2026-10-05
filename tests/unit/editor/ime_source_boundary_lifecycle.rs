// @author kongweiguang

use std::fs;

use futures::channel::oneshot;
use gpui::{AppContext, Entity, TestAppContext, VisualTestContext};

use super::super::Editor;
use crate::document_host::DocumentHost;
use crate::editor::{DocumentKind, ViewMode};
use crate::i18n::I18nManager;

/// 给生命周期测试提供真实 Paged Source Host，保持文件后端与输入边界一致。
fn open_paged_editor<'a>(
    cx: &'a mut TestAppContext,
    name: &str,
) -> (
    Entity<Editor>,
    Entity<DocumentHost>,
    &'a mut VisualTestContext,
    tempfile::TempDir,
) {
    cx.update(|cx| {
        I18nManager::init(cx);
        crate::theme::ThemeManager::init(cx);
        crate::components::init(cx);
    });
    let temp = tempfile::tempdir().expect("Paged Source lifecycle tempdir");
    let path = temp.path().join(name);
    let original = (0..40)
        .map(|line| format!("source row {line:03}\n"))
        .collect::<String>();
    fs::write(&path, original).expect("Paged Source lifecycle fixture");
    let probe = gmark_paged_document::probe_file(
        &path,
        gmark_paged_document::ProbeOptions {
            max_resident_bytes: 1,
            ..gmark_paged_document::ProbeOptions::default()
        },
    )
    .expect("Paged Source lifecycle probe");
    assert_eq!(probe.strategy, gmark_paged_document::OpenStrategy::Paged);
    let source =
        gmark_paged_document::FileSource::open(&path).expect("Paged Source lifecycle source");
    let task_path = path.clone();
    let (editor, visual) = cx.add_window_view(move |_window, cx| {
        Editor::from_source_backed_file(cx, task_path, probe, source)
    });
    visual.run_until_parked();
    let host = editor
        .read_with(visual, |editor, _cx| editor.document_host.clone())
        .expect("Paged Source lifecycle Host");
    (editor, host, visual, temp)
}

/// 窗口关闭只能等待原 Source 输入目标提交，不能提前拆掉承载确认文字的 Host。
#[gpui::test]
async fn pending_source_boundary_vetoes_window_close_and_commits_text(cx: &mut TestAppContext) {
    let (editor, host, visual, _temp) = open_paged_editor(cx, "close-pending.txt");
    let (release, wait) = oneshot::channel();

    visual.update(|window, cx| {
        host.update(cx, |host, cx| {
            host.select_source_range_and_focus_for_test(0..0, false, window);
            host.hold_source_boundary_with_confirmed_text_for_test(wait, "confirmed-", window, cx);
            assert!(host.has_pending_source_input());
        });
        editor.update(cx, |editor, cx| {
            let host_id = host.entity_id();
            assert!(editor.has_pending_source_input_in_window(cx));
            assert!(!editor.on_window_should_close(window, cx));
            assert!(
                editor
                    .document_host
                    .as_ref()
                    .is_some_and(|active| active.entity_id() == host_id)
            );
        });
    });

    release.send(()).expect("release boundary lookup");
    visual.run_until_parked();
    assert!(
        host.read_with(visual, |host, _cx| host.source_text_for_test())
            .starts_with("confirmed-")
    );
    assert!(!host.read_with(visual, |host, _cx| host.has_pending_source_input()));
    assert!(!editor.read_with(visual, |editor, cx| {
        editor.has_pending_source_input_in_window(cx)
    }));
}

/// 模式与标签关闭意图排在边界提交之后，提交前保留原活动 Host 和 Source 表面。
#[gpui::test]
async fn source_boundary_defers_mode_and_tab_close_without_detaching_host(cx: &mut TestAppContext) {
    let (editor, host, visual, _temp) = open_paged_editor(cx, "tab-pending.txt");
    let (release, wait) = oneshot::channel();

    visual.update(|window, cx| {
        host.update(cx, |host, cx| {
            host.select_source_range_and_focus_for_test(0..0, false, window);
            host.hold_source_boundary_with_confirmed_text_for_test(wait, "confirmed-", window, cx);
        });
        editor.update(cx, |editor, cx| {
            editor.set_view_mode(ViewMode::Preview, cx);
            editor.request_close_tab_index(0, cx);
            assert_eq!(editor.view_mode, ViewMode::Source);
            assert!(
                editor
                    .document_host
                    .as_ref()
                    .is_some_and(|active| { active.entity_id() == host.entity_id() })
            );
        });
    });

    release.send(()).expect("release boundary lookup");
    visual.run_until_parked();
    assert!(
        host.read_with(visual, |host, _cx| host.source_text_for_test())
            .starts_with("confirmed-")
    );
    editor.read_with(visual, |editor, _cx| {
        assert!(
            editor
                .document_host
                .as_ref()
                .is_some_and(|active| { active.entity_id() == host.entity_id() })
        );
    });
    assert!(visual.debug_bounds("tab-close-dialog").is_some());
}

/// 恢复文字尚未复制时关闭仍等待；Ctrl+C 的 Host 捕获清除恢复门禁并保留文字供粘贴。
#[gpui::test]
async fn source_boundary_recovery_copy_releases_window_lifecycle_gate(cx: &mut TestAppContext) {
    let (editor, host, visual, _temp) = open_paged_editor(cx, "recovery-pending.txt");
    let (release, wait) = oneshot::channel();

    visual.update(|window, cx| {
        host.update(cx, |host, cx| {
            host.select_source_range_and_focus_for_test(0..0, false, window);
            host.fail_next_native_clipboard_write_for_test();
            host.hold_source_boundary_with_confirmed_text_for_test(wait, "recovered-", window, cx);
        });
    });
    drop(release);
    visual.run_until_parked();

    assert!(host.read_with(visual, |host, _cx| host.has_source_boundary_recovery()));
    visual.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            assert!(editor.has_pending_source_input_in_window(cx));
            assert!(!editor.on_window_should_close(window, cx));
        });
    });

    visual.simulate_keystrokes("ctrl-c");
    assert_eq!(
        visual.read(|cx| cx.read_from_clipboard().and_then(|item| item.text()),),
        Some("recovered-".into())
    );
    assert!(!host.read_with(visual, |host, _cx| host.has_pending_source_input()));
    assert!(!editor.read_with(visual, |editor, cx| {
        editor.has_pending_source_input_in_window(cx)
    }));
}

/// 首次拆分没有窗格身份，仍要保留用户意图，并将原 Host 的确认文字带入两个共享视图。
#[gpui::test]
async fn first_pane_split_waits_for_source_boundary_and_preserves_text(cx: &mut TestAppContext) {
    let (editor, host, visual, _temp) = open_paged_editor(cx, "split-pending.txt");
    let (release, wait) = oneshot::channel();
    visual.update(|window, cx| {
        host.update(cx, |host, cx| {
            host.select_source_range_and_focus_for_test(0..0, false, window);
            host.hold_source_boundary_with_confirmed_text_for_test(wait, "confirmed-", window, cx);
        });
        editor.update(cx, |editor, cx| {
            editor.split_pane_toward(crate::editor::panes::PaneSplitDirection::Right, cx);
            assert!(editor.pane_workspace.is_none());
            assert!(editor.has_pending_source_input_in_window(cx));
        });
    });
    release.send(()).expect("release boundary lookup");
    visual.run_until_parked();
    visual.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.sync_pending_ime_operations(window, cx)
        });
    });
    visual.run_until_parked();
    editor.read_with(visual, |editor, cx| {
        let workspace = editor
            .pane_workspace
            .as_ref()
            .expect("deferred first split");
        assert_eq!(workspace.read(cx).workspace().pane_ids().len(), 2);
        assert!(!editor.has_pending_source_input_in_window(cx));
        let texts = editor
            .pane_canvas_entities
            .borrow()
            .values()
            .filter_map(|(_, _, canvas)| match canvas {
                crate::editor::panes::PaneCanvasEntity::DocumentHost(canvas) => {
                    Some(canvas.read(cx).host().read(cx).source_text_for_test())
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(texts.len(), 2);
        assert!(texts.iter().all(|text| text.starts_with("confirmed-")));
    });
}

/// 根窗口的 Paged Host 必须以真实文档身份进入关窗清单，干净文档可直接关闭。
#[gpui::test]
async fn root_paged_host_inventory_is_clean_and_allows_window_close(cx: &mut TestAppContext) {
    let (editor, host, visual, _temp) = open_paged_editor(cx, "root-host-clean.txt");

    visual.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            let document_id = host
                .read(cx)
                .document_id()
                .expect("root Paged Host document identity");
            let inventory = editor.document_close_states(cx);

            assert_eq!(inventory.len(), 1);
            assert_eq!(inventory[0].document_id, document_id);
            assert!(!inventory[0].dirty);
            assert!(editor.on_window_should_close(window, cx));
        });
    });
}

/// 根标签中的隐藏 Host 保持自己的 dirty 状态：干净时放行，变脏后关窗会先切回提示。
#[gpui::test]
async fn inactive_root_host_only_intercepts_window_close_when_dirty(cx: &mut TestAppContext) {
    let (editor, host, visual, _temp) = open_paged_editor(cx, "root-host-inactive.txt");

    editor.update(visual, |editor, cx| {
        assert!(editor.new_document_tab(DocumentKind::Markdown, cx));
    });
    visual.run_until_parked();

    visual.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            assert_eq!(editor.tabs.active, 1);
            assert!(editor.on_window_should_close(window, cx));
            assert_eq!(editor.tabs.active, 1);
        });
    });

    visual.update(|window, cx| {
        editor.update(cx, |editor, cx| assert!(editor.switch_to_tab_index(0, cx)));
        host.update(cx, |host, cx| host.begin_line_edit_for_test(0, window, cx));
    });
    visual.run_until_parked();
    let (_, edit) = host
        .read_with(visual, |host, _cx| host.active_edit_for_test())
        .expect("active root Paged Host edit");
    edit.update(visual, |block, cx| {
        block.replace_text_in_visible_range(0..0, "dirty ", None, false, cx);
    });
    visual.run_until_parked();
    assert!(host.read_with(visual, |host, _cx| host.is_dirty()));

    editor.update(visual, |editor, cx| {
        assert!(editor.new_document_tab(DocumentKind::Markdown, cx));
    });
    visual.run_until_parked();
    visual.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            assert!(!editor.on_window_should_close(window, cx));
            assert_eq!(editor.tabs.active, 0);
            assert!(
                editor
                    .document_host
                    .as_ref()
                    .is_some_and(|active| active.entity_id() == host.entity_id())
            );
        });
    });
}

/// 未保存提示拦截文档快捷键，Esc 恢复编辑且不改变原 Host 的正文或 dirty 状态。
#[gpui::test]
async fn root_host_close_dialog_escape_preserves_text_and_cancels_close(cx: &mut TestAppContext) {
    let (editor, host, visual, _temp) = open_paged_editor(cx, "root-host-escape.txt");
    visual.update(|window, cx| {
        host.update(cx, |host, cx| host.begin_line_edit_for_test(0, window, cx));
    });
    visual.run_until_parked();
    let (_, edit) = host
        .read_with(visual, |host, _cx| host.active_edit_for_test())
        .expect("active root Paged Host edit");
    edit.update(visual, |block, cx| {
        block.replace_text_in_visible_range(0..0, "dirty ", None, false, cx);
    });
    visual.run_until_parked();
    let before = host.read_with(visual, |host, _cx| host.source_text_for_test());
    visual.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            assert!(!editor.on_window_should_close(window, cx));
            assert!(editor.show_unsaved_changes_dialog);
        });
    });
    visual.run_until_parked();
    visual.simulate_keystrokes("ctrl-n");
    visual.run_until_parked();
    editor.read_with(visual, |editor, _cx| {
        assert_eq!(editor.tabs.records.len(), 1);
        assert!(editor.show_unsaved_changes_dialog);
    });
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();
    editor.read_with(visual, |editor, _cx| {
        assert!(!editor.show_unsaved_changes_dialog);
        assert!(!editor.pending_close_after_save);
        assert!(
            editor
                .document_host
                .as_ref()
                .is_some_and(|active| { active.entity_id() == host.entity_id() })
        );
    });
    host.read_with(visual, |host, _cx| {
        assert!(host.is_dirty());
        assert_eq!(host.source_text_for_test(), before);
    });
}

/// 关闭提示的默认保存按钮可用 Tab/Shift+Tab 循环到取消，Enter/Space 与鼠标走同一入口。
#[gpui::test]
async fn root_host_close_dialog_keyboard_buttons_preserve_unsaved_text(cx: &mut TestAppContext) {
    let (editor, host, visual, _temp) = open_paged_editor(cx, "root-host-dialog-keys.txt");
    visual.update(|window, cx| {
        host.update(cx, |host, cx| host.begin_line_edit_for_test(0, window, cx));
    });
    visual.run_until_parked();
    let (_, edit) = host
        .read_with(visual, |host, _cx| host.active_edit_for_test())
        .expect("active root Paged Host edit");
    edit.update(visual, |block, cx| {
        block.replace_text_in_visible_range(0..0, "dirty ", None, false, cx);
    });
    visual.run_until_parked();
    let before = host.read_with(visual, |host, _cx| host.source_text_for_test());
    for keys in [
        vec!["tab", "enter"],
        vec!["shift-tab", "shift-tab", "space"],
    ] {
        visual.update(|window, cx| {
            editor.update(cx, |editor, cx| {
                assert!(!editor.on_window_should_close(window, cx))
            });
        });
        visual.run_until_parked();
        for key in keys {
            visual.simulate_keystrokes(key);
            if matches!(key, "enter" | "space") {
                // simulate_keystrokes 仅派发 KeyDown；原生按钮在 KeyUp 产生 KeyboardClickEvent。
                visual.simulate_event(gpui::KeyUpEvent {
                    keystroke: gpui::Keystroke::parse(key).expect("dialog activation key"),
                });
            }
            visual.run_until_parked();
        }
        editor.read_with(visual, |editor, _cx| {
            assert!(!editor.show_unsaved_changes_dialog)
        });
        host.read_with(visual, |host, _cx| {
            assert!(host.is_dirty());
            assert_eq!(host.source_text_for_test(), before);
        });
    }
}

/// 默认保存必须等待真实磁盘终态再卸载 Host；旧实体迟到的通知不能污染替换后的空白标签。
#[gpui::test]
async fn root_host_save_and_close_finishes_after_disk_commit(cx: &mut TestAppContext) {
    let (editor, host, visual, temp) = open_paged_editor(cx, "root-host-save-close.txt");
    visual.update(|window, cx| {
        host.update(cx, |host, cx| host.begin_line_edit_for_test(0, window, cx));
    });
    visual.run_until_parked();
    visual.simulate_input(" changed");
    visual.run_until_parked();
    let expected = host.read_with(visual, |host, _cx| host.source_text_for_test());
    assert!(host.read_with(visual, |host, _cx| host.is_dirty()));
    visual.simulate_keystrokes("ctrl-w");
    visual.run_until_parked();
    assert!(visual.debug_bounds("tab-close-dialog").is_some());
    visual.simulate_keystrokes("enter");
    visual.simulate_event(gpui::KeyUpEvent {
        keystroke: gpui::Keystroke::parse("enter").expect("save-close activation key"),
    });
    for _ in 0..5_000 {
        visual.run_until_parked();
        if !host.read_with(visual, |host, _cx| host.is_saving_for_test()) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    visual.run_until_parked();
    assert_eq!(
        fs::read_to_string(temp.path().join("root-host-save-close.txt"))
            .expect("committed save-close fixture"),
        expected
    );
    editor.read_with(visual, |editor, _cx| {
        assert!(
            editor.document_host.is_none(),
            "saved Host must be unloaded"
        );
        assert!(editor.file_path.is_none());
        assert_eq!(editor.source_document.text(), "");
    });
    // Holding the old entity models a queued platform/worker callback after tab replacement.
    visual.update(|_window, cx| {
        host.update(cx, |_host, cx| {
            cx.emit(crate::document_host::DocumentHostEvent::StateChanged)
        });
    });
    visual.run_until_parked();
    editor.read_with(visual, |editor, _cx| {
        assert!(!editor.is_document_dirty());
        assert!(editor.file_path.is_none());
        assert_eq!(editor.tabs.records.len(), 1);
    });
    visual.update(|window, cx| {
        crate::app::document_service::DocumentService::init(cx);
        editor.update(cx, |editor, cx| {
            editor.on_reopen_closed_tab_action(&crate::components::ReopenClosedTab, window, cx);
        });
    });
    for _ in 0..5_000 {
        visual.run_until_parked();
        if editor.read_with(visual, |editor, _cx| editor.document_host.is_some()) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    let reopened = editor
        .read_with(visual, |editor, _cx| editor.document_host.clone())
        .expect("saved Host must reopen with current disk identity");
    assert_eq!(
        reopened.read_with(visual, |host, _cx| host.source_text_for_test()),
        expected
    );
}

/// 非活动 Host 仍可由后台任务持有，其通知不能把新文档的 dirty 清掉或误改标题。
#[gpui::test]
async fn inactive_root_host_event_preserves_current_tab_dirty(cx: &mut TestAppContext) {
    let (editor, host, visual, _temp) = open_paged_editor(cx, "inactive-host.txt");
    editor.update(visual, |editor, cx| {
        assert!(editor.new_document_tab(DocumentKind::Markdown, cx));
    });
    visual.run_until_parked();
    visual.simulate_input("unsaved new document");
    visual.run_until_parked();
    assert!(editor.read_with(visual, |editor, _cx| editor.is_document_dirty()));
    visual.update(|_window, cx| {
        host.update(cx, |_host, cx| {
            cx.emit(crate::document_host::DocumentHostEvent::StateChanged)
        });
    });
    visual.run_until_parked();
    editor.read_with(visual, |editor, _cx| {
        assert!(editor.is_document_dirty());
        assert!(editor.file_path.is_none());
        assert_eq!(editor.source_document.text(), "unsaved new document");
    });
}

/// 同一修订的重复保存可折叠，但关闭意图仍须在排队请求消耗后完成。
#[gpui::test]
async fn root_host_queued_save_and_close_finishes_once(cx: &mut TestAppContext) {
    let (editor, host, visual, temp) = open_paged_editor(cx, "queued-save-close.txt");
    visual.update(|window, cx| {
        host.update(cx, |host, cx| host.begin_line_edit_for_test(0, window, cx));
    });
    visual.run_until_parked();
    visual.simulate_input(" queued change");
    visual.run_until_parked();
    let expected = host.read_with(visual, |host, _cx| host.source_text_for_test());
    visual.update(|window, cx| {
        host.update(cx, |host, cx| {
            host.on_save_document(&crate::components::SaveDocument, window, cx);
        });
        editor.update(cx, |editor, cx| {
            editor.on_save_tab_close(&gpui::ClickEvent::default(), window, cx);
            assert!(editor.document_host.is_some());
        });
    });
    for _ in 0..5_000 {
        visual.run_until_parked();
        if editor.read_with(visual, |editor, _cx| editor.document_host.is_none()) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert_eq!(
        fs::read_to_string(temp.path().join("queued-save-close.txt")).expect("queued save result"),
        expected
    );
    editor.read_with(visual, |editor, _cx| {
        assert!(editor.document_host.is_none());
        assert_eq!(editor.tabs.records.len(), 1);
        assert_eq!(editor.source_document.text(), "");
    });
}

/// 写盘失败必须撤销关闭意图；用户随后手动重试成功也不能执行已取消的关闭。
#[gpui::test]
async fn root_host_failed_save_and_close_preserves_editing(cx: &mut TestAppContext) {
    let (editor, host, visual, temp) = open_paged_editor(cx, "failed-save-close.txt");
    let path = temp.path().join("failed-save-close.txt");
    let baseline = fs::read(&path).expect("failed save baseline");
    visual.update(|window, cx| {
        host.update(cx, |host, cx| host.begin_line_edit_for_test(0, window, cx));
    });
    visual.run_until_parked();
    visual.simulate_input(" unsaved change");
    visual.run_until_parked();
    let expected = host.read_with(visual, |host, _cx| host.source_text_for_test());
    let moved = temp.path().join("original-moved.txt");
    fs::rename(&path, &moved).expect("make save target unavailable");
    visual.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.on_save_tab_close(&gpui::ClickEvent::default(), window, cx);
        });
    });
    for _ in 0..5_000 {
        visual.run_until_parked();
        if !host.read_with(visual, |host, _cx| host.is_saving_for_test()) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    visual.run_until_parked();
    host.read_with(visual, |host, _cx| {
        assert!(host.error_for_test().is_some());
        assert!(host.is_dirty());
        assert_eq!(host.source_text_for_test(), expected);
    });
    assert_eq!(fs::read(&moved).expect("preserved disk baseline"), baseline);
    fs::rename(&moved, &path).expect("restore original save target for retry");
    visual.update(|window, cx| {
        host.update(cx, |host, cx| {
            host.on_save_document(&crate::components::SaveDocument, window, cx);
        });
    });
    for _ in 0..5_000 {
        visual.run_until_parked();
        if !host.read_with(visual, |host, _cx| host.is_saving_for_test()) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    visual.run_until_parked();
    assert!(!host.read_with(visual, |host, _cx| host.is_dirty()));
    assert_eq!(
        fs::read_to_string(&path).expect("save retry result"),
        expected
    );
    editor.read_with(visual, |editor, _cx| {
        assert!(editor.document_host.is_some());
        assert_eq!(editor.file_path.as_deref(), Some(path.as_path()));
    });
}

/// 系统关窗的保存按钮须关闭真实窗口，不能仅清 dirty 而遗留空壳。
#[gpui::test]
async fn root_host_window_save_and_close_removes_window(cx: &mut TestAppContext) {
    let (editor, host, visual, temp) = open_paged_editor(cx, "window-save-close.txt");
    visual.update(|window, cx| {
        host.update(cx, |host, cx| host.begin_line_edit_for_test(0, window, cx));
    });
    visual.run_until_parked();
    visual.simulate_input(" window change");
    visual.run_until_parked();
    let expected = host.read_with(visual, |host, _cx| host.source_text_for_test());
    visual.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            assert!(!editor.on_window_should_close(window, cx));
            editor.on_save_and_close(&gpui::ClickEvent::default(), window, cx);
        });
    });
    for _ in 0..5_000 {
        visual.run_until_parked();
        if !host.read_with(visual, |host, _cx| host.is_dirty()) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    visual.run_until_parked();
    assert_eq!(
        fs::read_to_string(temp.path().join("window-save-close.txt")).expect("window close save"),
        expected
    );
    assert!(editor.read_with(visual, |_editor, cx| cx.windows().is_empty()));
}
