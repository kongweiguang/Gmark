// @author kongweiguang

//! Preferences window state and control identifiers.

use super::*;

impl PreferencesNav {
    pub(super) const ORDER: [Self; 6] = [
        Self::File,
        Self::Editor,
        Self::Theme,
        Self::Image,
        Self::Shortcuts,
        Self::StatusBar,
    ];

    pub(super) fn index(self) -> usize {
        Self::ORDER
            .iter()
            .position(|candidate| *candidate == self)
            .expect("preferences navigation is part of the fixed order")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PreferencesDropdown {
    Startup,
    AutoSave,
    Language,
    Image,
    Font,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PreferencesSwitch {
    AutoCheckUpdates,
    ShowHiddenFiles,
    SpellCheck,
    AutoPairBrackets,
    AutoPairMarkdown,
    CodeFolding,
    FormatOnSave,
    ShowTabBarActions,
    StatusBarEnabled,
    StatusBarWordCount,
    StatusBarCursorPosition,
    StatusBarSidebarToggle,
    StatusBarModeSwitch,
}

/// Visual accessibility controls are intentionally grouped as a segmented
/// control.  Keeping a stable order makes keyboard navigation predictable and
/// lets the preference window preserve focus when the layout reflows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PreferencesAccessibilityControl {
    ReducedMotion,
    ReducedTransparency,
    HighContrast,
}

impl PreferencesAccessibilityControl {
    pub(super) const COUNT: usize = 3;

    pub(super) fn index(self) -> usize {
        match self {
            Self::ReducedMotion => 0,
            Self::ReducedTransparency => 1,
            Self::HighContrast => 2,
        }
    }

    pub(super) fn id(self) -> &'static str {
        match self {
            Self::ReducedMotion => "preferences-accessibility-reduced-motion",
            Self::ReducedTransparency => "preferences-accessibility-reduced-transparency",
            Self::HighContrast => "preferences-accessibility-high-contrast",
        }
    }
}

impl PreferencesSwitch {
    pub(super) const COUNT: usize = 13;

    pub(super) fn index(self) -> usize {
        match self {
            Self::SpellCheck => 0,
            Self::AutoPairBrackets => 1,
            Self::AutoPairMarkdown => 2,
            Self::ShowTabBarActions => 3,
            Self::StatusBarEnabled => 4,
            Self::StatusBarWordCount => 5,
            Self::StatusBarCursorPosition => 6,
            Self::StatusBarSidebarToggle => 7,
            Self::StatusBarModeSwitch => 8,
            Self::AutoCheckUpdates => 9,
            Self::CodeFolding => 10,
            Self::FormatOnSave => 11,
            Self::ShowHiddenFiles => 12,
        }
    }

    pub(super) fn id(self) -> &'static str {
        match self {
            Self::AutoCheckUpdates => "preferences-auto-check-updates",
            Self::ShowHiddenFiles => "preferences-show-hidden-files",
            Self::SpellCheck => "preferences-spell-check",
            Self::AutoPairBrackets => "preferences-auto-pair-brackets",
            Self::AutoPairMarkdown => "preferences-auto-pair-markdown",
            Self::CodeFolding => "preferences-code-folding",
            Self::FormatOnSave => "preferences-format-on-save",
            Self::ShowTabBarActions => "preferences-show-tab-bar-actions",
            Self::StatusBarEnabled => "preferences-status-bar-enabled",
            Self::StatusBarWordCount => "preferences-status-bar-word-count",
            Self::StatusBarCursorPosition => "preferences-status-bar-cursor-position",
            Self::StatusBarSidebarToggle => "preferences-status-bar-sidebar-toggle",
            Self::StatusBarModeSwitch => "preferences-status-bar-mode-switch",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PreferencesStepperControl {
    FontSizeDecrease,
    FontSizeIncrease,
    LineHeightDecrease,
    LineHeightIncrease,
    ContentWidthDecrease,
    ContentWidthIncrease,
    ResidentMibDecrease,
    ResidentMibIncrease,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PreferencesNumericInput {
    FontSize,
    LineHeight,
    ContentWidth,
    ResidentMib,
}

impl PreferencesNumericInput {
    pub(super) const COUNT: usize = 4;

    pub(super) const ORDER: [Self; Self::COUNT] = [
        Self::FontSize,
        Self::LineHeight,
        Self::ContentWidth,
        Self::ResidentMib,
    ];

    pub(super) fn index(self) -> usize {
        Self::ORDER
            .iter()
            .position(|candidate| *candidate == self)
            .expect("numeric preference is part of the fixed order")
    }

    pub(super) fn bounds(self) -> (u64, u64) {
        match self {
            Self::FontSize => (
                u64::from(MIN_EDITOR_FONT_SIZE),
                u64::from(MAX_EDITOR_FONT_SIZE),
            ),
            Self::LineHeight => (
                u64::from(MIN_EDITOR_LINE_HEIGHT_PERCENT),
                u64::from(MAX_EDITOR_LINE_HEIGHT_PERCENT),
            ),
            Self::ContentWidth => (
                u64::from(MIN_EDITOR_CONTENT_WIDTH),
                u64::from(MAX_EDITOR_CONTENT_WIDTH),
            ),
            Self::ResidentMib => (1, 1_024),
        }
    }

    pub(super) fn input_id(self) -> &'static str {
        match self {
            Self::FontSize => "preferences-editor-font-size-input",
            Self::LineHeight => "preferences-editor-line-height-input",
            Self::ContentWidth => "preferences-editor-content-width-input",
            Self::ResidentMib => "preferences-document-resident-mib-input",
        }
    }
}

pub(super) fn parse_numeric_input(field: PreferencesNumericInput, text: &str) -> Option<u64> {
    let value = text.trim().parse::<u64>().ok()?;
    let (minimum, maximum) = field.bounds();
    (minimum..=maximum).contains(&value).then_some(value)
}

impl PreferencesStepperControl {
    pub(super) const COUNT: usize = 8;

    pub(super) fn index(self) -> usize {
        match self {
            Self::FontSizeDecrease => 0,
            Self::FontSizeIncrease => 1,
            Self::LineHeightDecrease => 2,
            Self::LineHeightIncrease => 3,
            Self::ContentWidthDecrease => 4,
            Self::ContentWidthIncrease => 5,
            Self::ResidentMibDecrease => 6,
            Self::ResidentMibIncrease => 7,
        }
    }

    pub(super) fn id(self) -> &'static str {
        match self {
            Self::FontSizeDecrease => "preferences-editor-font-size-decrease",
            Self::FontSizeIncrease => "preferences-editor-font-size-increase",
            Self::LineHeightDecrease => "preferences-editor-line-height-decrease",
            Self::LineHeightIncrease => "preferences-editor-line-height-increase",
            Self::ContentWidthDecrease => "preferences-editor-content-width-decrease",
            Self::ContentWidthIncrease => "preferences-editor-content-width-increase",
            Self::ResidentMibDecrease => "preferences-document-resident-mib-decrease",
            Self::ResidentMibIncrease => "preferences-document-resident-mib-increase",
        }
    }
}

impl PreferencesDropdown {
    pub(super) const COUNT: usize = 5;

    pub(super) fn index(self) -> usize {
        match self {
            Self::Startup => 0,
            Self::AutoSave => 1,
            Self::Language => 2,
            Self::Image => 3,
            Self::Font => 4,
        }
    }
}

#[derive(Clone)]
pub(super) struct PreferenceSearchItem {
    pub(super) nav: PreferencesNav,
    pub(super) category: String,
    pub(super) label: String,
}

/// Independent preferences window view.
pub(crate) struct PreferencesWindow {
    pub(super) nav: PreferencesNav,
    pub(super) startup_open: StartupOpenPreference,
    pub(super) auto_check_updates: bool,
    pub(super) show_hidden_files: bool,
    pub(super) auto_save: AutoSavePreference,
    pub(super) spell_check: bool,
    pub(super) auto_pair_brackets: bool,
    pub(super) auto_pair_markdown: bool,
    pub(super) code_folding: bool,
    pub(super) format_on_save: bool,
    pub(super) editor_font_size: u8,
    pub(super) editor_line_height_percent: u16,
    pub(super) editor_content_width: u16,
    pub(super) editor_font_family: String,
    pub(super) show_tab_bar_actions: bool,
    pub(super) theme_appearance: ThemeAppearance,
    pub(super) theme_palette: ThemePalette,
    pub(super) selected_language_id: String,
    pub(super) image_paste_behavior: ImagePasteBehavior,
    pub(super) keybindings: BTreeMap<String, Vec<String>>,
    pub(super) document_loading: DocumentLoadingPreferences,
    pub(super) visual_accessibility: VisualAccessibilityPreferences,
    pub(super) saved_startup_open: StartupOpenPreference,
    pub(super) saved_auto_check_updates: bool,
    pub(super) saved_show_hidden_files: bool,
    pub(super) saved_auto_save: AutoSavePreference,
    pub(super) saved_spell_check: bool,
    pub(super) saved_auto_pair_brackets: bool,
    pub(super) saved_auto_pair_markdown: bool,
    pub(super) saved_code_folding: bool,
    pub(super) saved_format_on_save: bool,
    pub(super) saved_editor_font_size: u8,
    pub(super) saved_editor_line_height_percent: u16,
    pub(super) saved_editor_content_width: u16,
    pub(super) saved_editor_font_family: String,
    pub(super) saved_show_tab_bar_actions: bool,
    pub(super) saved_theme_appearance: ThemeAppearance,
    pub(super) saved_theme_palette: ThemePalette,
    pub(super) saved_language_id: String,
    pub(super) saved_image_paste_behavior: ImagePasteBehavior,
    pub(super) saved_keybindings: BTreeMap<String, Vec<String>>,
    pub(super) saved_document_loading: DocumentLoadingPreferences,
    pub(super) saved_visual_accessibility: VisualAccessibilityPreferences,
    pub(super) language_options: Vec<LanguageCatalogEntry>,
    pub(super) font_options: Vec<String>,
    pub(super) focus_handle: FocusHandle,
    pub(super) action_focus_handles: [FocusHandle; 2],
    pub(super) nav_focus_handles: [FocusHandle; 6],
    pub(super) dropdown_focus_handles: [FocusHandle; PreferencesDropdown::COUNT],
    pub(super) theme_appearance_focus_handles: [FocusHandle; 3],
    pub(super) theme_palette_focus_handles: [FocusHandle; 4],
    pub(super) accessibility_focus_handles:
        [[FocusHandle; 3]; PreferencesAccessibilityControl::COUNT],
    pub(super) dropdown_selected_indices: [usize; PreferencesDropdown::COUNT],
    pub(super) switch_focus_handles: [FocusHandle; PreferencesSwitch::COUNT],
    pub(super) stepper_focus_handles: [FocusHandle; PreferencesStepperControl::COUNT],
    pub(super) numeric_inputs: [Entity<Block>; PreferencesNumericInput::COUNT],
    pub(super) search_input: Entity<Block>,
    pub(super) search_selected: usize,
    pub(super) startup_dropdown_open: bool,
    pub(super) auto_save_dropdown_open: bool,
    pub(super) language_dropdown_open: bool,
    pub(super) image_dropdown_open: bool,
    pub(super) font_dropdown_open: bool,
    pub(super) recording_shortcut: Option<ShortcutCommand>,
    pub(super) shortcut_error: Option<String>,
    pub(super) status_bar_enabled: bool,
    pub(super) status_bar_show_word_count: bool,
    pub(super) status_bar_show_cursor_position: bool,
    pub(super) status_bar_show_sidebar_toggle: bool,
    pub(super) status_bar_show_mode_switch: bool,
    pub(super) status_bar_custom_buttons: Vec<StatusBarButton>,
    pub(super) saved_status_bar_enabled: bool,
    pub(super) saved_status_bar_show_word_count: bool,
    pub(super) saved_status_bar_show_cursor_position: bool,
    pub(super) saved_status_bar_show_sidebar_toggle: bool,
    pub(super) saved_status_bar_show_mode_switch: bool,
}
