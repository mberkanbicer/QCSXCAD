//! GUI widget abstractions.
//!
//! Defines traits for the tree widget, grid editor, property editor, and
//! primitive editors. Concrete implementations depend on the chosen Rust
//! GUI framework and are intentionally left out of this crate.

use crate::model::{GuiState, Selection};
use csxcad_core::model::Structure;

/// Active viewport drag tool. Kept in the shared presentation state so the
/// command strip, the status bar and the viewport all read one value.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TransformMode {
    #[default]
    Move,
    Rotate,
    Scale,
}

impl TransformMode {
    pub const ALL: [Self; 3] = [Self::Move, Self::Rotate, Self::Scale];

    pub fn label(self) -> &'static str {
        match self {
            Self::Move => "Move",
            Self::Rotate => "Rotate",
            Self::Scale => "Scale",
        }
    }
}

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
