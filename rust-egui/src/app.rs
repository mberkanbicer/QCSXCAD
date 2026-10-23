//! Main application struct tying together egui widgets and wgpu renderer.

use csxcad_core::model::Structure;
use qcsxcad_core::model::{GuiError, GuiState, Selection};
use qcsxcad_core::rendering::StructureRenderer;
use qcsxcad_core::widgets::{
    GridEditor, PrimitiveEditor, PropertyEditor, TreeWidget,
};
use winit::event::Event;
use winit::event_loop::EventLoop;
use winit::window::Window;

use crate::renderer::WgpuRenderer;
use crate::widgets::{
    EguiGridEditor, EguiPrimitiveEditor, EguiPropertyEditor, EguiTreeWidget,
};

/// The main QCSXCAD application: egui UI + wgpu 3D viewport.
pub struct EguiApp {
    window: Option<Window>,
    renderer: WgpuRenderer,
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
            renderer: WgpuRenderer::new(),
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

        self.renderer
            .configure_surface(&surface, &device, size.width, size.height);
        self.renderer.init(adapter, device, queue, &config);
    }

    /// Handle a winit event. Returns true if the event was consumed.
    pub fn handle_event(&mut self, _event: &Event<()>) -> bool {
        // Let egui process input events first.
        if let Some(window) = self.window.as_ref() {
            // egui-winit integration would go here
            let _ = window;
        }
        false
    }

    /// Render one frame: egui UI + wgpu 3D viewport.
    pub fn render(&mut self) -> Result<(), GuiError> {
        // Render 3D viewport.
        if let Some(structure) = self.structure.as_ref() {
            self.renderer.render(structure, &self.gui_state)?;
        }

        // Draw egui UI.
        if let Some(window) = self.window.as_ref() {
            let _ = window;
            // egui rendering would go here using egui-winit + egui-wgpu
        }

        Ok(())
    }

    /// Load a structure into the application.
    pub fn load_structure(&mut self, structure: Structure) {
        self.structure = Some(structure);
        self.tree_widget.refresh(self.structure.as_ref().unwrap(), &self.gui_state);
        self.grid_editor.refresh(self.structure.as_ref().unwrap());
        self.property_editor.refresh(self.structure.as_ref().unwrap(), &self.gui_state);
        self.primitive_editor.refresh(self.structure.as_ref().unwrap(), &self.gui_state);
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
    let _ = app;
    let _ = event_loop;
    // Standalone runner would go here; primarily intended for embedding
    // in AppCSXCAD or a custom host application.
}