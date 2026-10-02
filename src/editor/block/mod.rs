// @author kongweiguang

//! Block runtime and semantic state.
//!
//! This module groups the block entity itself, block-level Markdown parsing,
//! inline text-tree handling, rendering, input bridging, and interaction
//! handlers. A block owns local editing state while the editor owns tree
//! structure and cross-block mutations.

mod editing_command;
mod element;
mod input;
mod interactions;
mod line_edit;
mod pointer_selection;
mod render;
mod runtime;
mod selection_toolbar;
mod slash_command;
mod state;

pub(crate) use crate::components::markdown::code_highlight::*;
pub(crate) use crate::components::markdown::footnote::*;
pub(crate) use crate::components::markdown::image::*;
pub use crate::components::markdown::inline::*;
pub(crate) use crate::components::markdown::link::*;
pub(crate) use editing_command::*;
pub(crate) use element::source_line_number_gutter_width;
pub(crate) use input::{BlockImeComposition, BlockImeCompositionOwner};
pub(crate) use line_edit::{LineOperation, plan_line_operation};
pub(crate) use pointer_selection::{PointerSelectionGranularity, PointerSelectionSession};
pub(crate) use render::rendered_content_inset;
pub use runtime::*;
pub(crate) use slash_command::*;
pub use state::*;
pub(crate) use state::{BlockImeInteraction, BlockInputCommand};
