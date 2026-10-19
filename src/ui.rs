//! egui overlay: settings panel (F1 / Ctrl+Alt+S), now-playing toast, stats HUD.

use egui::{Align2, Color32, CornerRadius, Margin, RichText, Stroke, Vec2};

use crate::gpu::SCENE_NAMES;
use crate::media::{TrackInfo, ART_SIZE};
use crate::palette::{Palette, PRESETS};
use crate::settings::{Settings, WindowMode};
use crate::sys::ProcStats;
use crate::vis::VisState;

pub enum UiAction {
    SetMode(WindowMode),
    Scene(usize),
    CloseSettings,
}

pub struct UiState {
    art: Option<egui::TextureHandle>,
    styled: bool,
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
    pub show_settings: bool,
    pub toast_alpha: f32,
}

fn to_c32(c: [f32; 3], a: u8) -> Color32 {
    let f = |v: f32| (crate::palette::linear_to_srgb(v.clamp(0.0, 1.0)) * 255.0) as u8;
    Color32::from_rgba_unmultiplied(f(c[0]), f(c[1]), f(c[2]), a)
}

impl UiState {
    pub fn new() -> Self {
        Self { art: None, styled: false }
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
            use egui::{FontFamily, FontId, TextStyle};
            s.text_styles = [
                (TextStyle::Heading, FontId::new(19.0, FontFamily::Proportional)),
                (TextStyle::Body, FontId::new(13.5, FontFamily::Proportional)),
                (TextStyle::Button, FontId::new(13.5, FontFamily::Proportional)),
                (TextStyle::Small, FontId::new(11.0, FontFamily::Proportional)),
                (TextStyle::Monospace, FontId::new(12.0, FontFamily::Monospace)),
            ]
            .into();
            s.spacing.item_spacing = egui::vec2(8.0, 7.0);
            s.spacing.slider_width = 170.0;
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

    pub fn draw(&mut self, ctx: &egui::Context, c: UiCtx) -> Vec<UiAction> {
        self.style(ctx);
        let mut actions = Vec::new();
        let accent = to_c32(c.palette[0], 255);
        ctx.all_styles_mut(|s| {
            s.visuals.selection.bg_fill = accent.gamma_multiply(0.75);
            s.visuals.selection.stroke = Stroke::new(1.0_f32, accent);
            s.visuals.hyperlink_color = accent;
            s.visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, accent.gamma_multiply(0.6));
        });

        if c.settings.show_stats {
            egui::Area::new(egui::Id::new("stats"))
                .anchor(Align2::LEFT_TOP, egui::vec2(14.0, 12.0))
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
                .anchor(Align2::LEFT_BOTTOM, egui::vec2(26.0, -26.0))
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

        if !c.show_settings {
            return actions;
        }

        let screen_h = ctx.content_rect().height();
        let frame = egui::Frame::new()
            .fill(Color32::from_rgba_unmultiplied(9, 9, 15, 232))
            .corner_radius(CornerRadius::same(16))
            .stroke(Stroke::new(1.0_f32, Color32::from_white_alpha(20)))
            .inner_margin(Margin::same(16))
            .shadow(egui::epaint::Shadow { offset: [0, 10], blur: 28, spread: 0, color: Color32::from_black_alpha(140) });

        egui::Window::new("BlazeAudio")
            .title_bar(false)
            .resizable(false)
            .anchor(Align2::RIGHT_TOP, egui::vec2(-16.0, 16.0))
            .frame(frame)
            .default_width(336.0)
            .min_width(336.0)
            .max_width(336.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("BLAZE").size(22.0).strong().color(accent));
                    ui.label(RichText::new("AUDIO").size(22.0).strong().color(Color32::WHITE));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button(RichText::new("×").size(18.0)).on_hover_text("Close (F1)").clicked() {
                            actions.push(UiAction::CloseSettings);
                        }
                    });
                });
                ui.add_space(2.0);

                // Now playing.
                ui.horizontal(|ui| {
                    if let Some(t) = &self.art {
                        ui.add(egui::Image::new((t.id(), Vec2::splat(48.0))).corner_radius(CornerRadius::same(7)));
                    }
                    ui.vertical(|ui| {
                        if c.track.title.is_empty() {
                            ui.label(RichText::new("Nothing playing").color(Color32::from_gray(160)));
                        } else {
                            ui.label(RichText::new(&c.track.title).strong().color(Color32::WHITE));
                            ui.label(RichText::new(&c.track.artist).color(Color32::from_gray(170)));
                        }
                        ui.label(RichText::new(format!("🔊 {}", c.device)).small().color(Color32::from_gray(120)));
                    });
                });
                spectrum_meter(ui, c.vis, accent);
                ui.separator();

                egui::ScrollArea::vertical().max_height((screen_h - 250.0).max(200.0)).show(ui, |ui| {
                    let s = &mut *c.settings;

                    section(ui, "Scenes", true, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            for (i, name) in SCENE_NAMES.iter().enumerate() {
                                let label = format!("{} {}", i + 1, name);
                                if ui.selectable_label(c.scene == i, label).clicked() {
                                    actions.push(UiAction::Scene(i));
                                }
                            }
                        });
                        ui.checkbox(&mut s.auto_cycle, "Auto-cycle scenes (on the beat)");
                        ui.add_enabled(s.auto_cycle, egui::Slider::new(&mut s.cycle_seconds, 10.0..=180.0).suffix(" s").text("Cycle"));
                        ui.checkbox(&mut s.switch_on_drop, "Switch scene on drops");
                        ui.add(egui::Slider::new(&mut s.transition_seconds, 0.3..=6.0).suffix(" s").text("Crossfade"));
                    });

                    section(ui, "Visuals", true, |ui| {
                        ui.add(egui::Slider::new(&mut s.intensity, 0.2..=2.0).text("Intensity"));
                        ui.add(egui::Slider::new(&mut s.bloom_strength, 0.0..=3.0).text("Bloom"));
                        ui.add(egui::Slider::new(&mut s.bloom_threshold, 0.0..=3.0).text("Bloom threshold"));
                        ui.add(egui::Slider::new(&mut s.exposure, 0.3..=3.0).text("Exposure"));
                        ui.add(egui::Slider::new(&mut s.chromatic, 0.0..=2.0).text("Chromatic"));
                        ui.add(egui::Slider::new(&mut s.grain, 0.0..=1.5).text("Film grain"));
                        ui.add(egui::Slider::new(&mut s.vignette, 0.0..=1.0).text("Vignette"));
                        ui.add(egui::Slider::new(&mut s.trails, 0.0..=0.95).text("Particle trails"));
                        ui.add(egui::Slider::new(&mut s.particle_fraction, 0.1..=1.0).text("Particle density"));
                        ui.add(egui::Slider::new(&mut s.render_scale, 0.5..=1.0).text("Render scale"));
                    });

                    section(ui, "Audio", false, |ui| {
                        ui.add(egui::Slider::new(&mut s.sensitivity, 0.3..=2.5).text("Sensitivity"));
                        ui.add(egui::Slider::new(&mut s.attack_ms, 1.0..=80.0).suffix(" ms").text("Attack"));
                        ui.add(egui::Slider::new(&mut s.decay_ms, 30.0..=600.0).suffix(" ms").text("Decay"));
                        ui.add(egui::Slider::new(&mut s.beat_sensitivity, 0.3..=2.5).text("Beat sensitivity"));
                    });

                    section(ui, "Palette", false, |ui| {
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
                        let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 22.0), egui::Sense::hover());
                        let w = rect.width() / 4.0;
                        for (i, col) in c.palette.iter().enumerate() {
                            let r = egui::Rect::from_min_size(rect.min + egui::vec2(w * i as f32, 0.0), egui::vec2(w - 4.0, 22.0));
                            ui.painter().rect_filled(r, CornerRadius::same(5), to_c32(*col, 255));
                        }
                    });

                    section(ui, "Window", false, |ui| {
                        ui.horizontal(|ui| {
                            for (m, n) in [(WindowMode::Windowed, "Windowed"), (WindowMode::Fullscreen, "Fullscreen"), (WindowMode::Overlay, "Overlay")] {
                                if ui.selectable_label(c.mode == m, n).clicked() {
                                    actions.push(UiAction::SetMode(m));
                                }
                            }
                        });
                        if !c.transparent_ok {
                            ui.label(RichText::new("Transparent overlay unsupported on this GPU; overlay will be opaque.").small().color(Color32::from_rgb(255, 170, 90)));
                        }
                        ui.add(egui::Slider::new(&mut s.overlay_opacity, 0.2..=1.0).text("Overlay opacity"));
                        ui.checkbox(&mut s.overlay_click_through, "Overlay is click-through");
                        egui::ComboBox::from_label("Start in")
                            .selected_text(format!("{:?}", s.start_mode))
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut s.start_mode, WindowMode::Windowed, "Windowed");
                                ui.selectable_value(&mut s.start_mode, WindowMode::Fullscreen, "Fullscreen");
                                ui.selectable_value(&mut s.start_mode, WindowMode::Overlay, "Overlay");
                            });
                        ui.add(egui::Slider::new(&mut s.idle_fps, 5..=60).suffix(" fps").text("Idle frame cap"));
                        ui.checkbox(&mut s.show_track_toast, "Now-playing toast on track change");
                        ui.checkbox(&mut s.show_stats, "Show performance stats (F3)");
                    });

                    section(ui, "Hotkeys", false, |ui| {
                        let rows = [
                            ("F1 / Tab", "Settings"),
                            ("F11 / Alt+Enter / dbl-click", "Fullscreen"),
                            ("O", "Overlay mode"),
                            ("← → / 1-4", "Scenes"),
                            ("T", "Show now playing"),
                            ("F3", "Stats"),
                            ("Esc", "Back / close"),
                            ("Ctrl+Alt+O", "Overlay (global)"),
                            ("Ctrl+Alt+S", "Settings (global)"),
                            ("Ctrl+Alt+N", "Next scene (global)"),
                        ];
                        egui::Grid::new("keys").num_columns(2).spacing([14.0, 4.0]).show(ui, |ui| {
                            for (k, d) in rows {
                                ui.label(RichText::new(k).monospace().color(accent));
                                ui.label(d);
                                ui.end_row();
                            }
                        });
                    });

                    ui.add_space(4.0);
                    ui.label(RichText::new(format!("GPU: {}  ·  DX12", c.adapter)).small().color(Color32::from_gray(110)));
                    if ui.small_button("Reset to defaults").clicked() {
                        let mode = s.start_mode;
                        *s = Settings::default();
                        s.start_mode = mode;
                    }
                });
            });
        actions
    }
}

fn section(ui: &mut egui::Ui, title: &str, open: bool, body: impl FnOnce(&mut egui::Ui)) {
    egui::CollapsingHeader::new(RichText::new(title).strong().size(14.5))
        .default_open(open)
        .show(ui, body);
}

fn spectrum_meter(ui: &mut egui::Ui, vis: &VisState, accent: Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 40.0), egui::Sense::hover());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, CornerRadius::same(6), Color32::from_black_alpha(80));
    let n = vis.bands.len();
    let w = rect.width() / n as f32;
    for (i, v) in vis.bands.iter().enumerate() {
        let h = (v.clamp(0.0, 1.0) * (rect.height() - 4.0)).max(1.0);
        let x = rect.min.x + i as f32 * w;
        let r = egui::Rect::from_min_max(egui::pos2(x + 0.5, rect.max.y - 2.0 - h), egui::pos2(x + w - 0.5, rect.max.y - 2.0));
        p.rect_filled(r, CornerRadius::ZERO, accent.gamma_multiply(0.35 + 0.65 * v));
    }
    let k = vis.kick.clamp(0.0, 1.0);
    p.circle_filled(egui::pos2(rect.max.x - 10.0, rect.min.y + 10.0), 3.0 + 3.0 * k, Color32::WHITE.gamma_multiply(0.2 + 0.8 * k));
}
