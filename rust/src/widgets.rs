//! GUI widget abstractions.
//!
//! Defines traits for the tree widget, grid editor, property editor, and
//! primitive editors. Concrete implementations depend on the chosen Rust
//! GUI framework and are intentionally left out of this crate.

use crate::model::{GuiState, Selection};
use csxcad_core::model::Structure;

/// A structure/property tree widget: displays primitives grouped under
/// properties, supports selection, expand/collapse, and context menus.
///
/// The concrete widget owns the model edits (see `qcsxcad_core::document`);
/// this trait only exposes the tree state the application has to keep.
pub trait TreeWidget {
    fn refresh(&mut self, structure: &Structure, state: &GuiState);
    fn selection(&self) -> Selection;
    fn set_selection(&mut self, selection: Selection);
}
