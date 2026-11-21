use std::{f32::consts::TAU, time::Duration};

use bevy::{
    asset::RenderAssetUsages,
    prelude::*,
    render::{
        Extract, ExtractSchedule, RenderApp,
        camera::extract_cameras,
        render_resource::{Extent3d, TextureDimension, TextureFormat},
        sync_world::TemporaryRenderEntity,
    },
    sprite_render::{ExtractedSprite, ExtractedSpriteKind, ExtractedSprites, SpriteSystems},
};

use crate::nanobot::{
    Charge, Health, NanobotType, PresentedNanobot, ResolvedCombatFact, SwarmMember,
    swarm_presentation_color,
};
use crate::tuning::Tuning;

use super::{
    CombatView, CombatZoomTier,
    render_anchor::{CombatCameras, queue_camera_sprite_anchor},
};

const HEALTH_DURATION: Duration = Duration::from_secs(2);
pub(super) const ARC_RADIUS: f32 = 20.0;
pub(super) const ARC_THICKNESS: f32 = 2.5;
const CORE_RADIUS: f32 = 5.0;
const TILE_SIZE: usize = 44;
const ATLAS_COLUMNS: usize = 16;
const ATLAS_ROWS: usize = 8;
const FULL_ARC_TILE: usize = ATLAS_COLUMNS * ATLAS_ROWS - 1;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CombatIndicatorKind {
    HealthArc { fraction: f32 },
    ChargeCore,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CombatIndicator {
    pub owner: Entity,
    pub kind: CombatIndicatorKind,
    pub position: Vec2,
    pub color: Color,
    pub(super) z: f32,
}

#[derive(Resource, Default)]
pub struct ActiveCombatIndicators(Vec<CombatIndicator>);

impl ActiveCombatIndicators {
    pub fn iter(&self) -> impl ExactSizeIterator<Item = &CombatIndicator> {
        self.0.iter()
    }
}

#[derive(Component)]
struct DamageIndicator {
    health: Health,
    last_damage: Option<Duration>,
}

#[derive(Resource)]
struct IndicatorAtlas(Handle<Image>);

pub(super) struct CombatIndicatorsPlugin;

impl Plugin for CombatIndicatorsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ActiveCombatIndicators>()
            .add_observer(track_nanobot_health)
            .add_systems(PostUpdate, (record_damage, collect_indicators).chain());
        if app.get_sub_app(RenderApp).is_some() {
            let atlas = app
                .world_mut()
                .resource_mut::<Assets<Image>>()
                .add(indicator_atlas());
            app.insert_resource(IndicatorAtlas(atlas));
            app.sub_app_mut(RenderApp).add_systems(
                ExtractSchedule,
                extract_indicators
                    .after(SpriteSystems::ExtractSprites)
                    .after(extract_cameras),
            );
        }
    }
}

pub(super) fn reset_session(world: &mut World) {
    if let Some(mut indicators) = world.get_resource_mut::<ActiveCombatIndicators>() {
        indicators.0.clear();
    }
}

fn track_nanobot_health(
    added: On<Add, (Health, PresentedNanobot)>,
    mut commands: Commands,
    units: Query<&Health, (With<PresentedNanobot>, Without<DamageIndicator>)>,
) {
    if let Ok(&health) = units.get(added.entity) {
        commands.entity(added.entity).insert(DamageIndicator {
            health,
            last_damage: None,
        });
    }
}

fn record_damage(
    time: Option<Res<Time>>,
    health: Query<(&Health, Entity), Changed<Health>>,
    mut units: Query<&mut DamageIndicator>,
    mut facts: MessageReader<ResolvedCombatFact>,
) {
    let now = time.as_ref().map_or(Duration::ZERO, |time| time.elapsed());
    for (health, entity) in &health {
        let Ok(mut indicator) = units.get_mut(entity) else {
            continue;
        };
        if indicator.health.max == health.max && health.current < indicator.health.current {
            indicator.last_damage = Some(now);
        }
        indicator.health = *health;
    }

    for &fact in facts.read() {
        if let ResolvedCombatFact::Hit(hit) = fact
            && hit.damage > 0
            && let Ok(mut indicator) = units.get_mut(hit.target.entity)
        {
            indicator.last_damage = Some(now);
        }
    }
}

#[allow(clippy::type_complexity)]
fn collect_indicators(
    time: Option<Res<Time>>,
    tuning: Res<Tuning>,
    view: Res<CombatView>,
    units: Query<(
        Entity,
        &PresentedNanobot,
        &NanobotType,
        &SwarmMember,
        Option<&Charge>,
        Option<&DamageIndicator>,
    )>,
    mut indicators: ResMut<ActiveCombatIndicators>,
) {
    indicators.0.clear();
    if view.tier == CombatZoomTier::Far {
        return;
    }
    let now = time.as_ref().map_or(Duration::ZERO, |time| time.elapsed());
    let pulse_phase = now.as_secs_f64() * 1.5 % 1.0;
    let pulse_strength = 0.5 + 0.2 * (TAU * pulse_phase as f32).sin();
    let margin =
        view.effect_size(Vec2::splat(ARC_RADIUS * 2.0)).x * 0.5 + view.line_width(ARC_THICKNESS);

    for (owner, presented, kind, member, charge, damage) in &units {
        let translation = presented.sprite_translation();
        let position = translation.truncate();
        let z = translation.z + 0.02;
        if !view.contains_with_margin(position, margin) {
            continue;
        }

        if let Some(damage) = damage
            && let Some(last) = damage.last_damage
        {
            let age = now.saturating_sub(last);
            let health = damage.health;
            if age < HEALTH_DURATION && health.current > 0 {
                let fraction = (health.current as f32 / health.max.max(1) as f32).min(1.0);
                let alpha = 1.0 - age.as_secs_f32() / HEALTH_DURATION.as_secs_f32();
                let color = if fraction > 0.6 {
                    Color::srgb(0.28, 0.91, 0.39)
                } else if fraction > 0.3 {
                    Color::srgb(1.0, 0.72, 0.2)
                } else {
                    Color::srgb(1.0, 0.25, 0.25)
                };
                indicators.0.push(CombatIndicator {
                    owner,
                    kind: CombatIndicatorKind::HealthArc { fraction },
                    position,
                    color: color.with_alpha(alpha),
                    z,
                });
            }
        }

        if *kind == NanobotType::Defender
            && let Some(charge) = charge
        {
            let strength = if charge.current < tuning.charge.weakened_threshold {
                pulse_strength
            } else {
                (charge.current / charge.max.max(f32::EPSILON)).clamp(0.0, 1.0)
            };
            let base = swarm_presentation_color(member.0).to_linear();
            indicators.0.push(CombatIndicator {
                owner,
                kind: CombatIndicatorKind::ChargeCore,
                position,
                color: Color::linear_rgb(
                    base.red * strength,
                    base.green * strength,
                    base.blue * strength,
                ),
                z,
            });
        }
    }
}

pub(super) fn draw_scaled_health_arc(view: &CombatView) -> bool {
    view.line_width(ARC_THICKNESS) != ARC_THICKNESS
        || view.effect_size(Vec2::splat(2.0 * ARC_RADIUS)).x > 2.0 * ARC_RADIUS
}

fn tile_rect(tile: usize) -> Rect {
    let min = Vec2::new(
        (tile % ATLAS_COLUMNS * TILE_SIZE) as f32,
        (tile / ATLAS_COLUMNS * TILE_SIZE) as f32,
    );
    Rect::from_corners(min, min + Vec2::splat(TILE_SIZE as f32))
}

// Draw each health arc with a single quad and share its texture with the charge cores.
fn indicator_atlas() -> Image {
    let width = TILE_SIZE * ATLAS_COLUMNS;
    let height = TILE_SIZE * ATLAS_ROWS;
    let mut data = vec![255; width * height * 4];
    for y in 0..TILE_SIZE {
        for x in 0..TILE_SIZE {
            let center = (TILE_SIZE as f32 - 1.0) * 0.5;
            let point = Vec2::new(x as f32 - center, y as f32 - center);
            let distance = point.length();
            let turn = point.x.atan2(-point.y).rem_euclid(TAU) / TAU;
            let ring = (ARC_THICKNESS * 0.5 + 0.5 - (distance - ARC_RADIUS).abs()).clamp(0.0, 1.0);
            for tile in 0..=FULL_ARC_TILE {
                let coverage = if tile == 0 {
                    (CORE_RADIUS + 0.5 - distance).clamp(0.0, 1.0)
                } else if turn <= tile as f32 / FULL_ARC_TILE as f32 {
                    ring
                } else {
                    0.0
                };
                let column = tile % ATLAS_COLUMNS * TILE_SIZE + x;
                let row = tile / ATLAS_COLUMNS * TILE_SIZE + y;
                data[(row * width + column) * 4 + 3] = (coverage * 255.0).round() as u8;
            }
        }
    }
    Image::new(
        Extent3d {
            width: width as u32,
            height: height as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    )
}

fn extract_indicators(
    mut commands: Commands,
    indicators: Extract<Res<ActiveCombatIndicators>>,
    atlas: Extract<Res<IndicatorAtlas>>,
    view: Extract<Res<CombatView>>,
    cameras: CombatCameras,
    mut extracted: ResMut<ExtractedSprites>,
) {
    if indicators.0.is_empty() {
        return;
    }
    let scaled_arcs = draw_scaled_health_arc(&view);
    let core_scale = view.effect_size(Vec2::splat(2.0 * CORE_RADIUS)).x / (2.0 * CORE_RADIUS);
    for (camera, render) in &cameras {
        queue_camera_sprite_anchor(&mut commands, camera, render);
        for indicator in indicators.iter() {
            let (tile, size) = match indicator.kind {
                CombatIndicatorKind::HealthArc { fraction } => {
                    if scaled_arcs {
                        continue;
                    }
                    (
                        ((fraction * FULL_ARC_TILE as f32).round() as usize).max(1),
                        TILE_SIZE as f32,
                    )
                }
                CombatIndicatorKind::ChargeCore => (0, TILE_SIZE as f32 * core_scale),
            };
            extracted.sprites.push(ExtractedSprite {
                main_entity: camera,
                render_entity: commands.spawn(TemporaryRenderEntity).id(),
                transform: GlobalTransform::from_translation(
                    indicator.position.extend(indicator.z),
                ),
                color: indicator.color.to_linear(),
                image_handle_id: atlas.0.id(),
                flip_x: false,
                flip_y: false,
                kind: ExtractedSpriteKind::Single {
                    anchor: Vec2::ZERO,
                    rect: Some(tile_rect(tile)),
                    scaling_mode: None,
                    custom_size: Some(Vec2::splat(size)),
                },
            });
        }
    }
}
