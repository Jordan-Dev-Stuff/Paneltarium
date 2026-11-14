//! The numbered profile overlay (roadmap step 6).
//!
//! Hold the overlay modifiers (default Ctrl+Alt) and tap D: a panel appears
//! in the middle of *every* active monitor. Press a number to apply that
//! profile (0 = duplicate on all monitors), move with the arrow keys and
//! press Enter, or point and click. Let go of the modifiers or press Esc to
//! cancel (once the mouse is used, it stays open until you click or press
//! Esc). A game controller can open it too (see `crate::gamepad`): hold the
//! button combination, move with the D-pad, let go to apply.
//!
//! - [`session`]: numbering and key handling (pure, tested)
//! - [`render`] + [`canvas`]: drawing (pure, tested)
//! - this module: windows, temporary digit hotkeys, modifier polling
//!
//! The windows never take keyboard focus: digits arrive as temporary global
//! hotkeys, so whatever app you were using keeps its focus.
//!
//! Linux: the windows are X11 override-redirect windows (XWayland on a
//! Wayland desktop), the only kind that can be placed on a given monitor
//! and kept above everything without taking focus. On Wayland the keys
//! come from the desktop's shortcut portal, bound up front
//! ([`desktop_keys`]), and only with the trigger's modifiers held.

pub mod canvas;
pub mod render;
pub mod session;

use std::collections::HashMap;
use std::num::NonZeroU32;
use std::rc::Rc;
use std::time::{Duration, Instant};

use global_hotkey::hotkey::{Code, HotKey};
use pt_core::current::Current;
use pt_core::hotkey::NamedKey;
use pt_core::{Hotkey, Key, KeyboardState, Modifiers, OverlayMode, Profile, Theme};
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::window::{Window, WindowAttributes, WindowId, WindowLevel};

use crate::bindings::{self, HotkeyRegistry};
use canvas::Fonts;
use render::View;
use session::{Decision, Input, Opener, Session};

/// How often to check whether the modifiers were released.
const POLL: Duration = Duration::from_millis(30);
/// Close anyway after this long (also the only way out on platforms where
/// the modifier state can't be read, besides Esc and the digits).
const TIMEOUT: Duration = Duration::from_secs(15);

type Surface = softbuffer::Surface<Rc<Window>, Rc<Window>>;

struct OverlayWindow {
    window: Rc<Window>,
    surface: Surface,
    /// Last pointer position over this window (physical px), if inside.
    cursor: Option<(f64, f64)>,
    /// Drawing scale: the monitor's scale factor, shrunk to fit if needed.
    scale: f64,
}

struct Open {
    session: Session,
    required: Modifiers,
    windows: Vec<OverlayWindow>,
    /// Hotkey id → what it means while the overlay is open.
    keys: HashMap<u32, Input>,
    /// Hotkeys we registered just for this session (to unregister on close).
    temporary: Vec<HotKey>,
    /// Last interaction; the overlay closes after [`TIMEOUT`] of inactivity.
    last_activity: Instant,
}

pub struct Overlay {
    fonts: Fonts,
    keyboard: Box<dyn KeyboardState>,
    open: Option<Open>,
    /// Accent colour from settings (applies the next time it opens).
    theme: Theme,
    /// Hold the hotkey's modifiers to keep it open, or not.
    mode: OverlayMode,
}

impl Overlay {
    pub fn new(keyboard: Box<dyn KeyboardState>) -> Self {
        Overlay {
            fonts: Fonts::load(),
            keyboard,
            open: None,
            theme: Theme::default(),
            mode: OverlayMode::default(),
        }
    }

    pub fn set_mode(&mut self, mode: OverlayMode) {
        self.mode = mode;
    }

    pub fn set_theme(&mut self, theme: Theme) {
        self.theme = theme;
    }

    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    pub fn opener(&self) -> Option<Opener> {
        self.open.as_ref().map(|o| o.session.opener)
    }

    /// What a hotkey press means while the overlay is open, if anything.
    pub fn input_for(&self, hotkey_id: u32) -> Option<Input> {
        self.open.as_ref()?.keys.get(&hotkey_id).copied()
    }

    /// Open on every monitor. `trigger` is the keyboard overlay hotkey: its
    /// modifiers decide the numbering (and, for a keyboard opener, which keys
    /// must stay held).
    pub fn open(
        &mut self,
        event_loop: &ActiveEventLoop,
        trigger: Hotkey,
        opener: Opener,
        profiles: &[Profile],
        current: &Current,
        registry: &mut HotkeyRegistry,
    ) {
        self.close(registry);
        let required = trigger.modifiers;
        // Close when the modifiers are let go only in "hold" mode (and where the
        // OS lets us see them); otherwise it stays open until a choice or Esc.
        let track_release = self.mode == OverlayMode::Hold && self.keyboard.modifiers_held().is_some();
        let session = Session::new(profiles, required, current, opener, track_release);

        // While open: digits (main row and numpad), arrows, Enter and Esc —
        // each both with the trigger's modifiers (still held) and without.
        let mods = bindings::to_global(&trigger).map(|h| h.mods).unwrap_or_default();
        let mut keys = HashMap::new();
        let mut wanted = Vec::new();
        for (code, input) in key_codes() {
            for m in [Some(mods), None] {
                let hk = HotKey::new(m, code);
                keys.insert(hk.id(), input);
                wanted.push(hk);
            }
        }
        let temporary = registry.register_temporary(&wanted);

        let hint = session.hint();
        let view = View {
            entries: &session.entries,
            overflow: session.overflow,
            hint: &hint,
            selected: session.selected,
            theme: self.theme,
        };
        let mut windows = Vec::new();
        for monitor in event_loop.available_monitors() {
            let (mpos, msize) = (monitor.position(), monitor.size());
            // Monitors can have different scale factors (and sit at negative
            // coordinates, left of/above the primary): size and centre the
            // panel per monitor, in that monitor's physical pixels.
            let scale = render::fit_scale(&view, monitor.scale_factor(), msize.width, msize.height);
            let (w, h) = render::size(&view, scale);
            let x = mpos.x + (msize.width as i32 - w as i32) / 2;
            let y = mpos.y + (msize.height as i32 - h as i32) / 2;
            match create_window(event_loop, w, h, x, y, scale) {
                Ok(win) => windows.push(win),
                Err(e) => tracing::error!("overlay window: {e}"),
            }
        }
        tracing::info!(
            "overlay open ({opener:?}) on {} monitor(s), {} entries",
            windows.len(),
            session.entries.len()
        );
        self.open = Some(Open {
            session,
            required,
            windows,
            keys,
            temporary,
            last_activity: Instant::now(),
        });
    }

    pub fn close(&mut self, registry: &mut HotkeyRegistry) {
        if let Some(open) = self.open.take() {
            registry.unregister_temporary(&open.temporary);
            tracing::info!("overlay closed");
            // Windows close when dropped.
        }
    }

    /// Feed an input. Repaints on `Redraw`; closes on anything but
    /// `KeepOpen`/`Redraw`.
    pub fn handle(&mut self, input: Input, registry: &mut HotkeyRegistry) -> Decision {
        let Some(open) = &mut self.open else {
            return Decision::Close;
        };
        open.last_activity = Instant::now();
        let decision = open.session.handle(input);
        match decision {
            Decision::KeepOpen => {}
            Decision::Redraw => open.windows.iter().for_each(|w| w.window.request_redraw()),
            _ => self.close(registry),
        }
        decision
    }

    /// Called while idle: closes on keyboard-modifier release (keyboard
    /// opener, mouse not in use) or after a period of inactivity.
    pub fn poll(&mut self, registry: &mut HotkeyRegistry) {
        let Some(open) = &self.open else { return };
        let input = if open.last_activity.elapsed() > TIMEOUT {
            Some(Input::Timeout)
        } else if open.session.closes_on_modifier_release()
            && let Some(held) = self.keyboard.modifiers_held()
            && !session::still_held(open.required, held)
        {
            Some(Input::ModifiersReleased)
        } else {
            None
        };
        if let Some(i) = input {
            self.handle(i, registry);
        }
    }

    /// When the event loop should wake up next to poll, if the overlay is open.
    pub fn next_wake(&self) -> Option<Instant> {
        self.open.as_ref().map(|_| Instant::now() + POLL)
    }

    /// Paint, track the pointer, and turn hovers/clicks into inputs (which
    /// the caller feeds back through [`Overlay::handle`]).
    pub fn window_event(&mut self, id: WindowId, event: &WindowEvent) -> Option<Input> {
        let open = self.open.as_mut()?;
        let rows = open.session.entries.len();
        let w = open.windows.iter_mut().find(|w| w.window.id() == id)?;
        let row_under_cursor = |w: &OverlayWindow| {
            let (x, y) = w.cursor?;
            render::row_at(rows, w.scale, w.window.inner_size().width, x, y)
        };
        match event {
            WindowEvent::RedrawRequested => {
                let hint = open.session.hint();
                let view = View {
                    entries: &open.session.entries,
                    overflow: open.session.overflow,
                    hint: &hint,
                    selected: open.session.selected,
                    theme: self.theme,
                };
                if let Err(e) = paint(w, &view, &self.fonts) {
                    tracing::error!("overlay paint: {e}");
                }
                None
            }
            WindowEvent::Resized(size) => {
                round_corners(&w.window, *size, w.scale);
                w.window.request_redraw();
                None
            }
            WindowEvent::ScaleFactorChanged { .. } => {
                w.window.request_redraw();
                None
            }
            WindowEvent::CursorMoved { position, .. } => {
                w.cursor = Some((position.x, position.y));
                row_under_cursor(w).map(Input::Hover)
            }
            WindowEvent::CursorLeft { .. } => {
                w.cursor = None;
                None
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } => row_under_cursor(w).map(Input::Click),
            _ => None,
        }
    }
}

fn create_window(
    event_loop: &ActiveEventLoop,
    w: u32,
    h: u32,
    x: i32,
    y: i32,
    scale: f64,
) -> Result<OverlayWindow, String> {
    let attrs = Window::default_attributes()
        .with_title("Paneltarium")
        .with_decorations(false)
        .with_resizable(false)
        .with_window_level(WindowLevel::AlwaysOnTop)
        .with_active(false) // never steal focus from the app you're in
        .with_inner_size(PhysicalSize::new(w, h))
        .with_position(PhysicalPosition::new(x, y));
    let window = Rc::new(
        event_loop
            .create_window(platform_attributes(attrs))
            .map_err(|e| e.to_string())?,
    );
    let context = softbuffer::Context::new(window.clone()).map_err(|e| e.to_string())?;
    let surface = softbuffer::Surface::new(&context, window.clone()).map_err(|e| e.to_string())?;
    round_corners(&window, window.inner_size(), scale);
    // Clicking the overlay must not take focus away from your game/app.
    pt_platform::make_window_non_activating(window.as_ref());
    window.request_redraw();
    Ok(OverlayWindow {
        window,
        surface,
        cursor: None,
        scale,
    })
}

/// Clip the window to the panel's rounded shape (see
/// `pt_platform::round_window_corners`: Windows 11 rounds it natively,
/// Windows 10 gets a window region).
fn round_corners(window: &Window, size: PhysicalSize<u32>, scale: f64) {
    let radius = (render::CORNER_RADIUS as f64 * scale).round() as u32;
    pt_platform::round_window_corners(window, size.width, size.height, radius);
}

/// Every key the overlay listens to while open, and what it means.
fn key_codes() -> Vec<(Code, Input)> {
    let mut out = Vec::new();
    for n in 0..=9u8 {
        out.push((digit_code(n), Input::Digit(n)));
        out.push((numpad_code(n), Input::Digit(n)));
    }
    out.extend([
        (Code::ArrowUp, Input::Move(-1)),
        (Code::ArrowLeft, Input::Move(-1)),
        (Code::ArrowDown, Input::Move(1)),
        (Code::ArrowRight, Input::Move(1)),
        (Code::Enter, Input::Confirm),
        (Code::NumpadEnter, Input::Confirm),
        (Code::Escape, Input::Cancel),
    ]);
    out
}

/// The overlay keys to bind up front where the desktop owns shortcuts
/// (Wayland): the trigger's modifiers with the main-row digits, ↑/↓, Enter
/// and Esc. Left/right and the numpad are left out to keep the list the
/// desktop shows short (and GNOME uses Ctrl+Alt+←/→ for workspaces).
pub fn desktop_keys(trigger: Hotkey) -> Vec<(Hotkey, String)> {
    let with = |key| Hotkey::new(trigger.modifiers, key);
    let mut out: Vec<(Hotkey, String)> = (0..=9u8)
        .map(|n| {
            let what = if n == 0 {
                "duplicate".to_string()
            } else {
                format!("profile {n}")
            };
            (with(Key::Digit(n)), format!("Profile list: {what}"))
        })
        .collect();
    out.extend([
        (with(Key::Named(NamedKey::Up)), "Profile list: previous".into()),
        (with(Key::Named(NamedKey::Down)), "Profile list: next".into()),
        (with(Key::Named(NamedKey::Enter)), "Profile list: choose".into()),
        (with(Key::Named(NamedKey::Escape)), "Profile list: close".into()),
    ]);
    out
}

/// Window-creation options only winit's Windows backend has: a shadow, no
/// taskbar button, and no drag-and-drop (which would initialise COM in a way
/// that can clash with other libraries on this thread).
#[cfg(windows)]
fn platform_attributes(attrs: WindowAttributes) -> WindowAttributes {
    use winit::platform::windows::WindowAttributesExtWindows;
    attrs
        .with_skip_taskbar(true)
        .with_drag_and_drop(false)
        .with_undecorated_shadow(true)
}

/// Linux: an X11 override-redirect window is placed exactly where asked,
/// stays above other windows, has no taskbar entry and is never focused by
/// the window manager. (Plain Wayland windows can do none of that; the app
/// runs its windows through XWayland when it's there, see `main.rs`.)
#[cfg(target_os = "linux")]
fn platform_attributes(attrs: WindowAttributes) -> WindowAttributes {
    use winit::platform::x11::{WindowAttributesExtX11, WindowType};
    attrs
        .with_override_redirect(true)
        .with_x11_window_type(vec![WindowType::Notification])
}

#[cfg(not(any(windows, target_os = "linux")))]
fn platform_attributes(attrs: WindowAttributes) -> WindowAttributes {
    attrs
}

fn paint(w: &mut OverlayWindow, view: &View, fonts: &Fonts) -> Result<(), String> {
    let size = w.window.inner_size();
    let (Some(width), Some(height)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height)) else {
        return Ok(());
    };
    let canvas = render::draw(view, fonts, size.width, size.height, w.scale);
    w.surface.resize(width, height).map_err(|e| e.to_string())?;
    let mut buffer = w.surface.buffer_mut().map_err(|e| e.to_string())?;
    buffer.copy_from_slice(&canvas.pixels);
    buffer.present().map_err(|e| e.to_string())
}

fn digit_code(n: u8) -> Code {
    [
        Code::Digit0,
        Code::Digit1,
        Code::Digit2,
        Code::Digit3,
        Code::Digit4,
        Code::Digit5,
        Code::Digit6,
        Code::Digit7,
        Code::Digit8,
        Code::Digit9,
    ][n as usize]
}

fn numpad_code(n: u8) -> Code {
    [
        Code::Numpad0,
        Code::Numpad1,
        Code::Numpad2,
        Code::Numpad3,
        Code::Numpad4,
        Code::Numpad5,
        Code::Numpad6,
        Code::Numpad7,
        Code::Numpad8,
        Code::Numpad9,
    ][n as usize]
}
