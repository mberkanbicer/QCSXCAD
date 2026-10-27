//! wgpu-based 3D structure renderer for QCSXCAD.
//!
//! Geometry comes from `qcsxcad_core::tessellate`, which mirrors the native
//! `VTKPrimitives` mapping for every CSX primitive type. Surfaces are drawn
//! as triangles with a per-property fill color and the wireframe on top, using
//! a camera that fits the structure (parallel projection for the 2D views).

use csxcad_core::model::Structure;
use qcsxcad_core::model::{GuiError, GuiState, ViewMode};
use qcsxcad_core::rendering::StructureRenderer;
use qcsxcad_core::tessellate::Tessellator;
use wgpu::util::DeviceExt;

/// A 3D vertex: position + color.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    position: [f32; 3],
    color: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct CameraUniform {
    view_proj: [[f32; 4]; 4],
}

/// Orbit camera fitted to the structure bounds.
#[derive(Clone, Copy, Debug)]
pub struct Camera {
    pub center: [f32; 3],
    pub distance: f32,
    /// Rotation around the world z axis, radians.
    pub yaw: f32,
    /// Elevation above the xy plane, radians.
    pub pitch: f32,
    pub orthographic: bool,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            center: [0.0; 3],
            distance: 3.0,
            // the native default view looks along -y with a slight tilt
            yaw: -std::f32::consts::FRAC_PI_2,
            pitch: std::f32::consts::FRAC_PI_2 - 0.35,
            orthographic: false,
        }
    }
}

impl Camera {
    /// Fit the camera to `[xmin, xmax, ymin, ymax, zmin, zmax]`.
    pub fn fit(&mut self, bounds: [f64; 6]) {
        let center = [
            ((bounds[0] + bounds[1]) * 0.5) as f32,
            ((bounds[2] + bounds[3]) * 0.5) as f32,
            ((bounds[4] + bounds[5]) * 0.5) as f32,
        ];
        let size = [
            (bounds[1] - bounds[0]) as f32,
            (bounds[3] - bounds[2]) as f32,
            (bounds[5] - bounds[4]) as f32,
        ];
        let diagonal = (size[0] * size[0] + size[1] * size[1] + size[2] * size[2])
            .sqrt()
            .max(1e-6);
        self.center = center;
        self.distance = 1.4 * diagonal;
    }

    /// View-projection matrix (wgpu clip space, column-major).
    pub fn view_proj(&self, aspect: f32) -> [[f32; 4]; 4] {
        let (sin_yaw, cos_yaw) = self.yaw.sin_cos();
        let (sin_pitch, cos_pitch) = self.pitch.sin_cos();
        let direction = [
            cos_pitch * cos_yaw,
            cos_pitch * sin_yaw,
            sin_pitch,
        ];
        let eye = [
            self.center[0] + self.distance * direction[0],
            self.center[1] + self.distance * direction[1],
            self.center[2] + self.distance * direction[2],
        ];
        let view = look_at(eye, self.center, [0.0, 0.0, 1.0]);
        let projection = if self.orthographic {
            let half = self.distance * 0.5;
            orthographic(-half * aspect, half * aspect, -half, half, 0.01 * self.distance, 20.0 * self.distance)
        } else {
            perspective(45f32.to_radians(), aspect.max(1e-3), 0.01 * self.distance, 50.0 * self.distance)
        };
        multiply(projection, view)
    }

    /// Native 2D/3D view presets (`0..=2` axis views, `3` free 3D).
    pub fn set_view_dir(&mut self, dir: i32) {
        match dir {
            0 => {
                // xy plane (view along -z)
                self.yaw = -std::f32::consts::FRAC_PI_2;
                self.pitch = std::f32::consts::FRAC_PI_2;
                self.orthographic = true;
            }
            1 => {
                // xz plane (view along +y)
                self.yaw = -std::f32::consts::FRAC_PI_2;
                self.pitch = 0.0;
                self.orthographic = true;
            }
            2 => {
                // yz plane (view along +x)
                self.yaw = 0.0;
                self.pitch = 0.0;
                self.orthographic = true;
            }
            _ => {
                self.yaw = -std::f32::consts::FRAC_PI_2;
                self.pitch = std::f32::consts::FRAC_PI_2 - 0.35;
                self.orthographic = false;
            }
        }
    }
}

fn multiply(a: [[f32; 4]; 4], b: [[f32; 4]; 4]) -> [[f32; 4]; 4] {
    let mut out = [[0.0f32; 4]; 4];
    for column in 0..4 {
        for row in 0..4 {
            out[column][row] = (0..4).map(|k| a[k][row] * b[column][k]).sum();
        }
    }
    out
}

fn look_at(eye: [f32; 3], center: [f32; 3], up: [f32; 3]) -> [[f32; 4]; 4] {
    let forward = normalize([center[0] - eye[0], center[1] - eye[1], center[2] - eye[2]]);
    let right = normalize(cross(forward, up));
    let true_up = cross(right, forward);
    [
        [right[0], true_up[0], -forward[0], 0.0],
        [right[1], true_up[1], -forward[1], 0.0],
        [right[2], true_up[2], -forward[2], 0.0],
        [
            -dot(right, eye),
            -dot(true_up, eye),
            dot(forward, eye),
            1.0,
        ],
    ]
}

fn perspective(fov_y: f32, aspect: f32, near: f32, far: f32) -> [[f32; 4]; 4] {
    let f = 1.0 / (fov_y * 0.5).tan();
    [
        [f / aspect, 0.0, 0.0, 0.0],
        [0.0, f, 0.0, 0.0],
        [0.0, 0.0, far / (near - far), -1.0],
        [0.0, 0.0, near * far / (near - far), 0.0],
    ]
}

fn orthographic(left: f32, right: f32, bottom: f32, top: f32, near: f32, far: f32) -> [[f32; 4]; 4] {
    [
        [2.0 / (right - left), 0.0, 0.0, 0.0],
        [0.0, 2.0 / (top - bottom), 0.0, 0.0],
        [0.0, 0.0, 1.0 / (near - far), 0.0],
        [
            -(right + left) / (right - left),
            -(top + bottom) / (top - bottom),
            near / (near - far),
            1.0,
        ],
    ]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn normalize(value: [f32; 3]) -> [f32; 3] {
    let length = dot(value, value).sqrt().max(1e-9);
    [value[0] / length, value[1] / length, value[2] / length]
}

/// wgpu-backed 3D renderer.
pub struct WgpuRenderer {
    adapter: Option<wgpu::Adapter>,
    device: Option<wgpu::Device>,
    queue: Option<wgpu::Queue>,
    config: Option<wgpu::SurfaceConfiguration>,
    triangle_pipeline: Option<wgpu::RenderPipeline>,
    line_pipeline: Option<wgpu::RenderPipeline>,
    bind_group: Option<wgpu::BindGroup>,
    uniform_buffer: Option<wgpu::Buffer>,
    depth_view: Option<wgpu::TextureView>,
    triangle_buffer: Option<wgpu::Buffer>,
    line_buffer: Option<wgpu::Buffer>,
    num_triangle_vertices: u32,
    num_line_vertices: u32,
    view_mode: ViewMode,
    view_dir: i32,
    parallel_projection: bool,
    camera: Camera,
    /// Fit the camera to the geometry on the next render.
    auto_fit: bool,
    /// Cached vertex data for the current structure.
    triangle_vertices: Vec<Vertex>,
    line_vertices: Vec<Vertex>,
}

impl Default for WgpuRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl WgpuRenderer {
    pub fn new() -> Self {
        Self {
            adapter: None,
            device: None,
            queue: None,
            config: None,
            triangle_pipeline: None,
            line_pipeline: None,
            bind_group: None,
            uniform_buffer: None,
            depth_view: None,
            triangle_buffer: None,
            line_buffer: None,
            num_triangle_vertices: 0,
            num_line_vertices: 0,
            view_mode: ViewMode::View3D,
            view_dir: 3,
            parallel_projection: false,
            camera: Camera::default(),
            auto_fit: true,
            triangle_vertices: Vec::new(),
            line_vertices: Vec::new(),
        }
    }

    pub fn camera(&self) -> &Camera {
        &self.camera
    }

    pub fn camera_mut(&mut self) -> &mut Camera {
        &mut self.camera
    }

    /// Depth attachment for the application render pass.
    pub fn depth_view(&self) -> Option<&wgpu::TextureView> {
        self.depth_view.as_ref()
    }

    /// Initialize the renderer with a wgpu adapter, device and queue.
    pub fn init(
        &mut self,
        adapter: wgpu::Adapter,
        device: wgpu::Device,
        queue: wgpu::Queue,
        config: &wgpu::SurfaceConfiguration,
    ) {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("qcsxcad shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });

        let bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("qcsxcad camera layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("qcsxcad camera uniform"),
            size: std::mem::size_of::<CameraUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("qcsxcad camera bind group"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("qcsxcad pipeline layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let make_pipeline = |topology: wgpu::PrimitiveTopology, label: &str| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<Vertex>() as wgpu::BufferAddress,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x4],
                    })],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: config.format,
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState {
                    topology,
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::LessEqual),
                    stencil: wgpu::StencilState::default(),
                    bias: wgpu::DepthBiasState::default(),
                }),
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        self.triangle_pipeline = Some(make_pipeline(
            wgpu::PrimitiveTopology::TriangleList,
            "qcsxcad triangle pipeline",
        ));
        self.line_pipeline = Some(make_pipeline(
            wgpu::PrimitiveTopology::LineList,
            "qcsxcad line pipeline",
        ));

        self.adapter = Some(adapter);
        self.device = Some(device);
        self.queue = Some(queue);
        self.config = Some(config.clone());
        self.uniform_buffer = Some(uniform_buffer);
        self.bind_group = Some(bind_group);
        self.create_depth(config.width, config.height);
    }

    fn create_depth(&mut self, width: u32, height: u32) {
        let Some(device) = self.device.as_ref() else {
            return;
        };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("qcsxcad depth"),
            size: wgpu::Extent3d {
                width: width.max(1),
                height: height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        self.depth_view = Some(texture.create_view(&wgpu::TextureViewDescriptor::default()));
    }

    /// Configure the surface (call after `init` and after window resize).
    pub fn configure_surface(
        &mut self,
        surface: &wgpu::Surface,
        device: &wgpu::Device,
        width: u32,
        height: u32,
    ) {
        let adapter = self.adapter.as_ref().expect("adapter not set");
        let caps = surface.get_capabilities(adapter);
        let color_spaces = caps.color_spaces(caps.formats[0]);
        let color_space = if color_spaces.contains(wgpu::SurfaceColorSpaces::SRGB) {
            wgpu::SurfaceColorSpace::Srgb
        } else {
            wgpu::SurfaceColorSpace::Auto
        };
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: caps.formats[0],
            width: width.max(1),
            height: height.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: wgpu::CompositeAlphaMode::Auto,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
            color_space,
        };
        surface.configure(device, &config);
        self.create_depth(config.width, config.height);
        self.config = Some(config);
    }

    /// Resize the surface (call on window resize).
    pub fn resize(
        &mut self,
        surface: &wgpu::Surface,
        device: &wgpu::Device,
        width: u32,
        height: u32,
    ) {
        self.configure_surface(surface, device, width, height);
    }

    /// Build CPU geometry from a structure (no GPU state required).
    pub fn build_geometry(&mut self, structure: &Structure) {
        self.triangle_vertices.clear();
        self.line_vertices.clear();

        let tessellator = Tessellator::new();
        for primitive in tessellator.tessellate(structure) {
            for triangle in &primitive.mesh.triangles {
                for vertex in triangle {
                    self.triangle_vertices.push(Vertex {
                        position: [vertex[0] as f32, vertex[1] as f32, vertex[2] as f32],
                        color: primitive.fill_color,
                    });
                }
            }
            for line in &primitive.mesh.lines {
                for vertex in line {
                    self.line_vertices.push(Vertex {
                        position: [vertex[0] as f32, vertex[1] as f32, vertex[2] as f32],
                        color: primitive.edge_color,
                    });
                }
            }
        }
        self.num_triangle_vertices = self.triangle_vertices.len() as u32;
        self.num_line_vertices = self.line_vertices.len() as u32;
        if self.auto_fit {
            let mut bounds: Option<[f64; 6]> = None;
            for vertex in &self.triangle_vertices {
                let position = [
                    vertex.position[0] as f64,
                    vertex.position[1] as f64,
                    vertex.position[2] as f64,
                ];
                bounds = Some(match bounds {
                    None => [
                        position[0], position[0], position[1], position[1],
                        position[2], position[2],
                    ],
                    Some(bounds) => [
                        bounds[0].min(position[0]),
                        bounds[1].max(position[0]),
                        bounds[2].min(position[1]),
                        bounds[3].max(position[1]),
                        bounds[4].min(position[2]),
                        bounds[5].max(position[2]),
                    ],
                });
            }
            if let Some(bounds) = bounds {
                self.camera.fit(bounds);
                self.auto_fit = false;
            }
        }
    }

    /// Upload geometry and the camera matrix to the GPU.
    fn upload_buffers(&mut self) {
        let Some(device) = self.device.as_ref() else {
            return;
        };
        if !self.triangle_vertices.is_empty() {
            self.triangle_buffer = Some(device.create_buffer_init(
                &wgpu::util::BufferInitDescriptor {
                    label: Some("qcsxcad triangle buffer"),
                    contents: bytemuck::cast_slice(&self.triangle_vertices),
                    usage: wgpu::BufferUsages::VERTEX,
                },
            ));
        }
        if !self.line_vertices.is_empty() {
            self.line_buffer = Some(device.create_buffer_init(
                &wgpu::util::BufferInitDescriptor {
                    label: Some("qcsxcad line buffer"),
                    contents: bytemuck::cast_slice(&self.line_vertices),
                    usage: wgpu::BufferUsages::VERTEX,
                },
            ));
        }
        let aspect = self
            .config
            .as_ref()
            .map(|config| config.width as f32 / config.height.max(1) as f32)
            .unwrap_or(1.0);
        let uniform = CameraUniform {
            view_proj: self.camera.view_proj(aspect),
        };
        if let (Some(queue), Some(buffer)) = (self.queue.as_ref(), self.uniform_buffer.as_ref()) {
            queue.write_buffer(buffer, 0, bytemuck::bytes_of(&uniform));
        }
    }

    /// Draw the 3D geometry within the given render pass.
    pub fn draw_3d(&self, render_pass: &mut wgpu::RenderPass<'_>) {
        let Some(bind_group) = self.bind_group.as_ref() else {
            return;
        };
        render_pass.set_bind_group(0, bind_group, &[]);
        if let (Some(pipeline), Some(buffer)) =
            (self.triangle_pipeline.as_ref(), self.triangle_buffer.as_ref())
        {
            if self.num_triangle_vertices > 0 {
                render_pass.set_pipeline(pipeline);
                render_pass.set_vertex_buffer(0, buffer.slice(..));
                render_pass.draw(0..self.num_triangle_vertices, 0..1);
            }
        }
        if let (Some(pipeline), Some(buffer)) =
            (self.line_pipeline.as_ref(), self.line_buffer.as_ref())
        {
            if self.num_line_vertices > 0 {
                render_pass.set_pipeline(pipeline);
                render_pass.set_vertex_buffer(0, buffer.slice(..));
                render_pass.draw(0..self.num_line_vertices, 0..1);
            }
        }
    }
}

impl StructureRenderer for WgpuRenderer {
    fn render(&mut self, structure: &Structure, _state: &GuiState) -> Result<(), GuiError> {
        self.device
            .as_ref()
            .ok_or_else(|| GuiError::Model("renderer not initialized: call init first".into()))?;
        self.triangle_pipeline
            .as_ref()
            .ok_or_else(|| GuiError::Model("render pipeline not initialized".into()))?;

        self.build_geometry(structure);
        self.upload_buffers();
        Ok(())
    }

    fn set_view_mode(&mut self, mode: ViewMode) {
        self.view_mode = mode;
    }

    fn best_view(&mut self) {
        self.view_dir = 3;
        self.camera.set_view_dir(3);
        self.auto_fit = true;
    }

    fn set_view_dir(&mut self, dir: i32) {
        self.view_dir = dir;
        self.camera.set_view_dir(dir);
    }

    fn set_parallel_projection(&mut self, enabled: bool) {
        self.parallel_projection = enabled;
        self.camera.orthographic = enabled;
    }

    fn export_image(&self, _path: &str) -> Result<(), GuiError> {
        Err(GuiError::Model("image export not yet implemented".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use csxcad_core::model::Primitive;

    fn project(view_proj: [[f32; 4]; 4], point: [f32; 3]) -> [f32; 4] {
        let mut out = [0.0f32; 4];
        for row in 0..4 {
            out[row] = (0..4)
                .map(|k| view_proj[k][row] * [point[0], point[1], point[2], 1.0][k])
                .sum();
        }
        out
    }

    #[test]
    fn camera_centers_the_structure() {
        let mut camera = Camera::default();
        camera.fit([-1.0, 1.0, -2.0, 2.0, 0.0, 4.0]);
        let view_proj = camera.view_proj(1.0);
        let ndc = project(view_proj, camera.center);
        assert!(ndc[3] > 0.0, "w {ndc:?}");
        assert!(
            (ndc[0] / ndc[3]).abs() < 1e-5 && (ndc[1] / ndc[3]).abs() < 1e-5,
            "center {ndc:?}"
        );
    }

    #[test]
    fn fitted_camera_keeps_the_bounds_visible() {
        let mut camera = Camera::default();
        let bounds = [-2.0, 2.0, -1.0, 1.0, 0.0, 3.0];
        camera.fit(bounds);
        for orthographic in [false, true] {
            camera.orthographic = orthographic;
            let view_proj = camera.view_proj(1.5);
            for corner in [
                &[bounds[0], bounds[2], bounds[4]],
                &[bounds[1], bounds[3], bounds[5]],
                &[bounds[0], bounds[3], bounds[4]],
                &[bounds[1], bounds[2], bounds[5]],
            ] {
                let point = [corner[0] as f32, corner[1] as f32, corner[2] as f32];
                let ndc = project(view_proj, point);
                assert!(ndc[3] > 0.0, "corner {point:?} behind the camera: {ndc:?}");
                for axis in 0..3 {
                    assert!(
                        (ndc[axis] / ndc[3]).abs() <= 1.0 + 1e-4,
                        "corner {point:?} outside the clip volume: {ndc:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn build_geometry_collects_triangles_and_lines() {
        let mut structure = Structure::new();
        let mut property =
            csxcad_core::model::Property::new("mat", csxcad_core::model::PropertyType::Material);
        property.add_primitive(Primitive::box_primitive([0.0, 0.0, 0.0], [1.0, 2.0, 3.0]));
        structure.add_property(property);
        let mut renderer = WgpuRenderer::new();
        renderer.build_geometry(&structure);
        assert_eq!(renderer.num_triangle_vertices, 36);
        assert_eq!(renderer.num_line_vertices, 24);
        // the fit camera looks at the box center
        assert!((renderer.camera.center[0] - 0.5).abs() < 1e-6);
        assert!((renderer.camera.center[1] - 1.0).abs() < 1e-6);
        assert!((renderer.camera.center[2] - 1.5).abs() < 1e-6);
    }
}
