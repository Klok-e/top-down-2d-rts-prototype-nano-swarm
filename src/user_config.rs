use std::{
    fs, io,
    path::{Path, PathBuf},
};

use bevy::window::PresentMode;
use serde::Deserialize;

pub(crate) fn directory() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME")
                .filter(|value| !value.is_empty())
                .map(|home| PathBuf::from(home).join(".config"))
        })
        .map(|root| root.join("nano-swarm"))
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Settings {
    present_mode: SavedPresentMode,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum SavedPresentMode {
    // FIFO can block simulation while a Wayland compositor hides the window.
    #[default]
    Mailbox,
    Fifo,
}

impl Settings {
    fn load(path: &Path) -> Self {
        let result = fs::read(path).and_then(|bytes| {
            ron::de::from_bytes(&bytes)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
        });
        match result {
            Ok(settings) => settings,
            Err(error) => {
                if error.kind() != io::ErrorKind::NotFound {
                    eprintln!(
                        "warning: could not load settings from {}: {error}. Using defaults.",
                        path.display()
                    );
                }
                Self::default()
            }
        }
    }
}

pub(crate) fn present_mode(config_directory: Option<&Path>) -> PresentMode {
    let settings = config_directory
        .map(|directory| Settings::load(&directory.join("settings.ron")))
        .unwrap_or_default();
    match settings.present_mode {
        SavedPresentMode::Mailbox => PresentMode::Mailbox,
        SavedPresentMode::Fifo => PresentMode::Fifo,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);
            let scratch = Path::new(env!("CARGO_MANIFEST_DIR")).join(".tmp");
            fs::create_dir_all(&scratch).unwrap();
            loop {
                let directory = scratch.join(format!(
                    "settings-test-{}-{}",
                    std::process::id(),
                    NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
                ));
                match fs::create_dir(&directory) {
                    Ok(()) => return Self(directory),
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                    Err(error) => panic!("Could not create settings test directory: {error}"),
                }
            }
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn primary_window_reads_present_mode_when_created() {
        let config_directory = TestDirectory::new();
        let settings_path = config_directory.0.join("settings.ron");
        let start_window = || crate::primary_window_config(Some(&config_directory.0));

        let defaults = start_window();
        fs::write(&settings_path, "(present_mode: fifo)").unwrap();
        let first = start_window();
        fs::write(&settings_path, "(present_mode: mailbox)").unwrap();
        let second = start_window();
        fs::write(&settings_path, "(present_mode: invalid)").unwrap();
        let invalid = start_window();

        assert_eq!(defaults.present_mode, PresentMode::Mailbox);
        assert_eq!(first.present_mode, PresentMode::Fifo);
        assert_eq!(second.present_mode, PresentMode::Mailbox);
        assert_eq!(invalid.present_mode, PresentMode::Mailbox);
    }
}
