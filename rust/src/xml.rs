//! GUI XML read/write for CSXCAD project files.
//!
//! Reads and writes CSXCAD XML documents, including openEMS wrapper
//! boundary-condition attributes, and maps them to/from the GUI state
//! and `csxcad-core` model types.

use csxcad_core::serialization::{StructureXmlError, StructureXmlWarning};
use crate::model::GuiState;
use std::collections::HashMap;

/// Errors specific to GUI XML I/O.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum GuiXmlError {
    #[error("structure XML error: {0}")]
    Structure(#[from] StructureXmlError),
    #[error("missing file: {0}")]
    MissingFile(String),
    #[error("unsupported format: {0}")]
    UnsupportedFormat(String),
}

/// Warnings specific to GUI XML I/O.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum GuiXmlWarning {
    #[error("structure XML warning: {0:?}")]
    Structure(StructureXmlWarning),
    #[error("unknown property retained: {0}")]
    UnknownProperty(String),
    #[error("unknown primitive retained: {0}")]
    UnknownPrimitive(String),
}

impl From<StructureXmlWarning> for GuiXmlWarning {
    fn from(w: StructureXmlWarning) -> Self {
        Self::Structure(w)
    }
}

/// Boundary-condition attributes read from an openEMS XML wrapper.
#[derive(Clone, Debug, Default)]
pub struct BoundaryConditions {
    pub values: HashMap<String, String>,
}

impl BoundaryConditions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(|v| v.as_str())
    }

    pub fn set(&mut self, key: String, value: String) {
        self.values.insert(key, value);
    }
}

/// Read a CSXCAD XML document (optionally wrapped in openEMS XML) into
/// a `csxcad_core` model and GUI state.
pub fn read_gui_xml(
    _xml: &str,
) -> Result<(csxcad_core::model::Structure, GuiState, BoundaryConditions), GuiXmlError> {
    Err(GuiXmlError::UnsupportedFormat(
        "full XML reader not yet implemented".to_string(),
    ))
}

/// Write a `csxcad_core` model and GUI state to a CSXCAD XML document.
pub fn write_gui_xml(
    _structure: &csxcad_core::model::Structure,
    _gui_state: &GuiState,
    _bc: &BoundaryConditions,
) -> Result<String, GuiXmlError> {
    Err(GuiXmlError::UnsupportedFormat(
        "full XML writer not yet implemented".to_string(),
    ))
}
