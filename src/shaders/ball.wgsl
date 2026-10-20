// Scene: "Crystalized Ball" — a glass sphere of light, after the React Bits component. Electric
// strands braid around the rim with slow travelling flares and short spark arcs along the inside;
// a cloud of dust pools in the bowl and drifts, sways and twinkles in 3D.
//
// Audio: the kick surges the rim (crackle, glow, speed) and tosses the dust like a shaken snow
// globe; hats add crackle; the spectrum shapes the flares around the rim (bass at the bottom,
// treble at the top); a hot spot wanders the rim, glowing with the overall energy. Sparks fire
// on beats as well as on their own timer.
//
// Passes: fs_ball (rim, glow, haze and arcs, full screen), vs_dust/fs_dust (one instanced quad per
// dust particle, added on top). The strand tables (HARM, RATE, ...) are generated in ball.rs and
// prepended to this file.

struct Params {
  size: f32, time: f32, strands: f32, crackle: f32,
  flares: f32, glow: f32, haze: f32, fill: f32,
  presence: f32, unfold: f32, inside: f32, reveal: f32,
  heat_angle: f32, heat: f32, dust_time: f32, motion: f32,
  turn: vec2f, depth: f32, twinkle: f32,
  shape: f32, dust_gain: f32, spectrum: f32, _pad0: f32,
  // Spark arcs: start angle, span, curl, brightness (0 = unused).
  arcs: array<vec4f, 4>,
  // Linear colours in rgb. The w components carry the two most recent beat shakes:
  // rim/rim_hot = age/amount of the newest, spark_glow/edge = the one before, tone_a/tone_b = seeds.
  rim: vec4f, rim_hot: vec4f, spark_glow: vec4f, edge: vec4f, tone_a: vec4f, tone_b: vec4f,
};

@group(0) @binding(3) var<uniform> P: Params;

struct Dust { home: vec4f, seed: vec4f };
@group(1) @binding(0) var<storage, read> D: array<Dust>;

const AMPS = vec4f(0.35, 0.3, 0.22, 0.13);

fn scramble(v0: u32) -> u32 {
  let v = v0 * 747796405u + 2891336453u;
  let w = ((v >> ((v >> 28u) + 4u)) ^ v) * 277803737u;
  return (w >> 22u) ^ w;
}

fn rnd(v: u32) -> f32 {
  return f32(scramble(v)) * (1.0 / 4294967295.0);
}

fn bell(d: f32, w: f32) -> f32 {
  let x = d / w;
  return exp(-x * x);
}

// Angle wrapped to -PI..PI.
fn wrap_pi(a: f32) -> f32 {
  return a - TAU * floor((a + PI) / TAU);
}

// Ball radius in pixels; the kick swells it a touch.
fn ball_r() -> f32 {
  return max(8.0, P.size * min(G.res.x, G.res.y) * 0.5) * (1.0 + G.kick * G.intensity * 0.01);
}

// Glow pooled under the dust, rising with the fill level.
fn pool(q: vec2f) -> f32 {
  let t = length(q - vec2f(0.0, mix(-1.0, -0.66, P.fill))) / mix(0.8, 1.35, P.fill);
  return select(mix(0.22, 0.0, clamp((t - 0.6) / 0.4, 0.0, 1.0)), mix(0.5, 0.22, t / 0.6), t < 0.6);
}

@fragment
fn fs_ball(in: VsOut) -> @location(0) vec4f {
  let R = ball_r();
  let c = G.res * 0.5;
  // Pixels from the centre, y up.
  let p = vec2f(in.pos.x - c.x, c.y - in.pos.y);
  let r = length(p);
  let theta = atan2(p.y, p.x);
  let lift = p.y / max(r, 1e-3);
  let top_dim = 1.0 - 0.35 * pow(max(lift, 0.0), 1.5);
  let line = max(0.6, R * 0.0035);

  let rim = P.rim.rgb;
  let rim_hot = P.rim_hot.rgb;
  let rim_mid = rim * 0.7;
  let rim_deep = rim * 0.27;
  let spark = mix(rim, rim_hot, 0.6);
  let bloom = P.presence * P.presence;
  var col = vec3f(0.0);

  if (r < R) {
    let q = p / R;
    let chord = sqrt(max(1.0 - dot(q, q), 0.0));
    col += rim * 0.13 * pool(q) * (0.55 + 0.45 * chord) * P.haze * P.inside;
    let e = length(q - vec2f(0.0, 0.2)) / 1.2;
    let edge = 0.5 * pow(smoothstep(0.62, 1.0, e), 1.4);
    col += P.edge.rgb * edge * P.inside * (0.5 + 0.5 * P.haze);
  }

  let dr = r - R;
  let halo = 1.0 - smoothstep(R * 0.05, R * 0.105, abs(dr - R * 0.0213));
  col += rim_deep * 0.18 * halo * top_dim * P.glow * bloom;
  col += rim * 0.12 * bell(dr + R * 0.0383, R * 0.032) * P.glow * bloom;
  col += rim * 0.32 * bell(dr, max(R * 0.0117, line)) * P.presence * P.unfold;

  let delta = wrap_pi(theta - P.heat_angle);
  let heat = P.heat * exp(-delta * delta / 0.3);
  // Spectrum around the rim, mirrored: bass at the bottom, treble at the top.
  let spec = mix(1.0, 0.35 + 1.6 * band(abs(wrap_pi(theta + PI * 0.5)) / PI * 0.8), P.spectrum);
  let crackle = P.crackle + heat * 0.9;
  let amp = R * 0.016 * (0.3 + 1.4 * crackle) * P.unfold * (0.75 + 0.25 * spec);
  let jitter_amp = R * 0.0048 * (0.3 + 1.4 * P.crackle) * P.unfold * (1.0 + heat * 0.3);
  let flare_amp = R * 0.064 * (0.4 + 1.2 * crackle) * P.flares * 1.6 * P.unfold * spec;

  if (abs(dr) < amp * 2.0 + flare_amp + R * 0.3) {
    // The strands are sampled in angular bins (with per-bin jitter that changes 24 times a
    // second) and interpolated between them, which gives the crackling, wiry look.
    let bins = clamp(round(TAU * R / 2.6), 240.0, 900.0);
    let stp = TAU / bins;
    let bin_pos = (theta + PI) / stp;
    let bin0 = floor(bin_pos);
    let bin_f = bin_pos - bin0;
    let th0 = bin0 * stp - PI;
    let th1 = th0 + stp;
    let b0 = u32(bin0 - bins * floor(bin0 / bins));
    let b1 = u32(bin0 + 1.0 - bins * floor((bin0 + 1.0) / bins));
    let gain = min(1.0, 2.4 / max(P.strands, 1.0));
    let frame = u32(floor(P.time * 24.0)) % 100000u;
    for (var k = 0u; k < 8u; k++) {
      if (f32(k) >= P.strands) {
        break;
      }
      let m = HARM[k];
      let lead = RATE[k] * P.time + PHASE[k];
      let wave0 = dot(AMPS, sin(m * th0 + lead));
      let wave1 = dot(AMPS, sin(m * th1 + lead));
      let salt = k * 1013u + frame * 7919u;
      let shape = FHARM[k];
      let look = FRATE[k];
      let tone = FPHASE[k];
      let a = amp * shape.w;
      let off0 = a * wave0 + jitter_amp * (rnd(b0 + salt) - 0.5);
      let off1 = a * wave1 + jitter_amp * (rnd(b1 + salt) - 0.5);
      let rc = R + mix(off0, off1, bin_f);
      let grade = (off1 - off0) / (stp * max(r, 1.0));
      let d = abs(r - rc) * inverseSqrt(1.0 + grade * grade);
      let swell = 0.8 + 0.4 * (0.5 + 0.5 * sin(3.0 * theta + P.time * 0.7 + tone.x * 1.7));
      let width = line * look.w * swell * (1.0 + heat * 0.25);
      let core = exp(-d * d / (width * width));
      let sheath = exp(-d * d / (width * width * 3.0));
      let bright = tone.w * (1.0 + heat * 0.45) * P.presence;
      col += (rim_hot * core * 0.8 + rim * sheath * 0.32) * bright;

      let fargs = shape.xyz * theta + look.xyz * P.time + tone.xyz;
      var flare = max(0.0, (sin(fargs.x) + sin(fargs.y) + sin(fargs.z)) / 3.0);
      flare = (flare * flare + heat * 0.35) * flare_amp;
      let dw = abs(r - rc - flare);
      let dm = abs(r - rc - flare * 0.6);
      let dn = abs(r - rc - flare * 0.25);
      var glow = rim_deep * (0.07 * bell(dw, R * 0.09) + 0.16 * bell(dw, R * 0.05));
      glow += rim_mid * 0.28 * bell(dm, R * 0.027) + rim * 0.38 * bell(dn, R * 0.013);
      col += glow * gain * top_dim * P.glow * bloom * (1.0 + heat * 1.4);
    }
  }

  // Spark arcs: short quadratic curves bowing inward from just inside the rim.
  for (var i = 0; i < 4; i++) {
    let arc = P.arcs[i];
    if (arc.w <= 0.002) {
      continue;
    }
    let mid = arc.x + arc.y * 0.5;
    let centre = R * 0.93 * vec2f(cos(mid), sin(mid));
    if (distance(p, centre) > R * (abs(arc.y) * 0.6 + 0.1) + 12.0) {
      continue;
    }
    let ar = R * 0.955;
    let a0 = ar * vec2f(cos(arc.x), sin(arc.x));
    let a2 = ar * vec2f(cos(arc.x + arc.y), sin(arc.x + arc.y));
    let a1 = R * (0.955 - arc.z) * vec2f(cos(mid), sin(mid));
    var dmin = 1e5;
    var along = 0.0;
    var prev = a0;
    for (var s = 1; s <= 10; s++) {
      let u = f32(s) / 10.0;
      let b = mix(mix(a0, a1, u), mix(a1, a2, u), u);
      let pa = p - prev;
      let ba = b - prev;
      let h = clamp(dot(pa, ba) / max(dot(ba, ba), 1e-5), 0.0, 1.0);
      let dist = length(pa - ba * h);
      if (dist < dmin) {
        dmin = dist;
        along = (f32(s) - 1.0 + h) / 10.0;
      }
      prev = b;
    }
    let taper = sqrt(max(sin(PI * along), 0.0));
    let arc_width = line * (0.55 + 0.6 * taper);
    let arc_core = exp(-dmin * dmin / (arc_width * arc_width));
    let arc_glow = exp(-dmin * dmin / (arc_width * arc_width * 12.0));
    col += (spark * 0.95 * arc_core + P.spark_glow.rgb * 0.4 * arc_glow) * arc.w * taper;
  }

  return vec4f(col * G.intensity, 1.0);
}

// Dust: home position remapped so `fill` pools the dust low or floats it high (xyz), and how
// much that squeezed it together (w), which dims it so a dense pool doesn't blow out.
fn settle(p: vec3f) -> vec4f {
  let bend = exp2((0.5 - P.fill) * 1.8);
  let h = clamp((p.y + 1.0) * 0.5, 0.0, 1.0);
  let y = 2.0 * pow(h, bend) - 1.0;
  let w0 = sqrt(max(1.0 - p.y * p.y, 1e-4));
  let w1 = sqrt(max(1.0 - y * y, 0.0));
  let spread = clamp(bend * pow(max(h, 1e-3), bend - 1.0) * w1 / w0, 0.25, 1.0);
  return vec4f(p.x * (w1 / w0), y, p.z * (w1 / w0), spread);
}

struct Drift { p: vec3f, life: f32, spread: f32 };

// Where particle `id` is now, inside the unit ball (y up, z toward the viewer).
fn drift(home: vec4f, seed: vec4f, id: u32) -> Drift {
  let s = settle(home.xyz);
  var p = s.xyz;
  let t = P.dust_time;
  let u = fract(t / seed.z + seed.y);
  let travel = seed.w;
  if (P.motion < 0.5) {
    p.y += travel * (u - 0.5);
  } else if (P.motion < 1.5) {
    p.y -= travel * (u - 0.5);
  } else if (P.motion < 2.5) {
    p += 0.045 * vec3f(
      sin(t * 0.37 + seed.y * 17.0) + 0.5 * sin(t * 0.83 + seed.z * 5.0),
      sin(t * 0.29 + seed.z * 11.0) + 0.5 * sin(t * 0.61 + seed.w * 7.0),
      sin(t * 0.33 + seed.w * 23.0)
    );
  } else {
    let dir = select(1.0, -1.0, rnd(id * 7u + 10u) < 0.5);
    let lp = t * (0.6 + 0.9 * rnd(id * 7u + 3u)) * dir + seed.y * TAU;
    p.x += (0.025 + 0.05 * rnd(id * 7u + 16u)) * cos(lp);
    p.y += (0.025 + 0.05 * rnd(id * 7u + 16u)) * sin(lp);
  }
  let wobble = 0.002 + 0.006 * rnd(id * 7u + 1u);
  let w1 = 0.8 + 2.4 * rnd(id * 7u + 2u);
  let w2 = 0.8 + 2.4 * rnd(id * 7u + 4u);
  p += wobble * vec3f(sin(t * w1 + seed.y * 40.0), cos(t * w2 + seed.z * 30.0), sin(t * (w1 + w2) * 0.5 + seed.w * 20.0));
  // Sway: turn the whole cloud a little about y, then x.
  let cy = cos(P.turn.x);
  let sy = sin(P.turn.x);
  p = vec3f(cy * p.x + sy * p.z, p.y, -sy * p.x + cy * p.z);
  let cx = cos(P.turn.y);
  let sx = sin(P.turn.y);
  p = vec3f(p.x, cx * p.y + sx * p.z, -sx * p.y + cx * p.z);
  return Drift(p, sin(PI * u), s.w);
}

// One beat's toss: a random jolt, a spin about the vertical axis and a lift that throws the pooled
// dust upward, rising and settling back over about half a second.
fn shake(p: vec3f, id: u32, age: f32, amount: f32, seed: f32) -> vec3f {
  if (amount <= 0.0) {
    return vec3f(0.0);
  }
  let x = age * 7.0;
  let impulse = x * exp(1.0 - x);
  let s = id * 7u + u32(seed) * 2654435761u;
  let jolt = vec3f(rnd(s + 12u), rnd(s + 13u), rnd(s + 14u)) - 0.5;
  let spin = vec3f(-p.z, 0.0, p.x) * 0.9 * select(1.0, -1.0, rnd(u32(seed) * 977u) < 0.5);
  let lift = vec3f(0.0, 0.5 + 0.8 * rnd(id * 7u + 15u), 0.0) * (0.3 + 0.7 * (1.0 - smoothstep(-0.8, 0.2, p.y)));
  return (jolt * 0.8 + spin + lift) * amount * impulse;
}

struct DustOut {
  @builtin(position) pos: vec4f,
  @location(0) color: vec3f,
  // Offset from the particle centre in pixels, and the particle's size.
  @location(1) q: vec2f,
  @location(2) size: f32,
};

@vertex
fn vs_dust(@builtin(vertex_index) vi: u32, @builtin(instance_index) id: u32) -> DustOut {
  let home = D[id].home;
  let seed = D[id].seed;
  let dr = drift(home, seed, id);
  var p = dr.p;
  p += shake(p, id, P.rim.w, P.rim_hot.w, P.tone_a.w) + shake(p, id, P.spark_glow.w, P.edge.w, P.tone_b.w);
  let len = length(p);
  let inside = 1.0 - smoothstep(0.955, 0.985, length(p.xy));
  if (len > 0.975) {
    p *= 0.975 / len;
  }

  // Dust at the back of the ball is smaller and dimmer.
  let front = p.z * 0.5 + 0.5;
  let depth_size = mix(1.0, 0.7 + 0.6 * front, P.depth);
  let depth_light = mix(1.0, 0.35 + 0.8 * front, P.depth);
  let blink = sin(P.dust_time * (2.0 + 6.0 * rnd(id * 7u + 5u)) + rnd(id * 7u + 6u) * TAU);
  let twinkle = mix(1.0, smoothstep(-0.9, -0.6, blink), P.twinkle);
  let fade = smoothstep(0.04, 0.12, dr.life);
  // The intro lights the dust from the bottom of the ball up.
  let order = (home.y + 1.0) * 0.5 * 0.55 + rnd(id * 7u + 8u) * 0.25;
  let reveal = smoothstep(order, order + 0.2, P.reveal);
  let alpha = select(0.6, 1.0, rnd(id * 7u + 9u) < 0.5 || home.w > 3.5);

  let R = ball_r();
  let size = seed.x * max(1.0, R / 491.0) * (0.35 + 0.95 * dr.life) * depth_size * mix(0.5, 1.0, reveal);
  let centre = G.res * 0.5 + vec2f(p.x, -p.y) * R;
  var corners = array<vec2f, 6>(vec2f(-1.0, -1.0), vec2f(1.0, -1.0), vec2f(-1.0, 1.0), vec2f(-1.0, 1.0), vec2f(1.0, -1.0), vec2f(1.0, 1.0));
  let off = corners[vi] * (size + 2.0) * 0.5;
  let px = centre + off;

  let rim = P.rim.rgb;
  var tones = array<vec3f, 5>(rim * 0.8, P.tone_a.rgb, P.tone_b.rgb, mix(rim, P.rim_hot.rgb, 0.35), mix(rim, P.rim_hot.rgb, 0.9));
  let rim_lit = smoothstep(0.8, 0.97, length(p.xy));
  let color = tones[min(u32(home.w + 0.5), 4u)] + rim * rim_lit * rim_lit * 0.12;

  var o: DustOut;
  o.pos = vec4f(px.x / G.res.x * 2.0 - 1.0, 1.0 - px.y / G.res.y * 2.0, 0.0, 1.0);
  o.color = color * alpha * fade * twinkle * depth_light * reveal * inside * sqrt(dr.spread) * P.dust_gain * G.intensity;
  o.q = off;
  o.size = size;
  return o;
}

// Pixel coverage of a crisp square or a round dot.
@fragment
fn fs_dust(in: DustOut) -> @location(0) vec4f {
  let h = max(in.size * 0.5, 0.5);
  var cover: f32;
  if (P.shape < 0.5) {
    let c = clamp(vec2f(h + 0.5) - abs(in.q), vec2f(0.0), vec2f(1.0));
    cover = c.x * c.y;
  } else {
    cover = clamp(h - length(in.q) + 0.5, 0.0, 1.0);
  }
  return vec4f(in.color * cover, 0.0);
}
