//! Output devices, playback and the speak queue.
//!
//! - [`list_outputs`]: render endpoints via cpal (WASAPI on Windows), each marked with the Zoom
//!   microphone it feeds when it is a known loopback ([`detect_virtual_mic`]).
//! - [`Player`]: plays any-rate PCM to the mic output and an optional monitor output.
//! - [`Speaker`]: the utterance queue (sentence split, per-sentence language, cache, playback).
//!
//! Device ids are endpoint names, with `#2`, `#3`… for repeats; see [`detect::assign_ids`].

pub mod convert;
pub mod defaults;
pub mod detect;
pub mod loopback;
mod player;
mod speaker;

pub use detect::detect_virtual_mic;
pub use player::Player;
pub use speaker::{SpeakEvent, Speaker, UtteranceId};

use cpal::traits::{DeviceTrait, HostTrait};
use serde::Serialize;
use std::sync::{Mutex, MutexGuard};

#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    #[error("audio device not found: {0}")]
    DeviceNotFound(String),
    #[error("audio backend: {0}")]
    Backend(String),
}

/// A render (playback) endpoint the app can write to.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OutputDevice {
    /// Stable across restarts where the backend allows; stored in settings.
    pub id: String,
    pub name: String,
    pub is_default: bool,
    /// Set when this render endpoint feeds a capture endpoint Zoom can select as a microphone.
    pub virtual_mic: Option<VirtualMic>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct VirtualMic {
    /// e.g. "VB-CABLE", "Voicemeeter", "Stereo Mix".
    pub family: String,
    /// The microphone name to pick in Zoom, e.g. "CABLE Output (VB-Audio Virtual Cable)".
    pub zoom_mic_name: String,
}

pub fn list_outputs() -> Result<Vec<OutputDevice>, AudioError> {
    let host = cpal::default_host();
    let backend = |e: cpal::Error| AudioError::Backend(e.to_string());
    // The default device handle does not compare equal to its enumerated twin; its id does.
    let default = host.default_output_device().and_then(|d| d.id().ok());
    let devices: Vec<cpal::Device> = host.output_devices().map_err(backend)?.collect();
    let names: Vec<String> = devices.iter().map(device_name).collect();
    let captures = list_inputs()?;
    Ok(detect::assign_ids(&names)
        .into_iter()
        .zip(names)
        .zip(&devices)
        .map(|((id, name), d)| OutputDevice {
            virtual_mic: detect_virtual_mic(&name, &captures),
            is_default: default.is_some() && d.id().ok() == default,
            id,
            name,
        })
        .collect())
}

/// The output to send speech to for Zoom: a detected virtual mic, preferring plain VB-CABLE
/// (`CABLE Input …`) over its A/B/Hi-Fi variants and over other families.
pub fn preferred_virtual_mic(outputs: &[OutputDevice]) -> Option<&OutputDevice> {
    let rank = |d: &OutputDevice| match d.virtual_mic.as_ref()?.family.as_str() {
        "VB-CABLE" if d.name.starts_with("CABLE Input") => Some(0),
        "VB-CABLE" => Some(1),
        _ => Some(2),
    };
    outputs
        .iter()
        .filter_map(|d| rank(d).map(|r| (r, d)))
        .min_by_key(|(r, _)| *r)
        .map(|(_, d)| d)
}

/// Names of the capture (recording) endpoints, i.e. what Zoom offers as microphones.
pub fn list_inputs() -> Result<Vec<String>, AudioError> {
    let host = cpal::default_host();
    Ok(host
        .input_devices()
        .map_err(|e| AudioError::Backend(e.to_string()))?
        .map(|d| device_name(&d))
        .collect())
}

fn device_name(d: &cpal::Device) -> String {
    d.description()
        .map(|desc| desc.name().to_string())
        .unwrap_or_else(|_| "(unnamed device)".into())
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlayerConfig {
    /// Where speech goes for Zoom. A device id from [`list_outputs`], or `None` for the system
    /// default output (the Zoom "Share → Advanced → Computer audio" path).
    pub mic: Option<String>,
    /// Optional second output so the user hears themselves (headphones). `None` = off.
    pub monitor: Option<String>,
    pub mic_volume: f32,
    pub monitor_volume: f32,
}

impl Default for PlayerConfig {
    fn default() -> Self {
        Self {
            mic: None,
            monitor: None,
            mic_volume: 1.0,
            monitor_volume: 0.6,
        }
    }
}

/// A panic elsewhere must not take the audio path down with it.
pub(crate) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev(name: &str, family: Option<&str>) -> OutputDevice {
        OutputDevice {
            id: name.into(),
            name: name.into(),
            is_default: false,
            virtual_mic: family.map(|f| VirtualMic {
                family: f.into(),
                zoom_mic_name: "x".into(),
            }),
        }
    }

    #[test]
    fn prefers_plain_cable_input_over_16ch_and_variants() {
        let outs = [
            dev("CABLE In 16ch (VB-Audio Virtual Cable)", None),
            dev("Speakers (Realtek(R) Audio)", None),
            dev("CABLE-A Input (VB-Audio Cable A)", Some("VB-CABLE")),
            dev("CABLE Input (VB-Audio Virtual Cable)", Some("VB-CABLE")),
        ];
        assert_eq!(
            preferred_virtual_mic(&outs).unwrap().name,
            "CABLE Input (VB-Audio Virtual Cable)"
        );
        assert!(preferred_virtual_mic(&outs[..2]).is_none());
    }

    /// Lists this machine's devices. Needs the audio stack (not CI):
    /// `cargo test -p soritong-audio -- --ignored lists_real_devices --nocapture`
    #[test]
    #[ignore = "needs the system audio stack"]
    fn lists_real_devices() {
        for d in super::list_outputs().unwrap() {
            println!("{d:?}");
        }
        println!("inputs: {:?}", super::list_inputs().unwrap());
    }
}
