use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub enum WindowMode {
    Windowed,
    Fullscreen,
    Overlay,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub scene: usize,
    pub auto_cycle: bool,
    pub cycle_seconds: f32,
    pub transition_seconds: f32,
    pub switch_on_drop: bool,

    pub intensity: f32,
    pub bloom_strength: f32,
    pub bloom_threshold: f32,
    pub exposure: f32,
    pub chromatic: f32,
    pub grain: f32,
    pub vignette: f32,
    pub trails: f32,
    pub particle_fraction: f32,
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
    pub overlay_opacity: f32,
    pub overlay_click_through: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            scene: 0,
            auto_cycle: true,
            cycle_seconds: 40.0,
            transition_seconds: 2.4,
            switch_on_drop: true,
            intensity: 1.0,
            bloom_strength: 0.9,
            bloom_threshold: 0.75,
            exposure: 1.0,
            chromatic: 0.6,
            grain: 0.35,
            vignette: 0.55,
            trails: 0.82,
            particle_fraction: 1.0,
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
            .and_then(|s| serde_json::from_str(&s).ok())
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
