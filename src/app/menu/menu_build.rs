// @author kongweiguang

//! Native menu construction and installation.

use super::*;

/// 行操作沿既有焦点分发，菜单与快捷键共享动作，避免维护第二套编辑语义。
pub(super) fn build_menus(
    _theme_manager: &ThemeManager,
    i18n_manager: &I18nManager,
    recent_files: &[PathBuf],
) -> Vec<Menu> {
    let strings = i18n_manager.strings().clone();
    let chinese = i18n_manager.current_language_id().starts_with("zh");
    let folding_label = if chinese { "折叠" } else { "Folding" };
    let format_label = if chinese { "格式化" } else { "Format" };

    let recent_items = if recent_files.is_empty() {
        vec![MenuItem::action(
            strings.menu_no_recent_files.clone(),
            NoRecentFiles,
        )]
    } else {
        recent_files
            .iter()
            .map(|path| {
                // into_owned on a Cow<str> reuses the Cow::Owned variant
                // (no copy) when the OS string is valid UTF-8 — the common
                // case — and only allocates for the lossy fallback. The
                // previous .to_string_lossy().to_string() always allocated.
                let label = path.to_string_lossy().into_owned();
                MenuItem::action(label.clone(), OpenRecentFile { path: label })
            })
            .collect()
    };

    #[cfg(target_os = "macos")]
    let initial_menus = {
        // On macOS, the first menu is the app menu (macOS overrides its title
        // with the app name). File operations go in a separate "File" menu to
        // match standard macOS conventions.
        vec![
            Menu {
                name: "Gmark".into(),
                items: vec![
                    MenuItem::action(strings.menu_preferences.clone(), OpenPreferences),
                    MenuItem::separator(),
                    MenuItem::action(strings.menu_quit.clone(), QuitApplication),
                ],
            },
            Menu {
                name: strings.menu_file.into(),
                items: vec![
                    MenuItem::action(strings.menu_new_tab.clone(), NewTab),
                    MenuItem::action(strings.menu_new_window.clone(), NewWindow),
                    MenuItem::action(strings.menu_reopen_closed_tab.clone(), ReopenClosedTab),
                    MenuItem::action(strings.menu_close_tab.clone(), CloseTab),
                    MenuItem::action(strings.menu_close_window.clone(), CloseWindow),
                    MenuItem::action(strings.menu_open_file.clone(), OpenFile),
                    MenuItem::action(strings.menu_open_safe_source.clone(), OpenSafeSource),
                    MenuItem::action(strings.menu_open_folder.clone(), OpenFolder),
                    MenuItem::submenu(Menu {
                        name: strings.menu_open_recent_file.clone().into(),
                        items: recent_items,
                    }),
                    MenuItem::separator(),
                    MenuItem::action(strings.menu_save.clone(), SaveDocument),
                    MenuItem::action(strings.menu_save_as.clone(), SaveDocumentAs),
                ],
            },
        ]
    };

    #[cfg(not(target_os = "macos"))]
    let initial_menus = {
        vec![
            // 客户端标题栏把这个应用菜单渲染为图标；名称只作平台菜单契约，不显示在界面上。
            Menu {
                name: "Gmark".into(),
                items: vec![
                    MenuItem::action(strings.menu_preferences.clone(), OpenPreferences),
                    MenuItem::separator(),
                    MenuItem::action(strings.menu_check_updates.clone(), CheckForUpdates),
                    MenuItem::separator(),
                    MenuItem::action(strings.menu_about.clone(), ShowAbout),
                    MenuItem::action(strings.menu_quit.clone(), QuitApplication),
                ],
            },
            Menu {
                name: strings.menu_file.into(),
                items: vec![
                    MenuItem::action(strings.menu_new_tab.clone(), NewTab),
                    MenuItem::action(strings.menu_new_window.clone(), NewWindow),
                    MenuItem::action(strings.menu_reopen_closed_tab.clone(), ReopenClosedTab),
                    MenuItem::action(strings.menu_close_tab.clone(), CloseTab),
                    MenuItem::action(strings.menu_close_window.clone(), CloseWindow),
                    MenuItem::action(strings.menu_open_file.clone(), OpenFile),
                    MenuItem::action(strings.menu_open_safe_source.clone(), OpenSafeSource),
                    MenuItem::action(strings.menu_open_folder.clone(), OpenFolder),
                    MenuItem::submenu(Menu {
                        name: strings.menu_open_recent_file.clone().into(),
                        items: recent_items,
                    }),
                    MenuItem::separator(),
                    MenuItem::action(strings.menu_save.clone(), SaveDocument),
                    MenuItem::action(strings.menu_save_as.clone(), SaveDocumentAs),
                ],
            },
        ]
    };

    #[cfg(target_os = "macos")]
    let help_items = {
        // Show different menu item depending on whether CLI is already
        // installed pointing to the current app.  Only portable
        // installations (drag-installed .app bundles) need this —
        // pkg-installed apps manage the symlink via postinstall.
        let cli_installed = is_cli_symlink_current_app();
        let mut items = vec![
            MenuItem::action(strings.menu_check_updates.clone(), CheckForUpdates),
            MenuItem::separator(),
        ];
        if cli_installed {
            items.push(MenuItem::action(
                SharedString::new(strings.menu_uninstall_cli_tool.as_str()),
                UninstallCliTool,
            ));
        } else {
            items.push(MenuItem::action(
                SharedString::new(strings.menu_install_cli_tool.as_str()),
                InstallCliTool,
            ));
        }
        items.push(MenuItem::separator());
        items.push(MenuItem::action(
            strings.menu_open_crash_reports.clone(),
            OpenCrashReports,
        ));
        items.push(MenuItem::action(
            strings.menu_privacy_policy.clone(),
            OpenPrivacyPolicy,
        ));
        items.push(MenuItem::separator());
        items.push(MenuItem::action(strings.menu_about.clone(), ShowAbout));
        items
    };
    #[cfg(not(target_os = "macos"))]
    let help_items = vec![
        MenuItem::action(strings.menu_open_crash_reports.clone(), OpenCrashReports),
        MenuItem::action(strings.menu_privacy_policy.clone(), OpenPrivacyPolicy),
    ];

    let mut menus = initial_menus;
    menus.extend([
        Menu {
            name: strings.menu_edit.clone().into(),
            items: vec![
                MenuItem::action(strings.preferences_shortcut_undo.clone(), Undo),
                MenuItem::action(strings.preferences_shortcut_redo.clone(), Redo),
                MenuItem::separator(),
                MenuItem::action(strings.preferences_shortcut_cut.clone(), Cut),
                MenuItem::action(strings.preferences_shortcut_copy.clone(), Copy),
                MenuItem::action(strings.preferences_shortcut_paste.clone(), Paste),
                MenuItem::action(
                    strings.preferences_shortcut_paste_as_plain_text.clone(),
                    PasteAsPlainText,
                ),
                MenuItem::action(strings.preferences_shortcut_select_all.clone(), SelectAll),
                MenuItem::submenu(Menu {
                    name: if chinese { "行操作" } else { "Lines" }.into(),
                    items: vec![
                        MenuItem::action(
                            strings.preferences_shortcut_duplicate_line.clone(),
                            DuplicateLine,
                        ),
                        MenuItem::action(
                            strings.preferences_shortcut_delete_line.clone(),
                            DeleteLine,
                        ),
                        MenuItem::action(
                            strings.preferences_shortcut_move_line_up.clone(),
                            MoveLineUp,
                        ),
                        MenuItem::action(
                            strings.preferences_shortcut_move_line_down.clone(),
                            MoveLineDown,
                        ),
                    ],
                }),
                MenuItem::separator(),
                MenuItem::action(
                    strings.preferences_shortcut_find_in_document.clone(),
                    FindInDocument,
                ),
                MenuItem::action(
                    strings.preferences_shortcut_replace_in_document.clone(),
                    ReplaceInDocument,
                ),
                MenuItem::separator(),
                MenuItem::action(strings.preferences_shortcut_find_next.clone(), FindNext),
                MenuItem::action(
                    strings.preferences_shortcut_find_previous.clone(),
                    FindPrevious,
                ),
                MenuItem::separator(),
                MenuItem::submenu(Menu {
                    name: folding_label.into(),
                    items: vec![
                        MenuItem::action(
                            if chinese {
                                "折叠当前区域"
                            } else {
                                "Collapse Fold"
                            },
                            CollapseFold,
                        ),
                        MenuItem::action(
                            if chinese {
                                "展开当前区域"
                            } else {
                                "Expand Fold"
                            },
                            ExpandFold,
                        ),
                        MenuItem::action(
                            if chinese {
                                "全部折叠"
                            } else {
                                "Collapse All"
                            },
                            CollapseAllFolds,
                        ),
                        MenuItem::action(
                            if chinese {
                                "全部展开"
                            } else {
                                "Expand All"
                            },
                            ExpandAllFolds,
                        ),
                    ],
                }),
                MenuItem::submenu(Menu {
                    name: format_label.into(),
                    items: vec![
                        MenuItem::action(
                            if chinese {
                                "格式化文档"
                            } else {
                                "Format Document"
                            },
                            FormatDocument,
                        ),
                        MenuItem::action(
                            if chinese {
                                "格式化选区"
                            } else {
                                "Format Selection"
                            },
                            FormatSelection,
                        ),
                        MenuItem::action(
                            if chinese {
                                "取消格式化"
                            } else {
                                "Cancel Formatting"
                            },
                            CancelFormatting,
                        ),
                    ],
                }),
            ],
        },
        Menu {
            name: strings.menu_view.into(),
            items: vec![
                MenuItem::action(strings.menu_toggle_workspace.clone(), ToggleWorkspace),
                MenuItem::action(
                    strings.menu_toggle_document_sidebar.clone(),
                    ToggleDocumentSidebar,
                ),
                MenuItem::separator(),
                MenuItem::action(strings.menu_toggle_focus_mode.clone(), ToggleFocusMode),
                MenuItem::action(
                    strings.menu_toggle_typewriter_mode.clone(),
                    ToggleTypewriterMode,
                ),
            ],
        },
        Menu {
            name: strings.menu_help.into(),
            items: help_items,
        },
    ]);
    menus
}

pub(crate) fn install_menus(cx: &mut App) {
    // 测试和窗口级重装菜单可能早于应用启动钩子；菜单快照必须能独立初始化。
    if cx.try_global::<AppMenuState>().is_none() {
        cx.set_global(AppMenuState::default());
    }
    let recent_files = cx.global::<AppMenuState>().recent_files.clone();
    install_menus_with_recent_files(cx, recent_files);
}

/// 用后台读取完成的历史快照重建菜单，避免菜单动作再次同步访问配置目录。
pub(crate) fn install_menus_with_recent_files(cx: &mut App, recent_files: Vec<PathBuf>) {
    if cx.try_global::<AppMenuState>().is_none() {
        cx.set_global(AppMenuState::default());
    }
    cx.global_mut::<AppMenuState>().recent_files = recent_files.clone();
    #[cfg(target_os = "macos")]
    let active_document_menu = cx.active_window().and_then(|window| {
        let editor = window.downcast::<Editor>()?;
        editor
            .update(cx, |editor, _window, cx| {
                Some((
                    editor.build_document_menu(cx),
                    editor.build_document_menu(cx),
                ))
            })
            .ok()
            .flatten()
    });
    // 原因: 仅 macOS 会插入动态文档菜单，其他平台保留同一构造顺序时变量不会被修改；移除条件: 平台菜单组装收敛为无条件的共享插入步骤。
    #[allow(unused_mut)]
    let mut owned = build_menus(
        cx.global::<ThemeManager>(),
        cx.global::<I18nManager>(),
        &recent_files,
    )
    .into_iter()
    .map(Menu::owned)
    .collect::<Vec<_>>();
    // 原因: 仅 macOS 会插入动态文档菜单，其他平台保留同一构造顺序时变量不会被修改；移除条件: 平台菜单组装收敛为无条件的共享插入步骤。
    #[allow(unused_mut)]
    let mut menus = build_menus(
        cx.global::<ThemeManager>(),
        cx.global::<I18nManager>(),
        &recent_files,
    );
    #[cfg(target_os = "macos")]
    if let Some((owned_dynamic, dynamic)) = active_document_menu {
        let owned_index = owned
            .iter()
            .position(|menu| matches!(menu.name.as_ref(), "Help" | "帮助"))
            .unwrap_or(owned.len());
        owned.insert(owned_index, owned_dynamic.owned());
        let menu_index = menus
            .iter()
            .position(|menu| matches!(menu.name.as_ref(), "Help" | "帮助"))
            .unwrap_or(menus.len());
        menus.insert(menu_index, dynamic);
    }
    cx.global_mut::<AppMenuState>().in_window_menus = owned;
    cx.set_menus(menus);
}
