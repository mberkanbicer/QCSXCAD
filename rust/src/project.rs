//! Project file handling: native CSXCAD XML and openEMS wrapper documents.
//!
//! Native `QCSXCAD::ReadFile` accepts a bare CSXCAD document and an openEMS
//! wrapper document (the solver settings live next to `<ContinuousStructure>`).
//! Native `Write2XML` writes only the `ContinuousStructure` element, so saving
//! an opened solver project drops every openEMS setting. This port keeps the
//! wrapper: `ProjectFile` remembers the source document and substitutes the
//! regenerated structure into it. The decision and its rationale are recorded
//! in `docs/migration/audits/gui.md`.

use std::path::{Path, PathBuf};

use crate::document::Document;
use crate::model::GuiError;

const STRUCTURE_TAG: &str = "ContinuousStructure";

/// The file a `Document` was loaded from, plus the openEMS wrapper it had.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProjectFile {
    path: Option<PathBuf>,
    /// Original file content, kept so the openEMS wrapper survives a save.
    source: Option<String>,
}

impl ProjectFile {
    pub fn new() -> Self {
        Self::default()
    }

    /// A file that was read from a string (no path yet).
    pub fn from_xml(xml: impl Into<String>) -> Self {
        Self {
            path: None,
            source: Some(xml.into()),
        }
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn set_path(&mut self, path: Option<PathBuf>) {
        self.path = path;
    }

    /// True when the loaded document had an openEMS wrapper.
    pub fn has_openems_wrapper(&self) -> bool {
        self.source
            .as_deref()
            .map(|xml| xml.contains("<openEMS"))
            .unwrap_or(false)
    }

    /// Read a project file. Returns the document and the file handle.
    pub fn read_file(path: impl AsRef<Path>) -> Result<(Document, Self), GuiError> {
        let path = path.as_ref();
        let xml = std::fs::read_to_string(path).map_err(|error| {
            GuiError::Io(format!("could not read {}: {error}", path.display()))
        })?;
        let (document, mut file) = Self::read_str(&xml)?;
        file.path = Some(path.to_path_buf());
        Ok((document, file))
    }

    /// Parse a project document.
    pub fn read_str(xml: &str) -> Result<(Document, Self), GuiError> {
        if !xml.contains(&format!("<{STRUCTURE_TAG}")) {
            return Err(GuiError::Xml(format!(
                "missing <{STRUCTURE_TAG}> element"
            )));
        }
        let structure = csxcad_core::model::Structure::from_xml_str(xml)
            .map_err(|error| GuiError::Xml(error.to_string()))?;
        Ok((
            Document::from_structure(structure),
            Self {
                path: None,
                source: Some(xml.to_owned()),
            },
        ))
    }

    /// Render the document the way it will be written.
    ///
    /// A source document with a `<ContinuousStructure>` element keeps every
    /// other element (openEMS settings, boundary conditions, ...); the
    /// structure element itself is replaced by the current model.
    pub fn render_xml(&self, document: &Document) -> Result<String, GuiError> {
        let structure_xml = document
            .structure()
            .to_xml_string()
            .map_err(|error| GuiError::Xml(error.to_string()))?;
        if let Some(source) = self.source.as_deref() {
            // the declaration belongs to the file, not to the nested element
            if let Some(rendered) = replace_structure(source, strip_declaration(&structure_xml)) {
                if source.trim_start().starts_with("<?xml") {
                    return Ok(rendered);
                }
                // a structure-only file still gets the native declaration
                if let Some(end) = structure_xml.find("?>") {
                    return Ok(format!("{}\n{rendered}", &structure_xml[..end + 2]));
                }
                return Ok(rendered);
            }
        }
        Ok(structure_xml)
    }

    /// Write the document to its current path.
    pub fn save(&mut self, document: &mut Document) -> Result<(), GuiError> {
        let Some(path) = self.path.clone() else {
            return Err(GuiError::Model(
                "the project has no path; use Save As".into(),
            ));
        };
        self.write(document, &path)
    }

    /// Write the document to `path` and remember it as the project path.
    pub fn write(&mut self, document: &mut Document, path: impl AsRef<Path>) -> Result<(), GuiError> {
        let path = path.as_ref();
        let xml = self.render_xml(document)?;
        std::fs::write(path, &xml).map_err(|error| {
            GuiError::Io(format!("could not write {}: {error}", path.display()))
        })?;
        self.path = Some(path.to_path_buf());
        self.source = Some(xml);
        document.set_modified(false);
        Ok(())
    }
}

/// Drop a leading `<?xml ...?>` declaration (the document already has one).
fn strip_declaration(xml: &str) -> &str {
    if let Some(rest) = xml.strip_prefix("<?xml") {
        if let Some(end) = rest.find("?>") {
            return rest[end + 2..].trim_start_matches(['\r', '\n']);
        }
    }
    xml
}

/// Replace the `<ContinuousStructure>` element of `source` with `structure_xml`.
fn replace_structure(source: &str, structure_xml: &str) -> Option<String> {
    let open = source.find(&format!("<{STRUCTURE_TAG}"))?;
    let open_end = open + source[open..].find('>')? + 1;
    let close = source.rfind(&format!("</{STRUCTURE_TAG}>"))?;
    if close < open_end {
        return None;
    }
    let close_end = close + format!("</{STRUCTURE_TAG}>").len();
    Some(format!(
        "{}{}{}",
        &source[..open],
        structure_xml,
        &source[close_end..]
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use csxcad_core::model::{PrimitiveShape, PrimitiveType, PropertyType};

    const GEOMETRY: &str = r#"<ContinuousStructure CoordSystem="0">
  <Properties>
    <Metal Name="ground">
      <Primitives>
        <Box Priority="1">
          <P1 X="-1" Y="-1" Z="-1"/>
          <P2 X="1" Y="1" Z="0"/>
        </Box>
      </Primitives>
    </Metal>
  </Properties>
</ContinuousStructure>"#;

    const WRAPPED: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<openEMS>
  <FDTD NumberOfTimesteps="1000" EndCriteria="1e-5"/>
  <BoundaryCond xmin="PML_8" xmax="PML_8" ymin="PML_8" ymax="PML_8" zmin="PEC" zmax="PEC"/>
  <ContinuousStructure CoordSystem="0">
    <Properties>
      <Metal Name="ground">
        <Primitives>
          <Box Priority="1">
          <P1 X="-1" Y="-1" Z="-1"/>
          <P2 X="1" Y="1" Z="0"/>
        </Box>
        </Primitives>
      </Metal>
    </Properties>
  </ContinuousStructure>
  <Excitation Type="0" f0="1e9"/>
</openEMS>"#;

    #[test]
    fn reads_a_bare_structure_and_writes_it_back() {
        let (mut document, file) = ProjectFile::read_str(GEOMETRY).expect("read");
        assert_eq!(document.structure().property_count(), 1);
        assert!(!file.has_openems_wrapper());
        let xml = file.render_xml(&document).expect("render");
        // a bare structure keeps its own XML declaration
        assert!(xml.starts_with("<?xml"));
        assert_eq!(xml.matches("<?xml").count(), 1);
        assert!(xml.contains("<ContinuousStructure"));
        let reloaded = csxcad_core::model::Structure::from_xml_str(&xml).expect("reload");
        assert_eq!(reloaded, *document.structure());

        // an edit appears in the written document
        let id = document.structure().property_by_index(0).expect("property").id().expect("id");
        document
            .apply_property_edit(id, crate::document::PropertyEdit::Name("renamed".into()))
            .expect("rename");
        let xml = file.render_xml(&document).expect("render");
        assert!(xml.contains("Name=\"renamed\""));
    }

    #[test]
    fn an_openems_wrapper_is_preserved_on_save() {
        let (mut document, file) = ProjectFile::read_str(WRAPPED).expect("read");
        assert!(file.has_openems_wrapper());
        let id = document.structure().property_by_index(0).expect("property").id().expect("id");
        document
            .apply_property_edit(id, crate::document::PropertyEdit::Name("shield".into()))
            .expect("rename");
        let xml = file.render_xml(&document).expect("render");
        assert!(xml.contains("<openEMS>"));
        assert!(xml.contains("<FDTD NumberOfTimesteps=\"1000\" EndCriteria=\"1e-5\"/>"));
        assert!(xml.contains("<BoundaryCond"));
        assert!(xml.contains("<Excitation Type=\"0\" f0=\"1e9\"/>"));
        assert!(xml.contains("Name=\"shield\""));
        // exactly one structure element and one declaration
        assert_eq!(xml.matches("<ContinuousStructure").count(), 1);
        assert_eq!(xml.matches("</ContinuousStructure>").count(), 1);
        assert_eq!(xml.matches("<?xml").count(), 1);
    }

    #[test]
    fn saving_updates_the_path_and_clears_modified() {
        let directory = std::env::temp_dir().join(format!(
            "qcsxcad-project-test-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&directory).expect("temp dir");
        let path = directory.join("project.xml");
        let (mut document, mut file) = ProjectFile::read_str(WRAPPED).expect("read");
        assert!(document.is_modified() == false);
        document
            .add_property(PropertyType::Material)
            .expect("property");
        assert!(document.is_modified());
        file.write(&mut document, &path).expect("write");
        assert!(!document.is_modified());
        assert_eq!(file.path(), Some(path.as_path()));
        let written = std::fs::read_to_string(&path).expect("read back");
        assert!(written.contains("<openEMS>"));
        assert!(written.contains("<FDTD"));
        let (reloaded, reloaded_file) = ProjectFile::read_file(&path).expect("reload");
        assert_eq!(reloaded.structure().property_count(), 2);
        assert!(reloaded_file.has_openems_wrapper());
        // a subsequent save without an explicit path writes the same file
        file.save(&mut document).expect("save");
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn missing_files_and_malformed_documents_are_reported() {
        let path = std::env::temp_dir().join("qcsxcad-missing-project.xml");
        std::fs::remove_file(&path).ok();
        let error = ProjectFile::read_file(&path).expect_err("missing file");
        assert!(matches!(error, GuiError::Io(_)));
        let error = ProjectFile::read_str("<openEMS/>").expect_err("no structure");
        assert!(matches!(error, GuiError::Xml(_)));
        let error = ProjectFile::read_str(
            "<ContinuousStructure><Properties><Metal><Primitives><Box Priority=\"0\"/></Primitives></Metal></Properties></ContinuousStructure>",
        )
        .expect_err("incomplete primitive");
        assert!(matches!(error, GuiError::Xml(_)));
        let mut file = ProjectFile::new();
        let mut document = Document::new();
        assert!(file.save(&mut document).is_err());
    }

    #[test]
    fn structure_replacement_keeps_surrounding_text() {
        let rendered = replace_structure("a<ContinuousStructure>x</ContinuousStructure>b", "<ContinuousStructure>y</ContinuousStructure>")
            .expect("replace");
        assert_eq!(
            rendered,
            "a<ContinuousStructure>y</ContinuousStructure>b"
        );
        assert!(replace_structure("no structure here", "<ContinuousStructure/>").is_none());
    }

    #[test]
    fn a_document_round_trips_through_the_file_layer() {
        let (mut document, file) = ProjectFile::read_str(WRAPPED).expect("read");
        let property = document.add_property(PropertyType::Metal).expect("property");
        let primitive = document
            .add_primitive(property, PrimitiveType::Box)
            .expect("primitive");
        document
            .edit_primitive(primitive, |primitive| {
                if let PrimitiveShape::Box(shape) = primitive.shape_mut() {
                    shape.set_coordinate(1, 2.0);
                }
            })
            .expect("edit");
        let xml = file.render_xml(&document).expect("render");
        let (reloaded, _) = ProjectFile::read_str(&xml).expect("reload");
        assert_eq!(reloaded.structure(), document.structure());
    }
}
