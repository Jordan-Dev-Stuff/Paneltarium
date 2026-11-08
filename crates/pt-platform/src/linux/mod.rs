//! Linux implementations. So far only autostart; the display backend comes
//! later (X11 RandR / Wayland protocols, see the roadmap).

mod autostart;

pub use autostart::XdgAutostart;

/// Is the Steam client running? Steam writes its process ID to
/// `~/.steam/steam.pid`; check that process still exists.
pub fn steam_running() -> bool {
    let Some(home) = dirs::home_dir() else { return false };
    steam_running_in(&home.join(".steam/steam.pid"), std::path::Path::new("/proc"))
}

fn steam_running_in(pid_file: &std::path::Path, proc_dir: &std::path::Path) -> bool {
    std::fs::read_to_string(pid_file)
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
        .filter(|&pid| pid != 0)
        .is_some_and(|pid| proc_dir.join(pid.to_string()).exists())
}

#[cfg(test)]
mod tests {
    #[test]
    fn steam_pid_file_must_point_at_a_live_process() {
        let dir = tempfile::tempdir().unwrap();
        let (pid_file, proc_dir) = (dir.path().join("steam.pid"), dir.path().join("proc"));
        std::fs::create_dir_all(proc_dir.join("4242")).unwrap();
        assert!(!super::steam_running_in(&pid_file, &proc_dir), "no pid file");
        std::fs::write(&pid_file, "4242\n").unwrap();
        assert!(super::steam_running_in(&pid_file, &proc_dir));
        std::fs::write(&pid_file, "999").unwrap();
        assert!(!super::steam_running_in(&pid_file, &proc_dir), "stale pid");
        std::fs::write(&pid_file, "garbage").unwrap();
        assert!(!super::steam_running_in(&pid_file, &proc_dir));
    }
}
