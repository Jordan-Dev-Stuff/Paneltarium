//! # pt-gamepad
//!
//! Reads game controllers on a background thread with SDL 3 and reports
//! presses/releases as `pt_core::gamepad::Button`s. Used by the background
//! app (overlay chord, D-pad), the profile manager (recording a chord) and
//! `paneltarium gamepads --watch`.
//!
//! SDL reads controllers directly (HID), so besides Xbox-style (XInput) pads
//! this covers PlayStation (DualShock 3/4, DualSense), Nintendo (Switch Pro,
//! Joy-Con, Switch 2) and Valve (Steam Controller) controllers, plus many
//! others through SDL's mapping database. It runs "listen only" as far as
//! SDL allows: no rumble, no PlayStation enhanced-report switching. Valve's
//! own controllers are only opened while Steam runs (see [`sdl`]).
//!
//! Switching it off ([`GamepadWatcher::set_enabled`]) shuts SDL down, so
//! controllers aren't read at all until it's switched back on.

mod sdl;

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use pt_core::gamepad::Button;

/// One button going down or up on one controller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PadEvent {
    /// Which controller (stable while it stays connected).
    pub pad: usize,
    pub button: Button,
    pub pressed: bool,
}

/// Everything the reader reports (for `paneltarium gamepads --watch`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Connected { pad: usize, name: String },
    Disconnected { pad: usize },
    Button(PadEvent),
}

/// How often to poll while a controller is connected (~120 Hz, so holding
/// and releasing the chord feels instant).
const POLL: Duration = Duration::from_millis(8);
/// How often to check while disabled, or while no controller is connected
/// (only for "connected" events, so 4× a second is plenty and keeps the
/// idle app near 0 % CPU).
const IDLE: Duration = Duration::from_millis(250);
/// How often to re-check whether Steam is running.
const STEAM_CHECK: Duration = Duration::from_secs(5);

pub struct GamepadWatcher {
    enabled: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl GamepadWatcher {
    /// Start the reader thread. `on_event` is called on that thread for
    /// every button press/release.
    pub fn spawn(enabled: bool, on_event: impl Fn(PadEvent) + Send + 'static) -> Self {
        Self::spawn_events(enabled, move |e| {
            if let Event::Button(b) = e {
                on_event(b);
            }
        })
    }

    /// Like [`Self::spawn`], but also reports controllers connecting and
    /// disconnecting.
    pub fn spawn_events(enabled: bool, on_event: impl Fn(Event) + Send + 'static) -> Self {
        let enabled = Arc::new(AtomicBool::new(enabled));
        let stop = Arc::new(AtomicBool::new(false));
        let (en, st) = (enabled.clone(), stop.clone());
        let thread = thread::Builder::new()
            .name("gamepad".into())
            .spawn(move || run(&en, &st, on_event))
            .expect("spawning the gamepad thread");
        GamepadWatcher {
            enabled,
            stop,
            thread: Some(thread),
        }
    }

    pub fn set_enabled(&self, on: bool) {
        self.enabled.store(on, Ordering::Relaxed);
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }
}

impl Drop for GamepadWatcher {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn run(enabled: &AtomicBool, stop: &AtomicBool, on_event: impl Fn(Event)) {
    let mut sdl: Option<sdl::Sdl> = None;
    let mut triggers = Triggers::default();
    let mut steam = (false, Instant::now() - STEAM_CHECK);
    let mut raw = Vec::new();
    while !stop.load(Ordering::Relaxed) {
        if !enabled.load(Ordering::Relaxed) {
            if sdl.take().is_some() {
                triggers = Triggers::default();
                tracing::info!("game controllers: off");
            }
            thread::sleep(IDLE);
            continue;
        }
        if steam.1.elapsed() >= STEAM_CHECK {
            let running = pt_platform::steam_running();
            if running != steam.0
                && let Some(s) = &sdl
            {
                tracing::info!("Steam {}", if running { "started" } else { "stopped" });
                s.read_valve_controllers(running);
            }
            steam = (running, Instant::now());
        }
        let s = match &mut sdl {
            Some(s) => s,
            None => match sdl::Sdl::start(steam.0) {
                Ok(s) => {
                    tracing::info!("game controllers: on (SDL {})", s.version());
                    sdl.insert(s)
                }
                Err(e) => {
                    tracing::warn!("game controllers unavailable: {e}");
                    thread::sleep(Duration::from_secs(5));
                    continue;
                }
            },
        };
        raw.clear();
        s.poll(&mut raw);
        for r in raw.drain(..) {
            let event = match r {
                sdl::Raw::Added { id, name } => {
                    tracing::info!("controller {id} connected: {name}");
                    Event::Connected { pad: id as usize, name }
                }
                sdl::Raw::Removed { id } => {
                    tracing::info!("controller {id} disconnected");
                    triggers.forget(id);
                    Event::Disconnected { pad: id as usize }
                }
                sdl::Raw::Button { id, button, down } => match map_button(button) {
                    Some(button) => Event::Button(PadEvent {
                        pad: id as usize,
                        button,
                        pressed: down,
                    }),
                    None => continue,
                },
                sdl::Raw::Axis { id, axis, value } => match triggers.update(id, axis, value) {
                    Some((button, pressed)) => Event::Button(PadEvent {
                        pad: id as usize,
                        button,
                        pressed,
                    }),
                    None => continue,
                },
            };
            on_event(event);
        }
        thread::sleep(poll_interval(s.any_open()));
    }
}

fn poll_interval(any_connected: bool) -> Duration {
    if any_connected { POLL } else { IDLE }
}

/// SDL's positional button numbers (`SDL_GamepadButton`: south = bottom face
/// button, ...) to our Xbox-style names.
pub fn map_button(b: i32) -> Option<Button> {
    Some(match b {
        0 => Button::A,          // SOUTH
        1 => Button::B,          // EAST
        2 => Button::X,          // WEST
        3 => Button::Y,          // NORTH
        4 => Button::Back,       // BACK (View, Share, Select, Minus)
        5 => Button::Guide,      // GUIDE (Xbox, PS, Home, Steam)
        6 => Button::Start,      // START (Menu, Options, Plus)
        7 => Button::LeftStick,  // LEFT_STICK
        8 => Button::RightStick, // RIGHT_STICK
        9 => Button::LB,         // LEFT_SHOULDER
        10 => Button::RB,        // RIGHT_SHOULDER
        11 => Button::DpadUp,
        12 => Button::DpadDown,
        13 => Button::DpadLeft,
        14 => Button::DpadRight,
        15 => Button::Misc,     // MISC1 (Share, mic, capture, Steam QAM)
        16 => Button::R4,       // RIGHT_PADDLE1
        17 => Button::L4,       // LEFT_PADDLE1
        18 => Button::R5,       // RIGHT_PADDLE2
        19 => Button::L5,       // LEFT_PADDLE2
        20 => Button::Touchpad, // TOUCHPAD
        _ => return None,
    })
}

/// SDL's axis numbers for the analog triggers (`SDL_GamepadAxis`).
const AXIS_LEFT_TRIGGER: i32 = 4;
const AXIS_RIGHT_TRIGGER: i32 = 5;
/// Trigger travel (0..=32767) that counts as pressed / released. The gap
/// stops a half-held trigger from flickering.
const TRIGGER_PRESS: i16 = 16_000;
const TRIGGER_RELEASE: i16 = 8_000;

/// Turns analog trigger movement into LT/RT presses and releases.
#[derive(Debug, Default)]
struct Triggers {
    held: HashSet<(u32, Button)>,
}

impl Triggers {
    fn update(&mut self, pad: u32, axis: i32, value: i16) -> Option<(Button, bool)> {
        let button = match axis {
            AXIS_LEFT_TRIGGER => Button::LT,
            AXIS_RIGHT_TRIGGER => Button::RT,
            _ => return None,
        };
        let held = self.held.contains(&(pad, button));
        if !held && value >= TRIGGER_PRESS {
            self.held.insert((pad, button));
            Some((button, true))
        } else if held && value <= TRIGGER_RELEASE {
            self.held.remove(&(pad, button));
            Some((button, false))
        } else {
            None
        }
    }

    fn forget(&mut self, pad: u32) {
        self.held.retain(|(p, _)| *p != pad);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn polls_slowly_without_controllers() {
        assert_eq!(poll_interval(false), IDLE);
        assert_eq!(poll_interval(true), POLL);
        assert!(IDLE >= POLL * 20);
    }

    #[test]
    fn sdl_buttons_map_to_xbox_names() {
        assert_eq!(map_button(0), Some(Button::A));
        assert_eq!(map_button(9), Some(Button::LB), "left shoulder is the bumper");
        assert_eq!(map_button(15), Some(Button::Misc), "Steam Controller QAM");
        assert_eq!(map_button(17), Some(Button::L4), "Steam Controller L4");
        assert_eq!(map_button(16), Some(Button::R4));
        assert_eq!(map_button(20), Some(Button::Touchpad));
        assert_eq!(map_button(21), None, "MISC2..6 aren't used");
        assert_eq!(map_button(-1), None);
        // Every mapped button is distinct.
        let mapped: HashSet<Button> = (0..21).filter_map(map_button).collect();
        assert_eq!(mapped.len(), 21);
    }

    #[test]
    fn triggers_press_and_release_with_hysteresis() {
        let mut t = Triggers::default();
        assert_eq!(t.update(1, AXIS_LEFT_TRIGGER, 5_000), None);
        assert_eq!(t.update(1, AXIS_LEFT_TRIGGER, 20_000), Some((Button::LT, true)));
        assert_eq!(t.update(1, AXIS_LEFT_TRIGGER, 32_767), None, "already held");
        assert_eq!(
            t.update(1, AXIS_LEFT_TRIGGER, 12_000),
            None,
            "between thresholds: still held"
        );
        assert_eq!(t.update(1, AXIS_LEFT_TRIGGER, 1_000), Some((Button::LT, false)));
        // Per controller, and the sticks are ignored.
        assert_eq!(t.update(2, AXIS_RIGHT_TRIGGER, 30_000), Some((Button::RT, true)));
        assert_eq!(t.update(1, AXIS_RIGHT_TRIGGER, 30_000), Some((Button::RT, true)));
        assert_eq!(t.update(1, 0, 32_767), None, "left stick X");
        // A controller that disconnects mid-press starts fresh.
        t.forget(2);
        assert_eq!(t.update(2, AXIS_RIGHT_TRIGGER, 30_000), Some((Button::RT, true)));
    }

    #[test]
    fn watcher_starts_toggles_and_stops() {
        // No controllers here; this checks SDL starts, stops and restarts
        // and the thread lifecycle doesn't hang.
        let w = GamepadWatcher::spawn(false, |_| {});
        assert!(!w.is_enabled());
        w.set_enabled(true);
        assert!(w.is_enabled());
        thread::sleep(Duration::from_millis(100));
        w.set_enabled(false);
        thread::sleep(Duration::from_millis(300));
        w.set_enabled(true);
        thread::sleep(Duration::from_millis(100));
        drop(w);
    }
}
