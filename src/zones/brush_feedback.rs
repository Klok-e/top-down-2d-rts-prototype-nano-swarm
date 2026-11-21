use bevy::{prelude::*, ui::FocusPolicy};

use crate::{ZONE_BLOCK_SIZE, ui::NoPointerCapture};

const UNREACHABLE_COLOR: Color = Color::srgb(1.0, 0.73, 0.38);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BrushHover {
    pub cell: IVec2,
    pub accepts_action: bool,
}

#[derive(Default, Resource)]
pub struct BrushFeedback {
    pub(super) enabled: bool,
    pub hover: Option<BrushHover>,
    pub unreachable: bool,
    pub(crate) active_gesture: bool,
}

#[derive(Component)]
pub(super) struct BrushOutline;

#[derive(Component)]
pub(super) struct BrushNotice;

pub(super) fn setup_brush_feedback(mut commands: Commands, assets: Res<AssetServer>) {
    commands.spawn((
        BrushOutline,
        NoPointerCapture,
        FocusPolicy::Pass,
        ZIndex(-1),
        Visibility::Hidden,
        Node {
            position_type: PositionType::Absolute,
            border: UiRect::all(Val::Px(2.0)),
            ..default()
        },
        BorderColor::all(Color::WHITE),
        Outline::new(Val::Px(1.0), Val::Px(0.0), Color::BLACK),
    ));
    commands
        .spawn((
            BrushNotice,
            NoPointerCapture,
            FocusPolicy::Pass,
            Visibility::Hidden,
            Node {
                position_type: PositionType::Absolute,
                bottom: Val::Px(20.0),
                width: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                ..default()
            },
        ))
        .with_children(|parent| {
            parent
                .spawn((
                    Node {
                        padding: UiRect::axes(Val::Px(12.0), Val::Px(6.0)),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.04, 0.05, 0.06, 0.95)),
                ))
                .with_child((
                    Text::new("Cannot paint an unreachable cell"),
                    TextFont {
                        font: assets.load("fonts/fira_sans/FiraSans-Bold.ttf"),
                        font_size: 16.0,
                        ..default()
                    },
                    TextColor(UNREACHABLE_COLOR),
                ));
        });
}

#[allow(clippy::type_complexity)]
pub(super) fn update_brush_feedback(
    feedback: Res<BrushFeedback>,
    time: Res<Time<Real>>,
    scale: Res<UiScale>,
    cameras: Query<(&Camera, &Transform)>,
    mut outline: Query<(&mut Node, &mut BorderColor, &mut Visibility), With<BrushOutline>>,
    mut notice: Query<&mut Visibility, (With<BrushNotice>, Without<BrushOutline>)>,
    mut notice_seconds: Local<f32>,
) {
    let rect = feedback.hover.and_then(|hover| {
        let (camera, transform) = cameras.single().ok()?;
        let camera_pose = GlobalTransform::from(*transform);
        let origin = hover.cell.as_vec2() * ZONE_BLOCK_SIZE;
        let a = camera
            .world_to_viewport(&camera_pose, origin.extend(0.0))
            .ok()?;
        let b = camera
            .world_to_viewport(
                &camera_pose,
                (origin + Vec2::splat(ZONE_BLOCK_SIZE)).extend(0.0),
            )
            .ok()?;
        Some((
            a.min(b) / scale.0,
            (b - a).abs() / scale.0,
            hover.accepts_action,
        ))
    });
    if let Ok((mut node, mut color, mut visibility)) = outline.single_mut() {
        if let Some((origin, size, accepts_action)) = rect {
            let bounds = (
                Val::Px(origin.x),
                Val::Px(origin.y),
                Val::Px(size.x),
                Val::Px(size.y),
            );
            if (node.left, node.top, node.width, node.height) != bounds {
                (node.left, node.top, node.width, node.height) = bounds;
            }
            color.set_if_neq(BorderColor::all(if accepts_action {
                Color::WHITE
            } else {
                UNREACHABLE_COLOR
            }));
            visibility.set_if_neq(Visibility::Inherited);
        } else {
            visibility.set_if_neq(Visibility::Hidden);
        }
    }

    if !feedback.enabled {
        *notice_seconds = 0.0;
    } else if feedback.unreachable {
        *notice_seconds = 2.0;
    } else {
        *notice_seconds -= time.delta_secs();
    }
    if let Ok(mut visibility) = notice.single_mut() {
        visibility.set_if_neq(if *notice_seconds > 0.0 {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
    }
}
