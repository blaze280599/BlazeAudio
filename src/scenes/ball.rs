//! Crystalized Ball: a full-screen pass draws the electric rim, its glow and spark arcs, then up to
//! 40 000 dust particles are added on top as instanced quads, placed in the vertex shader from a
//! fixed table of home positions. After the React Bits component, driven by the music instead of
//! the cursor.

use std::f32::consts::{PI, TAU};

use bytemuck::{Pod, Zeroable};
use serde::{Deserialize, Serialize};

use super::{Params, Scene, SceneConfig, SceneFrame, SceneSettings};
use crate::gpu::{self, GpuCtx, HDR_FORMAT};
use crate::palette::{linear_to_oklab, linear_to_srgb, oklab_to_linear, srgb_to_linear, Rgb};
use crate::ui::widgets::{choice, knob, num, PCT};
use crate::ui::Tokens;
use crate::vis::VisState;

const MAX_STRANDS: usize = 8;
const MAX_ARCS: usize = 4;
pub const MAX_PARTICLES: u32 = 40_000;
/// Rim power-up and dust reveal after the scene comes on screen.
const INTRO_SECONDS: f32 = 2.2;
/// Dust brightness against the rim, for the linear HDR pipeline.
const DUST_GAIN: f32 = 1.0;

/// Which album palette colour the ball is made of.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub enum Hue {
    Primary,
    Secondary,
    Accent,
}

/// How the dust moves (order matches the shader).
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub enum Motion {
    Rise,
    Fall,
    Drift,
    Orbit,
}

/// Dust particle shape (order matches the shader).
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub enum Grain {
    Square,
    Round,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub hue: Hue,
    /// Diameter as a fraction of the shorter side.
    pub size: f32,
    pub strands: u32,
    pub crackle: f32,
    pub flares: f32,
    pub glow: f32,
    pub sparks: f32,
    /// How strongly beats surge the rim and the energy heats it.
    pub reactivity: f32,
    /// How much the spectrum shapes the flares around the rim.
    pub spectrum: f32,
    pub speed: f32,
    pub particles: u32,
    pub fill: f32,
    pub motion: Motion,
    pub grain: Grain,
    pub depth: f32,
    pub sway: f32,
    pub twinkle: f32,
    pub haze: f32,
    pub dust_speed: f32,
    /// How hard beats toss the dust.
    pub shake: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            hue: Hue::Primary,
            size: 0.7,
            strands: 6,
            crackle: 0.85,
            flares: 0.65,
            glow: 0.9,
            sparks: 0.6,
            reactivity: 0.7,
            spectrum: 0.6,
            speed: 1.0,
            particles: 15_000,
            fill: 0.5,
            motion: Motion::Rise,
            grain: Grain::Square,
            depth: 0.6,
            sway: 0.5,
            twinkle: 0.5,
            haze: 0.7,
            dust_speed: 1.0,
            shake: 0.5,
        }
    }
}

impl SceneConfig for Settings {
    fn ui(&mut self, ui: &mut egui::Ui, t: &Tokens) {
        let d = Self::default();
        choice(ui, t, "Colour", &mut self.hue, d.hue, &[(Hue::Primary, "Primary"), (Hue::Secondary, "Secondary"), (Hue::Accent, "Accent")]);
        knob(ui, t, "Size", &mut self.size, d.size, 0.2..=1.2, PCT);
        knob(ui, t, "Strands", &mut self.strands, d.strands, 1..=MAX_STRANDS as u32, num(0, ""));
        knob(ui, t, "Crackle", &mut self.crackle, d.crackle, 0.0..=1.0, PCT);
        knob(ui, t, "Flares", &mut self.flares, d.flares, 0.0..=1.0, PCT);
        knob(ui, t, "Glow", &mut self.glow, d.glow, 0.0..=2.0, PCT);
        knob(ui, t, "Sparks", &mut self.sparks, d.sparks, 0.0..=1.0, PCT);
        knob(ui, t, "Reactivity", &mut self.reactivity, d.reactivity, 0.0..=1.0, PCT);
        knob(ui, t, "Spectrum", &mut self.spectrum, d.spectrum, 0.0..=1.0, PCT);
        knob(ui, t, "Rim speed", &mut self.speed, d.speed, 0.1..=3.0, num(2, "×"));
        knob(ui, t, "Particles", &mut self.particles, d.particles, 0..=MAX_PARTICLES, num(0, ""));
        knob(ui, t, "Fill", &mut self.fill, d.fill, 0.0..=1.0, PCT);
        let motions = [(Motion::Rise, "Rise"), (Motion::Fall, "Fall"), (Motion::Drift, "Drift"), (Motion::Orbit, "Orbit")];
        choice(ui, t, "Motion", &mut self.motion, d.motion, &motions);
        choice(ui, t, "Particle shape", &mut self.grain, d.grain, &[(Grain::Square, "Square"), (Grain::Round, "Round")]);
        knob(ui, t, "Depth", &mut self.depth, d.depth, 0.0..=1.0, PCT);
        knob(ui, t, "Sway", &mut self.sway, d.sway, 0.0..=1.0, PCT);
        knob(ui, t, "Twinkle", &mut self.twinkle, d.twinkle, 0.0..=1.0, PCT);
        knob(ui, t, "Haze", &mut self.haze, d.haze, 0.0..=2.0, PCT);
        knob(ui, t, "Dust speed", &mut self.dust_speed, d.dust_speed, 0.0..=3.0, num(2, "×"));
        knob(ui, t, "Shake", &mut self.shake, d.shake, 0.0..=1.0, PCT);
    }

    fn randomize(&mut self, r: &mut dyn FnMut(f32, f32) -> f32) {
        self.hue = [Hue::Primary, Hue::Secondary, Hue::Accent][(r(0.0, 3.0) as usize).min(2)];
        self.strands = r(3.0, 8.99) as u32;
        self.crackle = r(0.3, 1.0);
        self.flares = r(0.3, 1.0);
        self.glow = r(0.6, 1.1);
        self.sparks = r(0.2, 0.8);
        self.fill = r(0.35, 0.8);
        self.motion = [Motion::Rise, Motion::Fall, Motion::Drift, Motion::Orbit][(r(0.0, 4.0) as usize).min(3)];
        self.grain = if r(0.0, 1.0) < 0.5 { Grain::Square } else { Grain::Round };
        self.depth = r(0.5, 0.8);
        self.sway = r(0.3, 0.8);
        self.twinkle = r(0.4, 0.7);
        self.haze = r(0.5, 0.9);
        self.dust_speed = r(0.7, 1.6);
    }
}

/// Must match `Params` in ball.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct BallParams {
    size: f32,
    time: f32,
    strands: f32,
    crackle: f32,
    flares: f32,
    glow: f32,
    haze: f32,
    fill: f32,
    presence: f32,
    unfold: f32,
    inside: f32,
    reveal: f32,
    heat_angle: f32,
    heat: f32,
    dust_time: f32,
    motion: f32,
    turn: [f32; 2],
    depth: f32,
    twinkle: f32,
    shape: f32,
    dust_gain: f32,
    spectrum: f32,
    _pad: f32,
    arcs: [[f32; 4]; MAX_ARCS],
    rim: [f32; 4],
    rim_hot: [f32; 4],
    spark_glow: [f32; 4],
    edge: [f32; 4],
    tone_a: [f32; 4],
    tone_b: [f32; 4],
}

/// Must match `Dust` in ball.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Dust {
    /// Position in the unit ball, and the colour tone (0..4).
    home: [f32; 4],
    /// Size, cycle phase, cycle length in seconds, travel per cycle.
    seed: [f32; 4],
}

/// A spark arc along the inside of the rim.
struct Arc {
    start: f32,
    span: f32,
    curl: f32,
    life: f32,
    duration: f32,
    drift: f32,
    strength: f32,
}

/// One beat's toss of the dust.
#[derive(Clone, Copy, Default)]
struct Shake {
    age: f32,
    amount: f32,
    seed: f32,
}

pub struct CrystalBall {
    field: wgpu::RenderPipeline,
    dust: wgpu::RenderPipeline,
    /// Owns the buffer behind `bg_dust`.
    _dust_buf: wgpu::Buffer,
    bg_dust: wgpu::BindGroup,
    dust_len: u32,
    particles: u32,
    rim_time: f32,
    dust_time: f32,
    sway_clock: f32,
    heat_clock: f32,
    next_arc: f32,
    arcs: Vec<Arc>,
    rng: u32,
    last_beat: Option<u32>,
    /// Newest first.
    shakes: [Shake; 2],
}

impl CrystalBall {
    pub fn new(gpu: &GpuCtx) -> Self {
        let src = format!("{}\n{}", strand_tables(), include_str!("../shaders/ball.wgsl"));
        let m = gpu.module("ball", &src);
        let field = gpu.fullscreen(&gpu.layout(false), &m, "fs_ball", HDR_FORMAT, None);

        // The vertex shader reads the dust table, so it gets its own group 1.
        let bgl = gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ball dust"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None },
                count: None,
            }],
        });
        let layout = gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&gpu.layouts.g0, &bgl],
            push_constant_ranges: &[],
        });
        let add = wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add };
        let keep = wgpu::BlendComponent { src_factor: wgpu::BlendFactor::Zero, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add };
        let dust = gpu.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("vs_dust"),
            layout: Some(&layout),
            vertex: wgpu::VertexState { module: &m, entry_point: Some("vs_dust"), compilation_options: Default::default(), buffers: &[] },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &m,
                entry_point: Some("fs_dust"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: HDR_FORMAT,
                    blend: Some(wgpu::BlendState { color: add, alpha: keep }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview: None,
            cache: None,
        });

        let table = build_dust(MAX_PARTICLES as usize);
        let buf = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ball dust"),
            size: (table.len() * std::mem::size_of::<Dust>()) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        gpu.queue.write_buffer(&buf, 0, bytemuck::cast_slice(&table));
        let bg_dust = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &bgl,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: buf.as_entire_binding() }],
        });

        Self {
            field,
            dust,
            _dust_buf: buf,
            bg_dust,
            dust_len: table.len() as u32,
            particles: 0,
            rim_time: 0.0,
            dust_time: 0.0,
            sway_clock: 0.0,
            heat_clock: 0.0,
            next_arc: 0.3,
            arcs: Vec::with_capacity(MAX_ARCS),
            rng: 913,
            last_beat: None,
            shakes: [Shake::default(); 2],
        }
    }

    fn random(&mut self) -> f32 {
        mulberry32(&mut self.rng) as f32
    }

    fn spawn_arc(&mut self, angle: f32, strength: f32) {
        if self.arcs.len() >= MAX_ARCS {
            return;
        }
        let span = (0.05 + self.random() * 0.07) * if self.random() < 0.5 { -1.0 } else { 1.0 };
        let curl = 0.02 + self.random() * 0.04;
        let duration = 0.2 + self.random() * 0.2;
        let drift = (self.random() - 0.5) * 0.3;
        self.arcs.push(Arc { start: angle, span, curl, life: 0.0, duration, drift, strength });
    }
}

impl Scene for CrystalBall {
    fn params(&mut self, vis: &VisState, settings: &SceneSettings, p: &mut Params) {
        let s = &settings.ball;
        let dt = vis.dt;
        let intro = (vis.scene_time / INTRO_SECONDS).min(1.0);
        let presence = ease_out(smooth(0.0, 0.42, intro));
        let unfold = ease_out(smooth(0.0, 0.32, intro));
        let charge = (PI * smooth(0.22, 0.78, intro)).sin() * 0.35;

        // The music stands in for the cursor: beats are the click surge, the energy is the
        // hover heat, wandering around the rim.
        let react = s.reactivity.clamp(0.0, 1.0);
        let surge = vis.kick * react;
        let heat = react * (0.3 + 0.7 * vis.energy.min(1.0)) * presence;
        let beats = vis.beats();
        let beat = self.last_beat.is_some_and(|b| b != beats);
        self.last_beat = Some(beats);

        let pace = 0.7 + 0.5 * vis.energy.min(1.5) + surge * 0.6;
        let step = dt * s.speed.max(0.0) * 2.0 * pace;
        self.rim_time += step;
        self.dust_time += dt * s.dust_speed.max(0.0) * (0.7 + 0.6 * vis.energy.min(1.5) + 0.4 * surge);
        self.sway_clock += dt * s.dust_speed.max(0.0);
        self.heat_clock += dt * (0.25 + 0.5 * vis.energy.min(1.5));
        let heat_angle = PI / 2.0 + self.heat_clock * 0.35 + 0.8 * (self.heat_clock * 0.9).sin();

        // Sparks: on their own timer, more often while it crackles, and on beats near the hot spot.
        let crackle_now = (s.crackle + surge * 0.5 + vis.hat * 0.4).clamp(0.0, 1.5);
        if s.sparks > 0.01 && intro > 0.55 {
            self.next_arc -= step * s.sparks * 2.0 * (0.6 + crackle_now * 0.6);
            if self.next_arc <= 0.0 {
                let angle = if self.random() < heat * 0.7 {
                    heat_angle + (self.random() - 0.5) * 0.8
                } else if self.random() < 0.7 {
                    PI / 2.0 + (self.random() - 0.5) * 1.5
                } else {
                    self.random() * TAU
                };
                self.spawn_arc(angle, 0.8 + heat * 0.6 + surge * 0.4);
                self.next_arc = 0.15 + self.random() * 0.6;
            }
            if beat && self.random() < s.sparks + 0.2 {
                let angle = heat_angle + (self.random() - 0.5) * 0.6;
                self.spawn_arc(angle, 1.0 + surge * 0.4);
            }
        }
        for a in &mut self.arcs {
            a.life += step;
            a.start += a.drift * step;
        }
        self.arcs.retain(|a| a.life < a.duration);
        let mut arcs = [[0.0f32; 4]; MAX_ARCS];
        for (slot, a) in arcs.iter_mut().zip(&self.arcs) {
            *slot = [a.start, a.span, a.curl, (PI * a.life / a.duration).sin() * a.strength];
        }

        if beat {
            self.shakes[1] = self.shakes[0];
            self.shakes[0] = Shake { age: 0.0, amount: s.shake * vis.kick * 0.12, seed: (beats % 4096) as f32 };
        }
        for sh in &mut self.shakes {
            sh.age = (sh.age + dt).min(10.0);
        }
        self.particles = s.particles.min(self.dust_len);

        let c = ball_colours(vis.palette()[s.hue as usize]);
        let [sh0, sh1] = self.shakes;
        let swing = s.sway.clamp(0.0, 1.0);
        p.set(&BallParams {
            size: s.size,
            time: self.rim_time,
            strands: s.strands.clamp(1, MAX_STRANDS as u32) as f32,
            crackle: crackle_now + charge,
            flares: s.flares,
            glow: s.glow * (1.0 + surge * 0.5 + charge),
            haze: s.haze,
            fill: s.fill,
            presence,
            unfold,
            inside: smooth(0.2, 0.95, intro),
            reveal: smooth(0.18, 1.0, intro),
            heat_angle,
            heat,
            dust_time: self.dust_time,
            motion: s.motion as u32 as f32,
            turn: [swing * 0.42 * (self.sway_clock * 0.23).sin(), swing * 0.12 * (self.sway_clock * 0.17 + 1.3).sin()],
            depth: s.depth,
            twinkle: s.twinkle,
            shape: s.grain as u32 as f32,
            dust_gain: DUST_GAIN,
            spectrum: s.spectrum,
            _pad: 0.0,
            arcs,
            rim: with_w(c.rim, sh0.age),
            rim_hot: with_w(c.rim_hot, sh0.amount),
            spark_glow: with_w(c.spark_glow, sh1.age),
            edge: with_w(c.edge, sh1.amount),
            tone_a: with_w(c.tone_a, sh0.seed),
            tone_b: with_w(c.tone_b, sh1.seed),
        });
    }

    fn encode(&mut self, enc: &mut wgpu::CommandEncoder, f: &SceneFrame) {
        gpu::fullscreen_pass(enc, f.target, wgpu::LoadOp::Clear(wgpu::Color::BLACK), &self.field, f.g0, None);
        if self.particles == 0 {
            return;
        }
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("ball dust"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: f.target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_pipeline(&self.dust);
        pass.set_bind_group(0, f.g0, &[]);
        pass.set_bind_group(1, &self.bg_dust, &[]);
        pass.draw(0..6, 0..self.particles);
    }
}

/// The ball's colours, all derived from one (linear RGB).
struct Colours {
    rim: Rgb,
    rim_hot: Rgb,
    spark_glow: Rgb,
    edge: Rgb,
    /// Dust tones turned 16° either way around the hue.
    tone_a: Rgb,
    tone_b: Rgb,
}

/// Brightens `base` to full emission, then derives the rest the way the original does: white-hot
/// mixes in sRGB, the darker and hue-turned shades in Oklab.
fn ball_colours(base: Rgb) -> Colours {
    let mx = base[0].max(base[1]).max(base[2]).max(1e-3);
    let rim = base.map(|v| (v / mx).clamp(0.0, 1.0));
    let to_white = |t: f32| rim.map(|v| srgb_to_linear(linear_to_srgb(v) + (1.0 - linear_to_srgb(v)) * t));
    let shift = |hue_deg: f32, dl: f32, chroma: f32| {
        let [l, a, b] = linear_to_oklab(rim);
        let c = a.hypot(b) * chroma;
        let h = b.atan2(a) + hue_deg.to_radians();
        oklab_to_linear([(l + dl).clamp(0.0, 1.0), c * h.cos(), c * h.sin()]).map(|v| v.min(1.0))
    };
    Colours {
        rim,
        rim_hot: to_white(0.72),
        spark_glow: shift(0.0, -0.15, 1.0),
        edge: shift(0.0, -0.3, 0.9),
        tone_a: shift(16.0, -0.02, 1.0),
        tone_b: shift(-16.0, -0.12, 1.05),
    }
}

fn with_w(c: Rgb, w: f32) -> [f32; 4] {
    [c[0], c[1], c[2], w]
}

fn smooth(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t).powi(3)
}

/// The original's seeded generator (mulberry32), so the strands and dust match it.
fn mulberry32(state: &mut u32) -> f64 {
    *state = state.wrapping_add(0x6d2b_79f5);
    let mut t = *state;
    t = (t ^ (t >> 15)).wrapping_mul(t | 1);
    t ^= t.wrapping_add((t ^ (t >> 7)).wrapping_mul(t | 61));
    (t ^ (t >> 14)) as f64 / 4_294_967_296.0
}

/// The rim's strand tables as WGSL: per strand, four wave harmonics with their rates and phases,
/// and three flare harmonics plus (in w) the strand's amplitude, width and brightness.
fn strand_tables() -> String {
    let mut st = 99u32;
    let mut r = || mulberry32(&mut st);
    let bands = [(6.0, 12.0, 0.6, 1.6), (18.0, 34.0, 1.5, 3.5), (40.0, 70.0, 3.0, 7.0), (80.0, 130.0, 6.0, 12.0)];
    // Flat, four per strand, in the order the original pushes them.
    let mut t: [Vec<f64>; 6] = Default::default();
    for s in 0..MAX_STRANDS {
        for (low, high, slow, fast) in bands {
            t[0].push((low + r() * (high - low)).round());
            let rate = slow + r() * (fast - slow);
            t[1].push(rate * if r() < 0.5 { -1.0 } else { 1.0 });
            t[2].push(r() * std::f64::consts::TAU);
        }
        for _ in 0..3 {
            t[3].push(4.0 + (r() * 6.0).floor());
            let rate = 0.4 + r() * 0.8;
            t[4].push(rate * if r() < 0.5 { -1.0 } else { 1.0 });
            t[5].push(r() * std::f64::consts::TAU);
        }
        let lead = s == 0;
        t[3].push(if lead { 0.75 } else { 0.95 + r() * 0.3 });
        t[4].push(if lead { 1.15 } else { 0.8 + r() * 0.2 });
        t[5].push(if lead { 1.0 } else { 0.7 + r() * 0.25 });
    }
    let mut out = String::new();
    for (name, flat) in ["HARM", "RATE", "PHASE", "FHARM", "FRATE", "FPHASE"].iter().zip(&t) {
        let items: Vec<String> = flat.chunks(4).map(|v| format!("vec4f({:.6}, {:.6}, {:.6}, {:.6})", v[0], v[1], v[2], v[3])).collect();
        out += &format!("var<private> {name}: array<vec4f, {MAX_STRANDS}> = array<vec4f, {MAX_STRANDS}>({});
", items.join(", "));
    }
    out
}

/// The dust's home positions, as the original scatters them: about a quarter in a band just
/// inside the lower rim, the rest in a bowl at the bottom, plus a sparse pale haze everywhere.
/// The same seed gives the same first `n` particles for any count, so the slider only changes
/// how many are drawn.
fn build_dust(count: usize) -> Vec<Dust> {
    let mut st = 1337u32;
    let mut r = || mulberry32(&mut st);
    let sm = |e0: f64, e1: f64, x: f64| {
        let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    };
    let mut out = Vec::with_capacity(count);
    let mut guard = 0;
    while out.len() < count && guard < count * 80 {
        guard += 1;
        let (x, y, tone);
        if r() < 0.28 {
            let angle = r() * std::f64::consts::TAU;
            let radius = 0.87 + r().sqrt() * 0.105;
            x = angle.cos() * radius;
            y = angle.sin() * radius;
            if r() > sm(-0.6, 0.3, -y) {
                continue;
            }
            let q = r();
            tone = if q < 0.5 { 3.0 } else if q < 0.8 { 2.0 } else { 1.0 };
        } else {
            x = r() * 2.0 - 1.0;
            y = r() * 2.0 - 1.0;
            let radius = x.hypot(y);
            if radius > 0.975 {
                continue;
            }
            let bowl = sm(-0.25, 0.85, -y).powf(1.3);
            let band = sm(0.66, 0.96, radius) * sm(-0.7, 0.3, -y);
            let weight = bowl.max(band * 0.9);
            if r() > 0.012 + 0.988 * weight {
                continue;
            }
            tone = if weight < 0.12 {
                4.0
            } else {
                let q = r();
                if q < 0.32 { 0.0 } else if q < 0.52 { 1.0 } else if q < 0.8 { 2.0 } else { 3.0 }
            };
        }
        let chord = (0.95 - x * x - y * y).max(0.0).sqrt();
        let z = (r() * 2.0 - 1.0) * chord;
        let size = if tone == 4.0 { 1.8 } else if r() < 0.22 { 2.1 } else { 1.3 };
        let seed = [size, r() as f32, (2.5 + r() * 3.5) as f32, (0.16 + r() * 0.26) as f32];
        out.push(Dust { home: [x as f32, y as f32, z as f32, tone as f32], seed });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dust_prefix_is_stable() {
        let small = build_dust(500);
        let big = build_dust(2000);
        assert_eq!(small.len(), 500);
        assert!(small.iter().zip(&big).all(|(a, b)| a.home == b.home && a.seed == b.seed));
        assert!(big.iter().all(|d| (d.home[0] * d.home[0] + d.home[1] * d.home[1] + d.home[2] * d.home[2]) < 1.0));
    }

    #[test]
    fn params_fit() {
        assert!(std::mem::size_of::<BallParams>() <= super::super::PARAMS_BYTES);
        assert_eq!(std::mem::size_of::<BallParams>() % 16, 0);
    }
}
