use bevy::prelude::*;

use crate::nanobot::{Nanobot, NanobotType, OpponentSwarm, Swarm, SwarmId, SwarmMember};

/// The single render-time visual owned by a nanobot gameplay root.
#[derive(Debug, Component)]
pub struct NanobotVisual;

#[derive(Debug, Clone, Resource)]
pub struct NanobotSprites {
    pub player_worker: Handle<Image>,
    pub player_hauler: Handle<Image>,
    pub player_defender: Handle<Image>,
    pub opponent_worker: Handle<Image>,
    pub opponent_hauler: Handle<Image>,
    pub opponent_defender: Handle<Image>,
}

impl NanobotSprites {
    pub fn load(asset_server: &AssetServer) -> Self {
        Self {
            player_worker: asset_server.load("worker_nanobot.png"),
            player_hauler: asset_server.load("hauler_nanobot.png"),
            player_defender: asset_server.load("defender_nanobot.png"),
            opponent_worker: asset_server.load("opponent_worker_nanobot.png"),
            opponent_hauler: asset_server.load("opponent_hauler_nanobot.png"),
            opponent_defender: asset_server.load("opponent_defender_nanobot.png"),
        }
    }

    pub fn handle(&self, kind: NanobotType, is_opponent: bool) -> Handle<Image> {
        match (kind, is_opponent) {
            (NanobotType::Worker, false) => self.player_worker.clone(),
            (NanobotType::Hauler, false) => self.player_hauler.clone(),
            (NanobotType::Defender, false) => self.player_defender.clone(),
            (NanobotType::Worker, true) => self.opponent_worker.clone(),
            (NanobotType::Hauler, true) => self.opponent_hauler.clone(),
            (NanobotType::Defender, true) => self.opponent_defender.clone(),
        }
    }
}

/// Installs nanobot rendering without making simulation apps depend on assets.
pub struct NanobotPresentationPlugin;

impl Plugin for NanobotPresentationPlugin {
    fn build(&self, app: &mut App) {
        if !app.world().contains_resource::<NanobotSprites>() {
            let sprites = NanobotSprites::load(app.world().resource::<AssetServer>());
            app.insert_resource(sprites);
        }
        app.add_observer(attach_nanobot_visual);
    }
}

fn attach_nanobot_visual(
    added: On<Add, Nanobot>,
    mut commands: Commands,
    nanobots: Query<(&NanobotType, &SwarmMember, Option<&Visibility>), With<Nanobot>>,
    swarms: Query<(&SwarmId, Has<OpponentSwarm>), With<Swarm>>,
    sprites: Res<NanobotSprites>,
) {
    let Ok((kind, member, visibility)) = nanobots.get(added.entity) else {
        return;
    };
    let is_opponent = swarms
        .iter()
        .any(|(swarm, is_opponent)| *swarm == member.0 && is_opponent);

    let mut root = commands.entity(added.entity);
    if visibility.is_none() {
        root.insert(Visibility::default());
    }
    root.with_child((
        NanobotVisual,
        Sprite::from_image(sprites.handle(*kind, is_opponent)),
        Transform::IDENTITY,
    ));
}
