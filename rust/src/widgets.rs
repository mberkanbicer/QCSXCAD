//! GUI widget abstractions.
//!
//! Defines traits for the tree widget, grid editor, property editor, and
//! primitive editors. Concrete implementations depend on the chosen Rust
//! GUI framework and are intentionally left out of this crate.

use crate::model::{GuiError, GuiState, Selection};
use csxcad_core::model::Structure;

/// A structure/property tree widget: displays primitives grouped under
/// properties, supports selection, expand/collapse, and context menus.
pub trait TreeWidget {
    fn refresh(&mut self, structure: &Structure, state: &GuiState);
    fn selection(&self) -> Selection;
    fn set_selection(&mut self, selection: Selection);
}

/// A grid editor: axis line entry, length units, homogeneous/inhomogeneous
/// discretization, resolution increase, edge detection, grid-plane selection.
pub trait GridEditor {
    fn refresh(&mut self, structure: &Structure);
    fn edit_grid(&mut self) -> Result<(), GuiError>;
}

/// A property editor: specialized widgets for material, excitation, probe,
/// dump, and resolution-box properties, plus generic name/color controls.
pub trait PropertyEditor {
    fn refresh(&mut self, structure: &Structure, state: &GuiState);
    fn edit_property(&mut self, property_id: csxcad_core::model::PropertyId) -> Result<(), GuiError>;
}

/// A primitive editor: box, multi-box, sphere, cylinder, cylindrical shell,
/// polygon, and user-defined primitive forms.
pub trait PrimitiveEditor {
    fn refresh(&mut self, structure: &Structure, state: &GuiState);
    fn create_primitive(&mut self, kind: &str) -> Result<(), GuiError>;
    fn edit_primitive(&mut self, primitive_id: csxcad_core::model::PrimitiveId) -> Result<(), GuiError>;
}
