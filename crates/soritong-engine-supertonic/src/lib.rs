//! Supertonic 3 (Supertone): soritong's free, offline default engine.
//!
//! A ~99 M-parameter flow-matching TTS in four ONNX graphs. It runs faster than real time on a CPU,
//! speaks 31 languages including Korean and English, and outputs mono 44.1 kHz. This crate ports
//! upstream's `rust/` example (github.com/supertone-inc/supertonic, MIT, archived) onto
//! [`soritong_core::TtsEngine`]:
//!
//! 1. text → NFKD jamo/code points → ids ([`text`]);
//! 2. `duration_predictor` → utterance length (divided by speed);
//! 3. `text_encoder` → text embedding, conditioned on the voice style;
//! 4. `vector_estimator` run `denoise_steps` times on Gaussian noise (`cancel` is checked between steps);
//! 5. `vocoder` → waveform.
//!
//! # Model
//!
//! [`ModelStore::ensure`] downloads [`store::TOTAL_BYTES`] (≈ 401 MB) from Hugging Face
//! `supertone-oss-archive/supertonic-3` at the pinned commit [`store::REVISION`] into
//! `%LOCALAPPDATA%\soritong\models\supertonic-3`, checking each file's sha256. The weights are
//! licensed under the **BigScience OpenRAIL-M** licence (© Supertone Inc.), and its use-based
//! restrictions bind anyone using soritong with this engine. The licence text is downloaded next
//! to the weights (`LICENSE`), and the app must show that attribution. Weights are never committed
//! or bundled.
//!
//! # ONNX Runtime
//!
//! Through the `ort` crate (2.0.0-rc.15, ONNX Runtime 1.31) with its default `download-binaries`
//! feature. At build time it fetches pyke's prebuilt ONNX Runtime and links it **statically**, so
//! there is no `onnxruntime.dll` and nothing to install by hand. The first build needs network
//! access. Inference runs on the CPU execution provider. For the installer:
//!
//! - **`DirectML.dll`**: the Windows prebuilt includes the DirectML provider, so the exe imports
//!   `DirectML.dll` at load time even though soritong never uses it. `copy-dylibs` drops a copy in
//!   `target/<profile>/`; ship it next to the exe. Windows 10 1903+ and 11 also have an older one in
//!   System32, which is enough for CPU inference (checked by removing the copy).
//! - **`MSVCP140.dll`**: ONNX Runtime is C++, so the exe also needs the VC++ 2015–2022
//!   redistributable, not just `VCRUNTIME140.dll`. Bundle `vc_redist.x64.exe` or check for it.

pub mod store;
pub mod text;

pub use store::{ModelPaths, ModelStore};

use ort::session::Session;
use ort::session::builder::GraphOptimizationLevel;
use ort::value::Tensor;
use rand_distr::{Distribution, StandardNormal};
use serde::Deserialize;
use soritong_core::text::detect_lang;
use soritong_core::{
    Cancel, Lang, PcmChunk, Result, SynthRequest, TtsEngine, TtsError, Voice, VoiceKind,
};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

#[derive(Debug, Clone, PartialEq)]
pub struct SupertonicOptions {
    /// ONNX Runtime intra-op threads. `None` = ONNX Runtime's default (one per physical core).
    pub threads: Option<usize>,
    /// Flow-matching steps. More is cleaner but slower, roughly linearly; upstream's default is 8.
    pub denoise_steps: usize,
    /// Pause inserted between the chunks of a long input, in seconds (upstream: 0.3).
    pub chunk_silence_secs: f32,
}

impl Default for SupertonicOptions {
    fn default() -> Self {
        Self {
            threads: None,
            denoise_steps: 8,
            chunk_silence_secs: 0.3,
        }
    }
}

/// Upstream's default `--speed`, which is what `SynthRequest::speed == 1.0` maps to.
const BASE_SPEED: f32 = 1.05;

#[derive(Deserialize)]
struct Config {
    ae: AeConfig,
    ttl: TtlConfig,
}

#[derive(Deserialize)]
struct AeConfig {
    sample_rate: u32,
    base_chunk_size: usize,
}

#[derive(Deserialize)]
struct TtlConfig {
    chunk_compress_factor: usize,
    latent_dim: usize,
}

#[derive(Deserialize)]
struct StyleFile {
    style_ttl: StyleTensor,
    style_dp: StyleTensor,
}

#[derive(Deserialize)]
struct StyleTensor {
    data: Vec<Vec<Vec<f32>>>,
    dims: Vec<usize>,
}

impl StyleTensor {
    fn into_parts(self) -> ([usize; 3], Vec<f32>) {
        let dims = [
            self.dims.first().copied().unwrap_or(1),
            self.dims.get(1).copied().unwrap_or(0),
            self.dims.get(2).copied().unwrap_or(0),
        ];
        (dims, self.data.into_iter().flatten().flatten().collect())
    }
}

struct Style {
    ttl: ([usize; 3], Vec<f32>),
    dp: ([usize; 3], Vec<f32>),
}

struct Sessions {
    duration: Session,
    text_encoder: Session,
    vector_estimator: Session,
    vocoder: Session,
}

pub struct SupertonicEngine {
    opts: SupertonicOptions,
    config: Config,
    indexer: Vec<i64>,
    styles: BTreeMap<String, Style>,
    /// `Session::run` needs `&mut`; one synthesis at a time is what soritong does anyway.
    sessions: Mutex<Sessions>,
}

fn engine_err(context: &str) -> impl Fn(ort::Error) -> TtsError + '_ {
    move |e| TtsError::Engine(format!("supertonic {context}: {e}"))
}

/// ONNX Runtime allows one environment per process and only holds it weakly, so it lives here.
fn environment() -> Result<&'static ort::environment::Environment> {
    static ENV: OnceLock<std::result::Result<ort::environment::Environment, String>> =
        OnceLock::new();
    ENV.get_or_init(|| {
        ort::init()
            .with_name("soritong")
            .build()
            .map_err(|e| e.to_string())
    })
    .as_ref()
    .map_err(|e| TtsError::Engine(format!("onnx runtime: {e}")))
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T> {
    let file = std::fs::File::open(path).map_err(|e| {
        TtsError::Config(format!(
            "supertonic model file {} missing ({e}); download the model first",
            path.display()
        ))
    })?;
    serde_json::from_reader(std::io::BufReader::new(file))
        .map_err(|e| TtsError::Engine(format!("{}: {e}", path.display())))
}

impl SupertonicEngine {
    /// Loads the four ONNX sessions and all voice styles. **Slow** (a few seconds, ~400 MB read
    /// and graph optimisation): do it once, on a worker thread, and keep the engine.
    pub fn new(paths: ModelPaths, opts: SupertonicOptions) -> Result<Self> {
        let config: Config = read_json(&paths.onnx("tts.json"))?;
        let indexer: Vec<i64> = read_json(&paths.onnx("unicode_indexer.json"))?;
        let mut styles = BTreeMap::new();
        for &voice in store::VOICES {
            let f: StyleFile = read_json(&paths.voice_style(voice))?;
            styles.insert(
                voice.to_string(),
                Style {
                    ttl: f.style_ttl.into_parts(),
                    dp: f.style_dp.into_parts(),
                },
            );
        }
        let env = environment()?;
        let load = |name: &str| -> Result<Session> {
            let err = engine_err(name);
            let mut b = Session::builder(env)
                .map_err(&err)?
                .with_optimization_level(GraphOptimizationLevel::Level3)
                .map_err(|e| err(e.into()))?;
            if let Some(n) = opts.threads {
                b = b.with_intra_threads(n).map_err(|e| err(e.into()))?;
            }
            b.commit_from_file(paths.onnx(name)).map_err(&err)
        };
        let sessions = Sessions {
            duration: load("duration_predictor.onnx")?,
            text_encoder: load("text_encoder.onnx")?,
            vector_estimator: load("vector_estimator.onnx")?,
            vocoder: load("vocoder.onnx")?,
        };
        Ok(Self {
            opts,
            config,
            indexer,
            styles,
            sessions: Mutex::new(sessions),
        })
    }

    pub fn sample_rate(&self) -> u32 {
        self.config.ae.sample_rate
    }

    /// One chunk of text → samples.
    fn infer(
        &self,
        text: &str,
        lang: &str,
        style: &Style,
        speed: f32,
        cancel: &Cancel,
    ) -> Result<Vec<f32>> {
        let ids = text::to_ids(&text::preprocess(text, lang), &self.indexer);
        let n = ids.len();
        let text_ids = Tensor::from_array(([1, n], ids)).map_err(engine_err("text ids"))?;
        let text_mask = || Tensor::from_array(([1, 1, n], vec![1.0f32; n]));
        let style_ttl = Tensor::from_array(style.ttl.clone()).map_err(engine_err("style"))?;
        let style_dp = Tensor::from_array(style.dp.clone()).map_err(engine_err("style"))?;
        let mut s = self.sessions.lock().unwrap_or_else(|e| e.into_inner());

        let out = s
            .duration
            .run(ort::inputs! {
                "text_ids" => &text_ids,
                "style_dp" => style_dp,
                "text_mask" => text_mask().map_err(engine_err("mask"))?,
            })
            .map_err(engine_err("duration"))?;
        let (_, d) = out["duration"]
            .try_extract_tensor::<f32>()
            .map_err(engine_err("duration"))?;
        let secs = d.first().copied().unwrap_or(0.0).max(0.0) / speed;
        drop(out);

        let out = s
            .text_encoder
            .run(ort::inputs! {
                "text_ids" => &text_ids,
                "style_ttl" => &style_ttl,
                "text_mask" => text_mask().map_err(engine_err("mask"))?,
            })
            .map_err(engine_err("text encoder"))?;
        let (shape, emb) = out["text_emb"]
            .try_extract_tensor::<f32>()
            .map_err(engine_err("text encoder"))?;
        let emb_shape: Vec<usize> = shape.iter().map(|&d| d as usize).collect();
        let text_emb =
            Tensor::from_array((emb_shape, emb.to_vec())).map_err(engine_err("text emb"))?;
        drop(out);

        let sr = self.config.ae.sample_rate as f32;
        let wav_len = (secs * sr) as usize;
        let chunk = self.config.ae.base_chunk_size * self.config.ttl.chunk_compress_factor;
        let latent_len = wav_len.div_ceil(chunk).max(1);
        let latent_dim = self.config.ttl.latent_dim * self.config.ttl.chunk_compress_factor;
        let mut rng = rand::rng();
        let mut xt: Vec<f32> = (0..latent_dim * latent_len)
            .map(|_| StandardNormal.sample(&mut rng))
            .collect();
        let latent_shape = [1, latent_dim, latent_len];
        let latent_mask = Tensor::from_array(([1, 1, latent_len], vec![1.0f32; latent_len]))
            .map_err(engine_err("latent mask"))?;
        let total = self.opts.denoise_steps.max(1);
        for step in 0..total {
            if cancel.is_cancelled() {
                return Err(TtsError::Cancelled);
            }
            let out = s
                .vector_estimator
                .run(ort::inputs! {
                    "noisy_latent" => Tensor::from_array((latent_shape, xt)).map_err(engine_err("latent"))?,
                    "text_emb" => &text_emb,
                    "style_ttl" => &style_ttl,
                    "latent_mask" => &latent_mask,
                    "text_mask" => text_mask().map_err(engine_err("mask"))?,
                    "current_step" => Tensor::from_array(([1], vec![step as f32])).map_err(engine_err("step"))?,
                    "total_step" => Tensor::from_array(([1], vec![total as f32])).map_err(engine_err("step"))?,
                })
                .map_err(engine_err("vector estimator"))?;
            let (_, denoised) = out["denoised_latent"]
                .try_extract_tensor::<f32>()
                .map_err(engine_err("vector estimator"))?;
            xt = denoised.to_vec();
        }
        if cancel.is_cancelled() {
            return Err(TtsError::Cancelled);
        }

        let out = s
            .vocoder
            .run(ort::inputs! {
                "latent" => Tensor::from_array((latent_shape, xt)).map_err(engine_err("latent"))?,
            })
            .map_err(engine_err("vocoder"))?;
        let (_, wav) = out["wav_tts"]
            .try_extract_tensor::<f32>()
            .map_err(engine_err("vocoder"))?;
        Ok(wav[..wav_len.min(wav.len())].to_vec())
    }
}

impl TtsEngine for SupertonicEngine {
    fn id(&self) -> &'static str {
        "supertonic"
    }

    fn display_name(&self) -> &str {
        "Supertonic 3 (offline)"
    }

    fn is_offline(&self) -> bool {
        true
    }

    fn voices(&self) -> Result<Vec<Voice>> {
        Ok(self
            .styles
            .keys()
            .map(|id| Voice {
                id: id.clone(),
                name: id.clone(),
                langs: vec![Lang::Ko, Lang::En],
                kind: VoiceKind::Stock,
            })
            .collect())
    }

    fn settings_fingerprint(&self) -> String {
        format!(
            "supertonic-3@{};steps={};gap={:.2}",
            store::REVISION,
            self.opts.denoise_steps,
            self.opts.chunk_silence_secs
        )
    }

    fn synthesize(
        &self,
        req: &SynthRequest,
        cancel: &Cancel,
        on_chunk: &mut dyn FnMut(PcmChunk),
    ) -> Result<()> {
        let style = self.styles.get(&req.voice_id).ok_or_else(|| {
            TtsError::Config(format!("unknown supertonic voice {:?}", req.voice_id))
        })?;
        let lang = match req.lang {
            Lang::Auto => detect_lang(&req.text),
            l => l,
        }
        .code();
        // Upstream recommends 0.9–1.5; outside roughly that range the model garbles.
        let speed = (BASE_SPEED * req.speed).clamp(0.7, 2.0);
        let sr = self.sample_rate();
        for (i, chunk) in text::chunk_text(&req.text, text::max_chunk_len(lang))
            .iter()
            .enumerate()
        {
            if cancel.is_cancelled() {
                return Err(TtsError::Cancelled);
            }
            let mut samples = if i == 0 {
                Vec::new()
            } else {
                vec![0.0; (self.opts.chunk_silence_secs.max(0.0) * sr as f32) as usize]
            };
            samples.extend(self.infer(chunk, lang, style, speed, cancel)?);
            on_chunk(PcmChunk {
                sample_rate: sr,
                channels: 1,
                samples,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Downloads the model (≈ 401 MB, once) and synthesises Korean and English.
    /// `cargo test --release -p soritong-engine-supertonic -- --ignored --nocapture`
    #[test]
    #[ignore = "downloads the 401 MB model and runs it"]
    fn speaks_korean_and_english() {
        let paths = ModelStore::default().ensure(&mut |_, _| {}).unwrap();
        let engine = SupertonicEngine::new(paths, SupertonicOptions::default()).unwrap();
        assert_eq!(engine.voices().unwrap().len(), 10);
        for (text, lang) in [
            ("오늘 책 정말 재밌었어요.", Lang::Auto),
            ("I agree with that point.", Lang::En),
        ] {
            let req = SynthRequest {
                text: text.into(),
                voice_id: "F1".into(),
                lang,
                speed: 1.0,
            };
            let mut pcm = Vec::new();
            engine
                .synthesize(&req, &Cancel::new(), &mut |c| {
                    assert_eq!((c.sample_rate, c.channels), (44_100, 1));
                    pcm.extend(c.samples)
                })
                .unwrap();
            let secs = pcm.len() as f32 / 44_100.0;
            let rms = (pcm.iter().map(|s| s * s).sum::<f32>() / pcm.len() as f32).sqrt();
            println!("{text:?}: {secs:.2} s, rms {rms:.3}");
            assert!((0.8..6.0).contains(&secs), "{secs}");
            assert!(rms > 0.01 && pcm.iter().all(|s| s.is_finite()));
        }

        let cancel = Cancel::new();
        cancel.cancel();
        let req = SynthRequest {
            text: "취소".into(),
            voice_id: "M1".into(),
            lang: Lang::Ko,
            speed: 1.0,
        };
        assert!(matches!(
            engine.synthesize(&req, &cancel, &mut |_| {}),
            Err(TtsError::Cancelled)
        ));
        let bad = SynthRequest {
            voice_id: "X9".into(),
            ..req
        };
        assert!(matches!(
            engine.synthesize(&bad, &Cancel::new(), &mut |_| {}),
            Err(TtsError::Config(_))
        ));
    }
}
