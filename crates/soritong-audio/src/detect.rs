//! Which render endpoints come back out as a microphone Zoom can select.
//!
//! Windows names an endpoint `"<endpoint> (<device description>)"`, e.g.
//! `"CABLE Input (VB-Audio Virtual Cable)"`. Users can rename the endpoint part, but the
//! description in parentheses comes from the driver, and a driver's render and capture endpoints
//! share it. So pairs are matched on that suffix rather than on exact strings, and a match is only
//! reported when the capture side is actually present.
//!
//! Known pairs:
//!
//! - **VB-CABLE** (and CABLE-A…D, Hi-Fi Cable): `CABLE Input (VB-Audio Virtual
//!   Cable)` → `CABLE Output (VB-Audio Virtual Cable)`. A hard loop in the driver.
//! - **Voicemeeter**: the virtual inputs feed the B buses. `Voicemeeter Input (VB-Audio Voicemeeter
//!   VAIO)` → `Voicemeeter Out B1 (…VAIO)`, `Voicemeeter AUX Input (…AUX VAIO)` → `Out B2`,
//!   `Voicemeeter VAIO3 Input (…VAIO3)` → `Out B3`. Older Voicemeeter versions name the capture side
//!   `Voicemeeter Output` / `Voicemeeter AUX Output` / `Voicemeeter VAIO3 Output`; both are matched
//!   by suffix. This is routing, not a hard loop: the strip's B1/B2/B3 button must be on in the
//!   Voicemeeter window (it is by default for the first virtual input, unverified for the others).
//! - **Stereo Mix** (`Stereo Mix`, Korean `스테레오 믹스`, German `Stereomix`, Creative `What U Hear`):
//!   records whatever the sound card plays, so every render endpoint of that card is its source.
//!   Caveat: it also carries the meeting audio Zoom plays on that card, so it echoes unless the
//!   meeting is on other headphones.
//!
//! Deliberately left out:
//!
//! - **Steam Streaming Microphone**: not a loopback of Steam Streaming Speakers.
//! - **VirtualDrivers' Virtual Audio Driver**: a SYSVAD derivative with a separate "Virtual Mic
//!   Driver"; its README does not document a speaker→mic loopback, so we do not claim one.

use crate::VirtualMic;

/// Splits `"Speakers (Realtek(R) Audio)"` into `("Speakers", Some("Realtek(R) Audio"))`.
fn split_name(name: &str) -> (&str, Option<&str>) {
    match name.trim().split_once(" (") {
        Some((endpoint, rest)) => (
            endpoint.trim(),
            Some(rest.strip_suffix(')').unwrap_or(rest)),
        ),
        None => (name.trim(), None),
    }
}

const STEREO_MIX: [&str; 4] = ["stereo mix", "스테레오 믹스", "stereomix", "what u hear"];

/// Marks `render_name` as a virtual mic when its audio comes back out of one of `capture_names`.
/// Render endpoints that cannot be opened in shared stereo mode and are hidden from the output
/// list: VB-CABLE 45's `CABLE In 16ch` fails with AUDCLNT_E_UNSUPPORTED_FORMAT, while the plain
/// `CABLE Input` feeds the same `CABLE Output`.
pub fn is_unplayable(render_name: &str) -> bool {
    render_name.starts_with("CABLE In 16ch")
}

pub fn detect_virtual_mic<S: AsRef<str>>(
    render_name: &str,
    capture_names: &[S],
) -> Option<VirtualMic> {
    let (endpoint, suffix) = split_name(render_name);
    let suffix = suffix?;
    let lower_suffix = suffix.to_lowercase();
    let captures = capture_names.iter().map(|c| c.as_ref());
    let same_suffix = || captures.clone().filter(|c| split_name(c).1 == Some(suffix));
    let mic = |family: &str, name: &str| {
        Some(VirtualMic {
            family: family.into(),
            zoom_mic_name: name.into(),
        })
    };

    if lower_suffix.contains("voicemeeter") {
        // One render endpoint per VAIO device; its capture twin is the matching B bus.
        let out = same_suffix().find(|c| {
            let e = split_name(c).0.to_lowercase();
            e.contains("out b") || e.contains("output")
        })?;
        return mic("Voicemeeter", out);
    }

    if lower_suffix.contains("vb-audio") {
        // `CABLE In 16ch` (pack 45+) feeds the same CABLE Output, but rejects its own reported
        // stereo mix format in shared mode (AUDCLNT_E_UNSUPPORTED_FORMAT), so it cannot be played to.
        if endpoint.contains("16ch") {
            return None;
        }
        let wanted = endpoint.replace("Input", "Output");
        let exact = same_suffix().find(|c| split_name(c).0.eq_ignore_ascii_case(&wanted));
        let out = exact.or_else(|| {
            same_suffix().find(|c| split_name(c).0.to_lowercase().contains("output"))
        })?;
        return mic("VB-CABLE", out);
    }

    let stereo_mix = same_suffix().find(|c| {
        let e = split_name(c).0.to_lowercase();
        STEREO_MIX.iter().any(|m| e.contains(m))
    })?;
    mic("Stereo Mix", stereo_mix)
}

/// Device ids are names, with `#2`, `#3`… appended to repeats in enumeration order, so a
/// setting survives restarts and driver reinstalls (WASAPI endpoint GUIDs do not).
pub fn assign_ids<S: AsRef<str>>(names: &[S]) -> Vec<String> {
    let mut seen = std::collections::HashMap::<&str, usize>::new();
    names
        .iter()
        .map(|n| {
            let n = n.as_ref();
            let count = seen.entry(n).or_default();
            *count += 1;
            if *count == 1 {
                n.to_string()
            } else {
                format!("{n}#{count}")
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zoom(render: &str, captures: &[&str]) -> Option<(String, String)> {
        detect_virtual_mic(render, captures).map(|m| (m.family, m.zoom_mic_name))
    }

    fn pair(family: &str, name: &str) -> Option<(String, String)> {
        Some((family.into(), name.into()))
    }

    #[test]
    fn vb_cable_and_its_variants() {
        let caps = [
            "Microphone (Realtek(R) Audio)",
            "CABLE Output (VB-Audio Virtual Cable)",
            "CABLE-A Output (VB-Audio Cable A)",
            "CABLE-B Output (VB-Audio Cable B)",
            "Hi-Fi Cable Output (VB-Audio Hi-Fi Cable)",
        ];
        assert_eq!(
            zoom("CABLE Input (VB-Audio Virtual Cable)", &caps),
            pair("VB-CABLE", "CABLE Output (VB-Audio Virtual Cable)")
        );
        assert_eq!(zoom("CABLE In 16ch (VB-Audio Virtual Cable)", &caps), None);
        assert_eq!(
            zoom("CABLE-A Input (VB-Audio Cable A)", &caps),
            pair("VB-CABLE", "CABLE-A Output (VB-Audio Cable A)")
        );
        assert_eq!(
            zoom("CABLE-B Input (VB-Audio Cable B)", &caps),
            pair("VB-CABLE", "CABLE-B Output (VB-Audio Cable B)")
        );
        assert_eq!(
            zoom("Hi-Fi Cable Input (VB-Audio Hi-Fi Cable)", &caps),
            pair("VB-CABLE", "Hi-Fi Cable Output (VB-Audio Hi-Fi Cable)")
        );
        // A renamed endpoint still pairs through the driver's description.
        assert_eq!(
            zoom("Zoom feed (VB-Audio Virtual Cable)", &caps),
            pair("VB-CABLE", "CABLE Output (VB-Audio Virtual Cable)")
        );
    }

    #[test]
    fn capture_side_must_exist() {
        assert_eq!(
            zoom(
                "CABLE-A Input (VB-Audio Cable A)",
                &["CABLE Output (VB-Audio Virtual Cable)"]
            ),
            None
        );
        assert_eq!(zoom("CABLE Input (VB-Audio Virtual Cable)", &[]), None);
        assert_eq!(
            zoom("Voicemeeter Input (VB-Audio Voicemeeter VAIO)", &[]),
            None
        );
    }

    #[test]
    fn voicemeeter_inputs_map_to_their_b_bus() {
        let caps = [
            "Voicemeeter Out B1 (VB-Audio Voicemeeter VAIO)",
            "Voicemeeter Out B2 (VB-Audio Voicemeeter AUX VAIO)",
            "Voicemeeter Out B3 (VB-Audio Voicemeeter VAIO3)",
        ];
        assert_eq!(
            zoom("Voicemeeter Input (VB-Audio Voicemeeter VAIO)", &caps),
            pair("Voicemeeter", caps[0])
        );
        assert_eq!(
            zoom(
                "Voicemeeter AUX Input (VB-Audio Voicemeeter AUX VAIO)",
                &caps
            ),
            pair("Voicemeeter", caps[1])
        );
        assert_eq!(
            zoom(
                "Voicemeeter VAIO3 Input (VB-Audio Voicemeeter VAIO3)",
                &caps
            ),
            pair("Voicemeeter", caps[2])
        );
        // Older releases call the capture side "Output".
        assert_eq!(
            zoom(
                "Voicemeeter AUX Input (VB-Audio Voicemeeter AUX VAIO)",
                &["Voicemeeter AUX Output (VB-Audio Voicemeeter AUX VAIO)"]
            ),
            pair(
                "Voicemeeter",
                "Voicemeeter AUX Output (VB-Audio Voicemeeter AUX VAIO)"
            )
        );
    }

    #[test]
    fn stereo_mix_marks_renders_of_the_same_card() {
        let caps = [
            "Microphone Array (Realtek(R) Audio)",
            "Stereo Mix (Realtek(R) Audio)",
        ];
        assert_eq!(
            zoom("Speakers (Realtek(R) Audio)", &caps),
            pair("Stereo Mix", "Stereo Mix (Realtek(R) Audio)")
        );
        assert_eq!(zoom("Speakers (USB Audio Device)", &caps), None);
        assert_eq!(
            zoom(
                "스피커 (Realtek(R) Audio)",
                &["스테레오 믹스 (Realtek(R) Audio)"]
            ),
            pair("Stereo Mix", "스테레오 믹스 (Realtek(R) Audio)")
        );
        // A card without Stereo Mix: its own mic is not a loopback.
        assert_eq!(
            zoom(
                "Speakers (Realtek(R) Audio)",
                &["Microphone Array (Realtek(R) Audio)"]
            ),
            None
        );
    }

    #[test]
    fn steam_and_virtual_audio_driver_are_not_virtual_mics() {
        assert_eq!(
            zoom(
                "Speakers (Steam Streaming Speakers)",
                &["Microphone (Steam Streaming Microphone)"]
            ),
            None
        );
        assert_eq!(
            zoom(
                "Speakers (Virtual Audio Driver)",
                &["Microphone (Virtual Mic Driver)"]
            ),
            None
        );
        assert_eq!(zoom("Speakers", &["Stereo Mix"]), None);
    }

    #[test]
    fn ids_disambiguate_repeats() {
        assert_eq!(
            assign_ids(&[
                "Speakers (USB)",
                "Headphones",
                "Speakers (USB)",
                "Speakers (USB)"
            ]),
            vec![
                "Speakers (USB)",
                "Headphones",
                "Speakers (USB)#2",
                "Speakers (USB)#3"
            ]
        );
    }
}
