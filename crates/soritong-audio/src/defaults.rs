//! Reading and restoring Windows default audio endpoints, and detecting "Listen to this device".

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct EndpointSnapshot {
    /// The IMMDevice endpoint ID string, e.g. "{0.0.0.00000000}.{c748bb65-...}".
    pub endpoint_id: String,
    /// Friendly name, e.g. "Speakers (Realtek(R) Audio)".
    pub friendly_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AudioDefaultsSnapshot {
    pub render_console: Option<EndpointSnapshot>,
    pub render_multimedia: Option<EndpointSnapshot>,
    pub render_communications: Option<EndpointSnapshot>,
    pub capture_console: Option<EndpointSnapshot>,
    pub capture_multimedia: Option<EndpointSnapshot>,
    pub capture_communications: Option<EndpointSnapshot>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AudioRole {
    Console,
    Multimedia,
    Communications,
    All,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RestoreReport {
    pub render_restored: bool,
    pub capture_restored: bool,
    pub restored_devices: Vec<String>,
}

#[cfg(windows)]
mod imp {
    use super::*;
    use std::ffi::c_void;
    use windows::Win32::Foundation::PROPERTYKEY;
    use windows::Win32::Media::Audio::*;
    use windows::Win32::System::Com::StructuredStorage::{
        PropVariantToBoolean, PropVariantToStringAlloc,
    };
    use windows::Win32::System::Com::*;
    use windows::core::{GUID, HRESULT, Interface, PCWSTR};

    pub const CLSID_POLICY_CONFIG_CLIENT: GUID =
        GUID::from_u128(0x870af99c_171d_4f9e_af0d_e63df40c2bc9);
    pub const IID_IPOLICY_CONFIG: GUID = GUID::from_u128(0xf8679f50_850a_41cf_9c72_430f290290c8);

    pub const PKEY_DEVICE_FRIENDLY_NAME: PROPERTYKEY = PROPERTYKEY {
        fmtid: GUID::from_u128(0xa45c254e_df1c_4efd_8020_67d146a850e0),
        pid: 14,
    };

    pub const PKEY_AUDIO_ENDPOINT_LISTEN: PROPERTYKEY = PROPERTYKEY {
        fmtid: GUID::from_u128(0x24dbb0fc_9311_4b3d_9cf0_18ff155639d4),
        pid: 1,
    };

    #[repr(C)]
    #[allow(non_snake_case)]
    pub struct IPolicyConfigVtbl {
        pub base__: windows::core::IUnknown_Vtbl,
        pub GetMixFormat:
            unsafe extern "system" fn(*mut c_void, *const u16, *mut *mut c_void) -> HRESULT,
        pub GetDeviceFormat:
            unsafe extern "system" fn(*mut c_void, *const u16, i32, *mut *mut c_void) -> HRESULT,
        pub ResetDeviceFormat: unsafe extern "system" fn(*mut c_void, *const u16) -> HRESULT,
        pub SetDeviceFormat:
            unsafe extern "system" fn(*mut c_void, *const u16, *mut c_void, *mut c_void) -> HRESULT,
        pub GetProcessingPeriod:
            unsafe extern "system" fn(*mut c_void, *const u16, i32, *mut i64, *mut i64) -> HRESULT,
        pub SetProcessingPeriod:
            unsafe extern "system" fn(*mut c_void, *const u16, *mut i64) -> HRESULT,
        pub GetShareMode:
            unsafe extern "system" fn(*mut c_void, *const u16, *mut c_void) -> HRESULT,
        pub SetShareMode:
            unsafe extern "system" fn(*mut c_void, *const u16, *mut c_void) -> HRESULT,
        pub GetPropertyValue: unsafe extern "system" fn(
            *mut c_void,
            *const u16,
            i32,
            *const c_void,
            *mut c_void,
        ) -> HRESULT,
        pub SetPropertyValue: unsafe extern "system" fn(
            *mut c_void,
            *const u16,
            i32,
            *const c_void,
            *mut c_void,
        ) -> HRESULT,
        pub SetDefaultEndpoint: unsafe extern "system" fn(*mut c_void, PCWSTR, ERole) -> HRESULT,
        pub SetEndpointVisibility:
            unsafe extern "system" fn(*mut c_void, *const u16, i32) -> HRESULT,
    }

    fn init_com() {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
    }

    fn get_endpoint_info(device: &IMMDevice) -> Option<EndpointSnapshot> {
        unsafe {
            let id_pwstr = device.GetId().ok()?;
            let endpoint_id = id_pwstr.to_string().ok()?;
            CoTaskMemFree(Some(id_pwstr.as_ptr() as *const c_void));

            let store = device.OpenPropertyStore(STGM_READ).ok()?;
            let prop = store.GetValue(&PKEY_DEVICE_FRIENDLY_NAME).ok()?;
            let friendly_name = PropVariantToStringAlloc(&prop)
                .ok()
                .and_then(|pw| {
                    let s = pw.to_string().ok();
                    CoTaskMemFree(Some(pw.as_ptr() as *const c_void));
                    s
                })
                .unwrap_or_else(|| endpoint_id.clone());

            Some(EndpointSnapshot {
                endpoint_id,
                friendly_name,
            })
        }
    }

    pub fn get_current_defaults() -> Result<AudioDefaultsSnapshot, String> {
        init_com();
        unsafe {
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
                    .map_err(|e| format!("IMMDeviceEnumerator: {e}"))?;

            let get_one = |flow: EDataFlow, role: ERole| -> Option<EndpointSnapshot> {
                let dev = enumerator.GetDefaultAudioEndpoint(flow, role).ok()?;
                get_endpoint_info(&dev)
            };

            Ok(AudioDefaultsSnapshot {
                render_console: get_one(eRender, eConsole),
                render_multimedia: get_one(eRender, eMultimedia),
                render_communications: get_one(eRender, eCommunications),
                capture_console: get_one(eCapture, eConsole),
                capture_multimedia: get_one(eCapture, eMultimedia),
                capture_communications: get_one(eCapture, eCommunications),
            })
        }
    }

    pub fn set_default_endpoint(endpoint_id: &str, role: AudioRole) -> Result<(), String> {
        init_com();
        let wide_id: Vec<u16> = endpoint_id
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let pcwstr = PCWSTR(wide_id.as_ptr());

        unsafe {
            let unk: windows::core::IUnknown =
                CoCreateInstance(&CLSID_POLICY_CONFIG_CLIENT, None, CLSCTX_ALL)
                    .map_err(|e| format!("CoCreateInstance PolicyConfigClient: {e}"))?;

            let policy_ptr = unk.as_raw();
            let vtbl = *(policy_ptr as *mut *mut IPolicyConfigVtbl);

            let roles = match role {
                AudioRole::Console => vec![eConsole],
                AudioRole::Multimedia => vec![eMultimedia],
                AudioRole::Communications => vec![eCommunications],
                AudioRole::All => vec![eConsole, eMultimedia, eCommunications],
            };

            for r in roles {
                let res = ((*vtbl).SetDefaultEndpoint)(policy_ptr, pcwstr, r);
                if res.is_err() {
                    return Err(format!("SetDefaultEndpoint failed for {r:?}: {res:?}"));
                }
            }

            Ok(())
        }
    }

    pub fn is_listen_to_device_enabled(capture_endpoint_id_or_name: &str) -> Result<bool, String> {
        init_com();
        unsafe {
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
                    .map_err(|e| format!("IMMDeviceEnumerator: {e}"))?;

            let collection = enumerator
                .EnumAudioEndpoints(eCapture, DEVICE_STATE_ACTIVE)
                .map_err(|e| format!("EnumAudioEndpoints: {e}"))?;

            let count = collection
                .GetCount()
                .map_err(|e| format!("GetCount: {e}"))?;
            for i in 0..count {
                if let Ok(dev) = collection.Item(i) {
                    if let Some(info) = get_endpoint_info(&dev) {
                        let matches = info.endpoint_id == capture_endpoint_id_or_name
                            || info
                                .friendly_name
                                .eq_ignore_ascii_case(capture_endpoint_id_or_name)
                            || (capture_endpoint_id_or_name.is_empty()
                                && info.friendly_name.contains("CABLE Output"));
                        if matches {
                            if let Ok(store) = dev.OpenPropertyStore(STGM_READ) {
                                if let Ok(prop) = store.GetValue(&PKEY_AUDIO_ENDPOINT_LISTEN) {
                                    if let Ok(b) = PropVariantToBoolean(&prop) {
                                        return Ok(b.as_bool());
                                    }
                                }
                            }
                        }
                    }
                }
            }
            Ok(false)
        }
    }
}

#[cfg(windows)]
pub use imp::*;

#[cfg(not(windows))]
pub fn get_current_defaults() -> Result<AudioDefaultsSnapshot, String> {
    Ok(AudioDefaultsSnapshot::default())
}

#[cfg(not(windows))]
pub fn set_default_endpoint(_endpoint_id: &str, _role: AudioRole) -> Result<(), String> {
    Ok(())
}

#[cfg(not(windows))]
pub fn is_listen_to_device_enabled(_capture: &str) -> Result<bool, String> {
    Ok(false)
}

/// Checks if a friendly name or ID corresponds to VB-CABLE.
pub fn is_cable_endpoint(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.contains("vb-audio") || lower.contains("cable input") || lower.contains("cable output")
}

/// Determines if render or capture defaults need to be restored because a CABLE device took over.
pub fn should_restore_defaults(
    current: &AudioDefaultsSnapshot,
    snapshot: &AudioDefaultsSnapshot,
) -> (bool, bool) {
    let render_is_cable = current
        .render_console
        .as_ref()
        .is_some_and(|s| is_cable_endpoint(&s.friendly_name))
        || current
            .render_multimedia
            .as_ref()
            .is_some_and(|s| is_cable_endpoint(&s.friendly_name))
        || current
            .render_communications
            .as_ref()
            .is_some_and(|s| is_cable_endpoint(&s.friendly_name));

    let target_render_ok = snapshot
        .render_console
        .as_ref()
        .or(snapshot.render_multimedia.as_ref())
        .or(snapshot.render_communications.as_ref())
        .is_some_and(|s| !is_cable_endpoint(&s.friendly_name));

    let capture_is_cable = current
        .capture_console
        .as_ref()
        .is_some_and(|s| is_cable_endpoint(&s.friendly_name))
        || current
            .capture_multimedia
            .as_ref()
            .is_some_and(|s| is_cable_endpoint(&s.friendly_name))
        || current
            .capture_communications
            .as_ref()
            .is_some_and(|s| is_cable_endpoint(&s.friendly_name));

    let target_capture_ok = snapshot
        .capture_console
        .as_ref()
        .or(snapshot.capture_multimedia.as_ref())
        .or(snapshot.capture_communications.as_ref())
        .is_some_and(|s| !is_cable_endpoint(&s.friendly_name));

    (
        render_is_cable && target_render_ok,
        capture_is_cable && target_capture_ok,
    )
}

/// Restores snapshot defaults if any current default is a CABLE device.
pub fn restore_defaults_if_cable(
    snapshot: &AudioDefaultsSnapshot,
) -> Result<RestoreReport, String> {
    let current = get_current_defaults()?;
    let (restore_render, restore_capture) = should_restore_defaults(&current, snapshot);
    let mut report = RestoreReport::default();

    if restore_render {
        if let Some(target) = snapshot
            .render_console
            .as_ref()
            .or(snapshot.render_multimedia.as_ref())
            .or(snapshot.render_communications.as_ref())
        {
            set_default_endpoint(&target.endpoint_id, AudioRole::All)?;
            report.render_restored = true;
            report.restored_devices.push(target.friendly_name.clone());
        }
    }

    if restore_capture {
        if let Some(target) = snapshot
            .capture_console
            .as_ref()
            .or(snapshot.capture_multimedia.as_ref())
            .or(snapshot.capture_communications.as_ref())
        {
            set_default_endpoint(&target.endpoint_id, AudioRole::All)?;
            report.capture_restored = true;
            report.restored_devices.push(target.friendly_name.clone());
        }
    }

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_restore_defaults_pure_test() {
        let snapshot = AudioDefaultsSnapshot {
            render_console: Some(EndpointSnapshot {
                endpoint_id: "dev-speakers".into(),
                friendly_name: "Realtek Speakers".into(),
            }),
            capture_console: Some(EndpointSnapshot {
                endpoint_id: "dev-mic".into(),
                friendly_name: "Logitech Mic".into(),
            }),
            ..Default::default()
        };

        // When current is normal speakers/mic: no restore needed
        let current_normal = AudioDefaultsSnapshot {
            render_console: Some(EndpointSnapshot {
                endpoint_id: "dev-speakers".into(),
                friendly_name: "Realtek Speakers".into(),
            }),
            capture_console: Some(EndpointSnapshot {
                endpoint_id: "dev-mic".into(),
                friendly_name: "Logitech Mic".into(),
            }),
            ..Default::default()
        };
        let (res_render, res_capture) = should_restore_defaults(&current_normal, &snapshot);
        assert!(!res_render);
        assert!(!res_capture);

        // When current render was taken over by VB-CABLE: restore render only
        let current_cable_render = AudioDefaultsSnapshot {
            render_console: Some(EndpointSnapshot {
                endpoint_id: "dev-cable".into(),
                friendly_name: "CABLE Input (VB-Audio Virtual Cable)".into(),
            }),
            capture_console: Some(EndpointSnapshot {
                endpoint_id: "dev-mic".into(),
                friendly_name: "Logitech Mic".into(),
            }),
            ..Default::default()
        };
        let (res_render, res_capture) = should_restore_defaults(&current_cable_render, &snapshot);
        assert!(res_render);
        assert!(!res_capture);
    }

    #[test]
    fn cable_name_detection() {
        assert!(is_cable_endpoint("CABLE Input (VB-Audio Virtual Cable)"));
        assert!(is_cable_endpoint("CABLE Output (VB-Audio Virtual Cable)"));
        assert!(is_cable_endpoint("VB-Audio Cable A"));
        assert!(!is_cable_endpoint("Speakers (Realtek(R) Audio)"));
        assert!(!is_cable_endpoint("Microphone (Logitech Webcam C925e)"));
    }

    #[test]
    #[ignore = "queries real Windows audio hardware"]
    fn live_query_defaults() {
        let defaults = get_current_defaults().expect("query defaults");
        println!("Render console: {:?}", defaults.render_console);
        println!("Capture console: {:?}", defaults.capture_console);
        println!("Listen enabled: {:?}", is_listen_to_device_enabled(""));
    }
}
