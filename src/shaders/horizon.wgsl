// Scene: "Horizon" — raymarched synthwave landscape. Mountains on either side of the road
// are a 3D spectrogram: lateral distance = frequency, depth = time. A striped sun pulses
// with the bass.

const ROW_DEPTH: f32 = 0.1333; // world units per spectrogram row (34 units / 255 rows)

fn terrain(x: f32, z: f32) -> f32 {
  let ax = abs(x);
  let side = smoothstep(0.75, 2.3, ax);
  let bx = clamp((ax - 1.0) / 7.0, 0.0, 1.0);
  let age = clamp(z / 34.0, 0.0, 1.0);
  let s = spec(bx, age);
  let wz = z - G.scroll * ROW_DEPTH;
  let n = noise2(vec2f(x * 0.32, wz * 0.32));
  return side * (s * s * 2.4 * G.intensity + s * 0.55 + n * 0.4 * (0.35 + bx) + ax * 0.05);
}

fn sky(rd: vec3f, punch: f32) -> vec3f {
  let y = rd.y;
  var c = mix(pal(2) * 0.22, pal(3) * 0.35, smoothstep(-0.02, 0.45, y));
  c += pal(0) * exp(-abs(y) * 14.0) * 0.45 * (1.0 + G.bass_slow);

  let sp = vec2f(rd.x / rd.z, rd.y / rd.z) - vec2f(0.0, 0.17);
  let sr = 0.3 + G.bass_slow * 0.035 + punch * 0.025;
  let d = length(sp);
  let grad = mix(pal(1), pal(0), smoothstep(sr, -sr, sp.y));
  let lower = clamp(-sp.y / sr, 0.0, 1.0);
  let stripe = fract(sp.y * 16.0 - G.time * 0.35);
  let keep = select(1.0, step(lower * 0.55, stripe), sp.y < 0.0);
  let disc = 1.0 - smoothstep(sr - 0.003, sr, d);
  c += grad * disc * keep * (0.5 + punch * 0.6 + G.bass * 0.35);
  c += grad * exp(-max(d - sr, 0.0) * 6.5) * 0.22 * (1.0 + G.bass);

  let st = vec2f(atan2(rd.x, rd.z) * 3.0, y * 3.0);
  c += vec3f(stars(st, 26.0)) * smoothstep(0.05, 0.3, y) * 0.6;
  return c;
}

@fragment
fn fs_horizon(in: VsOut) -> @location(0) vec4f {
  var uv = aspect_uv(in.uv);
  uv.y = -uv.y;
  let punch = G.kick * G.intensity;

  let ro = vec3f(sin(G.time * 0.11) * 0.35, 1.2 + G.bass_slow * 0.3 + punch * 0.05, 0.0);
  var rd = normalize(vec3f(uv.x, uv.y - 0.1, 1.55));
  let roll = sin(G.time * 0.17) * 0.035;
  let rxy = rot2(roll) * rd.xy;
  rd = vec3f(rxy.x, rxy.y, rd.z);

  var col = sky(rd, punch);

  var t = 0.3;
  var hit = false;
  var hp = vec3f(0.0);
  for (var i = 0; i < 100; i++) {
    let pos = ro + rd * t;
    let h = terrain(pos.x, pos.z);
    let dh = pos.y - h;
    if (dh < 0.002 * t) {
      hit = true;
      hp = pos;
      break;
    }
    t += max(dh * 0.45, 0.02 + t * 0.004);
    if (t > 42.0) { break; }
  }

  let horizon_col = pal(0) * 0.35 + pal(2) * 0.15;
  if (hit) {
    let e = 0.04;
    let h0 = terrain(hp.x, hp.z);
    let nx = terrain(hp.x + e, hp.z) - h0;
    let nz = terrain(hp.x, hp.z + e) - h0;
    let n = normalize(vec3f(-nx, e, -nz));

    let gz = hp.z - G.scroll * ROW_DEPTH;
    let fx = abs(fract(hp.x) - 0.5);
    let fz = abs(fract(gz * 0.5) - 0.5);
    let wl = 0.025 + t * 0.003;
    let lx = 1.0 - smoothstep(0.0, wl, 0.5 - fx);
    let lz = 1.0 - smoothstep(0.0, wl * 1.6, 0.5 - fz);
    let grid = max(lx, lz) * exp(-t * 0.02);

    let bx = clamp((abs(hp.x) - 1.0) / 7.0, 0.0, 1.0);
    let s = spec(bx, clamp(hp.z / 34.0, 0.0, 1.0));
    let gc = mix(pal(0), pal(1), clamp(h0 * 0.45, 0.0, 1.0));
    var c = pal(3) * 0.5 * (0.25 + 0.75 * max(n.y, 0.0));
    c += gc * grid * (0.45 + s * 3.2 + punch * 0.9);
    c += pal(2) * pow(s, 3.0) * 0.7 * smoothstep(0.75, 2.3, abs(hp.x));

    // Road: center dashes and edge rails.
    let road = 1.0 - smoothstep(0.7, 0.8, abs(hp.x));
    let dash = step(0.5, fract(gz * 0.35)) * (1.0 - smoothstep(0.02, 0.04, abs(hp.x)));
    let rail = exp(-abs(abs(hp.x) - 0.72) * 60.0);
    c += (pal(1) * dash * 1.2 + pal(0) * rail * (0.8 + punch * 1.5)) * road;

    let fog = 1.0 - exp(-t * 0.06);
    col = mix(c, horizon_col, fog);
  }

  return vec4f(col, 1.0);
}
