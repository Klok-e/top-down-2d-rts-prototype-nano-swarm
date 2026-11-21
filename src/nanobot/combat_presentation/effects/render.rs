use std::f32::consts::{FRAC_PI_2, PI, TAU};

use bevy::{
    asset::RenderAssetUsages,
    ecs::system::SystemParam,
    image::ImageSampler,
    math::vec2,
    prelude::*,
    render::{
        Extract, ExtractSchedule, RenderApp,
        camera::extract_cameras,
        render_resource::{Extent3d, TextureDimension, TextureFormat},
        sync_world::TemporaryRenderEntity,
    },
    sprite_render::{ExtractedSprite, ExtractedSpriteKind, ExtractedSprites, SpriteSystems},
};

use crate::{
    GAMEPLAY_SPRITE_Z, nanobot::PresentedNanobot, navigation::STRUCTURE_SPRITE_SIZE, tuning::Tuning,
};

use super::{
    ActiveCombatEffects, CombatParticleKind, MUZZLE_LIFETIME, POP_LIFETIME,
    STRUCTURE_FLASH_LIFETIME,
};
use crate::nanobot::combat_presentation::{
    ActiveCombatBolts, ActiveCombatIndicators, ActiveSkirmishRings, CombatIndicatorKind,
    CombatView, CombatZoomTier, combat_hdr,
    indicators::{ARC_RADIUS, ARC_THICKNESS, draw_scaled_health_arc},
    render_anchor::{CombatCameras, queue_camera_sprite_anchor},
};

const DECAL_Z: f32 = GAMEPLAY_SPRITE_Z - 0.02;
const EFFECT_Z: f32 = GAMEPLAY_SPRITE_Z + 1.25;
const TILE_SIDE: u32 = 64;
const TILE_COLUMNS: u32 = 8;

#[repr(u32)]
#[derive(Clone, Copy)]
enum Art {
    Disc,
    Flash,
    Tail,
    Streak,
    Fragment,
    Smoke,
    Scorch,
    Rubble,
    RingSegment,
    SoftRingSegment,
}

#[derive(Resource)]
struct CombatAtlas(Handle<Image>);

pub(super) fn configure(app: &mut App) {
    if app.get_sub_app(RenderApp).is_none() {
        return;
    }
    let atlas = app
        .world_mut()
        .resource_mut::<Assets<Image>>()
        .add(make_atlas());
    app.insert_resource(CombatAtlas(atlas));
    app.sub_app_mut(RenderApp).add_systems(
        ExtractSchedule,
        extract_effect_sprites
            .after(SpriteSystems::ExtractSprites)
            .after(extract_cameras),
    );
}

#[derive(SystemParam)]
struct EffectExtraction<'w, 's> {
    atlas: Extract<'w, 's, Res<'static, CombatAtlas>>,
    bolts: Extract<'w, 's, Res<'static, ActiveCombatBolts>>,
    effects: Extract<'w, 's, Res<'static, ActiveCombatEffects>>,
    indicators: Extract<'w, 's, Res<'static, ActiveCombatIndicators>>,
    rings: Extract<'w, 's, Res<'static, ActiveSkirmishRings>>,
    view: Extract<'w, 's, Res<'static, CombatView>>,
    tuning: Extract<'w, 's, Res<'static, Tuning>>,
    atlas_layouts: Extract<'w, 's, Res<'static, Assets<TextureAtlasLayout>>>,
    nanobots: Extract<'w, 's, Query<'static, 'static, &'static PresentedNanobot>>,
    cameras: CombatCameras<'w, 's>,
}

fn extract_effect_sprites(
    mut commands: Commands,
    extraction: EffectExtraction,
    mut extracted: ResMut<ExtractedSprites>,
) {
    let EffectExtraction {
        atlas,
        bolts,
        effects,
        indicators,
        rings,
        view,
        tuning,
        atlas_layouts,
        nanobots,
        cameras,
    } = extraction;
    let draw_casualties = view.tier == CombatZoomTier::Far && !effects.casualties.is_empty();
    let draw_arcs = draw_scaled_health_arc(&view) && indicators.iter().len() > 0;
    if bolts.is_empty()
        && effects.particles.is_empty()
        && effects.muzzles.is_empty()
        && effects.death_pops.is_empty()
        && effects.decals.is_empty()
        && !draw_casualties
        && !draw_arcs
        && rings.iter().next().is_none()
    {
        return;
    }
    let settings = &tuning.presentation;
    let intensity = settings.effects_intensity;
    let image = atlas.0.id();
    for (camera, render) in &cameras {
        queue_camera_sprite_anchor(&mut commands, camera, render);
        for victim in bolts.pending_victims() {
            let Some(sprite) = &victim.sprite else {
                continue;
            };
            let radius = victim.transform.scale.truncate().abs().max_element()
                * sprite
                    .custom_size
                    .unwrap_or(Vec2::splat(64.0))
                    .max_element()
                * 0.75;
            if !view.contains_with_margin(victim.snapshot.position, radius) {
                continue;
            }
            let atlas_rect = sprite
                .texture_atlas
                .as_ref()
                .and_then(|atlas| atlas.texture_rect(&atlas_layouts))
                .map(|rect| rect.as_rect());
            let rect = match (atlas_rect, sprite.rect) {
                (Some(atlas), Some(sprite)) => Some(Rect {
                    min: atlas.min + sprite.min,
                    max: atlas.min + sprite.max,
                }),
                (atlas, sprite) => sprite.or(atlas),
            };
            extracted.sprites.push(ExtractedSprite {
                main_entity: camera,
                render_entity: commands.spawn(TemporaryRenderEntity).id(),
                transform: GlobalTransform::from(victim.transform),
                color: sprite.color.into(),
                image_handle_id: sprite.image.id(),
                flip_x: sprite.flip_x,
                flip_y: sprite.flip_y,
                kind: ExtractedSpriteKind::Single {
                    anchor: Vec2::ZERO,
                    rect,
                    scaling_mode: sprite.image_mode.scale(),
                    custom_size: sprite.custom_size,
                },
            });
        }
        let mut batch = EffectBatch {
            sprites: &mut extracted.sprites,
            view: &view,
            image,
            camera,
            z: DECAL_Z,
            render_entity: None,
            commands: &mut commands,
        };

        for decal in effects.decals.values() {
            let remaining = decal.lifetime() - (effects.clock - decal.born);
            let fade = (remaining / 10.0).clamp(0.0, 1.0) as f32;
            let (size, color, art) = if decal.structure {
                (
                    Vec2::new(64.0, 52.0),
                    Color::srgba(0.37, 0.39, 0.4, fade * 0.8),
                    Art::Rubble,
                )
            } else {
                (
                    Vec2::splat(36.0),
                    Color::srgba(0.03, 0.02, 0.02, fade * 0.9),
                    Art::Scorch,
                )
            };
            batch.sized(decal.position, size, 0.0, color, art);
        }
        batch.begin_layer(EFFECT_Z);

        for bolt in bolts.iter() {
            if intensity == 0.0 || !view.contains_with_margin(bolt.position, 36.0) {
                continue;
            }
            let travelled = bolt.position.distance(bolt.start).min(36.0);
            let color = combat_hdr(bolt.color, intensity);
            let angle = bolt.direction.to_angle();
            if view.tier == CombatZoomTier::Close && travelled > 0.1 {
                batch.push(
                    bolt.position - bolt.direction * (travelled * 0.5),
                    Vec2::new(
                        view.effect_size(Vec2::splat(travelled)).x,
                        view.line_width(4.0),
                    ),
                    angle,
                    color,
                    Art::Tail as u32,
                );
            }
            batch.sized(bolt.position, Vec2::new(10.0, 6.0), angle, color, Art::Disc);
        }

        for muzzle in &effects.muzzles {
            if intensity == 0.0 || view.tier != CombatZoomTier::Close {
                continue;
            }
            let position = nanobots
                .get(muzzle.attacker)
                .map_or(muzzle.position, |bot| bot.sprite_translation().truncate());
            let opacity = 1.0 - muzzle.elapsed / MUZZLE_LIFETIME;
            let color = combat_hdr(muzzle.color.with_alpha(opacity), intensity);
            batch.sized(position, Vec2::splat(20.0), 0.0, color, Art::Flash);
        }

        for particle in &effects.particles {
            if view.tier == CombatZoomTier::Mid && particle.close_only
                || view.tier == CombatZoomTier::Close
                    && particle.kind == CombatParticleKind::ImpactFlash
            {
                continue;
            }
            let progress = particle.elapsed / particle.lifetime;
            let opacity = 1.0 - progress;
            let (size, color, art) = match particle.kind {
                CombatParticleKind::ImpactSpark | CombatParticleKind::StructureSpark => (
                    Vec2::new(
                        view.effect_size(Vec2::splat(particle.size * opacity.lerp(0.5, progress)))
                            .x,
                        view.line_width(particle.size * 0.42),
                    ),
                    combat_hdr(particle.color.with_alpha(opacity), intensity),
                    Art::Streak,
                ),
                CombatParticleKind::ImpactFlash => (
                    Vec2::splat(particle.size),
                    combat_hdr(particle.color.with_alpha(opacity), intensity),
                    Art::Flash,
                ),
                CombatParticleKind::Fragment => (
                    Vec2::new(particle.size, particle.size * 0.59),
                    particle.color.with_alpha(opacity),
                    Art::Fragment,
                ),
                CombatParticleKind::Smoke => (
                    Vec2::new(1.0, 1.35) * particle.size * (1.0 + progress),
                    particle
                        .color
                        .with_alpha(opacity * progress.min(0.15) * 3.4),
                    Art::Smoke,
                ),
            };
            if matches!(art, Art::Streak) {
                if view.contains_with_margin(particle.position, size.length() * 0.5) {
                    batch.push(particle.position, size, particle.angle, color, art as u32);
                }
            } else {
                batch.sized(particle.position, size, particle.angle, color, art);
            }
        }

        for pop in &effects.death_pops {
            let position = pop.victim.position;
            if pop.is_structure() {
                let progress = pop.ring_progress;
                if progress < 1.0 {
                    let radius = settings
                        .combat_structure_ring_start_radius
                        .lerp(settings.combat_structure_ring_end_radius, progress);
                    batch.ring(
                        position,
                        radius,
                        settings.combat_structure_ring_thickness,
                        pop.color.with_alpha(1.0 - progress),
                        None,
                        Art::RingSegment,
                    );
                }
                let progress = pop.elapsed / STRUCTURE_FLASH_LIFETIME;
                if progress < 1.0 && intensity > 0.0 {
                    let white = combat_hdr(Color::WHITE.with_alpha(1.0 - progress), intensity);
                    batch.sized(
                        position,
                        Vec2::splat(STRUCTURE_SPRITE_SIZE),
                        0.0,
                        white,
                        Art::Flash,
                    );
                }
            } else {
                let progress = pop.elapsed / POP_LIFETIME;
                if intensity > 0.0 {
                    let color = Color::WHITE
                        .mix(&pop.color, progress.sqrt())
                        .with_alpha(1.0 - progress);
                    batch.sized(
                        position,
                        Vec2::splat(2.0 * 6.0_f32.lerp(22.0, progress)),
                        0.0,
                        combat_hdr(color, intensity),
                        Art::Disc,
                    );
                }

                let ring_progress = pop.elapsed / 0.2;
                if ring_progress < 1.0 {
                    batch.ring(
                        position,
                        10.0_f32.lerp(40.0, ring_progress),
                        2.5,
                        pop.color.with_alpha(1.0 - ring_progress),
                        None,
                        Art::RingSegment,
                    );
                }
            }
        }

        for ring in rings.iter() {
            batch.ring(
                ring.center,
                ring.radius(),
                2.5,
                ring.color,
                None,
                Art::SoftRingSegment,
            );
        }

        if draw_casualties {
            batch.begin_layer(EFFECT_Z + 0.1);
            for flash in effects.casualty_flashes() {
                let size = flash.diameter_px() * view.world_per_pixel;
                if view.contains_with_margin(flash.position, size * 0.5) {
                    batch.push(
                        flash.position,
                        Vec2::splat(size),
                        0.0,
                        flash.color.with_alpha(flash.opacity()),
                        Art::Disc as u32,
                    );
                }
            }
        }

        if draw_arcs {
            for indicator in indicators.iter() {
                if let CombatIndicatorKind::HealthArc { fraction } = indicator.kind {
                    batch.begin_layer(indicator.z);
                    batch.ring(
                        indicator.position,
                        ARC_RADIUS,
                        ARC_THICKNESS,
                        indicator.color,
                        Some(fraction),
                        Art::RingSegment,
                    );
                }
            }
        }
    }
}

struct EffectBatch<'a, 'w, 's> {
    sprites: &'a mut Vec<ExtractedSprite>,
    view: &'a CombatView,
    image: AssetId<Image>,
    camera: Entity,
    z: f32,
    render_entity: Option<Entity>,
    commands: &'a mut Commands<'w, 's>,
}

impl EffectBatch<'_, '_, '_> {
    fn begin_layer(&mut self, z: f32) {
        // Bevy keys each contiguous run in the sorted phase by its render entity.
        self.render_entity = None;
        self.z = z;
    }

    fn sized(&mut self, position: Vec2, size: Vec2, angle: f32, color: Color, art: Art) {
        let size = self.view.effect_size(size);
        if self
            .view
            .contains_with_margin(position, size.length() * 0.5)
        {
            self.push(position, size, angle, color, art as u32);
        }
    }

    fn push(&mut self, position: Vec2, size: Vec2, angle: f32, color: Color, tile: u32) {
        let min = UVec2::new(tile % TILE_COLUMNS, tile / TILE_COLUMNS).as_vec2() * TILE_SIDE as f32;
        let render_entity = *self
            .render_entity
            .get_or_insert_with(|| self.commands.spawn(TemporaryRenderEntity).id());
        self.sprites.push(ExtractedSprite {
            main_entity: self.camera,
            render_entity,
            transform: GlobalTransform::from(
                Transform::from_translation(position.extend(self.z))
                    .with_rotation(Quat::from_rotation_z(angle)),
            ),
            color: color.into(),
            image_handle_id: self.image,
            flip_x: false,
            flip_y: false,
            kind: ExtractedSpriteKind::Single {
                anchor: Vec2::ZERO,
                rect: Some(Rect::from_corners(min, min + Vec2::splat(TILE_SIDE as f32))),
                scaling_mode: None,
                custom_size: Some(size),
            },
        });
    }

    fn ring(
        &mut self,
        center: Vec2,
        radius: f32,
        thickness: f32,
        color: Color,
        fraction: Option<f32>,
        art: Art,
    ) {
        let max_chord_error = 0.25 * self.view.world_per_pixel;
        let thickness = self.view.line_width(thickness);
        let outer_diameter = self
            .view
            .effect_size(Vec2::splat(2.0 * radius + thickness))
            .x;
        let radius = (outer_diameter - thickness) * 0.5;
        let half_thickness = thickness * 0.5;
        let outer_radius = radius + half_thickness;
        if radius <= half_thickness {
            self.sized(
                center,
                Vec2::splat(2.0 * outer_radius),
                0.0,
                color,
                Art::Disc,
            );
            return;
        }
        let sides = (PI * (outer_radius / (2.0 * max_chord_error)).sqrt())
            .ceil()
            .max(16.0) as i32;
        let (start, end) = if let Some(fraction) = fraction {
            (FRAC_PI_2 - TAU * fraction, FRAC_PI_2)
        } else if let Some(arc) =
            visible_ring_arc(self.view, center, radius, half_thickness + max_chord_error)
        {
            arc
        } else {
            return;
        };
        let count = (((end - start) / TAU * sides as f32).ceil() as i32).clamp(1, sides);
        let step = (end - start) / count as f32;
        let (sin, cos) = (step * 0.5).sin_cos();
        let size = Vec2::new(2.0 * outer_radius * sin, thickness);
        let chord_radius = radius * cos;
        for side in 0..count {
            let angle = start + (side as f32 + 0.5) * step;
            let point = center + Vec2::from_angle(angle) * chord_radius;
            self.push(point, size, angle + FRAC_PI_2, color, art as u32);
        }
    }
}

fn visible_ring_arc(
    view: &CombatView,
    center: Vec2,
    radius: f32,
    margin: f32,
) -> Option<(f32, f32)> {
    let Some(bounds) = view.bounds.map(|bounds| bounds.inflate(margin)) else {
        return Some((0.0, TAU));
    };
    let toward_view = bounds.center() - center;
    let closest = center.clamp(bounds.min, bounds.max) - center;
    let farthest = (bounds.min - center).abs().max((bounds.max - center).abs());
    if closest.length_squared() > radius * radius || farthest.length_squared() < radius * radius {
        return None;
    }
    if bounds.contains(center) {
        return Some((0.0, TAU));
    }
    let corners = [
        bounds.min,
        Vec2::new(bounds.min.x, bounds.max.y),
        bounds.max,
        Vec2::new(bounds.max.x, bounds.min.y),
    ];
    let (start, end) = corners
        .into_iter()
        .fold((0.0_f32, 0.0_f32), |(min, max), corner| {
            let offset = corner - center;
            let angle = toward_view.perp_dot(offset).atan2(toward_view.dot(offset));
            (min.min(angle), max.max(angle))
        });
    let reference = toward_view.to_angle();
    Some((reference + start, reference + end))
}

fn make_atlas() -> Image {
    let tile_count = Art::SoftRingSegment as u32 + 1;
    let width = TILE_COLUMNS * TILE_SIDE;
    let height = tile_count.div_ceil(TILE_COLUMNS) * TILE_SIDE;
    let mut pixels = vec![255; (width * height * 4) as usize];
    for y in 0..height {
        for x in 0..width {
            let tile = x / TILE_SIDE + y / TILE_SIDE * TILE_COLUMNS;
            let point = Vec2::new((x % TILE_SIDE) as f32, (y % TILE_SIDE) as f32);
            let p = (point + Vec2::splat(0.5)) * (2.0 / TILE_SIDE as f32) - Vec2::ONE;
            let alpha = if tile < tile_count {
                shape_alpha(tile, p)
            } else {
                0.0
            };
            pixels[((y * width + x) * 4 + 3) as usize] = (alpha * 255.0).round() as u8;
        }
    }
    let mut atlas = Image::new(
        Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        pixels,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    atlas.sampler = ImageSampler::linear();
    atlas
}

fn coverage(distance: f32) -> f32 {
    (distance * 16.0).clamp(0.0, 1.0)
}

fn shape_alpha(tile: u32, p: Vec2) -> f32 {
    let r = p.length();
    let along = (p.x + 1.0) * 0.5;
    match tile {
        x if x == Art::RingSegment as u32 => ((1.0 - p.y.abs()) * 32.0).clamp(0.0, 1.0),
        x if x == Art::SoftRingSegment as u32 => (1.0 - p.y * p.y).max(0.0).powi(2),
        x if x == Art::Disc as u32 => coverage(1.0 - r) * (1.0 - r * 0.22),
        x if x == Art::Flash as u32 => {
            let star = p.x.abs().sqrt() + p.y.abs().sqrt();
            coverage(1.0 - r) * ((1.0 - r * 1.2).max(0.0).powi(3) + coverage(0.96 - star)).min(1.0)
        }
        x if x == Art::Tail as u32 => coverage(along - p.y.abs()) * along.powi(2),
        x if x == Art::Streak as u32 => coverage(0.94 - p.x.abs() * 0.84 - p.y.abs() * 0.84),
        x if x == Art::Fragment as u32 => coverage(0.82 - p.x.abs().max((p.y + p.x * 0.42).abs())),
        x if x == Art::Smoke as u32 => {
            let billow = (1.0 - r).max(0.0);
            billow * billow * (0.7 + (p.y * 21.0 + p.x * 14.0).sin() * 0.18)
        }
        x if x == Art::Scorch as u32 => {
            (1.0 - r).max(0.0).powi(2) * (0.78 + (p.x * 74.0).sin() * (p.y * 101.0).cos() * 0.2)
        }
        _ => {
            let alpha = (1.0 - r).max(0.0).powi(2) * 0.8;
            let rubble = [
                (vec2(-0.45, -0.2), vec2(0.12, 0.29), -0.7),
                (vec2(0.38, -0.36), vec2(0.14, 0.22), 0.66),
                (vec2(0.08, 0.42), vec2(0.32, 0.14), -0.24),
                (vec2(0.55, 0.24), vec2(0.19, 0.07), 0.43),
                (vec2(-0.39, 0.47), vec2(0.11, 0.17), -0.12),
                (vec2(0.1, -0.09), vec2(0.21, 0.08), 0.83),
            ];
            rubble
                .into_iter()
                .fold(alpha, |alpha, (center, half_size, rotation)| {
                    let local = Vec2::from_angle(rotation).rotate(p - center);
                    alpha.max(coverage((half_size - local.abs()).min_element()) * 0.95)
                })
        }
    }
}
