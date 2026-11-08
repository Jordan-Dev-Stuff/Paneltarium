//! Plain data types describing monitors and their configuration.
//! Nothing in here talks to the OS.

use std::fmt;

use serde::{Deserialize, Serialize};

/// How we recognise the same physical monitor again later.
///
/// `device_path` is the strongest key (on Windows it is the monitor's PnP
/// device interface path, which is stable across reboots for the same
/// monitor on the same port). The EDID ids and name are used as a fallback
/// when the monitor was moved to another port.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MonitorId {
    pub device_path: String,
    #[serde(default)]
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edid_manufacturer: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edid_product: Option<u16>,
}

impl MonitorId {
    /// True when both ids carry the same EDID manufacturer/product and name,
    /// i.e. it is probably the same model of monitor.
    pub fn same_model(&self, other: &MonitorId) -> bool {
        self.edid_manufacturer.is_some()
            && self.edid_manufacturer == other.edid_manufacturer
            && self.edid_product == other.edid_product
            && self.name == other.name
    }

    /// A short label for messages: the friendly name, or the device path.
    pub fn label(&self) -> &str {
        if self.name.is_empty() {
            &self.device_path
        } else {
            &self.name
        }
    }
}

/// Refresh rate as an exact rational number (e.g. 60000/1001 for 59.94 Hz).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RefreshRate {
    pub numerator: u32,
    pub denominator: u32,
}

impl RefreshRate {
    pub fn from_hz(hz: u32) -> Self {
        RefreshRate {
            numerator: hz,
            denominator: 1,
        }
    }

    pub fn hz(&self) -> f64 {
        if self.denominator == 0 {
            0.0
        } else {
            self.numerator as f64 / self.denominator as f64
        }
    }
}

impl fmt::Display for RefreshRate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let hz = self.hz();
        if (hz - hz.round()).abs() < 0.005 {
            write!(f, "{} Hz", hz.round() as u64)
        } else {
            write!(f, "{hz:.2} Hz")
        }
    }
}

/// Desktop resolution (in desktop orientation) plus refresh rate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DisplayMode {
    pub width: u32,
    pub height: u32,
    pub refresh: RefreshRate,
}

impl fmt::Display for DisplayMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}x{} @ {}", self.width, self.height, self.refresh)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct Position {
    pub x: i32,
    pub y: i32,
}

impl fmt::Display for Position {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "({}, {})", self.x, self.y)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Rotation {
    #[default]
    None,
    Rotate90,
    Rotate180,
    Rotate270,
}

/// The state of one connected monitor: on or off, and if on, how.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonitorState {
    pub id: MonitorId,
    pub enabled: bool,
    #[serde(default)]
    pub primary: bool,
    /// `None` when the monitor is off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<DisplayMode>,
    #[serde(default)]
    pub position: Position,
    #[serde(default)]
    pub rotation: Rotation,
}

impl MonitorState {
    pub fn disabled(id: MonitorId) -> Self {
        MonitorState {
            id,
            enabled: false,
            primary: false,
            mode: None,
            position: Position::default(),
            rotation: Rotation::None,
        }
    }
}

/// A full snapshot of every connected monitor.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct DisplayLayout {
    #[serde(default)]
    pub monitors: Vec<MonitorState>,
}

impl DisplayLayout {
    pub fn enabled(&self) -> impl Iterator<Item = &MonitorState> {
        self.monitors.iter().filter(|m| m.enabled)
    }

    /// One-line human summary, e.g. "DELL U2720Q 3840x2160 @ 60 Hz; LG off".
    pub fn summary(&self) -> String {
        self.monitors
            .iter()
            .map(|m| match (&m.mode, m.enabled) {
                (Some(mode), true) => format!("{} {}", m.id.label(), mode),
                _ => format!("{} off", m.id.label()),
            })
            .collect::<Vec<_>>()
            .join("; ")
    }

    /// Order monitors for display: enabled ones left-to-right, top-to-bottom,
    /// then disabled ones by name. Gives the CLI and UI a stable numbering.
    pub fn sort_for_display(&mut self) {
        self.monitors.sort_by(|a, b| {
            b.enabled
                .cmp(&a.enabled)
                .then(a.position.x.cmp(&b.position.x))
                .then(a.position.y.cmp(&b.position.y))
                .then(a.id.name.cmp(&b.id.name))
                .then(a.id.device_path.cmp(&b.id.device_path))
        });
    }
}

/// Built-in arrangements the OS can switch to without a saved profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Topology {
    /// Show the same picture on every monitor (the "I lost my screen" escape hatch).
    Clone,
    /// Turn every monitor on as one extended desktop.
    Extend,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refresh_formats_whole_and_fractional() {
        assert_eq!(RefreshRate::from_hz(60).to_string(), "60 Hz");
        let ntsc = RefreshRate {
            numerator: 60000,
            denominator: 1001,
        };
        assert_eq!(ntsc.to_string(), "59.94 Hz");
        let zero = RefreshRate {
            numerator: 0,
            denominator: 0,
        };
        assert_eq!(zero.hz(), 0.0);
    }

    #[test]
    fn same_model_requires_edid() {
        let a = MonitorId {
            device_path: "a".into(),
            name: "Dell".into(),
            edid_manufacturer: None,
            edid_product: None,
        };
        let b = MonitorId {
            device_path: "b".into(),
            ..a.clone()
        };
        assert!(!a.same_model(&b));
        let a = MonitorId {
            edid_manufacturer: Some(1),
            edid_product: Some(2),
            ..a
        };
        let b = MonitorId {
            edid_manufacturer: Some(1),
            edid_product: Some(2),
            ..b
        };
        assert!(a.same_model(&b));
    }
}
