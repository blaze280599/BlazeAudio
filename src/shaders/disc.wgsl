// Scene: "Picture Disc" — late-night music TV. A CD printed with the album cover (or a bare
// chrome one) spins at the left edge, half of it in view, under a rainbow sheen that stays put
// while the disc turns. Beside it a "now playing." card shows the cover in three slipped strips,
// the title, artist and album, and a dotted spectrum meter, over the cover blurred into a wash of
// its own colours and the artist's name in giant ghost letters. Energy and beats speed up the spin
// (in disc.rs), beats knock the cover strips sideways and flash the disc, and a new track glitches
// the card in. Everything then goes through a VHS pass: soft luma, chroma that smears and lags,
// wobbling lines, a rolling tracking band and tape noise; beats shake the lines.
//
// Passes: fs_disc (everything -> composite), fs_vhs (composite -> HDR scene).

struct Params {
  angle: f32, radius: f32, face: f32, backdrop: f32,
  ghost: f32, vhs: f32, has_art: f32, info_t: f32,
  // Text width of atlas rows 0-3 (now playing, title, artist, album), in row heights.
  widths: vec4f,
  ghost_w: f32, ghost_h: f32, ghost_period: f32, scroll: f32,
};

@group(0) @binding(3) var<uniform> P: Params;
// fs_disc: t0 = album art, t1 = text atlas. fs_vhs: t0 = the composite.
@group(1) @binding(0) var t0: texture_2d<f32>;
@group(1) @binding(1) var t1: texture_2d<f32>;

const ROW_H: f32 = 128.0;
const ATLAS: vec2f = vec2f(2048.0, 640.0);
// The card, in card units (it scales with the room beside the disc).
const CARD: vec2f = vec2f(0.95, 0.56);
const DISC_X: f32 = -0.01;

fn spectrum(h: f32) -> vec3f {
  return clamp(abs(fract(h + vec3f(0.0, 2.0 / 3.0, 1.0 / 3.0)) * 6.0 - 3.0) - 1.0, vec3f(0.0), vec3f(1.0));
}

// Thin bright line at radius r0.
fn ring(r: f32, r0: f32, w: f32) -> f32 {
  let d = (r - r0) / max(w, 1e-4);
  return exp(-d * d);
}

// Colour bars for when nothing has a cover.
fn bars(uv: vec2f) -> vec3f {
  let i = u32(clamp(uv.x * 7.0, 0.0, 6.0));
  var top = array<vec3f, 7>(
    vec3f(0.52), vec3f(0.52, 0.52, 0.0), vec3f(0.0, 0.52, 0.52), vec3f(0.0, 0.52, 0.0),
    vec3f(0.52, 0.0, 0.52), vec3f(0.52, 0.0, 0.0), vec3f(0.0, 0.0, 0.52));
  if (uv.y < 0.67) { return top[i]; }
  if (uv.y < 0.75) { return select(vec3f(0.01), top[6u - i], i % 2u == 0u); }
  return vec3f(uv.x * uv.x * 0.6);
}

fn art(uv: vec2f) -> vec3f {
  if (P.has_art > 0.5) {
    return textureSampleLevel(t0, samp_clamp, uv, 0.0).rgb;
  }
  return bars(uv);
}

// Coverage of atlas row `row` at `l` (row heights from the row's top-left), 0 outside its text.
fn row_cov(row: f32, l: vec2f, w: f32, lod: f32) -> f32 {
  if (l.x < 0.0 || l.x > w || l.y < 0.0 || l.y > 1.0) { return 0.0; }
  let uv = vec2f(l.x, row + clamp(l.y, 0.01, 0.99)) * ROW_H / ATLAS;
  return textureSampleLevel(t1, samp_clamp, uv, lod).r;
}

// Row `row` drawn with its top-left at `o` and height `h` (card units, card scale `s`). Returns
// coverage and a soft drop shadow.
fn text(b: vec2f, o: vec2f, h: f32, row: f32, w: f32, s: f32) -> vec2f {
  let l = (b - o) / h;
  if (l.y < -0.2 || l.y > 1.3 || l.x < -0.4 || l.x > w + 0.4) { return vec2f(0.0); }
  let lod = clamp(log2(ROW_H / max(h * s * G.res.y, 1.0)), 0.0, 5.0);
  let cov = row_cov(row, l, w, lod);
  let sh = row_cov(row, l - vec2f(0.04, 0.06), w, min(lod + 1.5, 5.0));
  return vec2f(cov, sh);
}

fn backdrop(p: vec2f, aspect: f32) -> vec3f {
  if (P.backdrop > 0.5) {
    let d = p - vec2f(aspect * 0.6, 0.5);
    return vec3f(0.004) + pal(0) * 0.012 * exp(-dot(d, d) * 2.0);
  }
  // The cover itself, blurred to a few soft patches of its colours across the screen (from a small
  // mip, four taps to hide the bilinear creases). The palette is pushed toward vivid colours, so a
  // grey cover would come out tinted; this keeps it grey. Without a cover, the palette stands in.
  var tone = pal(0) * 0.3 + pal(1) * 0.08;
  if (P.has_art > 0.5) {
    let uv = (p - vec2f(aspect * 0.5, 0.5)) / aspect + 0.5;
    let o = 0.07;
    let c = (textureSampleLevel(t0, samp_clamp, uv + vec2f(o, o), 5.5).rgb + textureSampleLevel(t0, samp_clamp, uv + vec2f(-o, o), 5.5).rgb
      + textureSampleLevel(t0, samp_clamp, uv + vec2f(o, -o), 5.5).rgb + textureSampleLevel(t0, samp_clamp, uv - vec2f(o, o), 5.5).rgb) * 0.25;
    // Dark enough for the white text, without lifting a black cover off black.
    let l = luma(c);
    tone = c * clamp(l, 0.04, 0.16) / max(l, 1e-4);
  }
  // Darker at the top left, brightening toward the bottom right.
  let t = clamp(p.x / aspect * 0.65 + p.y * 0.6 - 0.15, 0.0, 1.0);
  var col = mix(tone * 0.12 + vec3f(0.003), tone, t * t);
  let d = p - vec2f(aspect * 0.62, 0.48);
  col += tone * 0.15 * exp(-dot(d, d) * 4.0) * (1.0 + G.energy * 0.5);
  return col;
}

// The giant ghost word: along the top, and turned upside down along the bottom.
fn ghost(p: vec2f, aspect: f32) -> f32 {
  if (P.ghost_w <= 0.0) { return 0.0; }
  var q = p;
  if (p.y > 0.5) { q = vec2f(aspect - p.x, 1.0 - p.y); }
  let x = q.x + P.scroll;
  let lx = x - floor(x / P.ghost_period) * P.ghost_period;
  return row_cov(4.0, vec2f(lx, q.y + 0.12) / P.ghost_h, P.ghost_w, 0.0);
}

fn disc(p: vec2f, bg: vec3f) -> vec3f {
  let R = max(P.radius, 0.05);
  let d = (p - vec2f(DISC_X, 0.5)) / R;
  let r = length(d);
  let aa = 1.5 / (R * G.res.y);
  var col = bg * (1.0 - 0.5 * smoothstep(1.1, 0.98, r));
  if (r > 1.0 + aa) { return col; }

  let a = atan2(d.y, d.x);
  // The disc's own frame, turned by the spin.
  let ca = cos(P.angle);
  let sa = sin(P.angle);
  let dr = vec2f(ca * d.x + sa * d.y, -sa * d.x + ca * d.y);
  // Light catches the disc in two wedges, top and bottom; the rainbow follows the angle.
  let wedge = pow(abs(sin(a + 0.2)), 8.0);
  let hue = a / PI * 0.9 + r * 0.25 + 0.55 + G.flow * 0.01;
  let rain = spectrum(hue);

  var face: vec3f;
  if (P.face < 0.5) {
    face = art(dr * 0.5 + 0.5) * (0.82 + 0.18 * wedge);
    face += rain * 0.07 + vec3f(1.0, 0.95, 0.97) * wedge * 0.12;
  } else {
    let silver = vec3f(0.4 + 0.2 * wedge);
    face = mix(silver, rain * (0.65 + 0.35 * r), 0.6 * (1.0 - wedge * 0.6));
    face += vec3f(1.0, 0.92, 0.95) * wedge * 0.35;
    // Faint printing turning with the disc, so the spin shows on bare chrome.
    let ra = atan2(dr.y, dr.x);
    let mark = step(0.55, hash11(floor((ra + PI) / TAU * 64.0) + 3.0));
    face *= 1.0 - 0.25 * mark * smoothstep(0.36, 0.365, r) * smoothstep(0.39, 0.385, r);
  }
  face *= 1.0 + 0.25 * G.kick * G.intensity;

  // Clear hub: mostly what's behind, with moulded rings, stamped text and the mirror band.
  var hub = bg * 0.3 + vec3f(0.012);
  hub += vec3f(0.3) * ring(r, 0.16, aa) + vec3f(0.2) * ring(r, 0.205, aa) + vec3f(0.26) * ring(r, 0.232, aa);
  let ra = atan2(dr.y, dr.x);
  let stamp = step(0.5, hash11(floor((ra + PI) / TAU * 120.0)));
  hub += vec3f(0.08) * stamp * smoothstep(0.248, 0.25, r) * smoothstep(0.262, 0.26, r);
  let mirror = smoothstep(0.276 - aa, 0.276 + aa, r);
  hub = mix(hub, vec3f(0.14 + 0.25 * wedge) + rain * 0.05, mirror * 0.85);

  var cd = select(face, hub, r < 0.33);
  cd *= 1.0 - 0.7 * ring(r, 0.33, aa * 1.5);
  cd = mix(cd, vec3f(0.55), ring(r, 0.992, aa) * 0.6);
  let cover = smoothstep(1.0 + aa, 1.0 - aa, r) * smoothstep(0.075 - aa, 0.075 + aa, r);
  return mix(col, cd, cover);
}

// Dotted spectrum meter: a column of dashes per band, mirrored about a resting dotted line.
fn meter(b: vec2f, s: f32) -> vec3f {
  let my = 0.475;
  let dy = abs(b.y - my);
  if (b.x < -0.03 || b.x > CARD.x + 0.03 || dy > 0.1) { return vec3f(0.0); }
  let pxu = 1.0 / (s * G.res.y);
  let n = 40.0;
  let ci = floor(b.x / CARD.x * n);
  let cx = abs(fract(b.x / CARD.x * n) - 0.5);
  let lvl = band(mix(0.04, 0.9, (ci + 0.5) / n)) * select(1.0, 0.0, ci < 0.0 || ci >= n);
  let step_y = 0.016;
  let k = floor(dy / step_y + 0.5);
  let ly = abs(dy - k * step_y);
  let cw = pxu * n / CARD.x;
  let dash = (1.0 - smoothstep(0.3 - cw, 0.3 + cw, cx)) * (1.0 - smoothstep(0.0035 - pxu, 0.0035 + pxu, ly));
  let top = floor(lvl * 5.5);
  let lit = select(0.0, 1.0, k <= top && lvl > 0.04);
  let peak = select(0.0, 1.0, k == top && k > 0.0);
  var c = vec3f(0.07) * select(0.0, 1.0, k == 0.0 && ci >= 0.0 && ci < n);
  c = max(c, mix(vec3f(0.9), pal(1), 0.25) * lit * (0.8 + G.energy * 0.6));
  c += pal(2) * peak * 1.2;
  c *= dash * select(0.0, 1.0, k <= 5.0);
  // End ticks.
  let tick = (1.0 - smoothstep(0.0015, 0.0015 + pxu, min(abs(b.x + 0.02), abs(b.x - CARD.x - 0.02)))) * step(dy, 0.05);
  return c + vec3f(0.5) * tick;
}

fn card(p: vec2f, col_in: vec3f, o: vec2f, s: f32) -> vec3f {
  var col = col_in;
  var b = (p - o) / s;
  if (b.x < -0.1 || b.x > 1.5 || b.y < -0.1 || b.y > CARD.y + 0.1) { return col; }

  // A new track (or coming on screen) glitches the card in: slipped lines that settle.
  let ti = min(P.info_t, G.scene_time);
  let g = 1.0 - smoothstep(0.0, 0.7, ti);
  let line = floor(b.y * 38.0);
  b.x += (hash11(line * 3.7 + floor(ti * 24.0)) - 0.5) * 0.3 * g * g * step(0.4, hash11(line + floor(ti * 12.0) * 1.3));
  let fade = smoothstep(0.0, 0.3, ti);

  // The cover in three strips, slipped sideways and knocked further on beats.
  let tq = (b - vec2f(0.0, 0.175)) / 0.21;
  if (tq.x > -0.2 && tq.x < 1.2 && tq.y > -0.1 && tq.y < 1.1) {
    let pxt = 1.0 / (s * G.res.y * 0.21);
    let strip = clamp(floor(tq.y * 3.0), 0.0, 2.0);
    let slip = (hash11(strip * 7.1 + G.beat_count * 1.37) - 0.5) * (0.04 + 0.16 * G.kick * G.intensity);
    let q = vec2f(tq.x + slip, tq.y);
    let ex = smoothstep(-pxt, pxt, q.x) * smoothstep(-pxt, pxt, 1.0 - q.x);
    let ey = smoothstep(-pxt, pxt, q.y) * smoothstep(-pxt, pxt, 1.0 - q.y);
    let f = fract(tq.y * 3.0);
    let gap = smoothstep(0.012, 0.012 + pxt * 3.0, min(f, 1.0 - f) / 3.0 + select(1.0, 0.0, tq.y > 0.2 && tq.y < 0.8));
    let sh = smoothstep(0.05, 0.0, max(max(-tq.x + 0.03, tq.x - 1.03), max(-tq.y + 0.04, tq.y - 1.04)));
    col *= 1.0 - 0.5 * sh * fade;
    col = mix(col, art(clamp(q, vec2f(0.0), vec2f(1.0))) * 1.05, ex * ey * gap * fade);
  }

  let np = text(b, vec2f(0.0, 0.0), 0.15, 0.0, P.widths.x, s);
  let t1 = text(b, vec2f(0.235, 0.165), 0.092, 1.0, P.widths.y, s);
  let t2 = text(b, vec2f(0.235, 0.235), 0.092, 2.0, P.widths.z, s);
  let t3 = text(b, vec2f(0.235, 0.305), 0.07, 3.0, P.widths.w, s);
  let cov = max(max(np.x, t1.x), max(t2.x, t3.x));
  let sh = max(max(np.y, t1.y), max(t2.y, t3.y));
  col *= 1.0 - 0.7 * sh * fade;
  col = mix(col, vec3f(1.05), cov * fade);
  return col + meter(b, s) * fade;
}

@fragment
fn fs_disc(in: VsOut) -> @location(0) vec4f {
  let aspect = G.res.x / G.res.y;
  let p = vec2f(in.uv.x * aspect, in.uv.y);
  var col = backdrop(p, aspect);
  let gh = ghost(p, aspect);
  col = mix(col, col * 0.35, gh * P.ghost) + col * gh * (1.0 - gh) * 0.8 * P.ghost;

  col = disc(p, col);
  // The card sits centred in the room beside the disc, scaled to fit it.
  let disc_right = DISC_X + P.radius;
  let room = aspect - disc_right;
  let s = clamp((room - 0.1) / CARD.x, 0.45, 1.15);
  let o = vec2f(disc_right + (room - CARD.x * s) * 0.5, 0.5 - CARD.y * 0.5 * s);
  col = card(p, col, o, s);
  return vec4f(max(col, vec3f(0.0)), 1.0);
}

fn to_yiq(c: vec3f) -> vec3f {
  return vec3f(dot(c, vec3f(0.299, 0.587, 0.114)), dot(c, vec3f(0.596, -0.274, -0.322)), dot(c, vec3f(0.211, -0.523, 0.312)));
}

fn from_yiq(c: vec3f) -> vec3f {
  return vec3f(c.x + 0.956 * c.y + 0.621 * c.z, c.x - 0.272 * c.y - 0.647 * c.z, c.x - 1.106 * c.y + 1.703 * c.z);
}

fn comp(x: f32, y: f32) -> vec3f {
  return textureSampleLevel(t0, samp_clamp, vec2f(x, y), 0.0).rgb;
}

@fragment
fn fs_vhs(in: VsOut) -> @location(0) vec4f {
  let v = P.vhs;
  let uv = in.uv;
  let t = G.time;
  // Tape lines, the same count at any resolution.
  let line = floor(uv.y * 480.0);
  // The tracking band rolls slowly down the picture.
  var by = uv.y - fract(t * 0.06);
  by -= round(by);
  let band_m = exp(-by * by / 0.0006);
  let wob = (noise2(vec2f(line * 0.35, t * 9.0)) - 0.5) * 0.0018;
  let kick = (hash11(line + floor(t * 60.0)) - 0.5) * 0.008 * G.kick * G.intensity;
  let torn = (hash11(line * 1.3 + floor(t * 30.0)) - 0.5) * 0.014 * band_m;
  let head = smoothstep(0.975, 1.0, uv.y) * (0.015 + 0.01 * hash11(line + floor(t * 30.0)));
  let x = uv.x + (wob + kick + torn + head) * v;

  let k = G.inv_res.x * G.res.y / 720.0 * v;
  let c0 = comp(x, uv.y);
  let cl = comp(x - 1.5 * k, uv.y);
  let cr = comp(x + 1.5 * k, uv.y);
  // Soft luma; chroma smeared wider and lagging to the right.
  let y = to_yiq(c0 * 0.5 + (cl + cr) * 0.25).x;
  let chroma = (cl + comp(x - 4.0 * k, uv.y) + comp(x - 7.0 * k, uv.y)) / 3.0;
  var col = max(from_yiq(vec3f(y, to_yiq(chroma).yz * 1.1)), vec3f(0.0));
  col = mix(c0, col, v);

  // Milky blacks, tape grain and white specks in the tracking band.
  let grain = hash21(in.pos.xy + fract(t) * vec2f(97.0, 31.0)) - 0.5;
  col += vec3f(0.004 + grain * 0.03 * (0.2 + y)) * v;
  let speck = step(0.994 - band_m * 0.03, hash21(vec2f(floor(in.pos.x / max(3.0 * G.res.y / 720.0, 1.0)), line + floor(t * 50.0))));
  col += vec3f(0.7) * speck * band_m * v;
  return vec4f(max(col, vec3f(0.0)), 1.0);
}
