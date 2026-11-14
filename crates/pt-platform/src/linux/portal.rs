//! Global hotkeys on Wayland: the XDG GlobalShortcuts portal
//! (`org.freedesktop.portal.GlobalShortcuts`; GNOME 48+, KDE Plasma 5.27+,
//! Hyprland). Apps can't grab keys on Wayland, so Paneltarium lists its
//! shortcuts and the desktop reports when they're pressed and released.
//! The desktop asks you to approve them the first time (and whenever the
//! list changes); its keyboard settings can change the keys afterwards.
//!
//! One thread binds (a portal session can bind only once, so a new list
//! means a new session), another delivers `Activated`/`Deactivated`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};

use pt_core::hotkey::NamedKey;
use pt_core::{Error, Hotkey, Key, Result, Shortcut, ShortcutEvent, ShortcutService};
use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};

const DESTINATION: &str = "org.freedesktop.portal.Desktop";
const PATH: &str = "/org/freedesktop/portal/desktop";
const INTERFACE: &str = "org.freedesktop.portal.GlobalShortcuts";

type Callback = Arc<dyn Fn(ShortcutEvent) + Send + Sync>;

pub struct PortalShortcuts {
    requests: Mutex<Sender<Vec<Shortcut>>>,
}

impl ShortcutService for PortalShortcuts {
    fn bind(&self, shortcuts: Vec<Shortcut>) {
        if let Ok(tx) = self.requests.lock() {
            let _ = tx.send(shortcuts);
        }
    }
}

impl PortalShortcuts {
    /// Connect to the portal; fails if the desktop doesn't offer it.
    pub fn start(app_id: &str, on_event: impl Fn(ShortcutEvent) + Send + Sync + 'static) -> Result<Self> {
        let conn = Connection::session().map_err(|e| failed("session bus", e))?;
        register(&conn, app_id);
        let portal = proxy(&conn, PATH, INTERFACE)?;
        portal
            .get_property::<u32>("version")
            .map_err(|_| failed(INTERFACE, "this desktop doesn't offer global shortcuts"))?;

        let on_event: Callback = Arc::new(on_event);
        let session: Arc<Mutex<Option<OwnedObjectPath>>> = Arc::default();

        // Pressed / released.
        let mut signals = portal.receive_all_signals().map_err(|e| failed(INTERFACE, e))?;
        let (events, current) = (on_event.clone(), session.clone());
        std::thread::Builder::new()
            .name("shortcut-signals".into())
            .spawn(move || {
                for msg in &mut signals {
                    let member = msg.header().member().map(|m| m.to_string()).unwrap_or_default();
                    let Ok((handle, id, ..)) = msg
                        .body()
                        .deserialize::<(OwnedObjectPath, String, u64, HashMap<String, OwnedValue>)>()
                    else {
                        continue;
                    };
                    if current.lock().ok().and_then(|s| s.clone()) != Some(handle) {
                        continue; // an older session's
                    }
                    match member.as_str() {
                        "Activated" => events(ShortcutEvent::Pressed(id)),
                        "Deactivated" => events(ShortcutEvent::Released(id)),
                        _ => {}
                    }
                }
            })
            .map_err(|e| failed("thread", e))?;

        // Binding.
        let (tx, rx) = mpsc::channel::<Vec<Shortcut>>();
        std::thread::Builder::new()
            .name("shortcut-binder".into())
            .spawn(move || {
                let mut bound: Option<Vec<Shortcut>> = None;
                while let Ok(mut wanted) = rx.recv() {
                    // Only the latest list matters.
                    while let Ok(newer) = rx.try_recv() {
                        wanted = newer;
                    }
                    if bound.as_ref() == Some(&wanted) {
                        continue;
                    }
                    if let Some(old) = session.lock().ok().and_then(|mut s| s.take()) {
                        let _ = conn.call_method(
                            Some(DESTINATION),
                            old.as_str(),
                            Some("org.freedesktop.portal.Session"),
                            "Close",
                            &(),
                        );
                    }
                    match bind(&conn, &wanted) {
                        Ok((handle, count)) => {
                            if let Ok(mut s) = session.lock() {
                                *s = Some(handle);
                            }
                            on_event(ShortcutEvent::Bound(count));
                        }
                        Err(e) => on_event(ShortcutEvent::Failed(e)),
                    }
                    bound = Some(wanted);
                }
            })
            .map_err(|e| failed("thread", e))?;
        Ok(PortalShortcuts {
            requests: Mutex::new(tx),
        })
    }
}

/// Tell the portal who we are; it needs an app id to remember the approval.
/// Must come before any other portal call on this connection; older
/// portals don't have it (then they guess). The portal only accepts ids
/// with an installed `.desktop` file: the install script puts one in place;
/// when running from a build folder, write one here.
fn register(conn: &Connection, app_id: &str) {
    let call = || {
        conn.call_method(
            Some(DESTINATION),
            PATH,
            Some("org.freedesktop.host.portal.Registry"),
            "Register",
            &(app_id, HashMap::<&str, Value>::new()),
        )
    };
    let Err(e) = call() else { return };
    if !e.to_string().contains("not found") {
        return;
    }
    let (Some(data), Ok(exe)) = (dirs::data_dir(), std::env::current_exe()) else {
        return;
    };
    let file = data.join("applications").join(format!("{app_id}.desktop"));
    if file.exists() {
        return; // there, but not good enough for the portal; leave it alone
    }
    let written = std::fs::create_dir_all(file.parent().expect("has a parent"))
        .and_then(|()| std::fs::write(&file, desktop_entry(app_id, &exe)));
    if written.is_err() {
        return;
    }
    // The portal notices new desktop files after a moment.
    for _ in 0..20 {
        std::thread::sleep(std::time::Duration::from_millis(150));
        if call().is_ok() {
            return;
        }
    }
}

/// The `.desktop` entry for the background app at `exe` (the same as the
/// one the Linux install script ships, but pointing at this executable).
pub fn desktop_entry(app_id: &str, exe: &std::path::Path) -> String {
    format!(
        "[Desktop Entry]\nType=Application\nName=Paneltarium\n\
         Comment=Switch monitor profiles with a hotkey or a game controller\n\
         Exec=\"{}\"\nIcon={app_id}\nTerminal=false\nCategories=Utility;Settings;HardwareSettings;\n\
         Keywords=monitor;display;screen;profile;hotkey;\n",
        exe.display()
    )
}

fn failed(call: &'static str, e: impl std::fmt::Display) -> Error {
    Error::Os {
        call,
        code: None,
        message: e.to_string(),
    }
}

fn proxy<'a>(conn: &Connection, path: &'a str, interface: &'a str) -> Result<Proxy<'a>> {
    zbus::blocking::proxy::Builder::new(conn)
        .destination(DESTINATION)
        .and_then(|b| b.path(path))
        .and_then(|b| b.interface(interface))
        .map(|b| b.cache_properties(zbus::proxy::CacheProperties::No))
        .and_then(|b| b.build())
        .map_err(|e| failed(INTERFACE, e))
}

fn token() -> String {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    format!(
        "paneltarium_{}_{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

/// Make a portal request and wait for its `Response`. `call` gets the
/// `handle_token` to pass.
fn request(
    conn: &Connection,
    call: impl FnOnce(&str) -> zbus::Result<OwnedObjectPath>,
) -> std::result::Result<HashMap<String, OwnedValue>, String> {
    let token = token();
    let sender = conn
        .unique_name()
        .map(|n| n.trim_start_matches(':').replace('.', "_"))
        .ok_or("not connected")?;
    let path = format!("{PATH}/request/{sender}/{token}");
    // Listen before calling, or a quick answer could be missed.
    let req = proxy(conn, &path, "org.freedesktop.portal.Request").map_err(|e| e.to_string())?;
    let mut responses = req.receive_signal("Response").map_err(|e| e.to_string())?;
    call(&token).map_err(|e| e.to_string())?;
    let msg = responses.next().ok_or("the portal went away")?;
    let (code, results): (u32, HashMap<String, OwnedValue>) = msg.body().deserialize().map_err(|e| e.to_string())?;
    match code {
        0 => Ok(results),
        1 => Err("the shortcuts were not approved".into()),
        _ => Err("the desktop couldn't set up the shortcuts".into()),
    }
}

/// Create a session and bind `shortcuts` to it; its handle and how many
/// the desktop accepted.
fn bind(conn: &Connection, shortcuts: &[Shortcut]) -> std::result::Result<(OwnedObjectPath, usize), String> {
    let created = request(conn, |token| {
        let options = HashMap::from([
            ("handle_token", Value::from(token)),
            ("session_handle_token", Value::from(token)),
        ]);
        conn.call_method(Some(DESTINATION), PATH, Some(INTERFACE), "CreateSession", &(options,))?
            .body()
            .deserialize()
    })?;
    let handle = created
        .get("session_handle")
        .and_then(|v| match &**v {
            Value::Str(s) => OwnedObjectPath::try_from(s.as_str()).ok(),
            Value::ObjectPath(p) => Some(p.clone().into()),
            _ => None,
        })
        .ok_or("the portal returned no session")?;

    let list: Vec<(String, HashMap<&str, Value>)> = shortcuts
        .iter()
        .map(|s| {
            (
                s.id.clone(),
                HashMap::from([
                    ("description", Value::from(s.description.as_str())),
                    ("preferred_trigger", Value::from(trigger(&s.hotkey))),
                ]),
            )
        })
        .collect();
    let session = ObjectPath::try_from(handle.as_str()).map_err(|e| e.to_string())?;
    let results = request(conn, |token| {
        let options = HashMap::from([("handle_token", Value::from(token))]);
        conn.call_method(
            Some(DESTINATION),
            PATH,
            Some(INTERFACE),
            "BindShortcuts",
            &(&session, &list, "", options),
        )?
        .body()
        .deserialize()
    })?;
    let count = results
        .get("shortcuts")
        .and_then(|v| match &**v {
            Value::Array(a) => Some(a.len()),
            _ => None,
        })
        .unwrap_or(shortcuts.len());
    Ok((handle, count))
}

/// A hotkey in the XDG shortcut format: `CTRL+ALT+d`, `LOGO+F1`,
/// `CTRL+ALT+KP_1` (modifiers, then the key's XKB keysym name).
pub fn trigger(hotkey: &Hotkey) -> String {
    let m = hotkey.modifiers;
    let mut parts: Vec<String> = [(m.ctrl, "CTRL"), (m.alt, "ALT"), (m.shift, "SHIFT"), (m.win, "LOGO")]
        .into_iter()
        .filter(|(on, _)| *on)
        .map(|(_, name)| name.to_string())
        .collect();
    parts.push(keysym(hotkey.key));
    parts.join("+")
}

fn keysym(key: Key) -> String {
    match key {
        Key::Letter(c) => c.to_ascii_lowercase().to_string(),
        Key::Digit(d) => d.to_string(),
        Key::F(n) => format!("F{n}"),
        Key::Numpad(d) => format!("KP_{d}"),
        Key::Named(n) => match n {
            NamedKey::Up => "Up",
            NamedKey::Down => "Down",
            NamedKey::Left => "Left",
            NamedKey::Right => "Right",
            NamedKey::Home => "Home",
            NamedKey::End => "End",
            NamedKey::PageUp => "Page_Up",
            NamedKey::PageDown => "Page_Down",
            NamedKey::Insert => "Insert",
            NamedKey::Delete => "Delete",
            NamedKey::Space => "space",
            NamedKey::Minus => "minus",
            NamedKey::Equal => "equal",
            NamedKey::Comma => "comma",
            NamedKey::Period => "period",
            NamedKey::Slash => "slash",
            NamedKey::Backslash => "backslash",
            NamedKey::Semicolon => "semicolon",
            NamedKey::Quote => "apostrophe",
            NamedKey::Backquote => "grave",
            NamedKey::BracketLeft => "bracketleft",
            NamedKey::BracketRight => "bracketright",
            NamedKey::Enter => "Return",
            NamedKey::Escape => "Escape",
        }
        .to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> String {
        trigger(&s.parse().unwrap())
    }

    #[test]
    fn triggers_use_the_xdg_format() {
        assert_eq!(t("Ctrl+Alt+D"), "CTRL+ALT+d");
        assert_eq!(t("Ctrl+Alt+Shift+D"), "CTRL+ALT+SHIFT+d");
        assert_eq!(t("Win+F1"), "LOGO+F1");
        assert_eq!(t("Ctrl+Alt+1"), "CTRL+ALT+1");
        assert_eq!(t("Ctrl+Alt+Num1"), "CTRL+ALT+KP_1");
        assert_eq!(t("Ctrl+Alt+PageUp"), "CTRL+ALT+Page_Up");
        assert_eq!(t("Ctrl+Alt+Enter"), "CTRL+ALT+Return");
        assert_eq!(t("Ctrl+Alt+Esc"), "CTRL+ALT+Escape");
        assert_eq!(t("Ctrl+Alt+'"), "CTRL+ALT+apostrophe");
    }

    #[test]
    fn desktop_entry_points_at_the_executable() {
        let text = desktop_entry("org.example.App", std::path::Path::new("/opt/pt/paneltarium-app"));
        assert!(text.starts_with("[Desktop Entry]\n"));
        assert!(text.contains("Exec=\"/opt/pt/paneltarium-app\"\n"), "{text}");
        assert!(text.contains("Icon=org.example.App\n"));
    }

    #[test]
    fn tokens_are_unique_and_valid_path_elements() {
        let (a, b) = (token(), token());
        assert_ne!(a, b);
        assert!(a.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'), "{a}");
    }
}
