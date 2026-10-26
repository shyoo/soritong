# soritong-app

The Tauri 2 desktop app. Rust does the work: engines, the speak queue, settings and secrets. `ui/`
is one plain HTML page with ES-module JS, served as-is: no framework, no bundler, no npm.

```
cargo run -p soritong-app           # builds from a clean checkout, Node not needed
node --test crates/soritong-app/ui-tests/keys.test.mjs   # key-handling tests (Node ≥ 20)
```

The Tauri CLI is only needed for installers: `cargo install tauri-cli --version ^2`, then
`cargo tauri build` from this directory.

## Layout

- `src/lib.rs`: Tauri commands, the `Speaker` + `Player` in app state, `speak-event` forwarding,
  the window (geometry, always on top) and the global shortcut.
- `src/registry.rs`: `EngineRegistry`, plus a descriptor per engine (fields, secret or not,
  offline, needs a model download). The UI renders settings from the descriptor.
- `src/settings.rs`: `settings.json` in `dirs::config_dir()/soritong/`. Non-secret settings only.
- `src/secrets.rs`: the `SecretStore` trait. `KeyringStore` writes to Windows Credential Manager
  (service `soritong`, user = engine id); `MemoryStore` is for tests.
- `src/request.rs`: text → `SynthRequest` (`Lang::Auto`, trimmed, voice required).
- `ui/keys.js`: decides what a key press means. Korean IME handling lives here (see below).
- `ui/strings.js`: every UI string. Add a `ko` table with the same keys to translate.

## Adding an engine

In `registry::builtin()`:

```rust
r.register(
    EngineDescriptor::new("elevenlabs", "ElevenLabs", false)
        .field(SettingField::secret("api_key", "API key"))
        .field(SettingField::text("model_id", "Model").default_value("eleven_flash_v2_5")),
    Box::new(|cfg| {
        Ok(Arc::new(ElevenLabs::new(cfg.require("api_key")?, cfg.get("model_id"))?) as Arc<dyn TtsEngine>)
    }),
);
```

and add the crate to `Cargo.toml`. Engines with no settings use `r.register_fixed(Arc::new(E))`.
Offline engines that fetch weights add `.needs_model_download()`. The factory runs lazily, on the
first voice list or speak. It runs again after that engine's settings are saved. `cfg.require`
returns `TtsError::Config`, and the UI shows that error as-is.

## Korean IME

Never submit while composing (`isComposing`, keyCode 229, key `"Process"`, or our own
`compositionstart` flag). With the Microsoft Korean IME in WebView2, Enter on an uncommitted
syllable first commits it (`compositionend`), then delivers a plain Enter keydown. That keydown
submits the full text. If a browser instead reports the Enter as part of the composition, the
submit is deferred until after `compositionend`. Shift+Enter is always a newline.

## Trying it without touching your settings

`scripts/run-app.ps1` sets all of these for you. The other scripts in `scripts/` drive the
running app (see AGENTS.md). Underneath:

- `SORITONG_SETTINGS=<path>` uses another settings file.
- `SORITONG_DEMO_ENGINES=1` (debug builds only) adds a fake cloud engine with a secret field. Use
  it to exercise the settings pane and Credential Manager before a real cloud engine lands.
- `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9333` exposes the page to
  DevTools/CDP for scripted checks.
