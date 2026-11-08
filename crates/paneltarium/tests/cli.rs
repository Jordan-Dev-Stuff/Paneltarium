//! End-to-end tests of the CLI against the file-backed mock backend.
//! They run on any OS (no real monitors involved).

use assert_cmd::Command;
use predicates::prelude::*;
use pt_core::DisplayLayout;
use pt_core::testing::sample_layout;
use tempfile::TempDir;

struct Env {
    dir: TempDir,
}

impl Env {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("monitors.toml"),
            toml::to_string(&sample_layout()).unwrap(),
        )
        .unwrap();
        Env { dir }
    }

    fn cmd(&self, args: &[&str]) -> assert_cmd::assert::Assert {
        Command::cargo_bin("paneltarium")
            .unwrap()
            .env("PANELTARIUM_HOME", self.dir.path().join("config"))
            .env("PANELTARIUM_MOCK_LAYOUT", self.dir.path().join("monitors.toml"))
            .args(args)
            .assert()
    }

    fn layout(&self) -> DisplayLayout {
        toml::from_str(&std::fs::read_to_string(self.dir.path().join("monitors.toml")).unwrap()).unwrap()
    }
}

#[test]
fn lists_monitors() {
    let env = Env::new();
    env.cmd(&["monitors"])
        .success()
        .stdout(predicate::str::contains("Monitor LEFT"))
        .stdout(predicate::str::contains("3840x2160 @ 60 Hz"))
        .stdout(predicate::str::contains("off"));
    env.cmd(&["--json", "list"])
        .success()
        .stdout(predicate::str::contains("\"device_path\": \"LEFT\""));
}

#[test]
fn save_duplicate_apply_round_trip() {
    let env = Env::new();
    env.cmd(&["save", "Desk", "--hotkey", "ctrl+alt+1"])
        .success()
        .stdout(predicate::str::contains("Saved profile 'Desk'"))
        .stdout(predicate::str::contains("Ctrl+Alt+1"));

    env.cmd(&["duplicate"]).success();
    assert_eq!(env.layout().enabled().count(), 3);

    env.cmd(&["apply", "desk", "--dry-run"])
        .success()
        .stdout(predicate::str::contains("Nothing was changed"));
    assert_eq!(env.layout().enabled().count(), 3);

    env.cmd(&["apply", "Desk"])
        .success()
        .stdout(predicate::str::contains("2 monitor(s) on, 1 off"));
    let now = env.layout();
    assert_eq!(now.enabled().count(), 2);
    assert!(now.monitors.iter().any(|m| m.id.device_path == "TV" && !m.enabled));
}

#[test]
fn profiles_show_hotkey_delete() {
    let env = Env::new();
    env.cmd(&["profiles"])
        .success()
        .stdout(predicate::str::contains("No profiles yet"));
    env.cmd(&["save", "Gaming"]).success();
    env.cmd(&["save", "Gaming"])
        .failure()
        .stderr(predicate::str::contains("already exists"));
    env.cmd(&["save", "Gaming", "-f", "--hotkey", "Ctrl+Alt+G"]).success();

    env.cmd(&["profiles"])
        .success()
        .stdout(predicate::str::contains("Gaming"))
        .stdout(predicate::str::contains("Ctrl+Alt+G"));
    env.cmd(&["show", "gaming"])
        .success()
        .stdout(predicate::str::contains("Hotkey:  Ctrl+Alt+G"));

    env.cmd(&["hotkey", "Gaming", "--clear"])
        .success()
        .stdout(predicate::str::contains("cleared"));
    env.cmd(&["hotkey", "Gaming"])
        .success()
        .stdout(predicate::str::contains("has no hotkey"));

    env.cmd(&["delete", "Gaming"]).success();
    env.cmd(&["show", "Gaming"])
        .failure()
        .stderr(predicate::str::contains("was not found"));
}

#[test]
fn hotkey_conflicts_are_reported() {
    let env = Env::new();
    env.cmd(&["save", "A", "--hotkey", "Ctrl+Alt+1"]).success();
    env.cmd(&["save", "B", "--hotkey", "Ctrl+Alt+1"])
        .failure()
        .stderr(predicate::str::contains("profile 'A'"));
    // The default rescue hotkey is reserved.
    env.cmd(&["save", "B", "--hotkey", "Ctrl+Alt+Shift+D"])
        .failure()
        .stderr(predicate::str::contains("duplicate-all"));
    env.cmd(&["save", "B", "--hotkey", "Banana"])
        .failure()
        .stderr(predicate::str::contains("invalid hotkey"));
}

#[test]
fn settings_can_change_rescue_hotkey() {
    let env = Env::new();
    env.cmd(&["settings"])
        .success()
        .stdout(predicate::str::contains("Ctrl+Alt+Shift+D"));
    env.cmd(&[
        "settings",
        "--duplicate-hotkey",
        "Win+Alt+F12",
        "--extend-hotkey",
        "none",
    ])
    .success()
    .stdout(predicate::str::contains("Alt+Win+F12"));
    env.cmd(&["save", "A", "--hotkey", "Ctrl+Alt+1"]).success();
    env.cmd(&["settings", "--extend-hotkey", "Ctrl+Alt+1"])
        .failure()
        .stderr(predicate::str::contains("profile 'A'"));
}

#[test]
fn controller_settings_toggle_and_keep_combination() {
    let env = Env::new();
    env.cmd(&["settings"])
        .success()
        .stdout(predicate::str::contains("gamepad_overlay    = A+LB+RB+LT+RT"))
        .stdout(predicate::str::contains("gamepad            = on"));
    env.cmd(&["settings", "--gamepad-overlay", "Cross+Options"])
        .success()
        .stdout(predicate::str::contains("gamepad_overlay    = A+Start"));
    env.cmd(&["settings", "--gamepad", "off"])
        .success()
        .stdout(predicate::str::contains("gamepad            = off"))
        .stdout(predicate::str::contains("gamepad_overlay    = A+Start"));
    env.cmd(&["settings", "--gamepad-overlay", "A+Up"])
        .failure()
        .stderr(predicate::str::contains("D-pad"));
}

#[test]
fn startup_profile_setting() {
    let env = Env::new();
    env.cmd(&["settings"])
        .success()
        .stdout(predicate::str::contains("startup_profile    = none"));
    env.cmd(&["save", "Desk"]).success();
    env.cmd(&["settings", "--startup", "desk"])
        .success()
        .stdout(predicate::str::contains("startup_profile    = profile:Desk"));
    env.cmd(&["settings", "--startup", "Nope"])
        .failure()
        .stderr(predicate::str::contains("was not found"));
    env.cmd(&["settings", "--startup", "last"])
        .success()
        .stdout(predicate::str::contains("startup_profile    = last"));
    env.cmd(&["apply", "Desk"]).success();
    env.cmd(&["autostart"])
        .success()
        .stdout(predicate::str::contains("load the last used profile (now: Desk)"));
}

#[test]
fn modes_by_number_and_name() {
    let env = Env::new();
    env.cmd(&["modes", "1"])
        .success()
        .stdout(predicate::str::contains("3840x2160 @ 60 Hz  (current)"));
    env.cmd(&["modes", "right"])
        .success()
        .stdout(predicate::str::contains("Monitor RIGHT"));
    env.cmd(&["modes", "monitor"])
        .failure()
        .stderr(predicate::str::contains("matches 3 monitors"));
    env.cmd(&["modes", "9"]).failure();
}

#[test]
fn applying_with_nothing_connected_fails_safely() {
    let env = Env::new();
    env.cmd(&["save", "Desk"]).success();
    // Pretend only the TV is plugged in.
    let mut only_tv = sample_layout();
    only_tv.monitors.retain(|m| m.id.device_path == "TV");
    only_tv.monitors[0].enabled = true;
    std::fs::write(env.dir.path().join("monitors.toml"), toml::to_string(&only_tv).unwrap()).unwrap();
    env.cmd(&["apply", "Desk"])
        .failure()
        .stderr(predicate::str::contains("none of the monitors"));
    assert!(env.layout().monitors[0].enabled, "layout must be left untouched");
}

#[test]
fn a_broken_profile_file_does_not_hide_the_others() {
    let env = Env::new();
    env.cmd(&["save", "Desk", "--hotkey", "Ctrl+Alt+1"]).success();
    std::fs::write(env.dir.path().join("config/profiles/broken.toml"), "name = [oops").unwrap();
    env.cmd(&["profiles"])
        .success()
        .stdout(predicate::str::contains("Desk"))
        .stderr(predicate::str::contains("broken.toml"));
    env.cmd(&["apply", "Desk"]).success();
    // Reserved Windows device names can't be profile files.
    env.cmd(&["save", "CON"])
        .failure()
        .stderr(predicate::str::contains("CON"));
}

#[test]
fn accent_colour_setting() {
    let env = Env::new();
    env.cmd(&["settings"])
        .success()
        .stdout(predicate::str::contains("accent_color       = #7C3AED"));
    env.cmd(&["settings", "--accent", "22c55e"])
        .success()
        .stdout(predicate::str::contains("accent_color       = #22C55E"));
    env.cmd(&["settings", "--accent", "teal"])
        .failure()
        .stderr(predicate::str::contains("#RRGGBB"));
    env.cmd(&["settings", "--accent", "default"])
        .success()
        .stdout(predicate::str::contains("accent_color       = #7C3AED"));
}
