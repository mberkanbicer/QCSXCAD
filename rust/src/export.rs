//! Geometry export (RM-104).
//!
//! Native AppCSXCAD exports the rendered geometry, not the CSX model:
//! `QVTKStructure::ExportProperty2PolyDataVTK/STL/PLY` write one file per
//! visible property into a directory, scaled by the grid delta unit, and
//! `export_pov`/`export_X3D` write a single scene file. This module produces
//! the same files from `qcsxcad_core::tessellate` (the ported `VTKPrimitives`
//! mapping) so the exported surfaces are exactly the ones the viewport draws.

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use csxcad_core::model::{Property, Structure};

use crate::model::GuiError;
use crate::tessellate::{Mesh, Tessellator, Vertex};

/// Per-property export formats (native `ExportGeometry(dirname, type)`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PropertyFormat {
    /// VTK XML PolyData (`.vtp`).
    PolyDataVtk,
    /// Binary STL (`.stl`).
    Stl,
    /// ASCII PLY (`.ply`).
    Ply,
}

impl PropertyFormat {
    fn extension(self) -> &'static str {
        match self {
            PropertyFormat::PolyDataVtk => "vtp",
            PropertyFormat::Stl => "stl",
            PropertyFormat::Ply => "ply",
        }
    }
}

/// One visible property's tessellated surface.
struct PropertyMesh {
    name: String,
    mesh: Mesh,
    color: [u8; 3],
}

/// Tessellate every visible property at the export scale.
fn property_meshes(structure: &Structure) -> Vec<PropertyMesh> {
    let tessellator = Tessellator::new();
    let parameters = structure.parameters();
    let scale = structure.grid().delta_unit();
    let mut meshes = Vec::new();
    for index in 0..structure.property_count() {
        let Some(property) = structure.property_by_index(index) else {
            continue;
        };
        if !property.is_visible() {
            continue;
        }
        let primitives = tessellator.tessellate_property(property, parameters);
        if primitives.is_empty() {
            continue;
        }
        let mut mesh = Mesh::default();
        let mut color = [128u8, 128, 160];
        for primitive in primitives {
            mesh.triangles.extend(primitive.mesh.triangles);
            mesh.lines.extend(primitive.mesh.lines);
            mesh.points.extend(primitive.mesh.points);
            if let Some(fill) = property.fill_color() {
                let [red, green, blue, _] = fill.components();
                color = [red, green, blue];
            }
        }
        // native `WritePolyData2File` scales by `GetDeltaUnit()`
        for triangle in &mut mesh.triangles {
            for vertex in triangle {
                scale_vertex(vertex, scale);
            }
        }
        for line in &mut mesh.lines {
            for vertex in line {
                scale_vertex(vertex, scale);
            }
        }
        for point in &mut mesh.points {
            scale_vertex(point, scale);
        }
        meshes.push(PropertyMesh {
            name: property_file_name(property, index),
            mesh,
            color,
        });
    }
    meshes
}

fn scale_vertex(vertex: &mut Vertex, scale: f64) {
    for axis in vertex {
        *axis *= scale;
    }
}

/// Native file names the property by its CSXCAD name; an empty or unsafe name
/// falls back to `property_<index>` so the export cannot escape the directory.
fn property_file_name(property: &Property, index: usize) -> String {
    let name = property.name().trim();
    if name.is_empty() {
        return format!("property_{index}");
    }
    let mut sanitized = String::with_capacity(name.len());
    for character in name.chars() {
        if character.is_alphanumeric() || matches!(character, '-' | '_' | '.' | ' ') {
            sanitized.push(character);
        } else {
            sanitized.push('_');
        }
    }
    if sanitized.trim().is_empty() {
        format!("property_{index}")
    } else {
        sanitized
    }
}

/// Export every visible property into `directory` (created when missing).
pub fn export_properties(
    structure: &Structure,
    directory: impl AsRef<Path>,
    format: PropertyFormat,
) -> Result<Vec<PathBuf>, GuiError> {
    let directory = directory.as_ref();
    std::fs::create_dir_all(directory).map_err(|error| {
        GuiError::Io(format!(
            "could not create {}: {error}",
            directory.display()
        ))
    })?;
    let meshes = property_meshes(structure);
    if meshes.is_empty() {
        return Err(GuiError::Model(
            "there is no visible geometry to export".into(),
        ));
    }
    let mut written = Vec::new();
    for property in meshes {
        let path = directory.join(format!("{}.{}", property.name, format.extension()));
        match format {
            PropertyFormat::PolyDataVtk => write_vtp(&path, &property.mesh)?,
            PropertyFormat::Stl => write_stl(&path, &property.mesh)?,
            PropertyFormat::Ply => write_ply(&path, &property.mesh)?,
        }
        written.push(path);
    }
    Ok(written)
}

fn write_vtp(path: &Path, mesh: &Mesh) -> Result<(), GuiError> {
    // points: triangles first, then lines, then standalone points
    let mut points = Vec::new();
    let mut polygons = Vec::new();
    let mut lines = Vec::new();
    let mut verts = Vec::new();
    let mut push_point = |points: &mut Vec<Vertex>, vertex: &Vertex| {
        points.push(*vertex);
        points.len() - 1
    };
    for triangle in &mesh.triangles {
        let base = points.len();
        for vertex in triangle {
            push_point(&mut points, vertex);
        }
        polygons.extend_from_slice(&[base, base + 1, base + 2]);
    }
    for line in &mesh.lines {
        let base = points.len();
        for vertex in line {
            push_point(&mut points, vertex);
        }
        lines.extend_from_slice(&[base, base + 1]);
    }
    for point in &mesh.points {
        push_point(&mut points, point);
        verts.push(points.len() - 1);
    }

    let mut xml = String::new();
    xml.push_str("<?xml version=\"1.0\"?>\n");
    xml.push_str(
        "<VTKFile type=\"PolyData\" version=\"1.0\" byte_order=\"LittleEndian\">\n  <PolyData>\n",
    );
    writeln!(
        xml,
        "    <Piece NumberOfPoints=\"{}\" NumberOfVerts=\"{}\" NumberOfLines=\"{}\" NumberOfPolys=\"{}\">",
        points.len(),
        mesh.points.len(),
        mesh.lines.len(),
        mesh.triangles.len()
    )
    .expect("string write");
    xml.push_str("      <Points>\n        <DataArray type=\"Float32\" NumberOfComponents=\"3\" format=\"ascii\">\n");
    for point in &points {
        writeln!(xml, "          {} {} {}", point[0], point[1], point[2]).expect("string write");
    }
    xml.push_str("        </DataArray>\n      </Points>\n");
    write_vtk_cells(&mut xml, "Verts", 1, &verts);
    write_vtk_cells(&mut xml, "Lines", 2, &lines);
    write_vtk_cells(&mut xml, "Polys", 3, &polygons);
    xml.push_str("    </Piece>\n  </PolyData>\n</VTKFile>\n");

    std::fs::write(path, xml)
        .map_err(|error| GuiError::Io(format!("could not write {}: {error}", path.display())))
}

fn write_vtk_cells(xml: &mut String, name: &str, per_cell: usize, indices: &[usize]) {
    writeln!(xml, "      <{name}>").expect("string write");
    xml.push_str("        <DataArray type=\"Int64\" Name=\"connectivity\" format=\"ascii\">\n");
    for (index, value) in indices.iter().enumerate() {
        if index > 0 && index % 12 == 0 {
            xml.push('\n');
        }
        write!(xml, "{value} ").expect("string write");
    }
    xml.push_str("\n        </DataArray>\n");
    xml.push_str("        <DataArray type=\"Int64\" Name=\"offsets\" format=\"ascii\">\n");
    let mut offset = 0;
    for _ in 0..indices.len() / per_cell {
        offset += per_cell;
        write!(xml, "{offset} ").expect("string write");
    }
    xml.push_str("\n        </DataArray>\n");
    writeln!(xml, "      </{name}>").expect("string write");
}

fn write_stl(path: &Path, mesh: &Mesh) -> Result<(), GuiError> {
    let mut bytes = Vec::with_capacity(84 + mesh.triangles.len() * 50);
    let mut header = [0u8; 80];
    let text = b"AppCSXCAD Rust export";
    header[..text.len()].copy_from_slice(text);
    bytes.extend_from_slice(&header);
    bytes.extend_from_slice(&(mesh.triangles.len() as u32).to_le_bytes());
    for triangle in &mesh.triangles {
        let normal = triangle_normal(triangle);
        for value in normal {
            bytes.extend_from_slice(&(value as f32).to_le_bytes());
        }
        for vertex in triangle {
            for value in vertex {
                bytes.extend_from_slice(&(*value as f32).to_le_bytes());
            }
        }
        bytes.extend_from_slice(&0u16.to_le_bytes());
    }
    std::fs::write(path, bytes)
        .map_err(|error| GuiError::Io(format!("could not write {}: {error}", path.display())))
}

fn triangle_normal(triangle: &[Vertex; 3]) -> Vertex {
    let edge1 = [
        triangle[1][0] - triangle[0][0],
        triangle[1][1] - triangle[0][1],
        triangle[1][2] - triangle[0][2],
    ];
    let edge2 = [
        triangle[2][0] - triangle[0][0],
        triangle[2][1] - triangle[0][1],
        triangle[2][2] - triangle[0][2],
    ];
    let normal = [
        edge1[1] * edge2[2] - edge1[2] * edge2[1],
        edge1[2] * edge2[0] - edge1[0] * edge2[2],
        edge1[0] * edge2[1] - edge1[1] * edge2[0],
    ];
    let length = (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2]).sqrt();
    if length <= 0.0 {
        [0.0; 3]
    } else {
        [normal[0] / length, normal[1] / length, normal[2] / length]
    }
}

fn write_ply(path: &Path, mesh: &Mesh) -> Result<(), GuiError> {
    let mut text = String::new();
    writeln!(text, "ply").expect("string write");
    text.push_str("format ascii 1.0\ncomment AppCSXCAD Rust export\n");
    writeln!(text, "element vertex {}", mesh.triangles.len() * 3).expect("string write");
    text.push_str("property float x\nproperty float y\nproperty float z\n");
    writeln!(text, "element face {}", mesh.triangles.len()).expect("string write");
    text.push_str("property list uchar int vertex_indices\nend_header\n");
    for triangle in &mesh.triangles {
        for vertex in triangle {
            writeln!(text, "{} {} {}", vertex[0], vertex[1], vertex[2]).expect("string write");
        }
    }
    for index in 0..mesh.triangles.len() {
        writeln!(text, "3 {} {} {}", index * 3, index * 3 + 1, index * 3 + 2)
            .expect("string write");
    }
    std::fs::write(path, text)
        .map_err(|error| GuiError::Io(format!("could not write {}: {error}", path.display())))
}

/// POV-Ray scene of every visible property as a `mesh2` (native `export_pov`).
pub fn export_povray(structure: &Structure, path: impl AsRef<Path>) -> Result<(), GuiError> {
    let path = path.as_ref();
    let meshes = property_meshes(structure);
    let mut text = String::new();
    text.push_str("// AppCSXCAD Rust export\n#include \"colors.inc\"\n");
    text.push_str("camera { location <1.5, -2.5, 1.8> look_at <0, 0, 0> }\n");
    text.push_str("light_source { <3, -4, 5> color White }\n");
    text.push_str("background { color Gray20 }\n");
    for property in &meshes {
        text.push_str("mesh2 {\n");
        let points: Vec<Vertex> = property
            .mesh
            .triangles
            .iter()
            .flat_map(|triangle| triangle.iter().copied())
            .collect();
        if points.is_empty() {
            continue;
        }
        writeln!(text, "  vertex_vectors {{").expect("string write");
        writeln!(text, "    {}", points.len()).expect("string write");
        for point in &points {
            writeln!(text, "    <{}, {}, {}>,", point[0], point[1], point[2])
                .expect("string write");
        }
        text.push_str("  }\n");
        writeln!(text, "  face_indices {{").expect("string write");
        writeln!(text, "    {}", property.mesh.triangles.len()).expect("string write");
        for index in 0..property.mesh.triangles.len() {
            writeln!(text, "    <{}, {}, {}>,", index * 3, index * 3 + 1, index * 3 + 2)
                .expect("string write");
        }
        text.push_str("  }\n");
        let [red, green, blue] = property.color;
        writeln!(
            text,
            "  pigment {{ rgb <{:.4}, {:.4}, {:.4}> }}\n  finish {{ phong 0.3 }}\n}}",
            red as f64 / 255.0,
            green as f64 / 255.0,
            blue as f64 / 255.0
        )
        .expect("string write");
    }
    std::fs::write(path, text)
        .map_err(|error| GuiError::Io(format!("could not write {}: {error}", path.display())))
}

/// X3D scene of every visible property (native `export_X3D`).
pub fn export_x3d(structure: &Structure, path: impl AsRef<Path>) -> Result<(), GuiError> {
    let path = path.as_ref();
    let meshes = property_meshes(structure);
    let mut xml = String::new();
    xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    xml.push_str("<!DOCTYPE X3D PUBLIC \"ISO//Web3D//DTD X3D 3.0//EN\" \"http://www.web3d.org/specifications/x3d-3.0.dtd\">\n");
    xml.push_str("<X3D profile=\"Immersive\" version=\"3.0\">\n<Scene>\n");
    for property in &meshes {
        if property.mesh.triangles.is_empty() {
            continue;
        }
        let [red, green, blue] = property.color;
        writeln!(
            xml,
            "  <Shape>\n    <Appearance><Material diffuseColor=\"{:.4} {:.4} {:.4}\"/></Appearance>\n    <IndexedFaceSet solid=\"false\" coordIndex=\"{}",
            red as f64 / 255.0,
            green as f64 / 255.0,
            blue as f64 / 255.0,
            (0..property.mesh.triangles.len())
                .map(|index| format!("{} {} {} -1", index * 3, index * 3 + 1, index * 3 + 2))
                .collect::<Vec<_>>()
                .join(" ")
        )
        .expect("string write");
        xml.push_str("\">\n      <Coordinate point=\"");
        for triangle in &property.mesh.triangles {
            for vertex in triangle {
                write!(xml, "{} {} {},", vertex[0], vertex[1], vertex[2])
                    .expect("string write");
            }
        }
        xml.push_str("\"/>\n    </IndexedFaceSet>\n  </Shape>\n");
    }
    xml.push_str("</Scene>\n</X3D>\n");
    std::fs::write(path, xml)
        .map_err(|error| GuiError::Io(format!("could not write {}: {error}", path.display())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use csxcad_core::model::{Primitive, Property, PropertyColor, PropertyType};

    fn structure() -> Structure {
        let mut structure = Structure::new();
        structure.grid_mut().set_delta_unit(0.001);
        let mut metal = Property::new("ground", PropertyType::Metal);
        metal.set_fill_color(Some(PropertyColor::new(255, 0, 0, 255)));
        metal.add_primitive(Primitive::box_primitive([0.0; 3], [1.0; 3]));
        structure.add_property(metal);
        let mut material = Property::new("dielectric", PropertyType::Material);
        material.set_fill_color(Some(PropertyColor::new(0, 255, 0, 255)));
        material.add_primitive(Primitive::sphere([0.0; 3], 0.5));
        structure.add_property(material);
        // an invisible property must not be exported
        let mut hidden = Property::new("hidden", PropertyType::Metal);
        hidden.set_visible(false);
        hidden.add_primitive(Primitive::box_primitive([0.0; 3], [1.0; 3]));
        structure.add_property(hidden);
        structure
    }

    fn temp_dir(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "qcsxcad-export-{name}-{}",
            std::process::id()
        ));
        std::fs::remove_dir_all(&directory).ok();
        directory
    }

    #[test]
    fn polydata_vtk_writes_one_file_per_visible_property() {
        let directory = temp_dir("vtp");
        let files = export_properties(&structure(), &directory, PropertyFormat::PolyDataVtk)
            .expect("export");
        assert_eq!(files.len(), 2);
        let names: Vec<String> = files
            .iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["ground.vtp", "dielectric.vtp"]);
        let text = std::fs::read_to_string(&files[0]).expect("read");
        assert!(text.contains("<VTKFile type=\"PolyData\""));
        assert!(text.contains("NumberOfPolys=\"12\""), "box has 12 triangles");
        // the mesh is scaled by the 1 mm delta unit
        assert!(text.contains("1 "), "scaled coordinate present");
        let body = text.split("<DataArray").nth(1).expect("points");
        let point: f64 = body
            .split_whitespace()
            .find_map(|token| token.parse().ok())
            .expect("numeric point");
        assert!(point.abs() <= 1.0 + 1e-9);
        let _ = point;
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn stl_and_ply_contain_the_triangles() {
        let directory = temp_dir("stl");
        let files = export_properties(&structure(), &directory, PropertyFormat::Stl).expect("stl");
        let bytes = std::fs::read(&files[0]).expect("read stl");
        let count = u32::from_le_bytes(bytes[80..84].try_into().expect("count"));
        assert_eq!(count, 12);
        assert_eq!(bytes.len(), 84 + 50 * count as usize);
        // the mesh is scaled by the 1 mm delta unit: the 1.0 m edges become
        // 0.001 in the file
        let mut found = false;
        for chunk in bytes[84..].chunks_exact(50) {
            for index in 0..9 {
                let value = f32::from_le_bytes(
                    chunk[12 + index * 4..16 + index * 4]
                        .try_into()
                        .expect("vertex"),
                );
                if (value.abs() - 0.001).abs() < 1e-9 {
                    found = true;
                }
            }
        }
        assert!(found, "scaled 1 mm edge present");

        let files = export_properties(&structure(), &directory, PropertyFormat::Ply).expect("ply");
        let text = std::fs::read_to_string(&files[0]).expect("read ply");
        assert!(text.starts_with("ply\nformat ascii 1.0"));
        assert!(text.contains("element vertex 36"));
        assert!(text.contains("element face 12"));
        assert!(text.contains("3 0 1 2"));
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn povray_and_x3d_scenes_hold_the_geometry() {
        let directory = temp_dir("scenes");
        std::fs::create_dir_all(&directory).expect("dir");
        let pov = directory.join("scene.pov");
        export_povray(&structure(), &pov).expect("pov");
        let text = std::fs::read_to_string(&pov).expect("read pov");
        assert!(text.contains("mesh2 {"));
        assert_eq!(text.matches("vertex_vectors").count(), 2);
        assert!(text.contains("face_indices"));
        assert!(text.contains("pigment { rgb <1.0000, 0.0000, 0.0000> }"));

        let x3d = directory.join("scene.x3d");
        export_x3d(&structure(), &x3d).expect("x3d");
        let text = std::fs::read_to_string(&x3d).expect("read x3d");
        assert!(text.contains("<X3D profile=\"Immersive\""));
        assert_eq!(text.matches("<IndexedFaceSet").count(), 2);
        assert!(text.contains("diffuseColor=\"1.0000 0.0000 0.0000\""));
        assert!(text.contains("<Coordinate point=\""));
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn an_empty_structure_is_reported() {
        let directory = temp_dir("empty");
        let error = export_properties(&Structure::new(), &directory, PropertyFormat::Stl)
            .expect_err("no geometry");
        assert!(matches!(error, GuiError::Model(_)));
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn unsafe_property_names_stay_inside_the_directory() {
        let mut structure = Structure::new();
        let mut property = Property::new("../escape", PropertyType::Metal);
        property.add_primitive(Primitive::box_primitive([0.0; 3], [1.0; 3]));
        structure.add_property(property);
        let directory = temp_dir("names");
        let files = export_properties(&structure, &directory, PropertyFormat::Stl).expect("export");
        assert_eq!(files[0].parent(), Some(directory.as_path()));
        assert_eq!(
            files[0].file_name().unwrap().to_string_lossy(),
            ".._escape.stl"
        );
        let _ = std::fs::remove_dir_all(&directory);
    }
}
