//! Which saved profile (or "duplicate on all monitors") is on screen right
//! now? Worked out from the actual monitor state, so it's right even after
//! changes made outside Paneltarium (Windows settings, Win+P, ...).

use crate::model::DisplayLayout;
use crate::plan::{expected_layout, plan};
use crate::profile::Profile;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Current {
    Profile(String),
    /// Every monitor shows the same picture.
    Duplicate,
    /// Doesn't match any profile.
    Unknown,
}

impl Current {
    pub fn is_profile(&self, name: &str) -> bool {
        matches!(self, Current::Profile(n) if n.eq_ignore_ascii_case(name))
    }
}

/// The first profile matching `now` wins; a duplicated setup that isn't
/// saved as a profile is reported as [`Current::Duplicate`].
pub fn detect(profiles: &[Profile], now: &DisplayLayout) -> Current {
    if let Some(p) = profiles.iter().find(|p| matches(&p.layout, now)) {
        return Current::Profile(p.name.clone());
    }
    if is_duplicate(now) {
        return Current::Duplicate;
    }
    Current::Unknown
}

/// Would applying `saved` leave the monitors exactly as they are `now`?
/// (Same monitors on/off, same resolution, refresh within 0.5 Hz, same
/// positions and rotation.)
pub fn matches(saved: &DisplayLayout, now: &DisplayLayout) -> bool {
    let connected: Vec<_> = now.monitors.iter().map(|m| m.id.clone()).collect();
    let Ok(p) = plan(saved, &connected) else {
        return false;
    };
    let want = expected_layout(&p);
    want.monitors.iter().all(|w| {
        let Some(n) = now.monitors.iter().find(|n| n.id.device_path == w.id.device_path) else {
            return false;
        };
        if w.enabled != n.enabled {
            return false;
        }
        if !w.enabled {
            return true;
        }
        match (w.mode, n.mode) {
            (Some(a), Some(b)) => {
                a.width == b.width
                    && a.height == b.height
                    && (a.refresh.hz() - b.refresh.hz()).abs() < 0.5
                    && w.position == n.position
                    && w.rotation == n.rotation
            }
            _ => false,
        }
    })
}

/// Two or more monitors on, all showing the same area of the desktop.
pub fn is_duplicate(now: &DisplayLayout) -> bool {
    let on: Vec<_> = now
        .enabled()
        .filter_map(|m| m.mode.map(|md| (m.position, md.width, md.height)))
        .collect();
    on.len() >= 2 && on.iter().all(|x| *x == on[0])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Position, RefreshRate};
    use crate::testing::sample_layout;

    fn profile(name: &str, layout: DisplayLayout) -> Profile {
        Profile::new(name, layout).unwrap()
    }

    fn duplicated() -> DisplayLayout {
        let mut l = sample_layout();
        let mode = l.monitors[0].mode;
        for m in &mut l.monitors {
            m.enabled = true;
            m.mode = mode;
            m.position = Position::default();
        }
        l
    }

    #[test]
    fn finds_the_matching_profile() {
        let mut left_only = sample_layout();
        left_only.monitors[1].enabled = false;
        left_only.monitors[1].mode = None;
        let profiles = [
            profile("Left only", left_only.clone()),
            profile("Both", sample_layout()),
        ];
        assert_eq!(detect(&profiles, &sample_layout()), Current::Profile("Both".into()));
        assert_eq!(detect(&profiles, &left_only), Current::Profile("Left only".into()));
        assert!(detect(&profiles, &sample_layout()).is_profile("both"));
    }

    #[test]
    fn duplicate_is_detected_when_no_profile_matches() {
        let profiles = [profile("Both", sample_layout())];
        assert_eq!(detect(&profiles, &duplicated()), Current::Duplicate);
        assert!(!is_duplicate(&sample_layout()));
        // A profile saved while duplicated wins over the generic "duplicate".
        let profiles = [profile("Mirror", duplicated())];
        assert_eq!(detect(&profiles, &duplicated()), Current::Profile("Mirror".into()));
    }

    #[test]
    fn differences_mean_no_match() {
        let profiles = [profile("Both", sample_layout())];
        let mut moved = sample_layout();
        moved.monitors[1].position.x += 10;
        assert_eq!(detect(&profiles, &moved), Current::Unknown);

        let mut other_refresh = sample_layout();
        other_refresh.monitors[1].mode.as_mut().unwrap().refresh = RefreshRate::from_hz(60);
        assert!(!matches(&sample_layout(), &other_refresh));

        let mut ntsc = sample_layout();
        ntsc.monitors[0].mode.as_mut().unwrap().refresh = RefreshRate {
            numerator: 60000,
            denominator: 1001,
        };
        assert!(matches(&sample_layout(), &ntsc), "59.94 vs 60 Hz is the same mode");
    }
}
