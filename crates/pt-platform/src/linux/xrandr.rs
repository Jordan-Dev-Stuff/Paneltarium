//! X11 sessions (Xfce, MATE, Cinnamon, i3, KDE on X11, ...): `xrandr`.
//! Reads `xrandr --verbose` (which includes each monitor's EDID), applies
//! with `xrandr --output DP-1 --mode 0x4a --pos 0x0 ...`; `--dryrun` checks
//! without changing anything.

use pt_core::{Error, Result, Rotation};

use super::command::Runner;
use super::edid::Edid;
use super::outputs::{Active, Coords, Driver, Output, OutputMode, Setting, Snapshot, monitor_id};

pub const PROGRAM: &str = "xrandr";

pub struct XrandrDriver {
    runner: Box<dyn Runner>,
}

impl XrandrDriver {
    pub fn new(runner: Box<dyn Runner>) -> Self {
        XrandrDriver { runner }
    }
}

impl Driver for XrandrDriver {
    fn name(&self) -> &'static str {
        "x11-xrandr"
    }

    fn snapshot(&self) -> Result<Snapshot> {
        parse(&self.runner.run(PROGRAM, &["--verbose".into()])?)
    }

    fn apply(&self, _: &Snapshot, settings: &[Setting], validate_only: bool) -> Result<()> {
        let mut a = args(settings);
        if validate_only {
            a.insert(0, "--dryrun".into());
        }
        self.runner.run(PROGRAM, &a).map(drop)
    }
}

/// RandR rotations: "left" = turned 90° counter-clockwise.
fn rotation(word: &str) -> Option<Rotation> {
    Some(match word {
        "normal" => Rotation::None,
        "left" => Rotation::Rotate90,
        "inverted" => Rotation::Rotate180,
        "right" => Rotation::Rotate270,
        _ => return None,
    })
}

fn rotation_arg(r: Rotation) -> &'static str {
    match r {
        Rotation::None => "normal",
        Rotation::Rotate90 => "left",
        Rotation::Rotate180 => "inverted",
        Rotation::Rotate270 => "right",
    }
}

/// `1920x1080+0+360` → (x, y). The size is the rotated one; the mode gives
/// the real size.
fn geometry(word: &str) -> Option<(i32, i32)> {
    let (_, rest) = word.split_once('+')?;
    let (x, y) = rest.split_once('+')?;
    Some((x.parse().ok()?, y.parse().ok()?))
}

/// One output while it's being read.
struct Reading {
    connector: String,
    connected: bool,
    position: Option<(i32, i32)>,
    rotation: Rotation,
    primary: bool,
    edid_hex: Option<String>,
    modes: Vec<OutputMode>,
    current: Option<usize>,
    /// A mode line waiting for its h:/v: lines: (id, MHz, flags, current).
    pending: Option<(String, f64, Vec<String>, bool)>,
    htotal: Option<(u32, u32)>,
}

impl Reading {
    /// The `v:` line completes a mode.
    fn finish_mode(&mut self, height: u32, vtotal: u32, vclock: Option<f64>) {
        let (Some((id, mhz, flags, current)), Some((width, htotal))) = (self.pending.take(), self.htotal.take()) else {
            return;
        };
        let mut hz = if htotal > 0 && vtotal > 0 {
            mhz * 1e6 / (f64::from(htotal) * f64::from(vtotal))
        } else {
            vclock.unwrap_or(0.0)
        };
        if flags.iter().any(|f| f == "Interlace") {
            hz *= 2.0;
        }
        if flags.iter().any(|f| f == "DoubleScan") {
            hz /= 2.0;
        }
        if current {
            self.current = Some(self.modes.len());
        }
        self.modes.push(OutputMode {
            id,
            width,
            height,
            refresh: (hz * 1000.0).round() / 1000.0,
            preferred: flags.iter().any(|f| f == "+preferred"),
            scales: Vec::new(),
            preferred_scale: None,
        });
    }

    fn into_output(self) -> Option<Output> {
        if !self.connected {
            return None;
        }
        let edid = self.edid_hex.as_deref().and_then(Edid::from_hex);
        let active = self.position.zip(self.current).map(|((x, y), mode)| Active {
            mode,
            x,
            y,
            rotation: self.rotation,
            scale: 1.0,
            primary: self.primary,
        });
        Some(Output {
            id: monitor_id(&self.connector, edid.as_ref(), None, ["", "", ""]),
            connector: self.connector,
            modes: self.modes,
            active,
        })
    }
}

/// The number after `key` among whitespace-separated words.
fn after<T: std::str::FromStr>(words: &[&str], key: &str) -> Option<T> {
    let i = words.iter().position(|w| *w == key)?;
    words.get(i + 1)?.parse().ok()
}

fn parse(text: &str) -> Result<Snapshot> {
    let mut outputs = Vec::new();
    let mut cur: Option<Reading> = None;
    let mut in_edid = false;
    for line in text.lines() {
        let words: Vec<&str> = line.split_whitespace().collect();
        let Some(first) = words.first().copied() else { continue };
        let indent = line.len() - line.trim_start().len();

        // "DP-1 connected primary 2560x1440+0+0 (0x4a) normal (normal left ...) 597mm x 336mm"
        if indent == 0 {
            in_edid = false;
            outputs.extend(cur.take().and_then(Reading::into_output));
            if words.get(1).is_some_and(|w| *w == "connected" || *w == "disconnected") {
                // Everything before the "(normal left inverted ...)" list.
                let head: Vec<&str> = words
                    .iter()
                    .copied()
                    .take_while(|w| !w.starts_with("(normal"))
                    .collect();
                cur = Some(Reading {
                    connector: first.to_string(),
                    connected: words[1] == "connected",
                    position: head.iter().find_map(|w| geometry(w)),
                    rotation: head.iter().find_map(|w| rotation(w)).unwrap_or_default(),
                    primary: head.contains(&"primary"),
                    edid_hex: None,
                    modes: Vec::new(),
                    current: None,
                    pending: None,
                    htotal: None,
                });
            }
            continue;
        }
        let Some(r) = cur.as_mut() else { continue };

        if first == "EDID:" {
            in_edid = true;
            r.edid_hex = Some(String::new());
            continue;
        }
        if in_edid {
            if words.len() == 1 && first.bytes().all(|b| b.is_ascii_hexdigit()) {
                r.edid_hex.get_or_insert_default().push_str(first);
                continue;
            }
            in_edid = false;
        }
        match first {
            "h:" => r.htotal = after(&words, "width").zip(after(&words, "total")),
            "v:" => {
                let clock = words
                    .iter()
                    .rev()
                    .find_map(|w| w.strip_suffix("Hz").and_then(|n| n.parse().ok()));
                if let (Some(h), Some(t)) = (after(&words, "height"), after(&words, "total")) {
                    r.finish_mode(h, t, clock);
                }
            }
            // "  2560x1440 (0x4a) 241.500MHz +HSync -VSync *current +preferred"
            _ if indent <= 2
                && words.get(1).is_some_and(|w| w.starts_with("(0x"))
                && words.get(2).is_some_and(|w| w.ends_with("MHz")) =>
            {
                let id = words[1].trim_matches(|c| c == '(' || c == ')').to_string();
                let mhz = words[2].trim_end_matches("MHz").parse().unwrap_or(0.0);
                let flags: Vec<String> = words[3..].iter().map(|w| w.to_string()).collect();
                let current = flags.iter().any(|f| f == "*current");
                r.pending = Some((id, mhz, flags, current));
                r.htotal = None;
            }
            _ => {}
        }
    }
    outputs.extend(cur.and_then(Reading::into_output));
    if outputs.is_empty() && !text.contains("Screen ") {
        return Err(Error::Os {
            call: PROGRAM,
            code: None,
            message: "unexpected output from xrandr --verbose".into(),
        });
    }
    Ok(Snapshot {
        outputs,
        coords: Coords::Physical,
        has_primary: true,
        serial: 0,
    })
}

fn args(settings: &[Setting]) -> Vec<String> {
    let mut out = Vec::new();
    for s in settings {
        out.extend(["--output".to_string(), s.connector.clone()]);
        match &s.on {
            None => out.push("--off".into()),
            Some(on) => {
                out.extend([
                    "--mode".into(),
                    on.mode.id.clone(),
                    "--pos".into(),
                    format!("{}x{}", on.x, on.y),
                    "--rotate".into(),
                    rotation_arg(on.rotation).into(),
                ]);
                if on.primary {
                    out.push("--primary".into());
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::linux::command::tests::Fake;
    use crate::linux::edid::tests::sample;
    use crate::linux::outputs::tests::simulate;
    use pt_core::plan::plan;
    use pt_core::{MonitorId, Position, current};
    use std::sync::Arc;

    fn edid_lines(name: &str) -> String {
        sample(name, None)
            .chunks(16)
            .map(|row| format!("\t\t{}\n", row.iter().map(|b| format!("{b:02x}")).collect::<String>()))
            .collect()
    }

    /// `xrandr --verbose` (trimmed): a laptop below-left of a monitor
    /// that's turned left (portrait), an HDMI TV that's connected but off,
    /// and an empty port.
    fn verbose() -> String {
        format!(
            "Screen 0: minimum 320 x 200, current 3360 x 2560, maximum 16384 x 16384
eDP-1 connected primary 1920x1080+0+1480 (0x47) normal (normal left inverted right x axis y axis) 344mm x 194mm
\tIdentifier: 0x42
\tTimestamp:  41622
\tSubpixel:   unknown
\tGamma:      1.0:1.0:1.0
\tBrightness: 1.0
\tClones:
\tCRTC:       0
\tCRTCs:      0 1 2 3
\tTransform:  1.000000 0.000000 0.000000
\t            0.000000 1.000000 0.000000
\t            0.000000 0.000000 1.000000
\t           filter:
\tEDID:
{laptop}\tBACKLIGHT: 9841
\t\trange: (0, 24242)
  1920x1080 (0x47) 138.700MHz +HSync -VSync *current +preferred
        h: width  1920 start 1968 end 2000 total 2080 skew    0 clock  66.68KHz
        v: height 1080 start 1083 end 1088 total 1111           clock  60.02Hz
  1280x720 (0x48) 74.250MHz +HSync +VSync
        h: width  1280 start 1390 end 1430 total 1650 skew    0 clock  45.00KHz
        v: height  720 start  725 end  730 total  750           clock  60.00Hz
DP-1 connected 1440x2560+1920+0 (0x4a) left (normal left inverted right x axis y axis) 597mm x 336mm
\tIdentifier: 0x43
\tEDID:
{dell}  2560x1440 (0x4a) 241.500MHz +HSync -VSync *current +preferred
        h: width  2560 start 2608 end 2640 total 2720 skew    0 clock  88.79KHz
        v: height 1440 start 1443 end 1448 total 1481           clock  59.95Hz
  1920x1080 (0x47) 148.500MHz +HSync +VSync
        h: width  1920 start 2008 end 2052 total 2200 skew    0 clock  67.50KHz
        v: height 1080 start 1084 end 1089 total 1125           clock  60.00Hz
HDMI-1 connected (normal left inverted right x axis y axis)
\tIdentifier: 0x44
  3840x2160 (0x4b) 594.000MHz +HSync +VSync +preferred
        h: width  3840 start 4016 end 4104 total 4400 skew    0 clock 135.00KHz
        v: height 2160 start 2168 end 2178 total 2250           clock  60.00Hz
  1920x1080i (0x4c) 74.250MHz +HSync +VSync Interlace
        h: width  1920 start 2008 end 2052 total 2200 skew    0 clock  33.75KHz
        v: height 1080 start 1084 end 1094 total 1125           clock  30.00Hz
DP-2 disconnected (normal left inverted right x axis y axis)
\tIdentifier: 0x45
",
            laptop = edid_lines("LAPTOP"),
            dell = edid_lines("DELL U2720Q"),
        )
    }

    #[test]
    fn reads_xrandr_verbose() {
        let s = parse(&verbose()).unwrap();
        assert_eq!(s.outputs.len(), 3, "disconnected ports are skipped");
        let laptop = &s.outputs[0];
        assert_eq!(laptop.id.name, "LAPTOP");
        assert_eq!(laptop.modes.len(), 2);
        assert!(
            (laptop.modes[0].refresh - 60.020).abs() < 0.001,
            "{}",
            laptop.modes[0].refresh
        );
        assert_eq!(laptop.modes[0].id, "0x47");
        let a = laptop.active.as_ref().unwrap();
        assert_eq!((a.x, a.y, a.primary), (0, 1480, true));

        let dell = &s.outputs[1];
        assert_eq!(dell.id.name, "DELL U2720Q");
        assert_eq!(dell.id.device_path, "DP-1:DELA0B1:12345678");
        assert_eq!(dell.active.as_ref().unwrap().rotation, Rotation::Rotate90);

        let tv = &s.outputs[2];
        assert!(tv.active.is_none());
        assert_eq!(tv.id.name, "HDMI-1", "no EDID: the connector");
        assert!(tv.modes[0].preferred);
        assert!((tv.modes[1].refresh - 60.0).abs() < 0.01, "interlaced counts fields");

        let l = s.layout();
        assert_eq!(l.monitors[0].position, Position { x: 0, y: 0 });
        assert_eq!(l.monitors[1].position, Position { x: 1920, y: -1480 });
        let m = l.monitors[1].mode.unwrap();
        assert_eq!((m.width, m.height), (1440, 2560));
    }

    #[test]
    fn rejects_unrelated_output() {
        assert!(parse("Can't open display").is_err());
        assert!(parse("Screen 0: minimum 8 x 8\n").unwrap().outputs.is_empty());
    }

    #[test]
    fn applies_with_xrandr_arguments() {
        let fake = Arc::new(Fake::new(&verbose()));
        let driver = XrandrDriver::new(Box::new(fake.clone()));
        let s = driver.snapshot().unwrap();
        // "TV only" at 4K.
        let mut saved = s.layout();
        saved.monitors[0].enabled = false;
        saved.monitors[1].enabled = false;
        saved.monitors[2] = pt_core::MonitorState {
            enabled: true,
            primary: true,
            mode: Some(pt_core::DisplayMode {
                width: 3840,
                height: 2160,
                refresh: pt_core::RefreshRate::from_hz(60),
            }),
            position: Position::default(),
            ..saved.monitors[2].clone()
        };
        let ids: Vec<MonitorId> = saved.monitors.iter().map(|m| m.id.clone()).collect();
        let settings = s.configure(&plan(&saved, &ids).unwrap(), driver.name()).unwrap();
        driver.apply(&s, &settings, true).unwrap();
        driver.apply(&s, &settings, false).unwrap();
        let calls = fake.calls();
        assert_eq!(calls[1].1[0], "--dryrun");
        assert_eq!(
            calls[2].1,
            [
                "--output",
                "eDP-1",
                "--off",
                "--output",
                "DP-1",
                "--off",
                "--output",
                "HDMI-1",
                "--mode",
                "0x4b",
                "--pos",
                "0x0",
                "--rotate",
                "normal",
                "--primary",
            ]
        );
        assert!(current::matches(&saved, &simulate(&s, &settings).layout()));
    }

    #[test]
    fn rotated_monitors_keep_their_rotation() {
        let s = parse(&verbose()).unwrap();
        let l = s.layout();
        let ids: Vec<MonitorId> = l.monitors.iter().map(|m| m.id.clone()).collect();
        let a = args(&s.configure(&plan(&l, &ids).unwrap(), "test").unwrap());
        let dell = a.iter().position(|w| w == "DP-1").unwrap();
        assert_eq!(
            &a[dell..dell + 7],
            ["DP-1", "--mode", "0x4a", "--pos", "1920x0", "--rotate", "left"]
        );
    }
}
