//! `soritong`: the headless front end, so engines and devices can be checked without the GUI.
//!
//! ```text
//! soritong devices
//! soritong say "안녕하세요" --engine mock --device "CABLE Input"
//! soritong say "Hello." --out hello.wav
//! ```

use clap::{Parser, Subcommand};
use soritong_audio::{OutputDevice, Player, PlayerConfig, SpeakEvent, Speaker};
use soritong_core::cache::{SynthCache, concat, write_wav};
use soritong_core::text::{detect_lang, split_sentences};
use soritong_core::{Cancel, Lang, SynthRequest, TtsEngine};
use soritong_core::{PcmChunk, TtsError, Voice};
use std::io::Write;
use std::path::Path;
use std::process::ExitCode;
use std::sync::{Arc, Mutex, mpsc};

struct LazyEngine {
    id: &'static str,
    display_name: &'static str,
    offline: bool,
    factory: Box<dyn Fn() -> Result<Arc<dyn TtsEngine>, String> + Send + Sync>,
    engine: Mutex<Option<Arc<dyn TtsEngine>>>,
}

impl LazyEngine {
    fn new(
        id: &'static str,
        display_name: &'static str,
        offline: bool,
        factory: impl Fn() -> Result<Arc<dyn TtsEngine>, String> + Send + Sync + 'static,
    ) -> Self {
        Self {
            id,
            display_name,
            offline,
            factory: Box::new(factory),
            engine: Mutex::new(None),
        }
    }

    fn get_or_init(&self) -> Result<Arc<dyn TtsEngine>, String> {
        let mut lock = self.engine.lock().unwrap();
        if let Some(ref e) = *lock {
            return Ok(e.clone());
        }
        let e = (self.factory)()?;
        *lock = Some(e.clone());
        Ok(e)
    }
}

impl TtsEngine for LazyEngine {
    fn id(&self) -> &'static str {
        self.id
    }
    fn display_name(&self) -> &str {
        self.display_name
    }
    fn is_offline(&self) -> bool {
        self.offline
    }
    fn settings_fingerprint(&self) -> String {
        self.get_or_init()
            .map(|e| e.settings_fingerprint())
            .unwrap_or_default()
    }
    fn voices(&self) -> soritong_core::Result<Vec<Voice>> {
        self.get_or_init().map_err(TtsError::Config)?.voices()
    }
    fn synthesize(
        &self,
        req: &SynthRequest,
        cancel: &Cancel,
        on_chunk: &mut dyn FnMut(PcmChunk),
    ) -> soritong_core::Result<()> {
        self.get_or_init()
            .map_err(TtsError::Config)?
            .synthesize(req, cancel, on_chunk)
    }
}

/// Every engine the CLI can use.
fn engines() -> Vec<Arc<dyn TtsEngine>> {
    vec![
        Arc::new(LazyEngine::new(
            "supertonic",
            "Supertonic 3 (Supertone)",
            true,
            || {
                let store = soritong_engine_supertonic::ModelStore::default();
                if !store.is_ready() {
                    eprintln!("Downloading Supertonic model (≈401 MB)...");
                    let mut last_pct = -1;
                    let mut progress = |done: u64, total: u64| {
                        let pct = if total > 0 {
                            ((done as f64 / total as f64) * 100.0) as i32
                        } else {
                            0
                        };
                        if pct != last_pct {
                            last_pct = pct;
                            let mb_done = done as f64 / 1_048_576.0;
                            let mb_total = total as f64 / 1_048_576.0;
                            eprint!(
                                "\rDownloading Supertonic model: {pct}% ({mb_done:.1} MB / {mb_total:.1} MB)..."
                            );
                            let _ = std::io::stderr().flush();
                        }
                    };
                    store
                        .ensure(&mut progress)
                        .map_err(|e| format!("model download: {e}"))?;
                    eprintln!("\nModel ready.");
                }
                let engine = soritong_engine_supertonic::SupertonicEngine::new(
                    store.paths(),
                    soritong_engine_supertonic::SupertonicOptions::default(),
                )
                .map_err(|e| e.to_string())?;
                Ok(Arc::new(engine))
            },
        )),
        Arc::new(LazyEngine::new("elevenlabs", "ElevenLabs", false, || {
            let api_key = std::env::var("ELEVENLABS_API_KEY")
                .map_err(|_| "ELEVENLABS_API_KEY environment variable is not set".to_string())?;
            if api_key.trim().is_empty() {
                return Err("ELEVENLABS_API_KEY environment variable is empty".into());
            }
            let model_id = std::env::var("ELEVENLABS_MODEL")
                .unwrap_or_else(|_| "eleven_flash_v2_5".to_string());
            let config = soritong_engine_elevenlabs::ElevenLabsConfig {
                api_key: api_key.trim().to_string(),
                model_id,
                ..Default::default()
            };
            Ok(Arc::new(soritong_engine_elevenlabs::ElevenLabsEngine::new(
                config,
            )))
        })),
        Arc::new(LazyEngine::new(
            "edge",
            "Microsoft Edge voices (online, unofficial)",
            false,
            || Ok(Arc::new(soritong_engine_edge::EdgeEngine::new())),
        )),
        Arc::new(LazyEngine::new(
            "system",
            "Windows built-in voices",
            true,
            || Ok(Arc::new(soritong_engine_system::SystemEngine::new())),
        )),
        Arc::new(LazyEngine::new(
            "openai",
            "OpenAI-compatible speech",
            false,
            || {
                let base_url = std::env::var("SORITONG_OPENAI_BASE_URL")
                    .unwrap_or_else(|_| "http://127.0.0.1:8004".to_string());
                let api_key = std::env::var("SORITONG_OPENAI_API_KEY")
                    .ok()
                    .filter(|k| !k.trim().is_empty());
                let model =
                    std::env::var("SORITONG_OPENAI_MODEL").unwrap_or_else(|_| "tts-1".to_string());
                let voices = std::env::var("SORITONG_OPENAI_VOICES")
                    .map(|v| {
                        v.split(',')
                            .map(|s| s.trim().to_string())
                            .filter(|s| !s.is_empty())
                            .collect()
                    })
                    .unwrap_or_default();
                let config = soritong_engine_openai::OpenAiConfig {
                    base_url,
                    api_key,
                    model,
                    voices,
                    response_format: "wav".to_string(),
                };
                Ok(Arc::new(soritong_engine_openai::OpenAiEngine::new(config)))
            },
        )),
        Arc::new(LazyEngine::new("mock", "Mock engine", true, || {
            Ok(Arc::new(soritong_core::mock::MockEngine::default()))
        })),
    ]
}

#[derive(Parser)]
#[command(
    name = "soritong",
    version,
    about = "Type Korean or English; speak it as your Zoom mic."
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// List output devices (and which Zoom mic each one feeds) and capture devices.
    Devices,
    /// List an engine's voices.
    Voices {
        #[arg(long, default_value = "mock")]
        engine: String,
    },
    /// Speak text on an output device, or write it to a WAV file.
    Say {
        text: String,
        #[arg(long, default_value = "mock")]
        engine: String,
        /// Voice id; defaults to the engine's first voice.
        #[arg(long)]
        voice: Option<String>,
        /// ko, en or auto (picked per sentence).
        #[arg(long, default_value = "auto", value_parser = parse_lang)]
        lang: Lang,
        #[arg(long, default_value_t = 1.0)]
        speed: f32,
        /// Mic output: a device id or part of its name. Default: the system default output.
        #[arg(long)]
        device: Option<String>,
        /// Also play on this device (id or part of its name), e.g. headphones.
        #[arg(long)]
        monitor: Option<String>,
        /// Write a WAV file instead of playing.
        #[arg(long)]
        out: Option<std::path::PathBuf>,
        /// Neither read nor write the synthesis cache.
        #[arg(long)]
        no_cache: bool,
    },
}

fn parse_lang(s: &str) -> Result<Lang, String> {
    match s {
        "ko" => Ok(Lang::Ko),
        "en" => Ok(Lang::En),
        "auto" => Ok(Lang::Auto),
        _ => Err("expected ko, en or auto".into()),
    }
}

fn main() -> ExitCode {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<(), String> {
    match cli.cmd {
        Cmd::Devices => devices(),
        Cmd::Voices { engine } => {
            for v in find_engine(&engine)?.voices().map_err(|e| e.to_string())? {
                let langs: Vec<_> = v.langs.iter().map(|l| l.code()).collect();
                println!("{}\t{}\t{:?}\t{}", v.id, v.name, v.kind, langs.join(","));
            }
            Ok(())
        }
        Cmd::Say {
            text,
            engine,
            voice,
            lang,
            speed,
            device,
            monitor,
            out,
            no_cache,
        } => {
            let engine = find_engine(&engine)?;
            let voice_id = match voice {
                Some(v) => v,
                None => {
                    engine
                        .voices()
                        .map_err(|e| e.to_string())?
                        .into_iter()
                        .next()
                        .ok_or("engine has no voices")?
                        .id
                }
            };
            let req = SynthRequest {
                text,
                voice_id,
                lang,
                speed,
            };
            if let Some(path) = out {
                return write_file(&*engine, &req, &path);
            }
            let outputs = soritong_audio::list_outputs().map_err(|e| e.to_string())?;
            let config = PlayerConfig {
                mic: device.map(|d| resolve(&outputs, &d)).transpose()?,
                monitor: monitor.map(|d| resolve(&outputs, &d)).transpose()?,
                ..PlayerConfig::default()
            };
            let cache = if no_cache { None } else { cache() };
            play(engine, req, config, cache)
        }
    }
}

fn devices() -> Result<(), String> {
    let outputs = soritong_audio::list_outputs().map_err(|e| e.to_string())?;
    println!("Output devices (* = system default):");
    for d in &outputs {
        let mark = if d.is_default { "*" } else { " " };
        println!(" {mark} {}", d.id);
        if let Some(m) = &d.virtual_mic {
            println!("      {} → in Zoom pick: {}", m.family, m.zoom_mic_name);
        }
    }
    if !outputs.iter().any(|d| d.virtual_mic.is_some()) {
        println!("   (no virtual mic found: use Zoom Share → Advanced → Computer audio,");
        println!("    or install VB-CABLE and pick \"CABLE Output\" as the mic in Zoom)");
    }
    println!("\nCapture devices (what Zoom lists as microphones):");
    for name in soritong_audio::list_inputs().map_err(|e| e.to_string())? {
        println!("   {name}");
    }
    Ok(())
}

fn find_engine(id: &str) -> Result<Arc<dyn TtsEngine>, String> {
    let all = engines();
    let ids: Vec<_> = all.iter().map(|e| e.id()).collect();
    all.iter()
        .find(|e| e.id() == id)
        .cloned()
        .ok_or_else(|| format!("unknown engine {id:?} (have: {})", ids.join(", ")))
}

/// An exact device id, else a unique case-insensitive name substring.
fn resolve(outputs: &[OutputDevice], query: &str) -> Result<String, String> {
    if let Some(d) = outputs.iter().find(|d| d.id == query) {
        return Ok(d.id.clone());
    }
    let q = query.to_lowercase();
    let hits: Vec<_> = outputs
        .iter()
        .filter(|d| d.name.to_lowercase().contains(&q))
        .collect();
    match hits.as_slice() {
        [one] => Ok(one.id.clone()),
        [] => Err(format!(
            "no output device matches {query:?}; run `soritong devices`"
        )),
        many => Err(format!(
            "{query:?} matches several devices: {}",
            many.iter()
                .map(|d| d.id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

fn cache() -> Option<SynthCache> {
    Some(SynthCache::new(
        dirs::cache_dir()?.join("soritong").join("tts"),
    ))
}

fn write_file(engine: &dyn TtsEngine, req: &SynthRequest, path: &Path) -> Result<(), String> {
    let mut chunks = Vec::new();
    for text in split_sentences(&req.text) {
        let lang = match req.lang {
            Lang::Auto => detect_lang(&text),
            l => l,
        };
        let sentence = SynthRequest {
            text,
            lang,
            ..req.clone()
        };
        engine
            .synthesize(&sentence, &Cancel::new(), &mut |c| chunks.push(c))
            .map_err(|e| e.to_string())?;
    }
    let pcm = concat(&chunks).ok_or("nothing to say")?;
    write_wav(path, &pcm).map_err(|e| e.to_string())?;
    println!(
        "wrote {} ({:.2} s, {} Hz)",
        path.display(),
        pcm.duration_secs(),
        pcm.sample_rate
    );
    Ok(())
}

fn play(
    engine: Arc<dyn TtsEngine>,
    req: SynthRequest,
    config: PlayerConfig,
    cache: Option<SynthCache>,
) -> Result<(), String> {
    let player = Arc::new(Player::new(config).map_err(|e| e.to_string())?);
    if let Some(e) = player.last_error() {
        eprintln!("warning: {e}");
    }
    let (tx, rx) = mpsc::channel();
    let speaker = Speaker::with_cache(player.clone(), cache, move |e| {
        let _ = tx.send(e);
    });
    speaker.speak(engine, req);
    let result = loop {
        match rx.recv().map_err(|_| "speaker stopped".to_string())? {
            SpeakEvent::Started { .. } => {}
            SpeakEvent::Finished { .. } => break Ok(()),
            SpeakEvent::Cancelled { .. } => break Err("cancelled".to_string()),
            SpeakEvent::Failed { message, .. } => break Err(message),
        }
    };
    if let Some(e) = player.last_error() {
        eprintln!("warning: {e}");
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev(id: &str) -> OutputDevice {
        OutputDevice {
            id: id.into(),
            name: id.split('#').next().unwrap().into(),
            is_default: false,
            virtual_mic: None,
        }
    }

    #[test]
    fn resolves_ids_and_unique_substrings() {
        let outs = [
            dev("Speakers (Realtek(R) Audio)"),
            dev("CABLE Input (VB-Audio Virtual Cable)"),
            dev("Headphones (USB)"),
            dev("Headphones (USB)#2"),
        ];
        assert_eq!(
            resolve(&outs, "cable input").unwrap(),
            "CABLE Input (VB-Audio Virtual Cable)"
        );
        assert_eq!(
            resolve(&outs, "Headphones (USB)#2").unwrap(),
            "Headphones (USB)#2"
        );
        assert!(resolve(&outs, "headphones").is_err(), "ambiguous");
        assert!(resolve(&outs, "nothing").is_err());
    }

    #[test]
    fn writes_a_wav_with_the_mock() {
        let dir = std::env::temp_dir().join(format!("soritong-cli-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let req = SynthRequest {
            text: "안녕. Hi!".into(),
            voice_id: "beep".into(),
            lang: Lang::Auto,
            speed: 1.0,
        };
        write_file(&*find_engine("mock").unwrap(), &req, &dir.join("out.wav")).unwrap();
        // SynthCache reads `<key>.wav`, so it doubles as a WAV reader here.
        let pcm = SynthCache::new(&dir).get("out").unwrap();
        assert_eq!(pcm.samples.len(), 6 * 2880); // one 120 ms tone per non-space char, 24 kHz
        std::fs::remove_dir_all(dir).ok();
    }
}
