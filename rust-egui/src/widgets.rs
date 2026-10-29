//! egui implementations of the QCSXCAD widget traits.
//!
//! The widgets are immediate mode: they read the current `Document`, emit
//! `Document` edit calls for every changed field (exactly the same calls the
//! tests exercise) and keep only presentation state (expanded tree items,
//! selection mirror, pending delete, editor drafts).
//!
//! Every editor follows the native dialogs: the structure tree shows
//! `<Type>::<name>` properties and `<Type> - ID: <id>` primitives with a
//! visibility bulb, the property editor covers material, excitation, probe,
//! dump and resolution-box properties, and the primitive editor covers the
//! native geometry types.

use std::collections::HashSet;

use csxcad_core::grid::CoordinateSystem;
use csxcad_core::material::{MaterialField, MaterialProperties};
use csxcad_core::model::{
    PrimitiveId, PrimitiveShape, PrimitiveType, Property, PropertyColor, PropertyId,
    PropertyType, Structure,
};
use qcsxcad_core::document::{parse_frequency_list, Document, PropertyEdit, CREATABLE_PROPERTY_TYPES};
use qcsxcad_core::model::{GuiError, GuiState, Selection};
use qcsxcad_core::tree::{tree_rows, TreeItem};
use qcsxcad_core::widgets::TreeWidget;

/// Shared error line for the editors.
fn error_label(ui: &mut egui::Ui, error: &Option<String>) {
    if let Some(message) = error {
        ui.colored_label(egui::Color32::from_rgb(220, 80, 80), message);
    }
}

fn apply(error: &mut Option<String>, result: Result<(), GuiError>) {
    match result {
        Ok(()) => *error = None,
        Err(failure) => *error = Some(failure.to_string()),
    }
}

fn color32(color: PropertyColor) -> egui::Color32 {
    let [red, green, blue, alpha] = color.components();
    egui::Color32::from_rgba_unmultiplied(red, green, blue, alpha)
}

fn property_color(color: egui::Color32) -> PropertyColor {
    let [red, green, blue, alpha] = color.to_srgba_unmultiplied();
    PropertyColor::new(red, green, blue, alpha)
}

/// A labelled drag value that reports a change.
fn drag(ui: &mut egui::Ui, label: &str, value: &mut f64, speed: f64) -> bool {
    ui.label(label);
    ui.add(
        egui::DragValue::new(value)
            .speed(speed)
            .max_decimals(12),
    )
    .changed()
}

/// egui-based structure/property tree widget.
pub struct EguiTreeWidget {
    /// Expanded property IDs in the tree.
    expanded: HashSet<PropertyId>,
    /// Mirror of the application selection (the tree is the owner of clicks).
    selection: Selection,
    /// Item waiting for the delete confirmation.
    pending_delete: Option<TreeItem>,
    last_error: Option<String>,
}

impl Default for EguiTreeWidget {
    fn default() -> Self {
        Self {
            expanded: HashSet::new(),
            selection: Selection::new(),
            pending_delete: None,
            last_error: None,
        }
    }
}

impl EguiTreeWidget {
    /// Draw the tree widget in an egui `Ui`. Call this each frame.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        document: &mut Document,
        state: &mut GuiState,
    ) {
        ui.heading("Structure");
        ui.horizontal(|ui| {
            if ui
                .add_enabled(document.can_undo(), egui::Button::new("Undo"))
                .on_hover_text("Ctrl+Z")
                .clicked()
            {
                document.undo();
            }
            if ui
                .add_enabled(document.can_redo(), egui::Button::new("Redo"))
                .on_hover_text("Ctrl+Shift+Z / Ctrl+Y")
                .clicked()
            {
                document.redo();
            }
            ui.menu_button("+ Property", |ui| {
                for kind in CREATABLE_PROPERTY_TYPES {
                    let label = kind.xml_string().unwrap_or("Unknown");
                    if ui.button(label).clicked() {
                        match document.add_property(kind) {
                            Ok(id) => {
                                self.expanded.insert(id);
                                self.select_item(document.structure(), state, TreeItem::Property(id));
                            }
                            Err(error) => self.last_error = Some(error.to_string()),
                        }
                        ui.close();
                    }
                }
            });
            let has_selection = !self.selection.is_empty();
            if ui
                .add_enabled(has_selection, egui::Button::new("Delete"))
                .clicked()
            {
                self.pending_delete = self.selection.properties.iter().copied().next().map(TreeItem::Property)
                    .or_else(|| {
                        self.selection
                            .primitives
                            .iter()
                            .copied()
                            .next()
                            .map(TreeItem::Primitive)
                    });
            }
        });
        error_label(ui, &self.last_error);
        ui.separator();

        // keep the selected primitives visible in the tree
        for primitive in self.selection.primitives.iter().copied() {
            if let Some(property) = document.structure().property_for_primitive(primitive) {
                if let Some(property_id) = property.id() {
                    self.expanded.insert(property_id);
                }
            }
        }

        egui::ScrollArea::vertical()
            .id_salt("structure_tree_rows")
            .show(ui, |ui| {
            let rows = tree_rows(document.structure(), |id| self.expanded.contains(&id));
            for row in rows {
                ui.horizontal(|ui| {
                    ui.add_space(row.depth as f32 * 14.0);
                    match row.item {
                        TreeItem::Property(id) => {
                            let expanded = self.expanded.contains(&id);
                            if ui
                                .small_button(if expanded { "-" } else { "+" })
                                .on_hover_text("Expand / collapse")
                                .clicked()
                            {
                                if expanded {
                                    self.expanded.remove(&id);
                                } else {
                                    self.expanded.insert(id);
                                }
                            }
                            if let Some(rgba) = row.color {
                                let (rect, _) = ui.allocate_exact_size(
                                    egui::vec2(10.0, 10.0),
                                    egui::Sense::hover(),
                                );
                                ui.painter().rect_filled(
                                    rect,
                                    1.0,
                                    egui::Color32::from_rgba_unmultiplied(
                                        rgba[0], rgba[1], rgba[2], rgba[3],
                                    ),
                                );
                            }
                            let selected = self.selection.properties.contains(&id);
                            if ui.selectable_label(selected, &row.label).clicked() {
                                self.select_item(document.structure(), state, row.item);
                            }
                            // Native `Vis` column: the visibility checkbox
                            // sits at the right edge of the row.
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    let mut visible = row.visible;
                                    if ui
                                        .checkbox(&mut visible, "")
                                        .on_hover_text("Visibility")
                                        .changed()
                                    {
                                        apply(
                                            &mut self.last_error,
                                            document.apply_property_edit(
                                                id,
                                                PropertyEdit::Visible(visible),
                                            ),
                                        );
                                    }
                                },
                            );
                        }
                        TreeItem::Primitive(id) => {
                            // a small color marker keeps the rows aligned
                            ui.add_space(14.0);
                            let selected = self.selection.primitives.contains(&id);
                            if ui.selectable_label(selected, &row.label).clicked() {
                                self.select_item(document.structure(), state, row.item);
                            }
                        }
                    }
                });
            }
            if document.structure().property_count() == 0 {
                ui.label("No properties. Use “+ Property” to create one.");
            }
        });

        self.show_delete_confirmation(ui, document, state);
    }

    fn select_item(&mut self, structure: &Structure, state: &mut GuiState, item: TreeItem) {
        state.selection.clear();
        match item {
            TreeItem::Property(id) => {
                state.selection.properties.insert(id);
            }
            TreeItem::Primitive(id) => {
                state.selection.primitives.insert(id);
                if let Some(property) = structure.property_for_primitive(id) {
                    if let Some(property_id) = property.id() {
                        state.selection.properties.insert(property_id);
                        self.expanded.insert(property_id);
                    }
                }
            }
        }
        self.selection = state.selection.clone();
    }

    fn show_delete_confirmation(
        &mut self,
        ui: &mut egui::Ui,
        document: &mut Document,
        state: &mut GuiState,
    ) {
        let Some(item) = self.pending_delete else {
            return;
        };
        let label = match item {
            TreeItem::Property(id) => format!("property {}", id.get()),
            TreeItem::Primitive(id) => format!("primitive {}", id.get()),
        };
        let mut open = true;
        egui::Window::new("Delete")
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ui.ctx(), |ui| {
                ui.label(format!("Delete {label}?"));
                ui.horizontal(|ui| {
                    if ui.button("Delete").clicked() {
                        let result = match item {
                            TreeItem::Property(id) => document.delete_property(id),
                            TreeItem::Primitive(id) => document.delete_primitive(id),
                        };
                        apply(&mut self.last_error, result);
                        state.selection.clear();
                        self.selection.clear();
                        self.pending_delete = None;
                    }
                    if ui.button("Cancel").clicked() {
                        self.pending_delete = None;
                    }
                });
            });
        if !open {
            self.pending_delete = None;
        }
    }
}

impl EguiTreeWidget {
    /// Expand every property (native `CSTree->expandAll()` after a load).
    pub fn expand_all(&mut self, structure: &Structure) {
        for index in 0..structure.property_count() {
            if let Some(id) = structure.property_by_index(index).and_then(|property| property.id())
            {
                self.expanded.insert(id);
            }
        }
    }

    /// Collapse every property (native `CSTree->collapseAll()`).
    pub fn collapse_all(&mut self) {
        self.expanded.clear();
    }
}

impl TreeWidget for EguiTreeWidget {
    fn refresh(&mut self, _structure: &Structure, _state: &GuiState) {
        // The tree is rebuilt from the document on each frame.
    }

    fn selection(&self) -> Selection {
        self.selection.clone()
    }

    fn set_selection(&mut self, selection: Selection) {
        self.selection = selection;
    }
}

/// egui-based property editor.
#[derive(Default)]
pub struct EguiPropertyEditor {
    last_error: Option<String>,
}

impl EguiPropertyEditor {
    /// Draw the property editor for the selected properties.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        document: &mut Document,
        state: &GuiState,
    ) -> Result<(), GuiError> {
        ui.heading("Property");
        error_label(ui, &self.last_error);

        let mut selected: Vec<PropertyId> = state.selection.properties.iter().copied().collect();
        selected.sort_by_key(|id| id.get());
        if selected.is_empty() {
            ui.label("Select a property in the structure tree.");
            return Ok(());
        }

        egui::ScrollArea::vertical()
            .id_salt("property_editor")
            .show(ui, |ui| {
            for id in selected {
                let Some(property) = document.structure().property(id).cloned() else {
                    continue;
                };
                let header = format!(
                    "{} #{}",
                    property.property_type().xml_string().unwrap_or("Unknown"),
                    id.get()
                );
                egui::CollapsingHeader::new(header)
                    .default_open(true)
                    .show(ui, |ui| {
                        self.edit_common(ui, document, id, &property);
                        match property.property_type() {
                            PropertyType::Material
                            | PropertyType::DispersiveMaterial
                            | PropertyType::LorentzMaterial
                            | PropertyType::DebyeMaterial
                            | PropertyType::DiscreteMaterial => {
                                if let Some(material) = property.material() {
                                    self.edit_material(ui, document, id, material);
                                }
                            }
                            PropertyType::Excitation => {
                                if let Some(excitation) = property.excitation() {
                                    self.edit_excitation(ui, document, id, excitation);
                                }
                            }
                            PropertyType::ProbeBox => {
                                if let Some(probe) = property.probe() {
                                    self.edit_probe(ui, document, id, probe);
                                }
                            }
                            PropertyType::DumpBox => {
                                if let Some(probe) = property.probe() {
                                    self.edit_probe(ui, document, id, probe);
                                }
                                if let Some(dump) = property.dump_box() {
                                    self.edit_dump(ui, document, id, dump);
                                }
                            }
                            PropertyType::ResBox => {
                                if let Some(resolution) = property.resolution_box() {
                                    let mut factor = resolution.factor() as f64;
                                    if drag(ui, "Resolution factor", &mut factor, 1.0) {
                                        apply(
                                            &mut self.last_error,
                                            document.apply_property_edit(
                                                id,
                                                PropertyEdit::ResolutionFactor(
                                                    factor.round().max(1.0) as u32,
                                                ),
                                            ),
                                        );
                                    }
                                }
                            }
                            _ => {}
                        }
                    });
            }
        });
        Ok(())
    }

    fn edit_common(
        &mut self,
        ui: &mut egui::Ui,
        document: &mut Document,
        id: PropertyId,
        property: &Property,
    ) {
        let mut name = property.name().to_owned();
        ui.horizontal(|ui| {
            ui.label("Name:");
            if ui.text_edit_singleline(&mut name).changed() {
                apply(
                    &mut self.last_error,
                    document.apply_property_edit(id, PropertyEdit::Name(name.clone())),
                );
            }
        });
        let mut visible = property.is_visible();
        if ui.checkbox(&mut visible, "Visible").changed() {
            apply(
                &mut self.last_error,
                document.apply_property_edit(id, PropertyEdit::Visible(visible)),
            );
        }
        let mut coordinate_system = property.coordinate_input_type();
        egui::ComboBox::from_label("Coordinates")
            .selected_text(coordinate_system_name(coordinate_system))
            .show_ui(ui, |ui| {
                for candidate in [
                    CoordinateSystem::Cartesian,
                    CoordinateSystem::Cylindrical,
                ] {
                    if ui
                        .selectable_value(
                            &mut coordinate_system,
                            candidate,
                            coordinate_system_name(candidate),
                        )
                        .clicked()
                    {
                        apply(
                            &mut self.last_error,
                            document.apply_property_edit(
                                id,
                                PropertyEdit::CoordinateSystem(candidate),
                            ),
                        );
                    }
                }
            });

        for (label, current, edit) in [
            (
                "Fill color",
                property.fill_color(),
                PropertyEdit::FillColor as fn(Option<PropertyColor>) -> PropertyEdit,
            ),
            (
                "Edge color",
                property.edge_color(),
                PropertyEdit::EdgeColor as fn(Option<PropertyColor>) -> PropertyEdit,
            ),
        ] {
            ui.horizontal(|ui| {
                ui.label(format!("{label}:"));
                let mut color = current
                    .map(color32)
                    .unwrap_or(egui::Color32::from_rgb(128, 128, 160));
                if ui.color_edit_button_srgba(&mut color).changed() {
                    apply(
                        &mut self.last_error,
                        document.apply_property_edit(id, edit(Some(property_color(color)))),
                    );
                }
                if current.is_some() && ui.small_button("clear").clicked() {
                    apply(
                        &mut self.last_error,
                        document.apply_property_edit(id, edit(None)),
                    );
                }
            });
        }
    }

    fn edit_material(
        &mut self,
        ui: &mut egui::Ui,
        document: &mut Document,
        id: PropertyId,
        material: &MaterialProperties,
    ) {
        let mut isotropic = material.is_isotropic();
        if ui.checkbox(&mut isotropic, "Isotropic").changed() {
            apply(
                &mut self.last_error,
                document.apply_property_edit(id, PropertyEdit::MaterialIsotropic(isotropic)),
            );
        }
        let components = if isotropic { 1 } else { 3 };
        for (field, label) in [
            (MaterialField::Epsilon, "Epsilon"),
            (MaterialField::Mue, "Mue"),
            (MaterialField::Kappa, "Kappa"),
            (MaterialField::Sigma, "Sigma"),
        ] {
            ui.horizontal(|ui| {
                ui.label(format!("{label}:"));
                for component in 0..components {
                    let mut value = material
                        .value(field, component, None)
                        .ok()
                        .flatten()
                        .unwrap_or(0.0);
                    if ui
                        .add(
                            egui::DragValue::new(&mut value)
                                .speed(0.01)
                                .max_decimals(12),
                        )
                        .changed()
                        && value.is_finite()
                    {
                        apply(
                            &mut self.last_error,
                            document.apply_property_edit(
                                id,
                                PropertyEdit::MaterialValue {
                                    field,
                                    component,
                                    value,
                                },
                            ),
                        );
                    }
                }
            });
        }
    }

    fn edit_excitation(
        &mut self,
        ui: &mut egui::Ui,
        document: &mut Document,
        id: PropertyId,
        excitation: &csxcad_core::material::ExcitationProperties,
    ) {
        let mut kind = excitation.excitation_type() as f64;
        if drag(ui, "Type", &mut kind, 1.0) {
            apply(
                &mut self.last_error,
                document.apply_property_edit(
                    id,
                    PropertyEdit::ExcitationType(kind.round() as i32),
                ),
            );
        }
        let mut enabled = excitation.is_enabled();
        if ui.checkbox(&mut enabled, "Enabled").changed() {
            apply(
                &mut self.last_error,
                document.apply_property_edit(id, PropertyEdit::ExcitationEnabled(enabled)),
            );
        }
        let mut number = excitation.number() as f64;
        if drag(ui, "Number", &mut number, 1.0) {
            apply(
                &mut self.last_error,
                document.apply_property_edit(
                    id,
                    PropertyEdit::ExcitationNumber(number.round().max(0.0) as u32),
                ),
            );
        }
        let mut frequency = excitation.frequency_term();
        ui.horizontal(|ui| {
            ui.label("Frequency:");
            if ui.text_edit_singleline(&mut frequency).lost_focus() {
                apply(
                    &mut self.last_error,
                    document.apply_property_edit(
                        id,
                        PropertyEdit::ExcitationFrequency(frequency.clone()),
                    ),
                );
            }
        });
        let mut delay = excitation.delay_term();
        ui.horizontal(|ui| {
            ui.label("Delay:");
            if ui.text_edit_singleline(&mut delay).lost_focus() {
                apply(
                    &mut self.last_error,
                    document.apply_property_edit(id, PropertyEdit::ExcitationDelay(delay.clone())),
                );
            }
        });
        ui.separator();
        for (component, label) in ["X", "Y", "Z"].iter().enumerate() {
            ui.horizontal(|ui| {
                let mut term = excitation
                    .excitation_term(component)
                    .unwrap_or_default();
                ui.label(format!("{label} excitation:"));
                if ui.text_edit_singleline(&mut term).lost_focus() {
                    apply(
                        &mut self.last_error,
                        document.apply_property_edit(
                            id,
                            PropertyEdit::ExcitationTerm {
                                component,
                                term: term.clone(),
                            },
                        ),
                    );
                }
                let mut direction = excitation
                    .propagation_direction_term(component)
                    .unwrap_or_default();
                ui.label("k:");
                if ui.text_edit_singleline(&mut direction).lost_focus() {
                    apply(
                        &mut self.last_error,
                        document.apply_property_edit(
                            id,
                            PropertyEdit::PropagationTerm {
                                component,
                                term: direction.clone(),
                            },
                        ),
                    );
                }
            });
        }
    }

    fn edit_probe(
        &mut self,
        ui: &mut egui::Ui,
        document: &mut Document,
        id: PropertyId,
        probe: &csxcad_core::material::ProbeProperties,
    ) {
        let mut number = probe.number() as f64;
        if drag(ui, "Number", &mut number, 1.0) {
            apply(
                &mut self.last_error,
                document.apply_property_edit(
                    id,
                    PropertyEdit::ProbeNumber(number.round().max(0.0) as u32),
                ),
            );
        }
        let mut probe_type = probe.probe_type() as f64;
        if drag(ui, "Probe type", &mut probe_type, 1.0) {
            apply(
                &mut self.last_error,
                document.apply_property_edit(
                    id,
                    PropertyEdit::ProbeType(probe_type.round() as i32),
                ),
            );
        }
        let mut normal = probe.normal_direction() as f64;
        if drag(ui, "Normal direction", &mut normal, 1.0) {
            apply(
                &mut self.last_error,
                document.apply_property_edit(
                    id,
                    PropertyEdit::ProbeNormalDirection(normal.round() as i32),
                ),
            );
        }
        let mut weight = probe.weight();
        if drag(ui, "Weight", &mut weight, 0.01) {
            apply(
                &mut self.last_error,
                document.apply_property_edit(id, PropertyEdit::ProbeWeight(weight)),
            );
        }
        let mut start = probe.start_time();
        if drag(ui, "Start time", &mut start, 1e-12) {
            apply(
                &mut self.last_error,
                document.apply_property_edit(id, PropertyEdit::ProbeStartTime(start)),
            );
        }
        let mut stop = probe.stop_time();
        if drag(ui, "Stop time", &mut stop, 1e-12) {
            apply(
                &mut self.last_error,
                document.apply_property_edit(id, PropertyEdit::ProbeStopTime(stop)),
            );
        }
        let mut oversampling = probe.oversampling() as f64;
        if drag(ui, "Oversampling", &mut oversampling, 1.0) {
            apply(
                &mut self.last_error,
                document.apply_property_edit(
                    id,
                    PropertyEdit::ProbeOversampling(oversampling.round() as i32),
                ),
            );
        }
        self.edit_fd_samples(ui, document, id, probe.fd_samples(), false);
    }

    fn edit_dump(
        &mut self,
        ui: &mut egui::Ui,
        document: &mut Document,
        id: PropertyId,
        dump: &csxcad_core::material::DumpBoxProperties,
    ) {
        let mut dump_type = dump.dump_type() as f64;
        if drag(ui, "Dump type", &mut dump_type, 1.0) {
            apply(
                &mut self.last_error,
                document.apply_property_edit(
                    id,
                    PropertyEdit::DumpType(dump_type.round() as i32),
                ),
            );
        }
        let mut dump_mode = dump.dump_mode() as f64;
        if drag(ui, "Dump mode", &mut dump_mode, 1.0) {
            apply(
                &mut self.last_error,
                document.apply_property_edit(
                    id,
                    PropertyEdit::DumpMode(dump_mode.round() as i32),
                ),
            );
        }
        let mut file_type = dump.file_type() as f64;
        if drag(ui, "File type", &mut file_type, 1.0) {
            apply(
                &mut self.last_error,
                document.apply_property_edit(
                    id,
                    PropertyEdit::DumpFileType(file_type.round() as i32),
                ),
            );
        }
        let mut level = dump.multi_grid_level() as f64;
        if drag(ui, "Multi-grid level", &mut level, 1.0) {
            apply(
                &mut self.last_error,
                document.apply_property_edit(
                    id,
                    PropertyEdit::DumpMultiGridLevel(level.round() as i32),
                ),
            );
        }
        let mut sub_sampling_enabled = dump.sub_sampling_enabled();
        if ui
            .checkbox(&mut sub_sampling_enabled, "Sub-sampling")
            .changed()
        {
            apply(
                &mut self.last_error,
                document.apply_property_edit(
                    id,
                    PropertyEdit::DumpSubSamplingEnabled(sub_sampling_enabled),
                ),
            );
        }
        ui.horizontal(|ui| {
            let sub_sampling = dump.sub_sampling();
            for (direction, label) in ["x", "y", "z"].iter().enumerate() {
                let mut value = sub_sampling[direction] as f64;
                ui.label(*label);
                if ui
                    .add_enabled(
                        sub_sampling_enabled,
                        egui::DragValue::new(&mut value).speed(1.0),
                    )
                    .changed()
                {
                    apply(
                        &mut self.last_error,
                        document.apply_property_edit(
                            id,
                            PropertyEdit::DumpSubSampling {
                                direction,
                                value: value.round().max(1.0) as u32,
                            },
                        ),
                    );
                }
            }
        });
        self.edit_fd_samples(ui, document, id, dump.fd_samples(), true);
        let mut sar_method = dump.sar_method().to_owned();
        ui.horizontal(|ui| {
            ui.label("SAR method:");
            if ui.text_edit_singleline(&mut sar_method).lost_focus() {
                apply(
                    &mut self.last_error,
                    document.apply_property_edit(
                        id,
                        PropertyEdit::DumpSarMethod(sar_method.clone()),
                    ),
                );
            }
        });
    }

    fn edit_fd_samples(
        &mut self,
        ui: &mut egui::Ui,
        document: &mut Document,
        id: PropertyId,
        samples: &[f64],
        is_dump: bool,
    ) {
        let mut text = qcsxcad_core::document::format_frequency_list(samples);
        ui.horizontal(|ui| {
            ui.label("FD samples:");
            if ui.text_edit_singleline(&mut text).lost_focus() {
                match parse_frequency_list(&text) {
                    Ok(samples) => {
                        let edit = if is_dump {
                            PropertyEdit::DumpFdSamples(samples)
                        } else {
                            PropertyEdit::ProbeFdSamples(samples)
                        };
                        apply(&mut self.last_error, document.apply_property_edit(id, edit));
                    }
                    Err(error) => self.last_error = Some(error.to_string()),
                }
            }
        });
    }
}

fn coordinate_system_name(system: CoordinateSystem) -> &'static str {
    match system {
        CoordinateSystem::Cartesian => "Cartesian",
        CoordinateSystem::Cylindrical => "Cylindrical",
        _ => "Undefined",
    }
}

/// Transform controls for the selected primitive: translate, rotate, scale
/// and reset, with one undo entry per action.
#[derive(Clone, Debug)]
pub struct EguiTransformEditor {
    translate: [f64; 3],
    rotate: [f64; 3],
    scale: [f64; 3],
    last_error: Option<String>,
}

impl Default for EguiTransformEditor {
    fn default() -> Self {
        Self {
            translate: [0.0; 3],
            rotate: [0.0; 3],
            scale: [1.25; 3],
            last_error: None,
        }
    }
}

impl EguiTransformEditor {
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        document: &mut Document,
        state: &GuiState,
    ) {
        ui.heading("Transform");
        error_label(ui, &self.last_error);
        let Some(id) = state.selection.primitives.iter().copied().next() else {
            ui.label("Select a primitive to transform.");
            return;
        };

        ui.horizontal(|ui| {
            for (axis, label) in ["dX", "dY", "dZ"].iter().enumerate() {
                drag(ui, label, &mut self.translate[axis], 1.0e-3);
            }
        });
        if ui.button("Translate").clicked() {
            document.begin_gesture();
            apply(
                &mut self.last_error,
                document.move_primitive_by(id, self.translate),
            );
        }

        ui.horizontal(|ui| {
            for (axis, label) in ["X°", "Y°", "Z°"].iter().enumerate() {
                drag(ui, label, &mut self.rotate[axis], 1.0);
            }
        });
        if ui.button("Rotate").clicked() {
            document.begin_gesture();
            for axis in 0..3 {
                if self.rotate[axis] != 0.0 {
                    let mut direction = [0.0; 3];
                    direction[axis] = 1.0;
                    apply(
                        &mut self.last_error,
                        document.rotate_primitive_by(
                            id,
                            direction,
                            self.rotate[axis].to_radians(),
                        ),
                    );
                }
            }
        }

        ui.horizontal(|ui| {
            ui.label("Scale");
            for (axis, label) in ["sX", "sY", "sZ"].iter().enumerate() {
                ui.add(
                    egui::DragValue::new(&mut self.scale[axis])
                        .speed(0.01)
                        .range(0.05..=20.0),
                )
                .on_hover_text(*label);
            }
            if ui.button("Apply").clicked() {
                document.begin_gesture();
                apply(
                    &mut self.last_error,
                    document.scale_primitive_by_xyz(id, self.scale),
                );
            }
        });

        if ui.button("Reset transform").clicked() {
            document.begin_gesture();
            apply(
                &mut self.last_error,
                document.reset_primitive_transform(id),
            );
        }
    }
}

/// egui-based grid and parameter editor.
#[derive(Clone, Debug)]
pub struct EguiGridEditor {
    axis: usize,
    new_line: f64,
    resolution_factor: usize,
    new_parameter: String,
    new_parameter_value: f64,
    new_parameter_linear: bool,
    new_parameter_min: f64,
    new_parameter_max: f64,
    new_parameter_step: f64,
    last_error: Option<String>,
}

impl Default for EguiGridEditor {
    fn default() -> Self {
        Self {
            axis: 0,
            new_line: 0.0,
            resolution_factor: 2,
            new_parameter: String::new(),
            new_parameter_value: 1.0,
            new_parameter_linear: false,
            new_parameter_min: 0.0,
            new_parameter_max: 1.0,
            new_parameter_step: 0.1,
            last_error: None,
        }
    }
}

impl EguiGridEditor {
    /// Draw both sections (used by embedders that do not split them).
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        document: &mut Document,
    ) -> Result<(), GuiError> {
        self.show_mesh(ui, document)?;
        self.show_parameters(ui, document)
    }

    /// Rectilinear grid section (native `Rectilinear Grid` dock).
    pub fn show_mesh(
        &mut self,
        ui: &mut egui::Ui,
        document: &mut Document,
    ) -> Result<(), GuiError> {
        ui.heading("Rectilinear Grid");
        error_label(ui, &self.last_error);

        ui.horizontal(|ui| {
            for (axis, label) in ["X", "Y", "Z"].iter().enumerate() {
                ui.selectable_value(&mut self.axis, axis, *label);
            }
        });

        let mut delta_unit = document.structure().grid().delta_unit();
        if drag(ui, "Delta unit", &mut delta_unit, 1e-4)
            && delta_unit.is_finite()
            && delta_unit > 0.0
        {
            apply(&mut self.last_error, document.set_delta_unit(delta_unit));
        }

        // `RectGrid::lines` lazily sorts the axis, so read a snapshot instead
        // of taking the mutable grid and marking the document modified.
        let mut grid = document.structure().grid().clone();
        let lines = grid
            .lines(self.axis)
            .map(|lines| lines.to_vec())
            .unwrap_or_default();
        ui.label(format!(
            "Axis {} lines ({})",
            ["x", "y", "z"][self.axis],
            lines.len()
        ));
        egui::ScrollArea::vertical()
            .max_height(160.0)
            .id_salt(("grid_lines", self.axis))
            .show(ui, |ui| {
                let mut remove = None;
                for (index, line) in lines.iter().enumerate() {
                    ui.horizontal(|ui| {
                        let mut value = *line;
                        if ui
                            .add(egui::DragValue::new(&mut value).speed(1e-3).max_decimals(12))
                            .changed()
                        {
                            apply(
                                &mut self.last_error,
                                document.set_grid_line(self.axis, index, value),
                            );
                        }
                        if ui.small_button("x").clicked() {
                            remove = Some(index);
                        }
                    });
                }
                if let Some(index) = remove {
                    apply(
                        &mut self.last_error,
                        document.remove_grid_line(self.axis, index),
                    );
                }
            });
        ui.horizontal(|ui| {
            ui.label("Add line:");
            ui.add(egui::DragValue::new(&mut self.new_line).speed(1e-3).max_decimals(12));
            if ui.button("Add").clicked() {
                apply(
                    &mut self.last_error,
                    document.add_grid_line(self.axis, self.new_line).map(|_| ()),
                );
            }
        });
        ui.horizontal(|ui| {
            ui.label("Increase resolution:");
            ui.add(egui::DragValue::new(&mut self.resolution_factor).range(1..=32));
            if ui.button("Apply").clicked() {
                apply(
                    &mut self.last_error,
                    document.increase_resolution(self.axis, self.resolution_factor),
                );
            }
        });

        Ok(())
    }

    /// Parameter section (native `Parameter` dock).
    pub fn show_parameters(
        &mut self,
        ui: &mut egui::Ui,
        document: &mut Document,
    ) -> Result<(), GuiError> {
        ui.heading("Parameters");
        let parameters = document.structure().parameters().iter().cloned().collect::<Vec<_>>();
        let mut remove = None;
        for (index, parameter) in parameters.iter().enumerate() {
            ui.group(|ui| {
                ui.horizontal(|ui| {
                    let mut name = parameter.name().to_owned();
                    ui.label("Name:");
                    if ui.text_edit_singleline(&mut name).lost_focus() {
                        apply(
                            &mut self.last_error,
                            document.set_parameter_name(index, &name),
                        );
                    }
                    let mut value = parameter.value();
                    if drag(ui, "Value", &mut value, 0.01) {
                        apply(
                            &mut self.last_error,
                            document.set_parameter_value(index, value),
                        );
                    }
                    if ui.small_button("x").clicked() {
                        remove = Some(index);
                    }
                });
                if let Some((min, max, step)) = parameter.linear_range() {
                    ui.horizontal(|ui| {
                        let mut min_value = min;
                        let mut max_value = max;
                        let mut step_value = step;
                        let changed = drag(ui, "min", &mut min_value, 0.01)
                            | drag(ui, "max", &mut max_value, 0.01)
                            | drag(ui, "step", &mut step_value, 0.01);
                        if changed {
                            apply(
                                &mut self.last_error,
                                document.set_parameter_range(
                                    index,
                                    min_value,
                                    max_value,
                                    step_value,
                                ),
                            );
                        }
                    });
                    let mut sweep = parameter.sweep_enabled();
                    if ui.checkbox(&mut sweep, "Sweep").changed() {
                        apply(
                            &mut self.last_error,
                            document.set_parameter_sweep(index, sweep),
                        );
                    }
                }
            });
        }
        if let Some(index) = remove {
            apply(&mut self.last_error, document.remove_parameter(index));
        }

        ui.separator();
        ui.horizontal(|ui| {
            ui.label("New:");
            ui.text_edit_singleline(&mut self.new_parameter);
            ui.add(egui::DragValue::new(&mut self.new_parameter_value).speed(0.01));
            ui.checkbox(&mut self.new_parameter_linear, "linear");
        });
        if self.new_parameter_linear {
            ui.horizontal(|ui| {
                drag(ui, "min", &mut self.new_parameter_min, 0.01);
                drag(ui, "max", &mut self.new_parameter_max, 0.01);
                drag(ui, "step", &mut self.new_parameter_step, 0.01);
            });
        }
        if ui.button("Add parameter").clicked() {
            let result = if self.new_parameter_linear {
                document
                    .add_linear_parameter(
                        &self.new_parameter,
                        self.new_parameter_value,
                        self.new_parameter_min,
                        self.new_parameter_max,
                        self.new_parameter_step,
                    )
                    .map(|_| ())
            } else {
                document
                    .add_parameter(&self.new_parameter, self.new_parameter_value)
                    .map(|_| ())
            };
            match result {
                Ok(()) => {
                    self.new_parameter.clear();
                    self.last_error = None;
                }
                Err(error) => self.last_error = Some(error.to_string()),
            }
        }


        Ok(())
    }
}

/// egui-based primitive editor.
#[derive(Default)]
pub struct EguiPrimitiveEditor {
    new_vertex: [f64; 2],
    new_point: [f64; 3],
    last_error: Option<String>,
}

impl EguiPrimitiveEditor {
    /// Draw the primitive editor for the selected primitives.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        document: &mut Document,
        state: &GuiState,
    ) -> Result<(), GuiError> {
        ui.heading("Primitive");
        error_label(ui, &self.last_error);

        let target_property = state
            .selection
            .properties
            .iter()
            .copied()
            .min_by_key(|id| id.get());
        ui.menu_button("+ Primitive", |ui| {
            for kind in [
                PrimitiveType::Box,
                PrimitiveType::MultiBox,
                PrimitiveType::Sphere,
                PrimitiveType::Cylinder,
                PrimitiveType::Polygon,
                PrimitiveType::UserDefined,
            ] {
                if ui.button(kind.type_name()).clicked() {
                    match target_property {
                        Some(property_id) => {
                            apply(
                                &mut self.last_error,
                                document.add_primitive(property_id, kind).map(|_| ()),
                            );
                        }
                        None => {
                            self.last_error =
                                Some("Select a property before adding a primitive".into());
                        }
                    }
                    ui.close();
                }
            }
        });

        let mut selected: Vec<PrimitiveId> = state.selection.primitives.iter().copied().collect();
        selected.sort_by_key(|id| id.get());
        if selected.is_empty() {
            ui.label("Select a primitive in the structure tree.");
            return Ok(());
        }

        egui::ScrollArea::vertical()
            .id_salt("primitive_editor")
            .show(ui, |ui| {
            for id in selected {
                let Some(primitive) = document.structure().primitive(id).cloned() else {
                    continue;
                };
                let header = format!("{} - ID: {}", primitive.primitive_type().type_name(), id.get());
                egui::CollapsingHeader::new(header)
                    .default_open(true)
                    .show(ui, |ui| {
                        let mut priority = primitive.priority() as f64;
                        if drag(ui, "Priority", &mut priority, 1.0) {
                            apply(
                                &mut self.last_error,
                                document.edit_primitive(id, |primitive| {
                                    primitive.set_priority(priority.round() as i32)
                                }),
                            );
                        }
                        match primitive.shape() {
                            PrimitiveShape::Box(shape) => {
                                let coordinates = shape
                                    .evaluated_coordinates(None)
                                    .unwrap_or([[0.0; 3]; 2]);
                                let mut start = coordinates[0];
                                let mut stop = coordinates[1];
                                let mut changed = false;
                                ui.horizontal(|ui| {
                                    for axis in 0..3 {
                                        changed |= drag(
                                            ui,
                                            ["x", "y", "z"][axis],
                                            &mut start[axis],
                                            0.01,
                                        );
                                    }
                                });
                                ui.horizontal(|ui| {
                                    for axis in 0..3 {
                                        changed |=
                                            drag(ui, ["x", "y", "z"][axis], &mut stop[axis], 0.01);
                                    }
                                });
                                if changed {
                                    apply(
                                        &mut self.last_error,
                                        document.edit_primitive(id, |primitive| {
                                            if let PrimitiveShape::Box(shape) = primitive.shape_mut()
                                            {
                                                for axis in 0..3 {
                                                    shape.set_coordinate(2 * axis, start[axis]);
                                                    shape.set_coordinate(2 * axis + 1, stop[axis]);
                                                }
                                            }
                                        }),
                                    );
                                }
                            }
                            PrimitiveShape::Sphere(shape) => {
                                let mut center = shape.center(None).unwrap_or([0.0; 3]);
                                let mut radius = shape.radius(None).unwrap_or(0.0);
                                let mut changed = false;
                                ui.horizontal(|ui| {
                                    for axis in 0..3 {
                                        changed |= drag(
                                            ui,
                                            ["x", "y", "z"][axis],
                                            &mut center[axis],
                                            0.01,
                                        );
                                    }
                                });
                                changed |= drag(ui, "Radius", &mut radius, 0.01);
                                if changed {
                                    apply(
                                        &mut self.last_error,
                                        document.edit_primitive(id, |primitive| {
                                            if let PrimitiveShape::Sphere(shape) = primitive.shape_mut()
                                            {
                                                for axis in 0..3 {
                                                    shape.set_center_coordinate(axis, center[axis]);
                                                }
                                                shape.set_radius(radius);
                                            }
                                        }),
                                    );
                                }
                            }
                            PrimitiveShape::Cylinder(shape) => {
                                let axis = shape
                                    .axis_coordinates(None, CoordinateSystem::Cartesian)
                                    .unwrap_or([[0.0; 3]; 2]);
                                let mut start = axis[0];
                                let mut stop = axis[1];
                                let mut radius = shape.radius(None).unwrap_or(0.0);
                                let shell_width = shape.shell_width(None).ok().flatten();
                                let mut changed = false;
                                ui.horizontal(|ui| {
                                    for index in 0..3 {
                                        changed |= drag(
                                            ui,
                                            ["x", "y", "z"][index],
                                            &mut start[index],
                                            0.01,
                                        );
                                    }
                                });
                                ui.horizontal(|ui| {
                                    for index in 0..3 {
                                        changed |= drag(
                                            ui,
                                            ["x", "y", "z"][index],
                                            &mut stop[index],
                                            0.01,
                                        );
                                    }
                                });
                                changed |= drag(ui, "Radius", &mut radius, 0.01);
                                let mut shell = shell_width;
                                if let Some(width) = &mut shell {
                                    changed |= drag(ui, "Shell width", width, 0.01);
                                }
                                if changed {
                                    apply(
                                        &mut self.last_error,
                                        document.edit_primitive(id, |primitive| {
                                            if let PrimitiveShape::Cylinder(shape) =
                                                primitive.shape_mut()
                                            {
                                                for index in 0..3 {
                                                    shape.set_coordinate(index, start[index]);
                                                    shape.set_coordinate(index + 3, stop[index]);
                                                }
                                                shape.set_radius(radius);
                                                if let Some(width) = shell {
                                                    shape.set_shell_width(width);
                                                }
                                            }
                                        }),
                                    );
                                }
                            }
                            PrimitiveShape::Polygon(shape) => {
                                let mut elevation = shape.elevation(None).unwrap_or(0.0);
                                if drag(ui, "Elevation", &mut elevation, 0.01) {
                                    apply(
                                        &mut self.last_error,
                                        document.edit_primitive(id, |primitive| {
                                            if let PrimitiveShape::Polygon(shape) =
                                                primitive.shape_mut()
                                            {
                                                shape.set_elevation(elevation);
                                            }
                                        }),
                                    );
                                }
                                let mut normal = shape.normal_direction() as f64;
                                if drag(ui, "Normal direction", &mut normal, 1.0) {
                                    apply(
                                        &mut self.last_error,
                                        document.edit_primitive(id, |primitive| {
                                            if let PrimitiveShape::Polygon(shape) =
                                                primitive.shape_mut()
                                            {
                                                shape.set_normal_direction(normal.round().max(0.0)
                                                    as usize);
                                            }
                                        }),
                                    );
                                }
                                let vertices = shape.evaluated_vertices(None).unwrap_or_default();
                                let mut remove = None;
                                for (index, vertex) in vertices.iter().enumerate() {
                                    ui.horizontal(|ui| {
                                        let mut u = vertex[0];
                                        let mut v = vertex[1];
                                        let changed = drag(ui, "u", &mut u, 0.01)
                                            | drag(ui, "v", &mut v, 0.01);
                                        if changed {
                                            apply(
                                                &mut self.last_error,
                                                document.edit_primitive(
                                                    id,
                                                    |primitive| {
                                                        if let PrimitiveShape::Polygon(shape) =
                                                            primitive.shape_mut()
                                                        {
                                                            shape.set_coordinate(index, 0, u);
                                                            shape.set_coordinate(index, 1, v);
                                                        }
                                                    },
                                                ),
                                            );
                                        }
                                        if ui.small_button("x").clicked() {
                                            remove = Some(index);
                                        }
                                    });
                                }
                                ui.horizontal(|ui| {
                                    drag(ui, "u", &mut self.new_vertex[0], 0.01);
                                    drag(ui, "v", &mut self.new_vertex[1], 0.01);
                                    if ui.button("Add vertex").clicked() {
                                        let vertex = self.new_vertex;
                                        apply(
                                            &mut self.last_error,
                                            document.edit_primitive(id, |primitive| {
                                                if let PrimitiveShape::Polygon(shape) =
                                                    primitive.shape_mut()
                                                {
                                                    shape.add_vertex(vertex);
                                                }
                                            }),
                                        );
                                    }
                                });
                                if let Some(index) = remove {
                                    apply(
                                        &mut self.last_error,
                                        document.edit_primitive(id, |primitive| {
                                            if let PrimitiveShape::Polygon(shape) =
                                                primitive.shape_mut()
                                            {
                                                shape.remove_vertex(index);
                                            }
                                        }),
                                    );
                                }
                            }
                            PrimitiveShape::Curve(shape) => {
                                let points = shape.evaluated_points(None).unwrap_or_default();
                                for (index, point) in points.iter().enumerate() {
                                    ui.horizontal(|ui| {
                                        let mut value = *point;
                                        let mut changed = false;
                                        for axis in 0..3 {
                                            changed |= drag(
                                                ui,
                                                ["x", "y", "z"][axis],
                                                &mut value[axis],
                                                0.01,
                                            );
                                        }
                                        if changed {
                                            apply(
                                                &mut self.last_error,
                                                document.edit_primitive(
                                                    id,
                                                    |primitive| {
                                                        if let PrimitiveShape::Curve(shape) =
                                                            primitive.shape_mut()
                                                        {
                                                            for axis in 0..3 {
                                                                shape.set_coordinate(
                                                                    index, axis, value[axis],
                                                                );
                                                            }
                                                        }
                                                    },
                                                ),
                                            );
                                        }
                                    });
                                }
                                ui.horizontal(|ui| {
                                    for axis in 0..3 {
                                        drag(
                                            ui,
                                            ["x", "y", "z"][axis],
                                            &mut self.new_point[axis],
                                            0.01,
                                        );
                                    }
                                    if ui.button("Add point").clicked() {
                                        let point = self.new_point;
                                        apply(
                                            &mut self.last_error,
                                            document.edit_primitive(id, |primitive| {
                                                if let PrimitiveShape::Curve(shape) =
                                                    primitive.shape_mut()
                                                {
                                                    shape.add_point(point);
                                                }
                                            }),
                                        );
                                    }
                                });
                            }
                            PrimitiveShape::MultiBox(shape) => {
                                let count = shape.box_count();
                                ui.label(format!("{count} box(es)"));
                                for index in 0..count {
                                    let Some(box_shape) = shape.box_at(index) else {
                                        continue;
                                    };
                                    let coordinates = box_shape
                                        .evaluated_coordinates(None)
                                        .unwrap_or([[0.0; 3]; 2]);
                                    ui.label(format!(
                                        "#{index}: [{:.3}, {:.3}, {:.3}] – [{:.3}, {:.3}, {:.3}]",
                                        coordinates[0][0],
                                        coordinates[0][1],
                                        coordinates[0][2],
                                        coordinates[1][0],
                                        coordinates[1][1],
                                        coordinates[1][2]
                                    ));
                                }
                            }
                            PrimitiveShape::Point(shape) => {
                                let mut point = shape.evaluated_coordinates(None).unwrap_or([0.0; 3]);
                                let mut changed = false;
                                ui.horizontal(|ui| {
                                    for axis in 0..3 {
                                        changed |= drag(
                                            ui,
                                            ["x", "y", "z"][axis],
                                            &mut point[axis],
                                            0.01,
                                        );
                                    }
                                });
                                if changed {
                                    apply(
                                        &mut self.last_error,
                                        document.edit_primitive(id, |primitive| {
                                            if let PrimitiveShape::Point(shape) =
                                                primitive.shape_mut()
                                            {
                                                for axis in 0..3 {
                                                    shape.set_coordinate(axis, point[axis]);
                                                }
                                            }
                                        }),
                                    );
                                }
                            }
                            _ => {
                                ui.label("This primitive type has no geometry editor yet.");
                            }
                        }
                        if ui.button("Delete primitive").clicked() {
                            apply(&mut self.last_error, document.delete_primitive(id));
                        }
                    });
            }
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use csxcad_core::model::Primitive;
    use qcsxcad_core::document::PropertyEdit;

    /// Run one full editor frame for the current selection.
    fn run_frame(document: &mut Document, state: &mut GuiState) {
        let mut tree = EguiTreeWidget::default();
        tree.set_selection(state.selection.clone());
        let mut property_editor = EguiPropertyEditor::default();
        let mut primitive_editor = EguiPrimitiveEditor::default();
        let mut grid_editor = EguiGridEditor::default();
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                tree.show(ui, document, state);
                let _ = property_editor.show(ui, document, state);
                let _ = primitive_editor.show(ui, document, state);
                let _ = grid_editor.show(ui, document);
            });
        });
        // a real renderer consumes the texture deltas; dropping them unapplied
        // is a panicking contract violation in epaint
        output.textures_delta.set.clear();
        output.textures_delta.free.clear();
    }

    fn document_with_every_editor() -> (Document, GuiState) {
        let mut document = Document::new();
        let mut state = GuiState::new();
        for kind in CREATABLE_PROPERTY_TYPES {
            let property = document.add_property(kind).expect("property");
            document
                .apply_property_edit(property, PropertyEdit::Name(kind.xml_string().unwrap().into()))
                .expect("name");
            // give the probe and dump boxes a visible FD sample list
            if kind == PropertyType::ProbeBox {
                document
                    .apply_property_edit(property, PropertyEdit::ProbeFdSamples(vec![1e9, 2e9]))
                    .expect("fd samples");
            }
            if kind == PropertyType::DumpBox {
                document
                    .apply_property_edit(property, PropertyEdit::DumpFdSamples(vec![1e9, 2e9]))
                    .expect("fd samples");
            }
            state.selection.properties.insert(property);
        }
        for kind in [
            PrimitiveType::Box,
            PrimitiveType::MultiBox,
            PrimitiveType::Sphere,
            PrimitiveType::Cylinder,
            PrimitiveType::Polygon,
            PrimitiveType::UserDefined,
            PrimitiveType::Point,
            PrimitiveType::Curve,
        ] {
            let property = state
                .selection
                .properties
                .iter()
                .copied()
                .min_by_key(|id| id.get())
                .expect("property");
            if document.add_primitive(property, kind).is_ok() {
                // some types are not creatable through the native menu
                let primitive = document
                    .structure()
                    .property(property)
                    .expect("property")
                    .primitives()
                    .last()
                    .and_then(|primitive| primitive.id())
                    .expect("primitive");
                state.selection.primitives.insert(primitive);
            }
        }
        document.add_parameter("height", 2.0).expect("parameter");
        document
            .add_linear_parameter("width", 1.0, 0.5, 3.0, 0.5)
            .expect("parameter");
        (document, state)
    }

    #[test]
    fn every_editor_renders_a_document_without_errors() {
        let (mut document, mut state) = document_with_every_editor();
        run_frame(&mut document, &mut state);
        // the frame must not have created or deleted anything
        assert_eq!(document.structure().property_count(), 6);
        assert_eq!(document.structure().parameters().len(), 2);
        assert!(!state.selection.properties.is_empty());
        assert!(!state.selection.primitives.is_empty());
    }

    #[test]
    fn every_primitive_shape_renders_in_the_primitive_editor() {
        let mut structure = Structure::new();
        let mut property = Property::new("shapes", PropertyType::Metal);
        property.add_primitive(Primitive::box_primitive([-1.0; 3], [1.0; 3]));
        property.add_primitive(Primitive::sphere([0.0; 3], 1.0));
        property.add_primitive(Primitive::cylinder([0.0, 0.0, 0.0], [0.0, 0.0, 1.0], 0.5));
        property.add_primitive(Primitive::cylindrical_shell(
            [0.0, 0.0, 0.0],
            [0.0, 0.0, 1.0],
            0.5,
            0.1,
        ));
        let mut curve = Primitive::curve();
        if let PrimitiveShape::Curve(shape) = curve.shape_mut() {
            shape.add_point([0.0, 0.0, 0.0]);
            shape.add_point([1.0, 0.0, 0.0]);
        }
        property.add_primitive(curve);
        let mut wire = Primitive::wire(0.1);
        if let PrimitiveShape::Curve(shape) = wire.shape_mut() {
            shape.add_point([0.0, 0.0, 0.0]);
            shape.add_point([1.0, 1.0, 0.0]);
        }
        property.add_primitive(wire);
        let mut polygon = Primitive::polygon();
        if let PrimitiveShape::Polygon(shape) = polygon.shape_mut() {
            shape.add_vertex([0.0, 0.0]);
            shape.add_vertex([1.0, 0.0]);
            shape.add_vertex([1.0, 1.0]);
        }
        property.add_primitive(polygon);
        let mut lin_poly = Primitive::lin_poly(1.0);
        if let PrimitiveShape::Polygon(shape) = lin_poly.shape_mut() {
            shape.add_vertex([0.0, 0.0]);
            shape.add_vertex([1.0, 0.0]);
        }
        property.add_primitive(lin_poly);
        let mut rot_poly = Primitive::rot_poly();
        if let PrimitiveShape::Polygon(shape) = rot_poly.shape_mut() {
            shape.add_vertex([0.0, 0.0]);
            shape.add_vertex([1.0, 0.0]);
            shape.set_rotation_axis_direction(2);
        }
        property.add_primitive(rot_poly);
        property.add_primitive(Primitive::point([1.0, 2.0, 3.0]));
        let mut multibox = Primitive::multibox();
        if let Some(shape) = multibox.multibox_shape_mut() {
            shape.add_box(csxcad_core::model::BoxPrimitive::new(
                [0.0; 3],
                [1.0; 3],
            ));
        }
        property.add_primitive(multibox);
        property.add_primitive(Primitive::user_defined());

        let property_id = structure.add_property(property);
        let primitive_ids: Vec<PrimitiveId> = structure
            .property(property_id)
            .expect("property")
            .primitives()
            .iter()
            .filter_map(|primitive| primitive.id())
            .collect();
        assert_eq!(primitive_ids.len(), 12);

        let mut document = Document::from_structure(structure);
        let mut state = GuiState::new();
        state.selection.properties.insert(property_id);
        for id in primitive_ids {
            state.selection.primitives.insert(id);
        }
        run_frame(&mut document, &mut state);
        assert_eq!(document.structure().primitive_count(), 12);
    }
}
