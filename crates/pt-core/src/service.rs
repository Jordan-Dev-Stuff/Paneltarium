//! The one facade every front end (CLI, hotkeys, tray, UI) goes through.

use crate::backend::DisplayBackend;
use crate::error::Result;
use crate::hotkey::Hotkey;
use crate::model::{DisplayLayout, Topology};
use crate::paths::AppPaths;
use crate::plan::{ApplyPlan, plan};
use crate::profile::{Profile, ProfileStore};
use crate::settings::Settings;
use crate::startup::{AppState, StartupProfile};

/// What happened when a profile was applied (or would have been).
#[derive(Debug, Clone)]
pub struct ApplyReport {
    pub profile: String,
    pub plan: ApplyPlan,
    pub dry_run: bool,
    /// Where the result differs from the profile (e.g. Windows picked another
    /// refresh rate). Empty for dry runs.
    pub differences: Vec<String>,
}

pub struct MonitorService {
    backend: Box<dyn DisplayBackend>,
    paths: AppPaths,
    store: ProfileStore,
}

impl MonitorService {
    pub fn new(backend: Box<dyn DisplayBackend>, paths: AppPaths) -> Self {
        let store = ProfileStore::new(paths.profiles_dir());
        MonitorService { backend, paths, store }
    }

    pub fn backend(&self) -> &dyn DisplayBackend {
        self.backend.as_ref()
    }

    pub fn paths(&self) -> &AppPaths {
        &self.paths
    }

    pub fn store(&self) -> &ProfileStore {
        &self.store
    }

    /// Settings are re-read on every call so edits by another process
    /// (CLI vs tray) are always picked up.
    pub fn settings(&self) -> Result<Settings> {
        Settings::load(&self.paths.settings_file())
    }

    pub fn save_settings(&self, settings: &Settings) -> Result<()> {
        settings.save(&self.paths.settings_file())
    }

    /// Current monitors, in a stable display order.
    pub fn current_layout(&self) -> Result<DisplayLayout> {
        let mut layout = self.backend.current_layout()?;
        layout.sort_for_display();
        Ok(layout)
    }

    /// Snapshot the current monitors into a profile.
    pub fn save_current(&self, name: &str, hotkey: Option<Hotkey>, overwrite: bool) -> Result<Profile> {
        let mut profile = Profile::new(name, self.current_layout()?)?;
        profile.hotkey = hotkey;
        // Keep an existing hotkey when overwriting without specifying one.
        if overwrite
            && hotkey.is_none()
            && let Ok(old) = self.store.get(name)
        {
            profile.hotkey = old.hotkey;
        }
        self.store
            .save(&profile, overwrite, &self.settings()?.reserved_hotkeys())?;
        Ok(profile)
    }

    pub fn plan_profile(&self, name: &str) -> Result<(Profile, ApplyPlan)> {
        let profile = self.store.get(name)?;
        let connected: Vec<_> = self
            .backend
            .current_layout()?
            .monitors
            .into_iter()
            .map(|m| m.id)
            .collect();
        let plan = plan(&profile.layout, &connected)?;
        Ok((profile, plan))
    }

    pub fn apply_profile(&self, name: &str, dry_run: bool) -> Result<ApplyReport> {
        let (profile, plan) = self.plan_profile(name)?;
        self.backend.apply(&plan, dry_run)?;
        if !dry_run {
            // Remember it for "load last used profile at sign-in". Best effort.
            let state = AppState {
                last_profile: Some(profile.name.clone()),
            };
            let _ = state.save(&self.paths.state_file());
        }
        let differences = if dry_run {
            Vec::new()
        } else {
            differences(&plan, &self.backend.current_layout()?)
        };
        Ok(ApplyReport {
            profile: profile.name,
            plan,
            dry_run,
            differences,
        })
    }

    pub fn apply_hotkey(&self, hotkey: Hotkey) -> Result<Option<ApplyReport>> {
        match self.store.find_by_hotkey(hotkey)? {
            Some(p) => self.apply_profile(&p.name, false).map(Some),
            None => Ok(None),
        }
    }

    /// The profile to apply at sign-in, per the setting, if it still exists.
    pub fn startup_target(&self) -> Result<Option<String>> {
        let name = match self.settings()?.startup_profile {
            StartupProfile::None => return Ok(None),
            StartupProfile::Last => match AppState::load(&self.paths.state_file()).last_profile {
                Some(n) => n,
                None => return Ok(None),
            },
            StartupProfile::Profile(n) => n,
        };
        Ok(self.store.exists(&name).then_some(name))
    }

    pub fn last_profile(&self) -> Option<String> {
        AppState::load(&self.paths.state_file()).last_profile
    }

    /// Rename a profile, keeping "last used" pointing at it.
    pub fn rename_profile(&self, old: &str, new: &str) -> Result<Profile> {
        let p = self.store.rename(old, new)?;
        let path = self.paths.state_file();
        let mut state = AppState::load(&path);
        if state
            .last_profile
            .as_deref()
            .is_some_and(|l| l.eq_ignore_ascii_case(old))
        {
            state.last_profile = Some(p.name.clone());
            let _ = state.save(&path);
        }
        Ok(p)
    }

    pub fn set_topology(&self, topology: Topology) -> Result<()> {
        self.backend.set_topology(topology)
    }

    /// "I lost my screen": duplicate onto every monitor, or, if the OS can't
    /// clone this combination (e.g. monitors on different GPUs), extend
    /// across all of them. Returns what was actually done.
    pub fn rescue(&self) -> Result<Topology> {
        match self.backend.set_topology(Topology::Clone) {
            Ok(()) => Ok(Topology::Clone),
            Err(clone_err) => self
                .backend
                .set_topology(Topology::Extend)
                .map(|()| Topology::Extend)
                .map_err(|_| clone_err),
        }
    }

    pub fn set_hotkey(&self, name: &str, hotkey: Option<Hotkey>) -> Result<Profile> {
        self.store
            .set_hotkey(name, hotkey, &self.settings()?.reserved_hotkeys())
    }
}

/// Human-readable differences between what a plan asked for and the layout
/// the OS reports afterwards.
pub fn differences(plan: &ApplyPlan, now: &DisplayLayout) -> Vec<String> {
    let mut out = Vec::new();
    for want in &plan.monitors {
        let label = want.target.label();
        let Some(got) = now
            .monitors
            .iter()
            .find(|m| m.id.device_path == want.target.device_path)
        else {
            out.push(format!("'{label}' is no longer connected"));
            continue;
        };
        match (want.enabled, got.enabled) {
            (true, false) => out.push(format!("'{label}' is still off")),
            (false, true) => out.push(format!("'{label}' is still on")),
            (false, false) => {}
            (true, true) => {
                if let (Some(w), Some(g)) = (want.mode, got.mode) {
                    if (w.width, w.height) != (g.width, g.height) {
                        out.push(format!(
                            "'{label}' is at {}x{} instead of {}x{}",
                            g.width, g.height, w.width, w.height
                        ));
                    } else if (w.refresh.hz() - g.refresh.hz()).abs() > 0.05 {
                        out.push(format!("'{label}' is at {} instead of {}", g.refresh, w.refresh));
                    }
                }
                if want.position != got.position {
                    out.push(format!("'{label}' is at {} instead of {}", got.position, want.position));
                }
                if want.rotation != got.rotation {
                    out.push(format!(
                        "'{label}' is rotated {:?} instead of {:?}",
                        got.rotation, want.rotation
                    ));
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::MockDisplayBackend;
    use crate::testing::sample_layout;

    fn service() -> (tempfile::TempDir, MonitorService) {
        let dir = tempfile::tempdir().unwrap();
        let svc = MonitorService::new(
            Box::new(MockDisplayBackend::new(sample_layout())),
            AppPaths::new(dir.path()),
        );
        (dir, svc)
    }

    #[test]
    fn save_then_apply_restores_layout() {
        let (_d, svc) = service();
        svc.save_current("Both", Some("Ctrl+Alt+1".parse().unwrap()), false)
            .unwrap();

        svc.set_topology(Topology::Clone).unwrap();
        assert_eq!(svc.current_layout().unwrap().enabled().count(), 3);

        let report = svc.apply_hotkey("Ctrl+Alt+1".parse().unwrap()).unwrap().unwrap();
        assert_eq!(report.profile, "Both");
        let now = svc.current_layout().unwrap();
        let mut expected = sample_layout();
        expected.sort_for_display();
        assert_eq!(now, expected);
    }

    #[test]
    fn dry_run_changes_nothing() {
        let (_d, svc) = service();
        svc.save_current("Both", None, false).unwrap();
        svc.set_topology(Topology::Extend).unwrap();
        let before = svc.current_layout().unwrap();
        let r = svc.apply_profile("both", true).unwrap();
        assert!(r.dry_run);
        assert_eq!(svc.current_layout().unwrap(), before);
    }

    #[test]
    fn overwrite_keeps_hotkey_and_reserved_hotkey_is_refused() {
        let (_d, svc) = service();
        svc.save_current("Desk", Some("Ctrl+Alt+2".parse().unwrap()), false)
            .unwrap();
        let p = svc.save_current("Desk", None, true).unwrap();
        assert_eq!(p.hotkey.unwrap().to_string(), "Ctrl+Alt+2");

        let err = svc.set_hotkey("Desk", Some(Hotkey::default_duplicate())).unwrap_err();
        assert!(err.to_string().contains("duplicate"), "{err}");
    }

    #[test]
    fn differences_are_reported() {
        let (_d, svc) = service();
        svc.save_current("Both", None, false).unwrap();
        let (_, plan) = svc.plan_profile("Both").unwrap();
        let mut now = sample_layout();
        assert!(differences(&plan, &now).is_empty());
        now.monitors[1].mode.as_mut().unwrap().refresh = crate::RefreshRate::from_hz(60);
        now.monitors[2].enabled = true;
        let d = differences(&plan, &now);
        assert_eq!(d.len(), 2, "{d:?}");
        assert!(d[0].contains("60 Hz instead of 144 Hz"), "{d:?}");
        assert!(d[1].contains("still on"), "{d:?}");

        now.monitors[2].enabled = false;
        now.monitors[1].mode.as_mut().unwrap().refresh = crate::RefreshRate::from_hz(144);
        now.monitors[0].rotation = crate::Rotation::Rotate90;
        let d = differences(&plan, &now);
        assert_eq!(d.len(), 1, "{d:?}");
        assert!(d[0].contains("rotated Rotate90 instead of None"), "{d:?}");
    }

    #[test]
    fn rescue_falls_back_to_extend() {
        struct NoClone(MockDisplayBackend);
        impl DisplayBackend for NoClone {
            fn name(&self) -> &'static str {
                "no-clone"
            }
            fn current_layout(&self) -> Result<DisplayLayout> {
                self.0.current_layout()
            }
            fn apply(&self, plan: &ApplyPlan, validate_only: bool) -> Result<()> {
                self.0.apply(plan, validate_only)
            }
            fn set_topology(&self, t: Topology) -> Result<()> {
                match t {
                    Topology::Clone => Err(crate::Error::Unsupported),
                    Topology::Extend => self.0.set_topology(t),
                }
            }
            fn supported_modes(&self, m: &crate::MonitorId) -> Result<Vec<crate::DisplayMode>> {
                self.0.supported_modes(m)
            }
        }

        let (_d, svc) = service();
        assert_eq!(svc.rescue().unwrap(), Topology::Clone);

        let dir = tempfile::tempdir().unwrap();
        let svc = MonitorService::new(
            Box::new(NoClone(MockDisplayBackend::new(sample_layout()))),
            AppPaths::new(dir.path()),
        );
        assert_eq!(svc.rescue().unwrap(), Topology::Extend);
        assert_eq!(svc.current_layout().unwrap().enabled().count(), 3);
    }

    #[test]
    fn startup_target_follows_setting_and_last_applied() {
        let (_d, svc) = service();
        svc.save_current("Desk", None, false).unwrap();
        svc.save_current("TV", None, false).unwrap();
        assert_eq!(svc.startup_target().unwrap(), None, "default: leave monitors alone");

        let set = |p: StartupProfile| {
            let s = Settings {
                startup_profile: p,
                ..svc.settings().unwrap()
            };
            svc.save_settings(&s).unwrap();
        };
        set(StartupProfile::Last);
        assert_eq!(svc.startup_target().unwrap(), None, "nothing applied yet");
        svc.apply_profile("tv", false).unwrap();
        assert_eq!(svc.startup_target().unwrap().as_deref(), Some("TV"));
        svc.apply_profile("Desk", true).unwrap();
        assert_eq!(svc.last_profile().as_deref(), Some("TV"), "dry runs don't count");

        svc.rename_profile("TV", "Television").unwrap();
        assert_eq!(svc.startup_target().unwrap().as_deref(), Some("Television"));

        set(StartupProfile::Profile("Desk".into()));
        assert_eq!(svc.startup_target().unwrap().as_deref(), Some("Desk"));
        svc.store().delete("Desk").unwrap();
        assert_eq!(svc.startup_target().unwrap(), None, "deleted profile is skipped");
    }

    #[test]
    fn unknown_hotkey_is_ignored() {
        let (_d, svc) = service();
        assert!(svc.apply_hotkey("Ctrl+Alt+9".parse().unwrap()).unwrap().is_none());
    }
}
