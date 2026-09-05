//! Physical logistics work at scaled exterior interaction positions.
use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    nanobot::{Cargo, HaulerAssignment, LogisticsReservation, OwnerSwarm, SwarmId, SwarmMember},
    resources::{ResourceKind, ResourceLedger, Stockpile},
};

#[path = "../common/mod.rs"]
mod common;

#[test]
fn scaled_pickup_pauses_when_displaced_and_keeps_material_custody() {
    for swarm in [SwarmId::PLAYER, SwarmId(1)] {
        let mut app = common::sim_app_with_gather_haul();
        let owner = common::spawn_swarm_at(&mut app, Vec2::ZERO);
        app.world_mut().entity_mut(owner).insert(swarm);
        let source = common::spawn_stockpile(&mut app, Vec2::ZERO, 20, 100);
        app.world_mut().entity_mut(source).insert((
            OwnerSwarm(owner),
            Transform::from_scale(Vec3::new(2.0, 1.0, 1.0)),
        ));
        let sink = common::spawn_sink_stockpile(&mut app, Vec2::new(400.0, 0.0), 0, 100);
        app.world_mut().entity_mut(sink).insert(OwnerSwarm(owner));
        let bot = common::spawn_hauler_at(&mut app, Vec2::new(100.0, 0.0));
        app.world_mut().entity_mut(bot).insert((
            SwarmMember::new(swarm),
            HaulerAssignment { source, sink },
            LogisticsReservation::new(source, sink, ResourceKind::Minerals, 20),
        ));
        app.world_mut()
            .resource_mut::<ResourceLedger>()
            .add_for(swarm, ResourceKind::Minerals, 20);
        app.update();
        assert_eq!(
            app.world().get::<Cargo>(bot).map(|cargo| cargo.amount),
            Some(4)
        );
        assert!(app.world().get::<Transform>(bot).unwrap().translation.x >= 98.0);
        app.world_mut()
            .get_mut::<Transform>(bot)
            .unwrap()
            .translation
            .x = 200.0;
        app.update();
        assert_eq!(
            app.world().get::<Cargo>(bot).unwrap().amount,
            4,
            "displacement suspends pickup"
        );
        assert_eq!(app.world().get::<Stockpile>(source).unwrap().amount, 16);
        let reservation = app.world().get::<LogisticsReservation>(bot).unwrap();
        assert_eq!(reservation.source_remaining, 16);
        assert_eq!(reservation.destination_remaining, 20);
        let mut resumed = false;
        for _ in 0..100 {
            app.update();
            if app
                .world()
                .get::<Cargo>(bot)
                .is_some_and(|cargo| cargo.amount > 4)
            {
                let pos = app.world().get::<Transform>(bot).unwrap().translation;
                assert!(
                    pos.x >= 98.0 && pos.x <= 102.01,
                    "pickup body clearance: {pos:?}"
                );
                resumed = true;
                break;
            }
        }
        assert!(resumed, "displaced hauler returns and resumes pickup");
        assert_eq!(
            app.world()
                .resource::<ResourceLedger>()
                .total_for(swarm, ResourceKind::Minerals),
            20
        );
    }
}

#[test]
fn both_swarms_deliver_to_scaled_exteriors_and_resume_after_displacement() {
    use top_down_2d_rts_prototype_nano_swarm::nanobot::{
        Charger, ProductionFacility, SwarmId, SwarmMember,
    };
    #[derive(Clone, Copy, Debug)]
    enum DestinationKind {
        Stockpile,
        Facility,
        Charger,
    }
    for swarm in [SwarmId::PLAYER, SwarmId(1)] {
        for destination_kind in [
            DestinationKind::Stockpile,
            DestinationKind::Facility,
            DestinationKind::Charger,
        ] {
            let mut app = common::sim_app_with_gather_haul();
            let owner = common::spawn_swarm_at(&mut app, Vec2::ZERO);
            app.world_mut().entity_mut(owner).insert(swarm);
            let source = match destination_kind {
                DestinationKind::Stockpile => common::spawn_stockpile(&mut app, Vec2::ZERO, 0, 100),
                DestinationKind::Facility | DestinationKind::Charger => {
                    common::spawn_sink_stockpile(&mut app, Vec2::ZERO, 0, 100)
                }
            };
            app.world_mut().entity_mut(source).insert(OwnerSwarm(owner));
            let target_transform =
                Transform::from_xyz(400.0, 0.0, 0.0).with_scale(Vec3::new(2.0, 1.0, 1.0));
            let sink = match destination_kind {
                DestinationKind::Stockpile => {
                    let sink =
                        common::spawn_sink_stockpile(&mut app, Vec2::new(400.0, 0.0), 0, 100);
                    app.world_mut()
                        .entity_mut(sink)
                        .insert((OwnerSwarm(owner), target_transform));
                    sink
                }
                DestinationKind::Facility => app
                    .world_mut()
                    .spawn((
                        ProductionFacility::new(),
                        OwnerSwarm(owner),
                        target_transform,
                    ))
                    .id(),
                DestinationKind::Charger => app
                    .world_mut()
                    .spawn((
                        Charger::new(IVec2::ZERO),
                        OwnerSwarm(owner),
                        target_transform,
                    ))
                    .id(),
            };
            let bot = common::spawn_hauler_at(&mut app, Vec2::new(300.0, 0.0));
            app.world_mut().entity_mut(bot).insert((
                SwarmMember::new(swarm),
                Cargo {
                    kind: ResourceKind::Minerals,
                    amount: 12,
                },
                HaulerAssignment { source, sink },
                LogisticsReservation::new(source, sink, ResourceKind::Minerals, 12),
            ));
            let delivered = |app: &App| match destination_kind {
                DestinationKind::Stockpile => app.world().get::<Stockpile>(sink).unwrap().amount,
                DestinationKind::Facility => {
                    app.world()
                        .get::<ProductionFacility>(sink)
                        .unwrap()
                        .input_amount
                }
                DestinationKind::Charger => app.world().get::<Charger>(sink).unwrap().amount,
            };
            app.update();
            assert_eq!(
                delivered(&app),
                4,
                "{swarm:?} {destination_kind:?} exterior unload"
            );
            app.world_mut()
                .get_mut::<Transform>(bot)
                .unwrap()
                .translation
                .x = 200.0;
            app.update();
            assert_eq!(delivered(&app), 4, "displaced delivery pauses");
            assert_eq!(app.world().get::<Cargo>(bot).unwrap().amount, 8);
            let mut previous = 4;
            for _ in 0..100 {
                app.update();
                let current = delivered(&app);
                if current > previous {
                    let x = app.world().get::<Transform>(bot).unwrap().translation.x;
                    assert!(
                        (297.99..=304.001).contains(&x),
                        "body outside scaled footprint during unload: {x}"
                    );
                    previous = current;
                }
                if current == 12 {
                    break;
                }
            }
            assert_eq!(
                delivered(&app),
                12,
                "{swarm:?} {destination_kind:?} resumes delivery"
            );
            assert!(app.world().get::<Cargo>(bot).is_none());
            assert!(app.world().get::<LogisticsReservation>(bot).is_none());
        }
    }
}
