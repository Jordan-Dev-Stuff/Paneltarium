//! `paneltarium` command-line tool.
//!
//! Every command goes through `pt_core::MonitorService`, the same facade the
//! tray app and hotkeys will use, so the CLI doubles as a test harness for
//! the core and the platform backend.

mod cli;
mod output;

use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use clap::Parser;
use pt_core::AutostartStatus;
use pt_core::gamepad::GamepadChord;
use pt_core::startup::StartupProfile;
use pt_core::{AppPaths, Hotkey, MonitorService, Topology};

use cli::{AutostartAction, Cli, Command, OnOff};

fn main() -> ExitCode {
    let cli = Cli::parse();
    if cli.debug {
        // SAFETY: still single-threaded; nothing else reads the environment yet.
        unsafe { std::env::set_var("PANELTARIUM_DEBUG", "1") };
    }
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn parse_hotkey(s: &str) -> Result<Hotkey> {
    Ok(s.parse::<Hotkey>()?)
}

/// `None` for "none"/"off"/"" (clear), otherwise a parsed hotkey.
fn parse_optional_hotkey(s: &str) -> Result<Option<Hotkey>> {
    match s.trim().to_ascii_lowercase().as_str() {
        "" | "none" | "off" => Ok(None),
        _ => parse_hotkey(s).map(Some),
    }
}

fn print_json<T: serde::Serialize>(value: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

fn run(cli: Cli) -> Result<()> {
    let svc = MonitorService::new(pt_platform::backend_from_env()?, AppPaths::from_env()?);

    match cli.command {
        Command::Monitors => {
            let layout = svc.current_layout()?;
            if cli.json {
                print_json(&layout)?;
            } else {
                println!("{}", output::layout(&layout));
            }
        }

        Command::Modes { monitor } => {
            let layout = svc.current_layout()?;
            let m = select_monitor(&layout, &monitor)?;
            let modes = svc.backend().supported_modes(&m.id)?;
            if cli.json {
                print_json(&modes)?;
            } else {
                println!("Modes for {}:", m.id.label());
                for mode in modes {
                    let current = if Some(mode) == m.mode { "  (current)" } else { "" };
                    println!("  {mode}{current}");
                }
            }
        }

        Command::Profiles => {
            let (profiles, problems) = svc.store().list_checked()?;
            for e in &problems {
                eprintln!("warning: skipped a profile: {e}");
            }
            if cli.json {
                print_json(&profiles)?;
            } else {
                println!("{}", output::profiles(&profiles));
            }
        }

        Command::Show { profile } => {
            let p = svc.store().get(&profile)?;
            if cli.json {
                print_json(&p)?;
            } else {
                println!("{}", output::profile(&p));
            }
        }

        Command::Save {
            name,
            hotkey,
            overwrite,
        } => {
            let hotkey = hotkey.as_deref().map(parse_hotkey).transpose()?;
            let p = svc.save_current(&name, hotkey, overwrite)?;
            println!("Saved profile '{}': {}", p.name, p.layout.summary());
            if let Some(h) = p.hotkey {
                println!("Hotkey: {h}");
            }
        }

        Command::Apply { profile, dry_run } => {
            let r = svc.apply_profile(&profile, dry_run)?;
            let on = r.plan.enabled().count();
            let off = r.plan.monitors.len() - on;
            if r.dry_run {
                println!(
                    "Dry run: '{}' would turn {on} monitor(s) on and {off} off. Nothing was changed.",
                    r.profile
                );
            } else {
                println!("Applied '{}': {on} monitor(s) on, {off} off.", r.profile);
            }
            for d in &r.differences {
                println!("  note: {d}");
            }
            for m in &r.plan.missing {
                println!("  note: '{}' is in the profile but not connected", m.label());
            }
            for m in &r.plan.unknown {
                println!("  note: '{}' is not in the profile, so it was turned off", m.label());
            }
        }

        Command::Duplicate => match svc.rescue().context("could not duplicate or extend the displays")? {
            Topology::Clone => println!("Duplicating the display on all monitors."),
            Topology::Extend => {
                println!("These monitors can't be duplicated, so the desktop was extended across all of them.")
            }
        },

        Command::Extend => {
            svc.set_topology(Topology::Extend)?;
            println!("Extended the desktop across all monitors.");
        }

        Command::Delete { profile } => {
            svc.store().delete(&profile)?;
            println!("Deleted profile '{profile}'.");
        }

        Command::Hotkey { profile, hotkey, clear } => {
            let hotkey = match (hotkey, clear) {
                (Some(h), false) => Some(parse_hotkey(&h)?),
                (None, true) => None,
                (None, false) => {
                    let p = svc.store().get(&profile)?;
                    match p.hotkey {
                        Some(h) => println!("'{}' hotkey: {h}", p.name),
                        None => println!("'{}' has no hotkey.", p.name),
                    }
                    return Ok(());
                }
                (Some(_), true) => bail!("give a hotkey or --clear, not both"),
            };
            let p = svc.set_hotkey(&profile, hotkey)?;
            match p.hotkey {
                Some(h) => println!("'{}' hotkey set to {h}.", p.name),
                None => println!("'{}' hotkey cleared.", p.name),
            }
        }

        Command::Settings {
            duplicate_hotkey,
            extend_hotkey,
            overlay_hotkey,
            gamepad_overlay,
            gamepad,
            startup,
            accent,
        } => {
            let mut s = svc.settings()?;
            let changing = startup.is_some()
                || accent.is_some()
                || duplicate_hotkey.is_some()
                || extend_hotkey.is_some()
                || overlay_hotkey.is_some()
                || gamepad_overlay.is_some()
                || gamepad.is_some();
            if let Some(v) = gamepad_overlay {
                s.gamepad_overlay = match v.trim().to_ascii_lowercase().as_str() {
                    "" | "none" | "off" => None,
                    _ => Some(v.parse::<GamepadChord>()?),
                };
            }
            if let Some(g) = gamepad {
                s.gamepad_enabled = g == OnOff::On;
            }
            if let Some(v) = startup {
                s.startup_profile = match v.trim().to_ascii_lowercase().as_str() {
                    "" | "none" | "off" => StartupProfile::None,
                    "last" => StartupProfile::Last,
                    _ => {
                        // A profile name: must exist; store its exact spelling.
                        let p = svc.store().get(v.trim())?;
                        StartupProfile::Profile(p.name)
                    }
                };
            }
            if let Some(v) = accent {
                s.accent_color = match v.trim().to_ascii_lowercase().as_str() {
                    "" | "default" | "reset" => pt_core::Color::DEFAULT_ACCENT,
                    _ => v.parse().map_err(anyhow::Error::msg)?,
                };
            }
            if let Some(v) = duplicate_hotkey {
                s.duplicate_hotkey = parse_optional_hotkey(&v)?;
            }
            if let Some(v) = extend_hotkey {
                s.extend_hotkey = parse_optional_hotkey(&v)?;
            }
            if let Some(v) = overlay_hotkey {
                s.overlay_hotkey = parse_optional_hotkey(&v)?;
            }
            if changing {
                s.check_conflicts(svc.store())?;
                svc.save_settings(&s)?;
            }
            if cli.json {
                print_json(&s)?;
            } else {
                let show = |h: Option<Hotkey>| h.map(|h| h.to_string()).unwrap_or_else(|| "(none)".into());
                println!("duplicate_hotkey   = {}", show(s.duplicate_hotkey));
                println!("extend_hotkey      = {}", show(s.extend_hotkey));
                println!("overlay_hotkey     = {}", show(s.overlay_hotkey));
                println!(
                    "gamepad_overlay    = {}",
                    s.gamepad_overlay
                        .as_ref()
                        .map(|c| c.to_string())
                        .unwrap_or_else(|| "(none)".into())
                );
                println!("gamepad            = {}", if s.gamepad_enabled { "on" } else { "off" });
                println!("startup_profile    = {}", s.startup_profile);
                println!("accent_color       = {}", s.accent_color);
            }
        }

        Command::Autostart { action } => {
            let a = pt_platform::autostart()?;
            match action {
                AutostartAction::On => {
                    let exe = pt_core::paths::sibling_exe("paneltarium-app")?;
                    if !exe.exists() {
                        bail!("{} not found (run `cargo build` first)", exe.display());
                    }
                    a.enable(&exe)?;
                    println!("Paneltarium will start when you sign in ({}).", exe.display());
                }
                AutostartAction::Off => {
                    a.disable()?;
                    println!("Paneltarium won't start at sign-in.");
                }
                AutostartAction::Status => {}
            }
            let s = svc.settings()?;
            match a.status()? {
                AutostartStatus::On => println!("Start at sign-in: on"),
                AutostartStatus::Off => println!("Start at sign-in: off"),
                AutostartStatus::DisabledInSystem => println!(
                    "Start at sign-in: off (switched off in Task Manager → Startup apps; `autostart on` re-enables it)"
                ),
            }
            println!(
                "At sign-in: {}",
                match &s.startup_profile {
                    StartupProfile::None => "leave the monitors as Windows set them up".to_string(),
                    StartupProfile::Last => format!(
                        "load the last used profile (now: {})",
                        svc.last_profile().unwrap_or_else(|| "none yet".into())
                    ),
                    StartupProfile::Profile(n) => format!("load '{n}'"),
                }
            );
        }

        Command::Paths => {
            let p = svc.paths();
            println!("config:   {}", p.root().display());
            println!("profiles: {}", p.profiles_dir().display());
            println!("settings: {}", p.settings_file().display());
            println!("backend:  {}", svc.backend().name());
        }
    }
    Ok(())
}

/// Pick a monitor by its 1-based number in `monitors`, or by (part of) its name.
fn select_monitor<'a>(layout: &'a pt_core::DisplayLayout, query: &str) -> Result<&'a pt_core::MonitorState> {
    if let Ok(n) = query.parse::<usize>() {
        return layout
            .monitors
            .get(n.wrapping_sub(1))
            .with_context(|| format!("there is no monitor #{n}; see `paneltarium monitors`"));
    }
    let q = query.to_lowercase();
    let hits: Vec<_> = layout
        .monitors
        .iter()
        .filter(|m| m.id.name.to_lowercase().contains(&q) || m.id.device_path.eq_ignore_ascii_case(query))
        .collect();
    match hits.as_slice() {
        [one] => Ok(one),
        [] => bail!("no monitor matches '{query}'; see `paneltarium monitors`"),
        _ => bail!(
            "'{query}' matches {} monitors; use the number from `paneltarium monitors`",
            hits.len()
        ),
    }
}
