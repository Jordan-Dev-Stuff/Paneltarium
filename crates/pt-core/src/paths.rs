use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// Environment variable that relocates all config files (used by tests and
/// handy for portable installs).
pub const HOME_ENV: &str = "PANELTARIUM_HOME";

/// Where Paneltarium keeps its files.
///
/// Windows default: `%APPDATA%\Paneltarium\`
/// Linux default:   `$XDG_CONFIG_HOME/paneltarium/`
#[derive(Debug, Clone)]
pub struct AppPaths {
    root: PathBuf,
}

impl AppPaths {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        AppPaths { root: root.into() }
    }

    /// Resolve from `PANELTARIUM_HOME`, falling back to the OS config dir.
    pub fn from_env() -> Result<Self> {
        if let Some(dir) = std::env::var_os(HOME_ENV).filter(|v| !v.is_empty()) {
            return Ok(Self::new(dir));
        }
        let base = dirs::config_dir().ok_or(Error::NoConfigDir)?;
        let (name, legacy) = if cfg!(windows) {
            ("Paneltarium", "MonitorPilot")
        } else {
            ("paneltarium", "monitor-pilot")
        };
        let root = base.join(name);
        if let Err(e) = migrate_legacy(&base.join(legacy), &root) {
            // Not fatal: the user just starts with empty settings.
            eprintln!("paneltarium: couldn't copy settings from the old Monitor Pilot folder: {e}");
        }
        Ok(Self::new(root))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn profiles_dir(&self) -> PathBuf {
        self.root.join("profiles")
    }

    pub fn settings_file(&self) -> PathBuf {
        self.root.join("settings.toml")
    }

    pub fn log_dir(&self) -> PathBuf {
        self.root.join("logs")
    }

    /// Remembered state (last applied profile); not settings.
    pub fn state_file(&self) -> PathBuf {
        self.root.join("state.toml")
    }
}

/// The app used to be called Monitor Pilot. On first run, copy its profiles
/// and settings across (the old folder is left alone as a backup). Does
/// nothing once `root` exists or if there's nothing to copy.
pub fn migrate_legacy(legacy: &Path, root: &Path) -> Result<bool> {
    if root.exists() || !legacy.is_dir() {
        return Ok(false);
    }
    copy_dir(legacy, root)?;
    Ok(true)
}

fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    std::fs::create_dir_all(to).map_err(|e| Error::io(to, e))?;
    for entry in std::fs::read_dir(from).map_err(|e| Error::io(from, e))? {
        let entry = entry.map_err(|e| Error::io(from, e))?;
        let (src, dst) = (entry.path(), to.join(entry.file_name()));
        let kind = entry.file_type().map_err(|e| Error::io(&src, e))?;
        if kind.is_dir() {
            // Old logs and lock files aren't worth carrying over.
            if entry.file_name() != "logs" {
                copy_dir(&src, &dst)?;
            }
        } else if kind.is_file() && src.extension().is_some_and(|x| x == "toml") {
            std::fs::copy(&src, &dst).map_err(|e| Error::io(&src, e))?;
        }
    }
    Ok(())
}

/// Another Paneltarium program shipped next to the running one, e.g.
/// `sibling_exe("paneltarium-app")` → `...\paneltarium-app.exe`.
pub fn sibling_exe(name: &str) -> Result<PathBuf> {
    let me = std::env::current_exe().map_err(|e| Error::io("current executable", e))?;
    Ok(me.with_file_name(format!("{name}{}", std::env::consts::EXE_SUFFIX)))
}

/// Write a file atomically-ish: write a sibling temp file, then rename over.
pub(crate) fn write_atomic(path: &Path, contents: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, contents).map_err(|e| Error::io(&tmp, e))?;
    std::fs::rename(&tmp, path).map_err(|e| Error::io(path, e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_folder_is_copied_once() {
        let dir = tempfile::tempdir().unwrap();
        let (old, new) = (dir.path().join("MonitorPilot"), dir.path().join("Paneltarium"));
        // Nothing to migrate.
        assert!(!migrate_legacy(&old, &new).unwrap());
        std::fs::create_dir_all(old.join("profiles")).unwrap();
        std::fs::create_dir_all(old.join("logs")).unwrap();
        std::fs::write(old.join("settings.toml"), "gamepad_enabled = false").unwrap();
        std::fs::write(old.join("profiles/desk.toml"), "name = 'Desk'").unwrap();
        std::fs::write(old.join("logs/app.log"), "old").unwrap();
        std::fs::write(old.join("app.lock"), "").unwrap();

        assert!(migrate_legacy(&old, &new).unwrap());
        assert!(new.join("settings.toml").is_file());
        assert!(new.join("profiles/desk.toml").is_file());
        assert!(!new.join("logs").exists());
        assert!(!new.join("app.lock").exists());
        assert!(old.join("settings.toml").is_file(), "old folder is kept as a backup");

        // Second run: the new folder exists, so nothing is overwritten.
        std::fs::write(new.join("settings.toml"), "changed").unwrap();
        assert!(!migrate_legacy(&old, &new).unwrap());
        assert_eq!(std::fs::read_to_string(new.join("settings.toml")).unwrap(), "changed");
    }
}
