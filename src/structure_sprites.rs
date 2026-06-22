use bevy::prelude::{AssetServer, Component, Handle, Image, Resource, Sprite};

use crate::nanobot::PlannedKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StructureVisualState {
    Planned,
    Completed,
}

#[derive(Debug, Clone, Copy, Component, PartialEq, Eq)]
pub struct StructureVisual {
    pub kind: PlannedKind,
    pub state: StructureVisualState,
}

impl StructureVisual {
    pub const fn planned(kind: PlannedKind) -> Self {
        Self {
            kind,
            state: StructureVisualState::Planned,
        }
    }

    pub const fn completed(kind: PlannedKind) -> Self {
        Self {
            kind,
            state: StructureVisualState::Completed,
        }
    }
}

#[derive(Debug, Clone, Resource)]
pub struct StructureSprites {
    pub planned_source_stockpile: Handle<Image>,
    pub source_stockpile: Handle<Image>,
    pub planned_sink_stockpile: Handle<Image>,
    pub sink_stockpile: Handle<Image>,
    pub planned_charger: Handle<Image>,
    pub charger: Handle<Image>,
    pub planned_production_facility: Handle<Image>,
    pub production_facility: Handle<Image>,
}

impl StructureSprites {
    pub fn load(asset_server: &AssetServer) -> Self {
        Self {
            planned_source_stockpile: asset_server.load("planned_source_stockpile.png"),
            source_stockpile: asset_server.load("source_stockpile.png"),
            planned_sink_stockpile: asset_server.load("planned_sink_stockpile.png"),
            sink_stockpile: asset_server.load("sink_stockpile.png"),
            planned_charger: asset_server.load("planned_charger.png"),
            charger: asset_server.load("charger.png"),
            planned_production_facility: asset_server.load("planned_production_facility.png"),
            production_facility: asset_server.load("production_facility.png"),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn from_handles(
        planned_source_stockpile: Handle<Image>,
        source_stockpile: Handle<Image>,
        planned_sink_stockpile: Handle<Image>,
        sink_stockpile: Handle<Image>,
        planned_charger: Handle<Image>,
        charger: Handle<Image>,
        planned_production_facility: Handle<Image>,
        production_facility: Handle<Image>,
    ) -> Self {
        Self {
            planned_source_stockpile,
            source_stockpile,
            planned_sink_stockpile,
            sink_stockpile,
            planned_charger,
            charger,
            planned_production_facility,
            production_facility,
        }
    }

    pub fn from_single_handle(handle: Handle<Image>) -> Self {
        Self::from_handles(
            handle.clone(),
            handle.clone(),
            handle.clone(),
            handle.clone(),
            handle.clone(),
            handle.clone(),
            handle.clone(),
            handle,
        )
    }

    pub fn handle(&self, kind: PlannedKind, state: StructureVisualState) -> Handle<Image> {
        match (kind, state) {
            (PlannedKind::SourceStockpile, StructureVisualState::Planned) => {
                self.planned_source_stockpile.clone()
            }
            (PlannedKind::SourceStockpile, StructureVisualState::Completed) => {
                self.source_stockpile.clone()
            }
            (PlannedKind::SinkStockpile, StructureVisualState::Planned) => {
                self.planned_sink_stockpile.clone()
            }
            (PlannedKind::SinkStockpile, StructureVisualState::Completed) => {
                self.sink_stockpile.clone()
            }
            (PlannedKind::Charger, StructureVisualState::Planned) => self.planned_charger.clone(),
            (PlannedKind::Charger, StructureVisualState::Completed) => self.charger.clone(),
            (PlannedKind::ProductionFacility, StructureVisualState::Planned) => {
                self.planned_production_facility.clone()
            }
            (PlannedKind::ProductionFacility, StructureVisualState::Completed) => {
                self.production_facility.clone()
            }
        }
    }

    pub fn sprite(&self, kind: PlannedKind, state: StructureVisualState) -> Sprite {
        Sprite::from_image(self.handle(kind, state))
    }
}
