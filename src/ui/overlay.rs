//! Things floating over the scene: stats HUD, now-playing toast, "nothing playing" hint, window
//! buttons, volume bar and timeline.

use egui::{Align2, Color32, CornerRadius, Margin, Rect, RichText, Sense, Stroke, Vec2};

use super::icons::{paint_icon, Icon};
use super::playback::seek_bar;
use super::theme::{Tokens, SPOTIFY_GREEN};
use super::widgets::{icon_button, labeled_icon_button};
use super::{UiAction, UiCtx, UiState};
use crate::settings::{DockSide, WindowMode};
use crate::wake::WakeStyle;

/// FPS, frame time, CPU / RAM and tempo in the scene's top-left corner.
pub(super) fn stats_hud(ctx: &egui::Context, scene: Rect, c: &UiCtx, t: &Tokens) {
    egui::Area::new(egui::Id::new("stats"))
        .fixed_pos(scene.min + egui::vec2(14.0, 12.0))
        .interactable(false)
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(t.float)
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
                    // Never wrap: the Area's width comes from its first frame, which a
                    // longer line later (e.g. a bigger RAM figure) would break.
                    ui.add(egui::Label::new(RichText::new(txt).monospace().color(t.text)).extend());
                });
        });
}

/// Where the now-playing card would be, when there's nothing to show in it: tells a new user
/// to play something, or that there's no output device to listen to.
pub(super) fn empty_hint(ctx: &egui::Context, scene: Rect, c: &UiCtx, t: &Tokens, lift: f32) {
    let (title, body) = if c.capturing {
        ("Nothing playing", "Play music in any app and the visuals follow it.")
    } else {
        ("No output device", "Connect speakers or headphones. BlazeAudio listens to the Windows default output.")
    };
    egui::Area::new(egui::Id::new(("empty-hint", c.capturing)))
        .pivot(Align2::LEFT_BOTTOM)
        .fixed_pos(scene.left_bottom() + egui::vec2(26.0, -26.0 - lift))
        .interactable(false)
        .show(ctx, |ui| {
            ui.multiply_opacity(c.hint_alpha);
            egui::Frame::new()
                .fill(t.float)
                .corner_radius(CornerRadius::same(14))
                .stroke(Stroke::new(1.0_f32, t.border))
                .inner_margin(Margin::symmetric(14, 10))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 12.0;
                        let (r, _) = ui.allocate_exact_size(Vec2::splat(36.0), Sense::hover());
                        ui.painter().rect_filled(r, CornerRadius::same(8), t.control);
                        paint_icon(ui.painter(), Icon::Audio, r.center(), t.dim);
                        ui.vertical(|ui| {
                            ui.set_max_width(300.0);
                            ui.spacing_mut().item_spacing.y = 2.0;
                            ui.label(RichText::new(title).size(14.0).strong().color(t.text));
                            ui.label(RichText::new(body).size(12.0).color(t.dim));
                        });
                    });
                });
        });
}

impl UiState {
    /// The now-playing card in the bottom-left corner, `lift` points above the bottom edge
    /// (clear of the volume bar and timeline).
    pub(super) fn toast(&self, ctx: &egui::Context, scene: Rect, c: &UiCtx, t: &Tokens, lift: f32) {
        // An Area keeps the size it measured last time, so a longer title would wrap to the
        // previous track's width. A fresh id per track makes it measure again.
        egui::Area::new(egui::Id::new(("toast", &c.track.title, &c.track.artist, &c.track.album)))
            .pivot(Align2::LEFT_BOTTOM)
            .fixed_pos(scene.left_bottom() + egui::vec2(26.0, -26.0 - lift))
            .interactable(false)
            .show(ctx, |ui| {
                ui.multiply_opacity(c.toast_alpha);
                egui::Frame::new()
                    .fill(t.float)
                    .corner_radius(CornerRadius::same(14))
                    .stroke(Stroke::new(1.0_f32, t.border))
                    .inner_margin(Margin::same(10))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            if let Some(tex) = &self.art {
                                ui.add(egui::Image::new((tex.id(), Vec2::splat(64.0))).corner_radius(CornerRadius::same(9)));
                            }
                            ui.vertical(|ui| {
                                ui.set_max_width(360.0);
                                ui.add_space(6.0);
                                ui.label(RichText::new(&c.track.title).size(20.0).strong().color(t.text));
                                ui.label(RichText::new(&c.track.artist).size(14.0).color(t.dim));
                                // Singles often have the album named after the track; don't repeat it.
                                let album = &c.track.album;
                                if !album.is_empty() && !album.eq_ignore_ascii_case(&c.track.title) {
                                    ui.label(RichText::new(album).size(11.5).color(t.faint));
                                }
                                let pb = c.track.playback;
                                if !pb.playing && pb.can_play_pause {
                                    ui.label(RichText::new("Paused").size(11.5).color(t.faint));
                                }
                            });
                            ui.add_space(8.0);
                        });
                    });
            });
    }

    /// The track timeline over the scene, above the volume bar: wide enough to scrub precisely.
    pub(super) fn timeline(&mut self, ctx: &egui::Context, scene: Rect, c: &UiCtx, t: &Tokens, lift: f32, actions: &mut Vec<UiAction>) {
        let pb = c.track.playback;
        let w = (scene.width() - 120.0).clamp(240.0, 640.0);
        let resp = egui::Area::new(egui::Id::new("timeline"))
            .pivot(Align2::CENTER_BOTTOM)
            .fixed_pos(scene.center_bottom() + egui::vec2(0.0, -18.0 - lift))
            .show(ctx, |ui| {
                ui.multiply_opacity(c.timeline_alpha);
                egui::Frame::new()
                    .fill(t.float)
                    .stroke(Stroke::new(1.0_f32, t.border))
                    .corner_radius(CornerRadius::same(16))
                    .inner_margin(Margin::symmetric(14, 6))
                    .show(ui, |ui| {
                        ui.set_width(w - 28.0);
                        seek_bar(ui, t, &pb, 22.0, 12.0, Some(&mut self.scrub), actions);
                    });
            });
        self.controls_hot |= self.scrub.is_some() || resp.response.contains_pointer();
    }

    /// Output volume over the bottom centre of the scene: mute button, wake slider, percent.
    pub(super) fn volume_bar(&mut self, ctx: &egui::Context, scene: Rect, c: &UiCtx, t: &Tokens, accent: Color32) {
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
                            let tip = if muted { "Unmute" } else { "Mute" };
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
        self.controls_hot = resp.inner.inner || resp.response.contains_pointer();
    }
}

/// Fullscreen button over the top-right corner of the scene.
/// An invisible strip along the docked panel's outer edge: the window has no frame while docked,
/// so this is where it's resized from.
pub(super) fn dock_grip(ctx: &egui::Context, edge: DockSide, actions: &mut Vec<UiAction>) {
    const W: f32 = 6.0;
    let full = ctx.content_rect();
    let x = if edge == DockSide::Left { full.left() } else { full.right() - W };
    let rect = Rect::from_min_size(egui::pos2(x, full.top()), egui::vec2(W, full.height()));
    // Sized by what it allocates and never pushed back on screen: a constrained Area at the
    // right edge gets shifted left and its hit rect then covers the controls beside it.
    let area = egui::Area::new(egui::Id::new("dock-grip")).order(egui::Order::Foreground).fixed_pos(rect.min).constrain(false);
    area.show(ctx, |ui| {
        let (_, resp) = ui.allocate_exact_size(rect.size(), Sense::click_and_drag());
        if resp.hovered() || resp.dragged() {
            ctx.set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        }
        if resp.hovered() && ui.input(|i| i.pointer.primary_pressed()) {
            actions.push(UiAction::DockResize);
        }
    });
}

pub(super) fn window_buttons(ctx: &egui::Context, scene: Rect, c: &UiCtx, t: &Tokens, actions: &mut Vec<UiAction>) {
    let frame = egui::Frame::new()
        .fill(t.float)
        .stroke(Stroke::new(1.0_f32, t.border))
        .corner_radius(CornerRadius::same(18))
        .inner_margin(Margin::symmetric(6, 4));

    // The dock toggle: Dock when off, Undock when docked, and Waiting (lit) while docking is on
    // but there's no Spotify window to dock to or no room beside it. Fullscreen suspends docking,
    // so it isn't offered there.
    let windowed = c.mode == WindowMode::Windowed;
    let last = if c.settings.dock_last == DockSide::Off { DockSide::Right } else { c.settings.dock_last };
    let dock_state = match (c.settings.dock, c.dock_edge) {
        (DockSide::Off, _) => 0,
        (_, Some(_)) => 1,
        _ => 2,
    };
    // The id follows the buttons and labels, so the Area doesn't keep a narrower size.
    egui::Area::new(egui::Id::new(("window-buttons", c.show_sidebar, windowed, dock_state)))
        .pivot(Align2::RIGHT_TOP)
        .fixed_pos(scene.right_top() + egui::vec2(-12.0, 12.0))
        .show(ctx, |ui| {
            ui.multiply_opacity(c.controls_alpha);
            frame.show(ui, |ui| {
                ui.horizontal(|ui| {
                    if !c.show_sidebar && icon_button(ui, t, Icon::ShowSidebar, 26.0).on_hover_text("Show sidebar").clicked() {
                        actions.push(UiAction::ShowSidebar);
                    }
                    if windowed {
                        // Green while it's an invitation (Dock) or in progress (Waiting); plain once
                        // docked, where Undock is just a way back out.
                        let side = if last == DockSide::Left { "left" } else { "right" };
                        let (icon, label, tip, tint, action) = match dock_state {
                            0 => (
                                Icon::Dock { left: last == DockSide::Left },
                                "Dock to Spotify",
                                format!("Sticks this window to the {side} of Spotify's window, at its height, and follows it around. Side and width are in General."),
                                Some((SPOTIFY_GREEN, 40)),
                                UiAction::Dock,
                            ),
                            1 => (
                                Icon::Undock { left: c.dock_edge == Some(DockSide::Left) },
                                "Undock",
                                "Back to a normal window, where it was before docking.".to_owned(),
                                None,
                                UiAction::Undock,
                            ),
                            _ => (
                                Icon::Dock { left: c.settings.dock == DockSide::Left },
                                "Waiting for Spotify",
                                format!("{} Click to turn docking off.", c.dock_status),
                                Some((SPOTIFY_GREEN, 18)),
                                UiAction::Undock,
                            ),
                        };
                        if labeled_icon_button(ui, t, icon, label, tint).on_hover_text(tip).clicked() {
                            actions.push(action);
                        }
                    }
                    let fs = c.mode == WindowMode::Fullscreen;
                    let tip = if fs { "Exit fullscreen" } else { "Fullscreen" };
                    if icon_button(ui, t, Icon::Fullscreen, 26.0).on_hover_text(tip).clicked() {
                        actions.push(UiAction::SetMode(if fs { WindowMode::Windowed } else { WindowMode::Fullscreen }));
                    }
                });
            });
        });
}
