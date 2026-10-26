//! soritong desktop app: the Tauri shell around one [`Speaker`] and [`Player`].
//!
//! All logic lives here in Rust; `ui/` is a small plain-JS page that calls these commands and
//! listens for `speak-event`. Engine calls never run on the main thread: synthesis happens on
//! the Speaker's worker, and commands that may block (voices, devices) are `async`.

pub mod registry;
pub mod request;
pub mod secrets;
pub mod settings;
pub mod wizard;

use registry::{EngineRegistry, EngineSettingsUpdate, EngineView};
use secrets::{KeyringStore, SecretStore};
use serde::{Deserialize, Serialize};
use settings::{Settings, SettingsStore, WindowGeometry};
use soritong_audio::{OutputDevice, Player, SpeakEvent, Speaker};
use soritong_core::{TtsEngine, TtsError, Voice};
use std::sync::{Arc, Mutex};
use tauri::{Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewWindow, WindowEvent};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

const SPEAK_EVENT: &str = "speak-event";

struct App {
    registry: EngineRegistry,
    secrets: Box<dyn SecretStore>,
    settings: Mutex<SettingsStore>,
    player: Arc<Player>,
    speaker: Speaker,
    /// Held while queueing and while forwarding Speaker events, so the frontend always sees an
    /// utterance's `queued` before the worker's `started` for it.
    emit_lock: Arc<Mutex<()>>,
}

type CmdResult<T> = Result<T, String>;

impl App {
    fn engine(&self, id: &str) -> Result<Arc<dyn TtsEngine>, TtsError> {
        self.registry.engine(id, |d| {
            let s = self.settings.lock().unwrap();
            registry::config_for(d, &s.settings, self.secrets.as_ref())
        })
    }

    fn update(&self, f: impl FnOnce(&mut Settings)) -> CmdResult<()> {
        self.settings
            .lock()
            .unwrap()
            .update(f)
            .map_err(|e| format!("could not save settings: {e}"))
    }
}

/// Everything the page needs on load. Contains no secret values.
#[derive(Serialize)]
struct StateView {
    engines: Vec<EngineView>,
    engine: Option<String>,
    model_ready: bool,
    voices: std::collections::BTreeMap<String, String>,
    mic_device: Option<String>,
    mic_volume: f32,
    monitor_enabled: bool,
    monitor_device: Option<String>,
    monitor_volume: f32,
    always_on_top: bool,
    shortcut: String,
    quick_phrases: Vec<String>,
    settings_path: Option<String>,
    wizard_completed: bool,
}

#[tauri::command]
fn get_state(app: tauri::State<'_, App>) -> StateView {
    let store = app.settings.lock().unwrap();
    let s = &store.settings;
    let resolved_engine = app
        .registry
        .resolve_id(s.engine.as_deref())
        .map(String::from);
    let model_ready = resolved_engine.as_deref().is_none_or(|id| {
        if id == "supertonic" {
            soritong_engine_supertonic::ModelStore::default().is_ready()
        } else {
            true
        }
    });
    StateView {
        engines: app
            .registry
            .descriptors()
            .map(|d| registry::view(d, s, app.secrets.as_ref()))
            .collect(),
        engine: resolved_engine,
        model_ready,
        voices: s.voices.clone(),
        mic_device: s.mic_device.clone(),
        mic_volume: s.mic_volume,
        monitor_enabled: s.monitor_enabled,
        monitor_device: s.monitor_device.clone(),
        monitor_volume: s.monitor_volume,
        always_on_top: s.always_on_top,
        shortcut: s.shortcut.clone(),
        quick_phrases: s.quick_phrases.clone(),
        settings_path: store.path().map(|p| p.display().to_string()),
        wizard_completed: s.wizard_completed,
    }
}

#[tauri::command]
fn check_model_ready(engine: String) -> bool {
    if engine == "supertonic" {
        soritong_engine_supertonic::ModelStore::default().is_ready()
    } else {
        true
    }
}

#[derive(Clone, Serialize)]
struct DownloadProgress {
    downloaded_bytes: u64,
    total_bytes: u64,
    percent: f32,
}

#[tauri::command]
async fn download_model(handle: tauri::AppHandle, engine: String) -> CmdResult<()> {
    if engine != "supertonic" {
        return Ok(());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let store = soritong_engine_supertonic::ModelStore::default();
        let mut last_emit = std::time::Instant::now();
        let mut progress = |done: u64, total: u64| {
            if done == total || last_emit.elapsed() >= std::time::Duration::from_millis(100) {
                last_emit = std::time::Instant::now();
                let percent = if total > 0 {
                    (done as f32 / total as f32) * 100.0
                } else {
                    0.0
                };
                let _ = handle.emit(
                    "model-download-progress",
                    DownloadProgress {
                        downloaded_bytes: done,
                        total_bytes: total,
                        percent,
                    },
                );
            }
        };
        store.ensure(&mut progress).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("join error: {e}"))?
    .map(|_| ())
}

#[derive(Serialize)]
struct VoicesView {
    voices: Vec<Voice>,
    selected: Option<String>,
}

/// May hit the network (cloud engines list voices remotely), hence async.
#[tauri::command]
async fn list_voices(app: tauri::State<'_, App>, engine: String) -> CmdResult<VoicesView> {
    let e = app.engine(&engine).map_err(|e| e.to_string())?;
    let voices = e.voices().map_err(|e| e.to_string())?;
    let saved = app
        .settings
        .lock()
        .unwrap()
        .settings
        .voices
        .get(&engine)
        .cloned();
    let selected = request::pick_voice(&voices, saved.as_deref()).map(String::from);
    Ok(VoicesView { voices, selected })
}

#[tauri::command]
async fn list_outputs() -> CmdResult<Vec<OutputDevice>> {
    soritong_audio::list_outputs().map_err(|e| e.to_string())
}

#[tauri::command]
fn set_selection(
    app: tauri::State<'_, App>,
    engine: String,
    voice: Option<String>,
) -> CmdResult<()> {
    app.update(|s| {
        s.engine = Some(engine.clone());
        if let Some(v) = voice {
            s.voices.insert(engine, v);
        }
    })
}

#[tauri::command]
async fn speak(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    text: String,
    engine: String,
    voice: String,
) -> CmdResult<u64> {
    let speed = app.settings.lock().unwrap().settings.speed;
    let req = request::build_request(&text, &voice, speed)?;
    let e = app.engine(&engine).map_err(|e| e.to_string())?;
    let text = req.text.clone();
    let _order = app.emit_lock.lock().unwrap();
    let id = app.speaker.speak(e, req);
    let _ = handle.emit(
        SPEAK_EVENT,
        serde_json::json!({ "type": "queued", "id": id, "text": text, "engine": engine, "voice": voice }),
    );
    Ok(id)
}

#[tauri::command]
fn stop(app: tauri::State<'_, App>) {
    app.speaker.stop();
    app.player.stop();
}

#[derive(Deserialize)]
struct AudioUpdate {
    mic_device: Option<String>,
    mic_volume: f32,
    monitor_enabled: bool,
    monitor_device: Option<String>,
    monitor_volume: f32,
}

#[tauri::command]
fn set_audio(app: tauri::State<'_, App>, audio: AudioUpdate) -> CmdResult<()> {
    let saved = app.update(|s| {
        s.mic_device = audio.mic_device;
        s.mic_volume = audio.mic_volume;
        s.monitor_enabled = audio.monitor_enabled;
        s.monitor_device = audio.monitor_device;
        s.monitor_volume = audio.monitor_volume;
    });
    let config = app.settings.lock().unwrap().settings.player_config();
    app.player.reconfigure(config).map_err(|e| e.to_string())?;
    saved
}

#[tauri::command]
fn set_quick_phrases(app: tauri::State<'_, App>, phrases: Vec<String>) -> CmdResult<()> {
    let phrases = phrases
        .into_iter()
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect();
    app.update(|s| s.quick_phrases = phrases)
}

#[tauri::command]
fn save_engine_settings(
    app: tauri::State<'_, App>,
    engine: String,
    update: EngineSettingsUpdate,
) -> CmdResult<EngineView> {
    let d = app
        .registry
        .descriptor(&engine)
        .ok_or_else(|| format!("unknown engine {engine:?}"))?;
    let mut store = app.settings.lock().unwrap();
    registry::apply_update(d, &update, &mut store.settings, app.secrets.as_ref())?;
    store
        .save()
        .map_err(|e| format!("could not save settings: {e}"))?;
    app.registry.invalidate(&engine);
    Ok(registry::view(d, &store.settings, app.secrets.as_ref()))
}

#[tauri::command]
fn set_always_on_top(app: tauri::State<'_, App>, window: WebviewWindow, on: bool) -> CmdResult<()> {
    window.set_always_on_top(on).map_err(|e| e.to_string())?;
    app.update(|s| s.always_on_top = on)
}

#[tauri::command]
fn set_shortcut(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    shortcut: String,
) -> CmdResult<()> {
    let shortcut = shortcut.trim().to_string();
    let new: Shortcut = shortcut
        .parse()
        .map_err(|e| format!("{shortcut:?} is not a valid shortcut: {e}"))?;
    let gs = handle.global_shortcut();
    let old = app.settings.lock().unwrap().settings.shortcut.clone();
    if let Ok(old) = old.parse::<Shortcut>() {
        if old == new {
            return app.update(|s| s.shortcut = shortcut);
        }
        let _ = gs.unregister(old);
        if let Err(e) = gs.register(new) {
            let _ = gs.register(old);
            return Err(format!("could not register {shortcut}: {e}"));
        }
    } else {
        gs.register(new)
            .map_err(|e| format!("could not register {shortcut}: {e}"))?;
    }
    app.update(|s| s.shortcut = shortcut)
}

fn show_and_focus(handle: &tauri::AppHandle) {
    if let Some(w) = handle.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        let _ = w.emit("focus-input", ());
    }
}

/// Restores the saved geometry only if it is still on a connected monitor, so unplugging a
/// second screen never leaves the window off-screen.
fn restore_geometry(w: &WebviewWindow, g: WindowGeometry) {
    let on_screen = w.available_monitors().unwrap_or_default().iter().any(|m| {
        let (p, s) = (m.position(), m.size());
        let (cx, cy) = (g.x + 40, g.y + 20); // a grabbable point of the title bar
        cx >= p.x && cy >= p.y && cx < p.x + s.width as i32 && cy < p.y + s.height as i32
    });
    if on_screen {
        let _ = w.set_size(PhysicalSize::new(g.width.max(300), g.height.max(300)));
        let _ = w.set_position(PhysicalPosition::new(g.x, g.y));
    }
}

fn geometry(w: &tauri::Window) -> Option<WindowGeometry> {
    if w.is_minimized().unwrap_or(false) || w.is_maximized().unwrap_or(false) {
        return None;
    }
    let p = w.outer_position().ok()?;
    let s = w.inner_size().ok()?;
    Some(WindowGeometry {
        x: p.x,
        y: p.y,
        width: s.width,
        height: s.height,
    })
}

#[derive(Serialize)]
struct OutputDeviceItem {
    id: String,
    name: String,
}

#[derive(Serialize)]
struct WizardViewState {
    step: wizard::WizardStep,
    wizard_completed: bool,
    wizard_path: Option<String>,
    cable_installed: bool,
    zoom_mic_name: Option<String>,
    cable_input_id: Option<String>,
    defaults_are_cable: bool,
    listen_to_device_enabled: bool,
    reboot_pending: bool,
    snapshot: Option<soritong_audio::defaults::AudioDefaultsSnapshot>,
    current_defaults: Option<soritong_audio::defaults::AudioDefaultsSnapshot>,
    real_speakers: Vec<OutputDeviceItem>,
    real_mics: Vec<String>,
}

#[tauri::command]
fn wizard_get_state(app: tauri::State<'_, App>) -> Result<WizardViewState, String> {
    let store = app.settings.lock().unwrap();
    let s = &store.settings;

    let outputs = soritong_audio::list_outputs().unwrap_or_default();
    let inputs = soritong_audio::list_inputs().unwrap_or_default();

    let cable_output_device = soritong_audio::preferred_virtual_mic(&outputs);
    let cable_installed = cable_output_device.is_some();
    let zoom_mic_name = cable_output_device
        .and_then(|d| d.virtual_mic.as_ref().map(|vm| vm.zoom_mic_name.clone()))
        .or_else(|| cable_installed.then(|| "CABLE Output (VB-Audio Virtual Cable)".to_string()));
    let cable_input_id = cable_output_device.map(|d| d.id.clone());

    let current_defaults = soritong_audio::defaults::get_current_defaults().ok();
    let defaults_are_cable = current_defaults.as_ref().is_some_and(|cd| {
        cd.render_console
            .as_ref()
            .is_some_and(|x| soritong_audio::defaults::is_cable_endpoint(&x.friendly_name))
            || cd
                .capture_console
                .as_ref()
                .is_some_and(|x| soritong_audio::defaults::is_cable_endpoint(&x.friendly_name))
    });

    let listen_to_device_enabled =
        soritong_audio::defaults::is_listen_to_device_enabled("").unwrap_or(false);

    let ctx = wizard::WizardContext {
        completed: s.wizard_completed,
        path_choice: match s.wizard_path.as_deref() {
            Some("virtual_mic") => Some(wizard::PathChoice::VirtualMic),
            Some("direct_share") => Some(wizard::PathChoice::DirectShare),
            _ => None,
        },
        has_snapshot: s.audio_snapshot.is_some(),
        cable_installed,
        defaults_are_cable,
        defaults_restored: !defaults_are_cable,
        soritong_configured: s.mic_device.is_some(),
        loopback_passed: false,
        reboot_pending: s.wizard_reboot_pending,
    };

    let step = wizard::decide_wizard_step(&ctx);

    let real_speakers: Vec<_> = outputs
        .into_iter()
        .filter(|d| !soritong_audio::defaults::is_cable_endpoint(&d.name))
        .map(|d| OutputDeviceItem {
            id: d.id,
            name: d.name,
        })
        .collect();

    let real_mics: Vec<_> = inputs
        .into_iter()
        .filter(|name| !soritong_audio::defaults::is_cable_endpoint(name))
        .collect();

    Ok(WizardViewState {
        step,
        wizard_completed: s.wizard_completed,
        wizard_path: s.wizard_path.clone(),
        cable_installed,
        zoom_mic_name,
        cable_input_id,
        defaults_are_cable,
        listen_to_device_enabled,
        reboot_pending: s.wizard_reboot_pending,
        snapshot: s.audio_snapshot.clone(),
        current_defaults,
        real_speakers,
        real_mics,
    })
}

#[tauri::command]
fn wizard_choose_path(app: tauri::State<'_, App>, path: String) -> CmdResult<()> {
    let mut store = app.settings.lock().unwrap();
    store.settings.wizard_path = Some(path.clone());
    if path == "direct_share" {
        store.settings.mic_device = None;
        store.settings.wizard_completed = true;
        let _ = app.player.reconfigure(store.settings.player_config());
    }
    store.save().map_err(|e| e.to_string())
}

#[tauri::command]
fn wizard_take_snapshot(
    app: tauri::State<'_, App>,
) -> CmdResult<soritong_audio::defaults::AudioDefaultsSnapshot> {
    let mut store = app.settings.lock().unwrap();
    let snap = soritong_audio::defaults::get_current_defaults().map_err(|e| e.to_string())?;
    store.settings.audio_snapshot = Some(snap.clone());
    store.save().map_err(|e| e.to_string())?;
    Ok(snap)
}

/// Opens the VB-CABLE page in the system browser (Tauri does not follow `target="_blank"`).
/// Only ever the pinned constant, so no caller-supplied URL reaches the shell.
#[tauri::command]
fn wizard_open_vbcable_page() -> CmdResult<()> {
    std::process::Command::new("explorer")
        .arg(wizard::VBCABLE_PAGE_URL)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("could not open the browser: {e}"))
}

#[tauri::command]
async fn wizard_install_vbcable() -> CmdResult<()> {
    tauri::async_runtime::spawn_blocking(move || {
        let temp_dir = tempfile::tempdir().map_err(|e| format!("Tempdir error: {e}"))?;
        let zip_path = temp_dir.path().join("vbcable.zip");
        let extract_dir = temp_dir.path().join("extracted");
        std::fs::create_dir_all(&extract_dir).map_err(|e| format!("Create dir error: {e}"))?;

        wizard::download_and_verify_vbcable(&zip_path)?;
        wizard::extract_zip(&zip_path, &extract_dir)?;
        wizard::launch_vbcable_installer(&extract_dir)?;
        Ok(())
    })
    .await
    .map_err(|e| format!("Task error: {e}"))?
}

#[tauri::command]
fn wizard_set_reboot_pending(app: tauri::State<'_, App>, pending: bool) -> CmdResult<()> {
    let mut store = app.settings.lock().unwrap();
    store.settings.wizard_reboot_pending = pending;
    store.save().map_err(|e| e.to_string())
}

#[tauri::command]
fn wizard_restore_defaults(
    app: tauri::State<'_, App>,
    render_id: Option<String>,
    capture_id: Option<String>,
) -> CmdResult<soritong_audio::defaults::RestoreReport> {
    if let Some(ref rid) = render_id {
        soritong_audio::defaults::set_default_endpoint(
            rid,
            soritong_audio::defaults::AudioRole::All,
        )
        .map_err(|e| e.to_string())?;
    }
    if let Some(ref cid) = capture_id {
        soritong_audio::defaults::set_default_endpoint(
            cid,
            soritong_audio::defaults::AudioRole::All,
        )
        .map_err(|e| e.to_string())?;
    }

    if render_id.is_none() && capture_id.is_none() {
        let store = app.settings.lock().unwrap();
        if let Some(ref snap) = store.settings.audio_snapshot {
            return soritong_audio::defaults::restore_defaults_if_cable(snap)
                .map_err(|e| e.to_string());
        }
    }

    Ok(soritong_audio::defaults::RestoreReport {
        render_restored: render_id.is_some(),
        capture_restored: capture_id.is_some(),
        restored_devices: vec![],
    })
}

#[tauri::command]
fn wizard_configure_soritong(
    app: tauri::State<'_, App>,
    mic_device: Option<String>,
    monitor_enabled: bool,
    monitor_device: Option<String>,
    monitor_volume: f32,
) -> CmdResult<()> {
    let mut store = app.settings.lock().unwrap();
    store.settings.mic_device = mic_device;
    store.settings.monitor_enabled = monitor_enabled;
    store.settings.monitor_device = monitor_device;
    store.settings.monitor_volume = monitor_volume;
    let _ = app.player.reconfigure(store.settings.player_config());
    store.save().map_err(|e| e.to_string())
}

#[tauri::command]
async fn wizard_run_loopback() -> CmdResult<soritong_audio::loopback::LoopbackReport> {
    tauri::async_runtime::spawn_blocking(move || {
        let outputs = soritong_audio::list_outputs().map_err(|e| e.to_string())?;
        let cable_in = soritong_audio::preferred_virtual_mic(&outputs);
        let out_id = match cable_in {
            Some(d) => d.id.as_str(),
            None => {
                return Ok(soritong_audio::loopback::LoopbackReport {
                    success: false,
                    rms: 0.0,
                    peak: 0.0,
                    delay_ms: None,
                    message: "no signal: CABLE Output got silence".into(),
                });
            }
        };
        let cap_name = "CABLE Output";

        let sample_rate = 44100;
        let num_samples = (sample_rate as f32 * 0.8) as usize;
        let mut pcm = Vec::with_capacity(num_samples);
        for i in 0..num_samples {
            let t = i as f32 / sample_rate as f32;
            let envelope = if t < 0.05 {
                t / 0.05
            } else if t > 0.75 {
                (0.8 - t) / 0.05
            } else {
                1.0
            };
            pcm.push(0.7 * envelope * (2.0 * std::f32::consts::PI * 440.0 * t).sin());
        }

        soritong_audio::loopback::loopback_test(out_id, cap_name, &pcm, sample_rate)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
fn wizard_complete(app: tauri::State<'_, App>) -> CmdResult<()> {
    let mut store = app.settings.lock().unwrap();
    store.settings.wizard_completed = true;
    store.settings.wizard_reboot_pending = false;
    store.save().map_err(|e| e.to_string())
}

#[tauri::command]
fn wizard_reset(app: tauri::State<'_, App>) -> CmdResult<()> {
    let mut store = app.settings.lock().unwrap();
    store.settings.wizard_completed = false;
    store.settings.wizard_reboot_pending = false;
    store.save().map_err(|e| e.to_string())
}

pub fn run() {
    tauri::Builder::default()
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|handle, _shortcut, event| {
                    if event.state == ShortcutState::Pressed {
                        show_and_focus(handle);
                    }
                })
                .build(),
        )
        .setup(|tauri_app| {
            let store = SettingsStore::load(settings::default_path());
            let player = Arc::new(Player::new(store.settings.player_config()).or_else(|e| {
                eprintln!("soritong: audio config rejected ({e}); using defaults");
                Player::new(Default::default())
            })?);
            let emit_lock = Arc::new(Mutex::new(()));
            let (handle, lock) = (tauri_app.handle().clone(), emit_lock.clone());
            let cache_dir = dirs::cache_dir()
                .unwrap_or_else(std::env::temp_dir)
                .join("soritong")
                .join("tts");
            let cache = Some(soritong_core::cache::SynthCache::new(cache_dir));
            let speaker = Speaker::with_cache(player.clone(), cache, move |ev: SpeakEvent| {
                let _order = lock.lock().unwrap();
                let _ = handle.emit(SPEAK_EVENT, &ev);
            });

            let window = tauri_app
                .get_webview_window("main")
                .expect("main window is declared in tauri.conf.json");
            if let Some(g) = store.settings.window {
                restore_geometry(&window, g);
            }
            let _ = window.set_always_on_top(store.settings.always_on_top);
            window.show()?;

            let shortcut = store
                .settings
                .shortcut
                .parse::<Shortcut>()
                .or_else(|_| settings::DEFAULT_SHORTCUT.parse())?;
            if let Err(e) = tauri_app.global_shortcut().register(shortcut) {
                // Another app may own it; the user can pick another in ⚙.
                eprintln!("soritong: global shortcut unavailable: {e}");
            }

            tauri_app.manage(App {
                registry: registry::builtin(),
                secrets: Box::new(KeyringStore),
                settings: Mutex::new(store),
                player,
                speaker,
                emit_lock,
            });
            Ok(())
        })
        .on_window_event(|w, ev| {
            if let WindowEvent::CloseRequested { .. } = ev
                && let Some(g) = geometry(w)
            {
                let app = w.state::<App>();
                if let Err(e) = app.update(|s| s.window = Some(g)) {
                    eprintln!("soritong: {e}");
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_state,
            list_voices,
            list_outputs,
            set_selection,
            speak,
            stop,
            set_audio,
            set_quick_phrases,
            save_engine_settings,
            set_always_on_top,
            set_shortcut,
            check_model_ready,
            download_model,
            wizard_get_state,
            wizard_choose_path,
            wizard_take_snapshot,
            wizard_install_vbcable,
            wizard_open_vbcable_page,
            wizard_set_reboot_pending,
            wizard_restore_defaults,
            wizard_configure_soritong,
            wizard_run_loopback,
            wizard_complete,
            wizard_reset,
        ])
        .run(tauri::generate_context!())
        .expect("error while running soritong");
}
