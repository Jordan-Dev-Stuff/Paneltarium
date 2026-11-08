//! The event loop. Everything that touches hotkey registrations or the tray
//! runs here, on the main thread (both require it on Windows). Display
//! changes are handed to the [`Worker`].

use std::sync::Mutex;

use global_hotkey::{GlobalHotKeyEvent, HotKeyState};
use std::collections::HashMap;

use pt_core::current::{self, Current};
use pt_core::gamepad::{Button, ChordEvent, ChordTracker};
use pt_core::{AppPaths, Autostart, AutostartStatus, DisplayBackend, Hotkey, ProfileStore, Settings};
use pt_gamepad::{GamepadWatcher, PadEvent};
use tray_icon::menu::MenuEvent;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoopProxy};
use winit::window::WindowId;

use crate::bindings::{self, Action, HotkeyRegistry, RepeatFilter};
use crate::overlay::Overlay;
use crate::overlay::session::{Decision, Input, Opener};
use crate::tray::{self, Status, Tray};
use crate::watcher::{self, ConfigWatcher};
use crate::worker::{Job, Outcome, Worker};

/// Events delivered into the event loop from other threads/callbacks.
#[derive(Debug)]
pub enum UserEvent {
    Hotkey(GlobalHotKeyEvent),
    /// A game-controller button went down or up.
    Pad(PadEvent),
    Menu(MenuEvent),
    ConfigChanged,
    JobDone(Outcome),
}

/// Make a `Send + Sync` callback that posts events into the loop.
pub fn poster<T: Send + 'static>(
    proxy: &EventLoopProxy<UserEvent>,
    wrap: impl Fn(T) -> UserEvent + Send + Sync + 'static,
) -> impl Fn(T) + Send + Sync + 'static {
    let proxy = Mutex::new(proxy.clone());
    move |value| {
        let event = wrap(value);
        if let Ok(p) = proxy.lock() {
            // Fails only while the loop is shutting down.
            let _ = p.send_event(event);
        }
    }
}

pub struct App {
    paths: AppPaths,
    store: ProfileStore,
    hotkeys: HotkeyRegistry,
    worker: Worker,
    overlay: Overlay,
    /// Ignores a held hotkey's auto-repeat.
    repeats: RepeatFilter,
    tray: Option<Tray>,
    status: Status,
    settings: Settings,
    /// What the hotkeys were last registered from (to skip no-op reloads).
    loaded: Option<(Settings, Vec<pt_core::Profile>)>,
    /// Read-only view of the monitors, to work out what's on screen when
    /// the overlay opens (display *changes* go through the worker).
    reader: Option<Box<dyn DisplayBackend>>,
    /// "Start with Windows" switch for this OS.
    autostart: Option<Box<dyn Autostart>>,
    /// Started by the OS at sign-in (`--autostart`).
    at_sign_in: bool,
    gamepad: GamepadWatcher,
    /// Per-controller chord state.
    pads: HashMap<usize, ChordTracker>,
    _watcher: Option<ConfigWatcher>,
}

impl App {
    pub fn new(paths: AppPaths, proxy: &EventLoopProxy<UserEvent>, at_sign_in: bool) -> Result<Self, String> {
        let hotkeys = HotkeyRegistry::new()?;
        let worker = Worker::spawn(paths.clone(), poster(proxy, UserEvent::JobDone));
        let changed = poster(proxy, |()| UserEvent::ConfigChanged);
        let watcher = watcher::watch(paths.root(), move || changed(()))
            .inspect_err(|e| tracing::warn!("{e}; hotkeys will only reload on restart"))
            .ok();
        Ok(App {
            store: ProfileStore::new(paths.profiles_dir()),
            paths,
            hotkeys,
            worker,
            overlay: Overlay::new(pt_platform::keyboard_state()),
            repeats: RepeatFilter::default(),
            tray: None,
            status: Status::default(),
            settings: Settings::default(),
            loaded: None,
            autostart: pt_platform::autostart()
                .inspect_err(|e| tracing::warn!("start-at-sign-in unavailable: {e}"))
                .ok()
                .inspect(|a| {
                    // Carry "Start with Windows" over from the Monitor Pilot name.
                    if let Ok(exe) = std::env::current_exe() {
                        match a.migrate_legacy(&exe) {
                            Ok(true) => tracing::info!("replaced the old Monitor Pilot sign-in entry"),
                            Ok(false) => {}
                            Err(e) => tracing::warn!("couldn't replace the old sign-in entry: {e}"),
                        }
                    }
                }),
            at_sign_in,
            reader: pt_platform::backend_from_env()
                .inspect_err(|e| tracing::warn!("can't read monitors: {e}"))
                .ok(),
            // Started switched off; `reload` switches it on if enabled.
            gamepad: GamepadWatcher::spawn(false, poster(proxy, UserEvent::Pad)),
            pads: HashMap::new(),
            _watcher: watcher,
        })
    }

    /// Re-read settings and profiles and re-register every hotkey.
    fn reload(&mut self) {
        let mut warnings = Vec::new();
        let settings = Settings::load(&self.paths.settings_file()).unwrap_or_else(|e| {
            warnings.push(format!("settings: {e}"));
            Settings::default()
        });
        let profiles = match self.store.list_checked() {
            Ok((profiles, problems)) => {
                warnings.extend(problems.iter().map(|e| {
                    format!(
                        "skipped a profile: {}",
                        e.to_string().lines().next().unwrap_or_default()
                    )
                }));
                profiles
            }
            Err(e) => {
                warnings.push(format!("profiles: {e}"));
                Vec::new()
            }
        };
        let snapshot = (settings.clone(), profiles.clone());
        if warnings.is_empty() && self.loaded.as_ref() == Some(&snapshot) {
            return; // nothing that matters changed
        }
        self.loaded = Some(snapshot);
        tracing::info!("config changed, re-registering hotkeys");
        // The overlay's temporary digit hotkeys must not clash with the new set.
        self.overlay.close(&mut self.hotkeys);
        let (bindings, clashes) = bindings::build(&settings, &profiles);
        warnings.extend(clashes);
        warnings.extend(self.hotkeys.replace(&bindings));
        tracing::info!(
            "loaded {} profile(s), {} hotkey(s), {} warning(s)",
            profiles.len(),
            bindings.len(),
            warnings.len()
        );

        self.overlay.set_theme(settings.theme());
        self.overlay.set_mode(settings.overlay_mode);
        self.status.overlay = settings.overlay_hotkey;
        self.status.rescue = settings.duplicate_hotkey;
        self.status.gamepad_enabled = settings.gamepad_enabled;
        self.status.autostart = self.autostart.as_ref().and_then(|a| a.status().ok());
        self.status.gamepad_chord = settings.gamepad_overlay.as_ref().map(|c| c.to_string());
        let listen = settings.active_gamepad_chord().is_some();
        self.gamepad.set_enabled(listen);
        if !listen {
            self.pads.clear();
        }
        self.status.warnings = warnings;
        self.settings = settings;
        self.refresh_tray();
    }

    fn refresh_tray(&self) {
        if let Some(t) = &self.tray {
            t.update(&self.status);
        }
    }

    fn on_hotkey(&mut self, event_loop: &ActiveEventLoop, event: GlobalHotKeyEvent) {
        if event.state == HotKeyState::Released {
            self.repeats.release(event.id);
            return;
        }
        // Holding a hotkey makes Windows repeat it; act on the first press only
        // (otherwise holding Ctrl+Alt+D flashes the overlay open and shut).
        if !self.repeats.press(event.id, std::time::Instant::now()) {
            return;
        }
        // While the overlay is open its digits/Esc win over everything else
        // (including a profile hotkey that uses the same combo).
        if let Some(input) = self.overlay.input_for(event.id) {
            self.overlay_input(input);
            return;
        }
        let Some(action) = self.hotkeys.action_for(event.id).cloned() else {
            return;
        };
        tracing::info!("hotkey -> {action:?}");
        match action {
            Action::ApplyProfile(name) => self.worker.send(Job::Apply(name)),
            Action::Rescue => self.worker.send(Job::Rescue),
            Action::Extend => self.worker.send(Job::Extend),
            Action::Overlay if self.overlay.is_open() => self.overlay_input(Input::Cancel),
            Action::Overlay => self.open_overlay(event_loop, Opener::Keyboard),
        }
    }

    fn on_pad(&mut self, event_loop: &ActiveEventLoop, ev: PadEvent) {
        let Some(chord) = self.settings.active_gamepad_chord().cloned() else {
            return;
        };
        // While a controller-opened overlay is up: D-pad moves, B cancels.
        if ev.pressed && self.overlay.opener() == Some(Opener::Gamepad) {
            let input = match ev.button {
                Button::DpadUp | Button::DpadLeft => Some(Input::Move(-1)),
                Button::DpadDown | Button::DpadRight => Some(Input::Move(1)),
                Button::B if !chord.contains(Button::B) => Some(Input::Cancel),
                _ => None,
            };
            if let Some(i) = input {
                self.overlay_input(i);
            }
        }
        let tracker = self.pads.entry(ev.pad).or_default();
        match tracker.update(&chord, ev.button, ev.pressed) {
            Some(ChordEvent::Activated) if !self.overlay.is_open() => {
                tracing::info!("controller {} held {chord}", ev.pad);
                self.open_overlay(event_loop, Opener::Gamepad);
            }
            Some(ChordEvent::Released) if self.overlay.opener() == Some(Opener::Gamepad) => {
                self.overlay_input(Input::ChordReleased);
            }
            _ => {}
        }
    }

    fn toggle_autostart(&mut self) {
        let Some(a) = &self.autostart else { return };
        let result = match a.status() {
            Ok(AutostartStatus::On) => a.disable(),
            Ok(_) => std::env::current_exe()
                .map_err(|e| pt_core::Error::Os {
                    call: "current_exe",
                    code: None,
                    message: e.to_string(),
                })
                .and_then(|exe| a.enable(&exe)),
            Err(e) => Err(e),
        };
        match result {
            Ok(()) => tracing::info!("start at sign-in: {:?}", a.status()),
            Err(e) => self.status.last = Some((false, format!("Start with Windows: {e}"))),
        }
        self.status.autostart = a.status().ok();
        self.refresh_tray();
    }

    fn toggle_gamepad(&mut self) {
        let mut s = self.settings.clone();
        s.gamepad_enabled = !s.gamepad_enabled;
        match s.save(&self.paths.settings_file()) {
            Ok(()) => tracing::info!("game controllers {}", if s.gamepad_enabled { "on" } else { "off" }),
            Err(e) => self.status.last = Some((false, format!("couldn't save settings: {e}"))),
        }
        // The config watcher would reload too; do it now so the menu updates at once.
        self.reload();
    }

    fn open_overlay(&mut self, event_loop: &ActiveEventLoop, opener: Opener) {
        // The keyboard hotkey's modifiers decide the numbering, so a
        // controller-opened overlay uses the same (or the default) one.
        let trigger = match (self.settings.overlay_hotkey, opener) {
            (Some(h), _) => h,
            (None, Opener::Gamepad) => Hotkey::default_overlay(),
            (None, Opener::Keyboard) => return,
        };
        let profiles = self.store.list().unwrap_or_else(|e| {
            tracing::warn!("profiles: {e}");
            Vec::new()
        });
        // What's actually on screen now, so the check mark and the starting
        // highlight are right even after changes made outside Paneltarium.
        let current = match self.reader.as_ref().map(|r| r.current_layout()) {
            Some(Ok(layout)) => current::detect(&profiles, &layout),
            Some(Err(e)) => {
                tracing::warn!("can't read monitors: {e}");
                Current::Unknown
            }
            None => Current::Unknown,
        };
        tracing::info!("on screen now: {current:?}");
        self.overlay
            .open(event_loop, trigger, opener, &profiles, &current, &mut self.hotkeys);
    }

    fn overlay_input(&mut self, input: Input) {
        let decision = self.overlay.handle(input, &mut self.hotkeys);
        if matches!(input, Input::Hover(_)) {
            tracing::debug!("overlay {input:?} -> {decision:?}");
        } else {
            tracing::info!("overlay {input:?} -> {decision:?}");
        }
        match decision {
            Decision::Apply(name) => self.worker.send(Job::Apply(name)),
            Decision::Rescue => self.worker.send(Job::Rescue),
            Decision::Close | Decision::KeepOpen | Decision::Redraw => {}
        }
    }
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, _event_loop: &ActiveEventLoop) {
        // Called once at start-up on desktop platforms. The tray must be
        // created after the event loop is running.
        if self.tray.is_none() {
            match Tray::new(&self.status) {
                Ok(t) => self.tray = Some(t),
                Err(e) => tracing::error!("{e}"),
            }
            self.reload();
            if self.at_sign_in {
                self.worker.send(Job::SignIn);
            }
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::Hotkey(e) => self.on_hotkey(event_loop, e),
            UserEvent::Pad(e) => self.on_pad(event_loop, e),
            UserEvent::ConfigChanged => self.reload(),
            UserEvent::JobDone(outcome) => {
                self.status.last = Some((outcome.ok, outcome.message));
                self.refresh_tray();
            }
            UserEvent::Menu(e) if e.id == tray::QUIT_ID => {
                tracing::info!("quit from tray");
                self.overlay.close(&mut self.hotkeys);
                self.tray = None; // removes the icon immediately
                event_loop.exit();
            }
            UserEvent::Menu(e) if e.id == tray::GAMEPAD_ID => self.toggle_gamepad(),
            UserEvent::Menu(e) if e.id == tray::AUTOSTART_ID => self.toggle_autostart(),
            UserEvent::Menu(e) if e.id == tray::MANAGER_ID => {
                if let Err(e) = launch_manager() {
                    tracing::error!("{e}");
                    self.status.last = Some((false, e));
                    self.refresh_tray();
                }
            }
            UserEvent::Menu(_) => {}
        }
    }

    fn window_event(&mut self, _: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        if let Some(input) = self.overlay.window_event(id, &event) {
            self.overlay_input(input);
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // Linux: let GTK (the tray menu) handle its pending events.
        #[cfg(target_os = "linux")]
        while gtk::events_pending() {
            gtk::main_iteration_do(false);
        }

        // While the overlay is open, wake up regularly to notice when the
        // modifier keys are released; otherwise sleep until the next event.
        self.overlay.poll(&mut self.hotkeys);
        match self.overlay.next_wake() {
            Some(at) => event_loop.set_control_flow(ControlFlow::WaitUntil(at)),
            None => event_loop.set_control_flow(ControlFlow::Wait),
        }
    }
}

/// Start the profile manager, which ships next to this executable.
/// (If it's already open it simply exits again; see its single-instance lock.)
fn launch_manager() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| format!("can't locate myself: {e}"))?;
    let name = format!("paneltarium-manager{}", std::env::consts::EXE_SUFFIX);
    let manager = exe.with_file_name(&name);
    if !manager.exists() {
        return Err(format!(
            "{name} not found next to {} (run `cargo build` to build it)",
            exe.display()
        ));
    }
    std::process::Command::new(&manager)
        .spawn()
        .map(|_| tracing::info!("started {}", manager.display()))
        .map_err(|e| format!("couldn't start the profile manager: {e}"))
}
