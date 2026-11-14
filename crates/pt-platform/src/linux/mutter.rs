//! GNOME: Mutter's `org.gnome.Mutter.DisplayConfig` D-Bus interface, the
//! same one GNOME Settings → Displays uses. Works on GNOME's Wayland and X11
//! sessions (and desktops built on Mutter).
//!
//! Changes are applied as *temporary* configurations: a *persistent* one
//! makes GNOME Shell ask "Keep these display settings?" and undo the change
//! after 20 seconds unless you click — no good for a hotkey or a controller.
//! The catch is that GNOME goes back to its own saved arrangement when a
//! monitor is plugged in or out; Paneltarium's sign-in profile covers the
//! next sign-in.

use std::collections::HashMap;

use pt_core::{Error, Result, Rotation};
use zbus::zvariant::{OwnedValue, Value};

use super::edid::{self, Edid};
use super::outputs::{Active, Coords, Driver, Output, OutputMode, Setting, Snapshot, monitor_id};

const SERVICE: &str = "org.gnome.Mutter.DisplayConfig";

/// `ApplyMonitorsConfig` methods.
const METHOD_VERIFY: u32 = 0;
const METHOD_TEMPORARY: u32 = 1;

/// The `layout-mode` property: 1 = logical (scaled) coordinates, 2 = physical.
const LAYOUT_PHYSICAL: u32 = 2;

type Props = HashMap<String, OwnedValue>;
type Spec = (String, String, String, String);
type ModeT = (String, i32, i32, f64, f64, Vec<f64>, Props);
type MonitorT = (Spec, Vec<ModeT>, Props);
type LogicalT = (i32, i32, f64, u32, bool, Vec<Spec>, Props);
type StateT = (u32, Vec<MonitorT>, Vec<LogicalT>, Props);
type ConfigMonitorT<'a> = (String, String, HashMap<String, Value<'a>>);
type ConfigLogicalT<'a> = (i32, i32, f64, u32, bool, Vec<ConfigMonitorT<'a>>);

#[zbus::proxy(
    interface = "org.gnome.Mutter.DisplayConfig",
    default_service = "org.gnome.Mutter.DisplayConfig",
    default_path = "/org/gnome/Mutter/DisplayConfig",
    gen_async = false,
    blocking_name = "DisplayConfig"
)]
trait DisplayConfigApi {
    fn get_current_state(&self) -> zbus::Result<StateT>;

    fn apply_monitors_config(
        &self,
        serial: u32,
        method: u32,
        logical_monitors: Vec<ConfigLogicalT<'_>>,
        properties: HashMap<String, Value<'_>>,
    ) -> zbus::Result<()>;
}

pub struct MutterDriver {
    proxy: DisplayConfig<'static>,
}

impl MutterDriver {
    /// Connects to the session bus; fails when Mutter isn't running.
    pub fn connect() -> Result<Self> {
        let conn = zbus::blocking::Connection::session().map_err(|e| dbus_error("session bus", e))?;
        let dbus = zbus::blocking::fdo::DBusProxy::new(&conn).map_err(|e| dbus_error("session bus", e))?;
        let name = SERVICE.try_into().expect("valid bus name");
        if !dbus.name_has_owner(name).unwrap_or(false) {
            return Err(Error::Os {
                call: "org.gnome.Mutter.DisplayConfig",
                code: None,
                message: "GNOME's display service isn't running".into(),
            });
        }
        let proxy = DisplayConfig::new(&conn).map_err(|e| dbus_error("DisplayConfig", e))?;
        Ok(MutterDriver { proxy })
    }
}

fn dbus_error(call: &'static str, e: impl std::fmt::Display) -> Error {
    Error::Os {
        call,
        code: None,
        message: e.to_string(),
    }
}

impl Driver for MutterDriver {
    fn name(&self) -> &'static str {
        "gnome-mutter"
    }

    fn snapshot(&self) -> Result<Snapshot> {
        let state = self
            .proxy
            .get_current_state()
            .map_err(|e| dbus_error("GetCurrentState", e))?;
        Ok(snapshot(state, &edid::for_connector))
    }

    fn apply(&self, snapshot: &Snapshot, settings: &[Setting], validate_only: bool) -> Result<()> {
        let method = if validate_only { METHOD_VERIFY } else { METHOD_TEMPORARY };
        self.proxy
            .apply_monitors_config(snapshot.serial, method, logical_monitors(settings), HashMap::new())
            .map_err(|e| dbus_error("ApplyMonitorsConfig", e))
    }
}

fn prop<'a, T: TryFrom<&'a Value<'a>>>(props: &'a Props, key: &str) -> Option<T> {
    props.get(key).and_then(|v| T::try_from(v).ok())
}

fn prop_string(props: &Props, key: &str) -> Option<String> {
    props
        .get(key)
        .and_then(|v| <&str>::try_from(&**v).ok())
        .map(str::to_string)
}

/// Mutter transforms are `wl_output` transforms: 0–3 = turned 0/90/180/270°
/// counter-clockwise, 4–7 = the same, mirrored (kept as their rotation).
pub fn rotation_from_transform(t: u32) -> Rotation {
    match t % 4 {
        1 => Rotation::Rotate90,
        2 => Rotation::Rotate180,
        3 => Rotation::Rotate270,
        _ => Rotation::None,
    }
}

pub fn transform_from_rotation(r: Rotation) -> u32 {
    match r {
        Rotation::None => 0,
        Rotation::Rotate90 => 1,
        Rotation::Rotate180 => 2,
        Rotation::Rotate270 => 3,
    }
}

/// `GetCurrentState` → [`Snapshot`]. `lookup_edid` finds a connector's EDID
/// (the kernel's copy; injected for tests).
fn snapshot(state: StateT, lookup_edid: &dyn Fn(&str) -> Option<Edid>) -> Snapshot {
    let (serial, monitors, logical, props) = state;
    let coords = match prop::<u32>(&props, "layout-mode") {
        Some(LAYOUT_PHYSICAL) => Coords::Physical,
        _ => Coords::Logical,
    };
    let outputs = monitors
        .into_iter()
        .map(|((connector, vendor, product, serial), modes, mprops)| {
            let modes: Vec<(OutputMode, bool)> = modes
                .into_iter()
                .map(|(id, w, h, refresh, preferred_scale, scales, p)| {
                    (
                        OutputMode {
                            id,
                            width: w.max(0) as u32,
                            height: h.max(0) as u32,
                            refresh,
                            preferred: prop::<bool>(&p, "is-preferred").unwrap_or(false),
                            scales,
                            preferred_scale: Some(preferred_scale).filter(|s| *s > 0.0),
                        },
                        prop::<bool>(&p, "is-current").unwrap_or(false),
                    )
                })
                .collect();
            let current = modes.iter().position(|(_, current)| *current);
            let active = logical
                .iter()
                .find(|l| l.5.iter().any(|spec| spec.0 == connector))
                .zip(current)
                .map(|((x, y, scale, transform, primary, ..), mode)| Active {
                    mode,
                    x: *x,
                    y: *y,
                    rotation: rotation_from_transform(*transform),
                    scale: *scale,
                    primary: *primary,
                });
            let display_name = prop_string(&mprops, "display-name");
            Output {
                id: monitor_id(
                    &connector,
                    lookup_edid(&connector).as_ref(),
                    display_name.as_deref(),
                    [&vendor, &product, &serial],
                ),
                connector,
                modes: modes.into_iter().map(|(m, _)| m).collect(),
                active,
            }
        })
        .collect();
    Snapshot {
        outputs,
        coords,
        has_primary: true,
        serial,
    }
}

/// Settings → `ApplyMonitorsConfig`'s logical monitors. Outputs sharing a
/// position (duplicated) become one logical monitor showing on both.
fn logical_monitors(settings: &[Setting]) -> Vec<ConfigLogicalT<'static>> {
    let mut out: Vec<ConfigLogicalT<'static>> = Vec::new();
    for s in settings {
        let Some(on) = &s.on else { continue };
        let monitor = (s.connector.clone(), on.mode.id.clone(), HashMap::new());
        let transform = transform_from_rotation(on.rotation);
        match out.iter_mut().find(|l| (l.0, l.1, l.3) == (on.x, on.y, transform)) {
            Some(l) => {
                l.4 |= on.primary;
                l.5.push(monitor);
            }
            None => out.push((on.x, on.y, on.scale, transform, on.primary, vec![monitor])),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::linux::edid::tests::sample;
    use crate::linux::outputs::tests::simulate;
    use pt_core::plan::plan;
    use pt_core::{DisplayMode, MonitorId, RefreshRate, current};

    fn props(pairs: &[(&str, Value<'static>)]) -> Props {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.try_to_owned().unwrap()))
            .collect()
    }

    fn mode(id: &str, w: i32, h: i32, hz: f64, current: bool, preferred: bool) -> ModeT {
        (
            id.into(),
            w,
            h,
            hz,
            1.0,
            vec![1.0, 1.25, 1.5, 2.0],
            props(&[("is-current", current.into()), ("is-preferred", preferred.into())]),
        )
    }

    fn spec(connector: &str) -> Spec {
        (connector.into(), "BOE".into(), "0x07c9".into(), "0x00000000".into())
    }

    /// The laptop in this repo's development machine (from a real
    /// `GetCurrentState`), plus an external monitor that is off.
    fn state() -> StateT {
        (
            42,
            vec![
                (
                    spec("eDP-1"),
                    vec![
                        mode("1920x1080@59.999", 1920, 1080, 59.999_149, true, true),
                        mode("1920x1080@47.997", 1920, 1080, 47.996_772, false, false),
                        mode("1280x720@59.855", 1280, 720, 59.855_125, false, false),
                    ],
                    props(&[("is-builtin", true.into()), ("display-name", "Built-in display".into())]),
                ),
                (
                    ("HDMI-1".into(), "GSM".into(), "LG TV".into(), "0x01010101".into()),
                    vec![
                        mode("3840x2160@60.000", 3840, 2160, 60.0, false, true),
                        mode("1920x1080@60.000", 1920, 1080, 60.0, false, false),
                        mode("1280x720@60.000", 1280, 720, 60.0, false, false),
                    ],
                    props(&[("display-name", "LG Electronics 65\"".into())]),
                ),
            ],
            vec![(0, 0, 1.0, 0, true, vec![spec("eDP-1")], Props::new())],
            props(&[
                ("layout-mode", 1u32.into()),
                ("supports-changing-layout-mode", false.into()),
            ]),
        )
    }

    fn no_edid(_: &str) -> Option<Edid> {
        None
    }

    #[test]
    fn reads_the_current_state() {
        let s = snapshot(state(), &no_edid);
        assert_eq!(s.serial, 42);
        assert_eq!(s.coords, Coords::Logical);
        let layout = s.layout();
        let laptop = &layout.monitors[0];
        assert_eq!(laptop.id.name, "Built-in display");
        assert_eq!(laptop.id.device_path, "eDP-1:BOE:0x07c9:0x00000000");
        assert!(laptop.enabled && laptop.primary);
        assert_eq!(laptop.mode.unwrap().to_string(), "1920x1080 @ 60 Hz");
        let tv = &layout.monitors[1];
        assert_eq!(tv.id.name, "LG Electronics 65\"");
        assert!(!tv.enabled);
    }

    #[test]
    fn kernel_edid_gives_stable_ids() {
        let lookup = |c: &str| (c == "HDMI-1").then(|| Edid::parse(&sample("LG TV", None)).unwrap());
        let s = snapshot(state(), &lookup);
        assert_eq!(s.outputs[1].id.device_path, "HDMI-1:DELA0B1:12345678");
        assert_eq!(s.outputs[1].id.name, "LG Electronics 65\"");
    }

    #[test]
    fn physical_layout_mode_is_respected() {
        let mut st = state();
        st.3 = props(&[("layout-mode", LAYOUT_PHYSICAL.into())]);
        assert_eq!(snapshot(st, &no_edid).coords, Coords::Physical);
    }

    #[test]
    fn transforms_round_trip() {
        for r in [
            Rotation::None,
            Rotation::Rotate90,
            Rotation::Rotate180,
            Rotation::Rotate270,
        ] {
            assert_eq!(rotation_from_transform(transform_from_rotation(r)), r);
        }
        assert_eq!(rotation_from_transform(5), Rotation::Rotate90, "flipped-90");
    }

    fn tv_right_of_laptop() -> (Snapshot, Vec<Setting>, pt_core::DisplayLayout) {
        let s = snapshot(state(), &no_edid);
        let mut saved = s.layout();
        saved.monitors[1].enabled = true;
        saved.monitors[1].mode = Some(DisplayMode {
            width: 3840,
            height: 2160,
            refresh: RefreshRate::from_hz(60),
        });
        saved.monitors[1].position = pt_core::Position { x: 1920, y: 0 };
        let ids: Vec<MonitorId> = saved.monitors.iter().map(|m| m.id.clone()).collect();
        let settings = s.configure(&plan(&saved, &ids).unwrap(), "test").unwrap();
        (s, settings, saved)
    }

    #[test]
    fn builds_logical_monitors() {
        let (s, settings, saved) = tv_right_of_laptop();
        let lm = logical_monitors(&settings);
        assert_eq!(lm.len(), 2);
        assert_eq!((lm[0].0, lm[0].1, lm[0].2, lm[0].3, lm[0].4), (0, 0, 1.0, 0, true));
        assert_eq!(lm[0].5[0].0, "eDP-1");
        assert_eq!(lm[0].5[0].1, "1920x1080@59.999");
        assert_eq!((lm[1].0, lm[1].1, lm[1].4), (1920, 0, false));
        assert_eq!(lm[1].5[0].1, "3840x2160@60.000");
        // And GNOME would then report the profile as on screen.
        assert!(current::matches(&saved, &simulate(&s, &settings).layout()));
    }

    #[test]
    fn duplicates_share_one_logical_monitor() {
        let s = snapshot(state(), &no_edid);
        let lm = logical_monitors(&s.duplicate_all("test").unwrap());
        assert_eq!(lm.len(), 1);
        assert!(lm[0].4, "primary");
        let modes: Vec<&str> = lm[0].5.iter().map(|m| m.1.as_str()).collect();
        assert_eq!(modes, ["1920x1080@59.999", "1920x1080@60.000"]);
    }

    /// On a GNOME desktop: `cargo test -p pt-platform -- --ignored`. Reads
    /// the real monitors and asks Mutter to *verify* (not apply) the
    /// current arrangement and "duplicate", so nothing on screen changes.
    #[test]
    #[ignore = "needs a running GNOME session"]
    fn live_gnome_round_trip() {
        let driver = MutterDriver::connect().expect("GNOME's display service");
        let s = driver.snapshot().unwrap();
        let layout = s.layout();
        eprintln!("{layout:#?}");
        assert!(layout.enabled().count() >= 1);
        let ids: Vec<MonitorId> = layout.monitors.iter().map(|m| m.id.clone()).collect();
        let settings = s.configure(&plan(&layout, &ids).unwrap(), driver.name()).unwrap();
        eprintln!("{:#?}", logical_monitors(&settings));
        driver
            .apply(&s, &settings, true)
            .expect("Mutter accepts the current layout");
        driver
            .apply(&s, &s.duplicate_all("test").unwrap(), true)
            .expect("Mutter accepts duplicate");
        driver.apply(&s, &s.extend_all(), true).expect("Mutter accepts extend");
    }

    #[test]
    fn switched_off_monitors_are_left_out() {
        let (_, mut settings, _) = tv_right_of_laptop();
        settings[1].on = None;
        let lm = logical_monitors(&settings);
        assert_eq!(lm.len(), 1);
        assert_eq!(lm[0].5[0].0, "eDP-1");
    }
}
