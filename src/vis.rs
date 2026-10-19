//! Render-side visual state: turns analysis frames into smoothed, frame-rate independent
//! animation parameters (peaks, shockwaves, camera, palette blending, scene sequencing).

use std::time::Instant;

use crate::audio::{AudioFrame, NUM_BANDS, WAVE_LEN};
use crate::gpu::{Globals, AUDIO_FLOATS, SPEC_COLS};
use crate::palette::Palette;
use crate::settings::Settings;

pub struct VisState {
    pub bands: [f32; NUM_BANDS],
    peaks: [f32; NUM_BANDS],
    peak_vel: [f32; NUM_BANDS],
    wave: [f32; WAVE_LEN],
    pub bass: f32,
    pub mid: f32,
    pub treble: f32,
    pub energy: f32,
    pub kick: f32,
    pub hat: f32,
    flux: f32,
    pub bass_slow: f32,
    beat_strength: f32,
    pub bpm: f32,
    last_beat_count: u32,
    last_drop_count: u32,
    ring_age: [f32; 4],
    ring_str: [f32; 4],
    ring_next: usize,
    travel: f32,
    cam_angle: f32,
    art_spin: f32,
    shape_m: f32,
    shape_target: f32,
    beats_since_shape: u32,
    shape_timer: f32,
    spec_acc: f32,
    pub spec_total: u32,
    pal_cur: Palette,
    pub pal_target: Palette,
    pub silent_for: f32,
    pub beat_this_frame: bool,
    pub drop_this_frame: bool,
    first: bool,
}

impl VisState {
    pub fn new(pal: Palette) -> Self {
        Self {
            bands: [0.0; NUM_BANDS],
            peaks: [0.0; NUM_BANDS],
            peak_vel: [0.0; NUM_BANDS],
            wave: [0.0; WAVE_LEN],
            bass: 0.0,
            mid: 0.0,
            treble: 0.0,
            energy: 0.0,
            kick: 0.0,
            hat: 0.0,
            flux: 0.0,
            bass_slow: 0.0,
            beat_strength: 0.0,
            bpm: 0.0,
            last_beat_count: 0,
            last_drop_count: 0,
            ring_age: [10.0; 4],
            ring_str: [0.0; 4],
            ring_next: 0,
            travel: 0.0,
            cam_angle: 0.6,
            art_spin: 0.0,
            shape_m: 0.0,
            shape_target: 0.0,
            beats_since_shape: 0,
            shape_timer: 0.0,
            spec_acc: 0.0,
            spec_total: 0,
            pal_cur: pal,
            pal_target: pal,
            silent_for: 0.0,
            beat_this_frame: false,
            drop_this_frame: false,
            first: true,
        }
    }

    /// Returns spectrogram rows to upload this frame as (row_index, data).
    pub fn update(&mut self, f: &AudioFrame, dt: f32, now: Instant) -> Vec<(u32, [u8; SPEC_COLS as usize])> {
        let stale = f.updated.map_or(true, |t| now.duration_since(t).as_secs_f32() > 0.15);
        let k = |tau: f32| 1.0 - (-dt / tau).exp();

        // Light display smoothing on top of the analyzer's attack/decay; when the capture
        // goes quiet (WASAPI stops sending packets) decay everything to rest.
        let ks = k(0.012);
        let kz = k(0.25);
        for i in 0..NUM_BANDS {
            let target = if stale { 0.0 } else { f.bands[i] };
            let kk = if stale { kz } else { ks };
            self.bands[i] += (target - self.bands[i]) * kk;
            let b = self.bands[i];
            if b >= self.peaks[i] {
                self.peaks[i] = b;
                self.peak_vel[i] = 0.0;
            } else {
                self.peak_vel[i] += dt * 1.4;
                self.peaks[i] = (self.peaks[i] - self.peak_vel[i] * dt).max(b);
            }
        }
        let kw = if stale { kz } else { k(0.008) };
        for i in 0..WAVE_LEN {
            let t = if stale { 0.0 } else { f.wave[i] };
            self.wave[i] += (t - self.wave[i]) * kw;
        }
        let sel = |v: f32| if stale { 0.0 } else { v };
        let ka = if stale { kz } else { ks };
        self.bass += (sel(f.bass) - self.bass) * ka;
        self.mid += (sel(f.mid) - self.mid) * ka;
        self.treble += (sel(f.treble) - self.treble) * ka;
        self.energy += (sel(f.energy) - self.energy) * ka;
        self.flux += (sel(f.flux) - self.flux) * ka;
        self.bass_slow += (self.bass - self.bass_slow) * k(0.35);
        self.hat = self.hat.max(sel(f.hat)) * (-dt / 0.08).exp();
        self.bpm = f.bpm;

        self.beat_this_frame = false;
        if self.first {
            self.last_beat_count = f.beat_count;
            self.last_drop_count = f.drop_count;
            self.first = false;
        }
        if f.beat_count != self.last_beat_count {
            self.last_beat_count = f.beat_count;
            if !stale {
                self.beat_this_frame = true;
                self.beat_strength = f.beat_strength;
                self.kick = self.kick.max(f.beat_strength.max(0.45));
                self.ring_age[self.ring_next] = 0.0;
                self.ring_str[self.ring_next] = f.beat_strength;
                self.ring_next = (self.ring_next + 1) % 4;
                self.beats_since_shape += 1;
            }
        }
        self.drop_this_frame = f.drop_count != self.last_drop_count;
        self.last_drop_count = f.drop_count;
        self.kick *= (-dt / 0.13).exp();
        for a in &mut self.ring_age {
            *a += dt;
        }

        // Motion integrators.
        self.travel += dt * (0.35 + self.energy * 2.2 + self.kick * 1.6);
        self.cam_angle += dt * (0.07 + self.mid * 0.35 + self.kick * 0.3);
        self.art_spin += dt * (0.05 + self.energy * 0.25);

        // Particle shape sequencing: advance every 16 beats (or 14 s without beats).
        self.shape_timer += dt;
        if (self.beats_since_shape >= 16 && self.shape_timer > 7.0) || self.shape_timer > 14.0 {
            self.shape_target += 1.0;
            self.beats_since_shape = 0;
            self.shape_timer = 0.0;
        }
        self.shape_m += (self.shape_target - self.shape_m) * k(0.9);
        if self.shape_target > 400.0 {
            self.shape_target -= 400.0;
            self.shape_m -= 400.0;
        }

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

        // Spectrogram history at a fixed 60 rows/s.
        let mut rows = Vec::new();
        self.spec_acc += dt * 60.0;
        let mut n = 0;
        while self.spec_acc >= 1.0 {
            self.spec_acc -= 1.0;
            n += 1;
            if n > 8 {
                self.spec_acc = 0.0;
                break;
            }
            let mut row = [0u8; SPEC_COLS as usize];
            for i in 0..NUM_BANDS {
                row[i] = (self.bands[i].clamp(0.0, 1.0) * 255.0) as u8;
            }
            rows.push((self.spec_total, row));
            self.spec_total = self.spec_total.wrapping_add(1);
        }
        rows
    }

    pub fn audio_buffer(&self) -> [f32; AUDIO_FLOATS] {
        let mut a = [0.0f32; AUDIO_FLOATS];
        a[..64].copy_from_slice(&self.bands);
        a[64..128].copy_from_slice(&self.peaks);
        a[128..].copy_from_slice(&self.wave);
        a
    }

    #[allow(clippy::too_many_arguments)]
    pub fn globals(
        &self,
        s: &Settings,
        res: (u32, u32),
        time: f32,
        dt: f32,
        scene_time: f32,
        transition: f32,
        overlay: bool,
        art_present: f32,
        particle_count: u32,
    ) -> Globals {
        let (w, h) = (res.0 as f32, res.1 as f32);
        let mut pal = [[0.0f32; 4]; 4];
        for i in 0..4 {
            pal[i] = [self.pal_cur[i][0], self.pal_cur[i][1], self.pal_cur[i][2], 1.0];
        }
        // Integer ring row for the texture; a separate continuous value for smooth world scroll.
        let spec_pos = (self.spec_total % 256) as f32 + 256.0;
        let scroll = (self.spec_total % 60_000) as f32 + self.spec_acc.min(0.999);
        Globals {
            res: [w, h],
            inv_res: [1.0 / w, 1.0 / h],
            time,
            dt,
            bass: self.bass,
            mid: self.mid,
            treble: self.treble,
            energy: self.energy,
            beat: self.kick,
            beat_count: self.last_beat_count as f32,
            flux: self.flux,
            intensity: s.intensity,
            scene_time,
            hat: self.hat,
            beat_strength: self.beat_strength,
            overlay: if overlay { 1.0 } else { 0.0 },
            trails: s.trails,
            spec_pos,
            bloom_threshold: s.bloom_threshold,
            bloom_knee: 0.5,
            bloom_strength: s.bloom_strength,
            exposure: s.exposure,
            chromatic: s.chromatic,
            grain: s.grain,
            vignette: if overlay { 0.0 } else { s.vignette },
            transition: smooth(transition),
            bass_slow: self.bass_slow,
            particle_count: particle_count as f32,
            beat_trigger: if self.beat_this_frame { 1.0 } else { 0.0 },
            shape_morph: self.shape_m,
            cam_angle: self.cam_angle,
            art_present,
            kick: self.kick,
            travel: self.travel,
            overlay_opacity: s.overlay_opacity,
            art_spin: self.art_spin,
            scroll,
            _pad0: 0.0,
            pal,
            ring_age: self.ring_age,
            ring_str: self.ring_str,
        }
    }
}

fn smooth(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Scene sequencing with crossfades between two ping-pong HDR slots.
pub struct SceneMgr {
    pub cur: usize,
    pub cur_slot: usize,
    pub prev: Option<usize>,
    pub t: f32,
    pub scene_time: f32,
}

impl SceneMgr {
    pub fn new(scene: usize) -> Self {
        Self { cur: scene % 4, cur_slot: 0, prev: None, t: 1.0, scene_time: 0.0 }
    }

    pub fn request(&mut self, next: usize) {
        let next = next % 4;
        if next == self.cur {
            return;
        }
        self.prev = Some(self.cur);
        self.cur = next;
        self.cur_slot ^= 1;
        self.t = 0.0;
        self.scene_time = 0.0;
    }

    pub fn update(&mut self, dt: f32, transition_seconds: f32) {
        self.scene_time += dt;
        if self.prev.is_some() {
            self.t += dt / transition_seconds.max(0.1);
            if self.t >= 1.0 {
                self.t = 1.0;
                self.prev = None;
            }
        }
    }
}
