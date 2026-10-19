//! egui UI: docked sidebar (expanded or icon rail) with a settings drawer, on-scene controls,
//! now-playing toast, and stats HUD.

use egui::{Align2, Color32, CornerRadius, FontId, Margin, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Vec2};

use crate::gpu::SCENE_NAMES;
use crate::media::{TrackInfo, ART_SIZE};
use crate::palette::{Palette, PRESETS};
use crate::settings::{Settings, WindowMode};
use crate::sys::ProcStats;
use crate::vis::VisState;

const SIDEBAR_W: f32 = 208.0;
const RAIL_W: f32 = 56.0;
const DRAWER_W: f32 = 300.0;
const SCENE_KEYS: [&str; 11] = ["1", "2", "3", "4", "5", "6", "7", "8", "9", "0", "-"];

pub enum UiAction {
    SetMode(WindowMode),
    Scene(usize),
    HideSidebar,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Scenes,
    Visuals,
    Audio,
    Palette,
    Window,
    Hotkeys,
}

const SECTIONS: [(Section, &str, Icon); 6] = [
    (Section::Scenes, "Scenes", Icon::Scenes),
    (Section::Visuals, "Visuals", Icon::Visuals),
    (Section::Audio, "Audio", Icon::Audio),
    (Section::Palette, "Palette", Icon::Palette),
    (Section::Window, "Window", Icon::Window),
    (Section::Hotkeys, "Hotkeys", Icon::Keys),
];

pub struct UiState {
    art: Option<egui::TextureHandle>,
    styled: bool,
    /// Settings section open in the drawer beside the sidebar.
    pub drawer: Option<Section>,
}

pub struct UiCtx<'a> {
    pub settings: &'a mut Settings,
    pub track: &'a TrackInfo,
    pub vis: &'a VisState,
    pub palette: Palette,
    pub scene: usize,
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
    /// Opacity of the on-scene scene switcher and window buttons (fades after the mouse rests).
    pub controls_alpha: f32,
}

fn to_c32(c: [f32; 3], a: u8) -> Color32 {
    let f = |v: f32| (crate::palette::linear_to_srgb(v.clamp(0.0, 1.0)) * 255.0) as u8;
    Color32::from_rgba_unmultiplied(f(c[0]), f(c[1]), f(c[2]), a)
}

impl UiState {
    pub fn new() -> Self {
        Self { art: None, styled: false, drawer: None }
    }

    pub fn set_art(&mut self, ctx: &egui::Context, rgba: Option<&[u8]>) {
        self.art = rgba.map(|px| {
            let img = egui::ColorImage::from_rgba_unmultiplied([ART_SIZE as usize, ART_SIZE as usize], px);
            ctx.load_texture("album-art", img, egui::TextureOptions::LINEAR)
        });
    }

    fn style(&mut self, ctx: &egui::Context) {
        if self.styled {
            return;
        }
        self.styled = true;
        ctx.set_theme(egui::Theme::Dark);
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
            let v = &mut s.visuals;
            v.window_corner_radius = CornerRadius::same(14);
            v.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, Color32::from_white_alpha(14));
            v.widgets.inactive.bg_fill = Color32::from_white_alpha(14);
            v.widgets.inactive.weak_bg_fill = Color32::from_white_alpha(10);
            v.widgets.inactive.corner_radius = CornerRadius::same(7);
            v.widgets.hovered.corner_radius = CornerRadius::same(7);
            v.widgets.active.corner_radius = CornerRadius::same(7);
            v.extreme_bg_color = Color32::from_black_alpha(90);
        });
    }

    /// Draws the UI and returns the actions taken plus the rect (in points) left for the scene.
    pub fn draw(&mut self, ctx: &egui::Context, mut c: UiCtx) -> (Vec<UiAction>, Rect) {
        self.style(ctx);
        let mut actions = Vec::new();
        let accent = to_c32(c.palette[0], 255);
        ctx.all_styles_mut(|s| {
            s.visuals.selection.bg_fill = accent.gamma_multiply(0.45);
            s.visuals.selection.stroke = Stroke::new(1.0_f32, Color32::WHITE);
            s.visuals.hyperlink_color = accent;
            s.visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, accent.gamma_multiply(0.6));
        });

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
                        .fill(Color32::from_rgba_unmultiplied(13, 13, 21, alpha))
                        .stroke(Stroke::new(1.0_f32, Color32::from_rgb(29, 29, 40)))
                        .inner_margin(Margin::symmetric(8, 10)),
                )
                .show(ctx, |ui| self.sidebar(ui, &mut c, accent, &mut actions));

            if let Some(sec) = self.drawer {
                egui::SidePanel::left("drawer")
                    .resizable(false)
                    .exact_width(DRAWER_W)
                    .frame(
                        egui::Frame::new()
                            .fill(Color32::from_rgba_unmultiplied(11, 11, 18, alpha))
                            .stroke(Stroke::new(1.0_f32, Color32::from_rgb(29, 29, 40)))
                            .inner_margin(Margin::same(16)),
                    )
                    .show(ctx, |ui| self.drawer(ui, sec, &mut c, accent, &mut actions));
            }
            scene_rect = ctx.available_rect();
        }

        if c.settings.show_stats {
            egui::Area::new(egui::Id::new("stats"))
                .fixed_pos(scene_rect.min + egui::vec2(14.0, 12.0))
                .interactable(false)
                .show(ctx, |ui| {
                    egui::Frame::new()
                        .fill(Color32::from_black_alpha(140))
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
                            ui.label(RichText::new(txt).monospace().color(Color32::from_gray(220)));
                        });
                });
        }

        if c.toast_alpha > 0.0 && c.settings.show_track_toast && !c.track.title.is_empty() {
            egui::Area::new(egui::Id::new("toast"))
                .pivot(Align2::LEFT_BOTTOM)
                // Rises above the scene switcher while that is showing.
                .fixed_pos(scene_rect.left_bottom() + egui::vec2(26.0, -26.0 - 48.0 * c.controls_alpha))
                .interactable(false)
                .show(ctx, |ui| {
                    ui.multiply_opacity(c.toast_alpha);
                    egui::Frame::new()
                        .fill(Color32::from_rgba_unmultiplied(8, 8, 14, 200))
                        .corner_radius(CornerRadius::same(14))
                        .stroke(Stroke::new(1.0_f32, accent.gamma_multiply(0.45)))
                        .inner_margin(Margin::same(10))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                if let Some(t) = &self.art {
                                    ui.add(egui::Image::new((t.id(), Vec2::splat(64.0))).corner_radius(CornerRadius::same(9)));
                                }
                                ui.vertical(|ui| {
                                    ui.add_space(6.0);
                                    ui.label(RichText::new(&c.track.title).size(20.0).strong().color(Color32::WHITE));
                                    ui.label(RichText::new(&c.track.artist).size(14.0).color(Color32::from_gray(190)));
                                    if !c.track.album.is_empty() {
                                        ui.label(RichText::new(&c.track.album).size(11.5).color(accent.gamma_multiply(0.9)));
                                    }
                                });
                                ui.add_space(8.0);
                            });
                        });
                });
        }

        if c.controls_alpha > 0.0 {
            scene_controls(ctx, scene_rect, &c, &mut actions);
        }

        (actions, scene_rect)
    }

    fn sidebar(&mut self, ui: &mut egui::Ui, c: &mut UiCtx, accent: Color32, actions: &mut Vec<UiAction>) {
        let expanded = c.settings.sidebar_expanded;

        // Logo.
        let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 34.0), Sense::hover());
        let p = ui.painter();
        let ic = egui::pos2(rect.left() + 20.0, rect.center().y);
        paint_icon(p, Icon::Logo, ic, accent);
        if expanded {
            let font = FontId::proportional(16.0);
            let r = p.text(ic + egui::vec2(18.0, 0.0), Align2::LEFT_CENTER, "BLAZE", font.clone(), accent);
            p.text(egui::pos2(r.right(), ic.y), Align2::LEFT_CENTER, "AUDIO", font, Color32::WHITE);
        }
        ui.add_space(10.0);

        for (sec, name, icon) in SECTIONS {
            let open = self.drawer == Some(sec);
            if nav_item(ui, icon, name, open, expanded, accent).clicked() {
                self.drawer = if open { None } else { Some(sec) };
            }
        }

        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            let (icon, label) = if expanded { (Icon::Collapse, "Collapse") } else { (Icon::Expand, "Expand") };
            if nav_item(ui, icon, label, false, expanded, accent).clicked() {
                c.settings.sidebar_expanded = !expanded;
            }
            if c.mode != WindowMode::Windowed && nav_item(ui, Icon::Close, "Hide sidebar", false, expanded, accent).on_hover_text("F1").clicked() {
                actions.push(UiAction::HideSidebar);
            }
            ui.add_space(4.0);
            self.now_playing(ui, c, accent, expanded);
        });
    }

    fn now_playing(&self, ui: &mut egui::Ui, c: &UiCtx, accent: Color32, expanded: bool) {
        let tip = if c.track.title.is_empty() { "Nothing playing".to_owned() } else { format!("{}\n{}", c.track.title, c.track.artist) };
        if !expanded {
            let (rect, resp) = ui.allocate_exact_size(Vec2::splat(40.0), Sense::hover());
            match &self.art {
                Some(t) => {
                    egui::Image::new((t.id(), rect.size())).corner_radius(CornerRadius::same(7)).paint_at(ui, rect);
                }
                None => {
                    ui.painter().rect_filled(rect, CornerRadius::same(7), Color32::from_white_alpha(10));
                    paint_icon(ui.painter(), Icon::Audio, rect.center(), Color32::from_gray(120));
                }
            }
            resp.on_hover_text(tip);
            return;
        }
        egui::Frame::new()
            .fill(Color32::from_white_alpha(8))
            .corner_radius(CornerRadius::same(8))
            .inner_margin(Margin::same(8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                self.now_playing_card(ui, c, accent, tip);
            });
    }

    /// Card body; the sidebar footer lays out bottom-up, so the spectrum goes in first.
    fn now_playing_card(&self, ui: &mut egui::Ui, c: &UiCtx, accent: Color32, tip: String) {
        spectrum_meter(ui, c.vis, accent, 22.0);
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            match &self.art {
                Some(t) => {
                    ui.add(egui::Image::new((t.id(), Vec2::splat(38.0))).corner_radius(CornerRadius::same(6)));
                }
                None => {
                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(38.0), Sense::hover());
                    ui.painter().rect_filled(rect, CornerRadius::same(6), Color32::from_white_alpha(10));
                    paint_icon(ui.painter(), Icon::Audio, rect.center(), Color32::from_gray(120));
                }
            }
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                if c.track.title.is_empty() {
                    ui.add(egui::Label::new(RichText::new("Nothing playing").color(Color32::from_gray(160))).truncate());
                } else {
                    ui.add(egui::Label::new(RichText::new(&c.track.title).strong().color(Color32::WHITE)).truncate());
                    ui.add(egui::Label::new(RichText::new(&c.track.artist).small().color(Color32::from_gray(150))).truncate());
                }
            });
        })
        .response
        .on_hover_text(tip);
    }

    fn drawer(&mut self, ui: &mut egui::Ui, sec: Section, c: &mut UiCtx, accent: Color32, actions: &mut Vec<UiAction>) {
        let title = SECTIONS.iter().find(|s| s.0 == sec).map_or("", |s| s.1);
        ui.horizontal(|ui| {
            ui.label(RichText::new(title).heading().strong().color(Color32::WHITE));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if icon_button(ui, Icon::Close, 26.0).on_hover_text("Close").clicked() {
                    self.drawer = None;
                }
            });
        });
        ui.add_space(6.0);

        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            // Sliders span the drawer, leaving room for the value box.
            ui.spacing_mut().slider_width = ui.available_width() - 64.0;
            let s = &mut *c.settings;
            match sec {
                Section::Scenes => {
                    let w = (ui.available_width() - 6.0) / 2.0;
                    egui::Grid::new("scenes").num_columns(2).spacing([6.0, 6.0]).show(ui, |ui| {
                        for (i, name) in SCENE_NAMES.iter().enumerate() {
                            let label = RichText::new(format!("{}  {name}", SCENE_KEYS[i]));
                            if ui.add_sized([w, 30.0], egui::Button::new(label).selected(c.scene == i)).clicked() {
                                actions.push(UiAction::Scene(i));
                            }
                            if i % 2 == 1 {
                                ui.end_row();
                            }
                        }
                    });
                    ui.add_space(8.0);
                    ui.checkbox(&mut s.auto_cycle, "Auto-cycle scenes (on the beat)");
                    ui.add_enabled_ui(s.auto_cycle, |ui| labeled(ui, "Cycle time", egui::Slider::new(&mut s.cycle_seconds, 10.0..=180.0).suffix(" s")));
                    ui.checkbox(&mut s.switch_on_drop, "Switch scene on drops");
                    labeled(ui, "Crossfade", egui::Slider::new(&mut s.transition_seconds, 0.3..=6.0).suffix(" s"));
                }
                Section::Visuals => {
                    labeled(ui, "Intensity", egui::Slider::new(&mut s.intensity, 0.2..=2.0));
                    labeled(ui, "Bloom", egui::Slider::new(&mut s.bloom_strength, 0.0..=3.0));
                    labeled(ui, "Bloom threshold", egui::Slider::new(&mut s.bloom_threshold, 0.0..=3.0));
                    labeled(ui, "Exposure", egui::Slider::new(&mut s.exposure, 0.3..=3.0));
                    labeled(ui, "Chromatic", egui::Slider::new(&mut s.chromatic, 0.0..=2.0));
                    labeled(ui, "Film grain", egui::Slider::new(&mut s.grain, 0.0..=1.5));
                    labeled(ui, "Vignette", egui::Slider::new(&mut s.vignette, 0.0..=1.0));
                    labeled(ui, "Particle trails", egui::Slider::new(&mut s.trails, 0.0..=0.95));
                    labeled(ui, "Particle density", egui::Slider::new(&mut s.particle_fraction, 0.1..=1.0));
                    labeled(ui, "Render scale", egui::Slider::new(&mut s.render_scale, 0.5..=1.0));
                }
                Section::Audio => {
                    spectrum_meter(ui, c.vis, accent, 56.0);
                    ui.label(RichText::new(c.device).small().color(Color32::from_gray(130)));
                    ui.add_space(4.0);
                    labeled(ui, "Sensitivity", egui::Slider::new(&mut s.sensitivity, 0.3..=2.5));
                    labeled(ui, "Attack", egui::Slider::new(&mut s.attack_ms, 1.0..=80.0).suffix(" ms"));
                    labeled(ui, "Decay", egui::Slider::new(&mut s.decay_ms, 30.0..=600.0).suffix(" ms"));
                    labeled(ui, "Beat sensitivity", egui::Slider::new(&mut s.beat_sensitivity, 0.3..=2.5));
                }
                Section::Palette => {
                    ui.checkbox(&mut s.album_palette, "Take palette from album art");
                    ui.add_enabled_ui(!s.album_palette || c.track.palette.is_none(), |ui| {
                        egui::ComboBox::from_label("Fallback")
                            .selected_text(PRESETS[s.preset_palette.min(PRESETS.len() - 1)].0)
                            .show_ui(ui, |ui| {
                                for (i, (n, _)) in PRESETS.iter().enumerate() {
                                    ui.selectable_value(&mut s.preset_palette, i, *n);
                                }
                            });
                    });
                    ui.add_space(4.0);
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 28.0), Sense::hover());
                    let w = rect.width() / 4.0;
                    for (i, col) in c.palette.iter().enumerate() {
                        let r = Rect::from_min_size(rect.min + egui::vec2(w * i as f32, 0.0), egui::vec2(w - 4.0, 28.0));
                        ui.painter().rect_filled(r, CornerRadius::same(5), to_c32(*col, 255));
                    }
                }
                Section::Window => {
                    let w = (ui.available_width() - 12.0) / 3.0;
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        for (m, n) in [(WindowMode::Windowed, "Windowed"), (WindowMode::Fullscreen, "Fullscreen"), (WindowMode::Overlay, "Overlay")] {
                            if ui.add_sized([w, 30.0], egui::Button::new(n).selected(c.mode == m)).clicked() {
                                actions.push(UiAction::SetMode(m));
                            }
                        }
                    });
                    if !c.transparent_ok {
                        ui.label(RichText::new("Transparent overlay unsupported on this GPU; overlay will be opaque.").small().color(Color32::from_rgb(255, 170, 90)));
                    }
                    labeled(ui, "Overlay opacity", egui::Slider::new(&mut s.overlay_opacity, 0.2..=1.0));
                    ui.checkbox(&mut s.overlay_click_through, "Overlay is click-through");
                    egui::ComboBox::from_label("Start in")
                        .selected_text(format!("{:?}", s.start_mode))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut s.start_mode, WindowMode::Windowed, "Windowed");
                            ui.selectable_value(&mut s.start_mode, WindowMode::Fullscreen, "Fullscreen");
                            ui.selectable_value(&mut s.start_mode, WindowMode::Overlay, "Overlay");
                        });
                    labeled(ui, "Idle frame cap", egui::Slider::new(&mut s.idle_fps, 5..=60).suffix(" fps"));
                    ui.checkbox(&mut s.show_track_toast, "Now-playing toast on track change");
                    ui.checkbox(&mut s.show_stats, "Show performance stats (F3)");
                    ui.separator();
                    ui.label(RichText::new(format!("GPU: {}  ·  DX12", c.adapter)).small().color(Color32::from_gray(110)));
                    if ui.small_button("Reset to defaults").clicked() {
                        let (mode, expanded) = (s.start_mode, s.sidebar_expanded);
                        *s = Settings::default();
                        s.start_mode = mode;
                        s.sidebar_expanded = expanded;
                    }
                }
                Section::Hotkeys => {
                    let rows = [
                        ("F1 / Tab", "Sidebar"),
                        ("F11 / Alt+Enter / dbl-click", "Fullscreen"),
                        ("O", "Overlay mode"),
                        ("← → / 1-9, 0, -", "Scenes"),
                        ("T", "Show now playing"),
                        ("F3", "Stats"),
                        ("Esc", "Back / close"),
                        ("Ctrl+Alt+O", "Overlay (global)"),
                        ("Ctrl+Alt+S", "Sidebar (global)"),
                        ("Ctrl+Alt+N", "Next scene (global)"),
                    ];
                    egui::Grid::new("keys").num_columns(2).spacing([14.0, 6.0]).show(ui, |ui| {
                        for (k, d) in rows {
                            ui.label(RichText::new(k).monospace().color(accent));
                            ui.label(d);
                            ui.end_row();
                        }
                    });
                }
            }
        });
    }
}

/// Scene switcher pill (bottom centre) and fullscreen / overlay buttons (top right) over the scene.
fn scene_controls(ctx: &egui::Context, scene: Rect, c: &UiCtx, actions: &mut Vec<UiAction>) {
    let frame = egui::Frame::new()
        .fill(Color32::from_rgba_unmultiplied(13, 13, 21, 200))
        .stroke(Stroke::new(1.0_f32, Color32::from_white_alpha(20)))
        .corner_radius(CornerRadius::same(18))
        .inner_margin(Margin::symmetric(6, 4));

    egui::Area::new(egui::Id::new("scene-switch"))
        .pivot(Align2::CENTER_BOTTOM)
        .fixed_pos(scene.center_bottom() - egui::vec2(0.0, 16.0))
        .show(ctx, |ui| {
            ui.multiply_opacity(c.controls_alpha);
            frame.show(ui, |ui| {
                ui.horizontal(|ui| {
                    if icon_button(ui, Icon::Prev, 26.0).on_hover_text("Previous scene (←)").clicked() {
                        actions.push(UiAction::Scene(c.scene + SCENE_NAMES.len() - 1));
                    }
                    ui.add_sized([96.0, 26.0], egui::Label::new(RichText::new(SCENE_NAMES[c.scene]).color(Color32::WHITE)));
                    if icon_button(ui, Icon::Next, 26.0).on_hover_text("Next scene (→)").clicked() {
                        actions.push(UiAction::Scene(c.scene + 1));
                    }
                });
            });
        });

    egui::Area::new(egui::Id::new("window-buttons"))
        .pivot(Align2::RIGHT_TOP)
        .fixed_pos(scene.right_top() + egui::vec2(-12.0, 12.0))
        .show(ctx, |ui| {
            ui.multiply_opacity(c.controls_alpha);
            frame.show(ui, |ui| {
                ui.horizontal(|ui| {
                    let ov = c.mode == WindowMode::Overlay;
                    let tip = if ov { "Leave overlay (O)" } else { "Overlay mode (O)" };
                    if icon_button(ui, Icon::Overlay, 26.0).on_hover_text(tip).clicked() {
                        actions.push(UiAction::SetMode(if ov { WindowMode::Windowed } else { WindowMode::Overlay }));
                    }
                    let fs = c.mode == WindowMode::Fullscreen;
                    let tip = if fs { "Exit fullscreen (F11)" } else { "Fullscreen (F11)" };
                    if icon_button(ui, Icon::Fullscreen, 26.0).on_hover_text(tip).clicked() {
                        actions.push(UiAction::SetMode(if fs { WindowMode::Windowed } else { WindowMode::Fullscreen }));
                    }
                });
            });
        });
}

/// One sidebar row: icon, plus a label when expanded (a tooltip stands in for it on the rail).
fn nav_item(ui: &mut egui::Ui, icon: Icon, label: &str, selected: bool, expanded: bool, accent: Color32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 36.0), Sense::click());
    let p = ui.painter();
    let hovered = resp.hovered();
    if selected {
        p.rect_filled(rect, CornerRadius::same(7), accent.gamma_multiply(0.16));
    } else if hovered {
        p.rect_filled(rect, CornerRadius::same(7), Color32::from_white_alpha(10));
    }
    let col = if selected || hovered { Color32::WHITE } else { Color32::from_gray(168) };
    let ic = egui::pos2(rect.left() + 20.0, rect.center().y);
    paint_icon(p, icon, ic, if selected { accent } else { col });
    if expanded {
        p.text(ic + egui::vec2(18.0, 0.0), Align2::LEFT_CENTER, label, FontId::proportional(13.5), col);
        resp
    } else {
        resp.on_hover_text(label)
    }
}

/// A slider with its label on the line above, so long labels fit the drawer.
fn labeled(ui: &mut egui::Ui, label: &str, slider: egui::Slider) {
    ui.add_space(2.0);
    ui.label(RichText::new(label).color(Color32::from_gray(185)));
    ui.add(slider);
}

fn icon_button(ui: &mut egui::Ui, icon: Icon, size: f32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    let hovered = resp.hovered();
    if hovered {
        ui.painter().rect_filled(rect, CornerRadius::same(6), Color32::from_white_alpha(14));
    }
    let col = if hovered { Color32::WHITE } else { Color32::from_gray(170) };
    paint_icon(ui.painter(), icon, rect.center(), col);
    resp
}

#[derive(Clone, Copy)]
enum Icon {
    Logo,
    Scenes,
    Visuals,
    Audio,
    Palette,
    Window,
    Keys,
    Collapse,
    Expand,
    Close,
    Prev,
    Next,
    Fullscreen,
    Overlay,
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
        Icon::Scenes => {
            for (x, y) in [(-4.0, -4.0), (4.0, -4.0), (-4.0, 4.0), (4.0, 4.0)] {
                p.rect_stroke(Rect::from_center_size(c + v(x, y), Vec2::splat(6.0)), CornerRadius::same(2), s, StrokeKind::Middle);
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
        Icon::Window => {
            p.rect_stroke(Rect::from_center_size(c, v(16.0, 13.0)), CornerRadius::same(2), s, StrokeKind::Middle);
            line(v(-8.0, -2.5), v(8.0, -2.5));
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
        Icon::Prev => chevron(0.0, 1.0),
        Icon::Next => chevron(0.0, -1.0),
        Icon::Fullscreen => {
            for (sx, sy) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                line(v(6.0 * sx, 6.0 * sy), v(2.0 * sx, 6.0 * sy));
                line(v(6.0 * sx, 6.0 * sy), v(6.0 * sx, 2.0 * sy));
            }
        }
        Icon::Overlay => {
            p.rect_stroke(Rect::from_center_size(c + v(-2.0, -2.0), v(10.0, 8.0)), CornerRadius::same(1), s, StrokeKind::Middle);
            p.rect_stroke(Rect::from_center_size(c + v(2.5, 2.5), v(10.0, 8.0)), CornerRadius::same(1), s, StrokeKind::Middle);
        }
    }
}

fn spectrum_meter(ui: &mut egui::Ui, vis: &VisState, accent: Color32, height: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), height), Sense::hover());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, CornerRadius::same(5), Color32::from_black_alpha(80));
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
        p.circle_filled(egui::pos2(rect.max.x - 10.0, rect.min.y + 10.0), 3.0 + 3.0 * k, Color32::WHITE.gamma_multiply(0.2 + 0.8 * k));
    }
}
