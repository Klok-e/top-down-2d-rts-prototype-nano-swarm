use bevy::prelude::*;
use bevy::ui::{AlignItems, BorderRadius, FlexDirection, UiRect};

use crate::{
    nanobot::{
        Nanobot, NanobotType, OpponentSwarm, OwnerSwarm, PopulationDemand, ProductionFacility,
        SupportCondition, Swarm, SwarmId, SwarmMember,
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
    pub worker_demand: u32,
    pub hauler_demand: u32,
    pub defender_demand: u32,
    pub facilities: u32,
    pub deposits_remaining: u32,
    pub producing_workers: u32,
    pub producing_haulers: u32,
    pub producing_defenders: u32,
    pub production_status: ProductionStatus,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ProductionStatus {
    #[default]
    DemandMet,
    Producing,
    WaitingForDelivery,
    Unavailable,
}

pub fn format_status_panel(state: PlayerHudState) -> String {
    format!(
        "Minerals: {}\nPopulation: W{} H{} D{}\nDemand: W{} H{} D{}\nProduction: {}\nFacilities: {}\nDeposits: {}",
        state.minerals,
        state.workers,
        state.haulers,
        state.defenders,
        state.worker_demand,
        state.hauler_demand,
        state.defender_demand,
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
    if !parts.is_empty() {
        return parts.join(", ");
    }
    match state.production_status {
        ProductionStatus::DemandMet => "demand met".to_string(),
        ProductionStatus::Producing => "producing".to_string(),
        ProductionStatus::WaitingForDelivery => "waiting for delivery".to_string(),
        ProductionStatus::Unavailable => "unavailable".to_string(),
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
    player_swarms: Query<(Entity, &SwarmId), (With<Swarm>, Without<OpponentSwarm>)>,
    nanobots: Query<(&NanobotType, &SwarmMember), With<Nanobot>>,
    stockpiles: Query<(&Stockpile, Option<&OwnerSwarm>)>,
    deposits: Query<(&ResourceDeposit, Option<&OwnerSwarm>)>,
    facilities: Query<(
        &ProductionFacility,
        Option<&OwnerSwarm>,
        Option<&SupportCondition>,
    )>,
    population_demand: Option<Res<PopulationDemand>>,
    mut text: Query<&mut Text, With<StatusPanelText>>,
) {
    let Ok(mut text) = text.single_mut() else {
        return;
    };

    let Some((player_swarm, swarm_id)) = player_swarms.iter().next() else {
        *text = Text::new(format_status_panel(PlayerHudState::default()));
        return;
    };

    let mut workers = 0;
    let mut haulers = 0;
    let mut defenders = 0;
    for (kind, member) in &nanobots {
        if member.0 == *swarm_id {
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
        worker_demand: population_demand
            .as_deref()
            .map(|demand| demand.desired_for(*swarm_id, NanobotType::Worker))
            .unwrap_or_default(),
        hauler_demand: population_demand
            .as_deref()
            .map(|demand| demand.desired_for(*swarm_id, NanobotType::Hauler))
            .unwrap_or_default(),
        defender_demand: population_demand
            .as_deref()
            .map(|demand| demand.desired_for(*swarm_id, NanobotType::Defender))
            .unwrap_or_default(),
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

    let mut operational_facilities = 0;
    for (facility, owner, condition) in &facilities {
        if !belongs_to_player(owner, player_swarm) {
            continue;
        }
        state.facilities += 1;
        let operational = condition.is_none_or(|condition| condition.is_operational());
        if !operational {
            continue;
        }
        operational_facilities += 1;
        match facility.current_target {
            Some(NanobotType::Worker) => state.producing_workers += 1,
            Some(NanobotType::Hauler) => state.producing_haulers += 1,
            Some(NanobotType::Defender) => state.producing_defenders += 1,
            None => {}
        }
    }

    let has_shortage = workers < state.worker_demand
        || haulers < state.hauler_demand
        || defenders < state.defender_demand;
    let active = state.producing_workers + state.producing_haulers + state.producing_defenders;
    state.production_status = if active > 0 {
        ProductionStatus::Producing
    } else if !has_shortage {
        ProductionStatus::DemandMet
    } else if operational_facilities == 0 {
        ProductionStatus::Unavailable
    } else {
        ProductionStatus::WaitingForDelivery
    };

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
            facilities: 1,
            deposits_remaining: 976,
            ..default()
        });

        assert_eq!(
            text,
            "Minerals: 24\nPopulation: W4 H2 D0\nDemand: W0 H0 D0\nProduction: demand met\nFacilities: 1\nDeposits: 976"
        );
        assert!(!text.contains("Selected"));
        assert!(!text.contains("NANO SWARM"));
        assert!(!text.contains("Target:"));
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

    #[test]
    fn format_status_panel_distinguishes_waiting_and_unavailable() {
        let waiting = format_status_panel(PlayerHudState {
            defender_demand: 1,
            production_status: ProductionStatus::WaitingForDelivery,
            ..default()
        });
        let unavailable = format_status_panel(PlayerHudState {
            defender_demand: 1,
            production_status: ProductionStatus::Unavailable,
            ..default()
        });

        assert!(waiting.contains("Demand: W0 H0 D1"));
        assert!(waiting.contains("Production: waiting for delivery"));
        assert!(unavailable.contains("Production: unavailable"));
    }
}
