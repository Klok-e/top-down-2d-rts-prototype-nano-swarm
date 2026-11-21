use bevy::{prelude::*, ui::UiGlobalTransform};
use top_down_2d_rts_prototype_nano_swarm::{
    MainCamera, ZONE_BLOCK_SIZE,
    fly_camera::{CameraZoom2d, FlyCamera2d},
    ui::NoPointerCapture,
    zones::BrushFeedback,
};

use crate::harness::{SCREENSHOT_DIR, TestContext, TestFlow};

const PARTLY_OPEN_TILE: IVec2 = IVec2::new(1, 5);
const SOLID_MESA: IVec2 = IVec2::new(5, 12);

pub fn brush_feedback(ctx: &mut TestContext) -> TestFlow {
    match ctx.frame {
        0 => {
            let mut window = Window {
                resolution: (1280, 720).into(),
                ..default()
            };
            window.set_cursor_position(Some(Vec2::new(640.0, 360.0)));
            ctx.world.spawn(window);
            center_camera(ctx.world, PARTLY_OPEN_TILE);
        }
        3 => return TestFlow::Screenshot("brush_feedback_hover".into()),
        4 => {
            let hover = ctx.world.resource::<BrushFeedback>().hover.unwrap();
            assert_eq!(hover.cell, PARTLY_OPEN_TILE);
            assert!(hover.accepts_action);
            assert_border("brush_feedback_hover", |[r, g, b, _]| {
                r > 240 && g > 240 && b > 240
            });
            set_zoom_and_velocity(ctx.world, 12.0, Vec2::new(700.0, 0.0));
            ctx.world
                .resource_mut::<ButtonInput<KeyCode>>()
                .press(KeyCode::ArrowRight);
        }
        5 => {
            assert_preview_follows_camera(ctx.world);
            ctx.world
                .resource_mut::<ButtonInput<KeyCode>>()
                .release(KeyCode::ArrowRight);
            set_zoom_and_velocity(ctx.world, 2.0, Vec2::ZERO);
            center_camera(ctx.world, SOLID_MESA);
        }
        7 => {
            let hover = ctx.world.resource::<BrushFeedback>().hover.unwrap();
            assert_eq!(hover.cell, SOLID_MESA);
            assert!(!hover.accepts_action);
            ctx.world
                .resource_mut::<ButtonInput<MouseButton>>()
                .press(MouseButton::Left);
        }
        8 => return TestFlow::Screenshot("brush_feedback_unreachable".into()),
        9 => {
            assert!(ctx.world.resource::<BrushFeedback>().unreachable);
            assert_border("brush_feedback_unreachable", is_amber);
            let image = open_capture("brush_feedback_unreachable");
            assert!(
                (660..700)
                    .flat_map(|y| (490..790).map(move |x| (x, y)))
                    .filter(|&(x, y)| is_amber(image.get_pixel(x, y).0))
                    .count()
                    > 120,
                "the refused paint must show an amber notice below the map view"
            );
            ctx.world
                .resource_mut::<ButtonInput<MouseButton>>()
                .release(MouseButton::Left);
            return TestFlow::Exit;
        }
        _ => {}
    }
    TestFlow::Continue
}

fn center_camera(world: &mut World, cell: IVec2) {
    let center = (cell.as_vec2() + Vec2::splat(0.5)) * ZONE_BLOCK_SIZE;
    let mut query = world.query_filtered::<&mut Transform, With<MainCamera>>();
    let mut camera = query.single_mut(world).unwrap();
    camera.translation.x = center.x;
    camera.translation.y = center.y;
}

fn set_zoom_and_velocity(world: &mut World, scale: f32, velocity: Vec2) {
    let mut query = world.query::<(&mut Projection, &mut CameraZoom2d, &mut FlyCamera2d)>();
    let (mut projection, mut zoom, mut movement) = query.single_mut(world).unwrap();
    let Projection::Orthographic(projection) = &mut *projection else {
        panic!("brush preview requires the strategy camera's orthographic projection");
    };
    projection.scale = scale;
    zoom.zoom = scale;
    movement.velocity = velocity;
}

fn assert_preview_follows_camera(world: &mut World) {
    assert_eq!(
        world.resource::<BrushFeedback>().hover.unwrap().cell,
        PARTLY_OPEN_TILE
    );
    let cell_center = ((PARTLY_OPEN_TILE.as_vec2() + 0.5) * ZONE_BLOCK_SIZE).extend(0.0);
    let (camera, pose) = world
        .query::<(&Camera, &GlobalTransform)>()
        .single(world)
        .unwrap();
    let expected = camera.world_to_viewport(pose, cell_center).unwrap();
    assert!(expected.x < 635.0, "fixture must pan several screen pixels");
    let outline = world
        .query_filtered::<&UiGlobalTransform, (With<Outline>, With<NoPointerCapture>)>()
        .single(world)
        .unwrap();
    assert!(
        outline.translation.abs_diff_eq(expected, 1.0),
        "the hovered cell outline must follow the camera in the same frame"
    );
}

fn open_capture(name: &str) -> image::RgbaImage {
    image::open(format!("{SCREENSHOT_DIR}/{name}.png"))
        .unwrap()
        .into_rgba8()
}

fn assert_border(name: &str, color: impl Fn([u8; 4]) -> bool) {
    let image = open_capture(name);
    assert!(
        (510..516).any(|x| {
            (245..475)
                .filter(|&y| color(image.get_pixel(x, y).0))
                .count()
                > 210
        }),
        "the hovered cell must show a continuous outline"
    );
}

fn is_amber([r, g, b, _]: [u8; 4]) -> bool {
    r > 215 && (140..205).contains(&g) && b < 120
}
