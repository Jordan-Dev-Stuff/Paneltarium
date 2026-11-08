use std::path::PathBuf;

/// Every error the core and the platform backends can report.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("profile '{0}' was not found")]
    ProfileNotFound(String),

    #[error("profile '{0}' already exists (use overwrite to replace it)")]
    ProfileExists(String),

    #[error("invalid profile name '{0}': use letters, digits, spaces, '-' or '_' (max 64 chars)")]
    InvalidProfileName(String),

    #[error("invalid hotkey '{input}': {reason}")]
    InvalidHotkey { input: String, reason: String },

    #[error("invalid controller combination '{input}': {reason}")]
    InvalidGamepadChord { input: String, reason: String },

    #[error("hotkey {hotkey} is already used by {used_by}")]
    HotkeyConflict { hotkey: String, used_by: String },

    #[error("none of the monitors this profile turns on are connected")]
    NoEnabledMonitors,

    #[error("monitor '{0}' was not found")]
    MonitorNotFound(String),

    #[error("this operation is not supported on this platform yet")]
    Unsupported,

    /// An OS call failed. `code` is the raw OS error code when there is one.
    #[error("{call} failed{}: {message}", code.map(|c| format!(" (code {c})")).unwrap_or_default())]
    Os {
        call: &'static str,
        code: Option<i64>,
        message: String,
    },

    #[error("could not read or write {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("could not parse {path}: {message}")]
    Parse { path: PathBuf, message: String },

    #[error("could not determine the user's config directory")]
    NoConfigDir,
}

impl Error {
    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Error::Io {
            path: path.into(),
            source,
        }
    }
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
