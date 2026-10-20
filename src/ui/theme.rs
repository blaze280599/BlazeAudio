//! Neutral colour tokens per UI theme, and the egui style built from them.

use egui::{Color32, CornerRadius, FontId, Stroke};

use super::UiState;
use crate::settings::UiTheme;

/// Neutral colour tokens for one UI theme. Album colours only appear in data (spectrum, swatches).
#[derive(Clone, Copy)]
pub struct Tokens {
    pub(super) sidebar: Color32,
    pub(super) drawer: Color32,
    pub(super) border: Color32,
    pub(super) text: Color32,
    pub(super) dim: Color32,
    pub(super) faint: Color32,
    pub(super) hover: Color32,
    pub(super) selected: Color32,
    pub(super) card: Color32,
    pub(super) control: Color32,
    /// Translucent fill for things floating over the scene (toast, stats, scene controls).
    pub(super) float: Color32,
}

pub(super) fn tokens(t: UiTheme) -> Tokens {
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

pub(super) fn with_alpha(c: Color32, a: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), a)
}

pub(super) fn to_c32(c: [f32; 3], a: u8) -> Color32 {
    let f = |v: f32| (crate::palette::linear_to_srgb(v.clamp(0.0, 1.0)) * 255.0) as u8;
    Color32::from_rgba_unmultiplied(f(c[0]), f(c[1]), f(c[2]), a)
}

impl UiState {
    pub(super) fn style(&mut self, ctx: &egui::Context, theme: UiTheme) {
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
}
