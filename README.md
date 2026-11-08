<p align="center"><img src="assets/paneltarium-256.png" width="96" alt="Paneltarium icon"></p>

# Paneltarium

**Switch between your monitor setups with one key, or from your game
controller.** A lightweight, fast Windows tray app written in Rust.

Paneltarium doesn't replace Windows' display settings. You still arrange
your monitors the usual way, in **Settings → System → Display** (or your
graphics card's control panel). Paneltarium **saves each arrangement as a
profile** and lets you switch between them instantly: desk, TV only,
everything on, presentation, and so on.

A profile remembers which monitors are on, and each one's resolution,
refresh rate, position, rotation and which one is the main display.

## Switch from the couch 🎮

Gaming on the TV, streaming to a handheld, back to the desk: you don't
need a keyboard for any of it.

- **Hold A + LB + RB + LT + RT** on your controller to bring up your
  profiles, **D-pad** to pick one, **let go** to switch. Press **B** to
  cancel.
- **Works while you play.** Paneltarium listens in the background, and the
  profile list doesn't take focus away from your game. It shows over
  borderless and windowed games; a game in exclusive fullscreen may hide it.
- **Any Xbox-compatible (XInput) controller.** That covers Xbox
  controllers and most PC pads in XInput mode. PlayStation and Switch
  controllers work when they're mapped to XInput, for example by Steam
  Input.
- **Your combination, your call.** Record any button combination in the
  profile manager, or turn controller support off with one switch. Your
  combination is kept for when you turn it back on.

## Get started

Install it with `Paneltarium-<version>-setup.exe` from the GitHub
Releases page (no admin rights needed; tick *Start Paneltarium when I sign
in*). Then:

1. **Set up your monitors** in Windows' display settings the way you want
   them.
2. **Save it as a profile:** right-click the Paneltarium tray icon →
   **Open profile manager…** → **Save current setup**, and give it a name.
   Repeat steps 1–2 for each setup you use.
3. **Switch:** press **Ctrl+Alt+D**, keep holding Ctrl+Alt, and press the
   profile's number (or click it). On a controller, hold
   **A + LB + RB + LT + RT** and use the D-pad.

That's it. Lost a screen and can't see anything? **Ctrl+Alt+Shift+D** shows
the same picture on every monitor.

## Everyday use

| Press | To |
|---|---|
| **Ctrl+Alt+D** | open the profile list: press a number, use the arrows + Enter, or click; let go of Ctrl+Alt to cancel |
| a profile's own hotkey (e.g. Ctrl+Alt+1) | switch straight to that profile |
| **Ctrl+Alt+Shift+D** | rescue: same picture on every monitor |
| controller **A + LB + RB + LT + RT** (hold) | open the profile list from the couch: D-pad to choose, let go to switch, B to cancel |

![Profile overlay](docs/overlay-preview.png)

The **profile manager** (tray → *Open profile manager…*) is where you:

- save, rename, update and delete profiles
- choose their numbers and record hotkeys
- change the app's hotkeys and the controller buttons, or switch
  controllers off
- start Paneltarium with Windows, and choose what happens at sign-in:
  leave the monitors as they are, load the last profile you used, or load a
  specific one
- pick the accent colour

Changes take effect immediately. Your profiles are saved as small files in
`%APPDATA%\Paneltarium\profiles\`, so you can back them up or copy them to
another PC.

![Profile manager](docs/manager-preview.png)

## Light by design

- **Native Rust, no runtime.** No .NET, Electron, Java or browser engine,
  just three small self-contained programs.
- **Idle means idle.** The tray app sleeps until you press a hotkey or click
  the tray. The only background check is for game controllers: 4× a second,
  rising to 120× a second only while one is plugged in. You can switch it
  off entirely.
- **Small footprint.** The pop-up list is drawn by a tiny built-in renderer
  and only exists while it's on screen. The profile manager is a separate
  program that only runs while it's open.
- **Instant switching.** Every monitor changes in one step through the same
  Windows display API the Settings app uses, after Windows has checked the
  change will work.
- **Plain files.** Profiles and settings are small text files you can read,
  back up or sync. The only registry entry is the optional "Start with
  Windows" one.

Measured on Linux with the release build, the tray app idles at about 0.2 %
of one CPU core with about 19 MB of private memory, most of it Linux's tray
library, which Windows doesn't need.

> **Status:** everything above works. Installers are built automatically
> for each release (see *Releases* below); until the first one is
> published, build from source and run `paneltarium-app.exe`. Renamed from
> *Monitor Pilot*: existing profiles and settings carry over on first run.

---

## Advanced

### Not saved in profiles

Display scaling (%), HDR and colour profiles are left to Windows, which
remembers them per monitor. If a monitor in a profile isn't plugged in, the
others are still switched on and slid together so there are no gaps.

### Command line

Everything the app does is also available from `paneltarium.exe`:

```powershell
paneltarium monitors                         # what's connected right now
paneltarium save "Desk" --hotkey Ctrl+Alt+1  # save the current setup
paneltarium profiles
paneltarium apply "TV only" --dry-run        # ask Windows first, change nothing
paneltarium apply "TV only"
paneltarium duplicate                        # same picture on every monitor
paneltarium extend
paneltarium settings --overlay-hotkey Ctrl+Alt+D --duplicate-hotkey Ctrl+Alt+Shift+D
paneltarium settings --gamepad off           # or --gamepad-overlay A+LB+RB+LT+RT
paneltarium settings --startup last          # at sign-in: none | last | a profile name
paneltarium settings --accent "#7C3AED"      # accent colour (or "default")
paneltarium autostart on                     # start the tray app when you sign in
paneltarium paths                            # where profiles live
```

Add `--json` for machine-readable output, or `--debug` to print what's sent
to Windows.

### At sign-in

With "Start with Windows" on, the sign-in choice (`--startup`) is only
applied when the app was started by signing in. It waits up to 30 s for the
profile's monitors to appear (TVs and docks can be slow) and does nothing if
that setup is already on screen.

### Files

Everything lives in `%APPDATA%\Paneltarium`
(`C:\Users\<you>\AppData\Roaming\Paneltarium`); `paneltarium paths` prints
the exact folders.

- Profiles: `profiles\*.toml`, one per profile.
- Settings: `settings.toml`.
- Logs: `logs\`.

A profile file that can't be read is skipped and reported in the tray and
manager; the others keep working.

### Build from source

Requires Rust 1.90+ (`rustup update`) and, on Windows, the MSVC toolchain.

```powershell
cargo build --release                # all three programs, in target\release
cargo run -p paneltarium-app         # dev build with a console showing the log
```

### Releases

Pushing a version tag builds and publishes a release (`.github/workflows/release.yml`):

```powershell
# 1. set `version` in Cargo.toml (e.g. 0.2.0) and commit it
git tag v0.2.0
git push origin v0.2.0
```

GitHub Actions then runs the tests, builds the three programs, and attaches
these to a new GitHub Release:

- `Paneltarium-0.2.0-setup.exe`: per-user installer that offers *Start
  Paneltarium when I sign in*, adds Start-menu shortcuts and closes a running
  copy when upgrading.
- `Paneltarium-0.2.0-portable.zip`: the programs on their own.
- `SHA256SUMS.txt`: checksums for both.

Uninstalling keeps your profiles. Every push and pull request also runs
`.github/workflows/ci.yml` (formatting, lints and tests on Windows and
Linux). To build the installer locally, install [Inno Setup 6](https://jrsoftware.org/isinfo.php), then:

```powershell
cargo build --release
& "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe" /DAppVersion=0.1.0 installer\paneltarium.iss   # → target\installer\
```

### Project layout

```
crates/
  pt-core/              OS-neutral: model, profiles, hotkeys, matching, theme, traits, mock
  pt-platform/          OS-specific code (src/windows/ today)
  pt-gamepad/           reads game controllers on a background thread (gilrs)
  pt-brand/             the app icon (planet in a display), drawn at any size
  paneltarium/          command line
  paneltarium-app/      tray app: hotkeys, profile overlay, tray icon
  paneltarium-manager/  profile manager window (egui)
assets/                 paneltarium.ico (embedded in the .exe files) and a 256 px PNG
installer/              Inno Setup script for the Windows installer
.github/workflows/      ci.yml (every push) and release.yml (version tags)
docs/
  ARCHITECTURE.md       how the pieces fit, key decisions
  ROADMAP.md            testable steps, with what to check at each one
```

### Tests

```powershell
cargo test            # 157 tests on Windows (151 elsewhere), no real monitors needed
cargo clippy --all-targets
```

The tests use a fake monitor backend that rejects the same layouts Windows
would. You can use it by hand too: set `$env:PANELTARIUM_MOCK_LAYOUT` to a
TOML layout file and `$env:PANELTARIUM_HOME` to a scratch folder.

## License

Paneltarium is open source under the [MIT License](LICENSE).

It builds on other open-source work, including the
[Phosphor Icons](https://phosphoricons.com) (MIT), the Ubuntu font bundled
with [egui](https://github.com/emilk/egui) (Ubuntu Font Licence), and the Rust
crates listed in `Cargo.lock`, each under its own licence.
