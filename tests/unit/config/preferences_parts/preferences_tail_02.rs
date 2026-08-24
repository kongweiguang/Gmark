// @author kongweiguang

/// 脏状态使用真正的自定义键位作为输入；平台默认组合会被规范化掉，不能代表草稿变化。
#[gpui::test]
async fn preferences_dirty_state_tracks_draft_changes(cx: &mut TestAppContext) {
    init_preferences_test_app(cx);

    let handle = cx.update(|cx| {
        open_preferences_window_with_state(cx, AppPreferences::default(), "Preferences".into())
    });
    cx.run_until_parked();

    handle
        .update(cx, |preferences, _window, _cx| {
            assert!(!preferences.has_unsaved_changes());
            preferences.startup_open = StartupOpenPreference::LastOpenedFile;
            assert!(preferences.has_unsaved_changes());
            preferences.startup_open = StartupOpenPreference::NewFile;
            assert!(!preferences.has_unsaved_changes());

            preferences.image_paste_behavior = ImagePasteBehavior::CopyToAssetsFolder;
            assert!(preferences.has_unsaved_changes());
            preferences.image_paste_behavior = ImagePasteBehavior::None;
            assert!(!preferences.has_unsaved_changes());

            preferences
                .keybindings
                .insert("save_document".into(), vec!["ctrl-alt-shift-s".into()]);
            assert!(preferences.has_unsaved_changes());
        })
        .expect("preferences window should be updateable");
}

#[gpui::test]
async fn applying_saved_preferences_keeps_window_open_and_focused(cx: &mut TestAppContext) {
    init_preferences_test_app(cx);

    let handle = cx.update(|cx| {
        open_preferences_window_with_state(cx, AppPreferences::default(), "Preferences".into())
    });
    cx.run_until_parked();

    handle
        .update(cx, |preferences, window, cx| {
            preferences.startup_open = StartupOpenPreference::LastOpenedFile;
            assert!(preferences.has_unsaved_changes());
            let saved = AppPreferences {
                startup_open: StartupOpenPreference::LastOpenedFile,
                ..AppPreferences::default()
            };
            preferences.apply_saved_preferences(saved, window, cx);
        })
        .expect("preferences window should be updateable");
    cx.run_until_parked();

    assert_eq!(cx.update(|cx| cx.windows().len()), 1);
    let active_window = cx.update(|cx| cx.active_window().expect("window should be active"));
    assert_eq!(active_window.window_id(), handle.window_id());
    assert!(
        handle
            .update(cx, |preferences, window, _cx| preferences
                .focus_handle
                .is_focused(window))
            .expect("preferences window should remain updateable")
    );
    assert!(
        !handle
            .update(cx, |preferences, _window, _cx| preferences
                .has_unsaved_changes())
            .expect("preferences window should remain updateable")
    );
}

#[test]
fn numeric_preference_input_rejects_empty_non_numeric_and_out_of_range_values() {
    assert_eq!(
        parse_numeric_input(PreferencesNumericInput::FontSize, " 20 "),
        Some(20)
    );
    assert_eq!(
        parse_numeric_input(PreferencesNumericInput::ResidentMib, "1024"),
        Some(1_024)
    );
    assert_eq!(
        parse_numeric_input(PreferencesNumericInput::FontSize, ""),
        None
    );
    assert_eq!(
        parse_numeric_input(PreferencesNumericInput::FontSize, "large"),
        None
    );
    assert_eq!(
        parse_numeric_input(PreferencesNumericInput::FontSize, "25"),
        None
    );
}

#[gpui::test]
async fn numeric_preference_text_input_updates_preview_and_stays_in_sync(cx: &mut TestAppContext) {
    init_preferences_test_app(cx);
    let handle = cx.update(|cx| {
        open_preferences_window_with_state(cx, AppPreferences::default(), "Preferences".into())
    });
    cx.run_until_parked();

    handle
        .update(cx, |preferences, _window, cx| {
            let input =
                preferences.numeric_inputs[PreferencesNumericInput::FontSize.index()].clone();
            input.update(cx, |input, cx| {
                let len = input.visible_len();
                input.replace_text_in_visible_range(0..len, "20", None, false, cx);
            });
        })
        .expect("preferences window should be updateable");
    cx.run_until_parked();
    handle
        .update(cx, |preferences, _window, cx| {
            assert_eq!(preferences.editor_font_size, 20);
            assert_eq!(
                cx.global::<ThemeManager>().current().typography.text_size,
                20.0
            );
            assert!(preferences.numeric_input_is_valid(PreferencesNumericInput::FontSize, cx));

            let input =
                preferences.numeric_inputs[PreferencesNumericInput::FontSize.index()].clone();
            input.update(cx, |input, cx| {
                let len = input.visible_len();
                input.replace_text_in_visible_range(0..len, "999", None, false, cx);
            });
        })
        .expect("preferences window should remain updateable");
    cx.run_until_parked();
    handle
        .update(cx, |preferences, _window, cx| {
            assert_eq!(preferences.editor_font_size, 20);
            assert!(preferences.has_invalid_numeric_input(cx));
            preferences.activate_stepper(PreferencesStepperControl::FontSizeIncrease, cx);
        })
        .expect("preferences window should remain updateable");
    cx.run_until_parked();
    handle
        .update(cx, |preferences, _window, cx| {
            assert_eq!(preferences.editor_font_size, 21);
            assert_eq!(
                preferences.numeric_inputs[PreferencesNumericInput::FontSize.index()]
                    .read(cx)
                    .display_text(),
                "21"
            );
            assert!(!preferences.has_invalid_numeric_input(cx));
        })
        .expect("preferences window should remain updateable");
}

#[gpui::test]
async fn visual_accessibility_drafts_preview_and_restore_with_keyboard_focus(
    cx: &mut TestAppContext,
) {
    init_preferences_test_app(cx);
    cx.update(|cx| {
        crate::ui::visual_preferences::VisualPreferencesManager::init(
            cx,
            gmark_config::VisualAccessibilityPreferences::default(),
        );
    });
    let initial_reduced_motion = cx.update(|cx| {
        cx.global::<crate::ui::visual_preferences::VisualPreferencesManager>()
            .current()
            .reduced_motion
    });
    let handle = cx.update(|cx| {
        open_preferences_window_with_state(cx, AppPreferences::default(), "Preferences".into())
    });
    cx.run_until_parked();

    handle
        .update(cx, |preferences, window, cx| {
            preferences.nav = PreferencesNav::Theme;
            preferences.accessibility_focus_handles[0][1].focus(window);
            preferences.set_accessibility_override(
                PreferencesAccessibilityControl::ReducedMotion,
                gmark_config::AccessibilityOverride::Enabled,
                cx,
            );
            assert_eq!(
                preferences.visual_accessibility.reduced_motion,
                gmark_config::AccessibilityOverride::Enabled
            );
            assert!(preferences.has_unsaved_changes());
            assert!(cx
                .global::<crate::ui::visual_preferences::VisualPreferencesManager>()
                .current()
                .reduced_motion);
            assert!(preferences.accessibility_focus_handles[0][1].is_focused(window));

            preferences.restore_saved_visual_accessibility(cx);
            assert_eq!(
                preferences.visual_accessibility,
                gmark_config::VisualAccessibilityPreferences::default()
            );
            assert!(!preferences.has_unsaved_changes());
            assert_eq!(
                cx.global::<crate::ui::visual_preferences::VisualPreferencesManager>()
                    .current()
                    .reduced_motion,
                initial_reduced_motion
            );
        })
        .expect("preferences window should remain updateable");
}

#[gpui::test]
async fn visual_accessibility_controls_reflow_below_navigation_on_narrow_windows(
    cx: &mut TestAppContext,
) {
    init_preferences_test_app(cx);
    let handle = cx.update(|cx| {
        open_preferences_window_with_state(cx, AppPreferences::default(), "Preferences".into())
    });
    cx.run_until_parked();
    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    visual.simulate_resize(size(px(680.0), px(520.0)));
    handle
        .update(&mut visual, |preferences, _window, cx| {
            preferences.nav = PreferencesNav::Theme;
            cx.notify();
        })
        .expect("preferences window should remain updateable");
    visual.update(|window, cx| window.draw(cx).clear());
    visual.run_until_parked();

    let content = visual.debug_bounds("preferences-content").unwrap();
    let navigation = visual.debug_bounds("preferences-navigation").unwrap();
    let main = visual.debug_bounds("preferences-main").unwrap();
    let page = visual.debug_bounds("preferences-page-scroll").unwrap();
    assert_eq!(navigation.left(), content.left());
    assert_eq!(navigation.right(), content.right());
    assert!(main.top() >= navigation.bottom());
    for selector in [
        "preferences-accessibility-reduced-motion",
        "preferences-accessibility-reduced-transparency",
        "preferences-accessibility-high-contrast",
    ] {
        let bounds = visual
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("missing {selector}"));
        assert!(bounds.left() >= page.left(), "{selector} escaped left");
        assert!(bounds.right() <= page.right(), "{selector} escaped right");
    }
}
