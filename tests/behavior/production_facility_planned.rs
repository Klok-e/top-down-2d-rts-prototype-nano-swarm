//! Planned Production Facility behavior under population demand.

use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::battle_statistics::BattleCounters;
use top_down_2d_rts_prototype_nano_swarm::tuning::Tuning;
use top_down_2d_rts_prototype_nano_swarm::{
    intent::{IntentGrid, IntentKind},
    nanobot::{
        Cargo, GatherPlugin, HaulPlugin, HaulerAssignment, InteractionRegion, LogisticsReservation,
        Nanobot, OwnerSwarm, PlannedKind, PlannedStructure, ProductionFacility, ProductionPressure,
        SwarmId, SwarmMember, completed_visual_color, planned_visual_color,
    },
    resources::{ResourceKind, Stockpile, StockpileRole},
    terrain::RockFormation,
};

#[path = "../common/mod.rs"]
mod common;

fn paint(app: &mut App, cell: IVec2, kind: IntentKind) {
    assert!(
        app.world_mut()
            .resource_mut::<IntentGrid>()
            .paint(cell, kind, SwarmId::PLAYER)
    );
}

fn production_plans(app: &mut App) -> Vec<Entity> {
    let world = app.world_mut();
    world
        .query::<(Entity, &PlannedStructure)>()
        .iter(world)
        .filter_map(|(entity, planned)| {
            (planned.kind == PlannedKind::ProductionFacility).then_some(entity)
        })
        .collect()
}

#[test]
fn zero_facilities_recovers_one_owned_plan_after_the_recovery_delay() {
    let mut app = common::sim_app_with_production_planned();
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    common::spawn_worker_at(&mut app, Vec2::new(-1024.0, -1024.0));
    paint(&mut app, IVec2::ZERO, IntentKind::Build);
    paint(&mut app, IVec2::X, IntentKind::Build);

    for _ in 0..Tuning::default().production.pressure_ticks - 1 {
        app.update();
    }
    assert_eq!(
        app.world()
            .resource::<ProductionPressure>()
            .ticks_for(SwarmId::PLAYER),
        Tuning::default().production.pressure_ticks - 1,
    );
    assert!(production_plans(&mut app).is_empty());

    for _ in 0..101 {
        app.update();
        if !production_plans(&mut app).is_empty() {
            break;
        }
    }
    let plans = production_plans(&mut app);
    assert_eq!(plans.len(), 1, "pending capacity prevents duplicate plans");
    let plan = app.world().entity(plans[0]);
    assert_eq!(plan.get::<OwnerSwarm>().map(|owner| owner.0), Some(swarm));
    assert_eq!(
        plan.get::<Sprite>().expect("planned visual").color,
        planned_visual_color(),
    );
    assert!(plan.get::<ProductionFacility>().is_none());
}

#[test]
fn demand_without_an_owned_build_zone_creates_no_facility_plan() {
    let mut app = common::sim_app_with_production_planned();
    common::spawn_swarm_at(&mut app, Vec2::ZERO);
    paint(&mut app, IVec2::ZERO, IntentKind::Gather);
    common::spawn_deposit(
        &mut app,
        common::DepositFixture {
            world_pos: common::cell_world_center(IVec2::ZERO),
            amount: 100,
            capacity: 100,
            radius: 32.0,
        },
    );

    for _ in 0..Tuning::default().production.pressure_ticks + 100 {
        app.update();
    }
    assert!(production_plans(&mut app).is_empty());
}

#[test]
fn worker_promotes_an_owned_plan_to_an_idle_unfunded_facility() {
    let mut app = common::sim_app_with_production_planned();
    app.init_resource::<BattleCounters>();
    let cell = IVec2::ZERO;
    let plan = common::spawn_planned_production_facility_at_cell(&mut app, cell);
    let owner = app.world().get::<OwnerSwarm>(plan).copied().unwrap();
    let center = common::cell_world_center(cell);
    common::spawn_worker_at(&mut app, center + Vec2::X * 72.0);

    for _ in 0..Tuning::default().construction.work_ticks + 250 {
        app.update();
        if app.world().get::<PlannedStructure>(plan).is_none() {
            break;
        }
    }

    let entity = app.world().entity(plan);
    assert!(entity.get::<PlannedStructure>().is_none());
    assert_eq!(
        entity.get::<OwnerSwarm>().map(|owner| owner.0),
        Some(owner.0)
    );
    let facility = entity
        .get::<ProductionFacility>()
        .expect("completed plan becomes a facility");
    assert_eq!(facility.current_target, None);
    assert_eq!(facility.input_amount, 0);
    assert_eq!(
        entity.get::<Sprite>().expect("completed visual").color,
        completed_visual_color(),
    );
    assert_eq!(
        app.world()
            .resource::<BattleCounters>()
            .totals_for(SwarmId::PLAYER)
            .structures_built,
        1
    );
}

const CYCLE_TICKS: u32 = 90;
const CYCLE_COST: u32 = 2;
const WINDOW_TICKS: u32 = 20 * top_down_2d_rts_prototype_nano_swarm::SIMULATION_HZ as u32;
const BUILD_CELL: IVec2 = IVec2::X;

struct Expansion {
    app: App,
    owner: Entity,
    facility: Entity,
    source: Entity,
    hauler: Entity,
}

impl Expansion {
    fn new(funding: u32) -> Self {
        let mut app = common::sim_app_with_production_planned();
        app.add_plugins((GatherPlugin, HaulPlugin));
        app.init_resource::<BattleCounters>();
        app.insert_resource(IntentGrid::new(12, 12));
        {
            let mut tuning = app.world_mut().resource_mut::<Tuning>();
            tuning.production.cost_per_bot = CYCLE_COST;
            tuning.production.ticks_per_bot = CYCLE_TICKS;
            tuning.logistics.worker_carry_capacity = 100;
            tuning.logistics.hauler_carry_capacity = 100;
            tuning.logistics.hauler_transfer_per_pulse = 100;
            tuning.logistics.material_work_ticks = 200;
            tuning.charge.drain_per_tick = 0.0;
        }
        let owner = common::spawn_swarm_at(&mut app, Vec2::ZERO);
        let facility = common::spawn_facility_at(&mut app, owner, Vec2::new(-396.0, 252.0));
        {
            let mut state = app
                .world_mut()
                .get_mut::<ProductionFacility>(facility)
                .unwrap();
            state.input_capacity = funding;
            state.input_amount = funding;
        }
        let source = common::spawn_stockpile(&mut app, Vec2::new(396.0, 252.0), 0, 100);
        app.world_mut().entity_mut(source).insert(OwnerSwarm(owner));
        let hauler = common::spawn_hauler_at(&mut app, Vec2::new(300.0, 252.0));
        // A live builder leaves room for every proposed facility to be physically built.
        common::spawn_worker_at(&mut app, Vec2::new(612.0, 252.0));
        Self {
            app,
            owner,
            facility,
            source,
            hauler,
        }
    }

    fn advance(&mut self, ticks: u32) {
        for _ in 0..ticks {
            self.app.update();
        }
    }

    fn dropoff(&mut self, source: Entity, amount: u32) {
        let transform = *self.app.world().get::<Transform>(source).unwrap();
        let center = transform.translation.truncate();
        let deposit = common::spawn_deposit(
            &mut self.app,
            common::DepositFixture {
                world_pos: center - 144.0 * Vec2::Y,
                amount: 1000,
                capacity: 1000,
                radius: 32.0,
            },
        );
        let work_position =
            InteractionRegion::structure(&transform).approach(center + Vec2::Y * 100.0);
        let owner = self.app.world().get::<OwnerSwarm>(source).unwrap().0;
        let swarm = *self.app.world().get::<SwarmId>(owner).unwrap();
        let worker = common::spawn_worker_at(&mut self.app, work_position);
        self.app
            .world_mut()
            .entity_mut(worker)
            .insert(SwarmMember(swarm));
        let existing = self.app.world().get::<Stockpile>(source).unwrap().amount;
        common::prepare_mineral_dropoff(&mut self.app, worker, deposit, source, amount);
        self.advance(1);
        assert_eq!(
            self.app.world().get::<Stockpile>(source).unwrap().amount,
            existing + amount
        );
        assert!(self.app.world().get::<Cargo>(worker).is_none());
        self.app.world_mut().despawn(deposit);
    }

    fn demand(&mut self) {
        paint(&mut self.app, BUILD_CELL, IntentKind::Build);
        self.paint_defend(SwarmId::PLAYER, -2..=0);
    }

    fn paint_defend(&mut self, swarm: SwarmId, rows: std::ops::RangeInclusive<i32>) {
        for x in -4..=4 {
            for y in rows.clone() {
                self.app.world_mut().resource_mut::<IntentGrid>().paint(
                    IVec2::new(x, y),
                    IntentKind::Defend,
                    swarm,
                );
            }
        }
    }

    fn set_funding(&mut self, facility: Entity, funding: u32) {
        let mut state = self
            .app
            .world_mut()
            .get_mut::<ProductionFacility>(facility)
            .unwrap();
        state.input_amount = funding;
    }

    fn observe_supply(&mut self, amount: u32) {
        self.advance(WINDOW_TICKS - 1);
        self.dropoff(self.source, amount);
    }

    fn births(&self) -> u64 {
        self.app
            .world()
            .resource::<BattleCounters>()
            .totals_for(SwarmId::PLAYER)
            .births
    }

    fn add_facility(&mut self, position: Vec2, funding: u32) -> Entity {
        let facility = common::spawn_facility_at(&mut self.app, self.owner, position);
        self.set_funding(facility, funding);
        facility
    }

    fn add_source(&mut self, position: Vec2, owner: Entity) -> Entity {
        let source = common::spawn_stockpile(&mut self.app, position, 0, 100);
        self.app
            .world_mut()
            .entity_mut(source)
            .insert((OwnerSwarm(owner), StockpileRole::Source));
        source
    }

    fn empty_source(&mut self, source: Entity) {
        self.app
            .world_mut()
            .get_mut::<Stockpile>(source)
            .unwrap()
            .amount = 0;
    }

    fn place_one(&mut self, owner: Entity) -> Entity {
        for _ in 0..150 {
            self.advance(1);
            let plans = production_plans(&mut self.app);
            if let Some(&plan) = plans
                .iter()
                .find(|&&plan| self.app.world().get::<OwnerSwarm>(plan).unwrap().0 == owner)
            {
                assert_eq!(
                    plans
                        .iter()
                        .filter(
                            |&&plan| self.app.world().get::<OwnerSwarm>(plan).unwrap().0 == owner
                        )
                        .count(),
                    1
                );
                return plan;
            }
        }
        let world = self.app.world_mut();
        let facilities = world
            .query::<(&ProductionFacility, &OwnerSwarm)>()
            .iter(world)
            .filter(|(_, facility_owner)| facility_owner.0 == owner)
            .map(|(facility, _)| facility.clone())
            .collect::<Vec<_>>();
        panic!("the funded facilities and supply did not authorize expansion: {facilities:?}");
    }

    fn put(&mut self, cell: IVec2, kind: IntentKind, swarm: SwarmId, painted: bool) {
        let mut grid = self.app.world_mut().resource_mut::<IntentGrid>();
        assert!(if painted {
            grid.paint(cell, kind, swarm)
        } else {
            grid.erase(cell, kind, swarm)
        });
    }

    fn assert_no_plan(&mut self) {
        assert!(production_plans(&mut self.app).is_empty());
    }

    fn set_cycle_ticks(&mut self, ticks: u32) {
        self.app
            .world_mut()
            .resource_mut::<Tuning>()
            .production
            .ticks_per_bot = ticks;
    }

    fn facility(&self) -> &ProductionFacility {
        self.app
            .world()
            .get::<ProductionFacility>(self.facility)
            .unwrap()
    }
}

#[test]
fn one_funded_cycle_followed_by_starvation_cannot_expand() {
    let mut game = Expansion::new(CYCLE_COST);
    game.advance(WINDOW_TICKS - 1);
    game.dropoff(game.source, 80);
    game.demand();

    game.advance(1);
    assert!(game.facility().current_target.is_some());
    assert_eq!(game.facility().input_amount, 0);
    game.advance(CYCLE_TICKS / 2 + 30);
    assert!(
        production_plans(&mut game.app).is_empty(),
        "one long funded cycle cannot authorize expansion",
    );

    game.advance(CYCLE_TICKS);
    assert!(game.facility().current_target.is_none());
    assert_eq!(game.facility().input_amount, 0);
    assert!(production_plans(&mut game.app).is_empty());
}

#[test]
fn staggered_facilities_qualify_only_after_both_fund_the_fourth_start() {
    let mut game = Expansion::new(100);
    let second = game.add_facility(Vec2::new(-684.0, 252.0), 0);
    game.observe_supply(100);
    game.demand();
    game.advance(13);
    assert_eq!(game.facility().progress, 13);
    game.set_funding(second, 80);
    game.advance(3 * CYCLE_TICKS);
    assert_eq!(game.births(), 6);
    assert!(
        game.app
            .world()
            .get::<ProductionFacility>(second)
            .unwrap()
            .current_target
            .is_none()
    );
    game.assert_no_plan();

    let plan = game.place_one(game.owner);
    let second = game.app.world().get::<ProductionFacility>(second).unwrap();
    assert!(second.current_target.is_some());
    assert_eq!(second.input_amount, 80 - 4 * CYCLE_COST);
    assert_eq!(
        game.app.world().get::<PlannedStructure>(plan).unwrap().cell,
        BUILD_CELL
    );
}

#[test]
fn one_facility_missing_its_fourth_start_defers_the_swarm() {
    let mut game = Expansion::new(100);
    let second = game.add_facility(Vec2::new(-684.0, 252.0), 3 * CYCLE_COST);
    game.observe_supply(100);
    game.demand();
    game.advance(3 * CYCLE_TICKS);
    assert_eq!(game.births(), 6);
    game.advance(7);
    assert!(game.facility().current_target.is_some());
    let second = game.app.world().get::<ProductionFacility>(second).unwrap();
    assert_eq!(second.input_amount, 0);
    assert!(second.current_target.is_none());
    game.assert_no_plan();
}

#[test]
fn an_exit_block_restarts_the_funded_streak() {
    let mut game = Expansion::new(100);
    game.observe_supply(80);
    game.demand();
    game.advance(3 * CYCLE_TICKS - 1);
    assert_eq!(game.births(), 2);
    let center = game
        .app
        .world()
        .get::<Transform>(game.facility)
        .unwrap()
        .translation;
    let mut blockers = Vec::new();
    for y in -1..=1 {
        for x in -1..=1 {
            if (x, y) != (0, 0) {
                let position = center + Vec3::new(x as f32, y as f32, 0.0) * 72.0;
                blockers.push(
                    game.app
                        .world_mut()
                        .spawn((Nanobot {}, Transform::from_translation(position)))
                        .id(),
                );
            }
        }
    }
    game.advance(5);
    assert_eq!(game.births(), 2, "the third funded output is retained");
    assert!(game.facility().finished_at.is_some());
    game.assert_no_plan();
    for blocker in blockers {
        game.app.world_mut().despawn(blocker);
    }
    game.advance(1);
    assert_eq!(game.births(), 3);
    game.advance(3 * CYCLE_TICKS);
    assert_eq!(game.births(), 6);
    game.assert_no_plan();

    game.place_one(game.owner);
    assert!(game.facility().current_target.is_some());
}

#[test]
fn funded_output_covers_remaining_demand_and_later_demand_needs_fresh_cycles() {
    let mut game = Expansion::new(100);
    game.observe_supply(80);
    paint(&mut game.app, BUILD_CELL, IntentKind::Build);
    for x in -3..=3 {
        paint(&mut game.app, IVec2::new(x, -1), IntentKind::Defend);
    }
    game.advance(3 * CYCLE_TICKS + 10);
    assert_eq!(game.births(), 3);
    assert!(
        game.facility().current_target.is_some(),
        "the fourth Nanobot is already funded"
    );
    game.assert_no_plan();

    game.paint_defend(SwarmId::PLAYER, -2..=0);
    game.advance(4 * CYCLE_TICKS - 10);
    assert_eq!(game.births(), 7);
    game.assert_no_plan();
    game.place_one(game.owner);
}

#[test]
fn expansion_requires_a_complete_window_even_after_the_fourth_start() {
    let mut game = Expansion::new(100);
    let cycle_ticks = 300;
    game.set_cycle_ticks(cycle_ticks);
    game.demand();
    game.advance(3 * cycle_ticks + 6);
    assert!(game.facility().current_target.is_some());
    game.dropoff(game.source, 80);
    game.advance(WINDOW_TICKS - 3 * cycle_ticks - 8);
    game.assert_no_plan();

    game.place_one(game.owner);
}

#[test]
fn worker_dropoffs_expire_while_the_minerals_remain_in_storage() {
    let mut game = Expansion::new(100);
    game.dropoff(game.source, 80);
    game.advance(WINDOW_TICKS);
    assert_eq!(
        game.app
            .world()
            .get::<Stockpile>(game.source)
            .unwrap()
            .amount,
        80
    );
    game.demand();
    game.advance(3 * CYCLE_TICKS + 30);
    assert!(game.births() >= 3);
    assert!(game.facility().current_target.is_some());
    game.assert_no_plan();
}

#[test]
fn existing_facilities_take_their_full_share_of_replenishment() {
    let mut game = Expansion::new(100);
    game.add_facility(Vec2::new(-684.0, 252.0), 80);
    game.observe_supply(70);
    game.demand();
    game.advance(3 * CYCLE_TICKS + 20);
    assert_eq!(game.births(), 6);
    game.assert_no_plan();
}

#[test]
fn defender_charge_upkeep_takes_its_share_of_replenishment() {
    let mut game = Expansion::new(100);
    game.app
        .world_mut()
        .resource_mut::<Tuning>()
        .charge
        .drain_per_tick = 0.03;
    let cell = IVec2::new(0, -1);
    let charger = common::spawn_charger(
        &mut game.app,
        common::ChargerFixture {
            cell,
            amount: 10,
            ticks_since_maintained: 0,
        },
    );
    game.app
        .world_mut()
        .entity_mut(charger)
        .insert(OwnerSwarm(game.owner));
    let defender = common::spawn_defender_at(&mut game.app, Vec2::new(500.0, -160.0));
    // Keep the Swarm out of demand until a full window has been observed.
    game.observe_supply(80);
    game.demand();
    game.advance(3 * CYCLE_TICKS + 25);
    assert_eq!(game.births(), 3);
    assert!(game.app.world().get_entity(defender).is_ok());
    game.assert_no_plan();
}

#[test]
fn ample_source_income_does_not_replace_idle_hauler_time() {
    let mut game = Expansion::new(100);
    let far_pickup = game.add_source(Vec2::new(-2280.0, 1536.0), game.owner);
    game.app
        .world_mut()
        .get_mut::<Stockpile>(far_pickup)
        .unwrap()
        .amount = 100;
    game.app
        .world_mut()
        .resource_mut::<Tuning>()
        .logistics
        .bot_speed = 0.05;
    game.app.world_mut().entity_mut(game.hauler).insert((
        HaulerAssignment {
            source: far_pickup,
            sink: game.facility,
            pickup_role: StockpileRole::Source,
        },
        LogisticsReservation::new(far_pickup, game.facility, ResourceKind::Minerals, 30),
    ));
    game.observe_supply(80);
    game.demand();
    game.advance(3 * CYCLE_TICKS);
    game.app
        .world_mut()
        .resource_mut::<Tuning>()
        .logistics
        .bot_speed = 5.0;
    game.advance(20);
    assert_eq!(game.births(), 3);
    assert!(
        game.app
            .world()
            .get::<HaulerAssignment>(game.hauler)
            .is_some()
    );
    game.assert_no_plan();
}

#[test]
fn adequate_supply_elsewhere_cannot_replace_site_reachable_spare_replenishment() {
    let mut game = Expansion::new(100);
    game.set_cycle_ticks(120);
    game.app.insert_resource(IntentGrid::new(12, 2));
    game.app
        .world_mut()
        .get_mut::<Transform>(game.facility)
        .unwrap()
        .translation
        .x = 108.0;
    game.app.world_mut().spawn((
        RockFormation::Rectangle {
            half: Vec2::new(18.0, 512.0),
        },
        Transform::from_xyz(0.0, 0.0, 0.0),
    ));
    let disconnected = game.add_source(Vec2::new(-396.0, 252.0), game.owner);
    game.observe_supply(20);
    game.dropoff(disconnected, 60);
    game.demand();
    game.advance(3 * 120 + 20);
    assert_eq!(game.births(), 3);
    game.assert_no_plan();

    game.app.insert_resource(
        top_down_2d_rts_prototype_nano_swarm::navigation_runtime::NavigationBudget(98_304),
    );
    game.dropoff(game.source, 20);
    game.place_one(game.owner);
}

#[test]
fn a_long_haul_needs_both_travel_legs_and_the_search_tries_other_sites() {
    use top_down_2d_rts_prototype_nano_swarm::nanobot::find_build_zone_placement;

    let mut game = Expansion::new(100);
    game.app
        .world_mut()
        .resource_mut::<Tuning>()
        .logistics
        .hauler_carry_capacity = 20;
    let far = IVec2::new(4, 0);
    let first = find_build_zone_placement(
        &[far, BUILD_CELL],
        &[],
        PlannedKind::ProductionFacility.footprint_half_extent(),
        27,
    )
    .unwrap();
    assert_eq!(
        first.0, far,
        "the more distant site must be considered first"
    );
    game.observe_supply(80);
    game.empty_source(game.source);
    game.put(far, IntentKind::Build, SwarmId::PLAYER, true);
    game.paint_defend(SwarmId::PLAYER, -2..=0);
    game.advance(3 * CYCLE_TICKS + 30);
    assert_eq!(game.births(), 3);
    game.assert_no_plan();

    game.put(BUILD_CELL, IntentKind::Build, SwarmId::PLAYER, true);
    let plan = game.place_one(game.owner);
    assert_eq!(
        game.app.world().get::<PlannedStructure>(plan).unwrap().cell,
        BUILD_CELL
    );
}

#[test]
fn separate_sources_and_haulers_share_one_exact_mineral_budget() {
    for haulers in [1, 2] {
        let mut game = Expansion::new(100);
        game.set_cycle_ticks(120);
        {
            let mut tuning = game.app.world_mut().resource_mut::<Tuning>();
            tuning.logistics.hauler_carry_capacity = 10;
            tuning.logistics.material_work_ticks = 400;
        }
        if haulers == 2 {
            common::spawn_hauler_at(&mut game.app, Vec2::new(500.0, 120.0));
        }
        let north = game.add_source(Vec2::new(396.0, 396.0), game.owner);
        let south = game.add_source(Vec2::new(396.0, 108.0), game.owner);
        game.observe_supply(18);
        game.empty_source(game.source);
        game.dropoff(north, 11);
        game.empty_source(north);
        game.dropoff(south, 11);
        game.empty_source(south);
        game.demand();
        game.advance(3 * 120 + 20);
        if haulers == 1 {
            game.assert_no_plan();
        } else {
            let plan = game.place_one(game.owner);
            assert_eq!(
                game.app.world().get::<PlannedStructure>(plan).unwrap().cell,
                BUILD_CELL
            );
        }
    }
}

#[test]
fn expansion_requires_the_current_tuned_mineral_rate() {
    let mut game = Expansion::new(100);
    game.app
        .world_mut()
        .resource_mut::<Tuning>()
        .production
        .cost_per_bot *= 2;
    game.observe_supply(80);
    game.demand();
    game.advance(3 * CYCLE_TICKS + 20);
    assert_eq!(game.births(), 3);
    assert!(game.facility().current_target.is_some());
    game.assert_no_plan();
}

#[test]
fn swarms_need_their_own_supply_and_each_hold_one_commitment() {
    let mut game = Expansion::new(100);
    let enemy = SwarmId(2);
    let enemy_owner = common::spawn_swarm_at(&mut game.app, Vec2::new(-1000.0, 900.0));
    game.app.world_mut().entity_mut(enemy_owner).insert(enemy);
    common::spawn_facility_at(&mut game.app, enemy_owner, Vec2::new(-1260.0, 900.0));
    let enemy_source = game.add_source(Vec2::new(-900.0, 900.0), enemy_owner);
    let hauler = common::spawn_hauler_at(&mut game.app, Vec2::new(-900.0, 780.0));
    let worker = common::spawn_worker_at(&mut game.app, Vec2::new(-900.0, 1150.0));
    for bot in [hauler, worker] {
        game.app
            .world_mut()
            .entity_mut(bot)
            .insert(SwarmMember(enemy));
    }
    game.observe_supply(80);
    game.demand();
    let enemy_cell = IVec2::new(-2, 2);
    game.put(enemy_cell, IntentKind::Build, enemy, true);
    game.paint_defend(enemy, 2..=4);
    game.advance(3 * CYCLE_TICKS);
    let player_plan = game.place_one(game.owner);
    assert_eq!(production_plans(&mut game.app), [player_plan]);

    game.dropoff(enemy_source, 80);
    let enemy_plan = game.place_one(enemy_owner);
    assert_eq!(
        game.app
            .world()
            .get::<PlannedStructure>(enemy_plan)
            .unwrap()
            .cell,
        enemy_cell
    );
    game.advance(50);
    let plans = production_plans(&mut game.app);
    assert_eq!(plans.len(), 2);
    assert!(plans.contains(&player_plan) && plans.contains(&enemy_plan));
}

#[test]
fn expansion_that_loses_supply_still_finishes_construction() {
    let mut game = Expansion::new(100);
    game.app
        .world_mut()
        .resource_mut::<Tuning>()
        .construction
        .work_ticks = 10;
    game.observe_supply(80);
    game.demand();
    game.advance(3 * CYCLE_TICKS);
    let plan = game.place_one(game.owner);
    game.app.world_mut().despawn(game.source);
    for _ in 0..400 {
        game.advance(1);
        if game.app.world().get::<ProductionFacility>(plan).is_some() {
            break;
        }
    }
    assert_eq!(
        game.app.world().get::<OwnerSwarm>(plan).unwrap().0,
        game.owner
    );
    assert!(game.app.world().get::<ProductionFacility>(plan).is_some());
    game.assert_no_plan();
}

#[test]
fn cancelled_expansion_earns_fresh_cycles_and_retains_recent_supply_evidence() {
    use top_down_2d_rts_prototype_nano_swarm::nanobot::StructureClearing;

    let mut game = Expansion::new(100);
    game.observe_supply(80);
    game.demand();
    game.advance(3 * CYCLE_TICKS);
    let first = game.place_one(game.owner);
    let spot = game
        .app
        .world()
        .get::<Transform>(first)
        .unwrap()
        .translation
        .truncate();
    let builder_position = spot + 72.0 * Vec2::Y;
    game.app.world_mut().entity_mut(first).insert((
        PlannedStructure::new(PlannedKind::ProductionFacility, BUILD_CELL).with_work_remaining(0),
        StructureClearing::awaiting_validation(builder_position),
    ));
    common::spawn_structure_at(&mut game.app, builder_position);
    for _ in 0..100 {
        game.advance(1);
        if game.app.world().get_entity(first).is_err() {
            break;
        }
    }
    assert!(game.app.world().get_entity(first).is_err());

    game.advance(3 * CYCLE_TICKS);
    game.assert_no_plan();
    let second = game.place_one(game.owner);
    assert_ne!(first, second);
}

#[test]
fn a_completed_expansion_requalifies_both_facilities_with_rolling_supply() {
    let mut game = Expansion::new(100);
    game.app
        .world_mut()
        .resource_mut::<Tuning>()
        .construction
        .work_ticks = 10;
    game.observe_supply(100);
    game.demand();
    game.advance(3 * CYCLE_TICKS);
    let first = game.place_one(game.owner);
    for _ in 0..350 {
        game.advance(1);
        if game.app.world().get::<ProductionFacility>(first).is_some() {
            break;
        }
    }
    assert!(game.app.world().get::<ProductionFacility>(first).is_some());
    common::fill_facility_input(&mut game.app, first);
    game.put(IVec2::ZERO, IntentKind::Build, SwarmId::PLAYER, true);
    game.app.insert_resource(
        top_down_2d_rts_prototype_nano_swarm::navigation_runtime::NavigationBudget(98_304),
    );

    game.advance(3 * CYCLE_TICKS);
    game.assert_no_plan();
    let second = game.place_one(game.owner);
    assert_ne!(first, second);
    assert!(game.app.world().get::<ProductionFacility>(first).is_some());
}

#[test]
fn an_earlier_candidate_can_qualify_through_a_budgeted_detour() {
    use top_down_2d_rts_prototype_nano_swarm::nanobot::find_build_zone_placement;

    let mut game = Expansion::new(100);
    let far = IVec2::new(4, 0);
    let first = find_build_zone_placement(
        &[far, BUILD_CELL],
        &[],
        PlannedKind::ProductionFacility.footprint_half_extent(),
        27,
    )
    .unwrap();
    assert_eq!(first.0, far);
    game.app.world_mut().spawn((
        RockFormation::Rectangle {
            half: Vec2::new(36.0, 400.0),
        },
        Transform::from_xyz(1400.0, 252.0, 0.0),
    ));
    game.observe_supply(80);
    game.demand();
    game.put(far, IntentKind::Build, SwarmId::PLAYER, true);
    game.advance(3 * CYCLE_TICKS);
    let plan = game.place_one(game.owner);
    assert_eq!(
        game.app.world().get::<PlannedStructure>(plan).unwrap().cell,
        far
    );
}
