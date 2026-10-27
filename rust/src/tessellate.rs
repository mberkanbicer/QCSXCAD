//! CPU tessellation of CSXCAD primitives.
//!
//! The native QCSXCAD renderer maps every CSX primitive onto a VTK source
//! (`VTKPrimitives::AddCube`, `AddCylindricalCube`, `AddCylinder2`,
//! `AddSphere`, `AddSphericalShell`, `AddClosedPoly`, `AddRotationalPoly`,
//! `AddTubePoly`, `AddPolyData`). This module mirrors that mapping without
//! VTK: every primitive becomes a triangle surface plus a wireframe line set
//! (and a point set for point primitives) in drawing units, with the
//! primitive transform applied.

use csxcad_core::grid::CoordinateSystem;
use csxcad_core::model::{Primitive, PrimitiveShape, PrimitiveType, PropertyColor, Structure};
use csxcad_core::parameter::ParameterSet;
use csxcad_core::transform::Transform;

/// A position in drawing units.
pub type Vertex = [f64; 3];
/// One surface triangle.
pub type Triangle = [Vertex; 3];
/// One wireframe segment.
pub type Line = [Vertex; 2];

const DEFAULT_FILL: [f32; 4] = [0.45, 0.62, 0.90, 0.55];
const DEFAULT_EDGE: [f32; 4] = [0.12, 0.18, 0.32, 1.0];

fn color_to_rgba(color: PropertyColor) -> [f32; 4] {
    let [red, green, blue, alpha] = color.components();
    [
        red as f32 / 255.0,
        green as f32 / 255.0,
        blue as f32 / 255.0,
        alpha as f32 / 255.0,
    ]
}

/// Tessellated geometry of one primitive.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Mesh {
    pub triangles: Vec<Triangle>,
    pub lines: Vec<Line>,
    pub points: Vec<Vertex>,
}

impl Mesh {
    pub fn is_empty(&self) -> bool {
        self.triangles.is_empty() && self.lines.is_empty() && self.points.is_empty()
    }

    /// Axis-aligned bounds `[xmin, xmax, ymin, ymax, zmin, zmax]`.
    pub fn bounds(&self) -> Option<[f64; 6]> {
        let mut bounds = [f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY,
            f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY];
        let mut any = false;
        let mut visit = |vertex: &Vertex| {
            for axis in 0..3 {
                bounds[2 * axis] = bounds[2 * axis].min(vertex[axis]);
                bounds[2 * axis + 1] = bounds[2 * axis + 1].max(vertex[axis]);
            }
        };
        for triangle in &self.triangles {
            for vertex in triangle {
                visit(vertex);
                any = true;
            }
        }
        for line in &self.lines {
            for vertex in line {
                visit(vertex);
                any = true;
            }
        }
        for point in &self.points {
            visit(point);
            any = true;
        }
        any.then_some(bounds)
    }

    /// Apply a CSXCAD transform to every vertex.
    pub fn apply_transform(&mut self, transform: &Transform) {
        let mut map = |vertex: &mut Vertex| *vertex = transform.transform(*vertex);
        for triangle in &mut self.triangles {
            for vertex in triangle {
                map(vertex);
            }
        }
        for line in &mut self.lines {
            for vertex in line {
                map(vertex);
            }
        }
        for point in &mut self.points {
            map(point);
        }
    }

    fn triangle(&mut self, a: Vertex, b: Vertex, c: Vertex) {
        self.triangles.push([a, b, c]);
    }

    fn quad(&mut self, a: Vertex, b: Vertex, c: Vertex, d: Vertex) {
        self.triangle(a, b, c);
        self.triangle(a, c, d);
    }

    fn line(&mut self, a: Vertex, b: Vertex) {
        self.lines.push([a, b]);
    }
}

/// One tessellated primitive with its property colors.
#[derive(Clone, Debug)]
pub struct PrimitiveMesh {
    pub kind: PrimitiveType,
    pub fill_color: [f32; 4],
    pub edge_color: [f32; 4],
    pub mesh: Mesh,
}

/// Tessellator settings and entry point.
#[derive(Clone, Debug)]
pub struct Tessellator {
    /// Segments per full revolution (native `iResolution`, default 32).
    pub resolution: usize,
}

impl Default for Tessellator {
    fn default() -> Self {
        Self::new()
    }
}

impl Tessellator {
    pub fn new() -> Self {
        Self { resolution: 32 }
    }

    /// Tessellate every visible property of a structure.
    pub fn tessellate(&self, structure: &Structure) -> Vec<PrimitiveMesh> {
        let parameters = structure.parameters();
        let mut meshes = Vec::new();
        for index in 0..structure.property_count() {
            let Some(property) = structure.property_by_index(index) else {
                continue;
            };
            if !property.is_visible() {
                continue;
            }
            let fill_color = property
                .fill_color()
                .map(color_to_rgba)
                .unwrap_or(DEFAULT_FILL);
            let edge_color = property
                .edge_color()
                .map(color_to_rgba)
                .unwrap_or(DEFAULT_EDGE);
            for primitive in property.primitives() {
                let mut mesh = Mesh::default();
                self.tessellate_primitive(primitive, parameters, &mut mesh);
                if let Some(transform) = primitive.transform() {
                    if transform.has_transform() {
                        mesh.apply_transform(transform);
                    }
                }
                if !mesh.is_empty() {
                    meshes.push(PrimitiveMesh {
                        kind: primitive.primitive_type(),
                        fill_color,
                        edge_color,
                        mesh,
                    });
                }
            }
        }
        meshes
    }

    fn tessellate_primitive(
        &self,
        primitive: &Primitive,
        parameters: &ParameterSet,
        mesh: &mut Mesh,
    ) {
        match primitive.shape() {
            PrimitiveShape::Point(shape) => {
                if let Ok(coordinate) = shape.evaluated_coordinates(Some(parameters)) {
                    mesh.points.push(coordinate);
                    let extent = 0.05;
                    for axis in 0..3 {
                        let mut low = coordinate;
                        let mut high = coordinate;
                        low[axis] -= extent;
                        high[axis] += extent;
                        mesh.line(low, high);
                    }
                }
            }
            PrimitiveShape::Box(shape) => {
                let Ok(coordinates) = shape.evaluated_coordinates(Some(parameters)) else {
                    return;
                };
                if primitive.coordinate_system() == CoordinateSystem::Cylindrical
                    || primitive.mesh_type() == CoordinateSystem::Cylindrical
                {
                    self.cylindrical_box(coordinates, mesh);
                } else {
                    self.cartesian_box(coordinates, mesh);
                }
            }
            PrimitiveShape::Sphere(shape) => {
                let (Ok(center), Ok(radius)) = (
                    shape.center(Some(parameters)),
                    shape.radius(Some(parameters)),
                ) else {
                    return;
                };
                self.sphere(center, radius, mesh);
            }
            PrimitiveShape::SphericalShell(shape) => {
                let (Ok(center), Ok(radius), Ok(width)) = (
                    shape.center(Some(parameters)),
                    shape.radius(Some(parameters)),
                    shape.shell_width(Some(parameters)),
                ) else {
                    return;
                };
                self.sphere(center, radius + width / 2.0, mesh);
                if radius - width / 2.0 > 0.0 {
                    self.sphere(center, radius - width / 2.0, mesh);
                }
            }
            PrimitiveShape::Cylinder(shape) => {
                let (Ok(axis), Ok(radius)) = (
                    shape.axis_coordinates(Some(parameters), CoordinateSystem::Cartesian),
                    shape.radius(Some(parameters)),
                ) else {
                    return;
                };
                let Ok(shell_width) = shape.shell_width(Some(parameters)) else {
                    return;
                };
                match shell_width {
                    Some(width) => self.cylinder_shell(axis, radius, width, mesh),
                    None => self.cylinder(axis, radius, mesh),
                }
            }
            PrimitiveShape::Curve(shape) => {
                let Ok(points) = shape.evaluated_points(Some(parameters)) else {
                    return;
                };
                let Ok(wire_radius) = shape.wire_radius(Some(parameters)) else {
                    return;
                };
                self.curve(&points, wire_radius, mesh);
            }
            PrimitiveShape::Polygon(shape) => {
                let Ok(vertices) = shape.evaluated_vertices(Some(parameters)) else {
                    return;
                };
                let Ok(elevation) = shape.elevation(Some(parameters)) else {
                    return;
                };
                let normal = shape.normal_direction();
                let Ok(extrusion) = shape.extrusion_length(Some(parameters)) else {
                    return;
                };
                let Ok(rotation) = shape.rotation_angles(Some(parameters)) else {
                    return;
                };
                let profile: Vec<[f64; 3]> = vertices
                    .iter()
                    .map(|vertex| {
                        let mut point = [0.0; 3];
                        point[normal] = elevation;
                        point[(normal + 1) % 3] = vertex[0];
                        point[(normal + 2) % 3] = vertex[1];
                        point
                    })
                    .collect();
                if let (Some(axis), Some(angles)) = (shape.rotation_axis_direction(), rotation) {
                    self.rotational_polygon(&profile, axis, angles, mesh);
                } else if let Some(length) = extrusion {
                    self.extruded_polygon(&profile, normal, length, mesh);
                } else {
                    self.polygon(&profile, mesh);
                }
            }
            PrimitiveShape::Polyhedron(shape) => {
                let vertices: Vec<Vertex> = (0..shape.vertex_count())
                    .filter_map(|index| shape.vertex(index))
                    .map(|vertex| [vertex[0] as f64, vertex[1] as f64, vertex[2] as f64])
                    .collect();
                for face in 0..shape.face_count() {
                    let Some(indices) = shape.face(face) else {
                        continue;
                    };
                    self.polyhedron_face(&vertices, indices, mesh);
                }
            }
            PrimitiveShape::PolyhedronReader(shape) => {
                // A file-backed polyhedron cannot be read here (native
                // `CSPrimPolyhedronReader` needs the file at render time).
                let _ = shape;
            }
            PrimitiveShape::MultiBox(shape) => {
                for index in 0..shape.box_count() {
                    let Some(multi_box) = shape.box_at(index) else {
                        continue;
                    };
                    if let Ok(coordinates) = multi_box.evaluated_coordinates(Some(parameters)) {
                        self.cartesian_box(coordinates, mesh);
                    }
                }
            }
            PrimitiveShape::UserDefined(_) => {
                // A user-defined function has no renderable geometry (native
                // cannot draw it either).
            }
        }
    }

    /// Native `VTKPrimitives::AddCube`.
    fn cartesian_box(&self, coordinates: [[f64; 3]; 2], mesh: &mut Mesh) {
        let [start, stop] = coordinates;
        let mut low = [0.0; 3];
        let mut high = [0.0; 3];
        for axis in 0..3 {
            low[axis] = start[axis].min(stop[axis]);
            high[axis] = start[axis].max(stop[axis]);
        }
        let corners: [Vertex; 8] = [
            [low[0], low[1], low[2]],
            [high[0], low[1], low[2]],
            [high[0], high[1], low[2]],
            [low[0], high[1], low[2]],
            [low[0], low[1], high[2]],
            [high[0], low[1], high[2]],
            [high[0], high[1], high[2]],
            [low[0], high[1], high[2]],
        ];
        let faces: [[usize; 4]; 6] = [
            [0, 1, 2, 3],
            [4, 7, 6, 5],
            [0, 4, 5, 1],
            [3, 2, 6, 7],
            [0, 3, 7, 4],
            [1, 5, 6, 2],
        ];
        let extents = [high[0] - low[0], high[1] - low[1], high[2] - low[2]];
        // the in-plane axes of each quad above; a degenerate box dimension
        // drops the faces that would collapse to zero area
        let face_axes = [
            (0usize, 1usize),
            (0, 1),
            (0, 2),
            (0, 2),
            (1, 2),
            (1, 2),
        ];
        for (face, indices) in faces.iter().enumerate() {
            let (first, second) = face_axes[face];
            if extents[first] > 0.0 && extents[second] > 0.0 {
                mesh.quad(
                    corners[indices[0]],
                    corners[indices[1]],
                    corners[indices[2]],
                    corners[indices[3]],
                );
            }
        }
        let edges: [[usize; 2]; 12] = [
            [0, 1], [1, 2], [2, 3], [3, 0],
            [4, 5], [5, 6], [6, 7], [7, 4],
            [0, 4], [1, 5], [2, 6], [3, 7],
        ];
        for edge in edges {
            mesh.line(corners[edge[0]], corners[edge[1]]);
        }
    }

    /// Native `VTKPrimitives::AddCylindricalCube`: a box in `(rho, alpha, z)`.
    fn cylindrical_box(&self, coordinates: [[f64; 3]; 2], mesh: &mut Mesh) {
        let [start, stop] = coordinates;
        let mut low = [0.0; 3];
        let mut high = [0.0; 3];
        for axis in 0..3 {
            low[axis] = start[axis].min(stop[axis]);
            high[axis] = start[axis].max(stop[axis]);
        }
        let (rho0, rho1) = (low[0], high[0]);
        let (alpha0, alpha1) = (low[1], high[1]);
        let (z0, z1) = (low[2], high[2]);
        let span = alpha1 - alpha0;
        let segments = ((span / std::f64::consts::TAU * self.resolution as f64).ceil() as usize)
            .max(1);
        let alpha_at = |index: usize| alpha0 + span * index as f64 / segments as f64;
        let point = |rho: f64, alpha: f64, z: f64| -> Vertex {
            [rho * alpha.cos(), rho * alpha.sin(), z]
        };
        let rings = [rho0, rho1];
        // lateral surfaces at both rho levels
        for rho in rings {
            if rho <= 0.0 {
                continue;
            }
            for index in 0..segments {
                let a = alpha_at(index);
                let b = alpha_at(index + 1);
                mesh.quad(
                    point(rho, a, z0),
                    point(rho, b, z0),
                    point(rho, b, z1),
                    point(rho, a, z1),
                );
            }
        }
        // top and bottom annuli
        if z1 > z0 {
            for index in 0..segments {
                let a = alpha_at(index);
                let b = alpha_at(index + 1);
                mesh.quad(
                    point(rho0, a, z0),
                    point(rho1, a, z0),
                    point(rho1, b, z0),
                    point(rho0, b, z0),
                );
                mesh.quad(
                    point(rho0, a, z1),
                    point(rho0, b, z1),
                    point(rho1, b, z1),
                    point(rho1, a, z1),
                );
            }
        }
        // radial side faces at the alpha bounds
        if span > 0.0 {
            for alpha in [alpha0, alpha1] {
                if z1 > z0 {
                    mesh.quad(
                        point(rho0, alpha, z0),
                        point(rho1, alpha, z0),
                        point(rho1, alpha, z1),
                        point(rho0, alpha, z1),
                    );
                }
            }
        }
        // wireframe: both alpha bounds, the rho levels and the verticals
        for alpha in [alpha0, alpha1] {
            if z1 > z0 {
                mesh.line(point(rho0, alpha, z0), point(rho1, alpha, z0));
                mesh.line(point(rho0, alpha, z1), point(rho1, alpha, z1));
            }
            for rho in rings {
                if rho > 0.0 {
                    mesh.line(point(rho, alpha, z0), point(rho, alpha, z1));
                }
            }
        }
        for index in 0..=segments {
            let alpha = alpha_at(index);
            mesh.line(point(rho0, alpha, z0), point(rho0, alpha, z1));
            mesh.line(point(rho1, alpha, z0), point(rho1, alpha, z1));
        }
    }

    /// A cylinder with an arbitrary axis (native `VTKPrimitives::AddCylinder2`).
    fn cylinder(&self, axis: [[f64; 3]; 2], radius: f64, mesh: &mut Mesh) {
        let segments = self.resolution.max(3);
        let Some((direction, u, v)) = axis_basis(axis) else {
            return;
        };
        let _ = direction;
        let ring = |center: Vertex, index: usize| -> Vertex {
            let angle = std::f64::consts::TAU * index as f64 / segments as f64;
            [
                center[0] + radius * (angle.cos() * u[0] + angle.sin() * v[0]),
                center[1] + radius * (angle.cos() * u[1] + angle.sin() * v[1]),
                center[2] + radius * (angle.cos() * u[2] + angle.sin() * v[2]),
            ]
        };
        let (start, stop) = (axis[0], axis[1]);
        for index in 0..segments {
            let next = (index + 1) % segments;
            let a0 = ring(start, index);
            let a1 = ring(start, next);
            let b0 = ring(stop, index);
            let b1 = ring(stop, next);
            mesh.quad(a0, a1, b1, b0);
            mesh.triangle(start, a1, a0);
            mesh.triangle(stop, b0, b1);
            mesh.line(a0, a1);
            mesh.line(b0, b1);
            mesh.line(a0, b0);
        }
    }

    /// A cylindrical shell (native `VTKPrimitives::AddCylindricalShell`).
    fn cylinder_shell(&self, axis: [[f64; 3]; 2], radius: f64, width: f64, mesh: &mut Mesh) {
        let segments = self.resolution.max(3);
        let Some((_direction, u, v)) = axis_basis(axis) else {
            return;
        };
        let inner = (radius - width / 2.0).max(0.0);
        let outer = radius + width / 2.0;
        let ring = |center: Vertex, radius: f64, index: usize| -> Vertex {
            let angle = std::f64::consts::TAU * index as f64 / segments as f64;
            [
                center[0] + radius * (angle.cos() * u[0] + angle.sin() * v[0]),
                center[1] + radius * (angle.cos() * u[1] + angle.sin() * v[1]),
                center[2] + radius * (angle.cos() * u[2] + angle.sin() * v[2]),
            ]
        };
        let (start, stop) = (axis[0], axis[1]);
        for index in 0..segments {
            let next = (index + 1) % segments;
            for radius in [inner, outer] {
                mesh.quad(
                    ring(start, radius, index),
                    ring(start, radius, next),
                    ring(stop, radius, next),
                    ring(stop, radius, index),
                );
                mesh.line(ring(start, radius, index), ring(stop, radius, index));
                mesh.line(ring(start, radius, index), ring(start, radius, next));
            }
            // annular end faces
            for center in [start, stop] {
                mesh.quad(
                    ring(center, inner, index),
                    ring(center, inner, next),
                    ring(center, outer, next),
                    ring(center, outer, index),
                );
            }
        }
    }

    /// Native `VTKPrimitives::AddSphere` (UV sphere).
    fn sphere(&self, center: Vertex, radius: f64, mesh: &mut Mesh) {
        let segments = self.resolution.max(4);
        let rings = (self.resolution / 2).max(2);
        let at = |ring: usize, segment: usize| -> Vertex {
            let phi = std::f64::consts::PI * ring as f64 / rings as f64;
            let theta = std::f64::consts::TAU * segment as f64 / segments as f64;
            [
                center[0] + radius * phi.sin() * theta.cos(),
                center[1] + radius * phi.sin() * theta.sin(),
                center[2] + radius * phi.cos(),
            ]
        };
        for ring in 0..rings {
            for segment in 0..segments {
                let next = (segment + 1) % segments;
                let a = at(ring, segment);
                let b = at(ring, next);
                let c = at(ring + 1, next);
                let d = at(ring + 1, segment);
                mesh.quad(a, b, c, d);
                if ring % (rings / 4).max(1) == 0 {
                    mesh.line(a, d);
                }
                mesh.line(a, b);
            }
        }
        for segment in (0..segments).step_by((segments / 8).max(1)) {
            for ring in 0..rings {
                mesh.line(at(ring, segment), at(ring + 1, segment));
            }
        }
    }

    /// A polyline, optionally a tube (native `VTKPrimitives::AddTubePoly`).
    fn curve(&self, points: &[Vertex], wire_radius: Option<f64>, mesh: &mut Mesh) {
        for pair in points.windows(2) {
            mesh.line(pair[0], pair[1]);
        }
        let Some(radius) = wire_radius.filter(|radius| *radius > 0.0) else {
            return;
        };
        for pair in points.windows(2) {
            self.cylinder([pair[0], pair[1]], radius, mesh);
        }
    }

    /// A flat polygon (native `VTKPrimitives::AddClosedPoly`, no extrusion).
    fn polygon(&self, points: &[[f64; 3]], mesh: &mut Mesh) {
        if points.len() < 3 {
            return;
        }
        for index in 1..points.len() - 1 {
            mesh.triangle(points[0], points[index], points[index + 1]);
        }
        for index in 0..points.len() {
            mesh.line(points[index], points[(index + 1) % points.len()]);
        }
    }

    /// A polygon extruded along its normal direction (native `AddClosedPoly`
    /// with an extrusion vector).
    fn extruded_polygon(&self, points: &[[f64; 3]], normal: usize, length: f64, mesh: &mut Mesh) {
        if points.len() < 3 {
            return;
        }
        let mut top: Vec<Vertex> = points.to_vec();
        for point in &mut top {
            point[normal] += length;
        }
        self.polygon(points, mesh);
        self.polygon(&top, mesh);
        for index in 0..points.len() {
            let next = (index + 1) % points.len();
            mesh.quad(points[index], points[next], top[next], top[index]);
            mesh.line(points[index], top[index]);
        }
    }

    /// A profile revolved around a coordinate axis (native
    /// `VTKPrimitives::AddRotationalPoly`).
    fn rotational_polygon(
        &self,
        profile: &[[f64; 3]],
        axis: usize,
        angles: [f64; 2],
        mesh: &mut Mesh,
    ) {
        if profile.len() < 2 {
            return;
        }
        let span = angles[1] - angles[0];
        let steps = ((span.abs() / std::f64::consts::TAU * self.resolution as f64).ceil()
            as usize)
            .max(1);
        let rotate = |point: Vertex, angle: f64| -> Vertex {
            let (sine, cosine) = angle.sin_cos();
            let (u, v) = match axis {
                0 => (1, 2),
                1 => (2, 0),
                _ => (0, 1),
            };
            let mut result = point;
            result[u] = point[u] * cosine - point[v] * sine;
            result[v] = point[u] * sine + point[v] * cosine;
            result
        };
        let angle_at = |index: usize| angles[0] + span * index as f64 / steps as f64;
        for index in 0..steps {
            let a = angle_at(index);
            let b = angle_at(index + 1);
            for pair in profile.windows(2) {
                mesh.quad(
                    rotate(pair[0], a),
                    rotate(pair[1], a),
                    rotate(pair[1], b),
                    rotate(pair[0], b),
                );
            }
            for point in profile {
                mesh.line(rotate(*point, a), rotate(*point, b));
            }
        }
        for index in 0..=steps {
            let angle = angle_at(index);
            for pair in profile.windows(2) {
                mesh.line(rotate(pair[0], angle), rotate(pair[1], angle));
            }
        }
        // close the end caps when the revolution does not close the profile
        for angle in [angles[0], angles[1]] {
            let cap: Vec<Vertex> = profile.iter().map(|point| rotate(*point, angle)).collect();
            if (span.abs() - std::f64::consts::TAU).abs() > 1e-9 {
                self.polygon(&cap, mesh);
            }
        }
    }

    /// One polyhedron face (native `VTKPrimitives::AddPolyData` fan).
    fn polyhedron_face(&self, vertices: &[Vertex], indices: &[i32], mesh: &mut Mesh) {
        let points: Vec<Vertex> = indices
            .iter()
            .filter_map(|index| {
                if *index < 0 {
                    None
                } else {
                    vertices.get(*index as usize).copied()
                }
            })
            .collect();
        self.polygon(&points, mesh);
    }
}

/// Orthonormal basis perpendicular to the cylinder axis.
fn axis_basis(axis: [[f64; 3]; 2]) -> Option<(Vertex, Vertex, Vertex)> {
    let direction = [
        axis[1][0] - axis[0][0],
        axis[1][1] - axis[0][1],
        axis[1][2] - axis[0][2],
    ];
    let length = (direction[0] * direction[0]
        + direction[1] * direction[1]
        + direction[2] * direction[2])
        .sqrt();
    if length <= 0.0 {
        return None;
    }
    let direction = [
        direction[0] / length,
        direction[1] / length,
        direction[2] / length,
    ];
    let reference = if direction[0].abs() < 0.9 {
        [1.0, 0.0, 0.0]
    } else {
        [0.0, 1.0, 0.0]
    };
    let u = cross(direction, reference);
    let u_length = (u[0] * u[0] + u[1] * u[1] + u[2] * u[2]).sqrt();
    if u_length <= 0.0 {
        return None;
    }
    let u = [u[0] / u_length, u[1] / u_length, u[2] / u_length];
    let v = cross(direction, u);
    Some((direction, u, v))
}

fn cross(a: Vertex, b: Vertex) -> Vertex {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use csxcad_core::model::Structure;

    fn tessellator() -> Tessellator {
        Tessellator { resolution: 16 }
    }

    fn mesh_of(primitive: Primitive) -> Mesh {
        let mut mesh = Mesh::default();
        let parameters = ParameterSet::new();
        tessellator().tessellate_primitive(&primitive, &parameters, &mut mesh);
        mesh
    }

    #[test]
    fn box_mesh_matches_the_native_cube() {
        let mesh = mesh_of(Primitive::box_primitive([0.0, 0.0, 0.0], [2.0, 4.0, 6.0]));
        assert_eq!(mesh.bounds(), Some([0.0, 2.0, 0.0, 4.0, 0.0, 6.0]));
        assert_eq!(mesh.triangles.len(), 12);
        assert_eq!(mesh.lines.len(), 12);
    }

    #[test]
    fn flat_box_becomes_a_rectangle() {
        let mesh = mesh_of(Primitive::box_primitive([0.0, 0.0, 2.0], [2.0, 4.0, 2.0]));
        assert_eq!(mesh.bounds(), Some([0.0, 2.0, 0.0, 4.0, 2.0, 2.0]));
        // the two rectangle faces survive, the four zero-height sides collapse
        assert_eq!(mesh.triangles.len(), 4);
    }

    #[test]
    fn cylinder_matches_axis_and_radius() {
        let primitive = Primitive::cylinder([0.0, 0.0, 0.0], [0.0, 0.0, 10.0], 2.0);
        let mesh = mesh_of(primitive);
        let bounds = mesh.bounds().expect("bounds");
        assert!((bounds[0] + 2.0).abs() < 1e-9, "xmin {bounds:?}");
        assert!((bounds[1] - 2.0).abs() < 1e-9, "xmax {bounds:?}");
        assert!((bounds[2] + 2.0).abs() < 1e-9, "ymin {bounds:?}");
        assert!((bounds[3] - 2.0).abs() < 1e-9, "ymax {bounds:?}");
        assert!((bounds[4] - 0.0).abs() < 1e-9, "zmin {bounds:?}");
        assert!((bounds[5] - 10.0).abs() < 1e-9, "zmax {bounds:?}");
        // every lateral vertex is exactly on the radius (the axis is z)
        for triangle in &mesh.triangles {
            for vertex in triangle {
                let radius = (vertex[0] * vertex[0] + vertex[1] * vertex[1]).sqrt();
                assert!(
                    (radius - 2.0).abs() < 1e-9 || radius < 1e-9,
                    "vertex {vertex:?} radius {radius}"
                );
            }
        }
    }

    #[test]
    fn sphere_vertices_lie_on_the_radius() {
        let mesh = mesh_of(Primitive::sphere([1.0, 2.0, 3.0], 5.0));
        for triangle in &mesh.triangles {
            for vertex in triangle {
                let distance = ((vertex[0] - 1.0) * (vertex[0] - 1.0)
                    + (vertex[1] - 2.0) * (vertex[1] - 2.0)
                    + (vertex[2] - 3.0) * (vertex[2] - 3.0))
                    .sqrt();
                assert!((distance - 5.0).abs() < 1e-9, "distance {distance}");
            }
        }
    }

    #[test]
    fn spherical_shell_draws_both_radii() {
        let primitive = Primitive::spherical_shell([0.0, 0.0, 0.0], 5.0, 2.0);
        let mesh = mesh_of(primitive);
        let mut radii: Vec<f64> = mesh
            .triangles
            .iter()
            .flatten()
            .map(|vertex| (vertex[0] * vertex[0] + vertex[1] * vertex[1] + vertex[2] * vertex[2]).sqrt())
            .collect();
        radii.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert!((radii[0] - 4.0).abs() < 1e-9);
        assert!((radii[radii.len() - 1] - 6.0).abs() < 1e-9);
    }

    #[test]
    fn extruded_polygon_spans_elevation_and_length() {
        let mut primitive = Primitive::lin_poly(3.0);
        if let PrimitiveShape::Polygon(shape) = primitive.shape_mut() {
            shape.add_vertex([0.0, 0.0]);
            shape.add_vertex([2.0, 0.0]);
            shape.add_vertex([2.0, 1.0]);
            shape.add_vertex([0.0, 1.0]);
            shape.set_elevation(2.0);
            shape.set_normal_direction(2);
        }
        let mesh = mesh_of(primitive);
        assert_eq!(mesh.bounds(), Some([0.0, 2.0, 0.0, 1.0, 2.0, 5.0]));
        // outline, cap and side wireframe plus the two caps
        assert!(mesh.triangles.len() >= 2 * 2 + 4 * 2);
    }

    #[test]
    fn rotational_polygon_revolves_the_profile() {
        let mut primitive = Primitive::rot_poly();
        if let PrimitiveShape::Polygon(shape) = primitive.shape_mut() {
            shape.add_vertex([1.0, -1.0]);
            shape.add_vertex([2.0, -1.0]);
            shape.add_vertex([2.0, 1.0]);
            shape.add_vertex([1.0, 1.0]);
            // revolve around z: the y coordinate becomes the swept radius
            assert!(shape.set_rotation_axis_direction(2));
        }
        let mesh = mesh_of(primitive);
        let bounds = mesh.bounds().expect("bounds");
        assert!((bounds[0] + 2.0).abs() < 1e-9, "xmin {bounds:?}");
        assert!((bounds[1] - 2.0).abs() < 1e-9, "xmax {bounds:?}");
        assert!((bounds[2] + 2.0).abs() < 1e-9, "ymin {bounds:?}");
        assert!((bounds[3] - 2.0).abs() < 1e-9, "ymax {bounds:?}");
        assert!((bounds[4] + 1.0).abs() < 1e-9, "zmin {bounds:?}");
        assert!((bounds[5] - 1.0).abs() < 1e-9, "zmax {bounds:?}");
    }

    #[test]
    fn tessellate_applies_transform_and_property_colors() {
        let mut structure = Structure::new();
        let mut property =
            csxcad_core::model::Property::new("mat", csxcad_core::model::PropertyType::Material);
        property.set_fill_color(Some(PropertyColor::new(255, 128, 0, 200)));
        let mut primitive = Primitive::box_primitive([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]);
        primitive.transform_mut().translate([2.0, 3.0, 4.0], false);
        property.add_primitive(primitive);
        structure.add_property(property);
        let meshes = tessellator().tessellate(&structure);
        assert_eq!(meshes.len(), 1);
        assert_eq!(meshes[0].fill_color, [1.0, 128.0 / 255.0, 0.0, 200.0 / 255.0]);
        assert_eq!(meshes[0].mesh.bounds(), Some([2.0, 3.0, 3.0, 4.0, 4.0, 5.0]));
    }

    #[test]
    fn cylindrical_box_is_a_wedge() {
        let mut primitive = Primitive::box_primitive([1.0, 0.0, 0.0], [3.0, 1.0, 2.0]);
        primitive.set_coordinate_system(CoordinateSystem::Cylindrical);
        let mesh = mesh_of(primitive);
        let bounds = mesh.bounds().expect("bounds");
        // alpha 0..1 rad, rho 1..3, z 0..2
        assert!((bounds[0] - 1.0 * 1.0f64.cos()).abs() < 1e-9);
        assert!(bounds[1] <= 3.0 + 1e-9);
        assert!((bounds[4] - 0.0).abs() < 1e-9);
        assert!((bounds[5] - 2.0).abs() < 1e-9);
        for triangle in &mesh.triangles {
            for vertex in triangle {
                let radius = (vertex[0] * vertex[0] + vertex[1] * vertex[1]).sqrt();
                assert!((1.0..=3.0 + 1e-9).contains(&radius), "radius {radius}");
                assert!(vertex[1] >= -1e-9, "negative y {vertex:?}");
            }
        }
    }

    #[test]
    fn polyhedron_faces_are_triangulated() {
        let mut primitive = Primitive::polyhedron();
        if let PrimitiveShape::Polyhedron(shape) = primitive.shape_mut() {
            shape.add_vertex([0.0, 0.0, 0.0]);
            shape.add_vertex([1.0, 0.0, 0.0]);
            shape.add_vertex([0.0, 1.0, 0.0]);
            shape.add_vertex([0.0, 0.0, 1.0]);
            shape.add_face(vec![0, 1, 2]);
            shape.add_face(vec![0, 1, 3]);
        }
        let mesh = mesh_of(primitive);
        assert_eq!(mesh.triangles.len(), 2);
        assert_eq!(mesh.lines.len(), 6);
    }

    #[test]
    fn wire_tube_adds_surface_triangles() {
        let mut primitive = Primitive::wire(0.1);
        if let PrimitiveShape::Curve(shape) = primitive.shape_mut() {
            shape.add_point([0.0, 0.0, 0.0]);
            shape.add_point([1.0, 0.0, 0.0]);
            shape.add_point([1.0, 1.0, 0.0]);
        }
        let mesh = mesh_of(primitive);
        assert_eq!(mesh.lines.len(), 2 + 2 * 16 * 3);
        assert!(mesh.triangles.len() >= 2 * 3 * 16);
    }
}
