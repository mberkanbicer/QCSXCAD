//! GUI-specific data model: view state, selection, and editor state.
//!
//! This layer sits on top of `csxcad-core`'s `Structure`, `Property`,
//! and `Primitive` types and adds GUI-only concerns such as which items
//! are selected, which properties are visible, and the current view mode.

use csxcad_core::model::{PrimitiveId, PropertyId};
use csxcad_core::parameter::ParameterError;
use std::collections::HashSet;

/// The active view mode: 2D orthographic or 3D.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ViewMode {
    #[default]
    View2D,
    View3D,
}

/// Simulation mode for the GUI.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SimMode {
    #[default]
    None,
    TimeDomain,
    FrequencyDomain,
}

/// Currently selected items in the structure tree.
#[derive(Clone, Debug, Default)]
pub struct Selection {
    pub properties: HashSet<PropertyId>,
    pub primitives: HashSet<PrimitiveId>,
}

impl Selection {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.properties.is_empty() && self.primitives.is_empty()
    }

    pub fn clear(&mut self) {
        self.properties.clear();
        self.primitives.clear();
    }
}

/// Overall GUI state: view mode, simulation mode, selection, visibility.
#[derive(Clone, Debug, Default)]
pub struct GuiState {
    pub view_mode: ViewMode,
    pub sim_mode: SimMode,
    pub selection: Selection,
    pub render_disc_models: bool,
    pub parallel_projection: bool,
    pub view_dir: i32,
    pub modified: bool,
}

impl GuiState {
    pub fn new() -> Self {
        Self::default()
    }
}

/// Errors that can occur during GUI operations.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum GuiError {
    #[error("no item selected")]
    NoSelection,
    #[error("invalid primitive type: {0}")]
    InvalidPrimitiveType(String),
    #[error("invalid property type: {0}")]
    InvalidPropertyType(String),
    #[error("model error: {0}")]
    Model(String),
    #[error("invalid value: {0}")]
    InvalidValue(String),
    #[error("parameter error: {0}")]
    Parameter(String),
    #[error("xml error: {0}")]
    Xml(String),
    #[error("io error: {0}")]
    Io(String),
}

impl From<ParameterError> for GuiError {
    fn from(error: ParameterError) -> Self {
        GuiError::Parameter(error.to_string())
    }
}
