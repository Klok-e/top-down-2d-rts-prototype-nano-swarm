//! Open basins, sheltered clearings, and rounded landforms in intent-cell units.

use bevy::prelude::*;

use crate::{ZONE_BLOCK_SIZE, terrain::RockFormation};

use super::{PLAYER_CELL, cell_origin};

const MIN: f32 = -2.0;
const TILE: f32 = 0.125;
const SIDE: usize = 224;

fn segment_distance(point: Vec2, start: Vec2, end: Vec2) -> f32 {
    let direction = end - start;
    let along = ((point - start).dot(direction) / direction.length_squared()).clamp(0.0, 1.0);
    point.distance(start + direction * along)
}

fn path_contains(point: Vec2, points: &[(f32, f32)], width: f32) -> bool {
    points
        .windows(2)
        .any(|pair| segment_distance(point, Vec2::from(pair[0]), Vec2::from(pair[1])) < width / 2.0)
}

fn rounded_square(point: Vec2, half: f32, radius: f32) -> bool {
    let q = (point - Vec2::splat(12.0)).abs() - Vec2::splat(half - radius);
    q.max(Vec2::ZERO).length() + q.max_element().min(0.0) < radius
}

fn oval(point: Vec2, center: Vec2, axes: Vec2, angle: f32) -> bool {
    let delta = point - center;
    let (sin, cos) = angle.sin_cos();
    let local = Vec2::new(
        cos * delta.x + sin * delta.y,
        -sin * delta.x + cos * delta.y,
    );
    (local / axes).length_squared() < 1.0
}

fn landforms(point: Vec2) -> bool {
    // A crescent shelters the spacious home clearing. The diagonal gateway
    // and northern side pass are the only breaks through its enclosing ridge.
    let shelter = path_contains(
        point,
        &[
            (-1.4, 4.0),
            (0.8, 4.8),
            (2.2, 4.6),
            (4.0, 2.3),
            (4.2, 0.2),
            (3.5, -1.5),
        ],
        0.9,
    ) && !path_contains(point, &[(0.0, 0.0), (6.0, 6.0)], 3.0)
        && !path_contains(point, &[(0.0, 2.0), (0.0, 6.0)], 1.8);

    // Long curved ridges frame the basin; rounded mesas and smaller outcrops
    // give the outer reaches recognizable landmarks without filling the plains.
    shelter
        || ((oval(point, Vec2::new(5.1, 12.1), Vec2::new(1.2, 3.3), -0.35)
            || oval(point, Vec2::new(7.9, 15.5), Vec2::new(3.5, 1.15), 0.52))
            && !oval(point, Vec2::new(5.1, 15.8), Vec2::new(1.2, 0.85), 0.1))
        || oval(point, Vec2::new(13.7, 21.1), Vec2::new(2.6, 1.2), 0.28)
        || oval(point, Vec2::new(15.3, 21.0), Vec2::new(1.3, 0.8), -0.45)
        || oval(point, Vec2::new(2.6, 20.8), Vec2::new(1.4, 2.1), -0.35)
        || oval(point, Vec2::new(2.1, 16.2), Vec2::new(0.7, 1.0), 0.3)
        || oval(point, Vec2::new(-1.8, 12.0), Vec2::new(2.0, 3.2), -0.2)
        || oval(point, Vec2::new(7.5, 25.6), Vec2::new(3.4, 1.5), 0.1)
}

fn solid_at(point: Vec2) -> bool {
    rounded_square(point, 14.0, 0.85)
        && (!rounded_square(point, 13.65, 0.5)
            || landforms(point)
            || landforms(Vec2::splat(24.0) - point))
}

/// Solid rectangles exactly cover the authored rock silhouette. Merging adjacent
/// tiles keeps rendering and physical geometry small without changing openings.
pub fn default_rock_geometry() -> Vec<(RockFormation, Transform)> {
    let mut solid = [[false; SIDE]; SIDE];
    for (y, row) in solid.iter_mut().enumerate() {
        for (x, tile) in row.iter_mut().enumerate() {
            let point = Vec2::splat(MIN) + Vec2::new(x as f32 + 0.5, y as f32 + 0.5) * TILE;
            *tile = solid_at(point);
        }
    }
    let mut rocks = Vec::new();
    for y in 0..SIDE {
        for x in 0..SIDE {
            if !solid[y][x] {
                continue;
            }
            let width = (x..SIDE).take_while(|&xx| solid[y][xx]).count();
            let height = (y..SIDE)
                .take_while(|&yy| solid[yy][x..x + width].iter().all(|&tile| tile))
                .count();
            for row in &mut solid[y..y + height] {
                row[x..x + width].fill(false);
            }
            let size = Vec2::new(width as f32, height as f32) * TILE * ZONE_BLOCK_SIZE;
            let center = cell_origin(PLAYER_CELL)
                + (Vec2::splat(MIN) + Vec2::new(x as f32, y as f32) * TILE) * ZONE_BLOCK_SIZE
                + size / 2.0;
            rocks.push((
                RockFormation::Rectangle { half: size / 2.0 },
                Transform::from_translation(center.extend(-10.0)),
            ));
        }
    }
    rocks
}

pub fn spawn_default_terrain(commands: &mut Commands<'_, '_>) {
    for (index, (rock, transform)) in default_rock_geometry().into_iter().enumerate() {
        let mut entity = commands.spawn((rock, transform, Visibility::default()));
        if index == 0 {
            entity.insert(crate::terrain_presentation::RockSurfaceRoot);
        }
    }
}

/// The surface and its exposed edge bands share the physical terrain mask.
pub(crate) fn rock_surface_mesh(origin: Vec2) -> Mesh {
    use bevy::{asset::RenderAssetUsages, mesh::PrimitiveTopology};
    let mut positions = Vec::<[f32; 3]>::new();
    let mut colors = Vec::<[f32; 4]>::new();
    let face = [0.12, 0.16, 0.21, 1.0];
    let mut quad = |points: [Vec2; 4], shades: [[f32; 4]; 4], z: f32| {
        for index in [0, 1, 2, 0, 2, 3] {
            positions.push((points[index] - origin).extend(z).to_array());
            colors.push(shades[index]);
        }
    };
    for (rock, transform) in default_rock_geometry() {
        let RockFormation::Rectangle { half } = rock else {
            unreachable!()
        };
        let min = transform.translation.truncate() - half;
        let max = min + half * 2.0;
        quad(
            [min, Vec2::new(max.x, min.y), max, Vec2::new(min.x, max.y)],
            [face; 4],
            0.0,
        );
    }
    let is_solid = |x: i32, y: i32| {
        (0..SIDE as i32).contains(&x)
            && (0..SIDE as i32).contains(&y)
            && solid_at(Vec2::splat(MIN) + Vec2::new(x as f32 + 0.5, y as f32 + 0.5) * TILE)
    };
    for y in 0..SIDE as i32 {
        for x in 0..SIDE as i32 {
            if !is_solid(x, y) {
                continue;
            }
            let min = cell_origin(PLAYER_CELL)
                + (Vec2::splat(MIN) + Vec2::new(x as f32, y as f32) * TILE) * ZONE_BLOCK_SIZE;
            let max = min + Vec2::splat(TILE * ZONE_BLOCK_SIZE);
            for (dx, dy, a, b, normal) in [
                (-1, 0, min, Vec2::new(min.x, max.y), -Vec2::X),
                (1, 0, Vec2::new(max.x, min.y), max, Vec2::X),
                (0, -1, min, Vec2::new(max.x, min.y), -Vec2::Y),
                (0, 1, Vec2::new(min.x, max.y), max, Vec2::Y),
            ] {
                if is_solid(x + dx, y + dy) {
                    continue;
                }
                let lit = normal.dot(Vec2::new(-1.0, 1.0).normalize());
                let edge = if lit > 0.0 {
                    [0.22, 0.28, 0.35, 1.0]
                } else {
                    [0.055, 0.075, 0.10, 1.0]
                };
                let inner = -normal * 24.0;
                quad([a, b, b + inner, a + inner], [edge, edge, face, face], 0.08);
                let shadow = Vec2::new(14.0, -14.0);
                if normal.dot(shadow) > 0.0 {
                    quad(
                        [a, b, b + shadow, a + shadow],
                        [
                            [0.005, 0.008, 0.012, 0.35],
                            [0.005, 0.008, 0.012, 0.35],
                            [0.005, 0.008, 0.012, 0.0],
                            [0.005, 0.008, 0.012, 0.0],
                        ],
                        -0.08,
                    );
                }
            }
        }
    }
    let vertex_count = positions.len();
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 0.0, 1.0]; vertex_count])
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0, 0.0]; vertex_count])
    .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, colors)
}
