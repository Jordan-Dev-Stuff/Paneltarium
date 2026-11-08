//! Turn a saved layout into concrete instructions for the monitors that are
//! connected right now. Pure logic, so it is fully unit-tested; each OS
//! backend only has to carry out an [`ApplyPlan`].

use crate::error::{Error, Result};
use crate::model::{DisplayLayout, DisplayMode, MonitorId, MonitorState, Position, Rotation};

/// One connected monitor and what it should become.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedMonitor {
    /// The id as the OS reports it *now* (may differ from the saved id if the
    /// monitor moved port).
    pub target: MonitorId,
    pub enabled: bool,
    pub primary: bool,
    pub mode: Option<DisplayMode>,
    pub position: Position,
    pub rotation: Rotation,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ApplyPlan {
    /// Every connected monitor, in the order they were reported.
    pub monitors: Vec<PlannedMonitor>,
    /// Saved monitors that the profile turns on but are not connected.
    pub missing: Vec<MonitorId>,
    /// Connected monitors the profile does not know about (they are turned off).
    pub unknown: Vec<MonitorId>,
}

impl ApplyPlan {
    pub fn enabled(&self) -> impl Iterator<Item = &PlannedMonitor> {
        self.monitors.iter().filter(|m| m.enabled)
    }
}

/// Match `saved` against the `connected` monitors and build a plan.
///
/// Matching: exact device path first, then (for monitors that moved port)
/// the same EDID model + name when that is unambiguous, then identical
/// models paired in order when the counts match.
/// Unknown connected monitors are switched off. The primary monitor is
/// moved to (0, 0) and everything else is shifted with it, because Windows
/// defines "primary" as the monitor at the desktop origin. If the saved
/// primary isn't connected, the monitor that was closest to it takes over,
/// and gaps left by missing monitors are closed (Windows needs every monitor
/// to touch another one).
pub fn plan(saved: &DisplayLayout, connected: &[MonitorId]) -> Result<ApplyPlan> {
    let mut used_saved = vec![false; saved.monitors.len()];
    let mut assignment: Vec<Option<usize>> = vec![None; connected.len()];

    // Pass 1: exact device path.
    for (ci, c) in connected.iter().enumerate() {
        if let Some(si) = saved
            .monitors
            .iter()
            .enumerate()
            .position(|(si, s)| !used_saved[si] && s.id.device_path == c.device_path)
        {
            used_saved[si] = true;
            assignment[ci] = Some(si);
        }
    }

    // Pass 2: same model, only if exactly one candidate on each side.
    for (ci, c) in connected.iter().enumerate() {
        if assignment[ci].is_some() {
            continue;
        }
        let saved_candidates: Vec<usize> = (0..saved.monitors.len())
            .filter(|&si| !used_saved[si] && saved.monitors[si].id.same_model(c))
            .collect();
        let connected_candidates = connected
            .iter()
            .enumerate()
            .filter(|(cj, other)| assignment[*cj].is_none() && other.same_model(c))
            .count();
        if let ([si], 1) = (saved_candidates.as_slice(), connected_candidates) {
            used_saved[*si] = true;
            assignment[ci] = Some(*si);
        }
    }

    // Pass 3: identical monitors that all moved (e.g. a different dock gives
    // both new device paths). Unless serial numbers differ there's no way to
    // tell them apart, so if the counts match, pair them in a fixed order —
    // saved left-to-right with connected by device path. Worst case two
    // identical screens swap places; better than leaving them dark.
    for ci in 0..connected.len() {
        if assignment[ci].is_some() {
            continue;
        }
        let c = &connected[ci];
        let mut saved_group: Vec<usize> = (0..saved.monitors.len())
            .filter(|&si| !used_saved[si] && saved.monitors[si].id.same_model(c))
            .collect();
        let mut connected_group: Vec<usize> = (0..connected.len())
            .filter(|&cj| assignment[cj].is_none() && connected[cj].same_model(c))
            .collect();
        if saved_group.len() < 2 || saved_group.len() != connected_group.len() {
            continue;
        }
        saved_group.sort_by_key(|&si| {
            let m = &saved.monitors[si];
            (!m.enabled, m.position.x, m.position.y)
        });
        connected_group.sort_by(|&a, &b| connected[a].device_path.cmp(&connected[b].device_path));
        for (si, cj) in saved_group.into_iter().zip(connected_group) {
            used_saved[si] = true;
            assignment[cj] = Some(si);
        }
    }

    let mut monitors: Vec<PlannedMonitor> = Vec::with_capacity(connected.len());
    let mut unknown = Vec::new();
    for (ci, c) in connected.iter().enumerate() {
        match assignment[ci].map(|si| &saved.monitors[si]) {
            Some(s) if s.enabled && s.mode.is_some() => monitors.push(PlannedMonitor {
                target: c.clone(),
                enabled: true,
                primary: s.primary,
                mode: s.mode,
                position: s.position,
                rotation: s.rotation,
            }),
            Some(_) => monitors.push(off(c)),
            None => {
                unknown.push(c.clone());
                monitors.push(off(c));
            }
        }
    }

    let missing = saved
        .monitors
        .iter()
        .zip(&used_saved)
        .filter(|(s, used)| !**used && s.enabled)
        .map(|(s, _)| s.id.clone())
        .collect();

    if !monitors.iter().any(|m| m.enabled) {
        return Err(Error::NoEnabledMonitors);
    }

    let saved_primary = saved
        .monitors
        .iter()
        .find(|m| m.enabled && m.primary)
        .and_then(|m| Some(Rect::new(m.position, m.mode?)));
    fix_primary_and_origin(&mut monitors, saved_primary);
    close_gaps(&mut monitors);
    Ok(ApplyPlan {
        monitors,
        missing,
        unknown,
    })
}

fn off(id: &MonitorId) -> PlannedMonitor {
    PlannedMonitor {
        target: id.clone(),
        enabled: false,
        primary: false,
        mode: None,
        position: Position::default(),
        rotation: Rotation::None,
    }
}

/// A monitor's desktop rectangle (right/bottom exclusive).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Rect {
    x0: i64,
    y0: i64,
    x1: i64,
    y1: i64,
}

impl Rect {
    fn new(p: Position, mode: DisplayMode) -> Self {
        let (x, y) = (i64::from(p.x), i64::from(p.y));
        Rect {
            x0: x,
            y0: y,
            x1: x + i64::from(mode.width),
            y1: y + i64::from(mode.height),
        }
    }

    fn of(m: &PlannedMonitor) -> Option<Rect> {
        Some(Rect::new(m.position, m.mode?))
    }

    /// How far `self` would have to move to touch `other`, per axis:
    /// positive = `self` is right of/below `other`, 0 = overlapping or
    /// touching on that axis.
    fn gap_to(&self, other: &Rect) -> (i64, i64) {
        let axis = |a0: i64, a1: i64, b0: i64, b1: i64| {
            if a0 >= b1 {
                a0 - b1
            } else if a1 <= b0 {
                a1 - b0
            } else {
                0
            }
        };
        (
            axis(self.x0, self.x1, other.x0, other.x1),
            axis(self.y0, self.y1, other.y0, other.y1),
        )
    }

    fn distance(&self, other: &Rect) -> i64 {
        let (gx, gy) = self.gap_to(other);
        gx.abs() + gy.abs()
    }
}

/// Exactly one enabled primary, located at (0, 0).
fn fix_primary_and_origin(monitors: &mut [PlannedMonitor], saved_primary: Option<Rect>) {
    let enabled: Vec<usize> = (0..monitors.len()).filter(|&i| monitors[i].enabled).collect();
    // Keep the first saved primary that is still enabled. Otherwise promote
    // the monitor that was nearest to the old primary (ties: top-left), so
    // e.g. losing the middle of three monitors doesn't jump to a far edge.
    let primary = enabled
        .iter()
        .copied()
        .find(|&i| monitors[i].primary)
        .unwrap_or_else(|| {
            *enabled
                .iter()
                .min_by_key(|&&i| {
                    let m = &monitors[i];
                    let near = match (saved_primary, Rect::of(m)) {
                        (Some(p), Some(r)) => r.distance(&p),
                        _ => 0,
                    };
                    (near, m.position.y, m.position.x)
                })
                .expect("caller checked at least one enabled monitor")
        });

    let origin = monitors[primary].position;
    for (i, m) in monitors.iter_mut().enumerate() {
        m.primary = i == primary;
        if m.enabled {
            m.position.x -= origin.x;
            m.position.y -= origin.y;
        }
    }
}

/// Make every enabled monitor touch the group around the primary. A saved
/// layout is always contiguous, so this only moves anything when monitors
/// in between are missing: e.g. left–middle–right with the middle one
/// unplugged slides the right one over to touch the left one. Monitors
/// beyond a closed gap move with it, keeping their arrangement.
fn close_gaps(monitors: &mut [PlannedMonitor]) {
    let Some(primary) = monitors.iter().position(|m| m.enabled && m.primary) else {
        return;
    };
    let mut placed = vec![primary];
    let mut rest: Vec<usize> = (0..monitors.len())
        .filter(|&i| i != primary && monitors[i].enabled && monitors[i].mode.is_some())
        .collect();
    while !rest.is_empty() {
        // The unplaced monitor nearest to the placed group, and its gap.
        let (k, (gx, gy)) = rest
            .iter()
            .enumerate()
            .map(|(k, &i)| {
                let r = Rect::of(&monitors[i]).expect("filtered to monitors with a mode");
                let gap = placed
                    .iter()
                    .map(|&j| r.gap_to(&Rect::of(&monitors[j]).expect("placed monitors have a mode")))
                    .min_by_key(|(gx, gy)| gx.abs() + gy.abs())
                    .expect("the primary is always placed");
                (k, gap)
            })
            .min_by_key(|(_, (gx, gy))| gx.abs() + gy.abs())
            .expect("rest is not empty");
        let i = rest.remove(k);
        if (gx, gy) != (0, 0) {
            let r = Rect::of(&monitors[i]).expect("has a mode");
            // Move it, and every other unplaced monitor on the far side of
            // the gap, by the size of the gap.
            let beyond = |o: &Rect| {
                (gx == 0 || (gx > 0 && o.x0 >= r.x0) || (gx < 0 && o.x1 <= r.x1))
                    && (gy == 0 || (gy > 0 && o.y0 >= r.y0) || (gy < 0 && o.y1 <= r.y1))
            };
            let movers: Vec<usize> = std::iter::once(i)
                .chain(
                    rest.iter()
                        .copied()
                        .filter(|&o| beyond(&Rect::of(&monitors[o]).expect("has a mode"))),
                )
                .collect();
            for o in movers {
                monitors[o].position.x -= gx as i32;
                monitors[o].position.y -= gy as i32;
            }
        }
        placed.push(i);
    }
}

/// Check a plan the way Windows will: exactly one primary, at (0, 0); no
/// two monitors partly overlapping (identical rectangles are fine — that's
/// duplicating); and every monitor touching the others (no islands).
/// The fake backend enforces this, so tests catch layouts Windows would
/// reject.
pub fn check_arrangement(plan: &ApplyPlan) -> std::result::Result<(), String> {
    let on: Vec<(&PlannedMonitor, Rect)> = plan
        .enabled()
        .map(|m| {
            Ok((
                m,
                Rect::of(m).ok_or_else(|| format!("'{}' is on but has no mode", m.target.label()))?,
            ))
        })
        .collect::<std::result::Result<_, String>>()?;
    let primaries: Vec<_> = on.iter().filter(|(m, _)| m.primary).collect();
    match primaries.as_slice() {
        [(m, _)] if m.position == Position::default() => {}
        [(m, _)] => return Err(format!("primary '{}' is not at (0, 0)", m.target.label())),
        _ => return Err(format!("{} primary monitors", primaries.len())),
    }
    for (i, (a, ra)) in on.iter().enumerate() {
        for (b, rb) in &on[i + 1..] {
            let overlap = ra.x0 < rb.x1 && rb.x0 < ra.x1 && ra.y0 < rb.y1 && rb.y0 < ra.y1;
            if overlap && ra != rb {
                return Err(format!("'{}' and '{}' overlap", a.target.label(), b.target.label()));
            }
        }
    }
    // Flood-fill from the primary over touching rectangles.
    let mut reached = vec![false; on.len()];
    let start = on.iter().position(|(m, _)| m.primary).expect("checked above");
    reached[start] = true;
    let mut stack = vec![start];
    while let Some(i) = stack.pop() {
        for j in 0..on.len() {
            if !reached[j] && on[i].1.gap_to(&on[j].1) == (0, 0) {
                reached[j] = true;
                stack.push(j);
            }
        }
    }
    match on.iter().zip(&reached).find(|(_, r)| !**r) {
        Some(((m, _), _)) => Err(format!("'{}' doesn't touch the other monitors", m.target.label())),
        None => Ok(()),
    }
}

/// Convenience: the state each monitor will be in after the plan is applied.
pub fn expected_layout(plan: &ApplyPlan) -> DisplayLayout {
    DisplayLayout {
        monitors: plan
            .monitors
            .iter()
            .map(|p| MonitorState {
                id: p.target.clone(),
                enabled: p.enabled,
                primary: p.primary,
                mode: p.mode,
                position: p.position,
                rotation: p.rotation,
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{id, sample_layout};

    fn connected(ids: &[&str]) -> Vec<MonitorId> {
        ids.iter().map(|s| id(s)).collect()
    }

    #[test]
    fn exact_match_keeps_settings() {
        let saved = sample_layout(); // LEFT on primary at 0,0; RIGHT on at 3840,0; TV off
        let p = plan(&saved, &connected(&["LEFT", "RIGHT", "TV"])).unwrap();
        assert!(p.missing.is_empty() && p.unknown.is_empty());
        let on: Vec<_> = p.enabled().map(|m| m.target.device_path.as_str()).collect();
        assert_eq!(on, ["LEFT", "RIGHT"]);
        assert!(p.monitors[0].primary);
        assert_eq!(p.monitors[1].position, Position { x: 3840, y: 0 });
        assert!(!p.monitors[2].enabled);
    }

    #[test]
    fn unknown_monitors_are_turned_off_and_reported() {
        let p = plan(&sample_layout(), &connected(&["LEFT", "NEW"])).unwrap();
        assert_eq!(p.unknown, [id("NEW")]);
        assert_eq!(p.missing, [id("RIGHT")]);
        assert!(!p.monitors[1].enabled);
    }

    #[test]
    fn moved_port_matches_by_model_when_unambiguous() {
        let saved = sample_layout();
        let mut moved = id("RIGHT");
        moved.device_path = "RIGHT-ON-NEW-PORT".into();
        let p = plan(&saved, &[id("LEFT"), moved.clone()]).unwrap();
        assert!(p.unknown.is_empty(), "{:?}", p.unknown);
        assert!(p.monitors[1].enabled);
        assert_eq!(p.monitors[1].target, moved);
    }

    #[test]
    fn ambiguous_model_is_not_guessed() {
        let saved = sample_layout();
        let mut a = id("RIGHT");
        a.device_path = "X1".into();
        let mut b = id("RIGHT");
        b.device_path = "X2".into();
        let p = plan(&saved, &[id("LEFT"), a, b]).unwrap();
        assert_eq!(p.unknown.len(), 2);
    }

    #[test]
    fn primary_missing_promotes_another_and_rebases_origin() {
        // Only RIGHT (at x=3840) is connected; it must become primary at 0,0.
        let p = plan(&sample_layout(), &connected(&["RIGHT"])).unwrap();
        assert!(p.monitors[0].primary);
        assert_eq!(p.monitors[0].position, Position::default());
        assert_eq!(p.missing, [id("LEFT")]);
    }

    #[test]
    fn primary_not_at_origin_is_rebased() {
        let mut saved = sample_layout();
        saved.monitors[0].primary = false;
        saved.monitors[1].primary = true; // RIGHT at 3840,0
        let p = plan(&saved, &connected(&["LEFT", "RIGHT"])).unwrap();
        assert_eq!(p.monitors[1].position, Position::default());
        assert_eq!(p.monitors[0].position, Position { x: -3840, y: 0 });
        assert_eq!(p.monitors.iter().filter(|m| m.primary).count(), 1);
    }

    #[test]
    fn nothing_to_enable_is_an_error() {
        assert!(matches!(
            plan(&sample_layout(), &connected(&["TV"])),
            Err(Error::NoEnabledMonitors)
        ));
        assert!(matches!(plan(&sample_layout(), &[]), Err(Error::NoEnabledMonitors)));
    }

    #[test]
    fn expected_layout_mirrors_plan() {
        let p = plan(&sample_layout(), &connected(&["LEFT", "RIGHT", "TV"])).unwrap();
        let l = expected_layout(&p);
        assert_eq!(l.enabled().count(), 2);
        assert_eq!(l.monitors[0].id, id("LEFT"));
    }
}
