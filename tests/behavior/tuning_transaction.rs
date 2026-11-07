use bevy::prelude::*;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
use top_down_2d_rts_prototype_nano_swarm::tuning::*;

struct TestDirectory(PathBuf);
impl TestDirectory {
    fn new() -> Self {
        static SEQUENCE: AtomicU64 = AtomicU64::new(0);
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
            ".tmp/live-tuning-transaction-test-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn store(&self) -> TuningStore {
        TuningStore::new(self.0.join("balance.ron"))
    }
}
impl Drop for TestDirectory {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn accepted_edit_is_persisted_before_reconciliation_and_records_its_tick() {
    #[derive(Resource)]
    struct Reconciled;
    fn reconcile(world: &mut World, old: &Tuning, next: &Tuning) {
        assert_eq!(old.production.ticks_per_bot, 600);
        assert_eq!(next.production.ticks_per_bot, 150);
        let (_, saved) = TuningStore::load(world.resource::<TuningStore>().path()).unwrap();
        assert_eq!(&saved, next);
        assert_eq!(world.resource::<Tuning>(), next);
        world.insert_resource(Reconciled);
    }
    let directory = TestDirectory::new();
    let store = directory.store();
    store.save(&Tuning::default()).unwrap();
    let mut world = World::new();
    world.insert_resource(Tuning::default());
    world.insert_resource(store);
    world.insert_resource(TuningReconciliation(vec![reconcile]));
    let mut time = Time::<Fixed>::from_hz(60.0);
    time.advance_by(time.timestep() * 17);
    world.insert_resource(time);
    assert_eq!(
        set_value(&mut world, "production.ticks_per_bot", 2.5).unwrap(),
        1
    );
    assert!(world.contains_resource::<Reconciled>());
    let entry = &world.resource::<TuningJournal>().entries[0];
    assert_eq!(entry.fixed_tick, 17);
    assert_eq!(entry.old_value, 10.0);
    assert_eq!(entry.value, 2.5);
    assert_eq!(entry.revision, 1);
    assert_eq!(
        set_value(&mut world, "production.ticks_per_bot", 2.5).unwrap(),
        1
    );
    assert_eq!(world.resource::<TuningJournal>().entries.len(), 1);
}

#[test]
fn mutation_without_a_store_is_rejected_before_live_changes() {
    let mut world = World::new();
    world.insert_resource(Tuning::default());
    world.insert_resource(TuningRevision { revision: 12 });
    world.insert_resource(TuningJournal::default());
    assert!(set_value(&mut world, "production.ticks_per_bot", 2.5).is_err());
    assert_eq!(*world.resource::<Tuning>(), Tuning::default());
    assert_eq!(world.resource::<TuningRevision>().revision, 12);
    assert!(world.resource::<TuningJournal>().entries.is_empty());
}
