# soritong — Handoff

**Current state and what to do next, not a changelog.** Replaced as work lands, never stacked.

## Where things stand

- **Release and Packaging Tooling (2026-10-25)**:
  - Added GitHub Actions workflows modelled on snapleaf:
    - `.github/workflows/ci.yml`: runs `cargo fmt`, `cargo clippy -D warnings`, `cargo test --workspace`, UI tests, release build (`./scripts/release.ps1 -SkipTests`), installer verification (`./scripts/test-installer.ps1`), and uploads `dist/*` artifacts.
    - `.github/workflows/release.yml`: triggered on `v*` tags; verifies commit is on `main`, release tag is annotated with notes, and CI passed; produces release artifacts, computes GitHub artifact provenance attestations, and publishes GitHub Release.
  - Release scripts in `scripts/`:
    - `scripts/version.ps1`: SemVer version resolution, bump logic, release comparison.
    - `scripts/release.ps1`: compiles release binaries, locates DirectML and VC++ CRT redistributable DLLs, generates per-user NSIS installer via Tauri, packages portable ZIP, computes `dist/SHA256SUMS.txt`.
    - `scripts/test-installer.ps1`: runs silent installation (`/S`, `/D=...`), verifies registry uninstall key and version, validates presence of `DirectML.dll` and `msvcp140.dll` next to `soritong-app.exe`, tests process launch, and verifies clean uninstallation.
    - `scripts/release-tag.ps1`: interactive and automated release tagging with changelog notes.
  - App branding:
    - Designed vector app icon `assets/soritong.svg` and `assets/soritong.png` (1024x1024, teal `#0f8a80` flat speech bubble with sound waves, no emoji).
    - Generated multi-size icon set in `crates/soritong-app/icons/` (`icon.ico`, `icon.png`, `Square*Logo.png`, etc.).
  - Runtime DLL distribution:
    - `crates/soritong-app/tauri.conf.json` maps `"resources": { "resources/*": "" }`, placing `DirectML.dll` and MSVC CRT DLLs directly alongside `soritong-app.exe` without extra subdirectories.
    - Per-user NSIS installer (`installMode: "currentUser"`), no administrator prompt required.
    - WebView2 bootstrapper downloader configured for systems without pre-installed WebView2 runtime.
  - Documentation and legal notices:
    - `README.md` (English) and `README.ko.md` (Korean): end-user documentation written for non-developers joining Zoom/Meet calls, with screenshots in `docs/screenshots/`.
    - `THIRD_PARTY_NOTICES.md`: comprehensive attribution and licensing notices for Supertonic 3 (BigScience OpenRAIL-M), VB-CABLE, Edge online voices, ONNX Runtime (MIT), DirectML (MIT / Microsoft DirectML Redistributable License), MSVC CRT runtime, Tauri, and WebView2.
    - `Cargo.toml`: workspace packages set `repository = "https://github.com/shyoo/soritong"`.

- **Virtual Mic Setup Wizard (`soritong-app` UI & Rust backend)**:
  - Step 0 (Path choice): Virtual microphone (recommended) vs direct playback.
  - Step 1 (Snapshot): Captures Windows audio defaults across console, multimedia, and communications roles via `IMMDeviceEnumerator::GetDefaultAudioEndpoint`.
  - Step 2 & 3 (VB-CABLE Driver): Official zip download with SHA-256 validation (`b950e39f...`), elevated installation, and reboot resumption.
  - Step 4 (Safeguard & Restore Defaults): Restores defaults via COM `IPolicyConfig::SetDefaultEndpoint`, warns if "Listen to this device" is enabled.
  - Step 5 (Configure Soritong): Configures mic to `CABLE Input` and monitor to real speakers.
  - Step 6 (Loopback Verification): Plays 440 Hz tone into CABLE Input, analyzes CABLE Output (RMS, onset latency, SNR).
  - Step 7 (Zoom Guide): Step-by-step Zoom / Meet / Discord configuration instructions with one-click mic name clipboard copy.
  - Bilingual UI: Complete English (`en`) and Korean (`ko`) strings in `ui/strings.js`, auto-detected via `navigator.language`.

- **Engine status**:
  - `edge`: live Korean OK (CLI, 2026-10-25).
  - `supertonic`: live Korean OK (CLI); RTF ≈ 0.17 on i7-12700K. The installer and portable ZIP ship `DirectML.dll` next to the exe plus the VC++ 2015-2022 redist.
  - `system`: English voices (no Korean speech pack on host).
  - `elevenlabs`: tested with mock and demo mode (needs key for live).
  - `openai`: tested against mock/demo server.

- **Checks and Verification**:
  - `cargo fmt --all --check` passed cleanly.
  - `cargo clippy --workspace --all-targets -- -D warnings` passed with 0 warnings.
  - `cargo test --workspace`: all 79 tests passed.
  - `node --test crates/soritong-app/ui-tests/keys.test.mjs crates/soritong-app/ui-tests/wizard.test.mjs`: all 13 tests passed.
  - `scripts/release.ps1 -SkipTests`: produced `soritong-0.1.0-windows-x64-setup.exe` (17.58 MB), `soritong-0.1.0-windows-x64-portable.zip` (35.5 MB), and `SHA256SUMS.txt`.
  - `scripts/test-installer.ps1`: verified silent install, registry registration, DLL placement, process startup, and clean uninstall.

## What is unproven

- Pushing to GitHub remote and observing the GitHub Actions CI run for this branch (ready to push).
- Owner reported (2026-10-25) partial Hangul (ㅇ, 아) not showing while typing; could not be reproduced in tests.
- Live Zoom call with real participants listening to virtual mic output.

## Next

1. Push branch to `origin` and verify GitHub Actions CI passes cleanly.
2. Cut first release tag (`v0.1.0`) when merged to `main`.
