// Scene: "Warp" — an infinite neon tunnel. The walls are lit by the spectrogram history
// (newest sound right next to you, older sound receding into the distance), with hyperspace
// streaks, beat rings rushing toward the camera and a pulsing light at the vanishing point.

@fragment
fn fs_warp(in: VsOut) -> @location(0) vec4f {
  var p = aspect_uv(in.uv);
  p.y = -p.y;
  let punch = G.kick * G.intensity;

  let drift = vec2f(sin(G.time * 0.31), cos(G.time * 0.23)) * (0.1 + 0.08 * G.bass_slow);
  p -= drift;
  p = rot2(G.time * 0.06 + sin(G.time * 0.13) * 0.5) * p;
  p *= 1.0 + 0.05 * punch;

  let r = max(length(p), 1e-4);
  let ang = atan2(p.y, p.x);
  let z = 0.42 / r;
  let tw = ang + z * 0.1 * sin(G.time * 0.27);
  let u = tw / TAU + 0.5;
  let v = z + G.travel;
  let mirror = abs(fract(u * 2.0) * 2.0 - 1.0);
  let age = clamp((z - 0.3) / 10.0, 0.0, 1.0);
  let s = spec(mirror, age);
  let fog = exp(-z * 0.15);

  // Panel grid. Lines thin out with depth for free because uv is perspective-mapped.
  let segs = 36.0;
  let rings = 1.6;
  let gu = fract(u * segs);
  let gv = fract(v * rings);
  let eu = min(gu, 1.0 - gu);
  let ev = min(gv, 1.0 - gv);
  let aa = clamp(1.0 - z * 0.04, 0.2, 1.0);
  let grid = (exp(-eu * 70.0) + exp(-ev * 45.0)) * aa;
  let cell = vec2f(floor(u * segs), floor(v * rings));
  let ch = hash21(cell);
  let fill = smoothstep(0.35, 0.98, s) * (0.25 + 0.75 * ch * ch);
  let base = pal_grad(mirror * 0.55 + G.time * 0.02 + cell.y * 0.025);
  var col = base * (grid * (0.06 + 1.5 * s * s) + fill * fill * 1.6) * fog;

  // Hyperspace streaks on sparse lanes.
  let lanes = 96.0;
  let lane = floor(fract(u) * lanes);
  let lh = hash11(lane * 7.13 + 1.0);
  let lu = abs(fract(u * lanes) - 0.5);
  let sp = fract(v * 0.12 + lh * 13.0);
  let streak = smoothstep(0.78, 1.0, sp) * exp(-lu * lu * 80.0) * step(0.7, lh);
  col += mix(pal(1), vec3f(1.0), 0.45) * streak * (0.12 + G.energy * 1.1 + punch * 1.6 + G.hat * 0.6) * fog * 1.4;

  // Beat rings flying toward the viewer.
  for (var i = 0; i < 4; i++) {
    let age_i = G.ring_age[i];
    let zr = 9.0 / (1.0 + age_i * 7.0);
    let ring = exp(-abs(z - zr) * (5.0 / max(zr, 0.5)) * 3.0) * G.ring_str[i] * exp(-age_i * 1.2);
    col += mix(pal(2), vec3f(1.0), 0.25) * ring * 1.8 * exp(-zr * 0.08);
  }

  // Light at the end of the tunnel.
  col += mix(pal(0), vec3f(1.0), 0.4) * (0.01 / (r * r + 0.003)) * (0.2 + G.bass * 0.7 + punch * 0.9) * 0.18;
  col += pal(3) * 0.35 * (1.0 - fog * 0.5);

  return vec4f(col, 1.0);
}
