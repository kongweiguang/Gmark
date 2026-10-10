// @author kongweiguang

//! Localised UI strings and runtime language selection.
//!
//! This module owns language packs, system-locale matching, and the global
//! manager used by menus and editor UI. Visual styling remains in `theme`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// All localisable UI strings for the editor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct I18nStrings {
    /// Marker prepended to the window title when the document is dirty.
    pub dirty_title_marker: String,
    /// Title of the unsaved-changes dialog.
    pub unsaved_changes_title: String,
    /// Body message of the unsaved-changes dialog.
    pub unsaved_changes_message: String,
    /// Label for the "save and close" button.
    pub unsaved_changes_save_and_close: String,
    /// Label for the "discard and close" button.
    pub unsaved_changes_discard_and_close: String,
    /// Label for the "keep editing" button.
    pub unsaved_changes_cancel: String,
    /// Title of the dropped-file replacement dialog.
    pub drop_replace_title: String,
    /// Body message of the dropped-file replacement dialog.
    pub drop_replace_message: String,
    /// Label for saving before replacing the current document.
    pub drop_replace_save_and_replace: String,
    /// Label for replacing the current document without saving.
    pub drop_replace_discard_and_replace: String,
    /// Label for cancelling a dropped-file replacement.
    pub drop_replace_cancel: String,
    /// Prompt detail shown when no supported Markdown file was dropped.
    pub drop_no_markdown_file_message: String,
    /// Label for dismissing simple informational dialogs.
    pub info_dialog_ok: String,
    /// Generic tooltip label for closing a transient panel.
    pub ui_close: String,
    /// Tooltip label for clearing a search field.
    pub ui_clear_search: String,
    /// Title of the placeholder update-check dialog.
    pub help_check_updates_title: String,
    /// Body text shown while an update check is running.
    pub help_check_updates_message: String,
    /// Title shown when a newer version is available.
    pub update_available_title: String,
    /// Message template for newer-version prompts. Supports `{current}` and `{latest}`.
    pub update_available_message_template: String,
    /// Title shown when the running app is already current.
    pub update_up_to_date_title: String,
    /// Message template for up-to-date prompts. Supports `{current}` and `{latest}`.
    pub update_up_to_date_message_template: String,
    /// Title shown when an update check fails.
    pub update_failed_title: String,
    /// Message template for update-check failures. Supports `{error}`.
    pub update_failed_message_template: String,
    /// Button label for opening the GitHub Releases page.
    pub update_open_release: String,
    /// Button label for dismissing an available-update prompt.
    pub update_later: String,
    /// Title of the About dialog.
    pub help_about_title: String,
    /// Supplemental About dialog text shown below the app name and version.
    pub help_about_message: String,
    /// Label for the project repository link in the About dialog.
    pub help_about_github_label: String,
    /// Star request shown in the About dialog.
    pub help_about_star_message: String,
    /// Top-level File menu label.
    pub menu_file: String,
    pub menu_edit: String,
    /// Top-level source format menu label.
    pub menu_format: String,
    /// Export submenu label.
    pub menu_export: String,
    /// Language submenu label.
    pub menu_language: String,
    /// Top-level View menu label.
    pub menu_view: String,
    /// Top-level Workspace menu label.
    pub menu_workspace: String,
    /// Top-level Help menu label.
    pub menu_help: String,
    /// Language menu item for importing a custom language pack.
    pub menu_add_language_config: String,
    /// File menu item for opening a new window.
    pub menu_new_window: String,
    /// File menu item for creating an untitled document tab.
    pub menu_new_tab: String,
    pub new_document_untyped: String,
    pub new_document_markdown: String,
    pub new_document_json: String,
    pub new_document_csv: String,
    pub json_graph_search_placeholder: String,
    pub json_graph_search_previous: String,
    pub json_graph_search_next: String,
    pub json_graph_preview_unavailable: String,
    pub json_graph_generating: String,
    pub json_graph_generating_detail: String,
    pub json_graph_focus_subtree: String,
    pub json_graph_reset_root: String,
    pub json_graph_stale: String,
    pub json_graph_source_changed: String,
    pub json_graph_truncated: String,
    pub json_graph_show_more_template: String,
    pub json_graph_details_title: String,
    pub json_graph_content: String,
    pub json_graph_path: String,
    pub json_graph_locate_byte_template: String,
    pub json_graph_fit: String,
    pub json_graph_zoom_out: String,
    pub json_graph_zoom_in: String,
    pub json_graph_locate_source: String,
    pub json_graph_copy_path: String,
    pub json_graph_copy_content: String,
    pub json_graph_collapse: String,
    pub json_graph_expand: String,
    pub json_graph_edit_value: String,
    pub json_graph_edit_help: String,
    pub json_graph_edit_invalid: String,
    pub json_graph_edit_too_large_template: String,
    pub json_graph_live_edit: String,
    pub json_graph_live_edit_hint: String,
    pub json_graph_reload_value: String,
    pub json_graph_edit_source: String,
    /// Stable large-document UI keys mapped to localized labels and status messages.
    pub large_document: BTreeMap<String, String>,
    /// File menu item for closing the current window.
    pub menu_close_window: String,
    pub menu_close_tab: String,
    pub menu_reopen_closed_tab: String,
    pub menu_pin_tab: String,
    pub menu_unpin_tab: String,
    pub menu_close_other_tabs: String,
    /// File menu item for opening Markdown files.
    pub menu_open_file: String,
    pub menu_open_safe_source: String,
    pub menu_open_folder: String,
    /// File menu item for opening a recent file submenu.
    pub menu_open_recent_file: String,
    /// File menu item for opening app preferences.
    pub menu_preferences: String,
    /// Placeholder item shown when no recent files are recorded.
    pub menu_no_recent_files: String,
    /// File menu item for saving the current document.
    pub menu_save: String,
    /// File menu item for saving the current document to a new path.
    pub menu_save_as: String,
    /// File menu item for quitting the app.
    pub menu_quit: String,
    /// Export menu item for writing an HTML document.
    pub menu_export_html: String,
    /// Export menu item for writing a PNG image.
    pub menu_export_image: String,
    /// Export menu item for writing a PDF document.
    pub menu_export_pdf: String,
    /// Help menu item for checking updates.
    pub menu_check_updates: String,
    /// Help menu item for opening the local crash-report directory.
    pub menu_open_crash_reports: String,
    /// Help menu item for opening the published privacy policy.
    pub menu_privacy_policy: String,
    /// Help menu item for showing About information.
    pub menu_about: String,
    /// Help menu item for installing the CLI tool (symlink to /usr/local/bin).
    pub menu_install_cli_tool: String,
    /// Help menu item for uninstalling the CLI tool.
    pub menu_uninstall_cli_tool: String,
    /// Workspace menu item for opening or closing the workspace drawer.
    pub menu_toggle_workspace: String,
    pub menu_toggle_focus_mode: String,
    pub menu_toggle_typewriter_mode: String,
    /// Format submenu label for line-ending normalization.
    pub menu_line_endings: String,
    /// Line-ending menu labels.
    pub menu_line_ending_lf: String,
    pub menu_line_ending_crlf: String,
    pub menu_line_ending_cr: String,
    /// Native file-dialog prompt for opening Markdown files.
    pub open_markdown_files_prompt: String,
    pub open_workspace_folder_prompt: String,
    /// Native file-dialog prompt for importing a language pack.
    pub add_language_config_prompt: String,
    /// Title of the open-file failure prompt.
    pub open_failed_title: String,
    /// Title shown when a recent file path no longer exists.
    pub recent_file_missing_title: String,
    /// Message template for missing recent files. Supports `{path}`.
    pub recent_file_missing_message_template: String,
    /// Title of the save failure prompt.
    pub save_failed_title: String,
    /// Title shown when the file changed outside gmark.
    pub external_change_title: String,
    /// Non-UTF-8 read-only open and explicit conversion dialog.
    pub encoding_conversion_title: String,
    pub encoding_conversion_message_template: String,
    pub encoding_convert_utf8: String,
    pub encoding_keep_read_only: String,
    /// Message asking the user to preserve recovery content with Save As.
    pub external_change_save_as_message: String,
    pub external_change_first_difference_template: String,
    pub external_change_metadata_only: String,
    pub external_change_local_label: String,
    pub external_change_disk_label: String,
    pub external_change_reload: String,
    pub external_change_overwrite: String,
    pub external_change_save_as: String,
    pub external_change_cancel: String,
    /// Status-bar label for a journal-restored session.
    pub recovery_status: String,
    /// Status-bar label when the restored base changed externally.
    pub recovery_conflict_status: String,
    /// Title of the export failure prompt.
    pub export_failed_title: String,
    pub export_in_progress: String,
    pub export_cancelling: String,
    pub export_cancel: String,
    pub export_saving: String,
    pub export_completed_template: String,
    pub export_completed_title: String,
    pub export_replace_title: String,
    pub export_replace_message_template: String,
    pub export_replace: String,
    pub export_cancelled: String,
    pub export_retry: String,
    pub export_failed_missing_browser: String,
    pub export_failed_browser: String,
    pub export_failed_timeout: String,
    pub export_failed_write: String,
    pub export_failed_generic_template: String,
    /// Title of the image-paste failure prompt.
    pub image_paste_failed_title: String,
    /// Title of the custom configuration import failure prompt.
    pub config_import_failed_title: String,
    /// Preferences window title.
    pub preferences_window_title: String,
    pub preferences_search_placeholder: String,
    pub preferences_search_results: String,
    pub preferences_search_no_results: String,
    /// File preferences navigation label.
    pub preferences_nav_file: String,
    pub preferences_nav_editor: String,
    /// Theme preferences navigation label.
    pub preferences_nav_theme: String,
    /// Image preferences navigation label.
    pub preferences_nav_image: String,
    /// Shortcut preferences navigation label.
    pub preferences_nav_shortcuts: String,
    /// Startup option field label.
    pub preferences_startup_option: String,
    /// Startup option for creating a new Markdown document.
    pub preferences_startup_new_file: String,
    /// Startup option for opening the last opened Markdown document.
    pub preferences_startup_last_opened_file: String,
    pub preferences_auto_save_option: String,
    pub preferences_auto_save_off: String,
    pub preferences_auto_save_after_delay: String,
    pub preferences_document_loading: String,
    pub preferences_document_max_resident_mib: String,
    pub preferences_document_loading_invalid: String,
    pub preferences_document_loading_next_open: String,
    pub preferences_spell_check: String,
    pub preferences_auto_pair_brackets: String,
    pub preferences_auto_pair_markdown: String,
    pub preferences_editor_font_size: String,
    pub preferences_editor_line_height: String,
    pub preferences_editor_content_width: String,
    pub preferences_editor_font_family: String,
    pub preferences_editor_font_system_placeholder: String,
    pub preferences_show_tab_bar_actions: String,
    /// Theme palette preference field label.
    pub preferences_palette: String,
    /// Theme appearance preference field label.
    pub preferences_theme_appearance: String,
    /// Fixed dark appearance option.
    pub preferences_theme_dark: String,
    /// Fixed light appearance option.
    pub preferences_theme_light: String,
    /// Theme option that follows the operating-system appearance.
    pub preferences_follow_system_theme: String,
    pub preferences_accessibility_title: String,
    pub preferences_reduced_motion: String,
    pub preferences_reduced_motion_hint: String,
    pub preferences_reduced_transparency: String,
    pub preferences_reduced_transparency_hint: String,
    pub preferences_high_contrast: String,
    pub preferences_high_contrast_hint: String,
    pub preferences_accessibility_system: String,
    pub preferences_accessibility_enabled: String,
    pub preferences_accessibility_disabled: String,
    /// Image paste behavior field label.
    pub preferences_image_insert_behavior: String,
    pub preferences_image_paste_none: String,
    pub preferences_image_paste_copy_to_document_folder: String,
    pub preferences_image_paste_copy_to_assets_folder: String,
    pub preferences_image_paste_copy_to_named_assets_folder: String,
    /// Save button label in the preferences window.
    pub preferences_save: String,
    /// Cancel button label in the preferences window.
    pub preferences_cancel: String,
    /// Title shown when preferences cannot be saved.
    pub preferences_save_failed_title: String,
    pub preferences_shortcuts_group_file: String,
    pub preferences_shortcuts_group_edit: String,
    pub preferences_shortcuts_group_navigation: String,
    pub preferences_shortcuts_group_formatting: String,
    pub preferences_shortcuts_group_block: String,
    pub preferences_shortcuts_group_other: String,
    pub preferences_shortcut_record: String,
    pub preferences_shortcut_reset: String,
    pub preferences_shortcut_recording: String,
    pub preferences_shortcut_conflict_template: String,
    pub preferences_shortcut_invalid_template: String,
    pub preferences_shortcut_newline: String,
    pub preferences_shortcut_delete_back: String,
    pub preferences_shortcut_delete: String,
    pub preferences_shortcut_word_delete_back: String,
    pub preferences_shortcut_word_delete_forward: String,
    pub preferences_shortcut_focus_prev: String,
    pub preferences_shortcut_focus_next: String,
    pub preferences_shortcut_move_left: String,
    pub preferences_shortcut_move_right: String,
    pub preferences_shortcut_word_move_left: String,
    pub preferences_shortcut_word_move_right: String,
    pub preferences_shortcut_home: String,
    pub preferences_shortcut_end: String,
    pub preferences_shortcut_move_to_document_start: String,
    pub preferences_shortcut_move_to_document_end: String,
    pub preferences_shortcut_block_up: String,
    pub preferences_shortcut_block_down: String,
    pub preferences_shortcut_page_up: String,
    pub preferences_shortcut_page_down: String,
    pub preferences_shortcut_jump_to_top: String,
    pub preferences_shortcut_jump_to_bottom: String,
    pub preferences_shortcut_select_left: String,
    pub preferences_shortcut_select_right: String,
    pub preferences_shortcut_select_up: String,
    pub preferences_shortcut_select_down: String,
    pub preferences_shortcut_select_page_up: String,
    pub preferences_shortcut_select_page_down: String,
    pub preferences_shortcut_word_select_left: String,
    pub preferences_shortcut_word_select_right: String,
    pub preferences_shortcut_select_home: String,
    pub preferences_shortcut_select_end: String,
    pub preferences_shortcut_select_to_document_start: String,
    pub preferences_shortcut_select_to_document_end: String,
    pub preferences_shortcut_select_all: String,
    pub preferences_shortcut_duplicate_line: String,
    pub preferences_shortcut_delete_line: String,
    pub preferences_shortcut_move_line_up: String,
    pub preferences_shortcut_move_line_down: String,
    pub preferences_shortcut_copy: String,
    pub preferences_shortcut_copy_as_markdown: String,
    pub preferences_shortcut_cut: String,
    pub preferences_shortcut_paste: String,
    pub preferences_shortcut_paste_as_plain_text: String,
    pub preferences_shortcut_undo: String,
    pub preferences_shortcut_redo: String,
    pub preferences_shortcut_bold_selection: String,
    pub preferences_shortcut_italic_selection: String,
    pub preferences_shortcut_underline_selection: String,
    pub preferences_shortcut_strikethrough_selection: String,
    pub preferences_shortcut_code_selection: String,
    pub preferences_shortcut_link_selection: String,
    pub preferences_shortcut_indent_block: String,
    pub preferences_shortcut_outdent_block: String,
    pub preferences_shortcut_exit_code_block: String,
    pub preferences_shortcut_save_document: String,
    pub preferences_shortcut_save_document_as: String,
    pub preferences_shortcut_new_window: String,
    pub preferences_shortcut_new_tab: String,
    pub preferences_shortcut_open_file: String,
    pub preferences_shortcut_open_folder: String,
    pub preferences_shortcut_quit_application: String,
    pub preferences_shortcut_close_window: String,
    pub preferences_shortcut_close_tab: String,
    pub preferences_shortcut_reopen_closed_tab: String,
    pub preferences_shortcut_previous_tab: String,
    pub preferences_shortcut_next_tab: String,
    pub preferences_shortcut_dismiss_transient_ui: String,
    pub preferences_shortcut_toggle_view_mode: String,
    pub preferences_shortcut_toggle_workspace: String,
    pub menu_toggle_document_sidebar: String,
    pub preferences_shortcut_split_right: String,
    pub preferences_shortcut_split_down: String,
    pub preferences_shortcut_close_pane: String,
    pub preferences_shortcut_focus_pane_left: String,
    pub preferences_shortcut_focus_pane_right: String,
    pub preferences_shortcut_focus_pane_up: String,
    pub preferences_shortcut_focus_pane_down: String,
    pub preferences_shortcut_move_tab_to_pane_left: String,
    pub preferences_shortcut_move_tab_to_pane_right: String,
    pub preferences_shortcut_move_tab_to_pane_up: String,
    pub preferences_shortcut_move_tab_to_pane_down: String,
    pub preferences_shortcut_balance_panes: String,
    pub preferences_shortcut_quick_open: String,
    pub preferences_shortcut_command_palette: String,
    pub preferences_shortcut_go_to_line: String,
    pub preferences_shortcut_find_in_document: String,
    pub preferences_shortcut_replace_in_document: String,
    pub preferences_shortcut_find_next: String,
    pub preferences_shortcut_find_previous: String,
    pub preferences_shortcut_toggle_focus_mode: String,
    pub preferences_shortcut_toggle_typewriter_mode: String,
    /// Workspace drawer Files tab.
    pub workspace_tab_files: String,
    /// Workspace drawer Outline tab.
    pub workspace_tab_outline: String,
    /// Workspace drawer Search tab and search states.
    pub workspace_tab_search: String,
    /// Document navigator labels and compact fallback states.
    pub document_sidebar_title: String,
    pub document_sidebar_structure: String,
    pub document_sidebar_columns: String,
    pub document_sidebar_info: String,
    pub document_sidebar_text: String,
    pub document_sidebar_format: String,
    pub document_sidebar_size: String,
    pub document_sidebar_lines: String,
    pub document_sidebar_encoding: String,
    pub document_sidebar_line_endings: String,
    pub document_sidebar_loading: String,
    pub document_sidebar_empty: String,
    pub document_sidebar_rows_template: String,
    pub document_sidebar_items_limit: String,
    pub document_sidebar_column_fallback: String,
    pub status_bar_document_sidebar: String,
    pub workspace_search_prompt: String,
    pub workspace_search_running: String,
    pub workspace_search_no_results: String,
    pub find_query_placeholder: String,
    pub find_replace_placeholder: String,
    pub find_match_count_template: String,
    pub find_no_results: String,
    pub find_case_sensitive: String,
    pub find_whole_word: String,
    pub find_regex: String,
    pub find_replace: String,
    pub find_replace_all: String,
    pub workspace_scanning_files: String,
    /// Title shown when no Markdown file path is available for workspace mode.
    pub workspace_no_file_title: String,
    /// Message shown when no Markdown file path is available for workspace mode.
    pub workspace_no_file_message: String,
    /// Message shown when a workspace directory has no visible Markdown files.
    pub workspace_empty_files: String,
    /// Message shown when the current document has no headings.
    pub workspace_empty_outline: String,
    pub workspace_outline_updating: String,
    /// Title shown when the workspace file tree cannot be scanned.
    pub workspace_scan_failed_title: String,
    pub file_open_failed_title: String,
    pub file_open_failed_message: String,
    pub file_open_with_system: String,
    pub file_reveal_in_manager: String,
    pub workspace_open: String,
    pub workspace_copy_path: String,
    pub workspace_copy_relative_path: String,
    pub workspace_refresh: String,
    pub workspace_delete: String,
    pub workspace_delete_title: String,
    pub workspace_delete_confirm: String,
    pub workspace_delete_message_template: String,
    pub workspace_delete_dirty_error: String,
    pub workspace_delete_completed_dirty_error: String,
    pub workspace_rename: String,
    pub workspace_move: String,
    pub workspace_new_file: String,
    pub workspace_new_folder: String,
    pub workspace_undo_file_operation: String,
    pub workspace_rename_title: String,
    pub workspace_move_title: String,
    pub workspace_new_file_title: String,
    pub workspace_new_folder_title: String,
    pub workspace_destination_label: String,
    pub workspace_review_operation: String,
    pub workspace_apply_operation: String,
    pub workspace_operation_affected_template: String,
    pub workspace_operation_busy: String,
    pub workspace_operation_dirty_error: String,
    pub workspace_rename_filename_only_error: String,
    pub workspace_invalid_path_error: String,
    pub quick_open_title: String,
    pub quick_open_prompt: String,
    pub quick_open_no_results: String,
    pub quick_open_scanning: String,
    pub command_palette_title: String,
    pub command_palette_prompt: String,
    pub command_palette_no_results: String,
    /// Stable slash-command item keys mapped to localized labels.
    pub slash_commands: BTreeMap<String, String>,
    /// Stable formula-palette UI and command keys mapped to localized labels.
    pub math_palette: BTreeMap<String, String>,
    /// Title of the link-opening confirmation prompt.
    pub open_link_title: String,
    /// Confirm button for the link-opening prompt.
    pub open_link_open: String,
    /// Cancel button for the link-opening prompt.
    pub open_link_cancel: String,
    /// Compact label shown when rendered mode can switch to source mode.
    pub view_mode_source: String,
    /// Hover label shown when rendered mode can switch to source mode.
    pub view_mode_switch_to_source: String,
    /// Compact label shown when source mode can switch to rendered mode.
    pub view_mode_rendered: String,
    /// Hover label shown when source mode can switch to rendered mode.
    pub view_mode_switch_to_rendered: String,
    /// Root context-menu insert label.
    pub context_menu_insert: String,
    /// Insert submenu item for tables.
    pub context_menu_table: String,
    /// Table-axis menu item for left-aligning a column.
    pub table_axis_align_column_left: String,
    /// Table-axis menu item for center-aligning a column.
    pub table_axis_align_column_center: String,
    /// Table-axis menu item for right-aligning a column.
    pub table_axis_align_column_right: String,
    /// Table-axis menu item for moving a column left.
    pub table_axis_move_column_left: String,
    /// Table-axis menu item for moving a column right.
    pub table_axis_move_column_right: String,
    /// Table-axis menu item for deleting a column.
    pub table_axis_delete_column: String,
    /// Table-axis menu item for moving a row up.
    pub table_axis_move_row_up: String,
    /// Table-axis menu item for moving a row down.
    pub table_axis_move_row_down: String,
    /// Table-axis menu item for deleting a row.
    pub table_axis_delete_row: String,
    /// Table header-row menu item that toggles header styling on the top row.
    pub table_header_row: String,
    /// Title of the table-insert dialog.
    pub table_insert_title: String,
    /// Body text of the table-insert dialog.
    pub table_insert_description: String,
    /// Label for table body rows in the table-insert dialog.
    pub table_insert_body_rows: String,
    /// Label for table columns in the table-insert dialog.
    pub table_insert_columns: String,
    /// Cancel button in the table-insert dialog.
    pub table_insert_cancel: String,
    /// Confirm button in the table-insert dialog.
    pub table_insert_confirm: String,
    /// Placeholder label for rendered images without alt text.
    pub image_placeholder: String,
    /// Loading label for rendered images without alt text.
    pub image_loading_without_alt: String,
    /// Loading label template for rendered images with alt text; `{alt}` is replaced.
    pub image_loading_with_alt_template: String,
    /// Placeholder shown in the code-block language input when no language is set.
    pub code_language_placeholder: String,
    pub code_language_menu: String,
    pub code_copy: String,
    pub code_copied: String,
    pub mermaid_copy_source: String,
    pub mermaid_copied: String,
    pub mermaid_export_svg: String,
    pub mermaid_expand: String,
    pub mermaid_read_only: String,
    pub mermaid_export_unavailable: String,
    pub selection_toolbar_bold: String,
    pub selection_toolbar_italic: String,
    pub selection_toolbar_strikethrough: String,
    pub selection_toolbar_inline_code: String,
    pub selection_toolbar_link: String,
    pub selection_toolbar_more: String,
    pub selection_toolbar_underline: String,
    pub image_resize: String,
    pub table_append_column: String,
    pub table_append_row: String,
    pub footnote_back_to_reference: String,
    /// Label for the sidebar/files toggle button in the status bar.
    pub status_bar_files: String,
    /// Label for source mode in the status bar mode switch.
    pub status_bar_mode_source: String,
    /// Label for rendered mode in the status bar mode switch.
    pub status_bar_mode_rendered: String,
    /// Label for read-only preview mode in the status bar mode switch.
    pub status_bar_mode_preview: String,
    /// Label for side-by-side source and preview mode.
    pub status_bar_mode_split: String,
    /// Short label shown when a document already exists in the target pane.
    pub pane_notice_duplicate_document_label: String,
    /// Explanation shown when a document already exists in the target pane.
    pub pane_notice_duplicate_document_description: String,
    /// Short label shown when the workspace has reached the eight-pane limit.
    pub pane_notice_pane_limit_label: String,
    /// Explanation shown when the workspace has reached the eight-pane limit.
    pub pane_notice_pane_limit_description: String,
    /// Short label shown when a pane operation cannot fit in the available space.
    pub pane_notice_insufficient_space_label: String,
    /// Explanation shown when a pane operation cannot fit in the available space.
    pub pane_notice_insufficient_space_description: String,
    /// Suffix shown after the character count number.
    pub status_bar_word_count_suffix: String,
    /// Source byte-format labels shown in the status bar.
    pub status_bar_encoding_utf8: String,
    pub status_bar_encoding_utf8_bom: String,
    pub status_bar_line_ending_mixed: String,
    pub status_bar_shared_document: String,
    pub status_bar_shared_views_template: String,
    /// Nav label for the status bar preferences tab.
    pub preferences_nav_status_bar: String,
    /// Label for the status bar enabled toggle.
    pub preferences_status_bar_enabled: String,
    /// Label for the character count toggle.
    pub preferences_status_bar_show_word_count: String,
    /// Label for the cursor position toggle.
    pub preferences_status_bar_show_cursor_position: String,
    /// Label for the sidebar toggle visibility.
    pub preferences_status_bar_show_sidebar_toggle: String,
    /// Label for the mode switch visibility.
    pub preferences_status_bar_show_mode_switch: String,
}

mod parts;

pub use gmark_i18n::{LanguageCatalogEntry, language_id_for_locale_preferences};
pub use parts::catalog::I18nManager;
#[cfg(test)]
#[path = "../../../tests/unit/i18n/tests.rs"]
mod tests;
