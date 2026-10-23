//! egui implementations of the QCSXCAD widget traits.
//!
//! These are immediate-mode widgets: they maintain internal state and are
//! drawn each frame via the `show` methods. The trait methods (`refresh`,
//! `selection`, `set_selection`, etc.) update that state.

use csxcad_core::model::{PrimitiveId, PropertyId, Structure};
use qcsxcad_core::model::{GuiError, GuiState, Selection};
use qcsxcad_core::widgets::{GridEditor, PrimitiveEditor, PropertyEditor, TreeWidget};

/// egui-based structure/property tree widget.
pub struct EguiTreeWidget {
    /// Expanded property IDs in the tree.
    expanded: std::collections::HashSet<PropertyId>,
    /// Currently selected items.
    selection: Selection,
}

impl Default for EguiTreeWidget {
    fn default() -> Self {
        Self {
            expanded: std::collections::HashSet::new(),
            selection: Selection::new(),
        }
    }
}

impl EguiTreeWidget {
    /// Draw the tree widget in an egui `Ui`. Call this each frame.
    pub fn show(&mut self, ui: &mut egui::Ui, structure: &Structure, state: &GuiState) {
        ui.heading("Structure Tree");
        egui::ScrollArea::vertical().show(ui, |ui| {
            for i in 0..structure.property_count() {
                if let Some(prop) = structure.property_by_index(i) {
                    self.show_property(ui, structure, prop, state);
                }
            }
        });
    }

    fn show_property(
        &mut self,
        ui: &mut egui::Ui,
        structure: &Structure,
        prop: &csxcad_core::model::Property,
        state: &GuiState,
    ) {
        let prop_id = match prop.id() {
            Some(id) => id,
            None => return,
        };
        let is_expanded = self.expanded.contains(&prop_id);
        let is_selected = self.selection.properties.contains(&prop_id);

        let header = egui::CollapsingHeader::new(format!("Property {:?}", prop_id))
            .open(Some(is_expanded));

        let header_response = header.show(ui, |ui| {
            if is_expanded {
                for prim in prop.primitives() {
                    if let Some(prim_id) = prim.id() {
                        self.show_primitive(ui, structure, prim_id, state);
                    }
                }
            }
        });

        if header_response.header_response.clicked() {
            if is_selected {
                self.selection.properties.remove(&prop_id);
            } else {
                self.selection.properties.insert(prop_id);
            }
        }
    }

    fn show_primitive(
        &mut self,
        ui: &mut egui::Ui,
        _structure: &Structure,
        prim_id: PrimitiveId,
        _state: &GuiState,
    ) {
        let is_selected = self.selection.primitives.contains(&prim_id);
        let label = format!("Primitive {:?}", prim_id);

        if ui.selectable_label(is_selected, &label).clicked() {
            if is_selected {
                self.selection.primitives.remove(&prim_id);
            } else {
                self.selection.primitives.insert(prim_id);
            }
        }
    }
}

impl TreeWidget for EguiTreeWidget {
    fn refresh(&mut self, _structure: &Structure, _state: &GuiState) {
        // Tree is rebuilt on each show() call; nothing to precompute.
    }

    fn selection(&self) -> Selection {
        self.selection.clone()
    }

    fn set_selection(&mut self, selection: Selection) {
        self.selection = selection;
    }
}

/// egui-based grid editor.
pub struct EguiGridEditor {
    /// Pending grid edit dialog result.
    pending_edit: Option<GridEditState>,
}

struct GridEditState {
    axis: i32,
    length: f64,
    resolution: i32,
}

impl Default for EguiGridEditor {
    fn default() -> Self {
        Self {
            pending_edit: None,
        }
    }
}

impl EguiGridEditor {
    /// Draw the grid editor in an egui `Ui`. Call this each frame.
    pub fn show(&mut self, ui: &mut egui::Ui, _structure: &Structure) -> Result<(), GuiError> {
        ui.heading("Grid Editor");

        if ui.button("Edit Grid").clicked() {
            self.pending_edit = Some(GridEditState {
                axis: 0,
                length: 1.0,
                resolution: 10,
            });
        }

        let mut window_open = self.pending_edit.is_some();
        if window_open {
            if let Some(edit) = &mut self.pending_edit {
                egui::Window::new("Grid Settings")
                    .show(ui.ctx(), |ui| {
                        ui.label("Axis:");
                        ui.add(egui::DragValue::new(&mut edit.axis).range(0..=2));
                        ui.label("Length:");
                        ui.add(egui::DragValue::new(&mut edit.length).speed(0.01));
                        ui.label("Resolution:");
                        ui.add(egui::DragValue::new(&mut edit.resolution).range(1..=10000));

                        if ui.button("Apply").clicked() {
                            window_open = false;
                        }
                        if ui.button("Cancel").clicked() {
                            window_open = false;
                        }
                    });
            }
        }
        if !window_open {
            self.pending_edit = None;
        }

        Ok(())
    }
}

impl GridEditor for EguiGridEditor {
    fn refresh(&mut self, _structure: &Structure) {
        // Grid data is read from structure on each show() call.
    }

    fn edit_grid(&mut self) -> Result<(), GuiError> {
        self.pending_edit = Some(GridEditState {
            axis: 0,
            length: 1.0,
            resolution: 10,
        });
        Ok(())
    }
}

/// egui-based property editor.
pub struct EguiPropertyEditor {
    /// Currently editing property ID.
    editing: Option<PropertyId>,
}

impl Default for EguiPropertyEditor {
    fn default() -> Self {
        Self { editing: None }
    }
}

impl EguiPropertyEditor {
    /// Draw the property editor in an egui `Ui`. Call this each frame.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        structure: &Structure,
        state: &GuiState,
    ) -> Result<(), GuiError> {
        ui.heading("Property Editor");

        for prop_id in &state.selection.properties {
            if let Some(prop) = structure.property(*prop_id) {
                egui::CollapsingHeader::new(format!("Property {:?}", prop_id))
                    .show(ui, |ui| {
                        ui.label(format!("Type: {:?}", prop.property_type()));
                        ui.label(format!("Name: {}", prop.name()));
                        // Add property-specific controls here
                    });
            }
        }

        Ok(())
    }
}

impl PropertyEditor for EguiPropertyEditor {
    fn refresh(&mut self, _structure: &Structure, _state: &GuiState) {
        // Property data is read on each show() call.
    }

    fn edit_property(&mut self, property_id: PropertyId) -> Result<(), GuiError> {
        self.editing = Some(property_id);
        Ok(())
    }
}

/// egui-based primitive editor.
pub struct EguiPrimitiveEditor {
    /// Currently editing primitive ID.
    editing: Option<PrimitiveId>,
    /// Pending primitive creation type.
    creating: Option<String>,
}

impl Default for EguiPrimitiveEditor {
    fn default() -> Self {
        Self {
            editing: None,
            creating: None,
        }
    }
}

impl EguiPrimitiveEditor {
    /// Draw the primitive editor in an egui `Ui`. Call this each frame.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        structure: &Structure,
        state: &GuiState,
    ) -> Result<(), GuiError> {
        ui.heading("Primitive Editor");

        ui.group(|ui| {
            ui.label("Create Primitive:");
            let types = ["box", "sphere", "cylinder", "polygon"];
            for kind in types {
                if ui.button(kind).clicked() {
                    self.creating = Some(kind.to_string());
                }
            }
        });

        for prim_id in &state.selection.primitives {
            if let Some(prim) = structure.primitive(*prim_id) {
                egui::CollapsingHeader::new(format!("Primitive {:?}", prim_id))
                    .show(ui, |ui| {
                        ui.label(format!("Type: {:?}", prim.primitive_type()));
                        if ui.button("Edit").clicked() {
                            self.editing = Some(*prim_id);
                        }
                        if ui.button("Delete").clicked() {
                            // Deletion handled by caller
                        }
                    });
            }
        }

        Ok(())
    }
}

impl PrimitiveEditor for EguiPrimitiveEditor {
    fn refresh(&mut self, _structure: &Structure, _state: &GuiState) {
        // Primitive data is read on each show() call.
    }

    fn create_primitive(&mut self, kind: &str) -> Result<(), GuiError> {
        self.creating = Some(kind.to_string());
        Ok(())
    }

    fn edit_primitive(&mut self, primitive_id: PrimitiveId) -> Result<(), GuiError> {
        self.editing = Some(primitive_id);
        Ok(())
    }
}