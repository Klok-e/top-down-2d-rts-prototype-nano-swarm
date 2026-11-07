mod values;

pub use values::*;

use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize)]
pub struct TuningField {
    pub key: &'static str,
    pub group: &'static str,
    pub label: &'static str,
    pub unit: &'static str,
    pub step: f64,
    /// Display units per native unit.
    pub scale: f64,
    pub min: f64,
    pub max: f64,
}

#[derive(Debug, Clone, Resource)]
pub struct TuningStore {
    path: PathBuf,
}

impl TuningStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn default_path() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("config/balance.ron")
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(path: impl Into<PathBuf>) -> Result<(Self, Tuning), String> {
        let store = Self::new(path);
        let contents = fs::read_to_string(&store.path)
            .map_err(|error| format!("Cannot read {}: {error}", store.path.display()))?;
        let tuning: Tuning = ron::from_str(&contents)
            .map_err(|error| format!("Invalid {}: {error}", store.path.display()))?;
        tuning.validate()?;
        Ok((store, tuning))
    }

    pub fn save(&self, tuning: &Tuning) -> Result<(), String> {
        tuning.validate()?;
        let contents = ron::ser::to_string_pretty(tuning, ron::ser::PrettyConfig::default())
            .map_err(|error| format!("Cannot encode tuning: {error}"))?;
        static SEQUENCE: AtomicU64 = AtomicU64::new(0);
        let file_name = self
            .path
            .file_name()
            .ok_or("Tuning path must name a file")?;
        let parent = self
            .path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let temporary = parent.join(format!(
            ".{}.{}.{}.tmp",
            file_name.to_string_lossy(),
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let result = (|| {
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temporary)?;
            file.write_all(contents.as_bytes())?;
            file.sync_all()?;
            fs::rename(&temporary, &self.path)
        })();
        if let Err(error) = result {
            let _ = fs::remove_file(&temporary);
            return Err(format!("Cannot save {}: {error}", self.path.display()));
        }
        Ok(())
    }
}

#[derive(Debug, Default, Clone, Copy, Resource, Serialize, Deserialize)]
pub struct TuningRevision {
    pub revision: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TuningEdit {
    pub fixed_tick: u64,
    pub revision: u64,
    pub key: String,
    pub old_value: f64,
    pub value: f64,
}

#[derive(Debug, Default, Clone, Resource)]
pub struct TuningJournal {
    pub entries: Vec<TuningEdit>,
}

pub type TuningReconciler = fn(&mut World, &Tuning, &Tuning);

#[derive(Default, Resource)]
pub struct TuningReconciliation(pub Vec<TuningReconciler>);

pub fn default_path() -> PathBuf {
    TuningStore::default_path()
}

pub fn fixed_tick(world: &World) -> u64 {
    world.get_resource::<Time<Fixed>>().map_or(0, |time| {
        (time.elapsed().as_nanos() / time.timestep().as_nanos()) as u64
    })
}

pub fn set_value(world: &mut World, key: &str, value: f64) -> Result<u64, String> {
    let old = world
        .get_resource::<Tuning>()
        .ok_or("Tuning is not initialized")?
        .clone();
    let next = old.with_value(key, value)?;
    let store = world
        .get_resource::<TuningStore>()
        .ok_or("Tuning store is not initialized")?;
    let revision = world
        .get_resource::<TuningRevision>()
        .map_or(0, |revision| revision.revision);
    if old == next {
        return Ok(revision);
    }
    let revision = revision.checked_add(1).ok_or("Tuning revision exhausted")?;
    let tick = fixed_tick(world);
    store.save(&next)?;
    let old_value = old.get(key).expect("validated key");
    let value = next.get(key).expect("validated key");
    world.insert_resource(next.clone());
    world.insert_resource(TuningRevision { revision });
    world.init_resource::<TuningJournal>();
    world
        .resource_mut::<TuningJournal>()
        .entries
        .push(TuningEdit {
            fixed_tick: tick,
            revision,
            key: key.into(),
            old_value,
            value,
        });
    let callbacks = world
        .get_resource::<TuningReconciliation>()
        .map(|hooks| hooks.0.clone())
        .unwrap_or_default();
    for callback in callbacks {
        callback(world, &old, &next);
    }
    Ok(revision)
}

pub fn rescale_ticks(value: u32, old_duration: u32, new_duration: u32) -> u32 {
    if old_duration == 0 {
        return value;
    }
    (value as f64 * new_duration as f64 / old_duration as f64).round() as u32
}

impl Tuning {
    fn validate_relationships(&self) -> Result<(), String> {
        if self.production.cost_per_bot > self.production.input_capacity {
            return Err("Production input capacity must cover one nanobot".into());
        }
        if self.maintenance.needs_threshold > self.maintenance.buffer_ticks {
            return Err("Maintenance threshold must not exceed its buffer".into());
        }
        if self.charge.weakened_threshold > self.charge.low_threshold
            || self.charge.low_threshold > self.charge.max_charge
        {
            return Err("Charge thresholds must be ordered within maximum charge".into());
        }
        if self.charge.per_pulse
            <= self.charge.drain_per_tick * self.charge.pulse_interval_ticks as f32
        {
            return Err(
                "Charging pulses must replenish more charge than drains between pulses".into(),
            );
        }
        if self.charge.material_per_pulse > self.charge.charger_capacity {
            return Err("Charger capacity must cover one charging pulse".into());
        }
        if self.presentation.tactical_far_merge_zoom < self.presentation.overlay_zoom_threshold {
            return Err("Far merge zoom must not be below overlay zoom".into());
        }
        if self.presentation.combat_reaction_flash_cap > 1.0
            || self.presentation.combat_reaction_flash_per_hit > 1.0
        {
            return Err("Combat flash intensity must not exceed one".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            static SEQUENCE: AtomicU64 = AtomicU64::new(0);
            let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
                ".tmp/live-tuning-core-test-{}-{}",
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
    fn displayed_duration_and_rate_edits_preserve_native_units_and_reject_invalid_values() {
        let tuning = Tuning::default();
        let changed = tuning
            .with_value("combat.attack_interval_ticks", 0.75)
            .unwrap();
        assert_eq!(changed.combat.attack_interval_ticks, 45);
        assert_eq!(changed.get("combat.attack_interval_ticks"), Some(0.75));
        let changed = tuning.with_value("charge.drain_per_tick", 0.009).unwrap();
        assert!((changed.charge.drain_per_tick - 0.00015).abs() < 0.00000001);
        for (key, value) in [
            ("combat.attack_interval_ticks", 0.001),
            ("combat.attack_interval_ticks", 0.0),
            ("combat.attack_interval_ticks", 2_000.0),
            ("production.cost_per_bot", 81.0),
            ("charge.low_threshold", 0.1),
            ("charge.drain_per_tick", 0.1),
            ("logistics.bot_speed", f64::NAN),
            ("logistics.bot_speed", f64::INFINITY),
            ("missing.field", 5.0),
        ] {
            assert!(tuning.with_value(key, value).is_err(), "{key}: {value}");
        }
    }

    #[test]
    fn store_requires_complete_valid_configuration() {
        let directory = TestDirectory::new();
        let store = directory.store();
        assert!(TuningStore::load(store.path()).is_err());
        fs::write(store.path(), "(production: (cost_per_bot: 12))").unwrap();
        assert!(TuningStore::load(store.path()).is_err());
        fs::write(store.path(), "invalid ron").unwrap();
        assert!(TuningStore::load(store.path()).is_err());
        let mut invalid = Tuning::default();
        invalid.production.cost_per_bot = 81;
        fs::write(store.path(), ron::to_string(&invalid).unwrap()).unwrap();
        assert!(TuningStore::load(store.path()).is_err());
    }
}
