# soritong (소리통) — implementation plan

Status: **decided 2026-10-25** (see §7). Skeleton crates exist; pieces in §6 are delegated.

## 1. What it is

A small desktop app: you type (mostly Korean, also English), press Enter, and the sentence is spoken
**as your microphone in Zoom**. The use case is a monthly Zoom book club at 4–5 am, when the owner
cannot speak aloud. Speech feels more personal to the other members than chat text. The app will
be public on GitHub.

Non-goals for v1: speech-to-text, voice changing of a live mic, mobile, Zoom bots.

## 2. The hard constraint: "a mic Zoom can select" on Windows

Zoom only reads audio from a **capture endpoint** (a recording device). On Windows a new capture
endpoint can only come from a **kernel-mode audio driver**. No supported user-mode API creates one.
Driver-free "voice changer" apps hook other processes' audio calls instead, which is fragile and
looks like malware. So every tool that shows up as a mic installs a driver. VB-CABLE is one; so is
any app with its own virtual mic.

What the app *can* remove is the annoying part of virtual cables, which is the routing (Windows
sound settings, "listen to this device", default-device juggling). The app writes PCM straight to
a chosen output device by name. Three ways to get the sound into Zoom, in order of setup cost:

| Path | Install | How Zoom gets it | Trade-offs |
|---|---|---|---|
| **A. Zoom "Share → Advanced → Computer audio"** | nothing | Zoom captures system output (it excludes its own output) | Uses the meeting's share slot. Any other system sound also goes through. Has to be started each meeting. Works on Win/macOS/Linux desktop Zoom. |
| **B. An existing loopback/virtual device** | 0–1 driver, once | The app plays into the device's render side; Zoom selects its capture side as the mic | The app auto-detects known pairs (VB-CABLE, Voicemeeter, Stereo Mix, VirtualDrivers' Virtual Audio Driver, where it loops back) and offers them in one dropdown. No routing setup. Licensing means we link to the installers instead of bundling them. |
| **C. Our own signed virtual-mic driver** | 1 driver, bundled | A real "Soritong Microphone" | The cleanest end state. Needs an EV code-signing certificate plus Microsoft Partner Center attestation signing, and kernel-driver maintenance. The open-source base (VirtualDrivers, MIT + MS-PL) is still beta and test-signed. **Proposed as a later phase, not v1.** |

v1 ships A and B behind one `AudioSink` abstraction, so C (and later macOS/Linux sinks) plug in
without touching the rest. On Linux a PipeWire virtual source can be created at runtime with no
driver. On macOS it takes a user-space HAL plug-in (BlackHole-style) with notarization. Both come
after Windows.

Whichever path is used, there is an optional **monitor** output (your headphones) at its own volume,
so you can hear what was said.

## 3. Voice engines (research summary)

All engines implement one trait. They return PCM chunks (f32 plus a sample rate). Audio is
resampled once, in the audio layer.

### Cloud

| Engine | Cost | Korean | Your cloned voice | Notes |
|---|---|---|---|---|
| **ElevenLabs** | paid (you have an account) | yes | **yes** (the instant clone already used in `inkland`) | `eleven_flash_v2_5` for low latency (~75 ms model time, streaming). `eleven_multilingual_v2` / `eleven_v3` for quality. Responses cached by text hash so replays cost no credits. |
| **Microsoft Edge online voices** | free, no key | yes (SunHi, InJoon, Hyunsu-Multilingual…) | no | Unofficial endpoint (the `edge-tts` approach). Natural sound but can break or be throttled. Labelled "unofficial" in the UI. |
| **Azure Speech (free F0 tier)** | free tier, needs an Azure key | yes (same voices) | no (custom voice is enterprise-gated) | The official, stable version of the Edge voices. |
| **OpenAI-compatible `/v1/audio/speech`** | depends on the server | depends | depends | One adapter covers OpenAI and the local servers in the next table. |

### Offline / private

| Engine | Korean | Clone | Runtime in Rust | Notes |
|---|---|---|---|---|
| **Supertonic 3** (Supertone) | yes (31 languages) | no (fixed voice styles) | **native**: ONNX via `ort`, upstream Rust example | 99 M params, real-time on CPU, 44.1 kHz. Code is MIT, weights OpenRAIL-M. The repo is archived, so we pin a model revision and download it on first use. **Recommended free and offline default.** |
| **Windows built-in voices** (WinRT `SpeechSynthesizer`) | yes, if the Korean speech pack is installed (Heami) | no | native via the `windows` crate | Nothing to install, but robotic. Zero-setup fallback. |
| **Chatterbox Multilingual** (Resemble AI) | yes (23 languages) | **yes, zero-shot from a ~10 s sample** | Python + GPU, as a local sidecar server | MIT. The offline way to speak in *your* voice. Reached through the OpenAI-compatible adapter, not bundled. Community ONNX exports exist (worth evaluating later for native). |
| GPT-SoVITS / CosyVoice / F5-TTS | yes (varies) | yes | Python sidecar | Same adapter; documented, not bundled. |

Proposed engine order: Supertonic (offline default) → ElevenLabs (your voice) → Windows built-in →
Edge online → OpenAI-compatible (which covers local cloning servers and Azure-like endpoints).

## 4. UI

The text box is the whole product, and it has to handle **Korean IME composition** perfectly.
egui/winit has had Windows Korean IME bugs (caret at (0,0), composition visuals) as recently as
egui 0.36 / winit PR #4746. A webview, by contrast, uses the OS text stack, which handles Korean
correctly. Hence the recommendation:

- **Tauri 2**, with every piece of logic in Rust and a tiny frontend (one HTML page, a few hundred
  lines of TS, no framework). Alternative if "Rust everywhere" matters more: a Leptos (Rust→wasm)
  frontend in the same Tauri shell, with the same IME behaviour at more build complexity.
- egui is the pure-Rust single-exe option. It stays on the table only if we accept testing and
  patching Korean IME ourselves.

v1 window (small, can stay always on top):

```
┌ soritong ───────────────────────── [⚙] ┐
│ Engine [Supertonic ▾] Voice [F1 ▾]      │
│ Mic out [Zoom share / CABLE Input ▾] 🎧 │
│ ─────────────────────────────────────── │
│ 오늘 책 정말 재밌었어요.          ▶  ↻  │  ← history (click = replay, cached)
│ I agree with that point.          ▶  ↻  │
│ ─────────────────────────────────────── │
│ [네 맞아요] [잠시만요] [ㅋㅋㅋ] [감사합니다] │  ← quick phrases (editable)
│ ┌─────────────────────────────────────┐ │
│ │ 타이핑하고 Enter…                     │ │  Enter = speak, Shift+Enter = newline
│ └─────────────────────────────────────┘ │  Esc = stop speaking
└─────────────────────────────────────────┘
```

Also: a global hotkey to focus the input from inside Zoom, a queue (sentences typed while one is
speaking play next), a per-engine voice picker, and a settings pane. API keys are stored in the
**Windows Credential Manager** (`keyring` crate), never in files.

## 5. Architecture

Cargo workspace:

```
crates/
  soritong-core/     TtsEngine trait, Utterance, text chunking (sentence split, KO/EN detection),
                     synthesis cache (hash of engine+voice+settings+text → file)
  soritong-audio/    cpal/WASAPI device enumeration, rubato resampling, playback queue,
                     dual output (mic sink + monitor), stop/interrupt, known-virtual-device detection,
                     the Speaker queue (sentence split + cache + synth + play)
  soritong-engine-*/ one crate per engine: supertonic (ort), elevenlabs, system (WinRT),
                     edge, openai (compatible servers)
  soritong-cli/      `soritong say "안녕하세요" --engine supertonic --device "CABLE Input"`,
                     `soritong devices`, `soritong voices --engine elevenlabs`
                     (headless: agents and CI can verify engines without a GUI)
  soritong-app/      Tauri 2 shell + tiny frontend (ui/); talks only to core/audio/engines
docs/                PLAN.md, ZOOM_SETUP.md (ko + en)
```

Latency approach: split text into sentences and synthesise sentence 1 while sentence 2 is queued.
Streaming engines (ElevenLabs websocket) start playback on the first chunk.

Checks to declare once code exists: `cargo fmt --check`, `cargo clippy --workspace -- -D warnings`,
`cargo test --workspace`. The `--engine mock` path makes tests hermetic. No network or keys go into CI.

## 6. Delivery: phases and delegation

Each piece is a Warmstart sub-task on its own branch, cut from this one. I review and merge.

| # | Piece | Agent | Depends on |
|---|---|---|---|
| 0 | Workspace skeleton and shared contracts (§8) | done in the planning task | — |
| 1 | `soritong-audio` for real (cpal/WASAPI, resample, dual output, virtual-mic detection, Speaker with sentence split + cache) + `soritong-cli` | ClaudeFirst Opus 5.5 (med) | 0 |
| 2 | `soritong-engine-elevenlabs` (streaming flash + quality model, voice list incl. clones) | Antigravity Flash 3.8 (high) | 0 |
| 3 | `soritong-engine-supertonic` (ort, pinned model download + checksum, voice styles, ko/en) | ClaudeFirst Opus 5.5 (med) | 0 |
| 4 | `soritong-engine-system` (WinRT), `-edge`, `-openai` | Antigravity Flash 3.8 (high) | 0 |
| 5 | `soritong-app` (Tauri): input/IME, history/replay, quick phrases, settings, keyring, device picker, hotkey | ClaudeFirst Opus 5.5 (med) | 0 |
| 6 | Integration: wire engines into the CLI and app (planning task) | — | 1–5 |
| 7 | README (ko/en), ZOOM_SETUP (paths A/B + echo test), installer via the Tauri bundler | Antigravity Flash 3.8 (high) | 6 |

Pieces 1–5 run in parallel, each cut from the skeleton. 7 is filed after 6 is merged.

Later phases: native offline cloning (Chatterbox ONNX), a signed Soritong driver (path C), and
PipeWire/macOS sinks.

## 7. Decisions (owner, 2026-10-25)

1. **Mic path for v1: A + B.** Our own driver (C) is a later phase. Cross-talk: path B cannot
   pick up meeting audio, because only soritong writes into the cable. Path A relies on Zoom
   leaving its own playback out of the shared computer audio, and any other system sound (alerts,
   browser tabs) does go through. So **B is the recommended setup and A the zero-install
   fallback**. ZOOM_SETUP must include a two-device echo test for A.
2. **UI: Tauri 2 + a tiny plain HTML/JS frontend** (no framework, no bundler).
3. **v1 engines: all proposed.** Supertonic 3, ElevenLabs, Windows built-in, Edge online,
   OpenAI-compatible.
4. **Speech only.** No Zoom chat mirroring.
5. **Windows-first with portable code. Licence: Apache-2.0.**

## 8. Shared contracts (in the skeleton)

- `soritong-core`: `TtsEngine` (blocking, `Send + Sync`, chunk callback, `Cancel`), `PcmChunk`,
  `Voice`, `SynthRequest`, `text::{detect_lang, split_sentences}`, `cache::{SynthCache, cache_key}`,
  `mock::MockEngine`.
- `soritong-audio`: `list_outputs`, `OutputDevice`/`VirtualMic`, `Player`/`PlayerConfig`, and the
  `Speaker` queue with `SpeakEvent`. These are stubs that make no sound until the audio piece lands.
  This differs from §5: the speak queue lives here rather than in the app, so the CLI and the app
  share it.
- Each engine is its own crate, `crates/soritong-engine-<name>`, so pieces never edit the same
  manifest. Integration into the CLI and app happens at merge time.
