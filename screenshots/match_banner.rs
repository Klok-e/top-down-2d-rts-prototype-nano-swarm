use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    intent::{IntentGrid, IntentKind},
    nanobot::{
        Charge, Commitment, DefenderAttackCooldown, Health, MatchOutcome, Nanobot, NanobotType,
        OpponentSwarm, SwarmId, SwarmMember, world_to_cell,
    },
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

pub fn hud_attack_and_wipeout(ctx: &mut TestContext) -> TestFlow {
    match ctx.frame {
        0 => {
            let position = ctx
                .world
                .query_filtered::<(&SwarmMember, &NanobotType, &Transform), With<Nanobot>>()
                .iter(ctx.world)
                .find_map(|(member, kind, transform)| {
                    (member.0 == SwarmId::PLAYER && *kind == NanobotType::Worker)
                        .then_some(transform.translation)
                })
                .expect("Standard starts with a player Worker");
            let opponent = *ctx
                .world
                .query_filtered::<&SwarmId, With<OpponentSwarm>>()
                .single(ctx.world)
                .expect("Standard starts with an opponent swarm");
            ctx.world.resource_mut::<IntentGrid>().paint(
                world_to_cell(position.truncate()),
                IntentKind::Defend,
                opponent,
            );
            ctx.world.spawn((
                Nanobot {},
                NanobotType::Defender,
                Commitment::Idle,
                Health::default(),
                Charge::default(),
                SwarmMember::new(opponent),
                Transform::from_translation(position + Vec3::X * 65.0),
                DefenderAttackCooldown::new(0),
            ));
        }
        4 => {
            assert!(visible_combat_alert(ctx.world).is_some_and(|fill| fill.0 != Color::NONE));
            assert!(!hud_shows_text(ctx.world, "Swarm under attack"));
            assert!(!hud_shows_text(ctx.world, "Hostiles in your territory"));
            return TestFlow::Screenshot("hud_under_attack".into());
        }
        5 => {
            let player_bots = ctx
                .world
                .query_filtered::<(Entity, &SwarmMember), With<Nanobot>>()
                .iter(ctx.world)
                .filter_map(|(entity, member)| (member.0 == SwarmId::PLAYER).then_some(entity))
                .collect::<Vec<_>>();
            for entity in player_bots {
                ctx.world.despawn(entity);
            }
        }
        6 => {
            assert_eq!(
                *ctx.world.resource::<MatchOutcome>(),
                MatchOutcome::InProgress
            );
            assert!(visible_combat_alert(ctx.world).is_none());
            assert!(hud_shows_text(ctx.world, "No nanobots remaining"));
            assert!(hud_shows_text(
                ctx.world,
                "Production: no Haulers to deliver"
            ));
            return TestFlow::Screenshot("hud_wipeout".into());
        }
        7 => return TestFlow::Exit,
        _ => {}
    }
    TestFlow::Continue
}

fn visible_combat_alert(world: &mut World) -> Option<BackgroundColor> {
    world
        .query::<(&Name, &Node, &BackgroundColor)>()
        .iter(world)
        .find_map(|(name, node, fill)| {
            (name.as_str() == "Combat alert" && node.display != Display::None).then_some(*fill)
        })
}

fn hud_shows_text(world: &mut World, message: &str) -> bool {
    world
        .query::<(&Text, &Node)>()
        .iter(world)
        .any(|(text, node)| node.display != Display::None && text.0.contains(message))
}
