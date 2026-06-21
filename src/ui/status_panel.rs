use bevy::prelude::*;
use bevy::ui::{AlignItems, BorderRadius, FlexDirection, UiRect};

use crate::{
    nanobot::{
        Nanobot, NanobotType, OpponentSwarm, OwnerSwarm, ProductionFacility, ProductionRatio, Swarm,
    },
    resources::{ResourceDeposit, ResourceKind, Stockpile},
};

use super::ui_setup::FontsResource;

#[derive(Debug, Component)]
pub struct StatusPanelText;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PlayerHudState {
    pub minerals: u32,
    pub workers: u32,
    pub haulers: u32,
    pub defenders: u32,
    pub target_workers: u32,
    pub target_haulers: u32,
    pub target_defenders: u32,
    pub facilities: u32,
    pub deposits_remaining: u32,
    pub producing_workers: u32,
    pub producing_haulers: u32,
    pub producing_defenders: u32,
}

pub fn format_status_panel(state: PlayerHudState) -> String {
    format!(
        "Minerals: {}\nPopulation: W{} H{} D{}\nTarget: W{} H{} D{}\nProduction: {}\nFacilities: {}\nDeposits: {}",
        state.minerals,
        state.workers,
        state.haulers,
        state.defenders,
        state.target_workers,
        state.target_haulers,
        state.target_defenders,
        format_production(state),
        state.facilities,
        state.deposits_remaining,
    )
}

fn format_production(state: PlayerHudState) -> String {
    let mut parts = Vec::new();
    if state.producing_workers > 0 {
        parts.push(format!("W x{}", state.producing_workers));
    }
    if state.producing_haulers > 0 {
        parts.push(format!("H x{}", state.producing_haulers));
    }
    if state.producing_defenders > 0 {
        parts.push(format!("D x{}", state.producing_defenders));
    }
    if parts.is_empty() {
        "idle".to_string()
    } else {
        parts.join(", ")
    }
}

pub fn setup_status_panel(mut commands: Commands, fonts: Res<FontsResource>) {
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(32.0),
                left: Val::Px(5.0),
                padding: UiRect::all(Val::Px(8.0)),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::FlexStart,
                border_radius: BorderRadius::all(Val::Px(4.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.03, 0.04, 0.05, 0.78)),
        ))
        .with_children(|parent| {
            parent.spawn((
                Text::new(format_status_panel(PlayerHudState::default())),
                TextFont {
                    font: fonts.font.clone(),
                    font_size: 16.0,
                    ..default()
                },
                TextColor(Color::WHITE),
                StatusPanelText,
            ));
        });
}

#[allow(clippy::type_complexity)]
pub fn update_status_panel_system(
    ratio: Res<ProductionRatio>,
    player_swarms: Query<(Entity, &Children), (With<Swarm>, Without<OpponentSwarm>)>,
    nanobots: Query<&NanobotType, With<Nanobot>>,
    stockpiles: Query<(&Stockpile, Option<&OwnerSwarm>)>,
    deposits: Query<(&ResourceDeposit, Option<&OwnerSwarm>)>,
    facilities: Query<(&ProductionFacility, Option<&OwnerSwarm>)>,
    mut text: Query<&mut Text, With<StatusPanelText>>,
) {
    let Ok(mut text) = text.single_mut() else {
        return;
    };

    let Some((player_swarm, children)) = player_swarms.iter().next() else {
        *text = Text::new(format_status_panel(PlayerHudState::default()));
        return;
    };

    let mut workers = 0;
    let mut haulers = 0;
    let mut defenders = 0;
    for child in children.iter() {
        if let Ok(kind) = nanobots.get(child) {
            match *kind {
                NanobotType::Worker => workers += 1,
                NanobotType::Hauler => haulers += 1,
                NanobotType::Defender => defenders += 1,
            }
        }
    }

    let mut state = PlayerHudState {
        workers,
        haulers,
        defenders,
        target_workers: ratio.target(NanobotType::Worker),
        target_haulers: ratio.target(NanobotType::Hauler),
        target_defenders: ratio.target(NanobotType::Defender),
        ..default()
    };

    for (stockpile, owner) in &stockpiles {
        if stockpile.kind != ResourceKind::Minerals || !belongs_to_player(owner, player_swarm) {
            continue;
        }
        state.minerals = state.minerals.saturating_add(stockpile.amount);
    }

    for (deposit, owner) in &deposits {
        if deposit.kind != ResourceKind::Minerals || !belongs_to_player(owner, player_swarm) {
            continue;
        }
        state.deposits_remaining = state.deposits_remaining.saturating_add(deposit.amount);
    }

    for (facility, owner) in &facilities {
        if !belongs_to_player(owner, player_swarm) {
            continue;
        }
        state.facilities += 1;
        match facility.current_target {
            Some(NanobotType::Worker) => state.producing_workers += 1,
            Some(NanobotType::Hauler) => state.producing_haulers += 1,
            Some(NanobotType::Defender) => state.producing_defenders += 1,
            None => {}
        }
    }

    *text = Text::new(format_status_panel(state));
}

fn belongs_to_player(owner: Option<&OwnerSwarm>, player_swarm: Entity) -> bool {
    owner.is_none_or(|OwnerSwarm(owner)| *owner == player_swarm)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_status_panel_shows_world_state_only() {
        let text = format_status_panel(PlayerHudState {
            minerals: 24,
            workers: 4,
            haulers: 2,
            defenders: 0,
            target_workers: 10,
            target_haulers: 4,
            target_defenders: 1,
            facilities: 1,
            deposits_remaining: 976,
            ..default()
        });

        assert_eq!(
            text,
            "Minerals: 24\nPopulation: W4 H2 D0\nTarget: W10 H4 D1\nProduction: idle\nFacilities: 1\nDeposits: 976"
        );
        assert!(!text.contains("Selected"));
        assert!(!text.contains("NANO SWARM"));
    }

    #[test]
    fn format_status_panel_summarizes_active_production() {
        let text = format_status_panel(PlayerHudState {
            producing_workers: 1,
            producing_defenders: 2,
            ..default()
        });

        assert!(text.contains("Production: W x1, D x2"));
    }
}
