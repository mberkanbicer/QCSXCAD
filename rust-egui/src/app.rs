//! Main application struct tying together egui widgets and wgpu renderer.

use csxcad_core::model::{PrimitiveId, PropertyId, PropertyType, Structure};
use qcsxcad_core::document::Document;
use qcsxcad_core::model::{GuiError, GuiState, Selection};
use qcsxcad_core::rendering::StructureRenderer;
use qcsxcad_core::widgets::{TransformMode, TreeWidget};

pub use qcsxcad_core::widgets::TransformMode as SharedTransformMode;
use winit::event::{Event, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::Window;

use crate::renderer::{Camera, WgpuRenderer};
use crate::theme;
use crate::widgets::{
    EguiGridEditor, EguiPrimitiveEditor, EguiPropertyEditor, EguiTransformEditor,
    EguiTreeWidget,
};

/// The main QCSXCAD application: egui UI + wgpu 3D viewport.
pub struct EguiApp {
    window: Option<Window>,
    surface: Option<wgpu::Surface<'static>>,
    device: Option<wgpu::Device>,
    queue: Option<wgpu::Queue>,
    config: Option<wgpu::SurfaceConfiguration>,
    renderer: WgpuRenderer,
    egui_ctx: egui::Context,
    egui_winit_state: Option<egui_winit::State>,
    egui_wgpu_renderer: Option<egui_wgpu::Renderer>,
    tree_widget: EguiTreeWidget,
    grid_editor: EguiGridEditor,
    transform_editor: EguiTransformEditor,
    property_editor: EguiPropertyEditor,
    primitive_editor: EguiPrimitiveEditor,
    document: Option<Document>,
    /// Revision of the structure the current GPU geometry was built from.
    last_geometry_revision: u64,
    /// Discrete-material rendering state the geometry was built with.
    last_render_disc_models: bool,
    gui_state: GuiState,
    /// Middle-button camera orbit in progress.
    orbiting: bool,
    /// Viewport rectangle from the last frame (status bar unprojection).
    viewport_rect: egui::Rect,
    /// Last pointer position in egui points (navigation gating).
    last_pointer: Option<egui::Pos2>,
    /// Persisted dock widths (logical points).
    dock_tree_width: f32,
    dock_inspector_width: f32,
    /// Drag-docking layout (ADR 0010); only with the `docking` feature.
    #[cfg(feature = "docking")]
    dock_layout: crate::docking::DockLayout,
    last_cursor: Option<(f64, f64)>,
}

impl EguiApp {
    /// Create a new application. The window must be set later via
    /// `set_window` before `run` can be called.
    pub fn new() -> Self {
        Self {
            window: None,
            surface: None,
            device: None,
            queue: None,
            config: None,
            renderer: WgpuRenderer::new(),
            egui_ctx: egui::Context::default(),
            egui_winit_state: None,
            egui_wgpu_renderer: None,
            tree_widget: EguiTreeWidget::default(),
            grid_editor: EguiGridEditor::default(),
            transform_editor: EguiTransformEditor::default(),
            property_editor: EguiPropertyEditor::default(),
            primitive_editor: EguiPrimitiveEditor::default(),
            document: None,
            last_geometry_revision: 0,
            last_render_disc_models: false,
            gui_state: GuiState::new(),
            orbiting: false,
            viewport_rect: egui::Rect::NOTHING,
            last_pointer: None,
            dock_tree_width: 320.0,
            dock_inspector_width: 360.0,
            #[cfg(feature = "docking")]
            dock_layout: crate::docking::DockLayout::default(),
            last_cursor: None,
        }
    }

    /// Set the winit window and configure the wgpu surface.
    pub async fn set_window(&mut self, window: Window) {
        let size = window.inner_size();
        self.window = Some(window);

        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            flags: wgpu::InstanceFlags::default(),
            backend_options: Default::default(),
            display: Default::default(),
            memory_budget_thresholds: Default::default(),
        });
        let surface = instance
            .create_surface(self.window.as_ref().unwrap())
            .unwrap();
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
                apply_limit_buckets: false,
            })
            .await
            .expect("failed to find wgpu adapter");
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                label: Some("qcsxcad device"),
                trace: wgpu::Trace::default(),
                memory_hints: wgpu::MemoryHints::default(),
                experimental_features: wgpu::ExperimentalFeatures::default(),
            })
            .await
            .expect("failed to create wgpu device");
        let config = surface
            .get_default_config(&adapter, size.width, size.height)
            .unwrap();
        surface.configure(&device, &config);

        // Store wgpu resources for egui integration.
        // SAFETY: The surface borrows from self.window, which is owned by this struct.
        // Both are dropped together, so the surface never outlives the window.
        let surface_static = unsafe {
            std::mem::transmute::<wgpu::Surface, wgpu::Surface<'static>>(surface)
        };
        self.surface = Some(surface_static);
        self.device = Some(device.clone());
        self.queue = Some(queue.clone());
        self.config = Some(config.clone());

        // Initialize egui-winit state.
        let window = self.window.as_ref().unwrap();
        let egui_winit_state = egui_winit::State::new(
            self.egui_ctx.clone(),
            egui::ViewportId::ROOT,
            window,
            None,
            None,
            Some(4096),
        );
        self.egui_winit_state = Some(egui_winit_state);

        // Initialize egui-wgpu renderer.
        let egui_device = self.device.as_ref().unwrap();
        let config = self.config.as_ref().unwrap();
        let egui_wgpu_renderer = egui_wgpu::Renderer::new(
            egui_device,
            config.format,
            egui_wgpu::RendererOptions {
                // the frame renders the 3D structure with a depth attachment,
                // so the egui pipeline has to declare the same depth state
                depth_stencil_format: Some(wgpu::TextureFormat::Depth32Float),
                ..Default::default()
            },
        );
        self.egui_wgpu_renderer = Some(egui_wgpu_renderer);

        self.renderer.init(adapter, device, queue, &config);
        self.renderer
            .configure_surface(self.surface.as_ref().unwrap(), egui_device, size.width, size.height);
    }

    /// Resize the wgpu surface and egui state.
    pub fn resize(&mut self, width: u32, height: u32) {
        let surface = self.surface.as_ref().unwrap();
        let device = self.device.as_ref().unwrap();
        if let Some(config) = self.config.as_mut() {
            config.width = width.max(1);
            config.height = height.max(1);
            surface.configure(device, config);
        }
        self.renderer.resize(surface, device, width, height);
    }

    /// Handle a winit event. Returns true if the event was consumed by egui.
    pub fn handle_event(&mut self, event: &Event<()>) -> bool {
        if let Event::WindowEvent { window_id: _, event } = event {
            self.handle_navigation(event);
            if let Some(window) = self.window.as_ref() {
                if let Some(egui_state) = self.egui_winit_state.as_mut() {
                    let response = egui_state.on_window_event(window, event);
                    // Input changes the UI: repaint once (the event loop no
                    // longer redraws continuously).
                    window.request_redraw();
                    return response.consumed;
                }
            }
        }
        false
    }

    /// True when the pointer was over the 3D viewport on the last frame.
    /// Navigation is confined to the viewport so scrolling the tree does not
    /// zoom the camera.
    fn pointer_over_viewport(&self) -> bool {
        self.last_pointer
            .map(|position| self.viewport_rect.contains(position))
            .unwrap_or(false)
    }

    /// Native navigation, independent of the egui panels: middle-drag orbits
    /// and the wheel zooms. Kept separate from `handle_event` so it can be
    /// tested without a window.
    pub fn handle_navigation(&mut self, event: &WindowEvent) {
        match event {
            WindowEvent::MouseWheel { delta, .. } => {
                let amount = match delta {
                    winit::event::MouseScrollDelta::LineDelta(_, y) => *y,
                    winit::event::MouseScrollDelta::PixelDelta(position) => {
                        position.y as f32 * 0.01
                    }
                };
                if amount != 0.0 && self.pointer_over_viewport() {
                    let camera = self.renderer.camera_mut();
                    camera.distance =
                        (camera.distance * (1.0 - 0.1 * amount)).clamp(1.0e-6, 1.0e6);
                    trace_input(format!("zoom distance={:.5}", camera.distance));
                }
            }
            WindowEvent::MouseInput {
                state,
                button: winit::event::MouseButton::Middle,
                ..
            } => {
                self.orbiting = *state == winit::event::ElementState::Pressed
                    && self.pointer_over_viewport();
                if !self.orbiting {
                    self.last_cursor = None;
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                let points = self.egui_ctx.pixels_per_point().max(1.0e-3);
                self.last_pointer = Some(egui::pos2(
                    position.x as f32 / points,
                    position.y as f32 / points,
                ));
                if !self.orbiting {
                    return;
                }
                if let Some((last_x, last_y)) = self.last_cursor {
                    let dx = position.x as f32 - last_x as f32;
                    let dy = position.y as f32 - last_y as f32;
                    let camera = self.renderer.camera_mut();
                    camera.yaw -= dx * 0.01;
                    camera.pitch = (camera.pitch + dy * 0.01).clamp(-1.5, 1.5);
                    trace_input(format!(
                        "orbit dx={dx:.1} dy={dy:.1} yaw={:.4} pitch={:.4}",
                        camera.yaw, camera.pitch
                    ));
                }
                self.last_cursor = Some((position.x, position.y));
            }
            WindowEvent::CursorLeft { .. } => {
                self.orbiting = false;
                self.last_cursor = None;
            }
            _ => {}
        }
    }

    /// Render one frame: egui UI + wgpu 3D viewport.
    pub fn render(&mut self) -> Result<(), GuiError> {
        self.render_with(|_, _, _| {})
    }
    /// Render one frame, letting the embedding application draw extra UI (the
    /// AppCSXCAD menu) with access to the document and the GUI state.
    pub fn render_with(
        &mut self,
        mut extra_ui: impl FnMut(&mut egui::Ui, &mut Document, &mut GuiState),
    ) -> Result<(), GuiError> {
        // Begin egui frame: gather input and run UI.
        // Use temporary borrows so no `self` field is borrowed across `run_ui`.
        let raw_input = self
            .egui_winit_state
            .as_mut()
            .unwrap()
            .take_egui_input(self.window.as_ref().unwrap());

        // Temporarily take egui_ctx to avoid borrow conflict: run_ui needs &mut egui_ctx,
        // while the closure needs &mut self (for build_ui -> widget show methods).
        let egui_ctx = std::mem::replace(&mut self.egui_ctx, egui::Context::default());
        let full_output = egui_ctx.run_ui(raw_input, |ui| {
            self.build_ui(ui, &mut extra_ui);
        });
        self.egui_ctx = egui_ctx;

        // Keep the swapchain in sync with the window. Maximizing the window or
        // a DPI change can leave the surface configuration outdated, and
        // acquiring a texture then fails until it is reconfigured.
        if let Some(window) = self.window.as_ref() {
            let size = window.inner_size();
            let (width, height) = (size.width.max(1), size.height.max(1));
            let stale = match self.config.as_ref() {
                Some(config) => config.width != width || config.height != height,
                None => false,
            };
            if stale {
                if let (Some(config), Some(surface), Some(device)) = (
                    self.config.as_mut(),
                    self.surface.as_ref(),
                    self.device.as_ref(),
                ) {
                    config.width = width;
                    config.height = height;
                    surface.configure(device, config);
                    self.renderer.resize(surface, device, width, height);
                }
            }
        }

        // Now re-borrow everything we need (run_ui is done, no more mutable self borrow).
        let surface = self.surface.as_ref().unwrap();
        let device = self.device.as_ref().unwrap();
        let queue = self.queue.as_ref().unwrap();
        let config = self.config.as_ref().unwrap();
        let egui_wgpu_renderer = self.egui_wgpu_renderer.as_mut().unwrap();
        let egui_winit_state = self.egui_winit_state.as_mut().unwrap();

        // Tessellate egui shapes into paint jobs.
        let paint_jobs = self
            .egui_ctx
            .tessellate(full_output.shapes, full_output.pixels_per_point);

        // Update egui textures before painting.
        for (id, image_deltas) in &full_output.textures_delta.set {
            for image_delta in image_deltas {
                egui_wgpu_renderer.update_texture(device, queue, *id, image_delta);
            }
        }

        // Get the surface texture for this frame. `Outdated`/`Lost` need a
        // reconfigure; `Timeout`/`Occluded` simply skip the frame.
        let surface_texture = {
            let mut attempt = 0;
            loop {
                match surface.get_current_texture() {
                    wgpu::CurrentSurfaceTexture::Success(surface_texture)
                    | wgpu::CurrentSurfaceTexture::Suboptimal(surface_texture) => {
                        break surface_texture
                    }
                    wgpu::CurrentSurfaceTexture::Outdated
                    | wgpu::CurrentSurfaceTexture::Lost => {
                        if attempt >= 2 {
                            eprintln!("surface could not be reconfigured");
                            trace_input("render: surface reconfigure failed");
                            return Ok(());
                        }
                        attempt += 1;
                        surface.configure(device, config);
                    }
                    wgpu::CurrentSurfaceTexture::Timeout
                    | wgpu::CurrentSurfaceTexture::Occluded => {
                        trace_input("render: surface skipped (timeout/occluded)");
                        return Ok(());
                    }
                    other => {
                        eprintln!("surface get_current_texture failed: {other:?}");
                        return Ok(());
                    }
                }
            }
        };
        let view = surface_texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        // Rebuild the 3D geometry only when the document changed.
        if let Some(document) = self.document.as_ref() {
            let revision = document.revision().max(1);
            let render_disc_models = self.gui_state.render_disc_models;
            if revision != self.last_geometry_revision
                || render_disc_models != self.last_render_disc_models
            {
                let structure = document.structure();
                self.renderer.render(structure, &self.gui_state)?;
                self.last_geometry_revision = revision;
                self.last_render_disc_models = render_disc_models;
            }
        }

        // Begin render pass. The clear colour follows the active theme so the
        // viewport reads as part of the workbench.
        let [clear_r, clear_g, clear_b] = self.renderer.background();
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("qcsxcad encoder"),
        });
        // egui requires `update_buffers` before its render pass; the command
        // buffers it returns must be submitted with the frame
        let screen_descriptor = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [config.width, config.height],
            pixels_per_point: full_output.pixels_per_point,
        };
        let egui_command_buffers = egui_wgpu_renderer.update_buffers(
            device,
            queue,
            &mut encoder,
            &paint_jobs,
            &screen_descriptor,
        );

        let render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("qcsxcad render pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: clear_r,
                        g: clear_g,
                        b: clear_b,
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            depth_stencil_attachment: self.renderer.depth_view().map(|view| {
                wgpu::RenderPassDepthStencilAttachment {
                    view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        let mut pass = render_pass.forget_lifetime();

        // Draw 3D viewport.
        self.renderer.draw_3d(&mut pass);

        // Render egui UI.
        egui_wgpu_renderer.render(&mut pass, &paint_jobs, &screen_descriptor);

        drop(pass);
        queue.submit(egui_command_buffers);
        queue.submit(std::iter::once(encoder.finish()));
        queue.present(surface_texture);

        // Free egui textures after painting.
        for id in &full_output.textures_delta.free {
            egui_wgpu_renderer.free_texture(id);
        }

        // Handle egui platform output (cursor, clipboard, etc.).
        egui_winit_state
            .handle_platform_output(self.window.as_ref().unwrap(), full_output.platform_output);

        // Only ask for another frame when egui still needs one. Redrawing
        // unconditionally spins the event loop at 100% CPU and starves the
        // swapchain, which shows up as a frozen window.
        if self.egui_ctx.has_requested_repaint() {
            if let Some(window) = self.window.as_ref() {
                window.request_redraw();
            }
        }

        Ok(())
    }


fn structure_pane_ui(
    ui: &mut egui::Ui,
    document: &mut Option<Document>,
    gui_state: &mut GuiState,
    tree_widget: &mut EguiTreeWidget,
    grid_editor: &mut EguiGridEditor,
    palette: &theme::Palette,
) {

    ui.add_space(2.0);
    ui.label(
        egui::RichText::new("Structure")
            .size(14.0)
            .strong()
            .color(palette.text),
    );
    // The tree keeps its own scroll area; bound it so the grid
    // and parameter sections below stay reachable at 800 px.
    let tree_height = (ui.available_height() * 0.46).max(180.0);
    ui.allocate_ui(
        egui::vec2(ui.available_width(), tree_height),
        |ui| {
            if let Some(document) = document.as_mut() {
                tree_widget.show(ui, document, gui_state);
            }
        },
    );
    ui.separator();
    egui::ScrollArea::vertical()
        .id_salt("grid_and_parameters")
        .show(ui, |ui| {
            egui::CollapsingHeader::new("Rectilinear Grid")
                .default_open(true)
                .show(ui, |ui| {
                    if let Some(document) = document.as_mut() {
                        let _ = grid_editor.show_mesh(ui, document);
                    }
                });
            ui.separator();
            egui::CollapsingHeader::new("Parameter")
                .default_open(true)
                .show(ui, |ui| {
                    if let Some(document) = document.as_mut() {
                        let _ = grid_editor.show_parameters(ui, document);
                    }
                });
        });

}

fn inspector_pane_ui(
    ui: &mut egui::Ui,
    document: &mut Option<Document>,
    gui_state: &mut GuiState,
    property_editor: &mut EguiPropertyEditor,
    primitive_editor: &mut EguiPrimitiveEditor,
    transform_editor: &mut EguiTransformEditor,
    palette: &theme::Palette,
) {

    ui.add_space(2.0);
    ui.label(
        egui::RichText::new("Inspector")
            .size(14.0)
            .strong()
            .color(palette.text),
    );
    ui.separator();
    if let Some(document) = document.as_mut() {
        egui::ScrollArea::vertical().show(ui, |ui| {
            let _ = property_editor.show(ui, document, gui_state);
            ui.separator();
            let _ = primitive_editor.show(ui, document, gui_state);
            ui.separator();
            transform_editor.show(ui, document, gui_state);
        });
    }

}

fn viewport_pane_ui(
    ui: &mut egui::Ui,
    document: &mut Option<Document>,
    gui_state: &mut GuiState,
    tree_widget: &mut EguiTreeWidget,
    renderer: &WgpuRenderer,
    viewport_rect: &mut egui::Rect,
    palette: &theme::Palette,
) {

    let rect = ui.max_rect();
    *viewport_rect = rect;
    let response = ui.allocate_rect(rect, egui::Sense::click_and_drag());
    draw_axis_gizmo(ui, rect, renderer.camera(), palette);
    let camera = *renderer.camera();
    let height = rect.height().max(1.0);
    let aspect = rect.width() / height;
    // Drag the primitive under the pointer. If nothing is selected
    // the press picks the primitive first, so a single gesture
    // selects and moves it.
    if response.drag_started() {
        if let (Some(position), Some(document)) =
            (response.interact_pointer_pos(), document.as_ref())
        {
            if gui_state.selection.primitives.is_empty() {
                if let Some(id) = pick_primitive(
                    renderer.camera(),
                    document.structure(),
                    rect,
                    position,
                ) {
                    trace_input(format!(
                        "pick id={} at=({:.0},{:.0})",
                        id.get(),
                        position.x,
                        position.y
                    ));
                    let mut selection = Selection::new();
                    selection.primitives.insert(id);
                    gui_state.selection = selection.clone();
                    tree_widget.set_selection(selection);
                }
            }
        }
        if !gui_state.selection.primitives.is_empty() {
            if let Some(document) = document.as_mut() {
                document.begin_gesture();
            }
            if let Some(id) = gui_state.selection.primitives.iter().copied().next() {
                trace_input(format!(
                    "drag_start id={} mode={:?}",
                    id.get(),
                    gui_state.transform_mode
                ));
            }
        }
    }
    if response.dragged() {
        if let Some(id) = gui_state.selection.primitives.iter().copied().next() {
            // Per-frame pointer movement: `drag_delta` is the whole
            // gesture and would compound every frame.
            let delta = ui.input(|input| input.pointer.delta());
            if delta == egui::Vec2::ZERO {
                // Nothing moved this frame; skip the model edit.
                return;
            }
            let rotate = ui.input(|input| input.modifiers.alt)
                || gui_state.transform_mode == TransformMode::Rotate;
            let scale = gui_state.transform_mode == TransformMode::Scale;
            let world_per_point = if camera.orthographic {
                camera.distance / height
            } else {
                2.0 * camera.distance * (22.5f32.to_radians()).tan() / height
            };
            let (sin_yaw, cos_yaw) = camera.yaw.sin_cos();
            let right = [-sin_yaw, cos_yaw, 0.0];
            let (sin_pitch, cos_pitch) = camera.pitch.sin_cos();
            let forward = [-cos_pitch * cos_yaw, -cos_pitch * sin_yaw, -sin_pitch];
            let up = [
                right[1] * forward[2] - right[2] * forward[1],
                right[2] * forward[0] - right[0] * forward[2],
                right[0] * forward[1] - right[1] * forward[0],
            ];
            let world = [
                (right[0] * delta.x - up[0] * delta.y) * world_per_point,
                (right[1] * delta.x - up[1] * delta.y) * world_per_point,
                (right[2] * delta.x - up[2] * delta.y) * world_per_point,
            ];
            if let Some(document) = document.as_mut() {
                if scale {
                    let factor =
                        (1.0 + delta.x as f64 * 0.01).clamp(0.1, 10.0);
                    let _ = document.scale_primitive_by(id, factor);
                    trace_input(format!("scale id={} factor={factor:.5}", id.get()));
                } else if rotate {
                    // Alt-drag or the Rotate tool rotates around z.
                    let angle = delta.x as f64 * 0.01;
                    let _ = document.rotate_primitive_by(id, [0.0, 0.0, 1.0], angle);
                    trace_input(format!(
                        "rotate id={} angle_rad={angle:.5}",
                        id.get()
                    ));
                } else {
                    let _ = document.move_primitive_by(
                        id,
                        [world[0] as f64, world[1] as f64, world[2] as f64],
                    );
                    trace_input(format!(
                        "move id={} dx={:.5e} dy={:.5e} dz={:.5e}",
                        id.get(),
                        world[0],
                        world[1],
                        world[2]
                    ));
                }
            }
        }
    }
    let _ = aspect;
    if response.clicked() {
        if let (Some(position), Some(document)) =
            (response.interact_pointer_pos(), document.as_ref())
        {
            if let Some(id) =
                pick_primitive(renderer.camera(), document.structure(), rect, position)
            {
                trace_input(format!("pick id={} at=({:.0},{:.0})", id.get(), position.x, position.y));
                let mut selection = Selection::new();
                selection.primitives.insert(id);
                gui_state.selection = selection.clone();
                tree_widget.set_selection(selection);
            }
        }
    }
    // Hover read-out: name the primitive under the cursor, like
    // the native viewport tooltip, so a click is predictable.
    if let (Some(position), Some(document)) =
        (ui.ctx().pointer_latest_pos(), document.as_ref())
    {
        if rect.contains(position) && !response.dragged() {
            if let Some(id) =
                pick_primitive(renderer.camera(), document.structure(), rect, position)
            {
                if let Some(primitive) = document.structure().primitive(id) {
                    let property = document
                        .structure()
                        .property_for_primitive(id)
                        .map(|property| property.name().to_string())
                        .unwrap_or_default();
                    let label = if property.is_empty() {
                        format!("{:?} - ID: {}", primitive.primitive_type(), id.get())
                    } else {
                        format!(
                            "{:?} - ID: {}   <   {}",
                            primitive.primitive_type(),
                            id.get(),
                            property
                        )
                    };
                    let galley = ui.painter().layout_no_wrap(
                        label,
                        egui::FontId::proportional(12.0),
                        palette.text,
                    );
                    let anchor = egui::pos2(
                        (position.x + 16.0).min(rect.right() - galley.size().x - 16.0),
                        (position.y + 18.0).min(rect.bottom() - 30.0),
                    );
                    let background = egui::Rect::from_min_size(
                        anchor - egui::vec2(7.0, 4.0),
                        galley.size() + egui::vec2(14.0, 8.0),
                    );
                    ui.painter().rect(
                        background,
                        egui::CornerRadius::same(theme::RADIUS_INPUT),
                        palette.panel_raised,
                        egui::Stroke::new(1.0, palette.border_strong),
                        egui::StrokeKind::Outside,
                    );
                    ui.painter().galley(anchor, galley, palette.text);
                }
            }
        }
    }
    // Navigation hint, muted in the lower right corner.
    ui.painter().text(
        rect.right_bottom() - egui::vec2(12.0, 10.0),
        egui::Align2::RIGHT_BOTTOM,
        "Drag: move   ·   Alt+drag: rotate   ·   MMB drag: orbit   ·   Wheel: zoom",
        egui::FontId::proportional(11.5),
        palette.text_tertiary,
    );

}

    /// Build the egui UI for one frame.
    fn build_ui(
        &mut self,
        ui: &mut egui::Ui,
        extra_ui: &mut impl FnMut(&mut egui::Ui, &mut Document, &mut GuiState),
    ) {
        let EguiApp {
            document,
            gui_state,
            tree_widget,
            property_editor,
            primitive_editor,
            grid_editor,
            transform_editor,
            renderer,
            viewport_rect,
            dock_tree_width,
            dock_inspector_width,
            ..
        } = self;

        // One palette, one spacing rhythm and one set of corner radii. The
        // embedding application may change the theme this frame; it is read
        // again after `extra_ui` for the chrome and the viewport clear colour.
        theme::apply(ui.ctx(), theme::current(ui.ctx()));

        // Document-level undo/redo. The shortcut is ignored while a text
        // field has the keyboard, so egui's own text undo still applies there.
        let (undo_pressed, redo_pressed) = ui.input(|input| {
            let command = input.modifiers.command;
            let key_z = input.key_pressed(egui::Key::Z);
            let key_y = input.key_pressed(egui::Key::Y);
            (
                command && !input.modifiers.shift && key_z,
                command && ((input.modifiers.shift && key_z) || key_y),
            )
        });
        if !ui.ctx().egui_wants_keyboard_input() {
            if let Some(document) = document.as_mut() {
                if undo_pressed {
                    document.undo();
                } else if redo_pressed {
                    document.redo();
                }
            }
        }

        // The embedding application (AppCSXCAD) draws its menu above the
        // library panels.
        if let Some(document) = document.as_mut() {
            extra_ui(ui, document, gui_state);
        }

        let palette = theme::palette(ui.ctx());
        renderer.set_background(palette.viewport_rgb());

        if gui_state.show_status_bar {
            egui::Panel::bottom("status_bar")
                .frame(chrome_frame(palette))
                .show(ui, |ui| {
                    let delta = document
                        .as_ref()
                        .map(|document| document.structure().grid().delta_unit())
                        .unwrap_or(1.0);
                    let unit = drawing_unit(delta);
                    // World position of the cursor on the drawing plane (z = 0),
                    // reported in drawing units like the native status bar.
                    let cursor = ui
                        .ctx()
                        .pointer_latest_pos()
                        .filter(|position| viewport_rect.contains(*position))
                        .and_then(|position| {
                            let width = viewport_rect.width().max(1.0);
                            let height = viewport_rect.height().max(1.0);
                            let ndc = [
                                2.0 * (position.x - viewport_rect.left()) / width - 1.0,
                                1.0 - 2.0 * (position.y - viewport_rect.top()) / height,
                            ];
                            let (origin, direction) = renderer.camera().ray(ndc, width / height);
                            if direction[2].abs() < 1.0e-9 {
                                return None;
                            }
                            let distance = -origin[2] / direction[2];
                            if distance <= 0.0 {
                                return None;
                            }
                            Some([
                                (origin[0] + direction[0] * distance) as f64 / delta.max(1e-30),
                                (origin[1] + direction[1] * distance) as f64 / delta.max(1e-30),
                            ])
                        });
                    ui.horizontal(|ui| {
                        let coordinates = match cursor {
                            Some([x, y]) => format!("X {x:>9.3}   Y {y:>9.3}   {unit}"),
                            None => format!("X      —     Y      —     {unit}"),
                        };
                        ui.label(
                            egui::RichText::new(coordinates)
                                .monospace()
                                .color(palette.text_secondary),
                        )
                        .on_hover_text(format!("1 drawing unit = {delta:.6e} m"));
                        ui.separator();
                        ui.label(
                            egui::RichText::new(gui_state.transform_mode.label())
                                .color(palette.text_tertiary),
                        )
                        .on_hover_text("Active viewport tool");
                        ui.separator();
                        ui.label(
                            egui::RichText::new(selection_summary(document.as_ref(), gui_state))
                                .color(palette.text_secondary),
                        );
                        ui.with_layout(
                            egui::Layout::right_to_left(egui::Align::Center),
                            |ui| {
                                let projection = if renderer.camera().orthographic {
                                    "Parallel"
                                } else {
                                    "Perspective"
                                };
                                if let Some(document) = document.as_ref() {
                                    if document.is_modified() {
                                        ui.label(
                                            egui::RichText::new("● Unsaved")
                                                .color(palette.warning),
                                        )
                                        .on_hover_text("The project has unsaved changes");
                                    }
                                    let structure = document.structure();
                                    let primitives = structure
                                        .all_primitives(
                                            false,
                                            csxcad_core::model::PropertyType::Any,
                                        )
                                        .len();
                                    ui.label(
                                        egui::RichText::new(format!(
                                            "{projection}  ·  {} props  ·  {} prims",
                                            structure.property_count(),
                                            primitives
                                        ))
                                        .color(palette.text_tertiary),
                                    );
                                } else {
                                    ui.label(
                                        egui::RichText::new(format!("{projection}  ·  no structure"))
                                            .color(palette.text_tertiary),
                                    );
                                }
                            },
                        );
                    });
                });
        }

        #[cfg(feature = "docking")]
        {
            use crate::docking::DockPane;
            let mut layout = std::mem::take(&mut self.dock_layout);
            layout.set_visible(DockPane::Structure, gui_state.show_structure_tree);
            layout.set_visible(DockPane::Inspector, gui_state.show_inspector);
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(ui, |ui| {
                    layout.ui(ui, &mut |pane, ui| match pane {
                        DockPane::Structure => Self::structure_pane_ui(
                            ui,
                            document,
                            gui_state,
                            tree_widget,
                            grid_editor,
                            &palette,
                        ),
                        DockPane::Inspector => Self::inspector_pane_ui(
                            ui,
                            document,
                            gui_state,
                            property_editor,
                            primitive_editor,
                            transform_editor,
                            &palette,
                        ),
                        DockPane::Viewport => Self::viewport_pane_ui(
                            ui,
                            document,
                            gui_state,
                            tree_widget,
                            renderer,
                            viewport_rect,
                            &palette,
                        ),
                    });
                });
            self.dock_layout = layout;
        }

        #[cfg(not(feature = "docking"))]
        {
        if gui_state.show_structure_tree {
            let response = egui::Panel::left("structure_tree")
                .default_size(*dock_tree_width)
                .min_size(240.0)
                .max_size(380.0)
                .frame(dock_frame(palette))
                .show(ui, |ui| {
                    Self::structure_pane_ui(
                        ui,
                        document,
                        gui_state,
                        tree_widget,
                        grid_editor,
                        palette,
                    );
                });
            *dock_tree_width = response.response.rect.width();
        }

        if gui_state.show_inspector {
            let response = egui::Panel::right("editors")
                .default_size(*dock_inspector_width)
                .min_size(280.0)
                .frame(dock_frame(palette))
                .show(ui, |ui| {
                    Self::inspector_pane_ui(
                        ui,
                        document,
                        gui_state,
                        property_editor,
                        primitive_editor,
                        transform_editor,
                        palette,
                    );
                });
            *dock_inspector_width = response.response.rect.width();
        }
        // The 3D structure is drawn by the wgpu pass underneath egui, so the
        // central panel must not paint an opaque background over it.
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| {
                Self::viewport_pane_ui(
                    ui,
                    document,
                    gui_state,
                    tree_widget,
                    renderer,
                    viewport_rect,
                    palette,
                );
            });
        }

    }

    /// Load a structure into the application.
    pub fn load_structure(&mut self, structure: Structure) {
        self.load_document(Document::from_structure(structure));
    }

    /// Load a document (structure + editing state) into the application.
    pub fn load_document(&mut self, document: Document) {
        self.tree_widget.expand_all(document.structure());
        self.document = Some(document);
        self.gui_state.selection.clear();
        self.gui_state.modified = false;
        self.tree_widget.set_selection(Selection::new());
        // force a geometry rebuild on the next frame
        self.last_geometry_revision = 0;
    }

    /// Get the GUI state (view options, selection).
    pub fn gui_state(&self) -> &GuiState {
        &self.gui_state
    }

    /// Get the GUI state mutably.
    pub fn gui_state_mut(&mut self) -> &mut GuiState {
        &mut self.gui_state
    }

    /// Get the current document (if any).
    pub fn document(&self) -> Option<&Document> {
        self.document.as_ref()
    }

    /// Get the current document mutably (if any).
    pub fn document_mut(&mut self) -> Option<&mut Document> {
        self.document.as_mut()
    }

    /// Get the current structure (if any).
    pub fn structure(&self) -> Option<&Structure> {
        self.document.as_ref().map(Document::structure)
    }

    /// Take the current structure out of the app (leaving None in its place).
    pub fn take_structure(&mut self) -> Option<Structure> {
        self.document.take().map(Document::into_structure)
    }

    /// Clear the current structure and refresh all widgets.
    pub fn clear_structure(&mut self) {
        self.document = None;
        self.last_geometry_revision = 0;
        self.gui_state.selection.clear();
        self.gui_state.modified = false;
        self.tree_widget.set_selection(Selection::new());
    }

    /// Capture the 3D view into a PNG file (native `ExportView2Image`).
    pub fn export_view_png(&self, path: &str) -> Result<(), GuiError> {
        self.renderer.export_image(path)
    }

    /// Set the window title.
    pub fn set_title(&self, title: &str) {
        if let Some(window) = &self.window {
            window.set_title(title);
        }
    }

    /// Current inner size of the window.
    pub fn window_size(&self) -> Option<(u32, u32)> {
        self.window.as_ref().map(|window| {
            let size = window.inner_size();
            (size.width, size.height)
        })
    }

    /// Request a redraw of the winit window.
    pub fn request_redraw(&self) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    /// Get the current selection.
    pub fn selection(&self) -> Selection {
        self.tree_widget.selection()
    }

    /// Set the current selection.
    pub fn set_selection(&mut self, selection: Selection) {
        self.gui_state.selection = selection.clone();
        self.tree_widget.set_selection(selection);
    }

    /// Select one property (used by the toolbar after a create).
    pub fn select_property(&mut self, id: PropertyId) {
        let mut selection = Selection::new();
        selection.properties.insert(id);
        self.set_selection(selection);
    }

    /// Select one primitive (used by the toolbar after a create).
    pub fn select_primitive(&mut self, id: PrimitiveId) {
        let mut selection = Selection::new();
        selection.primitives.insert(id);
        self.set_selection(selection);
    }

    /// Expand every property in the tree.
    pub fn expand_all(&mut self) {
        if let Some(document) = self.document.as_ref() {
            self.tree_widget.expand_all(document.structure());
        }
    }

    /// Collapse every property in the tree.
    pub fn collapse_all(&mut self) {
        self.tree_widget.collapse_all();
    }

    /// Native axis view presets: 0 = XY, 1 = XZ, 2 = YZ, 3 = free 3D.
    pub fn set_view_dir(&mut self, dir: i32) {
        self.gui_state.view_dir = dir;
        // The 2D presets are orthographic; the free 3D view is perspective.
        self.gui_state.parallel_projection = dir != 3;
        self.renderer.set_view_dir(dir);
    }

    /// Frame all geometry (Fit All).
    pub fn fit_all(&mut self) {
        self.renderer.fit_all();
        self.request_redraw();
    }

    /// Frame the selected primitives (Fit Selection). Falls back to Fit All
    /// when nothing is selected or the selection has no usable bounds.
    pub fn fit_selection(&mut self) {
        let Some(document) = self.document.as_ref() else {
            return;
        };
        let structure = document.structure();
        let parameters = structure.parameters();
        let mut bounds: Option<[f64; 6]> = None;
        for id in self.gui_state.selection.primitives.iter().copied() {
            let Some(primitive) = structure.primitive(id) else {
                continue;
            };
            let Ok(bbox) = primitive.bounding_box(Some(parameters), false) else {
                continue;
            };
            if !bbox.iter().all(|value| value.is_finite()) {
                continue;
            }
            bounds = Some(match bounds {
                None => bbox,
                Some(current) => [
                    current[0].min(bbox[0]),
                    current[1].max(bbox[1]),
                    current[2].min(bbox[2]),
                    current[3].max(bbox[3]),
                    current[4].min(bbox[4]),
                    current[5].max(bbox[5]),
                ],
            });
        }
        match bounds {
            Some(bounds) => self.renderer.camera_mut().fit(bounds),
            None => self.renderer.fit_all(),
        }
        self.request_redraw();
    }

    /// Apply persisted dock widths before the first frame.
    pub fn set_dock_widths(&mut self, tree: f32, inspector: f32) {
        self.dock_tree_width = tree.clamp(240.0, 380.0);
        self.dock_inspector_width = inspector.clamp(280.0, 640.0);
    }

    /// The dock widths last used by the layout.
    pub fn dock_widths(&self) -> (f32, f32) {
        (self.dock_tree_width, self.dock_inspector_width)
    }

    /// Serialized drag-docking layout for the settings file.
    #[cfg(feature = "docking")]
    pub fn dock_layout_json(&self) -> Option<String> {
        Some(self.dock_layout.to_json())
    }

    /// Restore a persisted drag-docking layout; corrupt input keeps the
    /// default so a panel cannot disappear silently.
    #[cfg(feature = "docking")]
    pub fn set_dock_layout_json(&mut self, json: &str) {
        if let Some(layout) = crate::docking::DockLayout::from_json(json) {
            self.dock_layout = layout;
        }
    }

    /// Apply a theme (used at startup for the persisted preference).
    pub fn set_theme(&mut self, mode: theme::ThemeMode) {
        theme::set(&self.egui_ctx, mode);
    }

    /// The theme currently in use.
    pub fn theme(&self) -> theme::ThemeMode {
        theme::current(&self.egui_ctx)
    }

    /// Switch the viewport between parallel and perspective projection.
    pub fn set_parallel_projection(&mut self, enabled: bool) {
        self.gui_state.parallel_projection = enabled;
        self.renderer.set_parallel_projection(enabled);
    }

    /// Select the active viewport drag tool.
    pub fn set_transform_mode(&mut self, mode: TransformMode) {
        self.gui_state.transform_mode = mode;
    }

    pub fn get_transform_mode(&self) -> TransformMode {
        self.gui_state.transform_mode
    }
}

impl Default for EguiApp {
    fn default() -> Self {
        Self::new()
    }
}

/// Minimal event loop runner for standalone testing.
pub fn run_event_loop(app: EguiApp, event_loop: EventLoop<()>) {
    struct Handler {
        app: EguiApp,
    }

    impl winit::application::ApplicationHandler<()> for Handler {
        fn resumed(&mut self, _event_loop: &ActiveEventLoop) {}

        fn window_event(
            &mut self,
            _event_loop: &ActiveEventLoop,
            _window_id: winit::window::WindowId,
            event: WindowEvent,
        ) {
            let event = Event::WindowEvent {
                window_id: _window_id,
                event,
            };
            let _consumed = self.app.handle_event(&event);

            match &event {
                Event::WindowEvent {
                    event: WindowEvent::Resized(size),
                    ..
                } => {
                    self.app.resize(size.width, size.height);
                }
                Event::WindowEvent {
                    event: WindowEvent::RedrawRequested,
                    ..
                } => {
                    if let Err(e) = self.app.render() {
                        eprintln!("render error: {e}");
                        _event_loop.exit();
                    }
                }
                _ => {}
            }
        }

        fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
            self.app.request_redraw();
        }
    }

    let mut handler = Handler { app };
    event_loop.run_app(&mut handler).unwrap();
}

/// Append a viewport navigation/picking line to the file named by the
/// `QCSXCAD_INPUT_TRACE` environment variable. A no-op when it is unset, so
/// the GUI smoke automation can prove what the mouse did.
pub fn trace_input(message: impl AsRef<str>) {
    let Ok(path) = std::env::var("QCSXCAD_INPUT_TRACE") else {
        return;
    };
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        use std::io::Write as _;
        let _ = writeln!(file, "{}", message.as_ref());
    }
}

/// Pick the nearest primitive whose projected bounding box contains `pointer`.
fn pick_primitive(
    camera: &Camera,
    structure: &Structure,
    rect: egui::Rect,
    pointer: egui::Pos2,
) -> Option<PrimitiveId> {
    let aspect = rect.width() / rect.height().max(1.0);
    let parameters = structure.parameters();
    let mut best: Option<(f32, PrimitiveId)> = None;
    for primitive in structure.all_primitives(false, PropertyType::Any) {
        let Some(id) = primitive.id() else { continue };
        let Ok(bounds) = primitive.bounding_box(Some(parameters), false) else {
            continue;
        };
        let corners = [
            [bounds[0], bounds[2], bounds[4]],
            [bounds[0], bounds[2], bounds[5]],
            [bounds[0], bounds[3], bounds[4]],
            [bounds[0], bounds[3], bounds[5]],
            [bounds[1], bounds[2], bounds[4]],
            [bounds[1], bounds[2], bounds[5]],
            [bounds[1], bounds[3], bounds[4]],
            [bounds[1], bounds[3], bounds[5]],
        ];
        let mut min = egui::pos2(f32::INFINITY, f32::INFINITY);
        let mut max = egui::pos2(f32::NEG_INFINITY, f32::NEG_INFINITY);
        let mut depth = f32::INFINITY;
        let mut visible = true;
        for corner in corners {
            let Some(ndc) = camera.project([corner[0] as f32, corner[1] as f32, corner[2] as f32], aspect) else {
                visible = false;
                break;
            };
            let point = egui::pos2(
                rect.left() + (ndc[0] + 1.0) * 0.5 * rect.width(),
                rect.top() + (1.0 - ndc[1]) * 0.5 * rect.height(),
            );
            min.x = min.x.min(point.x);
            min.y = min.y.min(point.y);
            max.x = max.x.max(point.x);
            max.y = max.y.max(point.y);
            depth = depth.min(ndc[2]);
        }
        if visible
            && pointer.x >= min.x
            && pointer.x <= max.x
            && pointer.y >= min.y
            && pointer.y <= max.y
            && best.map_or(true, |(best_depth, _)| depth < best_depth)
        {
            best = Some((depth, id));
        }
    }
    best.map(|(_, id)| id)
}

/// Native axis gizmo in the lower-left corner of the viewport.
/// Frame used by the menu, command strip and status bar.
pub fn chrome_frame(palette: &theme::Palette) -> egui::Frame {
    egui::Frame::new()
        .fill(palette.chrome_bg)
        .inner_margin(egui::Margin::symmetric(theme::PANEL_PADDING, 3))
        .stroke(egui::Stroke::new(1.0, palette.border))
}

/// Frame used by the left and right docks.
pub fn dock_frame(palette: &theme::Palette) -> egui::Frame {
    egui::Frame::new()
        .fill(palette.panel_bg)
        .inner_margin(egui::Margin::symmetric(theme::PANEL_PADDING, 8))
}

/// Label for one drawing unit, where `delta` is its length in metres.
fn drawing_unit(delta: f64) -> &'static str {
    const UNITS: [(f64, &str); 9] = [
        (1.0e-12, "pm"),
        (1.0e-9, "nm"),
        (1.0e-6, "µm"),
        (1.0e-3, "mm"),
        (1.0e-2, "cm"),
        (1.0, "m"),
        (1.0e3, "km"),
        (1.0e6, "Mm"),
        (1.0e9, "Gm"),
    ];
    for (scale, label) in UNITS {
        if delta > 0.0 && (delta / scale - 1.0).abs() <= 1.0e-9 {
            return label;
        }
    }
    "units"
}

/// One-line selection summary for the status bar.
fn selection_summary(document: Option<&Document>, gui_state: &GuiState) -> String {
    let Some(document) = document else {
        return "No structure loaded".to_owned();
    };
    let structure = document.structure();
    let primitives = gui_state.selection.primitives.len();
    let properties = gui_state.selection.properties.len();
    let total = primitives + properties;
    if total == 0 {
        return "No selection".to_owned();
    }
    let label = gui_state
        .selection
        .primitives
        .iter()
        .copied()
        .find_map(|id| {
            let primitive = structure.primitive(id)?;
            let property = structure
                .property_for_primitive(id)
                .map(|property| property.name().to_string())
                .unwrap_or_default();
            Some(if property.is_empty() {
                format!("{:?} {}", primitive.primitive_type(), id.get())
            } else {
                format!("{:?} · {}", primitive.primitive_type(), property)
            })
        })
        .or_else(|| {
            gui_state
                .selection
                .properties
                .iter()
                .copied()
                .find_map(|id| {
                    structure
                        .property(id)
                        .map(|property| format!("{:?} · {}", property.property_type(), property.name()))
                })
        })
        .unwrap_or_else(|| "Selection".to_owned());
    if total > 1 {
        format!("{label}  (+{} more)", total - 1)
    } else {
        label
    }
}

fn draw_axis_gizmo(
    ui: &egui::Ui,
    rect: egui::Rect,
    camera: &Camera,
    palette: &theme::Palette,
) {
    let aspect = rect.width() / rect.height().max(1.0);
    let Some(origin) = camera.project(camera.center, aspect) else {
        return;
    };
    let base = egui::pos2(rect.left() + 36.0, rect.bottom() - 36.0);
    let axes = [
        ([1.0, 0.0, 0.0], palette.axis_x, "X"),
        ([0.0, 1.0, 0.0], palette.axis_y, "Y"),
        ([0.0, 0.0, 1.0], palette.axis_z, "Z"),
    ];
    for (direction, color, label) in axes {
        let point = [
            camera.center[0] + direction[0],
            camera.center[1] + direction[1],
            camera.center[2] + direction[2],
        ];
        let Some(projected) = camera.project(point, aspect) else {
            continue;
        };
        let delta = egui::vec2(projected[0] - origin[0], -(projected[1] - origin[1]));
        if delta.length() < 1.0e-6 {
            continue;
        }
        let end = base + delta.normalized() * 28.0;
        ui.painter()
            .line_segment([base, end], egui::Stroke::new(1.6, color));
        ui.painter().text(
            end,
            egui::Align2::CENTER_CENTER,
            label,
            egui::FontId::proportional(10.0),
            color,
        );
    }
}

#[cfg(test)]
mod pick_tests {
    use super::*;
    use csxcad_core::model::{Primitive, Property};

    #[test]
    fn clicking_the_viewport_selects_the_primitive() {
        let mut structure = Structure::new();
        let property = structure.add_property(Property::new("m", PropertyType::Material));
        structure
            .add_primitive(
                property,
                Primitive::box_primitive([-5.0e-3; 3], [5.0e-3; 3]),
            )
            .expect("box");
        let id = structure
            .all_primitives(false, PropertyType::Any)
            .first()
            .and_then(|primitive| primitive.id())
            .expect("box id");

        let mut camera = Camera::default();
        camera.fit([-0.01, 0.01, -0.01, 0.01, -0.01, 0.01]);
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 600.0));
        assert_eq!(
            pick_primitive(&camera, &structure, rect, rect.center()),
            Some(id)
        );
        // A click far outside the projected box selects nothing.
        assert_eq!(
            pick_primitive(&camera, &structure, rect, egui::pos2(5.0, 5.0)),
            None
        );
    }
}

#[cfg(test)]
mod dock_tests {
    use super::*;

    #[test]
    fn dock_widths_have_defaults_and_are_clamped() {
        let mut app = EguiApp::new();
        assert_eq!(app.dock_widths(), (320.0, 360.0));
        app.set_dock_widths(100.0, 900.0);
        assert_eq!(app.dock_widths(), (240.0, 640.0));
        app.set_dock_widths(300.0, 400.0);
        assert_eq!(app.dock_widths(), (300.0, 400.0));
    }
}

#[cfg(test)]
mod status_tests {
    use super::*;

    #[test]
    fn drawing_units_map_to_si_labels() {
        assert_eq!(drawing_unit(1.0e-3), "mm");
        assert_eq!(drawing_unit(1.0e-6), "µm");
        assert_eq!(drawing_unit(1.0e-9), "nm");
        assert_eq!(drawing_unit(1.0), "m");
        // A unit that is not an SI prefix is reported honestly, not guessed.
        assert_eq!(drawing_unit(2.5e-3), "units");
        assert_eq!(drawing_unit(0.0), "units");
    }

    #[test]
    fn an_empty_selection_is_reported() {
        let document = Document::from_structure(Structure::new());
        assert_eq!(
            selection_summary(Some(&document), &GuiState::default()),
            "No selection"
        );
        assert_eq!(selection_summary(None, &GuiState::default()), "No structure loaded");
    }

    #[test]
    fn theme_switching_is_visible_to_later_frames() {
        let ctx = egui::Context::default();
        assert_eq!(theme::current(&ctx), theme::ThemeMode::Dark);
        theme::set(&ctx, theme::ThemeMode::Light);
        assert_eq!(theme::current(&ctx), theme::ThemeMode::Light);
        let palette = theme::palette(&ctx);
        assert!(!palette.is_dark);
        assert_eq!(palette.viewport_bg, theme::LIGHT.viewport_bg);
    }
}

#[cfg(test)]
mod navigation_tests {
    use super::*;
    use winit::dpi::PhysicalPosition;
    use winit::event::{
        ElementState, MouseButton, MouseScrollDelta, TouchPhase, WindowEvent,
    };

    /// Put the pointer over the middle of the viewport, as a frame would.
    fn pointer_in_viewport(app: &mut EguiApp) {
        app.viewport_rect =
            egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 600.0));
        app.last_pointer = Some(egui::pos2(400.0, 300.0));
    }

    #[test]
    fn wheel_zooms_and_middle_drag_orbits() {
        let mut app = EguiApp::new();
        pointer_in_viewport(&mut app);
        let distance = app.renderer.camera().distance;
        app.handle_navigation(&WindowEvent::MouseWheel {
            device_id: winit::event::DeviceId::dummy(),
            delta: MouseScrollDelta::LineDelta(0.0, 1.0),
            phase: TouchPhase::Moved,
        });
        assert!(
            app.renderer.camera().distance < distance,
            "wheel did not zoom"
        );

        let yaw = app.renderer.camera().yaw;
        app.handle_navigation(&WindowEvent::MouseInput {
            device_id: winit::event::DeviceId::dummy(),
            state: ElementState::Pressed,
            button: MouseButton::Middle,
        });
        app.handle_navigation(&WindowEvent::CursorMoved {
            device_id: winit::event::DeviceId::dummy(),
            position: PhysicalPosition::new(10.0, 10.0),
        });
        app.handle_navigation(&WindowEvent::CursorMoved {
            device_id: winit::event::DeviceId::dummy(),
            position: PhysicalPosition::new(40.0, 10.0),
        });
        assert!(
            (app.renderer.camera().yaw - yaw).abs() > 1.0e-3,
            "middle drag did not orbit"
        );
        app.handle_navigation(&WindowEvent::MouseInput {
            device_id: winit::event::DeviceId::dummy(),
            state: ElementState::Released,
            button: MouseButton::Middle,
        });
    }

    #[test]
    fn navigation_outside_the_viewport_is_ignored() {
        let mut app = EguiApp::new();
        app.viewport_rect =
            egui::Rect::from_min_size(egui::pos2(100.0, 100.0), egui::vec2(800.0, 600.0));
        // Over the left dock, not the viewport.
        app.last_pointer = Some(egui::pos2(40.0, 300.0));
        let distance = app.renderer.camera().distance;
        let yaw = app.renderer.camera().yaw;
        app.handle_navigation(&WindowEvent::MouseWheel {
            device_id: winit::event::DeviceId::dummy(),
            delta: MouseScrollDelta::LineDelta(0.0, 1.0),
            phase: TouchPhase::Moved,
        });
        assert_eq!(
            app.renderer.camera().distance,
            distance,
            "a wheel event over the tree must not zoom the 3D view"
        );
        app.handle_navigation(&WindowEvent::MouseInput {
            device_id: winit::event::DeviceId::dummy(),
            state: ElementState::Pressed,
            button: MouseButton::Middle,
        });
        app.handle_navigation(&WindowEvent::CursorMoved {
            device_id: winit::event::DeviceId::dummy(),
            position: PhysicalPosition::new(300.0, 300.0),
        });
        assert!(
            (app.renderer.camera().yaw - yaw).abs() < 1.0e-6,
            "a middle drag over a dock must not orbit"
        );
    }

    #[test]
    fn cursor_without_the_middle_button_does_not_orbit() {
        let mut app = EguiApp::new();
        let yaw = app.renderer.camera().yaw;
        app.handle_navigation(&WindowEvent::CursorMoved {
            device_id: winit::event::DeviceId::dummy(),
            position: PhysicalPosition::new(10.0, 10.0),
        });
        app.handle_navigation(&WindowEvent::CursorMoved {
            device_id: winit::event::DeviceId::dummy(),
            position: PhysicalPosition::new(40.0, 10.0),
        });
        assert!((app.renderer.camera().yaw - yaw).abs() < 1.0e-9);
    }
}
