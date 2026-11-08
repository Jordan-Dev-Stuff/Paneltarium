//! A fake [`DisplayBackend`] for tests and for trying the CLI on machines
//! without a real backend. It can keep its state in memory or in a TOML file
//! (so separate CLI processes see each other's changes).

use std::path::PathBuf;
use std::sync::Mutex;

use crate::backend::DisplayBackend;
use crate::error::{Error, Result};
use crate::model::{DisplayLayout, DisplayMode, MonitorId, Position, RefreshRate, Topology};
use crate::paths::write_atomic;
use crate::plan::{ApplyPlan, check_arrangement, expected_layout};

/// Environment variable that makes the app use a file-backed mock backend.
pub const MOCK_LAYOUT_ENV: &str = "PANELTARIUM_MOCK_LAYOUT";

pub struct MockDisplayBackend {
    state: Mutex<DisplayLayout>,
    file: Option<PathBuf>,
    /// Every call that changed state, for assertions in tests.
    pub calls: Mutex<Vec<String>>,
}

impl MockDisplayBackend {
    pub fn new(layout: DisplayLayout) -> Self {
        MockDisplayBackend {
            state: Mutex::new(layout),
            file: None,
            calls: Mutex::new(Vec::new()),
        }
    }

    /// Load the layout from `path`; every change is written back to it.
    pub fn from_file(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        let text = std::fs::read_to_string(&path).map_err(|e| Error::io(&path, e))?;
        let layout = toml::from_str(&text).map_err(|e| Error::Parse {
            path: path.clone(),
            message: e.to_string(),
        })?;
        Ok(MockDisplayBackend {
            state: Mutex::new(layout),
            file: Some(path),
            calls: Mutex::new(Vec::new()),
        })
    }

    pub fn layout(&self) -> DisplayLayout {
        self.state.lock().unwrap().clone()
    }

    fn store(&self, layout: DisplayLayout, call: String) -> Result<()> {
        if let Some(path) = &self.file {
            write_atomic(path, &toml::to_string_pretty(&layout).expect("layout serialises"))?;
        }
        *self.state.lock().unwrap() = layout;
        self.calls.lock().unwrap().push(call);
        Ok(())
    }
}

impl DisplayBackend for MockDisplayBackend {
    fn name(&self) -> &'static str {
        "mock"
    }

    fn current_layout(&self) -> Result<DisplayLayout> {
        // File-backed: re-read, so several processes/instances agree (like
        // the real OS, which every reader sees the same way).
        if let Some(path) = &self.file
            && let Ok(text) = std::fs::read_to_string(path)
            && let Ok(layout) = toml::from_str::<DisplayLayout>(&text)
        {
            *self.state.lock().unwrap() = layout;
        }
        Ok(self.layout())
    }

    fn apply(&self, plan: &ApplyPlan, validate_only: bool) -> Result<()> {
        let known: Vec<MonitorId> = self.layout().monitors.into_iter().map(|m| m.id).collect();
        if let Some(stranger) = plan.monitors.iter().find(|p| !known.contains(&p.target)) {
            return Err(Error::MonitorNotFound(stranger.target.label().to_string()));
        }
        // Reject what Windows would reject (overlaps, islands, no primary).
        check_arrangement(plan).map_err(|message| Error::Os {
            call: "SetDisplayConfig",
            code: Some(87),
            message,
        })?;
        if validate_only {
            return Ok(());
        }
        let on = plan.enabled().count();
        self.store(expected_layout(plan), format!("apply({on} on)"))
    }

    fn set_topology(&self, topology: Topology) -> Result<()> {
        let mut layout = self.layout();
        let mode = layout
            .enabled()
            .find(|m| m.primary)
            .or_else(|| layout.enabled().next())
            .and_then(|m| m.mode)
            .unwrap_or(DisplayMode {
                width: 1920,
                height: 1080,
                refresh: RefreshRate::from_hz(60),
            });
        let mut x = 0;
        for (i, m) in layout.monitors.iter_mut().enumerate() {
            m.enabled = true;
            m.primary = i == 0;
            let own = m.mode.unwrap_or(mode);
            match topology {
                Topology::Clone => {
                    m.mode = Some(mode);
                    m.position = Position::default();
                }
                Topology::Extend => {
                    m.mode = Some(own);
                    m.position = Position { x, y: 0 };
                    x += own.width as i32;
                }
            }
        }
        self.store(layout, format!("topology({topology:?})"))
    }

    fn supported_modes(&self, monitor: &MonitorId) -> Result<Vec<DisplayMode>> {
        let layout = self.layout();
        let m = layout
            .monitors
            .iter()
            .find(|m| &m.id == monitor)
            .ok_or_else(|| Error::MonitorNotFound(monitor.label().to_string()))?;
        let mut modes = vec![
            DisplayMode {
                width: 1920,
                height: 1080,
                refresh: RefreshRate::from_hz(60),
            },
            DisplayMode {
                width: 2560,
                height: 1440,
                refresh: RefreshRate::from_hz(60),
            },
            DisplayMode {
                width: 3840,
                height: 2160,
                refresh: RefreshRate::from_hz(60),
            },
        ];
        if let Some(current) = m.mode
            && !modes.contains(&current)
        {
            modes.push(current);
        }
        Ok(modes)
    }
}
