//! wlroots-based compositors (Sway, Hyprland, river, labwc, Wayfire, ...):
//! `wlr-randr`, which speaks the `wlr-output-management` protocol they all
//! implement. Reads with `wlr-randr --json`, applies with
//! `wlr-randr --output DP-1 --mode 2560x1440@59.951Hz --pos 0,0 ...`.
//!
//! These compositors have no "primary monitor"; the one at the top left
//! counts as primary, like on Windows.

use pt_core::{Error, Result, Rotation};
use serde_json::Value;

use super::command::{Runner, number};
use super::edid::{self, Edid};
use super::outputs::{Active, Coords, Driver, Output, OutputMode, Setting, Snapshot, monitor_id};

pub const PROGRAM: &str = "wlr-randr";

pub struct WlrootsDriver {
    runner: Box<dyn Runner>,
}

impl WlrootsDriver {
    pub fn new(runner: Box<dyn Runner>) -> Self {
        WlrootsDriver { runner }
    }
}

impl Driver for WlrootsDriver {
    fn name(&self) -> &'static str {
        "wlroots"
    }

    fn snapshot(&self) -> Result<Snapshot> {
        let json = self.runner.run(PROGRAM, &["--json".into()])?;
        parse(&json, &edid::for_connector)
    }

    fn apply(&self, _: &Snapshot, settings: &[Setting], validate_only: bool) -> Result<()> {
        let mut a = args(settings);
        if validate_only {
            a.insert(0, "--dryrun".into());
        }
        self.runner.run(PROGRAM, &a).map(drop)
    }
}

fn bad_json(message: impl Into<String>) -> Error {
    Error::Os {
        call: PROGRAM,
        code: None,
        message: message.into(),
    }
}

/// `wl_output` transforms: "90" = turned 90° counter-clockwise; the
/// "flipped" ones keep their rotation.
fn rotation(transform: &str) -> Rotation {
    match transform.trim_start_matches("flipped").trim_start_matches('-') {
        "90" => Rotation::Rotate90,
        "180" => Rotation::Rotate180,
        "270" => Rotation::Rotate270,
        _ => Rotation::None,
    }
}

fn transform_arg(r: Rotation) -> &'static str {
    match r {
        Rotation::None => "normal",
        Rotation::Rotate90 => "90",
        Rotation::Rotate180 => "180",
        Rotation::Rotate270 => "270",
    }
}

/// How `wlr-randr --mode` wants it: refresh in Hz with three decimals,
/// which is exactly the compositor's millihertz value.
fn mode_id(w: u32, h: u32, hz: f64) -> String {
    format!("{w}x{h}@{hz:.3}Hz")
}

fn parse(json: &str, lookup_edid: &dyn Fn(&str) -> Option<Edid>) -> Result<Snapshot> {
    let list: Value = serde_json::from_str(json).map_err(|e| bad_json(format!("unexpected output: {e}")))?;
    let list = list.as_array().ok_or_else(|| bad_json("expected a list of outputs"))?;
    let mut outputs = Vec::new();
    for o in list {
        let connector = o["name"].as_str().ok_or_else(|| bad_json("an output has no name"))?;
        let mut current = None;
        let modes: Vec<OutputMode> = o["modes"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|m| {
                let (w, h, hz) = (
                    m["width"].as_u64()? as u32,
                    m["height"].as_u64()? as u32,
                    m["refresh"].as_f64()?,
                );
                Some((
                    m["current"].as_bool() == Some(true),
                    OutputMode {
                        id: mode_id(w, h, hz),
                        width: w,
                        height: h,
                        refresh: (hz * 1000.0).round() / 1000.0,
                        preferred: m["preferred"].as_bool() == Some(true),
                        scales: Vec::new(),
                        preferred_scale: None,
                    },
                ))
            })
            .enumerate()
            .map(|(i, (is_current, mode))| {
                if is_current {
                    current = Some(i);
                }
                mode
            })
            .collect();
        let active = current
            .filter(|_| o["enabled"].as_bool().unwrap_or(false))
            .map(|mode| Active {
                mode,
                x: o["position"]["x"].as_i64().unwrap_or(0) as i32,
                y: o["position"]["y"].as_i64().unwrap_or(0) as i32,
                rotation: rotation(o["transform"].as_str().unwrap_or("normal")),
                scale: o["scale"].as_f64().unwrap_or(1.0),
                primary: false,
            });
        let text = |key: &str| o[key].as_str().unwrap_or("").to_string();
        let (make, model, serial) = (text("make"), text("model"), text("serial"));
        let friendly = format!("{make} {model}");
        outputs.push(Output {
            id: monitor_id(
                connector,
                lookup_edid(connector).as_ref(),
                Some(friendly.trim()).filter(|s| !s.is_empty() && !s.contains("Unknown")),
                [&make, &model, &serial],
            ),
            connector: connector.to_string(),
            modes,
            active,
        });
    }
    Ok(Snapshot {
        outputs,
        coords: Coords::Logical,
        has_primary: false,
        serial: 0,
    })
}

fn args(settings: &[Setting]) -> Vec<String> {
    let mut out = Vec::new();
    for s in settings {
        out.extend(["--output".to_string(), s.connector.clone()]);
        match &s.on {
            None => out.push("--off".into()),
            Some(on) => out.extend([
                "--on".into(),
                "--mode".into(),
                on.mode.id.clone(),
                "--pos".into(),
                format!("{},{}", on.x, on.y),
                "--transform".into(),
                transform_arg(on.rotation).into(),
                "--scale".into(),
                number(on.scale),
            ]),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::linux::command::tests::Fake;
    use crate::linux::outputs::tests::simulate;
    use pt_core::plan::plan;
    use pt_core::{MonitorId, Position, current};
    use std::sync::Arc;

    /// `wlr-randr --json` on Sway: a laptop, and a monitor turned on its
    /// side (portrait) above-left of it.
    const SWAY: &str = r#"[
      {"name": "eDP-1", "description": "BOE 0x07C9 (eDP-1)", "make": "BOE", "model": "0x07C9", "serial": null,
       "physical_size": {"width": 344, "height": 194}, "enabled": true,
       "modes": [
         {"width": 1920, "height": 1080, "refresh": 59.999001, "preferred": true, "current": true},
         {"width": 1920, "height": 1080, "refresh": 47.997002, "preferred": false, "current": false}
       ],
       "position": {"x": 1440, "y": 1480}, "transform": "normal", "scale": 1.000000, "adaptive_sync": false},
      {"name": "DP-3", "description": "Dell Inc. DELL U2720Q ABC123 (DP-3)", "make": "Dell Inc.", "model": "DELL U2720Q", "serial": "ABC123",
       "enabled": true,
       "modes": [
         {"width": 2560, "height": 1440, "refresh": 59.951000, "preferred": true, "current": true}
       ],
       "position": {"x": 0, "y": 0}, "transform": "90", "scale": 1.000000, "adaptive_sync": false},
      {"name": "HDMI-A-1", "description": "Unknown", "make": "Unknown", "model": "Unknown", "serial": "", "enabled": false,
       "modes": [{"width": 1920, "height": 1080, "refresh": 60.0, "preferred": true, "current": false}],
       "position": {"x": 0, "y": 0}, "transform": "normal", "scale": 1.0}
    ]"#;

    fn no_edid(_: &str) -> Option<Edid> {
        None
    }

    #[test]
    fn reads_wlr_randr() {
        let s = parse(SWAY, &no_edid).unwrap();
        assert_eq!(s.outputs[1].id.name, "Dell Inc. DELL U2720Q");
        assert_eq!(s.outputs[2].id.name, "HDMI-A-1", "\"Unknown\" isn't a name");
        assert_eq!(s.outputs[0].modes[0].id, "1920x1080@59.999Hz");
        let l = s.layout();
        // No primary in wlroots: the top-left monitor (the Dell) is it.
        let dell = &l.monitors[1];
        assert!(dell.primary);
        assert_eq!(dell.rotation, Rotation::Rotate90);
        assert_eq!((dell.mode.unwrap().width, dell.mode.unwrap().height), (1440, 2560));
        assert_eq!(l.monitors[0].position, Position { x: 1440, y: 1480 });
        assert!(!l.monitors[2].enabled);
    }

    #[test]
    fn transforms() {
        assert_eq!(rotation("normal"), Rotation::None);
        assert_eq!(rotation("270"), Rotation::Rotate270);
        assert_eq!(rotation("flipped-90"), Rotation::Rotate90);
        assert_eq!(rotation("flipped"), Rotation::None);
        for r in [
            Rotation::None,
            Rotation::Rotate90,
            Rotation::Rotate180,
            Rotation::Rotate270,
        ] {
            assert_eq!(rotation(transform_arg(r)), r);
        }
    }

    #[test]
    fn applies_with_wlr_randr_arguments() {
        let fake = Arc::new(Fake::new(SWAY));
        let driver = WlrootsDriver::new(Box::new(fake.clone()));
        let s = driver.snapshot().unwrap();
        // Turn the TV on to the right of the laptop.
        let mut saved = s.layout();
        saved.monitors[2] = pt_core::MonitorState {
            enabled: true,
            primary: false,
            mode: Some(pt_core::DisplayMode {
                width: 1920,
                height: 1080,
                refresh: pt_core::RefreshRate::from_hz(60),
            }),
            position: Position { x: 3360, y: 1480 },
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
                "--on",
                "--mode",
                "1920x1080@59.999Hz",
                "--pos",
                "1440,1480",
                "--transform",
                "normal",
                "--scale",
                "1",
                "--output",
                "DP-3",
                "--on",
                "--mode",
                "2560x1440@59.951Hz",
                "--pos",
                "0,0",
                "--transform",
                "90",
                "--scale",
                "1",
                "--output",
                "HDMI-A-1",
                "--on",
                "--mode",
                "1920x1080@60.000Hz",
                "--pos",
                "3360,1480",
                "--transform",
                "normal",
                "--scale",
                "1",
            ]
        );
        assert!(current::matches(&saved, &simulate(&s, &settings).layout()));
    }

    #[test]
    fn bad_output_is_an_error() {
        assert!(parse("{}", &no_edid).is_err());
        assert!(parse("[{}]", &no_edid).is_err());
    }
}
