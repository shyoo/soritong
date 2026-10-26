//! Engine-independent types shared by every soritong crate.
//!
//! An engine turns text into PCM chunks ([`TtsEngine::synthesize`]); `soritong-audio` plays them.
//! Engines are blocking by design: callers run them on a worker thread, and an engine that needs
//! async I/O owns its own runtime. This keeps the trait object-safe and the GUI free of executors.

pub mod cache;
pub mod mock;
pub mod text;

use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Interleaved f32 samples in [-1.0, 1.0]. Engines emit their native rate; the audio layer resamples.
#[derive(Debug, Clone, PartialEq)]
pub struct PcmChunk {
    pub sample_rate: u32,
    pub channels: u16,
    pub samples: Vec<f32>,
}

impl PcmChunk {
    pub fn duration_secs(&self) -> f32 {
        self.samples.len() as f32 / (self.sample_rate as f32 * self.channels.max(1) as f32)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Lang {
    Ko,
    En,
    /// Let the engine (or [`text::detect_lang`]) decide.
    Auto,
}

impl Lang {
    pub fn code(self) -> &'static str {
        match self {
            Lang::Ko => "ko",
            Lang::En => "en",
            Lang::Auto => "auto",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VoiceKind {
    /// Shipped by the engine.
    Stock,
    /// A clone of a real person's voice (e.g. an ElevenLabs instant clone).
    Cloned,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Voice {
    /// Stable id the engine understands; stored in settings.
    pub id: String,
    pub name: String,
    /// Languages the voice speaks well. Empty means "unknown / try anything".
    pub langs: Vec<Lang>,
    pub kind: VoiceKind,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SynthRequest {
    pub text: String,
    pub voice_id: String,
    pub lang: Lang,
    /// 1.0 = engine default. Engines that cannot change speed ignore it.
    pub speed: f32,
}

/// Cooperative cancellation: engines check it between chunks and return [`TtsError::Cancelled`].
#[derive(Debug, Clone, Default)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TtsError {
    /// Missing key, unknown voice, model not downloaded… something the user can fix in settings.
    #[error("configuration: {0}")]
    Config(String),
    #[error("network: {0}")]
    Network(String),
    #[error("engine: {0}")]
    Engine(String),
    #[error("cancelled")]
    Cancelled,
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub type Result<T, E = TtsError> = std::result::Result<T, E>;

pub trait TtsEngine: Send + Sync {
    /// Short stable id used in settings and cache keys, e.g. `"supertonic"`.
    fn id(&self) -> &'static str;
    fn display_name(&self) -> &str;
    /// True when no text leaves the machine. Shown in the UI.
    fn is_offline(&self) -> bool;
    fn voices(&self) -> Result<Vec<Voice>>;
    /// Anything besides the request that changes the audio (model id, stability…), folded into
    /// the cache key so a settings change never replays stale audio.
    fn settings_fingerprint(&self) -> String {
        String::new()
    }
    /// Blocking. Calls `on_chunk` as audio becomes available (once for non-streaming engines),
    /// and checks `cancel` between chunks.
    fn synthesize(
        &self,
        req: &SynthRequest,
        cancel: &Cancel,
        on_chunk: &mut dyn FnMut(PcmChunk),
    ) -> Result<()>;
}
