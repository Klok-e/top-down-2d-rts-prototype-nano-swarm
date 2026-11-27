static DEFAULT_CHARGE: std::sync::LazyLock<
    top_down_2d_rts_prototype_nano_swarm::tuning::ChargeTuning,
> = std::sync::LazyLock::new(top_down_2d_rts_prototype_nano_swarm::tuning::ChargeTuning::default);
#[path = "../common/mod.rs"]
mod common;

/// Bounded real-time wait for shared defender allocation at 60 Hz.
const DEFENDER_RESPONSE_TICK_BUDGET: u32 = 300;

use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    intent::{IntentGrid, IntentKind},
    nanobot::{MatchOutcome, Swarm, SwarmId},
    reachability::SwarmReachabilityMap,
    resources::{ResourceKind, ResourceLedger},
    strategic_controller::{Decision, IntentEditAction},
    strategic_runtime::{StrategicController, StrategicControllerPlugin},
};

fn spawnable_crew(
    owner: SwarmId,
    home: Vec2,
) -> [top_down_2d_rts_prototype_nano_swarm::strategic_controller::BotState; 3] {
    use top_down_2d_rts_prototype_nano_swarm::{
        nanobot::NanobotType, strategic_controller::BotState,
    };

    let crew = [
        NanobotType::Worker,
        NanobotType::Worker,
        NanobotType::Hauler,
    ];
    std::array::from_fn(|id| BotState {
        id: id as u64,
        owner,
        kind: crew[id],
        position: home,
        health: 100,
        max_health: 100,
        charge: 0.0,
        max_charge: 0.0,
        cargo: 0,
        displacement: Vec2::ZERO,
        defender_duty: Default::default(),
    })
}

fn apply_decision(grid: &mut IntentGrid, owner: SwarmId, decision: &Decision) {
    for edit in &decision.edits {
        match edit.action {
            IntentEditAction::Paint => grid.paint(edit.cell, edit.kind, owner),
            IntentEditAction::Erase => grid.erase(edit.cell, edit.kind, owner),
        };
    }
}

fn install_empty_reachability(app: &mut App, owners: impl IntoIterator<Item = SwarmId>) {
    let reachability = {
        let grid = app.world().resource::<IntentGrid>();
        SwarmReachabilityMap::new(
            grid,
            [],
            owners.into_iter().map(|owner| (owner, Vec2::ZERO)),
        )
        .expect("the controller fixture has clear swarm homes")
    };
    app.insert_resource(reachability);
}

mod raid_readiness {
    use super::*;
    use top_down_2d_rts_prototype_nano_swarm::{
        SIMULATION_HZ,
        nanobot::{
            Charge, ChargerAssignment, DefenderResponse, DefenderStagingAssignments,
            DirectMovementComponent, Health, OwnerSwarm, SwarmMember,
        },
        navigation_runtime::NavigationBudget,
        resources::Stockpile,
        strategic_runtime::ControllerTelemetry,
        terrain::RockFormation,
        tuning::Tuning,
    };

    const OWNER: SwarmId = SwarmId::PLAYER;
    const ENEMY: SwarmId = SwarmId(9);
    const TARGET: IVec2 = IVec2::new(9, 0);
    const REVIEW_TICKS: u64 = 5;
    const DISTANT_TARGET: IVec2 = IVec2::new(50, 0);
    const DISTANT_NEUTRAL: IVec2 = IVec2::new(-50, 0);

    fn raid_fixture<const N: usize>(app: App) -> (App, [Entity; N]) {
        raid_fixture_at(app, TARGET, 64)
    }

    fn raid_fixture_at<const N: usize>(
        mut app: App,
        target: IVec2,
        grid_width: i32,
    ) -> (App, [Entity; N]) {
        app.insert_resource(IntentGrid::new(grid_width, 64));
        app.add_plugins(StrategicControllerPlugin);
        let home = common::cell_world_center(IVec2::ZERO);
        app.world_mut().spawn((
            Swarm {},
            OWNER,
            Transform::from_translation(home.extend(0.0)),
            StrategicController::new(OWNER),
        ));
        let enemy_home = common::cell_world_center(target + 4 * IVec2::X);
        let enemy = app
            .world_mut()
            .spawn((
                Swarm {},
                ENEMY,
                Transform::from_translation(enemy_home.extend(0.0)),
            ))
            .id();
        add_enemy_mine(&mut app, enemy, target);
        common::spawn_facility_at(&mut app, enemy, enemy_home);
        let force =
            std::array::from_fn(|_| common::spawn_stationary_defender_at(&mut app, home, OWNER));
        install_empty_reachability(&mut app, [OWNER, ENEMY]);
        let mut tuning = app.world_mut().resource_mut::<Tuning>();
        tuning.autonomy.controller_review_period_ticks = REVIEW_TICKS;
        tuning.autonomy.strategy_raid_base = 10_000.0;
        (app, force)
    }

    fn add_enemy_mine(app: &mut App, enemy: Entity, cell: IVec2) -> Entity {
        let position = common::cell_world_center(cell);
        let mine = common::spawn_deposit(
            app,
            common::DepositFixture {
                world_pos: position,
                amount: 72_000,
                capacity: 72_000,
                radius: 80.0,
            },
        );
        let stockpile = common::spawn_stockpile(app, position, 80, 100);
        for entity in [mine, stockpile] {
            app.world_mut().entity_mut(entity).insert(OwnerSwarm(enemy));
        }
        stockpile
    }

    fn neutral_deposit(app: &mut App, cell: IVec2) -> Entity {
        common::spawn_deposit(
            app,
            common::DepositFixture {
                world_pos: common::cell_world_center(cell),
                amount: 72_000,
                capacity: 72_000,
                radius: 80.0,
            },
        )
    }

    fn add_claim_crew(app: &mut App) {
        let home = common::cell_world_center(IVec2::ZERO);
        common::spawn_worker_at(app, home);
        common::spawn_worker_at(app, home);
        common::spawn_hauler_at(app, home);
    }

    fn review(app: &mut App) {
        for _ in 0..REVIEW_TICKS {
            app.update();
        }
    }

    fn forward_defend_cells(app: &App) -> std::collections::HashSet<IVec2> {
        app.world()
            .resource::<IntentGrid>()
            .iter_active_cells()
            .filter_map(|(cell, intent)| {
                (cell.x >= 2 && intent.has_owned(IntentKind::Defend, OWNER)).then_some(cell)
            })
            .collect()
    }

    fn owns_defend(app: &App, cell: IVec2) -> bool {
        owns_intent(app, cell, IntentKind::Defend)
    }

    fn owns_intent(app: &App, cell: IVec2, kind: IntentKind) -> bool {
        app.world()
            .resource::<IntentGrid>()
            .cell(cell)
            .is_some_and(|intent| intent.has_owned(kind, OWNER))
    }

    fn explanation(app: &App) -> String {
        app.world().resource::<ControllerTelemetry>().profiles()[&OWNER.0]
            .last_explanation
            .clone()
    }

    #[test]
    fn an_ineligible_neutral_deposit_leaves_the_raid_available() {
        for has_crew in [false, true] {
            let (mut app, _) = raid_fixture::<2>(common::minimal_app());
            let neutral = IVec2::new(-4, 0);
            neutral_deposit(&mut app, neutral);
            if has_crew {
                add_claim_crew(&mut app);
                app.world_mut()
                    .resource_mut::<Tuning>()
                    .autonomy
                    .strategy_expansion_income_weight = 0.0;
            }

            review(&mut app);

            assert!(!owns_intent(&app, neutral, IntentKind::Gather));
            assert!(owns_defend(&app, IVec2::new(3, 0)), "{}", explanation(&app));
        }
    }

    #[test]
    fn a_viable_neutral_claim_keeps_the_active_raid_staging_point() {
        let (mut app, _) = raid_fixture::<2>(common::minimal_app());
        add_claim_crew(&mut app);
        review(&mut app);
        let front = forward_defend_cells(&app);
        let staging_point = IVec2::new(3, 0);
        assert!(front.contains(&staging_point), "{}", explanation(&app));

        let neutral = IVec2::new(-4, 0);
        neutral_deposit(&mut app, neutral);
        review(&mut app);

        assert!(
            owns_intent(&app, neutral, IntentKind::Gather),
            "{}",
            explanation(&app)
        );
        assert!(owns_intent(&app, neutral + IVec2::X, IntentKind::Build));
        assert!(owns_intent(&app, neutral, IntentKind::Corridor));
        assert!(owns_intent(&app, staging_point, IntentKind::Corridor));
        assert_eq!(forward_defend_cells(&app), front, "{}", explanation(&app));
    }

    #[test]
    fn paint_overflow_keeps_a_viable_raid_and_replaces_it_after_its_target_is_lost() {
        let (mut app, _) = over_budget_raid_and_claim_fixture();
        let target = DISTANT_TARGET;
        let neutral = DISTANT_NEUTRAL;
        review(&mut app);
        let revision = app.world().resource::<IntentGrid>().revision();
        assert!(owns_defend(&app, target), "{}", explanation(&app));
        assert!(owns_intent(&app, target, IntentKind::Corridor));

        neutral_deposit(&mut app, neutral);
        review(&mut app);

        assert_eq!(
            app.world().resource::<IntentGrid>().revision(),
            revision,
            "a paint-limited review must leave the whole previously applied plan intact: {}",
            explanation(&app)
        );
        assert!(explanation(&app).contains("paint"), "{}", explanation(&app));

        let stockpile = {
            let world = app.world_mut();
            world
                .query_filtered::<Entity, With<Stockpile>>()
                .single(world)
                .expect("the raid fixture has one target Stockpile")
        };
        let enemy = app.world().get::<OwnerSwarm>(stockpile).unwrap().0;
        app.world_mut().despawn(stockpile);
        let replacement = target + IVec2::Y;
        let next_target =
            common::spawn_stockpile(&mut app, common::cell_world_center(replacement), 80, 100);
        app.world_mut()
            .entity_mut(next_target)
            .insert(OwnerSwarm(enemy));
        review(&mut app);

        assert!(owns_defend(&app, replacement), "{}", explanation(&app));
        assert!(owns_intent(&app, replacement, IntentKind::Corridor));
        assert!(!owns_intent(&app, neutral, IntentKind::Gather));
    }

    #[test]
    fn an_over_budget_first_review_publishes_complete_home_support() {
        let (mut app, _) = over_budget_raid_and_claim_fixture();
        let neutral = DISTANT_NEUTRAL;
        neutral_deposit(&mut app, neutral);

        review(&mut app);

        assert!(owns_intent(&app, IVec2::ZERO, IntentKind::Build));
        assert!(owns_intent(&app, IVec2::ZERO, IntentKind::Corridor));
        assert!(owns_defend(&app, IVec2::ZERO));
        assert!(
            forward_defend_cells(&app).is_empty(),
            "{}",
            explanation(&app)
        );
        assert!(!owns_intent(&app, neutral, IntentKind::Gather));
    }

    #[test]
    fn paint_overflow_preserves_raid_readiness_observations() {
        let (mut app, force) = over_budget_raid_and_claim_fixture();
        review(&mut app);
        let target = DISTANT_TARGET;
        assert!(owns_defend(&app, target), "{}", explanation(&app));
        neutral_deposit(&mut app, DISTANT_NEUTRAL);

        let set_charge = |app: &mut App, charge| {
            for defender in force {
                app.world_mut().get_mut::<Charge>(defender).unwrap().current = charge;
            }
        };
        let wait_three_seconds = |app: &mut App| {
            for _ in 0..3 * SIMULATION_HZ as u64 {
                app.update();
            }
        };
        set_charge(&mut app, 0.1);
        wait_three_seconds(&mut app);
        assert!(owns_defend(&app, target), "{}", explanation(&app));

        set_charge(&mut app, 1.0);
        review(&mut app);
        assert!(!owns_intent(&app, DISTANT_NEUTRAL, IntentKind::Gather));

        set_charge(&mut app, 0.1);
        wait_three_seconds(&mut app);
        assert!(owns_defend(&app, target), "{}", explanation(&app));

        wait_three_seconds(&mut app);
        assert!(!owns_defend(&app, target), "{}", explanation(&app));
        assert!(owns_intent(&app, DISTANT_NEUTRAL, IntentKind::Gather));
    }

    fn over_budget_raid_and_claim_fixture() -> (App, [Entity; 2]) {
        let (mut app, force) = raid_fixture_at(common::minimal_app(), DISTANT_TARGET, 128);
        prepare_distant_claim(&mut app);
        (app, force)
    }

    fn prepare_distant_claim(app: &mut App) {
        add_claim_crew(app);
        let mut tuning = app.world_mut().resource_mut::<Tuning>();
        tuning.autonomy.strategy_staging_spacing_cells = 64;
        tuning.autonomy.strategy_expansion_travel_cost = 0.0;
        tuning.autonomy.strategy_expansion_haul_cost = 0.0;
    }

    #[test]
    fn paint_overflow_can_replace_an_ineligible_raid_with_one_of_equal_priority() {
        let guarded_target = IVec2::new(100, 0);
        let open_target = IVec2::new(55, -10);
        let (mut app, _) = raid_fixture_at::<2>(common::minimal_app(), guarded_target, 256);
        prepare_distant_claim(&mut app);
        review(&mut app);
        assert!(owns_defend(&app, DISTANT_TARGET), "{}", explanation(&app));

        let enemy = {
            let world = app.world_mut();
            world
                .query::<(Entity, &SwarmId)>()
                .iter(world)
                .find_map(|(entity, &owner)| (owner == ENEMY).then_some(entity))
                .unwrap()
        };
        add_enemy_mine(&mut app, enemy, open_target);
        neutral_deposit(&mut app, DISTANT_NEUTRAL);
        for cell in [guarded_target, guarded_target - IVec2::X] {
            common::spawn_stationary_defender_at(&mut app, common::cell_world_center(cell), ENEMY);
        }
        review(&mut app);
        assert!(owns_defend(&app, DISTANT_TARGET), "{}", explanation(&app));

        for _ in 0..6 * SIMULATION_HZ as u64 {
            app.update();
        }
        assert!(owns_defend(&app, open_target), "{}", explanation(&app));
        assert!(owns_intent(&app, open_target, IntentKind::Corridor));
        assert!(!owns_intent(&app, DISTANT_NEUTRAL, IntentKind::Gather));
    }

    #[test]
    fn too_few_available_charged_defenders_leave_a_high_scoring_raid_ineligible() {
        let (mut app, [_, defender]) = raid_fixture(common::minimal_app());
        let charger = app.world_mut().spawn_empty().id();
        app.world_mut()
            .entity_mut(defender)
            .insert(ChargerAssignment { charger });
        review(&mut app);
        assert!(
            forward_defend_cells(&app).is_empty(),
            "{}",
            explanation(&app)
        );
        assert!(owns_defend(&app, IVec2::ZERO));

        app.world_mut()
            .entity_mut(defender)
            .remove::<ChargerAssignment>();
        app.world_mut().get_mut::<Charge>(defender).unwrap().current = 0.1;
        review(&mut app);
        assert!(
            forward_defend_cells(&app).is_empty(),
            "{}",
            explanation(&app)
        );

        app.world_mut().get_mut::<Health>(defender).unwrap().current = 1;
        app.world_mut().get_mut::<Charge>(defender).unwrap().current = 1.0;
        review(&mut app);
        assert!(
            forward_defend_cells(&app).is_empty(),
            "{}",
            explanation(&app)
        );

        let mut health = app.world_mut().get_mut::<Health>(defender).unwrap();
        health.current = health.max;
        let intruder = common::spawn_worker_at(
            &mut app,
            common::cell_world_center(IVec2::ZERO) + Vec2::X * 120.0,
        );
        app.world_mut()
            .entity_mut(intruder)
            .insert(SwarmMember(ENEMY));
        app.world_mut()
            .entity_mut(defender)
            .insert(DefenderResponse { target: intruder });
        review(&mut app);
        assert!(
            forward_defend_cells(&app).is_empty(),
            "{}",
            explanation(&app)
        );

        app.world_mut()
            .entity_mut(defender)
            .remove::<DefenderResponse>();
        review(&mut app);
        assert!(
            !forward_defend_cells(&app).is_empty(),
            "{}",
            explanation(&app)
        );
    }

    #[test]
    fn guards_along_the_approach_and_at_the_target_change_raid_eligibility() {
        let (mut app, _) = raid_fixture::<2>(common::minimal_app());
        let on_route = common::spawn_stationary_defender_at(
            &mut app,
            common::cell_world_center(IVec2::new(4, 0)),
            ENEMY,
        );
        let at_target = common::spawn_stationary_defender_at(
            &mut app,
            common::cell_world_center(TARGET),
            ENEMY,
        );
        review(&mut app);
        let guarded = explanation(&app);
        assert!(
            !guarded.contains("raid enemy Resource Deposit"),
            "{guarded}"
        );
        assert!(!owns_defend(&app, TARGET));

        for (guard, cell) in [
            (on_route, IVec2::new(4, 6)),
            (at_target, TARGET + 6 * IVec2::Y),
        ] {
            app.world_mut()
                .get_mut::<Transform>(guard)
                .unwrap()
                .translation = common::cell_world_center(cell).extend(0.0);
        }
        review(&mut app);
        let undefended = explanation(&app);
        assert!(
            undefended.contains("raid enemy Resource Deposit"),
            "{undefended}"
        );
        assert!(!forward_defend_cells(&app).is_empty());
    }

    fn finishing_fixture<const N: usize>() -> (App, Entity, Entity) {
        use top_down_2d_rts_prototype_nano_swarm::nanobot::{
            PlannedKind, ProductionFacility, Structure, StructureKind,
        };

        let mut app = common::minimal_app();
        app.insert_resource(IntentGrid::new(64, 64))
            .add_plugins(StrategicControllerPlugin);
        let home = common::cell_world_center(IVec2::ZERO);
        let target = common::cell_world_center(TARGET);
        let enemy_home = common::cell_world_center(TARGET + IVec2::Y * 8);
        app.world_mut().spawn((
            Swarm {},
            OWNER,
            Transform::from_translation(home.extend(0.0)),
            StrategicController::new(OWNER),
        ));
        let enemy = app
            .world_mut()
            .spawn((
                Swarm {},
                ENEMY,
                Transform::from_translation(enemy_home.extend(0.0)),
            ))
            .id();
        let facility = app
            .world_mut()
            .spawn((
                ProductionFacility::new(),
                OwnerSwarm(enemy),
                Structure {
                    paint_suspended: true,
                    ..Structure::new(StructureKind::Basic)
                },
                Transform::from_translation(target.extend(0.0)),
            ))
            .id();
        let worker =
            common::spawn_worker_at(&mut app, common::cell_world_center(TARGET + IVec2::Y * 4));
        app.world_mut()
            .entity_mut(worker)
            .insert(SwarmMember(ENEMY));

        let planned_cell = TARGET + IVec2::X + IVec2::Y;
        let planned = common::spawn_planned_structure_of_kind_at_cell(
            &mut app,
            planned_cell,
            PlannedKind::ProductionFacility,
        );
        app.world_mut()
            .entity_mut(planned)
            .insert((OwnerSwarm(enemy), ProductionFacility::new()));

        for _ in 0..N {
            common::spawn_stationary_defender_at(&mut app, home, OWNER);
        }
        install_empty_reachability(&mut app, [OWNER, ENEMY]);
        let mut tuning = app.world_mut().resource_mut::<Tuning>();
        tuning.autonomy.controller_review_period_ticks = REVIEW_TICKS;
        tuning.autonomy.strategy_staging_spacing_cells = 64;
        (app, enemy, facility)
    }

    #[test]
    fn ready_force_finishes_an_inactive_facility_despite_a_worker_and_idle_neutral_site() {
        let (mut app, enemy, _) = finishing_fixture::<2>();
        let neutral = IVec2::new(-4, 0);
        neutral_deposit(&mut app, neutral);
        let worker_cell = TARGET + IVec2::Y * 4;
        let enemy_mine = neutral_deposit(&mut app, worker_cell);
        app.world_mut()
            .entity_mut(enemy_mine)
            .insert(OwnerSwarm(enemy));

        review(&mut app);

        assert!(owns_defend(&app, TARGET), "{}", explanation(&app));
        assert!(!owns_defend(&app, worker_cell));
        assert!(!owns_intent(&app, neutral, IntentKind::Gather));
    }

    #[test]
    fn final_structure_requires_defenders_that_can_take_the_entire_approach() {
        let (mut guarded, _, _) = finishing_fixture::<2>();
        for cell in [IVec2::new(3, 0), IVec2::new(4, 0)] {
            common::spawn_stationary_defender_at(
                &mut guarded,
                common::cell_world_center(cell),
                ENEMY,
            );
        }
        let (mut unstaffed, _, _) = finishing_fixture::<0>();
        for app in [&mut guarded, &mut unstaffed] {
            review(app);
            assert!(forward_defend_cells(app).is_empty(), "{}", explanation(app));
        }
    }

    #[test]
    fn elimination_bonus_wins_a_close_comparison_but_a_strong_raid_can_outscore_it() {
        let raid_target = IVec2::new(-9, 0);
        for (stock, extra_complete, finishing) in [
            (100, false, true),
            (1_000, false, false),
            (100, true, false),
        ] {
            let (mut app, enemy, _) = finishing_fixture::<2>();
            if extra_complete {
                app.world_mut()
                    .spawn((Stockpile::default(), OwnerSwarm(enemy)));
            }
            add_competing_raid(&mut app, raid_target, stock);

            review(&mut app);

            assert_eq!(
                owns_defend(&app, TARGET),
                finishing,
                "{}",
                explanation(&app)
            );
            assert_eq!(
                owns_defend(&app, raid_target),
                !finishing,
                "{}",
                explanation(&app)
            );
        }
    }

    fn add_competing_raid(app: &mut App, raid_target: IVec2, stock: u32) {
        let raided_swarm = app
            .world_mut()
            .spawn((
                Swarm {},
                SwarmId(8),
                Transform::from_xyz(-4_000.0, 4_000.0, 0.0),
            ))
            .id();
        let stockpile = add_enemy_mine(app, raided_swarm, raid_target);
        {
            let mut pile = app.world_mut().get_mut::<Stockpile>(stockpile).unwrap();
            pile.amount = stock;
            pile.capacity = stock;
        }
        common::spawn_facility_at(
            app,
            raided_swarm,
            common::cell_world_center(IVec2::new(-13, 10)),
        );
    }

    #[test]
    fn a_new_finishing_approach_checks_guards_behind_the_previous_staging_point() {
        let (mut app, _, facility) = finishing_fixture::<2>();
        let decoy_point = TARGET + IVec2::X * 6;
        add_competing_raid(&mut app, decoy_point, 100);
        let move_facility = |app: &mut App, cell| {
            app.world_mut()
                .get_mut::<Transform>(facility)
                .unwrap()
                .translation = common::cell_world_center(cell).extend(0.0);
        };
        move_facility(&mut app, TARGET + IVec2::Y * 40);
        review(&mut app);
        assert!(owns_defend(&app, decoy_point), "{}", explanation(&app));

        for cell in [IVec2::new(3, 0), IVec2::new(4, 0)] {
            common::spawn_stationary_defender_at(&mut app, common::cell_world_center(cell), ENEMY);
        }
        move_facility(&mut app, TARGET);
        review(&mut app);

        assert!(!owns_defend(&app, TARGET), "{}", explanation(&app));
    }

    #[test]
    fn only_a_ready_force_at_the_staging_point_skips_approach_guards() {
        for stationed_at_front in [1, 2] {
            let (mut app, _, _) = finishing_fixture::<2>();
            review(&mut app);
            assert!(owns_defend(&app, TARGET), "{}", explanation(&app));

            let world = app.world_mut();
            let mut defenders = world.query::<(&SwarmMember, &Charge, &mut Transform)>();
            for (_, _, mut transform) in defenders
                .iter_mut(world)
                .filter(|(member, _, _)| member.0 == OWNER)
                .take(stationed_at_front)
            {
                transform.translation = common::cell_world_center(TARGET + IVec2::Y).extend(0.0);
            }
            for cell in [IVec2::new(3, 0), IVec2::new(4, 0)] {
                common::spawn_stationary_defender_at(
                    &mut app,
                    common::cell_world_center(cell),
                    ENEMY,
                );
            }
            review(&mut app);
            assert!(
                owns_defend(&app, TARGET),
                "the active point has a grace period"
            );
            for _ in 0..6 * SIMULATION_HZ as u64 {
                app.update();
            }

            assert_eq!(
                owns_defend(&app, TARGET),
                stationed_at_front == 2,
                "{}",
                explanation(&app)
            );
        }
    }

    #[test]
    fn raid_holds_during_a_brief_recharge_and_withdraws_after_sustained_readiness_loss() {
        let (mut app, defenders) = raid_fixture::<2>(common::sim_app());
        review(&mut app);
        let first = forward_defend_cells(&app);
        let first_point = IVec2::new(3, 0);
        assert!(first.contains(&first_point), "{}", explanation(&app));
        for defender in defenders {
            app.world_mut()
                .get_mut::<Transform>(defender)
                .unwrap()
                .translation = common::cell_world_center(first_point).extend(0.0);
            app.world_mut().get_mut::<Charge>(defender).unwrap().current = 0.1;
        }
        review(&mut app);
        assert_eq!(forward_defend_cells(&app), first, "{}", explanation(&app));

        for defender in defenders {
            app.world_mut().get_mut::<Charge>(defender).unwrap().current = 1.0;
        }
        review(&mut app);
        assert!(!owns_defend(&app, first_point), "{}", explanation(&app));
        let second = forward_defend_cells(&app);
        assert!(!second.is_empty(), "recovered force advances");

        for defender in defenders {
            app.world_mut().get_mut::<Charge>(defender).unwrap().current = 0.1;
        }
        review(&mut app);
        assert_eq!(forward_defend_cells(&app), second);
        for _ in 0..(10 * SIMULATION_HZ as u64) {
            app.update();
        }
        assert!(
            forward_defend_cells(&app).is_empty(),
            "{}",
            explanation(&app)
        );
        assert!(owns_defend(&app, IVec2::ZERO));
    }

    #[test]
    fn an_unchanged_raid_approach_keeps_its_next_fixed_staging_point() {
        let target = IVec2::new(11, 0);
        let (mut app, defenders) = raid_fixture_at::<2>(common::sim_app(), target, 64);
        review(&mut app);
        let first = IVec2::new(3, 0);
        assert!(owns_defend(&app, first), "{}", explanation(&app));
        for defender in defenders {
            app.world_mut()
                .get_mut::<Transform>(defender)
                .unwrap()
                .translation = common::cell_world_center(first).extend(0.0);
        }
        review(&mut app);

        let next_block = forward_defend_cells(&app);
        assert!(
            next_block.contains(&IVec2::new(4, 0)) && !next_block.contains(&IVec2::new(7, 0)),
            "the second block stays centered at (5, 0): {}",
            explanation(&app)
        );
    }

    #[test]
    fn rear_mine_garrisons_do_not_sustain_a_raid_with_only_one_ready_defender() {
        let (mut app, [front, replacement]) = raid_fixture::<2>(common::sim_app());
        app.world_mut()
            .resource_mut::<Tuning>()
            .autonomy
            .strategy_staging_spacing_cells = 64;
        let own_swarm = {
            let world = app.world_mut();
            world
                .query_filtered::<Entity, With<StrategicController>>()
                .single(world)
                .unwrap()
        };
        let mine_cell = IVec2::new(-7, 0);
        let mine = neutral_deposit(&mut app, mine_cell);
        app.world_mut()
            .entity_mut(mine)
            .insert(OwnerSwarm(own_swarm));
        for (defender, offset) in [(front, IVec2::Y), (replacement, -IVec2::Y)] {
            app.world_mut()
                .get_mut::<Transform>(defender)
                .unwrap()
                .translation = common::cell_world_center(TARGET + offset).extend(0.0);
        }
        review(&mut app);
        assert!(owns_defend(&app, TARGET), "{}", explanation(&app));

        let rear_cells = [mine_cell, mine_cell + IVec2::X];
        let garrison = rear_cells.map(|cell| {
            common::spawn_stationary_defender_at(&mut app, common::cell_world_center(cell), OWNER)
        });
        app.world_mut().despawn(replacement);
        for _ in 0..8 {
            let exhausted = common::spawn_stationary_defender_at(
                &mut app,
                common::cell_world_center(TARGET),
                OWNER,
            );
            app.world_mut()
                .get_mut::<Charge>(exhausted)
                .unwrap()
                .current = 0.1;
        }
        review(&mut app);
        let assignments = app.world().resource::<DefenderStagingAssignments>();
        assert!(
            assignments
                .destination(front)
                .is_some_and(|cell| forward_defend_cells(&app).contains(&cell))
        );
        for defender in garrison {
            assert!(
                assignments
                    .destination(defender)
                    .is_some_and(|cell| rear_cells.contains(&cell)),
                "the Defender must be assigned to the rear mine, got {:?} ({})",
                assignments.destination(defender),
                explanation(&app)
            );
        }
        assert!(
            owns_defend(&app, TARGET),
            "the shortfall has a grace period"
        );

        for _ in 0..6 * SIMULATION_HZ as u64 {
            app.update();
        }
        assert!(!owns_defend(&app, TARGET), "{}", explanation(&app));
        assert!(owns_defend(&app, mine_cell));
    }

    #[test]
    fn staging_counts_pending_assignments_and_ignores_defenders_passing_through() {
        let (mut app, defenders) = raid_fixture::<7>(common::sim_app());
        app.insert_resource(NavigationBudget(0));
        let rock = RockFormation::Rectangle {
            half: Vec2::new(1_024.0, 128.0),
        };
        let rock_transform =
            Transform::from_translation(common::cell_world_center(IVec2::new(0, 3)).extend(0.0));
        let reachability = SwarmReachabilityMap::new(
            app.world().resource::<IntentGrid>(),
            [rock.obstacle(&rock_transform)],
            [
                (OWNER, common::cell_world_center(IVec2::ZERO)),
                (ENEMY, common::cell_world_center(TARGET + 4 * IVec2::X)),
            ],
        )
        .unwrap();
        app.insert_resource(reachability);
        app.world_mut().spawn((rock, rock_transform));

        let move_defender = |app: &mut App, defender, cell| {
            app.world_mut()
                .get_mut::<Transform>(defender)
                .unwrap()
                .translation = common::cell_world_center(cell).extend(0.0);
        };
        for &defender in &defenders[4..] {
            move_defender(&mut app, defender, IVec2::new(-2, 6));
        }
        let first_point = IVec2::new(3, 0);
        let charger = app.world_mut().spawn_empty().id();
        for _ in 0..3 {
            let passing = common::spawn_stationary_defender_at(
                &mut app,
                common::cell_world_center(first_point),
                OWNER,
            );
            app.world_mut()
                .entity_mut(passing)
                .insert(ChargerAssignment { charger });
        }

        review(&mut app);
        let first = forward_defend_cells(&app);
        assert!(first.contains(&first_point), "{}", explanation(&app));
        let assignments = app.world().resource::<DefenderStagingAssignments>();
        for defender in defenders {
            assert!(
                assignments
                    .destination(defender)
                    .is_some_and(|cell| first.contains(&cell))
            );
        }
        for &defender in &defenders[4..] {
            assert!(
                app.world()
                    .get::<DirectMovementComponent>(defender)
                    .is_none()
            );
        }

        for &defender in &defenders[..4] {
            move_defender(&mut app, defender, first_point);
        }
        review(&mut app);
        assert_eq!(
            forward_defend_cells(&app),
            first,
            "four arrivals out of seven assignments must hold even with passing Defenders"
        );

        move_defender(&mut app, defenders[4], first_point);
        review(&mut app);
        assert!(
            !owns_defend(&app, first_point),
            "five arrivals out of seven assignments must advance"
        );
        assert!(owns_defend(&app, IVec2::new(6, 0)));
    }

    #[test]
    fn raid_withdraws_when_an_obstacle_closes_the_target_approach() {
        let (mut app, _) = raid_fixture::<2>(common::minimal_app());
        review(&mut app);
        let first = forward_defend_cells(&app);
        let first_point = IVec2::new(3, 0);
        assert!(first.contains(&first_point), "{}", explanation(&app));
        app.world_mut().spawn((
            RockFormation::Rectangle {
                half: Vec2::splat(384.0),
            },
            Transform::from_translation(common::cell_world_center(TARGET).extend(0.0)),
        ));
        review(&mut app);
        assert!(
            forward_defend_cells(&app).is_empty(),
            "{}",
            explanation(&app)
        );
        assert!(!owns_defend(&app, first_point));
        assert!(owns_defend(&app, IVec2::ZERO));
    }
}

#[test]
fn strategic_adapter_applies_only_its_owners_intent_without_mutating_resources() {
    let mut app = common::minimal_app();
    app.add_plugins(StrategicControllerPlugin);
    let owner = SwarmId(7);
    let old = IVec2::new(2, 0);
    let next = IVec2::ZERO;
    app.world_mut().spawn((
        Swarm {},
        owner,
        Transform::default(),
        StrategicController::new(owner),
    ));
    install_empty_reachability(&mut app, [owner]);
    for swarm in [SwarmId::PLAYER, owner] {
        app.world_mut()
            .resource_mut::<IntentGrid>()
            .paint(old, IntentKind::Defend, swarm);
        app.world_mut()
            .resource_mut::<ResourceLedger>()
            .add_for(swarm, ResourceKind::Minerals, 37);
    }
    app.update();
    let grid = app.world().resource::<IntentGrid>();
    assert!(
        grid.cell(next)
            .unwrap()
            .has_owned(IntentKind::Defend, owner)
    );
    assert!(!grid.cell(old).unwrap().has_owned(IntentKind::Defend, owner));
    assert!(
        grid.cell(old)
            .unwrap()
            .has_owned(IntentKind::Defend, SwarmId::PLAYER)
    );
    assert!(
        !grid
            .cell(next)
            .unwrap()
            .has_owned(IntentKind::Defend, SwarmId::PLAYER)
    );
    for swarm in [SwarmId::PLAYER, owner] {
        assert_eq!(
            app.world()
                .resource::<ResourceLedger>()
                .total_for(swarm, ResourceKind::Minerals),
            37
        );
    }
    let revision = grid.revision();
    app.insert_resource(MatchOutcome::Winner(owner));
    for _ in 0..4 {
        app.update();
    }
    assert_eq!(app.world().resource::<IntentGrid>().revision(), revision);
}

#[test]
fn strategic_adapter_connects_the_deposit_to_home_around_rock() {
    use top_down_2d_rts_prototype_nano_swarm::terrain::RockFormation;

    let mut app = common::minimal_app();
    app.insert_resource(IntentGrid::new(16, 16));
    app.add_plugins(StrategicControllerPlugin);
    let owner = SwarmId::PLAYER;
    let home_cell = IVec2::new(-3, 0);
    let deposit_cell = IVec2::new(3, 0);
    let home = common::cell_world_center(home_cell);
    let rock = RockFormation::Rectangle {
        half: Vec2::new(256.0, 650.0),
    };
    let rock_transform =
        Transform::from_translation(common::cell_world_center(IVec2::ZERO).extend(0.0));
    let swarm = app
        .world_mut()
        .spawn((
            Swarm {},
            owner,
            Transform::from_translation(home.extend(0.0)),
            StrategicController::new(owner),
        ))
        .id();
    app.world_mut().spawn((rock, rock_transform));
    let deposit = common::spawn_deposit(
        &mut app,
        common::DepositFixture {
            world_pos: common::cell_world_center(deposit_cell),
            amount: 72_000,
            capacity: 72_000,
            radius: 80.0,
        },
    );
    app.world_mut().entity_mut(deposit).insert(
        top_down_2d_rts_prototype_nano_swarm::nanobot::OwnerSwarm(swarm),
    );
    let reachability = SwarmReachabilityMap::new(
        app.world().resource::<IntentGrid>(),
        [rock.obstacle(&rock_transform)],
        [(owner, home)],
    )
    .unwrap();
    app.insert_resource(reachability);
    app.update();
    let grid = app.world().resource::<IntentGrid>();
    let corridor = |cell| {
        grid.cell(cell)
            .is_some_and(|intent| intent.has_owned(IntentKind::Corridor, owner))
    };
    assert!(corridor(home_cell));
    assert!(!corridor(IVec2::ZERO), "the direct corridor is blocked");

    let mut connected = std::collections::HashSet::from([home_cell]);
    let mut frontier = vec![home_cell];
    while let Some(cell) = frontier.pop() {
        for next in (-1..=1).flat_map(|y| (-1..=1).map(move |x| cell + IVec2::new(x, y))) {
            if corridor(next) && connected.insert(next) {
                frontier.push(next);
            }
        }
    }
    assert!(
        connected.contains(&deposit_cell),
        "corridor cells: {connected:?}"
    );
}

#[test]
fn strategic_adapter_contests_a_deposit_when_a_hostile_worker_moves_toward_it() {
    use top_down_2d_rts_prototype_nano_swarm::{
        nanobot::{NanobotType, SwarmMember},
        tuning::Tuning,
    };

    let mut app = common::minimal_app();
    app.insert_resource(IntentGrid::new(16, 16));
    app.add_plugins(StrategicControllerPlugin);
    let enemy = SwarmId(9);
    let deposit = IVec2::new(5, 0);
    let deposit_position = common::cell_world_center(deposit);
    let swarm = common::spawn_opponent_swarm_with_nanobots(
        &mut app,
        Vec2::ZERO,
        &[(NanobotType::Worker, 2), (NanobotType::Hauler, 1)],
    );
    let owner = *app.world().get::<SwarmId>(swarm).unwrap();
    app.world_mut()
        .entity_mut(swarm)
        .insert(StrategicController::new(owner));
    install_empty_reachability(&mut app, [owner]);
    let enemy_home = common::cell_world_center(IVec2::new(0, -4));
    let enemy_swarm = app
        .world_mut()
        .spawn((
            Swarm {},
            enemy,
            Transform::from_translation(enemy_home.extend(0.0)),
        ))
        .id();
    common::spawn_facility_at(&mut app, enemy_swarm, enemy_home);
    common::spawn_deposit(
        &mut app,
        common::DepositFixture {
            world_pos: deposit_position,
            amount: 72_000,
            capacity: 72_000,
            radius: 80.0,
        },
    );
    let worker = common::spawn_worker_at(&mut app, deposit_position + Vec2::X * 400.0);
    app.world_mut()
        .entity_mut(worker)
        .insert(SwarmMember::new(enemy));
    let mut tuning = app.world_mut().resource_mut::<Tuning>();
    tuning.autonomy.controller_review_period_ticks = 1;
    tuning.autonomy.strategy_expansion_income_weight = 0.0;
    tuning.autonomy.strategy_staging_spacing_cells = 64;

    let owns_deposit_block = |app: &App| {
        app.world()
            .resource::<IntentGrid>()
            .cell(deposit)
            .is_some_and(|cell| cell.has_owned(IntentKind::Defend, owner))
    };
    app.update();
    assert!(!owns_deposit_block(&app));

    app.world_mut()
        .get_mut::<Transform>(worker)
        .unwrap()
        .translation
        .x -= 4.0;
    app.update();
    let review = &app
        .world()
        .resource::<top_down_2d_rts_prototype_nano_swarm::strategic_runtime::ControllerTelemetry>()
        .profiles()[&owner.0];
    assert!(owns_deposit_block(&app), "{review:?}");
}

#[test]
fn controller_telemetry_counts_invalidation_checks_between_reviews() {
    use top_down_2d_rts_prototype_nano_swarm::strategic_runtime::ControllerTelemetry;

    let mut app = common::minimal_app();
    app.add_plugins(StrategicControllerPlugin);
    let owner = SwarmId(7);
    let swarm = app
        .world_mut()
        .spawn((
            Swarm {},
            owner,
            Transform::default(),
            StrategicController::new(owner),
        ))
        .id();
    install_empty_reachability(&mut app, [owner]);
    let deposit = common::spawn_deposit(
        &mut app,
        common::DepositFixture {
            world_pos: common::cell_world_center(IVec2::new(-1, -1)),
            amount: 72_000,
            capacity: 72_000,
            radius: 80.0,
        },
    );
    app.world_mut().entity_mut(deposit).insert(
        top_down_2d_rts_prototype_nano_swarm::nanobot::OwnerSwarm(swarm),
    );
    app.update();
    let initial = app.world().resource::<ControllerTelemetry>().profiles()[&owner.0].clone();
    assert_eq!(initial.reviews, 1);
    app.update();
    let next = app.world().resource::<ControllerTelemetry>().profiles()[&owner.0].clone();
    assert_eq!(next.reviews, initial.reviews);
    assert_eq!(next.intent_edits, initial.intent_edits);
    assert!(
        next.work_units > initial.work_units,
        "checking the retained deposit consumes work even without a strategy review"
    );
}

#[test]
fn controller_eventually_erases_stale_owned_intent_after_large_foreign_prefix() {
    use top_down_2d_rts_prototype_nano_swarm::strategic_controller::{
        Controller, GameState, SwarmState,
    };

    let owner = SwarmId(4);
    let foreign = SwarmId(9);
    let foreign_sentinel = IVec2::new(-32, -32);
    let stale = IVec2::new(31, 31);
    let mut grid = IntentGrid::new(64, 64);
    for y in -32..-27 {
        for x in -32..32 {
            grid.paint(IVec2::new(x, y), IntentKind::Gather, foreign);
        }
    }
    grid.paint(stale, IntentKind::Defend, owner);

    let swarms = [SwarmState {
        id: owner,
        home: Vec2::ZERO,
        minerals: 0,
        mineral_income_per_second: 0.0,
    }];
    let mut controller = Controller::new(owner);
    for review in 0..3 {
        let decision = controller.decide(
            &top_down_2d_rts_prototype_nano_swarm::tuning::AutonomyTuning::default(),
            &GameState {
                grid: &grid,
                swarms: &swarms,
                bots: &[],
                structures: &[],
                deposits: &[],
                terrain: &[],
                charge: &DEFAULT_CHARGE,
                reachability: None,
                route_query: None,
                mine_query: None,
                supply: None,
                build_footprints: &[],
                tick: review * 30,
                finished: false,
            },
        );
        apply_decision(&mut grid, owner, &decision);
    }

    assert!(
        !grid
            .cell(stale)
            .unwrap()
            .has_owned(IntentKind::Defend, owner),
        "bounded cleanup must eventually pass foreign cells and erase stale owned intent"
    );
    assert!(
        grid.cell(foreign_sentinel)
            .unwrap()
            .has_owned(IntentKind::Gather, foreign),
        "cleanup must preserve foreign intent"
    );
}

#[test]
fn controller_reviews_immediately_when_an_owned_deposit_is_exhausted() {
    use top_down_2d_rts_prototype_nano_swarm::strategic_controller::{
        Controller, DepositState, GameState, SwarmState,
    };

    let mut grid = IntentGrid::new(32, 32);
    let owner = SwarmId(4);
    let world = |cell: IVec2| {
        (cell.as_vec2() + Vec2::splat(0.5)) * top_down_2d_rts_prototype_nano_swarm::ZONE_BLOCK_SIZE
    };
    let swarms = [SwarmState {
        id: owner,
        home: world(IVec2::ZERO),
        minerals: 0,
        mineral_income_per_second: 0.0,
    }];
    let bots = spawnable_crew(owner, swarms[0].home);
    let deposits = [
        DepositState {
            id: 20,
            owner: Some(owner),
            position: world(IVec2::new(2, 0)),
            amount: 1_000,
            radius: 100.0,
        },
        DepositState {
            id: 21,
            owner: None,
            position: world(IVec2::new(4, 0)),
            amount: 50,
            radius: 100.0,
        },
    ];
    let mut controller = Controller::new(owner);
    let initial = controller.decide(
        &top_down_2d_rts_prototype_nano_swarm::tuning::AutonomyTuning::default(),
        &GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &bots,
            structures: &[],
            deposits: &deposits,
            terrain: &[],
            charge: &DEFAULT_CHARGE,
            reachability: None,
            route_query: None,
            mine_query: None,
            supply: None,
            build_footprints: &[],
            tick: 0,
            finished: false,
        },
    );
    apply_decision(&mut grid, owner, &initial);

    let depleted = [
        DepositState {
            amount: 0,
            ..deposits[0]
        },
        deposits[1],
    ];
    let urgent = controller.decide(
        &top_down_2d_rts_prototype_nano_swarm::tuning::AutonomyTuning::default(),
        &GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &bots,
            structures: &[],
            deposits: &depleted,
            terrain: &[],
            charge: &DEFAULT_CHARGE,
            reachability: None,
            route_query: None,
            mine_query: None,
            supply: None,
            build_footprints: &[],
            tick: 1,
            finished: false,
        },
    );

    assert!(urgent.reviewed, "depletion must bypass the regular cadence");
    assert!(urgent.explanation.contains("exhausted"));
    assert!(urgent.edits.iter().any(|edit| {
        edit.action == IntentEditAction::Paint
            && edit.kind == IntentKind::Gather
            && edit.cell == IVec2::new(4, 0)
    }));
    assert!(urgent.edits.iter().any(|edit| {
        edit.action == IntentEditAction::Erase
            && edit.kind == IntentKind::Gather
            && edit.cell == IVec2::new(2, 0)
    }));
}

#[test]
fn controller_urgently_replans_when_key_support_is_lost() {
    use top_down_2d_rts_prototype_nano_swarm::strategic_controller::{
        Controller, DepositState, GameState, StructureKind, StructureState, SwarmState,
    };

    let mut grid = IntentGrid::new(32, 32);
    let owner = SwarmId(4);
    let world = |cell: IVec2| {
        (cell.as_vec2() + Vec2::splat(0.5)) * top_down_2d_rts_prototype_nano_swarm::ZONE_BLOCK_SIZE
    };
    let swarms = [SwarmState {
        id: owner,
        home: world(IVec2::ZERO),
        minerals: 20,
        mineral_income_per_second: 0.0,
    }];
    let deposits = [DepositState {
        id: 20,
        owner: None,
        position: world(IVec2::new(2, 0)),
        amount: 500,
        radius: 100.0,
    }];
    let charger = [StructureState {
        id: 42,
        owner,
        position: Some(world(IVec2::new(1, 0))),
        kind: StructureKind::Charger,
        operational: true,
        health: 100,
        max_health: 100,
        minerals: 25,
    }];
    let mut controller = Controller::new(owner);
    let initial = controller.decide(
        &top_down_2d_rts_prototype_nano_swarm::tuning::AutonomyTuning::default(),
        &GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &[],
            structures: &charger,
            deposits: &deposits,
            terrain: &[],
            charge: &DEFAULT_CHARGE,
            reachability: None,
            route_query: None,
            mine_query: None,
            supply: None,
            build_footprints: &[],
            tick: 0,
            finished: false,
        },
    );
    apply_decision(&mut grid, owner, &initial);

    let urgent = controller.decide(
        &top_down_2d_rts_prototype_nano_swarm::tuning::AutonomyTuning::default(),
        &GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &[],
            structures: &[],
            deposits: &deposits,
            terrain: &[],
            charge: &DEFAULT_CHARGE,
            reachability: None,
            route_query: None,
            mine_query: None,
            supply: None,
            build_footprints: &[],
            tick: 1,
            finished: false,
        },
    );

    assert!(urgent.reviewed);
    assert!(urgent.explanation.contains("key Charger 42 was lost"));
}

#[test]
fn regular_and_urgent_reviews_share_one_public_work_allowance() {
    use top_down_2d_rts_prototype_nano_swarm::{
        nanobot::NanobotType,
        strategic_controller::{
            BotState, Controller, DepositState, GameState, PLANNING_WORK_BUDGET, StructureKind,
            StructureState, SwarmState,
        },
    };

    let mut grid = IntentGrid::new(64, 64);
    assert_eq!(PLANNING_WORK_BUDGET, 100_000);
    let owner = SwarmId(4);
    let enemy = SwarmId(9);
    let world = |cell: IVec2| {
        (cell.as_vec2() + Vec2::splat(0.5)) * top_down_2d_rts_prototype_nano_swarm::ZONE_BLOCK_SIZE
    };
    let swarms = [
        SwarmState {
            id: owner,
            home: world(IVec2::ZERO),
            minerals: 100,
            mineral_income_per_second: 0.0,
        },
        SwarmState {
            id: enemy,
            home: world(IVec2::new(24, 24)),
            minerals: 0,
            mineral_income_per_second: 0.0,
        },
    ];
    let deposit = [DepositState {
        id: 20,
        owner: None,
        position: world(IVec2::new(-1, 0)),
        amount: 500,
        radius: 100.0,
    }];
    let remnant = BotState {
        id: 50,
        owner: enemy,
        kind: NanobotType::Worker,
        position: world(IVec2::new(12, 3)),
        health: 20,
        max_health: 100,
        charge: 1.0,
        max_charge: 1.0,
        cargo: 0,
        displacement: Vec2::ZERO,
        defender_duty: Default::default(),
    };
    let charger = StructureState {
        id: 42,
        owner,
        position: Some(world(IVec2::new(1, 0))),
        kind: StructureKind::Charger,
        operational: true,
        health: 100,
        max_health: 100,
        minerals: 25,
    };
    let [worker, builder, hauler] = spawnable_crew(owner, swarms[0].home);
    let bots = [remnant, worker, builder, hauler];
    let mut controller = Controller::new(owner);
    let initial = controller.decide(
        &top_down_2d_rts_prototype_nano_swarm::tuning::AutonomyTuning::default(),
        &GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &bots,
            structures: &[charger],
            deposits: &deposit,
            terrain: &[],
            charge: &DEFAULT_CHARGE,
            reachability: None,
            route_query: None,
            mine_query: None,
            supply: None,
            build_footprints: &[],
            tick: 0,
            finished: false,
        },
    );
    apply_decision(&mut grid, owner, &initial);

    let crowded_structures = (0..60_000)
        .map(|id| StructureState {
            id: 1_000 + id,
            owner,
            position: Some(swarms[0].home),
            kind: StructureKind::Planned,
            operational: false,
            health: 100,
            max_health: 100,
            minerals: 0,
        })
        .collect::<Vec<_>>();
    let support_loss = controller.decide(
        &top_down_2d_rts_prototype_nano_swarm::tuning::AutonomyTuning::default(),
        &GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &bots,
            structures: &crowded_structures,
            deposits: &deposit,
            terrain: &[],
            charge: &DEFAULT_CHARGE,
            reachability: None,
            route_query: None,
            mine_query: None,
            supply: None,
            build_footprints: &[],
            tick: 1,
            finished: false,
        },
    );
    apply_decision(&mut grid, owner, &support_loss);

    let moved = BotState {
        position: world(IVec2::new(13, 3)),
        ..remnant
    };
    let mut crowded_bots = (0..60_000)
        .map(|id| BotState {
            id: 1_000 + id,
            owner,
            kind: NanobotType::Worker,
            position: swarms[0].home,
            health: 100,
            max_health: 100,
            charge: 1.0,
            max_charge: 1.0,
            cargo: 0,
            displacement: Vec2::ZERO,
            defender_duty: Default::default(),
        })
        .collect::<Vec<_>>();
    crowded_bots.push(moved);
    let target_move = controller.decide(
        &top_down_2d_rts_prototype_nano_swarm::tuning::AutonomyTuning::default(),
        &GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &crowded_bots,
            structures: &[],
            deposits: &deposit,
            terrain: &[],
            charge: &DEFAULT_CHARGE,
            reachability: None,
            route_query: None,
            mine_query: None,
            supply: None,
            build_footprints: &[],
            tick: 2,
            finished: false,
        },
    );

    assert!(support_loss.reviewed);
    assert!(
        initial.work_units + support_loss.work_units + target_move.work_units
            <= PLANNING_WORK_BUDGET
    );
    assert!(!target_move.reviewed);
    assert!(target_move.edits.is_empty());
}

#[test]
fn strategic_adapter_waits_for_mine_saturation_despite_a_stocked_source() {
    use top_down_2d_rts_prototype_nano_swarm::{
        nanobot::{NanobotType, OwnerSwarm},
        resources::ResourceDeposit,
    };

    let owner = SwarmId::PLAYER;
    let opening = IVec2::new(2, 0);
    let expansion = IVec2::new(5, 0);
    let opening_position = common::cell_world_center(opening);
    let mut app = common::minimal_app();
    app.insert_resource(IntentGrid::new(16, 16));
    app.add_plugins(StrategicControllerPlugin);
    let swarm = common::spawn_swarm_with_nanobots(
        &mut app,
        common::cell_world_center(IVec2::ZERO),
        &[(NanobotType::Worker, 4), (NanobotType::Hauler, 2)],
    );
    app.world_mut()
        .entity_mut(swarm)
        .insert(StrategicController::new(owner));
    install_empty_reachability(&mut app, [owner]);
    let [mine, _] = [opening, expansion].map(|cell| {
        common::spawn_deposit(
            &mut app,
            common::DepositFixture {
                world_pos: common::cell_world_center(cell),
                amount: 72_000,
                capacity: 72_000,
                radius: 64.0,
            },
        )
    });
    app.world_mut().entity_mut(mine).insert(OwnerSwarm(swarm));
    let source = common::spawn_stockpile(&mut app, opening_position + Vec2::Y * 300.0, 59, 60);
    app.world_mut().entity_mut(source).insert(OwnerSwarm(swarm));

    app.update();
    app.world_mut()
        .get_mut::<ResourceDeposit>(mine)
        .unwrap()
        .amount -= 1;
    for _ in 0..120 {
        app.update();
    }
    let grid = app.world().resource::<IntentGrid>();
    assert!(
        grid.cell(opening)
            .unwrap()
            .has_owned(IntentKind::Gather, owner)
    );
    assert!(
        !grid
            .cell(expansion)
            .is_some_and(|cell| cell.has_owned(IntentKind::Gather, owner)),
        "existing stock and an extraction pulse cannot stand in for sustained Worker drop-offs"
    );
}

#[derive(Clone, Copy)]
enum Deliveries {
    Ongoing,
    OtherSwarm,
    OtherMine,
    Stall,
    Burst,
}

#[derive(Debug, Clone, Copy)]
enum SupplySituation {
    Shortfall,
    HealthyReserve,
    AdequateFlow,
    BriefSpendingIncrease,
    CommittedWorker,
    CommittedHauler,
    DeliveryBottleneck,
    BufferedConsumption,
}

#[test]
fn strategic_adapter_uses_sustained_local_dropoffs_and_reachable_source_capacity() {
    for (capacity, deliveries, split_work_ring, should_expand) in [
        (4, Deliveries::Ongoing, false, true),
        (4, Deliveries::Ongoing, true, true),
        (60, Deliveries::Ongoing, false, false),
        (4, Deliveries::OtherSwarm, false, false),
        (4, Deliveries::OtherMine, false, false),
        (4, Deliveries::Stall, false, false),
        (4, Deliveries::Burst, false, false),
    ] {
        review_expansion(
            capacity,
            deliveries,
            split_work_ring,
            SupplySituation::Shortfall,
            should_expand,
        );
    }
}

#[test]
fn strategic_adapter_defers_expansion_when_reserves_are_healthy() {
    review_expansion(
        4,
        Deliveries::Ongoing,
        false,
        SupplySituation::HealthyReserve,
        false,
    );
}

#[test]
fn strategic_adapter_defers_expansion_when_income_already_covers_spending() {
    review_expansion(
        4,
        Deliveries::Ongoing,
        false,
        SupplySituation::AdequateFlow,
        false,
    );
}

#[test]
fn strategic_adapter_defers_expansion_after_a_brief_spending_increase() {
    review_expansion(
        4,
        Deliveries::Ongoing,
        false,
        SupplySituation::BriefSpendingIncrease,
        false,
    );
}

#[test]
fn strategic_adapter_defers_expansion_when_workers_are_committed() {
    review_expansion(
        4,
        Deliveries::Ongoing,
        false,
        SupplySituation::CommittedWorker,
        false,
    );
}

#[test]
fn strategic_adapter_defers_expansion_when_haulers_are_committed() {
    review_expansion(
        4,
        Deliveries::Ongoing,
        false,
        SupplySituation::CommittedHauler,
        false,
    );
}

#[test]
fn strategic_adapter_keeps_existing_supply_supported_during_delivery_bottlenecks() {
    review_expansion(
        4,
        Deliveries::Ongoing,
        false,
        SupplySituation::DeliveryBottleneck,
        false,
    );
}

#[test]
fn strategic_adapter_requires_haul_activity_to_confirm_delivery() {
    review_expansion(
        4,
        Deliveries::Ongoing,
        false,
        SupplySituation::BufferedConsumption,
        false,
    );
}

fn review_expansion(
    capacity: u32,
    deliveries: Deliveries,
    split_work_ring: bool,
    supply: SupplySituation,
    should_expand: bool,
) {
    use top_down_2d_rts_prototype_nano_swarm::{
        nanobot::{
            ActionableProjection, Cargo, Charge, Charger, ChargerAssignment, ChargerProgress,
            ChargerPulseProgress, ExtractProgress, GatherAssignment, GatherQuotas, HaulQuotas,
            HaulerLoading, InteractionRegion, LogisticsReservation, MaintenanceAssignment,
            NanobotType, OwnerSwarm, PopulationDemandPlugin, ProductionFacility,
            ReturningToStockpile, SwarmMember, TerritorySnapshot, defender_charger_work_system,
            gather_quota_system, project_territory_snapshot_system, worker_gather_delivery_system,
        },
        resources::Stockpile,
        terrain::RockFormation,
        tuning::Tuning,
    };

    let owner = SwarmId::PLAYER;
    let foreign = SwarmId(3);
    let home = common::cell_world_center(IVec2::ZERO);
    let opening = IVec2::new(2, 0);
    let expansion = IVec2::new(0, 5);
    let deposit_position = common::cell_world_center(opening);
    let source_position = deposit_position + Vec2::new(-100.0, 340.0);
    let delivery_owner = if matches!(deliveries, Deliveries::OtherSwarm) {
        foreign
    } else {
        owner
    };
    let mut app = common::minimal_app();
    app.insert_resource(IntentGrid::new(16, 16))
        .init_resource::<ActionableProjection>()
        .init_resource::<TerritorySnapshot>()
        .add_plugins((StrategicControllerPlugin, PopulationDemandPlugin))
        .add_systems(
            FixedUpdate,
            (
                project_territory_snapshot_system.before(gather_quota_system),
                worker_gather_delivery_system,
                defender_charger_work_system,
            ),
        );
    let mut tuning = app.world_mut().resource_mut::<Tuning>();
    tuning.logistics.source_proximity_radius = 300.0;
    tuning.logistics.hauler_transfer_per_pulse = 2;
    tuning.production.cost_per_bot = 5;
    tuning.production.ticks_per_bot = 120;
    tuning.production.reserve_seconds = 10.0;
    tuning.charge.drain_per_tick = 0.0;
    tuning.charge.pulse_interval_ticks = 1;
    tuning.charge.material_per_pulse = 2;
    if matches!(
        supply,
        SupplySituation::AdequateFlow | SupplySituation::BriefSpendingIncrease
    ) {
        tuning.production.ticks_per_bot = 1200;
    }
    let committed_worker = matches!(supply, SupplySituation::CommittedWorker);
    let committed_haulers = matches!(supply, SupplySituation::CommittedHauler);
    let swarm = common::spawn_swarm_with_nanobots(
        &mut app,
        home,
        &[
            (NanobotType::Worker, if committed_worker { 2 } else { 3 }),
            (NanobotType::Hauler, if committed_haulers { 2 } else { 4 }),
        ],
    );
    app.world_mut()
        .entity_mut(swarm)
        .insert(StrategicController::new(owner));
    let foreign_home = Vec2::new(-2000.0, 0.0);
    let foreign_swarm = app
        .world_mut()
        .spawn((
            Swarm {},
            foreign,
            Transform::from_translation(foreign_home.extend(0.0)),
        ))
        .id();
    let mut obstacles = Vec::new();
    if split_work_ring {
        let rock = RockFormation::Rectangle {
            half: Vec2::new(22.0, 5000.0),
        };
        let transform = Transform::from_translation(deposit_position.extend(0.0));
        obstacles.push(rock.obstacle(&transform));
        app.world_mut().spawn((rock, transform));
        common::spawn_stockpile(&mut app, deposit_position + Vec2::X * 160.0, 0, 60);
    }
    let reachability = SwarmReachabilityMap::new(
        app.world().resource::<IntentGrid>(),
        obstacles,
        [(owner, home), (foreign, foreign_home)],
    )
    .unwrap();
    app.insert_resource(reachability);

    let [mine, ..] = [opening, expansion, IVec2::new(5, 4)].map(|cell| {
        common::spawn_deposit(
            &mut app,
            common::DepositFixture {
                world_pos: common::cell_world_center(cell),
                amount: 72_000,
                capacity: 72_000,
                radius: 64.0,
            },
        )
    });
    app.world_mut().entity_mut(mine).insert(OwnerSwarm(swarm));
    app.world_mut().spawn((
        ProductionFacility {
            demand_waiting: true,
            ..ProductionFacility::new()
        },
        OwnerSwarm(swarm),
        Transform::from_translation((home - Vec2::X * 160.0).extend(0.0)),
    ));
    let charger = common::spawn_charger(
        &mut app,
        common::ChargerFixture {
            cell: IVec2::ZERO,
            amount: 0,
            ticks_since_maintained: 0,
        },
    );
    app.world_mut()
        .entity_mut(charger)
        .insert(OwnerSwarm(swarm));
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(IVec2::ZERO, IntentKind::Defend, owner);
    let charger_region =
        InteractionRegion::structure(app.world().get::<Transform>(charger).unwrap());
    let defender = common::spawn_stationary_defender_at(
        &mut app,
        charger_region.approach(home + Vec2::Y * 160.0),
        owner,
    );
    if matches!(supply, SupplySituation::HealthyReserve) {
        let sink = common::spawn_sink_stockpile(&mut app, home - Vec2::Y * 180.0, 100, 120);
        app.world_mut().entity_mut(sink).insert(OwnerSwarm(swarm));
    }
    let world = app.world_mut();
    if committed_worker {
        let worker = world
            .query::<(Entity, &NanobotType)>()
            .iter(world)
            .find_map(|(entity, kind)| (*kind == NanobotType::Worker).then_some(entity))
            .unwrap();
        world.entity_mut(worker).insert(MaintenanceAssignment {
            cell: IVec2::ZERO,
            target: charger,
        });
    }
    let in_transit_haulers: Vec<_> = {
        world
            .query::<(Entity, &NanobotType)>()
            .iter(world)
            .filter_map(|(entity, kind)| (*kind == NanobotType::Hauler).then_some(entity))
            .take(match supply {
                SupplySituation::CommittedHauler => 2,
                SupplySituation::BufferedConsumption => 0,
                _ => 1,
            })
            .collect()
    };
    for hauler in in_transit_haulers {
        world.entity_mut(hauler).insert(HaulerLoading);
    }

    let second_mine = common::spawn_deposit(
        &mut app,
        common::DepositFixture {
            world_pos: deposit_position + Vec2::Y * 430.0,
            amount: 72_000,
            capacity: 72_000,
            radius: 64.0,
        },
    );
    app.world_mut()
        .entity_mut(second_mine)
        .insert(OwnerSwarm(foreign_swarm));
    let source = common::spawn_stockpile(&mut app, source_position, capacity - 4, capacity);
    let source_region = InteractionRegion::structure(app.world().get::<Transform>(source).unwrap());
    let deposit_approach = InteractionRegion::deposit(
        &Transform::from_translation(deposit_position.extend(0.0)),
        64.0,
    )
    .approach(source_position);
    let worker = common::spawn_worker_at(&mut app, deposit_approach);
    let other_deposit = matches!(deliveries, Deliveries::OtherMine);
    let delivery_worker = if delivery_owner == owner && !other_deposit {
        worker
    } else {
        let entity = common::spawn_worker_at(&mut app, source_region.approach(deposit_position));
        app.world_mut()
            .entity_mut(entity)
            .insert(SwarmMember::new(delivery_owner));
        entity
    };
    let origin = if other_deposit { second_mine } else { mine };
    let mut transfers = 0;
    let mut feeding_terminal = false;
    for tick in 0..1560 {
        if tick == 1350 && matches!(supply, SupplySituation::BriefSpendingIncrease) {
            app.world_mut()
                .resource_mut::<Tuning>()
                .production
                .ticks_per_bot = 120;
        }
        let active_claim = LogisticsReservation::new(mine, source, ResourceKind::Minerals, 4);
        app.world_mut().entity_mut(worker).insert((
            GatherAssignment::new(opening, mine),
            ExtractProgress::default(),
            active_claim,
            Transform::from_translation(deposit_approach.extend(0.0)),
        ));
        let deliver = match deliveries {
            Deliveries::Burst => matches!(tick, 357 | 362),
            Deliveries::Stall => tick % 240 == 30 && tick < 300,
            _ => tick % 240 == 30,
        };
        if deliver {
            let mut reservation =
                LogisticsReservation::new(origin, source, ResourceKind::Minerals, 4);
            reservation.source_remaining = 0;
            app.world_mut()
                .entity_mut(delivery_worker)
                .insert((
                    ReturningToStockpile { stockpile: source },
                    reservation,
                    Cargo {
                        kind: ResourceKind::Minerals,
                        amount: 2,
                    },
                    Transform::from_translation(
                        source_region.approach(deposit_position).extend(0.0),
                    ),
                ))
                .remove::<ExtractProgress>();
        }
        if matches!(deliveries, Deliveries::Stall) && tick == 300 {
            app.world_mut().get_mut::<Stockpile>(source).unwrap().amount = capacity;
        }
        if capacity > 4 {
            app.world_mut().get_mut::<Stockpile>(source).unwrap().amount = 0;
        }
        if feeding_terminal {
            app.world_mut().get_mut::<Charger>(charger).unwrap().amount = 2;
            let mut defender = app.world_mut().entity_mut(defender);
            defender.get_mut::<Charge>().unwrap().current = 0.0;
            defender.insert((
                ChargerAssignment { charger },
                ChargerProgress { charger },
                ChargerPulseProgress::default(),
            ));
        }
        app.update();
        if feeding_terminal {
            assert_eq!(app.world().get::<Charger>(charger).unwrap().amount, 0);
        }
        feeding_terminal = deliver && !matches!(supply, SupplySituation::DeliveryBottleneck);
        if deliver {
            let mut stock = app.world_mut().get_mut::<Stockpile>(source).unwrap();
            assert!(
                stock.amount > if capacity == 4 { capacity - 4 } else { 0 },
                "the Worker fixture must actually transfer its load to the Source"
            );
            transfers += 1;
            stock.amount -= 2;
        }
        let grid = app.world().resource::<IntentGrid>();
        assert!(
            grid.cell(opening)
                .is_some_and(|cell| cell.has_owned(IntentKind::Gather, owner)),
            "opening mine lost Gather at tick {tick}, split={split_work_ring}, situation={supply:?}, telemetry={:?}",
            app.world().resource::<top_down_2d_rts_prototype_nano_swarm::strategic_runtime::ControllerTelemetry>().profiles()
        );
        let expanded = grid
            .cell(expansion)
            .is_some_and(|cell| cell.has_owned(IntentKind::Gather, owner));
        if expanded {
            assert!(
                should_expand,
                "unexpected claim at tick {tick} with {supply:?}, capacity {capacity}, source deposits from {delivery_owner:?}, other deposit {other_deposit}. Gather: {:?}. Haul: {:?}",
                app.world().resource::<GatherQuotas>().economy(owner),
                app.world().resource::<HaulQuotas>().economy(owner),
            );
            assert!(tick >= 12 * 60, "the claim needs sustained activity");
            assert!(
                grid.cell(expansion - IVec2::Y)
                    .is_some_and(|cell| cell.has_owned(IntentKind::Build, owner)
                        && cell.has_owned(IntentKind::Corridor, owner))
            );
            assert!(
                grid.cell(expansion)
                    .unwrap()
                    .has_owned(IntentKind::Defend, owner)
            );
            assert!(
                !grid
                    .cell(IVec2::new(5, 4))
                    .is_some_and(|cell| cell.has_owned(IntentKind::Gather, owner))
            );
            break;
        }
        if tick == 1559 && should_expand {
            let profiles = app
                .world()
                .resource::<top_down_2d_rts_prototype_nano_swarm::strategic_runtime::ControllerTelemetry>()
                .profiles();
            panic!(
                "one active Worker with {transfers} completed drop-offs can sustain a four-unit reachable Source with its capacity reserved (split work ring {split_work_ring}): {profiles:?}"
            );
        }
    }
}

mod mine_recovery {
    use super::*;
    use top_down_2d_rts_prototype_nano_swarm::{
        SIMULATION_HZ,
        nanobot::{
            Cargo, ExtractProgress, GatherAssignment, InteractionRegion, LogisticsReservation,
            NanobotType, OwnerSwarm, ReturningToStockpile, deterministic_jitter,
            worker_gather_delivery_system,
        },
        navigation::align_structure,
        resources::{ResourceDeposit, Stockpile},
        terrain::RockFormation,
        tuning::Tuning,
    };

    const OWNER: SwarmId = SwarmId::PLAYER;
    const FAILED: IVec2 = IVec2::new(3, 0);
    const SURVIVOR: IVec2 = IVec2::new(-3, 0);
    const REPLACEMENT: IVec2 = IVec2::new(0, 3);
    const RADIUS: f32 = 80.0;

    fn fixture(blocked: bool) -> (App, Entity, Entity) {
        let mut app = common::minimal_app();
        app.insert_resource(IntentGrid::new(16, 16))
            .add_plugins(StrategicControllerPlugin)
            .add_systems(FixedUpdate, worker_gather_delivery_system);
        app.world_mut()
            .resource_mut::<Tuning>()
            .logistics
            .hauler_transfer_per_pulse = 4;
        let home = common::cell_world_center(IVec2::ZERO);
        let swarm = common::spawn_swarm_with_nanobots(&mut app, home, &[(NanobotType::Worker, 4)]);
        app.world_mut()
            .entity_mut(swarm)
            .insert(StrategicController::new(OWNER));
        for cell in [SURVIVOR, REPLACEMENT] {
            let mine = deposit(&mut app, cell);
            if cell == SURVIVOR {
                app.world_mut().entity_mut(mine).insert(OwnerSwarm(swarm));
            }
            source(&mut app, cell);
        }
        let mine = deposit(&mut app, FAILED);
        app.world_mut().entity_mut(mine).insert(OwnerSwarm(swarm));
        let source = source(&mut app, FAILED);
        let mut rocks = Vec::new();
        if blocked {
            let center = common::cell_world_center(FAILED);
            for index in 0..8 {
                let angle = index as f32 * std::f32::consts::TAU / 8.0;
                let transform = align_structure(Transform::from_translation(
                    (center
                        + 164.0 * Vec2::from_angle(angle)
                        + deterministic_jitter(index, FAILED, 16.0))
                    .extend(0.0),
                ));
                let rock = RockFormation::Rectangle {
                    half: Vec2::splat(12.0),
                };
                rocks.push(rock.obstacle(&transform));
                app.world_mut().spawn((rock, transform));
            }
        }
        let reachability =
            SwarmReachabilityMap::new(app.world().resource::<IntentGrid>(), rocks, [(OWNER, home)])
                .unwrap();
        app.insert_resource(reachability);
        app.update();
        assert!(owns(&app, FAILED, IntentKind::Gather));
        assert_support(&app, SURVIVOR, SURVIVOR + IVec2::X);
        drop_off(&mut app, mine, source);
        (app, mine, source)
    }

    fn deposit(app: &mut App, cell: IVec2) -> Entity {
        common::spawn_deposit(
            app,
            common::DepositFixture {
                world_pos: common::cell_world_center(cell),
                amount: 72_000,
                capacity: 72_000,
                radius: RADIUS,
            },
        )
    }

    fn source(app: &mut App, cell: IVec2) -> Entity {
        let offset = if cell == SURVIVOR { 330.0 } else { -330.0 };
        common::spawn_stockpile(
            app,
            common::cell_world_center(cell) + offset * Vec2::Y,
            0,
            4,
        )
    }

    fn drop_off(app: &mut App, mine: Entity, source: Entity) {
        let mine_pos = app
            .world()
            .get::<Transform>(mine)
            .unwrap()
            .translation
            .truncate();
        let approach = InteractionRegion::structure(app.world().get::<Transform>(source).unwrap())
            .approach(mine_pos);
        let worker = common::spawn_worker_at(app, approach);
        let mut reservation = LogisticsReservation::new(mine, source, ResourceKind::Minerals, 4);
        reservation.source_remaining = 0;
        app.world_mut().entity_mut(worker).insert((
            GatherAssignment::new(FAILED, mine),
            ExtractProgress::default(),
        ));
        app.update();
        app.world_mut()
            .entity_mut(worker)
            .insert((
                ReturningToStockpile { stockpile: source },
                reservation,
                Cargo {
                    kind: ResourceKind::Minerals,
                    amount: 4,
                },
            ))
            .remove::<ExtractProgress>();
        app.update();
        assert_eq!(app.world().get::<Stockpile>(source).unwrap().amount, 4);
        app.world_mut().despawn(worker);
        app.update();
    }

    fn run_ticks(app: &mut App, seconds: u32) {
        for _ in 0..seconds * SIMULATION_HZ as u32 {
            app.update();
            assert_support(app, SURVIVOR, SURVIVOR + IVec2::X);
        }
    }

    fn add_hauler(app: &mut App) -> Entity {
        common::spawn_hauler_at(app, common::cell_world_center(IVec2::ZERO))
    }

    fn owns(app: &App, cell: IVec2, kind: IntentKind) -> bool {
        app.world()
            .resource::<IntentGrid>()
            .cell(cell)
            .is_some_and(|intent| intent.has_owned(kind, OWNER))
    }

    fn assert_support(app: &App, gather: IVec2, build: IVec2) {
        assert!(owns(app, gather, IntentKind::Gather), "Gather at {gather}");
        assert!(owns(app, gather, IntentKind::Defend), "Defend at {gather}");
        assert!(owns(app, build, IntentKind::Build), "Build at {build}");
        assert!(
            owns(app, build, IntentKind::Corridor),
            "Corridor at {build}"
        );
    }

    #[test]
    fn replaces_a_proven_stall_only_after_a_staffed_replacement_is_ready() {
        let (mut app, _, source) = fixture(true);
        app.world_mut().despawn(source);
        let hauler = add_hauler(&mut app);
        run_ticks(&mut app, 8);
        assert_support(&app, FAILED, FAILED - IVec2::X);
        assert!(!owns(&app, REPLACEMENT, IntentKind::Gather));
        app.world_mut().despawn(hauler);

        run_ticks(&mut app, 10);
        assert_support(&app, FAILED, FAILED - IVec2::X);
        assert!(!owns(&app, REPLACEMENT, IntentKind::Gather));

        add_hauler(&mut app);
        run_ticks(&mut app, 2);
        assert_support(&app, REPLACEMENT, REPLACEMENT - IVec2::Y);
        assert!(!owns(&app, FAILED, IntentKind::Gather));
        assert!(!owns(&app, FAILED, IntentKind::Corridor));
        run_ticks(&mut app, 2);
        assert!(!owns(&app, FAILED, IntentKind::Gather));
    }

    #[test]
    fn an_exhausted_mine_leaves_a_vacancy_across_reviews() {
        let (mut app, mine, _) = fixture(false);
        app.world_mut()
            .get_mut::<ResourceDeposit>(mine)
            .unwrap()
            .amount = 0;
        run_ticks(&mut app, 2);
        assert!(!owns(&app, FAILED, IntentKind::Gather));
        assert!(!owns(&app, REPLACEMENT, IntentKind::Gather));

        add_hauler(&mut app);
        run_ticks(&mut app, 2);
        assert_support(&app, REPLACEMENT, REPLACEMENT - IVec2::Y);
    }

    #[test]
    fn a_disconnected_empty_source_can_prevent_automatic_restoration() {
        let (mut app, _, source) = fixture(false);
        app.world_mut().get_mut::<Stockpile>(source).unwrap().amount = 0;
        app.world_mut().spawn((
            RockFormation::Rectangle {
                half: Vec2::new(
                    8.0 * top_down_2d_rts_prototype_nano_swarm::ZONE_BLOCK_SIZE,
                    24.0,
                ),
            },
            Transform::default(),
        ));
        add_hauler(&mut app);
        run_ticks(&mut app, 16);
        assert_support(&app, REPLACEMENT, REPLACEMENT - IVec2::Y);
        assert!(!owns(&app, FAILED, IntentKind::Gather));
    }

    #[test]
    fn keeps_a_full_or_restorable_source_path_supported_until_worker_deliveries_resume() {
        let (mut app, mine, old_source) = fixture(false);
        add_hauler(&mut app);
        run_ticks(&mut app, 18);
        assert_support(&app, FAILED, FAILED - IVec2::X);
        assert!(!owns(&app, REPLACEMENT, IntentKind::Gather));
        app.world_mut()
            .get_mut::<Stockpile>(old_source)
            .unwrap()
            .amount = 0;
        drop_off(&mut app, mine, old_source);

        app.world_mut().despawn(old_source);
        run_ticks(&mut app, 18);
        assert_support(&app, FAILED, FAILED - IVec2::X);
        assert!(!owns(&app, REPLACEMENT, IntentKind::Gather));
        let new_source = source(&mut app, FAILED);
        drop_off(&mut app, mine, new_source);
        assert_support(&app, FAILED, FAILED - IVec2::X);
    }
}

#[test]
fn controller_keeps_several_deposits_extracting_and_hauling() {
    use top_down_2d_rts_prototype_nano_swarm::{
        nanobot::{Cargo, HaulerAssignment, NanobotType, OwnerSwarm},
        resources::ResourceDeposit,
    };

    let mut app = common::sim_app_with_gather_haul();
    app.add_plugins(StrategicControllerPlugin);
    let home = common::cell_world_center(IVec2::ZERO);
    let own = common::spawn_swarm_with_nanobots(
        &mut app,
        home,
        &[(NanobotType::Worker, 9), (NanobotType::Hauler, 4)],
    );
    app.world_mut()
        .entity_mut(own)
        .insert(StrategicController::new(SwarmId::PLAYER));
    install_empty_reachability(&mut app, [SwarmId::PLAYER]);

    let cells = [IVec2::new(-1, 0), IVec2::new(1, 0), IVec2::new(0, 1)];
    for cell in cells {
        app.world_mut().resource_mut::<IntentGrid>().paint(
            cell,
            IntentKind::Gather,
            SwarmId::PLAYER,
        );
    }
    let sites = cells.map(|cell| {
        let position = common::cell_world_center(cell);
        let deposit = common::spawn_deposit(
            &mut app,
            common::DepositFixture {
                world_pos: position,
                amount: 72_000,
                capacity: 72_000,
                radius: 80.0,
            },
        );
        let source = common::spawn_stockpile(
            &mut app,
            position + (home - position).normalize() * 140.0,
            0,
            200,
        );
        app.world_mut().entity_mut(source).insert(OwnerSwarm(own));
        (deposit, source)
    });
    let sink = common::spawn_sink_stockpile(&mut app, home - Vec2::Y * 180.0, 0, 2_000);
    app.world_mut().entity_mut(sink).insert(OwnerSwarm(own));

    let mut loaded_from_site = [false; 3];
    for _ in 0..3600 {
        app.update();
        let world = app.world_mut();
        for (assignment, cargo) in world.query::<(&HaulerAssignment, &Cargo)>().iter(world) {
            for (index, (_, source)) in sites.iter().enumerate() {
                loaded_from_site[index] |= assignment.source == *source && cargo.amount > 0;
            }
        }
        if loaded_from_site.iter().all(|loaded| *loaded) {
            break;
        }
    }
    let world = app.world();
    let grid = world.resource::<IntentGrid>();
    for (index, (deposit, _)) in sites.into_iter().enumerate() {
        assert!(
            grid.cell(cells[index])
                .is_some_and(|cell| cell.has_owned(IntentKind::Gather, SwarmId::PLAYER)),
            "site {} lost Gather paint",
            cells[index]
        );
        assert!(world.get::<ResourceDeposit>(deposit).unwrap().amount < 72_000);
        assert!(
            loaded_from_site[index],
            "site {} never filled a hauler",
            cells[index]
        );
    }
}

#[test]
fn controller_retains_the_progressing_fronts_shared_defender_response() {
    use top_down_2d_rts_prototype_nano_swarm::nanobot::{
        DefenderResponse, OwnerSwarm, PlannedKind, Structure, StructureKind,
    };
    use top_down_2d_rts_prototype_nano_swarm::strategic_runtime::ControllerTelemetry;

    let mut app = common::sim_app();
    app.insert_resource(IntentGrid::new(64, 64));
    app.add_plugins(StrategicControllerPlugin);
    let home = common::cell_world_center(IVec2::ZERO);
    let own = common::spawn_swarm_at(&mut app, home);
    app.world_mut()
        .entity_mut(own)
        .insert(StrategicController::new(SwarmId::PLAYER));
    install_empty_reachability(&mut app, [SwarmId::PLAYER]);
    app.world_mut()
        .resource_mut::<top_down_2d_rts_prototype_nano_swarm::tuning::Tuning>()
        .autonomy
        .strategy_staging_spacing_cells = 64;
    let enemy_cell = IVec2::new(12, 0);
    let enemy = app
        .world_mut()
        .spawn((
            Swarm {},
            SwarmId(9),
            Transform::from_translation(common::cell_world_center(enemy_cell).extend(0.0)),
        ))
        .id();
    app.world_mut().resource_mut::<ResourceLedger>().add_for(
        SwarmId::PLAYER,
        ResourceKind::Minerals,
        2_000,
    );
    let support = common::spawn_charger(
        &mut app,
        common::ChargerFixture {
            cell: IVec2::ZERO,
            amount: 100,
            ticks_since_maintained: 0,
        },
    );
    app.world_mut().entity_mut(support).insert(OwnerSwarm(own));
    let old_front =
        common::spawn_empty_completed_structure(&mut app, enemy, PlannedKind::SinkStockpile);
    app.world_mut().entity_mut(old_front).insert((
        Structure::new(StructureKind::Basic),
        Transform::from_translation(common::cell_world_center(enemy_cell).extend(0.0)),
    ));
    let defenders = (0..8)
        .map(|index| {
            common::spawn_defender_at(
                &mut app,
                common::cell_world_center(enemy_cell)
                    + Vec2::new(-160.0, index as f32 * 48.0 - 168.0),
            )
        })
        .collect::<Vec<_>>();
    let mut original_responder = None;
    for _ in 0..DEFENDER_RESPONSE_TICK_BUDGET {
        app.update();
        original_responder = defenders.iter().copied().find(|defender| {
            app.world()
                .get::<DefenderResponse>(*defender)
                .is_some_and(|response| response.target == old_front)
        });
        if original_responder.is_some() {
            break;
        }
    }
    let original_responder = original_responder.unwrap_or_else(|| {
        panic!(
            "the existing front must have a real shared-autonomy response; profiles={:?}",
            app.world()
                .resource::<top_down_2d_rts_prototype_nano_swarm::strategic_runtime::ControllerTelemetry>()
                .profiles()
        )
    });

    app.world_mut()
        .get_mut::<Structure>(old_front)
        .unwrap()
        .health = 80;
    let new_front = common::spawn_charger(
        &mut app,
        common::ChargerFixture {
            cell: IVec2::new(3, 5),
            amount: 100,
            ticks_since_maintained: 0,
        },
    );
    app.world_mut()
        .entity_mut(new_front)
        .insert(OwnerSwarm(enemy));
    app.world_mut()
        .get_mut::<Structure>(new_front)
        .unwrap()
        .health = 20;
    let prior_reviews = app.world().resource::<ControllerTelemetry>().profiles()[&0].reviews;
    for _ in 0..DEFENDER_RESPONSE_TICK_BUDGET {
        app.update();
        if app.world().resource::<ControllerTelemetry>().profiles()[&0].reviews > prior_reviews {
            break;
        }
    }
    assert!(app.world().resource::<ControllerTelemetry>().profiles()[&0].reviews > prior_reviews);

    assert!(
        !app.world()
            .resource::<IntentGrid>()
            .cell(IVec2::new(3, 5))
            .unwrap()
            .has_owned(IntentKind::Defend, SwarmId::PLAYER),
        "progress at an active front must keep a competing target from taking its place"
    );
    assert_eq!(
        app.world()
            .get::<DefenderResponse>(original_responder)
            .map(|response| response.target),
        Some(old_front),
        "the progressing attack must preserve its shared-autonomy response"
    );
}

#[test]
fn economic_defend_paint_changes_real_defender_population_demand() {
    use top_down_2d_rts_prototype_nano_swarm::{
        nanobot::{NanobotType, PopulationDemand},
        strategic_controller::{Controller, GameState, StructureKind, StructureState, SwarmState},
        tuning::AutonomyTuning,
    };

    let mut app = common::sim_app_with_population_demand();
    app.insert_resource(IntentGrid::new(24, 24));
    let owner = SwarmId::PLAYER;
    let home = common::cell_world_center(IVec2::ZERO);
    app.world_mut().spawn((
        Swarm {},
        owner,
        Transform::from_translation(home.extend(0.0)),
    ));
    let tuning = AutonomyTuning::default();
    let mut controller = Controller::new(owner);
    let mut swarms = [SwarmState {
        id: owner,
        home,
        minerals: tuning.strategy_mineral_reserve,
        mineral_income_per_second: 0.0,
    }];
    let primary = StructureState {
        id: 1,
        owner,
        position: Some(home),
        kind: StructureKind::Charger,
        operational: true,
        health: 100,
        max_health: 100,
        minerals: tuning.strategy_charger_reserve,
    };
    let chargers = [
        primary,
        StructureState {
            id: primary.id + 1,
            position: Some(home + Vec2::Y * top_down_2d_rts_prototype_nano_swarm::ZONE_BLOCK_SIZE),
            ..primary
        },
    ];
    let mut demand_after_review = |app: &mut App, swarms: &[SwarmState], tick| {
        let decision = controller.decide(
            &tuning,
            &GameState {
                grid: app.world().resource::<IntentGrid>(),
                swarms,
                bots: &[],
                structures: &chargers,
                deposits: &[],
                terrain: &[],
                charge: &DEFAULT_CHARGE,
                reachability: None,
                route_query: None,
                mine_query: None,
                supply: None,
                build_footprints: &[],
                tick,
                finished: false,
            },
        );
        apply_decision(
            &mut app.world_mut().resource_mut::<IntentGrid>(),
            owner,
            &decision,
        );
        app.update();
        app.world()
            .resource::<PopulationDemand>()
            .desired_for(owner, NanobotType::Defender)
    };
    let initial = demand_after_review(&mut app, &swarms, 0);
    assert!(initial > 0, "initial paint creates real Defender demand");
    swarms[0].mineral_income_per_second = 12.0;
    let funded = demand_after_review(&mut app, &swarms, tuning.controller_review_period_ticks);
    assert!(
        funded > initial,
        "expanded paint increased demand from {initial} to {funded}"
    );
    swarms[0].minerals = 0;
    let depleted =
        demand_after_review(&mut app, &swarms, tuning.controller_review_period_ticks * 2);
    assert!(
        depleted < funded,
        "erasing unaffordable paint reduces real demand"
    );
}

#[test]
fn stable_build_room_gives_production_pressure_a_site_outside_the_home_cell() {
    use top_down_2d_rts_prototype_nano_swarm::{
        nanobot::{PlannedKind, PlannedStructure},
        terrain::RockFormation,
        tuning::Tuning,
    };

    let mut app = common::sim_app_with_production();
    app.add_plugins(StrategicControllerPlugin);
    let owner = SwarmId::PLAYER;
    let home_cell = IVec2::ZERO;
    // Home is in a narrow reachable strip; a new facility needs a neighboring tile.
    let home = Vec2::new(496.0, 256.0);
    let rock = RockFormation::Rectangle {
        half: Vec2::new(236.0, 512.0),
    };
    let rock_transform = Transform::from_xyz(220.0, 512.0, 0.0);
    let swarm = common::spawn_swarm_at(&mut app, home);
    app.world_mut()
        .entity_mut(swarm)
        .insert(StrategicController::new(owner));
    app.world_mut().spawn((rock, rock_transform));
    common::spawn_worker_at(&mut app, Vec2::new(600.0, 256.0));
    app.world_mut()
        .resource_mut::<Tuning>()
        .production
        .pressure_ticks = 1;
    let reachability = SwarmReachabilityMap::new(
        app.world().resource::<IntentGrid>(),
        [rock.obstacle(&rock_transform)],
        [(owner, home)],
    )
    .unwrap();
    app.insert_resource(reachability);

    let mut planned_cell = None;
    for _ in 0..90 {
        app.update();
        let world = app.world_mut();
        planned_cell = world
            .query::<&PlannedStructure>()
            .iter(world)
            .find(|plan| plan.kind == PlannedKind::ProductionFacility)
            .map(|plan| plan.cell);
        if planned_cell.is_some() {
            break;
        }
    }
    let cell = planned_cell.expect("Production Pressure must use the controller's Build room");
    assert_eq!((cell - home_cell).abs().element_sum(), 1);
    assert!(
        app.world()
            .resource::<IntentGrid>()
            .cell(cell)
            .unwrap()
            .has_owned(IntentKind::Build, owner)
    );
}

mod route_reuse {
    use super::*;
    use top_down_2d_rts_prototype_nano_swarm::{
        strategic_runtime::{ControllerTelemetry, reconcile_controller_tuning},
        terrain::RockFormation,
        tuning::Tuning,
    };

    const OWNER: SwarmId = SwarmId::PLAYER;

    /// A home and a deposit separated by a rock, so every review needs a route around it.
    fn rock_fixture() -> App {
        let mut app = common::minimal_app();
        app.insert_resource(IntentGrid::new(16, 16));
        app.add_plugins(StrategicControllerPlugin);
        let home = common::cell_world_center(IVec2::new(-3, 0));
        let rock = RockFormation::Rectangle {
            half: Vec2::new(256.0, 650.0),
        };
        let rock_transform =
            Transform::from_translation(common::cell_world_center(IVec2::ZERO).extend(0.0));
        let swarm = app
            .world_mut()
            .spawn((
                Swarm {},
                OWNER,
                Transform::from_translation(home.extend(0.0)),
                StrategicController::new(OWNER),
            ))
            .id();
        app.world_mut().spawn((rock, rock_transform));
        let deposit = common::spawn_deposit(
            &mut app,
            common::DepositFixture {
                world_pos: common::cell_world_center(IVec2::new(3, 0)),
                amount: 72_000,
                capacity: 72_000,
                radius: 80.0,
            },
        );
        app.world_mut().entity_mut(deposit).insert(
            top_down_2d_rts_prototype_nano_swarm::nanobot::OwnerSwarm(swarm),
        );
        let reachability = SwarmReachabilityMap::new(
            app.world().resource::<IntentGrid>(),
            [rock.obstacle(&rock_transform)],
            [(OWNER, home)],
        )
        .unwrap();
        app.insert_resource(reachability);
        app.update();
        app
    }

    fn work(app: &App) -> u64 {
        app.world().resource::<ControllerTelemetry>().profiles()[&OWNER.0].work_units
    }

    /// Force one review through a tuning edit and return the work units it spent.
    fn forced_review_work(app: &mut App) -> u64 {
        let before = work(app);
        let old = app.world().resource::<Tuning>().clone();
        let mut new = old.clone();
        new.autonomy.strategy_home_income_lead += 1.0;
        reconcile_controller_tuning(app.world_mut(), &old, &new);
        app.insert_resource(new);
        app.update();
        work(app) - before
    }

    #[test]
    fn revision_bump_from_a_structure_off_the_route_costs_no_extra_search() {
        let mut app = rock_fixture();
        let steady = forced_review_work(&mut app);
        common::spawn_structure_at(&mut app, Vec2::new(256.0, 1000.0));
        assert_eq!(forced_review_work(&mut app), steady);
    }

    #[test]
    fn structure_across_the_cached_route_forces_a_new_search() {
        let mut app = rock_fixture();
        let steady = forced_review_work(&mut app);
        common::spawn_structure_at(&mut app, Vec2::new(256.0, -450.0));
        assert!(forced_review_work(&mut app) > steady);
    }

    #[test]
    fn repeated_review_of_the_same_tiles_spends_less_once_planning_points_are_cached() {
        use top_down_2d_rts_prototype_nano_swarm::{
            nanobot::NanobotType,
            strategic_controller::{BotState, Controller, GameState, SwarmState},
            tuning::AutonomyTuning,
        };
        let grid = IntentGrid::new(64, 64);
        let enemy = SwarmId(9);
        let world = |cell: IVec2| common::cell_world_center(cell);
        let swarm = |id, cell| SwarmState {
            id,
            home: world(cell),
            minerals: 100,
            mineral_income_per_second: 0.0,
        };
        let swarms = [swarm(OWNER, IVec2::ZERO), swarm(enemy, IVec2::new(24, 24))];
        let remnant = BotState {
            id: 50,
            owner: enemy,
            kind: NanobotType::Worker,
            position: world(IVec2::new(12, 3)),
            health: 20,
            max_health: 100,
            charge: 1.0,
            max_charge: 1.0,
            cargo: 0,
            displacement: Vec2::ZERO,
            defender_duty: Default::default(),
        };
        let rock = RockFormation::Rectangle {
            half: Vec2::new(256.0, 650.0),
        };
        let rock_transform = Transform::from_translation(world(IVec2::new(2, 0)).extend(0.0));
        let terrain = [rock.obstacle(&rock_transform)];
        let homes = [
            (OWNER, world(IVec2::ZERO)),
            (enemy, world(IVec2::new(24, 24))),
        ];
        let map = SwarmReachabilityMap::new(&grid, terrain, homes).unwrap();
        // Warm the cell reachability memo so the first review's cost is geometry only.
        for owner in [OWNER, enemy] {
            for y in -32..32 {
                for x in -32..32 {
                    map.reachable_point(owner, IVec2::new(x, y), &grid);
                }
            }
        }
        let tuning = AutonomyTuning::default();
        let state = GameState {
            reachability: Some(&map),
            terrain: &terrain,
            grid: &grid,
            swarms: &swarms,
            bots: &[remnant],
            structures: &[],
            deposits: &[],
            charge: &DEFAULT_CHARGE,
            route_query: None,
            mine_query: None,
            supply: None,
            build_footprints: &[],
            tick: 0,
            finished: false,
        };
        let first = Controller::new(OWNER).decide(&tuning, &state);
        let second = Controller::new(OWNER).decide(&tuning, &state);
        assert!(first.reviewed && second.reviewed);
        assert!(second.work_units < first.work_units);
    }
}

#[test]
fn strategic_adapter_keeps_build_footprints_until_retired_or_inaccessible() {
    use top_down_2d_rts_prototype_nano_swarm::{
        ZONE_BLOCK_SIZE,
        nanobot::{
            NanobotType, OwnerSwarm, PlannedKind, PlannedStructure, ProductionFacility,
            StructureClearing,
        },
        navigation::Obstacle,
        resources::{SinkUpkeep, Stockpile, StockpileRole},
        strategic_runtime::ControllerTelemetry,
    };

    let owner = SwarmId::PLAYER;
    let mut app = common::minimal_app();
    app.insert_resource(IntentGrid::new(16, 16))
        .add_plugins(StrategicControllerPlugin);
    let home = common::cell_world_center(IVec2::ZERO);
    let swarm = common::spawn_swarm_at(&mut app, home);
    app.world_mut()
        .entity_mut(swarm)
        .insert(StrategicController::new(owner));
    install_empty_reachability(&mut app, [owner]);

    let mine = common::spawn_deposit(
        &mut app,
        common::DepositFixture {
            world_pos: common::cell_world_center(IVec2::new(-5, 4)),
            amount: 72_000,
            capacity: 72_000,
            radius: 64.0,
        },
    );
    app.world_mut().entity_mut(mine).insert(OwnerSwarm(swarm));

    let facility_cells = [IVec2::new(2, 2), IVec2::new(3, 2)];
    let site = Vec2::new(3.0 * ZONE_BLOCK_SIZE, 2.5 * ZONE_BLOCK_SIZE);
    let footprint = PlannedKind::ProductionFacility.footprint_transform(site.extend(0.0));
    let facility = app
        .world_mut()
        .spawn((
            OwnerSwarm(swarm),
            footprint,
            PlannedStructure::new(PlannedKind::ProductionFacility, facility_cells[0])
                .with_work_remaining(0),
            StructureClearing::awaiting_validation(site - Vec2::X * 150.0),
        ))
        .id();

    let sink_cell = IVec2::new(-3, 1);
    let sink = common::spawn_stockpile(&mut app, common::cell_world_center(sink_cell), 0, 200);
    app.world_mut().entity_mut(sink).insert((
        StockpileRole::Sink,
        OwnerSwarm(swarm),
        SinkUpkeep::default(),
    ));
    let foreign = SwarmId(8);
    for cell in facility_cells.into_iter().chain([sink_cell]) {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        grid.paint(cell, IntentKind::Build, owner);
        grid.paint(cell, IntentKind::Build, foreign);
    }

    let mut last_review = 0;
    let mut review = |app: &mut App| {
        for _ in 0..60 {
            app.update();
            let reviews =
                app.world().resource::<ControllerTelemetry>().profiles()[&owner.0].reviews;
            if reviews > last_review {
                last_review = reviews;
                return;
            }
        }
        panic!("the controller must review its Build plans after review {last_review}");
    };
    let has_build = |app: &App, cell, who| {
        app.world()
            .resource::<IntentGrid>()
            .cell(cell)
            .unwrap()
            .has_owned(IntentKind::Build, who)
    };

    let spare_build = |app: &App| {
        app.world()
            .resource::<IntentGrid>()
            .iter_active_cells()
            .filter_map(|(cell, intent)| {
                (intent.has_owned(IntentKind::Build, owner)
                    && !facility_cells.contains(&cell)
                    && cell != sink_cell)
                    .then_some(cell)
            })
            .collect::<std::collections::HashSet<_>>()
    };

    review(&mut app);
    for cell in facility_cells.into_iter().chain([sink_cell]) {
        assert!(
            has_build(&app, cell, owner),
            "keep supported Build at {cell}"
        );
    }

    let initial_spare = spare_build(&app);
    assert_eq!(initial_spare.iter().filter(|cell| cell.x >= -1).count(), 3);
    let outpost_room = initial_spare.iter().filter(|cell| cell.x < -1).count();
    assert_eq!(
        outpost_room, 3,
        "one base cell and two spare cells at the mine"
    );

    let mut busy = ProductionFacility::new();
    busy.current_target = Some(NanobotType::Defender);
    busy.input_amount = busy.input_capacity;
    app.world_mut()
        .entity_mut(facility)
        .remove::<(PlannedStructure, StructureClearing)>()
        .insert(busy);
    review(&mut app);
    assert_eq!(spare_build(&app), initial_spare);
    let mut idle = app
        .world_mut()
        .get_mut::<ProductionFacility>(facility)
        .unwrap();
    idle.current_target = None;
    idle.input_amount = 0;
    app.world_mut().get_mut::<SinkUpkeep>(sink).unwrap().pays = false;
    app.world_mut().get_mut::<Stockpile>(sink).unwrap().amount = 1;
    review(&mut app);
    assert_eq!(spare_build(&app), initial_spare);
    for cell in facility_cells.into_iter().chain([sink_cell]) {
        assert!(
            has_build(&app, cell, owner),
            "keep idle facility and drawing-down Sink at {cell}"
        );
    }

    app.world_mut().get_mut::<Stockpile>(sink).unwrap().amount = 0;
    let boundary = SwarmReachabilityMap::new(
        app.world().resource::<IntentGrid>(),
        [Obstacle::Rectangle {
            center: common::cell_world_center(facility_cells[1]),
            half: Vec2::splat(ZONE_BLOCK_SIZE / 2.0),
        }],
        [(owner, home)],
    )
    .unwrap();
    app.insert_resource(boundary);
    review(&mut app);
    for cell in facility_cells.into_iter().chain([sink_cell]) {
        assert!(
            !has_build(&app, cell, owner),
            "drop obsolete Build at {cell}"
        );
        assert!(
            has_build(&app, cell, foreign),
            "preserve foreign Build at {cell}"
        );
    }
}
