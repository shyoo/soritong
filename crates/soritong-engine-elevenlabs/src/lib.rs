//! ElevenLabs TTS engine implementation for soritong.
//!
//! Implements [`soritong_core::TtsEngine`] against the ElevenLabs REST API.
//!
//! # Latency and Streaming
//!
//! ElevenLabs provides both HTTP chunked streaming (`/v1/text-to-speech/{voice_id}/stream`)
//! and WebSocket streaming (`/v1/text-to-speech/{voice_id}/stream-input`).
//!
//! In v1, soritong uses HTTP streaming with blocking I/O (`ureq`), requesting raw PCM
//! (`pcm_24000`). As the server streams s16le bytes, this crate decodes them into f32 samples
//! and dispatches chunks of approximately 100 ms (~2400 samples at 24 kHz) to the playback
//! callback as they arrive, allowing audio output to begin before the complete utterance has
//! downloaded.
//!
//! *Note on WebSocket streaming*: Migrating to ElevenLabs' bidirectional WebSocket protocol in
//! a future version would cut first-audio latency further:
//! 1. **Connection re-use / pre-warming**: Avoids the ~50–150 ms TCP + TLS handshake overhead on
//!    every synthesized sentence.
//! 2. **Token-level input streaming**: Can begin synthesis while the user is still typing or
//!    while earlier text is being chunked.
//! 3. **Sub-chunk frame delivery**: Delivers audio frames at the model's raw generation cadence
//!    (~75 ms on `eleven_flash_v2_5`).

pub mod api;
pub mod pcm;

use api::{build_synth_body, map_http_error, parse_voices_json, sanitize_text};
use pcm::decode_pcm_stream;
use serde::{Deserialize, Serialize};
use soritong_core::{Cancel, PcmChunk, Result, SynthRequest, TtsEngine, TtsError, Voice};
use std::fmt;
use std::time::Duration;

/// ElevenLabs model identifiers and friendly UI descriptions.
pub const MODELS: &[(&str, &str)] = &[
    ("eleven_flash_v2_5", "Flash v2.5 — lowest latency"),
    ("eleven_multilingual_v2", "Multilingual v2 — quality"),
    ("eleven_v3", "v3 — most expressive"),
];

const DEFAULT_BASE_URL: &str = "https://api.elevenlabs.io";
const DEFAULT_MODEL_ID: &str = "eleven_flash_v2_5";
const OUTPUT_FORMAT: &str = "pcm_24000";

/// Configuration for the ElevenLabs TTS engine.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct ElevenLabsConfig {
    pub api_key: String,
    pub model_id: String,
    pub stability: Option<f32>,
    pub similarity_boost: Option<f32>,
    pub style: Option<f32>,
    pub use_speaker_boost: Option<bool>,
    pub base_url: String,
}

impl Default for ElevenLabsConfig {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            model_id: DEFAULT_MODEL_ID.to_string(),
            stability: None,
            similarity_boost: None,
            style: None,
            use_speaker_boost: None,
            base_url: DEFAULT_BASE_URL.to_string(),
        }
    }
}

impl fmt::Debug for ElevenLabsConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ElevenLabsConfig")
            .field(
                "api_key",
                &if self.api_key.is_empty() {
                    "<empty>"
                } else {
                    "<redacted>"
                },
            )
            .field("model_id", &self.model_id)
            .field("stability", &self.stability)
            .field("similarity_boost", &self.similarity_boost)
            .field("style", &self.style)
            .field("use_speaker_boost", &self.use_speaker_boost)
            .field("base_url", &self.base_url)
            .finish()
    }
}

/// ElevenLabs TTS engine.
pub struct ElevenLabsEngine {
    config: ElevenLabsConfig,
    agent: ureq::Agent,
}

impl fmt::Debug for ElevenLabsEngine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ElevenLabsEngine")
            .field("config", &self.config)
            .finish()
    }
}

impl ElevenLabsEngine {
    pub fn new(cfg: ElevenLabsConfig) -> Self {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(60)))
            .build()
            .into();

        Self { config: cfg, agent }
    }

    pub fn config(&self) -> &ElevenLabsConfig {
        &self.config
    }

    fn check_api_key(&self) -> Result<()> {
        if self.config.api_key.trim().is_empty() {
            Err(TtsError::Config(
                "ElevenLabs API key is missing or invalid".to_string(),
            ))
        } else {
            Ok(())
        }
    }
}

impl TtsEngine for ElevenLabsEngine {
    fn id(&self) -> &'static str {
        "elevenlabs"
    }

    fn display_name(&self) -> &str {
        "ElevenLabs"
    }

    fn is_offline(&self) -> bool {
        false
    }

    fn settings_fingerprint(&self) -> String {
        format!(
            "model={}:stability={:?}:similarity_boost={:?}:style={:?}:use_speaker_boost={:?}:output_format={}",
            self.config.model_id,
            self.config.stability,
            self.config.similarity_boost,
            self.config.style,
            self.config.use_speaker_boost,
            OUTPUT_FORMAT,
        )
    }

    fn voices(&self) -> Result<Vec<Voice>> {
        self.check_api_key()?;

        let base = self.config.base_url.trim_end_matches('/');
        let url = format!("{base}/v1/voices");

        let resp = self
            .agent
            .get(&url)
            .header("xi-api-key", &self.config.api_key)
            .call()
            .map_err(|e| {
                TtsError::Network(sanitize_text(
                    &format!("Failed to connect to ElevenLabs voices endpoint: {e}"),
                    &self.config.api_key,
                ))
            })?;

        let status = resp.status().as_u16();
        if status != 200 {
            let body_str = resp.into_body().read_to_string().unwrap_or_default();
            return Err(map_http_error(status, &body_str, &self.config.api_key));
        }

        let body_str = resp
            .into_body()
            .read_to_string()
            .map_err(|e| TtsError::Network(format!("Failed to read voices response body: {e}")))?;

        parse_voices_json(&body_str)
    }

    fn synthesize(
        &self,
        req: &SynthRequest,
        cancel: &Cancel,
        on_chunk: &mut dyn FnMut(PcmChunk),
    ) -> Result<()> {
        self.check_api_key()?;

        if cancel.is_cancelled() {
            return Err(TtsError::Cancelled);
        }

        let base = self.config.base_url.trim_end_matches('/');
        let url = format!(
            "{base}/v1/text-to-speech/{}/stream?output_format={OUTPUT_FORMAT}",
            req.voice_id
        );

        let body = build_synth_body(&self.config, req);

        let resp = self
            .agent
            .post(&url)
            .header("xi-api-key", &self.config.api_key)
            .header("Content-Type", "application/json")
            .send_json(&body)
            .map_err(|e| {
                TtsError::Network(sanitize_text(
                    &format!("Failed to connect to ElevenLabs TTS endpoint: {e}"),
                    &self.config.api_key,
                ))
            })?;

        if cancel.is_cancelled() {
            return Err(TtsError::Cancelled);
        }

        let status = resp.status().as_u16();
        if status != 200 {
            let body_str = resp.into_body().read_to_string().unwrap_or_default();
            return Err(map_http_error(status, &body_str, &self.config.api_key));
        }

        let reader = resp.into_body().into_reader();
        decode_pcm_stream(reader, cancel, on_chunk)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soritong_core::Lang;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;

    #[test]
    fn test_config_debug_redaction() {
        let key = "super_secret_elevenlabs_api_key_xyz";
        let cfg = ElevenLabsConfig {
            api_key: key.to_string(),
            ..Default::default()
        };

        let debug_str = format!("{cfg:?}");
        assert!(
            !debug_str.contains(key),
            "Debug output must not contain API key!"
        );
        assert!(
            debug_str.contains("<redacted>"),
            "Debug output should indicate key is redacted"
        );

        let engine = ElevenLabsEngine::new(cfg);
        let engine_debug = format!("{engine:?}");
        assert!(
            !engine_debug.contains(key),
            "Engine debug output must not contain API key!"
        );
        assert!(engine_debug.contains("<redacted>"));

        let empty_cfg = ElevenLabsConfig::default();
        let empty_debug = format!("{empty_cfg:?}");
        assert!(empty_debug.contains("<empty>"));
    }

    #[test]
    fn test_settings_fingerprint() {
        let mut cfg = ElevenLabsConfig::default();
        let e1 = ElevenLabsEngine::new(cfg.clone());
        let fp1 = e1.settings_fingerprint();

        cfg.stability = Some(0.8);
        let e2 = ElevenLabsEngine::new(cfg.clone());
        let fp2 = e2.settings_fingerprint();

        assert_ne!(fp1, fp2);

        cfg.model_id = "eleven_multilingual_v2".to_string();
        let e3 = ElevenLabsEngine::new(cfg);
        let fp3 = e3.settings_fingerprint();

        assert_ne!(fp2, fp3);
    }

    #[test]
    fn test_empty_api_key_fails_early() {
        let engine = ElevenLabsEngine::new(ElevenLabsConfig::default());
        let req = SynthRequest {
            text: "Hello".to_string(),
            voice_id: "voice1".to_string(),
            lang: Lang::En,
            speed: 1.0,
        };
        let cancel = Cancel::new();
        let res = engine.synthesize(&req, &cancel, &mut |_| {});
        match res {
            Err(TtsError::Config(msg)) => {
                assert_eq!(msg, "ElevenLabs API key is missing or invalid");
            }
            other => panic!("expected Config error, got {other:?}"),
        }

        let voices_res = engine.voices();
        match voices_res {
            Err(TtsError::Config(msg)) => {
                assert_eq!(msg, "ElevenLabs API key is missing or invalid");
            }
            other => panic!("expected Config error, got {other:?}"),
        }
    }

    #[test]
    fn test_synthesize_against_local_http_server() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();

        let server_thread = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(socket.try_clone().unwrap());

            // Read HTTP request lines
            let mut request_line = String::new();
            reader.read_line(&mut request_line).unwrap();
            assert!(
                request_line.starts_with(
                    "POST /v1/text-to-speech/test_voice/stream?output_format=pcm_24000"
                ),
                "Unexpected request line: {request_line}"
            );

            let mut headers = Vec::new();
            let mut content_length = 0;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" || line == "\n" {
                    break;
                }
                if line.to_lowercase().starts_with("content-length:") {
                    let parts: Vec<&str> = line.split(':').collect();
                    content_length = parts[1].trim().parse::<usize>().unwrap();
                }
                headers.push(line);
            }

            // Verify xi-api-key was passed
            assert!(
                headers
                    .iter()
                    .any(|h| h.to_lowercase().starts_with("xi-api-key:") && h.contains("valid-key")),
                "Expected xi-api-key header"
            );

            // Read body
            let mut body_bytes = vec![0u8; content_length];
            std::io::Read::read_exact(&mut reader, &mut body_bytes).unwrap();
            let body_json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
            assert_eq!(body_json["text"], "테스트 문장입니다.");
            assert_eq!(body_json["language_code"], "ko");

            // Send HTTP response headers with Connection: close
            let response_headers =
                "HTTP/1.1 200 OK\r\nContent-Type: audio/pcm\r\nConnection: close\r\n\r\n";
            socket.write_all(response_headers.as_bytes()).unwrap();

            // Stream PCM in 3 writes of 1200 samples (2400 bytes each) = 3600 total samples
            let sample_bytes = 1000i16.to_le_bytes();
            let chunk_data = sample_bytes.repeat(1200);

            for _ in 0..3 {
                socket.write_all(&chunk_data).unwrap();
                socket.flush().unwrap();
                std::thread::sleep(Duration::from_millis(15));
            }
        });

        let cfg = ElevenLabsConfig {
            api_key: "valid-key".to_string(),
            base_url: format!("http://{addr}"),
            ..Default::default()
        };
        let engine = ElevenLabsEngine::new(cfg);
        let req = SynthRequest {
            text: "테스트 문장입니다.".to_string(),
            voice_id: "test_voice".to_string(),
            lang: Lang::Ko,
            speed: 1.0,
        };
        let cancel = Cancel::new();
        let mut received_chunks = Vec::new();

        engine
            .synthesize(&req, &cancel, &mut |chunk| received_chunks.push(chunk))
            .unwrap();

        server_thread.join().unwrap();

        // Total 3600 samples received
        // Target is 2400 samples per chunk, so we expect:
        // Chunk 0: 2400 samples
        // Chunk 1: 1200 samples (from finish)
        assert_eq!(received_chunks.len(), 2);
        assert_eq!(received_chunks[0].samples.len(), 2400);
        assert_eq!(received_chunks[1].samples.len(), 1200);

        let expected_f32 = 1000.0 / 32768.0;
        assert!((received_chunks[0].samples[0] - expected_f32).abs() < 1e-6);
    }

    #[test]
    fn test_cancel_mid_stream() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();

        let server_thread = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(socket.try_clone().unwrap());

            // Consume request headers
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap() == 0 || line == "\r\n" || line == "\n" {
                    break;
                }
            }

            let response_headers =
                "HTTP/1.1 200 OK\r\nContent-Type: audio/pcm\r\nConnection: close\r\n\r\n";
            let _ = socket.write_all(response_headers.as_bytes());

            // Write 1 chunk of 2400 samples
            let sample_bytes = 500i16.to_le_bytes();
            let chunk_data = sample_bytes.repeat(2400);
            let _ = socket.write_all(&chunk_data);
            let _ = socket.flush();

            // Wait a moment then attempt to write more
            std::thread::sleep(Duration::from_millis(50));
            let _ = socket.write_all(&chunk_data);
            let _ = socket.flush();
        });

        let cfg = ElevenLabsConfig {
            api_key: "valid-key".to_string(),
            base_url: format!("http://{addr}"),
            ..Default::default()
        };
        let engine = ElevenLabsEngine::new(cfg);
        let req = SynthRequest {
            text: "취소 테스트".to_string(),
            voice_id: "test_voice".to_string(),
            lang: Lang::Ko,
            speed: 1.0,
        };

        let cancel = Cancel::new();
        let cancel_trigger = cancel.clone();
        let mut chunk_count = 0;

        let res = engine.synthesize(&req, &cancel, &mut |_chunk| {
            chunk_count += 1;
            // Cancel as soon as first chunk arrives
            cancel_trigger.cancel();
        });

        assert_eq!(chunk_count, 1);
        match res {
            Err(TtsError::Cancelled) => {}
            other => panic!("expected Cancelled error, got {other:?}"),
        }

        server_thread.join().unwrap();
    }

    #[test]
    #[ignore = "requires ELEVENLABS_API_KEY and ELEVENLABS_VOICE_ID"]
    fn test_live_synthesize() {
        let api_key = match std::env::var("ELEVENLABS_API_KEY") {
            Ok(k) if !k.trim().is_empty() => k,
            _ => return,
        };
        let voice_id = match std::env::var("ELEVENLABS_VOICE_ID") {
            Ok(v) if !v.trim().is_empty() => v,
            _ => return,
        };

        let cfg = ElevenLabsConfig {
            api_key,
            model_id: "eleven_flash_v2_5".to_string(),
            ..Default::default()
        };
        let engine = ElevenLabsEngine::new(cfg);

        // First list voices to verify connection
        let voices = engine.voices().expect("Failed to fetch live voices");
        assert!(!voices.is_empty(), "Live account has no voices");

        // Synthesize a very short phrase to preserve credits
        let start = std::time::Instant::now();
        let req = SynthRequest {
            text: "안녕".to_string(),
            voice_id,
            lang: Lang::Ko,
            speed: 1.0,
        };
        let cancel = Cancel::new();
        let mut first_chunk_latency = None;
        let mut total_samples = 0;

        engine
            .synthesize(&req, &cancel, &mut |chunk| {
                if first_chunk_latency.is_none() {
                    first_chunk_latency = Some(start.elapsed());
                }
                total_samples += chunk.samples.len();
            })
            .expect("Live synthesize failed");

        eprintln!(
            "Live synthesize completed in {:?}, first chunk latency: {:?}, samples: {}",
            start.elapsed(),
            first_chunk_latency,
            total_samples
        );
        assert!(total_samples > 0);
    }
}
