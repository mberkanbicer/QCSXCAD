//! Rendering abstraction.
//!
//! Defines traits for 2D/3D visualization, camera controls, and image
//! capture. Concrete implementations depend on the chosen rendering
//! backend (VTK bindings, egui, iced, etc.) and are intentionally left
//! out of this crate.

use crate::model::{GuiError, GuiState, ViewMode};
use csxcad_core::model::Structure;

/// A 2D/3D structure renderer: draws primitives, axes, grid planes, and
/// optional discrete-material models.
pub trait StructureRenderer {
    fn render(&mut self, structure: &Structure, state: &GuiState) -> Result<(), GuiError>;
    fn set_view_mode(&mut self, mode: ViewMode);
    fn best_view(&mut self);
    fn set_view_dir(&mut self, dir: i32);
    fn set_parallel_projection(&mut self, enabled: bool);
    fn export_image(&self, path: &str) -> Result<(), GuiError>;
}
