// @author kongweiguang

//! Switch, preview, and numeric input interactions.

use super::*;

impl PreferencesWindow {
    pub(super) fn set_accessibility_override(
        &mut self,
        control: PreferencesAccessibilityControl,
        value: gmark_config::AccessibilityOverride,
        cx: &mut Context<Self>,
    ) {
        match control {
            PreferencesAccessibilityControl::ReducedMotion => {
                self.visual_accessibility.reduced_motion = value;
            }
            PreferencesAccessibilityControl::ReducedTransparency => {
                self.visual_accessibility.reduced_transparency = value;
            }
            PreferencesAccessibilityControl::HighContrast => {
                self.visual_accessibility.high_contrast = value;
            }
        }
        self.preview_visual_accessibility(cx);
    }

    pub(super) fn preview_visual_accessibility(&mut self, cx: &mut Context<Self>) {
        let preferences = self.visual_accessibility;
        let changed = if cx
            .try_global::<crate::ui::visual_preferences::VisualPreferencesManager>()
            .is_some()
        {
            cx.update_global::<crate::ui::visual_preferences::VisualPreferencesManager, _>(
                |manager, _cx| manager.set_preferences(preferences),
            )
        } else {
            false
        };
        if changed {
            cx.refresh_windows();
        }
        cx.notify();
    }

    pub(super) fn restore_saved_visual_accessibility(&mut self, cx: &mut Context<Self>) {
        if self.visual_accessibility == self.saved_visual_accessibility {
            return;
        }
        self.visual_accessibility = self.saved_visual_accessibility;
        self.preview_visual_accessibility(cx);
    }

    pub(super) fn toggle_preference_switch(
        &mut self,
        preference: PreferencesSwitch,
        cx: &mut Context<Self>,
    ) {
        match preference {
            PreferencesSwitch::AutoCheckUpdates => {
                self.auto_check_updates = !self.auto_check_updates
            }
            PreferencesSwitch::ShowHiddenFiles => self.show_hidden_files = !self.show_hidden_files,
            PreferencesSwitch::SpellCheck => self.spell_check = !self.spell_check,
            PreferencesSwitch::AutoPairBrackets => {
                self.auto_pair_brackets = !self.auto_pair_brackets
            }
            PreferencesSwitch::AutoPairMarkdown => {
                self.auto_pair_markdown = !self.auto_pair_markdown
            }
            PreferencesSwitch::CodeFolding => self.code_folding = !self.code_folding,
            PreferencesSwitch::FormatOnSave => self.format_on_save = !self.format_on_save,
            PreferencesSwitch::ShowTabBarActions => {
                self.show_tab_bar_actions = !self.show_tab_bar_actions
            }
            PreferencesSwitch::StatusBarEnabled => {
                self.status_bar_enabled = !self.status_bar_enabled
            }
            PreferencesSwitch::StatusBarWordCount => {
                self.status_bar_show_word_count = !self.status_bar_show_word_count
            }
            PreferencesSwitch::StatusBarCursorPosition => {
                self.status_bar_show_cursor_position = !self.status_bar_show_cursor_position
            }
            PreferencesSwitch::StatusBarSidebarToggle => {
                self.status_bar_show_sidebar_toggle = !self.status_bar_show_sidebar_toggle
            }
            PreferencesSwitch::StatusBarModeSwitch => {
                self.status_bar_show_mode_switch = !self.status_bar_show_mode_switch
            }
        }
        cx.notify();
    }

    pub(super) fn preference_switch(
        &self,
        preference: PreferencesSwitch,
        checked: bool,
        cx: &mut Context<Self>,
    ) -> Switch {
        let focus_handle = self.switch_focus_handles[preference.index()].clone();
        let pointer_focus_handle = focus_handle.clone();
        Switch::new(preference.id())
            .debug_selector(preference.id())
            .checked(checked)
            .focus_handle(focus_handle)
            .on_click(cx.listener(move |this, _, window, cx| {
                pointer_focus_handle.focus(window);
                this.toggle_preference_switch(preference, cx);
            }))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _window, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    this.toggle_preference_switch(preference, cx);
                    cx.stop_propagation();
                }
            }))
    }

    pub(super) fn preview_theme(&mut self, cx: &mut Context<Self>) {
        let platform_appearance = cx.window_appearance();
        let appearance = self.theme_appearance;
        let palette = self.theme_palette;
        let changed = cx.update_global::<ThemeManager, _>(|theme_manager, _cx| {
            let changed =
                theme_manager.set_theme_preference(appearance, palette, platform_appearance);
            if changed {
                theme_manager
                    .set_editor_typography(self.editor_font_size, self.editor_line_height_percent);
                theme_manager.set_editor_content_width(self.editor_content_width);
            }
            changed
        });
        if changed {
            cx.refresh_windows();
        }
        cx.notify();
    }

    pub(super) fn preview_theme_appearance(
        &mut self,
        appearance: ThemeAppearance,
        cx: &mut Context<Self>,
    ) {
        self.theme_appearance = appearance;
        self.preview_theme(cx);
    }

    pub(super) fn preview_theme_palette(&mut self, palette: ThemePalette, cx: &mut Context<Self>) {
        self.theme_palette = palette;
        self.preview_theme(cx);
    }

    pub(super) fn preview_editor_typography(&mut self, cx: &mut Context<Self>) {
        cx.update_global::<ThemeManager, _>(|theme_manager, _cx| {
            theme_manager
                .set_editor_typography(self.editor_font_size, self.editor_line_height_percent);
            theme_manager.set_editor_content_width(self.editor_content_width);
        });
        cx.refresh_windows();
        cx.notify();
    }

    pub(super) fn preview_editor_font_family(
        &mut self,
        font_family: String,
        cx: &mut Context<Self>,
    ) {
        self.editor_font_family = normalize_editor_font_family(&font_family);
        let font_family = self.editor_font_family.clone();
        cx.update_global::<EditorSettings, _>(|settings, _cx| {
            settings.editor_font_family = font_family;
        });
        self.close_all_dropdowns();
        cx.refresh_windows();
        cx.notify();
    }

    pub(super) fn activate_stepper(
        &mut self,
        control: PreferencesStepperControl,
        cx: &mut Context<Self>,
    ) {
        match control {
            PreferencesStepperControl::FontSizeDecrease => {
                self.editor_font_size = self
                    .editor_font_size
                    .saturating_sub(1)
                    .max(MIN_EDITOR_FONT_SIZE);
            }
            PreferencesStepperControl::FontSizeIncrease => {
                self.editor_font_size = self
                    .editor_font_size
                    .saturating_add(1)
                    .min(MAX_EDITOR_FONT_SIZE);
            }
            PreferencesStepperControl::LineHeightDecrease => {
                self.editor_line_height_percent = self
                    .editor_line_height_percent
                    .saturating_sub(EDITOR_LINE_HEIGHT_STEP)
                    .max(MIN_EDITOR_LINE_HEIGHT_PERCENT);
            }
            PreferencesStepperControl::LineHeightIncrease => {
                self.editor_line_height_percent = self
                    .editor_line_height_percent
                    .saturating_add(EDITOR_LINE_HEIGHT_STEP)
                    .min(MAX_EDITOR_LINE_HEIGHT_PERCENT);
            }
            PreferencesStepperControl::ContentWidthDecrease => {
                self.editor_content_width = self
                    .editor_content_width
                    .saturating_sub(EDITOR_CONTENT_WIDTH_STEP)
                    .max(MIN_EDITOR_CONTENT_WIDTH);
            }
            PreferencesStepperControl::ContentWidthIncrease => {
                self.editor_content_width = self
                    .editor_content_width
                    .saturating_add(EDITOR_CONTENT_WIDTH_STEP)
                    .min(MAX_EDITOR_CONTENT_WIDTH);
            }
            PreferencesStepperControl::ResidentMibDecrease => {
                let value = self.document_loading.effective_max_resident_mib();
                self.document_loading.max_resident_mib = Some(value.saturating_sub(1).max(1));
            }
            PreferencesStepperControl::ResidentMibIncrease => {
                let value = self.document_loading.effective_max_resident_mib();
                self.document_loading.max_resident_mib = Some(value.saturating_add(1).min(1_024));
            }
        }
        let input = match control {
            PreferencesStepperControl::FontSizeDecrease
            | PreferencesStepperControl::FontSizeIncrease => PreferencesNumericInput::FontSize,
            PreferencesStepperControl::LineHeightDecrease
            | PreferencesStepperControl::LineHeightIncrease => PreferencesNumericInput::LineHeight,
            PreferencesStepperControl::ContentWidthDecrease
            | PreferencesStepperControl::ContentWidthIncrease => {
                PreferencesNumericInput::ContentWidth
            }
            PreferencesStepperControl::ResidentMibDecrease
            | PreferencesStepperControl::ResidentMibIncrease => {
                PreferencesNumericInput::ResidentMib
            }
        };
        self.sync_numeric_input(input, cx);
        if matches!(
            control,
            PreferencesStepperControl::FontSizeDecrease
                | PreferencesStepperControl::FontSizeIncrease
                | PreferencesStepperControl::LineHeightDecrease
                | PreferencesStepperControl::LineHeightIncrease
                | PreferencesStepperControl::ContentWidthDecrease
                | PreferencesStepperControl::ContentWidthIncrease
        ) {
            self.preview_editor_typography(cx);
        } else {
            cx.notify();
        }
    }

    pub(super) fn on_numeric_input_event(
        &mut self,
        input: Entity<Block>,
        event: &BlockEvent,
        cx: &mut Context<Self>,
    ) {
        if !matches!(event, BlockEvent::Changed) {
            return;
        }
        let Some(index) = self
            .numeric_inputs
            .iter()
            .position(|candidate| candidate == &input)
        else {
            return;
        };
        let field = PreferencesNumericInput::ORDER[index];
        let text = input.read(cx).display_text().to_owned();
        let Some(value) = parse_numeric_input(field, &text) else {
            // 编辑期间允许空值或尚未完成的数字；无效值不会进入设置草稿，也会禁用保存。
            cx.notify();
            return;
        };
        match field {
            PreferencesNumericInput::FontSize => self.editor_font_size = value as u8,
            PreferencesNumericInput::LineHeight => self.editor_line_height_percent = value as u16,
            PreferencesNumericInput::ContentWidth => self.editor_content_width = value as u16,
            PreferencesNumericInput::ResidentMib => {
                self.document_loading.max_resident_mib = Some(value)
            }
        }
        if matches!(
            field,
            PreferencesNumericInput::FontSize
                | PreferencesNumericInput::LineHeight
                | PreferencesNumericInput::ContentWidth
        ) {
            self.preview_editor_typography(cx);
        } else {
            cx.notify();
        }
    }

    pub(super) fn numeric_input_is_valid(&self, field: PreferencesNumericInput, cx: &App) -> bool {
        parse_numeric_input(
            field,
            self.numeric_inputs[field.index()].read(cx).display_text(),
        )
        .is_some()
    }

    pub(super) fn has_invalid_numeric_input(&self, cx: &App) -> bool {
        PreferencesNumericInput::ORDER
            .iter()
            .any(|field| !self.numeric_input_is_valid(*field, cx))
    }

    fn numeric_input_value(&self, field: PreferencesNumericInput) -> u64 {
        match field {
            PreferencesNumericInput::FontSize => u64::from(self.editor_font_size),
            PreferencesNumericInput::LineHeight => u64::from(self.editor_line_height_percent),
            PreferencesNumericInput::ContentWidth => u64::from(self.editor_content_width),
            PreferencesNumericInput::ResidentMib => {
                self.document_loading.effective_max_resident_mib()
            }
        }
    }

    fn sync_numeric_input(&mut self, field: PreferencesNumericInput, cx: &mut Context<Self>) {
        let value = self.numeric_input_value(field).to_string();
        let input = self.numeric_inputs[field.index()].clone();
        input.update(cx, |input, cx| {
            if input.display_text() == value {
                return;
            }
            let len = input.visible_len();
            input.replace_text_in_visible_range(0..len, &value, None, false, cx);
        });
    }
}
