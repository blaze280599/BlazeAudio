// Scene: "Ferrofluid" — a glossy black sea of magnetic spikes, raymarched as a heightfield.
// Rolling swells raise fields of spikes, the spectrum lifts them left-to-right (bass at the
// center, highs at the sides) and the bass raises a crown in front of the camera.

const FERRO_CELL: f32 = 0.52;

fn hex_local(p: vec2f) -> vec2f {
  let s = vec2f(1.0, 1.7320508) * FERRO_CELL;
  let a = p - s * floor(p / s) - s * 0.5;
  let pb = p - s * 0.5;
  let b = pb - s * floor(pb / s) - s * 0.5;
  return select(b, a, dot(a, a) < dot(b, b));
}

fn ferro_amp(xz: vec2f) -> f32 {
  let swell = noise2(xz * 0.22 + vec2f(G.flow * 0.12, G.flow * 0.05));
  let e = smoothstep(0.42, 0.85, swell);
  let bx = clamp(abs(xz.x) / 4.5, 0.0, 1.0);
  let bandv = band(bx) * smoothstep(8.0, 2.0, xz.y);
  let cr = length(xz - vec2f(0.0, 3.0));
  let crown = exp(-cr * cr * 0.6) * (0.12 + G.bass * 0.85 + G.kick * 0.45);
  return (e * 0.45 + bandv * 0.6 + crown) * G.intensity;
}

fn ferro_h(xz: vec2f) -> f32 {
  let local = hex_local(xz);
  let d = length(local) / (FERRO_CELL * 0.5);
  let spike = pow(max(1.0 - d, 0.0), 1.8);
  let base = noise2(xz * 0.6 + G.flow * 0.08) * 0.06;
  return spike * ferro_amp(xz) * 1.25 + base;
}

fn ferro_env(r: vec3f) -> vec3f {
  // Studio: dark sky with two palette-tinted softboxes and a horizon line.
  var c = pal(3) * 0.02 + mix(pal(2), pal(1), 0.3) * 0.22 * smoothstep(-0.1, 0.9, r.y);
  let box1 = smoothstep(0.25, 0.32, r.y) * smoothstep(0.62, 0.55, r.y) * smoothstep(0.55, 0.2, abs(r.x + 0.35));
  let box2 = smoothstep(0.15, 0.22, r.y) * smoothstep(0.45, 0.4, r.y) * smoothstep(0.45, 0.15, abs(r.x - 0.5));
  c += mix(pal(1), vec3f(1.0), 0.5) * box1 * 2.2 * (0.7 + G.treble * 0.8);
  c += pal(0) * box2 * 1.6 * (0.6 + G.mid * 0.9);
  c += pal(2) * exp(-abs(r.y - 0.02) * 30.0) * 0.6;
  return c;
}

@fragment
fn fs_ferro(in: VsOut) -> @location(0) vec4f {
  var uv = aspect_uv(in.uv);
  uv.y = -uv.y;
  let ro = vec3f(sin(G.time * 0.07) * 0.3, 2.4 + G.bass_slow * 0.1, 0.0);
  // Positive pitch tilts the view down toward the fluid (rot2 maps y to c*y - s*z).
  let pitch = 0.62;
  var rd = normalize(vec3f(uv.x, uv.y, 1.6));
  let ryz = rot2(pitch) * vec2f(rd.y, rd.z);
  rd = vec3f(rd.x, ryz.x, ryz.y);

  var col = pal(3) * 0.04 * (1.0 + rd.y);
  var t = 0.2;
  var hit = false;
  for (var i = 0; i < 110; i++) {
    let p = ro + rd * t;
    let dh = p.y - ferro_h(p.xz);
    if (dh < 0.0015 * t) {
      hit = true;
      break;
    }
    t += max(dh * 0.38, 0.006 + t * 0.002);
    if (t > 22.0) { break; }
  }
  if (hit) {
    let p = ro + rd * t;
    let e = 0.006;
    let h0 = ferro_h(p.xz);
    let n = normalize(vec3f(h0 - ferro_h(p.xz + vec2f(e, 0.0)), e, h0 - ferro_h(p.xz + vec2f(0.0, e))));
    let r = reflect(rd, n);
    let fres = 0.04 + 0.96 * pow(1.0 - max(dot(n, -rd), 0.0), 5.0);
    let lp = normalize(vec3f(-0.4, 0.8, 0.5));
    let spec = pow(max(dot(reflect(-lp, n), -rd), 0.0), 140.0);
    var c = vec3f(0.004) + ferro_env(r) * (0.3 + 1.4 * fres);
    c += mix(pal(1), vec3f(1.0), 0.6) * spec * (1.5 + G.kick * 2.0);
    // Rim glow on the tallest spikes.
    c += pal(0) * smoothstep(0.5, 1.2, h0) * 0.03 * (0.5 + G.energy);
    let fog = exp(-t * 0.09);
    col = mix(col, c, fog);
  }
  return vec4f(col, 1.0);
}
