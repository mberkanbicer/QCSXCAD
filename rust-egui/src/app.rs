//! Main application struct tying together egui widgets and wgpu renderer.

use csxcad_core::model::Structure;
use qcsxcad_core::model::{GuiError, GuiState, Selection};
use qcsxcad_core::rendering::StructureRenderer;
use qcsxcad_core::widgets::{
    GridEditor, PrimitiveEditor, PropertyEditor, TreeWidget,
};
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
    structure: Option<Structure>,
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
            structure: None,
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
            egui_wgpu::RendererOptions::default(),
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
            self.build_ui(ui);
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

        // Build 3D geometry if we have a structure.
        if let Some(structure) = self.structure.as_ref() {
            self.renderer.render(structure, &self.gui_state)?;
        }

        // Begin render pass.
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("qcsxcad encoder"),
        });
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
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        let mut pass = render_pass.forget_lifetime();

        // Draw 3D viewport.
        self.renderer.draw_3d(&mut pass);

        // Render egui UI.
        let screen_descriptor = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [config.width, config.height],
            pixels_per_point: full_output.pixels_per_point,
        };
        egui_wgpu_renderer.render(&mut pass, &paint_jobs, &screen_descriptor);

        drop(pass);
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
    fn build_ui(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("menu_bar").show(ui, |ui| {
            egui::menu::MenuBar::default().ui(ui, |ui| {
                if ui.menu_button("File", |_ui| {}).response.clicked() {
                    // File menu handled by embedding application
                }
                if ui.menu_button("Info", |_ui| {}).response.clicked() {
                    // Info menu handled by embedding application
                }
            });
        });

        egui::Panel::left("structure_tree")
            .default_size(250.0)
            .show(ui, |ui| {
                if let Some(structure) = self.structure.as_ref() {
                    self.tree_widget.show(ui, structure, &self.gui_state);
                }
            });

        egui::CentralPanel::default().show(ui, |ui| {
            ui.heading("3D Viewport");
            if self.structure.is_none() {
                ui.label("No structure loaded.");
            } else {
                ui.label("3D viewport rendering via wgpu.");
            }
        });

        egui::Panel::right("editors")
            .default_size(250.0)
            .show(ui, |ui| {
                if let Some(structure) = self.structure.as_ref() {
                    let _ = self.grid_editor.show(ui, structure);
                    let _ = self.property_editor.show(ui, structure, &self.gui_state);
                    let _ = self.primitive_editor.show(ui, structure, &self.gui_state);
                }
            });
    }

    /// Load a structure into the application.
    pub fn load_structure(&mut self, structure: Structure) {
        self.structure = Some(structure);
        self.tree_widget.refresh(self.structure.as_ref().unwrap(), &self.gui_state);
        self.grid_editor.refresh(self.structure.as_ref().unwrap());
        self.property_editor.refresh(self.structure.as_ref().unwrap(), &self.gui_state);
        self.primitive_editor.refresh(self.structure.as_ref().unwrap(), &self.gui_state);
    }

    /// Get the current structure (if any).
    pub fn structure(&self) -> Option<&Structure> {
        self.structure.as_ref()
    }

    /// Take the current structure out of the app (leaving None in its place).
    pub fn take_structure(&mut self) -> Option<Structure> {
        self.structure.take()
    }

    /// Clear the current structure and refresh all widgets.
    pub fn clear_structure(&mut self) {
        if let Some(structure) = self.structure.as_ref() {
            self.tree_widget.refresh(structure, &self.gui_state);
            self.grid_editor.refresh(structure);
            self.property_editor.refresh(structure, &self.gui_state);
            self.primitive_editor.refresh(structure, &self.gui_state);
        }
        self.structure = None;
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