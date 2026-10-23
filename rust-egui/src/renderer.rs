//! wgpu-based 3D structure renderer for QCSXCAD.
//!
//! Provides a `WgpuRenderer` implementing `qcsxcad_core::rendering::StructureRenderer`.

use csxcad_core::model::{Primitive, PropertyType, Structure};
use qcsxcad_core::model::{GuiError, GuiState, ViewMode};
use qcsxcad_core::rendering::StructureRenderer;
use wgpu::util::DeviceExt;

/// A simple 3D vertex: position + color.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    position: [f32; 3],
    color: [f32; 4],
}

/// wgpu-backed 3D renderer.
pub struct WgpuRenderer {
    adapter: Option<wgpu::Adapter>,
    device: Option<wgpu::Device>,
    queue: Option<wgpu::Queue>,
    config: Option<wgpu::SurfaceConfiguration>,
    render_pipeline: Option<wgpu::RenderPipeline>,
    vertex_buffer: Option<wgpu::Buffer>,
    index_buffer: Option<wgpu::Buffer>,
    num_indices: u32,
    view_mode: ViewMode,
    view_dir: i32,
    parallel_projection: bool,
    /// Cached vertex data for the current structure.
    vertices: Vec<Vertex>,
    indices: Vec<u16>,
}

impl WgpuRenderer {
    /// Create a new renderer.
    pub fn new() -> Self {
        Self {
            adapter: None,
            device: None,
            queue: None,
            config: None,
            render_pipeline: None,
            vertex_buffer: None,
            index_buffer: None,
            num_indices: 0,
            view_mode: ViewMode::View3D,
            view_dir: 0,
            parallel_projection: false,
            vertices: Vec::new(),
            indices: Vec::new(),
        }
    }

    /// Initialize the renderer with a wgpu adapter, device and queue.
    /// The surface must be configured separately via `configure_surface`.
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

        let pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("qcsxcad pipeline layout"),
                bind_group_layouts: &[],
                immediate_size: 0,
            });

        let render_pipeline =
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("qcsxcad render pipeline"),
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
                        blend: Some(wgpu::BlendState::REPLACE),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::LineList,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            });

        self.adapter = Some(adapter);
        self.device = Some(device);
        self.queue = Some(queue);
        self.config = Some(config.clone());
        self.render_pipeline = Some(render_pipeline);
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

    /// Build vertex/index data from a structure for rendering.
    fn build_geometry(&mut self, structure: &Structure) {
        self.vertices.clear();
        self.indices.clear();

        let mut next_index: u16 = 0;
        for prim in structure.all_primitives(false, PropertyType::Any) {
            self.add_primitive_geometry(prim, &mut next_index);
        }

        self.num_indices = self.indices.len() as u32;
    }

    fn add_primitive_geometry(&mut self, prim: &Primitive, next_index: &mut u16) {
        let color = match prim.primitive_type() {
            csxcad_core::model::PrimitiveType::Box => [0.2, 0.6, 1.0, 1.0],
            csxcad_core::model::PrimitiveType::Sphere => [1.0, 0.6, 0.2, 1.0],
            csxcad_core::model::PrimitiveType::Cylinder => [0.2, 1.0, 0.6, 1.0],
            _ => [0.8, 0.8, 0.8, 1.0],
        };

        // A unit cube wireframe (12 lines = 24 vertices).
        let corners: [[f32; 3]; 8] = [
            [-0.5, -0.5, -0.5],
            [0.5, -0.5, -0.5],
            [0.5, 0.5, -0.5],
            [-0.5, 0.5, -0.5],
            [-0.5, -0.5, 0.5],
            [0.5, -0.5, 0.5],
            [0.5, 0.5, 0.5],
            [-0.5, 0.5, 0.5],
        ];

        let edges: [[usize; 2]; 12] = [
            [0, 1], [1, 2], [2, 3], [3, 0], // bottom face
            [4, 5], [5, 6], [6, 7], [7, 4], // top face
            [0, 4], [1, 5], [2, 6], [3, 7], // vertical edges
        ];

        let base = *next_index;
        for corner in corners {
            self.vertices.push(Vertex {
                position: corner,
                color,
            });
        }
        for edge in edges {
            self.indices.push(base + edge[0] as u16);
            self.indices.push(base + edge[1] as u16);
        }
        *next_index = base + corners.len() as u16;
    }

    /// Upload geometry to GPU buffers. Call after `build_geometry`.
    fn upload_buffers(&mut self) {
        let device = self.device.as_ref().unwrap();

        if !self.vertices.is_empty() {
            self.vertex_buffer = Some(device.create_buffer_init(
                &wgpu::util::BufferInitDescriptor {
                    label: Some("qcsxcad vertex buffer"),
                    contents: bytemuck::cast_slice(&self.vertices),
                    usage: wgpu::BufferUsages::VERTEX,
                },
            ));
        }
        if !self.indices.is_empty() {
            self.index_buffer = Some(device.create_buffer_init(
                &wgpu::util::BufferInitDescriptor {
                    label: Some("qcsxcad index buffer"),
                    contents: bytemuck::cast_slice(&self.indices),
                    usage: wgpu::BufferUsages::INDEX,
                },
            ));
        }
    }
}

impl Default for WgpuRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl StructureRenderer for WgpuRenderer {
    fn render(&mut self, structure: &Structure, _state: &GuiState) -> Result<(), GuiError> {
        // Check that renderer is initialized (immutable borrows, dropped immediately).
        let _device = self.device.as_ref().ok_or_else(|| {
            GuiError::Model("renderer not initialized: call init first".into())
        })?;
        let _queue = self.queue.as_ref().ok_or_else(|| {
            GuiError::Model("renderer not initialized: call init first".into())
        })?;
        let _pipeline = self.render_pipeline.as_ref().ok_or_else(|| {
            GuiError::Model("render pipeline not initialized".into())
        })?;

        // Rebuild geometry from structure (mutable borrow of self).
        self.build_geometry(structure);
        self.upload_buffers();

        Ok(())
    }

    fn set_view_mode(&mut self, mode: ViewMode) {
        self.view_mode = mode;
    }

    fn best_view(&mut self) {
        self.view_dir = 0;
    }

    fn set_view_dir(&mut self, dir: i32) {
        self.view_dir = dir;
    }

    fn set_parallel_projection(&mut self, enabled: bool) {
        self.parallel_projection = enabled;
    }

    fn export_image(&self, _path: &str) -> Result<(), GuiError> {
        Err(GuiError::Model("image export not yet implemented".into()))
    }
}