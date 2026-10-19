//! Render-side visual state: turns analysis frames into smoothed, frame-rate independent
//! animation parameters (band levels, kick envelope, audio clock, palette blending).

use std::time::Instant;

use crate::audio::{AudioFrame, NUM_BANDS};
use crate::gpu::{Globals, ASCII_CHARS, AUDIO_FLOATS};
use crate::palette::{linear_to_oklab, Palette};
use crate::settings::{AsciiShape, Settings};

pub struct VisState {
    pub bands: [f32; NUM_BANDS],
    pub treble: f32,
    pub energy: f32,
    pub kick: f32,
    pub hat: f32,
    pub bpm: f32,
    last_beat_count: u32,
    /// Seconds since the last beat.
    beat_age: f32,
    /// Seconds since the current scene came on screen (drives the Gradient ASCII intro).
    pub scene_time: f32,
    flow: f32,
    pal_cur: Palette,
    pub pal_target: Palette,
    pub silent_for: f32,
    first: bool,
}

impl VisState {
    pub fn new(pal: Palette) -> Self {
        Self {
            bands: [0.0; NUM_BANDS],
            treble: 0.0,
            energy: 0.0,
            kick: 0.0,
            hat: 0.0,
            bpm: 0.0,
            last_beat_count: 0,
            beat_age: 10.0,
            scene_time: 0.0,
            flow: 0.0,
            pal_cur: pal,
            pal_target: pal,
            silent_for: 0.0,
            first: true,
        }
    }

    pub fn update(&mut self, f: &AudioFrame, dt: f32, now: Instant) {
        let stale = f.updated.map_or(true, |t| now.duration_since(t).as_secs_f32() > 0.15);
        let k = |tau: f32| 1.0 - (-dt / tau).exp();

        // Light display smoothing on top of the analyzer's attack/decay; when the capture
        // goes quiet (WASAPI stops sending packets) decay everything to rest.
        let ks = k(0.012);
        let kz = k(0.25);
        let ka = if stale { kz } else { ks };
        for i in 0..NUM_BANDS {
            let target = if stale { 0.0 } else { f.bands[i] };
            self.bands[i] += (target - self.bands[i]) * ka;
        }
        let sel = |v: f32| if stale { 0.0 } else { v };
        self.treble += (sel(f.treble) - self.treble) * ka;
        self.energy += (sel(f.energy) - self.energy) * ka;
        self.hat = self.hat.max(sel(f.hat)) * (-dt / 0.08).exp();
        self.bpm = f.bpm;

        if self.first {
            self.last_beat_count = f.beat_count;
            self.first = false;
        }
        if f.beat_count != self.last_beat_count {
            self.last_beat_count = f.beat_count;
            if !stale {
                self.kick = self.kick.max(f.beat_strength.max(0.45));
                self.beat_age = 0.0;
            }
        }
        self.beat_age = (self.beat_age + dt).min(10.0);
        self.scene_time = (self.scene_time + dt).min(1e4);
        self.kick *= (-dt / 0.13).exp();

        // Audio clock for the scene: idles slowly, surges with the music.
        self.flow += dt * (0.35 + self.energy * 1.1 + self.kick * 0.9);

        // Palette crossfade.
        let kp = k(0.9);
        for i in 0..4 {
            for j in 0..3 {
                self.pal_cur[i][j] += (self.pal_target[i][j] - self.pal_cur[i][j]) * kp;
            }
        }

        if stale || f.level < 1e-4 {
            self.silent_for += dt;
        } else {
            self.silent_for = 0.0;
        }
    }

    pub fn audio_buffer(&self) -> [f32; AUDIO_FLOATS] {
        self.bands
    }

    pub fn globals(&self, s: &Settings, res: (u32, u32), time: f32, overlay: bool) -> Globals {
        let (w, h) = (res.0 as f32, res.1 as f32);
        let mut pal = [[0.0f32; 4]; 4];
        for i in 0..4 {
            pal[i] = [self.pal_cur[i][0], self.pal_cur[i][1], self.pal_cur[i][2], 1.0];
        }
        // Gradient ASCII ramp: the same colours in Oklab, from the edge of the shadow (darkest)
        // to the brightest point.
        let mut ramp = self.pal_cur;
        ramp.sort_by(|a, b| crate::palette::luminance(*a).total_cmp(&crate::palette::luminance(*b)));
        let mut pal_lab = [[0.0f32; 4]; 4];
        for i in 0..4 {
            let l = linear_to_oklab(ramp[i]);
            pal_lab[i] = [l[0], l[1], l[2], 0.0];
        }
        let shape = match s.ascii_shape {
            AsciiShape::Horizon => 0.0,
            AsciiShape::Pinch => 1.0,
            AsciiShape::Rise => 2.0,
            AsciiShape::Eclipse => 3.0,
        };
        Globals {
            res: [w, h],
            inv_res: [1.0 / w, 1.0 / h],
            time,
            treble: self.treble,
            hat: self.hat,
            kick: self.kick,
            beat_count: self.last_beat_count as f32,
            flow: self.flow,
            intensity: s.intensity,
            overlay: if overlay { 1.0 } else { 0.0 },
            overlay_opacity: s.overlay_opacity,
            bloom_threshold: s.bloom_threshold,
            bloom_knee: 0.5,
            bloom_strength: s.bloom_strength,
            exposure: s.exposure,
            chromatic: s.chromatic,
            grain: s.grain,
            vignette: if overlay { 0.0 } else { s.vignette },
            energy: self.energy,
            beat_age: self.beat_age,
            scene_time: self.scene_time,
            a_shape: shape,
            a_cell: s.ascii_cell,
            a_density: s.ascii_density,
            a_glyphs: s.ascii_glyphs,
            a_gradient: s.ascii_gradient,
            a_count: ASCII_CHARS.chars().count() as f32,
            _pad: [0.0; 3],
            pal,
            pal_lab,
        }
    }
}
