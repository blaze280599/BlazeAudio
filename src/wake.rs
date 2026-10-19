//! "Wake" slider for egui: a row of pill bars lit up to the value. Moving the handle raises a
//! wake of taller bars around it, wider behind than ahead, that grows with speed and settles
//! once the handle stops. Ported from the React Bits WakeSlider (springs and wake shape match;
//! the handle glides with a critically damped spring and the wake follows smoothed velocity).

use egui::{Color32, CornerRadius, Rect, Response, Sense};

/// Natural frequency x glide time that settles a critically damped spring in about `glide` s.
const SETTLE: f32 = 9.23;
/// Handle speed (percent of the track per second) that raises the wake fully.
const FULL_SPEED: f32 = 320.0;
const MIN_REACH: f32 = 1.5;
const FLAT: f32 = 0.002;

#[derive(Clone, Copy)]
pub struct WakeStyle {
    pub bars: usize,
    /// Full bar height in points (the wake's ceiling).
    pub height: f32,
    /// Bar height at rest in points.
    pub rest_height: f32,
    pub gap: f32,
    pub fill: Color32,
    pub track: Color32,
    /// Tint the raised bars take on by their lift.
    pub crest: Option<Color32>,
    pub sensitivity: f32,
    /// Half-width of the wake at full speed, in bars.
    pub reach: f32,
    /// How much wider the wake is behind the handle than ahead.
    pub skew: f32,
    /// Seconds the handle takes to settle.
    pub glide: f32,
    /// Milliseconds of velocity lag.
    pub smoothing_ms: f32,
    /// Snapping grid, in value units (the value runs 0..=100).
    pub step: f32,
}

impl Default for WakeStyle {
    fn default() -> Self {
        Self {
            bars: 32,
            height: 56.0,
            rest_height: 12.0,
            gap: 4.0,
            fill: Color32::from_gray(245),
            track: Color32::from_gray(39),
            crest: None,
            sensitivity: 1.0,
            reach: 6.0,
            skew: 0.6,
            glide: 0.3,
            smoothing_ms: 100.0,
            step: 1.0,
        }
    }
}

/// Animation state; keep one per slider across frames.
#[derive(Default)]
pub struct WakeSlider {
    init: bool,
    /// Handle position in percent, and its velocity.
    head: f32,
    head_v: f32,
    /// Smoothed handle velocity, and its own rate of change.
    speed: f32,
    speed_v: f32,
}

/// Advance a critically damped spring at natural frequency `w` toward `target` by `dt` (exact).
fn spring(x: &mut f32, v: &mut f32, target: f32, w: f32, dt: f32) {
    let d = *x - target;
    let k = *v + w * d;
    let e = (-w * dt).exp();
    *x = target + (d + k * dt) * e;
    *v = (*v - w * k * dt) * e;
}

fn smoothstep(s: f32) -> f32 {
    s * s * (3.0 - 2.0 * s)
}

impl WakeSlider {
    /// Draws the slider for `value` (0..=100) at `width` points. Returns the response and the
    /// new value when the user dragged, clicked or scrolled it.
    pub fn show(&mut self, ui: &mut egui::Ui, value: f32, width: f32, st: &WakeStyle) -> (Response, Option<f32>) {
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(width, st.height.max(28.0)), Sense::click_and_drag());
        let snap = |v: f32| ((v / st.step).round() * st.step).clamp(0.0, 100.0);

        let mut changed = None;
        if resp.is_pointer_button_down_on() || resp.dragged() {
            if let Some(p) = resp.interact_pointer_pos() {
                changed = Some(snap((p.x - rect.left()) / rect.width() * 100.0));
            }
        } else if resp.hovered() {
            let scroll = ui.input(|i| i.raw_scroll_delta.y + i.raw_scroll_delta.x);
            if scroll != 0.0 {
                changed = Some(snap(value + scroll.signum() * st.step * 2.0));
            }
        }
        if resp.hovered() || resp.dragged() {
            ui.ctx().set_cursor_icon(if resp.dragged() { egui::CursorIcon::Grabbing } else { egui::CursorIcon::Grab });
        }
        let changed = changed.filter(|v| (v - value).abs() > 1e-4);
        let target = changed.unwrap_or(value).clamp(0.0, 100.0);

        let dt = ui.input(|i| i.stable_dt).clamp(1e-4, 0.1);
        if !self.init {
            self.init = true;
            self.head = target;
        }
        let w0 = SETTLE / st.glide.max(0.01);
        spring(&mut self.head, &mut self.head_v, target, w0, dt);
        let wv = 2000.0 / st.smoothing_ms.max(1.0);
        spring(&mut self.speed, &mut self.speed_v, self.head_v, wv, dt);

        let n = st.bars.max(2);
        let h = self.head / 100.0 * (n - 1) as f32;
        let amp = smoothstep(((self.speed.abs() * st.sensitivity) / FULL_SPEED).clamp(0.0, 1.0));
        let dir = if self.speed < 0.0 { -1.0 } else { 1.0 };
        let r = MIN_REACH + (st.reach - MIN_REACH) * amp;
        let behind = r * (1.0 + st.skew);
        let ahead = r * (1.0 - 0.5 * st.skew);
        let lit = h.round() as usize;
        let rest = st.rest_height.min(st.height - 1.0) / st.height;

        let bar_w = ((rect.width() - st.gap * (n - 1) as f32) / n as f32).max(1.0);
        let p = ui.painter();
        for i in 0..n {
            let d = i as f32 - h;
            let reach = if d * dir < 0.0 { behind } else { ahead };
            let lift = if d.abs() < reach && amp >= FLAT { amp * (std::f32::consts::PI * d / (2.0 * reach)).cos().powi(2) } else { 0.0 };
            let bh = st.height * (rest + lift * (1.0 - rest));
            let x = rect.left() + i as f32 * (bar_w + st.gap);
            let bar = Rect::from_center_size(egui::pos2(x + bar_w / 2.0, rect.center().y), egui::vec2(bar_w, bh));
            let base = if i <= lit { st.fill } else { st.track };
            let col = match st.crest {
                Some(c) if lift > 0.0 => base.lerp_to_gamma(c, lift),
                _ => base,
            };
            p.rect_filled(bar, CornerRadius::same(255), col);
        }

        // Keep animating until the handle and the wake have settled.
        if (self.head - target).abs() > 0.01 || self.head_v.abs() > 0.05 || amp >= FLAT || self.speed.abs() > 0.5 {
            ui.ctx().request_repaint();
        }
        (resp, changed)
    }
}
