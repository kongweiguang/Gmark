// @author kongweiguang

use gpui::{ElementId, TestAppContext};

/// 保留的克隆和弱引用升级必须遵守同一焦点生命周期。
#[gpui::test]
async fn focus_handle_clone_keeps_weak_reference_alive(cx: &mut TestAppContext) {
    let handle = cx.update(|app| app.focus_handle());
    let clone = handle.clone();
    let weak = handle.downgrade();
    cx.update(|_| drop(handle));
    assert!(weak.upgrade().is_some());
    cx.update(|_| drop(clone));
    assert!(weak.upgrade().is_none());
}

/// 校验窗口记录的焦点身份被清除，不能只让弱引用无法升级。
#[gpui::test]
async fn dropping_focus_clears_focus_in_both_windows(cx: &mut TestAppContext) {
    let handle = cx.update(|app| app.focus_handle());
    let ElementId::FocusHandle(focus_id) = ElementId::from(&handle) else {
        panic!("focus handle must yield its focus identity");
    };
    let first = cx.add_window(|_, _| gpui::Empty);
    let second = cx.add_window(|_, _| gpui::Empty);
    cx.update(|app| {
        first
            .update(app, |_, window, _| window.focus(&handle))
            .unwrap();
        second
            .update(app, |_, window, _| window.focus(&handle))
            .unwrap();
    });
    cx.update(|app| {
        first
            .update(app, |_, window, _| assert!(focus_id.is_focused(window)))
            .unwrap();
        second
            .update(app, |_, window, _| assert!(focus_id.is_focused(window)))
            .unwrap();
        drop(handle);
    });
    cx.update(|app| {
        first
            .update(app, |_, window, _| assert!(!focus_id.is_focused(window)))
            .unwrap();
        second
            .update(app, |_, window, _| assert!(!focus_id.is_focused(window)))
            .unwrap();
    });
}
