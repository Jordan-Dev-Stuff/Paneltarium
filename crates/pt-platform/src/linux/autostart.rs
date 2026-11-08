//! Start at sign-in on Linux desktops: a `.desktop` file in
//! `$XDG_CONFIG_HOME/autostart/` (usually `~/.config/autostart/`), the
//! freedesktop.org standard that GNOME, KDE, XFCE, ... all follow.

use std::path::{Path, PathBuf};

use pt_core::{AUTOSTART_ARG, Autostart, AutostartStatus, Error, Result};

pub struct XdgAutostart {
    file: PathBuf,
    /// Written by versions released as "Monitor Pilot".
    legacy: PathBuf,
}

impl XdgAutostart {
    /// `dir` is the autostart folder (`~/.config/autostart`).
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        let dir = dir.into();
        XdgAutostart {
            file: dir.join("paneltarium.desktop"),
            legacy: dir.join("monitor-pilot.desktop"),
        }
    }

    pub fn for_user() -> Result<Self> {
        let config = dirs::config_dir().ok_or(Error::NoConfigDir)?;
        Ok(Self::new(config.join("autostart")))
    }
}

fn io(path: &Path, e: std::io::Error) -> Error {
    Error::Os {
        call: "autostart",
        code: e.raw_os_error().map(i64::from),
        message: format!("{}: {e}", path.display()),
    }
}

fn status_of(file: &Path) -> Result<AutostartStatus> {
    let text = match std::fs::read_to_string(file) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(AutostartStatus::Off),
        Err(e) => return Err(io(file, e)),
    };
    // Desktop "Startup Applications" tools switch entries off with these.
    let off = text
        .lines()
        .any(|l| matches!(l.trim(), "Hidden=true" | "X-GNOME-Autostart-enabled=false"));
    Ok(if off {
        AutostartStatus::DisabledInSystem
    } else {
        AutostartStatus::On
    })
}

fn remove(file: &Path) -> Result<()> {
    match std::fs::remove_file(file) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(io(file, e)),
    }
}

impl Autostart for XdgAutostart {
    fn status(&self) -> Result<AutostartStatus> {
        status_of(&self.file)
    }

    fn enable(&self, exe: &Path) -> Result<()> {
        if let Some(dir) = self.file.parent() {
            std::fs::create_dir_all(dir).map_err(|e| io(dir, e))?;
        }
        let text = format!(
            "[Desktop Entry]\nType=Application\nName=Paneltarium\nComment=Monitor profiles, hotkeys and overlay\n\
             Exec=\"{}\" {AUTOSTART_ARG}\nTerminal=false\nHidden=false\nX-GNOME-Autostart-enabled=true\n",
            exe.display()
        );
        std::fs::write(&self.file, text).map_err(|e| io(&self.file, e))
    }

    fn disable(&self) -> Result<()> {
        remove(&self.file)?;
        remove(&self.legacy)
    }

    fn migrate_legacy(&self, exe: &Path) -> Result<bool> {
        let old = status_of(&self.legacy)?;
        if old == AutostartStatus::Off {
            return Ok(false);
        }
        if old == AutostartStatus::On && self.status()? == AutostartStatus::Off {
            self.enable(exe)?;
        }
        remove(&self.legacy)?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enable_status_disable() {
        let dir = tempfile::tempdir().unwrap();
        let a = XdgAutostart::new(dir.path().join("autostart"));
        assert_eq!(a.status().unwrap(), AutostartStatus::Off);
        a.enable(Path::new("/opt/mp/paneltarium-app")).unwrap();
        assert_eq!(a.status().unwrap(), AutostartStatus::On);
        let text = std::fs::read_to_string(&a.file).unwrap();
        assert!(text.contains("Exec=\"/opt/mp/paneltarium-app\" --autostart"), "{text}");

        // Switched off by the desktop's startup settings.
        std::fs::write(&a.file, text.replace("Hidden=false", "Hidden=true")).unwrap();
        assert_eq!(a.status().unwrap(), AutostartStatus::DisabledInSystem);
        a.enable(Path::new("/opt/mp/paneltarium-app")).unwrap();
        assert_eq!(
            a.status().unwrap(),
            AutostartStatus::On,
            "enabling here turns it back on"
        );

        a.disable().unwrap();
        assert_eq!(a.status().unwrap(), AutostartStatus::Off);
        a.disable().unwrap();
    }

    #[test]
    fn legacy_entry_is_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let a = XdgAutostart::new(dir.path());
        let exe = Path::new("/opt/pt/paneltarium-app");
        assert!(!a.migrate_legacy(exe).unwrap(), "nothing to migrate");

        std::fs::write(&a.legacy, "[Desktop Entry]\nExec=/old/monitor-pilot-app --autostart\n").unwrap();
        assert!(a.migrate_legacy(exe).unwrap());
        assert!(!a.legacy.exists());
        assert_eq!(a.status().unwrap(), AutostartStatus::On);

        // Switched off under the old name: stays off under the new one.
        a.disable().unwrap();
        std::fs::write(&a.legacy, "[Desktop Entry]\nHidden=true\n").unwrap();
        assert!(a.migrate_legacy(exe).unwrap());
        assert!(!a.legacy.exists());
        assert_eq!(a.status().unwrap(), AutostartStatus::Off);
    }
}
