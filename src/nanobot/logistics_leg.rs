//! Pure Logistics Leg picker for hauler Resource Logistics.
//!
//! This module owns ADR-0005's downstream-first, sink-first
//! ranking without depending on Bevy `Query` shapes. Callers build
//! small candidate snapshots from ECS, then ask for the best
//! [`LogisticsLeg`] for a Hauler.

use bevy::prelude::{Entity, Vec2};

use crate::nanobot::SwarmId;
use crate::resources::{ResourceKind, StockpileRole};

/// Hauler facts needed to pick its next Logistics Leg.
#[derive(Debug, Clone, Copy)]
pub struct HaulerContext {
    pub pos: Vec2,
    pub swarm: SwarmId,
    pub kind: ResourceKind,
    pub carry_capacity: u32,
}

/// Stockpile snapshot used as both a possible source and a
/// possible buffer sink. A missing ECS `StockpileRole` is adapted
/// to [`StockpileRole::Source`] before entering this module.
#[derive(Debug, Clone, Copy)]
pub struct StockpileCandidate {
    pub entity: Entity,
    pub pos: Vec2,
    pub kind: ResourceKind,
    pub role: StockpileRole,
    pub amount: u32,
    pub free_space: u32,
    pub owner: Option<SwarmId>,
}

/// Terminal Consumer snapshot. Terminals are inflow-only: they
/// can be a sink for a Logistics Leg, never a source.
#[derive(Debug, Clone, Copy)]
pub enum TerminalCandidate {
    Facility {
        entity: Entity,
        pos: Vec2,
        kind: ResourceKind,
        free_space: u32,
        owner: Option<SwarmId>,
    },
    Charger {
        entity: Entity,
        pos: Vec2,
        kind: ResourceKind,
        free_space: u32,
        owner: Option<SwarmId>,
    },
}

impl TerminalCandidate {
    fn entity(self) -> Entity {
        match self {
            TerminalCandidate::Facility { entity, .. }
            | TerminalCandidate::Charger { entity, .. } => entity,
        }
    }

    fn pos(self) -> Vec2 {
        match self {
            TerminalCandidate::Facility { pos, .. } | TerminalCandidate::Charger { pos, .. } => pos,
        }
    }

    fn kind(self) -> ResourceKind {
        match self {
            TerminalCandidate::Facility { kind, .. } | TerminalCandidate::Charger { kind, .. } => {
                kind
            }
        }
    }

    fn free_space(self) -> u32 {
        match self {
            TerminalCandidate::Facility { free_space, .. }
            | TerminalCandidate::Charger { free_space, .. } => free_space,
        }
    }

    fn owner(self) -> Option<SwarmId> {
        match self {
            TerminalCandidate::Facility { owner, .. }
            | TerminalCandidate::Charger { owner, .. } => owner,
        }
    }

    fn source_filter(self) -> StockpileSourceFilter {
        StockpileSourceFilter::Sink
    }
}

/// Picked source/sink pair for one directed Logistics Leg.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LogisticsLeg {
    pub source: Entity,
    pub sink: Entity,
    pub amount: u32,
}

#[derive(Debug, Clone, Copy)]
enum StockpileSourceFilter {
    Source,
    Sink,
}

/// Pick the best Logistics Leg for a Hauler.
///
/// Ranking is ADR-0005: terminal sinks beat buffer sinks; within
/// a tier the shortest `hauler -> source -> sink` trip wins.
/// Both terminal kinds draw only from Sink Stockpiles; Sink
/// Stockpiles draw only from Source Stockpiles.
#[cfg(test)]
pub fn pick_logistics_leg(
    hauler: HaulerContext,
    stockpiles: &[StockpileCandidate],
    terminals: &[TerminalCandidate],
) -> Option<LogisticsLeg> {
    pick_logistics_leg_with_cost(hauler, stockpiles, terminals, |a, b| a.distance(b))
}

/// Pick the best Logistics Leg using caller-supplied route costs.
pub fn pick_logistics_leg_with_cost(
    hauler: HaulerContext,
    stockpiles: &[StockpileCandidate],
    terminals: &[TerminalCandidate],
    travel_cost: impl Fn(Vec2, Vec2) -> f32,
) -> Option<LogisticsLeg> {
    if let Some(leg) = best_terminal_leg(hauler, stockpiles, terminals, &travel_cost) {
        return Some(leg);
    }
    best_buffer_leg(hauler, stockpiles, &travel_cost)
}

fn best_terminal_leg(
    hauler: HaulerContext,
    stockpiles: &[StockpileCandidate],
    terminals: &[TerminalCandidate],
    travel_cost: &impl Fn(Vec2, Vec2) -> f32,
) -> Option<LogisticsLeg> {
    let mut best: Option<(f32, LogisticsLeg)> = None;
    for terminal in terminals.iter().copied() {
        if terminal.kind() != hauler.kind
            || terminal.free_space() == 0
            || !owner_matches_hauler(terminal.owner(), hauler.swarm)
        {
            continue;
        }
        let Some((source, amount, trip)) = best_source_for_sink(
            hauler,
            terminal.pos(),
            terminal.free_space(),
            stockpiles,
            terminal.source_filter(),
            Some(terminal.owner()),
            travel_cost,
        ) else {
            continue;
        };
        let leg = LogisticsLeg {
            source,
            sink: terminal.entity(),
            amount,
        };
        if best.is_none_or(|(best_trip, _)| trip < best_trip) {
            best = Some((trip, leg));
        }
    }
    best.map(|(_, leg)| leg)
}

fn best_buffer_leg(
    hauler: HaulerContext,
    stockpiles: &[StockpileCandidate],
    travel_cost: &impl Fn(Vec2, Vec2) -> f32,
) -> Option<LogisticsLeg> {
    let mut best: Option<(f32, LogisticsLeg)> = None;
    for sink in stockpiles.iter().copied() {
        if sink.role != StockpileRole::Sink
            || sink.kind != hauler.kind
            || sink.free_space == 0
            || !owner_matches_hauler(sink.owner, hauler.swarm)
        {
            continue;
        }
        let Some((source, amount, trip)) = best_source_for_sink(
            hauler,
            sink.pos,
            sink.free_space,
            stockpiles,
            StockpileSourceFilter::Source,
            None,
            travel_cost,
        ) else {
            continue;
        };
        let leg = LogisticsLeg {
            source,
            sink: sink.entity,
            amount,
        };
        if best.is_none_or(|(best_trip, _)| trip < best_trip) {
            best = Some((trip, leg));
        }
    }
    best.map(|(_, leg)| leg)
}

fn best_source_for_sink(
    hauler: HaulerContext,
    sink_pos: Vec2,
    sink_free_space: u32,
    stockpiles: &[StockpileCandidate],
    filter: StockpileSourceFilter,
    exact_owner: Option<Option<SwarmId>>,
    travel_cost: &impl Fn(Vec2, Vec2) -> f32,
) -> Option<(Entity, u32, f32)> {
    let mut best: Option<(f32, Entity, u32)> = None;
    for source in stockpiles.iter().copied() {
        if source.kind != hauler.kind
            || source.amount == 0
            || !role_matches_filter(source.role, filter)
            || !owner_matches_hauler(source.owner, hauler.swarm)
            || exact_owner.is_some_and(|owner| source.owner != owner)
        {
            continue;
        }
        let carried_amount = source
            .amount
            .min(hauler.carry_capacity)
            .min(sink_free_space);
        if carried_amount == 0 {
            continue;
        }
        let trip = travel_cost(hauler.pos, source.pos) + travel_cost(source.pos, sink_pos);
        if best.is_none_or(|(best_trip, _, _)| trip < best_trip) {
            best = Some((trip, source.entity, carried_amount));
        }
    }
    best.map(|(trip, entity, amount)| (entity, amount, trip))
}

fn role_matches_filter(role: StockpileRole, filter: StockpileSourceFilter) -> bool {
    match filter {
        StockpileSourceFilter::Source => role == StockpileRole::Source,
        StockpileSourceFilter::Sink => role == StockpileRole::Sink,
    }
}

fn owner_matches_hauler(owner: Option<SwarmId>, hauler_swarm: SwarmId) -> bool {
    owner == Some(hauler_swarm)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(id: u32) -> Entity {
        Entity::from_raw_u32(id).expect("test entity")
    }

    fn source(id: u32, pos: Vec2, amount: u32) -> StockpileCandidate {
        StockpileCandidate {
            entity: e(id),
            pos,
            kind: ResourceKind::Minerals,
            role: StockpileRole::Source,
            amount,
            free_space: 0,
            owner: Some(SwarmId::PLAYER),
        }
    }

    fn sink(id: u32, pos: Vec2, amount: u32, free_space: u32) -> StockpileCandidate {
        StockpileCandidate {
            entity: e(id),
            pos,
            kind: ResourceKind::Minerals,
            role: StockpileRole::Sink,
            amount,
            free_space,
            owner: Some(SwarmId::PLAYER),
        }
    }

    fn hauler(pos: Vec2) -> HaulerContext {
        HaulerContext {
            pos,
            swarm: SwarmId::PLAYER,
            kind: ResourceKind::Minerals,
            carry_capacity: 40,
        }
    }

    #[test]
    fn terminal_sink_beats_nearer_buffer_sink() {
        let stockpiles = [
            source(1, Vec2::new(10.0, 0.0), 100),
            sink(2, Vec2::new(20.0, 0.0), 100, 100),
            sink(3, Vec2::new(1_000.0, 0.0), 100, 0),
        ];
        let terminals = [TerminalCandidate::Facility {
            entity: e(4),
            pos: Vec2::new(1_010.0, 0.0),
            kind: ResourceKind::Minerals,
            free_space: 100,
            owner: Some(SwarmId::PLAYER),
        }];

        let leg = pick_logistics_leg(hauler(Vec2::ZERO), &stockpiles, &terminals).unwrap();

        assert_eq!(leg.source, e(2));
        assert_eq!(leg.sink, e(4));
    }

    #[test]
    fn facility_draws_only_from_sink_stockpile() {
        let stockpiles = [
            source(1, Vec2::new(1.0, 0.0), 100),
            sink(2, Vec2::new(100.0, 0.0), 100, 0),
        ];
        let terminals = [TerminalCandidate::Facility {
            entity: e(3),
            pos: Vec2::new(110.0, 0.0),
            kind: ResourceKind::Minerals,
            free_space: 100,
            owner: Some(SwarmId::PLAYER),
        }];

        let leg = pick_logistics_leg(hauler(Vec2::ZERO), &stockpiles, &terminals).unwrap();

        assert_eq!(leg.source, e(2));
        assert_eq!(leg.sink, e(3));
    }

    #[test]
    fn buffer_sink_draws_only_from_source_stockpile() {
        let stockpiles = [
            sink(1, Vec2::new(1.0, 0.0), 100, 0),
            source(2, Vec2::new(100.0, 0.0), 100),
            sink(3, Vec2::new(110.0, 0.0), 0, 100),
        ];

        let leg = pick_logistics_leg(hauler(Vec2::ZERO), &stockpiles, &[]).unwrap();

        assert_eq!(leg.source, e(2));
        assert_eq!(leg.sink, e(3));
    }

    #[test]
    fn charger_draws_only_from_sink_stockpile() {
        let stockpiles = [
            source(1, Vec2::new(1.0, 0.0), 100),
            sink(2, Vec2::new(50.0, 0.0), 100, 0),
        ];
        let terminals = [TerminalCandidate::Charger {
            entity: e(3),
            pos: Vec2::new(10.0, 0.0),
            kind: ResourceKind::Minerals,
            free_space: 100,
            owner: Some(SwarmId::PLAYER),
        }];

        let leg = pick_logistics_leg(hauler(Vec2::ZERO), &stockpiles, &terminals).unwrap();

        assert_eq!(leg.source, e(2));
        assert_eq!(leg.sink, e(3));
    }

    #[test]
    fn terminal_leg_reserves_only_available_free_space() {
        let stockpiles = [sink(1, Vec2::new(1.0, 0.0), 100, 0)];
        let terminals = [TerminalCandidate::Facility {
            entity: e(2),
            pos: Vec2::new(10.0, 0.0),
            kind: ResourceKind::Minerals,
            free_space: 39,
            owner: Some(SwarmId::PLAYER),
        }];

        let leg = pick_logistics_leg(hauler(Vec2::ZERO), &stockpiles, &terminals).unwrap();
        assert_eq!(leg.amount, 39);
    }

    #[test]
    fn terminal_leg_allows_partial_source_that_fits_free_space() {
        let stockpiles = [sink(1, Vec2::new(1.0, 0.0), 20, 0)];
        let terminals = [TerminalCandidate::Facility {
            entity: e(2),
            pos: Vec2::new(10.0, 0.0),
            kind: ResourceKind::Minerals,
            free_space: 20,
            owner: Some(SwarmId::PLAYER),
        }];

        let leg = pick_logistics_leg(hauler(Vec2::ZERO), &stockpiles, &terminals).unwrap();

        assert_eq!(leg.source, e(1));
        assert_eq!(leg.sink, e(2));
    }

    #[test]
    fn owner_mismatch_rejects_source_or_sink() {
        let stockpiles = [StockpileCandidate {
            owner: Some(SwarmId(99)),
            ..source(1, Vec2::new(1.0, 0.0), 100)
        }];
        let terminals = [TerminalCandidate::Charger {
            entity: e(2),
            pos: Vec2::new(10.0, 0.0),
            kind: ResourceKind::Minerals,
            free_space: 100,
            owner: Some(SwarmId::PLAYER),
        }];

        assert!(pick_logistics_leg(hauler(Vec2::ZERO), &stockpiles, &terminals).is_none());
    }

    #[test]
    fn route_cost_ranking_includes_hauler_to_source_and_source_to_sink() {
        let stockpiles = [
            source(1, Vec2::new(10.0, 0.0), 100),
            source(2, Vec2::new(100.0, 0.0), 100),
            sink(3, Vec2::new(200.0, 0.0), 0, 100),
        ];
        let travel_cost = |from: Vec2, to: Vec2| {
            if from == Vec2::ZERO && to == Vec2::new(10.0, 0.0) {
                1.0
            } else if from == Vec2::new(10.0, 0.0) && to == Vec2::new(200.0, 0.0) {
                1_000.0
            } else if from == Vec2::ZERO && to == Vec2::new(100.0, 0.0) {
                100.0
            } else if from == Vec2::new(100.0, 0.0) && to == Vec2::new(200.0, 0.0) {
                100.0
            } else {
                from.distance(to)
            }
        };

        let leg = pick_logistics_leg_with_cost(hauler(Vec2::ZERO), &stockpiles, &[], travel_cost)
            .expect("route-cost leg must be picked");

        assert_eq!(leg.source, e(2));
        assert_eq!(leg.sink, e(3));
    }
}
