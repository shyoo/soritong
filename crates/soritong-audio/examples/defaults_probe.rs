//! Prints the Windows default audio endpoints; with `--set <endpoint id>` makes that endpoint the
//! default for every role first (what the wizard's "Restore Windows Defaults" does).
//!
//! `cargo run -p soritong-audio --example defaults_probe [-- --set "{0.0.0.00000000}.{…}"]`
use soritong_audio::defaults::{AudioRole, get_current_defaults, set_default_endpoint};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--set") {
        let id = args.get(i + 1).expect("--set needs an endpoint id");
        println!("set: {:?}", set_default_endpoint(id, AudioRole::All));
    }
    let d = get_current_defaults().expect("query defaults");
    println!("speaker: {:?}", d.render_console.map(|e| e.friendly_name));
    println!("mic:     {:?}", d.capture_console.map(|e| e.friendly_name));
}
