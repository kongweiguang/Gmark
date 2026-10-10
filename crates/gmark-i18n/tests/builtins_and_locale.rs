// @author kongweiguang

use gmark_i18n::{
    BUILTIN_LANGUAGE_EN_US_ID, BUILTIN_LANGUAGE_ZH_CN_ID, DEFAULT_LANGUAGE_ID, I18nCatalog,
    LanguageSelection, language_id_for_locale_preferences, normalize_locale,
};
use std::sync::Arc;

/// 固定完整语言键集合及关键文案；新增导出阶段与恢复提示时，中英文必须同步扩展。
#[test]
fn builtins_preserve_the_complete_ui_key_set() {
    let english = I18nCatalog::new_with_language_id(BUILTIN_LANGUAGE_EN_US_ID).strings_clone();
    let chinese = I18nCatalog::new_with_language_id(BUILTIN_LANGUAGE_ZH_CN_ID).strings_clone();

    assert_eq!(english.scalars().len(), 450);
    assert_eq!(chinese.scalars().len(), 450);
    assert_eq!(
        english.scalars().keys().collect::<Vec<_>>(),
        chinese.scalars().keys().collect::<Vec<_>>()
    );
    assert_eq!(
        english.groups().keys().collect::<Vec<_>>(),
        chinese.groups().keys().collect::<Vec<_>>()
    );
    assert_eq!(english.groups()["slash_commands"].len(), 71);
    assert_eq!(english.groups()["large_document"].len(), 112);
    assert_eq!(
        english.groups()["slash_commands"]
            .keys()
            .collect::<Vec<_>>(),
        chinese.groups()["slash_commands"]
            .keys()
            .collect::<Vec<_>>()
    );
    assert_eq!(
        english.groups()["large_document"]
            .keys()
            .collect::<Vec<_>>(),
        chinese.groups()["large_document"]
            .keys()
            .collect::<Vec<_>>()
    );

    assert_eq!(english.get("menu_file"), Some("File"));
    assert_eq!(chinese.get("menu_file"), Some("文件"));
    assert_eq!(english.get("export_saving"), Some("Saving…"));
    assert_eq!(chinese.get("export_saving"), Some("正在保存…"));
    assert_eq!(english.get("export_cancelled"), Some("Export cancelled"));
    assert_eq!(chinese.get("export_cancelled"), Some("导出已取消"));
    assert_eq!(english.get("new_document_csv"), Some("CSV Document"));
    assert_eq!(chinese.get("new_document_csv"), Some("CSV 文档"));
    assert_eq!(
        english.get("large_document.recovered_structured_paused"),
        Some("Structured view is paused until recovered edits are saved")
    );
    assert_eq!(
        chinese.get("large_document.recovered_structured_paused"),
        Some("恢复的编辑保存前，结构化视图已暂停")
    );
    assert_eq!(english.get("slash_commands.table"), Some("Table"));
    assert_eq!(chinese.get("slash_commands.table"), Some("表格"));
    assert_eq!(
        english.get("preferences_shortcut_split_right"),
        Some("Split Right")
    );
    assert_eq!(
        chinese.get("preferences_shortcut_split_right"),
        Some("向右拆分")
    );
    assert_eq!(
        english.get("preferences_shortcut_split_down"),
        Some("Split Down")
    );
    assert_eq!(
        chinese.get("preferences_shortcut_split_down"),
        Some("向下拆分")
    );
    assert_eq!(
        english.get("preferences_shortcut_close_pane"),
        Some("Close Pane")
    );
    assert_eq!(
        chinese.get("preferences_shortcut_close_pane"),
        Some("关闭窗格")
    );
    assert_eq!(
        english.get("preferences_shortcut_focus_pane_left"),
        Some("Focus Pane Left")
    );
    assert_eq!(
        chinese.get("preferences_shortcut_focus_pane_left"),
        Some("聚焦左侧窗格")
    );
    assert_eq!(
        english.get("preferences_shortcut_focus_pane_right"),
        Some("Focus Pane Right")
    );
    assert_eq!(
        chinese.get("preferences_shortcut_focus_pane_right"),
        Some("聚焦右侧窗格")
    );
    assert_eq!(
        english.get("preferences_shortcut_focus_pane_up"),
        Some("Focus Pane Up")
    );
    assert_eq!(
        chinese.get("preferences_shortcut_focus_pane_up"),
        Some("聚焦上方窗格")
    );
    assert_eq!(
        english.get("preferences_shortcut_focus_pane_down"),
        Some("Focus Pane Down")
    );
    assert_eq!(
        chinese.get("preferences_shortcut_focus_pane_down"),
        Some("聚焦下方窗格")
    );
    assert_eq!(
        english.get("preferences_shortcut_move_tab_to_pane_left"),
        Some("Move Tab to Pane Left")
    );
    assert_eq!(
        chinese.get("preferences_shortcut_move_tab_to_pane_left"),
        Some("将标签页移至左侧窗格")
    );
    assert_eq!(
        english.get("preferences_shortcut_move_tab_to_pane_right"),
        Some("Move Tab to Pane Right")
    );
    assert_eq!(
        chinese.get("preferences_shortcut_move_tab_to_pane_right"),
        Some("将标签页移至右侧窗格")
    );
    assert_eq!(
        english.get("preferences_shortcut_move_tab_to_pane_up"),
        Some("Move Tab to Pane Up")
    );
    assert_eq!(
        chinese.get("preferences_shortcut_move_tab_to_pane_up"),
        Some("将标签页移至上方窗格")
    );
    assert_eq!(
        english.get("preferences_shortcut_move_tab_to_pane_down"),
        Some("Move Tab to Pane Down")
    );
    assert_eq!(
        chinese.get("preferences_shortcut_move_tab_to_pane_down"),
        Some("将标签页移至下方窗格")
    );
    assert_eq!(
        english.get("preferences_shortcut_balance_panes"),
        Some("Balance Panes")
    );
    assert_eq!(
        chinese.get("preferences_shortcut_balance_panes"),
        Some("平衡窗格")
    );
    assert_eq!(
        english.get("status_bar_mode_split"),
        Some("Source & Preview")
    );
    assert_eq!(chinese.get("status_bar_mode_split"), Some("源码与预览"));
    assert_eq!(
        english.get("pane_notice_duplicate_document_label"),
        Some("Duplicate Document")
    );
    assert_eq!(
        chinese.get("pane_notice_duplicate_document_label"),
        Some("文档已存在")
    );
    assert_eq!(
        english.get("pane_notice_duplicate_document_description"),
        Some("This document is already open in the target pane.")
    );
    assert_eq!(
        chinese.get("pane_notice_duplicate_document_description"),
        Some("目标窗格中已打开此文档。")
    );
    assert_eq!(
        english.get("pane_notice_pane_limit_label"),
        Some("Pane Limit Reached")
    );
    assert_eq!(
        chinese.get("pane_notice_pane_limit_label"),
        Some("已达到窗格上限")
    );
    assert_eq!(
        english.get("pane_notice_pane_limit_description"),
        Some("A workspace can contain at most 8 panes.")
    );
    assert_eq!(
        chinese.get("pane_notice_pane_limit_description"),
        Some("一个工作区最多包含 8 个窗格。")
    );
    assert_eq!(
        english.get("pane_notice_insufficient_space_label"),
        Some("Insufficient Space")
    );
    assert_eq!(
        chinese.get("pane_notice_insufficient_space_label"),
        Some("空间不足")
    );
    assert_eq!(
        english.get("pane_notice_insufficient_space_description"),
        Some("There is not enough space to create another pane.")
    );
    assert_eq!(
        chinese.get("pane_notice_insufficient_space_description"),
        Some("没有足够的空间创建新窗格。")
    );
    assert_eq!(
        english.get("status_bar_shared_document"),
        Some("Shared document")
    );
    assert_eq!(chinese.get("status_bar_shared_document"), Some("共享文档"));
    assert_eq!(
        english.get("status_bar_shared_views_template"),
        Some("{count} views")
    );
    assert_eq!(
        chinese.get("status_bar_shared_views_template"),
        Some("{count} 个视图")
    );

    let editing_shortcuts = [
        ("preferences_shortcut_delete_line", "Delete line", "删除行"),
        (
            "preferences_shortcut_duplicate_line",
            "Duplicate line",
            "复制行",
        ),
        (
            "preferences_shortcut_move_line_down",
            "Move line down",
            "下移行",
        ),
        (
            "preferences_shortcut_move_line_up",
            "Move line up",
            "上移行",
        ),
        (
            "preferences_shortcut_move_to_document_end",
            "Move caret to document end",
            "光标移至文档末尾",
        ),
        (
            "preferences_shortcut_move_to_document_start",
            "Move caret to document start",
            "光标移至文档开头",
        ),
        (
            "preferences_shortcut_select_down",
            "Extend selection down",
            "向下扩展选区",
        ),
        (
            "preferences_shortcut_select_page_down",
            "Extend selection one page down",
            "向下扩展一页选区",
        ),
        (
            "preferences_shortcut_select_page_up",
            "Extend selection one page up",
            "向上扩展一页选区",
        ),
        (
            "preferences_shortcut_select_to_document_end",
            "Select to document end",
            "扩展选区至文档末尾",
        ),
        (
            "preferences_shortcut_select_to_document_start",
            "Select to document start",
            "扩展选区至文档开头",
        ),
        (
            "preferences_shortcut_select_up",
            "Extend selection up",
            "向上扩展选区",
        ),
    ];
    for (key, expected_english, expected_chinese) in editing_shortcuts {
        assert_eq!(english.get(key), Some(expected_english));
        assert_eq!(chinese.get(key), Some(expected_chinese));
    }

    for value in english.scalars().values().chain(chinese.scalars().values()) {
        assert!(!value.trim().is_empty());
    }
}

#[test]
fn builtin_catalog_order_and_selection_match_legacy_behavior() {
    let mut catalog = I18nCatalog::default();
    assert_eq!(catalog.current_language_id(), DEFAULT_LANGUAGE_ID);
    assert_eq!(catalog.strings().get("menu_export"), Some("Export"));
    assert!(Arc::ptr_eq(&catalog.strings_arc(), &catalog.strings_arc()));
    assert_eq!(
        catalog
            .available_languages()
            .iter()
            .map(|entry| (entry.id.as_str(), entry.name.as_str()))
            .collect::<Vec<_>>(),
        vec![("zh-CN", "简体中文"), ("en-US", "English")]
    );

    assert_eq!(catalog.select_language("zh-CN"), LanguageSelection::Changed);
    assert_eq!(catalog.current_language_id(), "zh-CN");
    assert_eq!(catalog.strings().get("menu_export"), Some("导出"));
    assert_eq!(
        catalog.select_language("zh-CN"),
        LanguageSelection::Unchanged
    );
    assert_eq!(
        catalog.select_language("missing"),
        LanguageSelection::NotFound
    );
    assert!(!catalog.set_language_by_id("missing"));

    let fallback = I18nCatalog::new_with_language_id("missing");
    assert_eq!(fallback.current_language_id(), "en-US");
}

#[test]
fn locale_aliases_select_the_same_builtin_languages() {
    assert_eq!(normalize_locale(" zh_SG.UTF-8 "), Some("zh-SG".to_owned()));
    assert_eq!(
        normalize_locale("en_GB.UTF-8@calendar=gregorian"),
        Some("en-GB".to_owned())
    );
    assert_eq!(normalize_locale("!!!"), None);
    assert_eq!(normalize_locale(""), None);

    assert_eq!(language_id_for_locale_preferences(["zh-CN"]), "zh-CN");
    assert_eq!(language_id_for_locale_preferences(["zh-Hant-TW"]), "zh-CN");
    assert_eq!(language_id_for_locale_preferences(["zh_SG.UTF-8"]), "zh-CN");
    assert_eq!(language_id_for_locale_preferences(["en_GB.UTF-8"]), "en-US");
    assert_eq!(
        language_id_for_locale_preferences(["fr-FR", "zh-CN"]),
        "zh-CN"
    );
    assert_eq!(
        language_id_for_locale_preferences(Vec::<&str>::new()),
        "en-US"
    );
    assert_eq!(language_id_for_locale_preferences(["fr-FR"]), "en-US");
}
