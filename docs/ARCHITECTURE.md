# Architecture

```
            ┌───────────────────┐ ┌───────────────────┐ ┌────────────────────┐
front ends  │ paneltarium       │ │ paneltarium-app   │ │ paneltarium-       │
            │ (CLI, clap)       │ │ hotkeys · overlay │ │ manager (egui)     │
            │                   │ │ · tray            │ │ profiles, hotkeys  │
            └─────────┬─────────┘ └─────────┬─────────┘ └─────────┬──────────┘
                      │        all only talk to pt-core           │
                      ▼                     ▼                     ▼
            ┌───────────────────────────────────────────────────────┐
pt-core     │ MonitorService (facade)                               │
(no OS      │  ├─ ProfileStore  profiles/*.toml                     │
 calls)     │  ├─ Settings      settings.toml                       │
            │  ├─ plan()        saved profile → connected monitors  │
            │  ├─ slots         overlay numbers 1–9 (shared)        │
            │  ├─ gamepad       buttons, combinations, tracking     │
            │  └─ dyn DisplayBackend  ◄── trait = the OS seam       │
            └───────────────────────────┬───────────────────────────┘
                                        │ implemented by
            ┌───────────────────────────▼───────────────────────────┐
pt-platform │ windows/  ccd.rs (all unsafe Win32 calls)             │
            │           display.rs (WindowsDisplayBackend)          │
            │ linux/    autostart only so far                       │
            │ unsupported.rs (so everything builds and tests on CI) │
            └───────────────────────────────────────────────────────┘

pt-gamepad  background controller reader (gilrs; XInput on Windows),
            used by the app (overlay) and the manager (recording).
pt-brand    the app icon (Phosphor "monitor" + "planet", purple tile), drawn at
            any size: tray, overlay, manager, and assets/paneltarium.ico.
```

## Rules

1. **Only `pt-platform` calls OS APIs.** Each OS lives in its own module
   behind `#[cfg(...)]`. Adding Linux = new module + one `cfg` arm in
   `pt-platform/src/lib.rs`.
2. **New OS-specific needs get a trait in `pt-core`** (`DisplayBackend`,
   `KeyboardState`, `Autostart`), implemented per OS in
   `pt-platform`. The front ends otherwise use cross-platform crates
   (winit, global-hotkey, tray-icon, softbuffer); the only OS-specific lines
   outside `pt-platform` are window-style tweaks behind `#[cfg(windows)]`.
3. **Front ends only use `MonitorService`.** The CLI, hotkey handler and UI
   all apply profiles the same way, so the CLI tests cover the real logic.
4. **Logic that can be pure, is.** Matching saved monitors to connected ones,
   picking the primary and rebasing positions all live in `pt_core::plan` and
   are unit-tested without hardware.

## Key decisions

- **Monitor identity**: device path first, EDID model + name second (only
  when unambiguous), then identical models paired in order when the counts
  match (e.g. two identical monitors moved to another dock). GPU `LUID`s
  change every boot, so they are never saved.
- **Primary monitor** = the one at (0, 0); `plan()` shifts the layout so the
  saved primary lands there. If it isn't connected, the monitor that was
  nearest to it takes over.
- **Missing monitors leave no gaps**: Windows needs every monitor to touch
  another, so `plan()` slides the rest together (left–middle–right without
  the middle → left–right). `plan::check_arrangement` states the rules
  (one primary at the origin, no partial overlaps, no islands); the fake
  backend enforces them, so tests fail on layouts Windows would reject.
- **Duplicating**: monitors with identical rectangles are a clone group and
  share one Windows source (`display.rs` `shared_source`), whether the
  profile was saved while duplicated or the rescue hotkey made it so.
- **Accent colour** is a setting (`accent_color = "#7C3AED"`);
  `pt_core::Theme` derives the rest (readable text on it, the dark selected
  tint) so the overlay and manager always agree. The app icon itself stays
  purple.
- **Broken profile files are skipped, not fatal**: `ProfileStore::list_checked`
  returns the good profiles plus one error per unreadable file; the tray,
  manager and CLI show those as warnings.
- **Monitors not in a profile are turned off**, matching what the profile
  showed when it was saved. If that would turn everything off, applying fails
  instead.
- **Profiles are plain TOML**, one file each, safe to edit by hand or keep in git.
- **Not a Windows service** — see ROADMAP step 9.
- **Renamed from Monitor Pilot**: on first run the old config folder is
  copied (`paths::migrate_legacy`) and an old "Start with Windows" entry is
  replaced (`Autostart::migrate_legacy`).
- **Cleared settings are written as `""`**, not left out: a missing key
  means "use the default".
- **Testing without hardware**: set `PANELTARIUM_MOCK_LAYOUT=<file.toml>` and
  the CLI uses a file-backed fake backend; `PANELTARIUM_HOME=<dir>` moves
  all config files.

## Profile file example

```toml
name = "TV only"
hotkey = "Ctrl+Alt+2"

[[monitors]]
enabled = true
primary = true
rotation = "none"

[monitors.id]
device_path = '\\?\DISPLAY#GSM5BBF#5&1a2b3c&0&UID4352#{e6f07b5f-ee97-4a90-b076-33f57bf4eaa7}'
name = "LG TV"
edid_manufacturer = 7789
edid_product = 23487

[monitors.mode]
width = 3840
height = 2160

[monitors.mode.refresh]
numerator = 60
denominator = 1

[monitors.position]
x = 0
y = 0

[[monitors]]
enabled = false
# ...
```
