use std::time::Duration;

use bevy::prelude::*;
use bevy::ui::{AlignItems, BorderRadius, FlexDirection, RelativeCursorPosition, UiRect};

use crate::{
    nanobot::{
        MatchOutcome, Nanobot, NanobotSprites, NanobotType, OpponentSwarm, OwnerSwarm,
        PopulationDemand, ProductionFacility, ResolvedCombatFact, SupportCondition, Swarm, SwarmId,
        SwarmMember, TerritorySnapshot, swarm_presentation_color,
    },
    resources::{ResourceDeposit, ResourceKind, ResourceLedger},
    session::SessionRules,
    session_lifecycle::SessionGeneration,
};

use super::ui_setup::FontsResource;

const ATTACK_NOTICE_DURATION: Duration = Duration::from_secs(4);

#[derive(Debug, Component)]
pub(super) enum StatusPanelText {
    Player,
    NoNanobots,
    Spectator { opponent: bool },
}

#[derive(Component)]
pub(super) struct CombatAlert;

#[derive(Debug, Clone, Copy, Default)]
struct PlayerHudState {
    minerals: u32,
    workers: u32,
    haulers: u32,
    defenders: u32,
    worker_demand: u32,
    hauler_demand: u32,
    defender_demand: u32,
    facilities: u32,
    deposits_remaining: u32,
    producing_workers: u32,
    producing_haulers: u32,
    producing_defenders: u32,
    production_status: ProductionStatus,
}

#[derive(Debug, Clone, Copy, Default)]
enum ProductionStatus {
    #[default]
    DemandMet,
    Producing,
    WaitingForDelivery,
    NoHaulers,
    Unavailable,
}

#[derive(Default)]
pub(super) struct RecentAttack {
    generation: u64,
    until: Duration,
}

fn format_status_panel(state: PlayerHudState) -> String {
    format!(
        "Minerals: {}\nHome deposit minerals: {}\nFacilities: {}\n\nPopulation / Demand\nWorkers: {} / {}\nHaulers: {} / {}\nDefenders: {} / {}\n\nProduction: {}",
        state.minerals,
        state.deposits_remaining,
        state.facilities,
        state.workers,
        state.worker_demand,
        state.haulers,
        state.hauler_demand,
        state.defenders,
        state.defender_demand,
        format_production(state),
    )
}

fn format_production(state: PlayerHudState) -> String {
    match state.production_status {
        ProductionStatus::DemandMet => "demand met".into(),
        ProductionStatus::WaitingForDelivery => "waiting for delivery".into(),
        ProductionStatus::NoHaulers => "no Haulers to deliver".into(),
        ProductionStatus::Unavailable => "unavailable".into(),
        ProductionStatus::Producing => [
            (state.producing_workers, "Worker", "Workers"),
            (state.producing_haulers, "Hauler", "Haulers"),
            (state.producing_defenders, "Defender", "Defenders"),
        ]
        .into_iter()
        .filter(|(count, _, _)| *count > 0)
        .map(|(count, singular, plural)| {
            let label = if count == 1 { singular } else { plural };
            format!("{count} {label}")
        })
        .collect::<Vec<_>>()
        .join(", "),
    }
}

pub(super) fn setup_status_panel(
    mut commands: Commands,
    fonts: Res<FontsResource>,
    sprites: Option<Res<NanobotSprites>>,
) {
    let font = TextFont {
        font: fonts.font.clone(),
        font_size: 16.0,
        ..default()
    };
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(72.0),
                left: Val::Px(5.0),
                width: Val::Px(272.0),
                max_width: Val::Percent(90.0),
                padding: UiRect::axes(Val::Px(12.0), Val::Px(8.0)),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Stretch,
                row_gap: Val::Px(8.0),
                border_radius: BorderRadius::all(Val::Px(6.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.03, 0.04, 0.05, 0.85)),
            RelativeCursorPosition::default(),
        ))
        .with_children(|parent| {
            parent
                .spawn((
                    Name::new("Combat alert"),
                    CombatAlert,
                    Node {
                        display: Display::None,
                        width: Val::Px(34.0),
                        height: Val::Px(34.0),
                        align_self: AlignSelf::Start,
                        align_items: AlignItems::Center,
                        justify_content: JustifyContent::Center,
                        border: UiRect::all(Val::Px(2.0)),
                        border_radius: BorderRadius::all(Val::Px(6.0)),
                        ..default()
                    },
                    BackgroundColor::default(),
                    BorderColor::default(),
                ))
                .with_child((
                    ImageNode::new(sprites.as_ref().map_or_else(Handle::default, |sprites| {
                        sprites.handle(NanobotType::Defender, true)
                    })),
                    Node {
                        width: Val::Px(26.0),
                        height: Val::Px(26.0),
                        ..default()
                    },
                ));
            parent.spawn((
                Text::default(),
                font.clone(),
                TextColor(Color::srgb(1.0, 0.68, 0.40)),
                StatusPanelText::NoNanobots,
            ));
            parent.spawn((
                Text::default(),
                font.clone(),
                TextColor(Color::WHITE),
                StatusPanelText::Player,
            ));
            for opponent in [false, true] {
                parent
                    .spawn((
                        Text::default(),
                        font.clone(),
                        StatusPanelText::Spectator { opponent },
                    ))
                    .with_child((TextSpan::default(), font.clone(), TextColor(Color::WHITE)));
            }
        });
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub(super) fn update_status_panel_system(
    swarms: Query<(Entity, &SwarmId, Has<OpponentSwarm>), With<Swarm>>,
    rules: Res<SessionRules>,
    nanobots: Query<(&NanobotType, &SwarmMember), With<Nanobot>>,
    deposits: Query<(&ResourceDeposit, &OwnerSwarm)>,
    facilities: Query<(&ProductionFacility, &OwnerSwarm, Option<&SupportCondition>)>,
    ledger: Res<ResourceLedger>,
    population_demand: Option<Res<PopulationDemand>>,
    territory: Option<Res<TerritorySnapshot>>,
    outcome: Option<Res<MatchOutcome>>,
    generation: Option<Res<SessionGeneration>>,
    time: Option<Res<Time<Virtual>>>,
    mut combat: Option<MessageReader<ResolvedCombatFact>>,
    mut recent_attack: Local<RecentAttack>,
    mut panel: Query<
        (
            &StatusPanelText,
            &mut Node,
            &mut Text,
            &mut TextColor,
            Option<&Children>,
        ),
        Without<CombatAlert>,
    >,
    mut alerts: Query<(&mut Node, &mut BackgroundColor, &mut BorderColor), With<CombatAlert>>,
    mut details: Query<&mut TextSpan>,
) {
    let now = time.map_or(Duration::ZERO, |time| time.elapsed());
    let generation = generation.map_or(0, |generation| generation.0);
    if recent_attack.generation != generation {
        *recent_attack = RecentAttack {
            generation,
            ..default()
        };
    }
    if let Some(combat) = &mut combat {
        for fact in combat.read() {
            if let ResolvedCombatFact::Hit(hit) = fact
                && Some(hit.target.swarm) == rules.player_swarm
            {
                recent_attack.until = now + ATTACK_NOTICE_DURATION;
            }
        }
    }

    let player = swarms
        .iter()
        .find(|(_, id, _)| Some(**id) == rules.player_swarm);
    let counts = |swarm| {
        let (mut workers, mut haulers, mut defenders) = (0, 0, 0);
        for (kind, member) in &nanobots {
            if member.0 == swarm {
                match kind {
                    NanobotType::Worker => workers += 1,
                    NanobotType::Hauler => haulers += 1,
                    NanobotType::Defender => defenders += 1,
                }
            }
        }
        (workers, haulers, defenders)
    };

    let mut state = PlayerHudState::default();
    let mut no_nanobots = false;
    let mut alert_colors = None;
    if let Some((player, &id, _)) = player {
        let (workers, haulers, defenders) = counts(id);
        let demand = |kind| {
            population_demand
                .as_deref()
                .map_or(0, |demand| demand.desired_for(id, kind))
        };
        state = PlayerHudState {
            minerals: ledger.total_for(id, ResourceKind::Minerals),
            workers,
            haulers,
            defenders,
            worker_demand: demand(NanobotType::Worker),
            hauler_demand: demand(NanobotType::Hauler),
            defender_demand: demand(NanobotType::Defender),
            deposits_remaining: deposits
                .iter()
                .filter(|(deposit, owner)| {
                    deposit.kind == ResourceKind::Minerals && owner.0 == player
                })
                .fold(0u32, |total, (deposit, _)| {
                    total.saturating_add(deposit.amount)
                }),
            ..default()
        };

        let mut operational = false;
        for (facility, owner, condition) in &facilities {
            if owner.0 != player {
                continue;
            }
            state.facilities += 1;
            if condition.is_some_and(|condition| !condition.is_operational()) {
                continue;
            }
            operational = true;
            match facility.current_target {
                Some(NanobotType::Worker) => state.producing_workers += 1,
                Some(NanobotType::Hauler) => state.producing_haulers += 1,
                Some(NanobotType::Defender) => state.producing_defenders += 1,
                None => {}
            }
        }

        let active = state.producing_workers + state.producing_haulers + state.producing_defenders;
        let has_shortage = workers < state.worker_demand
            || haulers < state.hauler_demand
            || defenders < state.defender_demand;
        state.production_status = if active > 0 {
            ProductionStatus::Producing
        } else if !has_shortage {
            ProductionStatus::DemandMet
        } else if !operational {
            ProductionStatus::Unavailable
        } else if haulers == 0 {
            ProductionStatus::NoHaulers
        } else {
            ProductionStatus::WaitingForDelivery
        };

        if outcome.as_deref().copied().unwrap_or_default() == MatchOutcome::InProgress {
            no_nanobots = workers + haulers + defenders == 0;
            if !no_nanobots {
                alert_colors = if now < recent_attack.until {
                    Some((Color::srgb_u8(110, 43, 17), Color::srgb_u8(255, 140, 65)))
                } else if territory.is_some_and(|territory| territory.threat_count(id) > 0) {
                    Some((Color::NONE, swarm_presentation_color(SwarmId(1))))
                } else {
                    None
                };
            }
        }
    }

    for (mut node, mut background, mut border) in &mut alerts {
        let display = if let Some((fill, outline)) = alert_colors {
            background.set_if_neq(BackgroundColor(fill));
            border.set_if_neq(BorderColor::all(outline));
            Display::Flex
        } else {
            Display::None
        };
        if node.display != display {
            node.display = display;
        }
    }

    for (kind, mut node, mut text, mut color, children) in &mut panel {
        let next = match kind {
            StatusPanelText::Player if !rules.spectator() => Some(format_status_panel(state)),
            StatusPanelText::NoNanobots if no_nanobots => Some("No nanobots remaining".into()),
            StatusPanelText::Spectator { opponent } if rules.spectator() => {
                swarms.iter().find(|(_, _, side)| side == opponent).map(|(swarm, id, _)| {
                    let (workers, haulers, defenders) = counts(*id);
                    let minerals = ledger.total_for(*id, ResourceKind::Minerals);
                    let facilities = facilities
                        .iter()
                        .filter(|(_, owner, _)| owner.0 == swarm)
                        .count();
                    let content = format!(
                        "\nWorkers: {workers}    Haulers: {haulers}\nDefenders: {defenders}\nMinerals: {minerals}    Facilities: {facilities}",
                    );
                    if let Some(child) = children.and_then(|children| children.first())
                        && let Ok(mut detail) = details.get_mut(*child)
                        && detail.0 != content
                    {
                        detail.0 = content;
                    }
                    let accent = swarm_presentation_color(*id);
                    if color.0 != accent {
                        color.0 = accent;
                    }
                    format!("Swarm {}", id.0)
                })
            }
            _ => None,
        };
        let display = if next.is_some() {
            Display::Flex
        } else {
            Display::None
        };
        if node.display != display {
            node.display = display;
        }
        if let Some(next) = next
            && text.0 != next
        {
            text.0 = next;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hud_minerals_use_complete_player_ledger() {
        let mut app = App::new();
        app.init_resource::<crate::resources::ResourceLedger>()
            .init_resource::<SessionRules>()
            .add_systems(Update, update_status_panel_system);
        let player = app.world_mut().spawn((Swarm {}, SwarmId::PLAYER)).id();
        app.world_mut().spawn((
            crate::resources::Stockpile {
                kind: ResourceKind::Minerals,
                amount: 3,
                capacity: 100,
                radius: 32.0,
            },
            OwnerSwarm(player),
        ));
        app.world_mut()
            .resource_mut::<crate::resources::ResourceLedger>()
            .add_for(SwarmId::PLAYER, ResourceKind::Minerals, 17);
        let text = app
            .world_mut()
            .spawn((Text::new(""), StatusPanelText::Player))
            .id();

        app.update();

        assert!(
            app.world()
                .entity(text)
                .get::<Text>()
                .unwrap()
                .0
                .starts_with("Minerals: 17\n"),
            "HUD includes cargo and terminal minerals tracked by the player ledger",
        );
    }

    #[test]
    fn hud_deposits_exclude_neutral_and_foreign_resources() {
        let mut app = App::new();
        app.init_resource::<crate::resources::ResourceLedger>()
            .init_resource::<SessionRules>()
            .add_systems(Update, update_status_panel_system);
        let player = app.world_mut().spawn((Swarm {}, SwarmId::PLAYER)).id();
        let foreign = app.world_mut().spawn_empty().id();
        for (amount, owner) in [(7, Some(player)), (11, None), (13, Some(foreign))] {
            let mut deposit = app.world_mut().spawn(ResourceDeposit {
                kind: ResourceKind::Minerals,
                amount,
                capacity: amount,
                radius: 32.0,
            });
            if let Some(owner) = owner {
                deposit.insert(OwnerSwarm(owner));
            }
        }
        let text = app
            .world_mut()
            .spawn((Text::new(""), StatusPanelText::Player))
            .id();

        app.update();

        assert!(
            app.world()
                .entity(text)
                .get::<Text>()
                .unwrap()
                .0
                .contains("\nHome deposit minerals: 7\n"),
            "the player HUD reports only explicitly player-owned deposits",
        );
    }

    #[test]
    fn hud_ignores_unowned_production_facilities() {
        let mut app = App::new();
        app.init_resource::<crate::resources::ResourceLedger>()
            .init_resource::<SessionRules>()
            .add_systems(Update, update_status_panel_system);
        app.world_mut().spawn((Swarm {}, SwarmId::PLAYER));
        let mut orphan = ProductionFacility::new();
        orphan.current_target = Some(NanobotType::Worker);
        app.world_mut().spawn(orphan);
        let text = app
            .world_mut()
            .spawn((Text::new(""), StatusPanelText::Player))
            .id();

        app.update();

        let text = &app.world().entity(text).get::<Text>().unwrap().0;
        assert!(text.contains("Production: demand met"));
        assert!(text.contains("Facilities: 0"));
    }
}
