//! Visual evidence for segmented production-priority panel.

use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    nanobot::{NanobotType, ProductionPriority},
    ui::production_priority_panel::{
        HANDLE_WIDTH, HandleBoundary, ProductionPriorityHandle, ProductionPriorityValueText,
    },
};

use crate::harness::{TestContext, TestFlow};

pub fn production_priority_panel(ctx: &mut TestContext) -> TestFlow {
    let world = &mut *ctx.world;
    if ctx.frame == 0 {
        {
            let mut priority = world.resource_mut::<ProductionPriority>();
            priority.set_weight(NanobotType::Worker, 40);
            priority.set_weight(NanobotType::Hauler, 0);
            priority.set_weight(NanobotType::Defender, 60);
        }
        return TestFlow::Continue;
    }

    if ctx.frame < 10 {
        return TestFlow::Continue;
    }

    if ctx.frame == 10 {
        assert!(
            world.query::<&Text>().iter(world).any(|text| {
                text.0.contains("Demand: W") && !text.0.contains("Production: idle")
            })
        );
        assert!(
            world
                .query::<&Text>()
                .iter(world)
                .any(|text| text.0 == "Production Priority")
        );
        for (kind, expected) in [
            (NanobotType::Worker, "Worker 40%"),
            (NanobotType::Hauler, "Hauler 0%"),
            (NanobotType::Defender, "Defender 60%"),
        ] {
            assert!(
                world
                    .query::<(&ProductionPriorityValueText, &Text)>()
                    .iter(world)
                    .any(|(label, text)| label.0 == kind && text.0 == expected)
            );
        }
        let offsets: Vec<_> = world
            .query::<(&ProductionPriorityHandle, &Node)>()
            .iter(world)
            .map(|(handle, node)| (handle.0, node.margin.left))
            .collect();
        assert!(offsets.contains(&(HandleBoundary::WorkerEnd, Val::Px(-HANDLE_WIDTH))));
        assert!(offsets.contains(&(HandleBoundary::HaulerEnd, Val::Px(0.0))));
        return TestFlow::Screenshot("production_priority_panel".to_string());
    }

    TestFlow::Exit
}
