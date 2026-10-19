// Scene: "Liquid Ether" — a real-time incompressible fluid (stable fluids on a coarse grid)
// stirred by audio-driven emitters and rendered by flow speed through the palette.
// Velocity is stored in grid cells per second; all passes run at simulation resolution.

@group(1) @binding(0) var tex_a: texture_2d<f32>;
@group(1) @binding(1) var tex_b: texture_2d<f32>;

fn ld(t: texture_2d<f32>, c: vec2i) -> vec4f {
  let s = vec2i(textureDimensions(t)) - 1;
  return textureLoad(t, clamp(c, vec2i(0), s), 0);
}

fn sim_dt() -> f32 {
  return clamp(G.dt, 1.0 / 240.0, 1.0 / 30.0);
}

// Semi-Lagrangian self-advection with dissipation and solid walls.
@fragment
fn fs_fl_advect(in: VsOut) -> @location(0) vec4f {
  let size = vec2f(textureDimensions(tex_a));
  let c = vec2i(in.pos.xy);
  let v = ld(tex_a, c).xy;
  let back = (in.pos.xy - v * sim_dt()) / size;
  var nv = textureSampleLevel(tex_a, samp_clamp, back, 0.0).xy * exp(-sim_dt() * 0.9);
  let edge = min(min(in.pos.x, size.x - in.pos.x), min(in.pos.y, size.y - in.pos.y));
  nv *= smoothstep(0.5, 2.0, edge);
  return vec4f(nv, 0.0, 1.0);
}

@fragment
fn fs_fl_curl(in: VsOut) -> @location(0) vec4f {
  let c = vec2i(in.pos.xy);
  let l = ld(tex_a, c - vec2i(1, 0)).y;
  let r = ld(tex_a, c + vec2i(1, 0)).y;
  let b = ld(tex_a, c - vec2i(0, 1)).x;
  let t = ld(tex_a, c + vec2i(0, 1)).x;
  return vec4f(0.5 * ((r - l) - (t - b)), 0.0, 0.0, 1.0);
}

// Vorticity confinement + audio emitters. tex_a = velocity, tex_b = curl.
@fragment
fn fs_fl_force(in: VsOut) -> @location(0) vec4f {
  let size = vec2f(textureDimensions(tex_a));
  let c = vec2i(in.pos.xy);
  var v = ld(tex_a, c).xy;
  let cl = ld(tex_b, c - vec2i(1, 0)).x;
  let cr = ld(tex_b, c + vec2i(1, 0)).x;
  let cb = ld(tex_b, c - vec2i(0, 1)).x;
  let ct = ld(tex_b, c + vec2i(0, 1)).x;
  let cc = ld(tex_b, c).x;
  var f = 0.5 * vec2f(abs(ct) - abs(cb), abs(cr) - abs(cl));
  f /= length(f) + 1e-4;
  f *= (18.0 + G.mid * 22.0) * cc;
  f.y = -f.y;
  v += f * sim_dt();

  let uv = in.pos.xy / size;
  let aspect = size.x / size.y;
  for (var i = 0; i < 8; i++) {
    let s = SPL.d[i * 2];
    let fo = SPL.d[i * 2 + 1];
    if (s.z <= 0.0) { continue; }
    let d = (uv - s.xy) * vec2f(aspect, 1.0);
    let w = exp(-dot(d, d) / (s.z * s.z));
    if (s.w > 0.5) {
      v += normalize(d + vec2f(1e-5)) * fo.x * w * sim_dt();
    } else {
      v += fo.xy * w * sim_dt();
    }
  }
  return vec4f(v, 0.0, 1.0);
}

@fragment
fn fs_fl_div(in: VsOut) -> @location(0) vec4f {
  let c = vec2i(in.pos.xy);
  let l = ld(tex_a, c - vec2i(1, 0)).x;
  let r = ld(tex_a, c + vec2i(1, 0)).x;
  let b = ld(tex_a, c - vec2i(0, 1)).y;
  let t = ld(tex_a, c + vec2i(0, 1)).y;
  return vec4f(0.5 * (r - l + t - b), 0.0, 0.0, 1.0);
}

// Jacobi pressure iteration. tex_a = pressure, tex_b = divergence.
@fragment
fn fs_fl_jacobi(in: VsOut) -> @location(0) vec4f {
  let c = vec2i(in.pos.xy);
  let l = ld(tex_a, c - vec2i(1, 0)).x;
  let r = ld(tex_a, c + vec2i(1, 0)).x;
  let b = ld(tex_a, c - vec2i(0, 1)).x;
  let t = ld(tex_a, c + vec2i(0, 1)).x;
  let d = ld(tex_b, c).x;
  return vec4f((l + r + b + t - d) * 0.25, 0.0, 0.0, 1.0);
}

// Subtract the pressure gradient. tex_a = pressure, tex_b = velocity.
@fragment
fn fs_fl_grad(in: VsOut) -> @location(0) vec4f {
  let c = vec2i(in.pos.xy);
  let l = ld(tex_a, c - vec2i(1, 0)).x;
  let r = ld(tex_a, c + vec2i(1, 0)).x;
  let b = ld(tex_a, c - vec2i(0, 1)).x;
  let t = ld(tex_a, c + vec2i(0, 1)).x;
  let v = ld(tex_b, c).xy - 0.5 * vec2f(r - l, t - b);
  return vec4f(v, 0.0, 1.0);
}

@fragment
fn fs_ether(in: VsOut) -> @location(0) vec4f {
  let size = vec2f(textureDimensions(tex_a));
  let ts = 1.0 / size;
  // Light 5-tap blur hides the coarse grid.
  var v = textureSampleLevel(tex_a, samp_clamp, in.uv, 0.0).xy * 0.4;
  v += textureSampleLevel(tex_a, samp_clamp, in.uv + vec2f(ts.x, 0.0), 0.0).xy * 0.15;
  v += textureSampleLevel(tex_a, samp_clamp, in.uv - vec2f(ts.x, 0.0), 0.0).xy * 0.15;
  v += textureSampleLevel(tex_a, samp_clamp, in.uv + vec2f(0.0, ts.y), 0.0).xy * 0.15;
  v += textureSampleLevel(tex_a, samp_clamp, in.uv - vec2f(0.0, ts.y), 0.0).xy * 0.15;
  let s = length(v) / (size.y * 1.5);
  let dir = atan2(v.y, v.x) / TAU;

  var col = pal(3) * 0.06;
  col += mix(pal(2), pal_grad(dir + 0.33), 0.35) * smoothstep(0.04, 0.4, s) * 0.55;
  col += pal(0) * smoothstep(0.3, 0.85, s) * 1.1;
  col += mix(pal(1), vec3f(1.0), 0.45) * smoothstep(0.75, 1.6, s) * 2.0;
  return vec4f(col * G.intensity, 1.0);
}
