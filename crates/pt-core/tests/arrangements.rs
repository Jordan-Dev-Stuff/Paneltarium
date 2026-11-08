//! Edge cases involving different kinds and arrangements of monitors:
//! portrait/rotated screens, monitors left of or above the primary, mixed
//! resolutions and refresh rates, identical models, missing monitors (a
//! closed laptop lid, an unplugged middle screen), duplicated setups, and
//! large grids. Every plan is also checked against the rules Windows
//! enforces (`check_arrangement`), and the service-level tests go through the
//! fake backend, which rejects anything Windows would.

use pt_core::current::{self, Current};
use pt_core::mock::MockDisplayBackend;
use pt_core::plan::{ApplyPlan, check_arrangement, expected_layout, plan};
use pt_core::testing::{id, layout, on, twin};
use pt_core::{
    AppPaths, DisplayLayout, DisplayMode, Error, MonitorId, MonitorService, MonitorState, Position, Profile,
    RefreshRate, Rotation,
};

fn ids(l: &DisplayLayout) -> Vec<MonitorId> {
    l.monitors.iter().map(|m| m.id.clone()).collect()
}

fn connected(tags: &[&str]) -> Vec<MonitorId> {
    tags.iter().map(|t| id(t)).collect()
}

fn get<'a>(p: &'a ApplyPlan, tag: &str) -> &'a pt_core::plan::PlannedMonitor {
    p.monitors
        .iter()
        .find(|m| m.target.device_path == tag)
        .unwrap_or_else(|| panic!("{tag} not in plan"))
}

fn pos(x: i32, y: i32) -> Position {
    Position { x, y }
}

/// Plan, and insist the result is something Windows would accept.
fn valid_plan(saved: &DisplayLayout, now: &[MonitorId]) -> ApplyPlan {
    let p = plan(saved, now).expect("plan");
    check_arrangement(&p).unwrap_or_else(|e| panic!("invalid arrangement: {e}\n{p:#?}"));
    p
}

fn service(now: DisplayLayout) -> (tempfile::TempDir, MonitorService) {
    let dir = tempfile::tempdir().unwrap();
    let svc = MonitorService::new(Box::new(MockDisplayBackend::new(now)), AppPaths::new(dir.path()));
    (dir, svc)
}

// ---------------------------------------------------------------- shapes

#[test]
fn portrait_monitor_keeps_rotation_and_tall_size() {
    // 1440p landscape primary, plus a 1440p panel rotated to portrait on the
    // right, raised so it's centred vertically.
    let mut portrait = on("SIDE", 1440, 2560, 60, 2560, -560);
    portrait.rotation = Rotation::Rotate90;
    let saved = layout(vec![on("MAIN", 2560, 1440, 165, 0, 0), portrait]);
    let p = valid_plan(&saved, &ids(&saved));
    let side = get(&p, "SIDE");
    assert_eq!(side.rotation, Rotation::Rotate90);
    assert_eq!(side.mode.map(|m| (m.width, m.height)), Some((1440, 2560)));
    assert_eq!(side.position, pos(2560, -560));
    assert!(current::matches(&saved, &expected_layout(&p)));
}

#[test]
fn upside_down_and_270_rotations_round_trip() {
    let mut a = on("A", 1920, 1080, 60, 0, 0);
    a.rotation = Rotation::Rotate180;
    let mut b = on("B", 1080, 1920, 60, 1920, 0);
    b.rotation = Rotation::Rotate270;
    let saved = layout(vec![a, b]);
    let p = valid_plan(&saved, &ids(&saved));
    assert_eq!(get(&p, "A").rotation, Rotation::Rotate180);
    assert_eq!(get(&p, "B").rotation, Rotation::Rotate270);

    // Rotation alone is a difference: not "already on screen".
    let mut now = saved.clone();
    now.monitors[1].rotation = Rotation::Rotate90;
    assert!(!current::matches(&saved, &now));
}

#[test]
fn monitor_left_of_primary_keeps_negative_coordinates() {
    let saved = layout(vec![
        on("LEFT", 1920, 1080, 60, -1920, 0),
        on("MAIN", 2560, 1440, 144, 0, 0),
    ]);
    let p = valid_plan(&saved, &ids(&saved));
    assert_eq!(get(&p, "LEFT").position, pos(-1920, 0));
    assert!(get(&p, "MAIN").primary);
}

#[test]
fn monitor_above_primary_keeps_negative_y() {
    // A TV mounted above the desk monitor.
    let saved = layout(vec![
        on("DESK", 2560, 1440, 144, 0, 0),
        on("TV", 3840, 2160, 60, -640, -2160),
    ]);
    let p = valid_plan(&saved, &ids(&saved));
    assert_eq!(get(&p, "TV").position, pos(-640, -2160));
}

#[test]
fn bottom_aligned_mixed_heights_stay_as_saved() {
    // 1080p next to a 1440p, bottoms lined up (the 1080p is 360 px lower).
    let saved = layout(vec![
        on("BIG", 2560, 1440, 144, 0, 0),
        on("SMALL", 1920, 1080, 60, 2560, 360),
    ]);
    let p = valid_plan(&saved, &ids(&saved));
    assert_eq!(get(&p, "SMALL").position, pos(2560, 360));
}

#[test]
fn ultrawide_and_super_ultrawide_sizes_are_kept() {
    let saved = layout(vec![
        on("UW", 3440, 1440, 175, 0, 0),
        on("SUW", 5120, 1440, 240, 3440, 0),
    ]);
    let p = valid_plan(&saved, &ids(&saved));
    assert_eq!(get(&p, "SUW").mode.map(|m| m.width), Some(5120));
}

#[test]
fn fractional_refresh_is_kept_exactly() {
    let mut tv = on("TV", 3840, 2160, 60, 0, 0);
    let ntsc = RefreshRate {
        numerator: 60000,
        denominator: 1001,
    };
    tv.mode.as_mut().unwrap().refresh = ntsc;
    let saved = layout(vec![tv]);
    let p = valid_plan(&saved, &ids(&saved));
    assert_eq!(
        get(&p, "TV").mode.unwrap().refresh,
        ntsc,
        "59.94 Hz must not be rounded"
    );
}

#[test]
fn high_and_odd_refresh_rates_survive_a_save_and_load() {
    let mut a = on("A", 2560, 1440, 360, 0, 0);
    a.mode.as_mut().unwrap().refresh = RefreshRate {
        numerator: 143_856,
        denominator: 1000,
    };
    let saved = layout(vec![a, on("B", 1920, 1080, 500, 2560, 0)]);
    let profile = Profile::new("Odd", saved.clone()).unwrap();
    let text = toml::to_string(&profile).unwrap();
    let back: Profile = toml::from_str(&text).unwrap();
    assert_eq!(back.layout, saved);
}

// ---------------------------------------------------------- missing monitors

#[test]
fn closed_laptop_lid_promotes_the_external_monitor() {
    // Profile: laptop panel (primary) + external 4K on the right. The lid is
    // closed, so only the external monitor is connected.
    let saved = layout(vec![
        on("PANEL", 1920, 1200, 60, 0, 0),
        on("EXT", 3840, 2160, 60, 1920, -480),
    ]);
    let p = valid_plan(&saved, &connected(&["EXT"]));
    let ext = get(&p, "EXT");
    assert!(ext.primary);
    assert_eq!(ext.position, pos(0, 0));
    assert_eq!(p.missing, [id("PANEL")]);
}

#[test]
fn missing_middle_monitor_closes_the_gap() {
    let saved = layout(vec![
        on("L", 1920, 1080, 60, 0, 0),
        on("M", 1920, 1080, 60, 1920, 0),
        on("R", 1920, 1080, 60, 3840, 0),
    ]);
    let p = valid_plan(&saved, &connected(&["L", "R"]));
    assert_eq!(get(&p, "L").position, pos(0, 0));
    assert_eq!(get(&p, "R").position, pos(1920, 0), "slid left to touch L");
}

#[test]
fn missing_middle_primary_promotes_a_neighbour_not_a_far_edge() {
    // Left, middle (primary), right; the right one sits higher and is taller.
    let saved = layout(vec![
        on("L", 1920, 1080, 60, -1920, 0),
        on("M", 2560, 1440, 144, 0, 0),
        on("R", 1440, 2560, 60, 2560, -560),
    ]);
    let p = valid_plan(&saved, &connected(&["L", "R"]));
    // Both were touching M; the tie goes to the top-most (R).
    assert!(get(&p, "R").primary);
    assert_eq!(get(&p, "R").position, pos(0, 0));
    // L keeps its height offset and now touches R's left edge.
    let l = get(&p, "L");
    assert_eq!(l.position.x + 1920, 0, "L's right edge meets R");
    assert_eq!(l.position.y, 560);
}

#[test]
fn far_monitors_follow_when_a_gap_closes() {
    // Four in a row; the second is missing, so the 3rd and 4th both move.
    let saved = layout(vec![
        on("A", 1920, 1080, 60, 0, 0),
        on("B", 1920, 1080, 60, 1920, 0),
        on("C", 2560, 1440, 60, 3840, 0),
        on("D", 1920, 1080, 60, 6400, 0),
    ]);
    let p = valid_plan(&saved, &connected(&["A", "C", "D"]));
    assert_eq!(get(&p, "C").position, pos(1920, 0));
    assert_eq!(get(&p, "D").position, pos(4480, 0), "keeps touching C");
}

#[test]
fn gap_on_the_left_of_the_primary_closes_too() {
    let saved = layout(vec![
        on("FARLEFT", 1920, 1080, 60, -3840, 0),
        on("LEFT", 1920, 1080, 60, -1920, 0),
        on("MAIN", 1920, 1080, 60, 0, 0),
    ]);
    let p = valid_plan(&saved, &connected(&["FARLEFT", "MAIN"]));
    assert_eq!(get(&p, "FARLEFT").position, pos(-1920, 0));
}

#[test]
fn vertical_gap_closes() {
    // Stacked: desk monitor, TV above it, projector above the TV.
    let saved = layout(vec![
        on("DESK", 1920, 1080, 60, 0, 0),
        on("TV", 1920, 1080, 60, 0, -1080),
        on("PROJ", 1920, 1080, 60, 0, -2160),
    ]);
    let p = valid_plan(&saved, &connected(&["DESK", "PROJ"]));
    assert_eq!(get(&p, "PROJ").position, pos(0, -1080));
}

#[test]
fn grid_with_a_missing_monitor_is_left_alone_when_still_connected() {
    // 3×2 wall; the top middle one is missing, but everything still touches
    // through the bottom row, so nothing should move.
    let saved = layout(vec![
        on("A", 1920, 1080, 60, 0, 0),
        on("B", 1920, 1080, 60, 1920, 0),
        on("C", 1920, 1080, 60, 3840, 0),
        on("D", 1920, 1080, 60, 0, 1080),
        on("E", 1920, 1080, 60, 1920, 1080),
        on("F", 1920, 1080, 60, 3840, 1080),
    ]);
    let p = valid_plan(&saved, &connected(&["A", "C", "D", "E", "F"]));
    assert_eq!(get(&p, "C").position, pos(3840, 0));
    assert_eq!(get(&p, "F").position, pos(3840, 1080));
}

#[test]
fn only_unknown_monitors_connected_is_an_error_not_a_blackout() {
    let saved = layout(vec![on("HOME", 1920, 1080, 60, 0, 0)]);
    assert!(matches!(
        plan(&saved, &connected(&["HOTEL-TV"])),
        Err(Error::NoEnabledMonitors)
    ));
}

#[test]
fn unknown_extra_monitor_is_turned_off() {
    let saved = layout(vec![on("A", 1920, 1080, 60, 0, 0)]);
    let p = valid_plan(&saved, &connected(&["A", "PROJECTOR"]));
    assert!(!get(&p, "PROJECTOR").enabled);
    assert_eq!(p.unknown, [id("PROJECTOR")]);
}

// ------------------------------------------------------- identical monitors

#[test]
fn identical_monitors_on_their_own_ports_keep_their_own_settings() {
    let mut l = on("X", 2560, 1440, 144, 0, 0);
    l.id = twin("DELL", "PORT1");
    let mut r = on("X", 2560, 1440, 60, 2560, 0);
    r.id = twin("DELL", "PORT2");
    let saved = layout(vec![l, r]);
    let p = valid_plan(&saved, &[twin("DELL", "PORT2"), twin("DELL", "PORT1")]);
    assert_eq!(get(&p, "PORT1").mode.unwrap().refresh.hz(), 144.0);
    assert_eq!(get(&p, "PORT2").position, pos(2560, 0));
}

#[test]
fn one_identical_monitor_moved_port_is_still_matched() {
    let mut l = on("X", 2560, 1440, 144, 0, 0);
    l.id = twin("DELL", "PORT1");
    let mut r = on("X", 2560, 1440, 60, 2560, 0);
    r.id = twin("DELL", "PORT2");
    let saved = layout(vec![l, r]);
    let p = valid_plan(&saved, &[twin("DELL", "PORT1"), twin("DELL", "PORT3")]);
    assert!(p.unknown.is_empty());
    assert_eq!(get(&p, "PORT3").position, pos(2560, 0));
}

#[test]
fn identical_monitors_all_on_new_ports_are_paired_not_left_dark() {
    // Plugged into a different dock: both get new device paths.
    let mut l = on("X", 2560, 1440, 60, 0, 0);
    l.id = twin("DELL", "DOCK1-A");
    let mut r = on("X", 2560, 1440, 60, 2560, 0);
    r.id = twin("DELL", "DOCK1-B");
    let saved = layout(vec![l, r]);
    let p = valid_plan(&saved, &[twin("DELL", "DOCK2-B"), twin("DELL", "DOCK2-A")]);
    assert!(p.unknown.is_empty() && p.missing.is_empty(), "{p:?}");
    assert_eq!(p.enabled().count(), 2);
}

#[test]
fn identical_monitors_with_unequal_counts_are_not_guessed() {
    let mut a = on("X", 1920, 1080, 60, 0, 0);
    a.id = twin("HP", "P1");
    let mut laptop = on("PANEL", 1920, 1080, 60, -1920, 0);
    laptop.primary = false;
    let saved = layout(vec![a, laptop]);
    // Two HP monitors on unknown ports, one saved: can't tell which one.
    let p = valid_plan(&saved, &[id("PANEL"), twin("HP", "Q1"), twin("HP", "Q2")]);
    assert_eq!(p.unknown.len(), 2);
    assert!(get(&p, "PANEL").primary);
}

#[test]
fn monitors_without_edid_are_never_matched_by_model() {
    let mut generic = id("GENERIC");
    generic.edid_manufacturer = None;
    generic.edid_product = None;
    let saved = layout(vec![MonitorState {
        id: generic.clone(),
        ..on("GENERIC", 1024, 768, 60, 0, 0)
    }]);
    let moved = MonitorId {
        device_path: "ELSEWHERE".into(),
        ..generic
    };
    assert!(matches!(plan(&saved, &[moved]), Err(Error::NoEnabledMonitors)));
}

// -------------------------------------------------------------- duplicating

fn mirrored() -> DisplayLayout {
    // A profile saved while duplicated: both at (0, 0), same desktop size.
    // Windows reports both as being at the origin.
    let mut tv = on("TV", 1920, 1080, 60, 0, 0);
    tv.primary = true;
    layout(vec![on("DESK", 1920, 1080, 144, 0, 0), tv])
}

#[test]
fn duplicated_profile_plans_one_shared_desktop() {
    let saved = mirrored();
    let p = valid_plan(&saved, &ids(&saved));
    assert_eq!(p.enabled().count(), 2);
    assert_eq!(p.monitors.iter().filter(|m| m.primary).count(), 1);
    assert!(p.enabled().all(|m| m.position == pos(0, 0)), "not pulled apart");
    assert!(current::is_duplicate(&expected_layout(&p)));
}

#[test]
fn duplicated_profile_is_recognised_as_current() {
    let saved = mirrored();
    let profiles = [Profile::new("Mirror", saved.clone()).unwrap()];
    assert_eq!(current::detect(&profiles, &saved), Current::Profile("Mirror".into()));
}

#[test]
fn partial_overlap_is_rejected_by_the_checker() {
    let saved = layout(vec![on("A", 1920, 1080, 60, 0, 0), on("B", 1920, 1080, 60, 1000, 0)]);
    let p = plan(&saved, &ids(&saved)).unwrap();
    assert!(check_arrangement(&p).unwrap_err().contains("overlap"));
}

// ------------------------------------------------------ odd or broken files

#[test]
fn two_saved_primaries_still_give_exactly_one() {
    let mut b = on("B", 1920, 1080, 60, 1920, 0);
    b.primary = true; // hand-edited file
    let saved = layout(vec![on("A", 1920, 1080, 60, 0, 0), b]);
    let p = valid_plan(&saved, &ids(&saved));
    assert!(get(&p, "A").primary && !get(&p, "B").primary);
}

#[test]
fn enabled_monitor_without_a_mode_is_treated_as_off() {
    let mut broken = on("B", 1920, 1080, 60, 1920, 0);
    broken.mode = None;
    let saved = layout(vec![on("A", 1920, 1080, 60, 0, 0), broken]);
    let p = valid_plan(&saved, &ids(&saved));
    assert!(!get(&p, "B").enabled);
}

#[test]
fn saved_primary_not_at_origin_is_rebased() {
    // Profiles written by hand (or another tool) may not have the primary
    // at (0, 0); everything shifts with it.
    let mut a = on("A", 1920, 1080, 60, 500, 500);
    a.primary = true;
    let saved = layout(vec![a, on("B", 1920, 1080, 60, 2420, 500)]);
    let p = valid_plan(&saved, &ids(&saved));
    assert_eq!(get(&p, "A").position, pos(0, 0));
    assert_eq!(get(&p, "B").position, pos(1920, 0));
}

#[test]
fn six_monitor_wall_round_trips() {
    let mut monitors = Vec::new();
    for row in 0..2 {
        for col in 0..3 {
            monitors.push(on(&format!("W{row}{col}"), 3840, 2160, 60, col * 3840, row * 2160));
        }
    }
    let saved = layout(monitors);
    let p = valid_plan(&saved, &ids(&saved));
    assert_eq!(p.enabled().count(), 6);
    assert!(current::matches(&saved, &expected_layout(&p)));
}

// --------------------------------------------- through the service + backend

#[test]
fn applying_with_a_monitor_unplugged_gives_a_valid_desktop_and_is_current() {
    let three = layout(vec![
        on("L", 1920, 1080, 60, 0, 0),
        on("M", 2560, 1440, 144, 1920, -180),
        on("R", 1920, 1080, 60, 4480, 0),
    ]);
    let (_d, svc) = service(three.clone());
    svc.save_current("Three", None, false).unwrap();

    // Unplug the middle monitor, then apply the profile.
    let mut two = three.clone();
    two.monitors.remove(1);
    let (_d2, svc2) = service(two);
    svc2.store()
        .save(&svc.store().get("Three").unwrap(), false, &[])
        .unwrap();
    let report = svc2.apply_profile("Three", false).unwrap();
    assert_eq!(report.plan.missing, [id("M")]);
    let now = svc2.current_layout().unwrap();
    let r = now.monitors.iter().find(|m| m.id.device_path == "R").unwrap();
    assert_eq!(r.position, pos(1920, 0));
    // The overlay/tray will show it as the current profile.
    let profiles = svc2.store().list().unwrap();
    assert_eq!(current::detect(&profiles, &now), Current::Profile("Three".into()));
}

#[test]
fn switching_between_duplicate_and_extended_profiles_both_ways() {
    let extended = layout(vec![
        on("DESK", 1920, 1080, 144, 0, 0),
        on("TV", 1920, 1080, 60, 1920, 0),
    ]);
    let (_d, svc) = service(extended.clone());
    svc.save_current("Extended", None, false).unwrap();
    svc.store()
        .save(&Profile::new("Mirror", mirrored()).unwrap(), false, &[])
        .unwrap();

    svc.apply_profile("Mirror", false).unwrap();
    assert!(current::is_duplicate(&svc.current_layout().unwrap()));
    svc.apply_profile("Extended", false).unwrap();
    let now = svc.current_layout().unwrap();
    assert!(!current::is_duplicate(&now));
    let profiles = svc.store().list().unwrap();
    assert_eq!(current::detect(&profiles, &now), Current::Profile("Extended".into()));
}

#[test]
fn every_rotation_and_mode_is_restored_after_rescue() {
    let mut side = on("SIDE", 1080, 1920, 60, 2560, -240);
    side.rotation = Rotation::Rotate90;
    let saved = layout(vec![on("MAIN", 2560, 1440, 165, 0, 0), side]);
    let (_d, svc) = service(saved.clone());
    svc.save_current("Desk", None, false).unwrap();
    svc.rescue().unwrap();
    svc.apply_profile("Desk", false).unwrap();
    let mut want = saved;
    want.sort_for_display();
    assert_eq!(svc.current_layout().unwrap(), want);
}

#[test]
fn display_mode_text_is_readable() {
    let m = DisplayMode {
        width: 3840,
        height: 2160,
        refresh: RefreshRate {
            numerator: 60000,
            denominator: 1001,
        },
    };
    assert_eq!(m.to_string(), "3840x2160 @ 59.94 Hz");
}
