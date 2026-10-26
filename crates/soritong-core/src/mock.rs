//! A hermetic engine for tests, CI and UI work: one short tone per syllable, no network, no model.

use crate::{Cancel, Lang, PcmChunk, Result, SynthRequest, TtsEngine, TtsError, Voice, VoiceKind};

#[derive(Debug, Clone)]
pub struct MockEngine {
    pub sample_rate: u32,
}

impl Default for MockEngine {
    fn default() -> Self {
        Self {
            sample_rate: 24_000,
        }
    }
}

impl TtsEngine for MockEngine {
    fn id(&self) -> &'static str {
        "mock"
    }
    fn display_name(&self) -> &str {
        "Mock (test tones)"
    }
    fn is_offline(&self) -> bool {
        true
    }
    fn voices(&self) -> Result<Vec<Voice>> {
        Ok(vec![Voice {
            id: "beep".into(),
            name: "Beep".into(),
            langs: vec![Lang::Ko, Lang::En],
            kind: VoiceKind::Stock,
        }])
    }

    /// One 120 ms tone per non-space character, emitted as one chunk per character so streaming
    /// and cancellation paths get exercised.
    fn synthesize(
        &self,
        req: &SynthRequest,
        cancel: &Cancel,
        on_chunk: &mut dyn FnMut(PcmChunk),
    ) -> Result<()> {
        if req.voice_id != "beep" {
            return Err(TtsError::Config(format!(
                "unknown voice {:?}",
                req.voice_id
            )));
        }
        let len = (self.sample_rate as f32 * 0.12 / req.speed.max(0.25)) as usize;
        for (i, _) in req.text.chars().filter(|c| !c.is_whitespace()).enumerate() {
            if cancel.is_cancelled() {
                return Err(TtsError::Cancelled);
            }
            let freq = 330.0 + 55.0 * (i % 5) as f32;
            let samples = (0..len)
                .map(|n| {
                    let t = n as f32 / self.sample_rate as f32;
                    let env = (n.min(len - n) as f32 / 240.0).min(1.0); // click-free edges
                    0.2 * env * (std::f32::consts::TAU * freq * t).sin()
                })
                .collect();
            on_chunk(PcmChunk {
                sample_rate: self.sample_rate,
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

    #[test]
    fn emits_a_chunk_per_character_and_honours_cancel() {
        let e = MockEngine::default();
        let req = SynthRequest {
            text: "안녕 하세요".into(),
            voice_id: "beep".into(),
            lang: Lang::Ko,
            speed: 1.0,
        };
        let mut n = 0;
        e.synthesize(&req, &Cancel::new(), &mut |_| n += 1).unwrap();
        assert_eq!(n, 5);

        let cancel = Cancel::new();
        cancel.cancel();
        assert!(matches!(
            e.synthesize(&req, &cancel, &mut |_| {}),
            Err(TtsError::Cancelled)
        ));
    }
}
