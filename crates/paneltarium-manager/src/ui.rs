//! The manager window (egui). Draws the model and turns clicks into
//! [`Act`]ions, which are applied after the frame is drawn. No OS code.

use std::time::{Duration, Instant};

use eframe::egui::{
    self, Align, Align2, Color32, ComboBox, CornerRadius, FontId, Frame, Key, Layout, Margin, RichText, ScrollArea,
    Sense, Stroke, StrokeKind, TextEdit, Ui, Vec2,
};
use egui_phosphor::regular as icon;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver};

use pt_core::gamepad::{ChordRecorder, GamepadChord};
use pt_core::startup::StartupProfile;
use pt_core::{AutostartStatus, Color, DisplayLayout, Hotkey, OverlayMode, Theme};
use pt_gamepad::{GamepadWatcher, PadEvent};

use crate::keys;
use crate::model::{AppHotkey, Model, Msg, Row};

// Palette shared with the overlay.
const BG: Color32 = Color32::from_rgb(0x16, 0x17, 0x1B);
const CARD: Color32 = Color32::from_rgb(0x1F, 0x21, 0x27);
const BORDER: Color32 = Color32::from_rgb(0x30, 0x33, 0x3C);
const TEXT: Color32 = Color32::from_rgb(0xF3, 0xF4, 0xF6);
const MUTED: Color32 = Color32::from_rgb(0x9C, 0xA3, 0xAF);
const GOOD: Color32 = Color32::from_rgb(0x4A, 0xDE, 0x80);
const BAD: Color32 = Color32::from_rgb(0xF8, 0x71, 0x71);

/// The accent colour (from settings, user-configurable) as 0xRRGGBB. Global
/// so the small drawing helpers below can use it without threading it
/// through every call; only the UI thread touches it.
static ACCENT_RGB: AtomicU32 = AtomicU32::new(0x7C3AED);

/// Accent presets offered next to the colour picker.
const PRESETS: &[(&str, Color)] = &[
    ("Purple", Color::DEFAULT_ACCENT),
    ("Violet", Color::rgb(0xA8, 0x55, 0xF7)),
    ("Indigo", Color::rgb(0x4F, 0x46, 0xE5)),
    ("Blue", Color::rgb(0x25, 0x63, 0xEB)),
    ("Sky", Color::rgb(0x0E, 0xA5, 0xE9)),
    ("Cyan", Color::rgb(0x06, 0xB6, 0xD4)),
    ("Teal", Color::rgb(0x0D, 0x94, 0x88)),
    ("Green", Color::rgb(0x16, 0xA3, 0x4A)),
    ("Lime", Color::rgb(0x84, 0xCC, 0x16)),
    ("Yellow", Color::rgb(0xFA, 0xCC, 0x15)),
    ("Amber", Color::rgb(0xF5, 0x9E, 0x0B)),
    ("Orange", Color::rgb(0xEA, 0x58, 0x0C)),
    ("Red", Color::rgb(0xDC, 0x26, 0x26)),
    ("Rose", Color::rgb(0xE1, 0x1D, 0x48)),
    ("Pink", Color::rgb(0xDB, 0x27, 0x77)),
    ("Fuchsia", Color::rgb(0xC0, 0x26, 0xD3)),
    ("Slate", Color::rgb(0x64, 0x74, 0x8B)),
    ("White", Color::WHITE),
];

/// How long the picker must be still before the colour is saved.
const ACCENT_SAVE_DELAY: Duration = Duration::from_millis(400);

fn theme() -> Theme {
    let v = ACCENT_RGB.load(Ordering::Relaxed);
    Theme::new(Color::rgb((v >> 16) as u8, (v >> 8) as u8, v as u8))
}

fn c32(c: Color) -> Color32 {
    Color32::from_rgb(c.r, c.g, c.b)
}

fn accent() -> Color32 {
    c32(theme().accent)
}

/// Text/icon colour on accent fills (black on light accents).
fn on_accent() -> Color32 {
    c32(theme().on_accent)
}

/// Background of the open profile card: a dark tint of the accent.
fn card_open() -> Color32 {
    c32(theme().selected)
}

/// Switch the accent everywhere in this window, now.
fn set_accent(ctx: &egui::Context, color: Color) {
    let v = (u32::from(color.r) << 16) | (u32::from(color.g) << 8) | u32::from(color.b);
    if ACCENT_RGB.swap(v, Ordering::Relaxed) != v {
        apply_visuals(ctx);
    }
}

/// How often to look for profile/settings changes made elsewhere (CLI, app).
const POLL: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, PartialEq, Eq)]
enum Target {
    Profile(String),
    App(AppHotkey),
}

/// Things the user asked for this frame.
#[derive(Debug, Clone)]
enum Act {
    Apply(String),
    Toggle(String),
    Rename(String, String),
    AskDelete(String),
    Delete(String),
    Update(String),
    MoveSlot(String, u8),
    Hotkey(Target, Option<Hotkey>),
    Typed(Target, String),
    Record(Target),
    StopRecording,
    ToggleSave,
    Save(String),
    RefreshLayout,
    GamepadEnabled(bool),
    Chord(Option<GamepadChord>),
    TypedChord(String),
    RecordChord,
    Autostart(bool),
    StartupProfile(StartupProfile),
    /// Save this accent colour.
    Accent(Color),
    OverlayMode(OverlayMode),
}

pub struct ManagerApp {
    ctx: egui::Context,
    model: Model,
    status: Option<Msg>,
    /// The profile whose editor is open.
    open: Option<String>,
    rename_buf: String,
    hotkey_buf: String,
    app_bufs: [String; 3],
    save_open: bool,
    new_name: String,
    confirm_delete: Option<String>,
    recording: Option<Target>,
    last_poll: Instant,
    /// The app icon in the header (created on first draw).
    logo: Option<egui::TextureHandle>,
    /// Colour being picked but not saved yet, and when it last changed.
    accent_pending: Option<(Color, Instant)>,
    chord_buf: String,
    /// While recording a controller combination: the reader, its events and
    /// the recorder. Dropped (controllers no longer read) when done.
    chord_rec: Option<ChordRecording>,
}

struct ChordRecording {
    _watcher: GamepadWatcher,
    events: Receiver<PadEvent>,
    recorder: ChordRecorder,
}

impl ManagerApp {
    pub fn new(cc: &eframe::CreationContext<'_>, model: Model) -> Self {
        setup_style(&cc.egui_ctx);
        ManagerApp {
            ctx: cc.egui_ctx.clone(),
            model,
            status: None,
            open: None,
            rename_buf: String::new(),
            hotkey_buf: String::new(),
            app_bufs: Default::default(),
            save_open: false,
            new_name: String::new(),
            confirm_delete: None,
            recording: None,
            last_poll: Instant::now(),
            logo: None,
            accent_pending: None,
            chord_buf: String::new(),
            chord_rec: None,
        }
    }

    fn run(&mut self, act: Act) {
        let msg = match act {
            Act::Apply(n) => Some(self.model.apply(&n)),
            Act::Toggle(n) => {
                if self.open.as_deref() == Some(&n) {
                    self.open = None;
                } else {
                    self.rename_buf = n.clone();
                    self.hotkey_buf.clear();
                    self.open = Some(n);
                }
                self.confirm_delete = None;
                None
            }
            Act::Rename(old, new) => {
                let r = self.model.rename(&old, &new);
                if r.is_ok() {
                    self.open = Some(new.trim().to_string());
                }
                Some(r)
            }
            Act::AskDelete(n) => {
                self.confirm_delete = Some(n);
                None
            }
            Act::Delete(n) => {
                self.confirm_delete = None;
                self.open = None;
                Some(self.model.delete(&n))
            }
            Act::Update(n) => Some(self.model.update_from_current(&n)),
            Act::MoveSlot(n, slot) => Some(self.model.move_to_slot(&n, slot)),
            Act::Hotkey(t, h) => Some(self.set_hotkey(t, h)),
            Act::Typed(t, text) => Some(match text.trim() {
                "" => return,
                s => match s.parse::<Hotkey>() {
                    Ok(h) => self.set_hotkey(t, Some(h)),
                    Err(e) => Err(e.to_string()),
                },
            }),
            Act::Record(t) => {
                self.recording = Some(t);
                None
            }
            Act::StopRecording => {
                self.recording = None;
                self.chord_rec = None;
                None
            }
            Act::ToggleSave => {
                self.save_open = !self.save_open;
                None
            }
            Act::Save(name) => {
                let r = self.model.save_current(&name, false);
                if r.is_ok() {
                    self.save_open = false;
                    self.new_name.clear();
                }
                Some(r)
            }
            Act::GamepadEnabled(on) => Some(self.model.set_gamepad_enabled(on)),
            Act::Autostart(on) => Some(
                pt_core::paths::sibling_exe("paneltarium-app")
                    .map_err(|e| e.to_string())
                    .and_then(|exe| self.model.set_autostart(on, &exe)),
            ),
            Act::StartupProfile(p) => Some(self.model.set_startup_profile(p)),
            Act::OverlayMode(m) => Some(self.model.set_overlay_mode(m)),
            Act::Accent(c) => {
                self.accent_pending = None;
                set_accent(&self.ctx, c);
                Some(self.model.set_accent(c))
            }
            Act::Chord(c) => {
                self.chord_rec = None;
                let r = self.model.set_gamepad_chord(c);
                if r.is_ok() {
                    self.chord_buf.clear();
                }
                Some(r)
            }
            Act::TypedChord(text) => match text.trim() {
                "" => None,
                t => Some(match t.parse::<GamepadChord>() {
                    Ok(c) => {
                        let r = self.model.set_gamepad_chord(Some(c));
                        if r.is_ok() {
                            self.chord_buf.clear();
                        }
                        r
                    }
                    Err(e) => Err(e.to_string()),
                }),
            },
            Act::RecordChord => {
                self.recording = None;
                let (tx, rx) = mpsc::channel();
                let ctx = self.ctx.clone();
                self.chord_rec = Some(ChordRecording {
                    _watcher: GamepadWatcher::spawn(true, move |e| {
                        let _ = tx.send(e);
                        ctx.request_repaint();
                    }),
                    events: rx,
                    recorder: ChordRecorder::default(),
                });
                self.status = Some(Ok(
                    "Hold the controller buttons you want, then let go (Esc cancels)".into()
                ));
                None
            }
            Act::RefreshLayout => {
                self.model.refresh();
                None
            }
        };
        if let Some(m) = msg {
            self.status = Some(m);
        }
    }

    fn set_hotkey(&mut self, target: Target, h: Option<Hotkey>) -> Msg {
        let r = match target {
            Target::Profile(n) => self.model.set_hotkey(&n, h),
            Target::App(which) => self.model.set_app_hotkey(which, h),
        };
        if r.is_ok() {
            self.hotkey_buf.clear();
            self.app_bufs = Default::default();
        }
        r
    }

    /// While recording a controller combination: feed button events to the
    /// recorder; Esc cancels.
    fn capture_chord(&mut self, ctx: &egui::Context) -> Option<Act> {
        let rec = self.chord_rec.as_mut()?;
        if ctx.input(|i| i.key_pressed(Key::Escape)) {
            return Some(Act::StopRecording);
        }
        while let Ok(e) = rec.events.try_recv() {
            if let Some(result) = rec.recorder.update(e.button, e.pressed) {
                return Some(match result {
                    Ok(c) => Act::Chord(Some(c)),
                    Err(err) => {
                        self.status = Some(Err(err.to_string()));
                        Act::StopRecording
                    }
                });
            }
        }
        None
    }

    /// While recording, the next key combo pressed in this window becomes
    /// the hotkey. Esc cancels. Key events are consumed so text fields
    /// don't see them.
    fn capture_recording(&mut self, ctx: &egui::Context) -> Option<Act> {
        let target = self.recording.clone()?;
        let mut result = None;
        ctx.input_mut(|i| {
            i.events.retain(|e| match e {
                egui::Event::Key {
                    key,
                    physical_key,
                    pressed: true,
                    modifiers,
                    ..
                } => {
                    if result.is_none() {
                        result = Some(if *key == Key::Escape {
                            Act::StopRecording
                        } else {
                            match keys::to_hotkey(*key, *physical_key, *modifiers) {
                                Some(Ok(h)) => Act::Hotkey(target.clone(), Some(h)),
                                Some(Err(e)) => {
                                    self.status = Some(Err(e.to_string()));
                                    Act::StopRecording
                                }
                                None => return false, // unbindable key: keep waiting
                            }
                        });
                    }
                    false
                }
                egui::Event::Text(_) => false,
                _ => true,
            })
        });
        if matches!(result, Some(Act::Hotkey(..))) {
            self.recording = None;
        }
        result
    }
}

impl eframe::App for ManagerApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let mut acts: Vec<Act> = Vec::new();
        acts.extend(self.capture_recording(&ctx));
        acts.extend(self.capture_chord(&ctx));

        if self.last_poll.elapsed() > POLL {
            self.last_poll = Instant::now();
            self.model.refresh_files();
        }
        ctx.request_repaint_after(POLL);
        match self.accent_pending {
            // Picker went still: save it (one write, not one per frame).
            Some((c, at)) if at.elapsed() >= ACCENT_SAVE_DELAY => acts.push(Act::Accent(c)),
            Some(_) => ctx.request_repaint_after(ACCENT_SAVE_DELAY),
            // Follow the saved colour (also picks up `paneltarium settings --accent`).
            None => set_accent(&ctx, self.model.settings.accent_color),
        }

        egui::Panel::top("top")
            .frame(Frame::new().fill(BG).inner_margin(Margin::symmetric(20, 14)))
            .show_separator_line(false)
            .show(ui, |ui| self.header(ui, &mut acts));
        egui::Panel::bottom("status")
            .frame(Frame::new().fill(BG).inner_margin(Margin::symmetric(20, 8)))
            .show(ui, |ui| self.status_bar(ui));
        egui::Panel::right("side")
            .resizable(false)
            .exact_size(330.0)
            .frame(Frame::new().fill(BG).inner_margin(Margin {
                left: 8,
                right: 20,
                top: 4,
                bottom: 12,
            }))
            .show_separator_line(false)
            .show(ui, |ui| {
                ScrollArea::vertical()
                    .id_salt("side-scroll")
                    .auto_shrink(false)
                    .show(ui, |ui| self.side(ui, &mut acts))
            });
        egui::CentralPanel::default_margins()
            .frame(Frame::new().fill(BG).inner_margin(Margin {
                left: 20,
                right: 8,
                top: 4,
                bottom: 12,
            }))
            .show(ui, |ui| self.profiles(ui, &mut acts));

        for a in acts {
            self.run(a);
        }
    }
}

impl ManagerApp {
    fn header(&mut self, ui: &mut Ui, acts: &mut Vec<Act>) {
        ui.horizontal(|ui| {
            let logo = self.logo.get_or_insert_with(|| {
                // Drawn at 2× so it stays sharp on high-DPI screens.
                let image =
                    egui::ColorImage::from_rgba_unmultiplied([64, 64], &pt_brand::glyph_rgba(64, pt_brand::WHITE));
                ui.ctx().load_texture("logo", image, egui::TextureOptions::LINEAR)
            });
            ui.add(egui::Image::new((logo.id(), egui::vec2(32.0, 32.0))));
            ui.vertical(|ui| {
                ui.label(RichText::new("Paneltarium").size(21.0).color(TEXT));
                ui.label(RichText::new("Profiles").size(13.0).color(MUTED));
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let label = format!("{} Save current setup", icon::PLUS);
                if accent_button(ui, &label).clicked() {
                    acts.push(Act::ToggleSave);
                }
            });
        });
        if self.save_open {
            ui.add_space(10.0);
            card(ui, card_open(), |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Name").color(MUTED));
                    let r = ui.add(
                        TextEdit::singleline(&mut self.new_name)
                            .hint_text("e.g. Gaming, TV only")
                            .desired_width(220.0),
                    );
                    r.request_focus();
                    let enter = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                    if accent_button(ui, &format!("{} Save", icon::FLOPPY_DISK)).clicked() || enter {
                        acts.push(Act::Save(self.new_name.clone()));
                    }
                    if ui.button("Cancel").clicked() {
                        acts.push(Act::ToggleSave);
                    }
                });
                ui.label(
                    RichText::new("Saves which monitors are on, their resolution, refresh rate and position.")
                        .size(12.0)
                        .color(MUTED),
                );
            });
        }
    }

    fn status_bar(&self, ui: &mut Ui) {
        ui.horizontal(|ui| match &self.status {
            Some(Ok(m)) => {
                ui.label(RichText::new(format!("{} {m}", icon::CHECK_CIRCLE)).color(GOOD));
            }
            Some(Err(m)) => {
                ui.label(RichText::new(format!("{} {m}", icon::WARNING_CIRCLE)).color(BAD));
            }
            None => {
                ui.label(
                    RichText::new("Changes take effect immediately; the background app picks them up.")
                        .color(MUTED)
                        .size(12.0),
                );
            }
        });
    }

    fn profiles(&mut self, ui: &mut Ui, acts: &mut Vec<Act>) {
        let rows = self.model.rows();
        let mods = self.model.overlay_mods();
        for problem in &self.model.problems {
            ui.label(RichText::new(format!("⚠ {problem}")).size(12.0).color(BAD));
        }
        ui.label(RichText::new("Profiles").size(15.0).color(TEXT));
        ui.label(
            RichText::new(format!(
                "Numbers are what you press in the overlay ({}).",
                self.model
                    .settings
                    .overlay_hotkey
                    .map(|h| h.to_string())
                    .unwrap_or_else(|| "no overlay hotkey".into())
            ))
            .size(12.0)
            .color(MUTED),
        );
        ui.add_space(8.0);
        if rows.is_empty() {
            ui.label(
                RichText::new("No profiles yet. Arrange your monitors in Windows, then click “Save current setup”.")
                    .color(MUTED),
            );
            return;
        }
        ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            for row in rows {
                self.profile_card(ui, &row, mods, acts);
                ui.add_space(8.0);
            }
        });
    }

    fn profile_card(&mut self, ui: &mut Ui, row: &Row, mods: pt_core::Modifiers, acts: &mut Vec<Act>) {
        let open = self.open.as_deref() == Some(row.name.as_str());
        card(ui, if open { card_open() } else { CARD }, |ui| {
            ui.horizontal(|ui| {
                badge(ui, row.slot);
                // Right-hand controls first, so the text gets whatever width is
                // left and is cut with "…" instead of running underneath.
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let edit = if open { icon::CARET_UP } else { icon::PENCIL_SIMPLE };
                    if ui
                        .button(RichText::new(edit).size(16.0))
                        .on_hover_text("Edit")
                        .clicked()
                    {
                        acts.push(Act::Toggle(row.name.clone()));
                    }
                    if accent_button(ui, &format!("{} Apply", icon::PLAY)).clicked() {
                        acts.push(Act::Apply(row.name.clone()));
                    }
                    if let Some(h) = row.hotkey {
                        chip(ui, &h.to_string());
                    }
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        ui.vertical(|ui| {
                            ui.add(egui::Label::new(RichText::new(&row.name).size(16.0).color(TEXT)).truncate());
                            ui.add(egui::Label::new(RichText::new(&row.summary).size(12.0).color(MUTED)).truncate());
                        });
                    });
                });
            });
            if open {
                ui.add_space(8.0);
                ui.separator();
                ui.add_space(4.0);
                self.profile_editor(ui, row, mods, acts);
            }
        });
    }

    fn profile_editor(&mut self, ui: &mut Ui, row: &Row, mods: pt_core::Modifiers, acts: &mut Vec<Act>) {
        let name = row.name.clone();
        egui::Grid::new(("editor", &name))
            .num_columns(2)
            .spacing([12.0, 8.0])
            .show(ui, |ui| {
                ui.label(RichText::new("Name").color(MUTED));
                ui.horizontal(|ui| {
                    let r = ui.add(TextEdit::singleline(&mut self.rename_buf).desired_width(200.0));
                    let enter = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                    let changed = self.rename_buf.trim() != name;
                    if (ui.add_enabled(changed, egui::Button::new("Rename")).clicked() || enter) && changed {
                        acts.push(Act::Rename(name.clone(), self.rename_buf.clone()));
                    }
                });
                ui.end_row();

                ui.label(RichText::new("Overlay number").color(MUTED));
                ui.horizontal(|ui| {
                    let mut slot = row.slot.unwrap_or(0);
                    let before = slot;
                    ComboBox::from_id_salt(("slot", &name))
                        .width(60.0)
                        .selected_text(row.slot.map(|s| s.to_string()).unwrap_or_else(|| "–".into()))
                        .show_ui(ui, |ui| {
                            for n in 1..=9u8 {
                                ui.selectable_value(&mut slot, n, n.to_string());
                            }
                        });
                    if slot != before && slot != 0 {
                        acts.push(Act::MoveSlot(name.clone(), slot));
                    }
                    ui.label(
                        RichText::new(format!(
                            "also sets its hotkey to {}",
                            pt_core::slots::slot_hotkey(mods, slot.max(1))
                        ))
                        .size(12.0)
                        .color(MUTED),
                    );
                });
                ui.end_row();

                ui.label(RichText::new("Hotkey").color(MUTED));
                let target = Target::Profile(name.clone());
                let recording = self.recording.as_ref() == Some(&target);
                if let Some(a) = hotkey_editor(ui, &target, row.hotkey, &mut self.hotkey_buf, recording) {
                    acts.push(a);
                }
                ui.end_row();
            });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui
                .button(format!("{} Update from current setup", icon::ARROWS_CLOCKWISE))
                .on_hover_text("Replace this profile's monitors with how they are set up right now")
                .clicked()
            {
                acts.push(Act::Update(name.clone()));
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if self.confirm_delete.as_deref() == Some(name.as_str()) {
                    let b =
                        egui::Button::new(RichText::new(format!("{} Click again to delete", icon::TRASH)).color(BAD));
                    if ui.add(b).clicked() {
                        acts.push(Act::Delete(name.clone()));
                    }
                } else if ui.button(format!("{} Delete", icon::TRASH)).clicked() {
                    acts.push(Act::AskDelete(name.clone()));
                }
            });
        });
    }

    fn side(&mut self, ui: &mut Ui, acts: &mut Vec<Act>) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Current setup").size(15.0).color(TEXT));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .button(RichText::new(icon::ARROWS_CLOCKWISE).size(14.0))
                    .on_hover_text("Refresh")
                    .clicked()
                {
                    acts.push(Act::RefreshLayout);
                }
            });
        });
        ui.add_space(6.0);
        card(ui, CARD, |ui| match &self.model.layout {
            Ok(layout) => draw_layout(ui, layout),
            Err(e) => {
                ui.label(RichText::new(e).color(BAD));
            }
        });

        ui.add_space(16.0);
        ui.label(RichText::new("App hotkeys").size(15.0).color(TEXT));
        ui.add_space(6.0);
        card(ui, CARD, |ui| {
            for (i, which) in AppHotkey::ALL.into_iter().enumerate() {
                ui.label(RichText::new(which.label()).color(MUTED));
                let target = Target::App(which);
                let recording = self.recording.as_ref() == Some(&target);
                let current = which.get(&self.model.settings);
                if let Some(a) = hotkey_editor(ui, &target, current, &mut self.app_bufs[i], recording) {
                    acts.push(a);
                }
                if which == AppHotkey::Overlay
                    && let Some(hk) = current
                {
                    overlay_mode_choice(ui, hk, self.model.settings.overlay_mode, acts);
                }
                ui.add_space(6.0);
            }
            ui.label(
                RichText::new(
                    "Tip: combos Paneltarium already uses can't be recorded while it runs; type them instead.",
                )
                .size(11.5)
                .color(MUTED),
            );
        });

        ui.add_space(16.0);
        ui.label(RichText::new("Game controller").size(15.0).color(TEXT));
        ui.add_space(6.0);
        card(ui, CARD, |ui| self.gamepad_card(ui, acts));

        ui.add_space(16.0);
        ui.label(RichText::new("Startup").size(15.0).color(TEXT));
        ui.add_space(6.0);
        card(ui, CARD, |ui| self.startup_card(ui, acts));

        ui.add_space(16.0);
        ui.label(RichText::new("Appearance").size(15.0).color(TEXT));
        ui.add_space(6.0);
        card(ui, CARD, |ui| self.appearance_card(ui, acts));
    }

    fn appearance_card(&mut self, ui: &mut Ui, acts: &mut Vec<Act>) {
        let current = self
            .accent_pending
            .map(|(c, _)| c)
            .unwrap_or(self.model.settings.accent_color);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Accent colour").color(TEXT));
            let mut rgb = current.to_array();
            let r = ui
                .color_edit_button_srgb(&mut rgb)
                .on_hover_text("Pick any colour; it's saved when you stop dragging");
            if r.changed() {
                let c = Color::rgb(rgb[0], rgb[1], rgb[2]);
                set_accent(ui.ctx(), c); // preview right away
                self.accent_pending = Some((c, Instant::now()));
            }
            ui.label(RichText::new(current.to_string()).size(12.0).color(MUTED));
        });
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            for &(name, c) in PRESETS {
                let (rect, resp) = ui.allocate_exact_size(Vec2::splat(22.0), Sense::click());
                let selected = c == current;
                ui.painter()
                    .rect(rect, 6, c32(c), Stroke::new(1.0, BORDER), StrokeKind::Inside);
                if selected || resp.hovered() {
                    let ring = if selected { Color32::WHITE } else { MUTED };
                    ui.painter()
                        .rect_stroke(rect.expand(2.0), 7, Stroke::new(1.5, ring), StrokeKind::Outside);
                }
                if resp.on_hover_text(format!("{name} {c}")).clicked() {
                    acts.push(Act::Accent(c));
                }
            }
            if current != Color::DEFAULT_ACCENT && ui.small_button("Reset").clicked() {
                acts.push(Act::Accent(Color::DEFAULT_ACCENT));
            }
        });
        ui.label(
            RichText::new("Used by the overlay and this window. The app icon stays purple.")
                .size(11.5)
                .color(MUTED),
        );
    }

    fn startup_card(&mut self, ui: &mut Ui, acts: &mut Vec<Act>) {
        const START_LABEL: &str = if cfg!(windows) {
            "Start with Windows"
        } else {
            "Start when I sign in"
        };
        match self.model.autostart_status {
            Some(status) => {
                let mut on = status != AutostartStatus::Off;
                if toggle_row(ui, &mut on, START_LABEL)
                    .on_hover_text("Run Paneltarium in the background when you sign in.")
                    .changed()
                {
                    acts.push(Act::Autostart(on));
                }
                if status == AutostartStatus::DisabledInSystem {
                    ui.label(
                        RichText::new(if cfg!(windows) {
                            "⚠ Switched off in Task Manager → Startup apps. Turn it off and on here to re-enable it."
                        } else {
                            "⚠ Switched off in your desktop's startup settings. Turn it off and on here to re-enable it."
                        })
                        .size(11.5)
                        .color(BAD),
                    );
                }
            }
            None => {
                ui.label(RichText::new("Starting at sign-in isn't available on this system.").color(MUTED));
            }
        }
        ui.add_space(8.0);
        ui.label(RichText::new("When Paneltarium starts at sign-in").color(MUTED));
        let current = self.model.settings.startup_profile.clone();
        let names: Vec<String> = self.model.profiles.iter().map(|p| p.name.clone()).collect();
        let os_default = if cfg!(windows) {
            "Leave the monitors as Windows set them up"
        } else {
            "Leave the monitors as they are"
        };
        if ui.radio(current == StartupProfile::None, os_default).clicked() && current != StartupProfile::None {
            acts.push(Act::StartupProfile(StartupProfile::None));
        }
        let last = match self.model.last_profile() {
            Some(n) => format!("Load the last used profile (now: {n})"),
            None => "Load the last used profile".to_string(),
        };
        if ui.radio(current == StartupProfile::Last, last).clicked() && current != StartupProfile::Last {
            acts.push(Act::StartupProfile(StartupProfile::Last));
        }
        let chosen = match &current {
            StartupProfile::Profile(n) => Some(n.clone()),
            _ => None,
        };
        ui.horizontal(|ui| {
            let r = ui.add_enabled(!names.is_empty(), egui::RadioButton::new(chosen.is_some(), "Load"));
            if r.clicked()
                && chosen.is_none()
                && let Some(first) = names.first()
            {
                acts.push(Act::StartupProfile(StartupProfile::Profile(first.clone())));
            }
            ui.add_enabled_ui(!names.is_empty(), |ui| {
                let shown = chosen
                    .clone()
                    .or_else(|| names.first().cloned())
                    .unwrap_or_else(|| "no profiles yet".into());
                egui::ComboBox::from_id_salt("startup-profile")
                    .selected_text(shown)
                    .show_ui(ui, |ui| {
                        for n in &names {
                            if ui.selectable_label(chosen.as_deref() == Some(n), n).clicked() {
                                acts.push(Act::StartupProfile(StartupProfile::Profile(n.clone())));
                            }
                        }
                    });
            });
        });
        if let Some(n) = &chosen
            && !names.iter().any(|x| x.eq_ignore_ascii_case(n))
        {
            ui.label(
                RichText::new(format!("⚠ Profile '{n}' no longer exists; nothing will be loaded."))
                    .size(11.5)
                    .color(BAD),
            );
        }
        ui.label(
            RichText::new("Applied only at sign-in, after the monitors have shown up; skipped if already on screen.")
                .size(11.5)
                .color(MUTED),
        );
    }

    fn gamepad_card(&mut self, ui: &mut Ui, acts: &mut Vec<Act>) {
        let s = &self.model.settings;
        let mut on = s.gamepad_enabled;
        if toggle_row(ui, &mut on, "Listen to game controllers")
            .on_hover_text("Off = controllers aren't read at all. Your button combination is kept.")
            .changed()
        {
            acts.push(Act::GamepadEnabled(on));
        }
        ui.add_space(6.0);
        ui.add_enabled_ui(on, |ui| {
            ui.label(RichText::new("Hold to open the overlay").color(MUTED));
            ui.horizontal(|ui| {
                match &s.gamepad_overlay {
                    Some(c) => chip_with(ui, icon::GAME_CONTROLLER, &c.to_string()),
                    None => {
                        ui.label(RichText::new("none").color(MUTED));
                    }
                }
                if self.chord_rec.is_some() {
                    let b = egui::Button::new(
                        RichText::new(format!("{} Hold buttons… (Esc)", icon::RECORD)).color(on_accent()),
                    )
                    .fill(accent());
                    if ui.add(b).clicked() {
                        acts.push(Act::StopRecording);
                    }
                } else if ui
                    .button(format!("{} Record", icon::RECORD))
                    .on_hover_text("Click, hold the buttons on your controller, then let go")
                    .clicked()
                {
                    acts.push(Act::RecordChord);
                }
                if s.gamepad_overlay.is_some()
                    && ui
                        .button(RichText::new(icon::X).size(14.0))
                        .on_hover_text("Clear")
                        .clicked()
                {
                    acts.push(Act::Chord(None));
                }
            });
            let r = ui.add(
                TextEdit::singleline(&mut self.chord_buf)
                    .hint_text("or type, e.g. A+LB+RB+LT+RT, then Enter")
                    .desired_width(240.0),
            );
            if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                acts.push(Act::TypedChord(self.chord_buf.clone()));
            }
            ui.add_space(4.0);
            ui.label(
                RichText::new("In the overlay: D-pad to choose, let go to apply, B to cancel.")
                    .size(11.5)
                    .color(MUTED),
            );
        });
        if !on {
            ui.label(
                RichText::new("Controllers are off. Your combination is kept for when you switch them back on.")
                    .size(11.5)
                    .color(MUTED),
            );
        }
    }
}

/// Current hotkey chip + Record/Clear, and below it a text box to type one.
/// "Hold Ctrl+Alt" vs "stays open", spelled out for the actual hotkey.
fn overlay_mode_choice(ui: &mut Ui, hk: Hotkey, mode: OverlayMode, acts: &mut Vec<Act>) {
    let m = hk.modifiers;
    let held: Vec<&str> = [(m.ctrl, "Ctrl"), (m.alt, "Alt"), (m.shift, "Shift"), (m.win, "Win")]
        .into_iter()
        .filter_map(|(on, n)| on.then_some(n))
        .collect();
    ui.add_space(2.0);
    ui.label(
        RichText::new("After pressing it, the profile list…")
            .size(12.0)
            .color(MUTED),
    );
    let hold_label = if held.is_empty() {
        "Closes when you let go of the hotkey".to_string()
    } else {
        format!(
            "Stays open while you hold {} (you can let go of {}); let go to cancel",
            held.join("+"),
            hk.key
        )
    };
    if ui.radio(mode == OverlayMode::Hold, hold_label).clicked() && mode != OverlayMode::Hold {
        acts.push(Act::OverlayMode(OverlayMode::Hold));
    }
    if ui
        .radio(
            mode == OverlayMode::Toggle,
            "Stays open without holding anything, until you pick a profile or press Esc",
        )
        .clicked()
        && mode != OverlayMode::Toggle
    {
        acts.push(Act::OverlayMode(OverlayMode::Toggle));
    }
}

fn hotkey_editor(
    ui: &mut Ui,
    target: &Target,
    current: Option<Hotkey>,
    buf: &mut String,
    recording: bool,
) -> Option<Act> {
    let mut act = None;
    ui.vertical(|ui| {
        ui.horizontal(|ui| {
            match current {
                Some(h) => chip(ui, &h.to_string()),
                None => {
                    ui.label(RichText::new("none").color(MUTED));
                }
            }
            if recording {
                let b =
                    egui::Button::new(RichText::new(format!("{} Press keys… (Esc)", icon::RECORD)).color(on_accent()))
                        .fill(accent());
                if ui.add(b).clicked() {
                    act = Some(Act::StopRecording);
                }
            } else if ui
                .button(format!("{} Record", icon::RECORD))
                .on_hover_text("Click, then press the key combination")
                .clicked()
            {
                act = Some(Act::Record(target.clone()));
            }
            if current.is_some()
                && ui
                    .button(RichText::new(icon::X).size(14.0))
                    .on_hover_text("Clear")
                    .clicked()
            {
                act = Some(Act::Hotkey(target.clone(), None));
            }
        });
        let r = ui.add(
            TextEdit::singleline(buf)
                .hint_text("or type one, e.g. Ctrl+Alt+5, then Enter")
                .desired_width(240.0),
        );
        if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
            act = Some(Act::Typed(target.clone(), buf.clone()));
        }
    });
    act
}

/// Draw the monitor arrangement to scale; switched-off monitors listed below.
fn draw_layout(ui: &mut Ui, layout: &DisplayLayout) {
    let on: Vec<_> = layout.enabled().filter(|m| m.mode.is_some()).collect();
    if on.is_empty() {
        ui.label(RichText::new("No monitors are on?").color(MUTED));
    } else {
        let rect_of = |m: &pt_core::MonitorState| {
            let mode = m.mode.expect("filtered above");
            (
                m.position.x as f32,
                m.position.y as f32,
                mode.width as f32,
                mode.height as f32,
            )
        };
        let (min_x, min_y, max_x, max_y) = on.iter().map(|m| rect_of(m)).fold(
            (f32::MAX, f32::MAX, f32::MIN, f32::MIN),
            |(a, b, c, d), (x, y, w, h)| (a.min(x), b.min(y), c.max(x + w), d.max(y + h)),
        );
        let avail = ui.available_width();
        let height = 150.0;
        let (area, _) = ui.allocate_exact_size(Vec2::new(avail, height), Sense::hover());
        let scale = ((avail - 8.0) / (max_x - min_x)).min((height - 8.0) / (max_y - min_y));
        let offset = area.center() - Vec2::new((max_x - min_x) * scale / 2.0, (max_y - min_y) * scale / 2.0);
        let painter = ui.painter_at(area);
        for m in &on {
            let (x, y, w, h) = rect_of(m);
            let r = egui::Rect::from_min_size(
                offset + Vec2::new((x - min_x) * scale, (y - min_y) * scale),
                Vec2::new(w * scale, h * scale),
            )
            .shrink(2.0);
            let stroke = Stroke::new(
                if m.primary { 2.0 } else { 1.0 },
                if m.primary { accent() } else { BORDER },
            );
            painter.rect(r, CornerRadius::same(6), BG, stroke, StrokeKind::Inside);
            let mode = m.mode.expect("filtered above");
            painter.text(
                r.center() - Vec2::new(0.0, 8.0),
                Align2::CENTER_CENTER,
                m.id.label(),
                FontId::proportional(12.5),
                TEXT,
            );
            painter.text(
                r.center() + Vec2::new(0.0, 9.0),
                Align2::CENTER_CENTER,
                format!("{}x{} · {}", mode.width, mode.height, mode.refresh),
                FontId::proportional(11.0),
                MUTED,
            );
        }
    }
    for m in layout.monitors.iter().filter(|m| !m.enabled) {
        ui.label(RichText::new(format!("{} — off", m.id.label())).size(12.0).color(MUTED));
    }
}

fn card<R>(ui: &mut Ui, fill: Color32, add: impl FnOnce(&mut Ui) -> R) -> R {
    Frame::new()
        .fill(fill)
        .corner_radius(10)
        .stroke(Stroke::new(1.0, if fill == card_open() { accent() } else { BORDER }))
        .inner_margin(Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

fn badge(ui: &mut Ui, slot: Option<u8>) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(34.0), Sense::hover());
    let (fill, ink) = if slot.is_some() {
        (accent(), on_accent())
    } else {
        (BORDER, Color32::WHITE)
    };
    ui.painter().rect_filled(rect, 8, fill);
    let text = slot.map(|s| s.to_string()).unwrap_or_else(|| "–".into());
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        text,
        FontId::proportional(18.0),
        ink,
    );
}

/// A slide toggle followed by its label; clicking either flips it.
fn toggle_row(ui: &mut Ui, on: &mut bool, label: &str) -> egui::Response {
    ui.horizontal(|ui| {
        let mut r = toggle(ui, on);
        let text = ui.add(egui::Label::new(RichText::new(label).color(TEXT)).sense(Sense::click()));
        if text.clicked() {
            *on = !*on;
            r.mark_changed();
        }
        r | text
    })
    .inner
}

/// iOS/Windows-style switch: a pill with a knob that slides across.
fn toggle(ui: &mut Ui, on: &mut bool) -> egui::Response {
    let size = Vec2::new(40.0, 22.0);
    let (rect, mut response) = ui.allocate_exact_size(size, Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    let enabled = ui.is_enabled();
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, enabled, *on, ""));
    if ui.is_rect_visible(rect) {
        let t = ui.ctx().animate_bool_responsive(response.id, *on);
        let off_bg = Color32::from_rgb(0x3A, 0x3F, 0x4B);
        let bg = off_bg.lerp_to_gamma(accent(), t);
        let painter = ui.painter();
        painter.rect_filled(rect, rect.height() / 2.0, bg);
        if response.hovered() {
            painter.rect_stroke(
                rect,
                rect.height() / 2.0,
                Stroke::new(1.0, Color32::from_white_alpha(60)),
                StrokeKind::Inside,
            );
        }
        let r = rect.height() / 2.0 - 3.0;
        let x = egui::lerp((rect.left() + 3.0 + r)..=(rect.right() - 3.0 - r), t);
        // White knob, turning to the accent's text colour as it slides on
        // (so it stays visible on a white or yellow accent).
        let knob = Color32::WHITE.lerp_to_gamma(on_accent(), t);
        painter.circle_filled(egui::pos2(x, rect.center().y), r, knob);
    }
    response
}

/// Small read-only "keyboard shortcut" pill, same height as a button.
fn chip(ui: &mut Ui, text: &str) {
    chip_with(ui, icon::KEYBOARD, text);
}

/// Read-only pill with an icon, same height as a button.
fn chip_with(ui: &mut Ui, icon: &str, text: &str) {
    let galley = ui
        .painter()
        .layout_no_wrap(format!("{icon} {text}"), FontId::proportional(12.5), MUTED);
    let size = Vec2::new(
        galley.size().x + 16.0,
        galley.size().y + 2.0 * ui.spacing().button_padding.y,
    );
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter()
        .rect(rect, 6, BG, Stroke::new(1.0, BORDER), StrokeKind::Inside);
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, MUTED);
}

fn accent_button(ui: &mut Ui, text: &str) -> egui::Response {
    ui.add(egui::Button::new(RichText::new(text).color(on_accent())).fill(accent()))
}

fn setup_style(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    egui_phosphor::add_font_bytes_as_family(&mut fonts, "phosphor-fill", egui_phosphor::bytes::fill::FONT);
    ctx.set_fonts(fonts);
    apply_visuals(ctx);
    ctx.global_style_mut(|s| {
        s.spacing.button_padding = Vec2::new(10.0, 5.0);
        s.spacing.item_spacing = Vec2::new(8.0, 6.0);
    });
}

/// Colours and shapes; re-applied when the accent changes.
fn apply_visuals(ctx: &egui::Context) {
    let mut v = egui::Visuals::dark();
    v.panel_fill = BG;
    v.window_fill = CARD;
    v.extreme_bg_color = Color32::from_rgb(0x12, 0x13, 0x16); // text boxes
    // Selected text keeps its normal colour, so highlight with a darker
    // shade of the accent (a white accent would hide white text).
    v.selection.bg_fill = c32(Color::BACKGROUND.mix(theme().accent, 0.45));
    v.selection.stroke = Stroke::new(1.0, TEXT);
    v.hyperlink_color = accent();
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
    v.widgets.inactive.weak_bg_fill = Color32::from_rgb(0x2A, 0x2D, 0x35);
    v.widgets.inactive.bg_fill = Color32::from_rgb(0x2A, 0x2D, 0x35);
    v.widgets.hovered.weak_bg_fill = Color32::from_rgb(0x34, 0x38, 0x42);
    v.widgets.hovered.bg_fill = Color32::from_rgb(0x34, 0x38, 0x42);
    v.widgets.active.weak_bg_fill = accent();
    v.widgets.active.fg_stroke.color = on_accent();
    for w in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.corner_radius = CornerRadius::same(7);
    }
    ctx.set_visuals(v);
}
