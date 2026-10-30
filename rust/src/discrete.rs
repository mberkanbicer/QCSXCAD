//! Discrete-material surface model (native `CSPropDiscMaterial::CreatePolyDataModel`).
//!
//! A discrete-material property is a voxel grid: `material_indices` holds one
//! material id per cell (0 = background). The native viewer extracts the
//! boundary between material and background: for every mesh node and every
//! axis, the two neighbouring cells are compared and a quad is emitted when
//! exactly one of them is material. This module ports that sweep exactly,
//! including the winding of the "material above" and "material below" cases.

use csxcad_core::material::DiscreteMaterialGrid;
use csxcad_core::model::{PrimitiveType, Property, PropertyType};

use crate::tessellate::{Mesh, PrimitiveMesh, Vertex};

/// White surface, like the native `double rgb[3] = {1,1,1}`.
const DISCRETE_FILL: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
const DISCRETE_EDGE: [f32; 4] = [0.35, 0.35, 0.35, 1.0];

/// Build the material/background boundary surface of a discrete-material grid.
///
/// `scale` is the property scale (native `CSPropDiscMaterial::GetScale`); the
/// coordinates are the grid node coordinates multiplied by it. The extraction
/// itself is `DiscreteMaterialGrid::surface_poly_data`, the same routine the
/// Python and VTK front ends use.
pub fn surface_mesh(grid: &DiscreteMaterialGrid, scale: f64) -> Mesh {
    let mut mesh = Mesh::default();
    let Some(surface) = grid.surface_poly_data() else {
        return mesh;
    };
    let point = |index: u32| -> Vertex {
        let point = surface.points[index as usize];
        [point[0] * scale, point[1] * scale, point[2] * scale]
    };
    for quad in &surface.quads {
        let corners = [
            point(quad[0]),
            point(quad[1]),
            point(quad[2]),
            point(quad[3]),
        ];
        // The quad order is the native one; two triangles keep its winding.
        mesh.triangles.push([corners[0], corners[1], corners[2]]);
        mesh.triangles.push([corners[0], corners[2], corners[3]]);
        for corner in 0..4 {
            mesh.lines
                .push([corners[corner], corners[(corner + 1) % 4]]);
        }
    }
    mesh
}

/// Load a discrete-material property and return its surface mesh, with the
/// property scale and transform applied (native `RenderDiscMaterialModel`).
#[cfg(feature = "hdf5")]
pub fn property_surface_mesh(property: &Property) -> Option<PrimitiveMesh> {
    if property.property_type() != PropertyType::DiscreteMaterial {
        return None;
    }
    let properties = property.discrete_material()?;
    let grid = match properties.load_grid() {
        Ok(grid) => grid,
        Err(error) => {
            eprintln!(
                "QCSXCAD: discrete material '{}' could not be loaded: {error}",
                property.name()
            );
            return None;
        }
    };
    let mut mesh = surface_mesh(&grid, properties.scale());
    if mesh.is_empty() {
        return None;
    }
    if let Some(transform) = properties.transform() {
        mesh.apply_transform(transform);
    }
    Some(PrimitiveMesh {
        kind: PrimitiveType::Point,
        fill_color: DISCRETE_FILL,
        edge_color: DISCRETE_EDGE,
        mesh,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 3x3x3 node grid (2x2x2 cells) with the given material cells.
    fn grid(materials: &[(usize, usize, usize, u8)]) -> DiscreteMaterialGrid {
        let axis = vec![0.0f32, 1.0, 2.0];
        let mut indices = vec![0u8; 8];
        for (x, y, z, value) in materials {
            indices[x + y * 2 + z * 4] = *value;
        }
        DiscreteMaterialGrid::new([axis.clone(), axis.clone(), axis], indices, 2).expect("grid")
    }

    #[test]
    fn one_material_cell_exposes_all_six_faces() {
        let mesh = surface_mesh(&grid(&[(0, 0, 0, 1)]), 1.0);
        // six quads, two triangles each
        assert_eq!(mesh.triangles.len(), 12);
        assert_eq!(mesh.lines.len(), 24);
        let bounds = mesh.bounds().expect("bounds");
        assert_eq!(bounds, [0.0, 1.0, 0.0, 1.0, 0.0, 1.0]);
    }

    #[test]
    fn adjacent_cells_drop_the_shared_face() {
        let mesh = surface_mesh(&grid(&[(0, 0, 0, 1), (1, 0, 0, 1)]), 1.0);
        // a 2x1x1 block: 2*1*1 faces on x, 2*2*1 on y, 2*2*1 on z -> 10 quads
        assert_eq!(mesh.triangles.len(), 20);
        let bounds = mesh.bounds().expect("bounds");
        assert_eq!(bounds, [0.0, 2.0, 0.0, 1.0, 0.0, 1.0]);
    }

    #[test]
    fn an_empty_grid_has_no_surface() {
        assert!(surface_mesh(&grid(&[]), 1.0).is_empty());
    }

    #[test]
    fn the_property_scale_is_applied() {
        let mesh = surface_mesh(&grid(&[(1, 1, 1, 2)]), 0.5);
        let bounds = mesh.bounds().expect("bounds");
        assert_eq!(bounds, [0.5, 1.0, 0.5, 1.0, 0.5, 1.0]);
    }

    #[cfg(feature = "hdf5")]
    #[test]
    fn the_native_hdf5_fixture_loads_and_renders() {
        let directory = std::env::temp_dir().join(format!(
            "qcsxcad-discrete-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&directory).expect("temp dir");
        let path = directory.join("discrete_material_v2.h5");
        std::fs::write(
            &path,
            include_bytes!("../../../CSXCAD/rust/tests/fixtures/discrete_material_v2.h5"),
        )
        .expect("fixture");

        let mut property = Property::new("voxels", PropertyType::DiscreteMaterial);
        let properties = property
            .discrete_material_mut()
            .expect("discrete material");
        properties.set_file_type(0);
        properties.set_file_name(path.to_string_lossy().into_owned());
        let rendered = property_surface_mesh(&property).expect("surface");
        assert!(!rendered.mesh.triangles.is_empty());
        let bounds = rendered.mesh.bounds().expect("bounds");
        assert!(bounds[1] > bounds[0] && bounds[3] > bounds[2] && bounds[5] > bounds[4]);
        let _ = std::fs::remove_dir_all(&directory);
    }
}
