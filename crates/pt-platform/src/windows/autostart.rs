//! "Start with Windows": a value under
//! `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` — the per-user list of
//! programs Windows starts at sign-in (no admin rights needed; it's what
//! Task Manager → Startup apps shows).
//!
//! If the user switches it off in Task Manager, Windows keeps our Run value
//! and records "disabled" under `...\Explorer\StartupApproved\Run` (first
//! byte odd = disabled). We report that, and enabling here clears it.

use std::path::Path;

use pt_core::{AUTOSTART_ARG, Autostart, AutostartStatus, Result};
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, WIN32_ERROR};
use windows::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_BINARY, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
};
use windows::core::PCWSTR;

use super::ccd::os_error;

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const APPROVED_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";
const VALUE: &str = "Paneltarium";
/// The entry written by versions released as "Monitor Pilot".
const LEGACY_VALUE: &str = "MonitorPilot";

pub struct WindowsAutostart;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn check(call: &'static str, r: WIN32_ERROR) -> Result<()> {
    if r == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(os_error(call, r.0))
    }
}

/// Size in bytes of a registry value, or `None` if it doesn't exist.
fn value_size(
    key: &str,
    value: &str,
    flags: windows::Win32::System::Registry::REG_ROUTINE_FLAGS,
) -> Result<Option<u32>> {
    let (k, v) = (wide(key), wide(value));
    let mut size = 0u32;
    let r = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            PCWSTR(k.as_ptr()),
            PCWSTR(v.as_ptr()),
            flags,
            None,
            None,
            Some(&mut size),
        )
    };
    match r {
        ERROR_SUCCESS => Ok(Some(size)),
        ERROR_FILE_NOT_FOUND => Ok(None),
        e => Err(os_error("RegGetValueW", e.0)),
    }
}

/// Task Manager's on/off flag for our entry: `Some(true)` = disabled there.
fn disabled_in_task_manager(value: &str) -> Result<Option<bool>> {
    let Some(size) = value_size(APPROVED_KEY, value, RRF_RT_REG_BINARY)? else {
        return Ok(None);
    };
    let (k, v) = (wide(APPROVED_KEY), wide(value));
    let mut buf = vec![0u8; size.max(1) as usize];
    let mut len = buf.len() as u32;
    let r = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            PCWSTR(k.as_ptr()),
            PCWSTR(v.as_ptr()),
            RRF_RT_REG_BINARY,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut len),
        )
    };
    check("RegGetValueW", r)?;
    Ok(Some(buf.first().is_some_and(|b| b & 1 == 1)))
}

fn delete(key: &str, value: &str) -> Result<()> {
    let (k, v) = (wide(key), wide(value));
    match unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, PCWSTR(k.as_ptr()), PCWSTR(v.as_ptr())) } {
        ERROR_SUCCESS | ERROR_FILE_NOT_FOUND => Ok(()),
        e => Err(os_error("RegDeleteKeyValueW", e.0)),
    }
}

fn status_of(value: &str) -> Result<AutostartStatus> {
    if value_size(RUN_KEY, value, RRF_RT_REG_SZ)?.is_none() {
        return Ok(AutostartStatus::Off);
    }
    Ok(match disabled_in_task_manager(value)? {
        Some(true) => AutostartStatus::DisabledInSystem,
        _ => AutostartStatus::On,
    })
}

impl Autostart for WindowsAutostart {
    fn status(&self) -> Result<AutostartStatus> {
        status_of(VALUE)
    }

    fn enable(&self, exe: &Path) -> Result<()> {
        let command = format!("\"{}\" {AUTOSTART_ARG}", exe.display());
        let (k, v, data) = (wide(RUN_KEY), wide(VALUE), wide(&command));
        let r = unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                PCWSTR(k.as_ptr()),
                PCWSTR(v.as_ptr()),
                REG_SZ.0,
                Some(data.as_ptr().cast()),
                (data.len() * 2) as u32,
            )
        };
        check("RegSetKeyValueW", r)?;
        // Undo a "Disabled" from Task Manager, so turning it on here works.
        delete(APPROVED_KEY, VALUE)
    }

    fn disable(&self) -> Result<()> {
        for value in [VALUE, LEGACY_VALUE] {
            delete(RUN_KEY, value)?;
            delete(APPROVED_KEY, value)?;
        }
        Ok(())
    }

    fn migrate_legacy(&self, exe: &Path) -> Result<bool> {
        let old = status_of(LEGACY_VALUE)?;
        if old == AutostartStatus::Off {
            return Ok(false);
        }
        if old == AutostartStatus::On && self.status()? == AutostartStatus::Off {
            self.enable(exe)?;
        }
        delete(RUN_KEY, LEGACY_VALUE)?;
        delete(APPROVED_KEY, LEGACY_VALUE)?;
        Ok(true)
    }
}
