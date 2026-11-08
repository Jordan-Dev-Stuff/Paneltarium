//! Shared fixtures for tests in this crate and downstream crates.

use crate::model::{DisplayLayout, DisplayMode, MonitorId, MonitorState, Position, RefreshRate, Rotation};

/// A monitor id whose device path, name and model are derived from `tag`.
pub fn id(tag: &str) -> MonitorId {
    MonitorId {
        device_path: tag.to_string(),
        name: format!("Monitor {tag}"),
        edid_manufacturer: Some(0x10AC),
        edid_product: Some(tag.bytes().fold(0u16, |a, b| a.wrapping_mul(31).wrapping_add(b as u16))),
    }
}

/// LEFT: primary 3840x2160@60 at (0,0); RIGHT: 2560x1440@144 at (3840,0); TV: off.
pub fn sample_layout() -> DisplayLayout {
    DisplayLayout {
        monitors: vec![
            MonitorState {
                id: id("LEFT"),
                enabled: true,
                primary: true,
                mode: Some(DisplayMode {
                    width: 3840,
                    height: 2160,
                    refresh: RefreshRate::from_hz(60),
                }),
                position: Position { x: 0, y: 0 },
                rotation: Rotation::None,
            },
            MonitorState {
                id: id("RIGHT"),
                enabled: true,
                primary: false,
                mode: Some(DisplayMode {
                    width: 2560,
                    height: 1440,
                    refresh: RefreshRate::from_hz(144),
                }),
                position: Position { x: 3840, y: 0 },
                rotation: Rotation::None,
            },
            MonitorState::disabled(id("TV")),
        ],
    }
}

/// A monitor that is on: `w`x`h` @ `hz` at (`x`, `y`). Primary if at (0, 0).
pub fn on(tag: &str, w: u32, h: u32, hz: u32, x: i32, y: i32) -> MonitorState {
    MonitorState {
        id: id(tag),
        enabled: true,
        primary: x == 0 && y == 0,
        mode: Some(DisplayMode {
            width: w,
            height: h,
            refresh: RefreshRate::from_hz(hz),
        }),
        position: Position { x, y },
        rotation: Rotation::None,
    }
}

/// A layout from monitor states.
pub fn layout(monitors: Vec<MonitorState>) -> DisplayLayout {
    DisplayLayout { monitors }
}

/// Two physically identical monitors: same EDID model and name, different
/// device paths (ports).
pub fn twin(tag: &str, port: &str) -> MonitorId {
    MonitorId {
        device_path: port.to_string(),
        ..id(tag)
    }
}
