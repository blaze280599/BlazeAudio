//! The docked sidebar: logo, section list, collapse button and the now-playing card.

use egui::{Align2, Color32, CornerRadius, FontId, Margin, Rect, RichText, Sense, Stroke, Vec2};

use super::icons::{paint_icon, Icon};
use super::playback::{seek_bar, transport};
use super::theme::Tokens;
use super::widgets::{icon_button, nav_item, spectrum_meter};
use super::{Section, UiAction, UiCtx, UiState, SECTIONS};
use crate::media::MediaCmd;
use crate::update::Status as UpdateStatus;
use crate::settings::WindowMode;

impl UiState {
    pub(super) fn sidebar(&mut self, ui: &mut egui::Ui, c: &mut UiCtx, t: &Tokens, accent: Color32, actions: &mut Vec<UiAction>) {
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
            let resp = nav_item(ui, t, icon, name, open, expanded);
            // A dot on General while an update waits there.
            if sec == Section::General && matches!(c.update, UpdateStatus::Available(_) | UpdateStatus::Installed(_)) {
                ui.painter().circle_filled(egui::pos2(resp.rect.left() + 29.0, resp.rect.center().y - 7.0), 3.5, accent);
            }
            if resp.clicked() {
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
            self.now_playing(ui, c, t, accent, expanded, actions);
        });
    }

    pub(super) fn now_playing(&self, ui: &mut egui::Ui, c: &UiCtx, t: &Tokens, accent: Color32, expanded: bool, actions: &mut Vec<UiAction>) {
        let tip = if c.track.title.is_empty() { "Nothing playing".to_owned() } else { format!("{}\n{}", c.track.title, c.track.artist) };
        let pb = c.track.playback;
        let has_track = !c.track.title.is_empty();
        let show_time = has_track && pb.duration > 0.0;
        let show_controls = has_track && (pb.can_play_pause || pb.can_next || pb.can_previous);
        if !expanded {
            // Bottom-up: the play button goes under the cover.
            if show_controls && pb.can_play_pause {
                let (icon, tip) = if pb.playing { (Icon::Pause, "Pause (Space)") } else { (Icon::Play, "Play (Space)") };
                if icon_button(ui, t, icon, 40.0).on_hover_text(tip).clicked() {
                    actions.push(UiAction::Media(MediaCmd::PlayPause));
                }
            }
            let (rect, resp) = ui.allocate_exact_size(Vec2::splat(40.0), Sense::hover());
            self.paint_art(ui, rect, t, 7);
            resp.on_hover_text(tip);
            return;
        }
        // The sidebar footer lays out bottom-up; give the card a fixed top-down box of its own.
        // Each optional row adds its height plus the 7 pt item spacing.
        let extra = if show_time { 14.0 + 7.0 } else { 0.0 } + if show_controls { 28.0 + 7.0 } else { 0.0 };
        let size = egui::vec2(ui.available_width(), 12.0 * 2.0 + 48.0 + 10.0 + 28.0 + extra);
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
                    if show_time {
                        seek_bar(ui, t, &pb, 14.0, 10.5, None, actions);
                    }
                    if show_controls {
                        transport(ui, t, &pb, actions);
                    }
                    ui.add_space(10.0 - ui.spacing().item_spacing.y);
                    spectrum_meter(ui, c.vis, t, accent, 28.0);
                });
        });
    }

    pub(super) fn paint_art(&self, ui: &egui::Ui, rect: Rect, t: &Tokens, radius: u8) {
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
}
