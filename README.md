# soritong (소리통)

Type in Korean or English and it is spoken as your microphone in Zoom, so you can join a call
without speaking aloud.

## What it is

A small Windows-first desktop app (Rust + Tauri). You type a sentence and press Enter. A TTS
engine speaks it, and the audio goes to a device Zoom uses as your mic. Engines include your own
ElevenLabs voice clone, an offline Korean/English model (Supertonic 3) and free online voices.
It is not speech-to-text and not a live voice changer.

**Status: early development.** See [`docs/PLAN.md`](docs/PLAN.md) for the design and the options
considered.

## Getting started

Not usable yet. To build and test the libraries:

```
cargo test --workspace
```

## Checks

Run these before committing:

- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`

## Layout

- `crates/soritong-core`: the engine trait, PCM types, text splitting, synthesis cache, mock engine
- `crates/soritong-audio`: output devices, playback, the speak queue
- `crates/soritong-engine-*`: one crate per TTS engine
- `crates/soritong-app`: the Tauri desktop app
- `docs/`: plan and setup guides

## Licence

Apache-2.0. See [`LICENSE`](LICENSE).

---

Companion docs: [`AGENTS.md`](AGENTS.md) for how coding agents work here, and
[`HANDOFF.md`](HANDOFF.md) for the current state and what to pick up next.
