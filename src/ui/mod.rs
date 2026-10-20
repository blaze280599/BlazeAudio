//! egui UI: docked sidebar (expanded or icon rail) with a settings drawer, on-scene controls
//! (window buttons, volume bar, timeline), now-playing toast, and stats HUD.
//!
//! - `theme`: colour tokens per UI theme and the egui style
//! - `widgets`: reusable controls (scenes use `knob` / `choice` for their settings)
//! - `icons`: the painted line icons
//! - `playback`: seek bar and transport buttons
//! - `sidebar`, `drawer`: the docked panels
//! - `overlay`: everything floating over the scene

mod drawer;
mod icons;
mod overlay;
mod playback;
mod sidebar;
mod theme;
pub mod widgets;

use egui::{Margin, Rect, Stroke};

use icons::Icon;
pub use theme::Tokens;
use theme::{to_c32, tokens, with_alpha};

use crate::media::{MediaCmd, TrackInfo, ART_SIZE};
use crate::palette::Palette;
use crate::settings::{Settings, UiTheme, WindowMode};
use crate::sys::ProcStats;
use crate::vis::VisState;
use crate::volume::Volume;
use crate::wake::WakeSlider;

const SIDEBAR_W: f32 = 224.0;
const RAIL_W: f32 = 56.0;
const DRAWER_W: f32 = 300.0;

pub enum UiAction {
    SetMode(WindowMode),
    HideSidebar,
    Media(MediaCmd),
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

pub struct UiState {
    art: Option<egui::TextureHandle>,
    /// Theme the egui style was last built for.
    styled: Option<UiTheme>,
    /// Settings section open in the drawer beside the sidebar.
    pub drawer: Option<Section>,
    volume_slider: WakeSlider,
    /// The pointer is on the volume bar or timeline (or dragging one), so they shouldn't fade out.
    pub controls_hot: bool,
    /// Where the on-scene timeline is being dragged to; sent as a seek on release.
    scrub: Option<f32>,
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
    /// Opacity of the timeline: the on-scene controls, or a flash after a seek key.
    pub timeline_alpha: f32,
}

impl UiState {
    pub fn new() -> Self {
        Self {
            art: None,
            styled: None,
            drawer: None,
            volume_slider: WakeSlider::default(),
            controls_hot: false,
            scrub: None,
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
            overlay::stats_hud(ctx, scene_rect, &c, &ft);
        }

        // The bottom-centre stack: volume bar at the bottom, timeline above it. Each slides
        // down as the one below it fades, and the toast rises above whatever is showing.
        let vol_a = if c.settings.show_volume && c.volume.available() { c.volume_alpha } else { 0.0 };
        let tl_a = c.timeline_alpha;
        let toast_lift = 64.0 * vol_a + 46.0 * tl_a;

        if c.toast_alpha > 0.0 && c.settings.show_track_toast && !c.track.title.is_empty() {
            self.toast(ctx, scene_rect, &c, &ft, toast_lift);
        }

        if c.controls_alpha > 0.0 {
            overlay::window_buttons(ctx, scene_rect, &c, &ft, &mut actions);
        }
        self.controls_hot = false;
        if vol_a > 0.0 {
            self.volume_bar(ctx, scene_rect, &c, &ft, accent);
        }
        if tl_a > 0.0 {
            // The volume pill is 40 pt tall; 8 pt between them.
            self.timeline(ctx, scene_rect, &c, &ft, 48.0 * vol_a, &mut actions);
        } else if let Some(v) = self.scrub.take() {
            // Hidden mid-drag (e.g. the track ended): still go where it was dragged.
            actions.push(UiAction::Media(MediaCmd::Seek(v)));
        }

        (actions, scene_rect)
    }
}
