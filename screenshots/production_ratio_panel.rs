//! Visual evidence for segmented production-ratio panel.

use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    nanobot::{Nanobot, NanobotType, ProductionRatio, SwarmId, SwarmMember},
    ui::production_ratio_panel::{
        ActualCompositionTick, HandleBoundary, ProductionRatioHandle, ProductionRatioValueText,
        HANDLE_WIDTH,
    },
};

use crate::harness::{TestContext, TestFlow};

pub fn production_ratio_panel(ctx: &mut TestContext) -> TestFlow {
    let world = &mut *ctx.world;
    if ctx.frame == 0 {
        {
            let mut ratio = world.resource_mut::<ProductionRatio>();
            ratio.set_weight(NanobotType::Worker, 40);
            ratio.set_weight(NanobotType::Hauler, 0);
            ratio.set_weight(NanobotType::Defender, 60);
        }

        for kind in [
            NanobotType::Worker,
            NanobotType::Worker,
            NanobotType::Hauler,
            NanobotType::Hauler,
            NanobotType::Hauler,
            NanobotType::Defender,
            NanobotType::Defender,
            NanobotType::Defender,
            NanobotType::Defender,
            NanobotType::Defender,
        ] {
            world.spawn((Nanobot {}, kind, SwarmMember::new(SwarmId::PLAYER)));
        }
        return TestFlow::Continue;
    }

    if ctx.frame < 10 {
        return TestFlow::Continue;
    }

    if ctx.frame == 10 {
        for (kind, expected) in [
            (NanobotType::Worker, "Worker 40%"),
            (NanobotType::Hauler, "Hauler 0%"),
            (NanobotType::Defender, "Defender 60%"),
        ] {
            assert!(world
                .query::<(&ProductionRatioValueText, &Text)>()
                .iter(world)
                .any(|(label, text)| label.0 == kind && text.0 == expected));
        }
        let offsets: Vec<_> = world
            .query::<(&ProductionRatioHandle, &Node)>()
            .iter(world)
            .map(|(handle, node)| (handle.0, node.margin.left))
            .collect();
        assert!(offsets.contains(&(HandleBoundary::WorkerEnd, Val::Px(-HANDLE_WIDTH))));
        assert!(offsets.contains(&(HandleBoundary::HaulerEnd, Val::Px(0.0))));
        for visibility in world
            .query_filtered::<&Visibility, With<ActualCompositionTick>>()
            .iter(world)
        {
            assert_eq!(*visibility, Visibility::Visible);
        }
        return TestFlow::Screenshot("production_ratio_panel".to_string());
    }

    TestFlow::Exit
}
