//! # pt-core
//!
//! Everything in Paneltarium that does not depend on the operating system:
//!
//! - [`model`]: monitors, modes, layouts
//! - [`hotkey`]: parsing/printing shortcuts such as `Ctrl+Alt+1`
//! - [`gamepad`]: controller buttons and button combinations (`A+LB+RB+LT+RT`)
//! - [`profile`]: saved profiles on disk
//! - [`settings`]: app settings (rescue hotkeys, ...)
//! - [`startup`]: the sign-in profile setting and remembered state
//! - [`slots`]: overlay numbers 1–9 (shared by overlay and profile manager)
//! - [`theme`]: the configurable accent colour and colours derived from it
//! - [`current`]: which profile (or duplicate) is on screen right now
//! - [`plan`]: matching a saved profile to the monitors connected right now
//! - [`backend`]: the traits each OS implements (see the `pt-platform` crate)
//! - [`service`]: the facade the CLI, tray and hotkeys all use
//! - [`mock`]: a fake backend for tests
//! - [`text`]: wording that differs between Windows and Linux

pub mod backend;
pub mod current;
pub mod error;
pub mod gamepad;
pub mod hotkey;
pub mod instance;
pub mod mock;
pub mod model;
pub mod paths;
pub mod plan;
pub mod profile;
pub mod service;
pub mod settings;
pub mod slots;
pub mod startup;
pub mod testing;
pub mod text;
pub mod theme;

pub use backend::{
    AUTOSTART_ARG, Autostart, AutostartStatus, DisplayBackend, KeyboardState, Shortcut, ShortcutEvent, ShortcutService,
};
pub use error::{Error, Result};
pub use hotkey::{Hotkey, Key, Modifiers};
pub use model::{DisplayLayout, DisplayMode, MonitorId, MonitorState, Position, RefreshRate, Rotation, Topology};
pub use paths::AppPaths;
pub use plan::{ApplyPlan, PlannedMonitor};
pub use profile::{Profile, ProfileStore};
pub use service::{ApplyReport, MonitorService};
pub use settings::{OverlayMode, Settings};
pub use theme::{Color, Theme};
