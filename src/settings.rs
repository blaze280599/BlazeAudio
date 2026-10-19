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
}
