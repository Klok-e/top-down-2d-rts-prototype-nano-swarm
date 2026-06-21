use bevy::prelude::{AssetServer, Handle, Image, Resource};

use crate::nanobot::NanobotType;

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
