//! `DisplayBackend` for Windows.
//!
//! Reading: `QueryDisplayConfig(QDC_ONLY_ACTIVE_PATHS)` for monitors that are
//! on, plus `QDC_ALL_PATHS` to find monitors that are plugged in but off.
//!
//! Writing: we build a fresh set of *active* paths (one per monitor that
//! should be on) with a source mode giving the desktop size and position, and
//! let Windows pick the matching target timing (`SDC_ALLOW_CHANGES`). Paths we
//! leave out are switched off.

use std::collections::{HashMap, HashSet};

use pt_core::plan::PlannedMonitor;
use pt_core::{
    ApplyPlan, DisplayBackend, DisplayLayout, DisplayMode, Error, MonitorId, MonitorState, Position, Result, Topology,
};
use windows::Win32::Devices::Display::{
    DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_MODE_INFO_0, DISPLAYCONFIG_MODE_INFO_TYPE_SOURCE, DISPLAYCONFIG_PATH_INFO,
    DISPLAYCONFIG_PIXELFORMAT_32BPP, DISPLAYCONFIG_RATIONAL, DISPLAYCONFIG_SCALING_PREFERRED,
    DISPLAYCONFIG_SCANLINE_ORDERING_PROGRESSIVE, DISPLAYCONFIG_SCANLINE_ORDERING_UNSPECIFIED,
    DISPLAYCONFIG_SOURCE_MODE, QDC_ALL_PATHS, QDC_ONLY_ACTIVE_PATHS, SDC_ALLOW_CHANGES, SDC_APPLY,
    SDC_SAVE_TO_DATABASE, SDC_TOPOLOGY_CLONE, SDC_TOPOLOGY_EXTEND, SDC_USE_SUPPLIED_DISPLAY_CONFIG, SDC_VALIDATE,
};
use windows::Win32::Foundation::POINTL;
use windows::Win32::Graphics::Gdi::{DISPLAYCONFIG_PATH_ACTIVE, DISPLAYCONFIG_PATH_MODE_IDX_INVALID};

use super::ccd::{
    AdapterKey, DisplayConfig, debug_enabled, dump_config, enum_modes, rotation_from_win, rotation_to_win,
    set_display_config, source_gdi_name, source_key, target_key, target_name,
};

#[derive(Default)]
pub struct WindowsDisplayBackend;

impl WindowsDisplayBackend {
    pub fn new() -> Self {
        WindowsDisplayBackend
    }
}

/// Every target that has a monitor plugged in, with its stable id.
fn connected_targets(all: &DisplayConfig) -> Result<Vec<(AdapterKey, MonitorId)>> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for p in &all.paths {
        let key = target_key(p);
        if !p.targetInfo.targetAvailable.as_bool() || !seen.insert(key) {
            continue;
        }
        out.push((key, monitor_id(key)?));
    }
    Ok(out)
}

fn monitor_id(key: AdapterKey) -> Result<MonitorId> {
    let n = target_name(key)?;
    let device_path = if n.device_path.is_empty() {
        // Virtual/indirect displays may have no device path; this fallback
        // is only stable until reboot, but better than nothing.
        format!("adapter-{:08x}{:08x}-target-{}", n_high(key), key.low, key.id)
    } else {
        n.device_path
    };
    let name = if n.friendly_name.is_empty() {
        format!("Display {}", key.id)
    } else {
        n.friendly_name
    };
    Ok(MonitorId {
        device_path,
        name,
        edid_manufacturer: n.edid.map(|e| e.0),
        edid_product: n.edid.map(|e| e.1),
    })
}

fn n_high(key: AdapterKey) -> u32 {
    key.high as u32
}

impl DisplayBackend for WindowsDisplayBackend {
    fn name(&self) -> &'static str {
        "windows-ccd"
    }

    fn current_layout(&self) -> Result<DisplayLayout> {
        let active = DisplayConfig::query(QDC_ONLY_ACTIVE_PATHS)?;
        let all = DisplayConfig::query(QDC_ALL_PATHS)?;

        let mut monitors = Vec::new();
        let mut done = HashSet::new();
        for p in &active.paths {
            let key = target_key(p);
            if !done.insert(key) {
                continue;
            }
            let id = monitor_id(key)?;
            let state = match active.source_mode(p) {
                Some(sm) => MonitorState {
                    id,
                    enabled: true,
                    primary: sm.position.x == 0 && sm.position.y == 0,
                    mode: Some(DisplayMode {
                        width: sm.width,
                        height: sm.height,
                        refresh: active.refresh(p),
                    }),
                    position: Position {
                        x: sm.position.x,
                        y: sm.position.y,
                    },
                    rotation: rotation_from_win(p.targetInfo.rotation),
                },
                None => MonitorState::disabled(id),
            };
            monitors.push(state);
        }
        for (key, id) in connected_targets(&all)? {
            if done.insert(key) {
                monitors.push(MonitorState::disabled(id));
            }
        }
        Ok(DisplayLayout { monitors })
    }

    fn apply(&self, plan: &ApplyPlan, validate_only: bool) -> Result<()> {
        // First ask for the exact refresh rates. If Windows rejects the
        // configuration as invalid, retry letting Windows choose the refresh
        // rate (the service reports any difference afterwards).
        // 87 = invalid parameter, 31 = general failure, 1610 = bad
        // configuration: what drivers return for a timing they won't do
        // (e.g. 4K@120 over a cable that only carries 4K@60).
        match self.apply_with(plan, validate_only, RefreshPolicy::Exact) {
            Err(Error::Os {
                code: Some(87 | 31 | 1610),
                ..
            }) => self.apply_with(plan, validate_only, RefreshPolicy::Any),
            other => other,
        }
    }

    fn set_topology(&self, topology: Topology) -> Result<()> {
        let t = match topology {
            Topology::Clone => SDC_TOPOLOGY_CLONE,
            Topology::Extend => SDC_TOPOLOGY_EXTEND,
        };
        set_display_config(None, None, SDC_APPLY | t)
    }
    fn supported_modes(&self, monitor: &MonitorId) -> Result<Vec<DisplayMode>> {
        let active = DisplayConfig::query(QDC_ONLY_ACTIVE_PATHS)?;
        for p in &active.paths {
            if monitor_id(target_key(p))?.device_path == monitor.device_path {
                let name = source_gdi_name(source_key(p))?;
                return Ok(enum_modes(&name));
            }
        }
        Err(Error::Os {
            call: "EnumDisplaySettings",
            code: None,
            message: format!("'{}' is off; turn it on to list its modes", monitor.label()),
        })
    }
}

impl WindowsDisplayBackend {
    fn apply_with(&self, plan: &ApplyPlan, validate_only: bool, refresh: RefreshPolicy) -> Result<()> {
        let all = DisplayConfig::query(QDC_ALL_PATHS)?;
        let active = DisplayConfig::query(QDC_ONLY_ACTIVE_PATHS)?;
        if debug_enabled() {
            dump_config("current active config", &active.paths, &active.modes);
        }

        // device path -> target key, for every connected monitor.
        let targets: HashMap<String, AdapterKey> = connected_targets(&all)?
            .into_iter()
            .map(|(key, id)| (id.device_path, key))
            .collect();

        let builder = build(plan, &all, &active, &targets, refresh)?;

        // SDC_ALLOW_PATH_ORDER_CHANGES must NOT be added here: it is only valid
        // with SDC_TOPOLOGY_SUPPLIED, and combining it with
        // SDC_USE_SUPPLIED_DISPLAY_CONFIG makes Windows return error 87.
        let mut flags = SDC_USE_SUPPLIED_DISPLAY_CONFIG | SDC_ALLOW_CHANGES;
        flags = if validate_only {
            flags | SDC_VALIDATE
        } else {
            flags | SDC_APPLY | SDC_SAVE_TO_DATABASE
        };
        if debug_enabled() {
            dump_config(
                &format!("sending ({refresh:?} refresh, flags={:#x})", flags.0),
                &builder.paths,
                &builder.modes,
            );
        }
        set_display_config(Some(&builder.paths), Some(&builder.modes), flags)
    }
}

/// Work out the paths and modes to send for `plan`, given what Windows
/// reports (`all` paths, the `active` ones) and which target each connected
/// monitor is on. Pure: no OS calls, so it is unit-tested below.
fn build(
    plan: &ApplyPlan,
    all: &DisplayConfig,
    active: &DisplayConfig,
    targets: &HashMap<String, AdapterKey>,
    refresh: RefreshPolicy,
) -> Result<ConfigBuilder> {
    let mut builder = ConfigBuilder {
        refresh,
        ..Default::default()
    };
    let wanted: Vec<(&PlannedMonitor, AdapterKey)> = plan
        .enabled()
        .map(|m| {
            targets
                .get(&m.target.device_path)
                .map(|k| (m, *k))
                .ok_or_else(|| Error::MonitorNotFound(m.target.label().to_string()))
        })
        .collect::<Result<_>>()?;

    // Pass 1: monitors that are on now keep their current path (Windows'
    //         own, known-good values), adjusted to the profile.
    // Pass 2: everybody else takes any free source on their adapter (or
    //         joins a duplicate group's shared source).
    let mut pending = Vec::new();
    for &(m, target) in &wanted {
        let current = active.paths.iter().find(|p| target_key(p) == target);
        let kept = current.is_some_and(|p| builder.try_keep(active, p, m));
        if !kept {
            pending.push((m, target));
        }
    }
    for (m, target) in pending {
        if !builder.try_add(all, m, target) {
            return Err(Error::Os {
                call: "SetDisplayConfig",
                code: None,
                message: format!(
                    "no free display output on the graphics adapter for '{}' (too many monitors on one GPU?)",
                    m.target.label()
                ),
            });
        }
    }
    Ok(builder)
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum RefreshPolicy {
    /// Ask for the profile's exact refresh rate.
    #[default]
    Exact,
    /// Let Windows pick (used as a fallback).
    Any,
}

/// Accumulates the paths and source modes handed to `SetDisplayConfig`.
#[derive(Default)]
struct ConfigBuilder {
    refresh: RefreshPolicy,
    paths: Vec<DISPLAYCONFIG_PATH_INFO>,
    modes: Vec<DISPLAYCONFIG_MODE_INFO>,
    used_sources: HashSet<AdapterKey>,
}

impl ConfigBuilder {
    /// Keep a monitor that is on right now on its current path. If its
    /// resolution, refresh rate and rotation are unchanged, Windows' own
    /// target timing is reused too, so a no-change apply sends back exactly
    /// what Windows reported. Returns false if its source is already taken.
    fn try_keep(&mut self, active: &DisplayConfig, current: &DISPLAYCONFIG_PATH_INFO, m: &PlannedMonitor) -> bool {
        let mode = m.mode.expect("enabled planned monitors always have a mode");
        let src = source_key(current);
        // Part of a duplicate group whose shared desktop is already set up:
        // join it (`try_add`) instead of keeping a separate desktop at the
        // same spot, which Windows would reject as overlapping.
        if self.used_sources.contains(&src) || self.shared_source(m, target_key(current)).is_some() {
            return false;
        }
        let unchanged = active
            .source_mode(current)
            .is_some_and(|sm| sm.width == mode.width && sm.height == mode.height)
            && active.refresh(current) == mode.refresh
            && current.targetInfo.rotation == rotation_to_win(m.rotation)
            // When falling back, let Windows choose the timing instead.
            && self.refresh == RefreshPolicy::Exact;

        self.used_sources.insert(src);
        let source_idx = self.push_source_mode(src, m);
        let mut p = *current;
        p.flags = DISPLAYCONFIG_PATH_ACTIVE;
        p.sourceInfo.Anonymous.modeInfoIdx = source_idx;
        match active.target_mode(current).filter(|_| unchanged) {
            Some(target_mode) => {
                // Same timing as now: keep Windows' target mode and path values.
                p.targetInfo.Anonymous.modeInfoIdx = self.modes.len() as u32;
                self.modes.push(target_mode);
            }
            None => self.set_target(&mut p, m),
        }
        self.paths.push(p);
        true
    }

    /// Add an active path for a monitor that is currently off (or could not
    /// keep its source). Returns false when no usable path exists.
    fn try_add(&mut self, all: &DisplayConfig, m: &PlannedMonitor, target: AdapterKey) -> bool {
        let candidates = || {
            all.paths
                .iter()
                .filter(move |p| target_key(p) == target && p.targetInfo.targetAvailable.as_bool())
        };

        // Duplicating: share the desktop another monitor already shows.
        if let Some((mode_idx, src)) = self.shared_source(m, target)
            && let Some(p) = candidates().find(|p| source_key(p) == src)
        {
            self.push_new_path(*p, mode_idx, m);
            return true;
        }

        let Some(p) = candidates()
            .find(|p| !self.used_sources.contains(&source_key(p)))
            .copied()
        else {
            return false;
        };
        let src = source_key(&p);
        self.used_sources.insert(src);
        let mode_idx = self.push_source_mode(src, m);
        self.push_new_path(p, mode_idx, m);
        true
    }

    /// Duplicate (clone) group: a source already added on the same adapter
    /// with exactly this monitor's desktop rectangle. Its mode index and key.
    fn shared_source(&self, m: &PlannedMonitor, target: AdapterKey) -> Option<(u32, AdapterKey)> {
        let mode = m.mode?;
        self.modes.iter().enumerate().find_map(|(idx, mi)| {
            if mi.infoType != DISPLAYCONFIG_MODE_INFO_TYPE_SOURCE {
                return None;
            }
            // Only read the union member that `infoType` says is valid.
            let sm = unsafe { mi.Anonymous.sourceMode };
            let same_rect = sm.width == mode.width
                && sm.height == mode.height
                && sm.position.x == m.position.x
                && sm.position.y == m.position.y;
            let key = AdapterKey::new(mi.adapterId, mi.id);
            (same_rect && key.low == target.low && key.high == target.high).then_some((idx as u32, key))
        })
    }

    fn push_source_mode(&mut self, src: AdapterKey, m: &PlannedMonitor) -> u32 {
        let mode = m.mode.expect("enabled planned monitors always have a mode");
        let idx = self.modes.len() as u32;
        self.modes.push(DISPLAYCONFIG_MODE_INFO {
            infoType: DISPLAYCONFIG_MODE_INFO_TYPE_SOURCE,
            id: src.id,
            adapterId: src.luid(),
            Anonymous: DISPLAYCONFIG_MODE_INFO_0 {
                sourceMode: DISPLAYCONFIG_SOURCE_MODE {
                    width: mode.width,
                    height: mode.height,
                    pixelFormat: DISPLAYCONFIG_PIXELFORMAT_32BPP,
                    position: POINTL {
                        x: m.position.x,
                        y: m.position.y,
                    },
                },
            },
        });
        idx
    }

    fn push_new_path(&mut self, mut p: DISPLAYCONFIG_PATH_INFO, source_mode_idx: u32, m: &PlannedMonitor) {
        p.flags = DISPLAYCONFIG_PATH_ACTIVE;
        p.sourceInfo.Anonymous.modeInfoIdx = source_mode_idx;
        p.sourceInfo.statusFlags = 0;
        p.targetInfo.statusFlags = 0;
        p.targetInfo.scaling = DISPLAYCONFIG_SCALING_PREFERRED;
        self.set_target(&mut p, m);
        self.paths.push(p);
    }

    /// No explicit target timing: Windows picks one matching the refresh rate.
    fn set_target(&self, p: &mut DISPLAYCONFIG_PATH_INFO, m: &PlannedMonitor) {
        let mode = m.mode.expect("enabled planned monitors always have a mode");
        p.targetInfo.Anonymous.modeInfoIdx = DISPLAYCONFIG_PATH_MODE_IDX_INVALID;
        p.targetInfo.rotation = rotation_to_win(m.rotation);
        match self.refresh {
            RefreshPolicy::Exact => {
                p.targetInfo.refreshRate = DISPLAYCONFIG_RATIONAL {
                    Numerator: mode.refresh.numerator,
                    Denominator: mode.refresh.denominator,
                };
                // Windows only accepts UNSPECIFIED scan-line ordering when the
                // refresh rate is left unspecified too (this caused error 87).
                if p.targetInfo.scanLineOrdering == DISPLAYCONFIG_SCANLINE_ORDERING_UNSPECIFIED {
                    p.targetInfo.scanLineOrdering = DISPLAYCONFIG_SCANLINE_ORDERING_PROGRESSIVE;
                }
            }
            RefreshPolicy::Any => {
                p.targetInfo.refreshRate = DISPLAYCONFIG_RATIONAL {
                    Numerator: 0,
                    Denominator: 0,
                };
                p.targetInfo.scanLineOrdering = DISPLAYCONFIG_SCANLINE_ORDERING_UNSPECIFIED;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    //! The path-building logic against made-up `QueryDisplayConfig` results
    //! (one GPU, two outputs, two sources). Runs with `cargo test` on Windows.
    use super::*;
    use pt_core::plan::plan;
    use pt_core::testing::{id, layout, on};
    use windows::Win32::Foundation::LUID;

    const GPU: LUID = LUID {
        LowPart: 0x1234,
        HighPart: 0,
    };

    fn path(source: u32, target: u32, src_mode: u32, tgt_mode: u32) -> DISPLAYCONFIG_PATH_INFO {
        let mut p = DISPLAYCONFIG_PATH_INFO::default();
        p.sourceInfo.adapterId = GPU;
        p.sourceInfo.id = source;
        p.sourceInfo.Anonymous.modeInfoIdx = src_mode;
        p.targetInfo.adapterId = GPU;
        p.targetInfo.id = target;
        p.targetInfo.Anonymous.modeInfoIdx = tgt_mode;
        p.targetInfo.targetAvailable = true.into();
        p.targetInfo.refreshRate = DISPLAYCONFIG_RATIONAL {
            Numerator: 60,
            Denominator: 1,
        };
        p
    }

    fn source_mode(source: u32, x: i32, y: i32) -> DISPLAYCONFIG_MODE_INFO {
        DISPLAYCONFIG_MODE_INFO {
            infoType: DISPLAYCONFIG_MODE_INFO_TYPE_SOURCE,
            id: source,
            adapterId: GPU,
            Anonymous: DISPLAYCONFIG_MODE_INFO_0 {
                sourceMode: DISPLAYCONFIG_SOURCE_MODE {
                    width: 1920,
                    height: 1080,
                    pixelFormat: DISPLAYCONFIG_PIXELFORMAT_32BPP,
                    position: POINTL { x, y },
                },
            },
        }
    }

    /// Every source × target combination, as `QDC_ALL_PATHS` reports it.
    fn all_paths() -> DisplayConfig {
        let none = DISPLAYCONFIG_PATH_MODE_IDX_INVALID;
        DisplayConfig {
            paths: vec![
                path(0, 10, none, none),
                path(1, 10, none, none),
                path(0, 11, none, none),
                path(1, 11, none, none),
            ],
            modes: vec![],
        }
    }

    fn targets() -> HashMap<String, AdapterKey> {
        HashMap::from([
            ("A".to_string(), AdapterKey::new(GPU, 10)),
            ("B".to_string(), AdapterKey::new(GPU, 11)),
        ])
    }

    fn extended_now() -> DisplayConfig {
        DisplayConfig {
            paths: vec![
                path(0, 10, 0, DISPLAYCONFIG_PATH_MODE_IDX_INVALID),
                path(1, 11, 1, DISPLAYCONFIG_PATH_MODE_IDX_INVALID),
            ],
            modes: vec![source_mode(0, 0, 0), source_mode(1, 1920, 0)],
        }
    }

    fn duplicated_now() -> DisplayConfig {
        DisplayConfig {
            paths: vec![
                path(0, 10, 0, DISPLAYCONFIG_PATH_MODE_IDX_INVALID),
                path(0, 11, 0, DISPLAYCONFIG_PATH_MODE_IDX_INVALID),
            ],
            modes: vec![source_mode(0, 0, 0)],
        }
    }

    fn sources(b: &ConfigBuilder) -> Vec<u32> {
        b.paths.iter().map(|p| p.sourceInfo.id).collect()
    }

    fn source_modes(b: &ConfigBuilder) -> usize {
        b.modes
            .iter()
            .filter(|m| m.infoType == DISPLAYCONFIG_MODE_INFO_TYPE_SOURCE)
            .count()
    }

    fn extended_plan() -> ApplyPlan {
        let saved = layout(vec![on("A", 1920, 1080, 60, 0, 0), on("B", 1920, 1080, 60, 1920, 0)]);
        plan(&saved, &[id("A"), id("B")]).unwrap()
    }

    fn duplicated_plan() -> ApplyPlan {
        let saved = layout(vec![on("A", 1920, 1080, 60, 0, 0), on("B", 1920, 1080, 60, 0, 0)]);
        plan(&saved, &[id("A"), id("B")]).unwrap()
    }

    #[test]
    fn extended_to_duplicated_shares_one_source() {
        let b = build(
            &duplicated_plan(),
            &all_paths(),
            &extended_now(),
            &targets(),
            RefreshPolicy::Exact,
        )
        .unwrap();
        assert_eq!(sources(&b), [0, 0], "both outputs show source 0");
        assert_eq!(source_modes(&b), 1, "one shared desktop, not two stacked ones");
        let idx: Vec<u32> = b
            .paths
            .iter()
            .map(|p| unsafe { p.sourceInfo.Anonymous.modeInfoIdx })
            .collect();
        assert_eq!(idx[0], idx[1]);
    }

    #[test]
    fn duplicated_to_extended_uses_two_sources() {
        let b = build(
            &extended_plan(),
            &all_paths(),
            &duplicated_now(),
            &targets(),
            RefreshPolicy::Exact,
        )
        .unwrap();
        let mut s = sources(&b);
        s.sort();
        assert_eq!(s, [0, 1]);
        assert_eq!(source_modes(&b), 2);
    }

    #[test]
    fn unchanged_extended_layout_keeps_current_paths() {
        let b = build(
            &extended_plan(),
            &all_paths(),
            &extended_now(),
            &targets(),
            RefreshPolicy::Exact,
        )
        .unwrap();
        assert_eq!(sources(&b), [0, 1]);
        let pos: Vec<(i32, i32)> = b
            .modes
            .iter()
            .filter(|m| m.infoType == DISPLAYCONFIG_MODE_INFO_TYPE_SOURCE)
            .map(|m| {
                let sm = unsafe { m.Anonymous.sourceMode };
                (sm.position.x, sm.position.y)
            })
            .collect();
        assert_eq!(pos, [(0, 0), (1920, 0)]);
    }

    #[test]
    fn turning_a_monitor_on_takes_a_free_source() {
        let only_a = DisplayConfig {
            paths: vec![path(0, 10, 0, DISPLAYCONFIG_PATH_MODE_IDX_INVALID)],
            modes: vec![source_mode(0, 0, 0)],
        };
        let b = build(
            &extended_plan(),
            &all_paths(),
            &only_a,
            &targets(),
            RefreshPolicy::Exact,
        )
        .unwrap();
        assert_eq!(sources(&b), [0, 1]);
    }

    #[test]
    fn refresh_policy_any_leaves_the_rate_to_windows() {
        let b = build(
            &extended_plan(),
            &all_paths(),
            &duplicated_now(),
            &targets(),
            RefreshPolicy::Any,
        )
        .unwrap();
        assert!(b.paths.iter().all(|p| p.targetInfo.refreshRate.Numerator == 0));
    }

    #[test]
    fn unknown_monitor_is_an_error() {
        let mut t = targets();
        t.remove("B");
        let err = build(
            &extended_plan(),
            &all_paths(),
            &extended_now(),
            &t,
            RefreshPolicy::Exact,
        )
        .err()
        .unwrap();
        assert!(matches!(err, Error::MonitorNotFound(_)), "{err}");
    }
}
