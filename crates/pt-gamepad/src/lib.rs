//! # pt-gamepad
//!
//! Reads game controllers on a background thread with `gilrs` and reports
//! presses/releases as `pt_core::gamepad::Button`s. Used by the background
//! app (overlay chord, D-pad) and the profile manager (recording a chord).
//!
//! Switching it off ([`GamepadWatcher::set_enabled`]) drops the `gilrs`
//! instance, so controllers aren't read at all until it's switched back on.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use gilrs::{EventType, Gilrs};
use pt_core::gamepad::Button;

/// One button going down or up on one controller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PadEvent {
    /// Which controller (stable while it stays connected).
    pub pad: usize,
    pub button: Button,
    pub pressed: bool,
}

/// How often to poll while a controller is connected (~120 Hz, so holding
/// and releasing the chord feels instant).
const POLL: Duration = Duration::from_millis(8);
/// How often to check while disabled, or while no controller is connected
/// (only for "connected" events, so 4× a second is plenty and keeps the
/// idle app near 0 % CPU).
const IDLE: Duration = Duration::from_millis(250);

pub struct GamepadWatcher {
    enabled: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl GamepadWatcher {
    /// Start the reader thread. `on_event` is called on that thread.
    pub fn spawn(enabled: bool, on_event: impl Fn(PadEvent) + Send + 'static) -> Self {
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

fn run(enabled: &AtomicBool, stop: &AtomicBool, on_event: impl Fn(PadEvent)) {
    let mut gilrs: Option<Gilrs> = None;
    while !stop.load(Ordering::Relaxed) {
        if !enabled.load(Ordering::Relaxed) {
            if gilrs.take().is_some() {
                tracing::info!("game controllers: off");
            }
            thread::sleep(IDLE);
            continue;
        }
        let g = match &mut gilrs {
            Some(g) => g,
            None => match Gilrs::new() {
                Ok(g) => {
                    tracing::info!("game controllers: on ({} connected)", g.gamepads().count());
                    gilrs.insert(g)
                }
                Err(e) => {
                    tracing::warn!("game controllers unavailable: {e}");
                    thread::sleep(Duration::from_secs(5));
                    continue;
                }
            },
        };
        while let Some(ev) = g.next_event() {
            let pad = usize::from(ev.id);
            let (button, pressed) = match ev.event {
                EventType::ButtonPressed(b, _) => (b, true),
                EventType::ButtonReleased(b, _) => (b, false),
                EventType::Connected => {
                    tracing::info!("controller {pad} connected");
                    continue;
                }
                EventType::Disconnected => {
                    tracing::info!("controller {pad} disconnected");
                    continue;
                }
                _ => continue,
            };
            if let Some(button) = map(button) {
                on_event(PadEvent { pad, button, pressed });
            }
        }
        thread::sleep(poll_interval(g.gamepads().next().is_some()));
    }
}

fn poll_interval(any_connected: bool) -> Duration {
    if any_connected { POLL } else { IDLE }
}

/// gilrs uses positional names (South = bottom face button, ...).
pub fn map(b: gilrs::Button) -> Option<Button> {
    use gilrs::Button as G;
    Some(match b {
        G::South => Button::A,
        G::East => Button::B,
        G::West => Button::X,
        G::North => Button::Y,
        G::LeftTrigger => Button::LB,
        G::RightTrigger => Button::RB,
        G::LeftTrigger2 => Button::LT,
        G::RightTrigger2 => Button::RT,
        G::Select => Button::Back,
        G::Start => Button::Start,
        G::Mode => Button::Guide,
        G::LeftThumb => Button::LeftStick,
        G::RightThumb => Button::RightStick,
        G::DPadUp => Button::DpadUp,
        G::DPadDown => Button::DpadDown,
        G::DPadLeft => Button::DpadLeft,
        G::DPadRight => Button::DpadRight,
        _ => return None,
    })
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
    fn positional_names_map_to_xbox_names() {
        assert_eq!(map(gilrs::Button::South), Some(Button::A));
        assert_eq!(
            map(gilrs::Button::LeftTrigger),
            Some(Button::LB),
            "gilrs' LeftTrigger is the bumper"
        );
        assert_eq!(map(gilrs::Button::LeftTrigger2), Some(Button::LT));
        assert_eq!(map(gilrs::Button::Unknown), None);
    }

    #[test]
    fn watcher_starts_toggles_and_stops() {
        // No controllers here; this checks the thread lifecycle doesn't hang.
        let w = GamepadWatcher::spawn(false, |_| {});
        assert!(!w.is_enabled());
        w.set_enabled(true);
        assert!(w.is_enabled());
        thread::sleep(Duration::from_millis(50));
        w.set_enabled(false);
        drop(w);
    }
}
