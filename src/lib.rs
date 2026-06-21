pub mod ai;
pub mod building;
pub mod fly_camera;
pub mod game_settings;
pub mod intent;
pub mod materials;
pub mod nanobot;
pub mod resources;
pub mod ui;
pub mod zones;

use ai::AiPlugin;
use anyhow::Result;
use bevy::{
    math::vec3,
    prelude::*,
    render::storage::ShaderStorageBuffer,
    sprite_render::{Material2dPlugin, MeshMaterial2d},
};
use building::{Minerals, ProcessingFacility};
use fly_camera::{Camera2dFlyPlugin, CameraZoom2d, FlyCamera2d};
use game_settings::GameSettings;
use intent::{IntentGrid, IntentKind, PAINT_STRENGTH_CAP};
use materials::BackgroundMaterial;
use nanobot::{
    spawn_opponent_swarm, CollapsePlugin, NanobotBundle, NanobotPlugin, NanobotType,
    PrepaintedIntent, ProductionPlugin, ProductionRatio, SeedNanobots, Swarm, SwarmBundle,
};
use resources::{ResourceDeposit, ResourceKind, ResourceLedger, Stockpile};
use ui::NanoswarmUiSetupPlugin;
use zones::{ZoneMaterial, ZoneMaterialHandleComponent, ZonesPlugin};

pub fn build_app() -> App {
    let mut app = App::new();
    app.insert_resource(IntentGrid::new(MAP_WIDTH as i32, MAP_HEIGHT as i32))
        .init_resource::<ResourceLedger>()
        .init_resource::<ProductionRatio>()
        .init_resource::<nanobot::SoftWorkSlots>()
        .add_plugins(DefaultPlugins)
        .add_plugins(Material2dPlugin::<BackgroundMaterial>::default())
        // must be before NanobotPlugin because otherwise it receives events with despawned entities
        .add_plugins(NanoswarmUiSetupPlugin)
        // must be before NanobotPlugin because otherwise it receives events with despawned entities
        .add_plugins(ZonesPlugin::default())
        .add_plugins(NanobotPlugin::default())
        // GatherPlugin must come after NanobotPlugin: the gather
        // chain orders itself behind `move_velocity_system`, which
        // only exists once NanobotPlugin is registered.
        .add_plugins(nanobot::GatherPlugin)
        // HaulPlugin chains after `move_velocity_system`, which is
        // registered by NanobotPlugin above. The arrival signal the
        // hauler systems wait for is the same one the gather chain
        // uses, so they only need to run after the movement step.
        .add_plugins(nanobot::HaulPlugin)
        // BuildPlugin chains after `move_velocity_system` so the
        // arrive system sees the pruned DirectMovementComponent.
        .add_plugins(nanobot::BuildPlugin)
        // MaintenancePlugin chains after BuildPlugin so the
        // maintenance work system can reset a structure's
        // buffer counter before the degradation system runs.
        .add_plugins(nanobot::MaintenancePlugin)
        // ProductionPlugin chains after `move_velocity_system`
        // for the same reason; auto-creation runs last in its
        // own chain so it sees the post-pick / post-work state
        // before deciding to spawn a new facility.
        .add_plugins(ProductionPlugin)
        // CollapsePlugin must run after the production work
        // system so the "is this facility currently busy?"
        // check sees the post-work state, not the pre-work
        // state of the same tick.
        .add_plugins(CollapsePlugin)
        // DefendPlugin chains after `move_velocity_system` so
        // the arrive system sees the pruned
        // DirectMovementComponent, the same signal the rest of
        // the per-role systems use.
        .add_plugins(nanobot::DefendPlugin)
        // ChargePlugin chains after `move_velocity_system`
        // and after DefendPlugin so the defend hold is
        // established before the rotation system releases it.
        // The internal order (drain -> health loss ->
        // auto-creation -> rotation -> arrive -> work) keeps
        // the charge loop self-consistent per tick.
        .add_plugins(nanobot::ChargePlugin)
        .add_plugins(AiPlugin)
        .add_plugins(Camera2dFlyPlugin)
        .add_systems(Startup, setup_things_startup.pipe(error_handler))
        .add_systems(Startup, setup_opponent_swarm_startup);
    app
}

pub fn run() {
    build_app().run();
}

pub const MAP_WIDTH: u32 = 1000;
pub const MAP_HEIGHT: u32 = 1000;
pub const ZONE_BLOCK_SIZE: f32 = 512.;

/// Z-translation for the full-map background mesh. Bevy 2D draws
/// meshes with the largest `translation.z` first, so a negative z
/// keeps the background behind every other render layer.
pub const BACKGROUND_OVERLAY_Z: f32 = -100.0;

/// Z-translation for the player-intent zone mesh. Sits above the
/// background and below the gameplay sprites so the semi-transparent
/// zone shader is visible and the swarm always renders in front of
/// paint.
pub const ZONE_OVERLAY_Z: f32 = -99.0;

/// Z-translation for gameplay sprites (resource deposits, production
/// facilities, swarm children). Higher than the zone overlay so the
/// swarm renders in front of the player's paint.
pub const GAMEPLAY_SPRITE_Z: f32 = 1.0;

/// Build the [`Transform`] for the full-map background mesh. The
/// draw-order z lives on the translation (the field Bevy 2D reads
/// for draw order); the mesh scale keeps `z = 1.0` so the unit
/// rectangle is not distorted.
pub fn background_overlay_transform(width: f32, height: f32) -> Transform {
    Transform::from_translation(Vec3::new(0.0, 0.0, BACKGROUND_OVERLAY_Z))
        .with_scale(Vec3::new(width, height, 1.0))
}

/// Build the [`Transform`] for the player-intent zone mesh. Same
/// draw-order contract as [`background_overlay_transform`]: z on
/// the translation, scale `z = 1.0`, and the zone sits above the
/// background and below the gameplay sprites.
pub fn zone_overlay_transform(width: f32, height: f32) -> Transform {
    Transform::from_translation(Vec3::new(0.0, 0.0, ZONE_OVERLAY_Z))
        .with_scale(Vec3::new(width, height, 1.0))
}

fn setup_things_startup(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut bg_mats: ResMut<Assets<BackgroundMaterial>>,
    mut zone_mats: ResMut<Assets<ZoneMaterial>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut buffers: ResMut<Assets<ShaderStorageBuffer>>,
) -> Result<()> {
    let handle = zone_mats.add(ZoneMaterial::new(MAP_WIDTH, MAP_HEIGHT, &mut buffers));
    commands
        .spawn(Camera2d)
        .insert(FlyCamera2d::default())
        .insert(CameraZoom2d {
            zoom_speed: 10.,
            zoom_min_max: (1., 100.),
            zoom: 1.,
        })
        .insert(ZoneMaterialHandleComponent {
            handle: handle.clone(),
        });

    commands.insert_resource(GameSettings::from_file_ron("config/game_settings.ron")?);

    spawn_initial_swarm(&mut commands, &asset_server);

    // Resource Deposit (mineral-bearing deposit + visual marker)
    let resource_deposit_texture = asset_server.load("resource_deposit.png");
    commands.spawn((
        Minerals {},
        ResourceDeposit {
            kind: ResourceKind::Minerals,
            amount: 1000,
            capacity: 1000,
            radius: 64.0,
        },
        (
            Sprite::from_image(resource_deposit_texture.clone()),
            Transform::from_translation(vec3(-800., 0., GAMEPLAY_SPRITE_Z))
                .with_scale(vec3(2., 2., 1.)),
        ),
    ));

    // Production Facility (starting stockpile + visual marker)
    let production_facility_texture = asset_server.load("production_facility.png");
    commands.spawn((
        ProcessingFacility {},
        Stockpile {
            kind: ResourceKind::Minerals,
            amount: 0,
            capacity: 1000,
            radius: 64.0,
        },
        (
            Sprite::from_image(production_facility_texture.clone()),
            Transform::from_translation(vec3(-300., 0., GAMEPLAY_SPRITE_Z))
                .with_scale(vec3(3., 3., 1.)),
        ),
    ));

    // background
    commands.spawn((
        Mesh2d(meshes.add(Mesh::from(Rectangle::default()))),
        MeshMaterial2d(bg_mats.add(BackgroundMaterial {})),
        background_overlay_transform(
            MAP_WIDTH as f32 * ZONE_BLOCK_SIZE,
            MAP_HEIGHT as f32 * ZONE_BLOCK_SIZE,
        ),
    ));

    // zones
    commands.spawn((
        Mesh2d(meshes.add(Mesh::from(Rectangle::default()))),
        MeshMaterial2d(handle),
        zone_overlay_transform(
            MAP_WIDTH as f32 * ZONE_BLOCK_SIZE,
            MAP_HEIGHT as f32 * ZONE_BLOCK_SIZE,
        ),
    ));
    Ok(())
}

fn spawn_initial_swarm(commands: &mut Commands<'_, '_>, asset_server: &Res<'_, AssetServer>) {
    let texture = asset_server.load("circle.png");
    commands
        .spawn(SwarmBundle {
            swarm: Swarm {},
            transform: Transform::default(),
            global_transform: GlobalTransform::default(),
            visibility: Visibility::default(),
        })
        .with_children(|p| {
            for _ in 0..4 {
                p.spawn((
                    NanobotBundle::default(),
                    Sprite::from_image(texture.clone()),
                    Transform::from_translation(vec3(0., 0., GAMEPLAY_SPRITE_Z)),
                ));
            }
            for _ in 0..100 {
                p.spawn((
                    NanobotBundle::default(),
                    Sprite::from_image(texture.clone()),
                    Transform::from_translation(vec3(100., 0., GAMEPLAY_SPRITE_Z)),
                ));
            }
        });
}

fn error_handler(In(result): In<Result<()>>) {
    if let Err(err) = result {
        println!("encountered an error {:?}", err);
    }
}

/// Materialise the first opponent swarm on the far side of
/// the map with a fixed Hauler-heavy production ratio and a
/// small prepainted Gather/Defend territory, so a player
/// running the game out of the box sees the opponent working
/// through the same systems as the player swarm. Kept as a
/// separate system because it needs `&mut World` access to
/// paint the intent grid, while the main startup system
/// stays `Commands`-based.
fn setup_opponent_swarm_startup(world: &mut World) {
    let opponent_world_pos = vec3(40.0 * ZONE_BLOCK_SIZE, 0.0, 0.0);
    let mut opponent_ratio = ProductionRatio::new();
    // Hauler-heavy so logistics keep up; a Defender
    // presence so the prepainted base holds. No active AI.
    opponent_ratio.set_target(NanobotType::Hauler, 6);
    opponent_ratio.set_target(NanobotType::Defender, 4);
    opponent_ratio.set_target(NanobotType::Worker, 4);

    spawn_opponent_swarm(
        world,
        opponent_world_pos.truncate(),
        opponent_ratio,
        &[
            PrepaintedIntent::new(IVec2::new(40, 0), IntentKind::Gather, PAINT_STRENGTH_CAP),
            PrepaintedIntent::new(IVec2::new(41, 0), IntentKind::Defend, PAINT_STRENGTH_CAP),
        ],
        &[
            SeedNanobots::new(NanobotType::Worker, 2),
            SeedNanobots::new(NanobotType::Hauler, 1),
            SeedNanobots::new(NanobotType::Defender, 1),
        ],
    );
}

#[cfg(test)]
mod overlay_transform_tests {
    //! Pins the overlay draw-order contract: z lives on
    //! `translation.z` (the field Bevy 2D reads for draw order), the
    //! mesh scale keeps `z = 1.0`, and the zone overlay draws in
    //! front of the background and behind the gameplay sprites.

    use super::*;

    #[test]
    fn background_overlay_transform_uses_translation_z_not_scale_z() {
        let t = background_overlay_transform(1024.0, 2048.0);
        assert_eq!(
            t.translation.z, BACKGROUND_OVERLAY_Z,
            "draw order lives on translation.z, not scale.z"
        );
        assert_eq!(t.scale.x, 1024.0, "world width is preserved on scale.x");
        assert_eq!(t.scale.y, 2048.0, "world height is preserved on scale.y");
        assert_eq!(
            t.scale.z, 1.0,
            "mesh scale.z must stay 1.0 so the unit rectangle is not distorted"
        );
    }

    #[test]
    fn zone_overlay_transform_uses_translation_z_above_background() {
        let bg = background_overlay_transform(1024.0, 2048.0);
        let zone = zone_overlay_transform(1024.0, 2048.0);
        assert_eq!(
            zone.translation.z, ZONE_OVERLAY_Z,
            "draw order lives on translation.z, not scale.z"
        );
        assert!(
            zone.translation.z > bg.translation.z,
            "zone overlay must draw in front of the background \
             (zone z={} must be greater than background z={})",
            zone.translation.z,
            bg.translation.z
        );
        assert_eq!(zone.scale.z, 1.0);
    }

    #[test]
    fn zone_overlay_transform_sits_below_gameplay_sprite_z() {
        // Gameplay sprites (resource deposit, production facility,
        // swarm children) all sit at `GAMEPLAY_SPRITE_Z`. The zone overlay
        // must stay below that so the swarm renders in front of the
        // player's paint.
        let zone = zone_overlay_transform(1024.0, 2048.0);
        assert!(
            zone.translation.z < GAMEPLAY_SPRITE_Z,
            "zone overlay must draw behind gameplay sprites \
             (zone z={} must be less than gameplay z={})",
            zone.translation.z,
            GAMEPLAY_SPRITE_Z
        );
    }
}
