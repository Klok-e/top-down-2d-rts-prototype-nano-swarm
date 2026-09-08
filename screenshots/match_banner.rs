use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    nanobot::{MatchOutcome, SwarmId},
    ui::match_banner::{MatchBannerRoot, MatchBannerText},
};

use crate::harness::{TestContext, TestFlow};

pub fn victory_banner(ctx: &mut TestContext) -> TestFlow {
    banner(
        ctx,
        MatchOutcome::Winner(SwarmId::PLAYER),
        "VICTORY\nOpponent Swarm Eliminated",
        "match_victory",
    )
}

pub fn defeat_banner(ctx: &mut TestContext) -> TestFlow {
    banner(
        ctx,
        MatchOutcome::Winner(SwarmId(1)),
        "DEFEAT\nPlayer Swarm Eliminated",
        "match_defeat",
    )
}

pub fn draw_banner(ctx: &mut TestContext) -> TestFlow {
    banner(
        ctx,
        MatchOutcome::Draw,
        "DRAW\nBoth Swarms Eliminated",
        "match_draw",
    )
}

fn banner(ctx: &mut TestContext, outcome: MatchOutcome, text: &str, name: &str) -> TestFlow {
    if ctx.frame == 2 {
        ctx.world.insert_resource(outcome);
    }
    if ctx.frame < 5 {
        return TestFlow::Continue;
    }
    assert_eq!(*ctx.world.resource::<MatchOutcome>(), outcome);
    assert_eq!(
        *ctx.world
            .query_filtered::<&Visibility, With<MatchBannerRoot>>()
            .single(ctx.world)
            .unwrap(),
        Visibility::Visible,
    );
    assert_eq!(
        ctx.world
            .query_filtered::<&Text, With<MatchBannerText>>()
            .single(ctx.world)
            .unwrap()
            .0,
        text,
    );
    if ctx.frame == 5 {
        return TestFlow::Screenshot(name.to_string());
    }
    TestFlow::Exit
}
