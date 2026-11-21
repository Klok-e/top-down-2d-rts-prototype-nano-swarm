//! Aggregate combat activity for the zoomed-out view.

use std::{f32::consts::TAU, time::Duration};

use bevy::{
    asset::RenderAssetUsages,
    image::ImageSampler,
    mesh::MeshVertexBufferLayoutRef,
    prelude::*,
    render::{
        Extract, ExtractSchedule, Render, RenderApp, RenderSystems,
        camera::extract_cameras,
        extract_resource::{ExtractResource, ExtractResourcePlugin},
        render_asset::{RenderAssets, prepare_assets},
        render_resource::{
            AsBindGroup, BlendComponent, BlendFactor, BlendOperation, BlendState, Extent3d,
            RenderPipelineDescriptor, SpecializedMeshPipelineError, TexelCopyBufferLayout,
            TextureDimension, TextureFormat,
        },
        renderer::RenderQueue,
        sync_world::TemporaryRenderEntity,
        texture::GpuImage,
    },
    shader::ShaderRef,
    sprite_render::{
        AlphaMode2d, ExtractedSprite, ExtractedSpriteKind, ExtractedSprites, Material2d,
        Material2dKey, Material2dPlugin, SpriteSystems,
    },
};

use crate::{
    GAMEPLAY_SPRITE_Z,
    nanobot::{ResolvedCombatFact, SwarmId, UpdateNanobotPresentation, swarm_presentation_color},
    scenario::map_bounds,
    tactical_overlay::{TACTICAL_MARKER_ALPHA, TACTICAL_MARKER_Z, marker_world_size_for_zoom},
    tuning::Tuning,
};

use super::{
    CombatTickSnapshots, CombatView, CombatZoomTier,
    render_anchor::{CombatCameras, queue_camera_sprite_anchor},
    resolve_combat, update_combat_view,
};

const BUCKET_SIZE: f32 = 128.0;
const HEAT_HALF_LIFE: f32 = 0.7;
const MARKER_SIZE: f32 = 1024.0;
const BUCKETS_PER_MARKER_SIDE: u32 = (MARKER_SIZE / BUCKET_SIZE) as u32;
const HEAT_Z: f32 = GAMEPLAY_SPRITE_Z - 0.03;

fn remaining_heat(seconds: f32) -> f32 {
    (-seconds / HEAT_HALF_LIFE).exp2()
}

#[derive(Debug, Clone, Copy)]
pub struct BattleMarker {
    pub position: Vec2,
    pub heat: f32,
}

#[derive(Resource)]
pub struct CombatHeat {
    bounds: Rect,
    size: UVec2,
    channels: Vec<[f32; 2]>,
    active: Vec<usize>,
    marker_size: UVec2,
    marker_heat: Vec<f32>,
    markers: Vec<BattleMarker>,
    dirty: bool,
}

impl Default for CombatHeat {
    fn default() -> Self {
        let bounds = map_bounds();
        let size = (bounds.size() / BUCKET_SIZE).ceil().as_uvec2();
        let marker_size = (bounds.size() / MARKER_SIZE).ceil().as_uvec2();
        Self {
            bounds,
            size,
            channels: vec![[0.0; 2]; (size.x * size.y) as usize],
            active: Vec::new(),
            marker_size,
            marker_heat: vec![0.0; (marker_size.x * marker_size.y) as usize],
            markers: Vec::new(),
            dirty: false,
        }
    }
}

impl CombatHeat {
    /// Total hit activity remaining after decay.
    pub fn total(&self) -> f32 {
        self.active
            .iter()
            .map(|&index| self.channels[index].iter().sum::<f32>())
            .sum()
    }

    /// Battle markers eligible at the current zoom, including those outside the view.
    pub fn markers(&self) -> &[BattleMarker] {
        &self.markers
    }

    fn add(&mut self, position: Vec2, swarm: SwarmId, amount: f32) {
        if amount < f32::EPSILON || !self.bounds.contains(position) {
            return;
        }
        let cell = ((position - self.bounds.min) / BUCKET_SIZE)
            .as_uvec2()
            .min(self.size - UVec2::ONE);
        let index = (cell.y * self.size.x + cell.x) as usize;
        let channels = &mut self.channels[index];
        if *channels == [0.0; 2] {
            self.active.push(index);
        }
        channels[usize::from(!swarm.is_player())] += amount;
        self.dirty = true;
    }

    fn decay(&mut self, delta: f32) {
        self.dirty = delta > 0.0 && !self.active.is_empty();
        if !self.dirty {
            return;
        }
        let decay = remaining_heat(delta);
        self.active.retain(|&index| {
            let channels = &mut self.channels[index];
            channels[0] *= decay;
            channels[1] *= decay;
            if channels[0] + channels[1] < f32::EPSILON {
                *channels = [0.0; 2];
                return false;
            }
            true
        });
    }

    fn update_markers(&mut self, show: bool, threshold: f32) {
        self.markers.clear();
        if !show || self.active.is_empty() {
            return;
        }
        self.marker_heat.fill(0.0);
        for &index in &self.active {
            let cell = UVec2::new(index as u32 % self.size.x, index as u32 / self.size.x);
            let marker = cell / BUCKETS_PER_MARKER_SIDE;
            let [a, b] = self.channels[index];
            self.marker_heat[(marker.y * self.marker_size.x + marker.x) as usize] += a + b;
        }
        for (index, &heat) in self.marker_heat.iter().enumerate() {
            if heat > threshold {
                let cell = UVec2::new(
                    index as u32 % self.marker_size.x,
                    index as u32 / self.marker_size.x,
                );
                let position = self.bounds.min + (cell.as_vec2() + 0.5) * MARKER_SIZE;
                self.markers.push(BattleMarker { position, heat });
            }
        }
    }

    fn write_pixels(&self, pixels: &mut [u8], tier_opacity: f32) {
        pixels.fill(0);
        let [player, opponent] = [SwarmId::PLAYER, SwarmId(1)].map(|id| {
            let color = swarm_presentation_color(id).to_linear();
            Vec3::new(color.red, color.green, color.blue)
        });
        for &index in &self.active {
            let [a, b] = self.channels[index];
            let heat = a + b;
            let alpha = tier_opacity * (1.0 - (-0.15 * heat).exp());
            let color = opponent.lerp(player, a / heat) * alpha;
            let rgba = (color.extend(alpha) * 255.0).round();
            pixels[index * 4..index * 4 + 4].copy_from_slice(&rgba.to_array().map(|x| x as u8));
        }
    }
}

pub(super) struct CombatHeatPlugin;

impl Plugin for CombatHeatPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CombatHeat>().add_systems(
            Update,
            update_heat
                .after(update_combat_view)
                .before(resolve_combat)
                .in_set(UpdateNanobotPresentation),
        );
        if app.get_sub_app(RenderApp).is_none() {
            return;
        }
        app.add_plugins((
            Material2dPlugin::<HeatMaterial>::default(),
            ExtractResourcePlugin::<HeatTexture>::default(),
        ))
        .add_systems(Startup, setup_heat_quad)
        .add_systems(Update, update_heat_quad.after(update_heat));

        let icon = app
            .world()
            .resource::<AssetServer>()
            .load("battle_marker.png");
        app.insert_resource(BattleMarkerIcon(icon));
        app.sub_app_mut(RenderApp)
            .add_systems(
                Render,
                upload_heat_texture
                    .in_set(RenderSystems::PrepareResources)
                    .after(prepare_assets::<GpuImage>),
            )
            .add_systems(
                ExtractSchedule,
                extract_battle_markers
                    .after(SpriteSystems::ExtractSprites)
                    .after(extract_cameras),
            );
    }
}

pub(super) fn reset_session(world: &mut World) {
    crate::nanobot::reset_resource::<CombatHeat>(world);
}

fn update_heat(
    time: Option<Res<Time>>,
    view: Res<CombatView>,
    tuning: Res<Tuning>,
    snapshots: Res<CombatTickSnapshots>,
    mut facts: MessageReader<ResolvedCombatFact>,
    mut heat: ResMut<CombatHeat>,
) {
    let (now, delta) = time.as_ref().map_or((Duration::ZERO, 0.0), |time| {
        (time.elapsed(), time.delta_secs())
    });
    heat.decay(delta);
    for (fact, id) in facts.read_with_id() {
        if let ResolvedCombatFact::Hit(hit) = fact {
            let age = now.saturating_sub(snapshots.hit_time(id).unwrap_or(now));
            heat.add(
                hit.target.position,
                hit.attacker.swarm,
                remaining_heat(age.as_secs_f32()),
            );
        }
    }
    heat.update_markers(
        view.zoom >= tuning.presentation.tactical_far_merge_zoom,
        tuning.presentation.battle_marker_heat,
    );
}

#[derive(Component)]
struct HeatQuad;

#[derive(Resource, ExtractResource, Clone)]
struct HeatTexture {
    image: Handle<Image>,
    pixels: Vec<u8>,
    opacity: f32,
}

#[derive(Asset, TypePath, AsBindGroup, Clone)]
struct HeatMaterial {
    #[texture(0)]
    #[sampler(1)]
    image: Handle<Image>,
}

impl Material2d for HeatMaterial {
    fn fragment_shader() -> ShaderRef {
        "shaders/combat_heat.wgsl".into()
    }

    fn alpha_mode(&self) -> AlphaMode2d {
        AlphaMode2d::Blend
    }

    fn specialize(
        descriptor: &mut RenderPipelineDescriptor,
        _: &MeshVertexBufferLayoutRef,
        _: Material2dKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        if let Some(target) = descriptor.fragment.as_mut().unwrap().targets[0].as_mut() {
            target.blend = Some(BlendState {
                color: BlendComponent {
                    src_factor: BlendFactor::One,
                    dst_factor: BlendFactor::One,
                    operation: BlendOperation::Add,
                },
                alpha: BlendComponent {
                    src_factor: BlendFactor::Zero,
                    dst_factor: BlendFactor::One,
                    operation: BlendOperation::Add,
                },
            });
        }
        Ok(())
    }
}

fn setup_heat_quad(
    mut commands: Commands,
    heat: Res<CombatHeat>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<HeatMaterial>>,
) {
    let mut image = Image::new_uninit(
        Extent3d {
            width: heat.size.x,
            height: heat.size.y,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::linear();
    let image = images.add(image);
    commands.insert_resource(HeatTexture {
        image: image.clone(),
        pixels: vec![0; heat.channels.len() * 4],
        opacity: 0.0,
    });
    commands.spawn((
        HeatQuad,
        Mesh2d(meshes.add(Rectangle::from_size(heat.bounds.size()))),
        MeshMaterial2d(materials.add(HeatMaterial { image })),
        Transform::from_translation(heat.bounds.center().extend(HEAT_Z)),
        Visibility::Hidden,
    ));
}

fn update_heat_quad(
    heat: Res<CombatHeat>,
    view: Res<CombatView>,
    mut texture: ResMut<HeatTexture>,
    mut visibility: Single<&mut Visibility, With<HeatQuad>>,
) {
    let opacity = match view.tier {
        CombatZoomTier::Close => 0.0,
        CombatZoomTier::Mid => 0.4,
        CombatZoomTier::Far => 1.0,
    };
    if opacity == 0.0 || heat.active.is_empty() {
        visibility.set_if_neq(Visibility::Hidden);
        return;
    }
    if heat.dirty || **visibility == Visibility::Hidden || texture.opacity != opacity {
        heat.write_pixels(&mut texture.pixels, opacity);
        texture.opacity = opacity;
    }
    visibility.set_if_neq(Visibility::Inherited);
}

fn upload_heat_texture(
    texture: Res<HeatTexture>,
    images: Res<RenderAssets<GpuImage>>,
    queue: Res<RenderQueue>,
    mut needs_upload: Local<bool>,
) {
    *needs_upload |= texture.is_changed();
    if !*needs_upload {
        return;
    }
    let Some(image) = images.get(&texture.image) else {
        return;
    };
    // Reuse the GPU texture so the material's bind group stays valid as the heat changes.
    queue.write_texture(
        image.texture.as_image_copy(),
        &texture.pixels,
        TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(image.size.width * 4),
            rows_per_image: None,
        },
        image.size,
    );
    *needs_upload = false;
}

#[derive(Resource)]
struct BattleMarkerIcon(Handle<Image>);

#[allow(clippy::too_many_arguments)]
fn extract_battle_markers(
    mut commands: Commands,
    heat: Extract<Res<CombatHeat>>,
    view: Extract<Res<CombatView>>,
    icon: Extract<Res<BattleMarkerIcon>>,
    time: Extract<Res<Time>>,
    tuning: Extract<Res<Tuning>>,
    cameras: CombatCameras,
    mut extracted: ResMut<ExtractedSprites>,
) {
    if heat.markers.is_empty() {
        return;
    }
    let settings = &tuning.presentation;
    let base_size = marker_world_size_for_zoom(view.zoom, settings.tactical_marker_screen_size);
    let pulse = 0.85 + 0.15 * (time.elapsed_secs_wrapped() * TAU).cos();
    let color = Color::WHITE
        .with_alpha(TACTICAL_MARKER_ALPHA * pulse)
        .to_linear();
    for (camera, render) in &cameras {
        queue_camera_sprite_anchor(&mut commands, camera, render);
        let render_entity = commands.spawn(TemporaryRenderEntity).id();
        for marker in &heat.markers {
            let threshold = settings.battle_marker_heat;
            let growth = ((marker.heat - threshold) / (4.0 * threshold)).min(1.0);
            let size = base_size * (1.0 + 0.6 * growth);
            if !view.contains_with_margin(marker.position, size * 0.5) {
                continue;
            }
            extracted.sprites.push(ExtractedSprite {
                main_entity: camera,
                render_entity,
                transform: GlobalTransform::from_translation(
                    marker.position.extend(TACTICAL_MARKER_Z + 0.1),
                ),
                color,
                image_handle_id: icon.0.id(),
                flip_x: false,
                flip_y: false,
                kind: ExtractedSpriteKind::Single {
                    anchor: Vec2::ZERO,
                    rect: None,
                    scaling_mode: None,
                    custom_size: Some(Vec2::splat(size)),
                },
            });
        }
    }
}
