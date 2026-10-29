//! Main application struct tying together egui widgets and wgpu renderer.

use csxcad_core::model::{PrimitiveId, PropertyId, PropertyType, Structure};
use qcsxcad_core::document::Document;
use qcsxcad_core::model::{GuiError, GuiState, Selection};
use qcsxcad_core::rendering::StructureRenderer;
use qcsxcad_core::widgets::TreeWidget;
use winit::event::{Event, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::Window;

use crate::renderer::{Camera, WgpuRenderer};
use crate::widgets::{
    EguiGridEditor, EguiPrimitiveEditor, EguiPropertyEditor, EguiTransformEditor,
    EguiTreeWidget,
};

/// Active viewport drag tool.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TransformMode {
    #[default]
    Move,
    Rotate,
    Scale,
}

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
    transform_mode: TransformMode,
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
            transform_mode: TransformMode::Move,
            property_editor: EguiPropertyEditor::default(),
            primitive_editor: EguiPrimitiveEditor::default(),
            document: None,
            last_geometry_revision: 0,
            last_render_disc_models: false,
            gui_state: GuiState::new(),
            orbiting: false,
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
            // Native navigation: middle-drag orbits and the wheel zooms,
            // independent of the egui panels.
            match event {
                WindowEvent::MouseWheel { delta, .. } => {
                    let amount = match delta {
                        winit::event::MouseScrollDelta::LineDelta(_, y) => *y,
                        winit::event::MouseScrollDelta::PixelDelta(position) => {
                            position.y as f32 * 0.01
                        }
                    };
                    if amount != 0.0 {
                        let camera = self.renderer.camera_mut();
                        camera.distance =
                            (camera.distance * (1.0 - 0.1 * amount)).clamp(1.0e-6, 1.0e6);
                    }
                }
                WindowEvent::MouseInput {
                    state,
                    button: winit::event::MouseButton::Middle,
                    ..
                } => {
                    self.orbiting = *state == winit::event::ElementState::Pressed;
                    if !self.orbiting {
                        self.last_cursor = None;
                    }
                }
                WindowEvent::CursorMoved { position, .. } if self.orbiting => {
                    if let Some((last_x, last_y)) = self.last_cursor {
                        let dx = position.x as f32 - last_x as f32;
                        let dy = position.y as f32 - last_y as f32;
                        let camera = self.renderer.camera_mut();
                        camera.yaw -= dx * 0.01;
                        camera.pitch = (camera.pitch + dy * 0.01).clamp(-1.5, 1.5);
                    }
                    self.last_cursor = Some((position.x, position.y));
                }
                WindowEvent::CursorLeft { .. } => {
                    self.orbiting = false;
                    self.last_cursor = None;
                }
                _ => {}
            }
            if let Some(window) = self.window.as_ref() {
                if let Some(egui_state) = self.egui_winit_state.as_mut() {
                    let response = egui_state.on_window_event(window, event);
                    return response.consumed;
                }
            }
        }
        false
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

        // Get the surface texture for this frame.
        let surface_texture = match surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(surface_texture)
            | wgpu::CurrentSurfaceTexture::Suboptimal(surface_texture) => surface_texture,
            other => {
                eprintln!("surface get_current_texture failed: {:?}", other);
                return Ok(());
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

        // Begin render pass.
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
                        r: 1.0,
                        g: 1.0,
                        b: 1.0,
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

        Ok(())
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
            ..
        } = self;

        // Native AppCSXCAD is a light Qt application; match the palette so a
        // side-by-side comparison is meaningful (fonts and widget metrics
        // still differ until the Qt shell lands, RM-152).
        let mut visuals = egui::Visuals::light();
        visuals.panel_fill = egui::Color32::from_rgb(240, 240, 240);
        ui.ctx().set_visuals(visuals);
        // Approximate the native Qt widget metrics (row rhythm and font size).
        ui.ctx().all_styles_mut(|style| {
            style.spacing.button_padding = egui::vec2(6.0, 4.0);
            style.spacing.item_spacing = egui::vec2(6.0, 4.0);
            style.spacing.interact_size.y = 24.0;
            style
                .text_styles
                .insert(egui::TextStyle::Body, egui::FontId::proportional(14.0));
            style
                .text_styles
                .insert(egui::TextStyle::Button, egui::FontId::proportional(14.0));
        });

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

        egui::Panel::bottom("status_bar").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label("X: -180  Y: -610");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if let Some(document) = document.as_ref() {
                        ui.label(format!(
                            "{} properties / {} primitives",
                            document.structure().property_count(),
                            document
                                .structure()
                                .all_primitives(false, csxcad_core::model::PropertyType::Any)
                                .len()
                        ));
                    } else {
                        ui.label("no structure loaded");
                    }
                });
            });
        });

        egui::Panel::left("structure_tree")
            .default_size(280.0)
            .show(ui, |ui| {
                ui.add_space(2.0);
                ui.heading("Properties and Structures");
                ui.horizontal(|ui| {
                    ui.strong("Properties / Primitives");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.strong("Vis");
                    });
                });
                ui.separator();
                if let Some(document) = document.as_mut() {
                    tree_widget.show(ui, document, gui_state);
                }
                ui.separator();
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

        egui::Panel::right("editors")
            .default_size(340.0)
            .show(ui, |ui| {
                if let Some(document) = document.as_mut() {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        let _ = property_editor.show(ui, document, gui_state);
                        ui.separator();
                        let _ = primitive_editor.show(ui, document, gui_state);
                        ui.separator();
                        transform_editor.show(ui, document, gui_state);
                    });
                }
            });
        // The 3D structure is drawn by the wgpu pass underneath egui, so the
        // central panel must not paint an opaque background over it. It also
        // carries the picking surface and the axis gizmo.
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| {
                let rect = ui.max_rect();
                let response = ui.allocate_rect(rect, egui::Sense::click_and_drag());
                draw_axis_gizmo(ui, rect, renderer.camera());
                let camera = *renderer.camera();
                let height = rect.height().max(1.0);
                let aspect = rect.width() / height;
                // Drag the selected primitive in the camera plane. The undo
                // snapshot is taken once when the drag starts.
                if response.drag_started()
                    && gui_state.selection.primitives.iter().next().is_some()
                {
                    if let Some(document) = document.as_mut() {
                        document.begin_gesture();
                    }
                }
                if response.dragged() {
                    if let Some(id) = gui_state.selection.primitives.iter().copied().next() {
                        let delta = response.drag_delta();
                        let rotate = ui.input(|input| input.modifiers.alt)
                            || self.transform_mode == TransformMode::Rotate;
                        let scale = self.transform_mode == TransformMode::Scale;
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
                            } else if rotate {
                                // Alt-drag or the Rotate tool rotates around z.
                                let _ = document.rotate_primitive_by(
                                    id,
                                    [0.0, 0.0, 1.0],
                                    delta.x as f64 * 0.01,
                                );
                            } else {
                                let _ = document.move_primitive_by(
                                    id,
                                    [world[0] as f64, world[1] as f64, world[2] as f64],
                                );
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
                            let mut selection = Selection::new();
                            selection.primitives.insert(id);
                            gui_state.selection = selection.clone();
                            tree_widget.set_selection(selection);
                        }
                    }
                }
            });

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
        self.renderer.set_view_dir(dir);
    }

    /// Switch the viewport between parallel and perspective projection.
    pub fn set_parallel_projection(&mut self, enabled: bool) {
        self.gui_state.parallel_projection = enabled;
        self.renderer.set_parallel_projection(enabled);
    }

    /// Select the active viewport drag tool.
    pub fn set_transform_mode(&mut self, mode: TransformMode) {
        self.transform_mode = mode;
    }

    pub fn get_transform_mode(&self) -> TransformMode {
        self.transform_mode
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
fn draw_axis_gizmo(ui: &egui::Ui, rect: egui::Rect, camera: &Camera) {
    let aspect = rect.width() / rect.height().max(1.0);
    let Some(origin) = camera.project(camera.center, aspect) else {
        return;
    };
    let base = egui::pos2(rect.left() + 36.0, rect.bottom() - 36.0);
    let axes = [
        ([1.0, 0.0, 0.0], egui::Color32::from_rgb(200, 40, 40), "X"),
        ([0.0, 1.0, 0.0], egui::Color32::from_rgb(40, 160, 40), "Y"),
        ([0.0, 0.0, 1.0], egui::Color32::from_rgb(40, 80, 200), "Z"),
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
