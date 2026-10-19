// Scene: "Halo" — mirrored radial spectrum around the album art, with a waveform ring,
// falling peak caps, inner reflections and beat shockwaves.

@fragment
fn fs_halo(in: VsOut) -> @location(0) vec4f {
  var p = aspect_uv(in.uv);
  p.y = -p.y;
  let punch = G.kick * G.intensity;
  p *= 1.0 - 0.04 * punch;
  p += (vec2f(hash11(G.time * 91.0), hash11(G.time * 57.0 + 3.0)) - 0.5) * 0.007 * punch * G.beat_strength;

  let r = length(p);
  let ang = atan2(p.y, p.x);
  let spin = G.time * 0.03 + G.cam_angle * 0.25;
  let px = 2.0 * G.inv_res.y;

  // ---- Background: tinted nebula + stars ----
  let q = rot2(G.time * 0.015) * p;
  var col = pal(3) * (0.35 + 0.8 * G.bass_slow) * exp(-r * 1.2);
  let n1 = fbm(q * 1.4 + vec2f(G.time * 0.025, -G.time * 0.018));
  let n2 = fbm(q * 2.8 - vec2f(G.time * 0.04, 0.0) + n1 * 1.5);
  col += mix(pal(2), pal(0), n2) * pow(n1, 3.0) * 0.16 * (0.45 + G.energy * 1.4);
  col += vec3f(stars(q + vec2f(G.time * 0.004, 0.0), 20.0) + 0.6 * stars(q * 1.7 + 3.1, 38.0)) * 0.55;

  let r0 = 0.29 + 0.035 * G.bass_slow + 0.028 * punch;
  let N = 128.0;
  let an = fract((ang + spin) / TAU + 1.0);
  let seg0 = floor(an * N);
  let reach = 0.36 * G.intensity * (1.0 + 0.22 * punch);

  // ---- Bars (evaluate neighbours so glows don't clip at segment edges) ----
  var bars = vec3f(0.0);
  for (var k = -1; k <= 1; k++) {
    let seg = seg0 + f32(k);
    let segw = (seg + N) % N;
    let bi = select(segw, N - 1.0 - segw, segw >= N * 0.5);
    let v = A.bands[u32(bi)];
    let pk = A.peaks[u32(bi)];
    let len = 0.006 + v * reach;
    let ca = (seg + 0.5) / N;
    let da = (an - ca) * TAU * r;
    let halfw = PI * r / N * 0.36;
    let dx = max(abs(da) - halfw, 0.0);

    let dy = max(max(r0 - r, r - (r0 + len)), 0.0);
    let d = length(vec2f(dx, dy));
    let hue = bi / 64.0 * 0.7 + G.time * 0.012;
    let c = pal_grad(hue);
    let tip = smoothstep(r0, r0 + len + 1e-3, r);
    let core = 1.0 - smoothstep(0.0, px * 1.2, d);
    let glow = exp(-d * 95.0) * 0.55 + exp(-d * 20.0) * 0.07;
    let hot = mix(c, vec3f(1.0), tip * v * v * 0.35);
    bars += hot * core * (0.45 + 1.5 * v + tip * 0.9 * v) + c * glow * (0.2 + 1.1 * v);

    // Inner reflection, pointing toward the center.
    let rin = r0 - 0.014;
    let lin = v * 0.07 * G.intensity;
    let dyi = max(max((rin - lin) - r, r - rin), 0.0);
    let di = length(vec2f(dx, dyi));
    bars += c * (1.0 - smoothstep(0.0, px, di)) * (0.25 + v * 0.9) * 0.6;

    // Falling peak cap.
    let pr = r0 + 0.014 + pk * reach;
    let dc = length(vec2f(dx, max(abs(r - pr) - 0.0016, 0.0)));
    let cap = 1.0 - smoothstep(0.0, px * 1.2, dc);
    bars += mix(c, vec3f(1.0), 0.65) * (cap * 1.9 + exp(-dc * 120.0) * 0.25) * smoothstep(0.02, 0.12, pk);
  }
  col += bars;

  // ---- God rays riding the energy ----
  let mirror_a = abs(an * 2.0 - 1.0);
  let ray = pow(noise2(vec2f(mirror_a * 40.0, G.time * 0.6)), 7.0);
  col += pal(1) * ray * smoothstep(r0, r0 + 0.25, r) * exp(-(r - r0) * 1.6) * (0.05 + G.energy * 0.5);

  // ---- Beat shockwaves ----
  for (var i = 0; i < 4; i++) {
    let age = G.ring_age[i];
    let rr = r0 + 0.02 + age * 1.15;
    let fade = G.ring_str[i] * exp(-age * 2.0);
    let dd = abs(r - rr);
    col += mix(pal(0), pal(2), f32(i) / 3.0) * (exp(-dd * 160.0) * 1.1 + exp(-dd * 22.0) * 0.12) * fade;
  }

  // ---- Inner ring + oscilloscope ----
  let ring = abs(r - (r0 - 0.006));
  col += mix(pal(0), vec3f(1.0), 0.35) * (exp(-ring * 1100.0) * (1.6 + 3.5 * punch) + exp(-ring * 70.0) * 0.12 * (1.0 + 2.0 * punch));
  let w = wave(mirror_a);
  let rw = r0 * 0.84 + w * 0.045 * G.intensity;
  let dw = abs(r - rw);
  col += pal(1) * (exp(-dw * 800.0) * 1.7 + exp(-dw * 90.0) * 0.12) * (0.35 + G.energy * 1.6);

  // ---- Album art (or plasma orb) in the core ----
  // Only core pixels need this, and the plasma orb (two fbm) only while no art is shown.
  let rd = r0 * 0.74;
  if (r < rd) {
    var auv = rot2(G.art_spin) * p / (rd * 2.0) * (1.0 - 0.06 * punch);
    auv = vec2f(auv.x, -auv.y) + 0.5;
    let art = textureSampleLevel(art_tex, samp_clamp, auv, 0.0).rgb;
    let edge = smoothstep(rd * 0.55, rd, r);
    let art_col = art * (0.62 + 0.55 * G.bass_slow + 0.6 * punch) * (1.0 - edge * 0.35);
    var orb = vec3f(0.0);
    if (G.art_present < 0.999) {
      let plasma = fbm(p * 5.0 + vec2f(G.time * 0.3, -G.time * 0.2) + fbm(p * 3.0 - G.time * 0.1));
      orb = pal_grad(plasma * 1.4 + G.time * 0.04) * (0.18 + G.bass * 1.2 + punch * 0.6) * (1.2 - r / rd) * plasma;
    }
    let inner = mix(orb, art_col, G.art_present);
    let mask = 1.0 - smoothstep(rd - px * 1.5, rd, r);
    col = mix(col, inner, mask);
  }
  let rim = abs(r - rd);
  col += pal(2) * (exp(-rim * 650.0) * (0.9 + 2.5 * punch) + exp(-rim * 60.0) * 0.08);

  return vec4f(col, 1.0);
}
