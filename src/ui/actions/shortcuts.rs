// @author kongweiguang

use super::*;

pub(crate) fn shortcut_definitions() -> &'static [ShortcutDefinition] {
    super::definitions::all()
}

pub(crate) fn normalize_shortcut_keys(keys: &[String]) -> Option<Vec<String>> {
    let mut seen = BTreeSet::new();
    let mut normalized = Vec::new();
    for key in keys {
        let parsed = Keystroke::parse(key.trim()).ok()?;
        if parsed.is_ime_in_progress() {
            return None;
        }
        let key = parsed.unparse();
        if seen.insert(key.clone()) {
            normalized.push(key);
        }
    }
    (!normalized.is_empty()).then_some(normalized)
}

/// Formats one or more GPUI keystrokes for user-facing labels.
///
/// Shortcut values remain in GPUI's canonical form everywhere else. This
/// helper is deliberately display-only: it parses each space-separated
/// candidate and falls back to the original candidate when GPUI cannot parse
/// it, so a malformed preference still has a readable representation.
pub(crate) fn format_shortcut_for_display(raw: &str) -> String {
    if raw.trim().is_empty() {
        return raw.to_owned();
    }

    raw.split_whitespace()
        .map(format_single_shortcut_for_display)
        .collect::<Vec<_>>()
        .join(" ")
}

fn format_single_shortcut_for_display(raw: &str) -> String {
    let Ok(keystroke) = Keystroke::parse(raw) else {
        return raw.to_owned();
    };

    let modifiers = keystroke.modifiers;
    let mut parts = Vec::with_capacity(6);
    if modifiers.control {
        parts.push("Ctrl".to_owned());
    }
    if modifiers.alt {
        parts.push("Alt".to_owned());
    }
    if modifiers.shift {
        parts.push("Shift".to_owned());
    }
    if modifiers.platform {
        parts.push(platform_modifier_label().to_owned());
    }
    if modifiers.function {
        parts.push("Fn".to_owned());
    }
    parts.push(display_shortcut_key(&keystroke.key));
    parts.join("+")
}

fn platform_modifier_label() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "Cmd"
    }
    #[cfg(target_os = "windows")]
    {
        "Win"
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        "Super"
    }
}

fn display_shortcut_key(key: &str) -> String {
    match key {
        "backspace" => "Backspace".to_owned(),
        "delete" => "Delete".to_owned(),
        "escape" => "Esc".to_owned(),
        "enter" => "Enter".to_owned(),
        "space" => "Space".to_owned(),
        "tab" => "Tab".to_owned(),
        "pageup" => "Page Up".to_owned(),
        "pagedown" => "Page Down".to_owned(),
        "left" => "Left".to_owned(),
        "right" => "Right".to_owned(),
        "up" => "Up".to_owned(),
        "down" => "Down".to_owned(),
        "home" => "Home".to_owned(),
        "end" => "End".to_owned(),
        "insert" => "Insert".to_owned(),
        "back" => "Back".to_owned(),
        "forward" => "Forward".to_owned(),
        key if key.len() == 1 => key.to_uppercase(),
        key if key.strip_prefix('f').is_some_and(|suffix| {
            !suffix.is_empty() && suffix.chars().all(|character| character.is_ascii_digit())
        }) =>
        {
            key.to_uppercase()
        }
        other => other.to_owned(),
    }
}

/// 只实例化当前系统的默认键，避免冲突检测和运行时绑定混入另一平台组合。
fn default_keys(definition: ShortcutDefinition) -> Vec<String> {
    definition
        .default_keys
        .current()
        .iter()
        .map(|key| key.to_string())
        .collect()
}

/// Legacy preference keys that should feed a modern shortcut definition.
///
/// Select-all used to be represented by separate source/rendered commands. The
/// editor now cycles those behaviors through one action, so old preferences map
/// forward to `select_all` instead of being silently discarded on load.
fn legacy_shortcut_ids(definition: ShortcutDefinition) -> &'static [&'static str] {
    match definition.command {
        ShortcutCommand::SelectAll => LEGACY_SELECT_ALL_IDS,
        _ => &[],
    }
}

/// Reads a user shortcut override, preferring the current id before aliases.
fn configured_shortcut_keys(
    definition: ShortcutDefinition,
    config: &BTreeMap<String, Vec<String>>,
) -> Option<Vec<String>> {
    config
        .get(definition.id)
        .and_then(|keys| normalize_shortcut_keys(keys))
        .or_else(|| {
            legacy_shortcut_ids(definition).iter().find_map(|id| {
                config
                    .get(*id)
                    .and_then(|keys| normalize_shortcut_keys(keys))
            })
        })
}

fn shortcuts_conflict(
    left: ShortcutDefinition,
    left_keys: &[String],
    right: ShortcutDefinition,
    right_keys: &[String],
) -> bool {
    left.context == right.context && left_keys.iter().any(|key| right_keys.contains(key))
}

pub(crate) fn normalize_shortcut_config(
    config: &BTreeMap<String, Vec<String>>,
) -> BTreeMap<String, Vec<String>> {
    let mut effective: BTreeMap<&'static str, (bool, Vec<String>)> = BTreeMap::new();
    for definition in shortcut_definitions() {
        let custom = configured_shortcut_keys(*definition, config);
        effective.insert(
            definition.id,
            match custom {
                Some(keys) if keys != default_keys(*definition) => (true, keys),
                _ => (false, default_keys(*definition)),
            },
        );
    }

    loop {
        let mut conflicted = BTreeSet::new();
        for (index, left) in shortcut_definitions().iter().enumerate() {
            let (left_custom, left_keys) = effective.get(left.id).expect("known shortcut");
            for right in shortcut_definitions().iter().skip(index + 1) {
                let (right_custom, right_keys) = effective.get(right.id).expect("known shortcut");
                if shortcuts_conflict(*left, left_keys, *right, right_keys) {
                    if *left_custom {
                        conflicted.insert(left.id);
                    }
                    if *right_custom {
                        conflicted.insert(right.id);
                    }
                }
            }
        }

        if conflicted.is_empty() {
            break;
        }

        for id in conflicted {
            if let Some(definition) = shortcut_definitions()
                .iter()
                .find(|definition| definition.id == id)
            {
                effective.insert(definition.id, (false, default_keys(*definition)));
            }
        }
    }

    effective
        .into_iter()
        .filter_map(|(id, (custom, keys))| custom.then_some((id.to_string(), keys)))
        .collect()
}

pub(crate) fn resolved_shortcut_keys(
    config: &BTreeMap<String, Vec<String>>,
    command: ShortcutCommand,
) -> Vec<String> {
    let normalized = normalize_shortcut_config(config);
    let definition = shortcut_definitions()
        .iter()
        .find(|definition| definition.command == command)
        .expect("known shortcut command");
    normalized
        .get(definition.id)
        .cloned()
        .unwrap_or_else(|| default_keys(*definition))
}

pub(crate) fn shortcut_conflict_for(
    command: ShortcutCommand,
    proposed_keys: &[String],
    config: &BTreeMap<String, Vec<String>>,
) -> Option<ShortcutDefinition> {
    let definition = shortcut_definitions()
        .iter()
        .find(|definition| definition.command == command)?;
    let proposed_keys = normalize_shortcut_keys(proposed_keys)?;
    for other in shortcut_definitions()
        .iter()
        .filter(|other| other.command != command)
    {
        let other_keys = resolved_shortcut_keys(config, other.command);
        if shortcuts_conflict(*definition, &proposed_keys, *other, &other_keys) {
            return Some(*other);
        }
    }
    None
}

/// Converts a configurable command into a GPUI action while preserving its declared focus context.
fn key_binding_for(
    command: ShortcutCommand,
    key: &str,
    context: Option<&'static str>,
) -> KeyBinding {
    match command {
        ShortcutCommand::Newline => KeyBinding::new(key, Newline, context),
        ShortcutCommand::DeleteBack => KeyBinding::new(key, DeleteBack, context),
        ShortcutCommand::Delete => KeyBinding::new(key, Delete, context),
        ShortcutCommand::WordDeleteBack => KeyBinding::new(key, WordDeleteBack, context),
        ShortcutCommand::WordDeleteForward => KeyBinding::new(key, WordDeleteForward, context),
        ShortcutCommand::FocusPrev => KeyBinding::new(key, FocusPrev, context),
        ShortcutCommand::FocusNext => KeyBinding::new(key, FocusNext, context),
        ShortcutCommand::MoveLeft => KeyBinding::new(key, MoveLeft, context),
        ShortcutCommand::MoveRight => KeyBinding::new(key, MoveRight, context),
        ShortcutCommand::WordMoveLeft => KeyBinding::new(key, WordMoveLeft, context),
        ShortcutCommand::WordMoveRight => KeyBinding::new(key, WordMoveRight, context),
        ShortcutCommand::Home => KeyBinding::new(key, Home, context),
        ShortcutCommand::End => KeyBinding::new(key, End, context),
        ShortcutCommand::MoveToDocumentStart => KeyBinding::new(key, MoveToDocumentStart, context),
        ShortcutCommand::MoveToDocumentEnd => KeyBinding::new(key, MoveToDocumentEnd, context),
        ShortcutCommand::BlockUp => KeyBinding::new(key, BlockUp, context),
        ShortcutCommand::BlockDown => KeyBinding::new(key, BlockDown, context),
        ShortcutCommand::PageUp => KeyBinding::new(key, PageUp, context),
        ShortcutCommand::PageDown => KeyBinding::new(key, PageDown, context),
        ShortcutCommand::JumpToTop => KeyBinding::new(key, JumpToTop, context),
        ShortcutCommand::JumpToBottom => KeyBinding::new(key, JumpToBottom, context),
        ShortcutCommand::SelectLeft => KeyBinding::new(key, SelectLeft, context),
        ShortcutCommand::SelectRight => KeyBinding::new(key, SelectRight, context),
        ShortcutCommand::SelectUp => KeyBinding::new(key, SelectUp, context),
        ShortcutCommand::SelectDown => KeyBinding::new(key, SelectDown, context),
        ShortcutCommand::SelectPageUp => KeyBinding::new(key, SelectPageUp, context),
        ShortcutCommand::SelectPageDown => KeyBinding::new(key, SelectPageDown, context),
        ShortcutCommand::WordSelectLeft => KeyBinding::new(key, WordSelectLeft, context),
        ShortcutCommand::WordSelectRight => KeyBinding::new(key, WordSelectRight, context),
        ShortcutCommand::SelectHome => KeyBinding::new(key, SelectHome, context),
        ShortcutCommand::SelectEnd => KeyBinding::new(key, SelectEnd, context),
        ShortcutCommand::SelectToDocumentStart => {
            KeyBinding::new(key, SelectToDocumentStart, context)
        }
        ShortcutCommand::SelectToDocumentEnd => KeyBinding::new(key, SelectToDocumentEnd, context),
        ShortcutCommand::SelectAll => KeyBinding::new(key, SelectAll, context),
        ShortcutCommand::DuplicateLine => KeyBinding::new(key, DuplicateLine, context),
        ShortcutCommand::DeleteLine => KeyBinding::new(key, DeleteLine, context),
        ShortcutCommand::MoveLineUp => KeyBinding::new(key, MoveLineUp, context),
        ShortcutCommand::MoveLineDown => KeyBinding::new(key, MoveLineDown, context),
        ShortcutCommand::Copy => KeyBinding::new(key, Copy, context),
        ShortcutCommand::CopyAsMarkdown => KeyBinding::new(key, CopyAsMarkdown, context),
        ShortcutCommand::Cut => KeyBinding::new(key, Cut, context),
        ShortcutCommand::Paste => KeyBinding::new(key, Paste, context),
        ShortcutCommand::PasteAsPlainText => KeyBinding::new(key, PasteAsPlainText, context),
        ShortcutCommand::Undo => KeyBinding::new(key, Undo, context),
        ShortcutCommand::Redo => KeyBinding::new(key, Redo, context),
        ShortcutCommand::BoldSelection => KeyBinding::new(key, BoldSelection, context),
        ShortcutCommand::ItalicSelection => KeyBinding::new(key, ItalicSelection, context),
        ShortcutCommand::StrikethroughSelection => {
            KeyBinding::new(key, StrikethroughSelection, context)
        }
        ShortcutCommand::UnderlineSelection => KeyBinding::new(key, UnderlineSelection, context),
        ShortcutCommand::CodeSelection => KeyBinding::new(key, CodeSelection, context),
        ShortcutCommand::LinkSelection => KeyBinding::new(key, LinkSelection, context),
        ShortcutCommand::HighlightSelection => KeyBinding::new(key, HighlightSelection, context),
        ShortcutCommand::SuperscriptSelection => {
            KeyBinding::new(key, SuperscriptSelection, context)
        }
        ShortcutCommand::SubscriptSelection => KeyBinding::new(key, SubscriptSelection, context),
        ShortcutCommand::InlineMathSelection => KeyBinding::new(key, InlineMathSelection, context),
        ShortcutCommand::SetParagraph => KeyBinding::new(key, SetParagraph, context),
        ShortcutCommand::SetHeading1 => KeyBinding::new(key, SetHeading1, context),
        ShortcutCommand::SetHeading2 => KeyBinding::new(key, SetHeading2, context),
        ShortcutCommand::SetHeading3 => KeyBinding::new(key, SetHeading3, context),
        ShortcutCommand::SetHeading4 => KeyBinding::new(key, SetHeading4, context),
        ShortcutCommand::SetHeading5 => KeyBinding::new(key, SetHeading5, context),
        ShortcutCommand::SetHeading6 => KeyBinding::new(key, SetHeading6, context),
        ShortcutCommand::IndentBlock => KeyBinding::new(key, IndentBlock, context),
        ShortcutCommand::OutdentBlock => KeyBinding::new(key, OutdentBlock, context),
        ShortcutCommand::ExitCodeBlock => KeyBinding::new(key, ExitCodeBlock, context),
        ShortcutCommand::SaveDocument => KeyBinding::new(key, SaveDocument, context),
        ShortcutCommand::SaveDocumentAs => KeyBinding::new(key, SaveDocumentAs, context),
        ShortcutCommand::CollapseFold => KeyBinding::new(key, CollapseFold, context),
        ShortcutCommand::ExpandFold => KeyBinding::new(key, ExpandFold, context),
        ShortcutCommand::CollapseAllFolds => KeyBinding::new(key, CollapseAllFolds, context),
        ShortcutCommand::ExpandAllFolds => KeyBinding::new(key, ExpandAllFolds, context),
        ShortcutCommand::FormatDocument => KeyBinding::new(key, FormatDocument, context),
        ShortcutCommand::FormatSelection => KeyBinding::new(key, FormatSelection, context),
        ShortcutCommand::CancelFormatting => KeyBinding::new(key, CancelFormatting, context),
        ShortcutCommand::NewTab => KeyBinding::new(key, NewTab, context),
        ShortcutCommand::NewWindow => KeyBinding::new(key, NewWindow, context),
        ShortcutCommand::OpenFile => KeyBinding::new(key, OpenFile, context),
        ShortcutCommand::OpenFolder => KeyBinding::new(key, OpenFolder, context),
        ShortcutCommand::OpenPreferences => KeyBinding::new(key, OpenPreferences, context),
        ShortcutCommand::QuitApplication => KeyBinding::new(key, QuitApplication, context),
        ShortcutCommand::CloseWindow => KeyBinding::new(key, CloseWindow, context),
        ShortcutCommand::CloseTab => KeyBinding::new(key, CloseTab, context),
        ShortcutCommand::ReopenClosedTab => KeyBinding::new(key, ReopenClosedTab, context),
        ShortcutCommand::PreviousTab => KeyBinding::new(key, PreviousTab, context),
        ShortcutCommand::NextTab => KeyBinding::new(key, NextTab, context),
        ShortcutCommand::DismissTransientUi => KeyBinding::new(key, DismissTransientUi, context),
        ShortcutCommand::ToggleViewMode => KeyBinding::new(key, ToggleViewMode, context),
        ShortcutCommand::ToggleWorkspace => KeyBinding::new(key, ToggleWorkspace, context),
        ShortcutCommand::ToggleDocumentSidebar => {
            KeyBinding::new(key, ToggleDocumentSidebar, context)
        }
        ShortcutCommand::SplitRight => KeyBinding::new(key, SplitRight, context),
        ShortcutCommand::SplitDown => KeyBinding::new(key, SplitDown, context),
        ShortcutCommand::ClosePane => KeyBinding::new(key, ClosePane, context),
        ShortcutCommand::FocusPaneLeft => KeyBinding::new(key, FocusPaneLeft, context),
        ShortcutCommand::FocusPaneRight => KeyBinding::new(key, FocusPaneRight, context),
        ShortcutCommand::FocusPaneUp => KeyBinding::new(key, FocusPaneUp, context),
        ShortcutCommand::FocusPaneDown => KeyBinding::new(key, FocusPaneDown, context),
        ShortcutCommand::MoveTabToPaneLeft => KeyBinding::new(key, MoveTabToPaneLeft, context),
        ShortcutCommand::MoveTabToPaneRight => KeyBinding::new(key, MoveTabToPaneRight, context),
        ShortcutCommand::MoveTabToPaneUp => KeyBinding::new(key, MoveTabToPaneUp, context),
        ShortcutCommand::MoveTabToPaneDown => KeyBinding::new(key, MoveTabToPaneDown, context),
        ShortcutCommand::BalancePanes => KeyBinding::new(key, BalancePanes, context),
        ShortcutCommand::QuickOpen => KeyBinding::new(key, QuickOpen, context),
        ShortcutCommand::CommandPalette => KeyBinding::new(key, CommandPalette, context),
        ShortcutCommand::GoToLine => KeyBinding::new(key, GoToLine, context),
        ShortcutCommand::FindInDocument => KeyBinding::new(key, FindInDocument, context),
        ShortcutCommand::ReplaceInDocument => KeyBinding::new(key, ReplaceInDocument, context),
        ShortcutCommand::FindNext => KeyBinding::new(key, FindNext, context),
        ShortcutCommand::FindPrevious => KeyBinding::new(key, FindPrevious, context),
        ShortcutCommand::ToggleFocusMode => KeyBinding::new(key, ToggleFocusMode, context),
        ShortcutCommand::ToggleTypewriterMode => {
            KeyBinding::new(key, ToggleTypewriterMode, context)
        }
    }
}

pub(crate) fn resolved_keybindings(config: &BTreeMap<String, Vec<String>>) -> Vec<KeyBinding> {
    let normalized = normalize_shortcut_config(config);
    let mut bindings = Vec::new();
    for definition in shortcut_definitions() {
        let keys = normalized
            .get(definition.id)
            .cloned()
            .unwrap_or_else(|| default_keys(*definition));
        bindings.extend(
            keys.iter()
                .map(|key| key_binding_for(definition.command, key, definition.context)),
        );
    }
    bindings
}

pub(crate) fn install_keybindings(cx: &mut App, config: &BTreeMap<String, Vec<String>>) {
    cx.bind_keys(resolved_keybindings(config));
}

/// Register default key bindings for isolated GPUI tests.
#[cfg(test)]
pub(super) fn init(cx: &mut App) {
    install_keybindings(cx, &BTreeMap::new());
}

pub(crate) fn init_with_keybindings(cx: &mut App, config: &BTreeMap<String, Vec<String>>) {
    install_keybindings(cx, config);
}
