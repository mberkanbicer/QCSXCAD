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
    bind_group_layout: Option<wgpu::BindGroupLayout>,
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
    /// Include discrete-material models (native `--RenderDiscMaterial`).
    render_discrete_models: bool,
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
            bind_group_layout: None,
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
            render_discrete_models: false,
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
        self.bind_group_layout = Some(bind_group_layout);
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

        let tessellator = Tessellator {
            resolution: 32,
            render_discrete: self.render_discrete_models,
        };
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
        self.draw_with(render_pass, bind_group);
    }

    fn draw_with(&self, render_pass: &mut wgpu::RenderPass<'_>, bind_group: &wgpu::BindGroup) {
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

impl WgpuRenderer {
    /// Render the current geometry into an offscreen RGBA8 image.
    ///
    /// Native `QVTKStructure::ExportView2Image` grabs the 3D render window;
    /// this renders the same pipelines and camera into an offscreen target so
    /// the export does not depend on the compositor.
    pub fn capture_rgba(&self, width: u32, height: u32) -> Result<Vec<u8>, GuiError> {
        let width = width.max(1);
        let height = height.max(1);
        let device = self
            .device
            .as_ref()
            .ok_or_else(|| GuiError::Model("renderer not initialized".into()))?;
        let queue = self
            .queue
            .as_ref()
            .ok_or_else(|| GuiError::Model("renderer not initialized".into()))?;
        let format = self
            .config
            .as_ref()
            .ok_or_else(|| GuiError::Model("renderer not initialized".into()))?
            .format;
        let layout = self
            .bind_group_layout
            .as_ref()
            .ok_or_else(|| GuiError::Model("camera layout not initialized".into()))?;
        let triangle_pipeline = self
            .triangle_pipeline
            .as_ref()
            .ok_or_else(|| GuiError::Model("render pipeline not initialized".into()))?;

        let color = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("qcsxcad capture color"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let color_view = color.create_view(&wgpu::TextureViewDescriptor::default());
        let depth = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("qcsxcad capture depth"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let depth_view = depth.create_view(&wgpu::TextureViewDescriptor::default());

        // A dedicated camera uniform keeps the window aspect untouched.
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("qcsxcad capture camera"),
            size: std::mem::size_of::<CameraUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(
            &uniform,
            0,
            bytemuck::bytes_of(&CameraUniform {
                view_proj: self.camera.view_proj(width as f32 / height as f32),
            }),
        );
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("qcsxcad capture camera bind group"),
            layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            }],
        });

        let padded_bytes_per_row = {
            let unpadded = width as usize * 4;
            ((unpadded + 255) / 256) * 256
        };
        let buffer_size = (padded_bytes_per_row * height as usize) as u64;
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("qcsxcad capture readback"),
            size: buffer_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("qcsxcad capture encoder"),
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("qcsxcad capture pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &color_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 1.0,
                            g: 1.0,
                            b: 1.0,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            self.draw_with(&mut pass, &bind_group);
            drop(pass);
        }
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &color,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_bytes_per_row as u32),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        queue.submit(std::iter::once(encoder.finish()));

        let slice = readback.slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|error| GuiError::Model(format!("capture poll failed: {error}")))?;
        receiver
            .recv()
            .map_err(|error| GuiError::Model(format!("capture map failed: {error}")))?
            .map_err(|error| GuiError::Model(format!("capture map error: {error:?}")))?;
        let view = slice
            .get_mapped_range()
            .map_err(|error| GuiError::Model(format!("capture read failed: {error}")))?;
        let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
        let bgra = matches!(
            format,
            wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb
        );
        for row in 0..height as usize {
            let start = row * padded_bytes_per_row;
            let end = start + width as usize * 4;
            for pixel in view[start..end].chunks_exact(4) {
                if bgra {
                    rgba.extend_from_slice(&[pixel[2], pixel[1], pixel[0], 255]);
                } else {
                    rgba.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 255]);
                }
            }
        }
        drop(view);
        readback.unmap();
        let _ = triangle_pipeline;
        Ok(rgba)
    }
}

impl StructureRenderer for WgpuRenderer {
    fn render(&mut self, structure: &Structure, state: &GuiState) -> Result<(), GuiError> {
        self.device
            .as_ref()
            .ok_or_else(|| GuiError::Model("renderer not initialized: call init first".into()))?;
        self.triangle_pipeline
            .as_ref()
            .ok_or_else(|| GuiError::Model("render pipeline not initialized".into()))?;

        self.render_discrete_models = state.render_disc_models;
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

    fn export_image(&self, path: &str) -> Result<(), GuiError> {
        let (width, height) = self
            .config
            .as_ref()
            .map(|config| (config.width, config.height))
            .ok_or_else(|| GuiError::Model("renderer not initialized".into()))?;
        let rgba = self.capture_rgba(width, height)?;
        write_png(path, width, height, &rgba)
    }
}

/// Encode RGBA8 pixels as a PNG file.
fn write_png(path: &str, width: u32, height: u32, rgba: &[u8]) -> Result<(), GuiError> {
    if rgba.len() != width as usize * height as usize * 4 {
        return Err(GuiError::Model(format!(
            "captured image is {} bytes, expected {}",
            rgba.len(),
            width as usize * height as usize * 4
        )));
    }
    let file = std::fs::File::create(path)
        .map_err(|error| GuiError::Model(format!("could not create {path}: {error}")))?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder
        .write_header()
        .map_err(|error| GuiError::Model(format!("PNG header failed: {error}")))?;
    writer
        .write_image_data(rgba)
        .map_err(|error| GuiError::Model(format!("PNG write failed: {error}")))
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

#[cfg(test)]
mod image_tests {
    use super::*;
    use csxcad_core::model::{Primitive, Property, PropertyColor, PropertyType, Structure};
    use qcsxcad_core::widgets::TreeWidget;

    pub(super) fn test_structure() -> Structure {
        let mut structure = Structure::new();
        let mut property = Property::new("mat", PropertyType::Material);
        property.set_fill_color(Some(PropertyColor::new(230, 120, 40, 255)));
        property.add_primitive(Primitive::box_primitive([-1.0; 3], [1.0; 3]));
        structure.add_property(property);
        structure
    }

    fn gpu() -> Option<(wgpu::Adapter, wgpu::Device, wgpu::Queue)> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            flags: wgpu::InstanceFlags::default(),
            backend_options: Default::default(),
            display: Default::default(),
            memory_budget_thresholds: Default::default(),
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: None,
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        }))
        .ok()?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            label: Some("qcsxcad capture test device"),
            trace: wgpu::Trace::default(),
            memory_hints: wgpu::MemoryHints::default(),
            experimental_features: wgpu::ExperimentalFeatures::default(),
        }))
        .ok()?;
        Some((adapter, device, queue))
    }

    fn offscreen_config(width: u32, height: u32) -> wgpu::SurfaceConfiguration {
        wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            width,
            height,
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: wgpu::CompositeAlphaMode::Auto,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
            color_space: wgpu::SurfaceColorSpace::Auto,
        }
    }

    fn decode(path: &std::path::Path) -> (u32, u32, Vec<u8>) {
        let decoder = png::Decoder::new(std::fs::File::open(path).expect("open png"));
        let mut reader = decoder.read_info().expect("png header");
        let mut buffer = vec![0; reader.output_buffer_size()];
        let info = reader.next_frame(&mut buffer).expect("png frame");
        buffer.truncate(info.buffer_size());
        (info.width, info.height, buffer)
    }

    #[test]
    fn png_writer_round_trips_through_the_decoder() {
        let directory = std::env::temp_dir().join(format!(
            "qcsxcad-png-writer-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&directory).expect("temp dir");
        let path = directory.join("pixel.png");
        let rgba = vec![10u8, 20, 30, 255, 40, 50, 60, 255];
        write_png(path.to_str().expect("path"), 2, 1, &rgba).expect("write png");
        let (width, height, pixels) = decode(&path);
        assert_eq!((width, height), (2, 1));
        assert_eq!(pixels, rgba);
        assert!(write_png(path.to_str().expect("path"), 2, 1, &rgba[..7]).is_err());
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn full_ui_screenshot() {
        use crate::widgets::{EguiGridEditor, EguiPrimitiveEditor, EguiPropertyEditor, EguiTreeWidget};

        let Some((adapter, device, queue)) = gpu() else {
            eprintln!("skipping the UI screenshot test: no adapter available");
            return;
        };
        let width = 1280u32;
        let height = 800u32;
        let config = offscreen_config(width, height);
        let mut renderer = WgpuRenderer::new();
        renderer.init(adapter, device.clone(), queue.clone(), &config);

        let mut document = qcsxcad_core::document::Document::from_structure(test_structure());
        let mut state = GuiState::new();
        let property = document
            .structure()
            .property_by_index(0)
            .and_then(|property| property.id())
            .expect("property");
        document
            .apply_property_edit(property, qcsxcad_core::document::PropertyEdit::Name("substrate".into()))
            .expect("name");
        let box_id = document
            .add_primitive(property, csxcad_core::model::PrimitiveType::Sphere)
            .expect("sphere");
        document
            .edit_primitive(box_id, |primitive| {
                if let csxcad_core::model::PrimitiveShape::Sphere(shape) = primitive.shape_mut() {
                    shape.set_radius(1.5);
                }
            })
            .expect("radius");
        state.selection.properties.insert(property);
        state.selection.primitives.insert(box_id);
        renderer
            .render(document.structure(), &state)
            .expect("render geometry");

        let mut tree = EguiTreeWidget::default();
        tree.set_selection(state.selection.clone());
        let mut property_editor = EguiPropertyEditor::default();
        let mut primitive_editor = EguiPrimitiveEditor::default();
        let mut grid_editor = EguiGridEditor::default();

        let ctx = egui::Context::default();
        let mut raw_input = egui::RawInput::default();
        raw_input.screen_rect = Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(width as f32, height as f32),
        ));
        let mut output = ctx.run_ui(raw_input, |ui| {
            egui::Panel::top("status").show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.strong("QCSXCAD");
                    ui.label("Rust GUI screenshot (offscreen wgpu)");
                });
            });
            egui::Panel::left("tree")
                .default_size(300.0)
                .show(ui, |ui| {
                    tree.show(ui, &mut document, &mut state);
                });
            egui::Panel::right("editors")
                .default_size(380.0)
                .show(ui, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        let _ = property_editor.show(ui, &mut document, &state);
                        ui.separator();
                        let _ = primitive_editor.show(ui, &mut document, &state);
                        ui.separator();
                        let _ = grid_editor.show(ui, &mut document);
                    });
                });
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(ui, |_| {});
        });
        let paint_jobs = ctx.tessellate(output.shapes, output.pixels_per_point);
        let mut egui_renderer = egui_wgpu::Renderer::new(
            &device,
            config.format,
            egui_wgpu::RendererOptions {
                depth_stencil_format: Some(wgpu::TextureFormat::Depth32Float),
                ..Default::default()
            },
        );
        for (id, deltas) in &output.textures_delta.set {
            for delta in deltas {
                egui_renderer.update_texture(&device, &queue, *id, delta);
            }
        }
        output.textures_delta.set.clear();
        output.textures_delta.free.clear();

        let color = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("qcsxcad ui screenshot"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: config.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let color_view = color.create_view(&wgpu::TextureViewDescriptor::default());
        let depth = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("qcsxcad ui depth"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let depth_view = depth.create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("qcsxcad ui screenshot encoder"),
        });
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [width, height],
            pixels_per_point: output.pixels_per_point,
        };
        let commands = egui_renderer.update_buffers(&device, &queue, &mut encoder, &paint_jobs, &screen);
        {
            let render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("qcsxcad ui screenshot pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &color_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 1.0,
                            g: 1.0,
                            b: 1.0,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            let mut pass = render_pass.forget_lifetime();
            renderer.draw_3d(&mut pass);
            egui_renderer.render(&mut pass, &paint_jobs, &screen);
            drop(pass);
        }
        queue.submit(commands);
        queue.submit(std::iter::once(encoder.finish()));

        let rgba = read_texture(&device, &queue, &color, width, height, config.format)
            .expect("read back the screenshot");
        let path = std::env::temp_dir().join("qcsxcad-ui.png");
        write_png(path.to_str().expect("path"), width, height, &rgba).expect("write screenshot");
        let (saved_width, saved_height, pixels) = decode(&path);
        assert_eq!((saved_width, saved_height), (width, height));
        // the UI must contain more than the flat wgpu clear color
        let colors = pixels
            .chunks_exact(4)
            .map(|pixel| (pixel[0], pixel[1], pixel[2]))
            .collect::<std::collections::HashSet<_>>()
            .len();
        eprintln!("UI screenshot: {path:?} ({colors} distinct colors)");
        assert!(colors > 50, "the screenshot looks empty ({colors} colors)");
    }

    /// Read a rendered texture back as RGBA8.
    fn read_texture(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        texture: &wgpu::Texture,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
    ) -> Result<Vec<u8>, GuiError> {
        let padded_bytes_per_row = {
            let unpadded = width as usize * 4;
            ((unpadded + 255) / 256) * 256
        };
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("qcsxcad screenshot readback"),
            size: (padded_bytes_per_row * height as usize) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("qcsxcad screenshot readback encoder"),
        });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_bytes_per_row as u32),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        queue.submit(std::iter::once(encoder.finish()));
        let slice = buffer.slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|error| GuiError::Model(format!("readback poll failed: {error}")))?;
        receiver
            .recv()
            .map_err(|error| GuiError::Model(format!("readback map failed: {error}")))?
            .map_err(|error| GuiError::Model(format!("readback map error: {error:?}")))?;
        let view = slice
            .get_mapped_range()
            .map_err(|error| GuiError::Model(format!("readback failed: {error}")))?;
        let bgra = matches!(
            format,
            wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb
        );
        let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
        for row in 0..height as usize {
            let start = row * padded_bytes_per_row;
            let end = start + width as usize * 4;
            for pixel in view[start..end].chunks_exact(4) {
                if bgra {
                    rgba.extend_from_slice(&[pixel[2], pixel[1], pixel[0], 255]);
                } else {
                    rgba.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 255]);
                }
            }
        }
        drop(view);
        buffer.unmap();
        Ok(rgba)
    }

    #[test]
    fn offscreen_capture_renders_the_structure_to_a_png() {
        let Some((adapter, device, queue)) = gpu() else {
            eprintln!("skipping the GPU capture test: no adapter available");
            return;
        };
        let config = offscreen_config(64, 48);
        let mut renderer = WgpuRenderer::new();
        renderer.init(adapter, device, queue, &config);
        let structure = test_structure();
        renderer
            .render(&structure, &GuiState::new())
            .expect("render geometry");

        let rgba = renderer.capture_rgba(64, 48).expect("capture");
        assert_eq!(rgba.len(), 64 * 48 * 4);
        // the clear color is 0.1 linear; the orange box must cover pixels
        let colored = rgba
            .chunks_exact(4)
            .filter(|pixel| pixel[0] > 120 && pixel[1] < 220)
            .count();
        assert!(colored > 100, "only {colored} non-background pixels");

        let directory = std::env::temp_dir().join(format!(
            "qcsxcad-png-capture-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&directory).expect("temp dir");
        let path = directory.join("view.png");
        renderer
            .export_image(path.to_str().expect("path"))
            .expect("export image");
        let (width, height, pixels) = decode(&path);
        assert_eq!((width, height), (64, 48));
        assert_eq!(pixels.len(), 64 * 48 * 4);
        let colored = pixels
            .chunks_exact(4)
            .filter(|pixel| pixel[0] > 120 && pixel[1] < 220)
            .count();
        assert!(colored > 100, "the PNG lost the geometry ({colored} pixels)");
        let _ = std::fs::remove_dir_all(&directory);
    }
}
