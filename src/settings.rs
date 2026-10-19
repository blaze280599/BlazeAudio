use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub enum WindowMode {
    Windowed,
    Fullscreen,
    Overlay,
}

/// Which visual is on screen.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub enum Scene {
    Terminal,
    GradientAscii,
    Scope,
}

/// How Phosphor Scope plots the waveform (order matches the shader).
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub enum ScopeMode {
    /// Classic time sweep, triggered on a rising zero crossing. Also read for the X-Y mode
    /// earlier builds had, so their settings files still load.
    #[serde(alias = "Xy")]
    Yt,
    /// The sweep wrapped around a slowly turning circle.
    Polar,
}

/// Light shape for the Gradient ASCII scene (order matches the shader).
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub enum AsciiShape {
    Horizon,
    Pinch,
    Rise,
    Eclipse,
}

/// Colour scheme for the app's UI (not the scenes).
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub enum UiTheme {
    Dark,
    Oled,
    Light,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Not `scene`: older builds stored a numeric scene index under that name.
    pub visual: Scene,
    pub ascii_shape: AsciiShape,
    pub ascii_cell: f32,
    pub ascii_density: f32,
    pub ascii_glyphs: f32,
    pub ascii_gradient: f32,
    pub scope_mode: ScopeMode,
    /// Afterglow time constant in seconds.
    pub scope_persistence: f32,
    /// Beam width in pixels at 1440p.
    pub scope_beam: f32,
    pub scope_gain: f32,

    pub intensity: f32,
    pub bloom_strength: f32,
    pub bloom_threshold: f32,
    pub exposure: f32,
    pub chromatic: f32,
    pub grain: f32,
    pub vignette: f32,
    pub render_scale: f32,

    pub sensitivity: f32,
    pub attack_ms: f32,
    pub decay_ms: f32,
    pub beat_sensitivity: f32,

    pub album_palette: bool,
    pub preset_palette: usize,

    pub show_track_toast: bool,
    pub show_volume: bool,
    pub show_stats: bool,
    pub idle_fps: u32,

    pub start_mode: WindowMode,
    pub sidebar_expanded: bool,
    pub theme: UiTheme,
    pub overlay_opacity: f32,
    pub overlay_click_through: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            visual: Scene::Terminal,
            ascii_shape: AsciiShape::Horizon,
            ascii_cell: 18.0,
            ascii_density: 0.8,
            ascii_glyphs: 0.85,
            ascii_gradient: 0.9,
            scope_mode: ScopeMode::Yt,
            scope_persistence: 0.08,
            scope_beam: 3.0,
            scope_gain: 1.0,
            intensity: 1.0,
            bloom_strength: 0.9,
            bloom_threshold: 0.75,
            exposure: 1.0,
            chromatic: 0.6,
            grain: 0.35,
            vignette: 0.55,
            render_scale: 1.0,
            sensitivity: 1.0,
            attack_ms: 12.0,
            decay_ms: 140.0,
            beat_sensitivity: 1.0,
            album_palette: true,
            preset_palette: 0,
            show_track_toast: true,
            show_volume: true,
            show_stats: false,
            idle_fps: 30,
            start_mode: WindowMode::Windowed,
            sidebar_expanded: true,
            theme: UiTheme::Dark,
            overlay_opacity: 0.9,
            overlay_click_through: true,
        }
    }
}

fn path() -> Option<PathBuf> {
    let base = std::env::var_os("APPDATA")?;
    Some(PathBuf::from(base).join("BlazeAudio").join("settings.json"))
}

impl Settings {
    pub fn load() -> Self {
        path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            // Tolerate a UTF-8 BOM (e.g. after hand-editing with Windows tools).
            .and_then(|s| serde_json::from_str(s.trim_start_matches('\u{feff}')).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        if let Some(p) = path() {
            if let Some(dir) = p.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Ok(s) = serde_json::to_string_pretty(self) {
                let _ = std::fs::write(p, s);
            }
        }
    }

    /// Everything in the Visuals panel back to defaults, except the scene on screen and the
    /// render scale (a performance choice, not part of the look).
    pub fn reset_visuals(&mut self) {
        self.copy_look(&Settings::default());
    }

    /// Takes the look (what Randomize and Reset change) from `d`; scene and render scale stay.
    pub fn copy_look(&mut self, d: &Settings) {
        self.ascii_shape = d.ascii_shape;
        self.ascii_cell = d.ascii_cell;
        self.ascii_density = d.ascii_density;
        self.ascii_glyphs = d.ascii_glyphs;
        self.ascii_gradient = d.ascii_gradient;
        self.scope_mode = d.scope_mode;
        self.scope_persistence = d.scope_persistence;
        self.scope_beam = d.scope_beam;
        self.scope_gain = d.scope_gain;
        self.intensity = d.intensity;
        self.bloom_strength = d.bloom_strength;
        self.bloom_threshold = d.bloom_threshold;
        self.exposure = d.exposure;
        self.chromatic = d.chromatic;
        self.grain = d.grain;
        self.vignette = d.vignette;
    }

    /// A random look: every scene's shape/mode and sliders, plus the post effects. Ranges are
    /// narrower than the sliders' so the result is always watchable. Scene and render scale stay.
    pub fn randomize_visuals(&mut self) {
        let mut seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0x9e37_79b9_7f4a_7c15, |d| d.as_nanos() as u64)
            | 1;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 40) as f32 / (1u64 << 24) as f32
        };
        let mut r = |lo: f32, hi: f32| lo + (hi - lo) * next();

        self.ascii_shape = [AsciiShape::Horizon, AsciiShape::Pinch, AsciiShape::Rise, AsciiShape::Eclipse][(r(0.0, 4.0) as usize).min(3)];
        self.ascii_cell = r(10.0, 32.0).round();
        self.ascii_density = r(0.5, 1.0);
        self.ascii_glyphs = r(0.6, 1.0);
        self.ascii_gradient = r(0.6, 1.0);
        self.scope_mode = if r(0.0, 1.0) < 0.5 { ScopeMode::Yt } else { ScopeMode::Polar };
        // Log-uniform, so short and long afterglows are equally likely.
        self.scope_persistence = 0.04 * 15f32.powf(r(0.0, 1.0));
        self.scope_beam = r(1.5, 6.0);
        self.scope_gain = r(0.7, 1.6);
        self.intensity = r(0.7, 1.6);
        self.bloom_strength = r(0.4, 1.8);
        self.bloom_threshold = r(0.4, 1.2);
        self.exposure = r(0.8, 1.4);
        self.chromatic = r(0.0, 1.2);
        self.grain = r(0.0, 0.8);
        self.vignette = r(0.2, 0.8);
    }
}
