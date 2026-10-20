//! Reusable controls. `knob`, `choice`, `num` and `PCT` are what scenes use for their settings.

use egui::{Align2, Color32, CornerRadius, FontId, Rect, RichText, Sense, Vec2};

use super::icons::{paint_icon, Icon};
use super::theme::Tokens;
use crate::vis::VisState;

/// One sidebar row: icon, plus a label when expanded (a tooltip stands in for it on the rail).
pub(super) fn nav_item(ui: &mut egui::Ui, t: &Tokens, icon: Icon, label: &str, selected: bool, expanded: bool) -> egui::Response {
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

pub(super) fn icon_button(ui: &mut egui::Ui, t: &Tokens, icon: Icon, size: f32) -> egui::Response {
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
pub struct Fmt {
    decimals: usize,
    suffix: &'static str,
    pct: bool,
    log: bool,
}

pub const fn num(decimals: usize, suffix: &'static str) -> Fmt {
    Fmt { decimals, suffix, pct: false, log: false }
}

/// Whole percent; the value snaps to 0.01.
pub const PCT: Fmt = Fmt { decimals: 2, suffix: "%", pct: true, log: false };

impl Fmt {
    pub const fn log(self) -> Self {
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
pub(super) fn label_row(ui: &mut egui::Ui, t: &Tokens, label: &str, modified: bool, default: &str) -> bool {
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
pub fn knob<T: egui::emath::Numeric>(ui: &mut egui::Ui, t: &Tokens, label: &str, value: &mut T, default: T, range: std::ops::RangeInclusive<T>, fmt: Fmt) {
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
pub fn choice<T: Copy + PartialEq>(ui: &mut egui::Ui, t: &Tokens, label: &str, value: &mut T, default: T, options: &[(T, &str)]) {
    let name = options.iter().find(|o| o.0 == default).map_or("", |o| o.1);
    if label_row(ui, t, label, *value != default, name) {
        *value = default;
    }
    segmented(ui, t, value, options);
}

/// Equal-width segments, one per option. Returns true when the user picked a different one.
pub(super) fn segmented<T: Copy + PartialEq>(ui: &mut egui::Ui, t: &Tokens, value: &mut T, options: &[(T, &str)]) -> bool {
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
pub(super) fn heading(ui: &mut egui::Ui, t: &Tokens, text: &str) {
    ui.add_space(8.0);
    ui.label(RichText::new(text).small().strong().color(t.dim));
}

/// Live band levels, drawn in the album accent: the one place the UI takes the track's colour.
pub(super) fn spectrum_meter(ui: &mut egui::Ui, vis: &VisState, t: &Tokens, accent: Color32, height: f32) {
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
