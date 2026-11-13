//! User-facing wording that depends on the OS, so the tray, manager and CLI
//! don't talk about Windows on Linux (or the other way round).

/// The app id desktops know Paneltarium by (Linux: the `.desktop` file name
/// and the id the shortcut portal asks the user about).
pub const APP_ID: &str = "io.github.jordan_dev_stuff.Paneltarium";

/// The profile manager's app id (its window's Wayland app id and `.desktop`
/// file, so the desktop shows the right name and icon).
pub const MANAGER_APP_ID: &str = "io.github.jordan_dev_stuff.Paneltarium.Manager";

/// Label of the "start at sign-in" switch.
pub const START_AT_SIGN_IN: &str = if cfg!(windows) {
    "Start with Windows"
} else {
    "Start when I sign in"
};

/// Where the OS lets you switch sign-in apps off.
pub const STARTUP_SETTINGS: &str = if cfg!(windows) {
    "Task Manager"
} else {
    "your desktop's startup apps settings"
};

/// Who arranged the monitors when Paneltarium doesn't touch them.
pub const SYSTEM: &str = if cfg!(windows) { "Windows" } else { "your desktop" };

/// Where you arrange monitors by hand.
pub const DISPLAY_SETTINGS: &str = if cfg!(windows) {
    "Windows' display settings"
} else {
    "your desktop's display settings"
};
