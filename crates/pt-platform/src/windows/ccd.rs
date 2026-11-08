//! Safe wrappers over the Win32 display-configuration calls. All `unsafe`
//! for display configuration lives in this file.

use std::mem::size_of;

use pt_core::{DisplayMode, Error, RefreshRate, Result, Rotation};
use windows::Win32::Devices::Display::{
    DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME, DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME, DISPLAYCONFIG_MODE_INFO,
    DISPLAYCONFIG_MODE_INFO_TYPE_SOURCE, DISPLAYCONFIG_MODE_INFO_TYPE_TARGET, DISPLAYCONFIG_PATH_INFO,
    DISPLAYCONFIG_ROTATION, DISPLAYCONFIG_ROTATION_IDENTITY, DISPLAYCONFIG_ROTATION_ROTATE90,
    DISPLAYCONFIG_ROTATION_ROTATE180, DISPLAYCONFIG_ROTATION_ROTATE270, DISPLAYCONFIG_SOURCE_DEVICE_NAME,
    DISPLAYCONFIG_SOURCE_MODE, DISPLAYCONFIG_TARGET_DEVICE_NAME, DisplayConfigGetDeviceInfo,
    GetDisplayConfigBufferSizes, QUERY_DISPLAY_CONFIG_FLAGS, QueryDisplayConfig, SET_DISPLAY_CONFIG_FLAGS,
    SetDisplayConfig,
};
use windows::Win32::Foundation::{ERROR_INSUFFICIENT_BUFFER, ERROR_SUCCESS, LUID};
use windows::Win32::Graphics::Gdi::{
    DEVMODEW, DISPLAYCONFIG_PATH_MODE_IDX_INVALID, ENUM_DISPLAY_SETTINGS_MODE, EnumDisplaySettingsW,
};
use windows::core::{HRESULT, PCWSTR};

/// Identifies a graphics-adapter output (a "target") or input (a "source").
/// `LUID`s are only stable until reboot, so never persist these.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AdapterKey {
    pub low: u32,
    pub high: i32,
    pub id: u32,
}

impl AdapterKey {
    pub fn new(luid: LUID, id: u32) -> Self {
        AdapterKey {
            low: luid.LowPart,
            high: luid.HighPart,
            id,
        }
    }

    pub fn luid(&self) -> LUID {
        LUID {
            LowPart: self.low,
            HighPart: self.high,
        }
    }
}

pub fn target_key(p: &DISPLAYCONFIG_PATH_INFO) -> AdapterKey {
    AdapterKey::new(p.targetInfo.adapterId, p.targetInfo.id)
}

pub fn source_key(p: &DISPLAYCONFIG_PATH_INFO) -> AdapterKey {
    AdapterKey::new(p.sourceInfo.adapterId, p.sourceInfo.id)
}

pub fn os_error(call: &'static str, code: u32) -> Error {
    let message = HRESULT::from_win32(code).message();
    let hint = match code {
        5 => " (access denied: is the workstation locked or is this running as a service?)",
        87 => " (Windows rejected this combination of monitors/modes)",
        1610 => " (the configuration is not supported by the graphics driver)",
        _ => "",
    };
    Error::Os {
        call,
        code: Some(code as i64),
        message: format!("{}{hint}", message.trim()),
    }
}

/// Result of `QueryDisplayConfig`.
pub struct DisplayConfig {
    pub paths: Vec<DISPLAYCONFIG_PATH_INFO>,
    pub modes: Vec<DISPLAYCONFIG_MODE_INFO>,
}

impl DisplayConfig {
    pub fn query(flags: QUERY_DISPLAY_CONFIG_FLAGS) -> Result<Self> {
        // The configuration can change between the two calls (hot-plug), in
        // which case Windows reports ERROR_INSUFFICIENT_BUFFER: just retry.
        for _ in 0..8 {
            let (mut n_paths, mut n_modes) = (0u32, 0u32);
            let r = unsafe { GetDisplayConfigBufferSizes(flags, &mut n_paths, &mut n_modes) };
            if r != ERROR_SUCCESS {
                return Err(os_error("GetDisplayConfigBufferSizes", r.0));
            }
            let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); n_paths as usize];
            let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); n_modes as usize];
            let r = unsafe {
                QueryDisplayConfig(
                    flags,
                    &mut n_paths,
                    paths.as_mut_ptr(),
                    &mut n_modes,
                    modes.as_mut_ptr(),
                    None,
                )
            };
            if r == ERROR_INSUFFICIENT_BUFFER {
                continue;
            }
            if r != ERROR_SUCCESS {
                return Err(os_error("QueryDisplayConfig", r.0));
            }
            paths.truncate(n_paths as usize);
            modes.truncate(n_modes as usize);
            return Ok(DisplayConfig { paths, modes });
        }
        Err(Error::Os {
            call: "QueryDisplayConfig",
            code: None,
            message: "display configuration kept changing; try again".into(),
        })
    }

    /// The source (desktop) mode an active path uses, if any.
    pub fn source_mode(&self, p: &DISPLAYCONFIG_PATH_INFO) -> Option<DISPLAYCONFIG_SOURCE_MODE> {
        // We never pass QDC_VIRTUAL_MODE_AWARE, so the whole u32 is the index.
        let idx = unsafe { p.sourceInfo.Anonymous.modeInfoIdx };
        let mode = self.mode_at(idx)?;
        if mode.infoType == DISPLAYCONFIG_MODE_INFO_TYPE_SOURCE {
            // Only read the union member that `infoType` says is valid.
            Some(unsafe { mode.Anonymous.sourceMode })
        } else {
            None
        }
    }

    /// Refresh rate of an active path. Prefers the path's own rate and falls
    /// back to the target mode's vertical sync frequency.
    pub fn refresh(&self, p: &DISPLAYCONFIG_PATH_INFO) -> RefreshRate {
        let r = p.targetInfo.refreshRate;
        if r.Denominator != 0 && r.Numerator != 0 {
            return RefreshRate {
                numerator: r.Numerator,
                denominator: r.Denominator,
            };
        }
        let idx = unsafe { p.targetInfo.Anonymous.modeInfoIdx };
        match self.mode_at(idx) {
            Some(m) if m.infoType == DISPLAYCONFIG_MODE_INFO_TYPE_TARGET => {
                let v = unsafe { m.Anonymous.targetMode.targetVideoSignalInfo.vSyncFreq };
                RefreshRate {
                    numerator: v.Numerator,
                    denominator: v.Denominator,
                }
            }
            _ => RefreshRate {
                numerator: 0,
                denominator: 1,
            },
        }
    }

    /// The target (signal timing) mode an active path uses, if any.
    pub fn target_mode(&self, p: &DISPLAYCONFIG_PATH_INFO) -> Option<DISPLAYCONFIG_MODE_INFO> {
        let idx = unsafe { p.targetInfo.Anonymous.modeInfoIdx };
        self.mode_at(idx)
            .filter(|m| m.infoType == DISPLAYCONFIG_MODE_INFO_TYPE_TARGET)
            .copied()
    }

    fn mode_at(&self, idx: u32) -> Option<&DISPLAYCONFIG_MODE_INFO> {
        if idx == DISPLAYCONFIG_PATH_MODE_IDX_INVALID {
            None
        } else {
            self.modes.get(idx as usize)
        }
    }
}

/// Environment variable: when set, the raw configuration is printed to stderr.
pub const DEBUG_ENV: &str = "PANELTARIUM_DEBUG";

pub fn debug_enabled() -> bool {
    std::env::var_os(DEBUG_ENV).is_some_and(|v| !v.is_empty() && v != "0")
}

/// Print a path/mode array in a form that can be pasted into a bug report.
pub fn dump_config(label: &str, paths: &[DISPLAYCONFIG_PATH_INFO], modes: &[DISPLAYCONFIG_MODE_INFO]) {
    eprintln!("--- {label}: {} path(s), {} mode(s)", paths.len(), modes.len());
    for (i, p) in paths.iter().enumerate() {
        let s = source_key(p);
        let t = target_key(p);
        let (src_idx, tgt_idx) = unsafe { (p.sourceInfo.Anonymous.modeInfoIdx, p.targetInfo.Anonymous.modeInfoIdx) };
        let r = p.targetInfo.refreshRate;
        eprintln!(
            "path[{i}] flags={:#x} src={:x}:{:x}/{} srcMode={} tgt={:x}:{:x}/{} tgtMode={} refresh={}/{} scan={} rot={} scale={} avail={} tech={}",
            p.flags,
            s.high,
            s.low,
            s.id,
            idx(src_idx),
            t.high,
            t.low,
            t.id,
            idx(tgt_idx),
            r.Numerator,
            r.Denominator,
            p.targetInfo.scanLineOrdering.0,
            p.targetInfo.rotation.0,
            p.targetInfo.scaling.0,
            p.targetInfo.targetAvailable.as_bool(),
            p.targetInfo.outputTechnology.0,
        );
    }
    for (i, m) in modes.iter().enumerate() {
        if m.infoType == DISPLAYCONFIG_MODE_INFO_TYPE_SOURCE {
            let sm = unsafe { m.Anonymous.sourceMode };
            eprintln!(
                "mode[{i}] SOURCE {:x}:{:x}/{} {}x{} fmt={} pos=({}, {})",
                m.adapterId.HighPart,
                m.adapterId.LowPart,
                m.id,
                sm.width,
                sm.height,
                sm.pixelFormat.0,
                sm.position.x,
                sm.position.y
            );
        } else if m.infoType == DISPLAYCONFIG_MODE_INFO_TYPE_TARGET {
            let v = unsafe { m.Anonymous.targetMode.targetVideoSignalInfo };
            eprintln!(
                "mode[{i}] TARGET {:x}:{:x}/{} active={}x{} vsync={}/{} scan={}",
                m.adapterId.HighPart,
                m.adapterId.LowPart,
                m.id,
                v.activeSize.cx,
                v.activeSize.cy,
                v.vSyncFreq.Numerator,
                v.vSyncFreq.Denominator,
                v.scanLineOrdering.0
            );
        } else {
            eprintln!("mode[{i}] type={}", m.infoType.0);
        }
    }
}

fn idx(i: u32) -> String {
    if i == DISPLAYCONFIG_PATH_MODE_IDX_INVALID {
        "none".into()
    } else {
        i.to_string()
    }
}

pub fn set_display_config(
    paths: Option<&[DISPLAYCONFIG_PATH_INFO]>,
    modes: Option<&[DISPLAYCONFIG_MODE_INFO]>,
    flags: SET_DISPLAY_CONFIG_FLAGS,
) -> Result<()> {
    let r = unsafe { SetDisplayConfig(paths, modes, flags) };
    if r == 0 {
        Ok(())
    } else {
        Err(os_error("SetDisplayConfig", r as u32))
    }
}

/// Monitor information for a target: friendly name, device path, EDID ids.
pub struct TargetName {
    pub friendly_name: String,
    pub device_path: String,
    pub edid: Option<(u16, u16)>,
}

pub fn target_name(target: AdapterKey) -> Result<TargetName> {
    let mut req = DISPLAYCONFIG_TARGET_DEVICE_NAME::default();
    req.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME;
    req.header.size = size_of::<DISPLAYCONFIG_TARGET_DEVICE_NAME>() as u32;
    req.header.adapterId = target.luid();
    req.header.id = target.id;
    let r = unsafe { DisplayConfigGetDeviceInfo(&mut req.header) };
    if r != 0 {
        return Err(os_error("DisplayConfigGetDeviceInfo(target name)", r as u32));
    }
    // Flag bits: 0 friendlyNameFromEdid, 1 friendlyNameForced, 2 edidIdsValid.
    let edid_valid = unsafe { req.flags.Anonymous.value } & 0x4 != 0;
    Ok(TargetName {
        friendly_name: from_wide(&req.monitorFriendlyDeviceName),
        device_path: from_wide(&req.monitorDevicePath),
        edid: edid_valid.then_some((req.edidManufactureId, req.edidProductCodeId)),
    })
}

/// GDI device name of a source, e.g. `\\.\DISPLAY1`, NUL-terminated.
pub fn source_gdi_name(source: AdapterKey) -> Result<[u16; 32]> {
    let mut req = DISPLAYCONFIG_SOURCE_DEVICE_NAME::default();
    req.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME;
    req.header.size = size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32;
    req.header.adapterId = source.luid();
    req.header.id = source.id;
    let r = unsafe { DisplayConfigGetDeviceInfo(&mut req.header) };
    if r != 0 {
        return Err(os_error("DisplayConfigGetDeviceInfo(source name)", r as u32));
    }
    Ok(req.viewGdiDeviceName)
}

/// All 32-bit modes the GDI device supports, de-duplicated, best first.
pub fn enum_modes(gdi_name: &[u16; 32]) -> Vec<DisplayMode> {
    let mut modes = Vec::new();
    for i in 0.. {
        let mut dm = DEVMODEW {
            dmSize: size_of::<DEVMODEW>() as u16,
            ..Default::default()
        };
        let ok = unsafe { EnumDisplaySettingsW(PCWSTR(gdi_name.as_ptr()), ENUM_DISPLAY_SETTINGS_MODE(i), &mut dm) };
        if !ok.as_bool() {
            break;
        }
        if dm.dmBitsPerPel != 32 || dm.dmDisplayFrequency <= 1 {
            continue;
        }
        let mode = DisplayMode {
            width: dm.dmPelsWidth,
            height: dm.dmPelsHeight,
            refresh: RefreshRate::from_hz(dm.dmDisplayFrequency),
        };
        if !modes.contains(&mode) {
            modes.push(mode);
        }
    }
    modes.sort_by(|a, b| {
        (b.width * b.height)
            .cmp(&(a.width * a.height))
            .then(b.width.cmp(&a.width))
            .then(b.refresh.numerator.cmp(&a.refresh.numerator))
    });
    modes
}

pub fn rotation_from_win(r: DISPLAYCONFIG_ROTATION) -> Rotation {
    match r {
        DISPLAYCONFIG_ROTATION_ROTATE90 => Rotation::Rotate90,
        DISPLAYCONFIG_ROTATION_ROTATE180 => Rotation::Rotate180,
        DISPLAYCONFIG_ROTATION_ROTATE270 => Rotation::Rotate270,
        _ => Rotation::None,
    }
}

pub fn rotation_to_win(r: Rotation) -> DISPLAYCONFIG_ROTATION {
    match r {
        Rotation::None => DISPLAYCONFIG_ROTATION_IDENTITY,
        Rotation::Rotate90 => DISPLAYCONFIG_ROTATION_ROTATE90,
        Rotation::Rotate180 => DISPLAYCONFIG_ROTATION_ROTATE180,
        Rotation::Rotate270 => DISPLAYCONFIG_ROTATION_ROTATE270,
    }
}

fn from_wide(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}
