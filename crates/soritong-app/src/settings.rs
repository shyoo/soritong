//! Non-secret settings, persisted as JSON in `<config dir>/soritong/settings.json`.
//!
//! Secrets never pass through here: they live in [`crate::secrets`]. Every field has a default
//! and unknown fields are ignored, so a settings file from an older or newer build still loads.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

pub const DEFAULT_SHORTCUT: &str = "Ctrl+Alt+Space";

pub fn default_quick_phrases() -> Vec<String> {
    [
        "네, 맞아요",
        "잠시만요",
        "감사합니다",
        "좋은 질문이에요",
        "ㅋㅋㅋ",
        "I agree",
    ]
    .map(String::from)
    .to_vec()
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WindowGeometry {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Last-selected engine id.
    pub engine: Option<String>,
    /// Last-selected voice, per engine id, so switching engines back and forth keeps each choice.
    pub voices: BTreeMap<String, String>,
    /// Mic output device id; `None` = system default (Zoom "Computer audio" path).
    pub mic_device: Option<String>,
    pub mic_volume: f32,
    pub monitor_enabled: bool,
    pub monitor_device: Option<String>,
    pub monitor_volume: f32,
    pub speed: f32,
    pub always_on_top: bool,
    pub shortcut: String,
    pub quick_phrases: Vec<String>,
    pub window: Option<WindowGeometry>,
    /// Non-secret per-engine fields: engine id → field key → value.
    pub engines: BTreeMap<String, BTreeMap<String, String>>,
    pub wizard_completed: bool,
    pub wizard_path: Option<String>,
    pub wizard_reboot_pending: bool,
    pub audio_snapshot: Option<soritong_audio::defaults::AudioDefaultsSnapshot>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            engine: None,
            voices: BTreeMap::new(),
            mic_device: None,
            mic_volume: 1.0,
            monitor_enabled: false,
            monitor_device: None,
            monitor_volume: 0.6,
            speed: 1.0,
            always_on_top: false,
            shortcut: DEFAULT_SHORTCUT.into(),
            quick_phrases: default_quick_phrases(),
            window: None,
            engines: BTreeMap::new(),
            wizard_completed: false,
            wizard_path: None,
            wizard_reboot_pending: false,
            audio_snapshot: None,
        }
    }
}

impl Settings {
    pub fn player_config(&self) -> soritong_audio::PlayerConfig {
        soritong_audio::PlayerConfig {
            mic: self.mic_device.clone(),
            monitor: self
                .monitor_enabled
                .then(|| self.monitor_device.clone())
                .flatten(),
            mic_volume: self.mic_volume.clamp(0.0, 2.0),
            monitor_volume: self.monitor_volume.clamp(0.0, 2.0),
        }
    }
}

/// `SORITONG_SETTINGS` overrides the location (for trying the app without touching your own
/// settings).
pub fn default_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("SORITONG_SETTINGS") {
        return Some(PathBuf::from(p));
    }
    dirs::config_dir().map(|d| d.join("soritong").join("settings.json"))
}

/// The settings plus where they live. `path: None` keeps them in memory only.
#[derive(Debug)]
pub struct SettingsStore {
    path: Option<PathBuf>,
    pub settings: Settings,
}

impl SettingsStore {
    /// Loads from `path`. A missing file gives defaults; a corrupt one is set aside as
    /// `settings.json.bad` so the user's quick phrases are not silently overwritten.
    pub fn load(path: Option<PathBuf>) -> Self {
        let settings = match &path {
            Some(p) => match std::fs::read_to_string(p) {
                Ok(s) => serde_json::from_str(&s).unwrap_or_else(|e| {
                    eprintln!("soritong: settings unreadable ({e}); starting from defaults");
                    let _ = std::fs::rename(p, p.with_extension("json.bad"));
                    Settings::default()
                }),
                Err(_) => Settings::default(),
            },
            None => Settings::default(),
        };
        Self { path, settings }
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Writes to a temp file and renames, so a crash mid-write never truncates the settings.
    pub fn save(&self) -> io::Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        let json = serde_json::to_string_pretty(&self.settings).map_err(io::Error::other)?;
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, path)
    }

    /// Applies `f` and saves. Returns the save error, if any; the in-memory change stays.
    pub fn update(&mut self, f: impl FnOnce(&mut Settings)) -> io::Result<()> {
        f(&mut self.settings);
        self.save()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("soritong").join("settings.json");

        let mut store = SettingsStore::load(Some(path.clone()));
        assert_eq!(store.settings, Settings::default());
        store
            .update(|s| {
                s.engine = Some("mock".into());
                s.voices.insert("mock".into(), "beep".into());
                s.mic_device = Some("cable".into());
                s.always_on_top = true;
                s.quick_phrases = vec!["안녕하세요".into()];
                s.window = Some(WindowGeometry {
                    x: -1200,
                    y: 40,
                    width: 420,
                    height: 560,
                });
                s.engines
                    .entry("mock".into())
                    .or_default()
                    .insert("model".into(), "m1".into());
            })
            .unwrap();

        let again = SettingsStore::load(Some(path.clone()));
        assert_eq!(again.settings, store.settings);
        assert!(!path.with_extension("json.tmp").exists());
    }

    #[test]
    fn partial_and_corrupt_files_fall_back_to_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");

        std::fs::write(&path, r#"{"always_on_top": true, "future_field": 1}"#).unwrap();
        let s = SettingsStore::load(Some(path.clone())).settings;
        assert!(s.always_on_top);
        assert_eq!(s.shortcut, DEFAULT_SHORTCUT);
        assert_eq!(s.quick_phrases, default_quick_phrases());

        std::fs::write(&path, "{ not json").unwrap();
        assert_eq!(
            SettingsStore::load(Some(path.clone())).settings,
            Settings::default()
        );
        assert!(path.with_extension("json.bad").exists());
    }

    #[test]
    fn monitor_device_only_reaches_the_player_when_enabled() {
        let mut s = Settings {
            monitor_device: Some("headphones".into()),
            ..Settings::default()
        };
        assert_eq!(s.player_config().monitor, None);
        s.monitor_enabled = true;
        assert_eq!(s.player_config().monitor.as_deref(), Some("headphones"));
    }
}
