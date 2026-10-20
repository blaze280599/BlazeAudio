use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::scenes::{SceneId, SceneSettings};

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub enum WindowMode {
    /// Older builds also had an overlay mode; a file that starts in it opens windowed.
    #[serde(alias = "Overlay")]
    Windowed,
    Fullscreen,
}

/// When to move on to the next scene by itself.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub enum AutoScene {
    Off,
    /// On each new track.
    Track,
    /// Every `auto_scene_minutes`.
    Timer,
}

/// The windowed-mode window, in physical pixels: outer position and inner size.
#[derive(Clone, Copy, PartialEq, Serialize, Deserialize, Debug)]
pub struct WindowGeom {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    pub maximized: bool,
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
    pub visual: SceneId,
    pub auto_scene: AutoScene,
    pub auto_scene_minutes: f32,
    /// Each scene's own settings.
    pub scenes: SceneSettings,

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
    pub show_timeline: bool,
    pub show_stats: bool,
    pub idle_fps: u32,

    pub start_mode: WindowMode,
    pub sidebar_expanded: bool,
    pub theme: UiTheme,
    /// Where the window was when the app last closed.
    pub window: Option<WindowGeom>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            visual: SceneId::ALL[0],
            auto_scene: AutoScene::Off,
            auto_scene_minutes: 5.0,
            scenes: SceneSettings::default(),
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
            show_timeline: true,
            show_stats: false,
            idle_fps: 30,
            start_mode: WindowMode::Windowed,
            sidebar_expanded: true,
            theme: UiTheme::Dark,
            window: None,
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
            .and_then(|s| serde_json::from_str::<serde_json::Value>(s.trim_start_matches('\u{feff}')).ok())
            .and_then(|mut v| {
                migrate(&mut v);
                serde_json::from_value(v).ok()
            })
            .unwrap_or_default()
    }

    pub fn save(&self) {
        if let Some(p) = path() {
            if let Some(dir) = p.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            // Write a temporary file and swap it in, so a crash or full disk mid-write can't
            // leave a truncated settings file behind.
            if let Ok(s) = serde_json::to_string_pretty(self) {
                let tmp = p.with_extension("json.tmp");
                if std::fs::write(&tmp, s).is_ok() && std::fs::rename(&tmp, &p).is_err() {
                    let _ = std::fs::remove_file(&tmp);
                }
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
        self.scenes = d.scenes.clone();
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

        self.scenes.randomize(&mut r);
        self.intensity = r(0.7, 1.6);
        self.bloom_strength = r(0.4, 1.8);
        self.bloom_threshold = r(0.4, 1.2);
        self.exposure = r(0.8, 1.4);
        self.chromatic = r(0.0, 1.2);
        self.grain = r(0.0, 0.8);
        self.vignette = r(0.2, 0.8);
    }
}

/// Older builds kept scene settings at the top level with a scene prefix (`ascii_cell`,
/// `scope_gain`, ...). Move them into `scenes` so those files keep their values. New scenes
/// store under `scenes` from the start and never need an entry here.
fn migrate(v: &mut serde_json::Value) {
    let Some(root) = v.as_object_mut() else { return };
    for (prefix, scene) in [("ascii_", "ascii"), ("scope_", "scope")] {
        let keys: Vec<String> = root.keys().filter(|k| k.starts_with(prefix)).cloned().collect();
        if keys.is_empty() {
            continue;
        }
        let mut moved = serde_json::Map::new();
        for k in keys {
            if let Some(val) = root.remove(&k) {
                moved.insert(k[prefix.len()..].to_owned(), val);
            }
        }
        let scenes = root.entry("scenes").or_insert_with(|| serde_json::json!({}));
        if let Some(scenes) = scenes.as_object_mut() {
            scenes.entry(scene).or_insert(serde_json::Value::Object(moved));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrates_flat_scene_settings() {
        let old = r#"{"visual":"Scope","ascii_shape":"Rise","ascii_cell":24.0,"scope_mode":"Xy","scope_gain":1.5,"intensity":1.2}"#;
        let mut v: serde_json::Value = serde_json::from_str(old).unwrap();
        migrate(&mut v);
        let s: Settings = serde_json::from_value(v).unwrap();
        assert_eq!(s.visual, SceneId::Scope);
        assert_eq!(s.scenes.ascii.cell, 24.0);
        assert_eq!(s.scenes.ascii.density, SceneSettings::default().ascii.density);
        assert_eq!(s.scenes.scope.gain, 1.5);
        assert_eq!(s.intensity, 1.2);
    }

    #[test]
    fn overlay_start_mode_opens_windowed() {
        let old = r#"{"start_mode":"Overlay","overlay_opacity":0.5,"overlay_click_through":false,"intensity":1.2}"#;
        let mut v: serde_json::Value = serde_json::from_str(old).unwrap();
        migrate(&mut v);
        let s: Settings = serde_json::from_value(v).unwrap();
        assert_eq!(s.start_mode, WindowMode::Windowed);
        assert_eq!(s.intensity, 1.2);
    }

    #[test]
    fn round_trips() {
        let mut s = Settings::default();
        s.scenes.scope.beam = 7.0;
        let mut v = serde_json::to_value(&s).unwrap();
        migrate(&mut v);
        let back: Settings = serde_json::from_value(v).unwrap();
        assert_eq!(back.scenes.scope.beam, 7.0);
    }
}
