//! Rust foundations for the QCSXCAD GUI.
//!
//! This crate provides the data model, XML I/O, and widget/rendering
//! abstractions needed to port QCSXCAD and AppCSXCAD from C++/Qt/VTK to Rust.
//!
//! The GUI framework and rendering backend are intentionally left abstract
//! here so the integration can choose a Rust-native solution (egui, iced,
//! gtk, qt-rs, etc.) without coupling the data model to a specific toolkit.

pub mod model;
pub mod xml;
pub mod widgets;
pub mod rendering;

pub use model::{GuiError, GuiState, Selection};
pub use xml::{GuiXmlError, GuiXmlWarning, read_gui_xml, write_gui_xml};
