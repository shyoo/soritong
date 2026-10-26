//! Windows built-in speech synthesis engine (`Windows.Media.SpeechSynthesis`).
//!
//! Provides offline, zero-setup text-to-speech using voices installed in Windows
//! (e.g. Microsoft David, Zira, Mark for English; Microsoft Heami for Korean).
//!
//! # Requirements & Limits
//! - Windows 10/11 only. On non-Windows platforms, this crate still compiles cleanly,
//!   but [`SystemEngine::voices`] returns an empty list and [`SystemEngine::synthesize`]
//!   returns [`soritong_core::TtsError::Config`].
//! - Korean synthesis requires the Windows Korean speech pack to be installed.
//!   If no Korean voice is installed, a descriptive [`soritong_core::TtsError::Config`]
//!   error directs the user to Windows Settings > Time & Language > Speech.

use soritong_core::{
    Cancel, Lang, PcmChunk, Result, SynthRequest, TtsEngine, TtsError, Voice, VoiceKind,
};

pub const KOREAN_VOICE_HINT: &str = "No Korean voice installed. Install the Korean speech pack in Windows Settings > Time & Language > Speech.";

#[derive(Debug, Default, Clone)]
pub struct SystemEngine;

impl SystemEngine {
    pub fn new() -> Self {
        Self
    }
}

impl TtsEngine for SystemEngine {
    fn id(&self) -> &'static str {
        "system"
    }

    fn display_name(&self) -> &str {
        "Windows built-in voices"
    }

    fn is_offline(&self) -> bool {
        true
    }

    fn voices(&self) -> Result<Vec<Voice>> {
        #[cfg(windows)]
        {
            list_windows_voices()
        }
        #[cfg(not(windows))]
        {
            Ok(Vec::new())
        }
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

        #[cfg(windows)]
        {
            synthesize_windows(req, cancel, on_chunk)
        }
        #[cfg(not(windows))]
        {
            let _ = (req, on_chunk);
            Err(TtsError::Config("Windows only".into()))
        }
    }
}

/// Clamps the speed to the valid WinRT SpeakingRate range (0.5 to 6.0).
pub fn clamp_speed(speed: f32) -> f64 {
    (speed as f64).clamp(0.5, 6.0)
}

/// Converts WAV bytes parsed with hound into a 32-bit float [`PcmChunk`].
pub fn wav_bytes_to_pcm(bytes: &[u8]) -> Result<PcmChunk> {
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

#[cfg(windows)]
fn list_windows_voices() -> Result<Vec<Voice>> {
    use windows::Media::SpeechSynthesis::SpeechSynthesizer;

    let winrt_voices = SpeechSynthesizer::AllVoices().map_err(|e| {
        TtsError::Engine(format!("failed to query SpeechSynthesizer::AllVoices: {e}"))
    })?;

    let mut voices = Vec::new();
    for v in winrt_voices {
        let id = v
            .Id()
            .map_err(|e| TtsError::Engine(format!("voice Id error: {e}")))?;
        let name = v
            .DisplayName()
            .map_err(|e| TtsError::Engine(format!("voice DisplayName error: {e}")))?;
        let lang_tag = v
            .Language()
            .map_err(|e| TtsError::Engine(format!("voice Language error: {e}")))?;

        let lang_tag_str = lang_tag.to_string().to_lowercase();
        let mut langs = Vec::new();
        if lang_tag_str.starts_with("ko") {
            langs.push(Lang::Ko);
        } else if lang_tag_str.starts_with("en") {
            langs.push(Lang::En);
        }

        voices.push(Voice {
            id: id.to_string(),
            name: name.to_string(),
            langs,
            kind: VoiceKind::Stock,
        });
    }

    Ok(voices)
}

#[cfg(windows)]
fn synthesize_windows(
    req: &SynthRequest,
    cancel: &Cancel,
    on_chunk: &mut dyn FnMut(PcmChunk),
) -> Result<()> {
    use windows::Media::SpeechSynthesis::SpeechSynthesizer;
    use windows::Storage::Streams::DataReader;
    use windows::core::HSTRING;

    let all_voices = SpeechSynthesizer::AllVoices()
        .map_err(|e| TtsError::Engine(format!("failed to query all voices: {e}")))?;

    let synth = SpeechSynthesizer::new()
        .map_err(|e| TtsError::Engine(format!("failed to create SpeechSynthesizer: {e}")))?;

    // Pick voice
    let mut selected_voice = None;
    let mut has_korean_voice = false;

    for v in all_voices {
        let id = v.Id().map(|h| h.to_string()).unwrap_or_default();
        let lang = v
            .Language()
            .map(|h| h.to_string().to_lowercase())
            .unwrap_or_default();
        if lang.starts_with("ko") {
            has_korean_voice = true;
        }

        if !req.voice_id.is_empty() && id == req.voice_id {
            selected_voice = Some(v);
            break;
        }

        if req.voice_id.is_empty() {
            let matches = (req.lang == Lang::Ko && lang.starts_with("ko"))
                || (req.lang == Lang::En && lang.starts_with("en"));
            if matches {
                selected_voice = Some(v);
                break;
            }
        }
    }

    if selected_voice.is_none() {
        if req.lang == Lang::Ko && !has_korean_voice {
            return Err(TtsError::Config(KOREAN_VOICE_HINT.into()));
        }

        if !req.voice_id.is_empty() {
            if req.lang == Lang::Ko {
                return Err(TtsError::Config(format!(
                    "Voice '{}' not found. {}",
                    req.voice_id, KOREAN_VOICE_HINT
                )));
            } else {
                return Err(TtsError::Config(format!(
                    "Voice '{}' not found",
                    req.voice_id
                )));
            }
        }

        if req.lang == Lang::Ko {
            return Err(TtsError::Config(KOREAN_VOICE_HINT.into()));
        }
    }

    if let Some(voice) = selected_voice {
        synth
            .SetVoice(&voice)
            .map_err(|e| TtsError::Engine(format!("failed to set voice: {e}")))?;
    }

    // Configure speaking speed
    let rate = clamp_speed(req.speed);
    if let Ok(opts) = synth.Options() {
        let _ = opts.SetSpeakingRate(rate);
    }

    if cancel.is_cancelled() {
        return Err(TtsError::Cancelled);
    }

    let hstring_text = HSTRING::from(&req.text);
    let op = synth
        .SynthesizeTextToStreamAsync(&hstring_text)
        .map_err(|e| TtsError::Engine(format!("SynthesizeTextToStreamAsync failed: {e}")))?;

    let stream = op
        .join()
        .map_err(|e| TtsError::Engine(format!("async synthesis failed: {e}")))?;

    if cancel.is_cancelled() {
        return Err(TtsError::Cancelled);
    }

    let size = stream
        .Size()
        .map_err(|e| TtsError::Engine(format!("failed to get stream size: {e}")))?
        as usize;

    if size == 0 {
        return Ok(());
    }

    let reader = DataReader::CreateDataReader(&stream)
        .map_err(|e| TtsError::Engine(format!("CreateDataReader failed: {e}")))?;

    reader
        .LoadAsync(size as u32)
        .map_err(|e| TtsError::Engine(format!("LoadAsync failed: {e}")))?
        .join()
        .map_err(|e| TtsError::Engine(format!("failed to load stream bytes: {e}")))?;

    let mut buffer = vec![0u8; size];
    reader
        .ReadBytes(&mut buffer)
        .map_err(|e| TtsError::Engine(format!("ReadBytes failed: {e}")))?;

    if cancel.is_cancelled() {
        return Err(TtsError::Cancelled);
    }

    let pcm = wav_bytes_to_pcm(&buffer)?;
    on_chunk(pcm);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_metadata() {
        let engine = SystemEngine::new();
        assert_eq!(engine.id(), "system");
        assert_eq!(engine.display_name(), "Windows built-in voices");
        assert!(engine.is_offline());
        assert_eq!(engine.settings_fingerprint(), "");
    }

    #[test]
    fn speed_clamping() {
        assert_eq!(clamp_speed(1.0), 1.0);
        assert_eq!(clamp_speed(0.1), 0.5);
        assert_eq!(clamp_speed(10.0), 6.0);
    }

    #[test]
    fn cancel_is_honoured() {
        let engine = SystemEngine::new();
        let cancel = Cancel::new();
        cancel.cancel();
        let req = SynthRequest {
            text: "test".into(),
            voice_id: String::new(),
            lang: Lang::En,
            speed: 1.0,
        };
        let mut chunks = Vec::new();
        let err = engine.synthesize(&req, &cancel, &mut |c| chunks.push(c));
        assert!(matches!(err, Err(TtsError::Cancelled)));
    }

    #[test]
    fn wav_parsing_synthetic() {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 16000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut cursor = std::io::Cursor::new(Vec::new());
        {
            let mut writer = hound::WavWriter::new(&mut cursor, spec).unwrap();
            writer.write_sample(0i16).unwrap();
            writer.write_sample(16384i16).unwrap();
            writer.write_sample(-16384i16).unwrap();
            writer.finalize().unwrap();
        }
        let bytes = cursor.into_inner();
        let pcm = wav_bytes_to_pcm(&bytes).unwrap();
        assert_eq!(pcm.sample_rate, 16000);
        assert_eq!(pcm.channels, 1);
        assert_eq!(pcm.samples.len(), 3);
        assert!((pcm.samples[0] - 0.0).abs() < 1e-4);
        assert!((pcm.samples[1] - 0.5).abs() < 1e-4);
        assert!((pcm.samples[2] - (-0.5)).abs() < 1e-4);
    }

    #[test]
    #[cfg(windows)]
    fn live_voices_and_synth_english() {
        let engine = SystemEngine::new();
        let voices = match engine.voices() {
            Ok(v) => v,
            Err(e) => {
                eprintln!("Skipping: could not list voices: {e}");
                return;
            }
        };

        if voices.is_empty() {
            eprintln!("Skipping: no voices installed on this machine");
            return;
        }

        let en_voice = voices.iter().find(|v| v.langs.contains(&Lang::En));
        let Some(en_voice) = en_voice else {
            eprintln!("Skipping: no English voice installed");
            return;
        };

        let req = SynthRequest {
            text: "hello".into(),
            voice_id: en_voice.id.clone(),
            lang: Lang::En,
            speed: 1.0,
        };
        let cancel = Cancel::new();
        let mut chunks = Vec::new();
        engine
            .synthesize(&req, &cancel, &mut |chunk| chunks.push(chunk))
            .expect("Synthesis failed");

        assert!(!chunks.is_empty(), "Expected at least one PCM chunk");
        assert!(!chunks[0].samples.is_empty());
    }

    #[test]
    #[cfg(windows)]
    fn korean_voice_missing_gives_helpful_hint() {
        let engine = SystemEngine::new();
        let voices = engine.voices().unwrap_or_default();
        let has_korean = voices.iter().any(|v| v.langs.contains(&Lang::Ko));

        if !has_korean {
            let req = SynthRequest {
                text: "안녕하세요".into(),
                voice_id: String::new(),
                lang: Lang::Ko,
                speed: 1.0,
            };
            let cancel = Cancel::new();
            let mut chunks = Vec::new();
            let err = engine.synthesize(&req, &cancel, &mut |c| chunks.push(c));
            match err {
                Err(TtsError::Config(msg)) => {
                    assert!(
                        msg.contains("Korean speech pack"),
                        "Expected speech pack hint, got: {msg}"
                    );
                }
                other => panic!("Expected Config error with hint, got: {other:?}"),
            }
        }
    }
}
