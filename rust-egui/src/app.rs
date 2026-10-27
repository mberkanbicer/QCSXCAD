//! Main application struct tying together egui widgets and wgpu renderer.

use csxcad_core::model::Structure;
use qcsxcad_core::document::Document;
use qcsxcad_core::model::{GuiError, GuiState, Selection};
use qcsxcad_core::rendering::StructureRenderer;
use qcsxcad_core::widgets::TreeWidget;
use winit::event::{Event, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::Window;

use crate::renderer::WgpuRenderer;
use crate::widgets::{
    EguiGridEditor, EguiPrimitiveEditor, EguiPropertyEditor, EguiTreeWidget,
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
    property_editor: EguiPropertyEditor,
    primitive_editor: EguiPrimitiveEditor,
    document: Option<Document>,
    /// Revision of the structure the current GPU geometry was built from.
    last_geometry_revision: u64,
    gui_state: GuiState,
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
            property_editor: EguiPropertyEditor::default(),
            primitive_editor: EguiPrimitiveEditor::default(),
            document: None,
            last_geometry_revision: 0,
            gui_state: GuiState::new(),
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
            if revision != self.last_geometry_revision {
                let structure = document.structure();
                self.renderer.render(structure, &self.gui_state)?;
                self.last_geometry_revision = revision;
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
                        r: 0.1,
                        g: 0.1,
                        b: 0.1,
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
            ..
        } = self;

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

        egui::Panel::top("status_bar").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.strong("QCSXCAD");
                match document.as_ref() {
                    Some(document) => {
                        ui.label(format!(
                            "{} properties / {} primitives",
                            document.structure().property_count(),
                            document
                                .structure()
                                .all_primitives(false, csxcad_core::model::PropertyType::Any)
                                .len()
                        ));
                        if document.is_modified() {
                            ui.label("· modified");
                        }
                    }
                    None => {
                        ui.label("· no structure loaded");
                    }
                }
            });
        });

        egui::Panel::left("structure_tree")
            .default_size(280.0)
            .show(ui, |ui| {
                if let Some(document) = document.as_mut() {
                    tree_widget.show(ui, document, gui_state);
                }
            });

        // The 3D structure is drawn by the wgpu pass underneath egui, so the
        // central panel must not paint an opaque background over it.
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |_ui| {});

        egui::Panel::right("editors")
            .default_size(340.0)
            .show(ui, |ui| {
                if let Some(document) = document.as_mut() {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        let _ = property_editor.show(ui, document, gui_state);
                        ui.separator();
                        let _ = primitive_editor.show(ui, document, gui_state);
                        ui.separator();
                        let _ = grid_editor.show(ui, document);
                    });
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
        self.tree_widget.set_selection(selection);
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