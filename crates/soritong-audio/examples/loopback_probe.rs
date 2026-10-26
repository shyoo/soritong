//! Diagnoses the wizard's loopback test on a real machine: prints the CABLE endpoints' default
//! formats, then plays a tone into CABLE Input and listens on CABLE Output.
//!
//! `cargo run -p soritong-audio --example loopback_probe` (set `PROBE_OUT=<name part>` to force an
//! output instead of the one the app would pick).
use cpal::traits::{DeviceTrait, HostTrait};

fn main() {
    let host = cpal::default_host();
    for (kind, devs) in [
        ("out", host.output_devices().unwrap().collect::<Vec<_>>()),
        ("in", host.input_devices().unwrap().collect::<Vec<_>>()),
    ] {
        for d in devs {
            let name = d
                .description()
                .map(|x| x.name().to_string())
                .unwrap_or_default();
            if !name.contains("CABLE") {
                continue;
            }
            let cfg = if kind == "out" {
                d.default_output_config()
            } else {
                d.default_input_config()
            };
            println!("{kind}: {name} -> {cfg:?}");
        }
    }
    let sr = 44_100;
    let pcm: Vec<f32> = (0..sr * 8 / 10)
        .map(|i| 0.3 * (std::f32::consts::TAU * 440.0 * i as f32 / sr as f32).sin())
        .collect();
    let out = soritong_audio::list_outputs().unwrap();
    let cable = match std::env::var("PROBE_OUT") {
        Ok(part) => out
            .iter()
            .find(|d| d.name.contains(&part))
            .expect("no such output"),
        Err(_) => soritong_audio::preferred_virtual_mic(&out).expect("no virtual mic detected"),
    };
    println!("testing via {}", cable.name);
    println!(
        "{:?}",
        soritong_audio::loopback::loopback_test(&cable.id, "CABLE Output", &pcm, sr as u32)
    );
}
