//! What every Linux display stack has in common, and all the logic that
//! sits on top of it. Each driver ([`super::mutter`], [`super::kscreen`],
//! [`super::wlroots`], [`super::xrandr`]) only reads a [`Snapshot`] and
//! carries out a list of [`Setting`]s; turning a profile into settings
//! (choosing modes and scales, pixel ↔ logical coordinates, duplicate
//! groups) happens here, without any OS calls, so it is unit-tested.

use pt_core::{
    ApplyPlan, DisplayBackend, DisplayLayout, DisplayMode, Error, MonitorId, MonitorState, Position, RefreshRate,
    Result, Rotation, Topology,
};

use super::coords::{self, Rect};
use super::edid::Edid;

/// A mode as the display stack reports it, in the monitor's own
/// (unrotated) orientation.
#[derive(Debug, Clone, PartialEq)]
pub struct OutputMode {
    /// The driver's handle for it (Mutter's mode id, KScreen's mode id,
    /// RandR's XID; wlroots uses `WxH@Hz`).
    pub id: String,
    pub width: u32,
    pub height: u32,
    /// Hz, as precise as the driver reports it.
    pub refresh: f64,
    pub preferred: bool,
    /// Scales the compositor accepts with this mode (empty: any).
    pub scales: Vec<f64>,
    /// What the compositor would pick for this mode, if it says.
    pub preferred_scale: Option<f64>,
}

/// How an output that's switched on is set up right now.
#[derive(Debug, Clone, PartialEq)]
pub struct Active {
    /// Index into [`Output::modes`].
    pub mode: usize,
    /// Top-left corner in the driver's coordinates (see [`Coords`]).
    pub x: i32,
    pub y: i32,
    pub rotation: Rotation,
    pub scale: f64,
    pub primary: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Output {
    /// Connector name, e.g. `DP-1`; what the driver addresses it by.
    pub connector: String,
    pub id: MonitorId,
    pub modes: Vec<OutputMode>,
    /// `None` while the output is switched off.
    pub active: Option<Active>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Coords {
    /// Positions in pixels (X11, GNOME's "physical" layout mode).
    Physical,
    /// Positions in scaled units: a monitor is `pixels / scale` wide
    /// (Wayland compositors).
    Logical,
}

/// Everything a driver reports in one read.
#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub outputs: Vec<Output>,
    pub coords: Coords,
    /// Whether the stack has a "primary monitor" at all (wlroots doesn't).
    pub has_primary: bool,
    /// Mutter's configuration serial (0 elsewhere).
    pub serial: u32,
}

/// How one output should end up.
#[derive(Debug, Clone, PartialEq)]
pub struct Setting {
    pub connector: String,
    pub on: Option<On>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct On {
    pub mode: OutputMode,
    /// In the driver's coordinates, never negative.
    pub x: i32,
    pub y: i32,
    pub rotation: Rotation,
    pub scale: f64,
    pub primary: bool,
}

/// One display stack: reads the outputs and carries out settings.
pub trait Driver: Send + Sync {
    fn name(&self) -> &'static str;
    fn snapshot(&self) -> Result<Snapshot>;
    /// Every connected output appears in `settings`. With `validate_only`,
    /// check without changing anything (as far as the stack allows).
    fn apply(&self, snapshot: &Snapshot, settings: &[Setting], validate_only: bool) -> Result<()>;
}

/// The [`DisplayBackend`] for every Linux display stack: the shared logic
/// around one [`Driver`].
pub struct LinuxDisplayBackend {
    driver: Box<dyn Driver>,
}

impl LinuxDisplayBackend {
    pub fn new(driver: Box<dyn Driver>) -> Self {
        LinuxDisplayBackend { driver }
    }
}

impl DisplayBackend for LinuxDisplayBackend {
    fn name(&self) -> &'static str {
        self.driver.name()
    }

    fn current_layout(&self) -> Result<DisplayLayout> {
        Ok(self.driver.snapshot()?.layout())
    }

    fn apply(&self, plan: &ApplyPlan, validate_only: bool) -> Result<()> {
        let snapshot = self.driver.snapshot()?;
        let settings = snapshot.configure(plan, self.driver.name())?;
        debug_settings(&settings);
        self.driver.apply(&snapshot, &settings, validate_only)
    }

    fn set_topology(&self, topology: Topology) -> Result<()> {
        let snapshot = self.driver.snapshot()?;
        let settings = match topology {
            Topology::Clone => snapshot.duplicate_all(self.driver.name())?,
            Topology::Extend => snapshot.extend_all(),
        };
        debug_settings(&settings);
        self.driver.apply(&snapshot, &settings, false)
    }

    fn supported_modes(&self, monitor: &MonitorId) -> Result<Vec<DisplayMode>> {
        self.driver.snapshot()?.modes(monitor)
    }
}

/// `--debug` (PANELTARIUM_DEBUG): print what is about to be sent.
fn debug_settings(settings: &[Setting]) {
    if std::env::var_os("PANELTARIUM_DEBUG").is_some_and(|v| !v.is_empty()) {
        eprintln!("sending:");
        for s in settings {
            match &s.on {
                Some(on) => eprintln!(
                    "  {}: {}x{} @ {:.3} Hz (mode {}) at {},{} rotation {:?} scale {}{}",
                    s.connector,
                    on.mode.width,
                    on.mode.height,
                    on.mode.refresh,
                    on.mode.id,
                    on.x,
                    on.y,
                    on.rotation,
                    on.scale,
                    if on.primary { " primary" } else { "" }
                ),
                None => eprintln!("  {}: off", s.connector),
            }
        }
    }
}

/// A refresh rate in Hz as an exact fraction (thousandths of a hertz), so
/// the same mode always reads back as the same value.
pub fn refresh_rate(hz: f64) -> RefreshRate {
    RefreshRate {
        numerator: (hz * 1000.0).round().max(0.0) as u32,
        denominator: 1000,
    }
}

fn sideways(rotation: Rotation) -> bool {
    matches!(rotation, Rotation::Rotate90 | Rotation::Rotate270)
}

/// The mode's size on the desktop, i.e. after rotation.
pub fn desktop_size(mode: &OutputMode, rotation: Rotation) -> (u32, u32) {
    if sideways(rotation) {
        (mode.height, mode.width)
    } else {
        (mode.width, mode.height)
    }
}

/// How big `desktop` pixels are in the driver's coordinates.
fn extent(desktop: (u32, u32), scale: f64, coords: Coords) -> (u32, u32) {
    match coords {
        Coords::Physical => desktop,
        Coords::Logical => {
            let s = if scale > 0.0 { scale } else { 1.0 };
            (
                (f64::from(desktop.0) / s).round() as u32,
                (f64::from(desktop.1) / s).round() as u32,
            )
        }
    }
}

/// Build a [`MonitorId`] from a connector and what's known about the
/// monitor on it. `device_path` = connector + EDID ids, so like Windows'
/// device path it is stable for one monitor on one port, and the EDID
/// fields let a profile follow a monitor to another port.
pub fn monitor_id(connector: &str, edid: Option<&Edid>, friendly: Option<&str>, fallback: [&str; 3]) -> MonitorId {
    let friendly = friendly.map(str::trim).filter(|s| !s.is_empty());
    match edid {
        Some(e) => MonitorId {
            device_path: format!("{connector}:{}{:04X}:{}", e.vendor(), e.product, e.serial),
            name: friendly
                .map(str::to_string)
                .or_else(|| e.name.clone())
                .unwrap_or_else(|| connector.to_string()),
            edid_manufacturer: Some(e.manufacturer),
            edid_product: Some(e.product),
        },
        None => {
            let [vendor, product, serial] = fallback.map(str::trim);
            let model = [vendor, product]
                .into_iter()
                .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("unknown"))
                .collect::<Vec<_>>()
                .join(" ");
            MonitorId {
                device_path: format!("{connector}:{vendor}:{product}:{serial}"),
                name: friendly
                    .map(str::to_string)
                    .or_else(|| (!model.is_empty()).then_some(model))
                    .unwrap_or_else(|| connector.to_string()),
                edid_manufacturer: super::edid::pnp_id(vendor),
                edid_product: parse_hex(product),
            }
        }
    }
}

fn parse_hex(s: &str) -> Option<u16> {
    u16::from_str_radix(s.strip_prefix("0x").or_else(|| s.strip_prefix("0X"))?, 16).ok()
}

impl Output {
    fn active_mode(&self) -> Option<(&Active, &OutputMode)> {
        let a = self.active.as_ref()?;
        Some((a, self.modes.get(a.mode)?))
    }

    fn preferred_mode(&self) -> Option<&OutputMode> {
        self.modes.iter().find(|m| m.preferred).or_else(|| {
            self.modes
                .iter()
                .max_by_key(|m| (m.width * m.height, (m.refresh * 1000.0) as u64))
        })
    }

    /// The mode for `want` (desktop size + refresh), turned by `rotation`:
    /// exact refresh if there is one, else the closest refresh at that size.
    fn pick_mode(&self, want: DisplayMode, rotation: Rotation, driver: &'static str) -> Result<&OutputMode> {
        let (w, h) = if sideways(rotation) {
            (want.height, want.width)
        } else {
            (want.width, want.height)
        };
        let hz = want.refresh.hz();
        self.modes
            .iter()
            .filter(|m| m.width == w && m.height == h)
            .min_by(|a, b| {
                let da = (a.refresh - hz).abs();
                let db = (b.refresh - hz).abs();
                da.total_cmp(&db).then(b.preferred.cmp(&a.preferred))
            })
            .ok_or_else(|| Error::Os {
                call: driver,
                code: None,
                message: format!("'{}' has no {w}x{h} mode", self.id.label()),
            })
    }

    /// Keep the current scale when it still works with `mode`; otherwise
    /// what the compositor prefers for it, else 1.
    fn scale_for(&self, mode: &OutputMode) -> f64 {
        let fits = |s: f64| mode.scales.is_empty() || mode.scales.iter().any(|x| (x - s).abs() < 1e-4);
        self.active
            .as_ref()
            .map(|a| a.scale)
            .filter(|&s| fits(s))
            .or(mode.preferred_scale)
            .or_else(|| mode.scales.iter().copied().find(|s| (s - 1.0).abs() < 1e-4))
            .unwrap_or(1.0)
    }
}

impl Snapshot {
    /// The monitors in Paneltarium's terms: pixel positions, the primary
    /// at (0, 0).
    pub fn layout(&self) -> DisplayLayout {
        let on: Vec<(usize, &Active, &OutputMode)> = self
            .outputs
            .iter()
            .enumerate()
            .filter_map(|(i, o)| o.active_mode().map(|(a, m)| (i, a, m)))
            .collect();
        let rects: Vec<Rect> = on
            .iter()
            .map(|(_, a, m)| {
                let (w, h) = extent(desktop_size(m, a.rotation), a.scale, self.coords);
                Rect::new(a.x, a.y, w, h)
            })
            .collect();
        let pixels: Vec<(u32, u32)> = on.iter().map(|(_, a, m)| desktop_size(m, a.rotation)).collect();
        // The primary: the one the stack says, else (no such concept) the
        // top-left one, which is what Windows would call primary.
        let root = on
            .iter()
            .position(|(_, a, _)| self.has_primary && a.primary)
            .or_else(|| (0..on.len()).min_by_key(|&k| (rects[k].x != 0 || rects[k].y != 0, rects[k].y, rects[k].x)));
        let positions = match root {
            Some(r) => coords::convert(&rects, &pixels, r),
            None => Vec::new(),
        };

        let mut monitors: Vec<MonitorState> = self
            .outputs
            .iter()
            .map(|o| MonitorState::disabled(o.id.clone()))
            .collect();
        for (k, (i, a, m)) in on.iter().enumerate() {
            let (width, height) = pixels[k];
            let (x, y) = positions[k];
            monitors[*i] = MonitorState {
                id: self.outputs[*i].id.clone(),
                enabled: true,
                primary: Some(k) == root,
                mode: Some(DisplayMode {
                    width,
                    height,
                    refresh: refresh_rate(m.refresh),
                }),
                position: Position { x, y },
                rotation: a.rotation,
            };
        }
        DisplayLayout { monitors }
    }

    fn find(&self, id: &MonitorId) -> Option<&Output> {
        self.outputs.iter().find(|o| o.id.device_path == id.device_path)
    }

    /// Settings that carry out `plan`.
    pub fn configure(&self, plan: &ApplyPlan, driver: &'static str) -> Result<Vec<Setting>> {
        struct Want<'a> {
            output: &'a Output,
            mode: &'a OutputMode,
            rect: Rect,
            rotation: Rotation,
            primary: bool,
        }
        let mut wanted = Vec::new();
        for p in plan.enabled() {
            let output = self
                .find(&p.target)
                .ok_or_else(|| Error::MonitorNotFound(p.target.label().to_string()))?;
            let mode = p.mode.expect("enabled planned monitors always have a mode");
            wanted.push(Want {
                output,
                mode: output.pick_mode(mode, p.rotation, driver)?,
                rect: Rect::new(p.position.x, p.position.y, mode.width, mode.height),
                rotation: p.rotation,
                primary: p.primary,
            });
        }
        // Duplicated monitors (same rectangle) share one scale, so they
        // also share one logical rectangle.
        let scales: Vec<f64> = wanted
            .iter()
            .map(|w| {
                let first = wanted.iter().find(|o| o.rect == w.rect).expect("w itself matches");
                first.output.scale_for(first.mode)
            })
            .collect();
        let sizes: Vec<(u32, u32)> = wanted
            .iter()
            .zip(&scales)
            .map(|(w, &s)| extent(desktop_size(w.mode, w.rotation), s, self.coords))
            .collect();
        let rects: Vec<Rect> = wanted.iter().map(|w| w.rect).collect();
        let root = wanted.iter().position(|w| w.primary).unwrap_or(0);
        let positions = shift_to_origin(coords::convert(&rects, &sizes, root));

        let mut settings: Vec<Setting> = self
            .outputs
            .iter()
            .map(|o| Setting {
                connector: o.connector.clone(),
                on: None,
            })
            .collect();
        for ((w, (x, y)), scale) in wanted.iter().zip(positions).zip(scales) {
            let s = settings
                .iter_mut()
                .find(|s| s.connector == w.output.connector)
                .expect("every output has a setting");
            s.on = Some(On {
                mode: w.mode.clone(),
                x,
                y,
                rotation: w.rotation,
                scale,
                primary: w.primary,
            });
        }
        Ok(settings)
    }

    /// Every monitor on, showing the same picture: the largest resolution
    /// they all have, each at its best refresh rate for it.
    pub fn duplicate_all(&self, driver: &'static str) -> Result<Vec<Setting>> {
        let common = self
            .outputs
            .first()
            .into_iter()
            .flat_map(|o| o.modes.iter().map(|m| (m.width, m.height)))
            .filter(|&(w, h)| {
                self.outputs
                    .iter()
                    .all(|o| o.modes.iter().any(|m| m.width == w && m.height == h))
            })
            .max_by_key(|&(w, h)| (w * h, w));
        let Some((w, h)) = common else {
            return Err(Error::Os {
                call: driver,
                code: None,
                message: "the monitors have no resolution in common to duplicate".into(),
            });
        };
        // One scale for the shared picture: one every mode accepts.
        let modes: Vec<&OutputMode> = self
            .outputs
            .iter()
            .map(|o| {
                o.modes
                    .iter()
                    .filter(|m| m.width == w && m.height == h)
                    .max_by(|a, b| a.refresh.total_cmp(&b.refresh))
                    .expect("common size")
            })
            .collect();
        let accepted = |s: f64| {
            modes
                .iter()
                .all(|m| m.scales.is_empty() || m.scales.iter().any(|x| (x - s).abs() < 1e-4))
        };
        let scale = modes
            .iter()
            .filter_map(|m| m.preferred_scale)
            .find(|&s| accepted(s))
            .unwrap_or(1.0);
        Ok(self
            .outputs
            .iter()
            .zip(modes)
            .enumerate()
            .map(|(i, (o, mode))| Setting {
                connector: o.connector.clone(),
                on: Some(On {
                    mode: mode.clone(),
                    x: 0,
                    y: 0,
                    rotation: Rotation::None,
                    scale,
                    primary: i == 0,
                }),
            })
            .collect())
    }

    /// Every monitor on, side by side: monitors that are on keep their mode
    /// and order, the others join on the right at their preferred mode.
    pub fn extend_all(&self) -> Vec<Setting> {
        let mut order: Vec<usize> = (0..self.outputs.len()).collect();
        order.sort_by_key(|&i| match &self.outputs[i].active {
            Some(a) => (0, a.x, a.y),
            None => (1, 0, 0),
        });
        let primary = order
            .iter()
            .copied()
            .find(|&i| self.outputs[i].active.as_ref().is_some_and(|a| a.primary))
            .or(order.first().copied());
        let mut x = 0;
        let mut settings: Vec<Setting> = Vec::new();
        for i in order {
            let o = &self.outputs[i];
            let (mode, rotation) = match o.active_mode() {
                Some((a, m)) => (m, a.rotation),
                None => match o.preferred_mode() {
                    Some(m) => (m, Rotation::None),
                    None => continue,
                },
            };
            let scale = o.scale_for(mode);
            let (w, _) = extent(desktop_size(mode, rotation), scale, self.coords);
            settings.push(Setting {
                connector: o.connector.clone(),
                on: Some(On {
                    mode: mode.clone(),
                    x,
                    y: 0,
                    rotation,
                    scale,
                    primary: Some(i) == primary,
                }),
            });
            x += w as i32;
        }
        settings
    }

    pub fn modes(&self, id: &MonitorId) -> Result<Vec<DisplayMode>> {
        let o = self
            .find(id)
            .ok_or_else(|| Error::MonitorNotFound(id.label().to_string()))?;
        let rotation = o.active.as_ref().map(|a| a.rotation).unwrap_or_default();
        let mut modes: Vec<DisplayMode> = o
            .modes
            .iter()
            .map(|m| {
                let (width, height) = desktop_size(m, rotation);
                DisplayMode {
                    width,
                    height,
                    refresh: refresh_rate(m.refresh),
                }
            })
            .collect();
        modes.dedup();
        Ok(modes)
    }
}

/// Move everything so the top-left-most edge is at 0: display stacks on
/// Linux don't accept negative positions.
fn shift_to_origin(positions: Vec<(i32, i32)>) -> Vec<(i32, i32)> {
    let min_x = positions.iter().map(|p| p.0).min().unwrap_or(0);
    let min_y = positions.iter().map(|p| p.1).min().unwrap_or(0);
    positions.into_iter().map(|(x, y)| (x - min_x, y - min_y)).collect()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use pt_core::plan::{check_arrangement, plan};
    use pt_core::{Profile, current};

    pub fn mode(id: &str, w: u32, h: u32, hz: f64, preferred: bool) -> OutputMode {
        OutputMode {
            id: id.into(),
            width: w,
            height: h,
            refresh: hz,
            preferred,
            scales: Vec::new(),
            preferred_scale: None,
        }
    }

    pub fn output(connector: &str, modes: Vec<OutputMode>, active: Option<Active>) -> Output {
        Output {
            connector: connector.into(),
            id: monitor_id(
                connector,
                None,
                Some(&format!("Monitor {connector}")),
                ["DEL", "0x1234", "1"],
            ),
            modes,
            active,
        }
    }

    pub fn active(mode: usize, x: i32, y: i32, scale: f64, primary: bool) -> Option<Active> {
        Some(Active {
            mode,
            x,
            y,
            rotation: Rotation::None,
            scale,
            primary,
        })
    }

    /// Laptop 1920x1080 (100 %) on the left, a 4K monitor at 200 % right
    /// of it (1920 logical units wide), a TV that's off.
    fn hidpi() -> Snapshot {
        Snapshot {
            outputs: vec![
                output(
                    "eDP-1",
                    vec![mode("l60", 1920, 1080, 60.0, true)],
                    active(0, 0, 0, 1.0, true),
                ),
                output(
                    "DP-1",
                    vec![
                        mode("4k60", 3840, 2160, 59.997, true),
                        mode("4k30", 3840, 2160, 30.0, false),
                        mode("1440", 2560, 1440, 59.951, false),
                    ],
                    active(0, 1920, 0, 2.0, false),
                ),
                output(
                    "HDMI-A-1",
                    vec![
                        mode("tv", 3840, 2160, 60.0, true),
                        mode("tv1080", 1920, 1080, 60.0, false),
                    ],
                    None,
                ),
            ],
            coords: Coords::Logical,
            has_primary: true,
            serial: 7,
        }
    }

    fn on(s: &[Setting], connector: &str) -> Option<On> {
        s.iter().find(|s| s.connector == connector).unwrap().on.clone()
    }

    #[test]
    fn refresh_rates_are_exact_fractions() {
        let r = refresh_rate(59.997);
        assert_eq!((r.numerator, r.denominator), (59997, 1000));
        assert_eq!(refresh_rate(60.0).to_string(), "60 Hz");
        assert_eq!(refresh_rate(59.94).to_string(), "59.94 Hz");
    }

    #[test]
    fn layout_is_in_pixels_with_the_primary_at_the_origin() {
        let l = hidpi().layout();
        let laptop = &l.monitors[0];
        let big = &l.monitors[1];
        assert!(laptop.primary && laptop.enabled);
        assert_eq!(laptop.position, Position { x: 0, y: 0 });
        assert_eq!(big.position, Position { x: 1920, y: 0 });
        assert_eq!(big.mode.unwrap().width, 3840, "pixels, not logical units");
        assert_eq!(big.mode.unwrap().refresh, refresh_rate(59.997));
        assert!(!l.monitors[2].enabled);
        // A layout Windows would accept is one Linux stacks accept too.
        let p = plan(&l, &l.monitors.iter().map(|m| m.id.clone()).collect::<Vec<_>>()).unwrap();
        check_arrangement(&p).unwrap();
    }

    #[test]
    fn primary_on_the_right_is_rebased() {
        let mut s = hidpi();
        s.outputs[0].active.as_mut().unwrap().primary = false;
        s.outputs[1].active.as_mut().unwrap().primary = true;
        let l = s.layout();
        assert_eq!(l.monitors[1].position, Position { x: 0, y: 0 });
        assert_eq!(l.monitors[0].position, Position { x: -1920, y: 0 });
    }

    #[test]
    fn without_a_primary_concept_the_top_left_one_is_primary() {
        let mut s = hidpi();
        s.has_primary = false;
        s.outputs[1].active.as_mut().unwrap().primary = true; // ignored
        let l = s.layout();
        assert!(l.monitors[0].primary && !l.monitors[1].primary);
    }

    #[test]
    fn rotated_monitors_are_tall() {
        let mut s = hidpi();
        s.coords = Coords::Physical;
        let a = s.outputs[1].active.as_mut().unwrap();
        a.rotation = Rotation::Rotate90;
        a.scale = 1.0;
        let l = s.layout();
        let m = l.monitors[1].mode.unwrap();
        assert_eq!((m.width, m.height), (2160, 3840));
        assert_eq!(s.modes(&l.monitors[1].id).unwrap()[0].width, 2160);
    }

    #[test]
    fn saved_layout_round_trips_unchanged() {
        let s = hidpi();
        let l = s.layout();
        let p = plan(&l, &l.monitors.iter().map(|m| m.id.clone()).collect::<Vec<_>>()).unwrap();
        let settings = s.configure(&p, "test").unwrap();
        let big = on(&settings, "DP-1").unwrap();
        assert_eq!(
            (big.x, big.y, big.scale),
            (1920, 0, 2.0),
            "logical position, scale kept"
        );
        assert_eq!(big.mode.id, "4k60");
        assert_eq!(on(&settings, "eDP-1").unwrap().mode.id, "l60");
        assert!(on(&settings, "eDP-1").unwrap().primary);
        assert_eq!(on(&settings, "HDMI-A-1"), None);
    }

    #[test]
    fn applying_then_reading_gives_the_profile_back() {
        // Simulate the compositor: apply the settings to the snapshot,
        // read it again, and check the profile counts as "on screen".
        let s = hidpi();
        let mut saved = s.layout();
        // Profile: TV on the left of the laptop at 1080p, 4K off.
        saved.monitors[1].enabled = false;
        saved.monitors[1].mode = None;
        saved.monitors[2] = MonitorState {
            enabled: true,
            primary: false,
            mode: Some(DisplayMode {
                width: 1920,
                height: 1080,
                refresh: RefreshRate::from_hz(60),
            }),
            position: Position { x: -1920, y: 0 },
            ..saved.monitors[2].clone()
        };
        let ids: Vec<MonitorId> = saved.monitors.iter().map(|m| m.id.clone()).collect();
        let settings = s.configure(&plan(&saved, &ids).unwrap(), "test").unwrap();
        assert_eq!(on(&settings, "DP-1"), None);
        let tv = on(&settings, "HDMI-A-1").unwrap();
        assert_eq!((tv.x, tv.y, tv.mode.id.as_str()), (0, 0, "tv1080"), "shifted to x >= 0");
        assert_eq!(on(&settings, "eDP-1").unwrap().x, 1920);

        let after = simulate(&s, &settings).layout();
        let profile = Profile::new("TV left", saved).unwrap();
        assert!(current::matches(&profile.layout, &after), "{after:#?}");
    }

    /// What a compositor would report after applying `settings`.
    pub fn simulate(before: &Snapshot, settings: &[Setting]) -> Snapshot {
        let mut s = before.clone();
        for o in &mut s.outputs {
            let set = settings.iter().find(|x| x.connector == o.connector).unwrap();
            o.active = set.on.as_ref().map(|on| Active {
                mode: o.modes.iter().position(|m| m.id == on.mode.id).unwrap(),
                x: on.x,
                y: on.y,
                rotation: on.rotation,
                scale: on.scale,
                primary: on.primary,
            });
        }
        s
    }

    #[test]
    fn missing_mode_and_unknown_monitor_are_errors() {
        let s = hidpi();
        let mut saved = s.layout();
        saved.monitors[0].mode.as_mut().unwrap().width = 1234;
        let ids: Vec<MonitorId> = saved.monitors.iter().map(|m| m.id.clone()).collect();
        let err = s.configure(&plan(&saved, &ids).unwrap(), "test").unwrap_err();
        assert!(err.to_string().contains("no 1234x1080 mode"), "{err}");

        let mut stranger = s.clone();
        stranger.outputs.remove(0);
        let saved = s.layout();
        let p = plan(&saved, &ids).unwrap();
        assert!(matches!(stranger.configure(&p, "test"), Err(Error::MonitorNotFound(_))));
    }

    #[test]
    fn closest_refresh_when_the_exact_one_is_gone() {
        let s = hidpi();
        let mut saved = s.layout();
        saved.monitors[1].mode.as_mut().unwrap().refresh = RefreshRate::from_hz(144);
        let ids: Vec<MonitorId> = saved.monitors.iter().map(|m| m.id.clone()).collect();
        let settings = s.configure(&plan(&saved, &ids).unwrap(), "test").unwrap();
        assert_eq!(on(&settings, "DP-1").unwrap().mode.id, "4k60");
        saved.monitors[1].mode.as_mut().unwrap().refresh = RefreshRate::from_hz(30);
        let settings = s.configure(&plan(&saved, &ids).unwrap(), "test").unwrap();
        assert_eq!(on(&settings, "DP-1").unwrap().mode.id, "4k30");
    }

    #[test]
    fn scale_follows_what_the_mode_accepts() {
        let mut s = hidpi();
        // 1440p only accepts 1 and 1.25: the current 2.0 can't stay.
        s.outputs[1].modes[2].scales = vec![1.0, 1.25];
        s.outputs[1].modes[2].preferred_scale = Some(1.25);
        let mut saved = s.layout();
        saved.monitors[1].mode = Some(DisplayMode {
            width: 2560,
            height: 1440,
            refresh: refresh_rate(59.951),
        });
        let ids: Vec<MonitorId> = saved.monitors.iter().map(|m| m.id.clone()).collect();
        let settings = s.configure(&plan(&saved, &ids).unwrap(), "test").unwrap();
        let big = on(&settings, "DP-1").unwrap();
        assert_eq!((big.scale, big.x), (1.25, 1920));
        // A monitor that was off gets the preferred scale, or 1.
        let tv_mode = &s.outputs[2].modes[0];
        assert_eq!(s.outputs[2].scale_for(tv_mode), 1.0);
    }

    #[test]
    fn duplicated_profile_shares_position_and_scale() {
        let s = hidpi();
        let mut saved = s.layout();
        for m in &mut saved.monitors[..2] {
            m.mode = Some(DisplayMode {
                width: 1920,
                height: 1080,
                refresh: RefreshRate::from_hz(60),
            });
            m.position = Position::default();
        }
        // The 4K has no 1080p mode in this fixture; give it one.
        let mut s = s;
        s.outputs[1].modes.push(mode("1080", 1920, 1080, 60.0, false));
        let ids: Vec<MonitorId> = saved.monitors.iter().map(|m| m.id.clone()).collect();
        let settings = s.configure(&plan(&saved, &ids).unwrap(), "test").unwrap();
        let (a, b) = (on(&settings, "eDP-1").unwrap(), on(&settings, "DP-1").unwrap());
        assert_eq!((a.x, a.y, a.scale), (b.x, b.y, b.scale));
        assert!(current::is_duplicate(&simulate(&s, &settings).layout()));
    }

    #[test]
    fn duplicate_all_uses_the_largest_common_resolution() {
        let mut s = hidpi();
        s.outputs[1].modes.push(mode("1080", 1920, 1080, 60.0, false));
        let settings = s.duplicate_all("test").unwrap();
        assert!(settings.iter().all(|x| {
            let on = x.on.as_ref().unwrap();
            (on.x, on.y, on.mode.width) == (0, 0, 1920)
        }));
        // The laptop only has 1080p, so that's the common size.
        assert_eq!(settings.iter().filter(|x| x.on.as_ref().unwrap().primary).count(), 1);

        let mut no_common = s.clone();
        no_common.outputs[0].modes = vec![mode("odd", 1366, 768, 60.0, true)];
        no_common.outputs[0].active.as_mut().unwrap().mode = 0;
        assert!(no_common.duplicate_all("test").is_err());
    }

    #[test]
    fn extend_all_lines_everything_up() {
        let s = hidpi();
        let settings = s.extend_all();
        let xs: Vec<(String, i32)> = settings
            .iter()
            .map(|x| (x.connector.clone(), x.on.as_ref().unwrap().x))
            .collect();
        assert_eq!(
            xs,
            [("eDP-1".into(), 0), ("DP-1".into(), 1920), ("HDMI-A-1".into(), 3840)],
            "4K is 1920 logical units wide"
        );
        assert_eq!(on(&settings, "HDMI-A-1").unwrap().mode.id, "tv", "preferred mode");
        assert!(on(&settings, "eDP-1").unwrap().primary);
        let after = simulate(&s, &settings).layout();
        assert_eq!(after.enabled().count(), 3);
    }

    #[test]
    fn monitor_ids_prefer_edid() {
        let e = Edid::parse(&super::super::edid::tests::sample("DELL U2720Q", None)).unwrap();
        let id = monitor_id("DP-1", Some(&e), None, ["x", "y", "z"]);
        assert_eq!(id.device_path, "DP-1:DELA0B1:12345678");
        assert_eq!(id.name, "DELL U2720Q");
        assert_eq!(id.edid_manufacturer, Some(0x10AC));
        let id = monitor_id("DP-1", Some(&e), Some("Dell 27\""), ["x", "y", "z"]);
        assert_eq!(id.name, "Dell 27\"", "the desktop's own name wins");

        let id = monitor_id("eDP-1", None, Some(" "), ["BOE", "0x07c9", "0x00000000"]);
        assert_eq!(id.device_path, "eDP-1:BOE:0x07c9:0x00000000");
        assert_eq!(id.name, "BOE 0x07c9");
        assert_eq!(id.edid_product, Some(0x07C9));
        assert_eq!(id.edid_manufacturer, super::super::edid::pnp_id("BOE"));
        let id = monitor_id("HDMI-1", None, None, ["", "", ""]);
        assert_eq!(id.name, "HDMI-1");
    }
}
