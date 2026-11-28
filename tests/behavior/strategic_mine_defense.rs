#[path = "../common/mod.rs"]
mod common;

use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    SIMULATION_HZ,
    intent::{IntentGrid, IntentKind},
    nanobot::{
        CombatPlugin, DefenderAttackCooldown, ExtractProgress, GatherAssignment, Health,
        InteractionRegion, LogisticsReservation, OwnerSwarm, PlannedKind, Structure, StructureKind,
        SwarmId, SwarmMember, worker_gather_delivery_system,
    },
    reachability::SwarmReachabilityMap,
    resources::{ResourceKind, Stockpile},
    strategic_runtime::{StrategicController, StrategicControllerPlugin},
    tuning::Tuning,
};

const OWNER: SwarmId = SwarmId::PLAYER;
const HOME_MINE: IVec2 = IVec2::new(0, 2);
const OUTPOST: IVec2 = IVec2::new(4, 4);
const FRONT: IVec2 = IVec2::new(8, 0);
const REVIEW_TICKS: u64 = 30;

struct Fixture {
    app: App,
    own: Entity,
    enemy: SwarmId,
    mines: [(IVec2, Entity); 2],
}

impl Fixture {
    fn new(reverse: bool) -> Self {
        let mut app = common::minimal_app();
        app.insert_resource(IntentGrid::new(24, 24))
            .add_plugins((StrategicControllerPlugin, CombatPlugin))
            .add_systems(FixedUpdate, worker_gather_delivery_system);
        let home = common::cell_world_center(IVec2::ZERO);
        let own = common::spawn_swarm_at(&mut app, home);
        app.world_mut()
            .entity_mut(own)
            .insert(StrategicController::new(OWNER));
        let enemy_home = common::cell_world_center(FRONT);
        let enemy_entity = common::spawn_opponent_swarm_with_nanobots(&mut app, enemy_home, &[]);
        let enemy = *app.world().get::<SwarmId>(enemy_entity).unwrap();
        let reachability = SwarmReachabilityMap::new(
            app.world().resource::<IntentGrid>(),
            [],
            [(OWNER, home), (enemy, enemy_home)],
        )
        .unwrap();
        app.insert_resource(reachability);
        let target = common::spawn_empty_completed_structure(
            &mut app,
            enemy_entity,
            PlannedKind::SinkStockpile,
        );
        app.world_mut()
            .entity_mut(target)
            .insert(Transform::from_translation(enemy_home.extend(0.0)));
        for row in [-1.0, 0.0, 1.0] {
            common::spawn_stationary_defender_at(&mut app, home + Vec2::Y * row * 60.0, OWNER);
        }
        let mut cells = [HOME_MINE, OUTPOST];
        if reverse {
            cells.reverse();
        }
        let mines = cells.map(|cell| {
            app.world_mut()
                .resource_mut::<IntentGrid>()
                .paint(cell, IntentKind::Gather, OWNER);
            let deposit = common::spawn_deposit(
                &mut app,
                common::DepositFixture {
                    world_pos: common::cell_world_center(cell),
                    amount: 72_000,
                    capacity: 72_000,
                    radius: 64.0,
                },
            );
            (cell, deposit)
        });
        let mut tuning = app.world_mut().resource_mut::<Tuning>();
        tuning.autonomy.controller_review_period_ticks = REVIEW_TICKS;
        tuning.autonomy.strategy_staging_spacing_cells = 64;
        tuning.combat.attack_interval_ticks = 600;
        let mut fixture = Self {
            app,
            own,
            enemy,
            mines,
        };
        fixture.advance(REVIEW_TICKS);
        assert!(
            fixture.has_front(FRONT),
            "the opening offensive front must be in place"
        );
        fixture
    }

    fn advance(&mut self, ticks: u64) {
        for _ in 0..ticks {
            self.app.update();
        }
    }

    fn has_front(&self, cell: IVec2) -> bool {
        self.owns_defend(cell + IVec2::Y)
    }

    fn owns_defend(&self, cell: IVec2) -> bool {
        self.app
            .world()
            .resource::<IntentGrid>()
            .cell(cell)
            .is_some_and(|intent| intent.has_owned(IntentKind::Defend, OWNER))
    }

    fn passer_at(&mut self, cell: IVec2) {
        let worker = common::spawn_worker_at(&mut self.app, common::cell_world_center(cell));
        self.app
            .world_mut()
            .entity_mut(worker)
            .insert(SwarmMember(self.enemy));
    }

    fn attack(&mut self, cell: IVec2, attackers: usize) -> (Entity, Vec<Entity>) {
        let position = common::cell_world_center(cell);
        let victim = common::spawn_worker_at(&mut self.app, position + Vec2::Y * 110.0);
        let intruders = (0..attackers)
            .map(|index| {
                let intruder = common::spawn_stationary_defender_at(
                    &mut self.app,
                    position + Vec2::new(88.0 + index as f32 * 12.0, 90.0),
                    self.enemy,
                );
                self.app
                    .world_mut()
                    .entity_mut(intruder)
                    .insert(DefenderAttackCooldown::new(0));
                intruder
            })
            .collect();
        (victim, intruders)
    }

    fn deposit(&self, cell: IVec2) -> Entity {
        self.mines
            .into_iter()
            .find_map(|(candidate, deposit)| (candidate == cell).then_some(deposit))
            .unwrap()
    }

    fn deliver_minerals(&mut self, cell: IVec2) {
        let position = common::cell_world_center(cell);
        let source = common::spawn_stockpile(&mut self.app, position - 170.0 * Vec2::X, 0, 120);
        self.app
            .world_mut()
            .entity_mut(source)
            .insert(OwnerSwarm(self.own));
        let approach =
            InteractionRegion::structure(self.app.world().get::<Transform>(source).unwrap())
                .approach(position);
        let worker = common::spawn_worker_at(&mut self.app, approach);
        let deposit = self.deposit(cell);
        common::prepare_mineral_dropoff(&mut self.app, worker, deposit, source, 4);
        self.advance(4);
        assert_eq!(self.app.world().get::<Stockpile>(source).unwrap().amount, 4);
        self.app.world_mut().despawn(worker);
    }
}

#[test]
fn active_attack_on_the_outpost_takes_precedence_over_a_passing_worker_at_home() {
    for reverse in [false, true] {
        let mut fixture = Fixture::new(reverse);
        if reverse {
            fixture.passer_at(HOME_MINE);
        }
        let (victim, _) = fixture.attack(OUTPOST, 1);
        if !reverse {
            fixture.passer_at(HOME_MINE);
        }
        fixture.advance(REVIEW_TICKS);
        assert!(fixture.app.world().get::<Health>(victim).unwrap().current < Health::default().max);
        assert!(
            fixture.has_front(OUTPOST),
            "an attack on the outpost should redirect the front"
        );
        assert!(!fixture.has_front(HOME_MINE));
        assert!(!fixture.has_front(FRONT));
    }
}

#[test]
fn simultaneous_attacks_prioritize_damage_then_the_mines_recent_output() {
    for reverse in [false, true] {
        for (outpost_attackers, defended) in [(1, HOME_MINE), (2, OUTPOST)] {
            let mut fixture = Fixture::new(reverse);
            fixture.deliver_minerals(HOME_MINE);
            let mut attacks = [(HOME_MINE, 1), (OUTPOST, outpost_attackers)];
            if reverse {
                attacks.reverse();
            }
            for (cell, attackers) in attacks {
                fixture.attack(cell, attackers);
            }
            fixture.advance(REVIEW_TICKS);
            assert_eq!(fixture.has_front(HOME_MINE), defended == HOME_MINE);
            assert_eq!(fixture.has_front(OUTPOST), defended == OUTPOST);
        }
    }
}

#[test]
fn delivery_stall_requires_sustained_hostile_pressure_to_redirect_the_front() {
    for (defender_offset, should_defend) in [
        (None, false),
        (Some(Vec2::splat(230.0)), false),
        (Some(Vec2::X * 175.0), true),
    ] {
        let mut fixture = Fixture::new(false);
        fixture.deliver_minerals(OUTPOST);
        if let Some(offset) = defender_offset {
            let position = common::cell_world_center(OUTPOST) + offset;
            common::spawn_stationary_defender_at(&mut fixture.app, position, fixture.enemy);
        }
        fixture.advance(REVIEW_TICKS);
        assert!(
            fixture.has_front(FRONT),
            "brief presence with no attack should hold the front"
        );
        fixture.advance(6 * SIMULATION_HZ as u64);
        assert_eq!(fixture.has_front(OUTPOST), should_defend);
        assert_eq!(fixture.has_front(FRONT), !should_defend);
    }
}

#[test]
fn live_attack_takes_precedence_over_a_heavier_resolved_attack() {
    let mut fixture = Fixture::new(false);
    let (victim, departed) = fixture.attack(OUTPOST, 2);
    fixture.attack(HOME_MINE, 1);
    fixture.advance(2);
    assert!(fixture.app.world().get::<Health>(victim).unwrap().current < Health::default().max);
    for intruder in departed {
        fixture.app.world_mut().despawn(intruder);
    }
    fixture.advance(REVIEW_TICKS);
    assert!(fixture.has_front(HOME_MINE));
    assert!(!fixture.has_front(OUTPOST));
}

#[test]
fn attack_on_a_serving_source_outside_the_mine_paint_is_attributed_to_its_mine() {
    let mut fixture = Fixture::new(false);
    let position = common::cell_world_center(OUTPOST);
    let mine = fixture.deposit(OUTPOST);
    let source_position = position + 285.0 * Vec2::Y;
    let source = common::spawn_stockpile(&mut fixture.app, source_position, 0, 120);
    fixture.app.world_mut().entity_mut(source).insert((
        OwnerSwarm(fixture.own),
        Structure::new(StructureKind::Basic),
    ));
    let worker = common::spawn_worker_at(&mut fixture.app, position - 90.0 * Vec2::Y);
    fixture.app.world_mut().entity_mut(worker).insert((
        GatherAssignment::new(OUTPOST, mine),
        ExtractProgress::default(),
        LogisticsReservation::new(mine, source, ResourceKind::Minerals, 4),
    ));
    fixture.advance(1);
    let intruder = common::spawn_stationary_defender_at(
        &mut fixture.app,
        source_position + 100.0 * Vec2::Y,
        fixture.enemy,
    );
    fixture
        .app
        .world_mut()
        .entity_mut(intruder)
        .insert(DefenderAttackCooldown::new(0));
    let health = fixture.app.world().get::<Structure>(source).unwrap().health;
    fixture.advance(REVIEW_TICKS);
    assert!(fixture.app.world().get::<Structure>(source).unwrap().health < health);
    assert!(fixture.owns_defend(OUTPOST - IVec2::Y));
    assert!(!fixture.has_front(FRONT));
}

#[test]
fn pressure_covering_the_same_mine_continues_across_a_cell_edge() {
    let mut fixture = Fixture::new(false);
    let center = common::cell_world_center(OUTPOST) + Vec2::Y * 220.0;
    let deposit = fixture.deposit(OUTPOST);
    fixture
        .app
        .world_mut()
        .get_mut::<Transform>(deposit)
        .unwrap()
        .translation = center.extend(0.0);
    fixture.deliver_minerals(OUTPOST);
    let intruder = common::spawn_stationary_defender_at(
        &mut fixture.app,
        center + Vec2::X * 130.0,
        fixture.enemy,
    );
    fixture.advance(3 * SIMULATION_HZ as u64);
    assert!(fixture.has_front(FRONT));
    fixture
        .app
        .world_mut()
        .get_mut::<Transform>(intruder)
        .unwrap()
        .translation = (center + Vec2::new(130.0, 80.0)).extend(0.0);
    fixture.advance(3 * SIMULATION_HZ as u64 + REVIEW_TICKS);
    assert!(fixture.owns_defend(OUTPOST - IVec2::Y));
    assert!(!fixture.has_front(FRONT));
}

#[test]
fn a_stall_with_a_defender_outside_the_painted_work_area_stays_economic() {
    let mut fixture = Fixture::new(false);
    let center = common::cell_world_center(OUTPOST) + Vec2::X * 220.0;
    let deposit = fixture.deposit(OUTPOST);
    fixture
        .app
        .world_mut()
        .get_mut::<Transform>(deposit)
        .unwrap()
        .translation = center.extend(0.0);
    fixture.deliver_minerals(OUTPOST);
    common::spawn_stationary_defender_at(&mut fixture.app, center + Vec2::X * 180.0, fixture.enemy);
    fixture.advance(6 * SIMULATION_HZ as u64 + REVIEW_TICKS);
    assert!(!fixture.owns_defend(OUTPOST - IVec2::ONE));
}

#[test]
fn an_attack_that_prevents_the_first_dropoff_holds_defense_during_occupation() {
    let mut fixture = Fixture::new(false);
    let (victim, occupying_force) = fixture.attack(OUTPOST, 1);
    let mine = fixture.deposit(OUTPOST);
    fixture.app.world_mut().entity_mut(victim).insert((
        GatherAssignment::new(OUTPOST, mine),
        ExtractProgress::default(),
    ));
    fixture
        .app
        .world_mut()
        .get_mut::<Health>(victim)
        .unwrap()
        .current = 1;
    fixture.advance(REVIEW_TICKS);
    assert_eq!(
        fixture.app.world().get::<Health>(victim).unwrap().current,
        0
    );
    assert!(fixture.has_front(OUTPOST));
    fixture.advance(3 * SIMULATION_HZ as u64);
    assert!(fixture.has_front(OUTPOST));
    assert!(!fixture.has_front(FRONT));

    for defender in occupying_force {
        fixture.app.world_mut().despawn(defender);
    }
    fixture.advance(REVIEW_TICKS);
    assert!(fixture.has_front(FRONT));
    assert!(!fixture.has_front(OUTPOST));
}
