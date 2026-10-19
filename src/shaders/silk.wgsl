// Scene: "Silk" — flowing satin folds lit by a moving light. Bass deepens the folds, mids
// turn the light, highs sharpen the sheen and every beat sends a soft ripple through the cloth.

fn silk_ripple(p: vec2f) -> f32 {
  var r = 0.0;
  for (var i = 0; i < 4; i++) {
    let age = G.ring_age[i];
    let d = length(p) - age * 0.9;
    r += sin(d * 12.0) * exp(-d * d * 18.0) * G.ring_str[i] * exp(-age * 1.8);
  }
  return r;
}

fn silk_h(p: vec2f, t: f32, amp: f32) -> f32 {
  // Long diagonal folds, bent by slow cross-waves, with a finer secondary drape.
  let dir = normalize(vec2f(1.0, 0.55));
  let perp = vec2f(-dir.y, dir.x);
  let a = dot(p, dir);
  let b = dot(p, perp);
  let bend = 1.3 * sin(b * 0.9 + t * 0.45) + 0.6 * sin(b * 2.1 - t * 0.3 + a * 0.4);
  var h = sin(a * 2.3 + bend + t * 0.32);
  h += 0.38 * sin(a * 5.2 + bend * 1.8 - t * 0.5 + sin(b * 1.4 + t * 0.2) * 1.5);
  h += 0.12 * sin(a * 11.0 + bend * 2.4 + t * 0.7);
  return h * amp + silk_ripple(p) * 0.09;
}

@fragment
fn fs_silk(in: VsOut) -> @location(0) vec4f {
  var p = aspect_uv(in.uv);
  p.y = -p.y;
  let punch = G.kick * G.intensity;
  let t = G.flow * 0.6;
  p = rot2(sin(G.time * 0.03) * 0.2) * p * (1.0 - 0.05 * G.bass_slow);
  let amp = 0.8 + G.bass_slow * 0.55 * G.intensity + punch * 0.12;

  let e = 0.002;
  let h = silk_h(p, t, amp);
  let hx = silk_h(p + vec2f(e, 0.0), t, amp);
  let hy = silk_h(p + vec2f(0.0, e), t, amp);
  let n = normalize(vec3f(-(hx - h) / e, -(hy - h) / e, 2.6));

  let la = 2.2 + sin(G.time * 0.09) * 0.6 + G.cam_angle * 0.3;
  let l = normalize(vec3f(cos(la), sin(la), 0.8));
  let hv = normalize(l + vec3f(0.0, 0.0, 1.0));
  let diff = max(dot(n, l), 0.0);
  let nh = max(dot(n, hv), 0.0);
  let spec = pow(nh, 90.0);
  let sheen = pow(1.0 - n.z, 2.2);

  let tone = smoothstep(-1.4, 1.4, h);
  var col = mix(pal(3) * 0.35, pal(0) * 0.75, tone) * (0.04 + 0.96 * diff * diff);
  col += pal(2) * sheen * (0.25 + G.mid * 0.7);
  col += mix(pal(1), vec3f(1.0), 0.6) * spec * (1.1 + G.treble * 1.5 + punch * 2.0);
  col *= 0.75 + 0.4 * G.energy;
  return vec4f(col, 1.0);
}
