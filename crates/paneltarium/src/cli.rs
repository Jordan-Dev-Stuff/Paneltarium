use clap::{Parser, Subcommand, ValueEnum};

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum AutostartAction {
    On,
    Off,
    Status,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum OnOff {
    On,
    Off,
}

/// Save monitor setups as profiles and switch between them.
#[derive(Debug, Parser)]
#[command(name = "paneltarium", version, about, arg_required_else_help = true)]
pub struct Cli {
    /// Print machine-readable JSON instead of tables (where supported).
    #[arg(long, global = true)]
    pub json: bool,

    /// Print the raw display configuration sent to the OS (for bug reports).
    #[arg(long, global = true)]
    pub debug: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Show every connected monitor and its current state.
    #[command(visible_alias = "list")]
    Monitors,

    /// List the modes a monitor supports (number from `monitors`, or part of its name).
    Modes { monitor: String },

    /// List saved profiles.
    Profiles,

    /// Show what a profile contains.
    Show { profile: String },

    /// Save the current monitor setup as a profile.
    Save {
        name: String,
        /// Hotkey that applies this profile, e.g. "Ctrl+Alt+1".
        #[arg(long)]
        hotkey: Option<String>,
        /// Replace an existing profile with the same name.
        #[arg(long, short = 'f')]
        overwrite: bool,
    },

    /// Apply a saved profile.
    Apply {
        profile: String,
        /// Ask Windows whether the profile would work, without changing anything.
        #[arg(long)]
        dry_run: bool,
    },

    /// Show the same picture on every monitor (falls back to extend if impossible).
    Duplicate,

    /// Turn every monitor on as one extended desktop.
    Extend,

    /// Delete a saved profile.
    Delete { profile: String },

    /// Set or clear a profile's hotkey.
    Hotkey {
        profile: String,
        /// e.g. "Ctrl+Alt+1". Omit together with --clear to remove the hotkey.
        hotkey: Option<String>,
        #[arg(long, conflicts_with = "hotkey")]
        clear: bool,
    },

    /// Show or change app settings. Use "none" to clear a hotkey.
    Settings {
        #[arg(long, value_name = "HOTKEY")]
        duplicate_hotkey: Option<String>,
        #[arg(long, value_name = "HOTKEY")]
        extend_hotkey: Option<String>,
        #[arg(long, value_name = "HOTKEY")]
        overlay_hotkey: Option<String>,
        /// Controller buttons that open the overlay, e.g. "A+LB+RB+LT+RT" (or "none").
        #[arg(long, value_name = "BUTTONS")]
        gamepad_overlay: Option<String>,
        /// Listen to game controllers at all ("off" keeps the combination for later).
        #[arg(long, value_name = "on|off")]
        gamepad: Option<OnOff>,
        /// At sign-in: "none" (leave monitors alone), "last" (last used
        /// profile) or a profile name.
        #[arg(long, value_name = "none|last|PROFILE")]
        startup: Option<String>,
        /// After the overlay hotkey: "hold" = stays open while you hold its
        /// modifier keys (e.g. Ctrl+Alt); "toggle" = stays open until you
        /// choose or press Esc.
        #[arg(long, value_name = "hold|toggle")]
        overlay_mode: Option<String>,
        /// Accent colour of the overlay and profile manager, e.g. "#7C3AED"
        /// (or "default").
        #[arg(long, value_name = "#RRGGBB")]
        accent: Option<String>,
    },

    /// Start Paneltarium when you sign in to Windows: on, off or status.
    Autostart {
        #[arg(value_enum, default_value = "status")]
        action: AutostartAction,
    },

    /// List connected game controllers. With --watch, also show button
    /// presses live (Ctrl+C to stop), to check what Paneltarium receives.
    Gamepads {
        #[arg(long)]
        watch: bool,
        /// Stop watching after this many seconds.
        #[arg(long, value_name = "SECONDS")]
        seconds: Option<u64>,
    },

    /// Show where profiles and settings are stored.
    Paths,
}
