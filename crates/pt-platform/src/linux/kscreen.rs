//! KDE Plasma: `kscreen-doctor`, the command-line side of KDE's display
//! settings. Reads with `kscreen-doctor --json`, applies with
//! `kscreen-doctor output.DP-1.mode.3 output.DP-1.position.0,0 ...`.

use pt_core::{Error, Result, Rotation};
use serde_json::Value;

use super::command::{Runner, number};
use super::edid::{self, Edid};
use super::outputs::{Active, Coords, Driver, Output, OutputMode, Setting, Snapshot, monitor_id};

pub const PROGRAM: &str = "kscreen-doctor";

pub struct KScreenDriver {
    runner: Box<dyn Runner>,
    coords: Coords,
}

impl KScreenDriver {
    /// `coords`: Logical on Wayland, Physical on X11.
    pub fn new(runner: Box<dyn Runner>, coords: Coords) -> Self {
        KScreenDriver { runner, coords }
    }
}

impl Driver for KScreenDriver {
    fn name(&self) -> &'static str {
        "kde-kscreen"
    }

    fn snapshot(&self) -> Result<Snapshot> {
        let json = self.runner.run(PROGRAM, &["--json".into()])?;
        parse(&json, self.coords, &edid::for_connector)
    }

    fn apply(&self, _: &Snapshot, settings: &[Setting], validate_only: bool) -> Result<()> {
        // kscreen-doctor has no dry run; the modes and outputs were already
        // checked against what it reported.
        if validate_only {
            return Ok(());
        }
        self.runner.run(PROGRAM, &args(settings)).map(drop)
    }
}

fn bad_json(message: impl Into<String>) -> Error {
    Error::Os {
        call: PROGRAM,
        code: None,
        message: message.into(),
    }
}

/// KScreen's rotation flags: 1 none, 2 left (90° counter-clockwise),
/// 4 inverted, 8 right.
fn rotation(flag: u64) -> Rotation {
    match flag {
        2 => Rotation::Rotate90,
        4 => Rotation::Rotate180,
        8 => Rotation::Rotate270,
        _ => Rotation::None,
    }
}

fn rotation_arg(r: Rotation) -> &'static str {
    match r {
        Rotation::None => "none",
        Rotation::Rotate90 => "left",
        Rotation::Rotate180 => "inverted",
        Rotation::Rotate270 => "right",
    }
}

/// Mode ids are strings in newer versions and numbers in older ones.
fn id_string(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn parse(json: &str, coords: Coords, lookup_edid: &dyn Fn(&str) -> Option<Edid>) -> Result<Snapshot> {
    let root: Value = serde_json::from_str(json).map_err(|e| bad_json(format!("unexpected output: {e}")))?;
    let list = root["outputs"]
        .as_array()
        .ok_or_else(|| bad_json("no \"outputs\" in its output"))?;
    let mut outputs = Vec::new();
    for o in list {
        if !o["connected"].as_bool().unwrap_or(false) {
            continue;
        }
        let connector = o["name"].as_str().ok_or_else(|| bad_json("an output has no name"))?;
        let preferred: Vec<String> = o["preferredModes"]
            .as_array()
            .map(|a| a.iter().filter_map(id_string).collect())
            .unwrap_or_default();
        let modes: Vec<OutputMode> = o["modes"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|m| {
                let id = id_string(&m["id"])?;
                Some(OutputMode {
                    preferred: preferred.contains(&id),
                    width: m["size"]["width"].as_u64()? as u32,
                    height: m["size"]["height"].as_u64()? as u32,
                    refresh: m["refreshRate"].as_f64()?,
                    scales: Vec::new(),
                    preferred_scale: None,
                    id,
                })
            })
            .collect();
        let current = id_string(&o["currentModeId"]).and_then(|id| modes.iter().position(|m| m.id == id));
        // Plasma 5.25+ ranks outputs ("priority" 1 = primary); older
        // versions have a "primary" flag.
        let primary = o["priority"].as_u64().map(|p| p == 1).unwrap_or(false) || o["primary"].as_bool() == Some(true);
        let active = current
            .filter(|_| o["enabled"].as_bool().unwrap_or(false))
            .map(|mode| Active {
                mode,
                x: o["pos"]["x"].as_i64().unwrap_or(0) as i32,
                y: o["pos"]["y"].as_i64().unwrap_or(0) as i32,
                rotation: rotation(o["rotation"].as_u64().unwrap_or(1)),
                scale: o["scale"].as_f64().unwrap_or(1.0),
                primary,
            });
        let edid = lookup_edid(connector);
        outputs.push(Output {
            id: monitor_id(connector, edid.as_ref(), None, ["", "", ""]),
            connector: connector.to_string(),
            modes,
            active,
        });
    }
    Ok(Snapshot {
        outputs,
        coords,
        has_primary: true,
        serial: 0,
    })
}

fn args(settings: &[Setting]) -> Vec<String> {
    let mut out = Vec::new();
    // Primary first (priority 1), then the rest in order.
    let mut priority = 2;
    for s in settings {
        let o = format!("output.{}", s.connector);
        match &s.on {
            None => out.push(format!("{o}.disable")),
            Some(on) => {
                let rank = if on.primary {
                    1
                } else {
                    priority += 1;
                    priority - 1
                };
                out.extend([
                    format!("{o}.enable"),
                    format!("{o}.mode.{}", on.mode.id),
                    format!("{o}.position.{},{}", on.x, on.y),
                    format!("{o}.scale.{}", number(on.scale)),
                    format!("{o}.rotation.{}", rotation_arg(on.rotation)),
                    format!("{o}.priority.{rank}"),
                ]);
            }
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
    use pt_core::{DisplayMode, MonitorId, Position, current};
    use std::sync::Arc;

    /// Trimmed `kscreen-doctor --json` from Plasma 6 on Wayland: a laptop
    /// at 125 %, a 4K monitor at 150 % to its right, an unplugged HDMI.
    const PLASMA: &str = r#"{
      "features": 6,
      "outputs": [
        {"connected": true, "enabled": true, "id": 1, "name": "eDP-1",
         "currentModeId": "1", "preferredModes": ["1"],
         "modes": [
           {"id": "1", "name": "1920x1080@60", "refreshRate": 60.00099945068359, "size": {"height": 1080, "width": 1920}},
           {"id": "2", "name": "1920x1080@48", "refreshRate": 47.99599838256836, "size": {"height": 1080, "width": 1920}}
         ],
         "pos": {"x": 0, "y": 576}, "priority": 1, "rotation": 1, "scale": 1.25,
         "size": {"height": 1080, "width": 1920}, "type": 7},
        {"connected": true, "enabled": true, "id": 2, "name": "DP-2",
         "currentModeId": "5", "preferredModes": ["5"],
         "modes": [
           {"id": "5", "name": "3840x2160@60", "refreshRate": 59.99700164794922, "size": {"height": 2160, "width": 3840}},
           {"id": "6", "name": "2560x1440@60", "refreshRate": 59.95100021362305, "size": {"height": 1440, "width": 2560}}
         ],
         "pos": {"x": 1536, "y": 0}, "priority": 2, "rotation": 1, "scale": 1.5,
         "size": {"height": 2160, "width": 3840}, "type": 14},
        {"connected": false, "enabled": false, "id": 3, "name": "HDMI-A-1", "modes": [], "pos": {"x": 0, "y": 0}}
      ],
      "screen": {"id": 0, "maxActiveOutputsCount": 3}
    }"#;

    fn no_edid(_: &str) -> Option<Edid> {
        None
    }

    #[test]
    fn reads_plasma_outputs() {
        let s = parse(PLASMA, Coords::Logical, &no_edid).unwrap();
        assert_eq!(s.outputs.len(), 2, "unplugged outputs are skipped");
        let l = s.layout();
        let laptop = &l.monitors[0];
        let big = &l.monitors[1];
        assert!(laptop.primary);
        assert_eq!(laptop.position, Position { x: 0, y: 0 });
        // 1536 logical units = the laptop's 1920 px at 125 %; bottoms lined
        // up (576 + 864 = 1440 = 2160 / 1.5).
        assert_eq!(big.position, Position { x: 1920, y: -1080 });
        assert_eq!(big.mode.unwrap().to_string(), "3840x2160 @ 60 Hz");
        assert!(s.outputs[1].modes[0].preferred);
    }

    #[test]
    fn older_versions_use_numbers_and_a_primary_flag() {
        let json = r#"{"outputs": [{"connected": true, "enabled": true, "name": "HDMI-1",
            "currentModeId": 3, "modes": [{"id": 3, "refreshRate": 60, "size": {"width": 1920, "height": 1080}}],
            "pos": {"x": 0, "y": 0}, "primary": true, "rotation": 8, "scale": 1}]}"#;
        let s = parse(json, Coords::Physical, &no_edid).unwrap();
        let a = s.outputs[0].active.as_ref().unwrap();
        assert!(a.primary);
        assert_eq!(a.rotation, Rotation::Rotate270);
        assert_eq!(s.outputs[0].modes[0].id, "3");
    }

    #[test]
    fn bad_output_is_an_error() {
        assert!(parse("not json", Coords::Logical, &no_edid).is_err());
        assert!(parse("{}", Coords::Logical, &no_edid).is_err());
    }

    #[test]
    fn applies_with_kscreen_doctor_arguments() {
        let fake = Arc::new(Fake::new(PLASMA));
        let driver = KScreenDriver {
            runner: Box::new(fake.clone()),
            coords: Coords::Logical,
        };
        let s = driver.snapshot().unwrap();
        // Profile: only the 4K monitor, at 1440p.
        let mut saved = s.layout();
        saved.monitors[0].enabled = false;
        saved.monitors[1].mode = Some(DisplayMode {
            width: 2560,
            height: 1440,
            refresh: pt_core::RefreshRate::from_hz(60),
        });
        let ids: Vec<MonitorId> = saved.monitors.iter().map(|m| m.id.clone()).collect();
        let settings = s.configure(&plan(&saved, &ids).unwrap(), driver.name()).unwrap();

        driver.apply(&s, &settings, true).unwrap();
        assert_eq!(fake.calls().len(), 1, "validating runs nothing");
        driver.apply(&s, &settings, false).unwrap();
        let (program, args) = fake.calls().pop().unwrap();
        assert_eq!(program, "kscreen-doctor");
        assert_eq!(
            args,
            [
                "output.eDP-1.disable",
                "output.DP-2.enable",
                "output.DP-2.mode.6",
                "output.DP-2.position.0,0",
                "output.DP-2.scale.1.5",
                "output.DP-2.rotation.none",
                "output.DP-2.priority.1",
            ]
        );
        assert!(current::matches(&saved, &simulate(&s, &settings).layout()));
    }

    #[test]
    fn secondary_outputs_get_increasing_priorities() {
        let s = parse(PLASMA, Coords::Logical, &no_edid).unwrap();
        let a = args(&s.extend_all());
        assert!(a.contains(&"output.eDP-1.priority.1".to_string()), "{a:?}");
        assert!(a.contains(&"output.DP-2.priority.2".to_string()), "{a:?}");
        assert!(a.contains(&"output.DP-2.position.1536,0".to_string()), "{a:?}");
    }
}
