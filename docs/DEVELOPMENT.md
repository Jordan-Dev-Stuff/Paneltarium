# Development

Everything you need to build, run and test Paneltarium from source.

## 1. Install the tools

You need Rust (via rustup), a C/C++ compiler, CMake (SDL 3, used for game
controllers, is compiled from source), and git.

### Windows

In PowerShell:

```powershell
winget install Rustlang.Rustup
winget install Microsoft.VisualStudio.2022.BuildTools --override "--quiet --wait --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
winget install Kitware.CMake
winget install Git.Git
# Optional, only to build the installer locally:
winget install JRSoftware.InnoSetup
```

Close and reopen PowerShell so the new tools are on your PATH, then:

```powershell
rustup default stable
rustup toolchain install 1.97.0 --component clippy,rustfmt   # the version CI uses
cargo --version; cmake --version
```

If `cargo` or `cmake` is "not recognized", add `%USERPROFILE%\.cargo\bin` or
`C:\Program Files\CMake\bin` to your user PATH and open a new terminal.

### Linux (Ubuntu / Debian)

```bash
sudo apt install build-essential cmake pkg-config git \
  libgtk-3-dev libxdo-dev libayatana-appindicator3-dev libudev-dev
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup toolchain install 1.97.0 --component clippy,rustfmt
```

### Linux (Fedora)

```bash
sudo dnf install gcc gcc-c++ cmake pkgconf-pkg-config git \
  gtk3-devel libxdo-devel libayatana-appindicator-gtk3-devel systemd-devel
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup toolchain install 1.97.0 --component clippy,rustfmt
```

The GTK, xdo and appindicator libraries are for the tray icon and global
hotkeys; `libudev` is for game controllers. Switching monitors also needs,
depending on your desktop: nothing extra on GNOME, `kscreen-doctor` on KDE
Plasma (part of Plasma), `wlr-randr` on Sway/Hyprland and other wlroots
compositors, `xrandr` in X11 sessions. On GNOME, enable the
*AppIndicator and KStatusNotifierItem Support* extension to see the tray icon:

```bash
sudo dnf install gnome-shell-extension-appindicator   # Fedora; preinstalled on Ubuntu
gnome-extensions enable appindicatorsupport@rgcjonas.gmail.com
```

On Wayland the first `paneltarium-app` start shows your desktop's
"keyboard shortcuts" dialog; approve it, or hotkeys won't work. A build run
from `target/` writes `~/.local/share/applications/io.github.jordan_dev_stuff.Paneltarium.desktop`
pointing at itself (the shortcut portal needs one); `install.sh` replaces it.

## 2. Get the code and build

```bash
git clone https://github.com/<owner>/Paneltarium.git   # your repository
cd Paneltarium
cargo build              # debug build of everything (first build compiles SDL: a few minutes)
cargo build --release    # optimised build, in target/release
```

## 3. Run

```bash
cargo run -p paneltarium-app        # tray app; the debug build shows its log in a console
cargo run -p paneltarium-manager    # profile manager window
cargo run -p paneltarium -- --help  # command line (arguments after --)
cargo run -p paneltarium -- gamepads --watch   # see what each controller sends
```

### Without touching your real monitors

Point Paneltarium at a fake monitor layout and a scratch settings folder.
Every program then reads and "applies" the TOML file instead of your
displays:

```powershell
# PowerShell
$env:PANELTARIUM_MOCK_LAYOUT = "$PWD\scratch\monitors.toml"
$env:PANELTARIUM_HOME = "$PWD\scratch\config"
```

```bash
# bash
export PANELTARIUM_MOCK_LAYOUT=$PWD/scratch/monitors.toml
export PANELTARIUM_HOME=$PWD/scratch/config
```

A layout file looks like a profile without the name; copy one from
`%APPDATA%\Paneltarium\profiles\` (or `~/.config/paneltarium/profiles/`)
and delete its `name` and `hotkey` lines. Add `--debug` to any command
to print what would be sent to Windows (or, on Linux, to the display driver).

### Linux: checking a display driver

```bash
cargo test -p pt-platform -- --ignored        # GNOME: reads your monitors, verifies (doesn't apply) a few layouts
PANELTARIUM_DISPLAY_BACKEND=x11 cargo run -p paneltarium -- monitors   # force a driver: gnome | kde | wlroots | x11
```

## 4. Check your changes (what CI runs)

```bash
cargo +1.97.0 fmt --all --check
cargo +1.97.0 clippy --all-targets -- -D warnings
cargo +1.97.0 test --all
```

CI (`.github/workflows/ci.yml`) runs these on Windows and Linux for every
push and pull request, pinned to Rust 1.97.0 so a new Rust release can't
break it overnight. To move to a newer Rust, change the version in both
workflow files and here, then fix whatever clippy reports.

## 5. Other tasks

| Task | Command |
|---|---|
| Regenerate the icon files (`assets/`, `docs/icon-preview.png`) | `cargo run -p pt-brand --example export_icons` |
| Build the Linux release archive | `cargo build --release` then `installer/linux/package.sh 0.1.0` (→ `target/installer/`) |
| Build the installer locally (Windows) | `cargo build --release` then `& "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe" /DAppVersion=0.1.0 installer\paneltarium.iss` |
| Publish a release | bump `version` in `Cargo.toml`, commit, `git tag vX.Y.Z`, `git push origin vX.Y.Z` (see the README) |
| Log file | `%APPDATA%\Paneltarium\logs` (Windows), `~/.config/paneltarium/logs` (Linux) |

## Where things live

See [ARCHITECTURE.md](ARCHITECTURE.md) for how the crates fit together and
[ROADMAP.md](ROADMAP.md) for the step-by-step plan with what to check at
each step.
