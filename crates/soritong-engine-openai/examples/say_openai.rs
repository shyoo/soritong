use soritong_core::cache::{concat, write_wav};
use soritong_core::{Cancel, Lang, PcmChunk, SynthRequest, TtsEngine};
use soritong_engine_openai::{OpenAiConfig, OpenAiEngine};
use std::path::Path;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();

    let mut base_url =
        std::env::var("OPENAI_BASE_URL").unwrap_or_else(|_| "http://127.0.0.1:8004".to_string());
    let mut api_key = std::env::var("OPENAI_API_KEY").ok();
    let mut model = "tts-1".to_string();
    let mut response_format = "wav".to_string();
    let mut voice_id = "alloy".to_string();
    let mut text = "Hello from OpenAI speech engine!".to_string();
    let mut speed = 1.0f32;
    let mut out_path = "output.wav".to_string();
    let mut list_voices = false;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--voices" => {
                list_voices = true;
                i += 1;
            }
            "--base-url" if i + 1 < args.len() => {
                base_url = args[i + 1].clone();
                i += 2;
            }
            "--api-key" if i + 1 < args.len() => {
                api_key = Some(args[i + 1].clone());
                i += 2;
            }
            "--model" if i + 1 < args.len() => {
                model = args[i + 1].clone();
                i += 2;
            }
            "--format" if i + 1 < args.len() => {
                response_format = args[i + 1].clone();
                i += 2;
            }
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

    let config = OpenAiConfig {
        base_url,
        api_key,
        model,
        voices: Vec::new(),
        response_format,
    };
    let engine = OpenAiEngine::new(config);

    if list_voices {
        println!("Available voices for {}:", engine.display_name());
        let voices = engine.voices()?;
        for v in voices {
            println!("  - id: {}", v.id);
            println!("    name: {}", v.name);
            println!("    langs: {:?}", v.langs);
            println!("    kind: {:?}", v.kind);
        }
        return Ok(());
    }

    println!("Synthesizing: \"{}\"", text);
    println!("Base URL: {}", engine.config().base_url);
    println!("Voice: {}", voice_id);
    println!("Offline: {}", engine.is_offline());

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
