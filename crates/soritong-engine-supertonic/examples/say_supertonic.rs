//! `cargo run --release -p soritong-engine-supertonic --example say_supertonic -- "오늘 책 정말 재밌었어요." out.wav [--voice F1] [--lang ko|en|auto] [--speed 1.0] [--steps 8] [--threads N]`
//!
//! Downloads the model on first use, then prints the real-time factor (synthesis time / audio length).

use soritong_core::cache::{concat, write_wav};
use soritong_core::{Cancel, Lang, SynthRequest, TtsEngine};
use soritong_engine_supertonic::{ModelStore, SupertonicEngine, SupertonicOptions};
use std::io::Write;
use std::time::Instant;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let (mut text, mut out) = (None, None);
    let (mut voice, mut lang, mut speed) = ("F1".to_string(), Lang::Auto, 1.0f32);
    let mut opts = SupertonicOptions::default();
    while let Some(a) = args.next() {
        let mut value = || args.next().ok_or(format!("{a} needs a value"));
        match a.as_str() {
            "--voice" => voice = value()?,
            "--lang" => {
                lang = match value()?.as_str() {
                    "ko" => Lang::Ko,
                    "en" => Lang::En,
                    _ => Lang::Auto,
                }
            }
            "--speed" => speed = value()?.parse()?,
            "--steps" => opts.denoise_steps = value()?.parse()?,
            "--threads" => opts.threads = Some(value()?.parse()?),
            _ if text.is_none() => text = Some(a),
            _ if out.is_none() => out = Some(a),
            _ => return Err(format!("unexpected argument {a:?}").into()),
        }
    }
    let text = text.ok_or("usage: say <text> <out.wav> [--voice F1] [--lang ko|en|auto]")?;
    let out = out.unwrap_or_else(|| "out.wav".into());

    let store = ModelStore::default();
    if !store.is_ready() {
        eprintln!("downloading Supertonic 3 into {}", store.dir().display());
    }
    let mut last = 0;
    let paths = store.ensure(&mut |done, total| {
        let pct = done * 100 / total.max(1);
        if pct != last {
            last = pct;
            eprint!("\r  {pct:3}% of {} MB", total / 1_000_000);
            std::io::stderr().flush().ok();
        }
    })?;
    eprintln!();

    let t = Instant::now();
    let engine = SupertonicEngine::new(paths, opts)?;
    eprintln!("loaded in {:.2} s", t.elapsed().as_secs_f32());

    let req = SynthRequest {
        text,
        voice_id: voice,
        lang,
        speed,
    };
    // The first run includes ONNX Runtime warm-up; measure a second run as well.
    for run in 1..=2 {
        let mut chunks = Vec::new();
        let t = Instant::now();
        let mut first_chunk = None;
        engine.synthesize(&req, &Cancel::new(), &mut |c| {
            first_chunk.get_or_insert_with(|| t.elapsed());
            chunks.push(c)
        })?;
        let took = t.elapsed().as_secs_f32();
        let pcm = concat(&chunks).ok_or("no audio")?;
        let secs = pcm.duration_secs();
        eprintln!(
            "run {run}: {secs:.2} s of audio in {took:.2} s, RTF {:.3}, first chunk after {:.2} s",
            took / secs,
            first_chunk.unwrap_or_default().as_secs_f32()
        );
        if run == 2 {
            write_wav(out.as_ref(), &pcm)?;
            eprintln!("wrote {out} ({} Hz mono)", pcm.sample_rate);
        }
    }
    Ok(())
}
