//! Example: synthesize speech using ElevenLabs or list account voices.
//!
//! # Usage
//!
//! List account voices:
//! ```sh
//! cargo run -p soritong-engine-elevenlabs --example say_elevenlabs -- --voices
//! ```
//!
//! Synthesize speech:
//! ```sh
//! cargo run -p soritong-engine-elevenlabs --example say_elevenlabs -- "안녕하세요" out.wav
//! ```
//!
//! Environment variables:
//! - `ELEVENLABS_API_KEY`: ElevenLabs API key (required).
//! - `ELEVENLABS_VOICE_ID`: Voice ID to use (optional; defaults to first cloned or stock voice).
//! - `ELEVENLABS_MODEL`: Model ID (optional; defaults to `eleven_flash_v2_5`).

use soritong_core::cache::{concat, write_wav};
use soritong_core::text::detect_lang;
use soritong_core::{Cancel, PcmChunk, SynthRequest, TtsEngine, VoiceKind};
use soritong_engine_elevenlabs::{ElevenLabsConfig, ElevenLabsEngine};
use std::env;
use std::path::Path;
use std::process;
use std::time::Instant;

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();

    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!("Usage:");
        println!("  say --voices");
        println!("  say <text> <output.wav> [--voice <voice_id>] [--model <model_id>]");
        process::exit(0);
    }

    if args.is_empty() {
        eprintln!("Usage:");
        eprintln!("  say --voices");
        eprintln!("  say <text> <output.wav> [--voice <voice_id>] [--model <model_id>]");
        process::exit(1);
    }

    let api_key = match env::var("ELEVENLABS_API_KEY") {
        Ok(k) if !k.trim().is_empty() => k.trim().to_string(),
        _ => {
            eprintln!("Error: ELEVENLABS_API_KEY environment variable is not set.");
            process::exit(1);
        }
    };

    let model_id = env::var("ELEVENLABS_MODEL").unwrap_or_else(|_| "eleven_flash_v2_5".to_string());

    let config = ElevenLabsConfig {
        api_key,
        model_id,
        ..Default::default()
    };
    let engine = ElevenLabsEngine::new(config);

    if args.iter().any(|a| a == "--voices") {
        println!("Fetching voices from ElevenLabs...");
        match engine.voices() {
            Ok(voices) => {
                println!("Found {} voices:", voices.len());
                for v in voices {
                    let kind_tag = match v.kind {
                        VoiceKind::Cloned => "cloned",
                        VoiceKind::Stock => "stock ",
                    };
                    let langs_str = if v.langs.is_empty() {
                        "all/unknown".to_string()
                    } else {
                        v.langs
                            .iter()
                            .map(|l| l.code())
                            .collect::<Vec<_>>()
                            .join(", ")
                    };
                    println!("  [{kind_tag}] {} - {} (langs: {langs_str})", v.id, v.name);
                }
            }
            Err(e) => {
                eprintln!("Error fetching voices: {e}");
                process::exit(1);
            }
        }
        return;
    }

    // Synthesis mode
    let mut text = None;
    let mut out_path = None;
    let mut voice_override = None;
    let mut idx = 0;

    while idx < args.len() {
        if args[idx] == "--voice" && idx + 1 < args.len() {
            voice_override = Some(args[idx + 1].clone());
            idx += 2;
        } else if text.is_none() {
            text = Some(args[idx].clone());
            idx += 1;
        } else if out_path.is_none() {
            out_path = Some(args[idx].clone());
            idx += 1;
        } else {
            idx += 1;
        }
    }

    let text = match text {
        Some(t) => t,
        None => {
            eprintln!("Error: Missing text to synthesize.");
            process::exit(1);
        }
    };

    let out_path = match out_path {
        Some(p) => p,
        None => {
            eprintln!("Error: Missing output file path (e.g. out.wav).");
            process::exit(1);
        }
    };

    let voice_id = match voice_override.or_else(|| env::var("ELEVENLABS_VOICE_ID").ok()) {
        Some(v) if !v.trim().is_empty() => v.trim().to_string(),
        _ => {
            // Find first cloned or available voice
            println!("No ELEVENLABS_VOICE_ID specified, querying available voices...");
            match engine.voices() {
                Ok(voices) if !voices.is_empty() => {
                    let picked = &voices[0];
                    println!("Auto-selected voice: {} ({})", picked.name, picked.id);
                    picked.id.clone()
                }
                Ok(_) => {
                    eprintln!("Error: No voices found in account.");
                    process::exit(1);
                }
                Err(e) => {
                    eprintln!("Error discovering voices: {e}");
                    process::exit(1);
                }
            }
        }
    };

    let lang = detect_lang(&text);
    let req = SynthRequest {
        text: text.clone(),
        voice_id,
        lang,
        speed: 1.0,
    };

    println!(
        "Synthesizing \"{}\" (lang: {:?}, model: {})...",
        req.text,
        req.lang,
        engine.config().model_id
    );

    let cancel = Cancel::new();
    let mut chunks: Vec<PcmChunk> = Vec::new();
    let mut first_chunk_time = None;
    let start = Instant::now();

    let result = engine.synthesize(&req, &cancel, &mut |chunk| {
        if first_chunk_time.is_none() {
            first_chunk_time = Some(start.elapsed());
        }
        chunks.push(chunk);
    });

    if let Err(e) = result {
        eprintln!("Synthesis error: {e}");
        process::exit(1);
    }

    let elapsed = start.elapsed();
    let full_audio = match concat(&chunks) {
        Some(pcm) => pcm,
        None => {
            eprintln!("Error: No audio chunks received.");
            process::exit(1);
        }
    };

    if let Err(e) = write_wav(Path::new(&out_path), &full_audio) {
        eprintln!("Error writing WAV file '{out_path}': {e}");
        process::exit(1);
    }

    println!(
        "Done in {:.2}s (first chunk: {:.2}s, duration: {:.2}s, {} samples). Saved to {}",
        elapsed.as_secs_f32(),
        first_chunk_time.map_or(0.0, |t| t.as_secs_f32()),
        full_audio.duration_secs(),
        full_audio.samples.len(),
        out_path
    );
}
