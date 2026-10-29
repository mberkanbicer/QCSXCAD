//! egui + wgpu GUI implementation for QCSXCAD.
//!
//! Provides concrete implementations of the `qcsxcad-core` widget and
//! rendering traits using egui for the UI and wgpu for 3D rendering.

pub mod app;
pub mod renderer;
pub mod widgets;

pub use app::{trace_input, EguiApp, TransformMode};
pub use renderer::WgpuRenderer;
pub use widgets::{
    EguiGridEditor, EguiPrimitiveEditor, EguiPropertyEditor, EguiTreeWidget,
};