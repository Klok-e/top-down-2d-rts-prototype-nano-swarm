use bevy::prelude::*;

use crate::nanobot::{
    OwnerSwarm, PlannedKind, SwarmId, completed_visual_color, planned_visual_color,
};
use crate::navigation::STRUCTURE_SPRITE_SIZE;

pub(crate) const PRODUCTION_FACILITY_TEXTURE: &str = "production_facility.png";

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

    pub(crate) fn components(
        self,
        image: Handle<Image>,
        transform: Transform,
    ) -> (Sprite, Transform, Self) {
        let sprite = Sprite {
            image,
            color: match self.state {
                StructureVisualState::Planned => planned_visual_color(),
                StructureVisualState::Completed => completed_visual_color(),
            },
            custom_size: Some(Vec2::splat(STRUCTURE_SPRITE_SIZE)),
            ..default()
        };
        (sprite, transform, self)
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
            production_facility: asset_server.load(PRODUCTION_FACILITY_TEXTURE),
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

#[derive(Component)]
pub(crate) struct FootprintDecoration;

pub(crate) fn attach_footprint_outline(
    inserted: On<Insert, StructureVisual>,
    mut commands: Commands,
    structures: Query<(&StructureVisual, Option<&OwnerSwarm>, Option<&Children>)>,
    swarms: Query<&SwarmId>,
    decorations: Query<(), With<FootprintDecoration>>,
) {
    let Ok((visual, owner, children)) = structures.get(inserted.entity) else {
        return;
    };
    for child in children.into_iter().flat_map(|children| children.iter()) {
        if decorations.contains(child) {
            commands.entity(child).despawn();
        }
    }

    let planned = visual.state == StructureVisualState::Planned;
    let size = STRUCTURE_SPRITE_SIZE;
    let thickness = if planned { 4.0 } else { 3.0 };
    let edge = (size - thickness) / 2.0;
    let length = if planned { 15.0 } else { size };
    let along_edge = (size - length) / 2.0;
    let centers: &[f32] = if planned {
        &[-along_edge, along_edge]
    } else {
        &[0.0]
    };
    let owner_color = owner
        .and_then(|owner| swarms.get(owner.0).ok())
        .map_or(Color::srgb(0.71, 0.77, 0.80), |&swarm| {
            crate::nanobot::swarm_presentation_color(swarm)
        });
    let edge_color = if planned {
        Color::srgb(0.94, 0.96, 0.99)
    } else {
        owner_color
    };
    commands.entity(inserted.entity).with_children(|parent| {
        parent.spawn((
            FootprintDecoration,
            Sprite::from_color(
                Color::srgba(0.045, 0.055, 0.075, if planned { 0.65 } else { 0.88 }),
                Vec2::splat(size),
            ),
            Transform::from_xyz(0.0, 0.0, -0.01),
        ));
        for side in [-edge, edge] {
            for &center in centers {
                for (position, dimensions) in [
                    (Vec2::new(center, side), Vec2::new(length, thickness)),
                    (Vec2::new(side, center), Vec2::new(thickness, length)),
                ] {
                    parent.spawn((
                        FootprintDecoration,
                        Sprite::from_color(edge_color, dimensions),
                        Transform::from_translation(position.extend(0.01)),
                    ));
                }
            }
        }
    });
}
