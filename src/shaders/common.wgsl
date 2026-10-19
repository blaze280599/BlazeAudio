// Shared by every pipeline. Prepended to each shader module at load time.

struct Globals {
  res: vec2f, inv_res: vec2f,
  time: f32, treble: f32, hat: f32, kick: f32,
  beat_count: f32, flow: f32, intensity: f32, overlay: f32,
  overlay_opacity: f32, bloom_threshold: f32, bloom_knee: f32, bloom_strength: f32,
  exposure: f32, chromatic: f32, grain: f32, vignette: f32,
  energy: f32, beat_age: f32, scene_time: f32, a_shape: f32,
  a_cell: f32, a_density: f32, a_glyphs: f32, a_gradient: f32,
  a_count: f32, s_mode: f32, s_decay: f32, s_beam: f32,
  s_gain: f32, s_trigger: f32, s_points: f32, _pad0: f32,
  pal: array<vec4f, 4>,
  // Palette in Oklab, sorted darkest to brightest (Gradient ASCII ramp).
  pal_lab: array<vec4f, 4>,
};

const WAVE_LEN: u32 = 2048u;

struct AudioData {
  bands: array<f32, 64>,
  // Phosphor Scope: mono waveform, gain-normalized, oldest first.
  wave: array<f32, 2048>,
};

@group(0) @binding(0) var<uniform> G: Globals;
@group(0) @binding(1) var<storage, read> A: AudioData;
@group(0) @binding(2) var samp_clamp: sampler;

const PI: f32 = 3.14159265;
const TAU: f32 = 6.28318531;

struct VsOut {
  @builtin(position) pos: vec4f,
  @location(0) uv: vec2f,
};

@vertex
fn vs_full(@builtin(vertex_index) vi: u32) -> VsOut {
  let p = vec2f(f32((vi << 1u) & 2u), f32(vi & 2u));
  var o: VsOut;
  o.pos = vec4f(p * 2.0 - 1.0, 0.0, 1.0);
  o.uv = vec2f(p.x, 1.0 - p.y);
  return o;
}

fn hash11(p: f32) -> f32 {
  var x = fract(p * 0.1031);
  x *= x + 33.33;
  x *= x + x;
  return fract(x);
}

fn hash21(p: vec2f) -> f32 {
  var p3 = fract(vec3f(p.x, p.y, p.x) * 0.1031);
  p3 += dot(p3, p3.yzx + 33.33);
  return fract((p3.x + p3.y) * p3.z);
}

fn noise2(p: vec2f) -> f32 {
  let i = floor(p);
  let f = fract(p);
  let u = f * f * (3.0 - 2.0 * f);
  let a = hash21(i);
  let b = hash21(i + vec2f(1.0, 0.0));
  let c = hash21(i + vec2f(0.0, 1.0));
  let d = hash21(i + vec2f(1.0, 1.0));
  return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

fn pal(i: i32) -> vec3f {
  return G.pal[i].rgb;
}

// Continuous band lookup, x in [0,1].
fn band(x: f32) -> f32 {
  let f = clamp(x, 0.0, 1.0) * 63.0;
  let i = u32(floor(f));
  let j = min(i + 1u, 63u);
  return mix(A.bands[i], A.bands[j], fract(f));
}

fn luma(c: vec3f) -> f32 {
  return dot(c, vec3f(0.2126, 0.7152, 0.0722));
}

// Barrel distortion for CRT-style scenes; returns uv and an inside-screen mask.
fn crt_curve(uv: vec2f, k: f32) -> vec3f {
  let c = uv * 2.0 - 1.0;
  let w = c * (1.0 + k * dot(c.yx, c.yx));
  let q = w * 0.5 + 0.5;
  let e = smoothstep(vec2f(0.0), vec2f(0.004), q) * smoothstep(vec2f(0.0), vec2f(0.004), 1.0 - q);
  return vec3f(q, e.x * e.y);
}

// Shared CRT finish: scanlines, slot mask and edge falloff.
fn crt_finish(col: vec3f, frag: vec2f, q: vec2f, mask: f32) -> vec3f {
  let scan = 0.78 + 0.22 * sin(frag.y * PI * 0.5);
  let m = fract(frag.x / 3.0);
  let slot = vec3f(select(0.85, 1.08, m < 0.34), select(0.85, 1.08, m >= 0.34 && m < 0.67), select(0.85, 1.08, m >= 0.67));
  let edge = pow(max(16.0 * q.x * q.y * (1.0 - q.x) * (1.0 - q.y), 0.0), 0.18);
  return col * scan * slot * edge * mask;
}
