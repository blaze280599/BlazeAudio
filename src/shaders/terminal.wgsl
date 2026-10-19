// Scene: "Faulty Terminal" — a flickering CRT full of procedural glyphs. Columns double as a
// spectrum analyzer (glyphs light up to each band's level), highs make the glyphs churn,
// and beats tear rows sideways with RGB-split glitches.

const GLYPH: vec2f = vec2f(6.0, 9.0);

fn glyph_on(id: f32, lp: vec2f) -> f32 {
  if (lp.x >= 5.0 || lp.y < 1.0 || lp.y >= 8.0) { return 0.0; }
  // Mirror horizontally for symmetric, character-like shapes.
  let mx = min(lp.x, 4.0 - lp.x);
  let bit = (lp.y - 1.0) * 3.0 + mx;
  return step(0.52, hash11(id * 41.3 + bit * 7.17));
}

@fragment
fn fs_terminal(in: VsOut) -> @location(0) vec4f {
  let cv = crt_curve(in.uv, 0.09);
  let q = cv.xy;
  let gp = max(2.0, round(3.0 * G.res.y / 1440.0));
  var frag = q * G.res;
  let csize = GLYPH * gp;

  // Beat glitch: tear random row bands sideways.
  let punch = G.kick * G.intensity;
  let row_band = floor(frag.y / (csize.y * 3.0));
  let tear = step(0.82, hash11(row_band * 3.1 + G.beat_count * 7.7)) * punch;
  frag.x += (hash11(row_band + G.beat_count) - 0.5) * tear * 160.0 * gp;

  let cell = floor(frag / csize);
  let lp = floor(frag / gp) - cell * GLYPH;
  let cols = floor(G.res.x / csize.x);
  let rows = floor(G.res.y / csize.y);

  let h1 = hash21(cell);
  let h2 = hash21(cell + 17.0);
  let rate = 0.15 + h1 * 1.6 * (0.3 + G.treble * 2.0 + G.hat * 2.0);
  let id = floor(G.flow * rate + h2 * 10.0) + h1 * 97.0;
  let on = glyph_on(id, lp);

  // Spectrum bars made of glyphs, mirrored around the center column.
  let bx = abs(cell.x / max(cols - 1.0, 1.0) * 2.0 - 1.0);
  let level = band(bx);
  let height = 1.0 - (cell.y + 0.5) / rows;
  let lit = smoothstep(level + 0.02, level - 0.02, height);
  let cap = exp(-abs(height - level) * rows * 0.8);

  let drift = noise2(cell * 0.08 + vec2f(G.flow * 0.05, -G.flow * 0.11));
  let flick = 0.92 + 0.08 * hash11(floor(G.time * 30.0));
  var bright = 0.03 + pow(drift, 3.0) * 0.22 + lit * (0.45 + level * 0.9) + cap * 0.8;
  bright *= flick * (0.85 + 0.3 * step(0.97, hash21(cell + floor(G.time * 8.0))));

  // Soft phosphor dots.
  let sub = fract(frag / gp) - 0.5;
  let dot_mask = exp(-dot(sub, sub) * 3.0);
  let tint = mix(pal(0), mix(pal(1), vec3f(1.0), 0.4), clamp(lit * level + cap, 0.0, 1.0));
  var col = tint * on * bright * dot_mask * 1.6;
  col += pal(0) * 0.012;
  col += pal(2) * tear * 0.3 * on;

  col = crt_finish(col, in.pos.xy, q, cv.z);
  return vec4f(col, 1.0);
}
