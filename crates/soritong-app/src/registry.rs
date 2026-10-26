//! The engines the app offers, and how each is configured.
//!
//! An engine is registered as a [`EngineDescriptor`] (what the settings pane shows) plus a
//! factory that builds the engine from its settings. Engines that need a key cannot exist
//! before the user enters one, so instances are built lazily and rebuilt when settings change.
//! Adding an engine is one `register` call in [`builtin`]; the UI renders its fields generically.

use crate::secrets::{self, SecretStore};
use crate::settings::Settings;
use serde::Serialize;
use soritong_core::{TtsEngine, TtsError};
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FieldKind {
    Text,
    /// Stored in the OS credential store, never in settings.json, never sent back to the UI.
    Secret,
    Number,
    /// One of [`SettingField::options`].
    Select,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SettingField {
    pub key: String,
    pub label: String,
    pub kind: FieldKind,
    pub default: Option<String>,
    pub help: Option<String>,
    pub options: Vec<String>,
}

impl SettingField {
    fn new(key: &str, label: &str, kind: FieldKind) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            kind,
            default: None,
            help: None,
            options: Vec::new(),
        }
    }
    pub fn text(key: &str, label: &str) -> Self {
        Self::new(key, label, FieldKind::Text)
    }
    pub fn secret(key: &str, label: &str) -> Self {
        Self::new(key, label, FieldKind::Secret)
    }
    pub fn number(key: &str, label: &str) -> Self {
        Self::new(key, label, FieldKind::Number)
    }
    pub fn select(key: &str, label: &str, options: &[&str]) -> Self {
        Self {
            options: options.iter().map(|o| o.to_string()).collect(),
            ..Self::new(key, label, FieldKind::Select)
        }
    }
    pub fn default_value(mut self, v: &str) -> Self {
        self.default = Some(v.into());
        self
    }
    pub fn help(mut self, h: &str) -> Self {
        self.help = Some(h.into());
        self
    }
    pub fn is_secret(&self) -> bool {
        self.kind == FieldKind::Secret
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EngineDescriptor {
    pub id: String,
    pub name: String,
    /// No text leaves the machine. Shown as a 🔒 badge.
    pub offline: bool,
    /// The engine fetches model weights on first use (they are never shipped or committed).
    pub needs_model_download: bool,
    pub fields: Vec<SettingField>,
}

impl EngineDescriptor {
    pub fn new(id: &str, name: &str, offline: bool) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            offline,
            needs_model_download: false,
            fields: Vec::new(),
        }
    }
    pub fn needs_model_download(mut self) -> Self {
        self.needs_model_download = true;
        self
    }
    pub fn field(mut self, f: SettingField) -> Self {
        self.fields.push(f);
        self
    }
    pub fn get_field(&self, key: &str) -> Option<&SettingField> {
        self.fields.iter().find(|f| f.key == key)
    }
}

/// What a factory receives: every field's value (secrets included), defaults filled in.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EngineConfig {
    values: BTreeMap<String, String>,
}

impl EngineConfig {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.values
            .get(key)
            .map(String::as_str)
            .filter(|v| !v.is_empty())
    }
    /// For fields the engine cannot work without; the error is shown to the user as-is.
    pub fn require(&self, key: &str) -> Result<&str, TtsError> {
        self.get(key)
            .ok_or_else(|| TtsError::Config(format!("{key} is not set (open ⚙ settings)")))
    }
}

pub type Factory = Box<dyn Fn(&EngineConfig) -> Result<Arc<dyn TtsEngine>, TtsError> + Send + Sync>;

struct Entry {
    descriptor: EngineDescriptor,
    factory: Factory,
}

#[derive(Default)]
pub struct EngineRegistry {
    entries: Vec<Entry>,
    built: Mutex<HashMap<String, Arc<dyn TtsEngine>>>,
}

/// Every engine the app ships. Real engines plug in here (see the crate README).
pub fn builtin() -> EngineRegistry {
    let mut r = EngineRegistry::default();

    // 1. Supertonic (offline, needs model download)
    let supertonic_instance: Arc<Mutex<Option<Arc<dyn TtsEngine>>>> = Arc::new(Mutex::new(None));
    let store = soritong_engine_supertonic::ModelStore::default();
    r.register(
        EngineDescriptor::new("supertonic", "Supertonic 3", true).needs_model_download(),
        Box::new(move |_cfg| {
            let mut lock = supertonic_instance.lock().unwrap();
            if let Some(ref e) = *lock {
                return Ok(e.clone());
            }
            if !store.is_ready() {
                return Err(TtsError::Config(
                    "Supertonic voice model is not downloaded yet (click Download)".into(),
                ));
            }
            let engine = Arc::new(soritong_engine_supertonic::SupertonicEngine::new(
                store.paths(),
                soritong_engine_supertonic::SupertonicOptions::default(),
            )?);
            *lock = Some(engine.clone());
            Ok(engine as Arc<dyn TtsEngine>)
        }),
    );

    // 2. ElevenLabs (cloud, needs API key)
    let model_options: Vec<&str> = soritong_engine_elevenlabs::MODELS
        .iter()
        .map(|(id, _)| *id)
        .collect();
    r.register(
        EngineDescriptor::new("elevenlabs", "ElevenLabs", false)
            .field(SettingField::secret("api_key", "API key").help("From your ElevenLabs account."))
            .field(
                SettingField::select("model_id", "Model", &model_options)
                    .default_value("eleven_flash_v2_5"),
            ),
        Box::new(|cfg| {
            let api_key = cfg.require("api_key")?.to_string();
            let model_id = cfg
                .get("model_id")
                .unwrap_or("eleven_flash_v2_5")
                .to_string();
            let config = soritong_engine_elevenlabs::ElevenLabsConfig {
                api_key,
                model_id,
                ..Default::default()
            };
            Ok(
                Arc::new(soritong_engine_elevenlabs::ElevenLabsEngine::new(config))
                    as Arc<dyn TtsEngine>,
            )
        }),
    );

    // 3. Microsoft Edge voices (online, unofficial, no settings)
    r.register_fixed(Arc::new(soritong_engine_edge::EdgeEngine::new()));

    // 4. Windows built-in voices (offline, no settings)
    r.register_fixed(Arc::new(soritong_engine_system::SystemEngine::new()));

    // 5. OpenAI-compatible (local or cloud)
    r.register(
        EngineDescriptor::new("openai", "OpenAI-compatible", false)
            .field(
                SettingField::text("base_url", "Base URL")
                    .default_value("http://127.0.0.1:8004")
                    .help("e.g. http://127.0.0.1:8004 or https://api.openai.com"),
            )
            .field(
                SettingField::secret("api_key", "API key (optional)")
                    .help("Required for api.openai.com; optional for local servers."),
            )
            .field(
                SettingField::text("model", "Model")
                    .default_value("tts-1")
                    .help("Model name (e.g. tts-1 or local model name)."),
            )
            .field(
                SettingField::text("voices", "Voices")
                    .help("Comma-separated list of voice names (optional for local/OpenAI)."),
            )
            .field(
                SettingField::select("response_format", "Format", &["wav", "pcm"])
                    .default_value("wav"),
            ),
        Box::new(|cfg| {
            let base_url = cfg
                .get("base_url")
                .unwrap_or("http://127.0.0.1:8004")
                .to_string();
            let api_key = cfg.get("api_key").map(String::from);
            let model = cfg.get("model").unwrap_or("tts-1").to_string();
            let voices = cfg
                .get("voices")
                .map(|v| {
                    v.split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect()
                })
                .unwrap_or_default();
            let response_format = cfg.get("response_format").unwrap_or("wav").to_string();
            let open_ai_cfg = soritong_engine_openai::OpenAiConfig {
                base_url,
                api_key,
                model,
                voices,
                response_format,
            };
            Ok(
                Arc::new(soritong_engine_openai::OpenAiEngine::new(open_ai_cfg))
                    as Arc<dyn TtsEngine>,
            )
        }),
    );

    // Mock engine only in debug builds
    if cfg!(debug_assertions) {
        r.register_fixed(Arc::new(soritong_core::mock::MockEngine::default()));
    }

    if cfg!(debug_assertions) && std::env::var_os("SORITONG_DEMO_ENGINES").is_some() {
        register_demo(&mut r);
    }

    r
}

/// Debug builds only, with `SORITONG_DEMO_ENGINES=1`: a fake cloud engine that exercises the
/// settings pane and the credential store before a real cloud engine exists. It speaks with
/// the mock once a key is saved.
fn register_demo(r: &mut EngineRegistry) {
    let d = EngineDescriptor::new("demo-cloud", "Demo cloud (needs key)", false)
        .needs_model_download()
        .field(
            SettingField::secret("api_key", "API key")
                .help("Any text; stored in Credential Manager."),
        )
        .field(SettingField::text("model", "Model").default_value("demo-v1"))
        .field(SettingField::select("region", "Region", &["us", "eu"]))
        .field(SettingField::number("stability", "Stability"));
    r.register(
        d,
        Box::new(|cfg| {
            cfg.require("api_key")?;
            Ok(Arc::new(soritong_core::mock::MockEngine::default()) as Arc<dyn TtsEngine>)
        }),
    );
}

impl EngineRegistry {
    pub fn register(&mut self, descriptor: EngineDescriptor, factory: Factory) {
        assert!(
            self.descriptor(&descriptor.id).is_none(),
            "engine {:?} registered twice",
            descriptor.id
        );
        self.entries.push(Entry {
            descriptor,
            factory,
        });
    }

    /// An engine with no settings, built once up front.
    pub fn register_fixed(&mut self, engine: Arc<dyn TtsEngine>) {
        let d = EngineDescriptor::new(engine.id(), engine.display_name(), engine.is_offline());
        self.register(d, Box::new(move |_| Ok(engine.clone())));
    }

    pub fn descriptors(&self) -> impl Iterator<Item = &EngineDescriptor> {
        self.entries.iter().map(|e| &e.descriptor)
    }

    pub fn descriptor(&self, id: &str) -> Option<&EngineDescriptor> {
        self.descriptors().find(|d| d.id == id)
    }

    /// The saved engine if it still exists, else the first registered one.
    pub fn resolve_id(&self, saved: Option<&str>) -> Option<&str> {
        if let Some(d) = saved.and_then(|id| self.descriptor(id)) {
            return Some(d.id.as_str());
        }
        if self.descriptor("supertonic").is_some()
            && soritong_engine_supertonic::ModelStore::default().is_ready()
        {
            return Some("supertonic");
        }
        if self.descriptor("edge").is_some() {
            return Some("edge");
        }
        self.descriptors().next().map(|d| d.id.as_str())
    }

    /// Builds (or reuses) the engine. `config` is only called when a build is needed.
    pub fn engine(
        &self,
        id: &str,
        config: impl FnOnce(&EngineDescriptor) -> Result<EngineConfig, TtsError>,
    ) -> Result<Arc<dyn TtsEngine>, TtsError> {
        if let Some(e) = self.built.lock().unwrap().get(id) {
            return Ok(e.clone());
        }
        let entry = self
            .entries
            .iter()
            .find(|e| e.descriptor.id == id)
            .ok_or_else(|| TtsError::Config(format!("unknown engine {id:?}")))?;
        let engine = (entry.factory)(&config(&entry.descriptor)?)?;
        self.built
            .lock()
            .unwrap()
            .insert(id.to_string(), engine.clone());
        Ok(engine)
    }

    /// Call after an engine's settings change, so the next use rebuilds it.
    pub fn invalidate(&self, id: &str) {
        self.built.lock().unwrap().remove(id);
    }
}

/// Collects an engine's field values from settings.json and the secret store.
pub fn config_for(
    d: &EngineDescriptor,
    settings: &Settings,
    store: &dyn SecretStore,
) -> Result<EngineConfig, TtsError> {
    let saved = settings.engines.get(&d.id);
    let mut values = BTreeMap::new();
    for f in &d.fields {
        let v = if f.is_secret() {
            store
                .get(&secrets::account(&d.id, &f.key))
                .map_err(|e| TtsError::Config(format!("credential store: {e}")))?
        } else {
            saved.and_then(|m| m.get(&f.key)).cloned()
        };
        if let Some(v) = v.or_else(|| f.default.clone()) {
            values.insert(f.key.clone(), v);
        }
    }
    Ok(EngineConfig { values })
}

/// An engine as the frontend sees it: the descriptor, current non-secret values and which
/// secrets are set (never their values).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EngineView {
    #[serde(flatten)]
    pub descriptor: EngineDescriptor,
    pub values: BTreeMap<String, String>,
    pub secrets_set: BTreeMap<String, bool>,
}

pub fn view(d: &EngineDescriptor, settings: &Settings, store: &dyn SecretStore) -> EngineView {
    let saved = settings.engines.get(&d.id);
    let mut values = BTreeMap::new();
    let mut secrets_set = BTreeMap::new();
    for f in &d.fields {
        if f.is_secret() {
            let set = matches!(store.get(&secrets::account(&d.id, &f.key)), Ok(Some(_)));
            secrets_set.insert(f.key.clone(), set);
        } else if let Some(v) = saved.and_then(|m| m.get(&f.key)) {
            values.insert(f.key.clone(), v.clone());
        }
    }
    EngineView {
        descriptor: d.clone(),
        values,
        secrets_set,
    }
}

/// A settings-pane save. `secrets`: `Some(v)` stores `v`, `Some("")` or `None` leaves it as is.
/// `clear_secrets` lists secret fields to delete.
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(default)]
pub struct EngineSettingsUpdate {
    pub values: BTreeMap<String, String>,
    pub secrets: BTreeMap<String, String>,
    pub clear_secrets: Vec<String>,
}

/// Validates an update against the descriptor and applies it. Non-secret values go into
/// `settings` (the caller saves them); secrets go straight to the store.
pub fn apply_update(
    d: &EngineDescriptor,
    update: &EngineSettingsUpdate,
    settings: &mut Settings,
    store: &dyn SecretStore,
) -> Result<(), String> {
    for (k, v) in &update.values {
        let f = d
            .get_field(k)
            .filter(|f| !f.is_secret())
            .ok_or_else(|| format!("{}: no setting {k:?}", d.id))?;
        if f.kind == FieldKind::Select && !v.is_empty() && !f.options.contains(v) {
            return Err(format!("{}: {v:?} is not an option for {k}", d.id));
        }
        if f.kind == FieldKind::Number && !v.is_empty() && v.trim().parse::<f64>().is_err() {
            return Err(format!("{}: {k} must be a number", d.id));
        }
    }
    for k in update.secrets.keys().chain(&update.clear_secrets) {
        if !d.get_field(k).is_some_and(SettingField::is_secret) {
            return Err(format!("{}: no secret {k:?}", d.id));
        }
    }

    let saved = settings.engines.entry(d.id.clone()).or_default();
    for (k, v) in &update.values {
        if v.is_empty() {
            saved.remove(k);
        } else {
            saved.insert(k.clone(), v.trim().to_string());
        }
    }
    for k in &update.clear_secrets {
        store.delete(&secrets::account(&d.id, k))?;
    }
    for (k, v) in &update.secrets {
        let v = v.trim();
        if !v.is_empty() {
            store.set(&secrets::account(&d.id, k), v)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::MemoryStore;
    use soritong_core::mock::MockEngine;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn cloud() -> EngineDescriptor {
        EngineDescriptor::new("cloud", "Cloud TTS", false)
            .field(SettingField::secret("api_key", "API key"))
            .field(SettingField::text("model", "Model").default_value("fast-v1"))
            .field(SettingField::select("region", "Region", &["us", "eu"]))
    }

    /// A registry with the mock plus a "cloud" engine that needs a key, counting its builds.
    fn registry(builds: Arc<AtomicUsize>) -> EngineRegistry {
        let mut r = EngineRegistry::default();
        r.register_fixed(Arc::new(MockEngine::default()));
        r.register(
            cloud(),
            Box::new(move |cfg| {
                cfg.require("api_key")?;
                assert_eq!(cfg.get("model"), Some("fast-v1"));
                builds.fetch_add(1, Ordering::SeqCst);
                Ok(Arc::new(MockEngine::default()) as Arc<dyn TtsEngine>)
            }),
        );
        r
    }

    #[test]
    fn builtin_registers_all_engines_in_order_and_resolves_saved_ids() {
        let r = builtin();
        let ids: Vec<_> = r.descriptors().map(|d| d.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "supertonic",
                "elevenlabs",
                "edge",
                "system",
                "openai",
                "mock"
            ]
        );
        assert!(r.descriptor("supertonic").unwrap().offline);
        assert!(r.descriptor("supertonic").unwrap().needs_model_download);
        assert_eq!(r.resolve_id(Some("edge")), Some("edge"));
        assert_eq!(r.resolve_id(Some("system")), Some("system"));
        assert_eq!(r.resolve_id(Some("openai")), Some("openai"));
        assert_eq!(r.resolve_id(Some("elevenlabs")), Some("elevenlabs"));
    }

    #[test]
    fn mock_registry_resolves_saved_ids() {
        let r = registry(Arc::default());
        let ids: Vec<_> = r.descriptors().map(|d| d.id.as_str()).collect();
        assert_eq!(ids, ["mock", "cloud"]);
        assert!(r.descriptor("mock").unwrap().offline);
        assert_eq!(r.resolve_id(Some("cloud")), Some("cloud"));
        assert_eq!(r.resolve_id(Some("gone")), Some("mock"));
        assert_eq!(r.resolve_id(None), Some("mock"));
    }

    #[test]
    #[should_panic(expected = "registered twice")]
    fn duplicate_ids_are_rejected() {
        let mut r = builtin();
        r.register_fixed(Arc::new(MockEngine::default()));
    }

    #[test]
    fn engines_build_lazily_from_settings_and_secrets_and_rebuild_after_invalidate() {
        let builds = Arc::new(AtomicUsize::new(0));
        let r = registry(builds.clone());
        let store = MemoryStore::default();
        let mut settings = Settings::default();
        let cfg = |d: &EngineDescriptor, s: &Settings| config_for(d, s, &store);

        let err = r.engine("cloud", |d| cfg(d, &settings)).err().unwrap();
        assert!(matches!(err, TtsError::Config(m) if m.contains("api_key")));

        let update = EngineSettingsUpdate {
            secrets: [("api_key".into(), " sk-1 ".into())].into(),
            values: [("region".into(), "eu".into())].into(),
            ..Default::default()
        };
        apply_update(
            r.descriptor("cloud").unwrap(),
            &update,
            &mut settings,
            &store,
        )
        .unwrap();
        assert_eq!(store.get("cloud").unwrap().as_deref(), Some("sk-1"));
        // The key must never reach settings.json.
        assert!(!serde_json::to_string(&settings).unwrap().contains("sk-1"));

        r.engine("cloud", |d| cfg(d, &settings)).unwrap();
        r.engine("cloud", |d| cfg(d, &settings)).unwrap();
        assert_eq!(builds.load(Ordering::SeqCst), 1);
        r.invalidate("cloud");
        r.engine("cloud", |d| cfg(d, &settings)).unwrap();
        assert_eq!(builds.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn view_reports_whether_secrets_are_set_but_never_their_values() {
        let store = MemoryStore::default();
        let mut settings = Settings::default();
        let d = cloud();
        let v = view(&d, &settings, &store);
        assert!(!v.secrets_set["api_key"]);

        store.set("cloud", "sk-secret").unwrap();
        settings
            .engines
            .entry("cloud".into())
            .or_default()
            .insert("region".into(), "us".into());
        let v = view(&d, &settings, &store);
        assert!(v.secrets_set["api_key"]);
        assert_eq!(v.values["region"], "us");
        let json = serde_json::to_string(&v).unwrap();
        assert!(!json.contains("sk-secret"));
        assert!(json.contains(r#""needs_model_download":false"#));
    }

    #[test]
    fn updates_are_validated_against_the_descriptor() {
        let store = MemoryStore::default();
        let mut settings = Settings::default();
        let d = cloud();
        let bad = |u: EngineSettingsUpdate, s: &mut Settings| apply_update(&d, &u, s, &store);

        // A secret sent as a plain value would end up on disk: refuse it.
        let leak = EngineSettingsUpdate {
            values: [("api_key".into(), "sk".into())].into(),
            ..Default::default()
        };
        assert!(bad(leak, &mut settings).is_err());
        let not_an_option = EngineSettingsUpdate {
            values: [("region".into(), "mars".into())].into(),
            ..Default::default()
        };
        assert!(bad(not_an_option, &mut settings).is_err());
        let not_a_secret = EngineSettingsUpdate {
            secrets: [("model".into(), "x".into())].into(),
            ..Default::default()
        };
        assert!(bad(not_a_secret, &mut settings).is_err());
        assert!(store.get("cloud").unwrap().is_none());

        store.set("cloud", "sk").unwrap();
        let clear = EngineSettingsUpdate {
            clear_secrets: vec!["api_key".into()],
            ..Default::default()
        };
        apply_update(&d, &clear, &mut settings, &store).unwrap();
        assert!(store.get("cloud").unwrap().is_none());
    }
}
