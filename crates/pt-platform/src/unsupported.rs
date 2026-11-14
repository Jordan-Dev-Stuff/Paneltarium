//! Placeholder backend for OSes that are not implemented yet. Lets the whole
//! workspace build and test everywhere (e.g. Linux CI).

use pt_core::{
    ApplyPlan, Autostart, AutostartStatus, DisplayBackend, DisplayLayout, DisplayMode, Error, KeyboardState, Modifiers,
    MonitorId, Result, Topology,
};

#[allow(dead_code)] // used on OSes without a display backend
pub struct UnsupportedBackend;

impl DisplayBackend for UnsupportedBackend {
    fn name(&self) -> &'static str {
        "unsupported"
    }
    fn current_layout(&self) -> Result<DisplayLayout> {
        Err(Error::Unsupported)
    }
    fn apply(&self, _: &ApplyPlan, _: bool) -> Result<()> {
        Err(Error::Unsupported)
    }
    fn set_topology(&self, _: Topology) -> Result<()> {
        Err(Error::Unsupported)
    }
    fn supported_modes(&self, _: &MonitorId) -> Result<Vec<DisplayMode>> {
        Err(Error::Unsupported)
    }
}

/// A display backend that couldn't start; every call says why.
#[allow(dead_code)] // used where a backend can be missing at run time (Linux)
pub struct Unavailable(pub String);

impl Unavailable {
    fn error(&self) -> Error {
        Error::Os {
            call: "display backend",
            code: None,
            message: self.0.clone(),
        }
    }
}

impl DisplayBackend for Unavailable {
    fn name(&self) -> &'static str {
        "unavailable"
    }
    fn current_layout(&self) -> Result<DisplayLayout> {
        Err(self.error())
    }
    fn apply(&self, _: &ApplyPlan, _: bool) -> Result<()> {
        Err(self.error())
    }
    fn set_topology(&self, _: Topology) -> Result<()> {
        Err(self.error())
    }
    fn supported_modes(&self, _: &MonitorId) -> Result<Vec<DisplayMode>> {
        Err(self.error())
    }
}

pub struct UnsupportedKeyboard;

impl KeyboardState for UnsupportedKeyboard {
    fn modifiers_held(&self) -> Option<Modifiers> {
        None
    }
}

#[allow(dead_code)] // used on OSes without an autostart implementation
pub struct UnsupportedAutostart;

impl Autostart for UnsupportedAutostart {
    fn status(&self) -> Result<AutostartStatus> {
        Ok(AutostartStatus::Off)
    }
    fn enable(&self, _: &std::path::Path) -> Result<()> {
        Err(Error::Unsupported)
    }
    fn disable(&self) -> Result<()> {
        Ok(())
    }
}
