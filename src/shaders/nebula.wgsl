// Scene: "Nebula" — 131k GPU particles. A compute pass springs each particle toward a
// morphing audio-reactive shape (spectral sphere, torus, galaxy, cymatic plane), stirred by
// a divergence-free flow field; kicks fire radial bursts. Rendered as additive HDR sprites
// into a persistent trail buffer.

struct Particle {
  pos: vec3f,
  bandx: f32,
  vel: vec3f,
  seed: f32,
};

@group(1) @binding(0) var<storage, read_write> P: array<Particle>;

// xyz = target position, w = band coordinate in [0,1].
fn shape(s: u32, seed: f32) -> vec4f {
  let h1 = hash11(seed * 13.71 + 1.3);
  let h2 = hash11(seed * 7.13 + 4.7);
  let h3 = hash11(seed * 3.37 + 9.1);
  switch s {
    case 0u: {
      // Spectral sphere: latitude maps to frequency, radius bulges with that band.
      let lat = h2 * 2.0 - 1.0;
      let th = h1 * TAU + G.time * 0.1;
      let bx = abs(lat);
      let rad = 0.95 + band(bx) * 0.75 * G.intensity;
      let s2 = sqrt(1.0 - lat * lat);
      return vec4f(vec3f(cos(th) * s2, lat, sin(th) * s2) * rad, bx);
    }
    case 1u: {
      // Torus: tube thickness follows the spectrum around the ring.
      let u = h1 * TAU + G.time * 0.15;
      let v = h2 * TAU;
      let bx = abs(fract(h1 * 2.0) * 2.0 - 1.0);
      let tube = 0.22 + band(bx) * 0.55 * G.intensity;
      let rr = 1.15 + tube * cos(v);
      return vec4f(rr * cos(u), tube * sin(v), rr * sin(u), bx);
    }
    case 2u: {
      // Three-armed spiral galaxy; radius maps to frequency.
      let t = pow(h1, 0.7);
      let radius = 0.12 + t * 1.75;
      let arm = floor(h2 * 3.0);
      let a = arm * TAU / 3.0 + radius * 2.4 - G.time * 0.25;
      let spread = (vec2f(h3, hash11(seed * 1.91)) - 0.5) * (0.12 + 0.3 * t);
      let bv = band(t);
      let y = (hash11(seed * 5.3) - 0.5) * (0.06 + bv * 0.5 * G.intensity) * (1.2 - t);
      return vec4f(cos(a) * radius + spread.x, y, sin(a) * radius + spread.y, t);
    }
    default: {
      // Cymatic plane: concentric standing waves, radius maps to frequency.
      let x = h1 * 2.0 - 1.0;
      let z = h2 * 2.0 - 1.0;
      let rr = length(vec2f(x, z));
      let bx = clamp(rr, 0.0, 1.0);
      let y = sin(rr * 9.0 - G.time * 3.0) * 0.12 * (0.3 + G.bass * 1.5) + band(bx) * 0.65 * G.intensity - 0.3;
      return vec4f(x * 1.9, y, z * 1.9, bx);
    }
  }
}

// Each component is independent of its own axis, so the field is divergence-free:
// particles swirl without clumping or thinning out.
fn flow(p: vec3f, t: f32) -> vec3f {
  return vec3f(
    sin(p.y * 1.7 + t) + sin(p.z * 2.3 - t * 0.7),
    sin(p.z * 1.9 + t * 0.8) + sin(p.x * 2.1 + t * 0.5),
    sin(p.x * 1.6 - t * 0.6) + sin(p.y * 2.4 + t * 0.9)
  );
}

@compute @workgroup_size(256)
fn cs_update(@builtin(global_invocation_id) gid: vec3u) {
  let i = gid.x;
  if (i >= arrayLength(&P)) { return; }
  var p = P[i];
  let dt = min(G.dt, 1.0 / 30.0);

  let m = G.shape_morph;
  let sa = u32(floor(m)) % 4u;
  let sb = (sa + 1u) % 4u;
  let f = smoothstep(0.0, 1.0, fract(m));
  let ta = shape(sa, p.seed);
  let tb = shape(sb, p.seed);
  let tgt = mix(ta, tb, f);

  var acc = (tgt.xyz - p.pos) * (9.0 + 6.0 * G.energy);
  acc += flow(p.pos * 1.25, G.time * 0.55) * (0.12 + G.mid * 0.7 + G.hat * 0.5);
  if (G.beat_trigger > 0.0) {
    let dir = normalize(p.pos + vec3f(1e-4, 0.0, 0.0));
    p.vel += dir * G.beat_strength * (0.25 + hash11(p.seed * 91.7) * 0.9) * G.intensity;
  }
  p.vel += acc * dt;
  p.vel *= exp(-dt * 4.0);
  p.pos += p.vel * dt;
  p.bandx = tgt.w;
  P[i] = p;
}

struct PartOut {
  @builtin(position) pos: vec4f,
  @location(0) uv: vec2f,
  @location(1) col: vec3f,
};

@vertex
fn vs_particle(@builtin(vertex_index) vi: u32, @location(0) pl: vec4f, @location(1) pv: vec4f) -> PartOut {
  var corners = array<vec2f, 6>(
    vec2f(-1.0, -1.0), vec2f(1.0, -1.0), vec2f(-1.0, 1.0),
    vec2f(-1.0, 1.0), vec2f(1.0, -1.0), vec2f(1.0, 1.0)
  );
  let corner = corners[vi];
  var o: PartOut;

  let el = 0.32 * sin(G.time * 0.07) + 0.28;
  let dist = 3.5 - 0.3 * G.kick * G.intensity - 0.35 * G.bass_slow;
  let ang = G.cam_angle;
  let eye = vec3f(cos(ang) * cos(el), sin(el), sin(ang) * cos(el)) * dist;
  let fw = normalize(-eye);
  let rt = normalize(cross(fw, vec3f(0.0, 1.0, 0.0)));
  let up = cross(rt, fw);
  let rel = pl.xyz - eye;
  let z = dot(rel, fw);
  let x = dot(rel, rt);
  let y = dot(rel, up);
  let focal = 1.7;
  let aspect = G.res.x * G.inv_res.y;

  let v = band(pl.w);
  let speed = length(pv.xyz);
  let h = hash11(pv.w * 17.3);
  let sparkle = select(0.0, G.hat * 3.0, h > 0.965);
  let size = (0.0042 + v * 0.0065 + sparkle * 0.003) * focal / max(z, 0.2);
  let ndc = vec2f(x * focal / aspect, y * focal) / max(z, 0.2);
  o.pos = vec4f(ndc + corner * vec2f(size / aspect, size), 0.5, 1.0);
  if (z < 0.2) { o.pos = vec4f(4.0, 4.0, 0.5, 1.0); }

  let c = pal_grad(pl.w * 0.85 + speed * 0.06 + G.time * 0.015);
  let hot = mix(c, vec3f(1.0), clamp(speed * 0.12 + sparkle * 0.2, 0.0, 0.7));
  let depth_fade = exp(-max(z - 2.2, 0.0) * 0.3);
  o.col = hot * (0.03 + v * 0.17 + speed * 0.025 + sparkle * 0.25) * depth_fade * G.intensity;
  o.uv = corner;
  return o;
}

@fragment
fn fs_particle(in: PartOut) -> @location(0) vec4f {
  let d2 = dot(in.uv, in.uv);
  let a = exp(-d2 * 3.5) * (1.0 - smoothstep(0.7, 1.0, d2));
  return vec4f(in.col * a, 0.0);
}

// Multiplies the trail buffer toward black; blend = dst * (1 - srcAlpha).
@fragment
fn fs_fade(in: VsOut) -> @location(0) vec4f {
  let keep = pow(clamp(G.trails, 0.0, 0.97), G.dt * 60.0);
  return vec4f(0.0, 0.0, 0.0, 1.0 - keep);
}

@group(1) @binding(0) var trail_tex: texture_2d<f32>;

@fragment
fn fs_nebula(in: VsOut) -> @location(0) vec4f {
  var p = aspect_uv(in.uv);
  p.y = -p.y;
  let r = length(p);
  var col = pal(3) * 0.55 * exp(-r * 0.9) * (0.5 + G.bass_slow);
  let q = p + vec2f(G.cam_angle * 0.08, 0.0);
  col += mix(pal(2), pal(0), fbm(q * 1.1 + G.time * 0.02)) * pow(fbm(q * 0.7 - G.time * 0.012), 3.0) * 0.1;
  col += vec3f(stars(q, 16.0) + 0.5 * stars(q * 1.9 + 2.0, 30.0)) * 0.45;
  col += textureSampleLevel(trail_tex, samp_clamp, in.uv, 0.0).rgb;
  col += mix(pal(0), vec3f(1.0), 0.3) * (0.006 / (r * r + 0.02)) * (0.15 + G.kick * 0.9) * G.intensity;
  return vec4f(col, 1.0);
}
