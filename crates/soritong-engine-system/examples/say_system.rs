use soritong_core::cache::{concat, write_wav};
use soritong_core::{Cancel, Lang, PcmChunk, SynthRequest, TtsEngine};
use soritong_engine_system::SystemEngine;
use std::path::Path;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let engine = SystemEngine::new();

    if args.iter().any(|a| a == "--voices") {
        println!("Available voices for {}:", engine.display_name());
        let voices = engine.voices()?;
        if voices.is_empty() {
            println!("  (No voices available)");
        }
        for v in voices {
            println!("  - id: {}", v.id);
            println!("    name: {}", v.name);
            println!("    langs: {:?}", v.langs);
            println!("    kind: {:?}", v.kind);
        }
        return Ok(());
    }

    let mut text = "Hello from Windows speech synthesis!".to_string();
    let mut voice_id = String::new();
    let mut speed = 1.0f32;
    let mut out_path = "output.wav".to_string();

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--voice" if i + 1 < args.len() => {
                voice_id = args[i + 1].clone();
                i += 2;
            }
            "--speed" if i + 1 < args.len() => {
                speed = args[i + 1].parse().unwrap_or(1.0);
                i += 2;
            }
            "--out" if i + 1 < args.len() => {
                out_path = args[i + 1].clone();
                i += 2;
            }
            s if !s.starts_with('-') => {
                text = s.to_string();
                i += 1;
            }
            _ => {
                i += 1;
            }
        }
    }

    println!("Synthesizing: \"{}\"", text);
    let req = SynthRequest {
        text,
        voice_id,
        lang: Lang::Auto,
        speed,
    };

    let cancel = Cancel::new();
    let mut chunks: Vec<PcmChunk> = Vec::new();

    engine.synthesize(&req, &cancel, &mut |chunk| {
        chunks.push(chunk);
    })?;

    let pcm = concat(&chunks).ok_or("No audio generated")?;
    write_wav(Path::new(&out_path), &pcm)?;
    println!("Saved audio to {}", out_path);

    Ok(())
}
