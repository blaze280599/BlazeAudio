// Scene: "Gradient ASCII" — a soft light (horizon, pinch, rise or eclipse) seen through a grid
// of glyphs, brighter cells using denser characters. The light swells with the music and its
// edge follows the spectrum, hats make the glyphs flicker, and each beat sends a ring of
// scrambled characters out from the light. Fades in from its source while the glyphs decode.

@group(1) @binding(0) var atlas: texture_2d<f32>;

// Glyph cells in the atlas are 60x100 px, laid out in one row.
const CELL_ASPECT: f32 = 0.6;

fn hash22s(p: vec2f) -> vec2f {
  var q = fract(vec3f(p.x, p.y, p.x) * vec3f(0.1031, 0.103, 0.0973));
  q += dot(q, q.yzx + 33.33);
  return fract((q.xx + q.yz) * q.zy) * 2.0 - 1.0;
}

// Quintic gradient noise in [0, 1].
fn gnoise(p: vec2f) -> f32 {
  let i = floor(p);
  let f = fract(p);
  let u = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
  let a = dot(hash22s(i), f);
  let b = dot(hash22s(i + vec2f(1.0, 0.0)), f - vec2f(1.0, 0.0));
  let c = dot(hash22s(i + vec2f(0.0, 1.0)), f - vec2f(0.0, 1.0));
  let d = dot(hash22s(i + vec2f(1.0, 1.0)), f - vec2f(1.0, 1.0));
  return 0.5 + 0.5 * mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

fn oklab_to_rgb(lab: vec3f) -> vec3f {
  var l = lab.x + 0.3963377774 * lab.y + 0.2158037573 * lab.z;
  var m = lab.x - 0.1055613458 * lab.y - 0.0638541728 * lab.z;
  var s = lab.x - 0.0894841775 * lab.y - 1.2914855480 * lab.z;
  l = l * l * l;
  m = m * m * m;
  s = s * s * s;
  return max(vec3f(
    4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
    -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
    -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s
  ), vec3f(0.0));
}

// Shapes return (tone a, tone b, a->b mix, reveal order).
fn horizon(p: vec2f, time: f32, aspect: f32) -> vec4f {
  let span = 0.5 * max(aspect, 1.0);
  let x = p.x / span;
  // The rim ripples with the spectrum, bass in the middle, highs at the sides.
  let spec = band(abs(x) * 0.85) * G.intensity;
  let arch = -0.17 - 0.2 * x * x + 0.02 * sin(time * 0.6) + 0.07 * spec + 0.03 * G.kick * G.intensity;
  let edge = p.y - arch;
  let rim = (0.38 + 0.12 * G.energy) * smoothstep(-0.135, 0.42, edge);
  let sun = vec2f(0.08 * sin(time * 0.31) * span, 0.58);
  let spread = length((p - sun) / vec2f(0.95 * span, 0.75));
  let sky = mix(0.55, 1.0, smoothstep(0.2, 0.95, spread));
  return vec4f(rim, sky, smoothstep(0.18, 0.62, edge), clamp(edge * 1.3, 0.0, 1.0));
}

fn pinch(p: vec2f, time: f32, aspect: f32) -> vec4f {
  let span = 0.5 * max(aspect, 0.75);
  let x = p.x / span;
  let sway = 0.02 * sin(x * 1.7 + time * 0.55);
  let breathe = 1.0 + 0.06 * sin(time * 0.8) + 0.15 * G.kick * G.intensity;
  // Band thickness traces the mirrored spectrum.
  let spec = 1.0 + 0.6 * band(abs(x) * 0.6) * G.intensity;
  let width = (0.13 + 0.22 * x * x) * breathe * spec;
  let y = abs(p.y - sway) / width;
  let t = exp(-pow(y, 2.2)) * (0.42 + 0.58 * exp(-x * x * 1.8));
  return vec4f(t, t, 0.0, clamp(abs(p.y - sway) * 2.4 + abs(x) * 0.3, 0.0, 1.0));
}

fn rise(p: vec2f, time: f32, aspect: f32) -> vec4f {
  let wide = max(aspect, 0.9);
  let center = vec2f(0.05 * sin(time * 0.45) * wide, -0.7);
  let lift = 1.0 + 0.04 * sin(time * 0.8) + 0.16 * G.energy + 0.1 * G.kick * G.intensity;
  let spec = 1.0 + 0.25 * band(clamp(abs(p.x - center.x) / (0.82 * wide), 0.0, 1.0)) * G.intensity;
  let radius = vec2f(0.82 * wide, 0.64 * lift * spec);
  let d = length((p - center) / radius);
  let t = clamp(pow(clamp(1.0 - d, 0.0, 1.0), 0.875) * 1.35, 0.0, 1.0);
  return vec4f(t, t, 0.0, clamp((d - 0.2) * 1.3, 0.0, 1.0));
}

fn eclipse(p: vec2f, time: f32, aspect: f32) -> vec4f {
  let fit = min(1.0, aspect * 1.25);
  let r = 0.23 * fit * (1.0 + 0.015 * sin(time * 0.9) + 0.05 * G.kick * G.intensity);
  let angle = atan2(p.y, p.x);
  // Corona rays: drifting noise plus the spectrum wrapped around the disc.
  let spec = band(abs(angle) / PI) * G.intensity;
  let rays = gnoise(vec2f(cos(angle), sin(angle)) * 2.2 + time * 0.08) * 0.8 + 0.6 + spec * 1.1;
  let d = length(p) - r;
  let reach = 0.11 * rays * fit;
  let t = smoothstep(-0.006, 0.016, d) * exp(-max(d, 0.0) / reach);
  return vec4f(t, t, 0.0, clamp(d * 2.0, 0.0, 1.0));
}

// Palette ramp in Oklab, darkest to brightest; dark areas fade to nothing.
fn tone(t: f32) -> vec4f {
  let x = smoothstep(0.3, 1.0, t) * 3.0;
  let i = min(i32(floor(x)), 2);
  return vec4f(mix(G.pal_lab[i].xyz, G.pal_lab[i + 1].xyz, x - f32(i)), smoothstep(0.0, 0.38, t));
}

struct Field {
  col: vec3f,     // premultiplied linear colour
  alpha: f32,
  reveal: f32,
};

fn field(p: vec2f, aspect: f32) -> Field {
  let time = G.flow * 0.6;
  var q = p;
  let warp = vec2f(
    gnoise(q * 1.6 + vec2f(0.0, time * 0.3)),
    gnoise(q * 1.6 + vec2f(4.3, -time * 0.24))
  ) - 0.5;
  let shape_id = i32(G.a_shape);
  q += warp * (0.5 + 0.4 * G.energy) * select(0.18, 0.04, shape_id == 3);
  var sh: vec4f;
  if (shape_id == 0) {
    sh = horizon(q, time, aspect);
  } else if (shape_id == 1) {
    sh = pinch(q, time, aspect);
  } else if (shape_id == 2) {
    sh = rise(q, time, aspect);
  } else {
    sh = eclipse(q, time, aspect);
  }

  let intro = clamp(G.scene_time / 2.4, 0.0, 1.0);
  var reveal = clamp((intro * 1.45 - sh.w) / 0.45, 0.0, 1.0);
  reveal = reveal * reveal * (3.0 - 2.0 * reveal);

  let t = clamp(sh.xy * mix(0.7, 1.0, reveal), vec2f(0.0), vec2f(1.0));
  let mixed = mix(tone(t.x), tone(t.y), sh.z);
  let a = mixed.a * reveal;
  var f: Field;
  f.col = oklab_to_rgb(mixed.rgb) * a;
  f.alpha = a;
  f.reveal = reveal;
  return f;
}

// Where each beat's ring of scrambled glyphs starts, per shape.
fn light_source() -> vec2f {
  let shape_id = i32(G.a_shape);
  if (shape_id == 0) { return vec2f(0.0, -0.2); }
  if (shape_id == 2) { return vec2f(0.0, -0.55); }
  return vec2f(0.0);
}

@fragment
fn fs_ascii(in: VsOut) -> @location(0) vec4f {
  let aspect = G.res.x * G.inv_res.y;
  // y-up pixel coordinates, so the light sits the same way up as the original.
  let frag = vec2f(in.pos.x, G.res.y - in.pos.y);
  let cell_h = max(6.0, G.a_cell * G.res.y / 1080.0);
  let csize = vec2f(cell_h * CELL_ASPECT, cell_h);
  let offset = (G.res - ceil(G.res / csize) * csize) * 0.5;
  let coord = (frag - offset) / csize;
  let cell = floor(coord);
  let local = coord - cell;
  let count = G.a_count;
  // Derivatives of the continuous coordinate, so mip selection doesn't jump at cell edges.
  let gx = dpdx(coord) / vec2f(count, 1.0);
  let gy = dpdy(coord) / vec2f(count, 1.0);

  let pp = (frag * G.inv_res - 0.5) * vec2f(aspect, 1.0);
  let pc = (((cell + 0.5) * csize + offset) * G.inv_res - 0.5) * vec2f(aspect, 1.0);
  let bg = field(pp, aspect);
  let fc = field(pc, aspect);

  let value = pow(luma(fc.col), 1.0 / 2.2);
  let pick = hash21(cell);
  let level = hash21(cell * 1.73 + 4.1);
  let tick = floor(G.time * 9.0 + level * 9.0);
  let flicker = clamp(0.2 + G.hat * 0.9 * G.intensity, 0.0, 1.0);
  let wobble = (hash21(cell * 2.31 + tick * 17.7) - 0.5) * flicker * 0.4 + (level - 0.5) * 0.1;
  let tn = clamp(pow(value, 0.75) + wobble, 0.0, 1.0);
  var index = floor(tn * (count - 1.0) + 0.5);

  // Beat ring: expands from the light for ~1.2 s after each beat.
  let age = G.beat_age;
  let k = clamp(age / 1.2, 0.0, 1.0);
  let env = min(1.0, age / 0.08) * (1.0 - k) * (1.0 - k) * min(G.intensity, 1.0);
  let dr = length(pc - light_source());
  let ring = exp(-pow((dr - age * 0.9) / 0.06, 2.0)) * env;
  let decode = max(ring, 1.0 - fc.reveal);
  if (decode > 0.02) {
    let fr = floor(G.time * 20.0);
    if (hash21(cell * 3.7 + fr * 5.3) < decode * 1.25) {
      index = 1.0 + floor(hash21(cell + fr * 1.37) * (count - 1.0));
    }
  }

  let present = step(pick, G.a_density * mix(1.0, 0.3 + tn * 1.1, 0.55));
  let cutoff = smoothstep(0.03, 0.1, value);
  let uv = vec2f((index + local.x) / count, 1.0 - local.y);
  let coverage = textureSampleGrad(atlas, samp_clamp, uv, gx, gy).r;
  let ga = G.a_glyphs * present * cutoff * coverage * mix(1.0, 0.35 + 0.65 * level, 0.55);

  // Glyphs carry the light's own hue, pushed into HDR so the bright ones bloom.
  let gain = 0.75 + 0.6 * G.energy * G.intensity + 0.4 * G.kick * G.intensity;
  let hue = fc.col / max(fc.alpha, 1e-3);
  var col = bg.col * G.a_gradient * gain * 0.55;
  col += hue * ga * (0.35 + 1.4 * value) * gain * 1.6;
  col += pal(3) * 0.01;
  return vec4f(col, 1.0);
}
