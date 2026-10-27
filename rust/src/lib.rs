//! Rust foundations for the QCSXCAD GUI and AppCSXCAD.
//!
//! This crate holds everything the desktop applications need without a
//! toolkit dependency:
//!
//! - `document`: the editable CSXCAD model (property/primitive/grid/parameter
//!   edits with undo/redo), shared by the widgets and the tests;
//! - `tree`: the native structure-tree labels and rows;
//! - `tessellate`: the ported `VTKPrimitives` geometry mapping;
//! - `export`: the VTK/STL/PLY/POV-Ray/X3D exporters;
//! - `project`: native CSXCAD/openEMS project reading and writing;
//! - `model`, `widgets`, `rendering`: GUI state and the tree/renderer seams.
//!
//! The concrete toolkit implementation lives in the `qcsxcad-egui` crate.

pub mod discrete;
pub mod document;
pub mod export;
pub mod model;
pub mod project;
pub mod rendering;
pub mod tessellate;
pub mod tree;
pub mod widgets;

pub use discrete::surface_mesh;
pub use document::{parse_frequency_list, Document, PropertyEdit, CREATABLE_PROPERTY_TYPES};
pub use export::{export_povray, export_properties, export_x3d, PropertyFormat};
pub use model::{GuiError, GuiState, Selection};
pub use project::ProjectFile;
pub use tessellate::{Mesh, PrimitiveMesh, Tessellator};
pub use tree::{primitive_label, property_label, tree_rows, TreeItem, TreeRow};
