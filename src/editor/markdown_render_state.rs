// @author kongweiguang

//! Editor integration for process-local Markdown presentation state and
//! bounded local-image preparation. The policy/state machines live in their
//! small sibling modules; this file only adapts them to GPUI entities.

use std::collections::HashMap;
use std::path::Path;

use super::*;

fn html_node_id_from_asset_key(
    key: &render_asset_manager::AssetKey,
) -> Option<gmark_markdown::HtmlNodeId> {
    key.identity
        .rsplit_once("#html-node-")
        .and_then(|(_, value)| value.parse::<u64>().ok())
        .map(gmark_markdown::HtmlNodeId)
}

impl Editor {
    pub(crate) fn ensure_markdown_view_state(&mut self) {
        let tab_id = self.tabs.active_id();
        if self.view_state.state_for_tab(tab_id).is_some() {
            return;
        }
        let identity = self
            .file_path
            .as_deref()
            .map(|path| markdown_view_state::MarkdownTabIdentity::saved(path, tab_id))
            .unwrap_or_else(|| markdown_view_state::MarkdownTabIdentity::untitled(tab_id));
        let _ = self.view_state.open_tab(identity);
    }

    /// Rebinds the active tab's presentation state when a different document
    /// is installed. The old snapshot remains available for a future reopen.
    pub(crate) fn reset_markdown_view_state_identity(&mut self, path: Option<&Path>) {
        let tab_id = self.tabs.active_id();
        let _ = self.view_state.close_tab(tab_id);
        let identity = path
            .map(|path| markdown_view_state::MarkdownTabIdentity::saved(path, tab_id))
            .unwrap_or_else(|| markdown_view_state::MarkdownTabIdentity::untitled(tab_id));
        let _ = self.view_state.open_tab(identity);
    }

    /// Applies fold and table-width choices to presentation-only block flags.
    pub(crate) fn sync_rendered_view_state(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.ensure_markdown_view_state();
        let tab_id = self.tabs.active_id();
        let state = self.view_state.state_for_tab(tab_id).unwrap_or_default();
        let blocks = self.document.flatten_visible_blocks();

        let mut heading_path = Vec::new();
        let mut heading_stack = Vec::new();
        let mut heading_ordinals = HashMap::new();
        let mut callout_ordinals = HashMap::new();
        let mut table_ordinals = HashMap::new();
        let mut callout_collapsed = HashMap::new();
        let mut focused_collapsed_owner = None;
        let mut presentation_changed = false;

        for block in &blocks {
            let (kind, title, callout_anchor, focused) =
                block.entity.read_with(cx, |block, _cx| {
                    (
                        block.kind(),
                        block.record.title.visible_text().to_owned(),
                        block.callout_anchor,
                        block.focus_handle.is_focused(window),
                    )
                });
            let mut fold_key = None;
            let mut fold_heading = false;
            let mut collapsed = false;
            let mut hidden_by_heading = heading_stack.iter().any(|(_, value, _)| *value);

            if let BlockKind::Heading { level } = kind {
                while heading_stack
                    .last()
                    .is_some_and(|(parent_level, _, _)| *parent_level >= level)
                {
                    heading_stack.pop();
                    heading_path.pop();
                }
                let path_refs = heading_path.iter().map(String::as_str).collect::<Vec<_>>();
                let ordinal_key = format!("{}\u{1f}{}", path_refs.join("/"), title.trim());
                let ordinal = heading_ordinals.entry(ordinal_key).or_default();
                let key = markdown_view_state::heading_view_key(&path_refs, &title, *ordinal);
                *ordinal += 1;
                collapsed = state.collapsed_headings.get(&key).copied().unwrap_or(false);
                fold_key = Some(key);
                fold_heading = true;
                hidden_by_heading = heading_stack.iter().any(|(_, value, _)| *value);
                heading_stack.push((level, collapsed, block.entity.clone()));
                heading_path.push(title.clone());
            }

            if let BlockKind::Callout(variant) = kind {
                let callout_path = heading_path.iter().map(String::as_str).collect::<Vec<_>>();
                let ordinal_key = format!(
                    "{}\u{1f}{}\u{1f}{}",
                    callout_path.join("/"),
                    variant.marker(),
                    title.trim()
                );
                let ordinal = callout_ordinals.entry(ordinal_key).or_default();
                let key = markdown_view_state::callout_view_key(
                    &callout_path,
                    variant.marker(),
                    &title,
                    *ordinal,
                );
                *ordinal += 1;
                collapsed = state.collapsed_callouts.get(&key).copied().unwrap_or(false);
                fold_key = Some(key);
                if let Some(anchor) = callout_anchor {
                    callout_collapsed.insert(anchor, (collapsed, block.entity.clone()));
                }
            }

            if !matches!(kind, BlockKind::Callout(_))
                && let Some(anchor) = callout_anchor
            {
                if let Some((collapsed, owner)) = callout_collapsed.get(&anchor)
                    && *collapsed
                {
                    hidden_by_heading = true;
                    if focused {
                        focused_collapsed_owner.get_or_insert_with(|| owner.clone());
                    }
                }
            }

            if focused && hidden_by_heading && focused_collapsed_owner.is_none() {
                focused_collapsed_owner = heading_stack
                    .iter()
                    .rev()
                    .find(|(_, collapsed, _)| *collapsed)
                    .map(|(_, _, owner)| owner.clone());
            }

            let (table_key, table_layout) = if let BlockKind::Table = kind {
                let headers = block.entity.read_with(cx, |block, _cx| {
                    block
                        .record
                        .table
                        .as_ref()
                        .map(|table| {
                            table
                                .header
                                .iter()
                                .map(|cell| cell.visible_text().to_owned())
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default()
                });
                let table_path = heading_path.iter().map(String::as_str).collect::<Vec<_>>();
                let ordinal_key = format!("{}\u{1f}{}", table_path.join("/"), headers.join("|"));
                let ordinal = table_ordinals.entry(ordinal_key).or_default();
                let header_refs = headers.iter().map(String::as_str).collect::<Vec<_>>();
                let key = markdown_view_state::table_view_key(&table_path, &header_refs, *ordinal);
                *ordinal += 1;
                let layout = state.table_column_widths.get(&key).and_then(|fractions| {
                    crate::components::TableColumnLayout::from_fractions(fractions)
                });
                (Some(key), layout)
            } else {
                (None, None)
            };

            let changed = block.entity.update(cx, |block, _cx| {
                let next_fold_key = fold_key.clone().map(Into::into);
                let next_table_key = table_key.clone().map(Into::into);
                let next_table_layout = table_layout
                    .clone()
                    .or_else(|| block.table_column_layout.clone());
                let changed = block.presentation_hidden != hidden_by_heading
                    || block.presentation_collapsed != collapsed
                    || block.presentation_fold_key != next_fold_key
                    || block.presentation_fold_heading != fold_heading
                    || block.table_view_key != next_table_key
                    || block.table_column_layout != next_table_layout;
                block.presentation_hidden = hidden_by_heading;
                block.presentation_collapsed = collapsed;
                block.presentation_fold_key = next_fold_key;
                block.presentation_fold_heading = fold_heading;
                block.table_view_key = next_table_key;
                block.table_column_layout = next_table_layout;
                changed
            });
            presentation_changed |= changed;
        }

        // Never leave keyboard focus inside a body which is about to be
        // hidden.  Move focus to the nearest owning heading/Callout while the
        // persisted collapsed choice remains untouched.
        let moved_focus_from_collapsed_body = focused_collapsed_owner.is_some();
        if let Some(owner) = focused_collapsed_owner {
            owner.read(cx).focus_handle.focus(window);
            self.focus_block(owner.entity_id());
        }
        if presentation_changed || moved_focus_from_collapsed_body {
            self.render_row_cache = None;
            self.prev_render_window = None;
            self.row_stride_cache.clear();
        }
    }

    /// Schedules bounded local-image decoding and routes retry requests from
    /// failed placeholders through the generation-safe asset manager.
    pub(crate) fn sync_render_asset_lifecycle(&mut self, window: &Window, cx: &mut Context<Self>) {
        let decode_limit = std::thread::available_parallelism()
            .map(std::num::NonZeroUsize::get)
            .map(render_asset_manager::recommended_decode_concurrency)
            .unwrap_or(1);
        let blocks = self.document.flatten_visible_blocks();
        for visible in blocks {
            let (requests, bounds, retry_requested) = visible.entity.update(cx, |block, _cx| {
                let mut requests = Vec::new();
                if let Some(path) =
                    block
                        .image_runtime()
                        .and_then(|runtime| match &runtime.resolved_source {
                            crate::components::ImageResolvedSource::Local(path) => {
                                Some(path.clone())
                            }
                            crate::components::ImageResolvedSource::Remote(_) => None,
                        })
                {
                    requests.push((None, path));
                }
                requests.extend(
                    block
                        .local_html_image_requests()
                        .into_iter()
                        .map(|request| (Some(request.node_id), request.path)),
                );
                (
                    requests,
                    block.last_bounds,
                    block.take_image_retry_request(),
                )
            });
            for (node_id, path) in requests {
                let canonical = dunce::canonicalize(&path).unwrap_or(path);
                let version = std::fs::metadata(&canonical)
                    .ok()
                    .and_then(|metadata| {
                        let modified = metadata
                            .modified()
                            .ok()?
                            .duration_since(std::time::UNIX_EPOCH)
                            .ok()?;
                        Some(format!("{}:{}", modified.as_nanos(), metadata.len()))
                    })
                    .unwrap_or_else(|| "missing".to_owned());
                let (logical_width, logical_height) = bounds
                    .map(|bounds| (f32::from(bounds.size.width), f32::from(bounds.size.height)))
                    .unwrap_or((512.0, 512.0));
                let (pixel_width, pixel_height) = render_asset_manager::target_pixel_size(
                    logical_width,
                    logical_height,
                    window.scale_factor(),
                );
                let node_suffix = node_id
                    .map(|node_id| format!("#html-node-{}", node_id.0))
                    .unwrap_or_default();
                let key = render_asset_manager::AssetKey::new(
                    format!(
                        // Keep the node discriminator at the end so the
                        // completion path can recover it even when the
                        // canonical Windows path contains ':' or '#'.
                        "doc/{}/{}/{}{}",
                        self.render_asset_scope,
                        self.document_epoch,
                        canonical.display(),
                        node_suffix,
                    ),
                    version,
                    pixel_width,
                    pixel_height,
                );
                if self.render_asset_tasks.contains_key(&key) {
                    let state = self.render_assets.state(&key);
                    visible.entity.update(cx, |block, _cx| {
                        if let Some(node_id) = node_id {
                            block.bind_html_image_asset_state(node_id, key.clone(), state.clone());
                        } else {
                            block.bind_image_asset_state(key.clone(), state.clone());
                        }
                    });
                    continue;
                }
                if !retry_requested && self.render_assets.entry(&key).is_some() {
                    let state = self.render_assets.state(&key);
                    visible.entity.update(cx, |block, _cx| {
                        if let Some(node_id) = node_id {
                            block.bind_html_image_asset_state(node_id, key.clone(), state.clone());
                        } else {
                            block.bind_image_asset_state(key.clone(), state.clone());
                        }
                    });
                    continue;
                }
                if self.render_asset_tasks.len() >= decode_limit {
                    // Keep the explicit idle state until a worker completes. The
                    // completion notifies the editor and opens the next slot.
                    continue;
                }
                let token = self.render_assets.begin_load(key.clone());
                let loading_state = self.render_assets.state(&key);
                visible.entity.update(cx, |block, _cx| {
                    if let Some(node_id) = node_id {
                        block.bind_html_image_asset_state(node_id, key.clone(), loading_state);
                    } else {
                        block.bind_image_asset_state(key.clone(), loading_state);
                    }
                });
                let task_key = key.clone();
                let task = cx.spawn(async move |this: WeakEntity<Editor>, cx| {
                    let result = cx
                        .background_spawn(async move {
                            if crate::document_io::is_svg_path(&canonical) {
                                render_asset_manager::decode_local_svg(
                                    &canonical,
                                    (pixel_width, pixel_height),
                                )
                            } else {
                                render_asset_manager::decode_local_image(
                                    &canonical,
                                    (pixel_width, pixel_height),
                                )
                            }
                        })
                        .await;
                    let _ = this.update(cx, |editor, cx| {
                        editor.render_asset_tasks.remove(&task_key);
                        match result {
                            Ok(value) => {
                                let _ = editor.render_assets.complete(&task_key, token, value);
                            }
                            Err(error) => {
                                let _ =
                                    editor
                                        .render_assets
                                        .fail(&task_key, token, error.to_string());
                            }
                        }
                        let state = editor.render_assets.state(&task_key);
                        let html_node_id = html_node_id_from_asset_key(&task_key);
                        for visible in editor.document.flatten_visible_blocks() {
                            visible.entity.update(cx, |block, _cx| {
                                if let Some(node_id) = html_node_id {
                                    block.set_html_image_asset_state(
                                        node_id,
                                        &task_key,
                                        state.clone(),
                                    );
                                } else {
                                    block.set_image_asset_state(task_key.clone(), state.clone());
                                }
                            });
                        }
                        cx.notify();
                    });
                });
                self.render_asset_tasks.insert(key, (task, token));
            }
        }
    }

    pub(crate) fn toggle_rendered_collapse(
        &mut self,
        key: &str,
        heading: bool,
        cx: &mut Context<Self>,
    ) {
        self.ensure_markdown_view_state();
        let tab_id = self.tabs.active_id();
        let current = self
            .view_state
            .state_for_tab(tab_id)
            .map(|state| {
                if heading {
                    state.collapsed_headings.get(key).copied().unwrap_or(false)
                } else {
                    state.collapsed_callouts.get(key).copied().unwrap_or(false)
                }
            })
            .unwrap_or(false);
        let next_collapsed = !current;
        if heading {
            self.view_state
                .set_heading_collapsed(tab_id, key.to_owned(), next_collapsed);
        } else {
            self.view_state
                .set_callout_collapsed(tab_id, key.to_owned(), next_collapsed);
        }
        if next_collapsed {
            // A collapse action must never leave keyboard focus inside the
            // hidden body. Move the pending focus to the owning heading or
            // Callout before the next layout pass hides descendants.
            if let Some(owner) =
                self.document
                    .flatten_visible_blocks()
                    .into_iter()
                    .find(|visible| {
                        visible
                            .entity
                            .read(cx)
                            .presentation_fold_key
                            .as_ref()
                            .is_some_and(|fold_key| fold_key == key)
                    })
            {
                self.focus_block(owner.entity.entity_id());
            }
        }
        self.render_row_cache = None;
        self.prev_render_window = None;
        self.row_stride_cache.clear();
        cx.notify();
    }

    pub(crate) fn persist_table_column_layout(
        &mut self,
        key: &str,
        fractions: Vec<f32>,
        cx: &mut Context<Self>,
    ) {
        self.ensure_markdown_view_state();
        self.view_state
            .set_table_layout(self.tabs.active_id(), key.to_owned(), fractions);
        cx.notify();
    }

    pub(crate) fn reset_table_column_layout(&mut self, key: &str, cx: &mut Context<Self>) {
        self.ensure_markdown_view_state();
        self.view_state
            .remove_table_layout(self.tabs.active_id(), key);
        cx.notify();
    }

    pub(crate) fn release_render_assets_for_active_document(&mut self, cx: &mut Context<Self>) {
        self.release_image_preview_assets(cx);
        let prefix = format!("doc/{}/{}/", self.render_asset_scope, self.document_epoch);
        self.cancel_render_asset_tasks(Some(&prefix));
        self.render_assets.close_document(&prefix);
        self.render_asset_tasks
            .retain(|key, _| !key.identity.starts_with(&prefix));
    }

    /// Releases all render assets owned by this editor during teardown.
    pub(crate) fn release_all_render_assets(&mut self) {
        self.cancel_render_asset_tasks(None);
        self.render_asset_tasks.clear();
        let prefix = format!("doc/{}/{}/", self.render_asset_scope, self.document_epoch);
        self.render_assets.close_document(&prefix);
    }

    /// Invalidates in-flight generations before dropping their GPUI tasks.
    /// A task may already be queued on the executor when the map entry is
    /// removed, so cancellation must happen at the manager boundary as well
    /// as by dropping the task handle.
    fn cancel_render_asset_tasks(&mut self, prefix: Option<&str>) {
        let cancellations = self
            .render_asset_tasks
            .iter()
            .filter(|(key, _)| prefix.is_none_or(|prefix| key.identity.starts_with(prefix)))
            .map(|(key, (_, token))| (key.clone(), *token))
            .collect::<Vec<_>>();
        for (key, token) in cancellations {
            let _ = self.render_assets.cancel(&key, token);
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/editor/markdown_render_state.rs"]
mod tests;
