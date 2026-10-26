//! OpenAI-compatible `/v1/audio/speech` TTS engine.
//!
//! Supports both official OpenAI TTS (`https://api.openai.com`) and local/self-hosted
//! voice-cloning servers exposing the same API (e.g. Chatterbox-TTS-Server for
//! Chatterbox Multilingual, which speaks Korean and clones voices from a short sample).
//!
//! # Requirements & Limits
//! - When pointing to `localhost`, `127.0.0.1`, or `::1`, the engine is recognized as
//!   offline ([`TtsEngine::is_offline`] returns `true`), and no text leaves your computer.
//! - For remote endpoints (e.g. `api.openai.com`), internet access and an API key are required.
//! - Supported response formats: `"wav"` and `"pcm"` (24 kHz 16-bit mono PCM).

use serde::{Deserialize, Serialize};
use soritong_core::{
    Cancel, Lang, PcmChunk, Result, SynthRequest, TtsEngine, TtsError, Voice, VoiceKind,
};
use std::fmt;
use std::io::Read;

pub const DEFAULT_OPENAI_BASE_URL: &str = "https://api.openai.com";
pub const DEFAULT_LOCAL_BASE_URL: &str = "http://127.0.0.1:8004";
pub const DEFAULT_MODEL: &str = "tts-1";

pub const OPENAI_STOCK_VOICES: &[&str] = &[
    "alloy", "echo", "fable", "onyx", "nova", "shimmer", "sage", "coral", "verse",
];

#[derive(Clone, Serialize, Deserialize)]
pub struct OpenAiConfig {
    pub base_url: String,
    pub api_key: Option<String>,
    pub model: String,
    pub voices: Vec<String>,
    pub response_format: String,
}

impl Default for OpenAiConfig {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_LOCAL_BASE_URL.to_string(),
            api_key: None,
            model: DEFAULT_MODEL.to_string(),
            voices: Vec::new(),
            response_format: "wav".to_string(),
        }
    }
}

impl fmt::Debug for OpenAiConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpenAiConfig")
            .field("base_url", &self.base_url)
            .field("api_key", &self.api_key.as_ref().map(|_| "[REDACTED]"))
            .field("model", &self.model)
            .field("voices", &self.voices)
            .field("response_format", &self.response_format)
            .finish()
    }
}

#[derive(Debug, Clone)]
pub struct OpenAiEngine {
    config: OpenAiConfig,
}

impl OpenAiEngine {
    pub fn new(config: OpenAiConfig) -> Self {
        Self { config }
    }

    pub fn config(&self) -> &OpenAiConfig {
        &self.config
    }
}

impl TtsEngine for OpenAiEngine {
    fn id(&self) -> &'static str {
        "openai"
    }

    fn display_name(&self) -> &str {
        "OpenAI-compatible speech"
    }

    fn is_offline(&self) -> bool {
        is_host_offline(&self.config.base_url)
    }

    fn settings_fingerprint(&self) -> String {
        format!(
            "{}:{}:{}",
            self.config.base_url, self.config.model, self.config.response_format
        )
    }

    fn voices(&self) -> Result<Vec<Voice>> {
        if !self.config.voices.is_empty() {
            return Ok(self
                .config
                .voices
                .iter()
                .map(|v| Voice {
                    id: v.clone(),
                    name: v.clone(),
                    langs: vec![Lang::Ko, Lang::En],
                    kind: VoiceKind::Cloned,
                })
                .collect());
        }

        // Try querying GET {base}/v1/audio/voices
        let base = self.config.base_url.trim_end_matches('/');
        let voices_url = format!("{base}/v1/audio/voices");

        let mut req = ureq::get(&voices_url);
        if let Some(ref key) = self.config.api_key {
            req = req.header("Authorization", &format!("Bearer {key}"));
        }

        if let Ok(res) = req.call()
            && res.status().is_success()
        {
            let mut reader = res.into_body().into_reader();
            if let Ok(voices) = parse_voices_response(&mut reader)
                && !voices.is_empty()
            {
                return Ok(voices);
            }
        }

        // Fall back to OpenAI stock names when the host is api.openai.com
        if is_openai_host(&self.config.base_url) {
            return Ok(OPENAI_STOCK_VOICES
                .iter()
                .map(|&name| Voice {
                    id: name.to_string(),
                    name: name.to_string(),
                    langs: vec![Lang::Ko, Lang::En],
                    kind: VoiceKind::Stock,
                })
                .collect());
        }

        Err(TtsError::Config(format!(
            "No voices configured and failed to query /v1/audio/voices from {}",
            self.config.base_url
        )))
    }

    fn synthesize(
        &self,
        req: &SynthRequest,
        cancel: &Cancel,
        on_chunk: &mut dyn FnMut(PcmChunk),
    ) -> Result<()> {
        if cancel.is_cancelled() {
            return Err(TtsError::Cancelled);
        }

        let voice = if req.voice_id.is_empty() {
            if let Some(first) = self.config.voices.first() {
                first.as_str()
            } else if is_openai_host(&self.config.base_url) {
                "alloy"
            } else {
                return Err(TtsError::Config(
                    "No voice_id specified and no default voices configured".into(),
                ));
            }
        } else {
            req.voice_id.as_str()
        };

        let base = self.config.base_url.trim_end_matches('/');
        let speech_url = format!("{base}/v1/audio/speech");

        let payload = serde_json::json!({
            "model": self.config.model,
            "input": req.text,
            "voice": voice,
            "response_format": self.config.response_format,
            "speed": req.speed,
        });

        let mut http_req = ureq::post(&speech_url).header("Content-Type", "application/json");

        if let Some(ref key) = self.config.api_key {
            http_req = http_req.header("Authorization", &format!("Bearer {key}"));
        }

        let response = match http_req.send_json(&payload) {
            Ok(res) => res,
            Err(ureq::Error::StatusCode(code)) => {
                if code == 401 {
                    return Err(TtsError::Config(
                        "Unauthorized (401): invalid or missing API key".into(),
                    ));
                } else if (400..500).contains(&code) {
                    return Err(TtsError::Config(format!(
                        "Client error ({code}) from TTS server"
                    )));
                } else {
                    return Err(TtsError::Network(format!(
                        "Server error ({code}) from TTS server"
                    )));
                }
            }
            Err(e) => {
                return Err(TtsError::Network(format!("HTTP request failed: {e}")));
            }
        };

        let status = response.status();
        if status == 401 {
            return Err(TtsError::Config(
                "Unauthorized (401): invalid or missing API key".into(),
            ));
        } else if status.is_client_error() {
            return Err(TtsError::Config(format!(
                "Client error ({}) from TTS server",
                status.as_u16()
            )));
        } else if status.is_server_error() {
            return Err(TtsError::Network(format!(
                "Server error ({}) from TTS server",
                status.as_u16()
            )));
        }

        if cancel.is_cancelled() {
            return Err(TtsError::Cancelled);
        }

        let mut body_bytes = Vec::new();
        response
            .into_body()
            .into_reader()
            .read_to_end(&mut body_bytes)
            .map_err(|e| TtsError::Network(format!("failed to read response body: {e}")))?;

        if cancel.is_cancelled() {
            return Err(TtsError::Cancelled);
        }

        let chunk = if self.config.response_format.eq_ignore_ascii_case("pcm") {
            parse_pcm_s16le_24k(&body_bytes)?
        } else {
            // Assume WAV format (or default)
            parse_wav_bytes(&body_bytes)?
        };

        on_chunk(chunk);
        Ok(())
    }
}

/// Checks whether the host in `base_url` points to a local address (offline).
pub fn is_host_offline(base_url: &str) -> bool {
    if let Ok(u) = url::Url::parse(base_url)
        && let Some(host) = u.host_str()
    {
        let h = host.trim_start_matches('[').trim_end_matches(']');
        return h.eq_ignore_ascii_case("localhost") || h == "127.0.0.1" || h == "::1";
    }
    false
}

/// Checks whether the host in `base_url` is `api.openai.com`.
pub fn is_openai_host(base_url: &str) -> bool {
    if let Ok(u) = url::Url::parse(base_url)
        && let Some(host) = u.host_str()
    {
        return host.eq_ignore_ascii_case("api.openai.com");
    }
    false
}

/// Parses raw 24 kHz, 16-bit little-endian mono PCM samples into [`PcmChunk`].
pub fn parse_pcm_s16le_24k(bytes: &[u8]) -> Result<PcmChunk> {
    if !bytes.len().is_multiple_of(2) {
        return Err(TtsError::Engine(
            "pcm bytes length must be a multiple of 2".into(),
        ));
    }
    let samples: Vec<f32> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| i16::from_le_bytes(*c) as f32 / 32768.0)
        .collect();
    Ok(PcmChunk {
        sample_rate: 24000,
        channels: 1,
        samples,
    })
}

/// Parses WAV audio bytes with `hound` into [`PcmChunk`].
pub fn parse_wav_bytes(bytes: &[u8]) -> Result<PcmChunk> {
    let mut reader = hound::WavReader::new(std::io::Cursor::new(bytes))
        .map_err(|e| TtsError::Engine(format!("wav parse: {e}")))?;
    let spec = reader.spec();

    let samples: Vec<f32> = match (spec.sample_format, spec.bits_per_sample) {
        (hound::SampleFormat::Int, 16) => reader
            .samples::<i16>()
            .map(|s| s.map(|v| v as f32 / 32768.0))
            .collect::<std::result::Result<Vec<f32>, _>>()
            .map_err(|e| TtsError::Engine(format!("wav sample read: {e}")))?,
        (hound::SampleFormat::Int, 24) => reader
            .samples::<i32>()
            .map(|s| s.map(|v| v as f32 / 8_388_608.0))
            .collect::<std::result::Result<Vec<f32>, _>>()
            .map_err(|e| TtsError::Engine(format!("wav sample read: {e}")))?,
        (hound::SampleFormat::Int, 32) => reader
            .samples::<i32>()
            .map(|s| s.map(|v| v as f32 / 2_147_483_648.0))
            .collect::<std::result::Result<Vec<f32>, _>>()
            .map_err(|e| TtsError::Engine(format!("wav sample read: {e}")))?,
        (hound::SampleFormat::Int, 8) => reader
            .samples::<i8>()
            .map(|s| s.map(|v| v as f32 / 128.0))
            .collect::<std::result::Result<Vec<f32>, _>>()
            .map_err(|e| TtsError::Engine(format!("wav sample read: {e}")))?,
        (hound::SampleFormat::Float, 32) => reader
            .samples::<f32>()
            .collect::<std::result::Result<Vec<f32>, _>>()
            .map_err(|e| TtsError::Engine(format!("wav sample read: {e}")))?,
        other => {
            return Err(TtsError::Engine(format!(
                "unsupported wav format: {other:?}"
            )));
        }
    };

    Ok(PcmChunk {
        sample_rate: spec.sample_rate,
        channels: spec.channels,
        samples,
    })
}

fn parse_voices_response<R: Read>(reader: &mut R) -> Result<Vec<Voice>> {
    let val: serde_json::Value = serde_json::from_reader(reader)
        .map_err(|e| TtsError::Engine(format!("failed to parse voices json: {e}")))?;

    let mut voices = Vec::new();

    let items = if let Some(arr) = val.as_array() {
        arr.as_slice()
    } else if let Some(arr) = val.get("voices").and_then(|v| v.as_array()) {
        arr.as_slice()
    } else {
        return Ok(Vec::new());
    };

    for item in items {
        if let Some(s) = item.as_str() {
            voices.push(Voice {
                id: s.to_string(),
                name: s.to_string(),
                langs: vec![Lang::Ko, Lang::En],
                kind: VoiceKind::Cloned,
            });
        } else if let Some(obj) = item.as_object() {
            let id = obj
                .get("id")
                .or_else(|| obj.get("voice_id"))
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            let name = obj.get("name").and_then(|v| v.as_str()).unwrap_or(id);
            if !id.is_empty() {
                voices.push(Voice {
                    id: id.to_string(),
                    name: name.to_string(),
                    langs: vec![Lang::Ko, Lang::En],
                    kind: VoiceKind::Cloned,
                });
            }
        }
    }

    Ok(voices)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::net::TcpListener;

    #[test]
    fn test_offline_detection() {
        assert!(is_host_offline("http://127.0.0.1:8004"));
        assert!(is_host_offline("http://localhost:8000"));
        assert!(is_host_offline("http://[::1]:8004"));
        assert!(!is_host_offline("https://api.openai.com"));
        assert!(!is_host_offline("https://api.openai.com/v1"));
        assert!(!is_host_offline("http://192.168.1.50:8004"));
    }

    #[test]
    fn test_openai_host_detection() {
        assert!(is_openai_host("https://api.openai.com"));
        assert!(is_openai_host("https://api.openai.com/v1"));
        assert!(!is_openai_host("http://127.0.0.1:8004"));
    }

    #[test]
    fn test_debug_redaction() {
        let config = OpenAiConfig {
            base_url: "https://api.openai.com".into(),
            api_key: Some("sk-secret123456789".into()),
            model: "tts-1".into(),
            voices: vec!["alloy".into()],
            response_format: "wav".into(),
        };
        let debug_str = format!("{config:?}");
        assert!(!debug_str.contains("sk-secret123456789"));
        assert!(debug_str.contains("[REDACTED]"));
    }

    #[test]
    fn test_settings_fingerprint() {
        let engine = OpenAiEngine::new(OpenAiConfig {
            base_url: "http://127.0.0.1:8004".into(),
            api_key: None,
            model: "chatterbox".into(),
            voices: vec![],
            response_format: "pcm".into(),
        });
        assert_eq!(
            engine.settings_fingerprint(),
            "http://127.0.0.1:8004:chatterbox:pcm"
        );
    }

    #[test]
    fn test_parse_pcm_s16le() {
        // 2 samples: 0, 16384 (0.5), -16384 (-0.5)
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&0i16.to_le_bytes());
        bytes.extend_from_slice(&16384i16.to_le_bytes());
        bytes.extend_from_slice(&(-16384i16).to_le_bytes());

        let pcm = parse_pcm_s16le_24k(&bytes).unwrap();
        assert_eq!(pcm.sample_rate, 24000);
        assert_eq!(pcm.channels, 1);
        assert_eq!(pcm.samples.len(), 3);
        assert!((pcm.samples[0] - 0.0).abs() < 1e-4);
        assert!((pcm.samples[1] - 0.5).abs() < 1e-4);
        assert!((pcm.samples[2] - (-0.5)).abs() < 1e-4);
    }

    #[test]
    fn test_configured_voices_priority() {
        let engine = OpenAiEngine::new(OpenAiConfig {
            base_url: "https://api.openai.com".into(),
            api_key: None,
            model: "tts-1".into(),
            voices: vec!["my_korean_clone".into()],
            response_format: "wav".into(),
        });
        let voices = engine.voices().unwrap();
        assert_eq!(voices.len(), 1);
        assert_eq!(voices[0].id, "my_korean_clone");
        assert_eq!(voices[0].kind, VoiceKind::Cloned);
    }

    #[test]
    fn test_stock_voices_fallback_for_openai() {
        let engine = OpenAiEngine::new(OpenAiConfig {
            base_url: "https://api.openai.com".into(),
            api_key: None,
            model: "tts-1".into(),
            voices: vec![],
            response_format: "wav".into(),
        });
        let voices = engine.voices().unwrap();
        assert_eq!(voices.len(), OPENAI_STOCK_VOICES.len());
        assert_eq!(voices[0].id, "alloy");
        assert_eq!(voices[0].kind, VoiceKind::Stock);
    }

    #[test]
    fn test_hermetic_wav_synthesis_and_errors() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        // Create a synthetic WAV
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 24000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut wav_cursor = std::io::Cursor::new(Vec::new());
        {
            let mut writer = hound::WavWriter::new(&mut wav_cursor, spec).unwrap();
            writer.write_sample(8192i16).unwrap();
            writer.finalize().unwrap();
        }
        let wav_bytes = wav_cursor.into_inner();

        let wav_clone = wav_bytes.clone();
        std::thread::spawn(move || {
            // Request 1: successful WAV
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 1024];
                let _ = stream.read(&mut buf);
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: audio/wav\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    wav_clone.len()
                );
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.write_all(&wav_clone);
                let _ = stream.flush();
                let _ = stream.shutdown(std::net::Shutdown::Write);
                let mut sink = Vec::new();
                let _ = stream.read_to_end(&mut sink);
            }

            // Request 2: 401 Unauthorized
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 1024];
                let _ = stream.read(&mut buf);
                let response =
                    "HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
                let _ = stream.shutdown(std::net::Shutdown::Write);
                let mut sink = Vec::new();
                let _ = stream.read_to_end(&mut sink);
            }

            // Request 3: 500 Server Error
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 1024];
                let _ = stream.read(&mut buf);
                let response = "HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
                let _ = stream.shutdown(std::net::Shutdown::Write);
                let mut sink = Vec::new();
                let _ = stream.read_to_end(&mut sink);
            }
        });

        let engine = OpenAiEngine::new(OpenAiConfig {
            base_url: format!("http://127.0.0.1:{port}"),
            api_key: Some("test-key".into()),
            model: "tts-1".into(),
            voices: vec!["alloy".into()],
            response_format: "wav".into(),
        });

        let cancel = Cancel::new();
        let req = SynthRequest {
            text: "Hello".into(),
            voice_id: "alloy".into(),
            lang: Lang::En,
            speed: 1.0,
        };

        // 1. Success
        let mut chunks = Vec::new();
        engine
            .synthesize(&req, &cancel, &mut |c| chunks.push(c))
            .expect("Synthesis failed");
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].samples.len(), 1);
        assert!((chunks[0].samples[0] - (8192.0 / 32768.0)).abs() < 1e-4);

        // 2. 401 Unauthorized -> Config error
        let err401 = engine.synthesize(&req, &cancel, &mut |_| {});
        match err401 {
            Err(TtsError::Config(msg)) => assert!(msg.contains("401")),
            other => panic!("Expected Config error on 401, got {other:?}"),
        }

        // 3. 500 Internal Server Error -> Network error
        let err500 = engine.synthesize(&req, &cancel, &mut |_| {});
        match err500 {
            Err(TtsError::Network(msg)) => assert!(msg.contains("500")),
            other => panic!("Expected Network error on 500, got {other:?}"),
        }
    }

    #[test]
    fn test_hermetic_pcm_and_voices_endpoint() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        // 3 samples: 0, 16384 (0.5), -16384 (-0.5)
        let mut pcm_bytes = Vec::new();
        pcm_bytes.extend_from_slice(&0i16.to_le_bytes());
        pcm_bytes.extend_from_slice(&16384i16.to_le_bytes());
        pcm_bytes.extend_from_slice(&(-16384i16).to_le_bytes());

        let pcm_clone = pcm_bytes.clone();
        std::thread::spawn(move || {
            // Request 1: GET /v1/audio/voices
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 1024];
                let _ = stream.read(&mut buf);
                let json = r#"["custom_voice_1", "custom_voice_2"]"#;
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    json.len(),
                    json
                );
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
                let _ = stream.shutdown(std::net::Shutdown::Write);
                let mut sink = Vec::new();
                let _ = stream.read_to_end(&mut sink);
            }

            // Request 2: POST /v1/audio/speech with PCM response
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 1024];
                let _ = stream.read(&mut buf);
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: audio/pcm\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    pcm_clone.len()
                );
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.write_all(&pcm_clone);
                let _ = stream.flush();
                let _ = stream.shutdown(std::net::Shutdown::Write);
                let mut sink = Vec::new();
                let _ = stream.read_to_end(&mut sink);
            }
        });

        let engine = OpenAiEngine::new(OpenAiConfig {
            base_url: format!("http://127.0.0.1:{port}"),
            api_key: None,
            model: "chatterbox".into(),
            voices: vec![],
            response_format: "pcm".into(),
        });

        // Test voices querying
        let voices = engine.voices().expect("Failed to get voices");
        assert_eq!(voices.len(), 2);
        assert_eq!(voices[0].id, "custom_voice_1");
        assert_eq!(voices[1].id, "custom_voice_2");

        // Test PCM synthesis
        let cancel = Cancel::new();
        let req = SynthRequest {
            text: "Hello PCM".into(),
            voice_id: "custom_voice_1".into(),
            lang: Lang::Ko,
            speed: 1.0,
        };
        let mut chunks = Vec::new();
        engine
            .synthesize(&req, &cancel, &mut |c| chunks.push(c))
            .expect("PCM synthesis failed");
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].sample_rate, 24000);
        assert_eq!(chunks[0].samples.len(), 3);
        assert!((chunks[0].samples[1] - 0.5).abs() < 1e-4);
    }
}
