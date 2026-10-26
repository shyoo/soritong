//! Turning what the user typed into a [`SynthRequest`].

use soritong_core::{Lang, SynthRequest, Voice};

/// Normalises line endings and surrounding whitespace, and refuses empty text or a missing
/// voice. Language is always `Auto`: messages mix Korean and English freely.
pub fn build_request(text: &str, voice_id: &str, speed: f32) -> Result<SynthRequest, String> {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let text = text.trim();
    if text.is_empty() {
        return Err("nothing to say".into());
    }
    if voice_id.trim().is_empty() {
        return Err("no voice selected".into());
    }
    let speed = if speed.is_finite() {
        speed.clamp(0.5, 2.0)
    } else {
        1.0
    };
    Ok(SynthRequest {
        text: text.to_string(),
        voice_id: voice_id.to_string(),
        lang: Lang::Auto,
        speed,
    })
}

/// The saved voice if the engine still offers it, else the engine's first voice.
pub fn pick_voice<'a>(voices: &'a [Voice], saved: Option<&str>) -> Option<&'a str> {
    saved
        .and_then(|s| voices.iter().find(|v| v.id == s))
        .or_else(|| voices.first())
        .map(|v| v.id.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use soritong_core::VoiceKind;

    #[test]
    fn builds_an_auto_language_request_from_trimmed_text() {
        let r = build_request("  오늘 책 정말\r\n재밌었어요.\n", "beep", 1.0).unwrap();
        assert_eq!(r.text, "오늘 책 정말\n재밌었어요.");
        assert_eq!(r.voice_id, "beep");
        assert_eq!(r.lang, Lang::Auto);
        assert_eq!(r.speed, 1.0);
    }

    #[test]
    fn refuses_blank_text_and_missing_voice_and_bounds_speed() {
        assert!(build_request(" \n\t", "beep", 1.0).is_err());
        assert!(build_request("네", "", 1.0).is_err());
        assert_eq!(build_request("네", "beep", 9.0).unwrap().speed, 2.0);
        assert_eq!(build_request("네", "beep", f32::NAN).unwrap().speed, 1.0);
    }

    #[test]
    fn picks_the_saved_voice_or_falls_back_to_the_first() {
        let v = |id: &str| Voice {
            id: id.into(),
            name: id.into(),
            langs: vec![],
            kind: VoiceKind::Stock,
        };
        let voices = [v("a"), v("b")];
        assert_eq!(pick_voice(&voices, Some("b")), Some("b"));
        assert_eq!(pick_voice(&voices, Some("gone")), Some("a"));
        assert_eq!(pick_voice(&voices, None), Some("a"));
        assert_eq!(pick_voice(&[], Some("a")), None);
    }
}
