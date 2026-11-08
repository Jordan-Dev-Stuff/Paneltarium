//! Watch the config folder so profile/settings edits (from the CLI, the
//! future profile manager, or by hand) re-register hotkeys straight away.

use std::path::Path;

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

/// Keep the returned value alive; dropping it stops watching.
pub type ConfigWatcher = RecommendedWatcher;

/// Call `changed` (on a background thread) when a `.toml` file under `root`
/// is created, modified, renamed or removed. Reads are ignored (Linux
/// reports them, and the app reading its own config must not trigger
/// another reload). Log files and lock files are ignored too. One save can
/// produce several events; the app only re-registers hotkeys if the
/// content actually changed.
pub fn watch(root: &Path, changed: impl Fn() + Send + 'static) -> Result<ConfigWatcher, String> {
    std::fs::create_dir_all(root.join("profiles")).map_err(|e| format!("create {}: {e}", root.display()))?;
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<Event>| match res {
        Ok(event) if is_relevant(&event) => changed(),
        Ok(_) => {}
        Err(e) => tracing::warn!("config watcher: {e}"),
    })
    .map_err(|e| format!("could not watch {}: {e}", root.display()))?;
    watcher
        .watch(root, RecursiveMode::Recursive)
        .map_err(|e| format!("could not watch {}: {e}", root.display()))?;
    Ok(watcher)
}

fn is_relevant(event: &Event) -> bool {
    !matches!(event.kind, EventKind::Access(_)) && event.paths.iter().any(|p| is_config_file(p))
}

fn is_config_file(path: &Path) -> bool {
    // state.toml (last applied profile) changes on every apply; it isn't config.
    path.extension().is_some_and(|e| e == "toml") && path.file_name().is_none_or(|n| n != "state.toml")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn only_toml_files_count() {
        assert!(is_config_file(Path::new("/x/profiles/tv-only.toml")));
        assert!(is_config_file(Path::new("/x/settings.toml")));
        assert!(!is_config_file(Path::new("/x/profiles/tv-only.tmp")));
        assert!(!is_config_file(Path::new("/x/logs/paneltarium-app.log.2026-11-07")));
        assert!(!is_config_file(Path::new("/x/app.lock")));
        assert!(!is_config_file(Path::new("/x/state.toml")));
    }

    #[test]
    fn reads_are_not_changes() {
        use notify::event::{AccessKind, AccessMode, ModifyKind};
        let path = std::path::PathBuf::from("/x/settings.toml");
        let read = Event::new(EventKind::Access(AccessKind::Close(AccessMode::Read))).add_path(path.clone());
        assert!(!is_relevant(&read));
        let write = Event::new(EventKind::Modify(ModifyKind::Any)).add_path(path);
        assert!(is_relevant(&write));
    }

    #[test]
    fn reading_does_not_trigger() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("settings.toml");
        std::fs::write(&file, "x = 1").unwrap();
        std::thread::sleep(Duration::from_millis(100));
        let (tx, rx) = mpsc::channel();
        let _w = watch(dir.path(), move || {
            let _ = tx.send(());
        })
        .unwrap();
        let _ = std::fs::read_to_string(&file).unwrap();
        assert!(
            rx.recv_timeout(Duration::from_millis(500)).is_err(),
            "a read must not count"
        );
    }

    #[test]
    fn notices_profile_changes() {
        let dir = tempfile::tempdir().unwrap();
        let (tx, rx) = mpsc::channel();
        let _w = watch(dir.path(), move || {
            let _ = tx.send(());
        })
        .unwrap();
        std::fs::write(dir.path().join("profiles").join("a.toml"), "name = \"A\"").unwrap();
        rx.recv_timeout(Duration::from_secs(5)).expect("change noticed");
    }
}
