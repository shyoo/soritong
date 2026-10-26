//! On-disk cache of synthesised audio, so replays (and repeated quick phrases) cost no credits.
//!
//! Stored as 32-bit float WAV, one file per key. Keys hash everything that changes the audio.

use crate::{PcmChunk, Result, SynthRequest, TtsEngine, TtsError};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

pub fn cache_key(engine: &dyn TtsEngine, req: &SynthRequest) -> String {
    let mut h = Sha256::new();
    for part in [
        engine.id(),
        &engine.settings_fingerprint(),
        &req.voice_id,
        req.lang.code(),
        &format!("{:.3}", req.speed),
        &req.text,
    ] {
        h.update(part.as_bytes());
        h.update([0u8]); // separator, so ("ab","c") != ("a","bc")
    }
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

#[derive(Debug, Clone)]
pub struct SynthCache {
    dir: PathBuf,
}

impl SynthCache {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn path(&self, key: &str) -> PathBuf {
        self.dir.join(format!("{key}.wav"))
    }

    pub fn get(&self, key: &str) -> Option<PcmChunk> {
        let mut r = hound::WavReader::open(self.path(key)).ok()?;
        let spec = r.spec();
        let samples = r
            .samples::<f32>()
            .collect::<std::result::Result<Vec<_>, _>>()
            .ok()?;
        Some(PcmChunk {
            sample_rate: spec.sample_rate,
            channels: spec.channels,
            samples,
        })
    }

    /// Writes to a temp file then renames, so a crash never leaves a truncated entry.
    pub fn put(&self, key: &str, pcm: &PcmChunk) -> Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let tmp = self.dir.join(format!("{key}.wav.tmp"));
        write_wav(&tmp, pcm)?;
        std::fs::rename(&tmp, self.path(key))?;
        Ok(())
    }
}

pub fn write_wav(path: &Path, pcm: &PcmChunk) -> Result<()> {
    let spec = hound::WavSpec {
        channels: pcm.channels,
        sample_rate: pcm.sample_rate,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let wav_err = |e: hound::Error| TtsError::Engine(format!("wav: {e}"));
    let mut w = hound::WavWriter::create(path, spec).map_err(wav_err)?;
    for &s in &pcm.samples {
        w.write_sample(s).map_err(wav_err)?;
    }
    w.finalize().map_err(wav_err)
}

/// Concatenates chunks of one utterance (all chunks of one synthesis share rate and channels).
pub fn concat(chunks: &[PcmChunk]) -> Option<PcmChunk> {
    let first = chunks.first()?;
    Some(PcmChunk {
        sample_rate: first.sample_rate,
        channels: first.channels,
        samples: chunks
            .iter()
            .flat_map(|c| c.samples.iter().copied())
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Lang, mock::MockEngine};

    fn req(text: &str) -> SynthRequest {
        SynthRequest {
            text: text.into(),
            voice_id: "beep".into(),
            lang: Lang::Ko,
            speed: 1.0,
        }
    }

    #[test]
    fn key_changes_with_text_and_is_stable() {
        let e = MockEngine::default();
        assert_eq!(cache_key(&e, &req("안녕")), cache_key(&e, &req("안녕")));
        assert_ne!(cache_key(&e, &req("안녕")), cache_key(&e, &req("안녕!")));
    }

    #[test]
    fn round_trips_through_disk() {
        let dir = std::env::temp_dir().join(format!("soritong-cache-test-{}", std::process::id()));
        let cache = SynthCache::new(&dir);
        let pcm = PcmChunk {
            sample_rate: 24_000,
            channels: 1,
            samples: vec![0.0, 0.5, -0.5],
        };
        assert!(cache.get("k").is_none());
        cache.put("k", &pcm).unwrap();
        assert_eq!(cache.get("k"), Some(pcm));
        std::fs::remove_dir_all(dir).ok();
    }
}
