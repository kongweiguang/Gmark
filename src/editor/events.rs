// @author kongweiguang

//! Editor-side handling for [`BlockEvent`] values emitted by child blocks.
//!
//! This is the central mutation engine for split, merge, indent, outdent,
//! delete, multiline paste, focus transfer, and dirty-state tracking. Runtime
//! tree mutations are delegated to [`DocumentTree`](super::tree::DocumentTree)
//! so visible-order metadata stays in sync with every edit.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context as _, anyhow};
use gpui::*;

use super::{
    Editor, ScrollbarDragSession, SmoothScrollAnimation, SplitScrollDriver, TableCellBinding,
    ViewMode, render, render::supports_in_window_menu, tree,
};
use crate::app_menu::dispatch_menu_action_for_editor;
use crate::components::{
    Block, BlockDropPlacement, BlockEvent, BlockKind, BlockRecord, CollapsedCaretAffinity,
    EditingCommandHistory, EditingCommandPlan, EditingViewMode, IndentBlock,
    InlineInsertionAttributes, InlineTextTree, NoRecentFiles, OutdentBlock, PastedImageSource,
    SlashCommand, TableCellPosition, TableData, is_table_row_candidate, parse_root_table_region,
    parse_table_body_row,
};
use crate::config::{ImagePasteBehavior, read_app_preferences};

impl Editor {}

#[path = "events_parts/commands.rs"]
mod commands;
#[path = "events_parts/focus.rs"]
mod focus;
#[path = "input/events/image_insert.rs"]
mod image_insert;
#[path = "events_parts/input.rs"]
mod input;
#[path = "events_parts/preflight.rs"]
mod preflight;
#[path = "input/events/quote_shortcuts.rs"]
mod quote_shortcuts;
#[path = "events_parts/scroll.rs"]
mod scroll;
#[path = "events_parts/smooth_scroll.rs"]
mod smooth_scroll;
#[path = "events_parts/state_machine.rs"]
mod state_machine;
#[path = "input/events/toggle_task.rs"]
mod toggle_task;

#[cfg(test)]
#[path = "../../tests/unit/editor/events.rs"]
mod tests;
