// @author kongweiguang

/// 使用未被任一平台默认键占用的组合验证持久化，避免产品默认值调整把有效覆盖
/// 规范化为空并掩盖偏好存储本身的契约。
#[test]
fn saving_preferences_window_persists_selected_language() {
    let root =
        std::env::temp_dir().join(format!("gmark-preferences-window-{}", uuid::Uuid::new_v4()));
    let dirs = AppDirs::from_root(&root);
    let preferences = AppPreferences {
        startup_open: StartupOpenPreference::NewFile,
        auto_check_updates: true,
        default_language_id: "zh-CN".into(),
        theme_appearance: ThemeAppearance::System,
        theme_palette: ThemePalette::Xcode,
        visual_accessibility: Default::default(),
        show_table_headers: true,
        image_paste_behavior: ImagePasteBehavior::None,
        auto_save: AutoSavePreference::Off,
        spell_check: true,
        auto_pair_brackets: true,
        auto_pair_markdown: true,
        code_folding: true,
        format_on_save: false,
        editor_font_size: 17,
        editor_line_height_percent: 160,
        editor_content_width: 880,
        editor_font_family: String::new(),
        show_tab_bar_actions: false,
        recent_editing_commands: Vec::new(),
        keybindings: BTreeMap::new(),
        status_bar: StatusBarPreferences::default(),
        document_loading: DocumentLoadingPreferences::default(),
    };
    save_app_preferences_with_dirs(&preferences, &dirs)
        .expect("preferences should save to config.toml");

    let status_bar = StatusBarPreferences {
        custom_buttons: vec![StatusBarButton {
            id: "publish".into(),
            label: "Publish".into(),
            action_id: "publish_document".into(),
        }],
        ..StatusBarPreferences::default()
    };
    let document_loading = DocumentLoadingPreferences {
        max_resident_mib: Some(96),
    };
    let saved = save_preferences_from_window_with_dirs(
        StartupOpenPreference::LastOpenedFile,
        false,
        AutoSavePreference::AfterDelay,
        false,
        false,
        false,
        true,
        false,
        18,
        175,
        960,
        "Georgia",
        true,
        ThemeAppearance::Light,
        ThemePalette::Fleet,
        "en-US",
        ImagePasteBehavior::CopyToNamedAssetsFolder,
        BTreeMap::from([(
            "save_document".to_string(),
            vec!["ctrl-alt-shift-s".to_string()],
        )]),
        &document_loading,
        &status_bar,
        &dirs,
    )
    .expect("window preferences should save");
    assert_eq!(saved.default_language_id, "en-US");
    assert_eq!(saved.startup_open, StartupOpenPreference::LastOpenedFile);
    assert!(!saved.auto_check_updates);
    assert_eq!(saved.theme_appearance, ThemeAppearance::Light);
    assert_eq!(saved.theme_palette, ThemePalette::Fleet);
    let text = std::fs::read_to_string(dirs.app_config_file()).expect("config.toml should exist");
    assert!(text.contains("palette = \"fleet\""));
    assert!(!text.contains("jetbrains"));
    assert_eq!(saved.auto_save, AutoSavePreference::AfterDelay);
    assert!(!saved.spell_check);
    assert!(!saved.auto_pair_brackets);
    assert!(!saved.auto_pair_markdown);
    assert_eq!(saved.editor_font_size, 18);
    assert_eq!(saved.editor_line_height_percent, 175);
    assert_eq!(saved.editor_content_width, 960);
    assert_eq!(saved.editor_font_family, "Georgia");
    assert!(saved.show_tab_bar_actions);
    assert_eq!(saved.document_loading, document_loading);
    assert_eq!(
        saved.image_paste_behavior,
        ImagePasteBehavior::CopyToNamedAssetsFolder
    );
    assert_eq!(
        saved.keybindings.get("save_document"),
        Some(&vec!["ctrl-alt-shift-s".to_string()])
    );
    assert_eq!(saved.status_bar.custom_buttons, status_bar.custom_buttons);
    let _ = std::fs::remove_dir_all(root);
}

#[gpui::test]
async fn preferences_window_activates_and_focuses_on_open(cx: &mut TestAppContext) {
    init_preferences_test_app(cx);

    let handle = cx.update(|cx| {
        open_preferences_window_with_state(cx, AppPreferences::default(), "Preferences".into())
    });
    cx.run_until_parked();

    let active_window = cx.update(|cx| cx.active_window().expect("window should be active"));
    assert_eq!(active_window.window_id(), handle.window_id());
    assert!(
        handle
            .update(cx, |preferences, window, _cx| preferences
                .focus_handle
                .is_focused(window))
            .expect("preferences window should be updateable")
    );
    assert!(
        !handle
            .update(cx, |preferences, _window, _cx| preferences
                .has_unsaved_changes())
            .expect("preferences window should be updateable")
    );
}

#[gpui::test]
async fn preferences_pages_keep_actions_visible_at_two_x_scale(cx: &mut TestAppContext) {
    init_preferences_test_app(cx);
    let handle = cx.update(|cx| {
        open_preferences_window_with_state(cx, AppPreferences::default(), "Preferences".into())
    });
    cx.run_until_parked();
    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    for viewport in [size(px(720.0), px(520.0)), size(px(980.0), px(700.0))] {
        visual.simulate_resize(viewport);
        for nav in [
            PreferencesNav::File,
            PreferencesNav::Editor,
            PreferencesNav::Theme,
            PreferencesNav::Image,
            PreferencesNav::Shortcuts,
            PreferencesNav::StatusBar,
        ] {
            handle
                .update(&mut visual, |preferences, _window, cx| {
                    preferences.nav = nav;
                    cx.notify();
                })
                .unwrap();
            visual.update(|window, cx| {
                assert_eq!(window.scale_factor(), 2.0);
                window.draw(cx).clear();
            });
            visual.run_until_parked();

            let content = visual.debug_bounds("preferences-content").unwrap();
            if let Some(titlebar) = visual.debug_bounds("preferences-titlebar") {
                let title_label = visual
                    .debug_bounds("preferences-titlebar-title-label")
                    .unwrap();
                assert!(title_label.left() >= titlebar.left());
                assert!(title_label.right() <= titlebar.right());
                if cfg!(target_os = "macos") {
                    assert!(
                        visual
                            .debug_bounds("preferences-titlebar-leading-icon")
                            .is_none()
                    );
                    assert!(
                        (f32::from(title_label.center().x) - f32::from(titlebar.center().x)).abs()
                            <= 1.0
                    );
                } else {
                    // Windows uses the text-only preference chrome so the
                    // localized title stays aligned with the content column.
                    assert!(
                        visual
                            .debug_bounds("preferences-titlebar-leading-icon")
                            .is_none()
                    );
                }
            }
            let navigation = visual.debug_bounds("preferences-navigation").unwrap();
            let search = visual.debug_bounds("preferences-search-input").unwrap();
            let search_icon = visual.debug_bounds("preferences-search-icon").unwrap();
            let main = visual.debug_bounds("preferences-main").unwrap();
            let title = visual.debug_bounds("preferences-page-title").unwrap();
            let page = visual.debug_bounds("preferences-page-scroll").unwrap();
            let actions = visual.debug_bounds("preferences-actions").unwrap();
            let cancel = visual.debug_bounds("preferences-cancel").unwrap();
            let save = visual.debug_bounds("preferences-save").unwrap();
            assert_eq!(f32::from(navigation.size.width), 200.0);
            assert_eq!(f32::from(search.size.height), 34.0);
            assert_eq!(navigation.right(), main.left());
            assert!(search.left() >= navigation.left());
            assert!(search.right() <= navigation.right());
            assert!(search_icon.left() >= search.left());
            assert!(search_icon.right() <= search.right());
            for selector in [
                "preferences-nav-file",
                "preferences-nav-editor",
                "preferences-nav-theme",
                "preferences-nav-image",
                "preferences-nav-shortcuts",
                "preferences-nav-status-bar",
            ] {
                let row = visual.debug_bounds(selector).unwrap();
                assert_eq!(f32::from(row.size.height), 36.0, "{selector}");
                assert!(row.left() >= navigation.left(), "{selector}");
                assert!(row.right() <= navigation.right(), "{selector}");
            }
            for (name, bounds) in [
                ("navigation", navigation),
                ("main", main),
                ("title", title),
                ("page", page),
                ("actions", actions),
            ] {
                assert!(bounds.left() >= content.left(), "{name} escaped left");
                assert!(bounds.right() <= content.right(), "{name} escaped right");
                assert!(bounds.top() >= content.top(), "{name} escaped top");
                assert!(bounds.bottom() <= content.bottom(), "{name} escaped bottom");
            }
            assert!(title.left() >= main.left());
            assert!(title.right() <= main.right());
            let page_control_selector = match nav {
                PreferencesNav::File => "preferences-startup-dropdown",
                PreferencesNav::Editor => "preferences-editor-font-size",
                PreferencesNav::Theme => "preferences-theme-appearance-system",
                PreferencesNav::Image => "preferences-image-dropdown",
                PreferencesNav::Shortcuts => "preferences-shortcuts-scroll",
                PreferencesNav::StatusBar => "preferences-status-bar-options",
            };
            let page_control = visual
                .debug_bounds(page_control_selector)
                .unwrap_or_else(|| {
                    panic!(
                        "missing {page_control_selector} for {nav:?} at {}x{}",
                        viewport.width, viewport.height
                    )
                });
            assert!(page_control.left() >= page.left());
            assert!(page_control.right() <= page.right());
            for (name, bounds) in [("cancel", cancel), ("save", save)] {
                assert!(bounds.left() >= actions.left(), "{name} escaped actions");
                assert!(bounds.right() <= actions.right(), "{name} escaped actions");
                assert!(bounds.top() >= actions.top(), "{name} escaped actions");
                assert!(
                    bounds.bottom() <= actions.bottom(),
                    "{name} escaped actions"
                );
            }
        }
    }
}

#[gpui::test]
async fn language_preference_is_editable_from_the_theme_page(cx: &mut TestAppContext) {
    init_preferences_test_app(cx);
    let handle = cx.update(|cx| {
        open_preferences_window_with_state(cx, AppPreferences::default(), "Preferences".into())
    });
    cx.run_until_parked();

    handle
        .update(cx, |preferences, _window, cx| {
            assert_eq!(preferences.selected_language_id, "en-US");
            preferences.commit_dropdown_selection(PreferencesDropdown::Language, 0, cx);
            assert_eq!(preferences.selected_language_id, "zh-CN");
            assert!(preferences.has_unsaved_changes());
            assert!(!preferences.language_dropdown_open);
        })
        .expect("preferences window should be updateable");
}

#[gpui::test]
async fn preferences_navigation_supports_directional_and_activation_keys(cx: &mut TestAppContext) {
    init_preferences_test_app(cx);
    let handle = cx.update(|cx| {
        open_preferences_window_with_state(cx, AppPreferences::default(), "Preferences".into())
    });
    cx.run_until_parked();
    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    visual.simulate_resize(size(px(720.0), px(520.0)));
    visual.update(|window, cx| window.draw(cx).clear());

    handle
        .update(&mut visual, |preferences, window, _cx| {
            preferences.nav_focus_handles[0].focus(window);
            assert!(preferences.nav_focus_handles[0].is_focused(window));
        })
        .unwrap();
    visual.simulate_keystrokes("right");
    visual.run_until_parked();
    handle
        .update(&mut visual, |preferences, window, _cx| {
            assert_eq!(preferences.nav, PreferencesNav::Editor);
            assert!(preferences.nav_focus_handles[1].is_focused(window));
        })
        .unwrap();

    visual.update(|window, cx| window.draw(cx).clear());
    visual.simulate_keystrokes("end");
    visual.run_until_parked();
    handle
        .update(&mut visual, |preferences, window, _cx| {
            assert_eq!(preferences.nav, PreferencesNav::StatusBar);
            assert!(preferences.nav_focus_handles[5].is_focused(window));
        })
        .unwrap();

    visual.update(|window, cx| window.draw(cx).clear());
    visual.simulate_keystrokes("home");
    visual.run_until_parked();
    handle
        .update(&mut visual, |preferences, window, _cx| {
            assert_eq!(preferences.nav, PreferencesNav::File);
            assert!(preferences.nav_focus_handles[0].is_focused(window));
            assert!(!preferences.has_unsaved_changes());
        })
        .unwrap();

    handle
        .update(&mut visual, |preferences, window, _cx| {
            preferences.nav_focus_handles[2].focus(window);
        })
        .unwrap();
    visual.simulate_keystrokes("space");
    visual.run_until_parked();
    handle
        .update(&mut visual, |preferences, window, _cx| {
            assert_eq!(preferences.nav, PreferencesNav::Theme);
            assert!(preferences.nav_focus_handles[2].is_focused(window));
            assert!(!preferences.has_unsaved_changes());
        })
        .unwrap();

    visual.simulate_resize(size(px(980.0), px(700.0)));
    visual.update(|window, cx| {
        assert_eq!(window.scale_factor(), 2.0);
        window.draw(cx).clear();
    });
    let navigation = visual.debug_bounds("preferences-navigation").unwrap();
    for selector in [
        "preferences-nav-file",
        "preferences-nav-editor",
        "preferences-nav-theme",
        "preferences-nav-image",
        "preferences-nav-shortcuts",
        "preferences-nav-status-bar",
    ] {
        let row = visual.debug_bounds(selector).unwrap();
        assert_eq!(f32::from(row.size.height), 36.0, "{selector}");
        assert!(row.left() >= navigation.left(), "{selector}");
        assert!(row.right() <= navigation.right(), "{selector}");
    }
}

#[gpui::test]
async fn preferences_dropdowns_support_keyboard_navigation_and_commit(cx: &mut TestAppContext) {
    init_preferences_test_app(cx);
    let handle = cx.update(|cx| {
        open_preferences_window_with_state(cx, AppPreferences::default(), "Preferences".into())
    });
    cx.run_until_parked();
    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    visual.simulate_resize(size(px(720.0), px(520.0)));
    visual.update(|window, cx| window.draw(cx).clear());

    handle
        .update(&mut visual, |preferences, window, _cx| {
            preferences.dropdown_focus_handles[PreferencesDropdown::Startup.index()].focus(window);
        })
        .unwrap();
    visual.simulate_keystrokes("enter down enter");
    visual.run_until_parked();
    handle
        .update(&mut visual, |preferences, window, _cx| {
            assert_eq!(
                preferences.startup_open,
                StartupOpenPreference::LastOpenedFile
            );
            assert!(!preferences.startup_dropdown_open);
            assert!(preferences.dropdown_focus_handles[0].is_focused(window));
            preferences.dropdown_focus_handles[PreferencesDropdown::AutoSave.index()].focus(window);
        })
        .unwrap();

    visual.simulate_keystrokes("space end enter");
    visual.run_until_parked();
    handle
        .update(&mut visual, |preferences, window, cx| {
            assert_eq!(preferences.auto_save, AutoSavePreference::AfterDelay);
            assert!(!preferences.auto_save_dropdown_open);
            preferences.select_nav(PreferencesNav::Image, cx);
            preferences.dropdown_focus_handles[PreferencesDropdown::Image.index()].focus(window);
        })
        .unwrap();
    visual.update(|window, cx| window.draw(cx).clear());

    visual.simulate_keystrokes("down end enter");
    visual.run_until_parked();
    handle
        .update(&mut visual, |preferences, _window, cx| {
            assert_eq!(
                preferences.image_paste_behavior,
                ImagePasteBehavior::CopyToNamedAssetsFolder
            );
            preferences.set_dropdown_open(PreferencesDropdown::Image, true);
            preferences.select_nav(PreferencesNav::File, cx);
            assert!(!preferences.image_dropdown_open);
        })
        .unwrap();

    for viewport in [size(px(720.0), px(520.0)), size(px(980.0), px(700.0))] {
        visual.simulate_resize(viewport);
        handle
            .update(&mut visual, |preferences, _window, cx| {
                preferences.set_dropdown_open(PreferencesDropdown::Startup, false);
                cx.notify();
            })
            .unwrap();
        visual.update(|window, cx| window.draw(cx).clear());
        let startup_row_before = visual.debug_bounds("preferences-startup-row").unwrap();
        handle
            .update(&mut visual, |preferences, _window, cx| {
                preferences.set_dropdown_open(PreferencesDropdown::Startup, true);
                cx.notify();
            })
            .unwrap();
        visual.update(|window, cx| {
            assert_eq!(window.scale_factor(), 2.0);
            window.draw(cx).clear();
        });
        visual.run_until_parked();
        let page = visual.debug_bounds("preferences-page-scroll").unwrap();
        let selector = "preferences-startup-dropdown";
        let bounds = visual
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("missing {selector}"));
        let startup_row_after = visual.debug_bounds("preferences-startup-row").unwrap();
        assert_eq!(startup_row_after, startup_row_before);
        assert!(bounds.left() >= page.left(), "{selector} escaped left");
        assert!(bounds.right() <= page.right(), "{selector} escaped right");
        assert!(bounds.top() >= page.top(), "{selector} escaped top");
        assert!(
            bounds.bottom() <= page.bottom(),
            "{selector} escaped bottom"
        );
    }
}
