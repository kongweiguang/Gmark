// @author kongweiguang

use super::layout::*;
use super::*;
use crate::editor::document_session::EditorDocumentSession;
use crate::editor::tabs::{DocumentTabSnapshot, TabRecord};
use crate::editor::workspace::document_sidebar_panel_width_for_viewport;
use crate::editor::{DocumentKind, UndoSelectionSnapshot};
use gmark_document::Revision;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::Arc;

#[path = "view_parts/footnote.rs"]
mod footnote;
use footnote::footnote_group_shell;
#[path = "view_parts/accessibility.rs"]
mod accessibility;

#[path = "view_parts/document_frame.rs"]
mod document_frame;
#[path = "view_parts/pane_lifecycle.rs"]
mod pane_lifecycle;
#[path = "view_parts/pane_migration.rs"]
mod pane_migration;
#[path = "view_parts/pane_split.rs"]
mod pane_split;
#[path = "view_parts/render.rs"]
mod render_view;
#[path = "view_parts/shared_events.rs"]
mod shared_events;
#[path = "view_parts/split_surface.rs"]
mod split_surface;

pub(super) fn submenu_panel_top(
    items: &[OwnedMenuItem],
    item_index: usize,
    dimensions: &ThemeDimensions,
) -> f32 {
    let prior_items_height: f32 = items
        .iter()
        .take(item_index)
        .map(|item| menu_item_visual_height(item, dimensions))
        .sum();
    let prior_gaps = dimensions.menu_panel_gap * item_index as f32;
    dimensions.menu_panel_top + dimensions.menu_panel_padding + prior_items_height + prior_gaps
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct MenuSubmenuBridgeGeometry {
    pub(super) left: f32,
    pub(super) top: f32,
    pub(super) width: f32,
    pub(super) height: f32,
}

#[cfg(test)]
pub(super) fn submenu_bridge_geometry<S: AsRef<str>, T: AsRef<str>>(
    open_index: usize,
    menu_labels: &[S],
    items: &[OwnedMenuItem],
    item_index: usize,
    submenu_labels: &[T],
    dimensions: &ThemeDimensions,
) -> Option<MenuSubmenuBridgeGeometry> {
    submenu_bridge_geometry_from_origin(
        0.0,
        open_index,
        menu_labels,
        items,
        item_index,
        submenu_labels,
        dimensions,
    )
}

pub(super) fn submenu_bridge_geometry_from_origin<S: AsRef<str>, T: AsRef<str>>(
    origin_x: f32,
    open_index: usize,
    menu_labels: &[S],
    items: &[OwnedMenuItem],
    item_index: usize,
    submenu_labels: &[T],
    dimensions: &ThemeDimensions,
) -> Option<MenuSubmenuBridgeGeometry> {
    let item = items.get(item_index)?;
    let main_panel_left =
        menu_panel_left_from_origin(origin_x, open_index, menu_labels, dimensions);
    let main_panel_width = menu_panel_width_for_labels(&owned_menu_item_labels(items), dimensions);
    let submenu_width = menu_panel_width_for_labels(submenu_labels, dimensions);
    let vertical_tolerance = dimensions.menu_panel_padding + dimensions.menu_panel_gap;
    let item_top = submenu_panel_top(items, item_index, dimensions);
    let top = (item_top - vertical_tolerance).max(dimensions.menu_panel_top);
    Some(MenuSubmenuBridgeGeometry {
        left: main_panel_left + main_panel_width,
        top,
        width: dimensions.menu_panel_gap + submenu_width,
        height: menu_item_visual_height(item, dimensions) + vertical_tolerance * 2.0,
    })
}

impl Editor {}

#[path = "../render_parts/dialogs.rs"]
mod dialogs;
#[path = "../render_parts/info_dialog.rs"]
mod info_dialog;
#[path = "../render_parts/window_actions.rs"]
mod window_actions;
#[path = "../render_parts/window_state.rs"]
mod window_state;

impl Editor {
    pub(crate) fn on_go_to_line_action(
        &mut self,
        action: &crate::components::GoToLine,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(document_host) = self.document_host.clone() {
            document_host.update(cx, |document_host, cx| {
                document_host.on_go_to_line(action, window, cx);
            });
        }
    }

    pub(in crate::editor) fn activate_document_toolbar_action(
        &mut self,
        action: DocumentToolbarAction,
        position: Option<Point<Pixels>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            DocumentToolbarAction::SplitPane => {
                if let Some(focus) = self.document_toolbar_focus_handles.get(action.index()) {
                    focus.focus(window);
                }
                self.open_split_pane_menu(
                    position.unwrap_or_else(|| window.mouse_position()),
                    Some(window),
                    cx,
                );
            }
            DocumentToolbarAction::QuickOpen => {
                self.on_quick_open_action(&crate::components::QuickOpen, window, cx)
            }
            DocumentToolbarAction::Find => {
                self.on_find_in_document_action(&crate::components::FindInDocument, window, cx)
            }
            DocumentToolbarAction::CommandPalette => {
                self.on_command_palette_action(&crate::components::CommandPalette, window, cx)
            }
        }
    }
}

#[path = "../render_parts/document_view.rs"]
mod document_view;
