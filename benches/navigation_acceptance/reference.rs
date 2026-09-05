//! Verification-only flat A*: shares physical clearance, never production connectivity or search.
use super::*;
use std::{
    cmp::Ordering,
    collections::{BinaryHeap, HashMap},
};
use top_down_2d_rts_prototype_nano_swarm::{
    intent::IntentKind,
    nanobot::{SwarmId, world_to_cell},
    navigation::{RouteGoal, RoutePriority, RouteStatus},
};
#[derive(Clone, Copy, PartialEq)]
struct Node {
    score: f32,
    cost: f32,
    cell: IVec2,
}
impl Eq for Node {}
impl Ord for Node {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .score
            .total_cmp(&self.score)
            .then_with(|| other.cost.total_cmp(&self.cost))
            .then_with(|| (self.cell.y, self.cell.x).cmp(&(other.cell.y, other.cell.x)))
    }
}
impl PartialOrd for Node {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
fn center(cell: IVec2) -> Vec2 {
    (cell.as_vec2() + Vec2::splat(0.5)) * CELL_WIDTH
}
fn weight(grid: &IntentGrid, cell: IVec2, hauler: bool) -> f32 {
    if hauler
        && grid
            .cell(world_to_cell(center(cell)))
            .is_some_and(|c| c.has_owned(IntentKind::Corridor, SwarmId::PLAYER))
    {
        0.35
    } else {
        1.0
    }
}
fn direct_cost(navigation: &Navigation, grid: &IntentGrid, a: Vec2, b: Vec2, hauler: bool) -> f32 {
    if !navigation.segment_clear(a, b) {
        return f32::INFINITY;
    }
    if !hauler {
        return a.distance(b);
    }
    let mut cuts = vec![0.0f32, 1.0];
    for axis in 0..2 {
        let delta = b[axis] - a[axis];
        if delta == 0.0 {
            continue;
        }
        for boundary in (a[axis].min(b[axis]) / CELL_WIDTH).floor() as i32
            ..=(a[axis].max(b[axis]) / CELL_WIDTH).ceil() as i32
        {
            let t = (boundary as f32 * CELL_WIDTH - a[axis]) / delta;
            if t > 0.0 && t < 1.0 {
                cuts.push(t);
            }
        }
    }
    cuts.sort_by(f32::total_cmp);
    a.distance(b)
        * cuts
            .windows(2)
            .map(|pair| {
                let midpoint = a.lerp(b, (pair[0] + pair[1]) * 0.5);
                let multiplier = weight(grid, (midpoint / CELL_WIDTH).floor().as_ivec2(), hauler);
                (pair[1] - pair[0]) * multiplier
            })
            .sum::<f32>()
}
fn flat(
    navigation: &Navigation,
    grid: &IntentGrid,
    start: IVec2,
    end: IVec2,
    hauler: bool,
) -> (f32, usize) {
    let scale = if hauler { 0.35 } else { 1.0 };
    let direct = direct_cost(navigation, grid, center(start), center(end), hauler);
    let mut queue = BinaryHeap::from([Node {
        score: center(start).distance(center(end)) * scale,
        cost: 0.0,
        cell: start,
    }]);
    let mut costs = HashMap::from([(start, 0.0)]);
    let mut expansions = 0;
    while let Some(Node { cost, cell, score }) = queue.pop() {
        if score >= direct {
            return (direct, expansions);
        }
        if cost > costs[&cell] {
            continue;
        }
        expansions += 1;
        if cell == end {
            return (cost, expansions);
        }
        for y in -1..=1 {
            for x in -1..=1 {
                if x == 0 && y == 0 {
                    continue;
                }
                let next = cell + IVec2::new(x, y);
                if !navigation.segment_clear(center(cell), center(next)) {
                    continue;
                }
                let candidate = cost
                    + center(cell).distance(center(next))
                        * (weight(grid, cell, hauler) + weight(grid, next, hauler))
                        * 0.5;
                if candidate < *costs.get(&next).unwrap_or(&f32::INFINITY) {
                    costs.insert(next, candidate);
                    queue.push(Node {
                        score: candidate + center(next).distance(center(end)) * scale,
                        cost: candidate,
                        cell: next,
                    });
                }
            }
        }
    }
    panic!("reference fixture must be connected")
}
fn check_literal_oracles() {
    let mut grid = IntentGrid::new(4, 4);
    let navigation = Navigation::new(
        &grid,
        vec![Obstacle::Rectangle {
            center: Vec2::splat(36.0),
            half: Vec2::splat(36.0),
        }],
    );
    let (cost, _) = flat(
        &navigation,
        &grid,
        IVec2::new(-2, 0),
        IVec2::new(2, 0),
        false,
    );
    assert!(
        (cost - 347.64676).abs() < 0.01,
        "literal one-cell obstacle detour: {cost}"
    );
    let open = Navigation::new(&grid, vec![]);
    assert!((flat(&open, &grid, IVec2::ONE, IVec2::new(3, 1), false).0 - 144.0).abs() < 0.01);
    grid.paint(IVec2::ZERO, IntentKind::Corridor, SwarmId::PLAYER);
    assert!((flat(&open, &grid, IVec2::ONE, IVec2::new(3, 1), true).0 - 50.4).abs() < 0.01);
    assert!((flat(&open, &grid, IVec2::ONE, IVec2::new(3, 1), false).0 - 144.0).abs() < 0.01);
    // Fine-cell centers switch paint at x=504: 468 painted units and 252 ordinary units.
    let crossing = direct_cost(
        &open,
        &grid,
        Vec2::new(36.0, 36.0),
        Vec2::new(756.0, 36.0),
        true,
    );
    assert!(
        (crossing - 415.8).abs() < 0.01,
        "fine-cell paint boundary cost: {crossing}"
    );
}
pub fn run() -> serde_json::Value {
    check_literal_oracles();
    let mut results = vec![];
    for name in [
        "open",
        "obstacle_dense",
        "bottleneck",
        "simultaneous_replanning",
    ] {
        for hauler in [false, true] {
            let mut grid = IntentGrid::new(MAP, MAP);
            for x in -16..20 {
                grid.paint(IVec2::new(x, 0), IntentKind::Corridor, SwarmId::PLAYER);
            }
            let navigation = Navigation::new(&grid, obstacles(name));
            let start = IVec2::new(-90, -20);
            let end = IVec2::new(120, 20);
            let now = Instant::now();
            let (flat_cost, flat_expansions) = flat(&navigation, &grid, start, end, hauler);
            let flat_ms = now.elapsed().as_secs_f64() * 1000.0;
            for cache in ["cold", "warm"] {
                let id = navigation.request(
                    center(start),
                    RouteGoal::Point(center(end)),
                    SwarmId::PLAYER,
                    hauler,
                    RoutePriority::Routine,
                );
                let mut work_units = 0;
                let mut hierarchy_cells = 0;
                let mut hierarchy_chunks = 0;
                let mut coarse = 0;
                let mut fine = 0;
                let now = Instant::now();
                let mut found = None;
                for _ in 0..10_000 {
                    let work = navigation.advance(&grid, 32768);
                    work_units += work.work;
                    hierarchy_cells += work.hierarchy_cells;
                    hierarchy_chunks += work.hierarchy_chunks;
                    coarse += work.coarse_expansions;
                    fine += work.fine_expansions;
                    match navigation.poll(id) {
                        RouteStatus::Found(route) => {
                            found = Some(route);
                            break;
                        }
                        RouteStatus::Unreachable => {
                            panic!("hierarchy lost reference connectivity for {name}")
                        }
                        RouteStatus::Pending => {}
                    }
                }
                let route = found.unwrap_or_else(|| {
                    panic!("route did not finish after 10000 advances: {name}, hauler={hauler}, cache={cache}")
                });
                let elapsed = now.elapsed().as_secs_f64() * 1000.0;
                navigation.cancel(id);
                let mut previous = center(start);
                for &point in &route.waypoints {
                    assert!(navigation.segment_clear(previous, point));
                    previous = point;
                }
                assert_eq!(previous, center(end));
                let result = json!({"scenario":name,"hauler":hauler,"cache":cache,"start_cell":[start.x,start.y],"end_cell":[end.x,end.y],"flat_expansions":flat_expansions,"flat_cost":flat_cost,"flat_ms":flat_ms,"hierarchy_cells":hierarchy_cells,"hierarchy_chunks":hierarchy_chunks,"coarse_expansions":coarse,"fine_expansions":fine,"search_expansions":coarse+fine,"cooperative_work":work_units,"hierarchical_cost":route.cost,"cost_ratio":route.cost/flat_cost,"hierarchical_ms":elapsed});
                println!("{result}");
                results.push(result);
            }
        }
    }
    json!(results)
}
