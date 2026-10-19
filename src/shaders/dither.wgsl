// Scene: "Dither" — domain-warped ridged noise waves quantized to four palette shades through
// an 8x8 Bayer matrix on chunky pixels. Energy speeds the waves, bass lifts their brightness
// and every beat sends a ring through the field.

fn bayer8(c: vec2u) -> f32 {
  var v = 0u;
  for (var l = 0u; l < 3u; l++) {
    let x = (c.x >> l) & 1u;
    let y = (c.y >> l) & 1u;
    v |= (((x ^ y) << 1u) | y) << (2u * (2u - l));
  }
  return f32(v) / 64.0;
}

fn dfbm(p_in: vec2f) -> f32 {
  var p = p_in;
  var v = 0.0;
  var a = 1.0;
  for (var i = 0; i < 4; i++) {
    v += a * abs(gnoise2(p));
    p *= 2.7;
    a *= 0.32;
  }
  return v;
}

@fragment
fn fs_dither(in: VsOut) -> @location(0) vec4f {
  let cell_px = max(2.0, round(3.0 * G.res.y / 1440.0));
  let cell = floor(in.pos.xy / cell_px);
  let uv = (cell + 0.5) * cell_px * G.inv_res;
  var p = aspect_uv(uv);
  p.y = -p.y;
  let punch = G.kick * G.intensity;

  var warp = vec2f(0.0);
  var ring = 0.0;
  for (var k = 0; k < 4; k++) {
    let d = length(p);
    let life = G.ring_age[k];
    let w = exp(-pow((d - life * 0.9) / 0.07, 2.0)) * G.ring_str[k] * exp(-life * 1.4);
    warp += p / max(d, 1e-3) * w * 0.035;
    ring += w;
  }

  let s = (p + warp) * (1.05 - 0.05 * G.bass_slow);
  let t = G.flow * 0.07;
  var f = dfbm(s + dfbm(s - vec2f(t, t * 0.6)));
  f += ring * 0.5;
  f = (f - 0.62) * (1.05 + G.energy * 0.5) + 0.5 + (G.bass_slow - 0.3) * 0.3 + punch * 0.12;

  let thr = bayer8(vec2u(cell) % vec2u(8u)) - 0.5;
  let v = clamp(f + thr / 3.0, 0.0, 1.0);
  let idx = i32(floor(v * 3.0 + 0.5));
  var col = pal(3) * 0.25;
  if (idx == 1) { col = mix(pal(3), pal(2), 0.7) * 0.8; }
  if (idx == 2) { col = pal(0) * 1.1; }
  if (idx >= 3) { col = mix(pal(1), vec3f(1.0), 0.35) * (1.6 + punch * 1.5); }
  return vec4f(col, 1.0);
}
