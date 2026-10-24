//! GUI XML read/write for CSXCAD project files.
//!
//! Reads and writes CSXCAD XML documents, including openEMS wrapper
//! boundary-condition attributes, and maps them to/from the GUI state
//! and `csxcad-core` model types.

use csxcad_core::serialization::{StructureXmlError, StructureXmlWarning};
use crate::model::{GuiState, SimMode, ViewMode};
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

/// Extract the content between opening and closing XML tags.
fn extract_tag_content(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{}>", tag);
    let close = format!("</{}>", tag);
    let start = xml.find(&open)?;
    let start = start + open.len();
    let end = xml[start..].find(&close)?;
    Some(xml[start..start + end].to_string())
}

/// Extract an attribute value from an XML tag.
fn extract_attribute(xml: &str, tag: &str, attr: &str) -> Option<String> {
    let pattern = format!(r#"{0} {1}="#, tag, attr);
    let start = xml.find(&pattern)?;
    let start = start + pattern.len();
    let end = xml[start..].find('"')?;
    Some(xml[start..start + end].to_string())
}

/// Read a CSXCAD XML document (optionally wrapped in openEMS XML) into
/// a `csxcad_core` model and GUI state.
pub fn read_gui_xml(
    xml: &str,
) -> Result<(csxcad_core::model::Structure, GuiState, BoundaryConditions), GuiXmlError> {
    // Extract the structure XML
    let structure_xml = extract_tag_content(xml, "Structure")
        .ok_or_else(|| GuiXmlError::UnsupportedFormat("missing <Structure> element".to_string()))?;

    let structure = csxcad_core::model::Structure::from_xml_str(&structure_xml)
        .map_err(GuiXmlError::Structure)?;

    // Extract GUI state
    let mut gui_state = GuiState::new();
    if let Some(gui_xml) = extract_tag_content(xml, "GuiState") {
        if let Some(vm) = extract_attribute(&gui_xml, "GuiState", "view_mode") {
            gui_state.view_mode = match vm.as_str() {
                "View3D" => ViewMode::View3D,
                _ => ViewMode::View2D,
            };
        }
        if let Some(sm) = extract_attribute(&gui_xml, "GuiState", "sim_mode") {
            gui_state.sim_mode = match sm.as_str() {
                "TimeDomain" => SimMode::TimeDomain,
                "FrequencyDomain" => SimMode::FrequencyDomain,
                _ => SimMode::None,
            };
        }
        if let Some(rdm) = extract_attribute(&gui_xml, "GuiState", "render_disc_models") {
            gui_state.render_disc_models = rdm == "true";
        }
        if let Some(pp) = extract_attribute(&gui_xml, "GuiState", "parallel_projection") {
            gui_state.parallel_projection = pp == "true";
        }
        if let Some(vd) = extract_attribute(&gui_xml, "GuiState", "view_dir") {
            gui_state.view_dir = vd.parse().unwrap_or(0);
        }
        if let Some(mod_) = extract_attribute(&gui_xml, "GuiState", "modified") {
            gui_state.modified = mod_ == "true";
        }
    }

    // Extract boundary conditions
    let mut bc = BoundaryConditions::new();
    if let Some(bc_xml) = extract_tag_content(xml, "BoundaryConditions") {
        // Parse key="value" pairs from BC elements
        let mut remaining = bc_xml.as_str();
        while let Some(bc_start) = remaining.find("<BC ") {
            let bc_section = &remaining[bc_start..];
            if let Some(bc_end) = bc_section.find("/>") {
                let bc_tag = &bc_section[..bc_end + 2];
                if let Some(key) = extract_attribute(bc_tag, "BC", "key") {
                    if let Some(value) = extract_attribute(bc_tag, "BC", "value") {
                        bc.set(key, value);
                    }
                }
                remaining = &bc_section[bc_end + 2..];
            } else {
                break;
            }
        }
    }

    Ok((structure, gui_state, bc))
}

/// Write a `csxcad_core` model and GUI state to a CSXCAD XML document.
pub fn write_gui_xml(
    structure: &csxcad_core::model::Structure,
    gui_state: &GuiState,
    bc: &BoundaryConditions,
) -> Result<String, GuiXmlError> {
    let structure_xml = structure.to_xml_string().map_err(GuiXmlError::Structure)?;

    let view_mode_str = match gui_state.view_mode {
        ViewMode::View3D => "View3D",
        ViewMode::View2D => "View2D",
    };
    let sim_mode_str = match gui_state.sim_mode {
        SimMode::TimeDomain => "TimeDomain",
        SimMode::FrequencyDomain => "FrequencyDomain",
        SimMode::None => "None",
    };

    let mut bc_xml = String::new();
    for (key, value) in &bc.values {
        bc_xml.push_str(&format!("<BC key=\"{}\" value=\"{}\"/>", key, value));
    }

    Ok(format!(
        r#"<GuiXml>
  <Structure>{}</Structure>
  <GuiState view_mode="{}" sim_mode="{}" render_disc_models="{}" parallel_projection="{}" view_dir="{}" modified="{}"/>
  <BoundaryConditions>{}</BoundaryConditions>
</GuiXml>"#,
        structure_xml,
        view_mode_str,
        sim_mode_str,
        gui_state.render_disc_models,
        gui_state.parallel_projection,
        gui_state.view_dir,
        gui_state.modified,
        bc_xml
    ))
}
