pub mod ai;
pub mod building;
pub mod fly_camera;
pub mod game_settings;
pub mod intent;
pub mod materials;
pub mod nanobot;
pub mod resources;
pub mod scenario;
pub mod structure_overlay;
pub mod structure_sprites;
pub mod tactical_overlay;
pub mod ui;
pub mod zones;

use ai::AiPlugin;
use anyhow::Result;
use bevy::{
    prelude::*,
    render::storage::ShaderStorageBuffer,
    sprite_render::{Material2dPlugin, MeshMaterial2d},
    window::{
        Window, WindowLevel, WindowMode, WindowPlugin, WindowResizeConstraints, WindowResolution,
    },
};
use fly_camera::{Camera2dFlyPlugin, CameraZoom2d, FlyCamera2d};
use game_settings::GameSettings;
use intent::IntentGrid;
use materials::BackgroundMaterial;
use nanobot::{
    CentralDemandPlugin, CollapsePlugin, NanobotPlugin, PlannedStructurePlugin, ProductionPlugin,
};
use resources::ResourceLedger;
use structure_overlay::StructureOverlayPlugin;
use structure_sprites::StructureSprites;
use tactical_overlay::TacticalOverlayPlugin;
use ui::NanoswarmUiSetupPlugin;
use zones::{ZoneMaterial, ZoneMaterialHandleComponent, ZonesPlugin};

/// Logical window size used for the primary window.
///
/// Kept fixed (min == max) so tiling Wayland compositors such as sway
/// float the window on map instead of tiling it. sway's `wants_floating`
/// predicate floats a view when `min_width > 0 && min_height > 0 &&
/// (min_width == max_width || min_height == max_height)`, evaluated from the
/// xdg-shell `set_min_size`/`set_max_size` hints (Wayland native) or the X11
/// `WM_NORMAL_HINTS` size hints (XWayland). Bevy's winit backend forwards
/// `WindowResizeConstraints` to both, so pinning the size triggers floating
/// on either path. See sway `sway/desktop/xdg_shell.c::wants_floating` and
/// `sway/desktop/xwayland.c::wants_floating`.
const PRIMARY_WINDOW_WIDTH: f32 = 1280.0;
const PRIMARY_WINDOW_HEIGHT: f32 = 720.0;

fn primary_window_config() -> Window {
    let size_constraints = WindowResizeConstraints {
        min_width: PRIMARY_WINDOW_WIDTH,
        min_height: PRIMARY_WINDOW_HEIGHT,
        max_width: PRIMARY_WINDOW_WIDTH,
        max_height: PRIMARY_WINDOW_HEIGHT,
    };
    Window {
        name: Some("nano-swarm".into()),
        mode: WindowMode::Windowed,
        resolution: WindowResolution::new(
            PRIMARY_WINDOW_WIDTH as u32,
            PRIMARY_WINDOW_HEIGHT as u32,
        ),
        resize_constraints: size_constraints,
        window_level: WindowLevel::AlwaysOnTop,
        ..default()
    }
}

pub fn build_app() -> App {
    let mut app = App::new();
    app.insert_resource(IntentGrid::new(MAP_WIDTH as i32, MAP_HEIGHT as i32))
        .init_resource::<ResourceLedger>()
        .insert_resource(scenario::default_player_ratio())
        .init_resource::<nanobot::SoftWorkSlots>()
        .init_resource::<nanobot::OpponentSwarmIdAlloc>()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(primary_window_config()),
            ..default()
        }))
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
        // The legacy `BuildPlugin` (issue #10's `BuildSite` /
        // `Structure` auto-creation) is intentionally NOT
        // registered. After the four Planned Structure
        // migration issues (#25 Source Stockpile, #26 Sink
        // Stockpile, #27 Production Facility, #28 Charger)
        // every demand-driven support structure lives on
        // the `PlannedStructurePlugin` lifecycle. Issue #29
        // removed the remaining "spontaneous Build-paint
        // spawn" path -- a Build cell now plans a Sink
        // Stockpile through the planned-structure chain,
        // and never spawns a legacy `BuildSite`. The
        // `BuildSite` and `BuildPlugin` types still exist
        // in the codebase for the unit tests in
        // `src/nanobot/build.rs`; the `Structure` type
        // also stays because the maintenance test fixtures
        // spawn `Structure` entities directly to drive the
        // maintenance chain. No production system
        // auto-spawns any of them.
        .add_plugins(PlannedStructurePlugin)
        // MaintenancePlugin chains after the planned-structure
        // work system so the maintenance work system can reset
        // a structure's buffer counter before the degradation
        // system runs. Maintenance operates on the legacy
        // `Structure` component, which is still in the codebase
        // for test fixtures and unit tests; production
        // structures (Stockpile, ProductionFacility, Charger)
        // do not currently participate in the maintenance
        // loop. Future support-structure kinds can opt in to
        // maintenance by carrying a `Structure` sidecar.
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
        // CentralDemandPlugin runs after `move_velocity_system`
        // and before the per-category assignment systems so the
        // first allocation of an idle nanobot flows through the
        // central allocator's Minimum Category Activation path
        // rather than through the per-category "steal a worker"
        // path (issue #35).
        .add_plugins(CentralDemandPlugin)
        // ChargePlugin chains after `move_velocity_system`
        // and after DefendPlugin so the defend hold is
        // established before the rotation system releases it.
        // The internal order (drain -> health loss ->
        // auto-creation -> rotation -> arrive -> work) keeps
        // the charge loop self-consistent per tick.
        .add_plugins(nanobot::ChargePlugin)
        // StructureOverlayPlugin is a consumer of the
        // simulation's per-structure state. It registers
        // its spawn/update/visibility/cleanup systems on
        // the `Update` schedule; nothing in the rest of
        // the plugin graph needs to be ordered relative
        // to it, so the placement near the end of the
        // plugin list is purely cosmetic.
        .add_plugins(StructureOverlayPlugin)
        // TacticalOverlayPlugin is the zoomed-out companion
        // to StructureOverlayPlugin. The two layers share
        // the same show / hide threshold so the structure
        // status labels fade out exactly as the tactical
        // overlay fades in.
        .add_plugins(TacticalOverlayPlugin)
        .add_plugins(AiPlugin)
        .add_plugins(Camera2dFlyPlugin)
        .add_systems(Startup, setup_things_startup.pipe(error_handler));
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

#[allow(clippy::too_many_arguments)]
fn setup_things_startup(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut bg_mats: ResMut<Assets<BackgroundMaterial>>,
    mut zone_mats: ResMut<Assets<ZoneMaterial>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut buffers: ResMut<Assets<ShaderStorageBuffer>>,
    mut grid: ResMut<IntentGrid>,
    opponent_id_alloc: ResMut<nanobot::OpponentSwarmIdAlloc>,
) -> Result<()> {
    let handle = zone_mats.add(ZoneMaterial::new(MAP_WIDTH, MAP_HEIGHT, &mut buffers));
    commands
        .spawn(Camera2d)
        .insert(FlyCamera2d::default())
        .insert(CameraZoom2d {
            zoom_speed: 10.,
            zoom_min_max: (1., 100.),
            zoom: 2.,
        })
        .insert(ZoneMaterialHandleComponent {
            handle: handle.clone(),
        });

    commands.insert_resource(GameSettings::from_file_ron("config/game_settings.ron")?);
    commands.insert_resource(StructureSprites::load(&asset_server));

    scenario::spawn_default_player_scenario(&mut commands, &asset_server, &mut grid);
    scenario::spawn_default_opponent_scenario(
        &mut commands,
        &asset_server,
        &mut grid,
        opponent_id_alloc,
    );

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

fn error_handler(In(result): In<Result<()>>) {
    if let Err(err) = result {
        println!("encountered an error {:?}", err);
    }
}

#[cfg(test)]
mod overlay_transform_tests {
    //! Pins the overlay draw-order contract: z lives on
    //! `translation.z` (the field Bevy 2D reads for draw order), the
    //! mesh scale keeps `z = 1.0`, and the zone overlay draws in
    //! front of the background and behind the gameplay sprites.

    use super::*;

    #[test]
    fn primary_window_starts_windowed_and_always_on_top() {
        let window = primary_window_config();
        assert_eq!(window.mode, WindowMode::Windowed);
        assert_eq!(window.window_level, WindowLevel::AlwaysOnTop);
    }

    #[test]
    fn primary_window_pins_size_so_sway_floats_it() {
        // sway floats a view when min_width > 0 && min_height > 0 &&
        // (min_width == max_width || min_height == max_height). Pinning the
        // window size satisfies that on both Wayland native (xdg-shell size
        // hints) and XWayland (WM_NORMAL_HINTS).
        let window = primary_window_config();
        let c = window.resize_constraints;
        assert!(c.min_width > 0.0 && c.min_height > 0.0);
        assert_eq!(c.min_width, c.max_width);
        assert_eq!(c.min_height, c.max_height);
        assert_eq!(window.name.as_deref(), Some("nano-swarm"));
    }

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
