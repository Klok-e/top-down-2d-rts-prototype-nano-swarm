use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use bevy::prelude::Resource;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scenario {
    #[default]
    Standard,
    Sandbox,
    AiBattle,
}

impl Scenario {
    pub fn label(self) -> &'static str {
        match self {
            Self::Standard => "Standard",
            Self::Sandbox => "Sandbox",
            Self::AiBattle => "AI Battle",
        }
    }
}

#[derive(Resource, Debug, Default)]
pub struct ScenarioSelection {
    pub current: Scenario,
    pub next_launch: Scenario,
    pub error: Option<String>,
    path: Option<PathBuf>,
}

impl ScenarioSelection {
    pub fn load(path: PathBuf) -> Self {
        let scenario = fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        Self {
            current: scenario,
            next_launch: scenario,
            error: None,
            path: Some(path),
        }
    }

    pub fn from_environment() -> Self {
        let config_root = std::env::var_os("XDG_CONFIG_HOME")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .or_else(|| {
                std::env::var_os("HOME")
                    .filter(|value| !value.is_empty())
                    .map(|home| PathBuf::from(home).join(".config"))
            });
        config_root
            .map(|root| Self::load(root.join("nano-swarm/scenario.json")))
            .unwrap_or_default()
    }

    pub fn select(&mut self, scenario: Scenario) {
        let result = self
            .path
            .as_deref()
            .ok_or_else(|| io::Error::other("No configuration directory is available"))
            .and_then(|path| save(path, scenario));
        match result {
            Ok(()) => {
                self.next_launch = scenario;
                self.error = None;
            }
            Err(error) => {
                log::warn!("Could not save scenario preference: {error}");
                self.error =
                    Some("Could not save scenario. Select a scenario again to retry.".into());
            }
        }
    }
}

fn save(path: &Path, scenario: Scenario) -> io::Result<()> {
    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let (temporary, mut file) = loop {
        let temporary = parent.join(format!(
            ".scenario-{}-{}.tmp",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
        {
            Ok(file) => break (temporary, file),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    };
    let result = (|| {
        serde_json::to_writer(&mut file, &scenario)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);
            loop {
                let path = std::env::temp_dir().join(format!(
                    "nano-swarm-selection-{}-{}",
                    std::process::id(),
                    NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
                ));
                match fs::create_dir(&path) {
                    Ok(()) => return Self(path),
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                    Err(error) => panic!("Cannot create test directory: {error}"),
                }
            }
        }

        fn settings(&self) -> PathBuf {
            self.0.join("config/scenario.json")
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn missing_and_invalid_selection_start_standard() {
        let directory = TestDirectory::new();
        let path = directory.settings();
        let missing = ScenarioSelection::load(path.clone());
        assert_eq!(missing.current, Scenario::Standard);
        assert_eq!(missing.next_launch, Scenario::Standard);
        assert!(!path.exists(), "Loading must not write settings");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        for invalid in ["{", "\"unknown-scenario\""] {
            fs::write(&path, invalid).unwrap();
            let loaded = ScenarioSelection::load(path.clone());
            assert_eq!(loaded.current, Scenario::Standard);
            assert_eq!(loaded.next_launch, Scenario::Standard);
        }
    }

    #[test]
    fn selection_takes_effect_only_on_next_launch_and_both_scenarios_roundtrip() {
        let directory = TestDirectory::new();
        let path = directory.settings();
        let mut running = ScenarioSelection::load(path.clone());
        running.select(Scenario::Sandbox);
        assert!(running.error.is_none());
        assert_eq!(running.current, Scenario::Standard);
        assert_eq!(running.next_launch, Scenario::Sandbox);
        let mut relaunched = ScenarioSelection::load(path.clone());
        assert_eq!(relaunched.current, Scenario::Sandbox);
        assert_eq!(relaunched.next_launch, Scenario::Sandbox);
        relaunched.select(Scenario::Standard);
        assert!(relaunched.error.is_none());
        assert_eq!(relaunched.current, Scenario::Sandbox);
        assert_eq!(relaunched.next_launch, Scenario::Standard);
        assert_eq!(ScenarioSelection::load(path).current, Scenario::Standard);
    }

    #[test]
    fn failed_save_preserves_confirmed_selection_and_existing_file_then_retry_clears_error() {
        let directory = TestDirectory::new();
        let path = directory.settings();
        let mut running = ScenarioSelection::load(path.clone());
        running.select(Scenario::Sandbox);
        assert!(running.error.is_none());
        let saved = fs::read(&path).unwrap();
        let parent = path.parent().unwrap();
        let displaced = directory.0.join("saved-config");
        fs::rename(parent, &displaced).unwrap();
        fs::write(parent, "blocks directory creation").unwrap();
        running.select(Scenario::Standard);
        assert!(running.error.is_some());
        assert_eq!(running.current, Scenario::Standard);
        assert_eq!(running.next_launch, Scenario::Sandbox);
        assert_eq!(fs::read(displaced.join("scenario.json")).unwrap(), saved);
        fs::remove_file(parent).unwrap();
        fs::rename(displaced, parent).unwrap();
        assert_eq!(ScenarioSelection::load(path).current, Scenario::Sandbox);
        running.select(Scenario::Standard);
        assert!(running.error.is_none());
        assert_eq!(running.next_launch, Scenario::Standard);
    }

    #[test]
    fn absent_persistence_path_reports_failure_without_confirming_selection() {
        let mut selection = ScenarioSelection::default();
        selection.select(Scenario::Sandbox);
        assert_eq!(selection.next_launch, Scenario::Standard);
        assert!(selection.error.is_some());
    }
}
