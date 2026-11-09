//! The only unsafe code in this crate: a small wrapper around SDL 3's
//! gamepad API. Everything here runs on the gamepad thread (SDL is started,
//! polled and shut down there).
//!
//! Settings ("hints") chosen for a background tray app:
//! - background events on: controllers are read while a game has focus;
//! - Windows.Gaming.Input off: it only reports to the focused window
//!   (SDL uses raw input and XInput instead);
//! - PlayStation enhanced reports "auto": SDL doesn't switch the controller's
//!   report mode (or touch its lights) unless an app asks for those features;
//! - Valve controllers (Steam Controller, Steam Deck) only while Steam runs:
//!   SDL turns off their built-in mouse mode ("lizard mode") while it has
//!   them open. With Steam running that's harmless (Steam does the same and
//!   does the desktop mapping itself); without Steam it would take away the
//!   controller's mouse.

use std::collections::HashMap;
use std::ffi::{CStr, c_char};
use std::sync::{Mutex, MutexGuard};

use sdl3_sys::everything::*;

/// What SDL reported, before mapping to our button names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Raw {
    Added { id: u32, name: String },
    Removed { id: u32 },
    Button { id: u32, button: i32, down: bool },
    Axis { id: u32, axis: i32, value: i16 },
}

/// SDL is one global engine per process and isn't safe to start from two
/// threads at once, so only one [`Sdl`] may exist at a time.
static IN_USE: Mutex<()> = Mutex::new(());

pub struct Sdl {
    pads: HashMap<u32, *mut SDL_Gamepad>,
    /// Released (after SDL_Quit) when this is dropped.
    _in_use: MutexGuard<'static, ()>,
}

fn set_hint(name: *const c_char, value: &CStr) {
    // SAFETY: both are valid NUL-terminated strings; SDL copies them.
    unsafe {
        SDL_SetHint(name, value.as_ptr());
    }
}

fn error() -> String {
    // SAFETY: SDL_GetError always returns a valid (possibly empty) string.
    unsafe { CStr::from_ptr(SDL_GetError()).to_string_lossy().into_owned() }
}

impl Sdl {
    pub fn start(read_valve: bool) -> Result<Self, String> {
        let in_use = IN_USE.lock().unwrap_or_else(|e| e.into_inner());
        set_hint(SDL_HINT_JOYSTICK_ALLOW_BACKGROUND_EVENTS, c"1");
        set_hint(SDL_HINT_JOYSTICK_WGI, c"0");
        set_hint(SDL_HINT_JOYSTICK_ENHANCED_REPORTS, c"auto");
        let s = Sdl {
            pads: HashMap::new(),
            _in_use: in_use,
        };
        s.read_valve_controllers(read_valve);
        // SAFETY: plain init call; paired with SDL_Quit in Drop.
        if unsafe { SDL_Init(SDL_INIT_GAMEPAD) } {
            Ok(s)
        } else {
            Err(error())
        }
    }

    /// SDL re-checks these hints live: controllers are opened/closed when
    /// they change.
    pub fn read_valve_controllers(&self, on: bool) {
        let v = if on { c"1" } else { c"0" };
        set_hint(SDL_HINT_JOYSTICK_HIDAPI_STEAM, v);
        set_hint(SDL_HINT_JOYSTICK_HIDAPI_STEAMDECK, v);
    }

    pub fn version(&self) -> String {
        let v = SDL_GetVersion();
        format!("{}.{}.{}", v / 1_000_000, v / 1000 % 1000, v % 1000)
    }

    pub fn any_open(&self) -> bool {
        !self.pads.is_empty()
    }

    /// Drain SDL's event queue into `out`.
    pub fn poll(&mut self, out: &mut Vec<Raw>) {
        // SAFETY: SDL_Event is a plain C union; SDL_PollEvent fills it in and
        // we only read the member that matches its `type`.
        unsafe {
            let mut ev: SDL_Event = std::mem::zeroed();
            while SDL_PollEvent(&mut ev) {
                let kind = SDL_EventType(ev.r#type);
                if kind == SDL_EVENT_GAMEPAD_ADDED {
                    let id = ev.gdevice.which;
                    let pad = SDL_OpenGamepad(id);
                    if pad.is_null() {
                        tracing::warn!("controller {}: can't open: {}", id.0, error());
                        continue;
                    }
                    let name = SDL_GetGamepadName(pad);
                    let name = if name.is_null() {
                        "Unknown controller".to_string()
                    } else {
                        CStr::from_ptr(name).to_string_lossy().into_owned()
                    };
                    self.pads.insert(id.0, pad);
                    out.push(Raw::Added { id: id.0, name });
                } else if kind == SDL_EVENT_GAMEPAD_REMOVED {
                    let id = ev.gdevice.which.0;
                    if let Some(pad) = self.pads.remove(&id) {
                        SDL_CloseGamepad(pad);
                    }
                    out.push(Raw::Removed { id });
                } else if kind == SDL_EVENT_GAMEPAD_BUTTON_DOWN || kind == SDL_EVENT_GAMEPAD_BUTTON_UP {
                    let b = ev.gbutton;
                    out.push(Raw::Button {
                        id: b.which.0,
                        button: i32::from(b.button),
                        down: b.down,
                    });
                } else if kind == SDL_EVENT_GAMEPAD_AXIS_MOTION {
                    let a = ev.gaxis;
                    out.push(Raw::Axis {
                        id: a.which.0,
                        axis: i32::from(a.axis),
                        value: a.value,
                    });
                }
            }
        }
    }
}

impl Drop for Sdl {
    fn drop(&mut self) {
        // SAFETY: every pointer came from SDL_OpenGamepad and is closed once;
        // SDL_Quit undoes SDL_Init.
        unsafe {
            for (_, pad) in self.pads.drain() {
                SDL_CloseGamepad(pad);
            }
            SDL_Quit();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// End to end through SDL with one of its virtual controllers (same
    /// event path as a real pad, no hardware needed).
    #[test]
    fn reports_a_virtual_controller() {
        let mut sdl = Sdl::start(false).expect("SDL starts");
        let mut out = Vec::new();
        // SAFETY: test-only SDL calls on this thread, on a joystick we attach,
        // open and detach here.
        unsafe {
            let desc = SDL_VirtualJoystickDesc {
                r#type: SDL_JOYSTICK_TYPE_GAMEPAD.0 as u16,
                naxes: 6,
                nbuttons: 21,
                name: c"Test Pad".as_ptr(),
                ..Default::default()
            };
            let id = SDL_AttachVirtualJoystick(&desc);
            assert_ne!(id.0, 0, "attach: {}", error());
            let joy = SDL_OpenJoystick(id);
            assert!(!joy.is_null());

            sdl.poll(&mut out);
            assert!(
                out.iter()
                    .any(|r| matches!(r, Raw::Added { name, .. } if name == "Test Pad")),
                "{out:?}"
            );
            assert!(sdl.any_open());

            out.clear();
            SDL_SetJoystickVirtualButton(joy, 0, true); // south = A
            SDL_SetJoystickVirtualAxis(joy, 5, 30_000); // right trigger
            sdl.poll(&mut out);
            assert!(
                out.contains(&Raw::Button {
                    id: id.0,
                    button: 0,
                    down: true
                }),
                "{out:?}"
            );
            assert!(
                out.iter()
                    .any(|r| matches!(r, Raw::Axis { axis: 5, value, .. } if *value > 16_000)),
                "{out:?}"
            );

            out.clear();
            SDL_SetJoystickVirtualButton(joy, 0, false);
            sdl.poll(&mut out);
            assert!(
                out.contains(&Raw::Button {
                    id: id.0,
                    button: 0,
                    down: false
                }),
                "{out:?}"
            );

            out.clear();
            SDL_CloseJoystick(joy);
            SDL_DetachVirtualJoystick(id);
            sdl.poll(&mut out);
            assert!(out.contains(&Raw::Removed { id: id.0 }), "{out:?}");
            assert!(!sdl.any_open());
        }
    }
}
