//! egui UI: docked sidebar (expanded or icon rail) with a settings drawer, on-scene controls
//! (window buttons, volume bar), now-playing toast, and stats HUD.

use egui::{Align2, Color32, CornerRadius, FontId, Margin, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Vec2};

use crate::media::{TrackInfo, ART_SIZE};
use crate::palette::{Palette, PRESETS};
use crate::settings::{AsciiShape, Scene, ScopeMode, Settings, UiTheme, WindowMode};
use crate::sys::ProcStats;
use crate::vis::VisState;
use crate::volume::Volume;
use crate::wake::{WakeSlider, WakeStyle};

const SIDEBAR_W: f32 = 224.0;
const RAIL_W: f32 = 56.0;
const DRAWER_W: f32 = 300.0;

pub enum UiAction {
    SetMode(WindowMode),
    HideSidebar,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Visuals,
    Audio,
    Palette,
    /// Window modes, overlay, theme and the on-scene extras.
    General,
    Hotkeys,
}

const SECTIONS: [(Section, &str, Icon); 5] = [
    (Section::Visuals, "Visuals", Icon::Visuals),
    (Section::Audio, "Audio", Icon::Audio),
    (Section::Palette, "Palette", Icon::Palette),
    (Section::General, "General", Icon::General),
    (Section::Hotkeys, "Hotkeys", Icon::Keys),
];

/// Looks to go back to with Undo (taken before each Randomize or Reset).
const UNDO_DEPTH: usize = 32;

/// Neutral colour tokens for one UI theme. Album colours only appear in data (spectrum, swatches).
#[derive(Clone, Copy)]
struct Tokens {
    sidebar: Color32,
    drawer: Color32,
    border: Color32,
    text: Color32,
    dim: Color32,
    faint: Color32,
    hover: Color32,
    selected: Color32,
    card: Color32,
    control: Color32,
    /// Translucent fill for things floating over the scene (toast, stats, scene controls).
    float: Color32,
}

fn tokens(t: UiTheme) -> Tokens {
    let g = Color32::from_gray;
    match t {
        UiTheme::Dark => Tokens {
            sidebar: g(17),
            drawer: g(22),
            border: g(38),
            text: g(240),
            dim: g(160),
            faint: g(112),
            hover: Color32::from_white_alpha(10),
            selected: Color32::from_white_alpha(24),
            card: Color32::from_white_alpha(8),
            control: Color32::from_white_alpha(14),
            float: Color32::from_rgba_unmultiplied(17, 17, 17, 215),
        },
        UiTheme::Oled => Tokens {
            sidebar: Color32::BLACK,
            drawer: Color32::BLACK,
            border: g(30),
            text: g(240),
            dim: g(155),
            faint: g(105),
            hover: Color32::from_white_alpha(12),
            selected: Color32::from_white_alpha(26),
            card: g(12),
            control: Color32::from_white_alpha(16),
            float: Color32::from_rgba_unmultiplied(0, 0, 0, 215),
        },
        UiTheme::Light => Tokens {
            sidebar: g(243),
            drawer: g(250),
            border: g(222),
            text: g(23),
            dim: g(82),
            faint: g(130),
            hover: Color32::from_black_alpha(10),
            selected: Color32::from_black_alpha(20),
            card: Color32::from_black_alpha(6),
            control: Color32::from_black_alpha(12),
            float: Color32::from_rgba_unmultiplied(250, 250, 250, 225),
        },
    }
}

fn with_alpha(c: Color32, a: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), a)
}

pub struct UiState {
    art: Option<egui::TextureHandle>,
    /// Theme the egui style was last built for.
    styled: Option<UiTheme>,
    /// Settings section open in the drawer beside the sidebar.
    pub drawer: Option<Section>,
    volume_slider: WakeSlider,
    /// The pointer is on the volume bar (or dragging it), so it shouldn't fade out.
    pub volume_hot: bool,
    look_undo: Vec<Settings>,
    /// "Reset all settings" was clicked and is waiting for a confirm.
    confirm_reset: bool,
}

pub struct UiCtx<'a> {
    pub settings: &'a mut Settings,
    pub track: &'a TrackInfo,
    pub vis: &'a VisState,
    pub palette: Palette,
    pub mode: WindowMode,
    pub stats: &'a ProcStats,
    pub fps: f32,
    pub frame_ms: f32,
    pub idle: bool,
    pub device: &'a str,
    pub adapter: &'a str,
    pub transparent_ok: bool,
    pub show_sidebar: bool,
    pub toast_alpha: f32,
    /// Opacity of the on-scene window buttons (fades after the mouse rests).
    pub controls_alpha: f32,
    pub volume: &'a Volume,
    /// Opacity of the volume bar: the on-scene controls, or a flash after a volume key.
    pub volume_alpha: f32,
}

fn to_c32(c: [f32; 3], a: u8) -> Color32 {
    let f = |v: f32| (crate::palette::linear_to_srgb(v.clamp(0.0, 1.0)) * 255.0) as u8;
    Color32::from_rgba_unmultiplied(f(c[0]), f(c[1]), f(c[2]), a)
}

impl UiState {
    pub fn new() -> Self {
        Self {
            art: None,
            styled: None,
            drawer: None,
            volume_slider: WakeSlider::default(),
            volume_hot: false,
            look_undo: Vec::new(),
            confirm_reset: false,
        }
    }

    pub fn set_art(&mut self, ctx: &egui::Context, rgba: Option<&[u8]>) {
        self.art = rgba.map(|px| {
            let img = egui::ColorImage::from_rgba_unmultiplied([ART_SIZE as usize, ART_SIZE as usize], px);
            ctx.load_texture("album-art", img, egui::TextureOptions::LINEAR)
        });
    }

    fn style(&mut self, ctx: &egui::Context, theme: UiTheme) {
        if self.styled == Some(theme) {
            return;
        }
        self.styled = Some(theme);
        let t = tokens(theme);
        let light = theme == UiTheme::Light;
        ctx.set_theme(if light { egui::Theme::Light } else { egui::Theme::Dark });
        ctx.all_styles_mut(|s| {
            use egui::{FontFamily, TextStyle};
            s.text_styles = [
                (TextStyle::Heading, FontId::new(17.0, FontFamily::Proportional)),
                (TextStyle::Body, FontId::new(13.5, FontFamily::Proportional)),
                (TextStyle::Button, FontId::new(13.5, FontFamily::Proportional)),
                (TextStyle::Small, FontId::new(11.0, FontFamily::Proportional)),
                (TextStyle::Monospace, FontId::new(12.0, FontFamily::Monospace)),
            ]
            .into();
            s.spacing.item_spacing = egui::vec2(8.0, 7.0);
            s.spacing.slider_width = 150.0;
            s.spacing.button_padding = egui::vec2(10.0, 5.0);

            s.visuals = if light { egui::Visuals::light() } else { egui::Visuals::dark() };
            let v = &mut s.visuals;
            let strong = if light { Color32::from_black_alpha(22) } else { Color32::from_white_alpha(28) };
            v.panel_fill = t.sidebar;
            v.window_fill = with_alpha(t.drawer, 255);
            v.window_stroke = Stroke::new(1.0_f32, t.border);
            v.window_corner_radius = CornerRadius::same(10);
            v.menu_corner_radius = CornerRadius::same(8);
            v.extreme_bg_color = t.control;
            v.faint_bg_color = t.card;
            v.hyperlink_color = t.text;
            v.selection.bg_fill = if light { Color32::from_gray(200) } else { Color32::from_gray(72) };
            v.selection.stroke = Stroke::new(1.0_f32, t.text);
            let w = &mut v.widgets;
            w.noninteractive.bg_fill = t.card;
            w.noninteractive.bg_stroke = Stroke::new(1.0_f32, t.border);
            w.noninteractive.fg_stroke = Stroke::new(1.0_f32, t.text);
            for (st, fill) in [(&mut w.inactive, t.control), (&mut w.hovered, strong), (&mut w.active, strong), (&mut w.open, strong)] {
                st.bg_fill = fill;
                st.weak_bg_fill = fill;
                st.bg_stroke = Stroke::NONE;
                st.fg_stroke = Stroke::new(1.0_f32, t.text);
                st.corner_radius = CornerRadius::same(7);
                st.expansion = 0.0;
            }
            w.hovered.bg_stroke = Stroke::new(1.0_f32, t.faint);
        });
    }

    /// Draws the UI and returns the actions taken plus the rect (in points) left for the scene.
    pub fn draw(&mut self, ctx: &egui::Context, mut c: UiCtx) -> (Vec<UiAction>, Rect) {
        self.style(ctx, c.settings.theme);
        if self.drawer != Some(Section::General) {
            self.confirm_reset = false;
        }
        let t = tokens(c.settings.theme);
        // Things floating over the scene sit on dark visuals, so they stay dark in the light theme.
        let ft = tokens(if c.settings.theme == UiTheme::Light { UiTheme::Dark } else { c.settings.theme });
        let mut actions = Vec::new();
        let accent = to_c32(c.palette[0], 255);

        // Panels are opaque, except over the desktop in overlay mode.
        let alpha = if c.mode == WindowMode::Overlay { 236 } else { 255 };
        let mut scene_rect = ctx.content_rect();
        if c.show_sidebar {
            let w = if c.settings.sidebar_expanded { SIDEBAR_W } else { RAIL_W };
            egui::SidePanel::left("sidebar")
                .resizable(false)
                .exact_width(w)
                .frame(
                    egui::Frame::new()
                        .fill(with_alpha(t.sidebar, alpha))
                        .stroke(Stroke::new(1.0_f32, t.border))
                        .inner_margin(Margin::symmetric(8, 10)),
                )
                .show(ctx, |ui| self.sidebar(ui, &mut c, &t, accent, &mut actions));

            if let Some(sec) = self.drawer {
                egui::SidePanel::left("drawer")
                    .resizable(false)
                    .exact_width(DRAWER_W)
                    .frame(
                        egui::Frame::new()
                            .fill(with_alpha(t.drawer, alpha))
                            .stroke(Stroke::new(1.0_f32, t.border))
                            .inner_margin(Margin::same(16)),
                    )
                    .show(ctx, |ui| self.drawer(ui, sec, &mut c, &t, accent, &mut actions));
            }
            scene_rect = ctx.available_rect();
        }

        if c.settings.show_stats {
            egui::Area::new(egui::Id::new("stats"))
                .fixed_pos(scene_rect.min + egui::vec2(14.0, 12.0))
                .interactable(false)
                .show(ctx, |ui| {
                    egui::Frame::new()
                        .fill(ft.float)
                        .corner_radius(CornerRadius::same(8))
                        .inner_margin(Margin::symmetric(10, 6))
                        .show(ui, |ui| {
                            let txt = format!(
                                "{:>5.1} fps  {:>5.2} ms{}\nCPU {:>4.1}%   RAM {:>4.1} MB (private {:.1})\nBPM {:>5.1}",
                                c.fps,
                                c.frame_ms,
                                if c.idle { "  idle" } else { "" },
                                c.stats.cpu_percent,
                                c.stats.working_set_mb,
                                c.stats.private_mb,
                                c.vis.bpm
                            );
                            ui.label(RichText::new(txt).monospace().color(ft.text));
                        });
                });
        }

        if c.toast_alpha > 0.0 && c.settings.show_track_toast && !c.track.title.is_empty() {
            // An Area keeps the size it measured last time, so a longer title would wrap to the
            // previous track's width. A fresh id per track makes it measure again.
            egui::Area::new(egui::Id::new(("toast", &c.track.title, &c.track.artist, &c.track.album)))
                .pivot(Align2::LEFT_BOTTOM)
                // Rises above the volume bar while that is showing.
                .fixed_pos(scene_rect.left_bottom() + egui::vec2(26.0, -26.0 - 64.0 * c.volume_alpha))
                .interactable(false)
                .show(ctx, |ui| {
                    ui.multiply_opacity(c.toast_alpha);
                    egui::Frame::new()
                        .fill(ft.float)
                        .corner_radius(CornerRadius::same(14))
                        .stroke(Stroke::new(1.0_f32, ft.border))
                        .inner_margin(Margin::same(10))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                if let Some(tex) = &self.art {
                                    ui.add(egui::Image::new((tex.id(), Vec2::splat(64.0))).corner_radius(CornerRadius::same(9)));
                                }
                                ui.vertical(|ui| {
                                    ui.set_max_width(360.0);
                                    ui.add_space(6.0);
                                    ui.label(RichText::new(&c.track.title).size(20.0).strong().color(ft.text));
                                    ui.label(RichText::new(&c.track.artist).size(14.0).color(ft.dim));
                                    if !c.track.album.is_empty() {
                                        ui.label(RichText::new(&c.track.album).size(11.5).color(ft.faint));
                                    }
                                });
                                ui.add_space(8.0);
                            });
                        });
                });
        }

        if c.controls_alpha > 0.0 {
            window_buttons(ctx, scene_rect, &c, &ft, &mut actions);
        }
        self.volume_hot = false;
        if c.volume_alpha > 0.0 && c.settings.show_volume && c.volume.available() {
            self.volume_bar(ctx, scene_rect, &c, &ft, accent);
        }

        (actions, scene_rect)
    }

    /// Output volume over the bottom centre of the scene: mute button, wake slider, percent.
    fn volume_bar(&mut self, ctx: &egui::Context, scene: Rect, c: &UiCtx, t: &Tokens, accent: Color32) {
        let vol = c.volume;
        let muted = vol.muted();
        let level = (vol.level() * 100.0).round();
        let style = WakeStyle {
            bars: 28,
            height: 30.0,
            rest_height: 8.0,
            gap: 3.0,
            fill: if muted { t.faint } else { t.text },
            track: t.control,
            crest: Some(accent),
            handle: Some(if muted { t.faint } else { t.text }),
            // The bar is short, so a relaxed drag should already raise most of the wake: full
            // height at ~130 %/s (crossing the track in under a second) instead of 320 %/s.
            sensitivity: 2.5,
            reach: 7.0,
            ..WakeStyle::default()
        };
        let slider_w = (scene.width() - 160.0).clamp(120.0, 260.0);
        let resp = egui::Area::new(egui::Id::new("volume"))
            .pivot(Align2::CENTER_BOTTOM)
            .fixed_pos(scene.center_bottom() + egui::vec2(0.0, -18.0))
            .show(ctx, |ui| {
                ui.multiply_opacity(c.volume_alpha);
                egui::Frame::new()
                    .fill(t.float)
                    .stroke(Stroke::new(1.0_f32, t.border))
                    .corner_radius(CornerRadius::same(22))
                    .inner_margin(Margin { left: 6, right: 14, top: 4, bottom: 4 })
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 8.0;
                            let icon = if muted || level <= 0.0 { Icon::Muted } else { Icon::Speaker };
                            let tip = if muted { "Unmute (M)" } else { "Mute (M)" };
                            if icon_button(ui, t, icon, 30.0).on_hover_text(tip).clicked() {
                                vol.set_muted(!muted);
                            }
                            let (r, changed) = self.volume_slider.show(ui, level, slider_w, &style);
                            if let Some(v) = changed {
                                vol.set_level(v / 100.0);
                            }
                            let shown = changed.unwrap_or(level);
                            let text = RichText::new(format!("{shown:>3.0}%")).monospace().color(if muted { t.faint } else { t.dim });
                            ui.add_sized([36.0, 30.0], egui::Label::new(text));
                            r.dragged()
                        })
                        .inner
                    })
            });
        self.volume_hot = resp.inner.inner || resp.response.contains_pointer();
    }

    fn sidebar(&mut self, ui: &mut egui::Ui, c: &mut UiCtx, t: &Tokens, accent: Color32, actions: &mut Vec<UiAction>) {
        let expanded = c.settings.sidebar_expanded;

        // Logo.
        let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 34.0), Sense::hover());
        let p = ui.painter();
        let ic = egui::pos2(rect.left() + 20.0, rect.center().y);
        paint_icon(p, Icon::Logo, ic, t.text);
        if expanded {
            let font = FontId::proportional(16.0);
            let r = p.text(ic + egui::vec2(18.0, 0.0), Align2::LEFT_CENTER, "BLAZE", font.clone(), t.text);
            p.text(egui::pos2(r.right(), ic.y), Align2::LEFT_CENTER, "AUDIO", font, t.faint);
        }
        ui.add_space(10.0);

        for (sec, name, icon) in SECTIONS {
            let open = self.drawer == Some(sec);
            if nav_item(ui, t, icon, name, open, expanded).clicked() {
                self.drawer = if open { None } else { Some(sec) };
            }
        }

        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            let (icon, label) = if expanded { (Icon::Collapse, "Collapse") } else { (Icon::Expand, "Expand") };
            if nav_item(ui, t, icon, label, false, expanded).clicked() {
                c.settings.sidebar_expanded = !expanded;
            }
            if c.mode != WindowMode::Windowed && nav_item(ui, t, Icon::HideSidebar, "Hide sidebar", false, expanded).on_hover_text("F1").clicked() {
                actions.push(UiAction::HideSidebar);
            }
            ui.add_space(6.0);
            self.now_playing(ui, c, t, accent, expanded);
        });
    }

    fn now_playing(&self, ui: &mut egui::Ui, c: &UiCtx, t: &Tokens, accent: Color32, expanded: bool) {
        let tip = if c.track.title.is_empty() { "Nothing playing".to_owned() } else { format!("{}\n{}", c.track.title, c.track.artist) };
        if !expanded {
            let (rect, resp) = ui.allocate_exact_size(Vec2::splat(40.0), Sense::hover());
            self.paint_art(ui, rect, t, 7);
            resp.on_hover_text(tip);
            return;
        }
        // The sidebar footer lays out bottom-up; give the card a fixed top-down box of its own.
        let size = egui::vec2(ui.available_width(), 12.0 * 2.0 + 48.0 + 10.0 + 28.0);
        ui.allocate_ui_with_layout(size, egui::Layout::top_down(egui::Align::Min), |ui| {
            egui::Frame::new()
                .fill(t.card)
                .stroke(Stroke::new(1.0_f32, t.border))
                .corner_radius(CornerRadius::same(10))
                .inner_margin(Margin::same(12))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 10.0;
                        let (rect, _) = ui.allocate_exact_size(Vec2::splat(48.0), Sense::hover());
                        self.paint_art(ui, rect, t, 7);
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 3.0;
                            ui.add_space(6.0);
                            if c.track.title.is_empty() {
                                ui.add(egui::Label::new(RichText::new("Nothing playing").color(t.dim)).truncate());
                            } else {
                                ui.add(egui::Label::new(RichText::new(&c.track.title).size(14.0).strong().color(t.text)).truncate());
                                ui.add(egui::Label::new(RichText::new(&c.track.artist).size(12.0).color(t.dim)).truncate());
                            }
                        });
                    })
                    .response
                    .on_hover_text(tip);
                    ui.add_space(10.0 - ui.spacing().item_spacing.y);
                    spectrum_meter(ui, c.vis, t, accent, 28.0);
                });
        });
    }

    fn paint_art(&self, ui: &egui::Ui, rect: Rect, t: &Tokens, radius: u8) {
        match &self.art {
            Some(tex) => {
                egui::Image::new((tex.id(), rect.size())).corner_radius(CornerRadius::same(radius)).paint_at(ui, rect);
            }
            None => {
                ui.painter().rect_filled(rect, CornerRadius::same(radius), t.control);
                paint_icon(ui.painter(), Icon::Audio, rect.center(), t.faint);
            }
        }
    }

    fn drawer(&mut self, ui: &mut egui::Ui, sec: Section, c: &mut UiCtx, t: &Tokens, accent: Color32, actions: &mut Vec<UiAction>) {
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

    fn visuals_footer(&mut self, ui: &mut egui::Ui, s: &mut Settings) {
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

    fn drawer_body(&mut self, ui: &mut egui::Ui, sec: Section, c: &mut UiCtx, t: &Tokens, accent: Color32, actions: &mut Vec<UiAction>) {
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let s = &mut *c.settings;
            let d = Settings::default();
            match sec {
                Section::Visuals => {
                    let w = ui.available_width();
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 4.0;
                        for (sc, name) in [
                            (Scene::Terminal, "1  Faulty Terminal"),
                            (Scene::GradientAscii, "2  Gradient ASCII"),
                            (Scene::Scope, "3  Phosphor Scope"),
                        ] {
                            if ui.add_sized([w, 28.0], egui::Button::new(name).selected(s.visual == sc)).clicked() {
                                s.visual = sc;
                            }
                        }
                    });
                    if s.visual == Scene::Scope {
                        ui.add_space(2.0);
                        choice(ui, t, "Mode", &mut s.scope_mode, d.scope_mode, &[(ScopeMode::Yt, "Y-T sweep"), (ScopeMode::Polar, "Polar")]);
                        knob(ui, t, "Persistence", &mut s.scope_persistence, d.scope_persistence, 0.02..=1.5, num(2, " s").log());
                        knob(ui, t, "Beam width", &mut s.scope_beam, d.scope_beam, 1.0..=10.0, num(1, " px"));
                        knob(ui, t, "Gain", &mut s.scope_gain, d.scope_gain, 0.25..=3.0, num(2, "×"));
                    }
                    if s.visual == Scene::GradientAscii {
                        ui.add_space(2.0);
                        let shapes = [(AsciiShape::Horizon, "Horizon"), (AsciiShape::Pinch, "Pinch"), (AsciiShape::Rise, "Rise"), (AsciiShape::Eclipse, "Eclipse")];
                        choice(ui, t, "Shape", &mut s.ascii_shape, d.ascii_shape, &shapes);
                        knob(ui, t, "Cell size", &mut s.ascii_cell, d.ascii_cell, 8.0..=48.0, num(0, " px"));
                        knob(ui, t, "Density", &mut s.ascii_density, d.ascii_density, 0.0..=1.0, PCT);
                        knob(ui, t, "Glyph opacity", &mut s.ascii_glyphs, d.ascii_glyphs, 0.0..=1.0, PCT);
                        knob(ui, t, "Gradient opacity", &mut s.ascii_gradient, d.ascii_gradient, 0.0..=1.0, PCT);
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
                Section::Audio => {
                    spectrum_meter(ui, c.vis, t, accent, 56.0);
                    ui.label(RichText::new(c.device).small().color(t.faint));
                    ui.add_space(4.0);
                    knob(ui, t, "Sensitivity", &mut s.sensitivity, d.sensitivity, 0.3..=2.5, num(2, ""));
                    knob(ui, t, "Attack", &mut s.attack_ms, d.attack_ms, 1.0..=80.0, num(0, " ms"));
                    knob(ui, t, "Decay", &mut s.decay_ms, d.decay_ms, 30.0..=600.0, num(0, " ms"));
                    knob(ui, t, "Beat sensitivity", &mut s.beat_sensitivity, d.beat_sensitivity, 0.3..=2.5, num(2, ""));
                }
                Section::Palette => {
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
                Section::General => {
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
                                ui.label(RichText::new("Reset every setting to its default? Theme, startup mode and sidebar size stay.").color(t.text));
                                ui.horizontal(|ui| {
                                    if ui.button("Reset").clicked() {
                                        let (mode, expanded, theme) = (s.start_mode, s.sidebar_expanded, s.theme);
                                        *s = Settings::default();
                                        s.start_mode = mode;
                                        s.sidebar_expanded = expanded;
                                        s.theme = theme;
                                        self.confirm_reset = false;
                                    }
                                    if ui.button("Cancel").clicked() {
                                        self.confirm_reset = false;
                                    }
                                });
                            });
                    }
                }
                Section::Hotkeys => {
                    let rows: [(&str, &[&str]); 9] = [
                        ("Sidebar", &["F1", "Tab"]),
                        ("Scene", &["1", "2", "3"]),
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
            }
        });
    }
}

/// Fullscreen / overlay buttons over the top-right corner of the scene.
fn window_buttons(ctx: &egui::Context, scene: Rect, c: &UiCtx, t: &Tokens, actions: &mut Vec<UiAction>) {
    let frame = egui::Frame::new()
        .fill(t.float)
        .stroke(Stroke::new(1.0_f32, t.border))
        .corner_radius(CornerRadius::same(18))
        .inner_margin(Margin::symmetric(6, 4));

    egui::Area::new(egui::Id::new("window-buttons"))
        .pivot(Align2::RIGHT_TOP)
        .fixed_pos(scene.right_top() + egui::vec2(-12.0, 12.0))
        .show(ctx, |ui| {
            ui.multiply_opacity(c.controls_alpha);
            frame.show(ui, |ui| {
                ui.horizontal(|ui| {
                    let ov = c.mode == WindowMode::Overlay;
                    let tip = if ov { "Leave overlay (O)" } else { "Overlay mode (O)" };
                    if icon_button(ui, t, Icon::Overlay, 26.0).on_hover_text(tip).clicked() {
                        actions.push(UiAction::SetMode(if ov { WindowMode::Windowed } else { WindowMode::Overlay }));
                    }
                    let fs = c.mode == WindowMode::Fullscreen;
                    let tip = if fs { "Exit fullscreen (F11)" } else { "Fullscreen (F11)" };
                    if icon_button(ui, t, Icon::Fullscreen, 26.0).on_hover_text(tip).clicked() {
                        actions.push(UiAction::SetMode(if fs { WindowMode::Windowed } else { WindowMode::Fullscreen }));
                    }
                });
            });
        });
}

/// One sidebar row: icon, plus a label when expanded (a tooltip stands in for it on the rail).
fn nav_item(ui: &mut egui::Ui, t: &Tokens, icon: Icon, label: &str, selected: bool, expanded: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 36.0), Sense::click());
    let p = ui.painter();
    let hovered = resp.hovered();
    if selected {
        p.rect_filled(rect, CornerRadius::same(7), t.selected);
    } else if hovered {
        p.rect_filled(rect, CornerRadius::same(7), t.hover);
    }
    let col = if selected || hovered { t.text } else { t.dim };
    let ic = egui::pos2(rect.left() + 20.0, rect.center().y);
    paint_icon(p, icon, ic, col);
    if expanded {
        p.text(ic + egui::vec2(18.0, 0.0), Align2::LEFT_CENTER, label, FontId::proportional(13.5), col);
        resp
    } else {
        resp.on_hover_text(label)
    }
}

fn icon_button(ui: &mut egui::Ui, t: &Tokens, icon: Icon, size: f32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    let hovered = resp.hovered();
    if hovered {
        ui.painter().rect_filled(rect, CornerRadius::same(6), t.hover);
    }
    paint_icon(ui.painter(), icon, rect.center(), if hovered { t.text } else { t.dim });
    resp
}

/// Width reserved for a slider's value box; wide enough for "140 ms", "0.08 s" or "100%".
const VALUE_W: f32 = 72.0;

/// How a slider shows its value: fixed decimals and a unit, or a 0..=1 value as a percentage.
#[derive(Clone, Copy)]
struct Fmt {
    decimals: usize,
    suffix: &'static str,
    pct: bool,
    log: bool,
}

const fn num(decimals: usize, suffix: &'static str) -> Fmt {
    Fmt { decimals, suffix, pct: false, log: false }
}

/// Whole percent; the value snaps to 0.01.
const PCT: Fmt = Fmt { decimals: 2, suffix: "%", pct: true, log: false };

impl Fmt {
    const fn log(self) -> Self {
        Self { log: true, ..self }
    }

    fn show(self, v: f64) -> String {
        if self.pct {
            format!("{:.0}{}", v * 100.0, self.suffix)
        } else {
            format!("{:.*}{}", self.decimals, v, self.suffix)
        }
    }

    fn apply(self, s: egui::Slider<'_>) -> egui::Slider<'_> {
        let s = s.fixed_decimals(self.decimals).suffix(self.suffix).logarithmic(self.log);
        if self.pct {
            s.custom_formatter(|v, _| format!("{:.0}", v * 100.0))
                .custom_parser(|s| s.trim().trim_end_matches('%').trim().parse::<f64>().ok().map(|v| v / 100.0))
        } else {
            s
        }
    }
}

/// The line above a control: its label, brighter and with a reset button when the value isn't
/// the default. Double-clicking the label also resets. Returns true when a reset was asked for.
fn label_row(ui: &mut egui::Ui, t: &Tokens, label: &str, modified: bool, default: &str) -> bool {
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        ui.set_min_height(18.0);
        let col = if modified { t.text } else { t.dim };
        let tip = if modified { format!("Default: {default}. Double-click to reset.") } else { format!("Default: {default}") };
        let text = egui::Label::new(RichText::new(label).color(col)).selectable(false).sense(Sense::click());
        let mut reset = ui.add(text).on_hover_text(tip).double_clicked();
        if modified {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                reset |= icon_button(ui, t, Icon::Reset, 18.0).on_hover_text(format!("Reset to {default}")).clicked();
            });
        }
        reset && modified
    })
    .inner
}

/// A slider with its label on the line above, so long labels fit the drawer. Double-click the
/// label or the slider to put it back to `default`.
fn knob<T: egui::emath::Numeric>(ui: &mut egui::Ui, t: &Tokens, label: &str, value: &mut T, default: T, range: std::ops::RangeInclusive<T>, fmt: Fmt) {
    let shown = fmt.show(default.to_f64());
    // Compared as shown, so a value that only differs past the last digit isn't flagged.
    let mut reset = label_row(ui, t, label, fmt.show(value.to_f64()) != shown, &shown);
    ui.scope(|ui| {
        // Slider, gap and a fixed-width value box fill the row exactly, so a wide value can't
        // push the drawer past its width.
        let w = ui.available_width();
        let s = ui.spacing_mut();
        s.slider_width = (w - VALUE_W - s.item_spacing.x).max(40.0);
        s.interact_size.x = VALUE_W;
        let rail_w = s.slider_width;
        let r = ui.add(fmt.apply(egui::Slider::new(value, range)));
        // The rail only senses drags, so look for the double-click ourselves. The value box is
        // left out: a click there starts typing.
        let rail = Rect::from_min_size(r.rect.min, egui::vec2(rail_w, r.rect.height()));
        reset |= ui.rect_contains_pointer(rail) && ui.input(|i| i.pointer.button_double_clicked(egui::PointerButton::Primary));
    });
    if reset {
        *value = default;
    }
}

/// Segmented buttons with a label above, like `knob`.
fn choice<T: Copy + PartialEq>(ui: &mut egui::Ui, t: &Tokens, label: &str, value: &mut T, default: T, options: &[(T, &str)]) {
    let name = options.iter().find(|o| o.0 == default).map_or("", |o| o.1);
    if label_row(ui, t, label, *value != default, name) {
        *value = default;
    }
    segmented(ui, t, value, options);
}

/// Equal-width segments, one per option. Returns true when the user picked a different one.
fn segmented<T: Copy + PartialEq>(ui: &mut egui::Ui, t: &Tokens, value: &mut T, options: &[(T, &str)]) -> bool {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 28.0), Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(7), t.control);
    let w = rect.width() / options.len() as f32;
    let mut changed = false;
    for (i, (v, name)) in options.iter().enumerate() {
        let seg = Rect::from_min_size(rect.min + egui::vec2(w * i as f32, 0.0), egui::vec2(w, rect.height()));
        let r = ui.interact(seg, resp.id.with(i), Sense::click());
        let on = *v == *value;
        if on {
            p.rect_filled(seg.shrink(2.0), CornerRadius::same(5), ui.visuals().selection.bg_fill);
        } else if r.hovered() {
            p.rect_filled(seg.shrink(2.0), CornerRadius::same(5), t.hover);
        }
        p.text(seg.center(), Align2::CENTER_CENTER, *name, FontId::proportional(13.0), if on || r.hovered() { t.text } else { t.dim });
        if r.clicked() && !on {
            *value = *v;
            changed = true;
        }
    }
    changed
}

/// Small heading that opens a group of settings in a drawer.
fn heading(ui: &mut egui::Ui, t: &Tokens, text: &str) {
    ui.add_space(8.0);
    ui.label(RichText::new(text).small().strong().color(t.dim));
}

/// Selectable palette preset: its four colours, then its name.
fn palette_row(ui: &mut egui::Ui, t: &Tokens, name: &str, pal: &Palette, selected: bool) -> egui::Response {
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
fn key_row(ui: &mut egui::Ui, t: &Tokens, desc: &str, keys: &[&str]) {
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
fn theme_card(ui: &mut egui::Ui, t: &Tokens, theme: UiTheme, name: &str, selected: bool, w: f32) -> egui::Response {
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

#[derive(Clone, Copy)]
enum Icon {
    Logo,
    Visuals,
    Audio,
    Palette,
    General,
    Keys,
    Collapse,
    Expand,
    Close,
    HideSidebar,
    Reset,
    Fullscreen,
    Overlay,
    Speaker,
    Muted,
}

/// Small line icons, drawn in a ~16 pt box centred on `c`.
fn paint_icon(p: &egui::Painter, icon: Icon, c: Pos2, col: Color32) {
    let s = Stroke::new(1.5_f32, col);
    let v = egui::vec2;
    let line = |a: Vec2, b: Vec2| p.line_segment([c + a, c + b], s);
    let chevron = |dx: f32, dir: f32| {
        line(v(dx + 2.5 * dir, -5.0), v(dx - 2.5 * dir, 0.0));
        line(v(dx - 2.5 * dir, 0.0), v(dx + 2.5 * dir, 5.0));
    };
    match icon {
        Icon::Logo => {
            p.circle_filled(c, 3.5, col);
            for i in 0..12 {
                let a = i as f32 / 12.0 * std::f32::consts::TAU;
                let d = v(a.cos(), a.sin());
                let len = if i % 2 == 0 { 4.0 } else { 2.5 };
                p.line_segment([c + d * 6.0, c + d * (6.0 + len)], Stroke::new(1.8_f32, col));
            }
        }
        Icon::Visuals => {
            for (y, k) in [(-5.0, -3.0), (0.0, 3.5), (5.0, -0.5)] {
                line(v(-7.0, y), v(7.0, y));
                p.circle_filled(c + v(k, y), 2.4, col);
            }
        }
        Icon::Audio => {
            for (i, h) in [4.0, 9.0, 14.0, 8.0, 5.0].iter().enumerate() {
                let x = -6.0 + i as f32 * 3.0;
                line(v(x, -h / 2.0), v(x, h / 2.0));
            }
        }
        Icon::Palette => {
            p.circle_stroke(c, 7.0, s);
            for (x, y) in [(-3.0, -2.5), (2.5, -3.0), (3.5, 2.0)] {
                p.circle_filled(c + v(x, y), 1.5, col);
            }
        }
        Icon::General => {
            // Gear: a ring with eight teeth around a hub.
            p.circle_stroke(c, 5.0, s);
            p.circle_stroke(c, 2.0, s);
            for i in 0..8 {
                let a = i as f32 / 8.0 * std::f32::consts::TAU;
                let d = v(a.cos(), a.sin());
                p.line_segment([c + d * 5.0, c + d * 7.5], Stroke::new(2.4_f32, col));
            }
        }
        Icon::Keys => {
            p.rect_stroke(Rect::from_center_size(c, v(16.0, 11.0)), CornerRadius::same(2), s, StrokeKind::Middle);
            for x in [-4.5, -1.5, 1.5, 4.5] {
                p.circle_filled(c + v(x, -1.8), 0.9, col);
            }
            line(v(-3.5, 2.2), v(3.5, 2.2));
        }
        Icon::Collapse => {
            chevron(-2.5, 1.0);
            chevron(2.5, 1.0);
        }
        Icon::Expand => {
            chevron(-2.5, -1.0);
            chevron(2.5, -1.0);
        }
        Icon::Close => {
            line(v(-4.5, -4.5), v(4.5, 4.5));
            line(v(-4.5, 4.5), v(4.5, -4.5));
        }
        Icon::HideSidebar => {
            // A panel with its left pane folding away.
            p.rect_stroke(Rect::from_center_size(c, v(16.0, 13.0)), CornerRadius::same(2), s, StrokeKind::Middle);
            line(v(-3.0, -6.5), v(-3.0, 6.5));
            line(v(3.5, -3.0), v(1.0, 0.0));
            line(v(1.0, 0.0), v(3.5, 3.0));
        }
        Icon::Reset => {
            // Circular arrow, most of a turn, head at the open end.
            let r = 4.5;
            let (a0, a1) = (-40f32.to_radians(), 230f32.to_radians());
            let at = |a: f32| v(a.cos(), a.sin());
            let pts: Vec<Pos2> = (0..=16).map(|i| c + at(a0 + (a1 - a0) * i as f32 / 16.0) * r).collect();
            p.add(egui::Shape::line(pts, s));
            let (end, out) = (c + at(a1) * r, at(a1));
            let tan = v(-out.y, out.x);
            let head = vec![end + tan * 2.5, end - tan * 1.0 + out * 2.5, end - tan * 1.0 - out * 2.5];
            p.add(egui::Shape::convex_polygon(head, col, Stroke::NONE));
        }
        Icon::Fullscreen => {
            for (sx, sy) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                line(v(6.0 * sx, 6.0 * sy), v(2.0 * sx, 6.0 * sy));
                line(v(6.0 * sx, 6.0 * sy), v(6.0 * sx, 2.0 * sy));
            }
        }
        Icon::Speaker | Icon::Muted => {
            let body = [v(-7.0, -2.5), v(-4.0, -2.5), v(0.0, -6.0), v(0.0, 6.0), v(-4.0, 2.5), v(-7.0, 2.5)];
            p.add(egui::Shape::convex_polygon(body.iter().map(|o| c + *o).collect(), col, Stroke::NONE));
            if matches!(icon, Icon::Muted) {
                line(v(3.0, -3.0), v(8.0, 3.0));
                line(v(3.0, 3.0), v(8.0, -3.0));
            } else {
                for r in [3.5, 7.0] {
                    let pts: Vec<Pos2> = (0..=8)
                        .map(|i| {
                            let a = (-45.0 + i as f32 * 11.25_f32).to_radians();
                            c + v(a.cos(), a.sin()) * r
                        })
                        .collect();
                    p.add(egui::Shape::line(pts, s));
                }
            }
        }
        Icon::Overlay => {
            p.rect_stroke(Rect::from_center_size(c + v(-2.0, -2.0), v(10.0, 8.0)), CornerRadius::same(1), s, StrokeKind::Middle);
            p.rect_stroke(Rect::from_center_size(c + v(2.5, 2.5), v(10.0, 8.0)), CornerRadius::same(1), s, StrokeKind::Middle);
        }
    }
}

/// Live band levels, drawn in the album accent: the one place the UI takes the track's colour.
fn spectrum_meter(ui: &mut egui::Ui, vis: &VisState, t: &Tokens, accent: Color32, height: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), height), Sense::hover());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, CornerRadius::same(5), t.control);
    let n = vis.bands.len();
    let w = rect.width() / n as f32;
    for (i, v) in vis.bands.iter().enumerate() {
        let h = (v.clamp(0.0, 1.0) * (rect.height() - 4.0)).max(1.0);
        let x = rect.min.x + i as f32 * w;
        let r = Rect::from_min_max(egui::pos2(x + 0.5, rect.max.y - 2.0 - h), egui::pos2(x + w - 0.5, rect.max.y - 2.0));
        p.rect_filled(r, CornerRadius::ZERO, accent.gamma_multiply(0.35 + 0.65 * v));
    }
    if height >= 40.0 {
        let k = vis.kick.clamp(0.0, 1.0);
        p.circle_filled(egui::pos2(rect.max.x - 10.0, rect.min.y + 10.0), 3.0 + 3.0 * k, t.text.gamma_multiply(0.2 + 0.8 * k));
    }
}
