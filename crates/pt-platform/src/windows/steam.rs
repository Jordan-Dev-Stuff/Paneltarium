//! Is the Steam client running? Steam writes its process ID to
//! `HKCU\Software\Valve\Steam\ActiveProcess\pid` while it runs (and usually
//! sets it back to 0 on exit). A stale ID after a crash is caught by checking
//! that the process still exists.

use windows::Win32::Foundation::{CloseHandle, ERROR_SUCCESS, STILL_ACTIVE};
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows::Win32::System::Threading::{GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
use windows::core::w;

pub fn steam_running() -> bool {
    let mut pid = 0u32;
    let mut size = std::mem::size_of::<u32>() as u32;
    let r = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!(r"Software\Valve\Steam\ActiveProcess"),
            w!("pid"),
            RRF_RT_REG_DWORD,
            None,
            Some((&mut pid as *mut u32).cast()),
            Some(&mut size),
        )
    };
    if r != ERROR_SUCCESS || pid == 0 {
        return false;
    }
    unsafe {
        let Ok(process) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return false;
        };
        let mut code = 0u32;
        let alive = GetExitCodeProcess(process, &mut code).is_ok() && code == STILL_ACTIVE.0 as u32;
        let _ = CloseHandle(process);
        alive
    }
}
