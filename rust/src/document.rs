//! Editable GUI document.
//!
//! `Document` owns the `csxcad_core::model::Structure` being edited and tracks
//! the modified flag. Every mutation goes through one of the editing methods so
//! the GUI widgets (and the tests) use the same code path, and each edit is
//! applied to the model exactly like the native property editor does when its
//! dialog is accepted.

use crate::model::GuiError;
use csxcad_core::grid::CoordinateSystem;
use csxcad_core::material::MaterialField;
use csxcad_core::model::{
    Primitive, PrimitiveId, PrimitiveType, Property, PropertyColor, PropertyId, PropertyType,
    Structure,
};

/// Property types the native tree offers under "New Property".
pub const CREATABLE_PROPERTY_TYPES: [PropertyType; 6] = [
    PropertyType::Material,
    PropertyType::Metal,
    PropertyType::Excitation,
    PropertyType::ProbeBox,
    PropertyType::ResBox,
    PropertyType::DumpBox,
];

/// A single edit of one property, as emitted by the property editor.
#[derive(Clone, Debug, PartialEq)]
pub enum PropertyEdit {
    Name(String),
    Visible(bool),
    FillColor(Option<PropertyColor>),
    EdgeColor(Option<PropertyColor>),
    CoordinateSystem(CoordinateSystem),
    MaterialIsotropic(bool),
    MaterialValue {
        field: MaterialField,
        component: usize,
        value: f64,
    },
    ExcitationType(i32),
    ExcitationEnabled(bool),
    ExcitationNumber(u32),
    ExcitationFrequency(String),
    ExcitationDelay(String),
    ExcitationTerm {
        component: usize,
        term: String,
    },
    PropagationTerm {
        component: usize,
        term: String,
    },
    ProbeNumber(u32),
    ProbeType(i32),
    ProbeNormalDirection(i32),
    ProbeWeight(f64),
    ProbeStartTime(f64),
    ProbeStopTime(f64),
    ProbeOversampling(i32),
    ProbeFdSamples(Vec<f64>),
    DumpNumber(u32),
    DumpType(i32),
    DumpMode(i32),
    DumpFileType(i32),
    DumpMultiGridLevel(i32),
    DumpSubSamplingEnabled(bool),
    DumpSubSampling {
        direction: usize,
        value: u32,
    },
    DumpOversampling(i32),
    DumpFdSamples(Vec<f64>),
    DumpSarMethod(String),
    ResolutionFactor(u32),
}

/// Maximum number of undo steps kept in memory.
const MAX_UNDO_STEPS: usize = 100;

/// The CSXCAD structure plus GUI editing state.
#[derive(Clone, Debug, PartialEq)]
pub struct Document {
    structure: Structure,
    modified: bool,
    /// Bumped on every edit so the renderer can rebuild its geometry only
    /// when the model actually changed.
    revision: u64,
    undo_stack: Vec<Structure>,
    redo_stack: Vec<Structure>,
}

impl Default for Document {
    fn default() -> Self {
        Self {
            structure: Structure::new(),
            modified: false,
            revision: 0,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
        }
    }
}

impl Document {
    pub fn new() -> Self {
        Self::default()
    }

    /// Wrap an already loaded structure (loading does not mark it modified).
    pub fn from_structure(structure: Structure) -> Self {
        Self {
            structure,
            ..Self::default()
        }
    }

    /// Take the structure back out (used when handing the model to the solver).
    pub fn into_structure(self) -> Structure {
        self.structure
    }

    /// Monotonic edit counter.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Snapshot the structure before an edit so it can be undone.
    fn begin_edit(&mut self) {
        self.undo_stack.push(self.structure.clone());
        if self.undo_stack.len() > MAX_UNDO_STEPS {
            self.undo_stack.remove(0);
        }
        self.redo_stack.clear();
        self.modified = true;
        self.revision += 1;
    }

    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    /// Restore the state before the last edit.
    ///
    /// Snapshots that equal the current structure (left behind by a rejected
    /// edit) are skipped so undo never appears to do nothing.
    pub fn undo(&mut self) -> bool {
        while let Some(previous) = self.undo_stack.pop() {
            if previous == self.structure {
                continue;
            }
            let current = std::mem::replace(&mut self.structure, previous);
            self.redo_stack.push(current);
            self.modified = true;
            self.revision += 1;
            return true;
        }
        false
    }

    /// Re-apply the last undone edit.
    pub fn redo(&mut self) -> bool {
        while let Some(next) = self.redo_stack.pop() {
            if next == self.structure {
                continue;
            }
            let current = std::mem::replace(&mut self.structure, next);
            self.undo_stack.push(current);
            self.modified = true;
            self.revision += 1;
            return true;
        }
        false
    }

    /// Drop the whole undo/redo history (after a load or save-as).
    pub fn clear_history(&mut self) {
        self.undo_stack.clear();
        self.redo_stack.clear();
    }

    pub fn structure(&self) -> &Structure {
        &self.structure
    }

    /// Mutable structure access for edits that have no dedicated method
    /// (grid editor, parameter editor, primitive editors). Marks the document
    /// modified.
    pub fn structure_mut(&mut self) -> &mut Structure {
        self.begin_edit();
        &mut self.structure
    }

    pub fn is_modified(&self) -> bool {
        self.modified
    }

    pub fn set_modified(&mut self, modified: bool) {
        self.modified = modified;
    }

    /// Add a property of one of the native creatable types (empty name, as the
    /// native "new property" dialog starts).
    pub fn add_property(&mut self, property_type: PropertyType) -> Result<PropertyId, GuiError> {
        if !CREATABLE_PROPERTY_TYPES.contains(&property_type) {
            return Err(GuiError::InvalidPropertyType(format!(
                "{}",
                property_type
                    .xml_string()
                    .unwrap_or("unsupported property type")
            )));
        }
        self.begin_edit();
        let id = self.structure.add_property(Property::new("", property_type));
        Ok(id)
    }

    pub fn delete_property(&mut self, id: PropertyId) -> Result<(), GuiError> {
        if self.structure.property(id).is_none() {
            return Err(GuiError::Model(format!(
                "property {} was not found",
                id.get()
            )));
        }
        self.begin_edit();
        if self.structure.delete_property(id) {
            Ok(())
        } else {
            Err(GuiError::Model(format!("property {} was not found", id.get())))
        }
    }

    /// Apply one property edit. Edits that do not match the property type
    /// (for example a material value on a metal) are rejected instead of being
    /// silently ignored.
    pub fn apply_property_edit(
        &mut self,
        id: PropertyId,
        edit: PropertyEdit,
    ) -> Result<(), GuiError> {
        if self.structure.property(id).is_none() {
            return Err(GuiError::Model(format!(
                "property {} was not found",
                id.get()
            )));
        }
        self.begin_edit();
        let Some(property) = self.structure.property_mut(id) else {
            return Err(GuiError::Model(format!("property {} was not found", id.get())));
        };
        let type_name = property
            .property_type()
            .xml_string()
            .unwrap_or("Unknown")
            .to_owned();
        match edit {
            PropertyEdit::Name(name) => property.set_name(name),
            PropertyEdit::Visible(visible) => property.set_visible(visible),
            PropertyEdit::FillColor(color) => property.set_fill_color(color),
            PropertyEdit::EdgeColor(color) => property.set_edge_color(color),
            PropertyEdit::CoordinateSystem(system) => property.set_coordinate_input_type(system),
            PropertyEdit::MaterialIsotropic(isotropic) => {
                material_mut(property, &type_name)?.set_isotropic(isotropic);
            }
            PropertyEdit::MaterialValue {
                field,
                component,
                value,
            } => {
                let changed = material_mut(property, &type_name)?.set_value(field, component, value);
                if !changed {
                    return Err(GuiError::Model(format!(
                        "material component {component} is out of range"
                    )));
                }
            }
            PropertyEdit::ExcitationType(kind) => {
                excitation_mut(property, &type_name)?.set_excitation_type(kind);
            }
            PropertyEdit::ExcitationEnabled(enabled) => {
                excitation_mut(property, &type_name)?.set_enabled(enabled);
            }
            PropertyEdit::ExcitationNumber(number) => {
                excitation_mut(property, &type_name)?.set_number(number);
            }
            PropertyEdit::ExcitationFrequency(term) => {
                excitation_mut(property, &type_name)?.set_frequency_term(&term)?;
            }
            PropertyEdit::ExcitationDelay(term) => {
                excitation_mut(property, &type_name)?.set_delay_term(&term)?;
            }
            PropertyEdit::ExcitationTerm { component, term } => {
                let changed = excitation_mut(property, &type_name)?
                    .set_excitation_term(component, &term)?;
                if !changed {
                    return Err(GuiError::Model(format!(
                        "excitation component {component} is out of range"
                    )));
                }
            }
            PropertyEdit::PropagationTerm { component, term } => {
                let changed = excitation_mut(property, &type_name)?
                    .set_propagation_direction_term(component, &term)?;
                if !changed {
                    return Err(GuiError::Model(format!(
                        "propagation component {component} is out of range"
                    )));
                }
            }
            PropertyEdit::ProbeNumber(number) => {
                probe_mut(property, &type_name)?.set_number(number);
            }
            PropertyEdit::ProbeType(kind) => {
                probe_mut(property, &type_name)?.set_probe_type(kind);
            }
            PropertyEdit::ProbeNormalDirection(direction) => {
                probe_mut(property, &type_name)?.set_normal_direction(direction);
            }
            PropertyEdit::ProbeWeight(weight) => {
                probe_mut(property, &type_name)?.set_weight(weight);
            }
            PropertyEdit::ProbeStartTime(time) => {
                probe_mut(property, &type_name)?.set_start_time(time as f32);
            }
            PropertyEdit::ProbeStopTime(time) => {
                probe_mut(property, &type_name)?.set_stop_time(time as f32);
            }
            PropertyEdit::ProbeOversampling(value) => {
                probe_mut(property, &type_name)?.set_oversampling(value);
            }
            PropertyEdit::ProbeFdSamples(samples) => {
                probe_mut(property, &type_name)?.set_fd_samples(samples);
            }
            PropertyEdit::DumpNumber(number) => {
                // native `CSPropDumpBox` derives from `CSPropProbeBox`
                probe_mut(property, &type_name)?.set_number(number);
            }
            PropertyEdit::DumpType(kind) => {
                dump_box_mut(property, &type_name)?.set_dump_type(kind);
            }
            PropertyEdit::DumpMode(mode) => {
                dump_box_mut(property, &type_name)?.set_dump_mode(mode);
            }
            PropertyEdit::DumpFileType(file_type) => {
                dump_box_mut(property, &type_name)?.set_file_type(file_type);
            }
            PropertyEdit::DumpMultiGridLevel(level) => {
                dump_box_mut(property, &type_name)?.set_multi_grid_level(level);
            }
            PropertyEdit::DumpSubSamplingEnabled(enabled) => {
                dump_box_mut(property, &type_name)?.set_sub_sampling_enabled(enabled);
            }
            PropertyEdit::DumpSubSampling { direction, value } => {
                let changed =
                    dump_box_mut(property, &type_name)?.set_sub_sampling(direction, value);
                if !changed {
                    return Err(GuiError::Model(format!(
                        "sub-sampling direction {direction} is out of range"
                    )));
                }
            }
            PropertyEdit::DumpOversampling(value) => {
                // native `CSPropDumpBox` inherits a single `m_OverSampling`
                // from `CSPropProbeBox`
                probe_mut(property, &type_name)?.set_oversampling(value);
            }
            PropertyEdit::DumpFdSamples(samples) => {
                dump_box_mut(property, &type_name)?.set_fd_samples(samples);
            }
            PropertyEdit::DumpSarMethod(method) => {
                dump_box_mut(property, &type_name)?.set_sar_method(method);
            }
            PropertyEdit::ResolutionFactor(factor) => {
                let Some(resolution) = property.resolution_box_mut() else {
                    return Err(type_mismatch(&type_name, "a resolution box"));
                };
                resolution.set_factor(factor);
            }
        }
        Ok(())
    }

    // ----- primitives (RM-100) -----

    /// Add a primitive of one of the native "New Primitive" types to a
    /// property. The primitive starts with the native constructor defaults and
    /// is completed by the primitive editor.
    pub fn add_primitive(
        &mut self,
        property_id: PropertyId,
        kind: PrimitiveType,
    ) -> Result<PrimitiveId, GuiError> {
        self.add_primitive_with(property_id, kind, |_| {})
    }

    /// Add a primitive and configure it before the edit is recorded, so the
    /// create-and-place step is one undo entry (native dialog semantics).
    pub fn add_primitive_with(
        &mut self,
        property_id: PropertyId,
        kind: PrimitiveType,
        configure: impl FnOnce(&mut Primitive),
    ) -> Result<PrimitiveId, GuiError> {
        let mut primitive = match kind {
            PrimitiveType::Box => Primitive::box_primitive([0.0; 3], [0.0; 3]),
            PrimitiveType::MultiBox => Primitive::multibox(),
            PrimitiveType::Sphere => Primitive::sphere([0.0; 3], 0.0),
            PrimitiveType::Cylinder => Primitive::cylinder([0.0; 3], [0.0; 3], 0.0),
            PrimitiveType::Polygon => Primitive::polygon(),
            PrimitiveType::UserDefined => Primitive::user_defined(),
            other => {
                return Err(GuiError::InvalidPrimitiveType(
                    other.type_name().to_owned(),
                ))
            }
        };
        configure(&mut primitive);
        self.structure_mut()
            .add_primitive(property_id, primitive)
            .map_err(|error| GuiError::Model(error.to_string()))
    }

    /// Start a multi-frame gesture (a viewport drag): one undo snapshot for
    /// the whole gesture. Mutations that follow must not call `begin_edit`.
    pub fn begin_gesture(&mut self) {
        self.begin_edit();
    }

    /// Scale a primitive around its bounding-box centre by `factor`.
    pub fn scale_primitive_by(
        &mut self,
        primitive_id: PrimitiveId,
        factor: f64,
    ) -> Result<(), GuiError> {
        if !factor.is_finite() || factor <= 0.0 {
            return Err(GuiError::Model("scale factor must be positive".to_string()));
        }
        let Some(primitive) = self.primitive_mut_by_id(primitive_id)? else {
            return Err(GuiError::Model(format!(
                "primitive {} was not found",
                primitive_id.get()
            )));
        };
        let bounds = primitive
            .bounding_box(None, false)
            .map_err(|error| GuiError::Model(error.to_string()))?;
        let center = [
            (bounds[0] + bounds[1]) * 0.5,
            (bounds[2] + bounds[3]) * 0.5,
            (bounds[4] + bounds[5]) * 0.5,
        ];
        let transform = primitive.transform_mut();
        transform.translate([-center[0], -center[1], -center[2]], true);
        transform.scale(factor, true);
        transform.translate(center, true);
        self.revision += 1;
        self.modified = true;
        Ok(())
    }

    /// Clear the primitive's transform (native "reset transform").
    pub fn reset_primitive_transform(
        &mut self,
        primitive_id: PrimitiveId,
    ) -> Result<(), GuiError> {
        let Some(primitive) = self.primitive_mut_by_id(primitive_id)? else {
            return Err(GuiError::Model(format!(
                "primitive {} was not found",
                primitive_id.get()
            )));
        };
        primitive.transform_mut().reset();
        self.revision += 1;
        self.modified = true;
        Ok(())
    }

    /// Resolve a primitive by id, mutably.
    fn primitive_mut_by_id(
        &mut self,
        primitive_id: PrimitiveId,
    ) -> Result<Option<&mut Primitive>, GuiError> {
        let Some(property_index) = self
            .structure
            .property_for_primitive(primitive_id)
            .map(|property| property.index())
        else {
            return Ok(None);
        };
        let Some(position) = self
            .structure
            .property_by_index(property_index)
            .and_then(|property| {
                property
                    .primitives()
                    .iter()
                    .position(|primitive| primitive.id() == Some(primitive_id))
            })
        else {
            return Ok(None);
        };
        Ok(self
            .structure
            .property_by_index_mut(property_index)
            .and_then(|property| property.primitive_mut(position)))
    }

    /// Rotate a primitive around its bounding-box centre by `angle` radians
    /// about `axis` (the viewport rotate handle; Alt+drag).
    pub fn rotate_primitive_by(
        &mut self,
        primitive_id: PrimitiveId,
        axis: [f64; 3],
        angle: f64,
    ) -> Result<(), GuiError> {
        let Some(property_index) = self
            .structure
            .property_for_primitive(primitive_id)
            .map(|property| property.index())
        else {
            return Err(GuiError::Model(format!(
                "primitive {} was not found",
                primitive_id.get()
            )));
        };
        let position = self
            .structure
            .property_by_index(property_index)
            .and_then(|property| {
                property
                    .primitives()
                    .iter()
                    .position(|primitive| primitive.id() == Some(primitive_id))
            });
        let Some(position) = position else {
            return Err(GuiError::Model(format!(
                "primitive {} was not found",
                primitive_id.get()
            )));
        };
        let Some(primitive) = self
            .structure
            .property_by_index_mut(property_index)
            .and_then(|property| property.primitive_mut(position))
        else {
            return Err(GuiError::Model("primitive was not found".to_string()));
        };
        let bounds = primitive
            .bounding_box(None, false)
            .map_err(|error| GuiError::Model(error.to_string()))?;
        let center = [
            (bounds[0] + bounds[1]) * 0.5,
            (bounds[2] + bounds[3]) * 0.5,
            (bounds[4] + bounds[5]) * 0.5,
        ];
        let transform = primitive.transform_mut();
        transform.translate([-center[0], -center[1], -center[2]], true);
        transform
            .rotate_origin(axis, angle, true)
            .map_err(|error| GuiError::Model(error.to_string()))?;
        transform.translate(center, true);
        self.revision += 1;
        self.modified = true;
        Ok(())
    }

    /// Translate a primitive without a new undo snapshot; call
    /// [`Self::begin_gesture`] once when the drag starts.
    pub fn move_primitive_by(
        &mut self,
        primitive_id: PrimitiveId,
        delta: [f64; 3],
    ) -> Result<(), GuiError> {
        let Some(property_index) = self
            .structure
            .property_for_primitive(primitive_id)
            .map(|property| property.index())
        else {
            return Err(GuiError::Model(format!(
                "primitive {} was not found",
                primitive_id.get()
            )));
        };
        let position = self
            .structure
            .property_by_index(property_index)
            .and_then(|property| {
                property
                    .primitives()
                    .iter()
                    .position(|primitive| primitive.id() == Some(primitive_id))
            });
        let Some(position) = position else {
            return Err(GuiError::Model(format!(
                "primitive {} was not found",
                primitive_id.get()
            )));
        };
        let Some(primitive) = self
            .structure
            .property_by_index_mut(property_index)
            .and_then(|property| property.primitive_mut(position))
        else {
            return Err(GuiError::Model("primitive was not found".to_string()));
        };
        if let Some(shape) = primitive.box_shape_mut() {
            shape
                .translate(delta)
                .map_err(|error| GuiError::Model(error.to_string()))?;
        } else if let Some(shape) = primitive.sphere_shape_mut() {
            shape
                .translate(delta)
                .map_err(|error| GuiError::Model(error.to_string()))?;
        } else {
            // Cylinders, polygons and user-defined shapes move through their
            // transform so every type supports the viewport drag.
            primitive.transform_mut().translate(delta, true);
        }
        self.revision += 1;
        self.modified = true;
        Ok(())
    }

    pub fn delete_primitive(&mut self, primitive_id: PrimitiveId) -> Result<(), GuiError> {
        if self.structure_mut().delete_primitive(primitive_id) {
            Ok(())
        } else {
            Err(GuiError::Model(format!(
                "primitive {} was not found",
                primitive_id.get()
            )))
        }
    }

    /// Apply one primitive edit (geometry, priority, ...).
    pub fn edit_primitive(
        &mut self,
        primitive_id: PrimitiveId,
        edit: impl FnOnce(&mut Primitive),
    ) -> Result<(), GuiError> {
        let Some(property_index) = self
            .structure()
            .property_for_primitive(primitive_id)
            .map(|property| property.index())
        else {
            return Err(GuiError::Model(format!(
                "primitive {} was not found",
                primitive_id.get()
            )));
        };
        let structure = self.structure_mut();
        let Some(property) = structure.property_by_index_mut(property_index) else {
            return Err(GuiError::Model(format!(
                "property {property_index} was not found"
            )));
        };
        let mut target = None;
        for index in 0..property.primitive_count() {
            let matches = property
                .primitive(index)
                .map(|primitive| primitive.id() == Some(primitive_id))
                .unwrap_or(false);
            if matches {
                target = property.primitive_mut(index);
                break;
            }
        }
        let Some(primitive) = target else {
            return Err(GuiError::Model(format!(
                "primitive {} was not found",
                primitive_id.get()
            )));
        };
        edit(primitive);
        Ok(())
    }

    // ----- grid lines and parameters (RM-101) -----

    /// Native `RectGrid::SetLine` followed by a re-sort of the axis.
    pub fn set_grid_line(
        &mut self,
        axis: usize,
        index: usize,
        value: f64,
    ) -> Result<(), GuiError> {
        check_axis(axis)?;
        if !value.is_finite() {
            return Err(GuiError::InvalidValue(format!(
                "grid line must be finite: {value}"
            )));
        }
        let grid = self.structure_mut().grid_mut();
        if !grid.set_line(axis, index, value) {
            return Err(GuiError::Model(format!(
                "grid line {index} does not exist on axis {axis}"
            )));
        }
        grid.sort(axis);
        Ok(())
    }

    pub fn add_grid_line(&mut self, axis: usize, value: f64) -> Result<usize, GuiError> {
        check_axis(axis)?;
        if !value.is_finite() {
            return Err(GuiError::InvalidValue(format!(
                "grid line must be finite: {value}"
            )));
        }
        let grid = self.structure_mut().grid_mut();
        grid.add_line(axis, value);
        Ok(grid.line_count(axis) - 1)
    }

    pub fn remove_grid_line(&mut self, axis: usize, index: usize) -> Result<(), GuiError> {
        check_axis(axis)?;
        let grid = self.structure_mut().grid_mut();
        if !grid.remove_line(axis, index) {
            return Err(GuiError::Model(format!(
                "grid line {index} does not exist on axis {axis}"
            )));
        }
        Ok(())
    }

    /// Native `RectGrid::IncreaseResolution`.
    pub fn increase_resolution(&mut self, axis: usize, factor: usize) -> Result<(), GuiError> {
        check_axis(axis)?;
        if factor < 1 {
            return Err(GuiError::InvalidValue(
                "the resolution factor must be at least 1".into(),
            ));
        }
        self.structure_mut()
            .grid_mut()
            .increase_resolution(axis, factor);
        Ok(())
    }

    pub fn set_delta_unit(&mut self, unit: f64) -> Result<(), GuiError> {
        if !unit.is_finite() || unit <= 0.0 {
            return Err(GuiError::InvalidValue(format!(
                "the delta unit must be positive: {unit}"
            )));
        }
        self.structure_mut().grid_mut().set_delta_unit(unit);
        Ok(())
    }

    pub fn add_parameter(&mut self, name: &str, value: f64) -> Result<usize, GuiError> {
        validate_parameter(name, value)?;
        // native `LinkParameter` returns the new size
        let count = self.structure_mut().parameters_mut().add(name, value);
        Ok(count.saturating_sub(1))
    }

    pub fn add_linear_parameter(
        &mut self,
        name: &str,
        value: f64,
        min: f64,
        max: f64,
        step: f64,
    ) -> Result<usize, GuiError> {
        validate_parameter(name, value)?;
        if !min.is_finite() || !max.is_finite() || !step.is_finite() || step <= 0.0 {
            return Err(GuiError::InvalidValue(
                "a linear parameter needs a finite range and a positive step".into(),
            ));
        }
        let count = self
            .structure_mut()
            .parameters_mut()
            .add_linear(name, value, min, max, step);
        Ok(count.saturating_sub(1))
    }

    pub fn set_parameter_name(&mut self, index: usize, name: &str) -> Result<(), GuiError> {
        if name.trim().is_empty() {
            return Err(GuiError::InvalidValue("a parameter needs a name".into()));
        }
        let parameters = self.structure_mut().parameters_mut();
        let parameter = parameters
            .get_mut(index)
            .ok_or_else(|| GuiError::Model(format!("parameter {index} was not found")))?;
        parameter.set_name(name);
        Ok(())
    }

    pub fn set_parameter_value(&mut self, index: usize, value: f64) -> Result<(), GuiError> {
        if !value.is_finite() {
            return Err(GuiError::InvalidValue(
                "a parameter needs a finite value".into(),
            ));
        }
        let parameters = self.structure_mut().parameters_mut();
        let parameter = parameters
            .get_mut(index)
            .ok_or_else(|| GuiError::Model(format!("parameter {index} was not found")))?;
        parameter.set_value(value);
        Ok(())
    }

    /// Native `LinearParameter::SetMin`/`SetMax`/`SetStep`; a constant
    /// parameter has no range and rejects the edit.
    pub fn set_parameter_range(
        &mut self,
        index: usize,
        min: f64,
        max: f64,
        step: f64,
    ) -> Result<(), GuiError> {
        if !min.is_finite() || !max.is_finite() || !step.is_finite() || step < 0.0 {
            return Err(GuiError::InvalidValue(
                "a parameter range needs finite values".into(),
            ));
        }
        let parameters = self.structure_mut().parameters_mut();
        let parameter = parameters
            .get_mut(index)
            .ok_or_else(|| GuiError::Model(format!("parameter {index} was not found")))?;
        if !parameter.set_linear_range(min, max, step) {
            return Err(GuiError::InvalidValue(format!(
                "parameter {index} is not linear"
            )));
        }
        Ok(())
    }

    pub fn remove_parameter(&mut self, index: usize) -> Result<(), GuiError> {
        if self.structure_mut().parameters_mut().remove(index) {
            Ok(())
        } else {
            Err(GuiError::Model(format!("parameter {index} was not found")))
        }
    }

    pub fn set_parameter_sweep(&mut self, index: usize, enabled: bool) -> Result<(), GuiError> {
        let parameters = self.structure_mut().parameters_mut();
        let parameter = parameters
            .get_mut(index)
            .ok_or_else(|| GuiError::Model(format!("parameter {index} was not found")))?;
        parameter.set_sweep(enabled);
        Ok(())
    }
}

fn check_axis(axis: usize) -> Result<(), GuiError> {
    if axis < 3 {
        Ok(())
    } else {
        Err(GuiError::InvalidValue(format!("axis {axis} is out of range")))
    }
}

fn validate_parameter(name: &str, value: f64) -> Result<(), GuiError> {
    if name.trim().is_empty() {
        return Err(GuiError::InvalidValue("a parameter needs a name".into()));
    }
    if !value.is_finite() {
        return Err(GuiError::InvalidValue(
            "a parameter needs a finite value".into(),
        ));
    }
    Ok(())
}

fn type_mismatch(type_name: &str, expected: &str) -> GuiError {
    GuiError::InvalidPropertyType(format!("{type_name} is not {expected}"))
}

fn material_mut<'a>(
    property: &'a mut Property,
    type_name: &str,
) -> Result<&'a mut csxcad_core::material::MaterialProperties, GuiError> {
    property
        .material_mut()
        .ok_or_else(|| type_mismatch(type_name, "a material"))
}

fn excitation_mut<'a>(
    property: &'a mut Property,
    type_name: &str,
) -> Result<&'a mut csxcad_core::material::ExcitationProperties, GuiError> {
    property
        .excitation_mut()
        .ok_or_else(|| type_mismatch(type_name, "an excitation"))
}

fn probe_mut<'a>(
    property: &'a mut Property,
    type_name: &str,
) -> Result<&'a mut csxcad_core::material::ProbeProperties, GuiError> {
    property
        .probe_mut()
        .ok_or_else(|| type_mismatch(type_name, "a probe box"))
}

fn dump_box_mut<'a>(
    property: &'a mut Property,
    type_name: &str,
) -> Result<&'a mut csxcad_core::material::DumpBoxProperties, GuiError> {
    property
        .dump_box_mut()
        .ok_or_else(|| type_mismatch(type_name, "a dump box"))
}

/// Parse the comma/semicolon/whitespace separated frequency list used by the
/// `<FD_Samples>` editor.
pub fn parse_frequency_list(text: &str) -> Result<Vec<f64>, GuiError> {
    let mut samples = Vec::new();
    for token in text.split([',', ';', ' ', '\t', '\n']) {
        let token = token.trim();
        if token.is_empty() {
            continue;
        }
        let value: f64 = token
            .parse()
            .map_err(|_| GuiError::InvalidValue(format!("not a number: {token}")))?;
        if !value.is_finite() {
            return Err(GuiError::InvalidValue(format!(
                "frequency must be finite: {token}"
            )));
        }
        samples.push(value);
    }
    Ok(samples)
}

/// Format a frequency list for the editor.
pub fn format_frequency_list(samples: &[f64]) -> String {
    samples
        .iter()
        .map(|value| value.to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use csxcad_core::model::Primitive;

    fn document_with(property_type: PropertyType) -> (Document, PropertyId) {
        let mut document = Document::new();
        let id = document.add_property(property_type).expect("property");
        (document, id)
    }

    #[test]
    fn new_properties_start_empty_and_unmodified_structure_is_clean() {
        let document = Document::new();
        assert!(!document.is_modified());
        assert_eq!(document.structure().property_count(), 0);

        let (mut document, id) = document_with(PropertyType::Material);
        assert!(document.is_modified());
        let property = document.structure().property(id).expect("property");
        assert_eq!(property.name(), "");
        assert_eq!(property.property_type(), PropertyType::Material);
        // a material starts isotropic with the CSXCAD defaults
        assert!(property.material().expect("material").is_isotropic());

        document.set_modified(false);
        assert!(!document.is_modified());
    }

    #[test]
    fn a_viewport_gesture_moves_a_primitive_and_undo_restores_it() {
        let (mut document, property) = document_with(PropertyType::Material);
        let id = document
            .add_primitive(property, PrimitiveType::Box)
            .expect("box");
        let before = document
            .structure()
            .primitive(id)
            .expect("primitive")
            .bounding_box(None, false)
            .expect("bounds");
        document.begin_gesture();
        document
            .move_primitive_by(id, [2.0e-3, 0.0, 0.0])
            .expect("move");
        let after = document
            .structure()
            .primitive(id)
            .expect("primitive")
            .bounding_box(None, false)
            .expect("bounds");
        assert!((after[0] - before[0] - 2.0e-3).abs() < 1.0e-12);
        assert!(document.undo());
        let restored = document
            .structure()
            .primitive(id)
            .expect("primitive")
            .bounding_box(None, false)
            .expect("bounds");
        assert_eq!(restored, before);
    }

    #[test]
    fn a_scale_gesture_resizes_around_the_centre_and_reset_clears_it() {
        let (mut document, property) = document_with(PropertyType::Material);
        let id = document
            .add_primitive(property, PrimitiveType::Box)
            .expect("box");
        document
            .edit_primitive(id, |primitive| {
                let shape = primitive.box_shape_mut().expect("box shape");
                for index in 0..6 {
                    let value = if index % 2 == 0 { 0.0 } else { 10.0e-3 };
                    shape.set_coordinate(index, value);
                }
            })
            .expect("geometry");
        document.begin_gesture();
        document.scale_primitive_by(id, 2.0).expect("scale");
        let transform = document
            .structure()
            .primitive(id)
            .expect("primitive")
            .transform()
            .expect("transform");
        let low = transform.transform([0.0, 0.0, 0.0]);
        let high = transform.transform([10.0e-3, 10.0e-3, 10.0e-3]);
        assert!((low[0] + 5.0e-3).abs() < 1.0e-12);
        assert!((high[0] - 15.0e-3).abs() < 1.0e-12);
        document
            .reset_primitive_transform(id)
            .expect("reset transform");
        assert!(!document
            .structure()
            .primitive(id)
            .expect("primitive")
            .has_transform());
    }

    #[test]
    fn a_viewport_rotation_is_one_undo_step() {
        let (mut document, property) = document_with(PropertyType::Material);
        let id = document
            .add_primitive(property, PrimitiveType::Box)
            .expect("box");
        assert!(!document
            .structure()
            .primitive(id)
            .expect("primitive")
            .has_transform());
        document.begin_gesture();
        document
            .rotate_primitive_by(id, [0.0, 0.0, 1.0], 0.5)
            .expect("rotate");
        assert!(document
            .structure()
            .primitive(id)
            .expect("primitive")
            .has_transform());
        assert!(document.undo());
        assert!(!document
            .structure()
            .primitive(id)
            .expect("primitive")
            .has_transform());
    }

    #[test]
    fn only_native_creatable_property_types_are_accepted() {        let mut document = Document::new();
        assert!(document.add_property(PropertyType::Any).is_err());
        assert!(document.add_property(PropertyType::Unknown).is_err());
        for property_type in CREATABLE_PROPERTY_TYPES {
            assert!(document.add_property(property_type).is_ok());
        }
        assert!(document
            .delete_property(PropertyId::from_u64(999))
            .is_err());
    }

    #[test]
    fn name_visibility_and_colors_round_trip_through_xml() {
        let (mut document, id) = document_with(PropertyType::Metal);
        document
            .apply_property_edit(id, PropertyEdit::Name("shield".into()))
            .expect("name");
        document
            .apply_property_edit(id, PropertyEdit::Visible(false))
            .expect("visible");
        document
            .apply_property_edit(
                id,
                PropertyEdit::FillColor(Some(PropertyColor::new(1, 2, 3, 200))),
            )
            .expect("fill color");
        document
            .apply_property_edit(
                id,
                PropertyEdit::EdgeColor(Some(PropertyColor::new(4, 5, 6, 255))),
            )
            .expect("edge color");

        let xml = document.structure().to_xml_string().expect("xml");
        let reloaded = Structure::from_xml_str(&xml).expect("reload");
        let property = reloaded.property(id).expect("property");
        assert_eq!(property.name(), "shield");
        assert!(!property.is_visible());
        assert_eq!(
            property.fill_color(),
            Some(PropertyColor::new(1, 2, 3, 200))
        );
        assert_eq!(
            property.edge_color(),
            Some(PropertyColor::new(4, 5, 6, 255))
        );
    }

    #[test]
    fn material_values_apply_per_component_and_reject_other_types() {
        let (mut document, id) = document_with(PropertyType::Material);
        for (field, base) in [
            (MaterialField::Epsilon, 2.0),
            (MaterialField::Mue, 3.0),
            (MaterialField::Kappa, 4.0),
            (MaterialField::Sigma, 5.0),
        ] {
            for component in 0..3 {
                document
                    .apply_property_edit(
                        id,
                        PropertyEdit::MaterialValue {
                            field,
                            component,
                            value: base + component as f64,
                        },
                    )
                    .expect("material value");
            }
        }
        document
            .apply_property_edit(id, PropertyEdit::MaterialIsotropic(false))
            .expect("isotropy");

        let xml = document.structure().to_xml_string().expect("xml");
        let reloaded = Structure::from_xml_str(&xml).expect("reload");
        let material = reloaded
            .property(id)
            .expect("property")
            .material()
            .expect("material");
        assert!(!material.is_isotropic());
        for (field, base) in [
            (MaterialField::Epsilon, 2.0),
            (MaterialField::Mue, 3.0),
            (MaterialField::Kappa, 4.0),
            (MaterialField::Sigma, 5.0),
        ] {
            for component in 0..3 {
                let value = material
                    .value(field, component, None)
                    .expect("value")
                    .expect("defined component");
                assert!((value - (base + component as f64)).abs() < 1e-12);
            }
        }

        // a metal has no material block: the edit must fail, not no-op
        let (mut metal, metal_id) = document_with(PropertyType::Metal);
        assert!(metal
            .apply_property_edit(
                metal_id,
                PropertyEdit::MaterialValue {
                    field: MaterialField::Epsilon,
                    component: 0,
                    value: 1.0,
                },
            )
            .is_err());
        assert!(metal.is_modified());
    }

    #[test]
    fn excitation_edits_survive_a_reload() {
        let (mut document, id) = document_with(PropertyType::Excitation);
        document
            .apply_property_edit(id, PropertyEdit::ExcitationType(0))
            .expect("type");
        document
            .apply_property_edit(id, PropertyEdit::ExcitationNumber(3))
            .expect("number");
        document
            .apply_property_edit(id, PropertyEdit::ExcitationEnabled(false))
            .expect("enabled");
        document
            .apply_property_edit(id, PropertyEdit::ExcitationFrequency("2.4e9".into()))
            .expect("frequency");
        document
            .apply_property_edit(id, PropertyEdit::ExcitationDelay("1e-9".into()))
            .expect("delay");
        for component in 0..3 {
            document
                .apply_property_edit(
                    id,
                    PropertyEdit::ExcitationTerm {
                        component,
                        term: format!("{}", component + 1),
                    },
                )
                .expect("excitation term");
            document
                .apply_property_edit(
                    id,
                    PropertyEdit::PropagationTerm {
                        component,
                        term: format!("{}", 1 - component as i32),
                    },
                )
                .expect("propagation term");
        }
        // Native `set_value_string` consumes expression parse errors and keeps
        // the term, but an empty expression is rejected (`""` -> -1).
        assert!(document
            .apply_property_edit(
                id,
                PropertyEdit::ExcitationTerm {
                    component: 0,
                    term: String::new(),
                },
            )
            .is_err());
        assert!(document
            .apply_property_edit(
                id,
                PropertyEdit::PropagationTerm {
                    component: 0,
                    term: String::new(),
                },
            )
            .is_err());

        let xml = document.structure().to_xml_string().expect("xml");
        let reloaded = Structure::from_xml_str(&xml).expect("reload");
        let excitation = reloaded
            .property(id)
            .expect("property")
            .excitation()
            .expect("excitation");
        assert_eq!(excitation.excitation_type(), 0);
        assert_eq!(excitation.number(), 3);
        assert!(!excitation.is_enabled());
        assert!((excitation.frequency(None).expect("frequency") - 2.4e9).abs() < 1.0);
        assert!((excitation.delay(None).expect("delay") - 1e-9).abs() < 1e-21);
        for component in 0..3 {
            assert_eq!(
                excitation.excitation(component, None).expect("excitation"),
                Some((component + 1) as f64)
            );
            assert_eq!(
                excitation
                    .propagation_direction(component, None)
                    .expect("propagation"),
                Some((1 - component as i32) as f64)
            );
        }
    }

    #[test]
    fn probe_and_dump_edits_survive_a_reload() {
        let (mut document, id) = document_with(PropertyType::DumpBox);
        document
            .apply_property_edit(id, PropertyEdit::DumpNumber(2))
            .expect("number");
        document
            .apply_property_edit(id, PropertyEdit::ProbeNormalDirection(2))
            .expect("normal");
        document
            .apply_property_edit(id, PropertyEdit::ProbeWeight(0.5))
            .expect("weight");
        document
            .apply_property_edit(id, PropertyEdit::ProbeStartTime(1e-9))
            .expect("start time");
        document
            .apply_property_edit(id, PropertyEdit::ProbeStopTime(2e-9))
            .expect("stop time");
        document
            .apply_property_edit(id, PropertyEdit::ProbeOversampling(4))
            .expect("probe oversampling");
        document
            .apply_property_edit(id, PropertyEdit::DumpType(10))
            .expect("dump type");
        document
            .apply_property_edit(id, PropertyEdit::DumpMode(1))
            .expect("dump mode");
        document
            .apply_property_edit(id, PropertyEdit::DumpFileType(1))
            .expect("file type");
        document
            .apply_property_edit(id, PropertyEdit::DumpMultiGridLevel(1))
            .expect("multigrid");
        document
            .apply_property_edit(id, PropertyEdit::DumpSubSamplingEnabled(true))
            .expect("sub sampling enabled");
        document
            .apply_property_edit(
                id,
                PropertyEdit::DumpSubSampling {
                    direction: 1,
                    value: 3,
                },
            )
            .expect("sub sampling");
        document
            .apply_property_edit(id, PropertyEdit::DumpOversampling(4))
            .expect("oversampling");
        document
            .apply_property_edit(id, PropertyEdit::DumpFdSamples(vec![1e9, 2e9]))
            .expect("fd samples");
        document
            .apply_property_edit(id, PropertyEdit::DumpSarMethod("1g".into()))
            .expect("sar method");

        let xml = document.structure().to_xml_string().expect("xml");
        let reloaded = Structure::from_xml_str(&xml).expect("reload");
        let property = reloaded.property(id).expect("property");
        let probe = property.probe().expect("probe");
        assert_eq!(probe.normal_direction(), 2);
        assert!((probe.weight() - 0.5).abs() < 1e-12);
        // native `SetStartTime(float)` stores the value through an f32
        let start_error = (probe.start_time() - 1e-9).abs();
        assert!(start_error < 1e-15, "start time error {start_error:e}");
        assert!((probe.stop_time() - 2e-9).abs() < 1e-15);
        assert_eq!(probe.oversampling(), 4);
        let dump = property.dump_box().expect("dump box");
        // the dump number lives on the probe part of the dump box
        assert_eq!(probe.number(), 2);
        assert_eq!(dump.dump_type(), 10);
        assert_eq!(dump.dump_mode(), 1);
        assert_eq!(dump.file_type(), 1);
        assert_eq!(dump.multi_grid_level(), 1);
        assert!(dump.sub_sampling_enabled());
        assert_eq!(dump.sub_sampling(), [1, 3, 1]);
        // the dump box shares the probe oversampling field
        assert_eq!(probe.oversampling(), 4);
        assert_eq!(dump.fd_samples(), &[1e9, 2e9]);
        assert_eq!(dump.sar_method(), "1g");
    }

    #[test]
    fn resolution_factor_edits_survive_a_reload() {
        let (mut document, id) = document_with(PropertyType::ResBox);
        document
            .apply_property_edit(id, PropertyEdit::ResolutionFactor(4))
            .expect("factor");
        let xml = document.structure().to_xml_string().expect("xml");
        let reloaded = Structure::from_xml_str(&xml).expect("reload");
        assert_eq!(
            reloaded
                .property(id)
                .expect("property")
                .resolution_box()
                .expect("resolution box")
                .factor(),
            4
        );
        // a material has no resolution box
        let (mut material, material_id) = document_with(PropertyType::Material);
        assert!(material
            .apply_property_edit(material_id, PropertyEdit::ResolutionFactor(2))
            .is_err());
    }

    #[test]
    fn deleting_a_property_removes_its_primitives() {
        let (mut document, id) = document_with(PropertyType::Material);
        let primitive_id = document
            .structure_mut()
            .add_primitive(id, Primitive::box_primitive([0.0; 3], [1.0; 3]))
            .expect("primitive");
        assert!(document.structure().primitive(primitive_id).is_some());
        document.delete_property(id).expect("delete");
        assert_eq!(document.structure().property_count(), 0);
        assert!(document.structure().property(id).is_none());
        assert!(document.structure().primitive(primitive_id).is_none());
    }

    #[test]
    fn frequency_lists_parse_and_format() {
        assert_eq!(
            parse_frequency_list("1e9, 2e9;3e9 4e9").expect("parse"),
            vec![1e9, 2e9, 3e9, 4e9]
        );
        assert_eq!(parse_frequency_list("  ").expect("empty"), Vec::<f64>::new());
        assert!(parse_frequency_list("1e9, abc").is_err());
        assert!(parse_frequency_list("inf").is_err());
        assert_eq!(format_frequency_list(&[1e9, 2e9]), "1000000000, 2000000000");
    }
}

#[cfg(test)]
mod edit_tests {
    use super::*;
    use csxcad_core::model::PrimitiveShape;

    fn round_trip(document: &Document) -> Structure {
        let xml = document.structure().to_xml_string().expect("serialize");
        Structure::from_xml_str(&xml).expect("reload")
    }

    #[test]
    fn primitives_can_be_added_edited_and_deleted() {
        let (mut document, property_id) = {
            let mut document = Document::new();
            let id = document.add_property(PropertyType::Material).expect("property");
            (document, id)
        };
        let primitive_id = document
            .add_primitive(property_id, PrimitiveType::Box)
            .expect("primitive");
        // only the native menu types are creatable
        assert!(document
            .add_primitive(property_id, PrimitiveType::Point)
            .is_err());
        assert!(document
            .add_primitive(PropertyId::from_u64(999), PrimitiveType::Box)
            .is_err());

        document
            .edit_primitive(primitive_id, |primitive| {
                primitive.set_priority(4);
                if let PrimitiveShape::Box(shape) = primitive.shape_mut() {
                    // 0/1: start/stop x, 2/3: y, 4/5: z
                    assert!(shape.set_coordinate(0, -1.0));
                    assert!(shape.set_coordinate(1, 1.0));
                    assert!(shape.set_coordinate(2, -2.0));
                    assert!(shape.set_coordinate(3, 2.0));
                    assert!(shape.set_coordinate(4, -3.0));
                    assert!(shape.set_coordinate(5, 3.0));
                    assert!(!shape.set_coordinate(6, 0.0));
                }
            })
            .expect("edit");

        let structure = round_trip(&document);
        let primitive = structure.primitive(primitive_id).expect("primitive");
        assert_eq!(primitive.priority(), 4);
        let bounds = primitive.bounding_box(None, true).expect("bounds");
        assert_eq!(bounds, [-1.0, 1.0, -2.0, 2.0, -3.0, 3.0]);

        document.delete_primitive(primitive_id).expect("delete");
        assert!(document.structure().primitive(primitive_id).is_none());
        assert!(document.delete_primitive(primitive_id).is_err());
    }

    #[test]
    fn every_creatable_primitive_type_is_built() {
        let mut document = Document::new();
        let property_id = document.add_property(PropertyType::Metal).expect("property");
        for kind in [
            PrimitiveType::Box,
            PrimitiveType::MultiBox,
            PrimitiveType::Sphere,
            PrimitiveType::Cylinder,
            PrimitiveType::Polygon,
            PrimitiveType::UserDefined,
        ] {
            let id = document.add_primitive(property_id, kind).expect("primitive");
            assert_eq!(
                document
                    .structure()
                    .primitive(id)
                    .expect("primitive")
                    .primitive_type(),
                kind
            );
        }
        assert_eq!(document.structure().property(property_id).unwrap().primitive_count(), 6);
    }

    #[test]
    fn grid_edits_round_trip() {
        let mut document = Document::new();
        for axis in 0..3 {
            document.add_grid_line(axis, 0.0).expect("line");
            document.add_grid_line(axis, 1.0).expect("line");
        }
        document.set_grid_line(0, 1, 2.0).expect("set line");
        let added = document.add_grid_line(1, -1.0).expect("add line");
        // the grid re-sorts after an edit
        let lines = document
            .structure_mut()
            .grid_mut()
            .lines(1)
            .expect("lines")
            .to_vec();
        assert_eq!(lines, vec![-1.0, 0.0, 1.0]);
        assert_eq!(added, 2);
        document.remove_grid_line(2, 0).expect("remove line");
        document.set_delta_unit(0.001).expect("delta unit");
        document.increase_resolution(0, 2).expect("resolution");

        assert!(document.set_grid_line(3, 0, 1.0).is_err());
        assert!(document.set_grid_line(0, 9, 1.0).is_err());
        assert!(document.remove_grid_line(0, 99).is_err());
        assert!(document.set_delta_unit(0.0).is_err());
        assert!(document.increase_resolution(0, 0).is_err());

        let mut structure = round_trip(&document);
        assert_eq!(structure.grid().delta_unit(), 0.001);
        let mut expected = document.structure().clone();
        for axis in 0..3 {
            assert_eq!(
                structure.grid_mut().lines(axis).map(|lines| lines.to_vec()),
                expected.grid_mut().lines(axis).map(|lines| lines.to_vec()),
                "axis {axis}"
            );
        }
    }

    #[test]
    fn parameter_edits_round_trip() {
        let mut document = Document::new();
        let constant = document.add_parameter("height", 2.0).expect("parameter");
        let linear = document
            .add_linear_parameter("width", 1.0, 0.5, 3.0, 0.5)
            .expect("parameter");
        assert!(document.add_parameter("", 1.0).is_err());
        assert!(document.add_parameter("nan", f64::NAN).is_err());
        assert!(document
            .add_linear_parameter("bad", 1.0, 0.0, 1.0, 0.0)
            .is_err());

        document
            .set_parameter_name(constant, "height2")
            .expect("name");
        document.set_parameter_value(constant, 4.0).expect("value");
        document
            .set_parameter_range(linear, 0.0, 5.0, 1.0)
            .expect("range");
        document.set_parameter_sweep(linear, false).expect("sweep");
        // a constant parameter has no range
        assert!(document.set_parameter_range(constant, 0.0, 5.0, 1.0).is_err());
        assert!(document.set_parameter_name(99, "missing").is_err());

        let mut structure = round_trip(&document);
        let parameters = structure.parameters();
        let first = parameters.get(constant).expect("parameter");
        assert_eq!(first.name(), "height2");
        assert_eq!(first.value(), 4.0);
        assert!(!first.is_linear());
        let second = parameters.get(linear).expect("parameter");
        assert_eq!(second.name(), "width");
        assert_eq!(second.linear_range(), Some((0.0, 5.0, 1.0)));
        assert!(!second.get_sweep());

        document.remove_parameter(constant).expect("remove");
        // the remaining parameter shifts down, so only index 1 is out of range
        assert!(document.remove_parameter(1).is_err());
        assert_eq!(round_trip(&document).parameters().len(), 1);
    }
}

#[cfg(test)]
mod history_tests {
    use super::*;
    use csxcad_core::model::{PrimitiveShape, PrimitiveType};

    #[test]
    fn undo_and_redo_walk_the_edit_history() {
        let mut document = Document::new();
        assert!(!document.can_undo());
        assert!(!document.can_redo());
        assert!(!document.undo());
        assert!(!document.redo());

        let id = document.add_property(PropertyType::Material).expect("property");
        document
            .apply_property_edit(id, PropertyEdit::Name("substrate".into()))
            .expect("name");
        assert_eq!(
            document.structure().property(id).expect("property").name(),
            "substrate"
        );

        assert!(document.undo());
        assert_eq!(
            document.structure().property(id).expect("property").name(),
            ""
        );
        assert!(document.undo());
        assert!(document.structure().property(id).is_none());
        assert!(!document.can_undo());

        assert!(document.redo());
        assert!(document.structure().property(id).is_some());
        assert!(document.redo());
        assert_eq!(
            document.structure().property(id).expect("property").name(),
            "substrate"
        );
        assert!(!document.redo());
    }

    #[test]
    fn a_new_edit_clears_the_redo_stack() {
        let mut document = Document::new();
        let id = document.add_property(PropertyType::Metal).expect("property");
        document
            .apply_property_edit(id, PropertyEdit::Name("a".into()))
            .expect("name");
        document.undo();
        assert!(document.can_redo());
        document
            .apply_property_edit(id, PropertyEdit::Name("b".into()))
            .expect("name");
        assert!(!document.can_redo());
        assert_eq!(
            document.structure().property(id).expect("property").name(),
            "b"
        );
    }

    #[test]
    fn primitive_grid_and_parameter_edits_are_undoable() {
        let mut document = Document::new();
        let property = document.add_property(PropertyType::Material).expect("property");
        let primitive = document
            .add_primitive(property, PrimitiveType::Box)
            .expect("primitive");
        document
            .edit_primitive(primitive, |primitive| {
                if let PrimitiveShape::Box(shape) = primitive.shape_mut() {
                    shape.set_coordinate(1, 1.0);
                }
            })
            .expect("edit");
        document.add_grid_line(0, 1.0).expect("line");
        document.add_parameter("height", 2.0).expect("parameter");

        // undo in reverse order: parameter, line, primitive edit, primitive,
        // property
        assert_eq!(document.structure().parameters().len(), 1);
        assert!(document.undo());
        assert_eq!(document.structure().parameters().len(), 0);
        assert!(document.undo());
        let mut grid = document.structure().grid().clone();
        assert_eq!(grid.line_count(0), 0);
        assert!(document.undo());
        let primitive_now = document.structure().primitive(primitive).expect("primitive");
        let bounds = primitive_now.bounding_box(None, true).expect("bounds");
        assert_eq!(bounds[1], 0.0);
        assert!(document.undo());
        assert!(document.structure().primitive(primitive).is_none());
        assert!(document.undo());
        assert!(document.structure().property(property).is_none());
        assert!(!document.can_undo());
    }

    #[test]
    fn the_history_is_bounded() {
        let mut document = Document::new();
        let id = document.add_property(PropertyType::Material).expect("property");
        for step in 0..(MAX_UNDO_STEPS + 5) {
            document
                .apply_property_edit(id, PropertyEdit::Name(format!("n{step}")))
                .expect("name");
        }
        let mut undone = 0;
        while document.undo() {
            undone += 1;
        }
        assert_eq!(undone, MAX_UNDO_STEPS);
        // the oldest snapshots fell off, the property itself survives
        assert!(document.structure().property(id).is_some());
    }

    #[test]
    fn clearing_the_history_keeps_the_current_state() {
        let mut document = Document::new();
        let id = document.add_property(PropertyType::Material).expect("property");
        document.clear_history();
        assert!(!document.can_undo());
        assert!(!document.can_redo());
        assert!(document.structure().property(id).is_some());
    }
}
