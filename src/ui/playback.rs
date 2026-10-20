//! Track position and transport controls, used by the sidebar card and the on-scene timeline.

use egui::{Align2, CornerRadius, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};

use super::icons::{paint_icon, Icon};
use super::theme::{with_alpha, Tokens};
use super::UiAction;
use crate::media::{MediaCmd, Playback};

/// "3:07", or "1:02:45" past an hour.
pub(super) fn fmt_time(secs: f32) -> String {
    let s = secs.max(0.0) as u32;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

/// Elapsed time, a seek bar and the track length. With `scrub` the bar can be dragged (the
/// drag position lives there and is sent on release) and scrolled ±5 s; without it a click
/// jumps. Hovering shows the time under the pointer.
pub(super) fn seek_bar(ui: &mut egui::Ui, t: &Tokens, pb: &Playback, height: f32, font_size: f32, scrub: Option<&mut Option<f32>>, actions: &mut Vec<UiAction>) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), height), Sense::hover());
    let font = FontId::monospace(font_size);
    // Both labels get the width of the longer one, so the bar doesn't shift as time passes.
    let slot = ui.painter().layout_no_wrap(fmt_time(pb.duration), font.clone(), t.faint).size().x;
    let y = rect.center().y;
    let track = Rect::from_min_max(egui::pos2(rect.left() + slot + 8.0, y), egui::pos2(rect.right() - slot - 8.0, y));
    if track.width() < 8.0 {
        return;
    }
    let draggable = scrub.is_some();
    let sense = if !pb.can_seek {
        Sense::hover()
    } else if draggable {
        Sense::click_and_drag()
    } else {
        Sense::click()
    };
    // The bar is a few points tall; take the whole row height (and a little past the ends) to
    // make it easy to hit.
    let hit = Rect::from_min_max(egui::pos2(track.left() - 6.0, rect.top()), egui::pos2(track.right() + 6.0, rect.bottom()));
    let resp = ui.interact(hit, ui.make_persistent_id("seek"), sense);
    let at = |x: f32| ((x - track.left()) / track.width()).clamp(0.0, 1.0) * pb.duration;
    let end = (pb.duration - 1.0).max(0.0);

    // A position chosen this frame, shown right away (the media thread catches up next frame).
    let mut chosen = None;
    if pb.can_seek {
        let seek = |v: f32, actions: &mut Vec<UiAction>| {
            actions.push(UiAction::Media(MediaCmd::Seek(v)));
            Some(v)
        };
        match scrub {
            Some(scrub) => {
                if resp.is_pointer_button_down_on() {
                    if let Some(p) = resp.interact_pointer_pos() {
                        *scrub = Some(at(p.x));
                    }
                } else if let Some(v) = scrub.take() {
                    chosen = seek(v, actions);
                } else if resp.hovered() {
                    let s = ui.input(|i| i.raw_scroll_delta.y + i.raw_scroll_delta.x);
                    if s != 0.0 {
                        chosen = seek((pb.position_now() + s.signum() * 5.0).clamp(0.0, end), actions);
                    }
                }
                chosen = chosen.or(*scrub);
            }
            None => {
                if let (true, Some(p)) = (resp.clicked(), resp.interact_pointer_pos()) {
                    chosen = seek(at(p.x), actions);
                }
            }
        }
    }

    let held = draggable && resp.is_pointer_button_down_on();
    let active = pb.can_seek && (resp.hovered() || held);
    let pos = chosen.unwrap_or_else(|| pb.position_now());
    // Over the scene the pill is translucent and the visuals busy, so labels and the unplayed
    // track need more contrast than on the solid sidebar.
    let (label, rail) = if draggable { (t.dim, t.faint.gamma_multiply(0.45)) } else { (t.faint, t.control) };
    let p = ui.painter();
    p.text(egui::pos2(track.left() - 8.0, y), Align2::RIGHT_CENTER, fmt_time(pos), font.clone(), if held { t.text } else { label });
    p.text(egui::pos2(track.right() + 8.0, y), Align2::LEFT_CENTER, fmt_time(pb.duration), font, label);
    let thick = if active { 5.0 } else { 3.0 };
    let bar = Rect::from_center_size(track.center(), egui::vec2(track.width(), thick));
    p.rect_filled(bar, CornerRadius::same(3), rail);
    let x = bar.left() + bar.width() * (pos / pb.duration).clamp(0.0, 1.0);
    let rest = if draggable { t.text.gamma_multiply(0.85) } else { t.dim };
    p.rect_filled(Rect::from_min_max(bar.min, egui::pos2(x, bar.max.y)), CornerRadius::same(3), if active { t.text } else { rest });
    if active {
        p.circle_filled(egui::pos2(x, y), thick + 1.5, t.text);
        // While dragging the bubble rides on the handle; otherwise it follows the pointer.
        let bx = if held { x } else { resp.hover_pos().map_or(x, |h| h.x.clamp(track.left(), track.right())) };
        time_bubble(ui.ctx(), egui::pos2(bx, rect.top() - 3.0), &fmt_time(at(bx)), t);
        let icon = match (draggable, held) {
            (true, true) => egui::CursorIcon::Grabbing,
            (true, false) => egui::CursorIcon::Grab,
            _ => egui::CursorIcon::PointingHand,
        };
        ui.ctx().set_cursor_icon(icon);
    }
}

/// A small time label floating above `anchor` (its bottom centre), on top of everything.
pub(super) fn time_bubble(ctx: &egui::Context, anchor: Pos2, text: &str, t: &Tokens) {
    let p = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("seek-bubble")));
    let galley = p.layout_no_wrap(text.to_owned(), FontId::monospace(11.0), t.text);
    let size = galley.size() + egui::vec2(12.0, 6.0);
    let r = Rect::from_min_size(anchor - egui::vec2(size.x / 2.0, size.y), size);
    p.rect(r, CornerRadius::same(5), with_alpha(t.drawer, 255), Stroke::new(1.0_f32, t.border), StrokeKind::Inside);
    p.galley(r.center() - galley.size() / 2.0, galley, t.text);
}

/// Previous / play-pause / next, centred. Buttons the app doesn't support are dimmed.
pub(super) fn transport(ui: &mut egui::Ui, t: &Tokens, pb: &Playback, actions: &mut Vec<UiAction>) {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 28.0), Sense::hover());
    let play = if pb.playing { (Icon::Pause, "Pause (Space)") } else { (Icon::Play, "Play (Space)") };
    let buttons = [
        (-40.0, Icon::Previous, "Previous track (P)", pb.can_previous, MediaCmd::Previous, false),
        (0.0, play.0, play.1, pb.can_play_pause, MediaCmd::PlayPause, true),
        (40.0, Icon::Next, "Next track (N)", pb.can_next, MediaCmd::Next, false),
    ];
    for (i, (dx, icon, tip, enabled, cmd, primary)) in buttons.into_iter().enumerate() {
        let r = Rect::from_center_size(rect.center() + egui::vec2(dx, 0.0), Vec2::splat(28.0));
        let sense = if enabled { Sense::click() } else { Sense::hover() };
        let b = ui.interact(r, resp.id.with(i), sense);
        let hovered = enabled && b.hovered();
        let p = ui.painter();
        let col = if primary {
            // A filled disc with the icon cut out of it in the panel colour.
            let disc = if !enabled { t.faint } else if hovered { t.text } else { t.text.gamma_multiply(0.85) };
            p.circle_filled(r.center(), 14.0, disc);
            t.sidebar
        } else {
            if hovered {
                p.circle_filled(r.center(), 14.0, t.hover);
            }
            if !enabled { t.faint } else if hovered { t.text } else { t.dim }
        };
        paint_icon(p, icon, r.center(), col);
        if b.on_hover_text(tip).clicked() {
            actions.push(UiAction::Media(cmd));
        }
    }
}
