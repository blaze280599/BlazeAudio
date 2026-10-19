// Shared by every pipeline. Prepended to each shader module at load time.

struct Globals {
  res: vec2f, inv_res: vec2f,
  time: f32, dt: f32, bass: f32, mid: f32,
  treble: f32, energy: f32, beat: f32, beat_count: f32,
  flux: f32, intensity: f32, scene_time: f32, hat: f32,
  beat_strength: f32, overlay: f32, trails: f32, spec_pos: f32,
  bloom_threshold: f32, bloom_knee: f32, bloom_strength: f32, exposure: f32,
  chromatic: f32, grain: f32, vignette: f32, transition: f32,
  bass_slow: f32, particle_count: f32, beat_trigger: f32, shape_morph: f32,
  cam_angle: f32, art_present: f32, kick: f32, travel: f32,
  overlay_opacity: f32, art_spin: f32, scroll: f32, pad0: f32,
  pal: array<vec4f, 4>,
  ring_age: vec4f,
  ring_str: vec4f,
};

struct AudioData {
  bands: array<f32, 64>,
  peaks: array<f32, 64>,
  wave: array<f32, 512>,
};

@group(0) @binding(0) var<uniform> G: Globals;
@group(0) @binding(1) var<storage, read> A: AudioData;
@group(0) @binding(2) var spec_tex: texture_2d<f32>;
@group(0) @binding(3) var samp_wrap: sampler;
@group(0) @binding(4) var art_tex: texture_2d<f32>;
@group(0) @binding(5) var samp_clamp: sampler;

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

fn hash22(p: vec2f) -> vec2f {
  var p3 = fract(vec3f(p.x, p.y, p.x) * vec3f(0.1031, 0.1030, 0.0973));
  p3 += dot(p3, p3.yzx + 33.33);
  return fract((p3.xx + p3.yz) * p3.zy);
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

fn fbm(p_in: vec2f) -> f32 {
  var p = p_in;
  var v = 0.0;
  var a = 0.5;
  for (var i = 0; i < 5; i++) {
    v += a * noise2(p);
    p = mat2x2f(1.6, 1.2, -1.2, 1.6) * p;
    a *= 0.5;
  }
  return v;
}

fn rot2(a: f32) -> mat2x2f {
  let c = cos(a);
  let s = sin(a);
  return mat2x2f(c, s, -s, c);
}

fn pal(i: i32) -> vec3f {
  return G.pal[i].rgb;
}

// Smooth cyclic gradient through the three palette hues.
fn pal_grad(t_in: f32) -> vec3f {
  let t = fract(t_in) * 3.0;
  let a = pal(0);
  let b = pal(1);
  let c = pal(2);
  if (t < 1.0) { return mix(a, b, smoothstep(0.0, 1.0, t)); }
  if (t < 2.0) { return mix(b, c, smoothstep(1.0, 2.0, t)); }
  return mix(c, a, smoothstep(2.0, 3.0, t));
}

// Continuous band lookup, x in [0,1].
fn band(x: f32) -> f32 {
  let f = clamp(x, 0.0, 1.0) * 63.0;
  let i = u32(floor(f));
  let j = min(i + 1u, 63u);
  return mix(A.bands[i], A.bands[j], fract(f));
}

fn wave(x: f32) -> f32 {
  let f = fract(x) * 511.0;
  let i = u32(floor(f));
  let j = min(i + 1u, 511u);
  return mix(A.wave[i], A.wave[j], fract(f));
}

// Spectrogram history: x = band position [0,1], age in [0,1] (0 = newest, 1 = ~4 s ago).
fn spec(x: f32, age: f32) -> f32 {
  let u = clamp(x, 0.0, 1.0) * (63.0 / 64.0) + 0.5 / 64.0;
  let row = floor(G.spec_pos) - 1.0 - clamp(age, 0.0, 1.0) * 254.0;
  let v = (row + 0.5) / 256.0;
  return textureSampleLevel(spec_tex, samp_wrap, vec2f(u, v), 0.0).r;
}

fn luma(c: vec3f) -> f32 {
  return dot(c, vec3f(0.2126, 0.7152, 0.0722));
}

fn aspect_uv(uv: vec2f) -> vec2f {
  let aspect = G.res.x * G.inv_res.y;
  return (uv - 0.5) * vec2f(aspect, 1.0) * 2.0;
}

// Faint parallax starfield; twinkle driven by highs.
fn stars(p: vec2f, density: f32) -> f32 {
  let g = p * density;
  let id = floor(g);
  let f = fract(g) - 0.5;
  let h = hash22(id);
  let off = (h - 0.5) * 0.7;
  let d = length(f - off);
  let present = step(0.86, hash21(id + 7.3));
  let tw = 0.55 + 0.45 * sin(G.time * (2.0 + h.x * 4.0) + h.y * 30.0);
  return present * exp(-d * d * 900.0) * tw * (0.6 + G.treble * 1.6 + G.hat * 1.5);
}
