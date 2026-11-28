//! Scripted playtests for ADR-0009 regional allocation: exhausted Gather Zones
//! persist as intent without invalid worker retries, and claimed work does not
//! strand idle Workers.

use bevy::{math::Vec2, prelude::*};
use top_down_2d_rts_prototype_nano_swarm::{
    intent::{IntentGrid, IntentKind},
    nanobot::{
        ActionableProjection, Cargo, Commitment, GatherAssignment, GatherQuotas, HaulPlugin,
        LogisticsReservation, NanobotType, OpportunityTarget, OwnerSwarm, PlannedKind,
        PlannedStructure, PopulationDemand, PopulationDemandPlugin, ProductionFacility,
        ProductionPlugin, RegionalLease, SwarmId, SwarmMember,
    },
    resources::{ResourceDeposit, Stockpile},
    tuning::Tuning,
};

#[path = "../common/mod.rs"]
mod common;

fn paint_gather(app: &mut App, cell: IVec2) {
    let mut grid = app.world_mut().resource_mut::<IntentGrid>();
    assert!(grid.paint(cell, IntentKind::Gather, SwarmId::PLAYER));
}

#[test]
fn exhausted_gather_zone_persists_without_invalid_worker_retries() {
    let mut app = common::sim_app_with_gather();
    let gather_cell = IVec2::new(0, 0);
    let deposit_pos = common::cell_world_center(gather_cell);
    let deposit = common::spawn_deposit(
        &mut app,
        common::DepositFixture {
            world_pos: deposit_pos,
            amount: 1,
            capacity: 1000,
            radius: 32.0,
        },
    );
    let _stockpile = common::spawn_stockpile(&mut app, deposit_pos + Vec2::new(96.0, 0.0), 0, 100);
    let worker = common::spawn_worker_at(&mut app, deposit_pos - Vec2::new(68.0, 0.0));
    paint_gather(&mut app, gather_cell);

    for _ in 0..300 {
        app.update();
    }

    let world = app.world();
    assert_eq!(
        world
            .entity(deposit)
            .get::<ResourceDeposit>()
            .unwrap()
            .amount,
        0,
        "deposit should be exhausted by the worker"
    );
    assert!(
        world
            .resource::<IntentGrid>()
            .cell(gather_cell)
            .is_some_and(|cell| cell.has(IntentKind::Gather)),
        "Gather intent persists after local depletion"
    );
    assert!(
        world.entity(worker).get::<GatherAssignment>().is_none(),
        "worker should not keep a stale GatherAssignment for exhausted deposit"
    );
    assert!(
        world.entity(worker).get::<RegionalLease>().is_none(),
        "regional Gather lease should be revoked once supporting work disappears"
    );

    for _ in 0..300 {
        app.update();
    }

    let world = app.world();
    assert!(
        world.entity(worker).get::<GatherAssignment>().is_none(),
        "worker must not repeatedly reclaim exhausted persistent Gather intent"
    );
    assert!(
        world.entity(worker).get::<RegionalLease>().is_none(),
        "worker must not repeatedly reacquire a lease for invalid Gather work"
    );
}

#[test]
fn lone_idle_worker_gathers_while_distant_construction_is_claimed() {
    let mut app = common::sim_app_with_gather_planned();
    app.insert_resource(IntentGrid::new(32, 32));

    // A Worker in another allocation region claims long-running construction.
    let plan_cell = IVec2::new(9, 1);
    let plan = common::spawn_planned_charger_at_cell(&mut app, plan_cell);
    app.world_mut()
        .entity_mut(plan)
        .insert(PlannedStructure::new(PlannedKind::Charger, plan_cell).with_work_budget(100_000));
    let builder = common::spawn_worker_at(
        &mut app,
        common::cell_world_center(plan_cell) - Vec2::X * 68.0,
    );
    for _ in 0..120 {
        if app
            .world()
            .get::<PlannedStructure>(plan)
            .unwrap()
            .active_worker()
            == Some(builder)
        {
            break;
        }
        app.update();
    }
    assert_eq!(
        app.world()
            .get::<PlannedStructure>(plan)
            .unwrap()
            .active_worker(),
        Some(builder),
        "setup: the builder should claim the construction"
    );

    // A second Worker, alone in its own region, beside painted minerals.
    let gather_cell = IVec2::new(1, 1);
    let deposit_pos = common::cell_world_center(gather_cell);
    common::spawn_deposit(
        &mut app,
        common::DepositFixture {
            world_pos: deposit_pos,
            amount: 1000,
            capacity: 1000,
            radius: 32.0,
        },
    );
    common::spawn_stockpile(&mut app, deposit_pos + Vec2::new(96.0, 0.0), 0, 100);
    paint_gather(&mut app, gather_cell);
    let gatherer = common::spawn_worker_at(&mut app, deposit_pos - Vec2::new(68.0, 0.0));

    for _ in 0..60 {
        app.update();
    }

    let world = app.world();
    assert_eq!(
        world.get::<PlannedStructure>(plan).unwrap().active_worker(),
        Some(builder),
        "the construction should stay claimed by its own Worker"
    );
    assert!(
        world.entity(gatherer).contains::<GatherAssignment>(),
        "a lone idle Worker must not wait on construction another Worker already claimed"
    );
}

struct SpareGatherFixture {
    app: App,
    owner: Entity,
    center: Vec2,
    deposit: Entity,
    source: Entity,
    required: Entity,
    spare_worker: Entity,
}

impl SpareGatherFixture {
    fn new(swarm: SwarmId, source_loads: u32, spare_offset: Vec2) -> Self {
        Self::with_layout(
            swarm,
            IVec2::ZERO,
            common::cell_world_center(IVec2::ZERO),
            Vec2::new(0.0, 160.0),
            source_loads,
            spare_offset,
        )
    }

    fn with_layout(
        swarm: SwarmId,
        cell: IVec2,
        center: Vec2,
        source_offset: Vec2,
        source_loads: u32,
        spare_offset: Vec2,
    ) -> Self {
        let mut app = common::sim_app_with_gather_planned();
        app.add_plugins((PopulationDemandPlugin, ProductionPlugin, HaulPlugin));
        let mut tuning = app.world_mut().resource_mut::<Tuning>();
        tuning.logistics.material_work_ticks = 12;
        tuning.logistics.hauler_carry_capacity = tuning.logistics.worker_carry_capacity;
        let owner = common::spawn_swarm_at(&mut app, center - Vec2::Y * 200.0);
        app.world_mut().entity_mut(owner).insert(swarm);
        let deposit = common::spawn_deposit(
            &mut app,
            common::DepositFixture {
                world_pos: center,
                amount: 1000,
                capacity: 1000,
                radius: 48.0,
            },
        );
        let source = common::spawn_owned_source(
            &mut app,
            owner,
            center + source_offset,
            0,
            source_loads * Tuning::default().logistics.worker_carry_capacity,
        );
        let sink = common::spawn_sink_stockpile(&mut app, center - Vec2::Y * 200.0, 1000, 1100);
        app.world_mut().entity_mut(sink).insert(OwnerSwarm(owner));
        app.world_mut().spawn((
            ProductionFacility::new(),
            OwnerSwarm(owner),
            Transform::from_translation((center - Vec2::Y * 360.0).extend(0.0)),
        ));
        assert!(app.world_mut().resource_mut::<IntentGrid>().paint(
            cell,
            IntentKind::Gather,
            swarm
        ));
        let required = common::spawn_worker_at(&mut app, center - Vec2::X * 170.0);
        let spare_worker = common::spawn_worker_at(&mut app, center + spare_offset);
        for worker in [required, spare_worker] {
            app.world_mut()
                .entity_mut(worker)
                .insert(SwarmMember::new(swarm));
        }
        app.world_mut()
            .entity_mut(spare_worker)
            .insert(Commitment::Working);
        Self {
            app,
            owner,
            center,
            deposit,
            source,
            required,
            spare_worker,
        }
    }

    fn advance(&mut self, ticks: usize) {
        for _ in 0..ticks {
            self.app.update();
        }
    }

    fn divide_navigation(&mut self) -> Entity {
        self.app.insert_resource(IntentGrid::new(2, 2));
        paint_gather(&mut self.app, IVec2::new(-1, 0));
        let wall = common::spawn_stockpile(&mut self.app, Vec2::ZERO, 0, 0);
        self.app
            .world_mut()
            .get_mut::<Transform>(wall)
            .unwrap()
            .scale = Vec3::new(1.0, 40.0, 1.0);
        wall
    }

    fn start_spare_worker(&mut self) {
        self.advance(8);
        self.app
            .world_mut()
            .entity_mut(self.spare_worker)
            .insert(Commitment::Idle);
    }

    fn worker_states(&self) -> String {
        let world = self.app.world();
        [self.required, self.spare_worker]
            .into_iter()
            .map(|entity| {
                format!(
                    "{entity}: {:?} Gather={:?} Claim={:?} Cargo={:?} Position={:?}",
                    world.get::<Commitment>(entity),
                    world.get::<GatherAssignment>(entity),
                    world.get::<LogisticsReservation>(entity),
                    world.get::<Cargo>(entity),
                    world
                        .get::<Transform>(entity)
                        .map(|transform| transform.translation),
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn source_amount(&self) -> u32 {
        self.app
            .world()
            .get::<Stockpile>(self.source)
            .unwrap()
            .amount
    }

    fn spare_cargo(&self) -> u32 {
        self.app
            .world()
            .get::<Cargo>(self.spare_worker)
            .map_or(0, |cargo| cargo.amount)
    }

    fn spare_reservation(&self) -> Option<&LogisticsReservation> {
        self.app
            .world()
            .get::<LogisticsReservation>(self.spare_worker)
    }

    fn source_plans(&mut self) -> usize {
        let world = self.app.world_mut();
        world
            .query::<&PlannedStructure>()
            .iter(world)
            .filter(|plan| plan.kind == PlannedKind::SourceStockpile)
            .count()
    }
}

#[test]
fn spare_gathering_makes_additional_dropoffs_without_raising_worker_demand() {
    for swarm in [SwarmId::PLAYER, SwarmId(1)] {
        let mut quota_only = SpareGatherFixture::new(swarm, 15, -Vec2::Y * 83.0);
        quota_only.app.world_mut().despawn(quota_only.spare_worker);
        quota_only.advance(208);

        let mut mine = SpareGatherFixture::new(swarm, 15, -Vec2::Y * 83.0);
        mine.start_spare_worker();
        let mut minimum_worker_demand = u32::MAX;
        let mut maximum_worker_demand = 0;
        for _ in 0..200 {
            mine.advance(1);
            let world = mine.app.world();
            assert_eq!(
                world
                    .resource::<GatherQuotas>()
                    .economy(swarm)
                    .unwrap()
                    .gather_quota,
                1
            );
            let demand = world
                .resource::<PopulationDemand>()
                .desired_for(swarm, NanobotType::Worker);
            minimum_worker_demand = minimum_worker_demand.min(demand);
            maximum_worker_demand = maximum_worker_demand.max(demand);
        }
        assert_eq!((minimum_worker_demand, maximum_worker_demand), (1, 1));
        assert!(
            quota_only.source_amount() > 0,
            "{}",
            quota_only.worker_states()
        );
        assert!(
            mine.source_amount() > quota_only.source_amount(),
            "{swarm:?} deposited {} with two Workers and {} with one",
            mine.source_amount(),
            quota_only.source_amount(),
        );
        let economy = |app: &App| {
            app.world()
                .resource::<GatherQuotas>()
                .economy(swarm)
                .unwrap()
        };
        let hauler_demand = |app: &App| {
            app.world()
                .resource::<PopulationDemand>()
                .desired_for(swarm, NanobotType::Hauler)
        };
        assert!(economy(&mine.app).reserve_stock > economy(&quota_only.app).reserve_stock);
        assert!(hauler_demand(&mine.app) > hauler_demand(&quota_only.app));
        assert_eq!(mine.source_plans(), 0);
    }
}

#[test]
fn spare_gathering_leaves_the_last_load_for_a_quota_gatherer_on_approach() {
    let mut mine = SpareGatherFixture::new(SwarmId::PLAYER, 1, -Vec2::Y * 83.0);
    mine.start_spare_worker();
    mine.advance(6);
    let world = mine.app.world();
    assert!(world.get::<GatherAssignment>(mine.required).is_some());
    assert!(world.get::<LogisticsReservation>(mine.required).is_none());
    assert!(world.get::<GatherAssignment>(mine.spare_worker).is_none());
    assert!(mine.spare_reservation().is_none());

    // Opening a second load of room makes the same Worker eligible.
    let carry = Tuning::default().logistics.worker_carry_capacity;
    mine.app
        .world_mut()
        .get_mut::<Stockpile>(mine.source)
        .unwrap()
        .capacity = 2 * carry;
    mine.advance(12);
    let claim = mine
        .spare_reservation()
        .expect("the second load should be available for Spare Gathering");
    assert_eq!(
        (claim.destination, claim.destination_remaining),
        (mine.source, carry)
    );
}

#[test]
fn spare_gathering_waits_for_a_reachable_local_source() {
    let mut mine = SpareGatherFixture::with_layout(
        SwarmId::PLAYER,
        IVec2::new(-1, 0),
        Vec2::new(-175.0, 256.0),
        Vec2::X * 350.0,
        15,
        -Vec2::Y * 83.0,
    );
    let wall = mine.divide_navigation();

    mine.start_spare_worker();
    mine.advance(36);
    assert!(
        mine.app
            .world()
            .get::<GatherAssignment>(mine.required)
            .is_some()
    );
    assert!(
        mine.app
            .world()
            .get::<GatherAssignment>(mine.spare_worker)
            .is_none()
    );
    assert!(mine.spare_reservation().is_none());
    assert_eq!(mine.source_plans(), 0);

    mine.app.world_mut().despawn(wall);
    mine.advance(24);
    assert_eq!(
        mine.spare_reservation().map(|claim| claim.destination),
        Some(mine.source)
    );
}

#[test]
fn unreachable_construction_does_not_block_spare_gathering() {
    let mut mine = SpareGatherFixture::with_layout(
        SwarmId::PLAYER,
        IVec2::new(-1, 0),
        Vec2::new(-175.0, 256.0),
        Vec2::Y * 160.0,
        15,
        -Vec2::Y * 83.0,
    );
    mine.divide_navigation();
    mine.advance(30);
    let plan = common::spawn_planned_charger_at_cell(&mut mine.app, IVec2::ZERO);
    mine.app
        .world_mut()
        .entity_mut(plan)
        .insert(OwnerSwarm(mine.owner));
    mine.advance(12);
    mine.start_spare_worker();
    mine.advance(36);

    let world = mine.app.world();
    assert!(world.resource::<ActionableProjection>().iter_regions().any(|(_, work)|
        work.iter().any(|opportunity| matches!(
            opportunity.target, OpportunityTarget::PlannedBuild { structure, .. } if structure == plan
        ))
    ));
    assert_eq!(
        world.get::<PlannedStructure>(plan).unwrap().active_worker(),
        None
    );
    assert_eq!(
        world
            .resource::<PopulationDemand>()
            .desired_for(SwarmId::PLAYER, NanobotType::Worker),
        1
    );
    assert_eq!(
        mine.spare_reservation().map(|claim| claim.destination),
        Some(mine.source)
    );
}

#[test]
fn an_empty_spare_gatherer_yields_to_reachable_construction() {
    let mut mine = SpareGatherFixture::new(SwarmId::PLAYER, 15, -Vec2::Y * 190.0 + Vec2::X * 90.0);
    mine.start_spare_worker();
    mine.advance(12);
    assert!(
        mine.spare_reservation().is_some(),
        "{}",
        mine.worker_states()
    );
    assert_eq!(mine.spare_cargo(), 0);

    let plan = common::spawn_planned_charger_at_cell(&mut mine.app, IVec2::new(1, 0));
    mine.advance(12);
    assert_eq!(
        mine.app
            .world()
            .get::<PlannedStructure>(plan)
            .unwrap()
            .active_worker(),
        Some(mine.spare_worker)
    );
    assert!(mine.spare_reservation().is_none());
}

#[test]
fn a_loaded_spare_gatherer_delivers_before_taking_required_work() {
    let mut mine = SpareGatherFixture::new(SwarmId::PLAYER, 15, -Vec2::Y * 83.0);
    mine.start_spare_worker();
    mine.advance(64);
    assert!(mine.spare_cargo() > 0, "{}", mine.worker_states());
    let plan = common::spawn_planned_charger_at_cell(&mut mine.app, IVec2::new(1, 0));
    mine.advance(12);
    assert!(mine.spare_cargo() > 0, "{}", mine.worker_states());
    assert!(
        mine.app
            .world()
            .get::<PlannedStructure>(plan)
            .unwrap()
            .active_worker()
            .is_none()
    );
    mine.advance(112);
    assert!(mine.source_amount() > Tuning::default().logistics.worker_carry_capacity);
    assert!(
        mine.app
            .world()
            .get::<PlannedStructure>(plan)
            .unwrap()
            .active_worker()
            .is_some()
    );
}

#[test]
fn a_spare_trip_does_not_plan_a_replacement_source() {
    let mut mine = SpareGatherFixture::new(SwarmId::PLAYER, 15, -Vec2::Y * 190.0 + Vec2::X * 90.0);
    mine.start_spare_worker();
    mine.advance(12);
    assert!(
        mine.spare_reservation().is_some(),
        "{}",
        mine.worker_states()
    );
    let far_source =
        common::spawn_stockpile(&mut mine.app, mine.center + Vec2::X * 500.0, 100, 100);
    common::prepare_mineral_dropoff(&mut mine.app, mine.required, mine.deposit, far_source, 4);
    mine.app
        .world_mut()
        .entity_mut(mine.required)
        .remove::<GatherAssignment>();
    mine.app.world_mut().despawn(mine.source);

    mine.advance(12);
    assert_eq!(mine.source_plans(), 0);
    assert!(mine.spare_reservation().is_none());
}
