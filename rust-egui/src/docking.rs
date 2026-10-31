//! Optional drag-docking layout for the two docks and the viewport.
//!
//! ADR 0010: `egui_tiles 0.18` is the only docking crate compatible with the
//! workspace's `egui 0.37`. The module owns the tile tree, its persistence and
//! the pane visibility state; the application supplies the pane contents, so
//! the widgets are the same ones the fixed layout uses.

use egui_tiles::{Behavior, Container, Tile, TileId, Tiles, Tree, UiResponse};
use egui::WidgetText;

/// The three panes that can be docked.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum DockPane {
    Structure,
    Viewport,
    Inspector,
}

impl DockPane {
    pub fn title(self) -> &'static str {
        match self {
            DockPane::Structure => "Structure",
            DockPane::Viewport => "Viewport",
            DockPane::Inspector => "Inspector",
        }
    }
}

const TREE_ID: &str = "openems_dock";

/// The persisted docking layout.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct DockLayout {
    tree: Tree<DockPane>,
}

impl Default for DockLayout {
    fn default() -> Self {
        let mut layout = Self {
            tree: Tree::new_horizontal(
                egui::Id::new(TREE_ID),
                vec![DockPane::Structure, DockPane::Viewport, DockPane::Inspector],
            ),
        };
        layout.set_default_shares();
        layout
    }
}

impl DockLayout {
    /// The tile that holds a pane (if it is still in the tree).
    pub fn pane_tile(&self, pane: DockPane) -> Option<TileId> {
        self.tree.tiles.iter().find_map(|(id, tile)| match tile {
            Tile::Pane(current) if *current == pane => Some(*id),
            _ => None,
        })
    }

    /// Default shares mirror the fixed layout: narrow docks, wide viewport.
    fn set_default_shares(&mut self) {
        let Some(root) = self.tree.root() else {
            return;
        };
        let structure = self.pane_tile(DockPane::Structure);
        let viewport = self.pane_tile(DockPane::Viewport);
        let inspector = self.pane_tile(DockPane::Inspector);
        let (Some(structure), Some(viewport), Some(inspector)) =
            (structure, viewport, inspector)
        else {
            return;
        };
        if let Some(Tile::Container(Container::Linear(linear))) = self.tree.tiles.get_mut(root) {
            linear.shares.set_share(structure, 0.22);
            linear.shares.set_share(viewport, 0.56);
            linear.shares.set_share(inspector, 0.22);
        }
    }

    /// A docked arrangement for the persistence/screenshot evidence:
    /// Structure and Inspector share one tab group on the left, the viewport
    /// stays on the right. This is the shape a drag produces when the
    /// Inspector is dropped onto the Structure tab bar.
    pub fn demo_docked() -> Self {
        let mut tiles = Tiles::default();
        let structure = tiles.insert_pane(DockPane::Structure);
        let inspector = tiles.insert_pane(DockPane::Inspector);
        let viewport = tiles.insert_pane(DockPane::Viewport);
        let tabs = tiles.insert_tab_tile(vec![structure, inspector]);
        let root = tiles.insert_horizontal_tile(vec![tabs, viewport]);
        let mut layout = Self {
            tree: Tree::new(egui::Id::new(TREE_ID), root, tiles),
        };
        if let Some(Tile::Container(Container::Linear(linear))) = layout.tree.tiles.get_mut(root) {
            linear.shares.set_share(tabs, 0.30);
            linear.shares.set_share(viewport, 0.70);
        }
        layout
    }

    /// Show or hide a pane without removing it from the tree (its shares are
    /// kept, so re-showing restores the size).
    pub fn set_visible(&mut self, pane: DockPane, visible: bool) {
        if let Some(tile) = self.pane_tile(pane) {
            self.tree.set_visible(tile, visible);
        }
    }

    pub fn is_visible(&self, pane: DockPane) -> bool {
        self.pane_tile(pane)
            .map(|tile| self.tree.is_visible(tile))
            .unwrap_or(false)
    }

    /// Serialize to the settings file. The tree stores tile ids, shares and
    /// the dock/undock structure the user created.
    pub fn to_json(&self) -> String {
        serde_json::to_string(&self.tree).unwrap_or_default()
    }

    /// Restore a persisted tree. A layout that lost one of the three panes is
    /// rejected, so a corrupt setting cannot silently hide a panel.
    pub fn from_json(json: &str) -> Option<Self> {
        let tree: Tree<DockPane> = serde_json::from_str(json).ok()?;
        let layout = Self { tree };
        for pane in [DockPane::Structure, DockPane::Viewport, DockPane::Inspector] {
            layout.pane_tile(pane)?;
        }
        Some(layout)
    }

    /// Draw the tree and dispatch each pane to `content`.
    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        content: &mut dyn FnMut(DockPane, &mut egui::Ui),
    ) {
        let mut viewer = Viewer { content };
        self.tree.ui(&mut viewer, ui);
    }
}

struct Viewer<'a> {
    content: &'a mut dyn FnMut(DockPane, &mut egui::Ui),
}

impl Behavior<DockPane> for Viewer<'_> {
    fn pane_ui(
        &mut self,
        ui: &mut egui::Ui,
        _tile_id: TileId,
        pane: &mut DockPane,
    ) -> UiResponse {
        // A visible grip is the drag affordance for a pane that lives directly
        // in a linear layout (egui_tiles only draws tabs inside a Tabs
        // container). Starting a drag on it hands the tile to the tree.
        let palette = crate::theme::palette(ui.ctx());
        let handle = ui
            .horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let (rect, response) =
                        ui.allocate_exact_size(egui::vec2(16.0, 14.0), egui::Sense::drag());
                    let painter = ui.painter();
                    for row in 0..3 {
                        let y = rect.center().y - 3.5 + row as f32 * 3.5;
                        painter.line_segment(
                            [
                                egui::pos2(rect.left() + 3.0, y),
                                egui::pos2(rect.right() - 3.0, y),
                            ],
                            egui::Stroke::new(1.0, palette.text_tertiary),
                        );
                    }
                    response.on_hover_text("Drag to dock this pane")
                })
                .inner
            })
            .inner;
        if handle.drag_started() {
            return UiResponse::DragStarted;
        }
        (self.content)(*pane, ui);
        UiResponse::None
    }

    fn tab_title_for_pane(&mut self, pane: &DockPane) -> WidgetText {
        pane.title().into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_layout_has_all_panes_and_expected_visibility() {
        let layout = DockLayout::default();
        for pane in [DockPane::Structure, DockPane::Viewport, DockPane::Inspector] {
            assert!(layout.pane_tile(pane).is_some(), "{pane:?}");
            assert!(layout.is_visible(pane), "{pane:?}");
        }
    }

    #[test]
    fn visibility_round_trips_without_losing_the_layout() {
        let mut layout = DockLayout::default();
        layout.set_visible(DockPane::Inspector, false);
        assert!(!layout.is_visible(DockPane::Inspector));
        assert!(layout.is_visible(DockPane::Structure));
        layout.set_visible(DockPane::Inspector, true);
        assert!(layout.is_visible(DockPane::Inspector));
    }

    #[test]
    fn json_round_trip_preserves_panes_visibility_and_shares() {
        let mut layout = DockLayout::default();
        layout.set_visible(DockPane::Inspector, false);
        let json = layout.to_json();
        let restored = DockLayout::from_json(&json).expect("valid layout");
        assert!(restored.is_visible(DockPane::Structure));
        assert!(restored.is_visible(DockPane::Viewport));
        assert!(!restored.is_visible(DockPane::Inspector));
        assert_eq!(
            restored.to_json().len(),
            json.len(),
            "round trip must not grow the tree"
        );
    }

    #[test]
    fn corrupt_or_incomplete_json_falls_back_to_none() {
        assert!(DockLayout::from_json("not json").is_none());
        assert!(DockLayout::from_json("{}").is_none());
    }

    #[test]
    fn docked_layout_groups_structure_and_inspector_in_tabs() {
        let layout = DockLayout::demo_docked();
        let json = layout.to_json();
        assert!(json.contains("Tabs"), "{json}");
        for pane in [DockPane::Structure, DockPane::Viewport, DockPane::Inspector] {
            assert!(layout.pane_tile(pane).is_some(), "{pane:?}");
        }
        let restored = DockLayout::from_json(&json).expect("round trip");
        assert!(restored.to_json().contains("Tabs"));
        println!("DOCKED_JSON={json}");
    }
}
