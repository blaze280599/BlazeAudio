// Scene: "Molten Metal" — swirling caustic filaments (iterated trigonometric domain warp)
// mapped through a heat ramp. Bass heats the metal, kicks flare white-hot, beats send heat waves.

fn heat_ramp(h: f32) -> vec3f {
  var c = pal(3) * 0.35 * smoothstep(0.0, 0.15, h);
  c += pal(2) * 0.9 * smoothstep(0.05, 0.4, h);
  c += pal(0) * 1.4 * smoothstep(0.3, 0.7, h);
  c += mix(pal(1), vec3f(1.0), 0.65) * 3.2 * smoothstep(0.62, 1.0, h);
  return c;
}

@fragment
fn fs_molten(in: VsOut) -> @location(0) vec4f {
  var p = aspect_uv(in.uv);
  p.y = -p.y;
  let punch = G.kick * G.intensity;
  let t = G.flow * 0.32 + 23.0;
  p = rot2(G.time * 0.01) * p * (1.0 - 0.06 * G.bass_slow);

  let q = p * 3.6 - 250.0;
  var i = q;
  var c = 1.0;
  let inten = 0.0048;
  for (var n = 0; n < 5; n++) {
    let tt = t * (1.0 - 3.5 / f32(n + 1));
    i = q + vec2f(cos(tt - i.x) + sin(tt + i.y), sin(tt - i.y) + cos(tt + i.x));
    c += 1.0 / length(vec2f(q.x / (sin(i.x + tt) / inten), q.y / (cos(i.y + tt) / inten)));
  }
  c /= 5.0;
  c = 1.17 - pow(c, 1.4);
  let fil = clamp(pow(abs(c), 7.0), 0.0, 1.5);

  var wave = 0.0;
  let r = length(p);
  for (var k = 0; k < 4; k++) {
    let d = r - G.ring_age[k] * 1.1;
    wave += exp(-d * d * 60.0) * G.ring_str[k] * exp(-G.ring_age[k] * 1.5);
  }
  let glow = fbm(p * 1.4 + vec2f(t * 0.05, -t * 0.03));
  let heat = fil * (0.55 + G.bass * 0.7 + punch * 0.8) + wave * 0.45 * fil + glow * 0.12 * (0.4 + G.energy);
  var col = heat_ramp(clamp(heat, 0.0, 1.2));
  col += pal(3) * 0.12 * glow;
  return vec4f(col, 1.0);
}
