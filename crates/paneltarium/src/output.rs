//! Plain-text tables for the terminal.

use pt_core::{DisplayLayout, MonitorState, Profile, Rotation};

fn table(headers: &[&str], rows: Vec<Vec<String>>) -> String {
    let mut widths: Vec<usize> = headers.iter().map(|h| h.len()).collect();
    for row in &rows {
        for (i, cell) in row.iter().enumerate() {
            widths[i] = widths[i].max(cell.chars().count());
        }
    }
    let line = |cells: Vec<String>| {
        cells
            .iter()
            .enumerate()
            .map(|(i, c)| format!("{c:<w$}", w = widths[i]))
            .collect::<Vec<_>>()
            .join("  ")
            .trim_end()
            .to_string()
    };
    let mut out = vec![line(headers.iter().map(|h| h.to_string()).collect())];
    out.extend(rows.into_iter().map(line));
    out.join("\n")
}

fn rotation(r: Rotation) -> &'static str {
    match r {
        Rotation::None => "",
        Rotation::Rotate90 => "90°",
        Rotation::Rotate180 => "180°",
        Rotation::Rotate270 => "270°",
    }
}

fn monitor_row(i: usize, m: &MonitorState) -> Vec<String> {
    let (state, mode, pos) = match (&m.mode, m.enabled) {
        (Some(mode), true) => ("on", mode.to_string(), m.position.to_string()),
        _ => ("off", String::new(), String::new()),
    };
    vec![
        (i + 1).to_string(),
        m.id.label().to_string(),
        state.into(),
        mode,
        pos,
        rotation(m.rotation).into(),
        if m.primary && m.enabled {
            "yes".into()
        } else {
            String::new()
        },
    ]
}

pub fn layout(layout: &DisplayLayout) -> String {
    if layout.monitors.is_empty() {
        return "No monitors found.".into();
    }
    table(
        &["#", "MONITOR", "STATE", "MODE", "POSITION", "ROTATION", "PRIMARY"],
        layout
            .monitors
            .iter()
            .enumerate()
            .map(|(i, m)| monitor_row(i, m))
            .collect(),
    )
}

pub fn profiles(profiles: &[Profile]) -> String {
    if profiles.is_empty() {
        return "No profiles yet. Save one with: paneltarium save <name>".into();
    }
    table(
        &["PROFILE", "HOTKEY", "MONITORS"],
        profiles
            .iter()
            .map(|p| {
                vec![
                    p.name.clone(),
                    p.hotkey.map(|h| h.to_string()).unwrap_or_default(),
                    p.layout.summary(),
                ]
            })
            .collect(),
    )
}

pub fn profile(p: &Profile) -> String {
    let mut out = format!("Profile: {}\n", p.name);
    out += &format!(
        "Hotkey:  {}\n\n",
        p.hotkey.map(|h| h.to_string()).unwrap_or_else(|| "(none)".into())
    );
    out += &layout(&p.layout);
    out
}
