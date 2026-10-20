//! The settings drawer beside the sidebar: one body per section.

use egui::{Align2, Color32, CornerRadius, FontId, Margin, Rect, RichText, Sense, Stroke, StrokeKind, Vec2};

use super::icons::Icon;
use super::theme::{to_c32, tokens, Tokens};
use super::widgets::{choice, heading, icon_button, knob, num, segmented, spectrum_meter, PCT};
use super::{Section, UiAction, UiCtx, UiState, SECTIONS};
use crate::palette::{Palette, PRESETS};
use crate::scenes::SceneId;
use crate::settings::{AutoScene, Settings, UiTheme, WindowMode};

/// Looks to go back to with Undo (taken before each Randomize or Reset).
const UNDO_DEPTH: usize = 32;

impl UiState {
    pub(super) fn drawer(&mut self, ui: &mut egui::Ui, sec: Section, c: &mut UiCtx, t: &Tokens, accent: Color32, actions: &mut Vec<UiAction>) {
        let title = SECTIONS.iter().find(|s| s.0 == sec).map_or("", |s| s.1);
        ui.horizontal(|ui| {
            ui.label(RichText::new(title).heading().strong().color(t.text));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if icon_button(ui, t, Icon::Close, 26.0).on_hover_text("Close").clicked() {
                    self.drawer = None;
                }
            });
        });
        ui.add_space(6.0);

        // A solid scroll bar takes its own space instead of floating over the value boxes.
        ui.spacing_mut().scroll = egui::style::ScrollStyle::solid();
        if sec == Section::Visuals {
            // Laid out from the bottom first, so Randomize / Undo / Reset stay in view however
            // long the scene's settings get.
            ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                self.visuals_footer(ui, c.settings);
                ui.separator();
                ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| self.drawer_body(ui, sec, c, t, accent, actions));
            });
        } else {
            self.drawer_body(ui, sec, c, t, accent, actions);
        }
    }

    pub(super) fn visuals_footer(&mut self, ui: &mut egui::Ui, s: &mut Settings) {
        let w = (ui.available_width() - 12.0) / 3.0;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let randomize = ui
                .add_sized([w, 28.0], egui::Button::new("Randomize"))
                .on_hover_text("Random shape, mode and effect settings for every scene")
                .clicked();
            let undo = ui
                .add_enabled_ui(!self.look_undo.is_empty(), |ui| ui.add_sized([w, 28.0], egui::Button::new("Undo")))
                .inner
                .on_hover_text("Back to the look before the last Randomize or Reset")
                .clicked();
            let reset = ui
                .add_sized([w, 28.0], egui::Button::new("Reset"))
                .on_hover_text("Visual settings back to defaults (keeps the scene and render scale)")
                .clicked();
            if randomize || reset {
                if self.look_undo.len() == UNDO_DEPTH {
                    self.look_undo.remove(0);
                }
                self.look_undo.push(s.clone());
                if randomize {
                    s.randomize_visuals();
                } else {
                    s.reset_visuals();
                }
            } else if undo {
                if let Some(prev) = self.look_undo.pop() {
                    s.copy_look(&prev);
                }
            }
        });
    }

    pub(super) fn drawer_body(&mut self, ui: &mut egui::Ui, sec: Section, c: &mut UiCtx, t: &Tokens, accent: Color32, actions: &mut Vec<UiAction>) {
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| match sec {
            Section::Visuals => visuals(ui, c.settings, t),
            Section::Audio => audio(ui, c, t, accent),
            Section::Palette => palette(ui, c, t),
            Section::General => self.general(ui, c, t, actions),
            Section::Hotkeys => hotkeys(ui, t),
        });
    }

    fn general(&mut self, ui: &mut egui::Ui, c: &mut UiCtx, t: &Tokens, actions: &mut Vec<UiAction>) {
        let s = &mut *c.settings;
        let d = Settings::default();
        let modes = [(WindowMode::Windowed, "Windowed"), (WindowMode::Fullscreen, "Fullscreen"), (WindowMode::Overlay, "Overlay")];
        ui.label(RichText::new("Mode").color(t.dim));
        let mut mode = c.mode;
        if segmented(ui, t, &mut mode, &modes) {
            actions.push(UiAction::SetMode(mode));
        }
        choice(ui, t, "Start in", &mut s.start_mode, d.start_mode, &modes);

        heading(ui, t, "Overlay");
        let overlay = c.mode == WindowMode::Overlay;
        if !overlay {
            ui.label(RichText::new("These apply in overlay mode. Switch to Overlay to change them.").small().color(t.faint));
        }
        ui.add_enabled_ui(overlay, |ui| {
            egui::Frame::new()
                .fill(t.card)
                .stroke(Stroke::new(1.0_f32, t.border))
                .corner_radius(CornerRadius::same(8))
                .inner_margin(Margin::same(10))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    if !c.transparent_ok {
                        ui.label(RichText::new("Transparent overlay unsupported on this GPU; overlay will be opaque.").small().color(Color32::from_rgb(230, 150, 70)));
                    }
                    knob(ui, t, "Opacity", &mut s.overlay_opacity, d.overlay_opacity, 0.2..=1.0, PCT);
                    ui.checkbox(&mut s.overlay_click_through, "Click-through");
                });
        });

        heading(ui, t, "Theme");
        let w = (ui.available_width() - 12.0) / 3.0;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            for (th, name) in [(UiTheme::Dark, "Dark"), (UiTheme::Oled, "OLED black"), (UiTheme::Light, "Light")] {
                if theme_card(ui, t, th, name, s.theme == th, w).clicked() {
                    s.theme = th;
                }
            }
        });
        ui.label(RichText::new("Changes the app's panels and controls. The visualizer keeps its own colours.").small().color(t.faint));

        heading(ui, t, "On the scene");
        ui.checkbox(&mut s.show_track_toast, "Now-playing toast on track change");
        ui.checkbox(&mut s.show_volume, "Volume bar");
        ui.checkbox(&mut s.show_timeline, "Track timeline (seek by dragging)");
        ui.checkbox(&mut s.show_stats, "Performance stats (F3)");
        knob(ui, t, "Idle frame cap", &mut s.idle_fps, d.idle_fps, 5..=60, num(0, " fps"));

        ui.separator();
        ui.label(RichText::new(format!("GPU: {}  ·  DX12", c.adapter)).small().color(t.faint));
        if !self.confirm_reset {
            if ui.small_button("Reset all settings…").clicked() {
                self.confirm_reset = true;
            }
        } else {
            egui::Frame::new()
                .fill(t.card)
                .stroke(Stroke::new(1.0_f32, t.border))
                .corner_radius(CornerRadius::same(8))
                .inner_margin(Margin::same(10))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new("Reset every setting to its default? Theme, startup mode, sidebar size and window position stay.").color(t.text));
                    ui.horizontal(|ui| {
                        if ui.button("Reset").clicked() {
                            let (mode, expanded, theme, window) = (s.start_mode, s.sidebar_expanded, s.theme, s.window);
                            *s = Settings::default();
                            s.start_mode = mode;
                            s.sidebar_expanded = expanded;
                            s.theme = theme;
                            s.window = window;
                            self.confirm_reset = false;
                        }
                        if ui.button("Cancel").clicked() {
                            self.confirm_reset = false;
                        }
                    });
                });
        }
    }
}

/// Scene picker, auto-switch, the scene's own settings, then the post effects every scene shares.
fn visuals(ui: &mut egui::Ui, s: &mut Settings, t: &Tokens) {
    let d = Settings::default();
    let w = ui.available_width();
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 4.0;
        for (i, id) in SceneId::ALL.into_iter().enumerate() {
            // The number keys only reach the first nine.
            let name = if i < 9 { format!("{}  {}", i + 1, id.name()) } else { id.name().to_owned() };
            if ui.add_sized([w, 28.0], egui::Button::new(name).selected(s.visual == id)).clicked() {
                s.visual = id;
            }
        }
    });
    let autos = [(AutoScene::Off, "Off"), (AutoScene::Track, "Each track"), (AutoScene::Timer, "Timed")];
    choice(ui, t, "Auto-switch scene", &mut s.auto_scene, d.auto_scene, &autos);
    if s.auto_scene == AutoScene::Timer {
        knob(ui, t, "Switch every", &mut s.auto_scene_minutes, d.auto_scene_minutes, 0.5..=30.0, num(1, " min"));
    }
    // A scene without settings of its own needs no section; the separator below covers it.
    if let Some(cfg) = s.scenes.config(s.visual) {
        ui.separator();
        ui.add_space(2.0);
        cfg.ui(ui, t);
    }
    ui.separator();
    knob(ui, t, "Intensity", &mut s.intensity, d.intensity, 0.2..=2.0, num(2, ""));
    knob(ui, t, "Bloom", &mut s.bloom_strength, d.bloom_strength, 0.0..=3.0, num(2, ""));
    knob(ui, t, "Bloom threshold", &mut s.bloom_threshold, d.bloom_threshold, 0.0..=3.0, num(2, ""));
    knob(ui, t, "Exposure", &mut s.exposure, d.exposure, 0.3..=3.0, num(2, ""));
    knob(ui, t, "Chromatic", &mut s.chromatic, d.chromatic, 0.0..=2.0, num(2, ""));
    knob(ui, t, "Film grain", &mut s.grain, d.grain, 0.0..=1.5, num(2, ""));
    knob(ui, t, "Vignette", &mut s.vignette, d.vignette, 0.0..=1.0, PCT);
    knob(ui, t, "Render scale", &mut s.render_scale, d.render_scale, 0.5..=1.0, PCT);
}

fn audio(ui: &mut egui::Ui, c: &mut UiCtx, t: &Tokens, accent: Color32) {
    let s = &mut *c.settings;
    let d = Settings::default();
    spectrum_meter(ui, c.vis, t, accent, 56.0);
    ui.label(RichText::new(c.device).small().color(t.faint));
    ui.add_space(4.0);
    knob(ui, t, "Sensitivity", &mut s.sensitivity, d.sensitivity, 0.3..=2.5, num(2, ""));
    knob(ui, t, "Attack", &mut s.attack_ms, d.attack_ms, 1.0..=80.0, num(0, " ms"));
    knob(ui, t, "Decay", &mut s.decay_ms, d.decay_ms, 30.0..=600.0, num(0, " ms"));
    knob(ui, t, "Beat sensitivity", &mut s.beat_sensitivity, d.beat_sensitivity, 0.3..=2.5, num(2, ""));
}

fn palette(ui: &mut egui::Ui, c: &mut UiCtx, t: &Tokens) {
    let s = &mut *c.settings;
    ui.checkbox(&mut s.album_palette, "Take palette from album art");
    let album_showing = s.album_palette && c.track.palette.is_some();
    heading(ui, t, "Fallback palette");
    let why = if !s.album_palette {
        "Album colours are off, so this palette is always used."
    } else if album_showing {
        "Album colours are showing. This palette is used when nothing is playing or the cover has no colours."
    } else {
        "Showing now: nothing is playing or the cover has no colours."
    };
    ui.label(RichText::new(why).small().color(t.faint));
    ui.add_space(2.0);
    ui.add_enabled_ui(!album_showing, |ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        for (i, (name, pal)) in PRESETS.iter().enumerate() {
            if palette_row(ui, t, name, pal, s.preset_palette == i).clicked() {
                s.preset_palette = i;
            }
        }
    });
    heading(ui, t, "Current colours");
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 28.0), Sense::hover());
    let w = rect.width() / 4.0;
    for (i, col) in c.palette.iter().enumerate() {
        let r = Rect::from_min_size(rect.min + egui::vec2(w * i as f32, 0.0), egui::vec2(w - 4.0, 28.0));
        ui.painter().rect_filled(r, CornerRadius::same(5), to_c32(*col, 255));
    }
}

fn hotkeys(ui: &mut egui::Ui, t: &Tokens) {
    let digits: Vec<String> = (1..=SceneId::ALL.len().min(9)).map(|i| i.to_string()).collect();
    let digits: Vec<&str> = digits.iter().map(String::as_str).collect();
    let rows: [(&str, &[&str]); 13] = [
        ("Sidebar", &["F1", "Tab"]),
        ("Scene", &digits),
        ("Next scene", &["S"]),
        ("Play / pause", &["Space"]),
        ("Next / previous track", &["N", "P"]),
        ("Seek 5 s", &["Left", "Right"]),
        ("Fullscreen", &["F11", "F", "Alt+Enter"]),
        ("Overlay mode", &["O"]),
        ("Show now playing", &["T"]),
        ("Volume up / down", &["Up", "Down"]),
        ("Mute", &["M"]),
        ("Performance stats", &["F3"]),
        ("Back", &["Esc"]),
    ];
    for (d, keys) in rows {
        key_row(ui, t, d, keys);
    }
    ui.label(RichText::new("Double-click the scene to toggle fullscreen.").small().color(t.faint));
    ui.add_space(10.0);
    ui.label(RichText::new("Global (work from any app, and in overlay)").small().strong().color(t.dim));
    ui.add_space(2.0);
    let global: [(&str, &[&str]); 2] = [("Overlay mode", &["Ctrl+Alt+O"]), ("Sidebar", &["Ctrl+Alt+S"])];
    for (d, keys) in global {
        key_row(ui, t, d, keys);
    }
}

/// Selectable palette preset: its four colours, then its name.
pub(super) fn palette_row(ui: &mut egui::Ui, t: &Tokens, name: &str, pal: &Palette, selected: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 30.0), Sense::click());
    let p = ui.painter();
    if selected {
        p.rect_filled(rect, CornerRadius::same(7), ui.visuals().selection.bg_fill);
    } else if resp.hovered() {
        p.rect_filled(rect, CornerRadius::same(7), t.hover);
    }
    for (i, col) in pal.iter().enumerate() {
        let r = Rect::from_min_size(egui::pos2(rect.left() + 10.0 + i as f32 * 17.0, rect.center().y - 7.0), Vec2::splat(14.0));
        p.rect_filled(r, CornerRadius::same(3), to_c32(*col, 255));
        p.rect_stroke(r, CornerRadius::same(3), Stroke::new(1.0_f32, t.border), StrokeKind::Inside);
    }
    let col = if selected || resp.hovered() { t.text } else { t.dim };
    p.text(egui::pos2(rect.left() + 10.0 + 4.0 * 17.0 + 8.0, rect.center().y), Align2::LEFT_CENTER, name, FontId::proportional(13.5), col);
    resp
}

/// Hotkey row: description on the left, key caps right-aligned.
pub(super) fn key_row(ui: &mut egui::Ui, t: &Tokens, desc: &str, keys: &[&str]) {
    ui.horizontal(|ui| {
        ui.set_min_height(26.0);
        ui.label(RichText::new(desc).color(t.text));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            for k in keys.iter().rev() {
                egui::Frame::new()
                    .fill(t.control)
                    .stroke(Stroke::new(1.0_f32, t.border))
                    .corner_radius(CornerRadius::same(5))
                    .inner_margin(Margin::symmetric(6, 2))
                    .show(ui, |ui| ui.label(RichText::new(*k).monospace().color(t.text)));
            }
        });
    });
}

/// Clickable preview of a theme: a miniature sidebar and panel in that theme's colours.
pub(super) fn theme_card(ui: &mut egui::Ui, t: &Tokens, theme: UiTheme, name: &str, selected: bool, w: f32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, 84.0), Sense::click());
    let p = ui.painter();
    let th = tokens(theme);
    let preview = Rect::from_min_size(rect.min, egui::vec2(w, 58.0));
    p.rect_filled(preview, CornerRadius::same(7), th.drawer);
    let side = Rect::from_min_size(preview.min, egui::vec2(w * 0.32, preview.height()));
    p.rect_filled(side, CornerRadius { nw: 7, sw: 7, ne: 0, se: 0 }, th.sidebar);
    p.line_segment([side.right_top(), side.right_bottom()], Stroke::new(1.0_f32, th.border));
    for i in 0..4 {
        let y = side.top() + 10.0 + i as f32 * 11.0;
        let fill = if i == 0 { th.text } else { th.faint };
        p.rect_filled(Rect::from_min_size(egui::pos2(side.left() + 6.0, y), egui::vec2(side.width() - 12.0, 4.0)), CornerRadius::same(2), fill);
    }
    for (i, frac) in [0.7, 0.5, 0.6].iter().enumerate() {
        let y = preview.top() + 12.0 + i as f32 * 13.0;
        let x0 = side.right() + 8.0;
        let width = (preview.right() - 8.0 - x0) * frac;
        p.rect_filled(Rect::from_min_size(egui::pos2(x0, y), egui::vec2(width, 5.0)), CornerRadius::same(2), if i == 0 { th.text } else { th.dim });
    }
    let ring = if selected { Stroke::new(2.0_f32, t.text) } else if resp.hovered() { Stroke::new(1.0_f32, t.dim) } else { Stroke::new(1.0_f32, t.border) };
    p.rect_stroke(preview, CornerRadius::same(7), ring, StrokeKind::Outside);
    let col = if selected { t.text } else { t.dim };
    p.text(egui::pos2(rect.center().x, rect.bottom() - 9.0), Align2::CENTER_CENTER, name, FontId::proportional(12.5), col);
    resp
}
