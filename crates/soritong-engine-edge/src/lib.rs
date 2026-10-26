//! Microsoft Edge "Read aloud" online text-to-speech engine (free, no API key, unofficial).
//!
//! Connects to Microsoft's Read Aloud WebSocket service using the Edge browser client protocol,
//! including the `Sec-MS-GEC` token generation required since late 2024 / 2026.
//!
//! # Requirements & Limits
//! - Requires internet connectivity. Text is sent over TLS to `speech.platform.bing.com`.
//! - No API key is required.
//! - **Unofficial**: Microsoft may update or rate-limit this endpoint at any time.
//!   Audio format is 24 kHz MP3 (`audio-24khz-48kbitrate-mono-mp3`), decoded to 24 kHz
//!   mono float PCM using [`symphonia`] as frames stream in.

pub mod drm;

use serde::Deserialize;
use soritong_core::{
    Cancel, Lang, PcmChunk, Result, SynthRequest, TtsEngine, TtsError, Voice, VoiceKind,
};
use std::io::Cursor;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{CODEC_TYPE_NULL, DecoderOptions};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use tungstenite::client::IntoClientRequest;
use tungstenite::http::header;

pub const VOICE_LIST_URL: &str = "https://speech.platform.bing.com/consumer/speech/synthesize/readaloud/voices/list?trustedclienttoken=6A5AA1D4EAFF4E9FB37E23D68491D6F4";

pub const WSS_BASE_URL: &str = "wss://speech.platform.bing.com/consumer/speech/synthesize/readaloud/edge/v1?TrustedClientToken=6A5AA1D4EAFF4E9FB37E23D68491D6F4";

pub const CHROMIUM_FULL_VERSION: &str = "143.0.3650.75";
pub const CHROMIUM_MAJOR_VERSION: &str = "143";
pub const SEC_MS_GEC_VERSION: &str = "2-143.0.3650.75";

pub const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/143.0.0.0 Safari/537.36 Edg/143.0.0.0";
pub const ORIGIN: &str = "chrome-extension://jdiccldimpdaibmpdkjnbmckianbfold";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct EdgeVoiceItem {
    name: String,
    short_name: String,
    locale: String,
    friendly_name: Option<String>,
}

#[derive(Debug, Default, Clone)]
pub struct EdgeEngine;

impl EdgeEngine {
    pub fn new() -> Self {
        Self
    }
}

impl TtsEngine for EdgeEngine {
    fn id(&self) -> &'static str {
        "edge"
    }

    fn display_name(&self) -> &str {
        "Microsoft Edge voices (online, unofficial)"
    }

    fn is_offline(&self) -> bool {
        false
    }

    fn voices(&self) -> Result<Vec<Voice>> {
        fetch_edge_voices()
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

        synthesize_edge(req, cancel, on_chunk)
    }
}

/// Converts a short voice name (e.g. `ko-KR-SunHiNeural`) to the full server voice name
/// required in the SSML `<voice name="...">` tag.
pub fn short_name_to_server_name(voice: &str) -> String {
    if voice.starts_with("Microsoft Server Speech Text to Speech Voice") {
        return voice.to_string();
    }

    let parts: Vec<&str> = voice.split('-').collect();
    if parts.len() >= 3 {
        let lang = parts[0];
        let region = parts[1..parts.len() - 1].join("-");
        let name = parts[parts.len() - 1];
        format!("Microsoft Server Speech Text to Speech Voice ({lang}-{region}, {name})")
    } else {
        voice.to_string()
    }
}

/// Escapes XML special characters for safe inclusion in SSML text.
pub fn escape_xml(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for c in input.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            other => out.push(other),
        }
    }
    out
}

/// Maps a float speed multiplier to an SSML prosody rate string (e.g. `1.0` -> `+0%`, `1.25` -> `+25%`).
pub fn speed_to_prosody_rate(speed: f32) -> String {
    let pct = ((speed - 1.0) * 100.0).round() as i32;
    format!("{pct:+}%")
}

/// Formats UTC timestamp in JavaScript's `Date().toString()` format expected by Edge TTS.
pub fn date_to_string() -> String {
    chrono::Utc::now()
        .format("%a %b %d %Y %H:%M:%S GMT+0000 (Coordinated Universal Time)")
        .to_string()
}

/// Builds the `speech.config` message text.
pub fn build_config_message(ts: &str) -> String {
    format!(
        "X-Timestamp:{ts}\r\nContent-Type:application/json; charset=utf-8\r\nPath:speech.config\r\n\r\n{{\"context\":{{\"synthesis\":{{\"audio\":{{\"metadataoptions\":{{\"sentenceBoundaryEnabled\":\"true\",\"wordBoundaryEnabled\":\"false\"}},\"outputFormat\":\"audio-24khz-48kbitrate-mono-mp3\"}}}}}}}}\r\n"
    )
}

/// Builds the `ssml` message text.
pub fn build_ssml_message(
    req_id: &str,
    ts: &str,
    server_voice: &str,
    prosody_rate: &str,
    text: &str,
) -> String {
    let escaped = escape_xml(text);
    let ssml = format!(
        "<speak version='1.0' xmlns='http://www.w3.org/2001/10/synthesis' xml:lang='en-US'><voice name='{server_voice}'><prosody pitch='+0Hz' rate='{prosody_rate}' volume='+0%'>{escaped}</prosody></voice></speak>"
    );
    format!(
        "X-RequestId:{req_id}\r\nContent-Type:application/ssml+xml\r\nX-Timestamp:{ts}Z\r\nPath:ssml\r\n\r\n{ssml}"
    )
}

/// Extracts the audio payload from a binary WebSocket message, skipping the 2-byte header length.
pub fn parse_binary_audio_frame(bytes: &[u8]) -> Option<&[u8]> {
    if bytes.len() < 2 {
        return None;
    }
    let header_len = u16::from_be_bytes([bytes[0], bytes[1]]) as usize;
    if bytes.len() >= 2 + header_len {
        Some(&bytes[2 + header_len..])
    } else {
        None
    }
}

/// Decodes MP3 bytes into a [`PcmChunk`] using `symphonia`.
pub fn decode_mp3_to_pcm(mp3_bytes: &[u8]) -> Result<PcmChunk> {
    if mp3_bytes.is_empty() {
        return Ok(PcmChunk {
            sample_rate: 24000,
            channels: 1,
            samples: Vec::new(),
        });
    }

    let cursor = Cursor::new(mp3_bytes.to_vec());
    let mss = MediaSourceStream::new(Box::new(cursor), Default::default());
    let mut hint = Hint::new();
    hint.with_extension("mp3");

    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|e| TtsError::Engine(format!("symphonia probe: {e}")))?;

    let mut format = probed.format;
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or_else(|| TtsError::Engine("no audio track found in mp3".into()))?;

    let sample_rate = track.codec_params.sample_rate.unwrap_or(24000);
    let channels = track
        .codec_params
        .channels
        .map(|c| c.count() as u16)
        .unwrap_or(1);

    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| TtsError::Engine(format!("symphonia decoder: {e}")))?;

    let mut samples: Vec<f32> = Vec::new();
    let mut sample_buf: Option<SampleBuffer<f32>> = None;

    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(SymphoniaError::IoError(ref e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break;
            }
            Err(SymphoniaError::ResetRequired) => break,
            Err(e) => return Err(TtsError::Engine(format!("symphonia packet: {e}"))),
        };

        match decoder.decode(&packet) {
            Ok(audio_buf) => {
                let spec = *audio_buf.spec();
                let duration = audio_buf.capacity() as u64;
                let buf = sample_buf.get_or_insert_with(|| SampleBuffer::new(duration, spec));
                buf.copy_interleaved_ref(audio_buf);
                samples.extend_from_slice(buf.samples());
            }
            Err(SymphoniaError::IoError(ref e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break;
            }
            Err(SymphoniaError::DecodeError(_)) => {
                continue;
            }
            Err(e) => return Err(TtsError::Engine(format!("symphonia decode: {e}"))),
        }
    }

    Ok(PcmChunk {
        sample_rate,
        channels,
        samples,
    })
}

fn fetch_edge_voices() -> Result<Vec<Voice>> {
    let response = ureq::get(VOICE_LIST_URL)
        .header("User-Agent", USER_AGENT)
        .header("Authority", "speech.platform.bing.com")
        .call()
        .map_err(|e| TtsError::Network(format!("failed to fetch edge voice list: {e}")))?;

    let mut reader = response.into_body().into_reader();
    let items: Vec<EdgeVoiceItem> = serde_json::from_reader(&mut reader)
        .map_err(|e| TtsError::Engine(format!("failed to parse edge voice list: {e}")))?;

    let mut ko_voices = Vec::new();
    let mut en_voices = Vec::new();
    let mut other_voices = Vec::new();

    for item in items {
        let mut langs = Vec::new();
        let locale_lower = item.locale.to_lowercase();
        if locale_lower.starts_with("ko") {
            langs.push(Lang::Ko);
        } else if locale_lower.starts_with("en") {
            langs.push(Lang::En);
        }

        let voice = Voice {
            id: item.short_name,
            name: item.friendly_name.unwrap_or_else(|| item.name.clone()),
            langs,
            kind: VoiceKind::Stock,
        };

        if locale_lower == "ko-kr" || locale_lower.starts_with("ko") {
            ko_voices.push(voice);
        } else if locale_lower == "en-us" || locale_lower.starts_with("en") {
            en_voices.push(voice);
        } else {
            other_voices.push(voice);
        }
    }

    let mut result = Vec::with_capacity(ko_voices.len() + en_voices.len() + other_voices.len());
    result.extend(ko_voices);
    result.extend(en_voices);
    result.extend(other_voices);

    Ok(result)
}

fn synthesize_edge(
    req: &SynthRequest,
    cancel: &Cancel,
    on_chunk: &mut dyn FnMut(PcmChunk),
) -> Result<()> {
    let voice_id = if req.voice_id.is_empty() {
        if req.lang == Lang::Ko {
            "ko-KR-SunHiNeural"
        } else {
            "en-US-AriaNeural"
        }
    } else {
        &req.voice_id
    };

    let server_voice = short_name_to_server_name(voice_id);
    let prosody_rate = speed_to_prosody_rate(req.speed);
    let conn_id = uuid::Uuid::new_v4().simple().to_string();
    let sec_ms_gec = drm::generate_sec_ms_gec();

    let url = format!(
        "{WSS_BASE_URL}&ConnectionId={conn_id}&Sec-MS-GEC={sec_ms_gec}&Sec-MS-GEC-Version={SEC_MS_GEC_VERSION}"
    );

    let mut request = url
        .into_client_request()
        .map_err(|e| TtsError::Network(format!("invalid websocket request: {e}")))?;

    let headers = request.headers_mut();
    headers.insert(header::PRAGMA, "no-cache".parse().unwrap());
    headers.insert(header::CACHE_CONTROL, "no-cache".parse().unwrap());
    headers.insert(header::USER_AGENT, USER_AGENT.parse().unwrap());
    headers.insert(header::ORIGIN, ORIGIN.parse().unwrap());
    headers.insert(
        header::ACCEPT_ENCODING,
        "gzip, deflate, br, zstd".parse().unwrap(),
    );
    headers.insert(header::ACCEPT_LANGUAGE, "en-US,en;q=0.9".parse().unwrap());

    let muid = uuid::Uuid::new_v4().simple().to_string().to_uppercase();
    headers.insert(header::COOKIE, format!("muid={muid};").parse().unwrap());

    if cancel.is_cancelled() {
        return Err(TtsError::Cancelled);
    }

    let (mut socket, _) = tungstenite::connect(request)
        .map_err(|e| TtsError::Network(format!("websocket connection failed: {e}")))?;

    let ts = date_to_string();
    let config_msg = build_config_message(&ts);
    socket
        .send(tungstenite::Message::Text(config_msg.into()))
        .map_err(|e| TtsError::Network(format!("failed to send speech.config: {e}")))?;

    let req_id = uuid::Uuid::new_v4().simple().to_string();
    let ssml_msg = build_ssml_message(&req_id, &ts, &server_voice, &prosody_rate, &req.text);
    socket
        .send(tungstenite::Message::Text(ssml_msg.into()))
        .map_err(|e| TtsError::Network(format!("failed to send ssml: {e}")))?;

    let mut all_audio_bytes = Vec::new();

    loop {
        if cancel.is_cancelled() {
            let _ = socket.close(None);
            return Err(TtsError::Cancelled);
        }

        let msg = match socket.read() {
            Ok(msg) => msg,
            Err(tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed) => {
                break;
            }
            Err(e) => {
                return Err(TtsError::Network(format!("websocket read error: {e}")));
            }
        };

        match msg {
            tungstenite::Message::Text(t) => {
                if t.contains("turn.end") {
                    break;
                }
            }
            tungstenite::Message::Binary(bin) => {
                if let Some(payload) = parse_binary_audio_frame(&bin)
                    && !payload.is_empty()
                {
                    all_audio_bytes.extend_from_slice(payload);
                }
            }
            tungstenite::Message::Close(_) => {
                break;
            }
            _ => {}
        }
    }

    if cancel.is_cancelled() {
        return Err(TtsError::Cancelled);
    }

    if all_audio_bytes.is_empty() {
        return Err(TtsError::Engine(
            "no audio received from edge tts service".into(),
        ));
    }

    let pcm = decode_mp3_to_pcm(&all_audio_bytes)?;
    on_chunk(pcm);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_metadata() {
        let engine = EdgeEngine::new();
        assert_eq!(engine.id(), "edge");
        assert_eq!(
            engine.display_name(),
            "Microsoft Edge voices (online, unofficial)"
        );
        assert!(!engine.is_offline());
        assert_eq!(engine.settings_fingerprint(), "");
    }

    #[test]
    fn xml_escaping() {
        let unescaped = "<hello & world> \"test\" 'quote'";
        let escaped = escape_xml(unescaped);
        assert_eq!(
            escaped,
            "&lt;hello &amp; world&gt; &quot;test&quot; &apos;quote&apos;"
        );
    }

    #[test]
    fn speed_to_prosody() {
        assert_eq!(speed_to_prosody_rate(1.0), "+0%");
        assert_eq!(speed_to_prosody_rate(1.2), "+20%");
        assert_eq!(speed_to_prosody_rate(0.8), "-20%");
        assert_eq!(speed_to_prosody_rate(1.5), "+50%");
    }

    #[test]
    fn short_name_conversion() {
        assert_eq!(
            short_name_to_server_name("ko-KR-SunHiNeural"),
            "Microsoft Server Speech Text to Speech Voice (ko-KR, SunHiNeural)"
        );
        assert_eq!(
            short_name_to_server_name("ko-KR-HyunsuMultilingualNeural"),
            "Microsoft Server Speech Text to Speech Voice (ko-KR, HyunsuMultilingualNeural)"
        );
        assert_eq!(
            short_name_to_server_name("en-US-AriaNeural"),
            "Microsoft Server Speech Text to Speech Voice (en-US, AriaNeural)"
        );
        let full = "Microsoft Server Speech Text to Speech Voice (ko-KR, InJoonNeural)";
        assert_eq!(short_name_to_server_name(full), full);
    }

    #[test]
    fn message_framing_format() {
        let ts = "Mon Oct 26 2026 00:00:00 GMT+0000 (Coordinated Universal Time)";
        let cfg = build_config_message(ts);
        assert!(cfg.starts_with(&format!("X-Timestamp:{ts}\r\n")));
        assert!(cfg.contains("Path:speech.config\r\n\r\n"));
        assert!(cfg.contains("audio-24khz-48kbitrate-mono-mp3"));

        let ssml_msg = build_ssml_message("req123", ts, "ServerVoice", "+10%", "안녕 & 하세요");
        assert!(ssml_msg.starts_with("X-RequestId:req123\r\n"));
        assert!(ssml_msg.contains(&format!("X-Timestamp:{ts}Z\r\n")));
        assert!(ssml_msg.contains("Path:ssml\r\n\r\n"));
        assert!(ssml_msg.contains("<voice name='ServerVoice'>"));
        assert!(ssml_msg.contains("rate='+10%'"));
        assert!(ssml_msg.contains("안녕 &amp; 하세요"));
    }

    #[test]
    fn binary_audio_frame_parsing() {
        let mut frame = Vec::new();
        let header = b"Path:audio\r\nX-RequestId:abc\r\n\r\n";
        let header_len = header.len() as u16;
        frame.extend_from_slice(&header_len.to_be_bytes());
        frame.extend_from_slice(header);
        let payload = b"FAKE_AUDIO_BYTES_HERE";
        frame.extend_from_slice(payload);

        let parsed = parse_binary_audio_frame(&frame);
        assert_eq!(parsed, Some(payload.as_slice()));

        // Truncated frame
        assert_eq!(parse_binary_audio_frame(&[0u8]), None);
        assert_eq!(parse_binary_audio_frame(&[0, 5, 1, 2]), None);
    }

    #[test]
    fn cancel_sends_cancelled_error() {
        let engine = EdgeEngine::new();
        let cancel = Cancel::new();
        cancel.cancel();
        let req = SynthRequest {
            text: "test".into(),
            voice_id: "en-US-AriaNeural".into(),
            lang: Lang::En,
            speed: 1.0,
        };
        let mut chunks = Vec::new();
        let err = engine.synthesize(&req, &cancel, &mut |c| chunks.push(c));
        assert!(matches!(err, Err(TtsError::Cancelled)));
    }

    #[test]
    #[ignore]
    fn live_korean_synthesis_works() {
        let engine = EdgeEngine::new();
        let req = SynthRequest {
            text: "안녕하세요, 소리통 엣지 한국어 음성 테스트입니다.".into(),
            voice_id: "ko-KR-SunHiNeural".into(),
            lang: Lang::Ko,
            speed: 1.0,
        };
        let cancel = Cancel::new();
        let mut chunks = Vec::new();
        engine
            .synthesize(&req, &cancel, &mut |chunk| chunks.push(chunk))
            .expect("Live Edge synthesis failed");

        assert!(!chunks.is_empty(), "Expected at least one PCM chunk");
        assert_eq!(chunks[0].sample_rate, 24000);
        assert_eq!(chunks[0].channels, 1);
        assert!(!chunks[0].samples.is_empty());
    }
}
