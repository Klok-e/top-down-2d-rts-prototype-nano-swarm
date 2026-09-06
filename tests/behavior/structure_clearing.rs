use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::structure_overlay::CancelledPlanVisual;
use top_down_2d_rts_prototype_nano_swarm::{
    nanobot::{
        PlannedStructure, PlannedStructureProgress, StructureClearing, SwarmId, SwarmMember,
    },
    navigation::{BODY_RADIUS, Obstacle},
    resources::Stockpile,
};
#[path = "../common/mod.rs"]
mod common;

#[test]
fn completing_plan_releases_worker_and_evacuates_either_swarm_before_activation() {
    for owner in [SwarmId::PLAYER, SwarmId(1)] {
        let mut app = common::sim_app_with_planned();
        common::spawn_swarm_at(&mut app, Vec2::ZERO);
        let plan = common::spawn_planned_structure_at_cell(&mut app, IVec2::ZERO);
        let transform = top_down_2d_rts_prototype_nano_swarm::navigation::align_structure(
            *app.world().get::<Transform>(plan).unwrap(),
        );
        app.world_mut().entity_mut(plan).insert(transform);
        let center = transform.translation.truncate();
        let worker = common::spawn_worker_at(&mut app, center - Vec2::X * 72.0);
        let occupant = common::spawn_defender_at(&mut app, center);
        app.world_mut()
            .entity_mut(occupant)
            .insert(SwarmMember::new(owner));
        {
            let mut state = app.world_mut().get_mut::<PlannedStructure>(plan).unwrap();
            *state = state.with_work_remaining(1);
        }
        assert!(
            app.world_mut()
                .get_mut::<PlannedStructure>(plan)
                .unwrap()
                .try_claim(worker)
        );
        app.world_mut()
            .entity_mut(worker)
            .insert(PlannedStructureProgress {
                cell: IVec2::ZERO,
                target: plan,
            });
        app.update();
        app.update();
        assert!(app.world().get::<StructureClearing>(plan).is_some());
        assert!(app.world().get::<Stockpile>(plan).is_none());
        assert!(
            app.world()
                .get::<PlannedStructureProgress>(worker)
                .is_none()
        );
        assert_eq!(
            app.world()
                .get::<PlannedStructure>(plan)
                .unwrap()
                .active_worker(),
            None
        );
        let mut previous = app
            .world()
            .get::<Transform>(occupant)
            .unwrap()
            .translation
            .truncate();
        for _ in 0..160 {
            app.update();
            let position = app
                .world()
                .get::<Transform>(occupant)
                .unwrap()
                .translation
                .truncate();
            assert!(
                previous.distance(position) <= 5.001,
                "clearing must move, never teleport"
            );
            if app.world().get::<Stockpile>(plan).is_some() {
                assert!(Obstacle::structure(&transform).surface_distance(position) >= BODY_RADIUS);
                break;
            }
            previous = position;
        }
        assert!(
            app.world().get::<Stockpile>(plan).is_some(),
            "clear site should activate for occupant {owner:?}; occupant {:?}, worker {:?}, evacuation {:?}, movement {:?}",
            app.world().get::<Transform>(occupant),
            app.world().get::<Transform>(worker),
            app.world()
                .get::<top_down_2d_rts_prototype_nano_swarm::nanobot::ClearingEvacuation>(occupant),
            app.world()
                .get::<top_down_2d_rts_prototype_nano_swarm::nanobot::DirectMovementComponent>(
                    occupant
                )
        );
        assert!(app.world().get::<StructureClearing>(plan).is_none());
    }
}

#[test]
fn changed_access_cancels_clearing_and_releases_footprint_during_visual_fade() {
    use bevy::ecs::system::RunSystemOnce;
    use top_down_2d_rts_prototype_nano_swarm::nanobot::{
        PlannedKind,
        construction_access::{CancelledSites, ConstructionAccess},
    };
    let mut app = common::sim_app_with_planned();
    let plan = common::spawn_planned_structure_at_cell(&mut app, IVec2::ZERO);
    let transform = top_down_2d_rts_prototype_nano_swarm::navigation::align_structure(
        *app.world().get::<Transform>(plan).unwrap(),
    );
    app.world_mut().entity_mut(plan).insert((
        transform,
        StructureClearing::awaiting_validation(Vec2::new(180., 252.)),
    ));
    {
        let mut state = app.world_mut().get_mut::<PlannedStructure>(plan).unwrap();
        *state = state.with_work_remaining(0);
    }
    let occupant = common::spawn_defender_at(&mut app, Vec2::new(252., 252.));
    app.update();
    app.update();
    assert!(
        app.world()
            .get::<StructureClearing>(plan)
            .unwrap()
            .bars_entry()
    );
    common::spawn_structure_at(&mut app, Vec2::new(180., 252.));
    for _ in 0..80 {
        app.update();
        if app.world().get_entity(plan).is_err() {
            break;
        }
    }

    assert!(
        app.world().get_entity(plan).is_err(),
        "changed builder access cancels the clearing plan"
    );
    assert!(
        app.world()
            .get::<top_down_2d_rts_prototype_nano_swarm::nanobot::ClearingEvacuation>(occupant)
            .is_none()
    );
    use top_down_2d_rts_prototype_nano_swarm::{
        intent::IntentGrid,
        navigation::{Navigation, RouteGoal, RoutePriority, RouteStatus},
        physical_world::PhysicalWorld,
    };
    let freed_position = Vec2::new(252.0, 252.0);
    let geometry = app
        .world_mut()
        .run_system_once(|world: PhysicalWorld| world.snapshot())
        .unwrap();
    assert!(
        geometry.can_occupy(freed_position),
        "cancellation frees body placement in the committing tick"
    );
    let navigation = app.world().resource::<Navigation>();
    let request = navigation.request(
        Vec2::new(396.0, 252.0),
        RouteGoal::Point(freed_position),
        SwarmId::PLAYER,
        false,
        RoutePriority::Routine,
    );
    for _ in 0..100 {
        navigation.advance(app.world().resource::<IntentGrid>(), 32_768);
        if !matches!(navigation.poll(request), RouteStatus::Pending) {
            break;
        }
    }
    assert!(
        matches!(navigation.poll(request), RouteStatus::Found(_)),
        "the committing tick removes the entry barrier from cached navigation"
    );

    let effect = app
        .world_mut()
        .query_filtered::<Entity, With<CancelledPlanVisual>>()
        .single(app.world())
        .unwrap();
    assert!(app.world().get::<PlannedStructure>(effect).is_none());
    assert!(app.world().get::<Stockpile>(effect).is_none());
    let layout = app
        .world_mut()
        .run_system_once(|access: ConstructionAccess| access.snapshot())
        .unwrap();
    assert!(
        app.world().resource::<CancelledSites>().excludes(
            &layout,
            SwarmId::PLAYER,
            PlannedKind::SourceStockpile,
            &transform
        ),
        "cancellation removal cannot enable immediate retry"
    );
    let scale = app.world().get::<Transform>(effect).unwrap().scale;
    for _ in 0..15 {
        app.update();
    }
    let faded = app.world().get::<Transform>(effect).unwrap().scale;
    assert!(faded.x > 0. && faded.x < scale.x);
    assert!(app.world().get::<Sprite>(effect).unwrap().color.alpha() < 1.);
    for _ in 0..20 {
        app.update();
    }
    assert!(app.world().get_entity(effect).is_err());
}

#[test]
fn clearing_waits_through_congestion_without_admitting_new_entrants() {
    use top_down_2d_rts_prototype_nano_swarm::nanobot::{DirectMovementComponent, Nanobot};
    let mut app = common::sim_app_with_planned();
    let plan = common::spawn_planned_structure_at_cell(&mut app, IVec2::ZERO);
    let transform = top_down_2d_rts_prototype_nano_swarm::navigation::align_structure(
        *app.world().get::<Transform>(plan).unwrap(),
    );
    app.world_mut().entity_mut(plan).insert((
        transform,
        StructureClearing::awaiting_validation(Vec2::new(180., 252.)),
    ));
    {
        let mut state = app.world_mut().get_mut::<PlannedStructure>(plan).unwrap();
        *state = state.with_work_remaining(0);
    }
    let trapped = common::spawn_defender_at(&mut app, Vec2::new(252., 252.));
    // Hostile stationary bodies fill adjacent cells and cannot be crossed during recovery.
    for y in -1..=1 {
        for x in -1..=1 {
            if x != 0 || y != 0 {
                app.world_mut().spawn((
                    Nanobot {},
                    top_down_2d_rts_prototype_nano_swarm::nanobot::SwarmMember(SwarmId(42)),
                    top_down_2d_rts_prototype_nano_swarm::nanobot::NanobotType::Worker,
                    top_down_2d_rts_prototype_nano_swarm::nanobot::VelocityComponent::default(),
                    top_down_2d_rts_prototype_nano_swarm::nanobot::Commitment::Working,
                    Transform::from_xyz(252. + x as f32 * 72., 252. + y as f32 * 72., 0.),
                ));
            }
        }
    }
    let entrant = common::spawn_worker_at(&mut app, Vec2::new(252., 500.));
    app.world_mut()
        .entity_mut(entrant)
        .insert(DirectMovementComponent {
            xy: Vec2::new(252., 252.),
            stop_radius: 0.,
            interaction: None,
            speed: None,
        });
    for _ in 0..300 {
        app.update();
        assert!(app.world().get::<StructureClearing>(plan).is_some());
        assert!(app.world().get::<Stockpile>(plan).is_none());
        assert!(
            Obstacle::structure(&transform).admits_body(
                app.world()
                    .get::<Transform>(entrant)
                    .unwrap()
                    .translation
                    .truncate()
            )
        );
    }
    assert!(
        !Obstacle::structure(&transform).admits_body(
            app.world()
                .get::<Transform>(trapped)
                .unwrap()
                .translation
                .truncate()
        )
    );
}

#[test]
fn simultaneous_cancellations_preserve_both_site_exclusions() {
    use bevy::ecs::system::RunSystemOnce;
    use top_down_2d_rts_prototype_nano_swarm::nanobot::{
        PlannedKind,
        construction_access::{CancelledSites, ConstructionAccess},
    };
    let mut app = common::sim_app_with_planned();
    let first = common::spawn_planned_structure_at_cell(&mut app, IVec2::ZERO);
    let second = common::spawn_planned_structure_at_cell(&mut app, IVec2::X);
    let transforms = [first, second].map(|entity| *app.world().get::<Transform>(entity).unwrap());
    let layout = app
        .world_mut()
        .run_system_once(|access: ConstructionAccess| access.snapshot())
        .unwrap();
    for (entity, transform) in [first, second].into_iter().zip(transforms) {
        app.world_mut().resource_mut::<CancelledSites>().record(
            &layout,
            entity,
            SwarmId::PLAYER,
            PlannedKind::SourceStockpile,
            &transform,
        );
        app.world_mut().despawn(entity);
    }
    let remaining = app
        .world_mut()
        .run_system_once(|access: ConstructionAccess| access.snapshot())
        .unwrap();
    for transform in transforms {
        assert!(
            app.world().resource::<CancelledSites>().excludes(
                &remaining,
                SwarmId::PLAYER,
                PlannedKind::SourceStockpile,
                &transform
            ),
            "another cancellation in the same batch cannot unblock this site"
        );
    }
}

#[test]
fn routes_detour_around_a_validated_clearing_footprint() {
    use top_down_2d_rts_prototype_nano_swarm::nanobot::{DirectMovementComponent, PlannedKind};
    let mut app = common::sim_app_with_movement();
    let transform = Transform::from_xyz(360., 324., 0.).with_scale(Vec3::new(1.125, 2.25, 1.));
    app.world_mut().spawn((
        PlannedStructure::new(PlannedKind::Charger, IVec2::ZERO),
        StructureClearing::validated(Vec2::new(200., 324.), 0),
        transform,
    ));
    let bot = common::spawn_worker_at(&mut app, Vec2::new(180., 324.));
    app.world_mut()
        .entity_mut(bot)
        .insert(DirectMovementComponent {
            xy: Vec2::new(540., 324.),
            stop_radius: 0.,
            interaction: None,
            speed: None,
        });
    let mut previous = Vec2::new(180., 324.);
    let mut detoured = false;
    for _ in 0..400 {
        app.update();
        let position = app
            .world()
            .get::<Transform>(bot)
            .unwrap()
            .translation
            .truncate();
        assert!(Obstacle::structure(&transform).segment_clear(previous, position));
        detoured |= (position.y - 324.).abs() > 100.;
        previous = position;
    }
    assert!(
        detoured,
        "routing must see the clearing site, not stop against its entry guard"
    );
    assert!(previous.distance(Vec2::new(540., 324.)) < 3.);
}

#[test]
fn unfinished_access_validation_preserves_the_plan_until_budget_opens() {
    use top_down_2d_rts_prototype_nano_swarm::navigation_runtime::NavigationBudget;
    let mut app = common::sim_app_with_planned();
    app.world_mut().resource_mut::<NavigationBudget>().0 = 0;
    let plan = common::spawn_planned_structure_at_cell(&mut app, IVec2::ZERO);
    let transform = top_down_2d_rts_prototype_nano_swarm::navigation::align_structure(
        *app.world().get::<Transform>(plan).unwrap(),
    );
    app.world_mut().entity_mut(plan).insert((
        transform,
        StructureClearing::awaiting_validation(Vec2::new(180., 252.)),
    ));
    {
        let mut state = app.world_mut().get_mut::<PlannedStructure>(plan).unwrap();
        *state = state.with_work_remaining(0);
    }
    for _ in 0..20 {
        app.update();
        assert!(app.world().get::<PlannedStructure>(plan).is_some());
        assert!(app.world().get::<Stockpile>(plan).is_none());
    }
    app.world_mut().resource_mut::<NavigationBudget>().0 = 32768;
    for _ in 0..100 {
        app.update();
        if app.world().get::<Stockpile>(plan).is_some() {
            break;
        }
    }
    assert!(
        app.world().get::<Stockpile>(plan).is_some(),
        "queued access validation must resume activation"
    );
}

#[test]
fn evacuated_bot_resumes_work_while_construction_validation_is_pending() {
    use top_down_2d_rts_prototype_nano_swarm::{
        nanobot::{Cargo, ClearingEvacuation, DirectMovementComponent},
        navigation_runtime::NavigationBudget,
        resources::ResourceKind,
    };
    let mut app = common::sim_app_with_planned();
    common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let plan = common::spawn_planned_structure_at_cell(&mut app, IVec2::ZERO);
    let transform = top_down_2d_rts_prototype_nano_swarm::navigation::align_structure(
        *app.world().get::<Transform>(plan).unwrap(),
    );
    let position = transform.translation.truncate() + Vec2::Y * 100.0;
    app.world_mut()
        .entity_mut(plan)
        .insert((transform, StructureClearing::awaiting_validation(position)));
    let bot = common::spawn_worker_at(&mut app, position);
    app.world_mut().entity_mut(bot).insert((
        Cargo {
            kind: ResourceKind::Minerals,
            amount: 4,
        },
        ClearingEvacuation {
            structure: plan,
            goal: position,
        },
        DirectMovementComponent {
            xy: position + Vec2::X * 100.0,
            stop_radius: 0.0,
            interaction: None,
            speed: None,
        },
    ));
    app.insert_resource(NavigationBudget(0));
    app.update();
    assert!(
        app.world().get::<StructureClearing>(plan).is_some(),
        "construction must still await its safety check"
    );
    assert!(
        app.world().get::<ClearingEvacuation>(bot).is_none(),
        "a body already outside the footprint must not remain assigned to evacuation"
    );
    app.update();
    assert!(
        app.world().get::<Transform>(bot).unwrap().translation.x > position.x,
        "pending construction validation must not override the next movement order"
    );
    assert_eq!(app.world().get::<Cargo>(bot).unwrap().amount, 4);
}
