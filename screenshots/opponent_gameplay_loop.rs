//! Full-app visual evidence for the default skirmish and
//! visible Swarm Elimination result.

use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    intent::{IntentGrid, IntentKind},
    nanobot::{
        Charger, MatchOutcome, Nanobot, OpponentSwarm, OwnerSwarm, ProductionFacility, Swarm,
        SwarmId, SwarmMember,
    },
    resources::Stockpile,
    scenario::{OPPONENT_BUILD_FLANK_CELL, OPPONENT_CELL, OPPONENT_DEFEND_CELL},
    ui::match_banner::MatchBannerRoot,
};

use crate::harness::{TestContext, TestFlow};

pub fn opponent_gameplay_loop(ctx: &mut TestContext) -> TestFlow {
    if ctx.frame == 0 {
        ctx.world.resource_mut::<IntentGrid>().paint(
            OPPONENT_DEFEND_CELL,
            IntentKind::Defend,
            SwarmId::PLAYER,
        );
    }

    if ctx.frame < 180 {
        return TestFlow::Continue;
    }

    if ctx.frame == 180 {
        let defender_count = ctx
            .world
            .query_filtered::<&SwarmMember, With<Nanobot>>()
            .iter(ctx.world)
            .count();
        assert!(
            defender_count > 0,
            "the full skirmish must still contain nanobots at the assault capture"
        );
        return TestFlow::Screenshot("opponent_gameplay_assault".to_string());
    }

    if ctx.frame == 181 {
        let (opponent_entity, opponent_id) = ctx
            .world
            .query_filtered::<(Entity, &SwarmId), (With<Swarm>, With<OpponentSwarm>)>()
            .iter(ctx.world)
            .next()
            .map(|(entity, id)| (entity, *id))
            .expect("default scenario must contain an opponent swarm");
        let opponent_nanobots = ctx
            .world
            .query_filtered::<(Entity, &SwarmMember), With<Nanobot>>()
            .iter(ctx.world)
            .filter_map(|(entity, member)| (member.0 == opponent_id).then_some(entity))
            .collect::<Vec<_>>();
        let opponent_structures = ctx
            .world
            .query_filtered::<(Entity, &OwnerSwarm), Or<(With<ProductionFacility>, With<Stockpile>, With<Charger>)>>()
            .iter(ctx.world)
            .filter_map(|(entity, owner)| (owner.0 == opponent_entity).then_some(entity))
            .collect::<Vec<_>>();
        for entity in opponent_nanobots.into_iter().chain(opponent_structures) {
            ctx.world.despawn(entity);
        }
        {
            let mut grid = ctx.world.resource_mut::<IntentGrid>();
            grid.paint(OPPONENT_CELL, IntentKind::Defend, opponent_id);
            grid.erase(OPPONENT_CELL, IntentKind::Build, opponent_id);
            grid.erase(OPPONENT_BUILD_FLANK_CELL, IntentKind::Build, opponent_id);
        }
        return TestFlow::Screenshot("opponent_gameplay_victory".to_string());
    }

    assert_eq!(
        *ctx.world.resource::<MatchOutcome>(),
        MatchOutcome::Victory,
        "scripted terminal state must be a victory"
    );
    let banner_visible = ctx
        .world
        .query_filtered::<&Visibility, With<MatchBannerRoot>>()
        .iter(ctx.world)
        .any(|visibility| *visibility == Visibility::Visible);
    assert!(banner_visible, "victory banner must be visible");
    TestFlow::Exit
}
