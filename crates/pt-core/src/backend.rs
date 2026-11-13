//! The seam between OS-neutral code and OS-specific code.
//!
//! Each supported OS provides one implementation of these traits in the
//! `pt-platform` crate. Everything else (CLI, tray, hotkeys, UI) only ever
//! talks to the traits.

use std::path::Path;

use crate::error::Result;
use crate::hotkey::{Hotkey, Modifiers};
use crate::model::{DisplayLayout, DisplayMode, MonitorId, Topology};
use crate::plan::ApplyPlan;

pub trait DisplayBackend: Send + Sync {
    /// Short name for logs and `--version` style output, e.g. "windows-ccd".
    fn name(&self) -> &'static str;

    /// Every connected monitor, including ones that are currently switched off.
    fn current_layout(&self) -> Result<DisplayLayout>;

    /// Carry out a plan produced by [`crate::plan::plan`]. With
    /// `validate_only`, ask the OS whether it would work without changing anything.
    fn apply(&self, plan: &ApplyPlan, validate_only: bool) -> Result<()>;

    /// Switch to a built-in arrangement (duplicate / extend everything).
    fn set_topology(&self, topology: Topology) -> Result<()>;

    /// Modes a monitor supports. May require the monitor to be switched on.
    fn supported_modes(&self, monitor: &MonitorId) -> Result<Vec<DisplayMode>>;
}

/// Which modifier keys are physically held right now. Used by the overlay
/// to close when you let go of the trigger's modifiers.
pub trait KeyboardState: Send + Sync {
    /// `None` when this platform can't tell (the overlay then falls back to
    /// a timeout and Esc).
    fn modifiers_held(&self) -> Option<Modifiers>;
}

/// Command-line flag the OS passes when it starts the app at sign-in.
pub const AUTOSTART_ARG: &str = "--autostart";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutostartStatus {
    On,
    Off,
    /// Registered, but switched off in the OS's own startup settings
    /// (Windows: Task Manager → Startup apps). Enabling here turns it back on.
    DisabledInSystem,
}

/// "Start with Windows" (or the Linux equivalent).
pub trait Autostart: Send + Sync {
    fn status(&self) -> Result<AutostartStatus>;
    /// Start `exe --autostart` at sign-in.
    fn enable(&self, exe: &Path) -> Result<()>;
    fn disable(&self) -> Result<()>;
    /// Replace an entry left by the app's old name (Monitor Pilot): remove
    /// it and, if it was switched on, register `exe` instead. Returns
    /// whether anything was migrated.
    fn migrate_legacy(&self, _exe: &Path) -> Result<bool> {
        Ok(false)
    }
}

/// A global shortcut for the desktop to deliver (see [`ShortcutService`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shortcut {
    /// Stable id, reported back in [`ShortcutEvent`]s.
    pub id: String,
    /// Shown to the user by the desktop (e.g. in its shortcut settings).
    pub description: String,
    pub hotkey: Hotkey,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShortcutEvent {
    Pressed(String),
    Released(String),
    /// A new set of shortcuts is active: how many the desktop accepted.
    Bound(usize),
    /// The desktop refused or failed to bind them.
    Failed(String),
}

/// Global shortcuts owned by the desktop rather than grabbed by the app:
/// on Wayland, apps can't watch the keyboard, so they ask the desktop
/// (the XDG GlobalShortcuts portal) to tell them when a combination is
/// pressed. The desktop may ask the user to approve them.
pub trait ShortcutService: Send {
    /// Replace the bound shortcuts. Works in the background; the outcome
    /// arrives as a [`ShortcutEvent::Bound`] or [`ShortcutEvent::Failed`].
    fn bind(&self, shortcuts: Vec<Shortcut>);
}
