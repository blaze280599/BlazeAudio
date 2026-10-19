// Post: scene crossfade, physically-based bloom (Jimenez 13-tap down / tent up), ACES.

@group(1) @binding(0) var tex_a: texture_2d<f32>;
@group(1) @binding(1) var tex_b: texture_2d<f32>;

// ---- Crossfade: noisy radial dissolve with a glowing seam ----
@fragment
fn fs_composite(in: VsOut) -> @location(0) vec4f {
  let a = textureSampleLevel(tex_a, samp_clamp, in.uv, 0.0).rgb;
  let t = G.transition;
  if (t >= 1.0) {
    return vec4f(a, 1.0);
  }
  let b = textureSampleLevel(tex_b, samp_clamp, in.uv, 0.0).rgb;
  let p = aspect_uv(in.uv);
  let n = fbm(p * 2.2 + G.time * 0.25);
  let rr = length(p) * 0.55 + (n - 0.5) * 0.45;
  let e = t * 1.95 - 0.3;
  let m = 1.0 - smoothstep(e - 0.05, e + 0.05, rr);
  let seam = exp(-abs(rr - e) * 26.0) * sin(t * PI);
  let col = mix(b, a, m) + mix(pal(0), vec3f(1.0), 0.35) * seam * 2.2;
  return vec4f(col, 1.0);
}

// ---- Bloom ----
fn s(uv: vec2f, o: vec2f, ts: vec2f) -> vec3f {
  return textureSampleLevel(tex_a, samp_clamp, uv + o * ts, 0.0).rgb;
}

fn karis(c: vec3f) -> f32 {
  return 1.0 / (1.0 + luma(c));
}

fn downsample(uv: vec2f, first: bool) -> vec3f {
  let ts = 1.0 / vec2f(textureDimensions(tex_a));
  let a = s(uv, vec2f(-2.0, 2.0), ts);
  let b = s(uv, vec2f(0.0, 2.0), ts);
  let c = s(uv, vec2f(2.0, 2.0), ts);
  let d = s(uv, vec2f(-2.0, 0.0), ts);
  let e = s(uv, vec2f(0.0, 0.0), ts);
  let f = s(uv, vec2f(2.0, 0.0), ts);
  let g = s(uv, vec2f(-2.0, -2.0), ts);
  let h = s(uv, vec2f(0.0, -2.0), ts);
  let i = s(uv, vec2f(2.0, -2.0), ts);
  let j = s(uv, vec2f(-1.0, 1.0), ts);
  let k = s(uv, vec2f(1.0, 1.0), ts);
  let l = s(uv, vec2f(-1.0, -1.0), ts);
  let m = s(uv, vec2f(1.0, -1.0), ts);
  let g0 = (j + k + l + m) * 0.25;
  let g1 = (a + b + d + e) * 0.25;
  let g2 = (b + c + e + f) * 0.25;
  let g3 = (d + e + g + h) * 0.25;
  let g4 = (e + f + h + i) * 0.25;
  if (first) {
    // Karis average suppresses fireflies from single very bright pixels.
    let w0 = karis(g0) * 0.5;
    let w1 = karis(g1) * 0.125;
    let w2 = karis(g2) * 0.125;
    let w3 = karis(g3) * 0.125;
    let w4 = karis(g4) * 0.125;
    return (g0 * w0 + g1 * w1 + g2 * w2 + g3 * w3 + g4 * w4) / (w0 + w1 + w2 + w3 + w4);
  }
  return g0 * 0.5 + (g1 + g2 + g3 + g4) * 0.125;
}

@fragment
fn fs_down_first(in: VsOut) -> @location(0) vec4f {
  var c = downsample(in.uv, true);
  // Soft-knee threshold.
  let br = max(max(c.r, c.g), c.b);
  let knee = max(G.bloom_knee, 1e-4);
  var rq = clamp(br - G.bloom_threshold + knee, 0.0, 2.0 * knee);
  rq = rq * rq / (4.0 * knee);
  let w = max(rq, br - G.bloom_threshold) / max(br, 1e-4);
  c *= w;
  return vec4f(c, 1.0);
}

@fragment
fn fs_down(in: VsOut) -> @location(0) vec4f {
  return vec4f(downsample(in.uv, false), 1.0);
}

@fragment
fn fs_up(in: VsOut) -> @location(0) vec4f {
  let ts = 1.0 / vec2f(textureDimensions(tex_a));
  var c = s(in.uv, vec2f(0.0, 0.0), ts) * 4.0;
  c += (s(in.uv, vec2f(-1.0, 0.0), ts) + s(in.uv, vec2f(1.0, 0.0), ts) + s(in.uv, vec2f(0.0, -1.0), ts) + s(in.uv, vec2f(0.0, 1.0), ts)) * 2.0;
  c += s(in.uv, vec2f(-1.0, -1.0), ts) + s(in.uv, vec2f(1.0, -1.0), ts) + s(in.uv, vec2f(-1.0, 1.0), ts) + s(in.uv, vec2f(1.0, 1.0), ts);
  return vec4f(c / 16.0, 1.0);
}

// ---- Tonemap ----
fn rrt_odt_fit(v: vec3f) -> vec3f {
  let a = v * (v + 0.0245786) - 0.000090537;
  let b = v * (0.983729 * v + 0.4329510) + 0.238081;
  return a / b;
}

// Stephen Hill's fitted ACES (sRGB -> AP1 -> RRT+ODT -> sRGB).
fn aces_fitted(c: vec3f) -> vec3f {
  let aces_in = mat3x3f(
    vec3f(0.59719, 0.07600, 0.02840),
    vec3f(0.35458, 0.90834, 0.13383),
    vec3f(0.04823, 0.01566, 0.83777)
  );
  let aces_out = mat3x3f(
    vec3f(1.60475, -0.10208, -0.00327),
    vec3f(-0.53108, 1.10813, -0.07276),
    vec3f(-0.07367, -0.00605, 1.07602)
  );
  return clamp(aces_out * rrt_odt_fit(aces_in * c), vec3f(0.0), vec3f(1.0));
}

fn to_srgb(c: vec3f) -> vec3f {
  let lo = c * 12.92;
  let hi = 1.055 * pow(c, vec3f(1.0 / 2.4)) - 0.055;
  return select(hi, lo, c <= vec3f(0.0031308));
}

fn to_linear(c: vec3f) -> vec3f {
  let lo = c / 12.92;
  let hi = pow((c + 0.055) / 1.055, vec3f(2.4));
  return select(hi, lo, c <= vec3f(0.04045));
}

@fragment
fn fs_tonemap(in: VsOut) -> @location(0) vec4f {
  let uv = in.uv;
  let d = uv - 0.5;
  let ca = G.chromatic * (0.0025 + 0.012 * G.kick * G.intensity) * length(d) * 2.0;
  let dir = normalize(d + vec2f(1e-5));
  let o = dir * ca;
  var col = vec3f(
    textureSampleLevel(tex_a, samp_clamp, uv - o, 0.0).r,
    textureSampleLevel(tex_a, samp_clamp, uv, 0.0).g,
    textureSampleLevel(tex_a, samp_clamp, uv + o, 0.0).b
  );
  let bl = vec3f(
    textureSampleLevel(tex_b, samp_clamp, uv - o * 1.5, 0.0).r,
    textureSampleLevel(tex_b, samp_clamp, uv, 0.0).g,
    textureSampleLevel(tex_b, samp_clamp, uv + o * 1.5, 0.0).b
  );
  col += bl * G.bloom_strength * 0.22;
  col *= G.exposure * (1.0 + 0.1 * G.kick * G.intensity);

  let vd = length(d * vec2f(G.res.x * G.inv_res.y, 1.0));
  col *= mix(1.0, smoothstep(1.15, 0.2, vd), G.vignette);

  var m = aces_fitted(col);

  // Film grain + dither in display space so blacks don't band or turn noisy.
  var sr = to_srgb(m);
  let n = hash21(in.pos.xy + fract(G.time * 7.31) * vec2f(193.0, 71.0)) - 0.5;
  let n2 = hash21(in.pos.yx * 1.37 + fract(G.time * 3.17) * vec2f(53.0, 151.0)) - 0.5;
  sr += vec3f(n * G.grain * 0.045 * (0.4 + 0.6 * (1.0 - luma(sr))) + (n2 + n) * (0.5 / 255.0));
  m = to_linear(clamp(sr, vec3f(0.0), vec3f(1.0)));

  var alpha = 1.0;
  if (G.overlay > 0.5) {
    // Premultiplied output: dark pixels become transparent, bright ones glow over the desktop.
    let mx = max(max(m.r, m.g), m.b);
    alpha = smoothstep(0.015, 0.35, mx) * G.overlay_opacity;
    m = max(m - vec3f(0.006), vec3f(0.0)) * G.overlay_opacity;
  }
  return vec4f(m, alpha);
}
