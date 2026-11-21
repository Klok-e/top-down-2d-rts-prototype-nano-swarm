#[path = "../common/combat_scenes.rs"]
mod scenes;

use scenes::{CombatMetrics, Scene, Variant, headless_scene};

const CONTACT_DEADLINE: u32 = 600;
const TICK_LIMIT: u32 = 3_600;

#[test]
fn combat_scenes_reach_contact_with_finite_transforms_and_no_new_defenders() {
    for scene in [Scene::Skirmish, Scene::Clash, Scene::Battle] {
        let mut base_alive = [0; 2];
        for variant in [Variant::Standard, Variant::Mirrored, Variant::ReversedSpawn] {
            let label = format!("{}/{}", scene.name(), variant.name());
            let mut app = headless_scene(scene, variant);
            let mut metrics = CombatMetrics::new(app.world_mut(), scene);
            // TimePlugin's first update initializes time without advancing a fixed tick.
            app.update();
            for tick in 1..=TICK_LIMIT {
                app.update();
                metrics.observe_tick(app.world(), tick, &label);
                if tick == CONTACT_DEADLINE || metrics.resolution_tick.is_some() {
                    assert!(
                        metrics
                            .contact_tick
                            .is_some_and(|contact| contact <= CONTACT_DEADLINE),
                        "{label}: no hit resolved in the first {CONTACT_DEADLINE} fixed ticks",
                    );
                }
                if metrics.resolution_tick.is_some() {
                    break;
                }
            }
            for sample in &metrics.samples {
                println!("{label} {sample:?}");
            }
            let summary = metrics.summary();
            println!("{label} {summary:?}");
            if variant == Variant::Standard {
                base_alive = summary.final_alive;
            } else {
                let difference = std::array::from_fn::<_, 2, _>(|side| {
                    summary.final_alive[side] as isize - base_alive[side] as isize
                });
                println!("{label} survivor_difference={difference:?}");
            }
        }
    }
}
