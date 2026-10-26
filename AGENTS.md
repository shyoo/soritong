# soritong — agent guide

How to work in this codebase. Read [`HANDOFF.md`](HANDOFF.md) first: it is the only file that
carries current state.

## Before you start

- Stack: Rust 2024 Cargo workspace (`crates/*`); the desktop app will be Tauri 2 with a plain HTML/JS frontend.
- No network, API keys or audio hardware may be needed by `cargo test`. Use `soritong_core::mock::MockEngine`;
  tests that need a key or a device are `#[ignore]` and documented.
- API keys come from the environment in examples (`ELEVENLABS_API_KEY`…) and from Windows Credential
  Manager in the app. Never from a committed file.
- Model weights (`*.onnx`) are downloaded at runtime, never committed.

## Checks

Run these and make them pass before you report done:

- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`
- When `crates/soritong-app/ui/` changes: `node --test crates/soritong-app/ui-tests/keys.test.mjs`

## When you commit

- Work on a branch, never directly on `main`.
- One coherent commit per task where it is safe to squash; explain *why*, not *what*.
- ⛔ Never commit generated output, dependency directories, credentials, or anything gitignored.
- Update [`HANDOFF.md`](HANDOFF.md) in the same commit: what changed, and what is next.

## House rules

- Shared contracts live in `soritong-core` (engine trait) and `soritong-audio` (Player, Speaker). Don't
  change their signatures in an engine or app change. Add to them, or change them together with every caller.
- One crate per engine (`crates/soritong-engine-<name>`), errors mapped to `soritong_core::TtsError`.
- Unit tests sit next to the code (`#[cfg(test)]`).
- UI: no emoji, anywhere (owner's rule). Use the line pictograms in `ui/icons.css` (`<span class="ico i-NAME">`,
  or `var(--i-NAME)` in CSS). Add new ones there as 24px stroke SVGs.
- Pitfalls: the things that have bitten somebody here before. An entry earns its place by having
  cost a real debugging session; delete it when the pitfall becomes impossible.
  - Example names are global across the workspace: two crates with `examples/say.rs` link to the
    same `target/debug/examples/say.exe`, and parallel builds fail with LNK1104. Name examples
    `say_<engine>`.
  - `Start-Process -RedirectStandardOutput` makes the app inherit the caller's stdout, so any
    shell reading the launcher's output hangs until the app exits. `scripts/run-app.ps1` avoids it.
    Use it to launch the app.
  - In PowerShell, `FindWindow($null, "soritong")` marshals `$null` as `""` and never matches.
    Use `(Get-Process soritong-app).MainWindowHandle`.
  - Node 22 `node --test <directory>` fails with MODULE_NOT_FOUND. Pass the test files.
  - CDP `Runtime.evaluate` shares one top-level scope across calls, so a second `const x` throws.
    Wrap snippets in `(() => { ... })()`.
  - VB-CABLE pack 45 adds `CABLE In 16ch`, listed before `CABLE Input`. It rejects its own reported
    format in shared mode (OS error -2004287478, AUDCLNT_E_UNSUPPORTED_FORMAT). It is hidden from `list_outputs` (`detect::is_unplayable`), and saved settings are moved off it on load.
    Pick the cable with `soritong_audio::preferred_virtual_mic`, never by "name contains VB-Audio".
    `cargo run -p soritong-audio --example loopback_probe` checks a real machine.
  - COM objects from `CoCreateInstance` as `IUnknown` must be `query`'d for the interface before
    calling vtable slots. Calling `IPolicyConfig::SetDefaultEndpoint` through the raw IUnknown returned
    S_OK and changed nothing. Verify Windows state afterwards (`examples/defaults_probe.rs`).
  - Tauri command results reach JS as plain JSON: a field the JS reads but Rust does not send is
    just `undefined`, so no error appears. The wizard read `completed` while Rust sent
    `wizard_completed`, and reopened on every start. Grep both sides when renaming a field.
  - Window size is saved in logical pixels, and restored by moving first and then sizing. On mixed-DPI
    monitors (owner: 150% main, 100% left) physical sizes shrank the window on every start.
  - `ui/` is compiled into the app binary: `scripts/run-app.ps1 -NoBuild` shows the old UI after a
    UI edit. Rebuild to see it.
  - The HTML `hidden` attribute loses to any CSS `display:` rule, so a wizard showed a warning and
    its "all OK" twin at once. `ui/style.css` starts with `[hidden] { display: none !important; }`; keep it.
  - Korean IME in WebView2 (Microsoft IME): Enter on an uncommitted syllable fires `compositionend`
    first, then a plain Enter keydown (keyCode 13, `isComposing` false). See
    `crates/soritong-app/README.md` before touching `ui/keys.js`.

## Scripts (`scripts/`): run and inspect the app

Reuse these instead of writing new drivers. They need Windows, Node ≥ 22 and a debug build.

- `scripts/run-app.ps1 [-Demo] [-NoBuild] [-Stop]`: builds and starts the app with settings in
  `%TEMP%\soritong-try` (never your own) and the WebView2 page on CDP port 9333. `-Demo` adds a
  fake keyed cloud engine for the settings pane and Credential Manager. `-Stop` closes the app the
  way a user would, which saves the window geometry.
- `node scripts/app-smoke.mjs [shot.png]`: speaks a message (queued → speaking → done), simulates
  IME composition, checks Shift+Enter, and takes a screenshot. Exits 1 on failure.
- `scripts/cdp.mjs`: a dependency-free CDP client (`connect()`, `eval`, `key`, `send`,
  `screenshot`) for one-off checks.
- `scripts/ime-type.ps1 -Keys "dkssudgktpdy{ENTER}"` with `node scripts/ime-watch.mjs arm|read`:
  types through the **real** Windows Korean IME and shows the events the box saw. It injects real
  keystrokes, so don't run it while someone is using the machine.
- `scripts/hotkey-check.ps1`: minimises the window, presses Ctrl+Alt+Space, and checks the window
  comes back in front.
