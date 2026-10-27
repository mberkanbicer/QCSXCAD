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

/// Cell index of the discrete grid (native `m_Disc_Ind` layout).
fn cell_index(size: [usize; 3], position: [usize; 3]) -> usize {
    position[0] + position[1] * (size[0] - 1) + position[2] * (size[0] - 1) * (size[1] - 1)
}

/// Build the material/background boundary surface of a discrete-material grid.
///
/// `scale` is the property scale (native `CSPropDiscMaterial::GetScale`); the
/// coordinates are the grid node coordinates multiplied by it.
pub fn surface_mesh(grid: &DiscreteMaterialGrid, scale: f64) -> Mesh {
    let size = grid.size();
    let indices = grid.material_indices();
    let mesh_axes = grid.mesh();
    let mut mesh = Mesh::default();
    if size.contains(&0) || size[0] < 2 || size[1] < 2 || size[2] < 2 {
        return mesh;
    }
    let expected = (size[0] - 1) * (size[1] - 1) * (size[2] - 1);
    if indices.len() < expected {
        return mesh;
    }

    let axis_steps = [[1usize, 0, 0], [0, 1, 0], [0, 0, 1]];
    let point = |position: [usize; 3]| -> Vertex {
        [
            f64::from(mesh_axes[0][position[0]]) * scale,
            f64::from(mesh_axes[1][position[1]]) * scale,
            f64::from(mesh_axes[2][position[2]]) * scale,
        ]
    };

    for pos_z in 0..size[2] {
        for pos_x in 0..size[0] {
            for pos_y in 0..size[1] {
                let pos = [pos_x, pos_y, pos_z];
                for axis in 0..3 {
                    let next = (axis + 1) % 3;
                    let next_next = (axis + 2) % 3;
                    if pos[next] >= size[next] - 1 || pos[next_next] >= size[next_next] - 1 {
                        continue;
                    }
                    let material_above = if pos[axis] < size[axis] - 1 {
                        indices[cell_index(size, pos)]
                    } else {
                        0
                    };
                    let material_below = if pos[axis] > 0 {
                        let mut relative = pos;
                        relative[axis] -= 1;
                        indices[cell_index(size, relative)]
                    } else {
                        0
                    };
                    if (material_above > 0) == (material_below > 0) {
                        continue;
                    }

                    // corners: base node, then one step in `next`, then in
                    // both, then back; the order flips with the material side
                    let step_next = axis_steps[next];
                    let step_next_next = axis_steps[next_next];
                    let corner = |a: usize, b: usize| -> [usize; 3] {
                        [
                            pos[0] + step_next[0] * a + step_next_next[0] * b,
                            pos[1] + step_next[1] * a + step_next_next[1] * b,
                            pos[2] + step_next[2] * a + step_next_next[2] * b,
                        ]
                    };
                    let (first, second) = if material_above > 0 && material_below == 0 {
                        (corner(1, 0), corner(1, 1))
                    } else {
                        (corner(0, 1), corner(1, 1))
                    };
                    let base = point(pos);
                    let a = point(first);
                    let b = point(second);
                    let d = point(corner(0, 0));
                    if material_above > 0 && material_below == 0 {
                        // up: base -> +next -> +next+next_next -> +next_next
                        mesh.triangles.push([base, a, b]);
                        mesh.triangles.push([base, b, d]);
                    } else {
                        // down: base -> +next_next -> +next_next+next -> +next
                        mesh.triangles.push([base, d, b]);
                        mesh.triangles.push([base, b, a]);
                    }
                    mesh.lines.push([base, d]);
                    mesh.lines.push([d, b]);
                    mesh.lines.push([b, a]);
                    mesh.lines.push([a, base]);
                }
            }
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
