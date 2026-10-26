//! ElevenLabs HTTP API client, request formatting, response parsing, and error mapping.

use crate::ElevenLabsConfig;
use serde::{Deserialize, Serialize};
use soritong_core::{Lang, Result, SynthRequest, TtsError, Voice, VoiceKind};
use std::collections::HashMap;

/// Redact any occurrence of `api_key` in a string to avoid leaking secrets in errors or logs.
pub fn sanitize_text(text: &str, api_key: &str) -> String {
    if !api_key.trim().is_empty() {
        text.replace(api_key, "<redacted>")
    } else {
        text.to_string()
    }
}

/// Map HTTP status code and response body to a [`TtsError`].
pub fn map_http_error(status: u16, body_str: &str, api_key: &str) -> TtsError {
    // 401 is always invalid/missing API key
    if status == 401 {
        return TtsError::Config("ElevenLabs API key is missing or invalid".to_string());
    }

    let parsed: Option<serde_json::Value> = serde_json::from_str(body_str).ok();
    let mut message = None;
    let mut code_or_status = None;

    if let Some(val) = &parsed {
        if let Some(d) = val.get("detail") {
            if let Some(s) = d.as_str() {
                message = Some(s.to_string());
            } else if let Some(obj) = d.as_object() {
                if let Some(m) = obj.get("message").and_then(|v| v.as_str()) {
                    message = Some(m.to_string());
                }
                if let Some(st) = obj.get("status").and_then(|v| v.as_str()) {
                    code_or_status = Some(st.to_string());
                } else if let Some(c) = obj.get("code").and_then(|v| v.as_str()) {
                    code_or_status = Some(c.to_string());
                }
            }
        } else if let Some(m) = val.get("message").and_then(|v| v.as_str()) {
            message = Some(m.to_string());
        }
    }

    let raw_msg = message
        .or_else(|| {
            let trimmed = body_str.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        })
        .unwrap_or_else(|| format!("HTTP status {status}"));

    let sanitized_msg = sanitize_text(&raw_msg, api_key);
    let code_str = code_or_status.unwrap_or_default().to_lowercase();
    let lower_msg = sanitized_msg.to_lowercase();

    // Quota or payment problems (402 or quota_exceeded)
    if status == 402 || code_str == "quota_exceeded" || lower_msg.contains("quota_exceeded") {
        return TtsError::Config(sanitized_msg);
    }

    // Unknown voice
    if code_str == "voice_not_found"
        || lower_msg.contains("voice_not_found")
        || lower_msg.contains("voice not found")
        || (status == 404 && lower_msg.contains("voice"))
    {
        return TtsError::Config(sanitized_msg);
    }

    // 429 or 5xx server errors are network-level transient/capacity issues
    if status == 429 || (500..=599).contains(&status) {
        return TtsError::Network(format!("HTTP {status}: {sanitized_msg}"));
    }

    // Other 4xx client errors (e.g. invalid arguments, unaccepted model) are config/user errors
    if (400..=499).contains(&status) {
        return TtsError::Config(sanitized_msg);
    }

    TtsError::Engine(format!("HTTP {status}: {sanitized_msg}"))
}

#[derive(Debug, Deserialize)]
pub struct VoicesApiResponse {
    pub voices: Vec<VoiceApiItem>,
}

#[derive(Debug, Deserialize)]
pub struct VoiceApiItem {
    pub voice_id: String,
    pub name: String,
    pub category: Option<String>,
    pub labels: Option<HashMap<String, String>>,
    pub verified_languages: Option<Vec<VerifiedLanguageItem>>,
}

#[derive(Debug, Deserialize)]
pub struct VerifiedLanguageItem {
    pub language: Option<String>,
    pub locale: Option<String>,
    pub accent: Option<String>,
}

fn parse_lang_tag(s: &str) -> Option<Lang> {
    let lower = s.trim().to_lowercase();
    if lower == "ko"
        || lower.starts_with("ko-")
        || lower.starts_with("ko_")
        || lower == "korean"
        || lower == "kor"
        || lower == "한국어"
    {
        Some(Lang::Ko)
    } else if lower == "en"
        || lower.starts_with("en-")
        || lower.starts_with("en_")
        || lower == "english"
        || lower == "eng"
        || lower == "영어"
    {
        Some(Lang::En)
    } else {
        None
    }
}

/// Parse a raw JSON response from `GET /v1/voices` into a sorted list of [`Voice`]s.
/// Cloned voices (`category` = "cloned" or "professional") are sorted first.
pub fn parse_voices_json(json_str: &str) -> Result<Vec<Voice>> {
    let resp: VoicesApiResponse = serde_json::from_str(json_str)
        .map_err(|e| TtsError::Engine(format!("Failed to parse voices JSON: {e}")))?;

    let mut voices = Vec::with_capacity(resp.voices.len());
    for item in resp.voices {
        let kind = match item.category.as_deref().map(str::to_lowercase).as_deref() {
            Some("cloned") | Some("professional") => VoiceKind::Cloned,
            _ => VoiceKind::Stock,
        };

        let mut langs = Vec::new();
        if let Some(verified) = &item.verified_languages {
            for v in verified {
                if let Some(l) = v.language.as_deref().and_then(parse_lang_tag) {
                    if !langs.contains(&l) {
                        langs.push(l);
                    }
                }
                if let Some(l) = v.locale.as_deref().and_then(parse_lang_tag) {
                    if !langs.contains(&l) {
                        langs.push(l);
                    }
                }
            }
        }

        if let Some(labels) = &item.labels {
            for (k, v) in labels {
                if k.eq_ignore_ascii_case("language")
                    || k.eq_ignore_ascii_case("lang")
                    || k.eq_ignore_ascii_case("accent")
                {
                    if let Some(l) = parse_lang_tag(v) {
                        if !langs.contains(&l) {
                            langs.push(l);
                        }
                    }
                }
            }
        }

        voices.push(Voice {
            id: item.voice_id,
            name: item.name,
            langs,
            kind,
        });
    }

    // Sort cloned voices first, then alphabetically by voice name
    voices.sort_by(|a, b| match (a.kind, b.kind) {
        (VoiceKind::Cloned, VoiceKind::Stock) => std::cmp::Ordering::Less,
        (VoiceKind::Stock, VoiceKind::Cloned) => std::cmp::Ordering::Greater,
        _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
    });

    Ok(voices)
}

/// Resolves `language_code` for the ElevenLabs request.
/// Returns None for Lang::Auto or for models that do not support language_code (such as monolingual models).
pub fn resolve_language_code(model_id: &str, lang: Lang) -> Option<&'static str> {
    if model_id.contains("monolingual")
        || model_id == "eleven_turbo_v2"
        || model_id == "eleven_flash_v2"
    {
        return None;
    }
    match lang {
        Lang::Ko => Some("ko"),
        Lang::En => Some("en"),
        Lang::Auto => None,
    }
}

#[derive(Debug, Serialize)]
pub struct SynthBody<'a> {
    pub text: &'a str,
    pub model_id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language_code: Option<&'static str>,
    pub voice_settings: VoiceSettings,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct VoiceSettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stability: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub similarity_boost: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub style: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub use_speaker_boost: Option<bool>,
    pub speed: f32,
}

pub fn build_synth_body<'a>(cfg: &'a ElevenLabsConfig, req: &'a SynthRequest) -> SynthBody<'a> {
    let speed = req.speed.clamp(0.7, 1.2);
    let language_code = resolve_language_code(&cfg.model_id, req.lang);
    let voice_settings = VoiceSettings {
        stability: cfg.stability,
        similarity_boost: cfg.similarity_boost,
        style: cfg.style,
        use_speaker_boost: cfg.use_speaker_boost,
        speed,
    };

    SynthBody {
        text: &req.text,
        model_id: &cfg.model_id,
        language_code,
        voice_settings,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_mapping() {
        let key = "sk-secret12345";

        // 401 Unauthorized
        let err = map_http_error(401, r#"{"detail":"Unauthorized"}"#, key);
        match err {
            TtsError::Config(msg) => {
                assert_eq!(msg, "ElevenLabs API key is missing or invalid");
                assert!(!msg.contains(key));
            }
            other => panic!("expected Config error, got {other:?}"),
        }

        // 402 Quota exceeded
        let err = map_http_error(
            402,
            r#"{"detail":{"status":"quota_exceeded","message":"Your character quota is exhausted"}}"#,
            key,
        );
        match err {
            TtsError::Config(msg) => {
                assert!(msg.contains("quota is exhausted"));
            }
            other => panic!("expected Config error, got {other:?}"),
        }

        // 400 with quota_exceeded code
        let err = map_http_error(
            400,
            r#"{"detail":{"code":"quota_exceeded","message":"Quota exceeded"}}"#,
            key,
        );
        match err {
            TtsError::Config(msg) => {
                assert!(msg.contains("Quota exceeded"));
            }
            other => panic!("expected Config error, got {other:?}"),
        }

        // 400 with voice_not_found
        let err = map_http_error(
            400,
            r#"{"detail":{"status":"voice_not_found","message":"Voice ID not found"}}"#,
            key,
        );
        match err {
            TtsError::Config(msg) => {
                assert!(msg.contains("Voice ID not found"));
            }
            other => panic!("expected Config error, got {other:?}"),
        }

        // 429 Rate limited
        let err = map_http_error(429, r#"{"detail":{"message":"Too many requests"}}"#, key);
        match err {
            TtsError::Network(msg) => {
                assert!(msg.contains("429"));
                assert!(msg.contains("Too many requests"));
            }
            other => panic!("expected Network error, got {other:?}"),
        }

        // 503 Service unavailable
        let err = map_http_error(503, "Service Unavailable", key);
        match err {
            TtsError::Network(msg) => {
                assert!(msg.contains("503"));
            }
            other => panic!("expected Network error, got {other:?}"),
        }

        // Ensure key is never present even if echoed in server response
        let err = map_http_error(
            400,
            &format!(r#"{{"detail":{{"message":"Bad key {key}"}}}}"#),
            key,
        );
        match err {
            TtsError::Config(msg) => {
                assert!(!msg.contains(key));
                assert!(msg.contains("<redacted>"));
            }
            other => panic!("expected Config error, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_voices_fixture() {
        let fixture = r#"{
            "voices": [
                {
                    "voice_id": "stock1",
                    "name": "Rachel",
                    "category": "premade",
                    "labels": {"accent": "american", "language": "en"},
                    "verified_languages": [
                        {"language": "en", "locale": "en-US"}
                    ]
                },
                {
                    "voice_id": "clone1",
                    "name": "My Cloned Voice",
                    "category": "cloned",
                    "labels": {"language": "korean"},
                    "verified_languages": [
                        {"language": "ko", "locale": "ko-KR"}
                    ]
                },
                {
                    "voice_id": "prof1",
                    "name": "Adam Cloned Pro",
                    "category": "professional",
                    "labels": {},
                    "verified_languages": []
                }
            ]
        }"#;

        let voices = parse_voices_json(fixture).unwrap();
        assert_eq!(voices.len(), 3);

        // Cloned voices must be sorted first!
        assert_eq!(voices[0].kind, VoiceKind::Cloned);
        assert_eq!(voices[0].id, "prof1");
        assert_eq!(voices[0].name, "Adam Cloned Pro");
        assert_eq!(voices[0].langs, Vec::<Lang>::new());

        assert_eq!(voices[1].kind, VoiceKind::Cloned);
        assert_eq!(voices[1].id, "clone1");
        assert_eq!(voices[1].name, "My Cloned Voice");
        assert_eq!(voices[1].langs, vec![Lang::Ko]);

        // Stock voices second
        assert_eq!(voices[2].kind, VoiceKind::Stock);
        assert_eq!(voices[2].id, "stock1");
        assert_eq!(voices[2].name, "Rachel");
        assert_eq!(voices[2].langs, vec![Lang::En]);
    }
}
