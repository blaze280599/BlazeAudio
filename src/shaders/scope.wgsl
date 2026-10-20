// Scene: "Phosphor Scope" — a vector oscilloscope on a CRT. The beam traces the live waveform
// (triggered Y-T sweep or polar loop) into an afterglow buffer that fades a
// little every frame; the composite tints that glow with the palette, lays a graticule over it
// whose lines light up with the spectrum, and curves it like the Faulty Terminal glass.
//
// Passes: fs_decay (afterglow, prev -> cur), vs_beam/fs_beam (segments, max-blended into cur),
// fs_scope (cur -> HDR scene).

struct Params {
  mode: f32, decay: f32, beam: f32, gain: f32,
  trigger: f32, points: f32, _pad0: f32, _pad1: f32,
};

@group(0) @binding(3) var<uniform> P: Params;
@group(1) @binding(0) var t_persist: texture_2d<f32>;

// Beam half-width in pixels: the setting is for 1440p; energy thickens it a little.
fn beam_width() -> f32 {
  return max(0.8, P.beam * 0.5 * G.res.y / 1440.0 * (0.8 + G.energy * 0.5));
}

// Point `i` of the trace in pixels (y down).
fn scope_point(i: u32) -> vec2f {
  let n = max(P.points, 2.0);
  let mode = u32(P.mode);
  let idx = min(u32(P.trigger) + i, WAVE_LEN - 1u);
  // Beats punch the trace outward, hats shake the beam.
  let punch = 1.0 + G.kick * G.intensity * 0.1;
  let g = P.gain * punch;
  let jitter = (hash11(f32(i) * 1.7 + fract(G.time) * 911.0) - 0.5) * G.hat * G.intensity * 0.025;
  let m = tanh(A.wave[idx] * g) + jitter;
  let t = f32(i) / (n - 1.0);
  let aspect = G.res.x / G.res.y;

  var p: vec2f;
  if (mode == 0u) {
    p = vec2f((t * 2.0 - 1.0) * aspect * 0.92, m * 0.6);
  } else {
    let a = t * TAU + G.flow * 0.15;
    let r = max(0.55 + m * 0.3, 0.0);
    p = r * vec2f(cos(a), sin(a));
  }
  return G.res * 0.5 + vec2f(p.x, -p.y) * G.res.y * 0.5;
}

struct BeamOut {
  @builtin(position) pos: vec4f,
  @location(0) @interpolate(flat) a: vec2f,
  @location(1) @interpolate(flat) b: vec2f,
  @location(2) @interpolate(flat) level: f32,
};

// One instance per segment, expanded to a quad around it (with room for the soft edge).
@vertex
fn vs_beam(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> BeamOut {
  var o: BeamOut;
  if (f32(ii + 1u) >= P.points) {
    o.pos = vec4f(2.0, 2.0, 0.0, 1.0);
    return o;
  }
  let a = scope_point(ii);
  let b = scope_point(ii + 1u);
  let hw = beam_width();
  let r = hw * 2.5 + 1.0;
  let d = b - a;
  let len = length(d);
  let dir = select(vec2f(1.0, 0.0), d / len, len > 1e-4);
  let nrm = vec2f(-dir.y, dir.x);
  // Corners: 0 1 2 / 2 1 3, with (end, side) from the corner index.
  var corners = array<u32, 6>(0u, 1u, 2u, 2u, 1u, 3u);
  let corner = corners[vi];
  let end = f32(corner & 1u);
  let side = f32(corner >> 1u) * 2.0 - 1.0;
  let px = mix(a, b, end) + dir * (end * 2.0 - 1.0) * r + nrm * side * r;
  o.pos = vec4f(px.x / G.res.x * 2.0 - 1.0, 1.0 - px.y / G.res.y * 2.0, 0.0, 1.0);
  o.a = a;
  o.b = b;
  // A real beam leaves more light where it moves slowly.
  let speed = clamp(hw * 3.0 / (len + hw * 0.5), 0.12, 1.0);
  o.level = speed * (0.75 + G.energy * 0.45 + G.kick * G.intensity * 0.7);
  return o;
}

@fragment
fn fs_beam(in: BeamOut) -> @location(0) vec4f {
  let pa = in.pos.xy - in.a;
  let ba = in.b - in.a;
  let h = clamp(dot(pa, ba) / max(dot(ba, ba), 1e-6), 0.0, 1.0);
  let d = length(pa - ba * h);
  let hw = beam_width();
  let v = exp(-d * d / (hw * hw)) * in.level;
  return vec4f(v, 0.0, 0.0, 1.0);
}

@fragment
fn fs_decay(in: VsOut) -> @location(0) vec4f {
  let v = textureLoad(t_persist, vec2i(in.pos.xy), 0).r * P.decay;
  return vec4f(v, 0.0, 0.0, 1.0);
}

// Distance in pixels to the nearest graticule line, every `div` pixels from the centre.
fn grid_dist(p: vec2f, div: f32) -> vec2f {
  return abs(fract(p / div + 0.5) - 0.5) * div;
}

@fragment
fn fs_scope(in: VsOut) -> @location(0) vec4f {
  let cv = crt_curve(in.uv, 0.07);
  var q = cv.xy;

  // Every fourth beat, tear a few row bands sideways like the terminal does.
  let glitch = select(0.0, G.kick * G.intensity, u32(G.beat_count) % 4u == 0u);
  let row_band = floor(q.y * 24.0);
  let tear = step(0.85, hash11(row_band * 3.1 + G.beat_count * 7.7)) * glitch;
  q.x += (hash11(row_band + G.beat_count) - 0.5) * tear * 0.06;

  let glow = textureSampleLevel(t_persist, samp_clamp, q, 0.0).r;
  // Fresh trace runs hot toward white; the afterglow cools to the darkest palette colour.
  var col = pal(0) * smoothstep(0.02, 0.4, glow) * 0.7;
  col += pal(1) * smoothstep(0.08, 0.6, glow) * 1.3;
  col += mix(pal(1), vec3f(1.0), 0.6) * smoothstep(0.55, 1.0, glow) * 1.2;
  col *= 1.0 + G.kick * G.intensity * 0.4;

  // Graticule: 8 divisions tall, square cells, minor ticks on the centre axes. Lines brighten
  // with the spectrum, mirrored around the centre like the terminal's bars.
  let frag = q * G.res;
  let c = frag - G.res * 0.5;
  let div = G.res.y / 8.0;
  let gd = grid_dist(c, div);
  let px = max(1.0, G.res.y / 1080.0);
  let line = 1.0 - smoothstep(0.0, px, min(gd.x, gd.y));
  let axis = 1.0 - smoothstep(0.0, px, min(abs(c.x), abs(c.y)));
  let td = grid_dist(c, div / 5.0);
  let tick_x = (1.0 - smoothstep(0.0, px, td.x)) * step(abs(c.y), div * 0.08);
  let tick_y = (1.0 - smoothstep(0.0, px, td.y)) * step(abs(c.x), div * 0.08);
  let bx = clamp(abs(c.x) / (G.res.x * 0.5), 0.0, 1.0);
  let spec = band(bx);
  let grat = line * (0.07 + spec * 0.3) + axis * 0.08 + max(tick_x, tick_y) * 0.1;
  col += mix(pal(0), pal(2), 0.5) * grat;

  col += pal(0) * 0.012;
  col += pal(2) * tear * 0.15;
  col = crt_finish(col, in.pos.xy, q, cv.z);
  return vec4f(col, 1.0);
}
