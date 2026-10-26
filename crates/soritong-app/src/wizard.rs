//! Setup wizard logic, state transitions, and VB-CABLE installer download/unpack.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::Path;

pub const VBCABLE_URL: &str =
    "https://download.vb-audio.com/Download_CABLE/VBCABLE_Driver_Pack45.zip";
pub const VBCABLE_SHA256: &str = "b950e39f01af1d04ea623c8f6d8eb9b6ea5c477c637295fabf20631c85116bfb";
pub const VBCABLE_PAGE_URL: &str = "https://vb-audio.com/Cable/";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WizardStep {
    ChoosePath,
    Detect,
    Install,
    RestoreDefaults,
    ConfigureSoritong,
    LoopbackTest,
    ZoomStep,
    Complete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PathChoice {
    VirtualMic,
    DirectShare,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WizardContext {
    pub completed: bool,
    pub path_choice: Option<PathChoice>,
    pub has_snapshot: bool,
    pub cable_installed: bool,
    pub defaults_are_cable: bool,
    pub defaults_restored: bool,
    pub soritong_configured: bool,
    pub loopback_passed: bool,
    pub reboot_pending: bool,
}

/// Pure decision function: determines which wizard screen to show.
pub fn decide_wizard_step(ctx: &WizardContext) -> WizardStep {
    if ctx.completed {
        return WizardStep::Complete;
    }
    match ctx.path_choice {
        None => WizardStep::ChoosePath,
        Some(PathChoice::DirectShare) => WizardStep::ChoosePath,
        Some(PathChoice::VirtualMic) => {
            if !ctx.has_snapshot {
                WizardStep::Detect
            } else if !ctx.cable_installed {
                WizardStep::Install
            } else if ctx.defaults_are_cable && !ctx.defaults_restored {
                WizardStep::RestoreDefaults
            } else if !ctx.soritong_configured {
                WizardStep::ConfigureSoritong
            } else if !ctx.loopback_passed {
                WizardStep::LoopbackTest
            } else {
                WizardStep::ZoomStep
            }
        }
    }
}

/// Downloads VB-CABLE driver zip from official source and verifies pinned SHA-256 hash.
pub fn download_and_verify_vbcable(target_zip_path: &Path) -> Result<(), String> {
    let resp = ureq::get(VBCABLE_URL)
        .header("User-Agent", "soritong/0.1")
        .call()
        .map_err(|e| format!("Download error: {e}"))?;

    let mut reader = resp.into_body().into_reader();
    let mut file =
        std::fs::File::create(target_zip_path).map_err(|e| format!("Create file error: {e}"))?;

    let mut hasher = Sha256::new();
    let mut buf = [0u8; 16384];
    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|e| format!("Read error: {e}"))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        file.write_all(&buf[..n])
            .map_err(|e| format!("Write error: {e}"))?;
    }

    let hash = format!("{:x}", hasher.finalize());
    if hash.to_lowercase() != VBCABLE_SHA256 {
        let _ = std::fs::remove_file(target_zip_path);
        return Err(format!(
            "Checksum mismatch: expected {VBCABLE_SHA256}, got {hash}"
        ));
    }
    Ok(())
}

/// Extracts a zip archive to the given directory using the zip crate.
pub fn extract_zip(zip_path: &Path, extract_to: &Path) -> Result<(), String> {
    let file = std::fs::File::open(zip_path).map_err(|e| format!("Open zip error: {e}"))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("Read zip archive: {e}"))?;
    archive
        .extract(extract_to)
        .map_err(|e| format!("Extract error: {e}"))?;
    Ok(())
}

/// Launches the official installer elevated via ShellExecuteExW with the 'runas' verb.
#[cfg(windows)]
pub fn launch_vbcable_installer(extract_dir: &Path) -> Result<(), String> {
    if std::env::var_os("SORITONG_MOCK_INSTALLER").is_some() {
        return Ok(());
    }

    let exe_name = "VBCABLE_Setup_x64.exe";
    let exe_path = extract_dir.join(exe_name);
    if !exe_path.exists() {
        return Err(format!(
            "Installer executable not found: {}",
            exe_path.display()
        ));
    }

    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::Foundation::{CloseHandle, HWND};
    use windows::Win32::System::Threading::{INFINITE, WaitForSingleObject};
    use windows::Win32::UI::Shell::{SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW};
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOW;
    use windows::core::PCWSTR;

    let wide_verb: Vec<u16> = "runas\0".encode_utf16().collect();
    let wide_file: Vec<u16> = exe_path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let wide_dir: Vec<u16> = extract_dir
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS,
        hwnd: HWND(std::ptr::null_mut()),
        lpVerb: PCWSTR(wide_verb.as_ptr()),
        lpFile: PCWSTR(wide_file.as_ptr()),
        lpParameters: PCWSTR::null(),
        lpDirectory: PCWSTR(wide_dir.as_ptr()),
        nShow: SW_SHOW.0,
        ..Default::default()
    };

    unsafe {
        let ok = ShellExecuteExW(&mut info);
        if ok.is_err() {
            return Err("Installer execution was declined or cancelled.".to_string());
        }
        if !info.hProcess.is_invalid() {
            WaitForSingleObject(info.hProcess, INFINITE);
            let _ = CloseHandle(info.hProcess);
        }
    }

    Ok(())
}

#[cfg(not(windows))]
pub fn launch_vbcable_installer(_extract_dir: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pure_wizard_step_transitions() {
        let mut ctx = WizardContext::default();
        // 0. Unconfigured
        assert_eq!(decide_wizard_step(&ctx), WizardStep::ChoosePath);

        // 1. Chose virtual mic -> needs snapshot
        ctx.path_choice = Some(PathChoice::VirtualMic);
        assert_eq!(decide_wizard_step(&ctx), WizardStep::Detect);

        // 2. Snapshot taken, but cable not installed -> install step
        ctx.has_snapshot = true;
        ctx.cable_installed = false;
        assert_eq!(decide_wizard_step(&ctx), WizardStep::Install);

        // 3. Cable installed, but took over defaults -> restore defaults step
        ctx.cable_installed = true;
        ctx.defaults_are_cable = true;
        ctx.defaults_restored = false;
        assert_eq!(decide_wizard_step(&ctx), WizardStep::RestoreDefaults);

        // 4. Defaults restored, but soritong not configured -> configure step
        ctx.defaults_restored = true;
        ctx.soritong_configured = false;
        assert_eq!(decide_wizard_step(&ctx), WizardStep::ConfigureSoritong);

        // 5. Configured, but loopback not passed -> loopback step
        ctx.soritong_configured = true;
        ctx.loopback_passed = false;
        assert_eq!(decide_wizard_step(&ctx), WizardStep::LoopbackTest);

        // 6. Loopback passed -> Zoom instructions step
        ctx.loopback_passed = true;
        assert_eq!(decide_wizard_step(&ctx), WizardStep::ZoomStep);

        // 7. Completed -> complete
        ctx.completed = true;
        assert_eq!(decide_wizard_step(&ctx), WizardStep::Complete);
    }

    #[test]
    fn pure_wizard_skip_to_restore_if_already_installed() {
        let ctx = WizardContext {
            completed: false,
            path_choice: Some(PathChoice::VirtualMic),
            has_snapshot: true,
            cable_installed: true,
            defaults_are_cable: false,
            defaults_restored: false,
            soritong_configured: false,
            loopback_passed: false,
            reboot_pending: false,
        };
        // When already installed and defaults are fine, skips straight to ConfigureSoritong
        assert_eq!(decide_wizard_step(&ctx), WizardStep::ConfigureSoritong);
    }
}
