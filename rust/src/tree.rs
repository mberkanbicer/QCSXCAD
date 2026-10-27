//! Structure tree model.
//!
//! The native tree widget (`QCSTreeWidget`) labels a property
//! `<TypeXMLString>::<name>` and a primitive `<TypeName> - ID: <id>`, groups
//! the primitives under their property and shows a visibility bulb per
//! property. This module produces the same rows without any UI toolkit so the
//! labels and grouping can be tested headlessly; the egui widget only renders
//! them.

use csxcad_core::model::{Primitive, PrimitiveId, Property, PropertyId, Structure};

/// A selectable tree node.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TreeItem {
    Property(PropertyId),
    Primitive(PrimitiveId),
}

impl TreeItem {
    pub fn property_id(self) -> Option<PropertyId> {
        match self {
            TreeItem::Property(id) => Some(id),
            TreeItem::Primitive(_) => None,
        }
    }

    pub fn primitive_id(self) -> Option<PrimitiveId> {
        match self {
            TreeItem::Primitive(id) => Some(id),
            TreeItem::Property(_) => None,
        }
    }
}

/// One rendered tree row.
#[derive(Clone, Debug, PartialEq)]
pub struct TreeRow {
    pub item: TreeItem,
    /// Native label text.
    pub label: String,
    /// 0 for a property, 1 for one of its primitives.
    pub depth: usize,
    /// Property visibility (`CSPrimitives` inherit it).
    pub visible: bool,
    /// Property fill color, shown as a swatch next to the label.
    pub color: Option<[u8; 4]>,
}

/// Native `QCSTreeWidget::AddPropItem`: `"<TypeXMLString>::<name>"`.
pub fn property_label(property: &Property) -> String {
    let type_name = property.property_type().xml_string().unwrap_or("Unknown");
    format!("{type_name}::{}", property.name())
}

/// Native `QCSTreeWidget::AddPrimItem`: `"<TypeName> - ID: <id>"`.
pub fn primitive_label(primitive: &Primitive) -> String {
    let id = primitive.id().map(|id| id.get().to_string()).unwrap_or_default();
    format!("{} - ID: {}", primitive.primitive_type().type_name(), id)
}

/// Flatten a structure into tree rows.
///
/// Primitive rows are only produced for properties reported as expanded, which
/// mirrors the collapsible tree item state. Rows keep the native order:
/// properties in model order, primitives in property order.
pub fn tree_rows(
    structure: &Structure,
    is_expanded: impl Fn(PropertyId) -> bool,
) -> Vec<TreeRow> {
    let mut rows = Vec::new();
    for index in 0..structure.property_count() {
        let Some(property) = structure.property_by_index(index) else {
            continue;
        };
        let Some(property_id) = property.id() else {
            continue;
        };
        rows.push(TreeRow {
            item: TreeItem::Property(property_id),
            label: property_label(property),
            depth: 0,
            visible: property.is_visible(),
            color: property
                .fill_color()
                .map(|color| color.components()),
        });
        if !is_expanded(property_id) {
            continue;
        }
        for primitive in property.primitives() {
            let Some(primitive_id) = primitive.id() else {
                continue;
            };
            rows.push(TreeRow {
                item: TreeItem::Primitive(primitive_id),
                label: primitive_label(primitive),
                depth: 1,
                visible: property.is_visible(),
                color: None,
            });
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use csxcad_core::model::{Primitive, Property, PropertyType};

    fn box_structure() -> (Structure, PropertyId) {
        let mut structure = Structure::new();
        let mut property = Property::new("substrate", PropertyType::Material);
        property.add_primitive(Primitive::box_primitive([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]));
        property.add_primitive(Primitive::box_primitive([1.0, 0.0, 0.0], [2.0, 1.0, 1.0]));
        let id = structure.add_property(property);
        (structure, id)
    }

    #[test]
    fn properties_and_primitives_use_the_native_labels() {
        let (structure, property_id) = box_structure();
        let rows = tree_rows(&structure, |id| id == property_id);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].label, "Material::substrate");
        assert_eq!(rows[0].depth, 0);
        assert_eq!(rows[1].label, "Box - ID: 1");
        assert_eq!(rows[1].depth, 1);
        assert_eq!(rows[2].label, "Box - ID: 2");
        assert_eq!(rows[0].item.property_id(), Some(property_id));
        assert_eq!(rows[1].item.primitive_id().map(|id| id.get()), Some(1));
    }

    #[test]
    fn collapsed_properties_hide_their_primitives() {
        let (structure, _) = box_structure();
        let rows = tree_rows(&structure, |_| false);
        assert_eq!(rows.len(), 1);
        assert!(rows.iter().all(|row| row.depth == 0));
    }

    #[test]
    fn the_visibility_and_color_come_from_the_property() {
        let (mut structure, property_id) = box_structure();
        let property = structure.property_mut(property_id).expect("property");
        property.set_visible(false);
        property.set_fill_color(Some(csxcad_core::model::PropertyColor::new(10, 20, 30, 255)));
        let rows = tree_rows(&structure, |_| true);
        for row in &rows {
            assert!(!row.visible);
        }
        assert_eq!(rows[0].color, Some([10, 20, 30, 255]));
        assert_eq!(rows[1].color, None);
    }

    #[test]
    fn empty_names_and_unassigned_ids_stay_printable() {
        let mut property = Property::new("", PropertyType::Excitation);
        property.add_primitive(Primitive::box_primitive([0.0; 3], [1.0; 3]));
        assert_eq!(property_label(&property), "Excitation::");
        // a detached primitive has no id yet
        assert_eq!(primitive_label(&property.primitives()[0]), "Box - ID: ");
    }
}
