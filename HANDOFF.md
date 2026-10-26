# soritong — Handoff

**Current state and what to do next, not a changelog.** Replaced as work lands, never stacked.

## Where things stand

- Release setup (2026-10-25): GitHub repo `shyoo/soritong`, **private until the owner flips it public**.
  CI (`.github/workflows/ci.yml`, windows-latest, ~19 min warm) runs the checks, builds the NSIS
  installer and portable ZIP (`scripts/release.ps1`) and smoke-installs it (`scripts/test-installer.ps1`).
  `release.yml` publishes from annotated `v*` tags on `main` with passing CI; see AGENTS.md "Releasing".
  No release cut yet. Installer: per-user NSIS, `DirectML.dll` + MSVC CRT next to the exe,
  WebView2 bootstrapper, identifier `com.shyoo.soritong`; not code-signed (SmartScreen warns); no auto-update.
  README.md / README.ko.md are user-facing, with screenshots from `scripts/capture-screenshots.mjs`.

- Fixed 2026-10-25: the wizard reopened at step 5 on every start (field-name mismatch between Rust and JS), and the window shrank on every start on mixed-DPI monitors. Both verified across two restarts on the owner's left (100%) monitor.

- UI (2026-10-25): line pictograms instead of emoji (`ui/icons.css`); "On this PC"/"Online" badges instead of a lock; history as right-aligned chat bubbles with send time and a day divider; "Clear chat" (two clicks); Restore shows "done" only if Windows actually changed.

PLAN §6 pieces 1–6 and the Windows-first virtual microphone setup wizard (Path B) are implemented in this branch.

- **Virtual Mic Setup Wizard (`soritong-app` UI & Rust backend)**:
  - **Step 0 (Path choice)**: Virtual microphone (recommended) vs direct playback (no install needed).
  - **Step 1 (Snapshot)**: Automatically captures Windows audio defaults across console, multimedia, and communications roles via `IMMDeviceEnumerator::GetDefaultAudioEndpoint` and stores in settings.
  - **Step 2 & 3 (VB-CABLE Driver)**:
    - Pinned official URL: `https://download.vb-audio.com/Download_CABLE/VBCABLE_Driver_Pack45.zip`
    - Pinned SHA-256: `b950e39f01af1d04ea623c8f6d8eb9b6ea5c477c637295fabf20631c85116bfb`
    - Downloads, verifies SHA-256, unzips with `zip`, and launches `VBCABLE_Setup_x64.exe` with elevated `runas` privileges (`ShellExecuteExW`).
    - Reboot support: "I'll reboot; resume after" sets `wizard_reboot_pending` and resumes at installation check upon relaunch.
  - **Step 4 (Safeguard & Restore Defaults)**:
    - Defaults restoration: Detects if VB-CABLE took over Windows defaults and restores user's original hardware endpoints via COM `IPolicyConfig::SetDefaultEndpoint` (CLSID `{870af99c-171d-4f9e-af0d-e63df40c2bc9}`, IID `{f8679f50-850a-41cf-9c72-430f290290c8}`).
    - Echo Guard: Checks read-only "Listen to this device" on `CABLE Output` using IMMDevice property store (`{24dbb0fc-9311-4b3d-9cf0-18ff155639d4},1`) and warns if enabled to prevent feedback loops.
  - **Step 5 (Configure Soritong)**:
    - Auto-configures mic output to `CABLE Input (VB-Audio Virtual Cable)` and headphone monitor to user's real default speakers.
  - **Step 6 (Loopback Verification)**:
    - Pure signal analysis (`analyze_loopback`) measuring RMS, peak amplitude, onset latency (ms), and signal presence.
    - Live cpal test playing a synthetic 440 Hz test tone into CABLE Input and recording from CABLE Output.
  - **Step 7 (Zoom Guide)**:
    - Clear visual instructions for Zoom / Discord / Google Meet audio settings.
    - One-click "Copy microphone name" button with clipboard visual feedback.
    - Can be reopened anytime from Settings (⚙).
  - **Bilingual Localization**:
    - Complete English (`en`) and Korean (`ko`) strings in `ui/strings.js`, auto-detected via `navigator.language`.

- Engine status:
  - `edge`: live Korean OK (CLI, 2026-10-25).
  - `supertonic`: live Korean OK (CLI); RTF ≈ 0.17 on i7-12700K. The installer must ship
    `DirectML.dll` next to the exe plus the VC++ 2015–2022 redist. The model dir can be overridden
    with `SORITONG_SUPERTONIC_MODEL_DIR`.
  - `system`: this machine has English voices only (no Korean speech pack).
  - `elevenlabs`: never run live (no key in env).
  - `openai`: tested only against an in-test server.
- VB-CABLE pack 45 sha256 re-verified against a fresh download (2026-10-25). If the install fails, the wizard opens vb-audio.com via `wizard_open_vbcable_page` and says which file to pick.
- Known gap: the installer is x64-only (no ARM64 setup is launched).

- **Checks and Verification**:
  - `cargo fmt --all --check` passed cleanly.
  - `cargo clippy --workspace --all-targets -- -D warnings` passed with 0 warnings.
  - `cargo test --workspace`: all 79 tests passed (including hermetic unit tests for step decisions, signal analysis, and restore logic; live device tests marked `#[ignore]`).
  - `node --test crates/soritong-app/ui-tests/keys.test.mjs crates/soritong-app/ui-tests/wizard.test.mjs`: all 13 tests passed.
  - Live CDP verification (`scripts/wizard-smoke.mjs` against running app): walked all 7 wizard steps, verified snapshot display, safeguarding, configure, copy button, settings modal reopening, and backend state persistence.

## What is unproven

- Owner reported (2026-10-25) that partial Hangul (ㅇ, 아) did not show while typing in their normal soritong. It could not be reproduced: in 3 real-IME tests by the owner in a test window (same monitor and size, also after sending a message), every composition step showed and reached the box. Waiting to hear what preceded it if it recurs.

- In-app loopback (`wizard_run_loopback` via CDP) passes on the owner's PC with their device settings (2026-10-25). `CABLE In 16ch` is now hidden, and old settings that saved it migrate to `CABLE Input`.

- "Restore Windows Defaults" now works and is verified (owner's PC back on Realtek speakers, 2026-10-25). It used to report success without changing anything. Not yet clicked through the wizard UI after the fix.

- Loopback via `CABLE Input` passes on the owner's PC (2026-10-25, 111 ms). The wizard had been testing `CABLE In 16ch`, which cannot be opened; fixed via `preferred_virtual_mic`. Not yet re-run through the wizard UI by the owner.

- Wizard step 4 "Restore Windows Defaults" on a real machine. On the owner's PC (2026-10-25) VB-CABLE had taken the default speaker; Listen was off. Step 4 used to show every warning together with its "OK" twin (CSS `display` beat `hidden`); fixed.

- No human has listened to any engine's output yet.
- Real elevated execution of `VBCABLE_Setup_x64.exe` was deliberately not run on this host machine to preserve environment state (mocked and tested via hermetic tests).
- Zoom call with real participants listening to virtual mic output.

## Next

1. With the owner's go-ahead: push `main`, wait for CI, cut `v0.1.0-rc.1`; the owner tries the installer and makes the repo public.
2. A live Zoom call with someone listening, and ElevenLabs with the owner's key.
3. Later: code signing, auto-update, `docs/ZOOM_SETUP.md` with the two-device echo test for the no-install path.
