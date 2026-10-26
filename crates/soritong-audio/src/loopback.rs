//! Measuring and testing virtual audio loopback between render and capture endpoints.

use crate::AudioError;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SignalAnalysis {
    pub rms: f32,
    pub peak: f32,
    pub delay_ms: Option<f32>,
    pub has_signal: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LoopbackReport {
    pub success: bool,
    pub rms: f32,
    pub peak: f32,
    pub delay_ms: Option<f32>,
    pub message: String,
}

/// Pure signal analysis: measures RMS, peak, and onset latency between sent and recorded audio.
pub fn analyze_loopback(
    sent: &[f32],
    sent_sample_rate: u32,
    recorded: &[f32],
    recorded_sample_rate: u32,
) -> SignalAnalysis {
    if recorded.is_empty() {
        return SignalAnalysis {
            rms: 0.0,
            peak: 0.0,
            delay_ms: None,
            has_signal: false,
        };
    }

    let mut sum_sq = 0.0f32;
    let mut peak = 0.0f32;
    for &s in recorded {
        let abs = s.abs();
        if abs > peak {
            peak = abs;
        }
        sum_sq += s * s;
    }
    let rms = (sum_sq / recorded.len() as f32).sqrt();

    // Signal detected if peak is at least ~1% of full scale (-40 dB) and RMS is not negligible.
    let has_signal = peak >= 0.01 && rms >= 0.001;

    let delay_ms =
        if has_signal && !sent.is_empty() && sent_sample_rate > 0 && recorded_sample_rate > 0 {
            let sent_peak = sent.iter().fold(0.0f32, |m, &x| m.max(x.abs()));
            let sent_thresh = (sent_peak * 0.15).max(0.01);
            let sent_onset = sent.iter().position(|&x| x.abs() >= sent_thresh);

            let rec_thresh = (peak * 0.15).max(0.01);
            let rec_onset = recorded.iter().position(|&x| x.abs() >= rec_thresh);

            match (sent_onset, rec_onset) {
                (Some(s_idx), Some(r_idx)) => {
                    let s_sec = s_idx as f32 / sent_sample_rate as f32;
                    let r_sec = r_idx as f32 / recorded_sample_rate as f32;
                    let diff_ms = (r_sec - s_sec) * 1000.0;
                    Some(diff_ms.max(0.0))
                }
                _ => None,
            }
        } else {
            None
        };

    SignalAnalysis {
        rms,
        peak,
        delay_ms,
        has_signal,
    }
}

/// Plays audio into `output_device_name_or_id`, simultaneously records from `capture_device_name`,
/// and returns the measured signal report.
pub fn loopback_test(
    output_device_name_or_id: &str,
    capture_device_name: &str,
    pcm: &[f32],
    sample_rate: u32,
) -> Result<LoopbackReport, AudioError> {
    let host = cpal::default_host();
    let backend = |e: cpal::Error| AudioError::Backend(e.to_string());

    // 1. Find output device
    let outputs: Vec<_> = host.output_devices().map_err(backend)?.collect();
    let out_dev = outputs
        .into_iter()
        .find(|d| {
            d.description()
                .map(|desc| {
                    desc.name().contains(output_device_name_or_id)
                        || output_device_name_or_id.contains(desc.name())
                })
                .unwrap_or(false)
        })
        .ok_or_else(|| AudioError::DeviceNotFound(output_device_name_or_id.to_string()))?;

    // 2. Find input device
    let inputs: Vec<_> = host.input_devices().map_err(backend)?.collect();
    let in_dev = inputs
        .into_iter()
        .find(|d| {
            d.description()
                .map(|desc| {
                    desc.name().contains(capture_device_name)
                        || capture_device_name.contains(desc.name())
                })
                .unwrap_or(false)
        })
        .ok_or_else(|| AudioError::DeviceNotFound(capture_device_name.to_string()))?;

    // 3. Set up recording stream
    let in_config = in_dev.default_input_config().map_err(backend)?;
    let in_sample_rate = in_config.sample_rate();
    let in_channels = in_config.channels() as usize;

    let recorded = Arc::new(Mutex::new(Vec::<f32>::new()));
    let rec_clone = recorded.clone();

    let err_fn = |err| log::error!("loopback input stream error: {err}");
    let in_stream = match in_config.sample_format() {
        cpal::SampleFormat::F32 => in_dev.build_input_stream(
            in_config.into(),
            move |data: &[f32], _| {
                let mut buf = rec_clone.lock().unwrap();
                for frame in data.chunks(in_channels) {
                    buf.push(frame[0]);
                }
            },
            err_fn,
            None,
        ),
        cpal::SampleFormat::I16 => in_dev.build_input_stream(
            in_config.into(),
            move |data: &[i16], _| {
                let mut buf = rec_clone.lock().unwrap();
                for frame in data.chunks(in_channels) {
                    buf.push(frame[0] as f32 / 32768.0);
                }
            },
            err_fn,
            None,
        ),
        cpal::SampleFormat::U16 => in_dev.build_input_stream(
            in_config.into(),
            move |data: &[u16], _| {
                let mut buf = rec_clone.lock().unwrap();
                for frame in data.chunks(in_channels) {
                    buf.push((frame[0] as f32 - 32768.0) / 32768.0);
                }
            },
            err_fn,
            None,
        ),
        fmt => {
            return Err(AudioError::Backend(format!(
                "unsupported sample format: {fmt:?}"
            )));
        }
    }
    .map_err(backend)?;

    in_stream.play().map_err(backend)?;

    // 4. Set up playback stream
    let out_config = out_dev.default_output_config().map_err(backend)?;
    let out_channels = out_config.channels() as usize;

    // Resample pcm to out_sample_rate if needed
    let out_sample_rate = out_config.sample_rate();
    let resampled = if out_sample_rate == sample_rate {
        pcm.to_vec()
    } else {
        let mut conv = crate::convert::Converter::new(sample_rate, 1, out_sample_rate, 1);
        let mut out = conv.feed(pcm);
        out.extend(conv.flush());
        out
    };

    let pcm_shared = Arc::new(resampled);
    let play_idx = Arc::new(Mutex::new(0usize));
    let play_idx_clone = play_idx.clone();
    let pcm_clone = pcm_shared.clone();

    let out_stream = out_dev
        .build_output_stream(
            out_config.into(),
            move |data: &mut [f32], _| {
                let mut idx = play_idx_clone.lock().unwrap();
                for frame in data.chunks_mut(out_channels) {
                    let s = if *idx < pcm_clone.len() {
                        let sample = pcm_clone[*idx];
                        *idx += 1;
                        sample
                    } else {
                        0.0
                    };
                    for channel in frame.iter_mut() {
                        *channel = s;
                    }
                }
            },
            |err| log::error!("loopback output stream error: {err}"),
            None,
        )
        .map_err(backend)?;

    out_stream.play().map_err(backend)?;

    // 5. Wait for playback to complete plus a small cushion
    let duration_sec = pcm.len() as f32 / sample_rate as f32;
    std::thread::sleep(Duration::from_secs_f32(duration_sec + 0.3));

    drop(out_stream);
    drop(in_stream);

    // 6. Analyze
    let recorded_data = recorded.lock().unwrap().clone();
    let analysis = analyze_loopback(pcm, sample_rate, &recorded_data, in_sample_rate);

    let report = if analysis.has_signal {
        LoopbackReport {
            success: true,
            rms: analysis.rms,
            peak: analysis.peak,
            delay_ms: analysis.delay_ms,
            message: "✔ Zoom will hear you".to_string(),
        }
    } else {
        LoopbackReport {
            success: false,
            rms: analysis.rms,
            peak: analysis.peak,
            delay_ms: None,
            message: "no signal: CABLE Output got silence".to_string(),
        }
    };

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pure_analysis_silence() {
        let sent = vec![0.5f32; 1000];
        let recorded = vec![0.0f32; 1000];
        let analysis = analyze_loopback(&sent, 48000, &recorded, 48000);
        assert!(!analysis.has_signal);
        assert_eq!(analysis.rms, 0.0);
        assert_eq!(analysis.peak, 0.0);
        assert_eq!(analysis.delay_ms, None);
    }

    #[test]
    fn pure_analysis_active_signal_and_delay() {
        let rate = 48000;
        // Sent signal: silence for 10ms, then a sine wave of 0.8 peak
        let mut sent = vec![0.0f32; 480]; // 10ms silence
        for i in 0..4800 {
            sent.push(0.8 * (2.0 * std::f32::consts::PI * 440.0 * i as f32 / rate as f32).sin());
        }

        // Recorded signal: delayed by an additional 20ms (total 30ms before wave starts)
        let mut recorded = vec![0.0f32; 480 + 960]; // 10ms + 20ms = 30ms silence
        for i in 0..4800 {
            recorded
                .push(0.7 * (2.0 * std::f32::consts::PI * 440.0 * i as f32 / rate as f32).sin());
        }

        let analysis = analyze_loopback(&sent, rate, &recorded, rate);
        assert!(analysis.has_signal);
        assert!(analysis.peak > 0.69 && analysis.peak <= 0.71);
        assert!(analysis.rms > 0.4);
        assert!(analysis.delay_ms.is_some());
        let delay = analysis.delay_ms.unwrap();
        // Delay between sent onset (10ms) and rec onset (30ms) is ~20ms
        assert!(
            (delay - 20.0).abs() < 2.0,
            "expected ~20ms delay, got {delay}ms"
        );
    }
}
