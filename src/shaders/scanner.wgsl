// Scene: "Scanner" — a CRT plotting the last few seconds of sound as stacked glowing ridge
// lines (newest at the front), swept by a scan beam that crosses the screen once per beat
// and leaves a phosphor afterglow. Beats ping rings; faint moiré interference shimmers behind.

const SC_LINES: f32 = 34.0;
const SC_BOTTOM: f32 = -0.62;
const SC_SPACING: f32 = 0.034;

fn sc_line_y(i: f32, x: f32) -> f32 {
  let age = i / SC_LINES;
  let bx = clamp(abs(x) / 0.85, 0.0, 1.0);
  let s = spec(bx, age * 0.9);
  let window = smoothstep(1.05, 0.25, abs(x));
  let jitter = 0.004 * sin(x * 37.0 + i * 1.7 + G.time * 2.0);
  return SC_BOTTOM + i * SC_SPACING + pow(s, 1.3) * 0.2 * window * G.intensity + jitter * window;
}

@fragment
fn fs_scanner(in: VsOut) -> @location(0) vec4f {
  let cv = crt_curve(in.uv, 0.06);
  let q = cv.xy;
  var p = aspect_uv(q);
  p.y = -p.y;
  let aspect = G.res.x * G.inv_res.y;
  let px = 2.0 * G.inv_res.y;

  // Background interference.
  let m1 = sin(p.y * 230.0 + sin(p.x * 2.7 + G.time * 0.6) * 5.0);
  let m2 = sin((p.y + p.x * 0.045) * 236.0);
  var col = pal(3) * 0.05 + pal(2) * max(m1 * m2, 0.0) * 0.025 * (0.5 + G.energy);

  // Scan beam, one sweep per beat.
  let bx = mix(-aspect - 0.1, aspect + 0.1, G.beat_phase);
  let behind = bx - p.x;
  let glow = select(0.12, 0.12 + 1.1 * exp(-behind * 2.2), behind >= 0.0);

  let k0 = floor((p.y - SC_BOTTOM) / SC_SPACING);
  for (var j = -1; j <= 6; j++) {
    let i = k0 - f32(j);
    if (i < 0.0 || i >= SC_LINES) { continue; }
    let y = sc_line_y(i, p.x);
    let d = abs(p.y - y) / px;
    if (d > 6.0) { continue; }
    // Hidden behind any nearer (lower) ridge that rises above this pixel.
    var hidden = 0.0;
    for (var f = 1; f <= 5; f++) {
      let fi = i - f32(f);
      if (fi < 0.0) { break; }
      if (sc_line_y(fi, p.x) > p.y + px) { hidden = 1.0; break; }
    }
    let fade = 1.0 - i / SC_LINES * 0.75;
    let line = exp(-d * d * 0.45) + exp(-d * 0.7) * 0.15;
    let tint = mix(pal(0), pal(1), i / SC_LINES);
    col += tint * line * glow * fade * (1.0 - hidden) * 1.4;
  }

  // The beam itself.
  let bd = abs(p.x - bx);
  col += mix(pal(1), vec3f(1.0), 0.5) * (exp(-bd * 90.0) * 1.4 + exp(-bd * 9.0) * 0.12) * (0.6 + G.kick);

  // Beat pings.
  let r = length(p - vec2f(0.0, -0.05));
  for (var k = 0; k < 4; k++) {
    let rr = abs(r - G.ring_age[k] * 0.9);
    col += pal(2) * exp(-rr / px * 0.6) * G.ring_str[k] * exp(-G.ring_age[k] * 2.0) * 0.8;
  }

  col = crt_finish(col, in.pos.xy, q, cv.z);
  return vec4f(col, 1.0);
}
